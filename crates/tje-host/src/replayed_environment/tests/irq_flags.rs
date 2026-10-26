//! Authored actual IRQs: preservation is not a saved-Original-SR value oracle.
use super::*;
use crate::replayed_environment::{inputs::ExceptionFrame, record};
use std::sync::Arc;

fn own_flags(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let incoming = context.status_register();
    context.set_status_register((incoming & !0x1f) | 4);
    context.wait_until(WaitRequest::ram_byte_equal(FLAG, 255, 0))?;
    // The terminal source TST sees zero. At the intervening IRQ the Original
    // busy TST had Z=0, while this Rust-owned scratch Z was 1 throughout.
    let own = context.status_register() & 4;
    context.write_u16(RESULT, own);
    Ok(None)
}
static FLAGS: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: own_flags,
}];
fn code() -> Vec<u16> {
    vec![
        0x4a39, 0x00ff, 0x9001, 0x66f8, 0x40c0, 0x0240, 4, 0x33c0, 0x00ff, 0x9002, 0x4e75,
    ]
}
fn tail() -> Vec<u16> {
    vec![0x13fc, 0, 0x00ff, 0x9001, 0x5239, 0x00ff, 0x9004, 0x4e73]
}
fn setup(revision: Revision, handler: &[u16]) -> Host {
    let mut host = host_with_handler(
        revision,
        &[],
        FLAGS,
        &[ROOT],
        &main_call(revision, true),
        &[(ROOT, code())],
        handler,
    );
    host.context().write_u8(FLAG, 1);
    let bus = host.emulator.hybrid_bus_mut();
    bus.write_word(0xc00004, 0x8010);
    bus.write_word(0xc00004, 0x8a00);
    bus.apply_pending_writes();
    host
}
#[test]
fn certified_own_rte_preserves_rust_scratch_flags_without_importing_original_saved_sr() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = setup(revision, &tail());
        finish_one(&mut host).unwrap();
        host.finalize_replayed_campaign().unwrap();
        assert_eq!(host.context().read_u16(RESULT), 4);
        assert_ne!(host.context().read_u8(IRQ_VISIBLE), 0);
        let inputs = &host.replayed_state.as_ref().unwrap().handler_inputs;
        assert!(
            inputs
                .values()
                .all(|mask| mask.get(tje_engine_core::ir::StateField::Sr) & 4 == 0),
            "unread saved Z is identity transport, not an input/value import"
        );
        assert_eq!(
            host.checks
                .replayed_environment
                .as_ref()
                .unwrap()
                .completed_proofs,
            1
        );
    }
}
#[test]
fn actual_entry_flag_branch_and_software_saved_sr_algorithm_remain_authenticated() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for prefix in [
            vec![0x6602, 0x4e71],
            vec![0x40c0, 0x0240, 4, 0x33c0, 0x00ff, 0x9006],
            vec![0x3017, 0x0240, 4, 0x33c0, 0x00ff, 0x9006],
        ] {
            let mut handler = prefix;
            handler.extend(tail());
            let mut host = setup(revision, &handler);
            let error = finish_one(&mut host).unwrap_err();
            assert!(
                matches!(&error, crate::HostError::Mismatch { .. })
                    && error.to_string().contains("consumes Sr"),
                "{revision:?}: {error}"
            );
            assert!(host.finalize_replayed_campaign().is_err());
            assert_eq!(
                host.checks
                    .replayed_environment
                    .as_ref()
                    .unwrap()
                    .completed_proofs,
                0
            );
        }
    }
}
#[test]
fn actual_same_value_software_header_rewrite_is_not_hardware_identity_transport() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut handler = vec![0x3e97]; // MOVE.W (SP),(SP), same actual word.
        handler.extend(tail());
        let mut host = setup(revision, &handler);
        let error = finish_one(&mut host).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("untouched actual hardware-save identity"),
            "{error}"
        );
        assert!(host.finalize_replayed_campaign().is_err());
        assert_eq!(
            host.checks
                .replayed_environment
                .as_ref()
                .unwrap()
                .completed_proofs,
            0
        );
    }
}

