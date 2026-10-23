//! Private-ROM tests. Inputs are read at runtime; captures never leave memory.

use genesis_components::{cartridge::Cartridge, vdp::VdpTickEffect};
use genesis_config::{GenesisEmulatorConfig, GenesisInputs};
#[cfg(feature = "game-adapter")]
use genesis_core::HybridWrite;
use genesis_core::{
    GenesisBus, GenesisEmulator,
    audio::GenesisAudioResampler,
    native::PeripheralState,
    timing_survey::{TimingCpu, TimingSurvey},
};
use jgenesis_common::frontend::{
    AudioOutput, Color, EmulatorTrait, FrameSize, InputPoller, RenderFrameOptions, Renderer,
    SaveWriter, TickEffect,
};
use m68000_emu::BusInterface as _;
use std::{convert::Infallible, io, path::PathBuf};
#[cfg(feature = "game-adapter")]
use tje_game::{FunctionId, functions};
use tje_host::input::InputScript;
use tje_mem::{AddressSpace, Revision, ram, rom_symbol};
use tje_native_platform::{ClockedDevices, DeviceRuntime, Origin};
#[cfg(feature = "game-adapter")]
use tje_native_platform::{DeviceWrite, InterruptHandlers, NativePlatform};
use z80_emu::Z80;

