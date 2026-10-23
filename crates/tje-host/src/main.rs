use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde::Serialize;
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use tje_host::{
    FrameSnapshot, Host, Mode, catalogue::FunctionCatalogue, hooks::HookRegistry,
    input::InputScript,
};

#[derive(Parser)]
#[command(about = "Headless ToeJam & Earl REV 02 reference/hybrid host")]
struct Arguments {
    #[arg(long)]
    rom: PathBuf,
    #[arg(long, default_value_t = 3600)]
    frames: u64,
    #[arg(long, value_enum, default_value_t = Mode::Check)]
    mode: Mode,
    #[arg(long)]
    input: Option<PathBuf>,
    /// Public decomp metadata; both --catalogue and --names must be provided.
    #[arg(long, requires = "names")]
    catalogue: Option<PathBuf>,
    #[arg(long, requires = "catalogue")]
    names: Option<PathBuf>,
    /// Use only the PreGenerateRandoms hook; GetRandomNumber remains untranslated.
    #[arg(long)]
    nested_demo: bool,
    /// Private output: per-frame RAM/VDP bytes and frames.jsonl. Never commit it.
    #[arg(long)]
    snapshot_dir: Option<PathBuf>,
    #[arg(long)]
    metrics: Option<PathBuf>,
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

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    ensure!(arguments.frames > 0, "--frames must be positive");
    let bytes = fs::read(&arguments.rom).context("read private ROM")?;
    let catalogue = match (&arguments.catalogue, &arguments.names) {
        (Some(catalogue), Some(names)) => FunctionCatalogue::from_decomp(
            &fs::read_to_string(catalogue)?,
            &fs::read_to_string(names)?,
        )?,
        _ => FunctionCatalogue::rev02_demo(),
    };
    let registry = if arguments.nested_demo {
        HookRegistry::nested_demo(&catalogue)?
    } else {
        HookRegistry::demo(&catalogue)?
    };
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
    let mut host = Host::new(&bytes, arguments.mode, registry)?;
    let start = Instant::now();
    let mut final_counter = 0;
    for frame in 0..arguments.frames {
        let snapshot = host.run_frame(script.at(frame))?;
        final_counter = snapshot.frame_counter;
        if let (Some(directory), Some(log)) = (&arguments.snapshot_dir, &mut snapshots) {
            save_snapshot(directory, log, &snapshot)?;
        }
    }
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
    let summary = serde_json::json!({
        "mode": format!("{:?}", arguments.mode), "frames": host.frame_count(),
        "elapsed_seconds": elapsed, "headless_fps": arguments.frames as f64 / elapsed,
        "final_frame_counter": final_counter,
        "intro_frame": host.measurements.intro_frame, "title_frame": host.measurements.title_frame,
        "menu_frame": host.measurements.menu_frame, "game_frame": host.measurements.game_frame,
        "logic_ticks": host.measurements.logic_ticks, "checked_calls": host.checks.calls,
        "checked_by_function": host.checks.by_function, "mismatches": host.checks.mismatches,
        "nested_original_calls": host.checks.nested_original_calls,
        "dead_stack_differing_calls": host.checks.dead_stack_differing_calls,
        "dead_stack_differing_bytes": host.checks.dead_stack_differing_bytes,
        "uninitialized_stack_read_sites": host.checks.uninitialized_stack_reads.len(),
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
        serde_json::to_writer_pretty(
            BufWriter::new(File::create(path)?),
            &serde_json::json!({
                "summary": summary, "checks": host.checks, "measurements": host.measurements,
            }),
        )?;
    }
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
