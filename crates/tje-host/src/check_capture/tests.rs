use super::*;
use crate::{
    Mode,
    abi::HookContext,
    execution::WaitInputPoller,
    fixture::tests::synthetic_host_from_rom,
    hooks::HookRegistry,
    input::InputScript,
    sinks::{Inputs, NoCartridgeSaves},
    stack_dependency::CheckSide,
};
use genesis_config::GenesisEmulatorConfig;
use jgenesis_common::frontend::{EmulatorTrait, TickEffect};
use std::sync::Arc;
use tje_game::{
    execution::{ExecutionClass, WaitRequest},
    ids,
};
use tje_rom::Revision;

const ROOT: u32 = 0x600;
const COUNTER: u32 = 0xff8000;

fn noop(_: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    Ok(None)
}
fn actual_wait(context: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    let previous = context.read_u16(COUNTER);
    tje_game::Platform::wait_until(context, WaitRequest::vblank_frame(COUNTER, previous))?;
    Ok(None)
}

fn actual_wait_twice(context: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    // The first actual IRQ can return before the frame/input callback. The
    // second wait forces that later callback after retaining the first RTE.
    actual_wait(context)?;
    actual_wait(context)
}

fn host(body: crate::hooks::HookFn) -> crate::Host {
    let mut rom = vec![0; 0x1000];
    rom[..4].copy_from_slice(&catalogue::STACK_TOP.to_be_bytes());
    rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    for vector in [28, 30] {
        rom[4 * vector..4 * vector + 4].copy_from_slice(&0x400_u32.to_be_bytes());
    }
    for (address, words) in [
        (
            0x100,
            &[
                0x33fc_u16, 0x8120, 0x00c0, 0x0004, 0x46fc, 0x2000, 0x4eb9, 0, 0x0600, 0x60f8,
            ][..],
        ),
        (0x200, &[0x60fe, 0x4e71][..]),
        // Actual MOVEM, JSR, LINK, scalar register push/pop and RTE. These are
        // observations, not a source-ownership or relocation approval.
        (
            0x400,
            &[0x48e7, 0xc0c0, 0x4eb9, 0, 0x0500, 0x4cdf, 0x0303, 0x4e73][..],
        ),
        (
            0x500,
            &[
                0x4e56, 0xfff8, 0x2f00, 0x5279, 0x00ff, 0x8000, 0x201f, 0x4e5e, 0x4e75,
            ][..],
        ),
        (ROOT, &[0x303c, 0x03ff, 0x51c8, 0xfffe, 0x4e75][..]),
    ] {
        for (index, word) in words.iter().enumerate() {
            let offset = address as usize + 2 * index;
            rom[offset..offset + 2].copy_from_slice(&word.to_be_bytes());
        }
    }
    let mut host = synthetic_host_from_rom(rom);
    host.wait_clock.poller = Some(Arc::new(InputScript::default()));
    let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
    symbol.name = "AuthoredCapturedCall";
    symbol.addresses = [ROOT; 2];
    symbol.sizes = [10; 2];
    symbol.abi.argument_count = 0;
    symbol.abi.returns_value = false;
    let mut registry = HookRegistry::empty(Revision::Rev02);
    registry
        .register_execution_contract(symbol, ExecutionClass::TimeAware, body)
        .unwrap();
    host.checks = crate::CheckReport::registered(&registry);
    host.registry = registry;
    // Reference-only advancement reaches a real call before a real VBlank.
    // No checking, virtual IRQ, guest-memory patch or ready-PC inference occurs.
    for _ in 0..1_000_000 {
        let bus = host.emulator.hybrid_bus();
        if host.cpu().pc() == ROOT
            && (214..224).contains(&bus.vdp.scanline())
            && bus.cycles.m68k_wait_cpu_cycles == 0
            && bus.interrupt_level() == 0
            && host.cpu().hybrid_exception_depth() == 0
            && host.cpu().hybrid_instruction_ready(bus)
        {
            host.wait_clock.frame = host.frame;
            host.mode = Mode::Check;
            return host;
        }
        if host
            .tick_live_original(&mut Inputs(Default::default()))
            .unwrap()
            == TickEffect::FrameRendered
        {
            host.finish_live_frame().unwrap();
        }
    }
    panic!("authored call did not naturally approach VBlank");
}