fn captured_root(revision: Revision, handler: &[u16]) -> (Host, record::Invocation) {
    let mut host = setup(revision, handler);
    let mut state = host.replayed_state.take().unwrap();
    let registry = host.registry.clone();
    for _ in 0..100000 {
        state
            .before_tick(
                &mut host.emulator,
                host.frame,
                host.checks.replayed_environment.as_mut().unwrap(),
            )
            .unwrap();
        let effect = host.tick(&mut Inputs(GenesisInputs::default())).unwrap();
        let rendered = effect == TickEffect::FrameRendered;
        state.after_tick(
            &mut host.emulator,
            &registry,
            &std::collections::BTreeSet::new(),
            rendered,
            GenesisInputs::default(),
            host.checks.replayed_environment.as_mut().unwrap(),
        );
        if rendered {
            host.frame += 1;
        }
        if let Some(error) = state.take_error() {
            panic!("actual capture failed: {error}");
        }
        if let Some(completed) = state.next_completed() {
            assert_eq!(completed.original.entry.function, Some(ROOT));
            return (host, Arc::try_unwrap(completed.original).unwrap());
        }
    }
    panic!("authored IRQ did not finish");
}
fn captured_irq(revision: Revision) -> record::Invocation {
    let (_, root) = captured_root(revision, &tail());
    let irq = root
        .trace
        .events
        .into_iter()
        .find_map(|event| match event {
            record::Event::Interrupt(irq) => Some(irq),
            _ => None,
        })
        .expect("actual IRQ required");
    Arc::try_unwrap(irq).unwrap()
}

