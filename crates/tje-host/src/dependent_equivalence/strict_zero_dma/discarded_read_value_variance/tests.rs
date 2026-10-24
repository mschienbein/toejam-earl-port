//! Synthetic journal/schema cases, not executed CPU/IRQ or canonical-source proof.
use super::*;
use crate::dependent_equivalence::Probe;

fn proof_progress() -> ProbeProgress {
    super::super::StrictDmaEvidence::default()
        .begin_call(0x1234)
        .unwrap()
}

fn read(address: u32, word: bool, value: u16, purpose: u8) -> CpuAccess {
    CpuAccess::Read {
        address,
        word,
        value,
        purpose,
    }
}
fn journal() -> [CpuAccess; 6] {
    [
        read(0xff1000, true, 0, 2),
        read(0xc00004, true, 0x3400, 0),
        CpuAccess::Write {
            address: 0xff2000,
            word: true,
            value: 0x1234,
        },
        CpuAccess::AcknowledgeInterrupt { level: 6 },
        read(0x1234, true, 0x4e71, 1),
        read(0xff1000, true, 0, 2),
    ]
}
fn controls(journal: &[CpuAccess]) -> ControlLog {
    let mut result = ControlLog::default();
    for &access in journal {
        match access {
            CpuAccess::Read {
                address,
                word,
                value,
                purpose,
            } if address >= 0x800000 => {
                let purpose = match purpose {
                    0 => ReadPurpose::Semantic,
                    1 => ReadPurpose::Prefetch,
                    2 => ReadPurpose::Discarded,
                    _ => panic!("unknown synthetic purpose"),
                };
                result.observe(HybridAccess::Read {
                    address,
                    word,
                    value,
                    purpose,
                });
            }
            CpuAccess::Write {
                address,
                word,
                value,
            } => {
                result.observe(HybridAccess::Write {
                    address,
                    word,
                    value,
                });
            }
            _ => {}
        }
    }
    result
}

#[test]
fn only_core_discarded_values_vary_with_both_values_probe_and_unique_ordinals_retained() {
    let baseline = journal();
    let mut probe = journal();
    probe[0] = read(0xff1000, true, 0x1111, 2);
    probe[5] = read(0xff1000, true, 0x2222, 2);
    let mut progress = proof_progress();
    let verified = verify(
        Some(&baseline),
        Some(&probe),
        Probe::Seeded(3),
        &mut progress,
    )
    .unwrap();
    verified
        .compare_controls(&controls(&baseline), &controls(&probe))
        .unwrap();
    assert_eq!(progress.discarded_read_journal_checks, 1);
    assert_eq!(progress.discarded_read_value_variances.len(), 2);
    for (record, ordinal, value) in [
        (&progress.discarded_read_value_variances[0], 0, 0x1111),
        (&progress.discarded_read_value_variances[1], 5, 0x2222),
    ] {
        assert_eq!(record.seed, 0x1234);
        assert_eq!(record.proof_call_ordinal, 1);
        assert_eq!(record.seeded_probe_index, Some(3));
        assert_eq!(record.collective_fill, None);
        assert_eq!(record.raw_cpu_ordinal, ordinal);
        assert_eq!(record.raw_address, 0xff1000);
        assert_eq!(record.canonical_ram_address, Some(0xff1000));
        assert_eq!(record.width_bytes, 2);
        assert_eq!(record.purpose, "Discarded");
        assert_eq!(record.baseline_value, 0);
        assert_eq!(record.probe_value, value);
    }
    let serialized = serde_json::to_value(&progress.discarded_read_value_variances).unwrap();
    assert_eq!(serialized[1]["baseline_value"], 0);
    assert_eq!(serialized[1]["probe_value"], 0x2222);
}

