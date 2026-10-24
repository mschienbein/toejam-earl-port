//! Authored component integration fixtures. These bind an extra rejection
//! obligation to an actually checked RTS producer; they do not claim a complete
//! commutation proof. IRQ entry/RTE/clock/frame bytes come from the real CPU/VDP.
use super::*;
use crate::{
    CheckFailurePolicy, CheckReport, Mode,
    abi::HookContext,
    catalogue,
    fixture::tests::synthetic_host,
    hooks::{HookFn, HookRegistry},
    sinks::{Inputs, NoCartridgeSaves},
};
use genesis_config::GenesisEmulatorConfig;
use genesis_core::GenesisEmulator;
use jgenesis_common::frontend::{EmulatorTrait, PartialClone, TickEffect};
use m68000_emu::BusInterface;
use tje_game::{FunctionMetadata, execution::ExecutionClass, ids};
use tje_rom::Revision;

const PRODUCER: u32 = 0x600;
const CONSUMER: u32 = 0x620;
const SLOT: u32 = 0xff7ffa;
const REGION: StackRegion = StackRegion {
    start: catalogue::STACK_BOTTOM,
    end: catalogue::STACK_TOP,
};
const DMA: [u16; 9] = [
    0x8114, 0x8f02, 0x9301, 0x9400, 0x95fd, 0x96bf, 0x977f, 0x4200, 0x0080,
];

fn noop(_: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    Ok(None)
}
fn store_zero(context: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    context.write_u16(SLOT, 0);
    let sr = tje_game::Context::status_register(context);
    tje_game::Context::set_status_register(context, (sr & !15) | 4);
    Ok(None)
}
fn read_then_error(context: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    let _value = context.read_u16(SLOT);
    Err(crate::HostError::Metadata(
        "authored read then error".into(),
    ))
}
fn dma(context: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    for command in DMA {
        context.bus().write_word(0xc00004, command);
    }
    Ok(None)
}
fn metadata(address: u32, name: &'static str) -> FunctionMetadata {
    let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
    symbol.name = name;
    symbol.addresses = [address; 2];
    symbol.sizes = [128; 2];
    symbol.abi.argument_count = 0;
    symbol.abi.returns_value = false;
    symbol
}

