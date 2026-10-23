//! Shared proof adapter for IR, native-code and browser WebAssembly backends.
use crate::{
    CpuSnapshot, FrameState, InstructionMachine, InstructionProof, Machine, Position, Step,
    reference::inputs,
    state::{ChipWrite, Origin, TimedChipWrite},
};
use anyhow::{Context, Result};
use genesis_config::GenesisEmulatorConfig;
use std::fmt::Display;
use tje_engine_runtime::{Backend, Engine};
use tje_mem::{AddressSpace, Bus};
use tje_native_platform::{DeviceRuntime, DeviceWrite, Origin as NativeOrigin};
use tje_rom::{Revision, Rom};
use tje_session::RamWrite;

pub struct RuntimeMachine<B: Backend> {
    engine: Engine<B>,
    backend_name: String,
    last_base_cycles: u32,
}

impl<B: Backend> RuntimeMachine<B> {
    pub fn new(rom: &Rom, backend: B, backend_name: impl Into<String>) -> Self {
        Self::from_synthetic_rom(rom.bytes().to_vec(), rom.revision(), backend, backend_name)
    }

    pub fn from_synthetic_rom(
        bytes: Vec<u8>,
        revision: Revision,
        backend: B,
        backend_name: impl Into<String>,
    ) -> Self {
        let mut devices = DeviceRuntime::new(
            AddressSpace::new(bytes, revision),
            &GenesisEmulatorConfig::default(),
        );
        devices.enable_trace();
        Self {
            engine: Engine::from_devices(devices, backend),
            backend_name: backend_name.into(),
            last_base_cycles: 0,
        }
    }

    pub fn engine(&self) -> &Engine<B> {
        &self.engine
    }
    pub fn engine_mut(&mut self) -> &mut Engine<B> {
        &mut self.engine
    }
}

impl<B: Backend> Machine for RuntimeMachine<B>
where
    B::Error: Display,
{
    fn backend_name(&self) -> &str {
        &self.backend_name
    }
    fn position(&self) -> Position {
        let devices = self.engine.devices();
        Position {
            pc: self.engine.cpu().pc,
            master_clock: devices.clock(),
            scanline: devices.vdp().scanline(),
            line_master_clock: devices.vdp().scanline_mclk(),
        }
    }
    fn begin_frame(&mut self) {
        self.engine.devices_mut().clear_audio();
        self.engine.devices_mut().clear_trace();
    }
    fn write_ram(&mut self, write: &RamWrite) -> Result<()> {
        let memory = self.engine.devices_mut().memory_mut();
        let bytes = write.value.to_be_bytes();
        for (offset, &value) in bytes[4 - usize::from(write.width)..].iter().enumerate() {
            memory.write_u8(write.address + offset as u32, value);
        }
        Ok(())
    }
    fn step(&mut self, pads: [u16; 2]) -> Result<Step> {
        self.engine.devices_mut().clear_instruction_trace();
        self.engine.devices_mut().clear_ram_write_trace();
        let step = self
            .engine
            .step(inputs(pads))
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        self.last_base_cycles = step.devices.cpu_cycles;
        Ok(Step {
            frame_complete: step.devices.frame_complete,
            instruction_pc: step.instruction_pc,
        })
    }
    fn capture(&self) -> Result<FrameState> {
        let devices = self.engine.devices();
        let cpu = self.engine.cpu();
        let mut address = [0; 8];
        address[..7].copy_from_slice(&cpu.a);
        address[7] = cpu.read_address(7);
        let (vram, cram, vsram) = devices.vdp().hybrid_memories();
        let (pixels, size) = devices.frame_buffer();
        let chip_writes = devices
            .trace()
            .expect("enabled runtime trace")
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
                        NativeOrigin::Main => Origin::M68k,
                        NativeOrigin::Z80 => Origin::Z80,
                    },
                    write,
                })
            })
            .collect();
        Ok(FrameState {
            position: self.position(),
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
            )
            .context("encoding engine VDP registers")?,
            vdp_state: bincode::encode_to_vec(devices.vdp(), bincode::config::standard())
                .context("encoding complete engine VDP state")?,
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
}

impl<B: Backend> InstructionMachine for RuntimeMachine<B>
where
    B::Error: Display,
{
    fn enable_instruction_proof(&mut self) {
        self.engine.devices_mut().enable_instruction_trace();
        self.engine.devices_mut().clear_instruction_trace();
        self.engine.devices_mut().enable_ram_write_trace();
        self.engine.devices_mut().clear_ram_write_trace();
        self.last_base_cycles = 0;
    }

    fn instruction_proof(&self) -> InstructionProof<'_> {
        InstructionProof {
            cpu: crate::instruction_proof::cpu_state_words(self.engine.cpu()),
            base_cycles: self.last_base_cycles,
            accesses: self
                .engine
                .devices()
                .instruction_trace()
                .expect("enabled instruction trace"),
            ram_writes: self
                .engine
                .devices()
                .ram_write_trace()
                .expect("enabled actual RAM commit trace"),
        }
    }
}