#[test]
fn frozen_irq_guard_separates_certified_alignment_from_real_register_and_stack_values() {
    use crate::replayed_environment::inputs::{
        CompilerCache, frozen_interrupt_guard, interrupt_inputs,
    };
    use tje_engine_core::{dependencies::StateMask, ir::StateField};
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (prefix, supported) in [
            (vec![], true),
            (vec![0x6602, 0x4e71], false), // genuine incoming Z branch
            (vec![0x13c0, 0x00ff, 0x9006], false), // genuine incoming D0
            (vec![0x200f, 0x23c0, 0x00ff, 0x9006], false), // whole SP as data
            (vec![0x3017, 0x0240, 4, 0x33c0, 0x00ff, 0x9006], false), // saved SR as data
            (vec![0x3e97], false),         // same-value software header rewrite
        ] {
            let mut handler = prefix;
            handler.extend(tail());
            let (host, root) = captured_root(revision, &handler);
            let reader = |pc| {
                Some(
                    host.emulator
                        .hybrid_bus()
                        .cartridge
                        .as_ref()
                        .unwrap()
                        .peek_word(pc),
                )
            };
            let mut compiler = CompilerCache::default();
            let result = frozen_interrupt_guard(&root, &reader, 7, &mut compiler);
            assert_eq!(
                result.is_ok(),
                supported,
                "{revision:?} handler {handler:04x?}"
            );
            if supported {
                let irq = root
                    .trace
                    .events
                    .iter()
                    .find_map(|event| match event {
                        record::Event::Interrupt(irq) => Some(irq),
                        _ => None,
                    })
                    .expect("actual accepted IRQ");
                let consumed = interrupt_inputs(irq, &reader, 7, &mut compiler).unwrap();
                assert_eq!(consumed.registers, StateMask::single(StateField::Ssp, 1));
                assert_eq!(irq.entry.boundary.cpu.words[16] & 1, 0);
            }
            assert_eq!(
                host.checks
                    .replayed_environment
                    .as_ref()
                    .unwrap()
                    .completed_proofs,
                0,
                "actual capture/input analysis is not a discharged strict-child proof"
            );
        }
    }
}
fn entry(original: &record::Invocation) -> (usize, &record::IdleStep) {
    let Some(record::Event::Idle(index)) = original.trace.events.first() else {
        panic!("entry idle required");
    };
    (*index, &original.trace.idles[*index])
}
fn rte(original: &record::Invocation) -> usize {
    let Some(record::Event::Instruction(index)) = original.trace.events.last() else {
        panic!("own RTE required");
    };
    *index
}
#[test]
fn hardware_identity_mechanism_rejects_foreign_ids_banks_depth_and_changed_raw_header_spans() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let foreign = captured_irq(revision).entry.evidence;
        for variant in 0..7 {
            let mut original = captured_irq(revision);
            if variant < 3 {
                match variant {
                    0 => original.completion_evidence = foreign,
                    1 => original.exit.cpu.words[15] ^= 4,
                    2 => original.exit.cpu.exception_depth += 1,
                    _ => unreachable!(),
                }
                let (index, idle) = entry(&original);
                assert!(ExceptionFrame::capture(&original, index, idle).is_err());
                continue;
            }
            let (index, idle) = entry(&original);
            let mut frame = ExceptionFrame::capture(&original, index, idle).unwrap();
            let index = rte(&original);
            if variant == 3 {
                let issues = original.trace.instructions[index].issues.clone();
                let reads: Vec<_> = issues
                    .filter(|&issue| {
                        original.trace.issues[issue].direction
                            == record::Direction::Read(m68000_emu::ReadPurpose::Semantic)
                    })
                    .collect();
                original.trace.issues.swap(reads[0], reads[1]);
            } else if variant == 4 {
                original.trace.instructions[index].after_sr ^= 4;
            } else if variant == 5 {
                let address = original.entry.boundary.cpu.sp();
                // Generic nonmain writer ingress model, NOT physical DMA-to-
                // main-RAM execution (the Genesis VDP cannot perform that).
                frame.observe_write(record::Access {
                    address,
                    word: true,
                    direction: record::Direction::Write,
                    value: entry(&original).1.before.cpu.sr() as u16,
                });
            } else {
                let (idle_index, idle) = entry(&original);
                let writes: Vec<_> = idle
                    .issues
                    .clone()
                    .filter(|&issue| {
                        original.trace.issues[issue].direction == record::Direction::Write
                    })
                    .collect();
                original.trace.issues.swap(writes[0], writes[1]);
                assert!(
                    ExceptionFrame::capture(
                        &original,
                        idle_index,
                        &original.trace.idles[idle_index]
                    )
                    .is_err()
                );
                continue;
            }
            assert!(
                frame
                    .begin_return(&original, index, &original.trace.instructions[index])
                    .is_err()
            );
        }
    }
}