fn authored_host(
    revision: Revision,
    main: &[u16],
    handler: &[u16],
    consumer: &[u16],
    body: HookFn,
    class: ExecutionClass,
) -> crate::Host {
    let mut rom = vec![0; 0x1000];
    rom[..4].copy_from_slice(&catalogue::STACK_TOP.to_be_bytes());
    rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    rom[28 * 4..29 * 4].copy_from_slice(&0x500_u32.to_be_bytes());
    rom[30 * 4..31 * 4].copy_from_slice(&0x400_u32.to_be_bytes());
    // Actual program enables VBlank, lowers the mask, then loops. Nothing in
    // the observer changes the CPU's PC/SP, pending level or IRQ delivery.
    let mut program = vec![0x33fc, 0x8120, 0x00c0, 0x0004, 0x46fc, 0x2000];
    program.extend_from_slice(main);
    for (address, words) in [
        (0x100, program.as_slice()),
        (0x200, &[0x60fe, 0x4e71][..]),
        (0x400, handler),
        (0x500, &[0x4e73][..]),
        (PRODUCER as usize, &[0x4e75][..]),
        (CONSUMER as usize, consumer),
    ] {
        for (index, word) in words.iter().enumerate() {
            rom[address + 2 * index..address + 2 * index + 2].copy_from_slice(&word.to_be_bytes());
        }
    }
    let mut host = synthetic_host(&[0x60fe], &[0x4e73]);
    host.emulator = GenesisEmulator::create(
        Some(rom),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    host.emulator.hybrid_bus_mut().hybrid_writes = Some(Vec::new());
    host.mode = Mode::Check;
    host.revision = revision;
    host.symbols = catalogue::RevisionSymbols::for_revision(revision);
    let mut registry = HookRegistry::empty(revision);
    registry
        .register_contract(metadata(PRODUCER, "AuthoredFutureProducer"), true, noop)
        .unwrap();
    registry
        .register_execution_contract(metadata(CONSUMER, "AuthoredFutureConsumer"), class, body)
        .unwrap();
    host.checks = CheckReport::registered(&registry);
    host.registry = registry;
    host.settle_fixture_boundary(Default::default()).unwrap();
    host.check_fixture(PRODUCER, &[]).unwrap();
    host.check_fixture(PRODUCER, &[]).unwrap();
    for _ in 0..100_000 {
        if host.emulator.hybrid_bus().interrupt_level() == 6
            && host.cpu().hybrid_exception_depth() == 0
        {
            return host;
        }
        let effect = host
            .tick_live_original(&mut Inputs(Default::default()))
            .unwrap();
        if effect == TickEffect::FrameRendered {
            host.finish_live_frame().unwrap();
        }
    }
    panic!("authored VDP did not produce VBlank");
}

fn observed_receipt(host: &mut crate::Host) -> WatchReceipt {
    let old_value = host.emulator.hybrid_bus().memory.read_main_ram_byte(SLOT);
    let mut oracle = host.emulator.partial_clone();
    host.execute_fork(&mut oracle, |machine| {
        machine.hybrid_bus_mut().hybrid_cpu_accesses = Some(Vec::new());
        machine.hybrid_bus_mut().hybrid_exception_boundaries = Some(Vec::new());
        let mut output = crate::sinks::HeadlessOutput;
        let mut audio = crate::sinks::HeadlessAudio::default();
        for _ in 0..1000 {
            let before = machine.hybrid_cpu().hybrid_state_words();
            machine
                .tick(
                    &mut output,
                    &mut audio,
                    &mut Inputs(Default::default()),
                    &mut NoCartridgeSaves,
                )
                .map_err(|error| crate::HostError::Emulator(error.to_string()))?;
            let bus = machine.hybrid_bus_mut();
            let event = bus
                .hybrid_exception_boundaries
                .as_ref()
                .unwrap()
                .iter()
                .find_map(|entry| {
                    if let m68000_emu::ExceptionBoundary::Entered {
                        vector: 30,
                        handler,
                        depth,
                    } = entry.boundary
                    {
                        Some(IrqIdentity {
                            vector: 30,
                            handler,
                            exception_depth: depth,
                            entry_master_clock: entry.master_clock,
                            interrupted_state: before,
                        })
                    } else {
                        None
                    }
                });
            if let Some(identity) = event {
                let value = bus.memory.read_main_ram_byte(SLOT);
                assert_ne!(
                    value, old_value,
                    "fixture needs two actual differing snapshots"
                );
                let commits: Vec<_> = bus
                    .hybrid_cpu_accesses
                    .as_ref()
                    .unwrap()
                    .iter()
                    .filter_map(|access| {
                        if let HybridAccess::Write { address, .. } = *access
                            && (SLOT..catalogue::STACK_TOP)
                                .contains(&canonical_ram_address(address))
                        {
                            Some(FrameCommit {
                                kind: FrameCommitKind::ExceptionEntry,
                                access: *access,
                            })
                        } else {
                            None
                        }
                    })
                    .collect();
                return WatchReceipt::from_observed_irq(
                    Side::Original,
                    identity,
                    vec![DifferingByte {
                        address: SLOT,
                        original: value,
                        translated: old_value,
                    }],
                    REGION,
                    &commits,
                )
                .map_err(|error| crate::HostError::Metadata(error.to_string()));
            }
            bus.hybrid_cpu_accesses.as_mut().unwrap().clear();
            bus.hybrid_exception_boundaries.as_mut().unwrap().clear();
        }
        Err(crate::HostError::Metadata(
            "no actual IRQ entry in oracle".into(),
        ))
    })
    .unwrap()
}

fn arm_component_requirement(host: &mut crate::Host, receipt: WatchReceipt) -> ApprovalId {
    // White-box certifier boundary only: the watch does not certify the other
    // commutation conditions and these tests never promote a game translation.
    let hook = host.registry.at(PRODUCER).unwrap().clone();
    host.future_observer.natural_checks += 1;
    let result = host.arm_future_frame_watch(
        &hook,
        "authored component obligation",
        0x0046_5241_4d45,
        vec![receipt],
    );
    host.future_observer.natural_checks -= 1;
    result.unwrap().unwrap()
}

fn tick_until_rte(host: &mut crate::Host) {
    let mut entered = false;
    for _ in 0..1000 {
        let before = host.cpu().hybrid_exception_depth();
        let effect = host
            .tick_live_original(&mut Inputs(Default::default()))
            .unwrap();
        if effect == TickEffect::FrameRendered {
            host.finish_live_frame().unwrap();
        }
        let after = host.cpu().hybrid_exception_depth();
        entered |= after > 0;
        if entered && before > 0 && after == 0 {
            return;
        }
    }
    panic!("actual IRQ did not return");
}

fn current_report(host: &crate::Host) -> serde_json::Value {
    serde_json::to_value(&host.checks.future_frame_provenance).unwrap()
}

#[test]
fn actual_cpu_semantic_discarded_and_prefetch_reads_reject_without_machine_repairs() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (main, handler, expected) in [
            (
                &[0x3039, 0x00ff, 0x7ffa, 0x60f8][..],
                &[0x4e73][..],
                "cpu_semantic",
            ),
            (
                &[0x4279, 0x00ff, 0x7ffa, 0x60f8][..],
                &[0x4e73][..],
                "cpu_discarded",
            ),
            (
                &[0x4a79, 0x00ff, 0x9000, 0x67f8, 0x4ef9, 0x00ff, 0x7ffa][..],
                &[0x33fc, 1, 0x00ff, 0x9000, 0x4e73][..],
                "cpu_prefetch",
            ),
        ] {
            let mut watched = authored_host(
                revision,
                main,
                handler,
                &[0x4e75],
                noop,
                ExecutionClass::Atomic,
            );
            let mut reference = authored_host(
                revision,
                main,
                handler,
                &[0x4e75],
                noop,
                ExecutionClass::Atomic,
            );
            let receipt = observed_receipt(&mut watched);
            let approval = arm_component_requirement(&mut watched, receipt);
            let mut failed = false;
            for _ in 0..2000 {
                let result = watched.tick_live_original(&mut Inputs(Default::default()));
                let original = reference
                    .tick_live_original(&mut Inputs(Default::default()))
                    .unwrap();
                if result.is_err() {
                    assert_eq!(
                        watched.future_frame_approval_outcome(approval),
                        ApprovalOutcome::Invalidated {
                            receipts: vec![ReceiptId(0)]
                        }
                    );
                    assert_eq!(watched.checks.mismatches, 1);
                    assert_eq!(
                        watched.checks.by_function["AuthoredFutureProducer"].mismatches,
                        1
                    );
                    let report = current_report(&watched);
                    assert_eq!(
                        report["current"]["watches"]["receipts"][0]["outcome"]["violation"]["origin"],
                        expected
                    );
                    assert_eq!(
                        bincode::encode_to_vec(&watched.emulator, bincode::config::standard())
                            .unwrap(),
                        bincode::encode_to_vec(&reference.emulator, bincode::config::standard())
                            .unwrap()
                    );
                    assert_eq!(
                        watched
                            .audio
                            .samples
                            .iter()
                            .map(|sample| sample.map(f64::to_bits))
                            .collect::<Vec<_>>(),
                        reference
                            .audio
                            .samples
                            .iter()
                            .map(|sample| sample.map(f64::to_bits))
                            .collect::<Vec<_>>()
                    );
                    failed = true;
                    break;
                }
                if original == TickEffect::FrameRendered {
                    watched.finish_live_frame().unwrap();
                    reference.finish_live_frame().unwrap();
                }
            }
            assert!(failed, "{revision:?} {expected} was not observed");
        }
    }
}

