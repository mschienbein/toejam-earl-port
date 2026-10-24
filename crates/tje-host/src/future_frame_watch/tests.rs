//! Authored model tests only; live CPU/DMA integration has separate regressions.
use super::*;

const ADDRESS: u32 = 0xff7fe8;
const REGION: StackRegion = StackRegion {
    start: 0xff0000,
    end: 0xff8000,
};

fn irq(depth: u32, clock: u64) -> IrqIdentity {
    let mut state = [0; 25];
    state[15] = 0xff7000;
    state[16] = 0xff7ff0;
    state[17] = 0x1234;
    state[18] = 0x2300;
    state[19] = 0x4e71;
    IrqIdentity {
        vector: 30,
        handler: 0x2000,
        exception_depth: depth,
        entry_master_clock: clock,
        interrupted_state: state,
    }
}

fn difference(address: u32, original: u8, translated: u8) -> DifferingByte {
    DifferingByte {
        address,
        original,
        translated,
    }
}

fn receipt(identity: IrqIdentity, translated: [u8; 2]) -> WatchReceipt {
    WatchReceipt::from_observed_irq(
        Side::Original,
        identity,
        vec![
            difference(ADDRESS, 0x12, translated[0]),
            difference(ADDRESS + 1, 0x34, translated[1]),
        ],
        REGION,
        &[FrameCommit {
            kind: FrameCommitKind::RegisterSave,
            access: HybridAccess::Write {
                address: ADDRESS,
                word: true,
                value: 0x1234,
            },
        }],
    )
    .unwrap()
}

fn arm(watches: &mut FutureFrameWatches, identity: IrqIdentity) -> (ApprovalId, ReceiptId) {
    let approval = watches.new_approval().unwrap();
    let id = watches
        .arm(approval, receipt(identity, [0x56, 0x78]))
        .unwrap();
    (approval, id)
}

fn commit_frame(watches: &mut FutureFrameWatches) {
    watches.observe(
        Side::Original,
        checkpoint(0),
        HybridAccess::Write {
            address: ADDRESS,
            word: true,
            value: 0x1234,
        },
    );
}

impl FutureFrameWatches {
    fn observe_rte_for_test(
        &mut self,
        side: Side,
        identity: &IrqIdentity,
        master_clock: u64,
    ) -> Result<(), WatchError> {
        let mut point = checkpoint(0);
        point.master_clock = master_clock;
        let payload = [genesis_core::HybridExceptionBoundary {
            master_clock,
            boundary: m68000_emu::ExceptionBoundary::Returned {
                depth: identity.exception_depth - 1,
            },
        }];
        self.observe_stream(
            side,
            point,
            &identity.interrupted_state,
            HybridAccess::ExceptionBoundary { index: 0 },
            &payload,
        )
        .map(|_| ())
    }
}

fn activate(watches: &mut FutureFrameWatches, identity: &IrqIdentity) {
    watches.enter_irq(Side::Original, identity.clone()).unwrap();
    commit_frame(watches);
    watches
        .observe_rte_for_test(Side::Original, identity, identity.entry_master_clock + 100)
        .unwrap();
}

fn checkpoint(frame: u64) -> AccessCheckpoint {
    AccessCheckpoint {
        frame,
        master_clock: 1000 + frame * 100,
        cpu_pc: Some(0x3456),
        origin: ObservationOrigin::Live,
    }
}

fn read(address: u32, purpose: ReadPurpose) -> HybridAccess {
    HybridAccess::Read {
        address,
        word: false,
        value: 0x12,
        purpose,
    }
}

fn write(address: u32, word: bool) -> HybridAccess {
    HybridAccess::Write {
        address,
        word,
        value: if word {
            0x5678
        } else if address & 1 == 0 {
            0x56
        } else {
            0x78
        },
    }
}

#[test]
fn only_exact_actual_irq_and_its_return_activate_the_scheduled_debt() {
    for changed in 0..7 {
        let mut watches = FutureFrameWatches::default();
        let expected = irq(1, 100);
        let (approval, _) = arm(&mut watches, expected.clone());
        let mut actual = expected.clone();
        match changed {
            0 => actual.entry_master_clock += 1,
            1 => actual.vector = 28,
            2 => actual.handler += 2,
            3 => actual.interrupted_state[17] += 2,
            4 => actual.interrupted_state[18] ^= 0x100,
            5 => actual.interrupted_state[16] -= 2,
            _ => actual.interrupted_state[19] ^= 1,
        }
        assert!(!watches.observe(
            Side::Original,
            checkpoint(0),
            read(ADDRESS, ReadPurpose::Semantic)
        ));
        watches.enter_irq(Side::Original, actual.clone()).unwrap();
        watches
            .observe_rte_for_test(Side::Original, &actual, 200)
            .unwrap();
        assert!(matches!(
            watches.outcome(approval),
            ApprovalOutcome::Provisional { .. }
        ));
        assert_eq!(watches.evidence()[0].outcome, ReceiptOutcome::PendingIrq);
        watches.finish(CampaignEnd::Completed);
        assert!(matches!(
            watches.outcome(approval),
            ApprovalOutcome::Provisional { .. }
        ));
    }
}

