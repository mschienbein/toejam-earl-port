use super::*;
use crate::{
    CheckReport,
    check_projection::{LogicEvent, LogicTrace},
    fixture::tests::synthetic_host,
    stack_dependency::CheckSide,
};
use genesis_core::native::CpuAccess;
use m68000_emu::{BusInterface, ReadPurpose};
use tje_game::{execution::ProofEnvelope, exits::EntryOutcome};
use tje_mem::Revision;

static CANDIDATE: &[tje_game::Translation] = &[tje_game::Translation {
    id: ids::DMA_COPY_WORDS_TO_VRAM,
    function: |_, _| panic!("metadata/negative fixture must not execute a source body"),
}];

fn host(revision: Revision) -> Host {
    let mut host = synthetic_host(&[0x60fe], &[0x4e73]);
    host.revision = revision;
    host.symbols = crate::catalogue::RevisionSymbols::for_revision(revision);
    host.mode = Mode::Check;
    host.registry = HookRegistry::proof_clocked_strict(
        revision,
        CANDIDATE,
        &[],
        &[ids::DMA_COPY_WORDS_TO_VRAM],
    )
    .unwrap();
    host.checks = CheckReport::registered(&host.registry);
    host
}

fn hook(host: &Host) -> Hook {
    host.registry
        .at(ids::DMA_COPY_WORDS_TO_VRAM
            .metadata()
            .address(host.revision))
        .unwrap()
        .clone()
}

fn arguments(host: &mut Host, source: u32, count: u32) {
    let sp = host.cpu().stack_pointer();
    for (index, value) in [source, 0xc002, count].into_iter().enumerate() {
        host.context().write_u32(sp + 4 + 4 * index as u32, value);
    }
}

fn bytes(host: &Host) -> Vec<u8> {
    bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap()
}

fn observations() -> CheckRun {
    CheckRun {
        audit: AtomicRun {
            stack_reads: Some(Vec::new()),
            strict_control: Some(ControlLog::default()),
            ..AtomicRun::default()
        },
        outcome: EntryOutcome::Returned(None),
        logic_trace: LogicTrace::default(),
        nested_calls: 0,
        wait_frames: 0,
        dependent_writes: Some(Default::default()),
        exact_device_reads: Vec::new(),
        audio: Vec::new(),
    }
}

#[test]
fn explicit_strict_activation_is_separate_atomic_and_state_preserving_both_revisions() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = host(revision);
        let before = bytes(&host);
        host.checks.calls = 17;
        host.checks.mismatches = 3;
        assert!(
            host.enable_clocked_strict_zero_dma(DependentProofConfig {
                seeded_probes: 15,
                ..Default::default()
            })
            .is_err()
        );
        assert!(hook(&host).dependent_proof.is_none());
        assert!(
            host.enable_dependent_equivalence(&[ids::DMA_COPY_WORDS_TO_VRAM], Default::default())
                .is_err()
        );
        host.enable_clocked_strict_zero_dma(Default::default())
            .unwrap();
        assert!(hook(&host).dependent_proof.unwrap().is_clocked_strict());
        assert_eq!(bytes(&host), before);
        assert_eq!((host.checks.calls, host.checks.mismatches), (17, 3));
        host.mode = Mode::Reference;
        assert!(
            host.enable_clocked_strict_zero_dma(Default::default())
                .is_err()
        );
    }
}

#[test]
fn exact_route_original_class_zero_encoding_and_ram_domain_are_required() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = host(revision);
        host.enable_clocked_strict_zero_dma(Default::default())
            .unwrap();
        let root = hook(&host);
        for count in [0, 65536, 0xffff0000] {
            arguments(&mut host, 0xffffb8db, count);
            let plan = resolve(&mut host, &root).unwrap();
            assert_eq!(plan.words(), 65536);
            assert_eq!(plan.summary().raw_first_address, 0xffb8da);
        }
        for count in [1, 65535, 65537] {
            arguments(&mut host, 0xffb8da, count);
            let before = bytes(&host);
            assert!(resolve(&mut host, &root).is_err());
            assert_eq!(bytes(&host), before);
        }
        arguments(&mut host, 0x100, 0);
        assert!(resolve(&mut host, &root).is_err());
        for envelope in [
            ProofEnvelope::ClassDefined,
            ProofEnvelope::ClockedCommutingInterrupts,
        ] {
            let mut wrong = root.clone();
            wrong.proof_envelope = envelope;
            assert!(validate_hook(&wrong).is_err());
        }
        for class in [ExecutionClass::Atomic, ExecutionClass::TimeAware] {
            let mut wrong = root.clone();
            wrong.execution_class = class;
            assert!(validate_hook(&wrong).is_err());
        }
        let mut wrong = root.clone();
        wrong.symbol =
            crate::hooks::HookSymbol::Generated(ids::UPLOAD_QUEUED_SPRITE_GRAPHICS.metadata());
        assert!(validate_hook(&wrong).is_err());
    }
}

