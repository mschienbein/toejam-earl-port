#![cfg(feature = "devices")]
//! Synthetic Z80 bank reads at a phase where the reference look-ahead changes
//! the observed byte. No cartridge instructions, assets, or states are used.

use bincode::{Decode, Encode};
use genesis_config::{GenesisEmulatorConfig, GenesisInputs, GenesisRegion};
use genesis_core::GenesisEmulator;
use jgenesis_common::frontend::{
    AudioOutput, Color, EmulatorTrait, FrameSize, InputPoller, RenderFrameOptions, Renderer,
    SaveWriter,
};
use m68000_emu::BusInterface;
use std::{convert::Infallible, io};
use tje_engine_runtime::{Backend, Bus, CpuState, Engine, StepKind, StepResult};
use tje_mem::AddressSpace;
use tje_native_platform::DeviceRuntime;
use tje_rom::Revision;

struct Sink;
impl Renderer for Sink {
    type Err = Infallible;
    fn render_frame(
        &mut self,
        _: &[Color],
        _: FrameSize,
        _: f64,
        _: RenderFrameOptions,
    ) -> Result<(), Infallible> {
        Ok(())
    }
}
impl AudioOutput for Sink {
    type Err = Infallible;
    fn push_sample(&mut self, _: f64, _: f64) -> Result<(), Infallible> {
        Ok(())
    }
}
impl SaveWriter for Sink {
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
#[derive(Default)]
struct Inputs(GenesisInputs);
impl InputPoller<GenesisInputs> for Inputs {
    fn poll(&mut self) -> &GenesisInputs {
        &self.0
    }
}
fn tick(reference: &mut GenesisEmulator) {
    reference
        .tick(&mut Sink, &mut Sink, &mut Inputs::default(), &mut Sink)
        .unwrap();
}

struct NoInstructions;
impl Backend for NoInstructions {
    type Error = Infallible;
    fn execute(&mut self, _: &mut CpuState, _: &mut dyn Bus) -> Result<StepResult, Infallible> {
        panic!("the pending hardware wait must run before any continuation")
    }
}

fn sample(reference: &GenesisEmulator, port: u8, adjustment: u64) -> u8 {
    let vdp = &reference.hybrid_bus().vdp;
    match port {
        5 => vdp.clone().native_read_status_at(adjustment) as u8,
        9 => vdp.native_hv_counter_at(adjustment) as u8,
        _ => unreachable!("only the authored status/HV fixtures are sampled"),
    }
}

#[test]
fn wait_steps_preserve_prefetched_opcode_phase_for_z80_banked_status_and_hv_reads() {
    for divider in [3, 7] {
        for opcode in [0x4E71_u16, 0x3010, 0x33FC, 0xB0C0] {
            for port in [5, 9] {
                let mut rom = vec![0; 0x10000];
                rom[..4].copy_from_slice(&0x00FFFE00_u32.to_be_bytes());
                rom[4..8].copy_from_slice(&0x200_u32.to_be_bytes());
                rom[0x200..0x202].copy_from_slice(&0x60FE_u16.to_be_bytes());
                rom[0x202..0x204].copy_from_slice(&0x4E71_u16.to_be_bytes());
                rom[0x300..0x302].copy_from_slice(&opcode.to_be_bytes());
                let config = GenesisEmulatorConfig {
                    forced_region: Some(GenesisRegion::Americas),
                    m68k_clock_divider: divider,
                    ..GenesisEmulatorConfig::default()
                };
                let mut reference = GenesisEmulator::create(Some(rom.clone()), config, &mut Sink);
                {
                    let bus = reference.hybrid_bus_mut();
                    bus.write_word(0xA11100, 0x100);
                    bus.write_word(0xA11200, 0x100);
                    bus.apply_pending_writes();
                    // LD A,($8000+port); LD ($0010),A; HALT. BUSREQ keeps
                    // this authored program parked until the compared wait.
                    for (offset, value) in [0x3A, port, 0x80, 0x32, 0x10, 0x00, 0x76]
                        .into_iter()
                        .enumerate()
                    {
                        bus.write_byte(0xA00000 + offset as u32, value);
                    }
                    let bank = 0xC00000_u32 >> 15;
                    for bit in 0..9 {
                        bus.write_byte(0xA06000, ((bank >> bit) & 1) as u8);
                    }
                    bus.apply_pending_writes();
                }
                let cycles =
                    m68000_emu::cycles_if_move_btst_cmp(opcode).map_or(8, |cycles| cycles - 4);
                let adjustment = u64::from(cycles) * divider;
                let mut values = None;
                for _ in 0..3000 {
                    let raw = sample(&reference, port, 0);
                    let adjusted = sample(&reference, port, adjustment);
                    if raw != adjusted {
                        values = Some((raw, adjusted));
                        break;
                    }
                    tick(&mut reference);
                }
                let (raw, expected) =
                    values.expect("fixture must distinguish zero from reference phase");
                {
                    let (cpu, bus) = reference.hybrid_parts();
                    cpu.set_pc(0x300, bus);
                    bus.write_word(0xA11100, 0);
                    bus.apply_pending_writes();
                    bus.cycles.m68k_wait_cpu_cycles = 175;
                }
                let original_cpu = reference.hybrid_cpu();
                let words = original_cpu.hybrid_state_words();
                let cpu = CpuState {
                    d: original_cpu.data_registers(),
                    a: original_cpu.address_registers(),
                    usp: original_cpu.user_stack_pointer(),
                    ssp: original_cpu.supervisor_stack_pointer(),
                    pc: original_cpu.pc(),
                    sr: u32::from(original_cpu.status_register()),
                    prefetch: u32::from(original_cpu.next_opcode()),
                    pending_interrupt: words[20],
                    stopped: words[21],
                    frozen: words[22],
                    address_error: words[23],
                    last_muldiv: words[24],
                };
                let state = reference.capture_native_state();
                let devices =
                    DeviceRuntime::from_state(AddressSpace::new(rom, Revision::Rev02), state)
                        .unwrap();
                let mut engine = Engine::from_state(cpu, devices.into(), NoInstructions, 0);
                let step = engine.step(GenesisInputs::default()).unwrap();
                assert_eq!(step.kind, StepKind::Wait);
                assert_eq!(step.instruction_pc, None);
                tick(&mut reference);
                let expected_state = reference.capture_native_state();
                assert_eq!(
                    expected_state.audio_ram[0x10], expected,
                    "reference bank read, opcode={opcode:04X} divider={divider} port={port}"
                );
                assert_ne!(expected, raw);
                assert_eq!(
                    engine.devices().audio_ram().as_slice(),
                    expected_state.audio_ram
                );
                assert_eq!(
                    engine.devices().clock(),
                    expected_state.cycles.m68k_mclk_cycles
                );
                assert!(engine.devices().vdp().hybrid_state_eq(&expected_state.vdp));
            }
        }
    }
}
