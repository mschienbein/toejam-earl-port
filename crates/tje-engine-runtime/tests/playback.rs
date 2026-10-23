#![cfg(feature = "devices")]
//! Authored BRA loop, not a cartridge or a generated instruction fixture.

use bincode::{Decode, Encode};
use genesis_config::{GenesisEmulatorConfig, GenesisInputs, GenesisRegion};
use genesis_core::GenesisEmulator;
use jgenesis_common::frontend::{
    AudioOutput, Color, EmulatorTrait, FrameSize, InputPoller, RenderFrameOptions, Renderer,
    SaveWriter, TickEffect,
};
use std::{convert::Infallible, io};
use tje_engine_runtime::{
    Backend, Bus, BusAccess, CpuState, Devices, Engine, PlatformDevices, ReadKind, StepResult,
};
use tje_mem::AddressSpace;
use tje_native_platform::DeviceRuntime;
use tje_rom::Revision;

struct BranchLoop;

impl Backend for BranchLoop {
    type Error = Infallible;

    fn execute(&mut self, cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<StepResult, Infallible> {
        assert_eq!(cpu.pc, 0x200);
        assert_eq!(cpu.prefetch, 0x60FE);
        // The authored BRA.S -2 body: consume the sequential prefetch, then
        // refill at the taken destination. No decoder is used by this backend.
        let _ = bus.read16(0x202, ReadKind::Prefetch);
        cpu.prefetch = u32::from(bus.read16(0x200, ReadKind::Prefetch));
        Ok(StepResult::normal(10))
    }
}

#[derive(Default)]
struct Output(Vec<[f64; 2]>);

impl AudioOutput for Output {
    type Err = Infallible;
    fn push_sample(&mut self, left: f64, right: f64) -> Result<(), Infallible> {
        self.0.push([left, right]);
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
    ) -> Result<(), Infallible> {
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

fn compare_frame(engine: &mut Engine<BranchLoop>, reference: &mut GenesisEmulator) {
    let samples = engine.play_frame(GenesisInputs::default()).unwrap();
    let mut output = Output::default();
    loop {
        if reference
            .tick(
                &mut Video,
                &mut output,
                &mut Inputs(GenesisInputs::default()),
                &mut Saves,
            )
            .unwrap()
            == TickEffect::FrameRendered
        {
            break;
        }
    }
    assert_eq!(samples.len(), output.0.len());
    for (native, original) in samples.iter().zip(&output.0) {
        assert_eq!(native.map(f64::to_bits), original.map(f64::to_bits));
    }
    let bus = reference.hybrid_bus();
    assert_eq!(engine.devices().clock(), bus.cycles.m68k_mclk_cycles);
    assert_eq!(engine.cpu().pc, reference.hybrid_cpu().pc());
    assert_eq!(
        engine.cpu().prefetch,
        u32::from(reference.hybrid_cpu().next_opcode())
    );
    assert!(engine.devices().vdp().hybrid_state_eq(&bus.vdp));
    assert_eq!(engine.frame_buffer().0, bus.vdp.frame_buffer());
    assert_eq!(engine.target_fps(), reference.target_fps());
    let (ram, _) = reference.hybrid_bus_mut().memory.debug_ram_view();
    for (native, original) in engine
        .devices()
        .memory()
        .ram()
        .as_chunks::<2>()
        .0
        .iter()
        .zip(ram)
    {
        assert_eq!(native, &original.to_be_bytes());
    }
}

#[test]
fn borrowed_playback_matches_reference_and_retains_sample_rate_after_reset() {
    let mut rom = vec![0; 0x10000];
    rom[..4].copy_from_slice(&0x00FFFF00_u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x200_u32.to_be_bytes());
    rom[0x200..0x202].copy_from_slice(&0x60FE_u16.to_be_bytes());
    rom[0x202..0x204].copy_from_slice(&0x4E71_u16.to_be_bytes());
    let config = GenesisEmulatorConfig {
        forced_region: Some(GenesisRegion::Americas),
        ..GenesisEmulatorConfig::default()
    };
    let mut reference = GenesisEmulator::create(Some(rom.clone()), config.clone(), &mut Saves);
    let devices = DeviceRuntime::new(AddressSpace::new(rom, Revision::Rev02), &config);
    let mut engine = Engine::from_devices(devices, BranchLoop);
    assert_eq!(engine.devices().clock(), 0);
    assert_eq!(engine.frame_count(), 0);
    assert!(engine.set_audio_sample_rate(7999).is_err());
    assert!(engine.set_audio_sample_rate(192001).is_err());
    compare_frame(&mut engine, &mut reference);
    engine.set_audio_sample_rate(44_100).unwrap();
    reference.update_audio_output_frequency(44_100);
    for _ in 0..3 {
        compare_frame(&mut engine, &mut reference);
    }
    assert_eq!(engine.frame_count(), 4);
    engine.reset();
    reference.hard_reset(&mut Saves);
    reference.update_audio_output_frequency(44_100);
    assert_eq!(engine.frame_count(), 0);
    assert_eq!(engine.devices().clock(), 0);
    compare_frame(&mut engine, &mut reference);
}

#[test]
fn optional_instruction_journal_preserves_issue_order_and_deferred_ram_commit() {
    let memory = AddressSpace::new(vec![0; 0x400], Revision::Rev02);
    let mut devices = PlatformDevices::from(DeviceRuntime::new(
        memory,
        &GenesisEmulatorConfig::default(),
    ));
    assert!(devices.instruction_trace().is_none());
    devices.enable_instruction_trace();
    Devices::begin_instruction(&mut devices, 0x200, 0x4E71);
    assert_eq!(Bus::read16(&mut devices, 0x200, ReadKind::Prefetch), 0);
    Bus::write16(&mut devices, 0xFFFF8000, 0x1234);
    assert_eq!(&devices.memory().ram()[0x8000..0x8002], &[0, 0]);
    Devices::finish_instruction(&mut devices, 0x200, 4);
    assert_eq!(&devices.memory().ram()[0x8000..0x8002], &[0x12, 0x34]);
    assert_eq!(
        devices.instruction_trace().unwrap(),
        &[
            BusAccess::Read {
                address: 0x200,
                word: true,
                value: 0,
                kind: ReadKind::Prefetch
            },
            BusAccess::Write {
                address: 0xFF8000,
                word: true,
                value: 0x1234
            },
        ]
    );
    devices.clear_instruction_trace();
    assert!(devices.instruction_trace().unwrap().is_empty());
    devices.disable_instruction_trace();
    assert!(devices.instruction_trace().is_none());
}
