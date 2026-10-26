//! Authored cartridge/CPU traces for the input consumer. These tests do not
//! admit a body or discharge a proof from a hand-built invocation/CPU snapshot.
use super::*;
use crate::replayed_environment::{inputs, record};
use std::sync::Arc;
use tje_engine_core::dependencies::StateMask;

pub(in crate::replayed_environment) fn capture(
    revision: Revision,
    body: Vec<u16>,
    extra: Vec<(FunctionId, Vec<u16>)>,
) -> (Host, Arc<record::Invocation>) {
    let (host, original, _) = capture_with_tracker(revision, body, extra);
    (host, original)
}

pub(in crate::replayed_environment) fn capture_with_tracker(
    revision: Revision,
    body: Vec<u16>,
    extra: Vec<(FunctionId, Vec<u16>)>,
) -> (Host, Arc<record::Invocation>, record::Tracker) {
    let mut bodies = vec![(ROOT, body)];
    bodies.extend(extra);
    let mut host = host(
        revision,
        &[],
        LITERAL,
        &[ROOT],
        &main_call(revision, false),
        &bodies,
    );
    let mut journal = record::Journal::default();
    let mut tracker = record::Tracker::new(Trajectory::default(), revision);
    let mut frame = 0;
    for _ in 0..10000 {
        let before = journal.begin(&mut host.emulator, frame).unwrap();
        let effect = host.tick(&mut Inputs(GenesisInputs::default())).unwrap();
        let completed = {
            let step = journal.observe(&host.emulator, frame, &before).unwrap();
            let function = bodies
                .iter()
                .map(|(id, _)| *id)
                .find(|id| id.metadata().address(revision) == step.after.cpu.pc());
            tracker
                .observe(
                    &host.emulator,
                    &step,
                    record::Target {
                        function,
                        wanted: function.is_some(),
                    },
                )
                .unwrap()
                .completed
        };
        journal.restore(&mut host.emulator, before).unwrap();
        if let Some(original) = completed
            && original.entry.function == Some(ROOT)
        {
            assert!(original.unsupported.is_none(), "{:?}", original.unsupported);
            return (host, original, tracker);
        }
        if effect == TickEffect::FrameRendered {
            frame += 1;
        }
    }
    panic!("authored actual JSR-to-RTS root did not complete");
}

pub(in crate::replayed_environment) fn analyze(
    host: &Host,
    original: &record::Invocation,
) -> Result<inputs::CalleeInputs, Failure> {
    let cartridge = host.emulator.hybrid_bus().cartridge.as_ref().unwrap();
    let reader = |pc: u32| {
        if pc & 1 != 0 || pc.checked_add(2).is_none_or(|end| end > 0x100000) {
            None
        } else {
            Some(cartridge.peek_word(pc))
        }
    };
    inputs::analyze(
        original,
        &reader,
        7,
        &mut inputs::CompilerCache::default(),
        StateMask::EMPTY,
    )
}

#[test]
fn private_word_load_and_byte_extension_kill_only_the_dead_initial_byte() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let body = vec![
            0x598f, 0x3017, 0x4880, 0x48c0, 0x23c0, 0x00ff, 0x9002, 0x588f, 0x4e75,
        ];
        let (host, original) = capture(revision, body, vec![]);
        let inputs = analyze(&host, &original).unwrap();
        let local = crate::abi::canonical_ram_address(original.entry.boundary.cpu.sp() - 4);
        assert!(
            !inputs.memory.contains_key(&local),
            "dead high byte is not an input"
        );
        assert_eq!(inputs.memory[&(local + 1)].bits, 0xff);
        let output = inputs
            .output_inputs
            .demand(tje_engine_core::ir::StateField::Data(0), u32::MAX)
            .unwrap();
        assert!(!output.memory.contains_key(&local));
        assert_eq!(output.memory[&(local + 1)].bits, 0xff);
    }
}

