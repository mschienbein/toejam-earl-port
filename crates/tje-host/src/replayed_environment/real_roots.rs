//! Opt-in real-input witnesses. Original-only prefixes stop BEFORE a genuine
//! requested JSR/BSR. The check then captures/finishes that call normally; no
//! replay advances a requested boundary or installs a fixture CPU snapshot.
use super::*;
use crate::{
    Host, Mode,
    execution::WaitInputPoller,
    hooks::HookRegistry,
    input::{Button, InputHold, InputScript},
    sinks::Inputs,
};
use genesis_config::{GenesisController, GenesisInputs};
use jgenesis_common::frontend::TickEffect;
use std::{
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tje_game::{FramedTranslation, Translation, ids};

const ROOTS: &[FunctionId] = &[
    ids::WAIT_FOR_NEXT_V_BLANK,
    ids::PROCESS_INPUT,
    ids::PRESENT_TRY_END_ACTIVE_POWERUP,
    ids::MAP_QUEUE_VISIBLE_METATILE_ROW,
    ids::UPDATE_CAMERA_FOLLOWING,
];
const GENUINE_BLOCKING_SPEECH_ENTRY_FRAME: u64 = 1089;
static VALUES: LazyLock<Vec<Translation>> =
    LazyLock::new(|| tje_game::pending_translations().copied().collect());
static FRAMED: LazyLock<Vec<FramedTranslation>> =
    LazyLock::new(|| tje_game::pending_framed_translations().copied().collect());

enum Timeline {
    Holds(InputScript),
    Recording(Vec<(u64, GenesisInputs)>),
}
struct ScenarioInputs {
    source: String,
    frames: u64,
    timeline: Timeline,
}
impl WaitInputPoller for ScenarioInputs {
    fn at(&self, frame: u64) -> crate::Result<GenesisInputs> {
        if frame >= self.frames {
            return Err(crate::HostError::Input(format!(
                "committed input {} ended at frame{}",
                self.source, self.frames
            )));
        }
        match &self.timeline {
            Timeline::Holds(script) => Ok(script.at(frame)),
            Timeline::Recording(runs) => {
                let index = runs.partition_point(|(end, _)| *end <= frame);
                runs.get(index).map(|(_, input)| *input).ok_or_else(|| {
                    crate::HostError::Input("recorded controller run is missing".into())
                })
            }
        }
    }
}
fn scenario(revision: Revision, id: FunctionId) -> Arc<ScenarioInputs> {
    let suffix = if revision == Revision::Rev02 {
        "02"
    } else {
        "00"
    };
    let recording = if let Ok(stem) = std::env::var("TJE_REPLAY_SESSION_STEM") {
        Some(format!("rev{suffix}-{stem}.tje.json"))
    } else if id == ids::GENERATE_SPEECH {
        Some(format!("rev{suffix}-two-player.tje.json"))
    } else if id == ids::PRESENT_TRY_END_ACTIVE_POWERUP {
        Some(format!("rev{suffix}-one-player.tje.json"))
    } else if id == ids::MAP_QUEUE_VISIBLE_METATILE_ROW {
        Some(
            if revision == Revision::Rev02 {
                "rev02-content-ice-cream.tje.json"
            } else {
                "rev00-content-exploration.tje.json"
            }
            .into(),
        )
    } else {
        None
    };
    let Some(recording) = recording else {
        return Arc::new(ScenarioInputs {
            source: "crates/tje-host/examples/start-game.json".into(),
            frames: 10000,
            timeline: Timeline::Holds(
                InputScript::parse(include_str!("../../examples/start-game.json")).unwrap(),
            ),
        });
    };
    let path = PathBuf::from("../../sessions").join(&recording);
    let session = tje_session::Session::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(Revision::from(session.revision), revision);
    recorded_inputs(format!("sessions/{recording}"), session)
}
fn recorded_inputs(source: String, session: tje_session::Session) -> Arc<ScenarioInputs> {
    assert!(
        matches!(session.start, tje_session::Start::PowerOn),
        "controller-only power-on input is required"
    );
    let frames = session.frames;
    // Reuse the existing hold/controller adapter for each RLE run. Only the
    // compact timeline is expanded, never machine state or per-frame snapshots.
    let mut one = InputScript {
        p2_connected: false,
        holds: vec![InputHold {
            start: 0,
            end: 1,
            p1: Vec::with_capacity(8),
            p2: Vec::with_capacity(8),
        }],
    };
    let buttons = [
        (tje_session::UP, Button::Up),
        (tje_session::DOWN, Button::Down),
        (tje_session::LEFT, Button::Left),
        (tje_session::RIGHT, Button::Right),
        (tje_session::A, Button::A),
        (tje_session::B, Button::B),
        (tje_session::C, Button::C),
        (tje_session::START, Button::Start),
    ];
    let mut end = 0;
    let mut runs = Vec::with_capacity(session.runs.len());
    for tje_session::Run(count, p1, p2) in session.runs {
        one.p2_connected = p2 & tje_session::CONNECTED != 0;
        one.holds[0].p1.clear();
        one.holds[0].p2.clear();
        for &(mask, button) in &buttons {
            if p1 & mask != 0 {
                one.holds[0].p1.push(button);
            }
            if p2 & mask != 0 {
                one.holds[0].p2.push(button);
            }
        }
        let mut input = one.at(0);
        if p1 & tje_session::CONNECTED == 0 {
            input.p1 = GenesisController::None;
        }
        end += count;
        runs.push((end, input));
    }
    Arc::new(ScenarioInputs {
        source,
        frames,
        timeline: Timeline::Recording(runs),
    })
}
struct FrameInputs {
    frame: Option<u64>,
    input: Inputs,
}
fn target(host: &Host) -> Option<u32> {
    let cpu = host.emulator.hybrid_cpu();
    let bus = host.emulator.hybrid_bus();
    if bus.cycles.m68k_wait_cpu_cycles != 0 || !cpu.hybrid_instruction_ready(bus) {
        return None;
    }
    let pc = cpu.pc();
    let cart = bus.cartridge.as_ref()?;
    let opcode = cpu.next_opcode();
    // Read-only selector only: actual successful JSR admission is still minted
    // by the recorder, never by this predicted address or a CPU-shaped fixture.
    let address = |index: usize| {
        if index == 7 {
            cpu.stack_pointer()
        } else {
            cpu.address_registers()[index]
        }
    };
    let indexed = |base: u32| {
        let extension = cart.peek_word(pc + 2);
        let register = usize::from((extension >> 12) & 7);
        let raw = if extension & 0x8000 != 0 {
            address(register)
        } else {
            cpu.data_registers()[register]
        };
        let index = if extension & 0x0800 != 0 {
            raw
        } else {
            raw as u16 as i16 as i32 as u32
        };
        base.wrapping_add(index)
            .wrapping_add_signed(i32::from(extension as u8 as i8))
            & 0xffffff
    };
    match opcode {
        0x4eb9 => {
            Some((u32::from(cart.peek_word(pc + 2)) << 16) | u32::from(cart.peek_word(pc + 4)))
        }
        0x4eb8 => Some(cart.peek_word(pc + 2) as i16 as i32 as u32 & 0xffffff),
        0x4eba => Some(
            pc.wrapping_add(2)
                .wrapping_add_signed(i32::from(cart.peek_word(pc + 2) as i16))
                & 0xffffff,
        ),
        0x4ebb => Some(indexed(pc.wrapping_add(2))),
        _ if opcode & 0xfff8 == 0x4e90 => Some(address(usize::from(opcode & 7)) & 0xffffff),
        _ if opcode & 0xfff8 == 0x4ea8 => Some(
            address(usize::from(opcode & 7))
                .wrapping_add_signed(i32::from(cart.peek_word(pc + 2) as i16))
                & 0xffffff,
        ),
        _ if opcode & 0xfff8 == 0x4eb0 => Some(indexed(address(usize::from(opcode & 7)))),
        _ if opcode & 0xff00 == 0x6100 => {
            let displacement = if opcode as u8 == 0 {
                i32::from(cart.peek_word(pc + 2) as i16)
            } else {
                i32::from(opcode as u8 as i8)
            };
            Some(pc.wrapping_add(2).wrapping_add_signed(displacement) & 0xffffff)
        }
        _ => None,
    }
}
fn step(host: &mut Host, script: &ScenarioInputs, cache: &mut FrameInputs) -> crate::Result<()> {
    let frame = host.frame_count();
    if cache.frame != Some(frame) {
        cache.input.0 = script.at(frame)?;
        cache.frame = Some(frame);
    }
    host.wait_clock.frame = frame;
    host.wait_clock.inputs = cache.input.0;
    let bus = host.emulator.hybrid_bus();
    if bus.cycles.m68k_wait_cpu_cycles == 0
        && host.emulator.hybrid_cpu().hybrid_instruction_ready(bus)
    {
        host.observe_live_instruction(host.cpu().pc());
    }
    if host.tick_live_original(&mut cache.input)? == TickEffect::FrameRendered {
        host.finish_live_frame()?;
    }
    Ok(())
}
fn rom(revision: Revision) -> Vec<u8> {
    let (key, name) = match revision {
        Revision::Rev02 => ("TJE_REV02_ROM", "rev02.bin"),
        Revision::Rev00 => ("TJE_REV00_ROM", "rev00.bin"),
    };
    let path = std::env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../roms").join(name));
    std::fs::read(path).unwrap()
}
fn demonstration_roots(id: FunctionId) -> Vec<FunctionId> {
    let mut roots = vec![id];
    let supporting: &[FunctionId] = match id {
        ids::PROCESS_INPUT => &[
            ids::CHECK_START_BUTTON,
            ids::CHECK_B_BUTTON,
            ids::PROCESS_INVENTORY,
            ids::CHECK_IF_OPEN_FOR_BOTH,
        ],
        ids::PRESENT_TRY_END_ACTIVE_POWERUP => &[ids::HUD_LOAD_ACTION_ICON_SPRITE],
        ids::PROCESS_EARTHLINGS => &[
            ids::EARTHLING_DISPATCH_ABILITIES,
            ids::EARTHLING_DISPATCH_PLAYER_CONTACTS,
        ],
        _ => &[],
    };
    roots.extend_from_slice(supporting);
    roots
}
#[test]
#[ignore = "requires both private retail ROMs; emits runtime-only actual-call evidence"]
fn genuine_roots_and_named_regressions_both_revisions() {
    let mut failures = Vec::new();
    let filter = std::env::var("TJE_REPLAY_CASE_ROOT").ok();
    let window_end = std::env::var("TJE_REPLAY_WINDOW_END").ok().map(|value| {
        value
            .parse::<u64>()
            .expect("exclusive scene-search endpoint")
    });
    let blocking_search = std::env::var("TJE_REPLAY_REQUIRE_BLOCKING_WAIT_SEARCH")
        .ok()
        .as_deref()
        == Some("1");
    let scene_minimum = std::env::var("TJE_REPLAY_MINIMUM_ENTRY_FRAME")
        .ok()
        .map(|value| value.parse::<u64>().expect("scene-search start frame"));
    if blocking_search {
        assert_eq!(
            filter.as_deref(),
            Some(ids::GENERATE_SPEECH.metadata().inventory_id),
            "a blocking-speech search requires the explicit GenerateSpeech selector"
        );
        assert!(
            scene_minimum
                .zip(window_end)
                .is_some_and(|(start, end)| start < end),
            "a blocking-speech search needs an explicit nonempty frame window"
        );
    } else {
        assert!(
            scene_minimum.is_none(),
            "an alternate minimum belongs only to an explicitly labelled blocking-speech search"
        );
    }
    let mut exercised_cases = 0;
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (id, minimum_entry_frame) in ROOTS
            .iter()
            .copied()
            .map(|id| {
                (
                    id,
                    if id == ids::WAIT_FOR_NEXT_V_BLANK {
                        475
                    } else {
                        0
                    },
                )
            })
            .chain([
                (ids::WAIT_FOR_NEXT_V_BLANK, 492),
                // Same actual-JSR state as the retained REV00 strict-VDP failure;
                // this independent semantic-tier case is required in BOTH revisions.
                (ids::UPDATE_SPEECH_AND_DIALOGUE, 1687),
                (ids::UPDATE_SPEECH_AND_DIALOGUE, 2437),
                (ids::PROCESS_EARTHLINGS, 2437),
                (ids::GENERATE_SPEECH, GENUINE_BLOCKING_SPEECH_ENTRY_FRAME),
            ])
            .filter(|(id, _)| {
                filter
                    .as_deref()
                    .is_none_or(|wanted| wanted == id.metadata().inventory_id)
            })
        {
            exercised_cases += 1;
            let named_minimum_entry_frame = minimum_entry_frame;
            let minimum_entry_frame = scene_minimum.unwrap_or(minimum_entry_frame);
            let script = scenario(revision, id);
            let prefix_end = if let Some(end) = window_end {
                end.min(script.frames)
            } else if minimum_entry_frame == 0 {
                script.frames
            } else {
                (minimum_entry_frame + 1).min(script.frames)
            };
            let mut cache = FrameInputs {
                frame: None,
                input: Inputs(Default::default()),
            };
            let required = demonstration_roots(id);
            let registry =
                HookRegistry::proof_replayed_environment(revision, &VALUES, &FRAMED, &required)
                    .unwrap();
            // Explicitly unchecked original-only prefix. This selects a real
            // source call; it grants no startup or composed-prefix proof.
            let mut host = Host::new(&rom(revision), Mode::Reference, registry).unwrap();
            host.set_wait_input_poller(script.clone());
            let prefix = (|| -> crate::Result<()> {
                while host.frame_count() < prefix_end
                    && (host.frame_count() < minimum_entry_frame
                        || target(&host) != Some(id.metadata().address(revision)))
                {
                    step(&mut host, &script, &mut cache)?;
                }
                Ok(())
            })();
            if let Err(error) = prefix {
                failures.push(format!(
                    "{} {revision:?}: retained prefix failure: {error}",
                    id.metadata().name
                ));
                println!(
                    "REPLAY_SEARCH_PREFIX_FAILURE {}",
                    serde_json::json!({
                        "revision":revision.name(),"inventory_id":id.metadata().inventory_id,
                        "minimum_entry_frame":minimum_entry_frame,"frame":host.frame_count(),
                        "input_session":script.source,"blocking_wait_search":blocking_search,
                        "error":error.to_string(),"report":host.checks.replayed_environment,
                    })
                );
                continue;
            }
            if target(&host) != Some(id.metadata().address(revision))
                || window_end.is_none()
                    && minimum_entry_frame != 0
                    && host.frame_count() != minimum_entry_frame
            {
                failures.push(format!(
                    "{} {revision:?}: retained prefix did not reach its genuine selected caller",
                    id.metadata().name
                ));
                let finalization = if blocking_search {
                    host.finalize_replayed_campaign()
                        .err()
                        .map(|error| error.to_string())
                } else {
                    None
                };
                println!(
                    "REPLAY_PREFIX_UNEXERCISED {}",
                    serde_json::json!({
                        "revision":revision.name(),"inventory_id":id.metadata().inventory_id,
                        "minimum_entry_frame":minimum_entry_frame,"frame":host.frame_count(),"pc":host.cpu().pc(),
                        "window_end":prefix_end,"exact_window":window_end.is_none() && minimum_entry_frame != 0,
                        "input_session":script.source,
                        "blocking_wait_search":blocking_search,
                        "named_minimum_entry_frame":named_minimum_entry_frame,
                        "finalization_error":finalization,
                        "report":host.checks.replayed_environment,
                    })
                );
                continue;
            }
            let entry_frame = host.frame_count();
            let prefix_master_clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
            if blocking_search || id == ids::GENERATE_SPEECH {
                assert!(
                    host.replayed_state.as_ref().unwrap().real_wait.is_none(),
                    "the explicitly unchecked prefix cannot supply a prior Rust wait witness"
                );
                assert_eq!(
                    host.checks
                        .replayed_environment
                        .as_ref()
                        .unwrap()
                        .by_function[id.metadata().inventory_id]
                        .checked,
                    0
                );
                println!(
                    "REPLAY_SEARCH_ACTIVATION {}",
                    serde_json::json!({
                        "revision":revision.name(),"input_session":script.source,
                        "inventory_id":id.metadata().inventory_id,
                        "prefix_phase":"reference_only_unchecked",
                        "prefix_completed_video_frames":entry_frame,
                        "prefix_master_clock":prefix_master_clock,
                        "activation_phase":"check_before_actual_jsr_step",
                        "activation_original_pc":host.cpu().pc(),
                        "requested_roots":required.iter().map(|id|id.metadata().inventory_id).collect::<Vec<_>>(),
                        "window_minimum":minimum_entry_frame,"window_end":prefix_end,
                        "startup_or_prefix_proven":false,
                    })
                );
            }
            host.mode = Mode::Check;
            let mut result = (|| -> crate::Result<()> {
                loop {
                    step(&mut host, &script, &mut cache)?;
                    let report = host.checks.replayed_environment.as_ref().unwrap();
                    let waiting = host
                        .replayed_state
                        .as_ref()
                        .unwrap()
                        .real_wait
                        .is_some_and(|(function, _)| function == id);
                    if report.by_function[id.metadata().inventory_id].checked != 0
                        && (!blocking_search || waiting)
                    {
                        break;
                    }
                    if blocking_search && host.frame_count() >= prefix_end {
                        return Err(crate::HostError::Metadata(
                            "bounded genuine speech search reached no successful authenticated false-poll case".into(),
                        ));
                    }
                    assert!(
                        blocking_search || host.frame_count() <= entry_frame + 1000,
                        "actual call exceeded fixture's requested original-only frame window"
                    );
                }
                host.finalize_replayed_campaign()
            })();
            let real_wait = host
                .replayed_state
                .as_ref()
                .unwrap()
                .real_wait
                .is_some_and(|(function, _)| function == id);
            let real_wait_invocation = host
                .replayed_state
                .as_ref()
                .unwrap()
                .real_wait
                .filter(|(function, _)| *function == id)
                .map(|(_, invocation)| invocation.serial());
            if id == ids::GENERATE_SPEECH && result.is_ok() && !real_wait {
                result = Err(crate::HostError::Metadata(
                    "genuine GenerateSpeech case reached no authenticated false poll".into(),
                ));
            }
            if id == ids::GENERATE_SPEECH
                && result.is_ok()
                && !blocking_search
                && window_end.is_none()
                && (entry_frame != GENUINE_BLOCKING_SPEECH_ENTRY_FRAME
                    || host.frame_count() != GENUINE_BLOCKING_SPEECH_ENTRY_FRAME + 1)
            {
                result = Err(crate::HostError::Metadata(
                    "default genuine two-player speech regression must cover exactly frames1089→1090".into()));
            }
            println!(
                "REPLAY_REAL_ROOT {}",
                serde_json::json!({"revision":revision.name(), "inventory_id":id.metadata().inventory_id,
                "entry_frame":entry_frame, "return_frame":host.frame_count(), "input_session":script.source,
                "result":result.as_ref().err().map(ToString::to_string),
                "real_wait":real_wait,
                "blocking_wait_search":blocking_search,"real_wait_invocation_id":real_wait_invocation,
                "report":host.checks.replayed_environment})
            );
            let report = host
                .checks
                .replayed_environment
                .as_ref()
                .expect("explicit replay report");
            println!(
                "REPLAY_ROOT_RESULT {}",
                serde_json::json!({
                    "revision":revision.name(),"inventory_id":id.metadata().inventory_id,
                    "minimum_entry_frame":minimum_entry_frame,"entry_frame":entry_frame,"return_frame":host.frame_count(),
                    "window_end":prefix_end,"exact_window":window_end.is_none() && minimum_entry_frame != 0,
                    "result":result.as_ref().err().map(ToString::to_string),
                    "input_session":script.source,"real_wait":real_wait,
                    "blocking_wait_search":blocking_search,
                    "named_minimum_entry_frame":named_minimum_entry_frame,
                    "real_wait_invocation_id":real_wait_invocation,
                    "prefix_phase":"reference_only_unchecked",
                    "prefix_completed_video_frames":entry_frame,
                    "prefix_master_clock":prefix_master_clock,
                    "startup_or_composition_proven":false,
                    "activation_phase":"check_before_actual_jsr_step",
                    "startup_or_prefix_proven":false,
                    "complete":report.complete,"finalized":report.finalized,"mismatches":report.mismatches,
                    "unsupported":report.unsupported,"unresolved_obligations":report.unresolved_obligations,
                    "root":report.by_function[id.metadata().inventory_id],"failures":report.failures
                })
            );
            let handler_reads: Vec<_> = host
                .replayed_state
                .as_ref()
                .expect("explicit replay state")
                .handler_inputs
                .iter()
                .map(|(&(root, vector, handler_pc), &mask)| {
                    let registers =
                        super::report::OutputsReport::covered(&super::ledger::OutputCoverage {
                            state: mask,
                            ..super::ledger::OutputCoverage::default()
                        })
                        .registers;
                    serde_json::json!({"root":root.metadata().inventory_id, "vector":vector,
                        "handler_pc":handler_pc,"registers":registers})
                })
                .collect();
            println!(
                "REPLAY_ISR_INPUTS {}",
                serde_json::json!({
                    "schema":"tje-observed-interrupted-registers/v1","revision":revision.name(),
                    "fixture_root":id.metadata().inventory_id,"entry_frame":entry_frame,
                    "return_frame":host.frame_count(),
                    "input_session":script.source,
                    "scope":"finite observed actual IRQ paths; diagnostic bit unions, probes are not invocation counts",
                    "handlers":handler_reads
                })
            );
            if let Err(error) = result {
                failures.push(format!("{} {revision:?}: {error}", id.metadata().name));
            }
        }
    }
    assert_ne!(
        exercised_cases, 0,
        "requested real-root selector matched no case"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn prefix_recognizes_pc_relative_jsr_before_the_genuine_original_call() {
    let mut host =
        crate::fixture::tests::synthetic_host(&[0x4eba, 6, 0x60fe, 0x4e71, 0x4e75], &[0x4e73]);
    let mut inputs = Inputs(Default::default());
    for _ in 0..64 {
        if target(&host).is_some() {
            break;
        }
        host.tick_live_original(&mut inputs).unwrap();
    }
    assert_eq!(
        target(&host),
        Some(0x108),
        "PC-relative base is the extension word at PC+2"
    );
    host.tick_live_original(&mut inputs).unwrap();
    assert_eq!(
        host.cpu().pc(),
        0x108,
        "the real reference JSR must reach that target"
    );
    let sp = host.cpu().stack_pointer();
    assert_eq!(
        host.context().read_u32(sp),
        0x104,
        "the actual call owns its four-byte return slot"
    );
}

#[test]
fn compact_controller_inputs_preserve_masks_connection_and_half_open_bounds() {
    use tje_session::{A, CONNECTED, DOWN, LEFT, START, UP};
    let mut session = tje_session::Session::new(Revision::Rev02, "authored controller runs");
    session.frames = 6;
    session.runs = vec![
        tje_session::Run(2, CONNECTED | UP | START, CONNECTED | DOWN),
        tje_session::Run(3, 0, CONNECTED | LEFT | A),
        tje_session::Run(1, CONNECTED, 0),
    ];
    session.validate().unwrap();
    let recording = recorded_inputs("synthetic controller input".into(), session);
    let expected = InputScript::parse(
        r#"{"p2_connected":true,"holds":[
        {"start":0,"end":2,"p1":["up","start"],"p2":["down"]},
        {"start":2,"end":5,"p2":["left","a"]}]}"#,
    )
    .unwrap();
    for frame in 0..6 {
        let mut input = expected.at(frame);
        if (2..5).contains(&frame) {
            input.p1 = GenesisController::None;
        }
        if frame == 5 {
            input.p2 = GenesisController::None;
        }
        assert_eq!(recording.at(frame).unwrap(), input, "frame {frame}");
    }
    assert!(
        recording.at(6).is_err(),
        "never extend a committed recording"
    );
}

