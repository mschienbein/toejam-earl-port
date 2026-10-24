//! Authored, ROM-free reproductions of CR-01. No cartridge instructions/data.
use super::*;
use crate::{
    CheckReport, Mode,
    abi::{CALL_SENTINEL, StackAuditState, set_sp},
    catalogue,
    fixture::tests::synthetic_host,
    hooks::{HookFn, HookRegistry},
};
use genesis_core::HybridAccess;
use jgenesis_common::frontend::PartialClone;
use m68000_emu::BusInterface;
use tje_game::{FunctionMetadata, ids};
use tje_rom::Revision;

const PRODUCER: u32 = 0xfff000;
const CONSUMER: u32 = 0xfff080;
const CALLEE: u32 = 0xfff300;
const SOURCE: u32 = 0xff1000;
const DESTINATION: usize = 0x0200;
// One work-RAM word FF1000 -> VRAM 0200, display/DMA enabled.
const COMMANDS: [u16; 9] = [
    0x8114, 0x8f02, 0x9301, 0x9400, 0x9500, 0x9688, 0x977f, 0x4200, 0x0080,
];

fn omitted(_: &mut HookContext<'_>) -> Result<Option<u32>> {
    Ok(None)
}
fn producer(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    c.write_u16(SOURCE, 0x1234);
    Ok(None)
}
fn dma(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    for value in COMMANDS {
        c.bus().write_word(0xc00004, value);
    }
    Ok(None)
}
fn initialized_dma(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    producer(c)?;
    dma(c)
}
fn dma_then_store(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    dma(c)?;
    producer(c)
}
fn nested_dma(c: &mut HookContext<'_>) -> Result<Option<u32>> {
    c.call_original_with_execution_contract(
        &metadata(CALLEE, "AuthoredDmaCallee"),
        ExecutionClass::Functional,
        &[],
    )?;
    Ok(None)
}
fn metadata(address: u32, name: &'static str) -> FunctionMetadata {
    let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
    symbol.name = name;
    symbol.addresses = [address; 2];
    symbol.sizes = [96; 2];
    symbol.abi.argument_count = 0;
    symbol.abi.returns_value = false;
    symbol
}
fn code(host: &mut Host, address: u32, words: &[u16]) {
    for (index, &word) in words.iter().enumerate() {
        host.context().write_u16(address + 2 * index as u32, word);
    }
}
fn original_consumer(before: bool, after: bool) -> Vec<u16> {
    let mut words = Vec::new();
    if before {
        words.extend([0x33fc, 0x1234, 0x00ff, 0x1000]); // MOVE.W #1234,FF1000
    }
    for value in COMMANDS {
        words.extend([0x33fc, value, 0x00c0, 4]); // MOVE.W #value,C00004
    }
    if after {
        words.extend([0x33fc, 0x1234, 0x00ff, 0x1000]);
    }
    words.push(0x4e75); // RTS
    words
}
fn host(
    revision: Revision,
    class: ExecutionClass,
    producer: HookFn,
    consumer: HookFn,
    original: &[u16],
) -> Host {
    // JSR producer; JSR consumer; BRA.S *.
    let mut host = synthetic_host(
        &[0x4eb9, 0x00ff, 0xf000, 0x4eb9, 0x00ff, 0xf080, 0x60fe],
        &[0x4e73],
    );
    host.revision = revision;
    host.symbols = catalogue::RevisionSymbols::for_revision(revision);
    let mut registry = HookRegistry::empty(revision);
    registry
        .register_execution_contract(
            metadata(PRODUCER, "AuthoredDmaProducerA"),
            ExecutionClass::Atomic,
            producer,
        )
        .unwrap();
    registry
        .register_execution_contract(metadata(CONSUMER, "AuthoredDmaConsumerB"), class, consumer)
        .unwrap();
    host.checks = CheckReport::registered(&registry);
    host.registry = registry;
    host.mode = Mode::Check;
    code(
        &mut host,
        PRODUCER,
        &[0x33fc, 0x1234, 0x00ff, 0x1000, 0x4e75],
    );
    code(&mut host, CONSUMER, original);
    host
}
fn vram_word(host: &mut Host) -> u16 {
    let (vram, _, _) = host.emulator.hybrid_bus_mut().vdp.hybrid_memories();
    u16::from_be_bytes([vram[DESTINATION], vram[DESTINATION + 1]])
}

