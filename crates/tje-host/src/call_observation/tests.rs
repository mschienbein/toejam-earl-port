use super::*;
use crate::{fixture::tests::synthetic_host, sinks::Inputs};
use genesis_config::GenesisInputs;
use jgenesis_common::frontend::TickEffect;

fn host_program() -> Host {
    let mut words = vec![0x4eb9, 0, 0x120, 0x60fe]; // JSR120; BRA106
    words.resize(16, 0x4e71);
    words.extend([0x4e71, 0x4e75]); // NOP; RTS
    synthetic_host(&words, &[0x4e73])
}
#[test]
fn interval_starts_after_actual_call_and_ends_after_matching_rts() {
    let mut host = host_program();
    host.enable_session_trace(&[0x120], &[], &[], false)
        .unwrap();
    host.enable_call_observations(&[0x120]).unwrap();
    let mut observed = Vec::new();
    for _ in 0..100 {
        assert_eq!(
            host.tick_live_original(&mut Inputs(GenesisInputs::default()))
                .unwrap(),
            TickEffect::None
        );
        observed.extend(host.take_call_observations());
        if !observed.is_empty() {
            break;
        }
    }
    assert_eq!(observed.len(), 1);
    let call = &observed[0];
    assert_eq!(
        (call.caller_pc, call.entry_pc, call.return_pc, call.rts_pc),
        (0x100, 0x120, 0x106, 0x122)
    );
    assert_eq!(call.return_sp, call.entry_sp + 4);
    assert_eq!(call.usage.main_thread_instructions, 2); // excludes caller JSR
    assert_eq!(call.usage.exception_handler_instructions, 0);
    assert_eq!(call.usage.irq_entries_by_level, [0; 8]);
    assert!(call.usage.ticks >= 2);
    assert!(call.usage.master_clocks >= 20 * 7);
    assert_eq!(host.unfinished_observed_calls(), 0);
}
#[test]
fn observer_changes_neither_original_machine_bytes_nor_completion() {
    let mut observed = host_program();
    let mut ordinary = host_program();
    observed
        .enable_session_trace(&[0x120], &[], &[], false)
        .unwrap();
    observed.enable_call_observations(&[0x120]).unwrap();
    for _ in 0..30 {
        observed
            .tick_live_original(&mut Inputs(GenesisInputs::default()))
            .unwrap();
        ordinary
            .tick_live_original(&mut Inputs(GenesisInputs::default()))
            .unwrap();
        assert_eq!(
            bincode::encode_to_vec(&observed.emulator, bincode::config::standard()).unwrap(),
            bincode::encode_to_vec(&ordinary.emulator, bincode::config::standard()).unwrap()
        );
    }
    assert_eq!(observed.take_call_observations().len(), 1);
}
fn point(pc: u32, sp: u32, exception_depth: u32) -> InvocationPoint {
    InvocationPoint {
        pc,
        sp,
        exception_depth,
        status: 0x2700,
        usp: 0,
        ssp: sp,
    }
}

#[test]
fn frame_measurement_deltas_handle_nested_calls_without_double_including_callers() {
    let mut observer = CallObserver::new(tje_rom::Revision::Rev02, &[0x120]);
    let mut counts = HybridInstructionCounts::default();
    let mut step = |pc, sp, opcode, return_word, ticks: u64| {
        counts.main_thread += 1;
        counts.last_instruction = Some(m68000_emu::HybridExecutedInstruction {
            pc: pc - 2,
            opcode,
            exception_depth: 0,
            completed_successfully: true,
        });
        observer
            .after_tick(
                &counts,
                CallBoundary {
                    point: point(pc, sp, 0),
                    clock: ticks * 7,
                    frame: ticks / 10,
                },
                return_word,
            )
            .unwrap();
    };
    step(0x120, 0xff7ffc, 0x4eb9, Some(0x106), 1);
    step(0x120, 0xff7ff8, 0x4eb9, Some(0x126), 2);
    step(0x126, 0xff7ffc, 0x4e75, None, 3);
    step(0x106, 0xff8000, 0x4e75, None, 4);
    assert!(observer.active.is_empty());
    assert_eq!(observer.completed.len(), 2);
    assert_eq!(observer.completed[0].usage.main_thread_instructions, 1);
    assert_eq!(observer.completed[1].usage.main_thread_instructions, 3);
}

