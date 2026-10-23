//! ROM-free throughput gate for native CI, including macOS ARM64.
//! Uses real compiled continuations/devices and authored PSG/YM register inputs;
//! it measures this synthetic workload, not untested retail-game performance.
use anyhow::{Result, ensure};
use clap::Parser;
use genesis_config::{GenesisEmulatorConfig, GenesisInputs};
use serde_json::json;
use std::time::Instant;
use tje_engine_core::{Program, compile_instruction};
use tje_engine_cranelift::NativeProgram;
use tje_engine_runtime::Engine;
use tje_mem::AddressSpace;
use tje_native_platform::{ClockedDevices, DeviceRuntime};
use tje_rom::Revision;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value_t = 600)]
    frames: u64,
}

fn write(devices: &mut DeviceRuntime, address: u32, value: u16, word: bool) {
    loop {
        devices.begin_instruction(0x4E71, 56);
        if devices.take_wait_step(0x200).is_none() {
            break;
        }
    }
    if word {
        devices.write_bus_word(address, value);
    } else {
        devices.write_bus_byte(address, value as u8);
    }
    devices.finish_instruction(0x200, 4);
}

fn initialize_audio(devices: &mut DeviceRuntime) {
    write(devices, 0xA11100, 0x100, true);
    write(devices, 0xA11200, 0x100, true);
    for value in [0x80, 0x10, 0x90] {
        write(devices, 0xC00011, value, false);
    }
    for (bank, register, value) in [(1, 0xB6, 0xC0), (0, 0x2B, 0x80), (0, 0x2A, 0x60)] {
        write(devices, 0xA04000 + 2 * bank, register, false);
        write(devices, 0xA04001 + 2 * bank, value, false);
        devices.advance_master_clocks(2048);
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(args.frames != 0, "--frames must be nonzero");
    let mut rom = vec![0; 1024];
    rom[..4].copy_from_slice(&0x00FFFE00_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x200_u32.to_be_bytes());
    // Authored BRA.S -2; NOP. No retail bytes or interpreter are involved.
    rom[0x200..0x202].copy_from_slice(&0x60FE_u16.to_be_bytes());
    rom[0x202..0x204].copy_from_slice(&0x4E71_u16.to_be_bytes());
    let program = Program {
        instructions: vec![compile_instruction(&rom, 0x200)?],
    };
    let backend = NativeProgram::compile([0; 20], &program)?;
    let devices = DeviceRuntime::new(
        AddressSpace::new(rom, Revision::Rev02),
        &GenesisEmulatorConfig::default(),
    );
    let mut engine = Engine::from_devices(devices, backend);
    let mut cases = Vec::with_capacity(2);
    for rate in [44_100, 48_000] {
        engine.reset();
        engine.set_audio_sample_rate(rate)?;
        initialize_audio(engine.devices_mut());
        for _ in 0..60 {
            engine.play_frame(GenesisInputs::default())?;
        }
        let start = Instant::now();
        let mut samples = 0;
        for _ in 0..args.frames {
            samples += engine.play_frame(GenesisInputs::default())?.len();
        }
        let seconds = start.elapsed().as_secs_f64();
        let fps = args.frames as f64 / seconds;
        let realtime_ratio = fps / engine.target_fps();
        let case = json!({
            "sample_rate": rate, "frames": args.frames, "samples": samples,
            "seconds": seconds, "fps": fps, "realtime_ratio": realtime_ratio,
        });
        println!("{case}");
        ensure!(samples > 0, "synthetic engine produced no audio");
        ensure!(
            realtime_ratio >= 1.0,
            "synthetic canonical-audio engine is below real time: {realtime_ratio}"
        );
        cases.push(case);
    }
    println!(
        "{}",
        json!({"workload":"authored BRA/PSG/YM", "architecture":std::env::consts::ARCH, "cases":cases})
    );
    Ok(())
}