fn completed(side: &CapturedSide) -> &CheckCaptureEvidence {
    match side {
        CapturedSide::Completed(evidence) => evidence,
        other => panic!("expected actual completed side: {other:?}"),
    }
}
fn binding() -> CaptureBinding {
    CaptureBinding {
        function: "AuthoredDirectCapture".into(),
        entry_pc: ROOT,
        revision: "rev02",
        side: Side::Original,
        purpose: "AuthoredObservation".into(),
        origin: "Fixture".into(),
        declared_class: "TimeAware",
        proof_envelope: "ClassDefined",
        actual_route: "TimeAwareLogic",
        explicit_proof_only_admission: false,
    }
}

#[test]
fn optin_is_check_only_and_late_enable_does_not_invent_history() {
    let mut host = host(noop);
    host.mode = Mode::Replace;
    let before = bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap();
    assert!(host.set_check_capture(true).is_err());
    assert_eq!(
        bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap(),
        before
    );
    host.mode = Mode::Check;
    let hook = host.registry.at(ROOT).unwrap().clone();
    let uncaptured = host.check_hook(&hook).unwrap_err().to_string();
    assert!(host.actual_check_capture().is_none());
    host.set_check_capture(true).unwrap();
    assert!(host.actual_check_capture().is_none());
    let captured = host.check_hook(&hook).unwrap_err().to_string();
    assert_eq!(captured, uncaptured);
    assert_eq!(
        bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap(),
        before
    );
}

#[test]
fn actual_marker_frames_distinguish_zero_irq_from_unexecuted_side() {
    let mut host = host(noop);
    host.set_check_capture(true).unwrap();
    let hook = host.registry.at(ROOT).unwrap().clone();
    host.check_hook(&hook).unwrap_err();
    let diagnostic = host.actual_check_capture().unwrap();
    assert_eq!(diagnostic.comparison, Some("failed"));
    let original = completed(&diagnostic.original);
    let translated = completed(&diagnostic.translated);
    assert_eq!(original.exceptions.len(), 1);
    assert!(translated.exceptions.is_empty()); // actually executed, not NotExecuted
    let irq = &original.exceptions[0];
    assert_eq!((irq.vector, irq.handler, irq.depth), (30, 0x400, 1));
    assert!(irq.returned.is_some());
    assert!(irq.issues.is_empty(), "{:?}", irq.issues);
    let before = irq.before_frame.as_ref().unwrap();
    let after = irq.after_frame.as_ref().unwrap();
    assert_eq!(before.start, after.start);
    assert_eq!(before.bytes.len(), after.bytes.len());
    let counterpart = irq.counterpart_final_frame.as_ref().unwrap();
    assert_eq!(counterpart.start, after.start);
    assert_eq!(counterpart.bytes.len(), after.bytes.len());
    assert_eq!(
        counterpart.checkpoint.words,
        translated.final_observer_checkpoint.unwrap().words
    );
    for (offset, value) in counterpart.bytes.iter().enumerate() {
        // The actually executed no-op side never writes this old frame area.
        assert_eq!(
            *value,
            host.emulator
                .hybrid_bus()
                .memory
                .read_main_ram_byte(counterpart.start + offset as u32)
        );
    }
    assert!(before.bytes.len() >= 42);
    assert_eq!(before.checkpoint.words, irq.interrupted.unwrap().words);
    assert!(original.native_journal_scope_established);
    assert!(original.bus_journals.cpu_issues.as_ref().unwrap().iter().any(|issue| matches!(issue, journal::CpuIssue::Read { address, purpose_code: 1, .. } if *address < 0x400000)));
    let opcodes: Vec<_> = original
        .observations
        .iter()
        .filter_map(|event| match event {
            Observation::Instruction(observation) => Some(observation.instruction.opcode),
            _ => None,
        })
        .collect();
    for opcode in [0x48e7, 0x4eb9, 0x4e56, 0x2f00, 0x4cdf, 0x4e73] {
        assert!(opcodes.contains(&opcode), "missing {opcode:04X}");
    }
    assert!(original.observations.iter().any(|event| matches!(event, Observation::Access(access) if access.address == COUNTER && access.writing)));
    assert_eq!(original.admission, "capture_only_no_projection_approval");
    assert_eq!(host.finish_future_frame_provenance().provisional, 0);
}

