#![cfg(feature = "devices")]
//! Private-cartridge proof of the canonical framed body through real Context.
#[path = "portable_wait/prepared_failures.rs"]
mod prepared_failures;
use bincode::{Decode, Encode};
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use genesis_core::{GenesisEmulator, native::CpuAccess};
use jgenesis_common::{
    frontend::{
        AudioOutput, Color, EmulatorTrait, FrameSize, InputPoller, MappableInputs, PartialClone,
        RenderFrameOptions, Renderer, SaveWriter, TickEffect,
    },
    input::Player,
};
use sha1::Digest;
use std::alloc::{GlobalAlloc, Layout, System};
use std::{cell::RefCell, convert::Infallible, fs, io, rc::Rc, sync::Arc};
use tje_engine_core::{CpuState, ReadKind, metadata};
use tje_engine_cranelift::{CacheConfig, CacheStatus, NativeProgram};
use tje_engine_runtime::{
    BusAccess, DeviceStep, Engine, PlatformDevices,
    portable_timing::{
        BoundProgram, TimingBackend,
        engine::{
            CompiledTimingCapability, FrameTimeline, NativeSourceState, NativeTimingContext,
            ProgressOwner, StepObserver,
        },
    },
};
use tje_game::{
    Dispatcher, FramedTranslation,
    execution::{CallCeiling, CallClock},
    ids,
};
use tje_mem::{AddressSpace, Bus};
use tje_native_platform::{ClockedDevices, DeviceRuntime, Registers};
use tje_rom::{Revision, Rom};
use tje_session::{Session, Start};

