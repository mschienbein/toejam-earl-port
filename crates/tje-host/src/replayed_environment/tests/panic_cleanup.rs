//! Actual authored root execution; unrelated owned payloads must escape intact.
use super::*;
use std::{
    any::Any,
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
};

type Payload = Box<dyn Any + Send>;
thread_local! {
    static THROW: RefCell<Option<Payload>> = const { RefCell::new(None) };
    static CAPTURED: RefCell<Option<Payload>> = const { RefCell::new(None) };
}
#[derive(Debug)]
struct Marker(u64);
fn panic_body(context: &mut dyn Context, sticky: bool) -> tje_game::Result<Option<u32>> {
    context.write_u8(RESULT, 9);
    if sticky {
        let _owned = catch_unwind(AssertUnwindSafe(|| context.read_u8(VALUE)))
            .expect_err("the unmatched infallible read must abort, never supply an operand");
    }
    resume_unwind(THROW.with(|slot| slot.borrow_mut().take().unwrap()))
}
fn capture_owned_abort(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.write_u8(RESULT, 9);
    let payload = catch_unwind(AssertUnwindSafe(|| context.read_u8(VALUE)))
        .expect_err("no unmatched original read may manufacture an operand");
    assert!(payload.is::<crate::replayed_environment::context::Abort>());
    CAPTURED.with(|slot| *slot.borrow_mut() = Some(payload));
    // Swallowing the private marker cannot swallow its exact first failure.
    Ok(None)
}
static PANIC: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: |ctx| panic_body(ctx, false),
}];
static STICKY_PANIC: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: |ctx| panic_body(ctx, true),
}];
static CAPTURE_ABORT: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: capture_owned_abort,
}];
fn body_code() -> Vec<u16> {
    vec![0x13fc, 9, 0x00ff, 0x9002, 0x4e75]
}
fn machine(revision: Revision, framed: &'static [FramedTranslation]) -> Host {
    host(
        revision,
        &[],
        framed,
        &[ROOT],
        &main_call(revision, false),
        &[(ROOT, body_code())],
    )
}
fn peripheral(host: &Host) -> Vec<u8> {
    let s = host.emulator.capture_native_state();
    bincode::encode_to_vec(
        (
            &s.ram,
            &s.audio_ram,
            &s.vdp,
            &s.ym2612,
            &s.psg,
            &s.input,
            &s.z80,
            &s.cycles,
            &s.audio_resampler,
            &s.config,
            s.z80_bank,
            s.bus_request,
            (
                s.reset,
                s.timing_mode,
                s.region,
                s.native_input_remainder,
                s.open_bus,
            ),
        ),
        bincode::config::standard(),
    )
    .unwrap()
}
fn warm_foreign_journals(host: &mut Host, counting: bool) {
    let (cpu, bus) = host.emulator.hybrid_parts();
    cpu.hybrid_enable_instruction_counts();
    cpu.hybrid_set_instruction_counting(counting);
    bus.native_cpu_accesses = Some(Vec::new());
    bus.hybrid_cpu_accesses = Some(Vec::new());
    bus.hybrid_exception_boundaries = Some(Vec::new());
    bus.reference_read_contexts = Some(Vec::new());
    bus.hybrid_dma_all_sources = false;
    let _ = bus.read_word(VALUE);
}
fn reference_step(host: &mut Host, counting: bool) {
    host.emulator
        .hybrid_parts()
        .0
        .hybrid_set_instruction_counting(true);
    let effect = host.tick(&mut Inputs(GenesisInputs::default())).unwrap();
    host.emulator
        .hybrid_parts()
        .0
        .hybrid_set_instruction_counting(counting);
    if effect == TickEffect::FrameRendered {
        host.finish_live_frame().unwrap();
    }
}
#[test]
fn semantic_unrelated_owned_payload_survives_full_flush_even_after_sticky_failure_and_continue_policy()
 {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for sticky in [false, true] {
            for counting in [false, true] {
                for policy in [
                    CheckFailurePolicy::FailFast,
                    CheckFailurePolicy::ContinueOriginal,
                ] {
                    let mut checked = machine(revision, if sticky { STICKY_PANIC } else { PANIC });
                    let mut reference = machine(revision, LITERAL);
                    checked.check_failure_policy = policy;
                    warm_foreign_journals(&mut checked, counting);
                    warm_foreign_journals(&mut reference, counting);
                    let prefix_native = checked
                        .emulator
                        .hybrid_bus()
                        .native_cpu_accesses
                        .clone()
                        .unwrap();
                    let prefix_hybrid = checked
                        .emulator
                        .hybrid_bus()
                        .hybrid_cpu_accesses
                        .clone()
                        .unwrap();
                    let prefix_context = checked
                        .emulator
                        .hybrid_bus()
                        .reference_read_contexts
                        .clone()
                        .unwrap();
                    assert!(!prefix_hybrid.is_empty() && !prefix_context.is_empty());
                    let observer = format!("{:?}", checked.future_observer);
                    let marker = Box::new(Marker(0x1122_3344_5566_7788));
                    let identity = std::ptr::from_ref(marker.as_ref());
                    THROW.with(|slot| *slot.borrow_mut() = Some(marker));
                    let mut caught = None;
                    for _ in 0..10000 {
                        let result = catch_unwind(AssertUnwindSafe(|| step(&mut checked)));
                        reference_step(&mut reference, counting);
                        match result {
                            Err(payload) => {
                                caught = Some(payload);
                                break;
                            }
                            Ok(result) => result.unwrap(),
                        }
                    }
                    let payload =
                        caught.expect("actual readable analysis must resume unrelated unwind");
                    let Ok(marker) = payload.downcast::<Marker>() else {
                        panic!("owned payload type changed");
                    };
                    assert!(std::ptr::eq(marker.as_ref(), identity));
                    assert_eq!(marker.0, 0x1122_3344_5566_7788);
                    assert_eq!(
                        checked.emulator.hybrid_cpu().hybrid_state_words(),
                        reference.emulator.hybrid_cpu().hybrid_state_words()
                    );
                    assert_eq!(
                        checked.emulator.hybrid_bus().memory.hybrid_raw_main_ram(),
                        reference.emulator.hybrid_bus().memory.hybrid_raw_main_ram()
                    );
                    assert_eq!(peripheral(&checked), peripheral(&reference));
                    assert_eq!(
                        checked.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
                        reference.emulator.hybrid_bus().cycles.m68k_mclk_cycles
                    );
                    assert_eq!(checked.frame, reference.frame);
                    assert_eq!(format!("{:?}", checked.future_observer), observer);
                    assert_eq!(
                        checked.cpu().hybrid_instruction_counts().unwrap().enabled(),
                        counting
                    );
                    let bus = checked.emulator.hybrid_bus();
                    assert!(
                        bus.native_cpu_accesses
                            .as_ref()
                            .unwrap()
                            .starts_with(&prefix_native)
                    );
                    assert!(
                        bus.hybrid_cpu_accesses
                            .as_ref()
                            .unwrap()
                            .starts_with(&prefix_hybrid)
                    );
                    assert!(
                        bus.reference_read_contexts
                            .as_ref()
                            .unwrap()
                            .starts_with(&prefix_context)
                    );
                    let expected = reference.emulator.hybrid_bus();
                    assert_eq!(
                        bus.native_cpu_accesses, expected.native_cpu_accesses,
                        "all real original suffix events retained; analysis emits no live events"
                    );
                    assert_eq!(bus.hybrid_cpu_accesses, expected.hybrid_cpu_accesses);
                    assert_eq!(
                        bus.reference_read_contexts,
                        expected.reference_read_contexts
                    );
                    assert!(bus.hybrid_exception_boundaries.as_ref().unwrap().is_empty());
                    assert!(!bus.hybrid_dma_all_sources);
                    let state = checked
                        .replayed_state
                        .as_ref()
                        .expect("private replay state restored before resume");
                    assert!(state.failed && state.proofs.is_empty());
                    let report = checked.checks.replayed_environment.as_ref().unwrap();
                    assert_eq!((report.own_matches, report.completed_proofs), (0, 0));
                    assert_eq!(
                        report.failures[0].verdict,
                        if sticky {
                            Verdict::Mismatch
                        } else {
                            Verdict::Unsupported
                        },
                        "sticky first cause is accounted separately, never permission to suppress another payload"
                    );
                    assert!(checked.finalize_replayed_campaign().is_err());
                }
            }
        }
    }
}
#[test]
fn caught_owned_abort_remains_failed_and_foreign_context_abort_is_not_converted_by_type() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut first = machine(revision, CAPTURE_ABORT);
        assert!(finish_one(&mut first).is_err());
        let foreign = CAPTURED.with(|slot| slot.borrow_mut().take().unwrap());
        let identity = std::ptr::from_ref(foreign.as_ref()) as *const ();
        THROW.with(|slot| *slot.borrow_mut() = Some(foreign));
        let mut second = machine(revision, PANIC);
        let caught = catch_unwind(AssertUnwindSafe(|| finish_one(&mut second)))
            .expect_err("foreign private Abort is an unrelated owned payload");
        assert!(caught.is::<crate::replayed_environment::context::Abort>());
        assert_eq!(std::ptr::from_ref(caught.as_ref()) as *const (), identity);
        assert!(second.replayed_state.as_ref().unwrap().failed);
        assert_eq!(
            second
                .checks
                .replayed_environment
                .as_ref()
                .unwrap()
                .completed_proofs,
            0
        );
        assert!(second.finalize_replayed_campaign().is_err());
        assert!(first.finalize_replayed_campaign().is_err());
    }
}