#[test]
fn both_actual_irq_versions_are_retained_without_a_commutation_approval() {
    let mut host = host(actual_wait);
    host.set_check_capture(true).unwrap();
    let hook = host.registry.at(ROOT).unwrap().clone();
    let _actual_comparison_result = host.check_hook(&hook);
    let diagnostic = host.actual_check_capture().unwrap();
    let original = completed(&diagnostic.original);
    let translated = completed(&diagnostic.translated);
    assert_eq!(original.exceptions.len(), 1);
    assert_eq!(translated.exceptions.len(), 1);
    for irq in [&original.exceptions[0], &translated.exceptions[0]] {
        assert!(irq.before_frame.is_some());
        assert!(irq.after_frame.is_some());
        assert!(irq.counterpart_final_frame.is_some());
        assert!(irq.returned.is_some());
    }
    assert_eq!(diagnostic.admission, "capture_only_no_projection_approval");
    assert_eq!(host.finish_future_frame_provenance().provisional, 0);
}

#[test]
fn borrowed_native_trace_owners_are_forwarded_and_restored() {
    let mut host = host(noop);
    let hook = host.registry.at(ROOT).unwrap().clone();
    let bus = host.emulator.hybrid_bus_mut();
    bus.native_cpu_accesses = Some(vec![genesis_core::native::CpuAccess::Read {
        address: 0x1234,
        word: true,
        value: 7,
        purpose: 1,
    }]);
    bus.native_ram_writes = Some(vec![genesis_core::native::RamWrite::new(
        0xff1234, false, 9,
    )]);
    bus.enable_native_write_trace();
    bus.native_writes
        .as_mut()
        .unwrap()
        .push(genesis_core::native::TimedWrite {
            clock: 17,
            origin: genesis_core::native::Origin::Z80,
            pc: 0x20,
            write: genesis_core::native::DeviceWrite::Psg { value: 0x9f },
        });
    let mut ordinary = host.fork();
    let mut captured = host.fork();
    host.run_check_side(
        &mut ordinary,
        &hook,
        CheckSide::Rust,
        crate::call_clock::ClockRunPurpose::Baseline,
        crate::future_frame_watch::ObservationOrigin::CheckFork,
    )
    .unwrap();
    host.set_check_capture(true).unwrap();
    host.run_check_side(
        &mut captured,
        &hook,
        CheckSide::Rust,
        crate::call_clock::ClockRunPurpose::Baseline,
        crate::future_frame_watch::ObservationOrigin::CheckFork,
    )
    .unwrap();
    assert_eq!(
        bincode::encode_to_vec(&ordinary, bincode::config::standard()).unwrap(),
        bincode::encode_to_vec(&captured, bincode::config::standard()).unwrap()
    );
    assert_eq!(
        captured.hybrid_bus().native_cpu_accesses.as_ref().unwrap()[0],
        host.emulator
            .hybrid_bus()
            .native_cpu_accesses
            .as_ref()
            .unwrap()[0]
    );
}

struct MissingInput;
impl WaitInputPoller for MissingInput {
    fn at(&self, frame: u64) -> crate::Result<genesis_core::GenesisInputs> {
        Err(crate::HostError::Input(format!(
            "authored missing input at {frame}"
        )))
    }
}
#[test]
fn late_missing_input_retains_partial_side_and_no_counterpart_is_fabricated() {
    let mut host = host(actual_wait_twice);
    host.wait_clock.poller = Some(Arc::new(MissingInput));
    host.set_check_capture(true).unwrap();
    let hook = host.registry.at(ROOT).unwrap().clone();
    let error = host.check_hook(&hook).unwrap_err();
    assert!(error.to_string().contains("authored missing input"));
    let diagnostic = host.actual_check_capture().unwrap();
    assert!(matches!(diagnostic.original, CapturedSide::NotExecuted));
    let CapturedSide::Failed(translated) = &diagnostic.translated else {
        panic!("{:?}", diagnostic.translated);
    };
    assert!(!translated.execution_complete);
    assert!(
        translated
            .execution_error
            .as_ref()
            .unwrap()
            .contains("authored missing input")
    );
    assert!(!translated.observations.is_empty());
}