#[test]
fn each_cpu_purpose_and_dma_read_invalidates_without_an_invented_dma_pc() {
    let accesses = [
        read(ADDRESS, ReadPurpose::Semantic),
        read(ADDRESS, ReadPurpose::Discarded),
        read(ADDRESS, ReadPurpose::Prefetch),
        HybridAccess::DmaRead {
            address: ADDRESS,
            value: 0x1234,
        },
    ];
    for access in accesses {
        let mut watches = FutureFrameWatches::default();
        let identity = irq(1, 100);
        let (approval, _) = arm(&mut watches, identity.clone());
        activate(&mut watches, &identity);
        assert!(watches.observe(Side::Original, checkpoint(9), access));
        let ReceiptOutcome::Invalidated { violation } = &watches.evidence()[0].outcome else {
            panic!("missing read evidence")
        };
        assert_eq!(violation.byte, difference(ADDRESS, 0x12, 0x56));
        assert_eq!(violation.checkpoint.frame, 9);
        assert_eq!(
            violation.checkpoint.cpu_pc.is_none(),
            matches!(access, HybridAccess::DmaRead { .. })
        );
        assert!(matches!(
            watches.outcome(approval),
            ApprovalOutcome::Invalidated { .. }
        ));
        watches.observe(Side::Original, checkpoint(10), write(ADDRESS, true));
        watches.finish(CampaignEnd::Completed);
        assert!(matches!(
            watches.outcome(approval),
            ApprovalOutcome::Invalidated { .. }
        ));
    }
}

#[test]
fn paired_model_equal_lanes_discharge_and_mirrors_share_byte_versions() {
    let mut watches = FutureFrameWatches::default();
    let identity = irq(1, 100);
    let (approval, _) = arm(&mut watches, identity.clone());
    activate(&mut watches, &identity);
    watches.observe_paired_actual_for_model_test(
        Side::Original,
        checkpoint(1),
        write(ADDRESS & 0xefffff, false),
    );
    watches.observe_paired_actual_for_model_test(Side::Rust, checkpoint(1), write(ADDRESS, false));
    assert_eq!(
        watches.evidence()[0].remaining,
        BTreeSet::from([ADDRESS + 1])
    );
    assert!(!watches.observe(
        Side::Original,
        checkpoint(2),
        read(ADDRESS, ReadPurpose::Prefetch)
    ));
    assert!(matches!(
        watches.outcome(approval),
        ApprovalOutcome::Provisional { .. }
    ));
    watches.observe_paired_actual_for_model_test(
        Side::Original,
        checkpoint(100_000),
        write(ADDRESS + 1, false),
    );
    watches.observe_paired_actual_for_model_test(
        Side::Rust,
        checkpoint(100_000),
        write(ADDRESS + 1, false),
    );
    assert_eq!(watches.outcome(approval), ApprovalOutcome::Discharged);
    assert!(!watches.needs_observation());
    assert!(!watches.observe(
        Side::Original,
        checkpoint(100_001),
        HybridAccess::DmaRead {
            address: ADDRESS,
            value: 0x1234
        }
    ));
}

#[test]
fn read_before_later_commit_stays_rejected_and_word_lanes_are_independent() {
    let mut watches = FutureFrameWatches::default();
    let identity = irq(1, 100);
    let (approval, _) = arm(&mut watches, identity.clone());
    activate(&mut watches, &identity);
    watches.observe_paired_actual_for_model_test(
        Side::Original,
        checkpoint(1),
        write(ADDRESS, false),
    );
    watches.observe_paired_actual_for_model_test(Side::Rust, checkpoint(1), write(ADDRESS, false));
    assert!(watches.observe(
        Side::Original,
        checkpoint(2),
        HybridAccess::DmaRead {
            address: ADDRESS,
            value: 0x1234
        }
    ));
    let ReceiptOutcome::Invalidated { violation } = &watches.evidence()[0].outcome else {
        panic!("missing second-lane read")
    };
    assert_eq!(violation.byte.address, ADDRESS + 1);
    watches.observe(Side::Original, checkpoint(3), write(ADDRESS, true));
    assert!(matches!(
        watches.outcome(approval),
        ApprovalOutcome::Invalidated { .. }
    ));
}

