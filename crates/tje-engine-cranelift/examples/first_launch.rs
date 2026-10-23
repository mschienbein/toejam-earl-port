//! The desktop frontend's complete loading/play path, without window/audio devices.
//! Example: cargo run --release -p tje-engine-cranelift --example first_launch --
//! --rom /private/rev02.bin --data-dir /private/native-cache --frames 600
use anyhow::{Context, Result};
use clap::Parser;
use genesis_config::{GenesisEmulatorConfig, GenesisInputs};
use serde_json::json;
use std::{path::PathBuf, time::Instant};
use tje_engine_cranelift::{CacheConfig, CompileProgress, ENGINE_VERSION, load_engine};
use tje_rom::Rom;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    rom: PathBuf,
    /// Defaults to the desktop frontend's per-user data directory.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long, default_value_t = 600)]
    frames: u64,
    #[arg(long, default_value_t = 60)]
    warmup_frames: u64,
    /// Metadata-only JSON; never contains ROM, RAM, pixels or audio samples.
    #[arg(long)]
    output: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    anyhow::ensure!(args.frames != 0, "--frames must be nonzero");
    let imported = Rom::import(&std::fs::read(&args.rom).context("read user's ROM")?)?;
    let revision = imported.revision();
    let cache = match args.data_dir {
        Some(data_dir) => CacheConfig { data_dir },
        None => CacheConfig::for_current_user()?,
    };
    let load_start = Instant::now();
    let mut last_percent = None;
    let mut engine = load_engine(
        imported,
        &GenesisEmulatorConfig::default(),
        &cache,
        |progress| match progress {
            CompileProgress::Compiling { completed, total } => {
                let percent = completed * 100 / total;
                if last_percent.is_none_or(|last| percent >= last + 5) || completed == total {
                    eprintln!("compiling native continuations: {completed}/{total} ({percent}%)");
                    last_percent = Some(percent);
                }
            }
            other => eprintln!("{other:?}"),
        },
    )?;
    let load_seconds = load_start.elapsed().as_secs_f64();
    engine.set_audio_sample_rate(48_000)?;
    for _ in 0..args.warmup_frames {
        engine.play_frame(GenesisInputs::default())?;
    }
    let run_start = Instant::now();
    let mut audio_samples = 0_u64;
    for _ in 0..args.frames {
        audio_samples += engine.play_frame(GenesisInputs::default())?.len() as u64;
    }
    let runtime_seconds = run_start.elapsed().as_secs_f64();
    let stats = engine.backend().stats();
    let fps = args.frames as f64 / runtime_seconds;
    let report = json!({
        "engine_version": ENGINE_VERSION,
        "os": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "revision": revision.name(),
        "instruction_count": stats.instruction_count,
        "code_bytes": stats.code_bytes,
        "cache_bytes": stats.cache_bytes,
        "compilation_seconds": stats.compilation_time.as_secs_f64(),
        "backend_startup_seconds": stats.startup_time.as_secs_f64(),
        "total_load_seconds": load_seconds,
        "cache_status": format!("{:?}", stats.cache_status),
        "cache_rejection": stats.cache_rejection,
        "warmup_frames": args.warmup_frames,
        "measured_frames": args.frames,
        "completed_frames": engine.frame_count(),
        "audio_sample_count": audio_samples,
        "runtime_seconds": runtime_seconds,
        "frames_per_second": fps,
        "realtime_multiple": fps / engine.target_fps(),
    });
    let text = serde_json::to_string_pretty(&report)?;
    if let Some(path) = args.output {
        std::fs::write(path, &text)?;
    }
    println!("{text}");
    Ok(())
}