#[test]
fn debt_survives_frame_boundaries_and_a_later_irq_overwrite_is_not_a_rust_write() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = authored_host(
            revision,
            &[0x60fe],
            &[0x4e73],
            &[0x4e75],
            noop,
            ExecutionClass::Atomic,
        );
        let receipt = observed_receipt(&mut host);
        let approval = arm_component_requirement(&mut host, receipt);
        tick_until_rte(&mut host);
        assert_eq!(
            current_report(&host)["current"]["watches"]["receipts"][0]["outcome"]["status"],
            "active"
        );
        let start = host.frame_count();
        let error = loop {
            match host.run_frame(Default::default()) {
                Ok(_) => assert!(host.frame_count() <= start + 2),
                Err(error) => break error,
            }
        };
        assert!(host.frame_count() > start);
        assert!(error.to_string().contains("future private-frame"));
        assert!(matches!(
            host.future_frame_approval_outcome(approval),
            ApprovalOutcome::Invalidated { .. }
        ));
    }
}

#[test]
fn actual_dma_after_rte_reads_the_watched_ram_without_an_invented_cpu_pc() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = authored_host(
            revision,
            &[0x60fe],
            &[0x4e73],
            &[0x4e75],
            noop,
            ExecutionClass::Atomic,
        );
        let receipt = observed_receipt(&mut host);
        arm_component_requirement(&mut host, receipt);
        tick_until_rte(&mut host);
        for command in DMA {
            host.context().bus().write_word(0xc00004, command);
        }
        let mut error = None;
        for _ in 0..1000 {
            if let Err(failure) = host.tick_live_original(&mut Inputs(Default::default())) {
                error = Some(failure);
                break;
            }
        }
        assert!(error.is_some());
        let report = current_report(&host);
        let violation = &report["current"]["watches"]["receipts"][0]["outcome"]["violation"];
        assert_eq!(violation["origin"], "vdp_dma");
        assert_eq!(violation["checkpoint"]["cpu_pc"], serde_json::Value::Null);
    }
}