#[test]
fn a_real_high_byte_effect_before_extension_remains_required() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let body = vec![
            0x598f, 0x3017, 0x33c0, 0x00ff, 0x9002, 0x4880, 0x48c0, 0x588f, 0x4e75,
        ];
        let (host, original) = capture(revision, body, vec![]);
        let inputs = analyze(&host, &original).unwrap();
        let local = crate::abi::canonical_ram_address(original.entry.boundary.cpu.sp() - 4);
        assert_eq!(inputs.memory[&local].bits, 0xff);
        assert_eq!(inputs.memory[&(local + 1)].bits, 0xff);
    }
}

#[test]
fn a_dead_nonprivate_semantic_word_read_is_still_a_full_input() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        // MOVE.W absolute,-(SP), release without consuming the private spill.
        let body = vec![0x3f39, 0x00ff, 0x9000, 0x548f, 0x4e75];
        let (host, original) = capture(revision, body, vec![]);
        let inputs = analyze(&host, &original).unwrap();
        assert_eq!(inputs.memory[&VALUE].bits, 0xff);
        assert_eq!(inputs.memory[&(VALUE + 1)].bits, 0xff);
    }
}

#[test]
fn missing_and_changed_private_read_chronology_cannot_hide_behind_deadness() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let body = vec![0x598f, 0x3017, 0x3217, 0x588f, 0x4e75];
        let (host, mut original) = capture(revision, body.clone(), vec![]);
        let local = crate::abi::canonical_ram_address(original.entry.boundary.cpu.sp() - 4);
        let missing = Arc::get_mut(&mut original).expect("sole actual captured root");
        let marker = missing.trace.chronology.iter_mut().find(|event| matches!(event, record::Chronology::CpuRead { access, .. } if access.address == local)).unwrap();
        *marker = record::Chronology::Acknowledge { level: 0 };
        assert!(
            analyze(&host, missing).is_err(),
            "no issue-order reconstruction may replace the missing marker"
        );
        let (host, mut original) = capture(revision, body, vec![]);
        let changed = Arc::get_mut(&mut original).expect("sole actual captured root");
        let (issue, access) = changed
            .trace
            .chronology
            .iter_mut()
            .filter_map(|event| match event {
                record::Chronology::CpuRead { access, issue } if access.address == local => {
                    Some((issue, access))
                }
                _ => None,
            })
            .nth(1)
            .unwrap();
        access.value ^= 1;
        changed.trace.issues[*issue].value = access.value;
        assert!(
            analyze(&host, changed).is_err(),
            "unused private RAM cannot change without an actual preceding physical write"
        );
    }
}

#[test]
fn appended_read_markers_keep_exact_offsets_across_nested_actual_calls() {
    fn visit(original: &record::Invocation, reads: &mut usize, children: &mut usize) {
        for event in &original.trace.chronology {
            if let record::Chronology::CpuRead { access, issue } = event {
                assert_eq!(original.trace.issues[*issue], *access);
                assert!(matches!(access.direction, record::Direction::Read(_)));
                *reads += 1;
            }
        }
        for event in &original.trace.events {
            if let record::Event::Callee(child) = event {
                *children += 1;
                visit(child, reads, children);
            }
        }
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        let body = [
            jsr(CHILD, revision).to_vec(),
            vec![0x3039, 0x00ff, 0x9000],
            jsr(CHILD, revision).to_vec(),
            vec![0x4e75],
        ]
        .concat();
        let (host, original) = capture(
            revision,
            body,
            vec![(CHILD, vec![0x3039, 0x00ff, 0x9000, 0x4e75])],
        );
        let (mut reads, mut children) = (0, 0);
        visit(&original, &mut reads, &mut children);
        assert!(reads >= 9 && children == 2);
        analyze(&host, &original).unwrap();
    }
}