#[test]
fn shared_bytes_keep_distinct_translated_values_and_reject_all_linked_approvals() {
    let mut watches = FutureFrameWatches::default();
    let identity = irq(1, 100);
    let (first, first_id) = arm(&mut watches, identity.clone());
    let second = watches.new_approval().unwrap();
    let second_id = watches
        .arm(second, receipt(identity.clone(), [0x9a, 0xbc]))
        .unwrap();
    assert_ne!(first_id, second_id);
    let third = watches.new_approval().unwrap();
    assert_ne!(
        watches
            .arm(third, receipt(identity.clone(), [0x56, 0x78]))
            .unwrap(),
        first_id
    );
    activate(&mut watches, &identity);
    assert!(watches.observe(
        Side::Original,
        checkpoint(1),
        read(ADDRESS, ReadPurpose::Discarded)
    ));
    for approval in [first, second, third] {
        assert!(matches!(
            watches.outcome(approval),
            ApprovalOutcome::Invalidated { .. }
        ));
    }
    let values: Vec<_> = watches
        .evidence()
        .iter()
        .map(|entry| {
            let ReceiptOutcome::Invalidated { violation } = &entry.outcome else {
                panic!("missing shared debt rejection")
            };
            violation.byte.translated
        })
        .collect();
    assert_eq!(values, [0x56, 0x9a, 0x56]);
}

#[test]
fn nested_rte_does_not_activate_its_parent_and_pre_rte_saves_do_not_clear_debt() {
    let mut watches = FutureFrameWatches::default();
    let parent = irq(1, 100);
    let mut child = irq(2, 110);
    child.vector = 28;
    let (approval, _) = arm(&mut watches, parent.clone());
    watches.enter_irq(Side::Original, parent.clone()).unwrap();
    watches.enter_irq(Side::Original, child.clone()).unwrap();
    let mut invalid_return = watches.clone();
    assert_eq!(
        invalid_return.observe_rte_for_test(Side::Original, &parent, 150),
        Err(WatchError::InterruptChronology)
    );
    watches.observe(
        Side::Original,
        checkpoint(1),
        HybridAccess::Write {
            address: ADDRESS,
            word: true,
            value: 0x1234,
        },
    );
    assert!(!watches.observe(
        Side::Original,
        checkpoint(1),
        read(ADDRESS, ReadPurpose::Semantic)
    ));
    watches
        .observe_rte_for_test(Side::Original, &child, 160)
        .unwrap();
    assert_eq!(watches.evidence()[0].outcome, ReceiptOutcome::InIrq);
    watches
        .observe_rte_for_test(Side::Original, &parent, 200)
        .unwrap();
    assert_eq!(watches.evidence()[0].outcome, ReceiptOutcome::Active);
    assert!(matches!(
        watches.outcome(approval),
        ApprovalOutcome::Provisional { .. }
    ));
}

#[test]
fn actual_rte_values_are_attested_not_assumed_from_an_entry_match() {
    for bad in [
        vec![],
        vec![HybridAccess::Write {
            address: ADDRESS,
            word: true,
            value: 0xff34,
        }],
        vec![HybridAccess::Write {
            address: ADDRESS,
            word: false,
            value: 0x12,
        }],
    ] {
        let mut watches = FutureFrameWatches::default();
        let identity = irq(1, 100);
        let (approval, _) = arm(&mut watches, identity.clone());
        watches.enter_irq(Side::Original, identity.clone()).unwrap();
        for access in bad {
            watches.observe(Side::Original, checkpoint(0), access);
        }
        assert_eq!(
            watches.observe_rte_for_test(Side::Original, &identity, 200),
            Err(WatchError::ActivationValues)
        );
        watches.observe(Side::Original, checkpoint(2), write(ADDRESS, true));
        assert!(matches!(
            watches.outcome(approval),
            ApprovalOutcome::Provisional { .. }
        ));
        assert!(matches!(
            watches.evidence()[0].outcome,
            ReceiptOutcome::Provisional {
                reason: CampaignEnd::EvidenceMismatch,
                ..
            }
        ));
    }
}

#[test]
fn fork_writes_never_clear_live_debt_but_fork_reads_propagate_sticky_rejection() {
    let mut live = FutureFrameWatches::default();
    let identity = irq(1, 100);
    let (approval, _) = arm(&mut live, identity.clone());
    activate(&mut live, &identity);
    let mut written = live.clone();
    written.observe(Side::Original, checkpoint(2), write(ADDRESS, true));
    assert!(matches!(
        written.outcome(approval),
        ApprovalOutcome::Provisional { .. }
    ));
    assert!(!live.merge_branch_rejections(&written).unwrap());
    assert!(matches!(
        live.outcome(approval),
        ApprovalOutcome::Provisional { .. }
    ));
    for origin in [
        ObservationOrigin::CheckFork,
        ObservationOrigin::DependencyProbe,
        ObservationOrigin::CanonicalReplay,
    ] {
        let mut read_branch = live.clone();
        let mut location = checkpoint(2);
        location.origin = origin;
        assert!(read_branch.observe(
            Side::Original,
            location,
            HybridAccess::DmaRead {
                address: ADDRESS,
                value: 0x1234
            }
        ));
        assert!(live.merge_branch_rejections(&read_branch).unwrap());
        assert!(matches!(
            live.outcome(approval),
            ApprovalOutcome::Invalidated { .. }
        ));
        // Every branch gets the same debt; a completed branch never makes a
        // retained rejection pass. Fresh campaign for the next origin.
        live = FutureFrameWatches::default();
        let (next, _) = arm(&mut live, identity.clone());
        assert_eq!(next, approval);
        activate(&mut live, &identity);
    }
}