#[derive(Default)]
struct Output {
    samples: Vec<[f64; 2]>,
}
impl Renderer for Output {
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
impl AudioOutput for Output {
    type Err = Infallible;
    fn push_sample(&mut self, left: f64, right: f64) -> Result<(), Self::Err> {
        self.samples.push([left, right]);
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
    fn load_serialized<D: bincode::Decode<()>>(&mut self, _: &str) -> io::Result<D> {
        Err(io::ErrorKind::NotFound.into())
    }
    fn persist_serialized<E: bincode::Encode>(&mut self, _: &str, _: E) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

fn rom(revision: Revision) -> Vec<u8> {
    let (env, name) = match revision {
        Revision::Rev02 => ("TJE_REV02_ROM", "rev02.bin"),
        Revision::Rev00 => ("TJE_REV00_ROM", "rev00.bin"),
    };
    let path = std::env::var_os(env).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../roms")
            .join(name)
    });
    std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "{}: {error}; private ROM test was explicitly requested",
            path.display()
        )
    })
}
#[cfg(feature = "game-adapter")]
fn id(name: &str) -> FunctionId {
    functions().find(|entry| entry.name == name).unwrap().id
}
fn encoded<T: bincode::Encode>(value: &T) -> Vec<u8> {
    bincode::encode_to_vec(value, bincode::config::standard()).unwrap()
}
fn frame(emulator: &mut GenesisEmulator, inputs: GenesisInputs) {
    let mut renderer = Output::default();
    let mut audio = Output::default();
    loop {
        if emulator
            .tick(&mut renderer, &mut audio, &mut Inputs(inputs), &mut Saves)
            .unwrap()
            == TickEffect::FrameRendered
        {
            break;
        }
    }
}
fn gameplay(bytes: &[u8]) -> GenesisEmulator {
    let config = GenesisEmulatorConfig::default();
    let mut emulator = GenesisEmulator::create(Some(bytes.to_vec()), config, &mut Saves);
    let script = InputScript::parse(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tje-host/examples/start-game.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for index in 0..1300 {
        frame(&mut emulator, script.at(index));
    }
    emulator
}
fn prepare(emulator: &mut GenesisEmulator, entry: u32, arguments: &[u32]) -> (u32, u32, u32) {
    let saved_pc = emulator.hybrid_cpu().pc();
    let saved_sp = emulator.hybrid_cpu().stack_pointer();
    let mut sp = saved_sp;
    let (cpu, bus) = emulator.hybrid_parts();
    for &argument in arguments.iter().rev() {
        sp = sp.wrapping_sub(4);
        bus.write_word(sp, (argument >> 16) as u16);
        bus.write_word(sp.wrapping_add(2), argument as u16);
    }
    let cleanup_sp = sp;
    sp = sp.wrapping_sub(4);
    bus.write_word(sp, 0x003f);
    bus.write_word(sp.wrapping_add(2), 0xfffe);
    bus.apply_pending_writes();
    let supervisor = cpu.status_register() & 0x2000 != 0;
    let mut banks = (cpu.user_stack_pointer(), cpu.supervisor_stack_pointer());
    if supervisor {
        banks.1 = sp;
    } else {
        banks.0 = sp;
    }
    cpu.set_address_registers(cpu.address_registers(), banks.0, banks.1);
    cpu.set_pc(entry, bus);
    (saved_pc, saved_sp, cleanup_sp)
}
// Labeled device-fixture setup, not a naturally generated sound request.
// G/src/asm/sound_pcm.s:21-61 posts the PCM fields only after the real
// BUSACK/mailbox-lock protocol; G/src/z80/sound_driver.s:126-132 otherwise
// legitimately polls command zero without producing any chip writes.
fn capture_with_requested_pcm(
    emulator: &mut GenesisEmulator,
    revision: Revision,
) -> PeripheralState {
    let sample = rom_symbol(revision, "Sfx_JamBassDrum")
        .expect("the revision's public ROM symbol table names the sample")
        .address;
    let pending = ram::g_pending_pcm_sample(revision).address();
    let bus = emulator.hybrid_bus_mut();
    bus.write_word(pending, (sample >> 16) as u16);
    bus.write_word(pending.wrapping_add(2), sample as u16);
    bus.apply_pending_writes();
    let entry = tje_host::catalogue::FunctionCatalogue::new(revision)
        .named("AudioSubmitPendingPcmSample")
        .expect("the public function catalogue names the original PCM sender")
        .address(revision);
    let (saved_pc, saved_sp, cleanup_sp) = prepare(emulator, entry, &[]);
    let mut renderer = Output::default();
    let mut audio = Output::default();
    let mut complete = false;
    for _ in 0..1_000_000 {
        if emulator.hybrid_cpu().pc() == 0x003f_fffe
            && emulator.hybrid_cpu().stack_pointer() == cleanup_sp
        {
            complete = true;
            break;
        }
        emulator
            .tick(
                &mut renderer,
                &mut audio,
                &mut Inputs(GenesisInputs::default()),
                &mut Saves,
            )
            .unwrap();
        audio.samples.clear();
    }
    assert!(complete, "{revision:?}: original PCM sender did not return");
    assert_eq!(
        emulator.hybrid_bus().memory.read_main_ram_word(pending),
        0,
        "{revision:?}: original PCM sender must accept the pending sample"
    );
    assert_eq!(
        emulator
            .hybrid_bus()
            .memory
            .read_main_ram_word(pending.wrapping_add(2)),
        0,
        "{revision:?}: original PCM sender must clear the complete pending pointer"
    );
    let (cpu, bus) = emulator.hybrid_parts();
    let mut banks = (cpu.user_stack_pointer(), cpu.supervisor_stack_pointer());
    if cpu.status_register() & 0x2000 != 0 {
        banks.1 = saved_sp;
    } else {
        banks.0 = saved_sp;
    }
    cpu.set_address_registers(cpu.address_registers(), banks.0, banks.1);
    cpu.set_pc(saved_pc, bus);
    emulator.capture_native_state()
}

#[cfg(feature = "game-adapter")]
#[test]
#[ignore = "requires both private user ROMs and executes real gameplay captures"]
fn translated_subsystems_match_original_from_live_captures() {
    let cases: &[(&str, &[u32])] = &[
        ("GetRandomNumber", &[]),
        ("GetRandomNumber", &[]),
        ("SelectSpriteChainStart", &[0]),
        ("SelectSpriteChainStart", &[1]),
        ("GetCtrl1Input", &[]),
        ("GetCtrl2Input", &[]),
        ("TextMapCharacterToGlyph", &[65]),
        ("DisableAllInterrupts", &[]),
        ("EnableAllInterrupts", &[]),
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let bytes = rom(revision);
        let mut original = gameplay(&bytes);
        let mut calls = 0;
        let mut dead_stack_differences = 0;
        for &(name, arguments) in cases {
            let root = id(name);
            let (saved_pc, saved_sp, cleanup_sp) =
                prepare(&mut original, root.metadata().address(revision), arguments);
            let entry_sp = original.hybrid_cpu().stack_pointer();
            let state = original.capture_native_state();
            let mut native = NativePlatform::from_state(
                AddressSpace::new(bytes.clone(), revision),
                state,
                &[root],
                InterruptHandlers::default(),
            )
            .unwrap();
            native.devices_mut().enable_trace();
            original.hybrid_bus_mut().hybrid_writes = Some(Vec::new());
            original.hybrid_bus_mut().hybrid_accesses = Some(Vec::new());
            let actual = native.invoke(root, arguments).unwrap();
            let mut completed = false;
            for _ in 0..1_000_000 {
                if original.hybrid_cpu().pc() == 0x003f_fffe
                    && original.hybrid_cpu().stack_pointer() == cleanup_sp
                {
                    completed = true;
                    break;
                }
                let (cpu, bus) = original.hybrid_parts();
                cpu.execute_instruction(bus);
                bus.apply_pending_writes();
            }
            assert!(completed, "{revision:?} {name}: original did not return");
            if root.metadata().abi.returns_value {
                assert_eq!(
                    actual,
                    original.hybrid_cpu().data_registers()[0],
                    "{revision:?} {name}: D0"
                );
            }
            let result = original.capture_native_state();
            let native_ram = native.devices().memory().ram();
            for (offset, (&reference, &translated)) in result.ram.iter().zip(native_ram).enumerate()
            {
                if reference != translated {
                    if offset < (entry_sp & 0xffff) as usize {
                        dead_stack_differences += 1;
                    } else {
                        panic!(
                            "{revision:?} {name}: live RAM differs at {:06x}",
                            0xff0000 + offset
                        );
                    }
                }
            }
            assert_eq!(
                result.audio_ram.as_slice(),
                native.devices().audio_ram(),
                "{revision:?} {name}: Z80 RAM"
            );
            assert!(
                result.vdp.hybrid_state_eq(native.devices().vdp()),
                "{revision:?} {name}: full VDP state"
            );
            assert_eq!(
                encoded(&result.ym2612),
                encoded(native.devices().ym2612()),
                "{revision:?} {name}: YM state"
            );
            assert_eq!(
                encoded(&result.psg),
                encoded(native.devices().psg()),
                "{revision:?} {name}: PSG state"
            );
            let expected: Vec<_> = original
                .hybrid_bus()
                .hybrid_writes
                .as_ref()
                .unwrap()
                .iter()
                .map(|write| match *write {
                    HybridWrite::Vdp { port, value } => DeviceWrite::Vdp { port, value },
                    HybridWrite::Ym2612 { port, value } => DeviceWrite::Ym2612 { port, value },
                    HybridWrite::Psg { value } => DeviceWrite::Psg { value },
                })
                .collect();
            let actual: Vec<_> = native
                .devices()
                .trace()
                .unwrap()
                .writes
                .iter()
                .map(|write| write.write)
                .collect();
            assert_eq!(
                expected, actual,
                "{revision:?} {name}: ordered chip dispatches"
            );
            let expected_accesses = original.hybrid_bus().hybrid_accesses.as_ref().unwrap();
            let actual_accesses = &native.devices().trace().unwrap().accesses;
            assert_eq!(
                expected_accesses.len(),
                actual_accesses.len(),
                "{revision:?} {name}: access count"
            );
            for (reference, actual) in expected_accesses.iter().zip(actual_accesses) {
                let (address, word, write, value) = match *reference {
                    genesis_core::HybridAccess::Read {
                        address,
                        word,
                        value,
                        ..
                    } => (address, word, false, value),
                    genesis_core::HybridAccess::Write {
                        address,
                        word,
                        value,
                    } => (address, word, true, value),
                };
                let mask = if !write && (0xc00004..=0xc00007).contains(&address) {
                    if word {
                        0x03ff
                    } else if address & 1 == 0 {
                        3
                    } else {
                        0xff
                    }
                } else if !write && address == 0xa11100 {
                    if word { 0x0100 } else { 1 }
                } else {
                    0xffff
                };
                assert_eq!(
                    (address, word, write, value & mask),
                    (
                        actual.address,
                        actual.word,
                        actual.write,
                        actual.value & mask
                    ),
                    "{revision:?} {name}: ordered device access"
                );
            }
            calls += 1;
            let (cpu, bus) = original.hybrid_parts();
            let mut banks = (cpu.user_stack_pointer(), cpu.supervisor_stack_pointer());
            if cpu.status_register() & 0x2000 != 0 {
                banks.1 = saved_sp;
            } else {
                banks.0 = saved_sp;
            }
            cpu.set_address_registers(cpu.address_registers(), banks.0, banks.1);
            cpu.set_pc(saved_pc, bus);
            frame(&mut original, GenesisInputs::default());
        }
        eprintln!(
            "{revision:?}: {calls} native/original live-state fixtures; zero live-RAM/device/ordered-chip differences; {dead_stack_differences} explicitly counted dead-stack byte differences (not cycle-exact evidence)"
        );
    }
}

struct ReferenceDevices {
    bus: GenesisBus,
    z80: Z80,
    resampler: GenesisAudioResampler,
    output: Output,
}
impl ReferenceDevices {
    fn new(bytes: &[u8], state: PeripheralState) -> Self {
        let cartridge = Cartridge::new(
            bytes.to_vec(),
            None,
            state.config.forced_region,
            &state.config.cheat_codes,
        );
        let mut bus = GenesisBus::new(state.timing_mode, Some(cartridge), &state.config);
        for (index, &byte) in state.ram.iter().enumerate() {
            bus.memory.write_main_ram_byte(index as u32, byte);
        }
        for (index, &byte) in state.audio_ram.iter().enumerate() {
            bus.memory.write_audio_ram(index as u16, byte);
        }
        bus.vdp = state.vdp;
        bus.ym2612 = state.ym2612;
        bus.psg = state.psg;
        bus.input = state.input;
        bus.cycles = state.cycles;
        bus.open_bus = state.open_bus;
        for bit in 0..9 {
            bus.z80_bank.write_bit(state.z80_bank & (1 << bit) != 0);
        }
        bus.write_word(0xa11100, u16::from(state.bus_request) << 8);
        bus.write_word(0xa11200, u16::from(!state.reset) << 8);
        bus.apply_pending_writes();
        bus.enable_native_write_trace();
        bus.native_ram_writes = Some(Vec::new());
        Self {
            bus,
            z80: state.z80,
            resampler: state.audio_resampler,
            output: Output::default(),
        }
    }
    fn step(&mut self, pc: u32, cycles: u32, wait: bool) -> bool {
        let clocks = self.bus.cycles.record_68k_instruction(
            pc,
            cycles,
            wait,
            self.bus.vdp.should_halt_cpu(),
        );
        while self.bus.cycles.should_tick_z80() {
            if !self.bus.cycles.z80_halt {
                let survey = self.bus.timing_survey.as_mut().unwrap();
                survey.cpu = TimingCpu::Z80;
                survey.pc = u32::from(self.z80.pc());
                self.z80.tick(&mut self.bus);
            }
            self.bus.cycles.z80_cycle();
        }
        let survey = self.bus.timing_survey.as_mut().unwrap();
        survey.cpu = TimingCpu::M68k;
        survey.pc = pc;
        let effect = self
            .bus
            .tick_components(cycles, clocks, wait, &mut self.resampler);
        self.resampler.output_samples(&mut self.output).unwrap();
        effect == VdpTickEffect::FrameComplete
    }
}

#[test]
#[ignore = "requires private ROMs; retained original Z80 sound driver is never embedded"]
fn live_sound_driver_and_every_device_match_reference_on_exact_cycle_steps() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let bytes = rom(revision);
        let mut original = gameplay(&bytes);
        let state = capture_with_requested_pcm(&mut original, revision);
        assert!(!state.reset, "live sound driver must be out of reset");
        let mut reference = ReferenceDevices::new(&bytes, state.clone());
        let mut native =
            DeviceRuntime::from_state(AddressSpace::new(bytes, revision), state).unwrap();
        native.enable_trace();
        native.enable_ram_write_trace();
        // The timed original sender posted a real runtime-ROM sample and
        // released BUSREQ. Hold main CPU work here, not the sound command.
        let mut frames = 0;
        let mut z80_writes = 0;
        for instruction in 0..20_000 {
            let pc = 0x100;
            native.set_instruction_pc(pc);
            native.begin_instruction(0x4e71, 56);
            reference.bus.m68k_opcode = 0x4e71;
            let reference_wait = reference.bus.cycles.m68k_wait_cpu_cycles;
            let (expected_frame, actual_frame) = if reference_wait != 0 {
                let cycles = reference.bus.cycles.take_m68k_wait_cpu_cycles();
                (
                    reference.step(pc, cycles, true),
                    native.take_wait_step(pc).unwrap().frame_complete,
                )
            } else {
                if instruction % 211 == 0 {
                    reference.bus.write_word(0xc00011, 0x9f);
                    native.write_bus_word(0xc00011, 0x9f);
                }
                (
                    reference.step(pc, 4, false),
                    native.finish_instruction(pc, 4).frame_complete,
                )
            };
            assert_eq!(expected_frame, actual_frame);
            frames += usize::from(actual_frame);
            assert_eq!(
                encoded(&reference.bus.cycles),
                encoded(native.cycles()),
                "{revision:?} step {instruction}: clock phase"
            );
            assert!(
                reference.bus.vdp.hybrid_state_eq(native.vdp()),
                "{revision:?} step {instruction}: VDP"
            );
            assert_eq!(
                encoded(&reference.bus.ym2612),
                encoded(native.ym2612()),
                "{revision:?} step {instruction}: YM"
            );
            assert_eq!(
                encoded(&reference.z80),
                encoded(native.z80()),
                "{revision:?} step {instruction}: Z80"
            );
            assert_eq!(
                encoded(&reference.bus.psg),
                encoded(native.psg()),
                "{revision:?} step {instruction}: PSG"
            );
            if actual_frame || instruction == 19_999 {
                for (offset, &byte) in native.audio_ram().iter().enumerate() {
                    assert_eq!(
                        reference.bus.memory.read_audio_ram(offset as u16),
                        byte,
                        "{revision:?} step {instruction}: Z80 RAM"
                    );
                }
                for (offset, &byte) in native.memory().ram().iter().enumerate() {
                    assert_eq!(
                        reference.bus.memory.read_main_ram_byte(offset as u32),
                        byte,
                        "{revision:?} step {instruction}: work RAM"
                    );
                }
            }
            assert_eq!(
                reference.bus.native_writes.as_ref().unwrap(),
                &native.trace().unwrap().writes,
                "{revision:?} step {instruction}: timestamped actual chip dispatch"
            );
            z80_writes += native
                .trace()
                .unwrap()
                .writes
                .iter()
                .filter(|write| write.origin == Origin::Z80)
                .count();
            assert_eq!(
                reference.bus.native_ram_writes.as_ref().unwrap().as_slice(),
                native.ram_write_trace().unwrap(),
                "{revision:?} step {instruction}: all-origin actual RAM commits"
            );
            assert_eq!(
                reference
                    .output
                    .samples
                    .iter()
                    .map(|pair| pair.map(f64::to_bits))
                    .collect::<Vec<_>>(),
                native
                    .audio_samples()
                    .iter()
                    .map(|pair| pair.map(f64::to_bits))
                    .collect::<Vec<_>>(),
                "{revision:?} step {instruction}: stereo sample bits"
            );
            reference.output.samples.clear();
            native.clear_audio();
            reference.bus.native_writes.as_mut().unwrap().clear();
            native.clear_trace();
            reference.bus.native_ram_writes.as_mut().unwrap().clear();
            native.clear_ram_write_trace();
            *reference.bus.timing_survey.as_mut().unwrap() = TimingSurvey::default();
        }
        assert!(
            z80_writes != 0,
            "captured original sound driver must actually issue chip writes"
        );
        eprintln!(
            "{revision:?}: 20000 exact cycle/device steps, {frames} frame boundaries, {z80_writes} actual original-driver chip writes after an authored timed PCM request; zero full-device/PCM/timestamped-chip differences"
        );
    }
}