#[test]
fn an_actual_call_needs_both_physical_return_commits_and_its_before_ready_observation() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = host(
            revision,
            &[],
            LITERAL,
            &[ROOT],
            &main_call(revision, false),
            &[(ROOT, vec![0x4e75])],
        );
        let mut journal = record::Journal::default();
        let mut reached = false;
        for _ in 0..100 {
            let before = journal.begin(&mut host.emulator, 0).unwrap();
            host.tick(&mut Inputs(GenesisInputs::default())).unwrap();
            {
                let step = journal.observe(&host.emulator, 0, &before).unwrap();
                if step
                    .actual
                    .is_some_and(crate::call_observation::successful_call_instruction)
                    && step.after.cpu.pc() == ROOT.metadata().address(revision)
                {
                    let cpu = host.cpu().hybrid_state_words();
                    let clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
                    for mutation in 0..6 {
                        let mut chronology = step.chronology.to_vec();
                        if mutation == 1 {
                            let index = chronology
                                .iter()
                                .position(|event| {
                                    matches!(event, record::Chronology::CpuCommit { .. })
                                })
                                .unwrap();
                            chronology.remove(index);
                        } else if mutation == 2 {
                            let record::Chronology::CpuCommit { access, .. } = chronology
                                .iter_mut()
                                .find(|event| matches!(event, record::Chronology::CpuCommit { .. }))
                                .unwrap()
                            else {
                                unreachable!()
                            };
                            access.value ^= 1;
                        }
                        let altered = record::ObservedStep {
                            before: step.before,
                            after: step.after,
                            actual: step.actual.map(|mut actual| {
                                if mutation == 5 {
                                    actual.pc = actual.pc.wrapping_add(2);
                                }
                                actual
                            }),
                            instruction_ready: mutation != 3 && step.instruction_ready,
                            issues: step.issues,
                            driven_masks: step.driven_masks,
                            chronology: &chronology,
                        };
                        let mut tracker = record::Tracker::new(Trajectory::default(), revision);
                        let result = tracker.observe(
                            &host.emulator,
                            &altered,
                            record::Target {
                                function: Some(if mutation == 4 { CHILD } else { ROOT }),
                                wanted: true,
                            },
                        );
                        assert_eq!(
                            result.is_ok(),
                            mutation == 0,
                            "mutation {mutation} cannot mint a source call from a plausible post-entry snapshot"
                        );
                    }
                    assert_eq!(host.cpu().hybrid_state_words(), cpu);
                    assert_eq!(host.emulator.hybrid_bus().cycles.m68k_mclk_cycles, clock);
                    reached = true;
                }
            }
            journal.restore(&mut host.emulator, before).unwrap();
            if reached {
                break;
            }
        }
        assert!(reached, "the authored real JSR must actually execute");
    }
}