#[test]
fn a_foreign_tracker_cannot_merge_even_when_its_numeric_ids_and_receipt_match() {
    let mut live = FutureFrameWatches::default();
    let mut foreign = FutureFrameWatches::default();
    let identity = irq(1, 100);
    arm(&mut live, identity.clone());
    arm(&mut foreign, identity.clone());
    activate(&mut foreign, &identity);
    foreign.observe(
        Side::Original,
        checkpoint(2),
        read(ADDRESS, ReadPurpose::Semantic),
    );
    assert_eq!(
        live.merge_branch_rejections(&foreign),
        Err(WatchError::ForeignBranch)
    );
}

#[test]
fn completed_reset_and_lost_observer_leave_unresolved_approvals_provisional() {
    for reason in [
        CampaignEnd::Completed,
        CampaignEnd::Reset,
        CampaignEnd::ObserverLost,
    ] {
        for state in 0..3 {
            let mut watches = FutureFrameWatches::default();
            let identity = irq(1, 100);
            let (approval, _) = arm(&mut watches, identity.clone());
            if state >= 1 {
                watches.enter_irq(Side::Original, identity.clone()).unwrap();
            }
            if state >= 2 {
                commit_frame(&mut watches);
                watches
                    .observe_rte_for_test(Side::Original, &identity, 200)
                    .unwrap();
            }
            watches.finish(reason);
            assert!(!watches.needs_observation());
            assert!(matches!(
                watches.outcome(approval),
                ApprovalOutcome::Provisional { .. }
            ));
            assert_eq!(watches.new_approval(), Err(WatchError::CampaignClosed));
            assert!(
                matches!(watches.evidence()[0].outcome, ReceiptOutcome::Provisional { reason: actual, unresolved_bytes: 2 } if actual == reason)
            );
        }
    }
}

#[test]
fn frame_ownership_requires_tagged_save_commits_not_just_an_address_below_ssp() {
    let bytes = vec![difference(ADDRESS, 0x12, 0x56)];
    let global = FrameCommit {
        kind: FrameCommitKind::Other,
        access: write(ADDRESS, false),
    };
    assert_eq!(
        WatchReceipt::from_observed_irq(
            Side::Original,
            irq(1, 100),
            bytes.clone(),
            REGION,
            &[global]
        ),
        Err(WatchError::UnownedByte(ADDRESS))
    );
    let save = FrameCommit {
        kind: FrameCommitKind::ExceptionEntry,
        access: write(ADDRESS, false),
    };
    assert!(
        WatchReceipt::from_observed_irq(Side::Original, irq(1, 100), bytes, REGION, &[save])
            .is_ok()
    );
    assert_eq!(
        WatchReceipt::from_observed_irq(
            Side::Original,
            irq(1, 100),
            vec![difference(ADDRESS, 1, 1)],
            REGION,
            &[save]
        ),
        Err(WatchError::EqualByte(ADDRESS))
    );
    assert_eq!(
        WatchReceipt::from_observed_irq(
            Side::Original,
            irq(1, 100),
            vec![
                difference(ADDRESS, 1, 2),
                difference(ADDRESS & 0xefffff, 1, 2)
            ],
            REGION,
            &[save]
        ),
        Err(WatchError::DuplicateByte(ADDRESS))
    );
    assert_eq!(
        WatchReceipt::from_observed_irq(
            Side::Original,
            irq(1, 100),
            vec![difference(0x1234, 1, 2)],
            REGION,
            &[save]
        ),
        Err(WatchError::UnownedByte(0x1234))
    );
    assert_eq!(
        WatchReceipt::from_observed_irq(
            Side::Original,
            irq(1, 100),
            vec![difference(ADDRESS, 1, 2)],
            StackRegion {
                start: ADDRESS + 1,
                end: 0xff8000
            },
            &[save]
        ),
        Err(WatchError::UnownedByte(ADDRESS))
    );
}

#[test]
fn allocated_but_unbound_approval_and_unknown_id_are_never_reported_as_approved() {
    let mut watches = FutureFrameWatches::default();
    let approval = watches.new_approval().unwrap();
    assert!(
        matches!(watches.outcome(approval), ApprovalOutcome::Provisional { receipts } if receipts.is_empty())
    );
    assert_eq!(watches.outcome(ApprovalId(1)), ApprovalOutcome::NotTracked);
    assert_eq!(
        watches.arm(ApprovalId(1), receipt(irq(1, 100), [0x56, 0x78])),
        Err(WatchError::UnknownApproval)
    );
    assert_eq!(watches.new_approval().unwrap().value(), 1);
}