#[test]
fn semantic_prefetch_address_width_purpose_write_acknowledgement_order_and_count_stay_exact() {
    let baseline = journal();
    for (index, replacement) in [
        (1, read(0xc00004, true, 0x3401, 0)),
        (4, read(0x1234, true, 0x4e70, 1)),
        (0, read(0xff1002, true, 7, 2)),
        (0, read(0xff1000, false, 7, 2)),
        (0, read(0xff1000, true, 7, 0)),
        (
            2,
            CpuAccess::Write {
                address: 0xff2000,
                word: true,
                value: 0x1235,
            },
        ),
        (3, CpuAccess::AcknowledgeInterrupt { level: 4 }),
    ] {
        let mut probe = journal();
        probe[index] = replacement;
        let mut progress = proof_progress();
        assert!(
            verify(
                Some(&baseline),
                Some(&probe),
                Probe::Seeded(0),
                &mut progress
            )
            .is_err()
        );
        assert_eq!(progress.discarded_read_journal_checks, 0);
    }
    let mut reordered = journal();
    reordered.swap(0, 1);
    let mut progress = proof_progress();
    assert!(
        verify(
            Some(&baseline),
            Some(&reordered),
            Probe::Seeded(0),
            &mut progress
        )
        .is_err()
    );
    assert!(
        verify(
            Some(&baseline),
            Some(&baseline[..5]),
            Probe::Seeded(0),
            &mut progress
        )
        .is_err()
    );
    assert!(verify(None, Some(&baseline), Probe::Seeded(0), &mut progress).is_err());
    assert!(verify(Some(&baseline), None, Probe::Seeded(0), &mut progress).is_err());
    assert!(
        verify(
            Some(&baseline),
            Some(&baseline),
            Probe::Byte {
                address: 0xff1000,
                value: 1
            },
            &mut progress
        )
        .is_err()
    );
}

#[test]
fn an_identical_other_time_address_or_context_authored_tag_cannot_borrow_core_variance() {
    let baseline = journal();
    let mut probe = journal();
    probe[0] = read(0xff1000, true, 0x1111, 2);
    let mut misplaced = journal();
    misplaced[5] = read(0xff1000, true, 0x1111, 2);
    let mut progress = proof_progress();
    let verified = verify(
        Some(&baseline),
        Some(&probe),
        Probe::Seeded(0),
        &mut progress,
    )
    .unwrap();
    assert!(
        verified
            .compare_controls(&controls(&baseline), &controls(&misplaced))
            .unwrap_err()
            .contains("core ordinal 0")
    );
    let core = [read(0xff1000, true, 0, 0)];
    let forged = [read(0xff1000, true, 0, 2)];
    let verified = verify(Some(&core), Some(&core), Probe::Seeded(0), &mut progress).unwrap();
    assert!(
        verified
            .compare_controls(&controls(&forged), &controls(&forged))
            .is_err()
    );
    assert!(
        verified
            .compare_controls(&ControlLog::default(), &ControlLog::default())
            .is_err()
    );
}

#[test]
fn previous_values_survive_later_rejection_and_non_ram_never_gets_an_invented_canonical_address() {
    let baseline = journal();
    let mut probe = journal();
    probe[0] = read(0xff1000, true, 0x1111, 2);
    probe[1] = read(0xc00004, true, 0x3401, 0);
    let mut progress = proof_progress();
    assert!(
        verify(
            Some(&baseline),
            Some(&probe),
            Probe::DmaFill(0x80),
            &mut progress
        )
        .is_err()
    );
    assert_eq!(progress.discarded_read_journal_checks, 0);
    assert_eq!(progress.discarded_read_value_variances.len(), 1);
    let record = &progress.discarded_read_value_variances[0];
    assert_eq!(record.seeded_probe_index, None);
    assert_eq!(record.collective_fill, Some(0x80));
    assert_eq!((record.baseline_value, record.probe_value), (0, 0x1111));
    let baseline = [read(0xc00004, true, 0, 2)];
    let probe = [read(0xc00004, true, 1, 2)];
    let mut progress = proof_progress();
    verify(
        Some(&baseline),
        Some(&probe),
        Probe::DmaFill(0xff),
        &mut progress,
    )
    .unwrap();
    assert_eq!(
        progress.discarded_read_value_variances[0].canonical_ram_address,
        None
    );
}