#[test]
fn late_override_and_actual_frozen_state_fail_before_fork_effects() {
    let mut host = host(Revision::Rev02);
    host.enable_clocked_strict_zero_dma(Default::default())
        .unwrap();
    for envelope in [
        ProofEnvelope::ClassDefined,
        ProofEnvelope::ClockedCommutingInterrupts,
    ] {
        let mut root = hook(&host);
        root.proof_envelope = envelope;
        let mut fork = host.fork();
        let before = bincode::encode_to_vec(&fork, bincode::config::standard()).unwrap();
        assert!(
            host.run_check_side(
                &mut fork,
                &root,
                CheckSide::Rust,
                crate::call_clock::ClockRunPurpose::Baseline,
                crate::future_frame_watch::ObservationOrigin::CheckFork
            )
            .is_err()
        );
        assert_eq!(
            before,
            bincode::encode_to_vec(&fork, bincode::config::standard()).unwrap()
        );
    }
    let root = hook(&host);
    let mut fork = host.fork();
    fork.hybrid_bus_mut().hybrid_set_functional_mode(true);
    let before = bincode::encode_to_vec(&fork, bincode::config::standard()).unwrap();
    assert!(
        host.run_check_side(
            &mut fork,
            &root,
            CheckSide::Rust,
            crate::call_clock::ClockRunPurpose::Baseline,
            crate::future_frame_watch::ObservationOrigin::CheckFork
        )
        .is_err()
    );
    assert_eq!(
        before,
        bincode::encode_to_vec(&fork, bincode::config::standard()).unwrap()
    );
    assert_eq!(host.checks.calls, 0);
}

#[test]
fn retiming_and_unowned_pending_dma_reject_without_normalizing() {
    let mut host = host(Revision::Rev02);
    let mut survey = genesis_core::timing_survey::TimingSurvey::default();
    survey.policy = Some(genesis_core::timing_survey::TimingPolicy {
        class: genesis_core::timing_survey::TimingClass::VdpAll,
        scanline: 7,
    });
    host.emulator.hybrid_bus_mut().timing_survey = Some(survey);
    let before = bytes(&host);
    assert!(
        host.enable_clocked_strict_zero_dma(Default::default())
            .is_err()
    );
    assert_eq!(before, bytes(&host));
    host.emulator.hybrid_bus_mut().timing_survey = None;
    for value in [
        0x8114, 0x9301, 0x9400, 0x9500, 0x9600, 0x977f, 0x4000, 0x0080,
    ] {
        host.emulator.hybrid_bus_mut().vdp.write_control(value);
    }
    let before = bytes(&host);
    assert!(validate_actual(&host.emulator).is_err());
    assert_eq!(before, bytes(&host));
}

