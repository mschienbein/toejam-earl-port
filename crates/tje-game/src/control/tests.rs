//! Authored raw-data consistency tests. No real-state/admission claim.
use super::*;
use crate::ids;

pub(super) fn span(retirements: u64) -> ControlSpan {
    ControlSpan {
        retirement_start: 10,
        retirement_end: 10 + retirements,
        access_start: 100,
        access_end: 200,
        epoch_tag: 7,
    }
}
fn site(entry: FunctionId, byte_offset: i32) -> ControlSite {
    ControlSite { entry, byte_offset }
}
fn cpu(revision: Revision, site: ControlSite, status: u16) -> CpuControlState {
    let banks = StackBanks {
        user: 0x1234567,
        supervisor: 0xff8000,
    };
    CpuControlState {
        pc: site.address(revision),
        sp: if status & SUPERVISOR != 0 {
            banks.supervisor
        } else {
            banks.user
        },
        status,
        banks,
        exception_depth: 1,
    }
}
fn status(revision: Revision, value: u16) -> ControlObservation {
    let site = site(ids::START, 0x88);
    let before = cpu(revision, site, 0x2700);
    let after = CpuControlState {
        pc: before.pc + 4,
        status: value & SR_MASK,
        sp: if value & SUPERVISOR != 0 {
            before.banks.supervisor
        } else {
            before.banks.user
        },
        ..before
    };
    ControlObservation {
        site,
        span: span(1),
        requested: ControlRequest::WriteStatusRegister { value },
        actual: ControlActual::WriteStatusRegister {
            value,
            before,
            after,
        },
    }
}
pub(super) fn device(entry: FunctionId, reset: bool, value: u16) -> ControlObservation {
    let before = Z80ControlState {
        bus_requested: false,
        reset_asserted: true,
        bus_ack: false,
    };
    let mut after = before;
    if reset {
        after.reset_asserted = value == 0;
    } else {
        after.bus_requested = value == 0x100;
    }
    after.bus_ack = after.bus_requested && !after.reset_asserted;
    let issue = DeviceWriteData {
        address: if reset { Z80_RESET } else { BUSREQ },
        word: true,
        value,
        index: 101,
    };
    let data = DeviceControlData {
        issue,
        commit: DeviceWriteData {
            index: 1000,
            ..issue
        },
        before,
        after,
    };
    ControlObservation {
        site: site(entry, 0),
        span: span(1),
        requested: if reset {
            ControlRequest::Z80Reset { value }
        } else {
            ControlRequest::Z80BusRequest { value }
        },
        actual: if reset {
            ControlActual::Z80Reset(data)
        } else {
            ControlActual::Z80BusRequest(data)
        },
    }
}
pub(super) fn delay(entry: FunctionId, kind: RetryDelayKind) -> ControlObservation {
    ControlObservation {
        site: site(entry, -6),
        span: span(kind.retirements()),
        requested: ControlRequest::RetryDelay(kind),
        actual: ControlActual::RetryDelay(RetryDelayData {
            kind,
            moveq_value: kind.moveq_value(),
            dbra_attempts: kind.attempts(),
            taken_edges: kind.attempts() - 1,
            terminal_fallthrough: true,
            final_d0: 0xffff,
            status_before: 0x271f,
            status_after: 0x2710,
            released_for_all_iterations: true,
        }),
    }
}
fn fill() -> ControlObservation {
    let site = site(ids::START, 0x62);
    let activation = FillBoundaryData {
        generation: 0,
        site,
        span: span(1),
    };
    let completion = FillBoundaryData {
        span: span(8),
        ..activation
    };
    ControlObservation {
        site,
        span: span(10),
        requested: ControlRequest::JoinVdpFill,
        actual: ControlActual::JoinVdpFill(FillJoinData {
            activation: Some(activation),
            completion: Some(completion),
            fill_mode: true,
            active_at_join: false,
            queued_data_writes: 0,
            queued_control_writes: 0,
            intervening_starts: 0,
            producer_span: span(9),
        }),
    }
}
fn rte(revision: Revision, resumed_status: u16, ssp: u32) -> ExitObservation {
    let site = site(ids::LEVEL6_ISR, 14);
    let before = CpuControlState {
        banks: StackBanks {
            user: 0x1234567,
            supervisor: ssp,
        },
        sp: ssp,
        exception_depth: 3,
        ..cpu(revision, site, 0x2700)
    };
    let banks = StackBanks {
        supervisor: ssp.wrapping_add(6),
        ..before.banks
    };
    let after = CpuControlState {
        pc: 0xff123456,
        status: resumed_status & SR_MASK,
        sp: if resumed_status & SUPERVISOR != 0 {
            banks.supervisor
        } else {
            banks.user
        },
        banks,
        exception_depth: 2,
    };
    let values = [resumed_status, 0xff12, 0x3456];
    let words = std::array::from_fn(|i| ControlWordRead {
        address: ssp.wrapping_add(i as u32 * 2) & 0xffffff,
        word: true,
        value: values[i],
        purpose: ControlReadPurpose::Semantic,
        issue_index: 101 + i as u64,
    });
    ExitObservation {
        site,
        span: span(1),
        contract: ExitContract::ExceptionReturn,
        transfer: Some(ControlTransfer {
            entry: site.entry,
            exit: Exit::ExceptionReturn,
        }),
        before,
        after,
        rte_pop: Some(RtePopData { words }),
    }
}