#[test]
fn runtime_evidence_retains_exact_bytes_identity_and_read_purpose() {
    let mut watches = FutureFrameWatches::default();
    let identity = irq(1, 100);
    let (_, id) = arm(&mut watches, identity.clone());
    activate(&mut watches, &identity);
    watches.observe(
        Side::Original,
        checkpoint(7),
        read(ADDRESS & 0xefffff, ReadPurpose::Prefetch),
    );
    let report = serde_json::to_value(&watches).unwrap();
    assert_eq!(report["receipts"][0]["id"], id.value());
    assert_eq!(
        report["receipts"][0]["receipt"]["irq"]["entry_master_clock"],
        100
    );
    assert_eq!(
        report["receipts"][0]["outcome"]["violation"]["origin"],
        "cpu_prefetch"
    );
    assert_eq!(
        report["receipts"][0]["outcome"]["violation"]["byte"]["original"],
        0x12
    );
    assert_eq!(
        report["receipts"][0]["outcome"]["violation"]["byte"]["translated"],
        0x56
    );
    assert_eq!(
        report["receipts"][0]["outcome"]["violation"]["raw_address"],
        ADDRESS & 0xefffff
    );
    assert!(report.get("active").is_none());
}

#[test]
fn actual_dma_completion_observer_keeps_read_before_queued_commit_order() {
    use genesis_config::GenesisEmulatorConfig;
    use genesis_core::GenesisBus;
    use jgenesis_common::frontend::TimingMode;
    use m68000_emu::BusInterface;

    for commit_before_dma in [false, true] {
        let mut watches = FutureFrameWatches::default();
        let identity = irq(1, 100);
        let (approval, _) = arm(&mut watches, identity.clone());
        activate(&mut watches, &identity);
        let mut bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        bus.memory.write_main_ram::<true>(ADDRESS, 0x1234);
        bus.hybrid_cpu_accesses = Some(Vec::new());
        // Authored DMA: one RAM word FF7FE8 -> VRAM0200. The seed above is
        // fixture setup, not a game store and not a discharge of version debt.
        for command in [
            0x8114, 0x8f02, 0x9301, 0x9400, 0x95f4, 0x96bf, 0x977f, 0x4200, 0x0080,
        ] {
            bus.write_word(0xc00004, command);
            bus.apply_pending_writes();
        }
        bus.write_word(ADDRESS, 0x5678);
        if commit_before_dma {
            bus.apply_pending_writes();
        }
        bus.hybrid_set_functional_mode(true);
        bus.apply_pending_writes();
        let mut rejected = false;
        // Paired-model positive control only: there is no live Rust trajectory
        // in Host check mode, and this test-only authority is not a Host API.
        watches.observe_paired_actual_for_model_test(
            Side::Rust,
            checkpoint(11),
            write(ADDRESS, true),
        );
        for access in bus.hybrid_cpu_accesses.as_mut().unwrap().drain(..) {
            rejected |= watches
                .observe_live_stream(checkpoint(12), &identity.interrupted_state, access, &[])
                .unwrap();
        }
        assert_eq!(rejected, !commit_before_dma);
        assert_eq!(
            matches!(watches.outcome(approval), ApprovalOutcome::Discharged),
            commit_before_dma
        );
        assert_eq!(
            matches!(
                watches.outcome(approval),
                ApprovalOutcome::Invalidated { .. }
            ),
            !commit_before_dma
        );
    }
}

#[test]
fn one_sided_store_is_not_an_opposite_write_and_only_actual_value_equality_discharges() {
    let mut watches = FutureFrameWatches::default();
    let identity = irq(1, 100);
    let (approval, _) = arm(&mut watches, identity.clone());
    activate(&mut watches, &identity);
    let store = HybridAccess::Write {
        address: ADDRESS,
        word: true,
        value: 0xabcd,
    };
    watches.observe(Side::Original, checkpoint(1), store);
    assert_eq!(
        watches.evidence()[0].current_values[&ADDRESS],
        CurrentByteValues {
            original: 0xab,
            translated: 0x56,
            original_current_attested: false,
            translated_current_attested: false,
            original_origin: ValueOrigin::CpuStore,
            translated_origin: ValueOrigin::EntrySnapshot,
        }
    );
    assert!(matches!(
        watches.outcome(approval),
        ApprovalOutcome::Provisional { .. }
    ));
    watches.observe(Side::Rust, checkpoint(2), store);
    assert!(matches!(
        watches.outcome(approval),
        ApprovalOutcome::Provisional { .. }
    ));
    watches.observe_paired_actual_for_model_test(Side::Original, checkpoint(3), store);
    watches.observe_paired_actual_for_model_test(Side::Rust, checkpoint(4), store);
    assert_eq!(watches.outcome(approval), ApprovalOutcome::Discharged);
}