#[test]
fn speculative_capture_clone_has_independent_buffers_and_no_created_frames() {
    let host = host(noop);
    let capture = RawCheckCapture::new(binding(), &host.emulator, host.frame);
    let fork = capture.clone();
    fork.observe_native(
        HybridAccess::Write {
            address: 0xff9000,
            word: false,
            value: 1,
        },
        AccessPoint::capture(&host.emulator, host.frame),
        false,
    );
    assert!(capture.0.borrow().evidence.observations.is_empty());
    assert_eq!(fork.0.borrow().evidence.observations.len(), 1);
    assert!(fork.0.borrow().evidence.exceptions.is_empty());
}

#[test]
fn reset_retains_optin_but_never_relabels_an_old_capture_as_a_new_run() {
    let mut host = host(noop);
    host.set_check_capture(true).unwrap();
    let hook = host.registry.at(ROOT).unwrap().clone();
    host.check_hook(&hook).unwrap_err();
    assert!(host.actual_check_capture().is_some());
    host.reset();
    assert!(host.checks.actual_capture_enabled);
    assert!(host.actual_check_capture().is_none());
}

fn direct_capture(host: &mut crate::Host) -> RawCheckCapture {
    let bus = host.emulator.hybrid_bus_mut();
    bus.hybrid_cpu_accesses = Some(Vec::new());
    bus.hybrid_exception_boundaries = Some(Vec::new());
    bus.hybrid_dma_all_sources = true;
    bus.native_cpu_accesses = Some(Vec::new());
    bus.native_ram_writes = Some(Vec::new());
    bus.enable_native_write_trace();
    host.emulator
        .hybrid_parts()
        .0
        .hybrid_enable_instruction_counts();
    let capture = RawCheckCapture::new(binding(), &host.emulator, host.frame);
    capture.mark_native_journal_scope();
    capture
}

fn observed_tick(
    host: &mut crate::Host,
    capture: &RawCheckCapture,
    audio: &mut crate::sinks::HeadlessAudio,
) -> bool {
    capture.begin_cpu_step(&host.emulator, host.frame);
    let copied_stack = capture
        .0
        .borrow()
        .step
        .as_ref()
        .unwrap()
        .before_stack
        .is_some();
    let effect = host
        .emulator
        .tick(
            &mut crate::sinks::HeadlessOutput,
            audio,
            &mut Inputs(Default::default()),
            &mut NoCartridgeSaves,
        )
        .unwrap();
    capture.finish_reference_cpu_step(&host.emulator, host.frame);
    capture.observe_cpu_batch_parts(
        host.emulator.hybrid_cpu(),
        host.emulator.hybrid_bus(),
        host.frame,
    );
    host.emulator
        .hybrid_bus_mut()
        .hybrid_cpu_accesses
        .as_mut()
        .unwrap()
        .clear();
    host.emulator
        .hybrid_bus_mut()
        .hybrid_exception_boundaries
        .as_mut()
        .unwrap()
        .clear();
    if effect == TickEffect::FrameRendered {
        host.frame += 1;
    }
    copied_stack
}

#[test]
fn disappearing_device_line_uses_fresh_latched_irq_snapshot_not_recognition_image() {
    let mut host = host(noop);
    let capture = direct_capture(&mut host);
    let mut audio = crate::sinks::HeadlessAudio::default();
    for _ in 0..100_000 {
        observed_tick(&mut host, &capture, &mut audio);
        if host.cpu().hybrid_state_words()[20] != 0 && host.cpu().hybrid_exception_depth() == 0 {
            break;
        }
    }
    assert_eq!(host.cpu().hybrid_state_words()[20], 6);
    let ssp = host.cpu().supervisor_stack_pointer();
    host.emulator.hybrid_bus_mut().acknowledge_interrupt(6);
    assert_eq!(host.emulator.hybrid_bus().interrupt_level(), 0);
    let changed = host
        .emulator
        .hybrid_bus()
        .memory
        .read_main_ram_byte(ssp - 1)
        ^ 0x5a;
    host.emulator
        .hybrid_bus_mut()
        .memory
        .write_main_ram_byte(ssp - 1, changed);
    capture.observe_native(
        HybridAccess::HostMutation {
            address: ssp - 1,
            word: false,
            value: u16::from(changed),
        },
        AccessPoint::capture(&host.emulator, host.frame),
        false,
    );
    for _ in 0..100_000 {
        observed_tick(&mut host, &capture, &mut audio);
        if capture
            .0
            .borrow()
            .evidence
            .exceptions
            .first()
            .is_some_and(|irq| irq.returned.is_some())
        {
            break;
        }
    }
    let state = capture.0.borrow();
    let irq = &state.evidence.exceptions[0];
    assert!(irq.returned.is_some());
    assert!(irq.issues.is_empty(), "{:?}", irq.issues);
    let before = irq.before_frame.as_ref().unwrap();
    assert_eq!(
        before.bytes[(canonical_ram_address(ssp - 1) - before.start) as usize],
        changed
    );
    assert_eq!(
        before.checkpoint.master_clock,
        irq.interrupted.unwrap().master_clock
    );
    assert_eq!(before.checkpoint.words[20], 6);
}