#[test]
fn speculative_dma_normalization_and_read_before_error_invalidate_the_inherited_binding() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for class in [ExecutionClass::Functional, ExecutionClass::TimeAware] {
            let mut words = Vec::new();
            for command in DMA {
                words.extend([0x33fc, command, 0x00c0, 4]);
            }
            words.push(0x4e75);
            let mut host = authored_host(revision, &[0x60fe], &[0x4e73], &words, dma, class);
            let receipt = observed_receipt(&mut host);
            arm_component_requirement(&mut host, receipt);
            tick_until_rte(&mut host);
            host.settle_fixture_boundary(Default::default()).unwrap();
            assert!(host.check_fixture(CONSUMER, &[]).is_err());
            let report = current_report(&host);
            let violation = &report["current"]["watches"]["receipts"][0]["outcome"]["violation"];
            assert_eq!(violation["side"], "rust");
            assert_eq!(violation["origin"], "vdp_dma");
            assert_ne!(violation["checkpoint"]["origin"], "live");
            assert_eq!(host.checks.mismatches, 1);
        }
        let mut host = authored_host(
            revision,
            &[0x60fe],
            &[0x4e73],
            &[0x3039, 0x00ff, 0x7ffa, 0x4e75],
            read_then_error,
            ExecutionClass::Atomic,
        );
        let receipt = observed_receipt(&mut host);
        arm_component_requirement(&mut host, receipt);
        tick_until_rte(&mut host);
        host.settle_fixture_boundary(Default::default()).unwrap();
        let error = host.check_fixture(CONSUMER, &[]).unwrap_err();
        assert!(
            error.to_string().contains("authored read then error"),
            "{error}"
        );
        assert_eq!(host.checks.mismatches, 1);
        assert_eq!(
            host.future_frame_provenance_outcome().status,
            ProvenanceStatus::Invalidated
        );
    }
}

#[test]
fn inspection_peek_does_not_read_game_data_but_actual_original_calls_do_even_on_late_error() {
    for endless in [false, true] {
        let consumer = if endless {
            &[0x3039, 0x00ff, 0x7ffa, 0x60fe][..]
        } else {
            &[0x3039, 0x00ff, 0x7ffa, 0x4e75][..]
        };
        let mut host = authored_host(
            Revision::Rev02,
            &[0x60fe],
            &[0x4e73],
            consumer,
            noop,
            ExecutionClass::Atomic,
        );
        let receipt = observed_receipt(&mut host);
        let old_rust = receipt.bytes()[0].translated;
        let approval = arm_component_requirement(&mut host, receipt);
        tick_until_rte(&mut host);
        let _debug_value = host.context().read_u8(SLOT);
        assert!(matches!(
            host.future_frame_approval_outcome(approval),
            ApprovalOutcome::Provisional { .. }
        ));
        host.context().write_u8(SLOT, old_rust);
        assert!(matches!(
            host.future_frame_approval_outcome(approval),
            ApprovalOutcome::Provisional { .. }
        ));
        let error = host
            .context()
            .call_original_with_contract(&metadata(CONSUMER, "InspectionOriginal"), true, &[])
            .unwrap_err();
        if endless {
            assert!(matches!(error, crate::HostError::CallDidNotReturn { .. }));
        }
        let summary = host.finish_future_frame_provenance();
        assert_eq!(summary.status, ProvenanceStatus::Invalidated);
        assert_eq!(host.checks.mismatches, 1);
        let report = current_report(&host);
        assert_eq!(
            report["current"]["watches"]["receipts"][0]["outcome"]["violation"]["current_values"]["original_origin"],
            "external_setup"
        );
    }
}

#[test]
fn reset_and_isolated_candidates_remain_archived_pending_without_id_reuse() {
    let mut host = authored_host(
        Revision::Rev02,
        &[0x60fe],
        &[0x4e73],
        &[0x4e75],
        noop,
        ExecutionClass::Atomic,
    );
    let receipt = observed_receipt(&mut host);
    let first = arm_component_requirement(&mut host, receipt.clone());
    tick_until_rte(&mut host);
    assert_eq!(
        host.finish_future_frame_provenance().status,
        ProvenanceStatus::Provisional
    );
    host.reset();
    assert_eq!(
        host.future_frame_provenance_outcome().status,
        ProvenanceStatus::Provisional
    );
    let hook = host.registry.at(PRODUCER).unwrap().clone();
    let second = host
        .arm_future_frame_watch(&hook, "isolated pending", 2, vec![receipt])
        .unwrap()
        .unwrap();
    assert!(second.value() > first.value());
    assert!(matches!(
        host.future_frame_approval_outcome(second),
        ApprovalOutcome::Provisional { .. }
    ));
    assert!(host.emulator.hybrid_bus().hybrid_cpu_accesses.is_none());
    assert_eq!(host.future_frame_provenance_outcome().provisional, 2);
}