#[test]
fn typed_control_chronology_keeps_host_z80_and_cpu_positions_without_fake_dma_values() {
    let mut a = ControlLog::default();
    let mut b = ControlLog::default();
    a.observe(HybridAccess::DmaRead {
        address: 0xff1000,
        value: 0x1234,
    });
    b.observe(HybridAccess::DmaRead {
        address: 0xff1000,
        value: 0xabcd,
    });
    assert_eq!(
        a, b,
        "DATA is compared in the separate complete actual journal"
    );
    let mutation = HybridAccess::HostMutation {
        address: 0xff1000,
        word: false,
        value: 7,
    };
    a.observe(mutation);
    a.observe(HybridAccess::DmaRead {
        address: 0xff1002,
        value: 8,
    });
    b.observe(HybridAccess::DmaRead {
        address: 0xff1002,
        value: 8,
    });
    b.observe(mutation);
    assert_ne!(a, b);
    let mut cpu = ControlLog::default();
    cpu.observe(HybridAccess::Write {
        address: 0xff1000,
        word: false,
        value: 7,
    });
    let mut z80 = ControlLog::default();
    z80.observe(HybridAccess::Z80Write {
        address: 0xff1000,
        word: false,
        value: 7,
    });
    assert_ne!(cpu, z80);
    let mut joined = ControlLog::default();
    joined.observe(HybridAccess::DmaRead {
        address: 0xff1000,
        value: 1,
    });
    let mut tail = ControlLog::default();
    tail.observe(HybridAccess::DmaRead {
        address: 0xff1002,
        value: 2,
    });
    joined.append(tail);
    assert_eq!(
        joined.events,
        vec![
            ControlEvent::DmaWord {
                raw_address: 0xff1000
            },
            ControlEvent::DmaWord {
                raw_address: 0xff1002
            },
        ]
    );
    let mut wrap = ControlLog::default();
    wrap.observe(HybridAccess::DmaRead {
        address: 0xfffffe,
        value: 1,
    });
    wrap.observe(HybridAccess::DmaRead {
        address: 0xfe0000,
        value: 2,
    });
    let mut wrong_alias = ControlLog::default();
    wrong_alias.observe(HybridAccess::DmaRead {
        address: 0xfffffe,
        value: 1,
    });
    wrong_alias.observe(HybridAccess::DmaRead {
        address: 0xff0000,
        value: 2,
    });
    assert_ne!(
        wrap, wrong_alias,
        "17-bit raw-bank wrapping is not canonical alias equality"
    );
}

#[test]
fn semantic_cpu_residuals_remain_rejected_but_actual_discarded_reads_are_retained() {
    let mut audit = AtomicRun {
        stack_reads: Some(Vec::new()),
        ..Default::default()
    };
    for purpose in [ReadPurpose::Discarded, ReadPurpose::Prefetch] {
        audit.stack_reads.as_mut().unwrap().push(
            super::super::journal::JournalRead::new(
                0x100,
                HybridAccess::Read {
                    address: 0xff1000,
                    word: true,
                    value: 7,
                    purpose,
                },
                3,
            )
            .unwrap(),
        );
    }
    reject_cpu_residual(&audit).unwrap();
    audit.stack_reads.as_mut().unwrap().push(
        super::super::journal::JournalRead::new(
            0x100,
            HybridAccess::Read {
                address: 0xff1002,
                word: true,
                value: 0,
                purpose: ReadPurpose::Semantic,
            },
            1,
        )
        .unwrap(),
    );
    assert!(reject_cpu_residual(&audit).is_err());
}

#[test]
fn irq_reads_require_accepted_relative_envelopes_and_every_actual_raw_read() {
    let mut host = host(Revision::Rev02);
    crate::check_envelope::install_strict_observers(&mut host.emulator).unwrap();
    for purpose in [
        ReadPurpose::Semantic,
        ReadPurpose::Discarded,
        ReadPurpose::Prefetch,
    ] {
        let mut run = observations();
        host.emulator.hybrid_bus_mut().native_cpu_accesses = Some(vec![
            CpuAccess::AcknowledgeInterrupt { level: 4 },
            CpuAccess::Read {
                address: 0xa04000,
                word: false,
                value: 0x93,
                purpose: purpose as u8,
            },
        ]);
        run.logic_trace.events = vec![
            LogicEvent::ExceptionEntered {
                vector: 28,
                handler: 0x300,
                depth: 1,
                master_clock: 1,
            },
            LogicEvent::Access {
                pc: 0x302,
                master_clock: 2,
                access: HybridAccess::Read {
                    address: 0xa04000,
                    word: false,
                    value: 0x93,
                    purpose,
                },
                exact_operand: false,
                ym_busy_loop: false,
            },
            LogicEvent::ExceptionReturned {
                depth: 0,
                master_clock: 3,
            },
        ];
        validate_irq_reads(&host.emulator, &run).unwrap();
        run.logic_trace.events.remove(0);
        assert!(
            validate_irq_reads(&host.emulator, &run).is_err(),
            "a caller already inside an ISR is still root-relative depth zero"
        );
        run.logic_trace.events.clear();
        assert!(
            validate_irq_reads(&host.emulator, &run).is_err(),
            "raw actual reads cannot vanish"
        );
    }
}

