//! Private-ROM snapshot determinism and timing evidence. Only hashes/metadata
//! are written; never commit runtime states, cartridge bytes or generated code.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use serde_json::json;
use sha1::{Digest, Sha1};
use std::{path::PathBuf, time::Instant};
use tje_engine_cranelift::{CacheConfig, Engine, load_engine};
use tje_engine_runtime::MachineSnapshot;
use tje_mem::Bus;
use tje_rom::Rom;
use tje_session::{CONNECTED, Session, Start};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    rom: PathBuf,
    #[arg(long)]
    data_dir: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(required = true)]
    sessions: Vec<PathBuf>,
}
fn inputs(pads: [u16; 2]) -> GenesisInputs {
    let [p1, p2] = pads.map(|bits| {
        let mut controller = if bits & CONNECTED != 0 {
            GenesisController::new(GenesisControllerType::ThreeButton)
        } else {
            GenesisController::None
        };
        for (bit, button) in [
            GenesisButton::Up,
            GenesisButton::Down,
            GenesisButton::Left,
            GenesisButton::Right,
            GenesisButton::A,
            GenesisButton::B,
            GenesisButton::C,
            GenesisButton::Start,
        ]
        .into_iter()
        .enumerate()
        {
            controller.set_field(button, bits & (1 << bit) != 0);
        }
        controller
    });
    GenesisInputs { p1, p2 }
}
fn step(engine: &mut Engine, session: &Session, frame: usize, pads: [u16; 2]) -> Result<()> {
    if let Start::Recipe { writes, .. } = &session.start {
        for write in writes
            .iter()
            .filter(|write| write.before_frame == frame as u64)
        {
            let bytes = write.value.to_be_bytes();
            for (offset, byte) in bytes[4 - usize::from(write.width)..].iter().enumerate() {
                engine
                    .devices_mut()
                    .memory_mut()
                    .write_u8(write.address + offset as u32, *byte);
            }
        }
    }
    engine.play_frame(inputs(pads))?;
    Ok(())
}
fn hashes(engine: &Engine) -> Result<[String; 3]> {
    let image = engine.snapshot()?;
    let (pixels, _) = engine.frame_buffer();
    let mut video = Sha1::new();
    for color in pixels {
        video.update([color.r, color.g, color.b, color.a]);
    }
    let mut audio = Sha1::new();
    for sample in engine.devices().audio_samples() {
        for channel in sample {
            audio.update(channel.to_bits().to_le_bytes());
        }
    }
    let hex = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    Ok([
        hex(&image.state_hash()),
        hex(&video.finalize()),
        hex(&audio.finalize()),
    ])
}
fn distribution(mut times: Vec<f64>) -> serde_json::Value {
    times.sort_by(f64::total_cmp);
    json!({ "iterations": times.len(), "min_us": times[0], "median_us": times[times.len() / 2], "p95_us": times[times.len() * 95 / 100], "max_us": times[times.len() - 1] })
}
fn main() -> Result<()> {
    let args = Args::parse();
    let rom = Rom::import(&std::fs::read(&args.rom)?)?;
    let revision = rom.revision();
    let mut engine = load_engine(
        rom,
        &GenesisEmulatorConfig::default(),
        &CacheConfig {
            data_dir: args.data_dir,
        },
        |_| {},
    )?;
    engine.set_audio_sample_rate(48_000)?;
    let mut reports = Vec::new();
    for path in args.sessions {
        let session = Session::from_json(&std::fs::read_to_string(&path)?)?;
        ensure!(
            session.revision == revision.into(),
            "session/ROM revision mismatch"
        );
        let pads: Vec<_> = session.inputs().collect();
        ensure!(pads.len() > 3, "session must contain at least four frames");
        engine.reset();
        let checkpoints = [1, pads.len() / 3, pads.len() * 2 / 3];
        let mut saved = Vec::new();
        let mut expected = Vec::new();
        for (frame, &pads) in pads.iter().enumerate() {
            step(&mut engine, &session, frame, pads)?;
            expected.push(hashes(&engine)?);
            if checkpoints.contains(&(frame + 1)) {
                saved.push(engine.snapshot()?);
            }
        }
        let mut replayed = 0;
        for checkpoint in &saved {
            let decoded = MachineSnapshot::from_bytes(checkpoint.as_bytes().to_vec())?;
            engine.restore(&decoded)?;
            ensure!(
                engine.snapshot()?.as_bytes() == checkpoint.as_bytes(),
                "immediate restore changed canonical state"
            );
            let start = checkpoint.frame_count() as usize;
            for frame in start..(start + 120).min(pads.len()) {
                step(&mut engine, &session, frame, pads[frame])?;
                ensure!(
                    hashes(&engine)? == expected[frame],
                    "snapshot divergence at frame {} in {}",
                    frame + 1,
                    path.display()
                );
                replayed += 1;
            }
        }
        let image = saved.last().context("no checkpoints")?;
        engine.restore(image)?;
        let mut capture = Vec::new();
        let mut restore = Vec::new();
        let mut reusable = MachineSnapshot::default();
        engine.snapshot_into(&mut reusable)?; // warm the allocation and lazy ROM digest
        for _ in 0..100 {
            let start = Instant::now();
            engine.snapshot_into(&mut reusable)?;
            capture.push(start.elapsed().as_secs_f64() * 1e6);
            let start = Instant::now();
            engine.restore(&reusable)?;
            restore.push(start.elapsed().as_secs_f64() * 1e6);
        }
        reports.push(json!({ "session": path, "revision": revision.name(), "frames": pads.len(), "restored_frames_compared": replayed,
            "checkpoints": saved.iter().map(MachineSnapshot::frame_count).collect::<Vec<_>>(), "snapshot_bytes": image.byte_len(),
            "capture": distribution(capture), "restore": distribution(restore), "frame_hashes": expected }));
    }
    let text = serde_json::to_string_pretty(
        &json!({ "backend": "native-cranelift", "comparison": ["full canonical machine SHA1", "full backing framebuffer SHA1", "exact f64 audio bits/count SHA1"], "sessions": reports }),
    )?;
    std::fs::write(args.output, &text)?;
    println!("{text}");
    Ok(())
}
