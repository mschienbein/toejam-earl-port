//! Observation-only wire encoding of every canonical engine-verifier FrameState
//! domain. No hashes, projection, reference CPU, or alternative execution path.
//! This module is absent from the default shipping build. Runtime bytes are
//! private ROM derivatives and must never become committed test fixtures.

use bincode::Encode;

#[derive(Encode)]
pub struct Cpu {
    pub data: [u32; 8],
    pub address: [u32; 8],
    pub usp: u32,
    pub ssp: u32,
    pub sr: u16,
    pub prefetch: u16,
    pub last_muldiv: bool,
}

#[derive(Encode)]
pub struct ChipWrite {
    pub master_clock: u64,
    /// Zero for the 68000, one for the original Z80 sound driver.
    pub origin: u8,
    pub pc: u32,
    /// Zero for YM2612, one for PSG (whose port is zero).
    pub chip: u8,
    pub port: u8,
    pub value: u8,
}

/// Field order mirrors tje-engine-verify::state::FrameState. The extra completed
/// frame number prevents a shifted stream from accidentally comparing frames.
#[derive(Encode)]
pub struct Frame {
    pub frame: u64,
    pub position: (u32, u64, u16, u64),
    pub cpu: Cpu,
    pub work_ram: Vec<u8>,
    pub vram: Vec<u8>,
    pub cram: Vec<u16>,
    pub vsram: Vec<u16>,
    pub vdp_registers: Vec<u8>,
    pub vdp_state: Vec<u8>,
    pub z80_ram: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u8; 4]>,
    pub audio_bits: Vec<[u64; 2]>,
    pub chip_writes: Vec<ChipWrite>,
}

pub fn encode(frame: &Frame) -> Result<Vec<u8>, bincode::error::EncodeError> {
    bincode::encode_to_vec(frame, bincode::config::standard())
}

#[cfg(all(target_arch = "wasm32", feature = "replay-observer"))]
pub fn capture(
    engine: &tje_engine_runtime::Engine<tje_engine_wasmgen::BrowserBackend>,
) -> Result<Vec<u8>, bincode::error::EncodeError> {
    use tje_native_platform::{DeviceWrite, Origin};
    let devices = engine.devices();
    let cpu = engine.cpu();
    let mut address = [0; 8];
    address[..7].copy_from_slice(&cpu.a);
    address[7] = cpu.read_address(7);
    let (vram, cram, vsram) = devices.vdp().hybrid_memories();
    let (pixels, size) = devices.frame_buffer();
    let chip_writes = devices
        .trace()
        .expect("replay-observer enables the device-write journal")
        .writes
        .iter()
        .filter_map(|event| {
            let (chip, port, value) = match event.write {
                DeviceWrite::Ym2612 { port, value } => (0, port, value),
                DeviceWrite::Psg { value } => (1, 0, value),
                DeviceWrite::Vdp { .. } => return None,
            };
            Some(ChipWrite {
                master_clock: event.clock,
                origin: match event.origin {
                    Origin::Main => 0,
                    Origin::Z80 => 1,
                },
                pc: event.pc,
                chip,
                port,
                value,
            })
        })
        .collect();
    encode(&Frame {
        frame: engine.frame_count(),
        position: (
            cpu.pc,
            devices.clock(),
            devices.vdp().scanline(),
            devices.vdp().scanline_mclk(),
        ),
        cpu: Cpu {
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