#[test]
fn strict_evidence_never_backfills_frozen_approval_and_ram_identity_is_not_sensitivity() {
    let mut combined = DependentEvidence {
        calls_proven: 7,
        seeded_probes_completed: 112,
        ..Default::default()
    };
    combined
        .clocked_strict
        .get_or_insert_with(Default::default)
        .merge(
            DependentEvidence {
                calls_proven: 1,
                seeded_probes_completed: 16,
                ..Default::default()
            },
            ProbeProgress {
                control_checks: 20,
                source_sensitive: 20,
                vram_sensitive: 20,
                ..Default::default()
            },
        );
    assert_eq!(
        (combined.calls_proven, combined.seeded_probes_completed),
        (7, 112)
    );
    let strict = combined.clocked_strict.as_ref().unwrap();
    assert_eq!(
        (strict.calls_proven, strict.seeded_probes_completed),
        (1, 16)
    );
    let baseline = host(Revision::Rev02);
    let mut changed = host(Revision::Rev02);
    changed.context().write_u16(0xff1000, 0xabcd);
    let a = AtomicRun::default();
    assert_eq!(
        sensitivity(&baseline.emulator, &changed.emulator, &a, &a),
        (false, false)
    );
}

// Authored device/controller witnesses for classifier negatives only. These do
// not execute the canonical Rust leaf and never constitute a consumer proof.
fn armed_witness() -> (Host, CheckRun) {
    let mut host = host(Revision::Rev02);
    let mut run = observations();
    crate::check_envelope::install_strict_observers(&mut host.emulator).unwrap();
    let actual_depth = host.cpu().hybrid_exception_depth();
    host.emulator
        .hybrid_bus_mut()
        .hybrid_arm_dma_capture(actual_depth);
    let pc = ids::DMA_COPY_WORDS_TO_VRAM
        .metadata()
        .address(host.revision);
    let bus = host.emulator.hybrid_bus_mut();
    bus.hybrid_cpu_accesses = Some(Vec::new());
    bus.timing_survey.as_mut().unwrap().pc = pc;
    for value in [
        0x8114, 0x8f02, 0x9301, 0x9400, 0x9500, 0x9600, 0x977f, 0x4000, 0x0080,
    ] {
        bus.write_word(0xc00004, value);
        bus.apply_pending_writes();
        run.logic_trace.access(
            pc,
            bus.cycles.m68k_mclk_cycles,
            HybridAccess::Write {
                address: 0xc00004,
                word: true,
                value,
            },
            false,
        );
    }
    let mut state = crate::abi::StackAuditState::new(host.cpu().stack_pointer(), true);
    for access in host
        .emulator
        .hybrid_bus_mut()
        .hybrid_cpu_accesses
        .as_mut()
        .unwrap()
        .drain(..)
    {
        state.observe(pc, access, &mut run.audit);
    }
    (host, run)
}

#[test]
fn actual_root_start_witness_uses_relative_depth_and_rejects_every_extra_start() {
    let (mut host, mut run) = armed_witness();
    validate_root_start(&host.emulator, &run).unwrap();
    let starts = &mut host
        .emulator
        .hybrid_bus_mut()
        .timing_survey
        .as_mut()
        .unwrap()
        .accesses;
    let index = starts
        .iter()
        .position(|event| event.kind == genesis_core::timing_survey::TimingKind::DmaStart)
        .unwrap();
    starts[index].interrupt = 6;
    validate_root_start(&host.emulator, &run).unwrap();
    run.logic_trace.events.insert(
        0,
        LogicEvent::ExceptionEntered {
            vector: 30,
            handler: 0x300,
            depth: 1,
            master_clock: 0,
        },
    );
    assert!(
        validate_root_start(&host.emulator, &run).is_err(),
        "a newly accepted child ISR is not root work"
    );
    run.logic_trace.events.remove(0);
    let starts = &mut host
        .emulator
        .hybrid_bus_mut()
        .timing_survey
        .as_mut()
        .unwrap()
        .accesses;
    let extra = starts[index];
    starts.push(extra);
    assert!(
        validate_root_start(&host.emulator, &run).is_err(),
        "no extra ISR DMA start is dropped"
    );
}