#[test]
fn either_actual_side_read_rejects_and_fork_progress_stays_local() {
    for side in [Side::Original, Side::Rust] {
        let mut live = FutureFrameWatches::default();
        let identity = irq(1, 100);
        let (approval, _) = arm(&mut live, identity.clone());
        activate(&mut live, &identity);
        let mut fork = live.clone();
        fork.observe(
            Side::Rust,
            checkpoint(1),
            HybridAccess::Write {
                address: ADDRESS,
                word: true,
                value: 0x1234,
            },
        );
        assert!(matches!(
            fork.outcome(approval),
            ApprovalOutcome::Provisional { .. }
        ));
        assert!(!live.merge_branch_rejections(&fork).unwrap());
        assert!(matches!(
            live.outcome(approval),
            ApprovalOutcome::Provisional { .. }
        ));
        assert!(live.observe(side, checkpoint(2), read(ADDRESS, ReadPurpose::Prefetch)));
        let ReceiptOutcome::Invalidated { violation } = &live.evidence()[0].outcome else {
            panic!("missing side-tagged rejection")
        };
        assert_eq!(violation.side, side);
    }
}

fn boundaries(identity: &IrqIdentity) -> [genesis_core::HybridExceptionBoundary; 2] {
    [
        genesis_core::HybridExceptionBoundary {
            master_clock: identity.entry_master_clock,
            boundary: m68000_emu::ExceptionBoundary::Entered {
                vector: identity.vector,
                handler: identity.handler,
                depth: identity.exception_depth,
            },
        },
        genesis_core::HybridExceptionBoundary {
            master_clock: identity.entry_master_clock + 100,
            boundary: m68000_emu::ExceptionBoundary::Returned {
                depth: identity.exception_depth - 1,
            },
        },
    ]
}

#[test]
fn indexed_rte_boundary_excludes_own_frame_read_but_catches_following_same_step_dma() {
    let mut watches = FutureFrameWatches::default();
    let identity = irq(1, 100);
    let (approval, _) = arm(&mut watches, identity.clone());
    let payloads = boundaries(&identity);
    watches.begin_access_batch(Side::Original);
    for access in [
        HybridAccess::ExceptionBoundary { index: 0 },
        HybridAccess::Write {
            address: ADDRESS,
            word: true,
            value: 0x1234,
        },
        HybridAccess::Read {
            address: ADDRESS,
            word: true,
            value: 0x1234,
            purpose: ReadPurpose::Semantic,
        },
        HybridAccess::ExceptionBoundary { index: 1 },
    ] {
        assert!(
            !watches
                .observe_stream(
                    Side::Original,
                    checkpoint(1),
                    &identity.interrupted_state,
                    access,
                    &payloads
                )
                .unwrap()
        );
    }
    assert_eq!(watches.evidence()[0].outcome, ReceiptOutcome::Active);
    assert!(
        watches
            .observe_stream(
                Side::Original,
                checkpoint(1),
                &identity.interrupted_state,
                HybridAccess::DmaRead {
                    address: ADDRESS,
                    value: 0x1234
                },
                &payloads,
            )
            .unwrap()
    );
    assert!(matches!(
        watches.outcome(approval),
        ApprovalOutcome::Invalidated { .. }
    ));
}

#[test]
fn immediate_provider_header_commits_are_associated_only_with_successful_actual_entry() {
    let identity = irq(1, 100);
    let header = canonical_ram_address(identity.supervisor_sp()) - 4;
    let receipt = WatchReceipt::from_observed_irq(
        Side::Original,
        identity.clone(),
        vec![difference(header, 0x12, 0x56)],
        REGION,
        &[FrameCommit {
            kind: FrameCommitKind::ExceptionEntry,
            access: HybridAccess::Write {
                address: header,
                word: true,
                value: 0x1234,
            },
        }],
    )
    .unwrap();
    let mut watches = FutureFrameWatches::default();
    let approval = watches.new_approval().unwrap();
    watches.arm(approval, receipt).unwrap();
    let payloads = boundaries(&identity);
    watches.begin_access_batch(Side::Original);
    for access in [
        HybridAccess::Write {
            address: header,
            word: true,
            value: 0x1234,
        },
        HybridAccess::ExceptionBoundary { index: 0 },
        HybridAccess::ExceptionBoundary { index: 1 },
    ] {
        watches
            .observe_stream(
                Side::Original,
                checkpoint(1),
                &identity.interrupted_state,
                access,
                &payloads,
            )
            .unwrap();
    }
    assert_eq!(watches.evidence()[0].outcome, ReceiptOutcome::Active);
    assert!(watches.observe(
        Side::Rust,
        checkpoint(2),
        read(header, ReadPurpose::Semantic)
    ));
}