#[test]
fn faulted_call_or_return_is_not_a_boundary_and_duplicate_ticks_do_not_add_calls() {
    let mut observer = CallObserver::new(tje_rom::Revision::Rev02, &[0x120]);
    let mut counts = HybridInstructionCounts::default();
    counts.main_thread = 1;
    counts.last_instruction = Some(m68000_emu::HybridExecutedInstruction {
        pc: 0x100,
        opcode: 0x4eb9,
        exception_depth: 0,
        completed_successfully: false,
    });
    observer
        .after_tick(
            &counts,
            CallBoundary {
                point: point(0x120, 0xff7ffc, 0),
                clock: 7,
                frame: 0,
            },
            Some(0x106),
        )
        .unwrap();
    assert!(observer.active.is_empty());
    counts.main_thread += 1;
    counts
        .last_instruction
        .as_mut()
        .unwrap()
        .completed_successfully = true;
    observer
        .after_tick(
            &counts,
            CallBoundary {
                point: point(0x120, 0xff7ffc, 0),
                clock: 14,
                frame: 0,
            },
            Some(0x106),
        )
        .unwrap();
    observer
        .after_tick(
            &counts,
            CallBoundary {
                point: point(0x120, 0xff7ffc, 0),
                clock: 15,
                frame: 0,
            },
            Some(0x106),
        )
        .unwrap();
    assert_eq!(observer.active.len(), 1);
    counts.main_thread += 1;
    counts.last_instruction = Some(m68000_emu::HybridExecutedInstruction {
        pc: 0x122,
        opcode: 0x4e75,
        exception_depth: 0,
        completed_successfully: false,
    });
    observer
        .after_tick(
            &counts,
            CallBoundary {
                point: point(0x106, 0xff8000, 0),
                clock: 21,
                frame: 0,
            },
            None,
        )
        .unwrap();
    assert!(observer.completed.is_empty());
    counts.main_thread = 0; // resetting an active counter must fail, not wrap a delta
    assert!(
        observer
            .after_tick(
                &counts,
                CallBoundary {
                    point: point(0x106, 0xff8000, 0),
                    clock: 28,
                    frame: 0
                },
                None
            )
            .is_err()
    );
}

#[test]
fn live_observer_pauses_counters_outside_original_ticks() {
    let mut host = host_program();
    host.enable_session_trace(&[0x120], &[], &[], false)
        .unwrap();
    host.enable_call_observations(&[0x120]).unwrap();
    let (cpu, bus) = host.emulator.hybrid_parts();
    cpu.execute_instruction(bus); // diagnostic/direct CPU execution is not live playback
    assert_eq!(cpu.hybrid_instruction_counts().unwrap().main_thread, 0);
    assert_eq!(host.unfinished_observed_calls(), 0);
}

#[test]
fn actual_hblank_handler_instructions_are_separate_inside_the_real_call() {
    use m68000_emu::BusInterface;
    let mut words = vec![0x4eb9, 0, 0x400, 0x60fe];
    words.resize(0x300 / 2, 0x4e71);
    words.extend(std::iter::repeat_n(0x4e71, 1000));
    words.push(0x4e75);
    // Set the next HBlank interval to255, increment D2, return from the real IRQ.
    let mut host = synthetic_host(&words, &[0x33fc, 0x8aff, 0x00c0, 0x0004, 0x5282, 0x4e73]);
    host.emulator.hybrid_parts().0.set_status_register(0x2000);
    for value in [0x8010, 0x8a00] {
        host.emulator.hybrid_bus_mut().write_word(0xc00004, value);
        host.emulator.hybrid_bus_mut().apply_pending_writes();
    }
    host.enable_session_trace(&[0x400], &[], &[], false)
        .unwrap();
    host.enable_call_observations(&[0x400]).unwrap();
    let mut observations = Vec::new();
    for _ in 0..10000 {
        host.tick_live_original(&mut Inputs(GenesisInputs::default()))
            .unwrap();
        observations.extend(host.take_call_observations());
        if !observations.is_empty() {
            break;
        }
    }
    assert_eq!(observations.len(), 1);
    let call = &observations[0];
    assert_eq!(call.usage.main_thread_instructions, 1001);
    assert!(call.usage.irq_entries_by_level[4] > 0);
    assert_eq!(
        call.usage.exception_handler_instructions,
        call.usage.irq_entries_by_level[4] * 3
    );
    assert_eq!(call.usage.faulted_opcode_attempts, 0);
    assert_eq!(call.usage.other_exception_entries, 0);
    let coverage = host.observed_call_entry_counts();
    assert_eq!(
        (
            coverage[0].executed_entry_opcodes,
            coverage[0].recognized_calls,
            coverage[0].completed_calls
        ),
        (1, 1, 1)
    );
}

