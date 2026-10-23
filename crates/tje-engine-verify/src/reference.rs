//! A fresh vendored jgenesis instance, never a clone of the engine under test.
use crate::{
    CpuSnapshot, FrameState, InstructionMachine, InstructionProof, Machine, Position, Step,
    state::{ChipWrite, Origin, TimedChipWrite},
};
use anyhow::{Context, Result, ensure};
use bincode::{Decode, Encode};
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use genesis_core::{
    GenesisEmulator,
    native::{CpuAccess, DeviceWrite, Origin as NativeOrigin},
    timing_survey::TimingSurvey,
};
use jgenesis_common::frontend::{
    AudioOutput, Color, EmulatorTrait, FrameSize, InputPoller, RenderFrameOptions, Renderer,
    SaveWriter, TickEffect,
};
use std::{convert::Infallible, io};
use tje_engine_core::ReadKind;
use tje_engine_runtime::BusAccess;
use tje_rom::Rom;
use tje_session::{CONNECTED, RamWrite};

pub struct ReferenceMachine {
    emulator: GenesisEmulator,
    audio: Audio,
    instruction_accesses: Vec<BusAccess>,
    last_base_cycles: u32,
}

impl ReferenceMachine {
    pub fn new(rom: &Rom) -> Self {
        Self::from_synthetic_rom(rom.bytes().to_vec())
    }

    /// Test-oracle entry point for authored, nonretail instruction streams.
    /// Release replay always identifies an entire retail image via `Rom::import`.
    pub fn from_synthetic_rom(bytes: Vec<u8>) -> Self {
        let mut emulator =
            GenesisEmulator::create(Some(bytes), GenesisEmulatorConfig::default(), &mut Saves);
        let bus = emulator.hybrid_bus_mut();
        bus.timing_survey = Some(TimingSurvey::default());
        bus.native_writes = Some(Vec::new());
        Self {
            emulator,
            audio: Audio::default(),
            instruction_accesses: Vec::new(),
            last_base_cycles: 0,
        }
    }

    pub fn emulator(&self) -> &GenesisEmulator {
        &self.emulator
    }
}

pub fn inputs(pads: [u16; 2]) -> GenesisInputs {
    use GenesisButton::*;
    let [p1, p2] = pads.map(|pad| {
        let mut controller = if pad & CONNECTED != 0 {
            GenesisController::new(GenesisControllerType::ThreeButton)
        } else {
            GenesisController::None
        };
        for (bit, button) in [Up, Down, Left, Right, A, B, C, Start]
            .into_iter()
            .enumerate()
        {
            controller.set_field(button, pad & (1 << bit) != 0);
        }
        controller
    });
    GenesisInputs { p1, p2 }
}