#[test]
fn thread_auto_traits_match_the_observed_baseline_contract() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<crate::Host>();
    assert_send_sync::<crate::CheckReport>();
}

#[test]
fn retained_check_call_original_store_is_not_mislabelled_a_speculative_or_rust_write() {
    let words = [0x33fc, 0, 0x00ff, 0x7ffa, 0x4e75];
    let mut host = authored_host(
        Revision::Rev02,
        &[0x60fe],
        &[0x4e73],
        &words,
        store_zero,
        ExecutionClass::Atomic,
    );
    let receipt = observed_receipt(&mut host);
    let approval = arm_component_requirement(&mut host, receipt);
    tick_until_rte(&mut host);
    host.settle_fixture_boundary(Default::default()).unwrap();
    host.check_call(CONSUMER, &[]).unwrap();
    let report = current_report(&host);
    let current = &report["current"]["watches"]["receipts"][0]["current_values"][SLOT.to_string()];
    assert_eq!(current["original"], current["translated"]);
    assert_eq!(current["original_current_attested"], true);
    assert_eq!(current["translated_current_attested"], false);
    assert_eq!(current["original_origin"], "cpu_store");
    assert!(matches!(
        host.future_frame_approval_outcome(approval),
        ApprovalOutcome::Provisional { .. }
    ));
    assert_eq!(host.checks.mismatches, 0);
}