#[test]
fn original_control_keeps_discarded_values_and_reports_bounded_first_difference() {
    // Synthetic observer/schema records, not CPU-accepted source/IRQ evidence.
    let (baseline, mut a) = armed_witness();
    let (probe, mut b) = armed_witness();
    let left = a.audit.strict_control.as_mut().unwrap();
    let right = b.audit.strict_control.as_mut().unwrap();
    let before = left.events.len();
    for index in 0..128_u32 {
        left.observe(HybridAccess::Read {
            address: 0xff1000 + 2 * index,
            word: true,
            value: index as u16,
            purpose: ReadPurpose::Discarded,
        });
        right.observe(HybridAccess::Read {
            address: 0xff1000 + 2 * index,
            word: true,
            value: if index == 64 { 0x1234 } else { index as u16 },
            purpose: ReadPurpose::Discarded,
        });
    }
    let domain = BTreeSet::from([0xff1080, 0xff1081]);
    let mut progress = ProbeProgress::default();
    let error = compare_original_control(
        &baseline.emulator,
        &probe.emulator,
        &a,
        &b,
        &domain,
        &mut progress,
    )
    .unwrap_err();
    assert!(error.starts_with("strict DMA baseline-to-original probe control:"));
    assert!(error.contains(&format!("at event {}", before + 64)));
    assert!(error.contains("purpose: Discarded"));
    assert!(error.contains("value: 4660"));
    assert!(error.contains("context left="));
    assert!(error.len() < 3000, "never dump the complete control stream");
    assert_eq!(progress.control_checks, 0);
    assert!(progress.baseline_control.is_none());
    assert!(progress.baseline_vdp_control.is_none());
}

#[test]
fn probe_control_checks_only_actual_input_domain_and_retains_all_other_devices_audio_and_recipes() {
    let (baseline, a) = armed_witness();
    let (mut probe, mut b) = armed_witness();
    let domain = BTreeSet::from([0xff1000]);
    probe
        .emulator
        .hybrid_bus_mut()
        .memory
        .write_main_ram_byte(0xff1000, 0xa5);
    let mut progress = ProbeProgress::default();
    compare_original_control(
        &baseline.emulator,
        &probe.emulator,
        &a,
        &b,
        &domain,
        &mut progress,
    )
    .unwrap();
    probe
        .emulator
        .hybrid_bus_mut()
        .memory
        .write_main_ram_byte(0xff1001, 7);
    assert!(
        compare_original_control(
            &baseline.emulator,
            &probe.emulator,
            &a,
            &b,
            &domain,
            &mut progress
        )
        .unwrap_err()
        .contains("outside")
    );
    probe
        .emulator
        .hybrid_bus_mut()
        .memory
        .write_main_ram_byte(0xff1001, 0);
    b.audio.push([0.0, -0.0]);
    assert!(
        compare_original_control(
            &baseline.emulator,
            &probe.emulator,
            &a,
            &b,
            &domain,
            &mut progress
        )
        .is_err()
    );
    b.audio.clear();
    b.audit
        .strict_control
        .as_mut()
        .unwrap()
        .observe(HybridAccess::HostMutation {
            address: 0xff1000,
            word: false,
            value: 0,
        });
    assert!(
        compare_original_control(
            &baseline.emulator,
            &probe.emulator,
            &a,
            &b,
            &domain,
            &mut progress
        )
        .is_err(),
        "even an external recipe store later overwritten cannot disappear"
    );
}

#[test]
fn strict_pair_still_requires_full_sr_and_dead_stack_write_value_footprint() {
    let (mut a, mut ar) = armed_witness();
    let (mut b, br) = armed_witness();
    a.enable_clocked_strict_zero_dma(Default::default())
        .unwrap_err(); // Active DMA is intentionally not a fresh entry.
    let root = hook(&a);
    let entry_sp = a.cpu().stack_pointer();
    let ram = a
        .emulator
        .hybrid_bus()
        .memory
        .hybrid_raw_main_ram()
        .to_vec();
    ar.dependent_writes
        .as_mut()
        .unwrap()
        .insert((0xff1000, 7), 1);
    let error = super::super::compare_outputs(
        &mut a.emulator,
        &mut b.emulator,
        &ar,
        &br,
        &root,
        &ram,
        entry_sp,
    )
    .unwrap_err();
    assert!(error.contains("RAM write/value footprint"));
    ar.dependent_writes.as_mut().unwrap().clear();
    let sr = b.cpu().status_register();
    b.emulator.hybrid_parts().0.set_status_register(sr ^ 1);
    let error = super::super::compare_outputs(
        &mut a.emulator,
        &mut b.emulator,
        &ar,
        &br,
        &root,
        &ram,
        entry_sp,
    )
    .unwrap_err();
    assert!(error.contains("SR/CCR"));
}