impl Machine for ReferenceMachine {
    fn backend_name(&self) -> &str {
        "reference"
    }
    fn position(&self) -> Position {
        let bus = self.emulator.hybrid_bus();
        Position {
            pc: self.emulator.hybrid_cpu().pc(),
            master_clock: bus.cycles.m68k_mclk_cycles,
            scanline: bus.vdp.scanline(),
            line_master_clock: bus.vdp.scanline_mclk(),
        }
    }
    fn begin_frame(&mut self) {
        self.audio.bits.clear();
        self.emulator
            .hybrid_bus_mut()
            .native_writes
            .as_mut()
            .expect("enabled write capture")
            .clear();
    }
    fn write_ram(&mut self, write: &RamWrite) -> Result<()> {
        let memory = &mut self.emulator.hybrid_bus_mut().memory;
        let bytes = write.value.to_be_bytes();
        for (offset, &value) in bytes[4 - usize::from(write.width)..].iter().enumerate() {
            memory.write_main_ram_byte(write.address + offset as u32, value);
        }
        Ok(())
    }
    fn step(&mut self, pads: [u16; 2]) -> Result<Step> {
        if let Some(journal) = &mut self.emulator.hybrid_bus_mut().native_cpu_accesses {
            journal.clear();
        }
        if let Some(journal) = &mut self.emulator.hybrid_bus_mut().native_ram_writes {
            journal.clear();
        }
        let bus = self.emulator.hybrid_bus();
        let cpu = self.emulator.hybrid_cpu();
        let instruction_pc = (bus.cycles.m68k_wait_cpu_cycles == 0
            && cpu.hybrid_instruction_ready(bus))
        .then(|| cpu.pc());
        let before_clock = bus.cycles.m68k_mclk_cycles;
        let divider = bus.cycles.m68k_divider.get();
        let effect = self
            .emulator
            .tick(
                &mut Video,
                &mut self.audio,
                &mut Inputs(inputs(pads)),
                &mut Saves,
            )
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        let bus = self.emulator.hybrid_bus_mut();
        if let Some(journal) = &mut bus.native_cpu_accesses {
            let elapsed = bus.cycles.m68k_mclk_cycles - before_clock;
            ensure!(
                elapsed.is_multiple_of(divider),
                "reference step charge is not an integer CPU cycle"
            );
            self.last_base_cycles = u32::try_from(elapsed / divider)?;
            self.instruction_accesses.clear();
            for event in journal.drain(..) {
                self.instruction_accesses.push(match event {
                    CpuAccess::Read {
                        address,
                        word,
                        value,
                        purpose,
                    } => BusAccess::Read {
                        address,
                        word,
                        value,
                        kind: match purpose {
                            0 => ReadKind::Semantic,
                            1 => ReadKind::Prefetch,
                            2 => ReadKind::Discarded,
                            _ => anyhow::bail!("unknown reference read-purpose tag {purpose}"),
                        },
                    },
                    CpuAccess::Write {
                        address,
                        word,
                        value,
                    } => BusAccess::Write {
                        address,
                        word,
                        value,
                    },
                    CpuAccess::AcknowledgeInterrupt { level } => {
                        BusAccess::AcknowledgeInterrupt { level }
                    }
                });
            }
        }
        // The shared passive observer is also the source of chip-write PCs. Its
        // high-volume RAM/read events are not part of this frame contract.
        self.emulator
            .hybrid_bus_mut()
            .timing_survey
            .as_mut()
            .expect("enabled timing observer")
            .accesses
            .clear();
        Ok(Step {
            frame_complete: effect == TickEffect::FrameRendered,
            instruction_pc,
        })
    }
    fn capture(&self) -> Result<FrameState> {
        let bus = self.emulator.hybrid_bus();
        let cpu = self.emulator.hybrid_cpu();
        let mut address = [0; 8];
        address[..7].copy_from_slice(&cpu.address_registers());
        address[7] = cpu.stack_pointer();
        let (vram, cram, vsram) = bus.vdp.hybrid_memories();
        let size = bus.vdp.frame_size();
        let chip_writes = bus
            .native_writes
            .as_ref()
            .expect("enabled write capture")
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
                data: cpu.data_registers(),
                address,
                usp: cpu.user_stack_pointer(),
                ssp: cpu.supervisor_stack_pointer(),
                sr: cpu.status_register(),
                prefetch: cpu.next_opcode(),
                last_muldiv: cpu.last_instruction_was_mul_or_div(),
            },
            work_ram: (0..65_536)
                .map(|offset| bus.memory.read_main_ram_byte(offset))
                .collect(),
            vram: vram.into(),
            cram: cram.into(),
            vsram: vsram.into(),
            vdp_registers: bincode::encode_to_vec(
                bus.vdp.hybrid_registers(),
                bincode::config::standard(),
            )
            .context("encoding reference VDP registers")?,
            vdp_state: bincode::encode_to_vec(&bus.vdp, bincode::config::standard())
                .context("encoding complete reference VDP state")?,
            z80_ram: (0..8192)
                .map(|offset| bus.memory.read_audio_ram(offset))
                .collect(),
            width: size.width,
            height: size.height,
            pixels: bus
                .vdp
                .frame_buffer()
                .iter()
                .map(|color| [color.r, color.g, color.b, color.a])
                .collect(),
            audio_bits: self.audio.bits.clone(),
            chip_writes,
        })
    }
}

impl InstructionMachine for ReferenceMachine {
    fn enable_instruction_proof(&mut self) {
        self.emulator
            .hybrid_bus_mut()
            .native_cpu_accesses
            .get_or_insert_with(|| Vec::with_capacity(40))
            .clear();
        self.emulator
            .hybrid_bus_mut()
            .native_ram_writes
            .get_or_insert_with(|| Vec::with_capacity(40))
            .clear();
        self.instruction_accesses.clear();
        self.last_base_cycles = 0;
    }

    fn instruction_proof(&self) -> InstructionProof<'_> {
        InstructionProof {
            cpu: self.emulator.hybrid_cpu().hybrid_state_words(),
            base_cycles: self.last_base_cycles,
            accesses: &self.instruction_accesses,
            ram_writes: self
                .emulator
                .hybrid_bus()
                .native_ram_writes
                .as_deref()
                .expect("enabled actual RAM commit trace"),
        }
    }
}

#[derive(Default)]
struct Audio {
    bits: Vec<[u64; 2]>,
}
impl AudioOutput for Audio {
    type Err = Infallible;
    fn push_sample(&mut self, left: f64, right: f64) -> Result<(), Self::Err> {
        self.bits.push([left.to_bits(), right.to_bits()]);
        Ok(())
    }
}
struct Video;
impl Renderer for Video {
    type Err = Infallible;
    fn render_frame(
        &mut self,
        _: &[Color],
        _: FrameSize,
        _: f64,
        _: RenderFrameOptions,
    ) -> Result<(), Self::Err> {
        Ok(())
    }
}
struct Inputs(GenesisInputs);
impl InputPoller<GenesisInputs> for Inputs {
    fn poll(&mut self) -> &GenesisInputs {
        &self.0
    }
}
struct Saves;
impl SaveWriter for Saves {
    type Err = io::Error;
    fn load_bytes(&mut self, _: &str) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::NotFound.into())
    }
    fn persist_bytes(&mut self, _: &str, _: &[u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn load_serialized<D: Decode<()>>(&mut self, _: &str) -> io::Result<D> {
        Err(io::ErrorKind::NotFound.into())
    }
    fn persist_serialized<E: Encode>(&mut self, _: &str, _: E) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