#[test]
fn completed_selected_and_unselected_measurements_reuse_only_active_depth_storage() {
    let mut observer = CallObserver::new(tje_rom::Revision::Rev02, &[0x120]);
    let mut counts = HybridInstructionCounts::default();
    let mut ticks = 0;
    let mut warmed_capacity = None;
    for _ in 0..4000 {
        let mut step = |caller_pc, opcode, pc, sp, return_word| {
            counts.main_thread += 1;
            counts.last_instruction = Some(HybridExecutedInstruction {
                pc: caller_pc,
                opcode,
                exception_depth: 0,
                completed_successfully: true,
            });
            ticks += 1;
            observer
                .after_tick(
                    &counts,
                    CallBoundary {
                        point: point(pc, sp, 0),
                        clock: ticks * 7,
                        frame: 0,
                    },
                    return_word,
                )
                .unwrap();
            assert!(observer.invocations.measurement_storage().0 <= 3);
            assert!(observer.invocations.current_witness().is_none());
        };
        step(0x100, 0x4eb9, 0x140, 0xff7ffc, Some(0x106));
        step(0x140, 0x4eb9, 0x120, 0xff7ff8, Some(0x146));
        step(0x120, 0x4eb9, 0x160, 0xff7ff4, Some(0x126));
        step(0x160, 0x4e75, 0x126, 0xff7ff8, None);
        step(0x126, 0x4e75, 0x146, 0xff7ffc, None);
        step(0x146, 0x4e75, 0x106, 0xff8000, None);
        assert!(observer.active.is_empty());
        let (retained, capacity) = observer.invocations.measurement_storage();
        assert_eq!(
            retained, 0,
            "completed unselected calls cannot accumulate history"
        );
        assert_eq!(*warmed_capacity.get_or_insert(capacity), capacity);
        assert_eq!(observer.completed.len(), 1);
        let call = observer.completed.pop().unwrap();
        assert_eq!(
            (
                call.caller_pc,
                call.entry_pc,
                call.entry_sp,
                call.return_pc,
                call.return_sp,
                call.rts_pc
            ),
            (0x140, 0x120, 0xff7ff8, 0x146, 0xff7ffc, 0x126)
        );
        assert_eq!(
            (
                call.usage.main_thread_instructions,
                call.usage.ticks,
                call.usage.master_clocks
            ),
            (3, 3, 21)
        );
    }
    assert_eq!(ticks, 24_000);
    let coverage = observer.coverage[0];
    assert_eq!(
        (
            coverage.executed_entry_opcodes,
            coverage.recognized_calls,
            coverage.completed_calls
        ),
        (4000, 4000, 4000)
    );
    assert_eq!(coverage.faulted_entry_opcodes, 0);
}

#[test]
fn enabling_call_observer_preserves_existing_tallies_and_uses_actual_baseline() {
    let mut host = host_program();
    #[cfg(feature = "reference-proof-hooks")]
    host.emulator.ensure_reference_instruction_counter();
    #[cfg(not(feature = "reference-proof-hooks"))]
    host.emulator
        .hybrid_parts()
        .0
        .hybrid_enable_instruction_counts();
    for _ in 0..16 {
        host.tick_live_original(&mut Inputs(GenesisInputs::default()))
            .unwrap();
    }
    let before = host.cpu().hybrid_instruction_counts().unwrap();
    let baseline = before.main_thread + before.exception_handler;
    assert!(baseline > 0);
    assert_eq!(host.frame_count(), 0);
    host.enable_session_trace(&[0x120], &[], &[], false)
        .unwrap();
    host.enable_call_observations(&[0x120]).unwrap();
    let after = host.cpu().hybrid_instruction_counts().unwrap();
    assert_eq!(after.main_thread + after.exception_handler, baseline);
    assert!(!after.enabled());
    let observer = host
        .session_trace
        .as_ref()
        .unwrap()
        .call_observer
        .as_ref()
        .unwrap();
    assert_eq!(observer.previous_instructions, baseline);
    host.observe_call_return(false).unwrap(); // No new original tick/JSR occurred.
    assert!(host.take_call_observations().is_empty());
    assert_eq!(host.unfinished_observed_calls(), 0);
}