#[test]
fn sensitivity_requires_actual_consumed_dma_values_and_real_vram_bytes() {
    let baseline = host(Revision::Rev02);
    let mut changed = host(Revision::Rev02);
    let mut a = AtomicRun {
        stack_reads: Some(Vec::new()),
        ..Default::default()
    };
    let mut b = AtomicRun {
        stack_reads: Some(Vec::new()),
        ..Default::default()
    };
    a.stack_reads.as_mut().unwrap().push(
        super::super::journal::JournalRead::new(
            0x100,
            HybridAccess::DmaRead {
                address: 0xff1000,
                value: 0,
            },
            3,
        )
        .unwrap(),
    );
    b.stack_reads.as_mut().unwrap().push(
        super::super::journal::JournalRead::new(
            0x100,
            HybridAccess::DmaRead {
                address: 0xff1000,
                value: 7,
            },
            3,
        )
        .unwrap(),
    );
    assert_eq!(
        sensitivity(&baseline.emulator, &changed.emulator, &a, &b),
        (true, false)
    );
    // Unit-only real device output setup, not a ClockedStrict consumer proof.
    let bus = changed.emulator.hybrid_bus_mut();
    bus.hybrid_set_functional_mode(true);
    for (address, value) in [(0xc00004, 0x4000), (0xc00004, 0), (0xc00000, 0x1234)] {
        bus.write_word(address, value);
        bus.apply_pending_writes();
    }
    assert_eq!(
        sensitivity(&baseline.emulator, &changed.emulator, &a, &b),
        (true, true)
    );
    assert_eq!(
        sensitivity(&baseline.emulator, &changed.emulator, &a, &a),
        (false, true)
    );
}

#[test]
fn domain_growth_or_shrink_rejects_without_retrying_away_control_or_prior_sensitivity() {
    let assigned = BTreeSet::from([0xff1000, 0xff1001]);
    assert!(require_stable_domain(&assigned, &assigned, &assigned).is_ok());
    let grown = BTreeSet::from([0xff1000, 0xff1001, 0xff1002]);
    let shrunk = BTreeSet::from([0xff1000]);
    assert!(require_stable_domain(&assigned, &grown, &assigned).is_err());
    assert!(require_stable_domain(&assigned, &assigned, &shrunk).is_err());
    // Attempt work can be sensitive before a later counterexample. It must not
    // become qualified evidence for a rejected or changed-domain invocation.
    let mut evidence = StrictDmaEvidence::default();
    evidence.merge(
        DependentEvidence {
            calls_rejected: 1,
            probe_pairs_matched: 1,
            ..Default::default()
        },
        ProbeProgress {
            control_checks: 1,
            source_sensitive: 1,
            vram_sensitive: 1,
            ..Default::default()
        },
    );
    assert_eq!(evidence.source_sensitive_pairs, 1);
    assert_eq!(evidence.qualified_final_probe_pairs, 0);
    evidence.merge(
        DependentEvidence {
            calls_rejected: 1,
            probe_pairs_matched: 20,
            ..Default::default()
        },
        ProbeProgress {
            control_checks: 20,
            source_sensitive: 0,
            vram_sensitive: 0,
            ..Default::default()
        },
    );
    assert_eq!(evidence.calls_proven, 0);
    assert_eq!(evidence.qualified_source_sensitive_pairs, 0);
    assert_eq!(evidence.qualified_vram_sensitive_pairs, 0);
}