#[test]
fn signed_sites_preserve_preceding_fragments_and_full_address_wrap() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (owner, fragment) in [
            (
                ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
                ids::AUDIO_RETRY_PENDING_PCM_MAILBOX,
            ),
            (
                ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
                ids::AUDIO_RETRY_PENDING_PSG_MAILBOX,
            ),
        ] {
            assert_eq!(
                site(owner, -14).address(revision),
                fragment.metadata().address(revision)
            );
            assert!(site(owner, -14).validate_shape().is_ok());
            assert_eq!(
                site(owner, -13).validate_shape(),
                Err(ControlDataError::Site)
            );
            assert_eq!(
                site(owner, i32::MIN).address(revision),
                owner.metadata().address(revision).wrapping_add(0x80000000)
            );
        }
    }
}
#[test]
fn spans_are_half_open_owner_local_and_do_not_trust_epoch_tags() {
    let valid = span(1);
    assert!(valid.contains_access(100));
    assert!(!valid.contains_access(200));
    assert_eq!(
        ControlSpan {
            retirement_end: 10,
            ..valid
        }
        .validate_shape(),
        Err(ControlDataError::Span)
    );
    assert_eq!(
        ControlSpan {
            access_end: 99,
            ..valid
        }
        .validate_shape(),
        Err(ControlDataError::Span)
    );
    assert!(
        ControlSpan {
            access_end: 100,
            epoch_tag: u64::MAX,
            ..valid
        }
        .validate_shape()
        .is_ok()
    );
}
#[test]
fn full_status_operand_and_idempotent_write_still_require_one_retirement() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for value in [0x2700, 0x001f, 0xffff] {
            let valid = status(revision, value);
            assert_eq!(valid.validate_shape(revision), Ok(()));
            let mut omitted = valid;
            omitted.span.retirement_end = omitted.span.retirement_start;
            assert!(omitted.validate_shape(revision).is_err());
            let mut masked_operand = valid;
            if let ControlActual::WriteStatusRegister { value, .. } = &mut masked_operand.actual {
                *value ^= 0x4000;
            }
            assert_eq!(
                masked_operand.validate_shape(revision),
                Err(ControlDataError::Operand)
            );
            let mut wrong_bank = valid;
            if let ControlActual::WriteStatusRegister { after, .. } = &mut wrong_bank.actual {
                after.banks.user ^= 2;
            }
            assert!(wrong_bank.validate_shape(revision).is_err());
        }
    }
}
#[test]
fn usp_write_checks_privilege_full_value_and_both_banks() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for value in [0, 0xffffffff] {
            let site = site(ids::START, 0x34);
            let mut before = cpu(revision, site, 0x2700);
            before.banks.user = value; // Deliberately idempotent.
            let after = CpuControlState {
                pc: before.pc + 2,
                ..before
            };
            let valid = ControlObservation {
                site,
                span: span(1),
                requested: ControlRequest::WriteUserStackPointer { value },
                actual: ControlActual::WriteUserStackPointer {
                    value,
                    before,
                    after,
                },
            };
            assert_eq!(valid.validate_shape(revision), Ok(()));
            let mut changed = valid;
            if let ControlActual::WriteUserStackPointer { after, .. } = &mut changed.actual {
                after.banks.user ^= 1;
            }
            assert!(changed.validate_shape(revision).is_err());
            let mut unprivileged = valid;
            if let ControlActual::WriteUserStackPointer { before, after, .. } =
                &mut unprivileged.actual
            {
                before.status = 0;
                before.sp = before.banks.user;
                after.status = 0;
                after.sp = after.banks.user;
            }
            assert_eq!(
                unprivileged.validate_shape(revision),
                Err(ControlDataError::Privilege)
            );
        }
    }
}
#[test]
fn exact_device_word_issue_commit_value_and_signals_are_not_interchangeable() {
    let mutants: [fn(&mut ControlObservation); 9] = [
        |o| {
            if let ControlActual::Z80BusRequest(d) = &mut o.actual {
                d.issue.word = false;
            }
        },
        |o| {
            if let ControlActual::Z80BusRequest(d) = &mut o.actual {
                d.commit.word = false;
            }
        },
        |o| {
            if let ControlActual::Z80BusRequest(d) = &mut o.actual {
                d.issue.address += 2;
            }
        },
        |o| {
            if let ControlActual::Z80BusRequest(d) = &mut o.actual {
                d.commit.value ^= 1;
            }
        },
        |o| {
            if let ControlActual::Z80BusRequest(d) = &mut o.actual {
                d.issue.index = o.span.access_end;
            }
        },
        |o| {
            if let ControlActual::Z80BusRequest(d) = &mut o.actual {
                d.after.bus_requested = !d.after.bus_requested;
            }
        },
        |o| {
            if let ControlActual::Z80BusRequest(d) = &mut o.actual {
                d.after.reset_asserted = !d.after.reset_asserted;
            }
        },
        |o| {
            if let ControlActual::Z80BusRequest(d) = &mut o.actual {
                d.before.bus_ack = true;
            }
        },
        |o| o.requested = ControlRequest::Z80Reset { value: 0x100 },
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for value in [0, 0x100] {
            let valid = device(ids::LOAD_SOUND_DRIVER_TO_Z80, false, value);
            assert_eq!(valid.validate_shape(revision), Ok(()));
            for mutate in mutants {
                let mut bad = valid;
                mutate(&mut bad);
                assert!(bad.validate_shape(revision).is_err());
            }
            let reset = device(ids::LOAD_SOUND_DRIVER_TO_Z80, true, value);
            assert_eq!(reset.validate_shape(revision), Ok(()));
        }
        assert_eq!(
            device(ids::START, false, 0x101).validate_shape(revision),
            Err(ControlDataError::Operand)
        );
    }
}
#[test]
fn released_delay_requires_every_attempt_edge_residue_and_own_retirement() {
    let mutants: [fn(&mut RetryDelayData); 7] = [
        |d| d.moveq_value += 1,
        |d| d.dbra_attempts -= 1,
        |d| d.taken_edges += 1,
        |d| d.terminal_fallthrough = false,
        |d| d.final_d0 = 0xffffffff,
        |d| d.status_after ^= 0x10,
        |d| d.released_for_all_iterations = false,
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (entry, kind) in [
            (ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE, RetryDelayKind::Pcm24),
            (
                ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
                RetryDelayKind::Psg124,
            ),
        ] {
            let valid = delay(entry, kind);
            assert_eq!(valid.validate_shape(revision), Ok(()));
            for mutate in mutants {
                let mut bad = valid;
                if let ControlActual::RetryDelay(d) = &mut bad.actual {
                    mutate(d);
                }
                assert!(bad.validate_shape(revision).is_err());
            }
            let mut nested_isr_does_not_count = valid;
            nested_isr_does_not_count.span.retirement_end += 1;
            assert_eq!(
                nested_isr_does_not_count.validate_shape(revision),
                Err(ControlDataError::Span)
            );
        }
    }
}
#[test]
fn fill_identity_mode_queue_and_producer_ranges_fail_closed() {
    let mutants: [fn(&mut FillJoinData); 9] = [
        |d| d.activation = None,
        |d| d.completion = None,
        |d| d.completion.as_mut().unwrap().generation += 1,
        |d| d.fill_mode = false,
        |d| d.active_at_join = true,
        |d| d.queued_data_writes = 1,
        |d| d.queued_control_writes = 1,
        |d| d.intervening_starts = 1,
        |d| d.producer_span.retirement_end = 99,
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let valid = fill();
        assert_eq!(valid.validate_shape(revision), Ok(())); // Known generation zero is not UNKNOWN.
        for mutate in mutants {
            let mut bad = valid;
            if let ControlActual::JoinVdpFill(d) = &mut bad.actual {
                mutate(d);
            }
            assert!(bad.validate_shape(revision).is_err());
        }
    }
}