#[test]
fn prefix_commit_outside_the_actual_exception_header_does_not_attest_a_later_save() {
    let mut watches = FutureFrameWatches::default();
    let identity = irq(1, 100);
    let (approval, _) = arm(&mut watches, identity.clone());
    let payloads = boundaries(&identity);
    watches.begin_access_batch(Side::Original);
    for access in [
        HybridAccess::Write {
            address: ADDRESS,
            word: true,
            value: 0x1234,
        },
        HybridAccess::ExceptionBoundary { index: 0 },
    ] {
        watches
            .observe_stream(
                Side::Original,
                checkpoint(1),
                &identity.interrupted_state,
                access,
                &payloads,
            )
            .unwrap();
    }
    assert_eq!(
        watches.observe_stream(
            Side::Original,
            checkpoint(1),
            &identity.interrupted_state,
            HybridAccess::ExceptionBoundary { index: 1 },
            &payloads,
        ),
        Err(WatchError::ActivationValues)
    );
    assert!(matches!(
        watches.outcome(approval),
        ApprovalOutcome::Provisional { .. }
    ));
}

#[test]
fn banked_z80_is_a_real_lane_commit_but_never_a_cpu_frame_save() {
    let z80 = HybridAccess::Z80Write {
        address: ADDRESS,
        word: false,
        value: 0x56,
    };
    assert_eq!(
        WatchReceipt::from_observed_irq(
            Side::Original,
            irq(1, 100),
            vec![difference(ADDRESS, 0x12, 0x56)],
            REGION,
            &[FrameCommit {
                kind: FrameCommitKind::RegisterSave,
                access: z80
            }],
        ),
        Err(WatchError::UnownedByte(ADDRESS))
    );
    let mut watches = FutureFrameWatches::default();
    let identity = irq(1, 100);
    let (approval, _) = arm(&mut watches, identity.clone());
    activate(&mut watches, &identity);
    watches.observe(Side::Original, checkpoint(1), z80);
    assert_eq!(
        watches.evidence()[0].remaining,
        BTreeSet::from([ADDRESS, ADDRESS + 1])
    );
    assert!(matches!(
        watches.outcome(approval),
        ApprovalOutcome::Provisional { .. }
    ));
    assert!(watches.observe(
        Side::Original,
        checkpoint(2),
        HybridAccess::DmaRead {
            address: ADDRESS,
            value: 0x5634
        }
    ));
}

#[test]
fn a_rust_irq_does_not_fabricate_the_required_original_irq() {
    let mut watches = FutureFrameWatches::default();
    let identity = irq(1, 100);
    let (approval, _) = arm(&mut watches, identity.clone());
    watches.enter_irq(Side::Rust, identity.clone()).unwrap();
    watches
        .observe_rte_for_test(Side::Rust, &identity, 200)
        .unwrap();
    assert_eq!(watches.evidence()[0].outcome, ReceiptOutcome::PendingIrq);
    watches.finish(CampaignEnd::Completed);
    assert!(matches!(
        watches.outcome(approval),
        ApprovalOutcome::Provisional { .. }
    ));
}

#[test]
fn missing_boundary_payload_is_unresolved_evidence_not_a_successful_activation() {
    let mut watches = FutureFrameWatches::default();
    let identity = irq(1, 100);
    let (approval, _) = arm(&mut watches, identity.clone());
    assert_eq!(
        watches.observe_stream(
            Side::Original,
            checkpoint(1),
            &identity.interrupted_state,
            HybridAccess::ExceptionBoundary { index: 0 },
            &[],
        ),
        Err(WatchError::BoundaryPayload)
    );
    assert!(matches!(
        watches.outcome(approval),
        ApprovalOutcome::Provisional { .. }
    ));
}

#[test]
fn an_origin_label_never_mints_current_rust_authority_or_read_permission() {
    for side in [Side::Original, Side::Rust] {
        let mut watches = FutureFrameWatches::default();
        let identity = irq(1, 100);
        let (approval, _) = arm(&mut watches, identity.clone());
        activate(&mut watches, &identity);
        let value = if side == Side::Original {
            0x5678
        } else {
            0x1234
        };
        watches.observe(
            side,
            checkpoint(1),
            HybridAccess::Write {
                address: ADDRESS,
                word: true,
                value,
            },
        );
        let versions = watches.evidence()[0].current_values[&ADDRESS];
        assert_eq!(versions.original, versions.translated);
        assert!(!versions.original_current_attested);
        assert!(!versions.translated_current_attested);
        assert!(matches!(
            watches.outcome(approval),
            ApprovalOutcome::Provisional { .. }
        ));
        assert!(watches.observe(side, checkpoint(2), read(ADDRESS, ReadPurpose::Discarded)));
    }
}