#[test]
fn irq_dma_start_cannot_borrow_an_identical_other_time_root_write() {
    let mut host = host(Revision::Rev02);
    let mut run = observations();
    crate::check_envelope::install_strict_observers(&mut host.emulator).unwrap();
    let depth = host.cpu().hybrid_exception_depth();
    host.emulator.hybrid_bus_mut().hybrid_arm_dma_capture(depth);
    let pc = ids::DMA_COPY_WORDS_TO_VRAM
        .metadata()
        .address(host.revision);
    {
        let bus = host.emulator.hybrid_bus_mut();
        bus.timing_survey.as_mut().unwrap().pc = pc;
        bus.write_word(0xc00004, 0x0080); // Not a DMA start: first command half.
        bus.apply_pending_writes();
        bus.read_word(0xc00004); // Real latch reset for the authored counterexample.
    }
    run.logic_trace.access(
        pc,
        0,
        HybridAccess::Write {
            address: 0xc00004,
            word: true,
            value: 0x0080,
        },
        false,
    );
    host.run_frame(Default::default()).unwrap(); // Real time, not a fabricated clock increment.
    let bus = host.emulator.hybrid_bus_mut();
    let clock = bus.cycles.m68k_mclk_cycles;
    assert!(clock > 0);
    bus.timing_survey.as_mut().unwrap().pc = pc;
    bus.observe_exception_boundary(m68000_emu::ExceptionBoundary::Entered {
        vector: 30,
        handler: 0x300,
        depth: depth + 1,
    });
    run.logic_trace.events.push(LogicEvent::ExceptionEntered {
        vector: 30,
        handler: 0x300,
        depth: 1,
        master_clock: clock,
    });
    for value in [
        0x8114, 0x8f02, 0x9301, 0x9400, 0x9500, 0x9600, 0x977f, 0x4000, 0x0080,
    ] {
        bus.write_word(0xc00004, value);
        bus.apply_pending_writes();
        run.logic_trace.access(
            pc,
            clock,
            HybridAccess::Write {
                address: 0xc00004,
                word: true,
                value,
            },
            false,
        );
    }
    bus.observe_exception_boundary(m68000_emu::ExceptionBoundary::Returned { depth });
    run.logic_trace.events.push(LogicEvent::ExceptionReturned {
        depth: 0,
        master_clock: clock,
    });
    let capture = bus.hybrid_dma_capture().unwrap();
    assert_eq!(capture.starts.len(), 1);
    assert_eq!(capture.starts[0].exception_depth, depth + 1);
    assert!(validate_root_start(&host.emulator, &run).is_err());
}

#[test]
fn root_dispatch_scope_is_relative_even_when_the_caller_already_is_an_isr() {
    let mut capture = genesis_core::HybridDmaCapture {
        entry_exception_depth: 1,
        current_exception_depth: 1,
        starts: Vec::new(),
        boundaries: Vec::new(),
        invalid_depth_sequence: false,
        missing_dispatch_context: false,
        active_dma_port_write_seen: false,
        deferred_dma_port_write_seen: false,
    };
    let root = genesis_core::HybridDmaStart {
        origin: genesis_core::native::Origin::Main,
        pc: 0x100,
        master_clock: 7,
        address: 0xc00004,
        value: 0x80,
        entry_exception_depth: 1,
        exception_depth: 1,
    };
    assert!(root_dispatch_scope(&capture, &root));
    let mut child = root;
    child.exception_depth = 2;
    assert!(!root_dispatch_scope(&capture, &child));
    capture.entry_exception_depth = 0;
    assert!(!root_dispatch_scope(&capture, &root));
}

#[test]
fn missing_owned_boundary_and_drained_deferred_write_never_regain_eligibility() {
    let (host, mut run) = armed_witness();
    run.logic_trace.events.insert(
        0,
        LogicEvent::ExceptionEntered {
            vector: 28,
            handler: 0x300,
            depth: 1,
            master_clock: 0,
        },
    );
    run.logic_trace.events.insert(
        1,
        LogicEvent::ExceptionReturned {
            depth: 0,
            master_clock: 0,
        },
    );
    assert!(validate_root_start(&host.emulator, &run).is_err());

    let (mut host, run) = armed_witness();
    host.emulator.hybrid_bus_mut().write_word(0xc00004, 0xc000);
    host.emulator.hybrid_bus_mut().apply_pending_writes();
    assert!(
        host.emulator
            .hybrid_bus()
            .hybrid_dma_capture()
            .unwrap()
            .deferred_dma_port_write_seen
    );
    host.run_frame(Default::default()).unwrap();
    assert_eq!(
        host.emulator
            .hybrid_bus()
            .vdp
            .hybrid_checkpoint()
            .pending_dma_port_writes,
        0
    );
    assert!(
        host.emulator
            .hybrid_bus()
            .hybrid_dma_capture()
            .unwrap()
            .deferred_dma_port_write_seen
    );
    assert!(validate_root_start(&host.emulator, &run).is_err());
}