#[test]
fn backward_fill_completion_is_inconsistent_even_with_equal_generation() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let valid = fill();
        // Independent regression for the review's [10,11) -> [1,2) counterexample.
        let mut backward = valid;
        if let ControlActual::JoinVdpFill(data) = &mut backward.actual {
            let completion = data.completion.as_mut().unwrap();
            completion.span.retirement_start = 1;
            completion.span.retirement_end = 2;
        }
        assert_eq!(
            backward.validate_shape(revision),
            Err(ControlDataError::FillCausality)
        );
        // Overlap is allowed; no duration or disjointness is inferred.
        let mut overlap = valid;
        if let ControlActual::JoinVdpFill(data) = &mut overlap.actual {
            data.activation.as_mut().unwrap().span.retirement_end = 15;
            data.completion.as_mut().unwrap().span.retirement_start = 12;
            data.completion.as_mut().unwrap().span.retirement_end = 18;
        }
        assert_eq!(overlap.validate_shape(revision), Ok(()));
        let mut backward_end = overlap;
        if let ControlActual::JoinVdpFill(data) = &mut backward_end.actual {
            data.completion.as_mut().unwrap().span.retirement_end = 14;
        }
        assert_eq!(
            backward_end.validate_shape(revision),
            Err(ControlDataError::FillCausality)
        );
        let mut backward_start = overlap;
        if let ControlActual::JoinVdpFill(data) = &mut backward_start.actual {
            data.completion.as_mut().unwrap().span.retirement_start = 9;
        }
        assert_eq!(
            backward_start.validate_shape(revision),
            Err(ControlDataError::FillCausality)
        );
    }
}

