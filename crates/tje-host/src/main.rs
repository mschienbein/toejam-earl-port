use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde::Serialize;
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use tje_host::future_frame_watch::{ProvenanceStatus, ProvenanceSummary};
use tje_host::{FrameSnapshot, Host, Mode, hooks::HookRegistry, input::InputScript};

#[derive(Parser)]
#[command(about = "Headless ToeJam & Earl REV 02/REV 00 reference/hybrid host")]
struct Arguments {
    #[arg(long)]
    rom: PathBuf,
    #[arg(long, default_value_t = 3600)]
    frames: u64,
    #[arg(long, value_enum, default_value_t = Mode::Check)]
    mode: Mode,
    #[arg(long)]
    input: Option<PathBuf>,
    /// Check only these subsystem hooks (repeat or comma-separate).
    #[arg(long, value_delimiter = ',')]
    subsystem: Vec<String>,
    /// Check only these canonical function names or inventory func_ IDs.
    #[arg(long, value_delimiter = ',')]
    function: Vec<String>,
    /// Private output: per-frame RAM/VDP bytes and frames.jsonl. Never commit it.
    #[arg(long)]
    snapshot_dir: Option<PathBuf>,
    #[arg(long)]
    metrics: Option<PathBuf>,
    /// Opt-in complete private check/IRQ observations; never commit this output.
    #[arg(long, requires = "metrics")]
    capture_checks: bool,
    /// Private final framebuffer as PPM. Never commit it.
    #[arg(long)]
    frame_out: Option<PathBuf>,
    #[arg(long)]
    require_game: bool,
    #[arg(long)]
    require_title: bool,
}

#[derive(Serialize)]
struct FrameLog<'a> {
    frame: u64,
    frame_counter: u16,
    pc: u32,
    idle_random_draws: u64,
    lag_events: &'a [tje_host::LagEvent],
    vdp_registers: &'a [u8],
    writes: &'a [tje_host::HardwareWrite],
    audio_samples: usize,
}

fn save_snapshot(
    directory: &Path,
    log: &mut BufWriter<File>,
    snapshot: &FrameSnapshot,
) -> Result<()> {
    let stem = format!("{:06}", snapshot.frame);
    fs::write(directory.join(format!("{stem}.ram")), &snapshot.ram)?;
    fs::write(directory.join(format!("{stem}.vram")), &snapshot.vdp.vram)?;
    let words = |values: &[u16]| {
        values
            .iter()
            .flat_map(|word| word.to_be_bytes())
            .collect::<Vec<_>>()
    };
    fs::write(
        directory.join(format!("{stem}.cram")),
        words(&snapshot.vdp.cram),
    )?;
    fs::write(
        directory.join(format!("{stem}.vsram")),
        words(&snapshot.vdp.vsram),
    )?;
    let mut audio = Vec::with_capacity(snapshot.audio_samples.len() * 16);
    for sample in &snapshot.audio_samples {
        for channel in sample {
            audio.extend_from_slice(&channel.to_bits().to_le_bytes());
        }
    }
    fs::write(directory.join(format!("{stem}.pcm64le")), audio)?;
    serde_json::to_writer(
        &mut *log,
        &FrameLog {
            frame: snapshot.frame,
            frame_counter: snapshot.frame_counter,
            pc: snapshot.pc,
            idle_random_draws: snapshot.idle_random_draws,
            lag_events: &snapshot.lag_events,
            vdp_registers: &snapshot.vdp.registers,
            writes: &snapshot.writes,
            audio_samples: snapshot.audio_samples.len(),
        },
    )?;
    writeln!(log)?;
    Ok(())
}

#[derive(Serialize)]
struct GuardSummary {
    unsupported_stack_read: Option<tje_host::UnsupportedStackRead>,
    unknown_origin_proof_debt: u64,
}
impl GuardSummary {
    fn capture(
        error: Option<&tje_host::HostError>,
        metrics: &tje_host::ReadableGuardMetrics,
    ) -> Self {
        Self {
            unsupported_stack_read: error
                .and_then(tje_host::HostError::typed_stack_read)
                .map(Into::into),
            unknown_origin_proof_debt: metrics.unknown_origin_proof_debt,
        }
    }
}