#[test]
fn dma_only_sequential_consumer_rejects_the_omitted_producer_in_both_revisions() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for class in [ExecutionClass::Functional, ExecutionClass::TimeAware] {
            for a in [omitted as HookFn, producer] {
                let mut check = host(revision, class, a, dma, &original_consumer(false, false));
                let error = check.run_frame(Default::default()).unwrap_err();
                assert_eq!(check.checks.calls, 2, "{revision:?} {class:?}: {error}");
                assert_eq!(check.checks.mismatches, 1);
                // Two physical bytes, each witnessed at original/Rust checkpoints.
                assert_eq!(check.checks.uninitialized_stack_reads.len(), 4);
                assert!(
                    check
                        .checks
                        .uninitialized_stack_reads
                        .iter()
                        .all(|site| { site.address == SOURCE || site.address == SOURCE + 1 })
                );
                assert!(
                    error.to_string().contains("uninitialized stack read:"),
                    "{error}"
                );
            }
            // Original-only audit still witnesses the physical producer through
            // DMA. The unapproved authored replacement is now rejected instead
            // of being permitted to demonstrate that known live divergence.
            for mode in [Mode::Reference, Mode::Replace] {
                let mut live = host(
                    revision,
                    class,
                    omitted,
                    dma,
                    &original_consumer(false, false),
                );
                live.mode = mode;
                if mode == Mode::Replace {
                    let before =
                        bincode::encode_to_vec(&live.emulator, bincode::config::standard())
                            .unwrap();
                    let error = live.run_frame(Default::default()).unwrap_err();
                    assert!(error.to_string().contains("canonical production bodies"));
                    assert_eq!(
                        before,
                        bincode::encode_to_vec(&live.emulator, bincode::config::standard())
                            .unwrap()
                    );
                    continue;
                }
                live.run_frame(Default::default()).unwrap();
                assert_eq!(
                    vram_word(&mut live),
                    0x1234,
                    "{revision:?} {class:?} {mode:?}"
                );
            }
        }
    }
}

#[test]
fn correct_direct_or_original_callee_stores_pass_but_read_before_later_write_rejects() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for class in [ExecutionClass::Functional, ExecutionClass::TimeAware] {
            for nested in [false, true] {
                for before in [false, true] {
                    let program = original_consumer(before, !before);
                    let root = if nested {
                        vec![0x4eb9, 0x00ff, 0xf300, 0x4e75]
                    } else {
                        program.clone()
                    };
                    let native = if nested {
                        nested_dma as HookFn
                    } else if before {
                        initialized_dma
                    } else {
                        dma_then_store
                    };
                    let mut check = host(revision, class, omitted, native, &root);
                    code(&mut check, CALLEE, &program);
                    let result = check.run_frame(Default::default());
                    assert_eq!(
                        result.is_ok(),
                        before,
                        "{revision:?} {class:?} nested={nested}: {result:?}"
                    );
                    assert_eq!(check.checks.calls, 2);
                    assert_eq!(check.checks.mismatches, u64::from(!before));
                    if before {
                        assert!(check.checks.uninitialized_stack_reads.is_empty());
                        assert_eq!(vram_word(&mut check), 0x1234);
                    }
                }
            }
            let mut omitted_own_store = host(
                revision,
                class,
                omitted,
                dma,
                &original_consumer(true, false),
            );
            assert!(omitted_own_store.run_frame(Default::default()).is_err());
            assert_eq!(omitted_own_store.checks.mismatches, 1);
        }
    }
}

