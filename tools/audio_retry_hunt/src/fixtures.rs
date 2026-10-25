//! Explicitly authored physical-byte fixtures, separated from natural search.
use super::{
    Args, GenesisEmulator, GenesisEmulatorConfig, Inputs, Result, Rom, Saves, Session, Sink, Start,
    TickEffect, inputs,
};
use anyhow::{Context, ensure};
use genesis_core::reference_proof::{
    AudioOwner, AudioRetryReceipt, AudioRetryRecipe, RetailRevision,
};
use genesis_core::timing_survey::{TimingCpu, TimingKind};
use jgenesis_common::frontend::EmulatorTrait;
use serde::Serialize;
use sha1::{Digest, Sha1};
use std::fs;

#[derive(Debug, Serialize)]
struct ExecutedEntry {
    frame: u64,
    original_instruction_ordinal: u64,
    pc: u32,
}
#[derive(Debug, Serialize)]
struct LockRead {
    frame: u64,
    original_instruction_ordinal: u64,
    pc: u32,
    scoped_access_ordinal: u64,
    clock: u64,
    value: u8,
}
#[derive(Serialize)]
struct FixtureReport {
    format: u32,
    scope: &'static str,
    natural: bool,
    source_session_frames: u64,
    started_input_frames: u64,
    completed_video_frames: u64,
    recipe_replayed_exactly: bool,
    stop_boundary: &'static str,
    executed_retry_entries: Vec<ExecutedEntry>,
    original_lock_reads: Vec<LockRead>,
    receipt: AudioRetryReceipt,
}
fn original_total(machine: &GenesisEmulator) -> u64 {
    let counts = machine
        .hybrid_cpu()
        .hybrid_instruction_counts()
        .expect("proof constructor enables original opcode observer");
    counts.main_thread + counts.exception_handler
}
pub(super) fn run(args: &Args, rom: &Rom, _pcs: &[u32; 7]) -> Result<()> {
    ensure!(
        args.session.len() == 1
            && args.sweep_end == 0
            && !args.neutrality
            && args.output_dir.is_none(),
        "fixture mode requires one input-only session and no natural-sweep/output options"
    );
    ensure!(
        args.fixture_call > 0,
        "one-based original owner occurrence required"
    );
    let expected = args
        .fixture_recipe
        .as_ref()
        .map(|path| -> Result<AudioRetryRecipe> {
            let document: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
            serde_json::from_value(
                document
                    .get("receipt")
                    .and_then(|receipt| receipt.get("recipe"))
                    .context("labelled fixture receipt.recipe missing")?
                    .clone(),
            )
            .map_err(Into::into)
        })
        .transpose()?;
    let owner = match args.fixture.as_deref() {
        Some("pcm") => AudioOwner::Pcm,
        Some("psg") => AudioOwner::Psg,
        None => {
            expected
                .as_ref()
                .context("fixture owner or exact recipe required")?
                .owner
        }
        _ => unreachable!("clap validates owner"),
    };
    let revision = match rom.revision() {
        tje_rom::Revision::Rev02 => RetailRevision::Rev02,
        tje_rom::Revision::Rev00 => RetailRevision::Rev00,
    };
    let source = &args.session[0];
    ensure!(
        source.is_relative(),
        "reproducible fixture session path must be repository-relative"
    );
    let text = fs::read_to_string(source)?;
    let session = Session::from_json(&text)?;
    ensure!(
        matches!(session.start, Start::PowerOn),
        "authored device fixture starts from an input-only power-on corpus tape"
    );
    ensure!(
        tje_rom::Revision::from(session.revision) == rom.revision(),
        "session/cartridge revision mismatch"
    );
    let session_sha1 = format!("{:x}", Sha1::digest(text.as_bytes()));
    if let Some(expected) = &expected {
        ensure!(
            expected.session == source.to_str().context("UTF-8 session path required")?
                && expected.session_sha1 == session_sha1
                && expected.revision == revision
                && expected.owner == owner,
            "recipe/input/revision binding differs"
        );
    }
    let mut machine = GenesisEmulator::create_with_reference_proof(
        Some(rom.bytes().to_vec()),
        GenesisEmulatorConfig::default(),
        &mut Saves,
    );
    let mut seen = 0;
    let mut armed = false;
    let mut retry_entries = Vec::new();
    let mut lock_reads = Vec::new();
    let mut scope_access = 0;
    let mut completed_frames = 0;
    let mut receipt = None;
    let mut started_frames = 0;
    'frames: for (frame, pads) in session
        .inputs()
        .take(args.limit.unwrap_or(1200))
        .enumerate()
    {
        started_frames = frame as u64 + 1;
        let mut pads = Inputs(inputs(pads));
        loop {
            let ready = machine.hybrid_bus().cycles.m68k_wait_cpu_cycles == 0
                && machine
                    .hybrid_cpu()
                    .hybrid_instruction_ready(machine.hybrid_bus());
            if !armed && ready && machine.hybrid_cpu().pc() == owner.entry(revision) {
                seen += 1;
                let ordinal = original_total(&machine) + 1;
                let selected = expected
                    .as_ref()
                    .map_or(seen == args.fixture_call, |recipe| {
                        recipe.frame == frame as u64
                            && recipe.original_instruction_ordinal == ordinal
                    });
                if selected {
                    let recipe = expected.clone().unwrap_or_else(|| AudioRetryRecipe {
                        id: format!(
                            "{}-audio-{}-busy-window-owner-{}",
                            match revision {
                                RetailRevision::Rev02 => "rev02",
                                RetailRevision::Rev00 => "rev00",
                            },
                            match owner {
                                AudioOwner::Pcm => "pcm",
                                AudioOwner::Psg => "psg",
                            },
                            seen
                        ),
                        session: source.to_str().unwrap().into(),
                        session_sha1: session_sha1.clone(),
                        revision,
                        frame: frame as u64,
                        original_instruction_ordinal: ordinal,
                        owner,
                    });
                    machine.arm_audio_retry_fixture(recipe)?;
                    armed = true;
                }
            }
            let before = original_total(&machine);
            let effect = match machine.tick(&mut Sink, &mut Sink, &mut pads, &mut Saves) {
                Ok(effect) => effect,
                Err(error) => {
                    let diagnostic = serde_json::json!({"format":1,"natural":false,"status":"failed","error":error.to_string(),"receipt":machine.audio_retry_fixture_diagnostic()});
                    fs::write(&args.report, serde_json::to_vec_pretty(&diagnostic)?)?;
                    return Err(error.into());
                }
            };
            let after = original_total(&machine);
            if armed && after != before {
                let actual = machine
                    .hybrid_cpu()
                    .hybrid_instruction_counts()
                    .unwrap()
                    .last_instruction
                    .unwrap();
                if actual.completed_successfully && actual.pc == owner.retry(revision) {
                    retry_entries.push(ExecutedEntry {
                        frame: frame as u64,
                        original_instruction_ordinal: after,
                        pc: actual.pc,
                    });
                }
            }
            for access in machine.drain_reference_timing_accesses().unwrap() {
                if access.cpu == TimingCpu::M68k {
                    if armed {
                        scope_access += 1;
                    }
                    if armed
                        && access.kind == TimingKind::Z80Ram
                        && !access.write
                        && access.address == 0xa00041
                    {
                        lock_reads.push(LockRead {
                            frame: frame as u64,
                            original_instruction_ordinal: after,
                            pc: access.pc,
                            scoped_access_ordinal: scope_access,
                            clock: access.clock,
                            value: access.value as u8,
                        });
                    }
                }
            }
            if effect == TickEffect::FrameRendered {
                completed_frames += 1;
            }
            let returned = machine
                .audio_retry_fixture_diagnostic()
                .is_some_and(|record| {
                    machine.hybrid_cpu().pc() == record.caller_return_pc
                        && machine.hybrid_cpu().stack_pointer() == record.caller_sp.wrapping_add(4)
                });
            if armed && returned {
                let live = machine.finish_audio_retry_fixture()?;
                receipt = Some(live.evidence().clone());
                break 'frames;
            }
            if effect == TickEffect::FrameRendered {
                break;
            }
        }
    }
    let receipt = receipt.context(
        "bounded input replay did not complete the authored interval and original owner RTS",
    )?;
    ensure!(
        !retry_entries.is_empty(),
        "original did not execute the branch/fallthrough retry entry"
    );
    ensure!(
        lock_reads
            .iter()
            .any(|read| read.pc == owner.entry(revision) + 0x1e && read.value == 1),
        "missing original owner lock-busy read"
    );
    ensure!(
        lock_reads
            .iter()
            .any(|read| read.pc == owner.entry(revision) + 0x1e && read.value == 0),
        "missing original unlocked re-acquisition"
    );
    if let Some(expected) = &expected {
        ensure!(&receipt.recipe == expected, "replayed recipe changed");
    }
    let report = FixtureReport {
        format: 1,
        scope: "Labelled authored transient Z80 byte41 perturbation on a real input-only corpus state; original reference execution only; not natural coverage or Rust/body/control-profile admission",
        natural: false,
        source_session_frames: session.frames,
        started_input_frames: started_frames,
        completed_video_frames: completed_frames,
        recipe_replayed_exactly: expected.is_some(),
        stop_boundary: "Immediately after the original owner RTS reaches its retained existing caller frame; no fake return frame or synthetic fragment RTS",
        executed_retry_entries: retry_entries,
        original_lock_reads: lock_reads,
        receipt,
    };
    fs::write(&args.report, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"id":report.receipt.recipe.id,"natural":false,"frame":report.receipt.recipe.frame,"retry_pc":owner.retry(revision),"executions":report.executed_retry_entries.len(),"eligible_ticks":report.receipt.eligible_ticks.len(),"physical_byte_events":report.receipt.byte_events.len(),"completed_original_owner":true})
    );
    Ok(())
}