#[test]
fn attempted_calls_keep_seed_and_ordinal_across_reconfiguration_rejection_and_repeat_seed() {
    use super::super::{DependentEvidence, StrictDmaEvidence};
    let baseline = [read(0xff1000, true, 0, 2)];
    let mut receipt = StrictDmaEvidence::default();
    for (ordinal, seed, value, succeeds) in [
        (1, 11, 0x1111, true),
        (2, 22, 0x2222, false),
        (3, 22, 0x3333, true),
    ] {
        let probe = [read(0xff1000, true, value, 2)];
        let mut progress = receipt.begin_call(seed).unwrap();
        let verified = verify(
            Some(&baseline),
            Some(&probe),
            Probe::Seeded(0),
            &mut progress,
        )
        .unwrap();
        let mut probe_control = controls(&probe);
        if !succeeds {
            probe_control.observe(HybridAccess::HostMutation {
                address: 0xff2000,
                word: true,
                value: 7,
            });
        }
        let outcome = verified.compare_controls(&controls(&baseline), &probe_control);
        assert_eq!(outcome.is_ok(), succeeds);
        if succeeds {
            progress.control_checks = 1;
            progress.source_sensitive = 1;
            progress.vram_sensitive = 1;
        }
        receipt.merge(
            DependentEvidence {
                seed,
                calls_proven: u64::from(succeeds),
                calls_rejected: u64::from(!succeeds),
                probe_pairs_matched: u64::from(succeeds),
                ..Default::default()
            },
            progress,
        );
        let record = receipt.discarded_read_value_variances.last().unwrap();
        assert_eq!((record.seed, record.proof_call_ordinal), (seed, ordinal));
        assert_eq!((record.baseline_value, record.probe_value), (0, value));
        assert_eq!(record.seeded_probe_index, Some(0));
    }
    assert_eq!(receipt.proof_calls_started, 3);
    assert_eq!((receipt.calls_proven, receipt.calls_rejected), (2, 1));
    assert_eq!(receipt.qualified_final_probe_pairs, 2);
    assert_eq!(receipt.qualified_source_sensitive_pairs, 2);
    assert_eq!(receipt.qualified_vram_sensitive_pairs, 2);
    assert_eq!(
        receipt.seed, 22,
        "summary seed cannot relabel historical rows"
    );
    let records = &receipt.discarded_read_value_variances;
    assert_eq!(records.len(), 3);
    assert_eq!((records[0].seed, records[0].proof_call_ordinal), (11, 1));
    assert_eq!((records[1].seed, records[1].proof_call_ordinal), (22, 2));
    assert_eq!((records[2].seed, records[2].proof_call_ordinal), (22, 3));
    let serialized = serde_json::to_value(&receipt).unwrap();
    assert_eq!(serialized["discarded_read_value_variances"][1]["seed"], 22);
    assert_eq!(
        serialized["discarded_read_value_variances"][1]["proof_call_ordinal"],
        2
    );
    assert_eq!(
        serialized["discarded_read_value_variances"][1]["probe_value"],
        0x2222
    );
    receipt.proof_calls_started = u64::MAX;
    let exhausted = serde_json::to_value(&receipt).unwrap();
    assert!(receipt.begin_call(33).is_err());
    assert_eq!(exhausted, serde_json::to_value(&receipt).unwrap());
}

#[test]
fn unallocated_progress_cannot_create_variance_receipts_or_a_verified_token() {
    let baseline = [read(0xff1000, true, 0, 2)];
    let probe = [read(0xff1000, true, 7, 2)];
    let mut progress = ProbeProgress::default();
    assert!(
        verify(
            Some(&baseline),
            Some(&probe),
            Probe::Seeded(0),
            &mut progress
        )
        .is_err()
    );
    assert!(progress.discarded_read_value_variances.is_empty());
    assert_eq!(progress.discarded_read_journal_checks, 0);
}