#[test]
fn actual_original_numeric_equality_without_a_rust_stream_stays_pending_then_read_rejects() {
    let mut watches = FutureFrameWatches::default();
    let identity = irq(1, 100);
    let (approval, _) = arm(&mut watches, identity.clone());
    activate(&mut watches, &identity);
    watches
        .observe_live_stream(
            checkpoint(1),
            &identity.interrupted_state,
            write(ADDRESS, true),
            &[],
        )
        .unwrap();
    let versions = watches.evidence()[0].current_values[&ADDRESS];
    assert_eq!(versions.original, versions.translated);
    assert!(versions.original_current_attested);
    assert!(!versions.translated_current_attested);
    assert!(matches!(
        watches.outcome(approval),
        ApprovalOutcome::Provisional { .. }
    ));
    assert!(
        watches
            .observe_live_stream(
                checkpoint(2),
                &identity.interrupted_state,
                read(ADDRESS, ReadPurpose::Prefetch),
                &[],
            )
            .unwrap()
    );
    assert!(matches!(
        watches.outcome(approval),
        ApprovalOutcome::Invalidated { .. }
    ));
}

#[test]
fn external_or_native_writes_cannot_invent_a_missing_actual_m68k_frame_save() {
    for actor in 0..4 {
        let mut watches = FutureFrameWatches::default();
        let identity = irq(1, 100);
        let (approval, _) = arm(&mut watches, identity.clone());
        watches.enter_irq(Side::Original, identity.clone()).unwrap();
        let access = match actor {
            0 => HybridAccess::HostMutation {
                address: ADDRESS,
                word: true,
                value: 0x1234,
            },
            1 => HybridAccess::Z80Write {
                address: ADDRESS,
                word: true,
                value: 0x1234,
            },
            _ => HybridAccess::Write {
                address: ADDRESS,
                word: true,
                value: 0x1234,
            },
        };
        let mut point = checkpoint(0);
        if actor == 2 {
            point.cpu_pc = None;
        }
        if actor == 3 {
            watches.observe_setup_write(Side::Original, ADDRESS, 0x12);
            watches.observe_setup_write(Side::Original, ADDRESS + 1, 0x34);
        } else {
            watches.observe(Side::Original, point, access);
        }
        assert_eq!(
            watches.observe_rte_for_test(Side::Original, &identity, 200),
            Err(WatchError::ActivationValues)
        );
        assert!(watches.evidence()[0].observed_frame_producers.is_empty());
        assert!(watches.evidence()[0].observed_frame_values.is_empty());
        assert!(matches!(
            watches.outcome(approval),
            ApprovalOutcome::Provisional { .. }
        ));
    }
}

#[test]
fn external_writes_update_witnessed_frame_values_without_creating_post_rte_authority() {
    for actor in 0..4 {
        let mut watches = FutureFrameWatches::default();
        let identity = irq(1, 100);
        let (approval, _) = arm(&mut watches, identity.clone());
        watches.enter_irq(Side::Original, identity.clone()).unwrap();
        watches.observe(
            Side::Original,
            checkpoint(0),
            HybridAccess::Write {
                address: ADDRESS,
                word: true,
                value: 0xffff,
            },
        );
        let access = match actor {
            0 => HybridAccess::HostMutation {
                address: ADDRESS,
                word: true,
                value: 0x1234,
            },
            1 => HybridAccess::Z80Write {
                address: ADDRESS,
                word: true,
                value: 0x1234,
            },
            _ => HybridAccess::Write {
                address: ADDRESS,
                word: true,
                value: 0x1234,
            },
        };
        let mut point = checkpoint(0);
        if actor == 2 {
            point.cpu_pc = None;
        }
        if actor == 3 {
            watches.observe_setup_write(Side::Original, ADDRESS, 0x12);
            watches.observe_setup_write(Side::Original, ADDRESS + 1, 0x34);
        } else {
            watches.observe(Side::Original, point, access);
        }
        watches
            .observe_rte_for_test(Side::Original, &identity, 200)
            .unwrap();
        let evidence = &watches.evidence()[0];
        assert_eq!(evidence.outcome, ReceiptOutcome::Active);
        assert_eq!(
            evidence.observed_frame_producers,
            BTreeSet::from([ADDRESS, ADDRESS + 1])
        );
        assert_eq!(
            evidence.observed_frame_values,
            BTreeMap::from([(ADDRESS, 0x12), (ADDRESS + 1, 0x34)])
        );
        assert!(evidence.current_values.values().all(|value| !value.original_current_attested && !value.translated_current_attested));
        assert!(matches!(
            watches.outcome(approval),
            ApprovalOutcome::Provisional { .. }
        ));
        assert!(watches.observe(
            Side::Original,
            checkpoint(1),
            read(ADDRESS, ReadPurpose::Semantic)
        ));
    }
}