#[test]
fn eligible_irq_during_real_dma_halt_does_not_copy_on_every_halted_tick() {
    let mut host = host(noop);
    host.emulator.hybrid_parts().0.set_status_register(0x2700);
    for _ in 0..100_000 {
        if host.emulator.hybrid_bus().interrupt_level() == 6 {
            break;
        }
        if host
            .tick_live_original(&mut Inputs(Default::default()))
            .unwrap()
            == TickEffect::FrameRendered
        {
            host.finish_live_frame().unwrap();
        }
    }
    assert_eq!(host.emulator.hybrid_bus().interrupt_level(), 6);
    for command in [
        0x8134, 0x8f02, 0x9300, 0x9408, 0x9500, 0x9600, 0x9700, 0x4000, 0x0080,
    ] {
        host.emulator.hybrid_bus_mut().write_word(0xc00004, command);
        host.emulator.hybrid_bus_mut().apply_pending_writes();
    }
    assert!(host.emulator.hybrid_bus().halt());
    host.emulator.hybrid_parts().0.set_status_register(0x2000);
    let capture = direct_capture(&mut host);
    let mut audio = crate::sinks::HeadlessAudio::default();
    let mut halted_ticks = 0;
    for _ in 0..100_000 {
        let halted = host.emulator.hybrid_bus().halt();
        let copied = observed_tick(&mut host, &capture, &mut audio);
        if halted {
            halted_ticks += 1;
            assert!(!copied);
        }
        if capture
            .0
            .borrow()
            .evidence
            .exceptions
            .first()
            .is_some_and(|irq| irq.returned.is_some())
        {
            break;
        }
    }
    assert!(halted_ticks > 1);
    let state = capture.0.borrow();
    let irq = &state.evidence.exceptions[0];
    assert!(irq.returned.is_some());
    assert!(irq.before_frame.is_some());
    assert!(irq.after_frame.is_some());
    assert!(state.evidence.observations.iter().any(|event| matches!(event, Observation::Access(access) if matches!(access.actor, AccessActor::VdpDma))));
}

#[test]
fn missing_boundary_payload_or_pre_snapshot_is_unknown_not_a_zero_irq_claim() {
    let host = host(noop);
    let capture = RawCheckCapture::new(binding(), &host.emulator, host.frame);
    let before = bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap();
    capture.observe_stream(
        HybridAccess::ExceptionBoundary { index: 1 },
        &[],
        CpuCheckpoint::capture(&host.emulator, host.frame),
    );
    assert!(capture.0.borrow().evidence.exceptions.is_empty());
    assert!(!capture.0.borrow().evidence.issues.is_empty());
    // Deliberately incomplete synthetic marker: negative schema/integrity test,
    // never an actual IRQ or a proof fixture.
    let payload = [HybridExceptionBoundary {
        master_clock: 7,
        boundary: ExceptionBoundary::Entered {
            vector: 30,
            handler: 0x400,
            depth: 1,
        },
    }];
    capture.observe_stream(
        HybridAccess::ExceptionBoundary { index: 0 },
        &payload,
        CpuCheckpoint::capture(&host.emulator, host.frame),
    );
    let state = capture.0.borrow();
    assert_eq!(state.evidence.exceptions.len(), 1);
    assert!(state.evidence.exceptions[0].interrupted.is_none());
    assert!(state.evidence.exceptions[0].before_frame.is_none());
    assert!(!state.evidence.exceptions[0].issues.is_empty());
    assert_eq!(
        bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap(),
        before
    );
}