#[test]
fn nested_fixture_store_progress_is_discarded_but_fixture_reads_invalidate_live_bindings() {
    let words = [0x33fc, 0, 0x00ff, 0x7ffa, 0x4e75];
    let mut host = authored_host(
        Revision::Rev02,
        &[0x60fe],
        &[0x4e73],
        &words,
        store_zero,
        ExecutionClass::Atomic,
    );
    let receipt = observed_receipt(&mut host);
    let approval = arm_component_requirement(&mut host, receipt);
    tick_until_rte(&mut host);
    host.settle_fixture_boundary(Default::default()).unwrap();
    let machine = bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap();
    let model = current_report(&host)["current"]["watches"].clone();
    host.isolated_fixture(|fixture| {
        fixture.check_call(CONSUMER, &[])?;
        fixture.isolated_fixture(|inner| inner.check_call(CONSUMER, &[]).map(|_| ()))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap(),
        machine
    );
    assert_eq!(current_report(&host)["current"]["watches"], model);
    assert!(matches!(
        host.future_frame_approval_outcome(approval),
        ApprovalOutcome::Provisional { .. }
    ));
    let metadata = metadata(CONSUMER, "FixturePhysicalRead");
    // A separate authored CPU read on the fixture trajectory, not a RAM peek.
    let reader = [0x3039, 0x00ff, 0x7ffa, 0x4e75];
    host.context().write_u16(0xfff000, reader[0]);
    for (index, word) in reader.iter().copied().enumerate().skip(1) {
        host.context().write_u16(0xfff000 + 2 * index as u32, word);
    }
    let mut metadata = metadata;
    metadata.addresses = [0xfff000; 2];
    let result = host.isolated_fixture(|fixture| {
        fixture
            .context()
            .call_original_with_contract(&metadata, true, &[])
            .map(|_| ())
    });
    assert!(result.is_err());
    assert_eq!(
        host.future_frame_provenance_outcome().status,
        ProvenanceStatus::Invalidated
    );
    assert_eq!(host.checks.mismatches, 1);
}

#[test]
fn prior_indexed_observer_keeps_its_prefix_and_receives_correctly_remapped_live_markers() {
    let mut host = authored_host(
        Revision::Rev02,
        &[0x60fe],
        &[0x4e73],
        &[0x4e75],
        noop,
        ExecutionClass::Atomic,
    );
    let receipt = observed_receipt(&mut host);
    let old = genesis_core::HybridExceptionBoundary {
        master_clock: 7,
        boundary: m68000_emu::ExceptionBoundary::Returned { depth: 0 },
    };
    let prefix = HybridAccess::ExceptionBoundary { index: 0 };
    host.emulator.hybrid_bus_mut().hybrid_cpu_accesses = Some(vec![prefix]);
    host.emulator.hybrid_bus_mut().hybrid_exception_boundaries = Some(vec![old]);
    arm_component_requirement(&mut host, receipt);
    tick_until_rte(&mut host);
    assert_eq!(
        host.finish_future_frame_provenance().status,
        ProvenanceStatus::Provisional
    );
    let bus = host.emulator.hybrid_bus();
    let accesses = bus.hybrid_cpu_accesses.as_ref().unwrap();
    let boundaries = bus.hybrid_exception_boundaries.as_ref().unwrap();
    assert_eq!(accesses[0], prefix);
    assert_eq!(boundaries[0], old);
    let observed: Vec<_> = accesses
        .iter()
        .filter_map(|access| {
            if let HybridAccess::ExceptionBoundary { index } = *access {
                Some(boundaries[index as usize].boundary)
            } else {
                None
            }
        })
        .collect();
    assert!(matches!(
        observed[1],
        m68000_emu::ExceptionBoundary::Entered { vector: 30, .. }
    ));
    assert!(matches!(
        observed[2],
        m68000_emu::ExceptionBoundary::Returned { depth: 0 }
    ));
}

#[test]
fn one_read_invalidates_all_earlier_approvals_without_swallowing_the_current_failed_attempt() {
    let main = [
        0x4a79,
        0x00ff,
        0x9000,
        0x67f8,
        0x4eb9,
        0,
        CONSUMER as u16,
        0x60fe,
    ];
    let handler = [0x33fc, 1, 0x00ff, 0x9000, 0x4e73];
    let mut host = authored_host(
        Revision::Rev02,
        &main,
        &handler,
        &[0x3039, 0x00ff, 0x7ffa, 0x4e75],
        read_then_error,
        ExecutionClass::Atomic,
    );
    let receipt = observed_receipt(&mut host);
    let first = arm_component_requirement(&mut host, receipt.clone());
    let second = arm_component_requirement(&mut host, receipt);
    host.set_check_failure_policy(CheckFailurePolicy::ContinueOriginal)
        .unwrap();
    host.run_frame(Default::default()).unwrap();
    for id in [first, second] {
        assert!(matches!(
            host.future_frame_approval_outcome(id),
            ApprovalOutcome::Invalidated { .. }
        ));
    }
    assert_eq!(host.checks.calls, 3);
    assert_eq!(host.checks.mismatches, 3);
    assert_eq!(
        host.checks.by_function["AuthoredFutureProducer"].mismatches,
        2
    );
    assert_eq!(
        host.checks.by_function["AuthoredFutureConsumer"].mismatches,
        1
    );
    assert_eq!(host.checks.failures.len(), 3);
    assert_eq!(
        host.future_frame_provenance_outcome().status,
        ProvenanceStatus::Invalidated
    );
}

fn unreachable_boundary() -> crate::exits::ReferenceBoundary {
    crate::exits::ReferenceBoundary {
        pc: 0x800,
        sp: None,
        source: None,
        min_instructions: 1,
        entry_exception_return: false,
    }
}

#[test]
fn a_read_before_a_later_clock_ceiling_error_still_invalidates_the_earlier_producer() {
    use tje_game::execution::CallCeiling;
    let main = [
        0x4a79, 0x00ff, 0x9000, 0x67f8, 0x3039, 0x00ff, 0x7ffa, 0x60fe,
    ];
    let handler = [0x33fc, 1, 0x00ff, 0x9000, 0x4e73];
    let mut host = authored_host(
        Revision::Rev02,
        &main,
        &handler,
        &[0x4e75],
        noop,
        ExecutionClass::Atomic,
    );
    let receipt = observed_receipt(&mut host);
    let approval = arm_component_requirement(&mut host, receipt);
    tick_until_rte(&mut host);
    let branch = host
        .future_frame_branch(Side::Original, ObservationOrigin::CanonicalReplay)
        .unwrap();
    let mut fork = host.emulator.partial_clone();
    let mut clock = crate::execution::WaitClock::default();
    let scope = clock
        .call_clock
        .enter(ids::GET_RANDOM_NUMBER, CallCeiling::new(32, 2).unwrap())
        .unwrap();
    let result = host.execute_fork(&mut fork, |machine| {
        crate::execution::run_original_timed_until(
            machine,
            &mut clock,
            unreachable_boundary(),
            Some(&branch),
        )
    });
    assert!(matches!(result, Err(crate::HostError::Clock(_))));
    assert_eq!(clock.call_clock.usage().ticks, 32);
    assert!(clock.call_clock.usage().instructions > 0);
    assert!(clock.call_clock.usage().master_clocks > 0);
    assert!(clock.call_clock.leave(scope).is_err());
    assert!(!clock.call_clock.active());
    assert!(host.merge_future_frame_branch(Some(branch)).is_err());
    assert!(matches!(
        host.future_frame_approval_outcome(approval),
        ApprovalOutcome::Invalidated { .. }
    ));
    let report = current_report(&host);
    let violation = &report["current"]["watches"]["receipts"][0]["outcome"]["violation"];
    assert_eq!(violation["checkpoint"]["origin"], "canonical_replay");
    assert_eq!(violation["origin"], "cpu_semantic");
    assert_eq!(host.checks.mismatches, 1);
}

#[test]
fn missing_inputs_and_partial_recipe_errors_keep_actual_progress_and_drain_partial_mutations() {
    use std::sync::Arc;
    use tje_game::execution::CallCeiling;
    struct FailingTimeline {
        partial_recipe: bool,
    }
    impl crate::execution::WaitInputPoller for FailingTimeline {
        fn at(&self, _: u64) -> crate::Result<genesis_config::GenesisInputs> {
            if self.partial_recipe {
                Ok(Default::default())
            } else {
                Err(crate::HostError::Input("authored missing sample".into()))
            }
        }
        fn before_frame(&self, _: u64, memory: &mut dyn tje_mem::Bus) -> crate::Result<()> {
            memory.write_u8(SLOT, 0);
            memory.write_u16(0xff9000, 1);
            Err(crate::HostError::Input("authored partial recipe".into()))
        }
    }
    let main = [
        0x4a79, 0x00ff, 0x9000, 0x67f8, 0x3039, 0x00ff, 0x7ffa, 0x60fe,
    ];
    // Disable further VBlank IRQs through the actual handler, not an observer
    // repair, so the next watched read is controlled only by the recipe flag.
    let handler = [0x33fc, 0x8100, 0x00c0, 4, 0x4e73];
    for kind in 0..3 {
        let mut host = authored_host(
            Revision::Rev02,
            &main,
            &handler,
            &[0x4e75],
            noop,
            ExecutionClass::Atomic,
        );
        let receipt = observed_receipt(&mut host);
        let approval = arm_component_requirement(&mut host, receipt);
        tick_until_rte(&mut host);
        let branch = host
            .future_frame_branch(Side::Original, ObservationOrigin::DependencyProbe)
            .unwrap();
        let mut fork = host.emulator.partial_clone();
        let mut clock = crate::execution::WaitClock::default();
        if kind != 0 {
            clock.poller = Some(Arc::new(FailingTimeline {
                partial_recipe: kind == 2,
            }));
        }
        let scope = clock
            .call_clock
            .enter(
                ids::GET_RANDOM_NUMBER,
                CallCeiling::new(100_000, 4).unwrap(),
            )
            .unwrap();
        let result = host.execute_fork(&mut fork, |machine| {
            crate::execution::run_original_timed_until(
                machine,
                &mut clock,
                unreachable_boundary(),
                Some(&branch),
            )
        });
        assert!(
            matches!(result, Err(crate::HostError::Input(_))),
            "{result:?}"
        );
        let usage = clock.call_clock.leave(scope).unwrap();
        assert!(usage.ticks > 0 && usage.instructions > 0 && usage.master_clocks > 0);
        assert_eq!(usage.frames, 1);
        assert_eq!(clock.frame, 1);
        assert!(
            fork.hybrid_bus()
                .hybrid_cpu_accesses
                .as_ref()
                .unwrap()
                .is_empty()
        );
        if kind == 2 {
            assert_eq!(fork.hybrid_bus().memory.read_main_ram_byte(SLOT), 0);
            // Continue the actual authored CPU on this same speculative fork.
            // The externally written equality is not a coupled game store.
            let mut clock = crate::execution::WaitClock::default();
            let scope = clock
                .call_clock
                .enter(ids::GET_RANDOM_NUMBER, CallCeiling::new(32, 2).unwrap())
                .unwrap();
            let result = host.execute_fork(&mut fork, |machine| {
                crate::execution::run_original_timed_until(
                    machine,
                    &mut clock,
                    unreachable_boundary(),
                    Some(&branch),
                )
            });
            assert!(matches!(result, Err(crate::HostError::Clock(_))));
            assert!(clock.call_clock.leave(scope).is_err());
            assert!(host.merge_future_frame_branch(Some(branch)).is_err());
            assert!(matches!(
                host.future_frame_approval_outcome(approval),
                ApprovalOutcome::Invalidated { .. }
            ));
            let report = current_report(&host);
            let violation = &report["current"]["watches"]["receipts"][0]["outcome"]["violation"];
            assert_eq!(violation["current_values"]["original"], 0);
            assert_eq!(
                violation["current_values"]["original_origin"],
                "external_recipe"
            );
            assert_eq!(
                violation["current_values"]["original_current_attested"],
                false
            );
        } else {
            host.merge_future_frame_branch(Some(branch)).unwrap();
            assert!(matches!(
                host.future_frame_approval_outcome(approval),
                ApprovalOutcome::Provisional { .. }
            ));
        }
    }
}

#[test]
fn finalized_zero_mismatch_campaign_is_pending_and_retains_full_private_evidence() {
    let mut host = authored_host(
        Revision::Rev02,
        &[0x60fe],
        &[0x4e73],
        &[0x4e75],
        noop,
        ExecutionClass::Atomic,
    );
    let receipt = observed_receipt(&mut host);
    arm_component_requirement(&mut host, receipt.clone());
    tick_until_rte(&mut host);
    let summary = host.finish_future_frame_provenance();
    assert_eq!(host.checks.mismatches, 0);
    assert_eq!(summary.status, ProvenanceStatus::Provisional);
    assert!(
        summary
            .ensure_proof_eligible()
            .unwrap_err()
            .to_string()
            .contains("pending")
    );
    let report = current_report(&host);
    let evidence = &report["current"]["watches"]["receipts"][0];
    assert_eq!(
        evidence["receipt"]["irq"]["interrupted_state"]
            .as_array()
            .unwrap()
            .len(),
        25
    );
    assert_eq!(
        evidence["receipt"]["bytes"][0]["original"],
        receipt.bytes()[0].original
    );
    assert_eq!(
        evidence["receipt"]["bytes"][0]["translated"],
        receipt.bytes()[0].translated
    );
    assert_eq!(
        evidence["observed_frame_producers"],
        serde_json::json!([SLOT])
    );
    assert_eq!(
        report["current"]["bindings"]["0"]["execution_class"],
        "atomic"
    );
    let binding = &report["current"]["bindings"]["0"];
    assert_eq!(binding["proof_envelope"], "ClassDefined");
    assert_eq!(binding["actual_check_route"], "FrozenAtomic");
    assert_eq!(binding["explicit_proof_only_admission"], false);
}

#[test]
fn producer_binding_rejects_changed_policy_before_creating_watch_debt() {
    let mut host = authored_host(
        Revision::Rev02,
        &[0x60fe],
        &[0x4e73],
        &[0x4e75],
        noop,
        ExecutionClass::Atomic,
    );
    let receipt = observed_receipt(&mut host);
    let active = host.registry.at(PRODUCER).unwrap().clone();
    let before = current_report(&host);
    for (class, envelope) in [
        (
            ExecutionClass::Atomic,
            tje_game::execution::ProofEnvelope::ClockedStrict,
        ),
        (
            ExecutionClass::Atomic,
            tje_game::execution::ProofEnvelope::ClockedCommutingInterrupts,
        ),
        (
            ExecutionClass::Functional,
            tje_game::execution::ProofEnvelope::ClassDefined,
        ),
    ] {
        let mut altered = active.clone();
        altered.execution_class = class;
        altered.proof_envelope = envelope;
        let error = host
            .arm_future_frame_watch(&altered, "wrong policy", 1, vec![receipt.clone()])
            .unwrap_err();
        assert!(error.to_string().contains("active hook policy"), "{error}");
        let report = current_report(&host);
        assert!(
            report["current"]["bindings"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        assert!(report["current"]["watches"].is_null());
        assert_eq!(report, before);
    }
    let approval = arm_component_requirement(&mut host, receipt);
    assert_eq!(approval.value(), 0);
}

#[test]
fn reset_preserves_invalidation_in_proof_eligibility_even_when_game_counters_restart() {
    let main = [
        0x4a79, 0x00ff, 0x9000, 0x67f8, 0x3039, 0x00ff, 0x7ffa, 0x60fe,
    ];
    let handler = [0x33fc, 1, 0x00ff, 0x9000, 0x4e73];
    let mut host = authored_host(
        Revision::Rev02,
        &main,
        &handler,
        &[0x4e75],
        noop,
        ExecutionClass::Atomic,
    );
    let receipt = observed_receipt(&mut host);
    arm_component_requirement(&mut host, receipt);
    tick_until_rte(&mut host);
    let mut rejected = false;
    for _ in 0..1000 {
        match host.tick_live_original(&mut Inputs(Default::default())) {
            Err(_) => {
                rejected = true;
                break;
            }
            Ok(TickEffect::FrameRendered) => {
                host.finish_live_frame().unwrap();
            }
            Ok(_) => {}
        }
    }
    assert!(rejected);
    assert_eq!(host.checks.mismatches, 1);
    host.reset();
    assert_eq!(host.checks.mismatches, 0);
    let summary = host.finish_future_frame_provenance();
    assert_eq!(summary.status, ProvenanceStatus::Invalidated);
    assert_eq!(summary.invalidated, 1);
    assert!(summary.ensure_proof_eligible().is_err());
    let report = current_report(&host);
    assert_eq!(report["previous"].as_array().unwrap().len(), 1);
    assert_eq!(
        report["previous"][0]["watches"]["receipts"][0]["outcome"]["status"],
        "invalidated"
    );
}