static FRAMED: &[FramedTranslation] = &[FramedTranslation {
    id: ids::WAIT_FOR_NEXT_V_BLANK,
    function: tje_game::core::wait_for_next_vblank_framed,
}];
struct Sink;
impl Renderer for Sink {
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
impl SaveWriter for Sink {
    type Err = io::Error;
    fn load_bytes(&mut self, _: &str) -> Result<Vec<u8>, Self::Err> {
        Err(io::ErrorKind::NotFound.into())
    }
    fn persist_bytes(&mut self, _: &str, _: &[u8]) -> Result<(), Self::Err> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn load_serialized<D: Decode<()>>(&mut self, _: &str) -> Result<D, Self::Err> {
        Err(io::ErrorKind::NotFound.into())
    }
    fn persist_serialized<E: Encode>(&mut self, _: &str, _: E) -> Result<(), Self::Err> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
#[derive(Default)]
struct Audio(Vec<[u64; 2]>);
impl AudioOutput for Audio {
    type Err = Infallible;
    fn push_sample(&mut self, left: f64, right: f64) -> Result<(), Self::Err> {
        self.0.push([left.to_bits(), right.to_bits()]);
        Ok(())
    }
}
struct Inputs(GenesisInputs);
impl InputPoller<GenesisInputs> for Inputs {
    fn poll(&mut self) -> &GenesisInputs {
        &self.0
    }
}

struct Timeline {
    session: Session,
    ends: Vec<u64>,
    oracle: Option<Rc<RefCell<Oracle>>>,
}
impl Timeline {
    fn new(session: Session) -> Self {
        assert!(
            matches!(session.start, Start::PowerOn),
            "controller-only proof"
        );
        let mut end = 0;
        let ends = session
            .runs
            .iter()
            .map(|run| {
                end += run.0;
                end
            })
            .collect();
        Self {
            session,
            ends,
            oracle: None,
        }
    }
    fn sample(&self, frame: u64) -> GenesisInputs {
        let run = self.session.runs[self.ends.partition_point(|&end| end <= frame)];
        let controller = |pad| {
            if pad & tje_session::CONNECTED != 0 {
                GenesisController::new(GenesisControllerType::ThreeButton)
            } else {
                GenesisController::None
            }
        };
        let mut inputs = GenesisInputs {
            p1: controller(run.1),
            p2: controller(run.2),
        };
        for (mask, player) in [(run.1, Player::One), (run.2, Player::Two)] {
            for (bit, button) in [
                (tje_session::UP, GenesisButton::Up),
                (tje_session::DOWN, GenesisButton::Down),
                (tje_session::LEFT, GenesisButton::Left),
                (tje_session::RIGHT, GenesisButton::Right),
                (tje_session::A, GenesisButton::A),
                (tje_session::B, GenesisButton::B),
                (tje_session::C, GenesisButton::C),
                (tje_session::START, GenesisButton::Start),
            ] {
                inputs.set_field(button, player, mask & bit != 0);
            }
        }
        inputs
    }
}
impl FrameTimeline for Timeline {
    fn latched_inputs(&self, frame: u64) -> tje_game::Result<GenesisInputs> {
        if frame >= self.session.frames {
            return Err(tje_game::Error::Platform(
                "missing recorded native sample".into(),
            ));
        }
        Ok(self.sample(frame))
    }
    fn sample_inputs(&self, frame: u64) -> tje_game::Result<GenesisInputs> {
        self.latched_inputs(frame)
    }
    fn frame_completed(&self, frame: u64, devices: &PlatformDevices) -> tje_game::Result<()> {
        if let Some(oracle) = &self.oracle {
            let mut oracle = oracle.borrow_mut();
            assert_eq!(frame, oracle.frame);
            assert!(
                devices
                    .vdp()
                    .hybrid_state_eq(&oracle.reference.hybrid_bus().vdp)
            );
            assert!(
                devices
                    .audio_samples()
                    .iter()
                    .map(|sample| sample.map(f64::to_bits))
                    .eq(oracle.audio.0.iter().copied())
            );
            let expected = oracle.reference.capture_native_state();
            let actual = devices.capture_state(Registers::from_state(&expected));
            assert_peripheral(&actual, &expected);
            oracle.audio.0.clear();
        }
        Ok(())
    }
}
fn cpu(reference: &GenesisEmulator) -> CpuState {
    let words = reference.hybrid_cpu().hybrid_state_words();
    let mut cpu = CpuState::default();
    cpu.d.copy_from_slice(&words[..8]);
    cpu.a.copy_from_slice(&words[8..15]);
    cpu.usp = words[15];
    cpu.ssp = words[16];
    cpu.pc = words[17];
    cpu.sr = words[18];
    cpu.prefetch = words[19];
    cpu.pending_interrupt = words[20];
    cpu.stopped = words[21];
    cpu.frozen = words[22];
    cpu.address_error = words[23];
    cpu.last_muldiv = words[24];
    cpu
}
fn words(cpu: &CpuState) -> [u32; 25] {
    let mut words = [0; 25];
    words[..8].copy_from_slice(&cpu.d);
    words[8..15].copy_from_slice(&cpu.a);
    words[15..].copy_from_slice(&[
        cpu.usp,
        cpu.ssp,
        cpu.pc,
        cpu.sr,
        cpu.prefetch,
        cpu.pending_interrupt,
        cpu.stopped,
        cpu.frozen,
        cpu.address_error,
        cpu.last_muldiv,
    ]);
    words
}
fn normalize(access: CpuAccess) -> BusAccess {
    match access {
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
                _ => panic!("read purpose"),
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
        CpuAccess::AcknowledgeInterrupt { level } => BusAccess::AcknowledgeInterrupt { level },
    }
}
fn fork(cartridge: &Rom, entry: &GenesisEmulator) -> GenesisEmulator {
    let mut fork = GenesisEmulator::create(
        Some(cartridge.bytes().to_vec()),
        GenesisEmulatorConfig::default(),
        &mut Sink,
    );
    fork.load_state(entry.partial_clone());
    let bus = fork.hybrid_bus_mut();
    bus.native_cpu_accesses = Some(Vec::new());
    bus.native_ram_writes = Some(Vec::new());
    bus.enable_native_write_trace();
    fork
}
fn capture(cartridge: &Rom, timeline: &Timeline, first_frame: u64) -> (GenesisEmulator, u64) {
    let mut reference = GenesisEmulator::create(
        Some(cartridge.bytes().to_vec()),
        GenesisEmulatorConfig::default(),
        &mut Sink,
    );
    let mut frame = 0;
    let mut audio = Audio::default();
    for _ in 0..30_000_000 {
        let original = reference.hybrid_cpu();
        let bus = reference.hybrid_bus();
        if frame >= first_frame
            && original.pc()
                == ids::WAIT_FOR_NEXT_V_BLANK
                    .metadata()
                    .address(cartridge.revision())
            && original.hybrid_instruction_ready(bus)
            && bus.cycles.m68k_wait_cpu_cycles == 0
            && bus.memory.read_main_ram_word(original.stack_pointer() + 6) == 1
            && bus.memory.read_main_ram_byte(
                tje_mem::ram::g_mode2_reg_softcopy(cartridge.revision()).address(),
            ) & 0x40
                != 0
        {
            return (reference, frame);
        }
        let event = reference
            .tick(
                &mut Sink,
                &mut audio,
                &mut Inputs(timeline.sample(frame)),
                &mut Sink,
            )
            .unwrap();
        audio.0.clear();
        if event == TickEffect::FrameRendered {
            frame += 1;
        }
    }
    panic!("canonical natural wait entry not reached");
}
struct Oracle {
    reference: GenesisEmulator,
    audio: Audio,
    frame: u64,
    timeline: Rc<Timeline>,
    accesses: usize,
    commits: usize,
    source: u64,
    compiled: u64,
    steps: u64,
}
struct Observe(Rc<RefCell<Oracle>>);
impl StepObserver for Observe {
    fn completed_step(
        &mut self,
        owner: ProgressOwner,
        step: DeviceStep,
        actual: &CpuState,
        devices: &PlatformDevices,
    ) -> tje_game::Result<()> {
        let mut oracle = self.0.borrow_mut();
        let inputs = oracle.timeline.sample(oracle.frame);
        let before = oracle.reference.hybrid_bus().cycles.m68k_mclk_cycles;
        let Oracle {
            reference, audio, ..
        } = &mut *oracle;
        let event = reference
            .tick(&mut Sink, audio, &mut Inputs(inputs), &mut Sink)
            .unwrap();
        assert_eq!(event == TickEffect::FrameRendered, step.frame_complete);
        assert_eq!(
            oracle.reference.hybrid_bus().cycles.m68k_mclk_cycles - before,
            step.master_clocks
        );
        assert_eq!(
            oracle.reference.hybrid_bus().cycles.m68k_mclk_cycles,
            devices.clock()
        );
        assert_eq!(
            words(actual),
            oracle.reference.hybrid_cpu().hybrid_state_words(),
            "all 25 CPU words at every actual step"
        );
        let expected = oracle
            .reference
            .hybrid_bus()
            .native_cpu_accesses
            .as_ref()
            .unwrap();
        let bus = devices.instruction_trace().unwrap();
        assert_eq!(expected.len(), bus.len());
        assert!(
            expected[oracle.accesses..]
                .iter()
                .copied()
                .map(normalize)
                .eq(bus[oracle.accesses..].iter().copied()),
            "actual bus sequence"
        );
        oracle.accesses = expected.len();
        let expected = oracle
            .reference
            .hybrid_bus()
            .native_ram_writes
            .as_ref()
            .unwrap();
        let commits = devices.ram_write_trace().unwrap();
        assert_eq!(expected.len(), commits.len());
        assert_eq!(
            &expected[oracle.commits..],
            &commits[oracle.commits..],
            "all-origin RAM commits"
        );
        oracle.commits = expected.len();
        oracle.steps += 1;
        oracle.source += u64::from(owner == ProgressOwner::SourceInstruction);
        oracle.compiled +=
            u64::from(owner == (ProgressOwner::CompiledEngineInterrupt { instruction: true }));
        if step.frame_complete {
            oracle.frame += 1;
        }
        Ok(())
    }
}
fn assert_peripheral(
    actual: &genesis_core::native::PeripheralState,
    expected: &genesis_core::native::PeripheralState,
) {
    assert_eq!(actual.ram, expected.ram);
    assert_eq!(actual.audio_ram, expected.audio_ram);
    let encoded = |value| bincode::encode_to_vec(value, bincode::config::standard()).unwrap();
    assert_eq!(
        encoded((
            &actual.vdp,
            &actual.ym2612,
            &actual.psg,
            &actual.input,
            &actual.z80,
            &actual.cycles,
            &actual.audio_resampler
        )),
        encoded((
            &expected.vdp,
            &expected.ym2612,
            &expected.psg,
            &expected.input,
            &expected.z80,
            &expected.cycles,
            &expected.audio_resampler
        )),
        "all encoded peripheral internals and resampler sample bits",
    );
    assert_eq!(actual.vdp.frame_buffer(), expected.vdp.frame_buffer());
    assert_eq!(
        bincode::encode_to_vec(&actual.config, bincode::config::standard()).unwrap(),
        bincode::encode_to_vec(&expected.config, bincode::config::standard()).unwrap(),
        "all configuration fields, including exact floating-point bits",
    );
    assert_eq!(
        (
            actual.z80_bank,
            actual.bus_request,
            actual.reset,
            actual.timing_mode,
            actual.region,
            actual.native_input_remainder,
            actual.open_bus
        ),
        (
            expected.z80_bank,
            expected.bus_request,
            expected.reset,
            expected.timing_mode,
            expected.region,
            expected.native_input_remainder,
            expected.open_bus
        )
    );
    assert_eq!(
        (
            actual.program_counter,
            actual.data,
            actual.address,
            actual.status,
            actual.user_stack_pointer,
            actual.supervisor_stack_pointer
        ),
        (
            expected.program_counter,
            expected.data,
            expected.address,
            expected.status,
            expected.user_stack_pointer,
            expected.supervisor_stack_pointer
        )
    );
}
fn registers(cpu: &CpuState) -> Registers {
    Registers {
        data: cpu.d,
        address: std::array::from_fn(|i| cpu.read_address(i as u8)),
        status: cpu.sr as u16,
        program_counter: cpu.pc,
        user_stack_pointer: cpu.usp,
        supervisor_stack_pointer: cpu.ssp,
    }
}
fn run_case(
    cartridge: &Rom,
    entry: &GenesisEmulator,
    frame: u64,
    session: &Session,
    backend: NativeProgram,
) -> (Engine<NativeProgram>, u64, u64) {
    let mut devices: PlatformDevices = DeviceRuntime::from_state(
        AddressSpace::new(cartridge.bytes().to_vec(), cartridge.revision()),
        entry.capture_native_state(),
    )
    .unwrap()
    .into();
    devices.enable_instruction_trace();
    devices.enable_ram_write_trace();
    devices.enable_trace();
    let mut machine = Engine::from_state(cpu(entry), devices, backend, frame);
    let capability = CompiledTimingCapability::validate(&machine, cartridge).unwrap();
    let cache = Arc::new(
        BoundProgram::compile(
            cartridge,
            ids::WAIT_FOR_NEXT_V_BLANK,
            tje_game::core::timed_wait::WaitSource::definition(),
            machine.devices().cycles().m68k_divider_u32.get(),
            &[4, 6],
        )
        .unwrap(),
    );
    let mut state = NativeSourceState::new(cache);
    let timeline = Rc::new(Timeline::new(session.clone()));
    let oracle = Rc::new(RefCell::new(Oracle {
        reference: fork(cartridge, entry),
        audio: Audio::default(),
        frame,
        timeline: Rc::clone(&timeline),
        accesses: 0,
        commits: 0,
        source: 0,
        compiled: 0,
        steps: 0,
    }));
    let mut frame_timeline = Timeline::new(session.clone());
    frame_timeline.oracle = Some(Rc::clone(&oracle));
    let mut observer = Observe(Rc::clone(&oracle));
    let dispatcher =
        Dispatcher::native_with_frames(&[ids::WAIT_FOR_NEXT_V_BLANK], &[], FRAMED).unwrap();
    let mut clock = CallClock::default();
    let root = clock
        .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
        .unwrap();
    let usage = {
        let mut context = NativeTimingContext::new(
            &mut machine,
            cartridge,
            &capability,
            &mut clock,
            &frame_timeline,
            dispatcher,
            &mut state,
        )
        .unwrap()
        .with_observer(&mut observer);
        assert_eq!(
            context.invoke_prepared(ids::WAIT_FOR_NEXT_V_BLANK).unwrap(),
            None
        );
        context.usage().unwrap()
    };
    clock.leave(root).unwrap();
    let oracle = oracle.borrow();
    assert_eq!(
        usage.original_isr_instructions, 0,
        "no original CPU in native composition"
    );
    assert_eq!(
        usage.scheduled_isr_instructions, 0,
        "compiled ISR is not readable ISR proof"
    );
    assert_eq!(usage.compiled_isr_instructions, oracle.compiled);
    assert_eq!(usage.instructions, oracle.source);
    assert_eq!(
        words(machine.cpu()),
        oracle.reference.hybrid_cpu().hybrid_state_words()
    );
    let expected = oracle.reference.capture_native_state();
    let observed = machine.devices().capture_state(registers(machine.cpu()));
    assert_peripheral(&observed, &expected);
    assert!(
        machine
            .devices()
            .audio_samples()
            .iter()
            .map(|sample| sample.map(f64::to_bits))
            .eq(oracle.audio.0.iter().copied())
    );
    assert_eq!(
        machine.devices().trace().unwrap().writes,
        *oracle
            .reference
            .hybrid_bus()
            .native_writes
            .as_ref()
            .unwrap(),
        "timed chip/video writes"
    );
    eprintln!(
        "{:?}: canonical framed Context source={}; compiled_engine_ISR={}; steps={}; frames={}→{}; per-step25CPU/bus/RAM and full devices/audio exact",
        cartridge.revision(),
        oracle.source,
        oracle.compiled,
        oracle.steps,
        frame,
        oracle.frame
    );
    (machine, oracle.source, oracle.compiled)
}

#[test]
#[ignore = "requires both private ROM revisions and recorded controller-edge sessions"]
fn both_revision_canonical_context_wait_composes_with_actual_compiled_irqs() {
    for (revision, rom_key, session_key) in [
        (Revision::Rev02, "TJE_REV02_ROM", "TJE_TIMING_SESSION_REV02"),
        (Revision::Rev00, "TJE_REV00_ROM", "TJE_TIMING_SESSION_REV00"),
    ] {
        let cartridge = Rom::import(&fs::read(std::env::var(rom_key).unwrap()).unwrap()).unwrap();
        assert_eq!(cartridge.revision(), revision);
        let session =
            Session::from_json(&fs::read_to_string(std::env::var(session_key).unwrap()).unwrap())
                .unwrap();
        let timeline = Timeline::new(session.clone());
        let (entry, frame) = capture(&cartridge, &timeline, 837);
        assert_eq!(frame, 837);
        let program =
            metadata::compile_program(cartridge.bytes(), &metadata::game_metadata(revision))
                .unwrap();
        let digest: [u8; 20] = sha1::Sha1::digest(cartridge.bytes()).into();
        let backend = NativeProgram::compile(digest, &program).unwrap();
        assert!(backend.authenticates_compilation(digest, &program));
        let (_, source, compiled) = run_case(&cartridge, &entry, frame, &session, backend);
        assert!(source > 11 && compiled > 0);
    }
}

#[test]
fn cold_and_cache_hit_capabilities_authenticate_rom_ir_and_actual_divider() {
    const MOVE_WORD_ABSOLUTE_TO_D0: u16 = 0x3039;
    const VDP_PORT_HIGH: u16 = 0x00c0;
    const VDP_STATUS_LOW: u16 = 0x0004;
    const NOP: u16 = 0x4e71;
    let mut rom = vec![0; 0x10a];
    for (index, word) in [MOVE_WORD_ABSOLUTE_TO_D0, VDP_PORT_HIGH, VDP_STATUS_LOW, NOP]
        .into_iter()
        .enumerate()
    {
        rom[0x100 + index * 2..0x102 + index * 2].copy_from_slice(&word.to_be_bytes());
    }
    let program = tje_engine_core::Program {
        instructions: vec![
            tje_engine_core::compile_instruction_with_divider(&rom, 0x100, 7).unwrap(),
        ],
    };
    let wrong_divider = tje_engine_core::Program {
        instructions: vec![
            tje_engine_core::compile_instruction_with_divider(&rom, 0x100, 6).unwrap(),
        ],
    };
    let mut wrong_ir = program.clone();
    let changed = wrong_ir.instructions[0]
        .ops
        .iter_mut()
        .find_map(|op| match op {
            tje_engine_core::ir::Op::Constant { value, .. } => Some(value),
            _ => None,
        })
        .unwrap();
    *changed ^= 1;
    let digest: [u8; 20] = sha1::Sha1::digest(&rom).into();
    let directory = tempfile::tempdir().unwrap();
    let cache = CacheConfig {
        data_dir: directory.path().into(),
    };
    let cold = NativeProgram::load_or_compile(digest, &program, &cache).unwrap();
    assert_eq!(cold.stats().cache_status, CacheStatus::Miss);
    let warm = NativeProgram::load_or_compile(digest, &program, &cache).unwrap();
    assert_eq!(warm.stats().cache_status, CacheStatus::Hit);
    assert_eq!(cold.compiled_identity(), warm.compiled_identity());
    let mut wrong_digest = digest;
    wrong_digest[0] ^= 1;
    for backend in [&cold, &warm] {
        assert!(backend.contains_compiled_instruction(0x100));
        assert!(!backend.contains_compiled_instruction(0x102));
        assert!(backend.authenticates_compilation(digest, &program));
        assert!(!backend.authenticates_compilation(wrong_digest, &program));
        assert!(!backend.authenticates_compilation(digest, &wrong_divider));
        assert!(!backend.authenticates_compilation(digest, &wrong_ir));
    }
}

thread_local! {
    static COUNT_ALLOCATIONS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static ALLOCATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ALLOCATED_BYTES: std::cell::Cell<[usize; 16]> = const { std::cell::Cell::new([0; 16]) };
}
struct CountingAllocator;
fn allocation(bytes: usize) {
    if COUNT_ALLOCATIONS
        .try_with(std::cell::Cell::get)
        .unwrap_or(false)
    {
        let _ = ALLOCATIONS.try_with(|count| {
            let index = count.get();
            if index < 16 {
                ALLOCATED_BYTES.with(|sizes| {
                    let mut values = sizes.get();
                    values[index] = bytes;
                    sizes.set(values);
                });
            }
            count.set(index + 1);
        });
    }
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocation(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocation(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocation(size);
        unsafe { System.realloc(pointer, layout, size) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn authored_banked_z80_bursts_near_ym_sync_keep_all_device_allocations_zero() {
    for prefixes in [false, true] {
        // Authored EX (SP),HL, or a complete 32 KiB DD prefix bank followed by
        // RAM NOP after PC wrap. No cartridge instruction bytes are used.
        let mut rom = vec![0; 0x10000];
        rom[..4].copy_from_slice(&0xffff00_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x200_u32.to_be_bytes());
        rom[0x200..0x202].copy_from_slice(&0x4e71_u16.to_be_bytes());
        if prefixes {
            rom[0x8000..].fill(0xdd);
        } else {
            rom[0x8000] = 0xe3;
        }
        let reference = GenesisEmulator::create(
            Some(rom.clone()),
            GenesisEmulatorConfig::default(),
            &mut Sink,
        );
        let mut state = reference.capture_native_state();
        state.audio_ram.fill(0);
        state.reset = false;
        state.bus_request = false;
        state.z80_bank = 1;
        state.z80.set_pc(0x8000);
        state.z80.set_sp(0x9000);
        state.cycles.m68k_mclk_cycles = genesis_core::timing::MAX_YM2612_LAG_MCLK - 1;
        state.cycles.z80_mclk_cycles =
            state.cycles.m68k_mclk_cycles - genesis_core::timing::Z80_DIVIDER;
        let before_z80 = state.cycles.z80_mclk_cycles;
        let mut devices =
            DeviceRuntime::from_state(AddressSpace::new(rom, Revision::Rev02), state).unwrap();
        devices.begin_instruction(0x4e71, 0);
        ALLOCATIONS.with(|count| count.set(0));
        ALLOCATED_BYTES.with(|sizes| sizes.set([0; 16]));
        COUNT_ALLOCATIONS.with(|enabled| enabled.set(true));
        let step = devices.finish_instruction(0x200, 1);
        COUNT_ALLOCATIONS.with(|enabled| enabled.set(false));
        assert_eq!(
            ALLOCATIONS.with(std::cell::Cell::get),
            0,
            "banked prefix={prefixes}, sizes={:?}",
            ALLOCATED_BYTES.with(std::cell::Cell::get)
        );
        let after = devices.capture_state(Registers::default());
        let accesses = if prefixes { 0x8000 } else { 5 };
        assert_eq!(
            after.cycles.z80_mclk_cycles - before_z80,
            (3 * accesses + 1) * genesis_core::timing::Z80_DIVIDER
        );
        assert_eq!(step.master_clocks, 7);
        assert!(!step.frame_complete);
    }
}

fn unobserved_call(
    machine: &mut Engine<NativeProgram>,
    cartridge: &Rom,
    capability: &CompiledTimingCapability,
    state: &mut NativeSourceState,
    timeline: &dyn FrameTimeline,
    counted: bool,
) -> (usize, tje_engine_runtime::timing_region::RegionUsage) {
    let dispatcher =
        Dispatcher::native_with_frames(&[ids::WAIT_FOR_NEXT_V_BLANK], &[], FRAMED).unwrap();
    let mut clock = CallClock::default();
    let root = clock
        .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
        .unwrap();
    ALLOCATIONS.with(|count| count.set(0));
    ALLOCATED_BYTES.with(|sizes| sizes.set([0; 16]));
    COUNT_ALLOCATIONS.with(|enabled| enabled.set(counted));
    let result = (|| {
        let mut context = NativeTimingContext::new(
            machine, cartridge, capability, &mut clock, timeline, dispatcher, state,
        )?;
        context.invoke_prepared(ids::WAIT_FOR_NEXT_V_BLANK)?;
        Ok::<_, tje_game::Error>(context.usage().unwrap())
    })();
    COUNT_ALLOCATIONS.with(|enabled| enabled.set(false));
    let count = ALLOCATIONS.with(std::cell::Cell::get);
    if count != 0 {
        eprintln!(
            "measured allocation byte sizes: {:?}",
            ALLOCATED_BYTES.with(std::cell::Cell::get)
        );
    }
    let usage = result.unwrap();
    clock.leave(root).unwrap();
    (count, usage)
}

struct LatchedTimeline<'a> {
    recorded: &'a Timeline,
    latch: std::cell::Cell<(u64, GenesisInputs)>,
    samples: std::cell::Cell<usize>,
    recipes: std::cell::Cell<usize>,
    outputs: std::cell::Cell<usize>,
    fault: u8,
}
impl<'a> LatchedTimeline<'a> {
    fn new(recorded: &'a Timeline, frame: u64) -> Self {
        let inputs = recorded.sample_inputs(frame).unwrap();
        Self {
            recorded,
            latch: std::cell::Cell::new((frame, inputs)),
            samples: std::cell::Cell::new(1),
            recipes: std::cell::Cell::new(0),
            outputs: std::cell::Cell::new(0),
            fault: 0,
        }
    }
}
impl FrameTimeline for LatchedTimeline<'_> {
    fn latched_inputs(&self, frame: u64) -> tje_game::Result<GenesisInputs> {
        let (sampled_frame, inputs) = self.latch.get();
        if sampled_frame != frame {
            return Err(tje_game::Error::Platform("stale caller input latch".into()));
        }
        Ok(inputs)
    }
    fn sample_inputs(&self, frame: u64) -> tje_game::Result<GenesisInputs> {
        let inputs = self.recorded.sample_inputs(frame)?;
        match self.fault {
            1 => {} // Seeded provider violates the latch-update contract.
            2 => {
                let mut wrong = inputs;
                wrong.p1 = GenesisController::None;
                self.latch.set((frame, wrong));
            }
            _ => self.latch.set((frame, inputs)),
        }
        self.samples.set(self.samples.get() + 1);
        Ok(inputs)
    }
    fn before_frame(&self, frame: u64, memory: &mut dyn Bus) -> tje_game::Result<()> {
        self.recipes.set(self.recipes.get() + 1);
        self.recorded.before_frame(frame, memory)
    }
    fn frame_completed(&self, frame: u64, devices: &PlatformDevices) -> tje_game::Result<()> {
        self.outputs.set(self.outputs.get() + 1);
        self.recorded.frame_completed(frame, devices)
    }
}

#[test]
#[ignore = "requires both private ROMs and recorded controller-edge sessions"]
fn warm_real_source_entries_and_actual_compiled_irqs_allocate_zero() {
    for (revision, rom_key, session_key) in [
        (Revision::Rev02, "TJE_REV02_ROM", "TJE_TIMING_SESSION_REV02"),
        (Revision::Rev00, "TJE_REV00_ROM", "TJE_TIMING_SESSION_REV00"),
    ] {
        let cartridge = Rom::import(&fs::read(std::env::var(rom_key).unwrap()).unwrap()).unwrap();
        let session =
            Session::from_json(&fs::read_to_string(std::env::var(session_key).unwrap()).unwrap())
                .unwrap();
        let timeline = Timeline::new(session);
        let (entry, frame) = capture(&cartridge, &timeline, 837);
        let program =
            metadata::compile_program(cartridge.bytes(), &metadata::game_metadata(revision))
                .unwrap();
        let digest: [u8; 20] = sha1::Sha1::digest(cartridge.bytes()).into();
        let backend = NativeProgram::compile(digest, &program).unwrap();
        let memory = AddressSpace::new(cartridge.bytes().to_vec(), revision);
        let devices = DeviceRuntime::from_state(memory.clone(), entry.capture_native_state())
            .unwrap()
            .into();
        let mut machine = Engine::from_state(cpu(&entry), devices, backend, frame);
        let capability = CompiledTimingCapability::validate(&machine, &cartridge).unwrap();
        let cache = Arc::new(
            BoundProgram::compile(
                &cartridge,
                ids::WAIT_FOR_NEXT_V_BLANK,
                tje_game::core::timed_wait::WaitSource::definition(),
                7,
                &[4, 6],
            )
            .unwrap(),
        );
        let mut state = NativeSourceState::new(Arc::clone(&cache));
        unobserved_call(
            &mut machine,
            &cartridge,
            &capability,
            &mut state,
            &timeline,
            false,
        );
        let (_, _, backend) = machine.into_parts();
        let devices = DeviceRuntime::from_state(memory.clone(), entry.capture_native_state())
            .unwrap()
            .into();
        let mut machine = Engine::from_state(cpu(&entry), devices, backend, frame);
        let latched = LatchedTimeline::new(&timeline, frame);
        let (allocations, usage) = unobserved_call(
            &mut machine,
            &cartridge,
            &capability,
            &mut state,
            &latched,
            true,
        );
        assert_eq!(allocations, 0, "warm real source+compiled ISR execution");
        assert!(usage.instructions > 11 && usage.compiled_isr_instructions > 0);
        assert_eq!(
            (
                latched.samples.get(),
                latched.recipes.get(),
                latched.outputs.get()
            ),
            (2, 1, 1),
            "entry borrows current input; each real renderer event samples/recipes/outputs once"
        );
        let previous_frame = machine.frame_count();
        for _ in 0..1_000_000 {
            let step = machine
                .step(latched.latched_inputs(machine.frame_count()).unwrap())
                .unwrap();
            if step.devices.frame_complete {
                let next = machine.frame_count();
                let sample = latched.sample_inputs(next).unwrap();
                assert_eq!(latched.latched_inputs(next).unwrap(), sample);
                latched
                    .before_frame(next, machine.devices_mut().memory_mut())
                    .unwrap();
                latched.frame_completed(next, machine.devices()).unwrap();
                machine.devices_mut().clear_audio();
                break;
            }
        }
        assert_eq!(machine.frame_count(), previous_frame + 1);
        assert_eq!(
            (
                latched.samples.get(),
                latched.recipes.get(),
                latched.outputs.get()
            ),
            (3, 2, 2),
            "the same caller latch/output coordinator spans compiled and source frames"
        );
        let (_, _, backend) = machine.into_parts();
        let mut devices: PlatformDevices =
            DeviceRuntime::from_state(memory.clone(), entry.capture_native_state())
                .unwrap()
                .into();
        let mut no_irq = cpu(&entry);
        no_irq.sr |= 0x700;
        devices
            .memory_mut()
            .write_u16(no_irq.read_address(7) + 6, 0);
        let mut machine = Engine::from_state(no_irq.clone(), devices, backend, frame);
        let latched = LatchedTimeline::new(&timeline, frame);
        for _ in 0..8 {
            // Repeated real prepared fixtures: preserve elapsed devices and the
            // same caller latch; seed only the next authentic count-zero entry.
            *machine.cpu_mut() = no_irq.clone();
            let (allocations, usage) = unobserved_call(
                &mut machine,
                &cartridge,
                &capability,
                &mut state,
                &latched,
                true,
            );
            assert_eq!(allocations, 0, "count-zero/no-IRQ source entry");
            assert_eq!(usage.compiled_isr_instructions, 0);
        }
        assert_eq!(
            (
                latched.samples.get(),
                latched.recipes.get(),
                latched.outputs.get()
            ),
            (1, 0, 0),
            "same-frame entries must neither poll again nor invent frame work"
        );
        let (_, _, mut backend) = machine.into_parts();
        for fault in [1, 2] {
            let devices = DeviceRuntime::from_state(memory.clone(), entry.capture_native_state())
                .unwrap()
                .into();
            let mut machine = Engine::from_state(cpu(&entry), devices, backend, frame);
            let mut state = NativeSourceState::new(Arc::clone(&cache));
            let mut latched = LatchedTimeline::new(&timeline, frame);
            latched.fault = fault;
            let mut clock = CallClock::default();
            let root = clock
                .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
                .unwrap();
            let dispatcher =
                Dispatcher::native_with_frames(&[ids::WAIT_FOR_NEXT_V_BLANK], &[], FRAMED).unwrap();
            let result = {
                let mut context = NativeTimingContext::new(
                    &mut machine,
                    &cartridge,
                    &capability,
                    &mut clock,
                    &latched,
                    dispatcher,
                    &mut state,
                )
                .unwrap();
                context.invoke_prepared(ids::WAIT_FOR_NEXT_V_BLANK)
            };
            let failure = match result {
                Err(error) => error,
                Ok(_) => panic!("invalid latch accepted"),
            };
            assert!(failure.to_string().contains("latch"), "{failure}");
            assert_eq!(
                (
                    latched.samples.get(),
                    latched.recipes.get(),
                    latched.outputs.get()
                ),
                (2, 0, 1)
            );
            clock.leave(root).unwrap();
            (_, _, backend) = machine.into_parts();
        }
        eprintln!(
            "{revision:?}: zero allocations in warm canonical source entries, with actual compiled IRQs and count-zero/no-IRQ fixture"
        );
    }
}

struct NeverSample(std::cell::Cell<usize>);
impl FrameTimeline for NeverSample {
    fn latched_inputs(&self, _: u64) -> tje_game::Result<GenesisInputs> {
        self.0.set(self.0.get() + 1);
        panic!("invalid native capability must reject before timeline/source access")
    }
    fn sample_inputs(&self, _: u64) -> tje_game::Result<GenesisInputs> {
        panic!("invalid native capability cannot sample a frame")
    }
    fn frame_completed(&self, _: u64, _: &PlatformDevices) -> tje_game::Result<()> {
        panic!("invalid capability cannot produce a frame")
    }
}
fn idle_machine(cartridge: &Rom, backend: NativeProgram) -> Engine<NativeProgram> {
    let reference = GenesisEmulator::create(
        Some(cartridge.bytes().to_vec()),
        GenesisEmulatorConfig::default(),
        &mut Sink,
    );
    let devices = DeviceRuntime::from_state(
        AddressSpace::new(cartridge.bytes().to_vec(), cartridge.revision()),
        reference.capture_native_state(),
    )
    .unwrap()
    .into();
    Engine::from_state(cpu(&reference), devices, backend, 0)
}
#[test]
#[ignore = "requires both private ROMs; authentic compiled capability admission"]
fn cross_cartridge_wrong_divider_ir_and_missing_continuations_reject_before_effects() {
    let a = Rom::import(&fs::read(std::env::var("TJE_REV02_ROM").unwrap()).unwrap()).unwrap();
    let b = Rom::import(&fs::read(std::env::var("TJE_REV00_ROM").unwrap()).unwrap()).unwrap();
    static FOREIGN_SCOPE: tje_game::timing::Program = tje_game::timing::Program {
        name: "negative cross-function scope",
        source: "authored declaration, no code or effects",
        entry: tje_game::timing::Location {
            function: ids::WAIT_FOR_NEXT_V_BLANK,
            role: 0,
        },
        functions: &[
            tje_game::timing::FunctionRegion {
                function: ids::WAIT_FOR_NEXT_V_BLANK,
                roles: 1,
            },
            tje_game::timing::FunctionRegion {
                function: ids::ABS,
                roles: 1,
            },
        ],
        edges: &[],
    };
    for cartridge in [&a, &b] {
        assert!(matches!(
            BoundProgram::compile(
                cartridge,
                ids::WAIT_FOR_NEXT_V_BLANK,
                &FOREIGN_SCOPE,
                7,
                &[4, 6]
            ),
            Err(tje_engine_core::timing_region::TimingError::InvalidIr(
                "ordinary timing sources require exactly one reviewed function"
            ))
        ));
    }
    let program =
        metadata::compile_program(a.bytes(), &metadata::game_metadata(a.revision())).unwrap();
    let digest: [u8; 20] = sha1::Sha1::digest(a.bytes()).into();
    let backend = NativeProgram::compile(digest, &program).unwrap();
    let machine = idle_machine(&a, backend);
    let capability = CompiledTimingCapability::validate(&machine, &a).unwrap();
    let (_, _, backend) = machine.into_parts();
    let mut wrong_cartridge = idle_machine(&b, backend);
    let before = wrong_cartridge.cpu().clone();
    let clock_before = wrong_cartridge.devices().clock();
    let ram_before = wrong_cartridge.devices().memory().ram().to_vec();
    let cache = Arc::new(
        BoundProgram::compile(
            &b,
            ids::WAIT_FOR_NEXT_V_BLANK,
            tje_game::core::timed_wait::WaitSource::definition(),
            7,
            &[4, 6],
        )
        .unwrap(),
    );
    let mut state = NativeSourceState::new(cache);
    let mut clock = CallClock::default();
    let root = clock
        .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
        .unwrap();
    let timeline = NeverSample(std::cell::Cell::new(0));
    let dispatcher =
        Dispatcher::native_with_frames(&[ids::WAIT_FOR_NEXT_V_BLANK], &[], FRAMED).unwrap();
    assert!(
        NativeTimingContext::new(
            &mut wrong_cartridge,
            &b,
            &capability,
            &mut clock,
            &timeline,
            dispatcher,
            &mut state
        )
        .is_err()
    );
    assert_eq!(timeline.0.get(), 0);
    assert_eq!(wrong_cartridge.cpu(), &before);
    assert_eq!(wrong_cartridge.devices().clock(), clock_before);
    assert_eq!(
        wrong_cartridge.devices().memory().ram().as_slice(),
        ram_before
    );
    clock.leave(root).unwrap();

    let wrong_divider = metadata::compile_program_with_divider(
        a.bytes(),
        &metadata::game_metadata(a.revision()),
        6,
    )
    .unwrap();
    let mut wrong_ir = program.clone();
    let value = wrong_ir
        .instructions
        .iter_mut()
        .flat_map(|ir| ir.ops.iter_mut())
        .find_map(|op| match op {
            tje_engine_core::ir::Op::Constant { value, .. } => Some(value),
            _ => None,
        })
        .unwrap();
    *value ^= 1;
    let incomplete = tje_engine_core::Program {
        instructions: vec![program.instructions[0].clone()],
    };
    for invalid in [wrong_divider, wrong_ir, incomplete] {
        let backend = NativeProgram::compile(digest, &invalid).unwrap();
        let machine = idle_machine(&a, backend);
        let before = machine.cpu().clone();
        let clock = machine.devices().clock();
        assert!(CompiledTimingCapability::validate(&machine, &a).is_err());
        assert_eq!(machine.cpu(), &before);
        assert_eq!(machine.devices().clock(), clock);
    }
}

struct CorruptReturn {
    timeline: Timeline,
    saved_pc: Rc<std::cell::Cell<Option<u32>>>,
    changed: std::cell::Cell<bool>,
    target: u32,
}
impl FrameTimeline for CorruptReturn {
    fn latched_inputs(&self, frame: u64) -> tje_game::Result<GenesisInputs> {
        self.timeline.latched_inputs(frame)
    }
    fn sample_inputs(&self, frame: u64) -> tje_game::Result<GenesisInputs> {
        self.timeline.sample_inputs(frame)
    }
    fn before_frame(&self, frame: u64, memory: &mut dyn Bus) -> tje_game::Result<()> {
        if frame == 838
            && !self.changed.get()
            && let Some(address) = self.saved_pc.get()
        {
            memory.write_u32(address, self.target);
            self.changed.set(true);
        }
        Ok(())
    }
    fn frame_completed(&self, _: u64, _: &PlatformDevices) -> tje_game::Result<()> {
        Ok(())
    }
}
struct ActualEntryObserver {
    saved_pc: Rc<std::cell::Cell<Option<u32>>>,
    boundaries: usize,
}
impl StepObserver for ActualEntryObserver {
    fn completed_step(
        &mut self,
        _: ProgressOwner,
        _: DeviceStep,
        cpu: &CpuState,
        devices: &PlatformDevices,
    ) -> tje_game::Result<()> {
        let observed = devices.exception_trace().unwrap();
        for event in &observed[self.boundaries..] {
            if matches!(
                event.boundary,
                tje_engine_core::ExceptionBoundary::Entered { vector: 30, .. }
            ) {
                // Actual accepted IRQ6 marker, before any handler opcode. This
                // is a corruption fixture, never a way to mint return authority.
                assert_ne!(cpu.sr & 0x2000, 0);
                self.saved_pc.set(Some(cpu.ssp.wrapping_add(2)));
            }
        }
        self.boundaries = observed.len();
        Ok(())
    }
}
#[test]
#[ignore = "requires private ROMs and actual controller-edge IRQ sessions"]
fn actual_wrong_frame_rte_keeps_real_return_marker_and_state_but_rejects_approval() {
    for (revision, rom_key, session_key) in [
        (Revision::Rev02, "TJE_REV02_ROM", "TJE_TIMING_SESSION_REV02"),
        (Revision::Rev00, "TJE_REV00_ROM", "TJE_TIMING_SESSION_REV00"),
    ] {
        let cartridge = Rom::import(&fs::read(std::env::var(rom_key).unwrap()).unwrap()).unwrap();
        let session =
            Session::from_json(&fs::read_to_string(std::env::var(session_key).unwrap()).unwrap())
                .unwrap();
        let timeline = Timeline::new(session.clone());
        let (entry, frame) = capture(&cartridge, &timeline, 837);
        let program =
            metadata::compile_program(cartridge.bytes(), &metadata::game_metadata(revision))
                .unwrap();
        let digest: [u8; 20] = sha1::Sha1::digest(cartridge.bytes()).into();
        let backend = NativeProgram::compile(digest, &program).unwrap();
        let mut devices: PlatformDevices = DeviceRuntime::from_state(
            AddressSpace::new(cartridge.bytes().to_vec(), revision),
            entry.capture_native_state(),
        )
        .unwrap()
        .into();
        devices.enable_exception_trace();
        let mut machine = Engine::from_state(cpu(&entry), devices, backend, frame);
        let capability = CompiledTimingCapability::validate(&machine, &cartridge).unwrap();
        let cache = Arc::new(
            BoundProgram::compile(
                &cartridge,
                ids::WAIT_FOR_NEXT_V_BLANK,
                tje_game::core::timed_wait::WaitSource::definition(),
                7,
                &[4, 6],
            )
            .unwrap(),
        );
        let mut state = NativeSourceState::new(cache);
        let saved_pc = Rc::new(std::cell::Cell::new(None));
        let corrupt = CorruptReturn {
            timeline: Timeline::new(session),
            saved_pc: Rc::clone(&saved_pc),
            changed: std::cell::Cell::new(false),
            target: ids::ERROR_TRAP.metadata().address(revision),
        };
        let mut observer = ActualEntryObserver {
            saved_pc,
            boundaries: 0,
        };
        let mut clock = CallClock::default();
        let root = clock
            .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
            .unwrap();
        let dispatcher =
            Dispatcher::native_with_frames(&[ids::WAIT_FOR_NEXT_V_BLANK], &[], FRAMED).unwrap();
        let error = {
            let mut context = NativeTimingContext::new(
                &mut machine,
                &cartridge,
                &capability,
                &mut clock,
                &corrupt,
                dispatcher,
                &mut state,
            )
            .unwrap()
            .with_observer(&mut observer);
            context
                .invoke_prepared(ids::WAIT_FOR_NEXT_V_BLANK)
                .unwrap_err()
        };
        assert!(
            corrupt.changed.get(),
            "actual accepted frame must have been corrupted"
        );
        assert!(
            error.to_string().contains("different stack/SR/pipeline"),
            "{error}"
        );
        assert_eq!(
            machine.cpu().pc,
            corrupt.target,
            "never repair the real wrong return"
        );
        let returned = machine.devices().exception_trace().unwrap().last().unwrap();
        assert_eq!(
            returned.boundary,
            tje_engine_core::ExceptionBoundary::Returned
        );
        assert!(
            returned.master_clock < machine.devices().clock(),
            "real RTE marker precedes device advance"
        );
        clock.leave(root).unwrap();
    }
}