#[test]
fn failed_architectural_rte_is_recorded_as_an_attempt_not_a_successful_return() {
    let mut host = host(noop);
    let (cpu, bus) = host.emulator.hybrid_parts();
    cpu.set_status_register(0);
    cpu.set_pc(0x40e, bus); // Actual authored RTE, now attempted in user mode.
    let capture = direct_capture(&mut host);
    let mut audio = crate::sinks::HeadlessAudio::default();
    for _ in 0..100 {
        observed_tick(&mut host, &capture, &mut audio);
        if capture
            .0
            .borrow()
            .evidence
            .observations
            .iter()
            .any(|event| {
                matches!(
                    event, Observation::Instruction(observation)
                        if observation.instruction.opcode == 0x4e73
                            && !observation.instruction.completed_successfully
                )
            })
        {
            break;
        }
    }
    let state = capture.0.borrow();
    assert!(state.evidence.observations.iter().any(|event| matches!(
        event, Observation::Instruction(observation)
            if observation.instruction.opcode == 0x4e73
                && !observation.instruction.completed_successfully
    )));
    assert!(!state.evidence.observations.iter().any(|event| matches!(
        event, Observation::Boundary(boundary) if boundary.returned
    )));
    assert!(
        state
            .evidence
            .exceptions
            .iter()
            .any(|exception| exception.vector == 8)
    );
}

#[test]
fn failed_wait_restores_prior_trace_owners_without_changing_actual_failure_progress() {
    let mut host = host(actual_wait_twice);
    host.wait_clock.poller = Some(Arc::new(MissingInput));
    let hook = host.registry.at(ROOT).unwrap().clone();
    let bus = host.emulator.hybrid_bus_mut();
    bus.native_cpu_accesses = Some(vec![genesis_core::native::CpuAccess::Read {
        address: 0x1234,
        word: true,
        value: 7,
        purpose: 1,
    }]);
    bus.native_ram_writes = Some(vec![genesis_core::native::RamWrite::new(
        0xff1234, false, 9,
    )]);
    bus.enable_native_write_trace();
    bus.native_writes
        .as_mut()
        .unwrap()
        .push(genesis_core::native::TimedWrite {
            clock: 17,
            origin: genesis_core::native::Origin::Z80,
            pc: 0x20,
            write: genesis_core::native::DeviceWrite::Psg { value: 0x9f },
        });
    let mut ordinary = host.fork();
    let mut captured = host.fork();
    let ordinary_error = host
        .run_check_side(
            &mut ordinary,
            &hook,
            CheckSide::Rust,
            crate::call_clock::ClockRunPurpose::Baseline,
            crate::future_frame_watch::ObservationOrigin::CheckFork,
        )
        .err()
        .unwrap()
        .to_string();
    host.set_check_capture(true).unwrap();
    let captured_error = host
        .run_check_side(
            &mut captured,
            &hook,
            CheckSide::Rust,
            crate::call_clock::ClockRunPurpose::Baseline,
            crate::future_frame_watch::ObservationOrigin::CheckFork,
        )
        .err()
        .unwrap()
        .to_string();
    assert_eq!(ordinary_error, captured_error);
    assert!(captured_error.contains("authored missing input"));
    assert_eq!(
        bincode::encode_to_vec(&ordinary, bincode::config::standard()).unwrap(),
        bincode::encode_to_vec(&captured, bincode::config::standard()).unwrap()
    );
    let diagnostic = host.actual_check_capture().unwrap();
    assert!(matches!(diagnostic.translated, CapturedSide::Failed(_)));
    assert!(matches!(diagnostic.original, CapturedSide::NotExecuted));
}

#[test]
fn execution_error_inside_actual_irq_retains_open_frame_bytes_without_return_fiction() {
    let mut host = host(noop);
    let capture = direct_capture(&mut host);
    let mut audio = crate::sinks::HeadlessAudio::default();
    for _ in 0..100_000 {
        observed_tick(&mut host, &capture, &mut audio);
        if host.cpu().hybrid_exception_depth() != 0 {
            break;
        }
    }
    assert_eq!(host.cpu().hybrid_exception_depth(), 1);
    let before = bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap();
    let evidence = capture.finish(
        &host.emulator,
        host.frame,
        Some("authored mid-IRQ abort".into()),
    );
    assert!(!evidence.execution_complete);
    assert!(
        evidence
            .execution_error
            .as_ref()
            .unwrap()
            .contains("mid-IRQ")
    );
    assert_eq!(evidence.exceptions.len(), 1);
    let irq = &evidence.exceptions[0];
    assert!(irq.returned.is_none());
    assert!(irq.return_master_clock.is_none());
    assert!(irq.before_frame.is_some());
    assert!(irq.after_frame.is_some());
    assert!(irq.issues.iter().any(|issue| issue.contains("open")));
    assert_eq!(
        bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap(),
        before
    );
}

