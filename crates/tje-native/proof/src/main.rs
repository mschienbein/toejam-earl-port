//! Development-only proof of the exact backend/replay path shipped by the UI.
//! Never included in the game package or its dependency graph.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::json;
use std::{path::PathBuf, time::Instant};
use tje_engine_cranelift::{CacheConfig, ENGINE_VERSION};
use tje_engine_verify::{
    CpuSnapshot, FrameState, Machine, Position, compare_frames,
    reference::ReferenceMachine,
    state::{ChipWrite, Origin, TimedChipWrite},
};
use tje_native::{
    backend::{GameBackend, NativeBackend},
    replay::Replay,
};
use tje_native_platform::{DeviceWrite, Origin as DeviceOrigin};
use tje_rom::Rom;

#[derive(Parser)]
#[command(
    about = "Compare every native-frontend replay frame byte-for-byte with the development reference"
)]
struct Args {
    #[arg(long)]
    rom: PathBuf,
    #[arg(long)]
    session: PathBuf,
    #[arg(long)]
    cache_dir: PathBuf,
    /// Metadata only: never contains ROM, RAM, pixels or audio sample payloads.
    #[arg(long)]
    report: PathBuf,
}

fn capture(backend: &NativeBackend) -> Result<FrameState> {
    let engine = backend.engine();
    let devices = engine.devices();
    let cpu = engine.cpu();
    let mut address = [0; 8];
    address[..7].copy_from_slice(&cpu.a);
    address[7] = cpu.read_address(7);
    let (vram, cram, vsram) = devices.vdp().hybrid_memories();
    let (pixels, size) = engine.frame_buffer();
    let chip_writes = devices
        .trace()
        .context("proof trace is disabled")?
        .writes
        .iter()
        .filter_map(|event| {
            let write = match event.write {
                DeviceWrite::Ym2612 { port, value } => ChipWrite::Ym2612 { port, value },
                DeviceWrite::Psg { value } => ChipWrite::Psg { value },
                DeviceWrite::Vdp { .. } => return None,
            };
            Some(TimedChipWrite {
                master_clock: event.clock,
                pc: event.pc,
                origin: match event.origin {
                    DeviceOrigin::Main => Origin::M68k,
                    DeviceOrigin::Z80 => Origin::Z80,
                },
                write,
            })
        })
        .collect();
    Ok(FrameState {
        position: Position {
            pc: cpu.pc,
            master_clock: devices.clock(),
            scanline: devices.vdp().scanline(),
            line_master_clock: devices.vdp().scanline_mclk(),
        },
        cpu: CpuSnapshot {
            data: cpu.d,
            address,
            usp: cpu.usp,
            ssp: cpu.ssp,
            sr: cpu.sr as u16,
            prefetch: cpu.prefetch as u16,
            last_muldiv: cpu.last_muldiv != 0,
        },
        work_ram: devices.memory().ram().to_vec(),
        vram: vram.into(),
        cram: cram.into(),
        vsram: vsram.into(),
        vdp_registers: bincode::encode_to_vec(
            devices.vdp().hybrid_registers(),
            bincode::config::standard(),
        )?,
        vdp_state: bincode::encode_to_vec(devices.vdp(), bincode::config::standard())?,
        z80_ram: devices.audio_ram().to_vec(),
        width: size.width,
        height: size.height,
        pixels: pixels
            .iter()
            .map(|color| [color.r, color.g, color.b, color.a])
            .collect(),
        audio_bits: devices
            .audio_samples()
            .iter()
            .map(|sample| sample.map(f64::to_bits))
            .collect(),
        chip_writes,
    })
}

fn main() -> Result<()> {
    let args = Args::parse();
    let rom = Rom::import(&std::fs::read(&args.rom)?)?;
    let mut replay = Replay::load(&args.session)?;
    ensure!(
        replay.revision() == rom.revision(),
        "session and cartridge revisions differ"
    );
    let mut reference = ReferenceMachine::new(&rom);
    let load_start = Instant::now();
    let mut backend = NativeBackend::load(
        rom,
        &CacheConfig {
            data_dir: args.cache_dir,
        },
        |_| {},
    )?;
    let load_seconds = load_start.elapsed().as_secs_f64();
    backend.set_audio_sample_rate(48_000)?;
    backend.set_trace_enabled(true);
    let initial_difference = compare_frames(&reference.capture()?, &capture(&backend)?);
    let mut difference = initial_difference.clone();
    let mut compared = 0_u64;
    let mut write_index = 0;
    let mut audio_samples = 0_u64;
    let mut chip_writes = 0_u64;
    let start = Instant::now();
    while difference.is_none() && !replay.finished() {
        let frame = replay.frame();
        let pads = replay.pads().context("session ended early")?;
        reference.begin_frame();
        while let Some(write) = replay.session().writes().get(write_index) {
            if write.before_frame != frame {
                break;
            }
            reference.write_ram(write)?;
            write_index += 1;
        }
        loop {
            if reference.step(pads)?.frame_complete {
                break;
            }
        }
        replay
            .advance(&mut backend)?
            .context("frontend replay ended early")?;
        let expected = reference.capture()?;
        let actual = capture(&backend)?;
        difference = compare_frames(&expected, &actual);
        if difference.is_none() {
            compared += 1;
            audio_samples += actual.audio_bits.len() as u64;
            chip_writes += actual.chip_writes.len() as u64;
        }
    }
    let stats = backend.compile_stats();
    let passed = difference.is_none() && compared == replay.frames();
    let report = json!({
        "format": 1, "passed": passed, "frontend": "tje-native", "engine_version": ENGINE_VERSION,
        "os": std::env::consts::OS, "architecture": std::env::consts::ARCH,
        "revision": backend.revision().name(), "session": args.session,
        "requested_frames": replay.frames(), "frames_compared": compared,
        "initial_state_equal": initial_difference.is_none(), "difference": difference,
        "comparison": "byte equality, no projection or hashes", "sample_rate": 48000,
        "domains": ["CPU registers", "master-clock/scanline position", "full work RAM", "VRAM", "CRAM", "VSRAM", "VDP registers", "complete VDP serialized state", "Z80 RAM", "complete frame pixels", "stereo f64 sample bits/counts", "ordered timestamped chip writes"],
        "audio_samples_compared": audio_samples, "chip_writes_compared": chip_writes,
        "load_seconds": load_seconds, "cache_status": format!("{:?}", stats.cache_status),
        "native_instruction_count": stats.instruction_count, "native_code_bytes": stats.code_bytes,
        "comparison_seconds": start.elapsed().as_secs_f64(),
    });
    std::fs::write(&args.report, serde_json::to_string_pretty(&report)?)?;
    println!("{}", serde_json::to_string(&report)?);
    ensure!(
        passed,
        "native frontend differs from reference; see {}",
        args.report.display()
    );
    Ok(())
}