#[test]
fn queued_cpu_store_does_not_retroactively_initialize_an_earlier_dma_read() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for commit_first in [false, true] {
            let mut check = host(revision, ExecutionClass::Functional, omitted, dma, &[]);
            let bus = check.emulator.hybrid_bus_mut();
            bus.hybrid_cpu_accesses = Some(Vec::new());
            // Start a real DMA without advancing it, then issue a queued RAM
            // store. Completing DMA before commit must still read the old word.
            for command in COMMANDS {
                bus.write_word(0xc00004, command);
                bus.apply_pending_writes();
            }
            bus.write_word(SOURCE, 0x1234);
            if commit_first {
                bus.apply_pending_writes();
            }
            bus.hybrid_set_functional_mode(true);
            bus.apply_pending_writes();
            let accesses = bus.hybrid_cpu_accesses.as_ref().unwrap();
            let read = accesses
                .iter()
                .position(|a| {
                    matches!(
                        a,
                        HybridAccess::DmaRead {
                            address: SOURCE,
                            ..
                        }
                    )
                })
                .unwrap();
            let write = accesses
                .iter()
                .position(|a| {
                    matches!(
                        a,
                        HybridAccess::Write {
                            address: SOURCE,
                            ..
                        }
                    )
                })
                .unwrap();
            assert_eq!(write < read, commit_first);
            let mut state = StackAuditState::new(catalogue::STACK_TOP - 4, true);
            let mut audit = state.empty_run();
            state.drain(bus, CONSUMER, &mut audit);
            assert_eq!(
                audit.uninitialized_reads.len(),
                if commit_first { 0 } else { 2 }
            );
            let reads = audit.stack_reads.as_ref().unwrap();
            let read = reads
                .iter()
                .find(|event| event.read.origin == "vdp_dma")
                .unwrap();
            assert_eq!(read.read.raw_address, SOURCE);
            assert_eq!(read.read.width, 2);
            assert_eq!(read.read.value, if commit_first { 0x1234 } else { 0 });
            assert_eq!(read.uninitialized_mask, if commit_first { 0 } else { 3 });
            assert_eq!(vram_word(&mut check), if commit_first { 0x1234 } else { 0 });
        }
    }
}

#[test]
fn timeaware_endpoint_normalization_is_inside_both_root_audits() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for initialized in [false, true] {
            let mut check = host(
                revision,
                ExecutionClass::TimeAware,
                omitted,
                if initialized { initialized_dma } else { dma },
                &original_consumer(initialized, false),
            );
            let sp = catalogue::STACK_TOP - 4;
            check.context().write_u32(sp, CALL_SENTINEL);
            let (cpu, bus) = check.emulator.hybrid_parts();
            set_sp(cpu, sp);
            cpu.set_pc(CONSUMER, bus);
            let hook = check.registry.at(CONSUMER).unwrap().clone();
            for side in [
                CheckSide::Rust,
                CheckSide::Original {
                    boundary: ReferenceBoundary::returned(CALL_SENTINEL, sp + 4),
                    outcome: tje_game::exits::EntryOutcome::Returned(None),
                },
            ] {
                let mut fork = check.emulator.partial_clone();
                let run = check
                    .run_check_side(
                        &mut fork,
                        &hook,
                        side,
                        crate::call_clock::ClockRunPurpose::Baseline,
                        crate::future_frame_watch::ObservationOrigin::CheckFork,
                    )
                    .unwrap();
                assert_eq!(
                    run.audit.uninitialized_reads.len(),
                    if initialized { 0 } else { 2 },
                    "{revision:?} initialized={initialized} {}",
                    side.name()
                );
                assert!(fork.hybrid_bus().hybrid_cpu_accesses.is_none());
                let (vram, _, _) = fork.hybrid_bus_mut().vdp.hybrid_memories();
                assert_eq!(
                    u16::from_be_bytes([vram[DESTINATION], vram[DESTINATION + 1]]),
                    if initialized { 0x1234 } else { 0 }
                );
            }
        }
    }
}