#[test]
fn out_of_known_stack_is_preserved_as_unknown_not_hidden_by_min_ssp_trimming() {
    let mut host = host(noop);
    crate::abi::set_sp(host.emulator.hybrid_parts().0, 0xff9000);
    let capture = direct_capture(&mut host);
    let mut audio = crate::sinks::HeadlessAudio::default();
    for _ in 0..100_000 {
        observed_tick(&mut host, &capture, &mut audio);
        if capture
            .0
            .borrow()
            .evidence
            .exceptions
            .first()
            .is_some_and(|irq| irq.returned.is_some())
        {
            break;
        }
    }
    let state = capture.0.borrow();
    let irq = &state.evidence.exceptions[0];
    assert_eq!(irq.interrupted.unwrap().words[16], 0xff9000);
    assert!(irq.before_frame.is_none());
    assert!(irq.after_frame.is_none());
    assert!(irq.issues.iter().any(|issue| issue.contains("known stack")));
    assert!(state.evidence.observations.iter().any(|event| matches!(
        event, Observation::Access(access)
            if access.writing && (0xff8f00..0xff9000).contains(&access.address)
    )));
}

#[test]
fn active_ram_cheat_never_substitutes_physical_before_after_or_counterpart_images() {
    let mut host = host(noop);
    // This authored MOVEM places D0's high word 22 bytes below interrupted SSP.
    // Changing its logical restored high word cannot change the DBF low-word loop.
    let slot = host.cpu().supervisor_stack_pointer() - 22;
    host.emulator
        .hybrid_bus_mut()
        .memory
        .write_main_ram_word(slot, 0xabcd);
    host.emulator
        .hybrid_bus_mut()
        .memory
        .reload_config(&GenesisEmulatorConfig {
            cheat_codes: vec![(slot, 0x1234)],
            ..GenesisEmulatorConfig::default()
        });
    assert_eq!(
        host.emulator.hybrid_bus().memory.read_main_ram_word(slot),
        0x1234
    );
    assert_eq!(
        physical_ram_byte(
            host.emulator.hybrid_bus().memory.hybrid_raw_main_ram(),
            slot
        ),
        0xab
    );
    host.set_check_capture(true).unwrap();
    let hook = host.registry.at(ROOT).unwrap().clone();
    host.check_hook(&hook).unwrap_err();
    let diagnostic = host.actual_check_capture().unwrap();
    let original = completed(&diagnostic.original);
    let irq = &original.exceptions[0];
    let before = irq.before_frame.as_ref().unwrap();
    let after = irq.after_frame.as_ref().unwrap();
    let counterpart = irq.counterpart_final_frame.as_ref().unwrap();
    let at = |image: &MemorySnapshot| (slot - image.start) as usize;
    assert_eq!(&before.bytes[at(before)..at(before) + 2], &[0xab, 0xcd]);
    let saved = (irq.interrupted.unwrap().words[0] >> 16) as u16;
    assert_eq!(&after.bytes[at(after)..at(after) + 2], &saved.to_be_bytes());
    assert_eq!(
        &counterpart.bytes[at(counterpart)..at(counterpart) + 2],
        &[0xab, 0xcd]
    );
    assert_ne!(saved, 0x1234);
    assert!(
        original
            .bus_journals
            .ram_commits
            .as_ref()
            .unwrap()
            .iter()
            .any(|commit| { commit.address == slot && commit.word && commit.value == saved })
    );
    // The actual live machine was not executed or patched by either check fork.
    assert_eq!(
        host.emulator.hybrid_bus().memory.read_main_ram_word(slot),
        0x1234
    );
    assert_eq!(
        physical_ram_byte(
            host.emulator.hybrid_bus().memory.hybrid_raw_main_ram(),
            slot
        ),
        0xab
    );
}