fn rom_identity(analysis: &mut genesis_core::GenesisEmulator) -> (*const u16, usize) {
    let rom = analysis
        .hybrid_bus_mut()
        .cartridge
        .as_mut()
        .unwrap()
        .debug_rom_view();
    (rom.as_ptr(), rom.len())
}
fn analysis_asset(host: &mut Host) -> (*const u16, usize) {
    let mut fork = host.fork();
    host.execute_fork(&mut fork, |fork| Ok(rom_identity(fork)))
        .unwrap()
}
fn live_rom(host: &Host) -> Vec<u16> {
    let cartridge = host.emulator.hybrid_bus().cartridge.as_ref().unwrap();
    (0..cartridge.rom_len() as u32)
        .step_by(2)
        .map(|address| cartridge.peek_word(address))
        .collect()
}
#[test]
fn strict_real_admitted_child_fault_seam_restores_loans_and_exact_payload_without_canonical_body_claim()
 {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for unwind in [false, true] {
            let (mut host, completed) = actual_strict_child(revision);
            let mut root = completed.root;
            let before_cpu = host.emulator.hybrid_cpu().hybrid_state_words();
            let before_peripheral = peripheral(&host);
            let allocation = analysis_asset(&mut host);
            let before_rom = live_rom(&host);
            let before_live =
                bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap();
            let before_observer = format!("{:?}", host.future_observer);
            let before_checks = serde_json::to_value(&host.checks).unwrap();
            let before_measurements = format!("{:?}", host.measurements);
            let marker = Box::new(Marker(0x6655_4433_2211));
            let identity = std::ptr::from_ref(marker.as_ref());
            let result = catch_unwind(AssertUnwindSafe(|| {
                host.with_strict_child_analysis(&mut root, |analysis, hook| {
                    assert_eq!(hook.symbol.id, ids::ABS);
                    assert_eq!(
                        rom_identity(&mut analysis.emulator),
                        allocation,
                        "one leased analysis asset, not a per-child copy"
                    );
                    let mut nested_fork = analysis.fork();
                    analysis.execute_fork(&mut nested_fork, |fork| {
                        assert_eq!(rom_identity(fork), allocation);
                        Ok(())
                    })?;
                    assert_eq!(rom_identity(&mut analysis.emulator), allocation);
                    let (cpu, bus) = analysis.emulator.hybrid_parts();
                    let mut d = cpu.data_registers();
                    d[0] = 0x12345678;
                    cpu.set_data_registers(d);
                    bus.memory.write_main_ram_byte(VALUE, 0x5a);
                    let nested = analysis.begin_function_proof_capture().unwrap();
                    analysis.finish_function_proof_capture(nested).unwrap();
                    if unwind {
                        resume_unwind(marker);
                    }
                    Err(crate::HostError::Metadata(
                        "authored strict child partial error".into(),
                    ))
                })
            }));
            if unwind {
                let payload = match result {
                    Err(payload) => payload,
                    Ok(_) => panic!("strict lease must resume unrelated payload"),
                };
                let Ok(marker) = payload.downcast::<Marker>() else {
                    panic!("strict payload type changed");
                };
                assert!(std::ptr::eq(marker.as_ref(), identity));
                assert_eq!(marker.0, 0x6655_4433_2211);
            } else {
                assert!(matches!(result, Ok(Err(crate::HostError::Metadata(detail)))
                    if detail == "authored strict child partial error"));
            }
            assert_eq!(
                analysis_asset(&mut host),
                allocation,
                "exact cached analysis asset reclaimed"
            );
            assert_eq!(
                live_rom(&host),
                before_rom,
                "all live ROM words observed immutably"
            );
            assert_eq!(
                bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap(),
                before_live,
                "live emulation/source state is untouched, never restored"
            );
            assert_eq!(host.emulator.hybrid_cpu().hybrid_state_words(), before_cpu);
            assert_eq!(peripheral(&host), before_peripheral);
            assert_eq!(format!("{:?}", host.future_observer), before_observer);
            assert_eq!(serde_json::to_value(&host.checks).unwrap(), before_checks);
            assert_eq!(format!("{:?}", host.measurements), before_measurements);
            let analysis = root
                .analysis_entry
                .as_mut()
                .expect("analysis emulator loan restored");
            assert_eq!(analysis.hybrid_cpu().data_registers()[0], 0x12345678);
            assert_eq!(
                analysis.hybrid_bus().memory.read_main_ram_byte(VALUE),
                0x5a,
                "fault effects remain on untrusted fork, never snapshot-restored"
            );
            assert_eq!(
                analysis.hybrid_bus().cartridge.as_ref().unwrap().rom_len(),
                0
            );
            let state = host.replayed_state.as_ref().unwrap();
            assert!(state.proofs.is_empty());
            assert_eq!(
                host.checks
                    .replayed_environment
                    .as_ref()
                    .unwrap()
                    .completed_proofs,
                0
            );
            // Callback injection occurred inside a real admitted child's
            // lease/capture, NOT inside its canonical production body. The
            // separate semantic control exercises the full top-level flush.
        }
    }
}