#[test]
fn prefix_selects_register_and_indexed_jsr_without_creating_call_authority() {
    for (program, expected, return_pc) in [
        (
            &[0x41f9, 0, 0x010c, 0x4e90, 0x60fe, 0x4e71, 0x4e75][..],
            0x10c,
            0x108,
        ),
        (
            &[0x7002, 0x4ebb, 6, 0x60fe, 0x4e71, 0x4e71, 0x4e75][..],
            0x10c,
            0x106,
        ),
        (
            &[
                0x41f9, 0, 0x0110, 0x72fe, 0x4eb0, 0x1002, 0x60fe, 0x4e71, 0x4e75,
            ][..],
            0x110,
            0x10c,
        ),
    ] {
        let mut host = crate::fixture::tests::synthetic_host(program, &[0x4e73]);
        let mut inputs = Inputs(Default::default());
        for _ in 0..100 {
            if target(&host).is_some() {
                break;
            }
            host.tick_live_original(&mut inputs).unwrap();
        }
        assert_eq!(target(&host), Some(expected));
        for _ in 0..100 {
            host.tick_live_original(&mut inputs).unwrap();
            if host.cpu().pc() == expected {
                break;
            }
        }
        assert_eq!(
            host.cpu().pc(),
            expected,
            "actual reference JSR must execute"
        );
        let sp = host.cpu().stack_pointer();
        assert_eq!(host.context().read_u32(sp), return_pc);
    }
}

mod mutations;