#[test]
fn rte_pops_full_words_before_selecting_user_bank_and_preserves_address_wrap() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for sr in [0x001f, 0x2519, 0xffff] {
            for ssp in [0xff8000, 0xfffffffc] {
                let valid = rte(revision, sr, ssp);
                assert_eq!(valid.validate_shape(revision), Ok(()));
                assert_eq!(valid.after.banks.supervisor, ssp.wrapping_add(6));
                assert_eq!(valid.after.banks.user, 0x1234567); // Inactive bank/odd USP is not repaired.
            }
        }
    }
}
#[test]
fn rte_wrong_width_purpose_order_pop_bank_sr_pc_depth_and_source_are_rejected() {
    let mutants: [fn(&mut ExitObservation); 12] = [
        |o| o.rte_pop = None,
        |o| o.rte_pop.as_mut().unwrap().words[0].word = false,
        |o| o.rte_pop.as_mut().unwrap().words[0].purpose = ControlReadPurpose::Discarded,
        |o| o.rte_pop.as_mut().unwrap().words.swap(1, 2),
        |o| o.rte_pop.as_mut().unwrap().words[2].issue_index = 101,
        |o| o.rte_pop.as_mut().unwrap().words[0].issue_index = o.span.access_end,
        |o| o.after.status ^= 1,
        |o| o.after.pc ^= 2,
        |o| o.after.banks.user ^= 2,
        |o| {
            o.after.banks.supervisor -= 2;
        },
        |o| o.after.exception_depth += 1,
        |o| o.transfer.as_mut().unwrap().entry = ids::START,
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let valid = rte(revision, 0x001f, 0xfffffffc);
        for mutate in mutants {
            let mut bad = valid;
            mutate(&mut bad);
            assert!(bad.validate_shape(revision).is_err());
        }
    }
}
#[test]
fn rts_and_tail_reuse_existing_exit_types_and_reject_other_control_shapes() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for sr in [0x000f, 0x270f] {
            let site = site(ids::LOAD_SOUND_DRIVER_TO_Z80, 96);
            let mut before = cpu(revision, site, sr);
            before.banks.user &= !1;
            before.sp = before.selected_stack();
            let mut after = before;
            after.pc = 0x123456;
            if sr & SUPERVISOR != 0 {
                after.banks.supervisor += 4;
            } else {
                after.banks.user += 4;
            }
            after.sp = after.selected_stack();
            let valid = ExitObservation {
                site,
                span: span(1),
                contract: ExitContract::Rts,
                transfer: None,
                before,
                after,
                rte_pop: None,
            };
            assert_eq!(valid.validate_shape(revision), Ok(()));
            let mut bad = valid;
            bad.after.status ^= 1;
            assert!(bad.validate_shape(revision).is_err());
            let mut bad = valid;
            bad.contract = ExitContract::Terminal;
            assert_eq!(
                bad.validate_shape(revision),
                Err(ControlDataError::UnsupportedExit)
            );
        }
        let site = site(ids::START, 0x8c);
        let before = cpu(revision, site, 0x2700);
        let after = CpuControlState {
            pc: ids::GAME_PROGRAM.metadata().address(revision),
            ..before
        };
        let valid = ExitObservation {
            site,
            span: span(1),
            contract: ExitContract::Tail {
                target: ids::GAME_PROGRAM,
            },
            transfer: Some(ControlTransfer {
                entry: ids::START,
                exit: Exit::Tail {
                    target: ids::GAME_PROGRAM,
                },
            }),
            before,
            after,
            rte_pop: None,
        };
        assert_eq!(valid.validate_shape(revision), Ok(()));
        for exit in [
            Exit::ExceptionReturn,
            Exit::Terminal,
            Exit::Tail { target: ids::START },
        ] {
            let mut bad = valid;
            bad.transfer.as_mut().unwrap().exit = exit;
            assert!(bad.validate_shape(revision).is_err());
        }
    }
}
#[test]
fn public_self_consistent_data_and_epoch_equality_are_explicitly_not_authentication() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut constructed = rte(revision, 0x001f, 0xff8000);
        constructed.span.epoch_tag = 0; // No trusted identity is required by this DATA validator.
        let result: Result<(), ControlDataError> = constructed.validate_shape(revision);
        assert_eq!(result, Ok(()));
        // This cannot create a RootAdmission: no such constructor/type/Context
        // operation exists in tje-game::control. Actual provenance is host-private.
        assert!(crate::platform::PENDING.contains(&ids::START));
        assert!(crate::platform::PENDING.contains(&ids::LEVEL6_ISR));
    }
}

#[path = "audio_requirements.rs"]
mod audio_requirements;