#[test]
fn nested_actual_irqs_each_use_their_own_hardware_save_and_matching_rte() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut rom = vec![0; 0x100000];
        rom[..4].copy_from_slice(&crate::catalogue::STACK_TOP.to_be_bytes());
        rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
        rom[28 * 4..28 * 4 + 4].copy_from_slice(&0x80000_u32.to_be_bytes());
        rom[30 * 4..30 * 4 + 4].copy_from_slice(&0x81000_u32.to_be_bytes());
        install(&mut rom, 0x100, &main_call(revision, true));
        install(&mut rom, ROOT.metadata().address(revision), &code());
        // Preserve D0, delay across the real VBlank while IRQ4 is active, then
        // restore it. The short IRQ6 handler cannot starve the interrupted IRQ4.
        let mut line = vec![
            0x48e7, 0x8000,
            // Disable further IRQ4 requests, not IRQ6, to avoid an intentional
            // every-line interrupt storm starving the real caller afterward.
            0x33fc, 0x8000, 0x00c0, 4, 0x303c, 0x37ff, 0x51c8, 0xfffe, 0x4cdf, 1,
        ];
        line.extend(tail());
        install(&mut rom, 0x80000, &line);
        install(&mut rom, 0x81000, &[0x5239, 0x00ff, 0x9004, 0x4e73]);
        let mut host = crate::fixture::tests::synthetic_host_from_rom(rom);
        host.revision = revision;
        host.symbols = crate::catalogue::RevisionSymbols::for_revision(revision);
        host.registry =
            HookRegistry::proof_replayed_environment(revision, &[], FLAGS, &[ROOT]).unwrap();
        host.mode = Mode::Check;
        host.wait_clock.revision = Some(revision);
        host.checks = CheckReport::registered(&host.registry);
        let (state, report) = State::new(&host.registry, 0x100000, 7).unwrap();
        host.replayed_state = Some(state);
        host.checks.replayed_environment = Some(report);
        host.context().write_u8(FLAG, 1);
        let bus = host.emulator.hybrid_bus_mut();
        for value in [0x8010, 0x8164, 0x8a00] {
            bus.write_word(0xc00004, value);
        }
        bus.apply_pending_writes();
        let registry = host.registry.clone();
        let mut saw_nested = false;
        for _ in 0..100000 {
            let mut state = host.replayed_state.take().unwrap();
            state
                .before_tick(
                    &mut host.emulator,
                    host.frame,
                    host.checks.replayed_environment.as_mut().unwrap(),
                )
                .unwrap();
            let effect = host.tick(&mut Inputs(GenesisInputs::default())).unwrap();
            let rendered = effect == TickEffect::FrameRendered;
            state.after_tick(
                &mut host.emulator,
                &registry,
                &std::collections::BTreeSet::new(),
                rendered,
                GenesisInputs::default(),
                host.checks.replayed_environment.as_mut().unwrap(),
            );
            if rendered {
                host.frame += 1;
            }
            if let Some(error) = state.take_error() {
                panic!("actual nested capture failed: {error}");
            }
            if let Some(completed) = state.peek_completed() {
                assert_eq!(completed.original.entry.function, Some(ROOT));
                saw_nested = completed.original.trace.events.iter().any(|event| {
                    let record::Event::Interrupt(line) = event else { return false; };
                    matches!(line.entry.kind, record::EntryKind::Interrupt { vector: 28 })
                        && line.trace.events.iter().any(|event|
                            matches!(event, record::Event::Interrupt(vertical)
                                if matches!(vertical.entry.kind, record::EntryKind::Interrupt { vector: 30 })
                                    && vertical.entry.boundary.cpu.exception_depth == 2))
                });
            }
            host.replayed_state = Some(state);
            if saw_nested {
                host.flush_completed_replays().unwrap();
                break;
            }
        }
        assert!(
            saw_nested,
            "the actual sealed IRQ4→IRQ6 tree, not a count/diagnostic key, proves nesting"
        );
        host.finalize_replayed_campaign().unwrap();
        assert_eq!(host.context().read_u16(RESULT), 4);
    }
}

#[test]
fn header_identity_span_ends_at_own_actual_return_not_a_later_writer() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let original = captured_irq(revision);
        let (entry_index, idle) = entry(&original);
        let index = rte(&original);
        let access = record::Access {
            address: original.entry.boundary.cpu.sp(),
            word: true,
            direction: record::Direction::Write,
            value: idle.before.cpu.sr() as u16,
        };
        let mut frame = ExceptionFrame::capture(&original, entry_index, idle).unwrap();
        frame
            .begin_return(&original, index, &original.trace.instructions[index])
            .unwrap();
        frame
            .finish_return(idle.before.cpu.exception_depth)
            .unwrap();
        // Generic later writer ingress, not a forged physical DMA-to-RAM case.
        frame.observe_write(access);
        assert!(
            frame.finishes(&original),
            "a later owner cannot retroactively alter the completed read span"
        );
        let mut interrupted = ExceptionFrame::capture(&original, entry_index, idle).unwrap();
        interrupted
            .begin_return(&original, index, &original.trace.instructions[index])
            .unwrap();
        interrupted.observe_write(access);
        assert!(
            interrupted
                .finish_return(idle.before.cpu.exception_depth)
                .is_err(),
            "the same-value writer BEFORE the actual return still loses identity"
        );
    }
}
