use super::*;
use crate::{
    CheckReport, Mode,
    abi::HookContext,
    execution::{WaitClock, WaitInputPoller},
    hooks::HookRegistry,
    input::{Button, InputHold, InputScript},
    sinks::{HeadlessAudio, HeadlessOutput, NoCartridgeSaves},
};
use genesis_config::GenesisEmulatorConfig;
use genesis_core::{GenesisBus, GenesisEmulator};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use tje_game::{
    execution::ExecutionClass,
    exits::{EntryOutcome, Exit, ExitContract},
    ids,
};
use tje_mem::{Address, ObjectState, Revision, ram};

pub(crate) fn synthetic_host(main: &[u16], handler: &[u16]) -> Host {
    // Authored reset/vector tables and short programs, never cartridge bytes.
    let mut rom = vec![0; 0x100000];
    rom[..4].copy_from_slice(&catalogue::STACK_TOP.to_be_bytes());
    rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    for vector in [28, 30, 32] {
        let offset = 4 * vector;
        rom[offset..offset + 4].copy_from_slice(&0x200_u32.to_be_bytes());
    }
    rom[4 * 33..4 * 34].copy_from_slice(&0x300_u32.to_be_bytes());
    for (address, words) in [(0x100, main), (0x200, handler), (0x300, &[0x4e73][..])] {
        for (index, word) in words.iter().enumerate() {
            let offset = address + 2 * index;
            rom[offset..offset + 2].copy_from_slice(&word.to_be_bytes());
        }
    }
    let mut emulator = GenesisEmulator::create(
        Some(rom),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    emulator.hybrid_bus_mut().hybrid_writes = Some(Vec::new());
    let revision = Revision::Rev02;
    let registry = HookRegistry::empty(revision);
    let checks = CheckReport::registered(&registry);
    Host {
        emulator,
        registry,
        mode: Mode::Reference,
        revision,
        symbols: catalogue::RevisionSymbols::for_revision(revision),
        frame: 0,
        main_idle: false,
        frame_progress: FrameProgress::default(),
        terminal_entry: None,
        output: HeadlessOutput,
        audio: HeadlessAudio::default(),
        saves: NoCartridgeSaves,
        audio_sample_rate: 48_000,
        session_trace: None,
        wait_clock: WaitClock::default(),
        checks,
        measurements: Default::default(),
    }
}

fn write(bus: &mut GenesisBus, address: u32, value: u16) {
    bus.write_word(address, value);
    bus.apply_pending_writes();
}

fn original_tick(host: &mut Host) {
    assert_eq!(
        host.tick_live_original(&mut Inputs(GenesisInputs::default()))
            .unwrap(),
        TickEffect::None,
    );
}

fn machine_bytes(host: &Host) -> Vec<u8> {
    bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap()
}

#[test]
fn settled_boundary_is_zero_work_and_does_not_rewrite_machine() {
    let mut host = synthetic_host(&[0x60fe], &[0x4e73]);
    let before = machine_bytes(&host);
    let advance = host
        .settle_fixture_boundary(GenesisInputs::default())
        .unwrap();
    assert_eq!(advance.ticks, 0);
    assert_eq!(advance.master_clocks, 0);
    assert_eq!(advance.start_pc, advance.end_pc);
    assert!(advance.completed_frames.is_empty());
    assert_eq!(before, machine_bytes(&host));
}

#[test]
fn nested_exception_returns_finish_before_seeding_even_in_replace_mode() {
    // TRAP #0; BRA.S *; handler: TRAP #1; NOP; RTE; inner handler: RTE.
    let mut host = synthetic_host(&[0x4e40, 0x60fe], &[0x4e41, 0x4e71, 0x4e73]);
    let mut reference = synthetic_host(&[0x4e40, 0x60fe], &[0x4e41, 0x4e71, 0x4e73]);
    original_tick(&mut host);
    original_tick(&mut reference);
    assert_eq!(host.cpu().hybrid_exception_depth(), 1);
    assert_eq!(host.cpu().hybrid_last_exception_vector(), Some(32));
    assert!(
        host.cpu()
            .hybrid_instruction_ready(host.emulator.hybrid_bus())
    );
    assert!(!host.fixture_boundary_is_settled());
    let mut contract = ids::GET_RANDOM_NUMBER.metadata().clone();
    contract.name = "NeverReplaceOriginalHandler";
    contract.addresses = [0x200; 2];
    host.registry
        .register_contract(contract, true, |_| panic!("settling dispatched a hook"))
        .unwrap();
    host.mode = Mode::Replace;
    let before = machine_bytes(&host);
    let error = host.check_fixture(0x200, &[]).unwrap_err();
    assert!(error.to_string().contains("before seeding fixture inputs"));
    assert_eq!(before, machine_bytes(&host));
    let advance = host
        .settle_fixture_boundary(GenesisInputs::default())
        .unwrap();
    assert_eq!(advance.handlers_completed, 2);
    assert_eq!(host.cpu().hybrid_last_exception_vector(), Some(33));
    assert_eq!(advance.end_pc, 0x102);
    assert!(host.fixture_boundary_is_settled());
    for _ in 0..advance.ticks {
        original_tick(&mut reference);
    }
    assert_eq!(machine_bytes(&host), machine_bytes(&reference));
}

#[test]
fn pending_irq_is_accepted_and_its_original_handler_returns() {
    // Main: BRA.S *. IRQ handler: ADDQ.L #1,D2; RTE.
    let mut host = synthetic_host(&[0x60fe], &[0x5282, 0x4e73]);
    host.emulator.hybrid_parts().0.set_status_register(0x2000);
    write(host.emulator.hybrid_bus_mut(), 0xc00004, 0x8010);
    write(host.emulator.hybrid_bus_mut(), 0xc00004, 0x8a00);
    for _ in 0..10000 {
        if host.emulator.hybrid_bus().interrupt_level() == 4
            && host.emulator.hybrid_bus().cycles.m68k_wait_cpu_cycles == 0
        {
            break;
        }
        original_tick(&mut host);
    }
    assert_eq!(host.emulator.hybrid_bus().interrupt_level(), 4);
    // The first original IRQ tick only latches the pending interrupt. Settling
    // must not stop there or inside the instruction-ready handler it enters.
    original_tick(&mut host);
    assert_eq!(host.cpu().hybrid_exception_depth(), 0);
    assert!(
        !host
            .cpu()
            .hybrid_instruction_ready(host.emulator.hybrid_bus())
    );
    let advance = host
        .settle_fixture_boundary(GenesisInputs::default())
        .unwrap();
    assert_eq!(advance.handlers_completed, 1);
    assert_eq!(host.cpu().hybrid_last_exception_vector(), Some(28));
    assert_eq!(host.cpu().data_registers()[2], 1);
    assert_eq!(host.cpu().pc(), 0x100);
    assert_eq!(host.cpu().stack_pointer(), catalogue::STACK_TOP);
    assert_eq!(host.cpu().status_register() & 0xff00, 0x2000);
    assert!(host.fixture_boundary_is_settled());
}

#[test]
fn fifo_halt_and_refresh_wait_advance_without_patching_cpu() {
    let mut host = synthetic_host(&[0x60fe], &[0x4e73]);
    let mut reference = synthetic_host(&[0x60fe], &[0x4e73]);
    for host in [&mut host, &mut reference] {
        let bus = host.emulator.hybrid_bus_mut();
        bus.cycles.m68k_wait_cpu_cycles = 20;
        write(bus, 0xc00004, 0x8f02);
        write(bus, 0xc00004, 0x4000);
        write(bus, 0xc00004, 0);
        for value in 0..10 {
            write(bus, 0xc00000, value);
        }
        assert!(bus.halt());
    }
    let before = machine_bytes(&host);
    let error = host.check_call(0x100, &[]).unwrap_err();
    assert!(error.to_string().contains("settled live boundary"));
    assert_eq!(before, machine_bytes(&host));
    let advance = host
        .settle_fixture_boundary(GenesisInputs::default())
        .unwrap();
    assert!(advance.ticks > 0);
    assert!(advance.master_clocks > 0);
    assert!(host.fixture_boundary_is_settled());
    for _ in 0..advance.ticks {
        original_tick(&mut reference);
    }
    assert_eq!(machine_bytes(&host), machine_bytes(&reference));
}

#[derive(Default)]
struct Timeline {
    samples: Mutex<Vec<u64>>,
    recipes: Mutex<Vec<u64>>,
}
impl WaitInputPoller for Timeline {
    fn at(&self, frame: u64) -> Result<GenesisInputs> {
        self.samples.lock().unwrap().push(frame);
        Ok(GenesisInputs::default())
    }
    fn before_frame(&self, frame: u64, memory: &mut dyn tje_mem::Bus) -> Result<()> {
        self.recipes.lock().unwrap().push(frame);
        memory.write_u32(0xff9000, frame as u32);
        Ok(())
    }
}

#[test]
fn crossed_and_partial_frames_retain_output_observations_and_timeline() {
    // TRAP #0. Handler burns several video frames: MOVE.W #32767,D0;
    // DBF D0,*; RTE. No synthetic parking loop or caller-state rewrite is used.
    let mut host = synthetic_host(&[0x4e40, 0x60fe], &[0x303c, 0x7fff, 0x51c8, 0xfffe, 0x4e73]);
    original_tick(&mut host);
    let timeline = Arc::new(Timeline::default());
    host.set_wait_input_poller(timeline.clone());
    host.frame_progress.idle_draws = 7;
    let lag = LagEvent {
        frame: 0,
        pc_at_vblank: 0x100,
        frame_counter: 0,
    };
    host.frame_progress.lag_events.push(lag.clone());
    let advance = host
        .settle_fixture_boundary(GenesisInputs::default())
        .unwrap();
    assert!(advance.completed_frames.len() >= 2);
    assert_eq!(advance.completed_frames[0].idle_random_draws, 7);
    assert_eq!(advance.completed_frames[0].lag_events, [lag]);
    assert_eq!(advance.end_frame, advance.completed_frames.len() as u64);
    assert_eq!(host.frame_count(), advance.end_frame);
    assert_eq!(host.wait_clock.frame, advance.end_frame);
    for (index, snapshot) in advance.completed_frames.iter().enumerate() {
        assert_eq!(snapshot.frame, index as u64);
        assert!(!snapshot.audio_samples.is_empty());
    }
    let expected: Vec<_> = (1..=advance.end_frame).collect();
    assert_eq!(*timeline.samples.lock().unwrap(), expected);
    assert_eq!(*timeline.recipes.lock().unwrap(), expected);
    assert_eq!(host.context().read_u32(0xff9000), advance.end_frame as u32);
    let partial_samples = host.audio.samples.clone();
    assert!(!partial_samples.is_empty());
    let next = host.run_frame(GenesisInputs::default()).unwrap();
    assert_eq!(next.frame, advance.end_frame);
    assert!(next.audio_samples.starts_with(&partial_samples));
    assert_eq!(host.measurements.idle_draws_per_frame[0], 7);
}

#[test]
fn missing_timeline_and_settling_bound_fail_without_faking_readiness() {
    let mut host = synthetic_host(&[0x4e40, 0x60fe], &[0x60fe]);
    original_tick(&mut host);
    let error = host
        .settle_fixture_boundary_inner(GenesisInputs::default(), 3)
        .unwrap_err();
    assert!(error.to_string().contains("after 3 original ticks"));
    assert_eq!(host.cpu().hybrid_exception_depth(), 1);
    assert!(!host.fixture_boundary_is_settled());
    let error = host
        .settle_fixture_boundary(GenesisInputs::default())
        .unwrap_err();
    assert!(matches!(error, HostError::Input(_)), "{error}");
    assert_eq!(host.frame_count(), 1);
    assert_eq!(host.wait_clock.frame, 1);
    assert_eq!(host.cpu().hybrid_exception_depth(), 1);
    host.reset();
    assert_eq!(host.cpu().hybrid_exception_depth(), 0);
    assert_eq!(host.cpu().hybrid_last_exception_vector(), None);
    assert_eq!(host.frame_progress.idle_draws, 0);
    assert!(host.frame_progress.lag_events.is_empty());
}

#[test]
fn prepared_real_exception_returns_match_in_both_classes_banks_and_revisions() {
    fn native_rte(_: &mut HookContext<'_>) -> Result<Option<u32>> {
        tje_game::exits::exception_return(ids::GET_RANDOM_NUMBER).map_err(Into::into)
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        for class in [ExecutionClass::Atomic, ExecutionClass::TimeAware] {
            for saved_sr in [0x0015_u16, 0x231a] {
                let mut host = synthetic_host(&[0x4e40, 0x60fe], &[0x4e73]);
                host.revision = revision;
                {
                    let mut context = host.context();
                    let cpu = context.cpu();
                    cpu.set_address_registers(
                        cpu.address_registers(),
                        0xff4000,
                        cpu.supervisor_stack_pointer(),
                    );
                    cpu.set_status_register(saved_sr);
                }
                original_tick(&mut host);
                assert_eq!(host.cpu().hybrid_exception_depth(), 1);
                let frame_sp = host.cpu().stack_pointer();
                assert_eq!(host.context().read_u16(frame_sp), saved_sr);
                let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
                symbol.name = "AuthoredEnteredRte";
                symbol.addresses = [0x200; 2];
                symbol.sizes = [2; 2];
                symbol.abi.argument_count = 0;
                symbol.abi.returns_value = false;
                let mut registry = HookRegistry::empty(revision);
                registry
                    .register_exit_contract(
                        symbol,
                        class,
                        ExitContract::ExceptionReturn,
                        native_rte,
                    )
                    .unwrap();
                host.checks = CheckReport::registered(&registry);
                host.registry = registry;
                let before = machine_bytes(&host);
                let outcome = host.check_entry_fixture(0x200).unwrap();
                assert!(matches!(
                    outcome,
                    EntryOutcome::Transfer(transfer) if transfer.exit == Exit::ExceptionReturn
                ));
                assert_eq!(before, machine_bytes(&host));
                assert_eq!(host.cpu().hybrid_exception_depth(), 1);
                assert_eq!(host.checks.calls, 1);
                assert_eq!(host.checks.mismatches, 0);
                if class == ExecutionClass::TimeAware {
                    let raw = host.checks.last_time_aware.as_ref().unwrap();
                    assert!(!raw.original.iter().any(|event| matches!(
                        event,
                        crate::check_projection::LogicEvent::ExceptionReturned { .. }
                    )));
                    let summary = &host.checks.time_aware["AuthoredEnteredRte"];
                    assert_eq!(summary.original_interrupts, 0);
                    assert_eq!(summary.translated_interrupts, 0);
                }
            }
        }
    }
}

#[test]
fn terminal_entry_is_dispatched_once_while_original_clocks_and_irqs_continue() {
    fn terminal(_: &mut HookContext<'_>) -> Result<Option<u32>> {
        tje_game::exits::terminal(ids::GET_RANDOM_NUMBER).map_err(Into::into)
    }
    fn rte(_: &mut HookContext<'_>) -> Result<Option<u32>> {
        tje_game::exits::exception_return(ids::LEVEL6_ISR).map_err(Into::into)
    }
    for mode in [Mode::Check, Mode::Replace] {
        let mut host = synthetic_host(&[0x60fe], &[0x4e73]);
        host.mode = mode;
        let mut registry = HookRegistry::empty(host.revision);
        for (id, name, pc, contract, body) in [
            (
                ids::GET_RANDOM_NUMBER,
                "AuthoredTerminal",
                0x100,
                ExitContract::Terminal,
                terminal as crate::hooks::HookFn,
            ),
            (
                ids::LEVEL6_ISR,
                "AuthoredIrqReturn",
                0x200,
                ExitContract::ExceptionReturn,
                rte,
            ),
        ] {
            let mut symbol = id.metadata().clone();
            symbol.name = name;
            symbol.addresses = [pc; 2];
            symbol.sizes = [2; 2];
            symbol.abi.argument_count = 0;
            symbol.abi.returns_value = false;
            registry
                .register_exit_contract(symbol, ExecutionClass::Atomic, contract, body)
                .unwrap();
        }
        host.checks = CheckReport::registered(&registry);
        host.registry = registry;
        host.context().cpu().set_status_register(0x2000);
        write(host.emulator.hybrid_bus_mut(), 0xc00004, 0x8120);
        for _ in 0..3 {
            if mode == Mode::Check {
                host.run_frame(GenesisInputs::default()).unwrap();
            } else {
                host.play_frame(GenesisInputs::default()).unwrap();
            }
        }
        let terminal = &host.checks.by_function["AuthoredTerminal"];
        assert_eq!(terminal.calls_checked + terminal.calls_replaced, 1);
        let irq = &host.checks.by_function["AuthoredIrqReturn"];
        assert!(irq.calls_checked + irq.calls_replaced > 0);
        assert_eq!(host.terminal_entry, Some(ids::GET_RANDOM_NUMBER));
        assert_eq!(host.frame_count(), 3);
        host.reset();
        assert_eq!(host.terminal_entry, None);
    }
}

#[test]
fn continuation_restores_explicit_parent_frame_without_rewriting_current_sp() {
    const CODE: u32 = 0xfff000;
    const ENTRY_SP: u32 = 0xff7ffc;
    fn unused(_: &mut HookContext<'_>) -> Result<Option<u32>> {
        unreachable!("continuation replaces the registered prefix body")
    }
    fn idle(context: &mut dyn tje_game::Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
        assert_eq!(context.frame_state().unwrap().entry_sp, ENTRY_SP);
        assert_eq!(context.stack_pointer()?, ENTRY_SP - 20);
        tje_game::exits::main_loop(ids::GET_RANDOM_NUMBER, tje_game::exits::MainLoopStage::Idle)
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        for class in [ExecutionClass::Atomic, ExecutionClass::TimeAware] {
            let mut host = synthetic_host(&[0x60fe], &[0x4e73]);
            host.revision = revision;
            host.context().write_u16(CODE, 0x4e71); // Authored NOP -> idle checkpoint.
            host.context().write_u16(CODE + 2, 0x60fe);
            crate::abi::set_sp(host.context().cpu(), ENTRY_SP - 20);
            let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
            symbol.name = "AuthoredPersistentContinuation";
            symbol.addresses = [CODE; 2];
            symbol.sizes = [4; 2];
            symbol.abi.argument_count = 0;
            symbol.abi.returns_value = false;
            let mut registry = HookRegistry::empty(revision);
            registry
                .register_exit_contract(
                    symbol,
                    class,
                    ExitContract::MainLoop {
                        begin_offset: 0,
                        idle_offset: 2,
                        calls: &[],
                    },
                    unused,
                )
                .unwrap();
            host.checks = CheckReport::registered(&registry);
            host.registry = registry;
            let before = machine_bytes(&host);
            let frame = tje_game::call_frame::FrameState::new(ENTRY_SP);
            let outcome = host
                .check_continuation_fixture(
                    ids::GET_RANDOM_NUMBER,
                    tje_game::exits::MainLoopStage::Begin,
                    idle,
                    frame,
                )
                .unwrap();
            assert!(matches!(
                outcome, EntryOutcome::Transfer(transfer)
                    if matches!(transfer.exit, Exit::MainLoop { stage: tje_game::exits::MainLoopStage::Idle })
            ));
            assert_eq!(before, machine_bytes(&host));
            assert_eq!(host.checks.calls, 1);
            let wrong = tje_game::call_frame::FrameState::new(ENTRY_SP - 22);
            let error = host
                .check_continuation_fixture(
                    ids::GET_RANDOM_NUMBER,
                    tje_game::exits::MainLoopStage::Begin,
                    idle,
                    wrong,
                )
                .unwrap_err();
            assert!(error.to_string().contains("frame/SP mismatch"), "{error}");
            assert_eq!(before, machine_bytes(&host));
        }
    }
}

#[test]
fn time_aware_pending_cartridge_dma_is_completed_before_rom_returns_to_live() {
    use crate::stack_dependency::{CheckSide, compare_check_runs};
    use jgenesis_common::debug::DebugMemoryView;
    const CODE: u32 = 0xfff000;
    const RETURN: u32 = 0xfff300;
    const COMMANDS: [u16; 9] = [
        0x8114, 0x8f02, 0x9302, 0x9400, 0x9580, 0x9600, 0x9700, 0x4000, 0x0080,
    ];
    fn dma(c: &mut HookContext<'_>) -> Result<Option<u32>> {
        for value in COMMANDS {
            tje_mem::Platform::write_vdp_control(c, value);
        }
        assert!(
            c.bus().halt(),
            "real cartridge DMA is pending at Rust return"
        );
        Ok(None)
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        // The synthetic cartridge's source bytes are authored test values.
        let mut host = synthetic_host(&[0x1234, 0x5678], &[0x4e73]);
        host.revision = revision;
        let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
        symbol.name = "AuthoredPendingCartridgeDma";
        symbol.addresses = [CODE; 2];
        symbol.sizes = [74; 2];
        symbol.abi.argument_count = 0;
        symbol.abi.returns_value = false;
        let mut registry = HookRegistry::empty(revision);
        registry
            .register_execution_contract(symbol, ExecutionClass::TimeAware, dma)
            .unwrap();
        let hook = registry.at(CODE).unwrap().clone();
        host.registry = registry;
        let code = COMMANDS
            .into_iter()
            .flat_map(|value| [0x33fc, value, 0x00c0, 0x0004])
            .chain(std::iter::once(0x4e75));
        for (index, word) in code.enumerate() {
            host.context().write_u16(CODE + 2 * index as u32, word);
        }
        let sp = host.cpu().stack_pointer() - 4;
        host.context().write_u32(sp, RETURN);
        host.context().write_u16(RETURN, 0x4e71);
        {
            let (cpu, bus) = host.emulator.hybrid_parts();
            crate::abi::set_sp(cpu, sp);
            cpu.set_pc(CODE, bus);
        }
        let live_before = machine_bytes(&host);
        let mut translated = host.fork();
        let rust = host
            .run_check_side(&mut translated, &hook, CheckSide::Rust)
            .unwrap();
        let mut original = host.fork();
        let reference = host
            .run_check_side(
                &mut original,
                &hook,
                CheckSide::Original {
                    boundary: crate::exits::ReferenceBoundary::returned(RETURN, sp + 4),
                    outcome: rust.outcome,
                },
            )
            .unwrap();
        for fork in [&original, &translated] {
            let (vram, _, _) = fork.hybrid_bus().vdp.hybrid_memories();
            assert_eq!(&vram[..4], &[0x12, 0x34, 0x56, 0x78]);
            assert_eq!(fork.hybrid_bus().vdp.hybrid_registers().dma_length, 0);
            assert_eq!(
                fork.hybrid_bus().vdp.hybrid_registers().dma_source_address,
                0x104
            );
        }
        let before_original =
            bincode::encode_to_vec(&original, bincode::config::standard()).unwrap();
        let before_rust = bincode::encode_to_vec(&translated, bincode::config::standard()).unwrap();
        let (entry_ram, _) = host.emulator.hybrid_bus_mut().memory.debug_ram_view();
        compare_check_runs(
            &mut original,
            &mut translated,
            &reference,
            &rust,
            &hook,
            entry_ram,
            sp,
        )
        .unwrap();
        assert_eq!(
            before_original,
            bincode::encode_to_vec(&original, bincode::config::standard()).unwrap()
        );
        assert_eq!(
            before_rust,
            bincode::encode_to_vec(&translated, bincode::config::standard()).unwrap()
        );
        translated
            .hybrid_bus_mut()
            .vdp
            .debug_vram_view()
            .write(0, 0xff);
        let error = compare_check_runs(
            &mut original,
            &mut translated,
            &reference,
            &rust,
            &hook,
            entry_ram,
            sp,
        )
        .unwrap_err();
        assert!(error.contains("VDP state differs"), "{error}");
        assert_eq!(live_before, machine_bytes(&host));
        assert_eq!(host.context().read_u32(0x100), 0x12345678);
        assert!(!host.emulator.hybrid_bus().hybrid_functional_mode());
    }
}

#[derive(Deserialize)]
struct Recording {
    format: u32,
    revision: String,
    frames: u64,
    runs: Vec<RecordedRun>,
}
#[derive(Deserialize)]
struct RecordedRun {
    frames: u64,
    pads: [u16; 2],
}
fn player_recording(revision: Revision) -> InputScript {
    let slug = match revision {
        Revision::Rev02 => "rev02",
        Revision::Rev00 => "rev00",
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../sessions/{}-one-player.tje.json", slug));
    let recording: Recording = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(recording.format, 1);
    assert_eq!(recording.revision, slug);
    assert!(recording.frames >= 10000);
    let mut frame = 0;
    let holds = recording
        .runs
        .into_iter()
        .map(|run| {
            assert_eq!(run.pads[0] & 0x100, 0x100);
            assert_eq!(run.pads[1], 0);
            let start = frame;
            frame += run.frames;
            let buttons = [
                Button::Up,
                Button::Down,
                Button::Left,
                Button::Right,
                Button::A,
                Button::B,
                Button::C,
                Button::Start,
            ];
            InputHold {
                start,
                end: frame,
                p1: buttons
                    .into_iter()
                    .enumerate()
                    .filter_map(|(bit, button)| (run.pads[0] & (1 << bit) != 0).then_some(button))
                    .collect(),
                p2: Vec::new(),
            }
        })
        .collect();
    assert_eq!(frame, recording.frames);
    InputScript {
        p2_connected: false,
        holds,
    }
}

// G/src/player/entity_motion_terrain.c:22, matching Player lane terrain.rs.
// A local test hook keeps this host regression independent of lane promotion.
fn player_predicate(context: &mut HookContext<'_>) -> Result<Option<u32>> {
    let entity = Address::<ObjectState>::new(context.argument_u32(0));
    let x = i32::from(context.argument_u32(1) as i16);
    let y = i32::from(context.argument_u32(2) as i16);
    let level = context.argument_u32(3) as i8;
    Ok(Some(u32::from(
        entity.level().read(context) == level
            && (i32::from(entity.x().read(context)) - x).abs() < 16
            && (i32::from(entity.y().read(context)) - y).abs() < 5,
    )))
}

#[test]
#[ignore = "requires both private ROMs; exact Player 10000-frame regression"]
fn player_entity_fixture_after_recorded_10000_frames() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (key, slug) = match revision {
            Revision::Rev02 => ("TJE_REV02_ROM", "rev02"),
            Revision::Rev00 => ("TJE_REV00_ROM", "rev00"),
        };
        let path = std::env::var_os(key)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../../roms/{slug}.bin"))
            });
        let mut registry = HookRegistry::empty(revision);
        registry
            .register_contract(
                ids::ENTITY_IS_NEAR_POSITION.metadata().clone(),
                true,
                player_predicate,
            )
            .unwrap();
        let mut host = Host::new(&std::fs::read(path).unwrap(), Mode::Reference, registry).unwrap();
        let script = Arc::new(player_recording(revision));
        host.set_wait_input_poller(script.clone());
        for frame in 0..10000 {
            host.run_frame(script.at(frame)).unwrap();
        }
        assert!(host.measurements.game_frame.is_some());
        let initial_settled = host.fixture_boundary_is_settled();
        if revision == Revision::Rev00 {
            assert!(
                !initial_settled,
                "regression must start at the unsettled natural boundary"
            );
        }
        let advance = host
            .settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        let player = ram::g_player_info(revision).at(0);
        // The exact first three fixture mutations from Player's failing case.
        player.level().write(&mut host.context(), 0);
        player.x().write(&mut host.context(), 0);
        player.y().write(&mut host.context(), 0);
        let before = machine_bytes(&host);
        let value = host
            .check_fixture(
                ids::ENTITY_IS_NEAR_POSITION.metadata().address(revision),
                &[player.address(), 0, 0, 0],
            )
            .unwrap();
        assert_eq!(value, 1);
        assert_eq!(host.checks.calls, 1);
        assert_eq!(host.checks.mismatches, 0);
        assert!(host.checks.uninitialized_stack_reads.is_empty());
        assert_eq!(before, machine_bytes(&host));
        println!(
            "SETTLED_PLAYER_FIXTURE {}",
            serde_json::json!({
                "revision": revision.name(), "natural_frames": 10000, "initial_settled": initial_settled,
                "start_pc": advance.start_pc, "end_pc": advance.end_pc, "ticks": advance.ticks,
                "master_clocks": advance.master_clocks, "start_frame": advance.start_frame,
                "end_frame": advance.end_frame, "handlers_completed": advance.handlers_completed,
                "completed_frames": advance.completed_frames.len(), "fixture_result": value,
                "checks": host.checks.calls, "mismatches": host.checks.mismatches
            })
        );
    }
}