fn proof_status(run_failed: bool, mismatches: u64, provenance: ProvenanceSummary) -> &'static str {
    if run_failed
        || mismatches != 0
        || provenance.invalidated != 0
        || provenance.status == ProvenanceStatus::Invalidated
    {
        "failed"
    } else if !provenance.is_proof_eligible() {
        "pending"
    } else {
        "complete"
    }
}

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    ensure!(arguments.frames > 0, "--frames must be positive");
    let bytes = fs::read(&arguments.rom).context("read private ROM")?;
    let rom = tje_rom::Rom::import(&bytes)?;
    let revision = rom.revision();
    ensure!(
        arguments.mode == Mode::Check
            || (arguments.subsystem.is_empty() && arguments.function.is_empty()),
        "hook selection requires --mode check"
    );
    let selected_subsystems: Vec<_> = arguments.subsystem.iter().map(String::as_str).collect();
    let selected_functions = arguments
        .function
        .iter()
        .map(|name| {
            tje_game::functions()
                .find(|function| function.name == name || function.inventory_id == name)
                .map(|function| function.id)
                .with_context(|| format!("unknown function name or inventory ID: {name}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let registry = HookRegistry::filtered(revision, &selected_subsystems, &selected_functions)?;
    let script = match arguments.input {
        Some(path) => InputScript::parse(&fs::read_to_string(path)?)?,
        None => InputScript::default(),
    };
    let mut snapshots = if let Some(directory) = &arguments.snapshot_dir {
        fs::create_dir_all(directory)?;
        Some(BufWriter::new(File::create(
            directory.join("frames.jsonl"),
        )?))
    } else {
        None
    };
    let mut host = Host::from_rom(rom, arguments.mode, registry)?;
    if arguments.capture_checks {
        host.set_check_capture(true)?;
    }
    let start = Instant::now();
    let mut final_counter = 0;
    let mut run_error = None;
    for frame in 0..arguments.frames {
        let snapshot = match host.run_frame(script.at(frame)) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                run_error = Some(error);
                break;
            }
        };
        final_counter = snapshot.frame_counter;
        if let (Some(directory), Some(log)) = (&arguments.snapshot_dir, &mut snapshots) {
            save_snapshot(directory, log, &snapshot)?;
        }
    }
    let provenance = host.finish_future_frame_provenance();
    if let Some(log) = &mut snapshots {
        log.flush()?;
    }
    let elapsed = start.elapsed().as_secs_f64();
    if let Some(path) = arguments.frame_out {
        let (pixels, size) = host.frame_buffer();
        let mut image = BufWriter::new(File::create(path)?);
        write!(image, "P6\n{} {}\n255\n", size.width, size.height)?;
        for color in &pixels[..size.len() as usize] {
            image.write_all(&[color.r, color.g, color.b])?;
        }
        image.flush()?;
    }
    let game_draws = host
        .measurements
        .game_frame
        .map(|frame| &host.measurements.idle_draws_per_frame[frame as usize..]);
    let draws = game_draws.unwrap_or(&[]);
    let idle_total: u64 = draws.iter().sum();
    let guard = GuardSummary::capture(run_error.as_ref(), &host.checks.readable_stack_guard);
    let summary = serde_json::json!({
        "revision": revision.name(),
        "error": run_error.as_ref().map(ToString::to_string),
        "unsupported_stack_read": guard.unsupported_stack_read,
        "unknown_origin_proof_debt": guard.unknown_origin_proof_debt,
        "proof_status": proof_status(run_error.is_some(), host.checks.mismatches, provenance),
        "future_frame_provenance": provenance,
        "mode": format!("{:?}", arguments.mode), "frames": host.frame_count(),
        "elapsed_seconds": elapsed, "headless_fps": arguments.frames as f64 / elapsed,
        "final_frame_counter": final_counter,
        "intro_frame": host.measurements.intro_frame, "title_frame": host.measurements.title_frame,
        "menu_frame": host.measurements.menu_frame, "game_frame": host.measurements.game_frame,
        "logic_ticks": host.measurements.logic_ticks, "checked_calls": host.checks.calls,
        "coverage_by_function": host.checks.by_function, "mismatches": host.checks.mismatches,
        "coverage_by_subsystem": host.checks.by_subsystem,
        "unexercised": host.checks.unexercised,
        "nested_original_calls": host.checks.nested_original_calls,
        "dead_stack_differing_calls": host.checks.dead_stack_differing_calls,
        "dead_stack_differing_bytes": host.checks.dead_stack_differing_bytes,
        "uninitialized_stack_read_sites": host.checks.uninitialized_stack_reads.len(),
        "nondependent_stack_read_sites": host.checks.nondependent_stack_reads.len(),
        "nondependent_stack_reads": host.checks.nondependent_stack_reads,
        "stack_dependency_reruns": host.checks.stack_dependency_reruns,
        "last_vdp_mismatch": host.checks.last_vdp_mismatch,
        "idle_random_draws": { "game_frames": draws.len(), "total": idle_total,
            "min": draws.iter().min(), "max": draws.iter().max(),
            "mean": if draws.is_empty() { 0.0 } else { idle_total as f64 / draws.len() as f64 } },
        "lag_frames": host.measurements.lag_events.len(),
        "vdp_writes": host.measurements.vdp_writes,
        "ym2612_writes": host.measurements.ym2612_writes,
        "psg_writes": host.measurements.psg_writes,
    });
    println!("{}", serde_json::to_string_pretty(&summary)?);
    if let Some(path) = arguments.metrics {
        let mut report = serde_json::json!({
            "summary": summary, "checks": host.checks, "measurements": host.measurements,
        });
        if arguments.capture_checks {
            report["actual_check_capture"] = serde_json::to_value(host.actual_check_capture())?;
        }
        serde_json::to_writer_pretty(BufWriter::new(File::create(path)?), &report)?;
    }
    if let Some(error) = run_error {
        return Err(error.into());
    }
    ensure!(
        host.checks.mismatches == 0,
        "check run completed with {} counted mismatches",
        host.checks.mismatches,
    );
    provenance.ensure_proof_eligible()?;
    ensure!(
        !arguments.require_title || host.measurements.title_frame.is_some(),
        "script did not reach TitleScreenVideo"
    );
    ensure!(
        !arguments.require_game || host.measurements.game_frame.is_some(),
        "script did not reach InGameLoop"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_capture_cli_is_explicit_and_requires_private_metrics_output() {
        assert!(
            !Arguments::try_parse_from(["tje-host", "--rom", "authored.bin"])
                .unwrap()
                .capture_checks
        );
        assert!(
            Arguments::try_parse_from(["tje-host", "--rom", "authored.bin", "--capture-checks"])
                .is_err()
        );
        let arguments = Arguments::try_parse_from([
            "tje-host",
            "--rom",
            "authored.bin",
            "--capture-checks",
            "--metrics",
            "/tmp/authored-capture.json",
        ])
        .unwrap();
        assert!(arguments.capture_checks);
        assert_eq!(
            arguments.metrics,
            Some(PathBuf::from("/tmp/authored-capture.json"))
        );
    }

    #[test]
    fn unresolved_zero_mismatch_provenance_is_pending_not_a_clean_cli_success() {
        let pending = ProvenanceSummary {
            status: ProvenanceStatus::Provisional,
            approvals: 1,
            provisional: 1,
            ..Default::default()
        };
        assert_eq!(proof_status(false, 0, pending), "pending");
        assert!(
            pending
                .ensure_proof_eligible()
                .unwrap_err()
                .to_string()
                .contains("pending")
        );
        assert_eq!(
            proof_status(false, 0, ProvenanceSummary::default()),
            "complete"
        );
        assert!(ProvenanceSummary::default().ensure_proof_eligible().is_ok());
    }

    #[test]
    fn invalidated_provenance_or_a_counted_failure_cannot_be_complete() {
        let invalidated = ProvenanceSummary {
            status: ProvenanceStatus::Invalidated,
            approvals: 1,
            invalidated: 1,
            ..Default::default()
        };
        assert_eq!(proof_status(false, 0, invalidated), "failed");
        assert!(invalidated.ensure_proof_eligible().is_err());
        assert_eq!(
            proof_status(false, 1, ProvenanceSummary::default()),
            "failed"
        );
        assert_eq!(
            proof_status(true, 0, ProvenanceSummary::default()),
            "failed"
        );
    }
    #[test]
    fn summary_retains_the_typed_top_level_guard_cause_through_probe_wrappers() {
        use tje_game::{
            ids,
            stack_read::{Guard, Reader},
        };
        let guard = Guard::default();
        let cause = guard
            .run_root(ids::GET_RANDOM_NUMBER, 0xff8000, || {
                guard.read(0xef7ff8, 2, Reader::RustMemory);
            })
            .unwrap_err();
        let error = tje_host::HostError::DependencyProbe {
            cause: Box::new(tje_host::HostError::Translation(cause.into())),
        };
        let summary = GuardSummary::capture(Some(&error), &Default::default());
        let value = serde_json::to_value(summary).unwrap();
        let read = &value["unsupported_stack_read"];
        assert_eq!(read["root"], "GetRandomNumber");
        assert_eq!(read["entry_sp"], 0xff8000);
        assert_eq!(read["raw_address"], 0xef7ff8);
        assert_eq!(read["address"], 0xff7ff8);
        assert_eq!(read["width"], 2);
        assert_eq!(read["reader"], "rust_memory");
        assert_eq!(read["reason"], "unwritten_stack_read");
        let ordinary = tje_host::HostError::Metadata("authored ordinary error".into());
        assert!(
            GuardSummary::capture(Some(&ordinary), &Default::default())
                .unsupported_stack_read
                .is_none()
        );
    }

    #[test]
    fn summary_unknown_origin_debt_is_independent_of_failures_and_retired_ownership() {
        let metrics = tje_game::stack_read::DmaMetrics {
            unknown_origin_proof_debt: 3,
            active: 0,
            queued: 0,
            failed: 0,
            ..Default::default()
        };
        let metrics = tje_host::ReadableGuardMetrics::from(metrics);
        let value = serde_json::to_value(GuardSummary::capture(None, &metrics)).unwrap();
        assert_eq!(value["unknown_origin_proof_debt"], 3);
        assert!(value["unsupported_stack_read"].is_null());
        let check_metrics = serde_json::to_value(metrics).unwrap();
        assert_eq!(check_metrics["unknown_origin_proof_debt"], 3);
        assert_eq!(check_metrics["active_owned_dma"], 0);
        assert_eq!(check_metrics["queued_owned_commands"], 0);
        assert_eq!(check_metrics["dma_failed"], 0);
        assert_eq!(
            serde_json::to_value(GuardSummary::capture(None, &Default::default())).unwrap()["unknown_origin_proof_debt"],
            0
        );
    }
}