#[test]
fn cleanup_failure_removes_only_the_new_actual_completion_and_preserves_published_history() {
    fn phase(
        host: &mut Host,
        state: &mut State,
        corrupt_return: Option<u32>,
    ) -> crate::Result<bool> {
        state.before_tick(
            &mut host.emulator,
            host.frame,
            host.checks.replayed_environment.as_mut().unwrap(),
        )?;
        let before = host
            .emulator
            .hybrid_cpu()
            .hybrid_instruction_counts()
            .unwrap();
        let before_count = before.main_thread + before.exception_handler;
        let effect = host.tick(&mut Inputs(GenesisInputs::default()))?;
        let after = host
            .emulator
            .hybrid_cpu()
            .hybrid_instruction_counts()
            .unwrap();
        let retired_rts = after.main_thread + after.exception_handler == before_count + 1
            && after.last_instruction.is_some_and(|actual| {
                actual.completed_successfully
                    && actual.opcode == 0x4e75
                    && actual.exception_depth == 0
            });
        let corrupt = retired_rts && corrupt_return == Some(host.emulator.hybrid_cpu().pc());
        state.after_tick_with_cleanup(
            &mut host.emulator,
            &host.registry,
            &host.disabled_check_roots,
            (
                effect == TickEffect::FrameRendered,
                GenesisInputs::default(),
            ),
            host.checks.replayed_environment.as_mut().unwrap(),
            |journal, emulator, before| {
                if corrupt {
                    // This is after actual observe/queue, not an injected
                    // completion or manufactured restoration error.
                    emulator.hybrid_bus_mut().reference_read_contexts = None;
                }
                let result = journal.restore(emulator, before);
                if corrupt {
                    assert!(result.is_err(), "the SAME real restore rejects ownership");
                }
                result
            },
        );
        if effect == TickEffect::FrameRendered {
            host.finish_live_frame()?;
        }
        Ok(corrupt)
    }

    for revision in [Revision::Rev02, Revision::Rev00] {
        for policy in [
            CheckFailurePolicy::FailFast,
            CheckFailurePolicy::ContinueOriginal,
        ] {
            let main = [
                vec![0x46fc, 0x2700],
                jsr(ROOT, revision).to_vec(),
                jsr(ROOT, revision).to_vec(),
                jsr(ROOT, revision).to_vec(),
                vec![0x60fe],
            ]
            .concat();
            let mut host = host(
                revision,
                &[],
                LITERAL,
                &[ROOT],
                &main,
                &[(ROOT, literal_code())],
            );
            host.set_check_failure_policy(policy).unwrap();
            finish_one(&mut host).unwrap();
            let report = host.checks.replayed_environment.as_ref().unwrap();
            assert_eq!(
                (report.reference_completions, report.completed_proofs),
                (1, 1)
            );
            assert_eq!(report.by_function[ROOT.metadata().inventory_id].passed, 1);

            // Move the SAME lifecycle recorder, never an emulator snapshot, so
            // the next real completion can remain queued rather than analyzed.
            let mut state = host.replayed_state.take().unwrap();
            assert_eq!(state.proofs.len(), 1);
            let published = *state.proofs.keys().next().unwrap();
            assert!(state.ledger.complete(published));
            for _ in 0..10000 {
                if state.reference_completions == 2 {
                    break;
                }
                assert!(!phase(&mut host, &mut state, None).unwrap());
            }
            assert_eq!(state.reference_completions, 2);
            assert!(
                state.has_pending(),
                "the second actual RTS is genuinely queued"
            );
            assert!(state.collecting());
            let before_fault_clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
            let mut corrupted = false;
            for _ in 0..10000 {
                if phase(&mut host, &mut state, Some(0x116)).unwrap() {
                    corrupted = true;
                    break;
                }
            }
            assert!(corrupted, "the third actual RTS and cleanup must execute");
            assert_eq!(state.reference_completions, 3);
            assert!(!state.collecting());
            let reason = state.infrastructure_failure().unwrap();
            assert_eq!(reason.kind, UnsupportedKind::Observation);
            assert!(
                reason
                    .detail
                    .starts_with("passive original journal cleanup:")
            );
            assert!(
                matches!(
                    state.take_error(),
                    Some(crate::HostError::ReplayUnsupported { function, reason })
                        if function == "actual original journal cleanup"
                            && reason.detail.starts_with("passive original journal cleanup:")
                ),
                "all source observation/queue work succeeded before the REAL cleanup failed"
            );

            let retained = state
                .next_completed()
                .expect("retain the second real completion");
            assert_eq!(retained.original.entry.function, Some(ROOT));
            assert_eq!(retained.original.exit.cpu.pc(), 0x110);
            assert!(retained.original.unsupported.is_none());
            assert!(
                state.next_completed().is_none(),
                "the third newly queued root must be removed (truncate deletion is observable)"
            );
            assert!(
                state.ledger.complete(published),
                "retain the actual published proof"
            );
            assert_eq!(state.proofs.len(), 1, "no new analysis/pass was published");
            let report = host.checks.replayed_environment.as_ref().unwrap();
            assert_eq!(report.completed_proofs, 1);
            assert_eq!(report.by_function[ROOT.metadata().inventory_id].passed, 1);

            let physical_cpu = host.emulator.hybrid_cpu().hybrid_state_words();
            let physical_clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
            assert!(
                physical_clock > before_fault_clock,
                "actual RTS progress remains"
            );
            host.replayed_state = Some(state);
            assert!(matches!(
                host.tick_live_original(&mut Inputs(GenesisInputs::default())),
                Err(crate::HostError::ReplayUnsupported { reason, .. })
                    if reason.detail.starts_with("passive original journal cleanup:")
            ));
            assert_eq!(
                host.emulator.hybrid_cpu().hybrid_state_words(),
                physical_cpu
            );
            assert_eq!(
                host.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
                physical_clock
            );
        }
    }
}
