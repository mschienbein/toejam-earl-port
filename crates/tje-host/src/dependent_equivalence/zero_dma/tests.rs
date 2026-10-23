use super::*;
use crate::{
    CheckReport, Mode,
    abi::{HookContext, canonical_ram_address},
    dependent_equivalence::DependentProofConfig,
    fixture::tests::synthetic_host,
    hooks::HookRegistry,
    stack_dependency::CheckSide,
};
use m68000_emu::ReadPurpose;
use tje_game::{FunctionId, execution::ExecutionClass};
use tje_mem::Revision;

fn never(_: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    Err(HostError::Emulator("authored early error".into()))
}
pub(super) fn fixture(revision: Revision, id: FunctionId) -> (Host, Hook) {
    let mut host = synthetic_host(&[0x60fe], &[0x4e73]);
    host.revision = revision;
    host.symbols = crate::catalogue::RevisionSymbols::for_revision(revision);
    host.mode = Mode::Check;
    let mut registry = HookRegistry::empty(revision);
    registry
        .register_execution_contract(id.metadata().clone(), ExecutionClass::Functional, never)
        .unwrap();
    let hook = registry
        .at(id.metadata().address(revision))
        .unwrap()
        .clone();
    host.checks = CheckReport::registered(&registry);
    host.registry = registry;
    (host, hook)
}
fn arguments(host: &mut Host, values: &[u32]) {
    let sp = host.cpu().stack_pointer();
    for (index, &value) in values.iter().enumerate() {
        host.context().write_u32(sp + 4 + 4 * index as u32, value);
    }
}

#[test]
fn platform_admits_only_ram_and_zero_encoded_word_counts_in_both_revisions() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (mut host, hook) = fixture(revision, ids::DMA_COPY_WORDS_TO_VRAM);
        for count in [0, 65536, 0xffff0000] {
            arguments(&mut host, &[0xffff_b8db, 0xc002, count]);
            let plan = ZeroDmaRoot::Platform.resolve(&mut host, &hook).unwrap();
            assert_eq!(plan.source, 0xffb8da);
            assert_eq!(plan.words(), 65536);
            assert_eq!(plan.address(0), 0xffb8da);
            assert_eq!(plan.address(32768), 0xfeb8da);
            assert_eq!(
                canonical_ram_address(plan.address(0)),
                canonical_ram_address(plan.address(32768))
            );
        }
        for count in [1, 2, 65535, 65537] {
            arguments(&mut host, &[0xff1000, 0x200, count]);
            assert!(
                ZeroDmaRoot::Platform
                    .resolve(&mut host, &hook)
                    .unwrap_err()
                    .to_string()
                    .contains("nonzero encoded")
            );
        }
        arguments(&mut host, &[0x100, 0xc002, 0]);
        assert!(ZeroDmaRoot::Platform.resolve(&mut host, &hook).is_err());
        let mut timed = hook.clone();
        timed.execution_class = ExecutionClass::TimeAware;
        assert!(ZeroDmaRoot::Platform.resolve(&mut host, &timed).is_err());
    }
}

#[test]
fn upload_replays_byte_bound_and_narrowed_run_encoding_without_a_table_prefix() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (mut host, hook) = fixture(revision, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS);
        arguments(&mut host, &[0, 1]);
        let runs = ram::g_sprite_tile_upload_runs(revision);
        for run in (0..=0xf000_u16).step_by(4096) {
            runs.at(0).write(&mut host.context(), 640);
            runs.at(1).write(&mut host.context(), run);
            ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut host.context(), 2);
            let plan = ZeroDmaRoot::QueuedSpriteGraphics
                .resolve(&mut host, &hook)
                .unwrap();
            assert_eq!(plan.words(), 65536);
            assert_eq!(plan.source, 0xffb8da);
        }
        runs.at(3).write(&mut host.context(), 0);
        ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut host.context(), 4);
        assert_eq!(
            ZeroDmaRoot::QueuedSpriteGraphics
                .resolve(&mut host, &hook)
                .unwrap()
                .words(),
            131072
        );
        runs.at(3).write(&mut host.context(), 1);
        assert!(
            ZeroDmaRoot::QueuedSpriteGraphics
                .resolve(&mut host, &hook)
                .unwrap_err()
                .to_string()
                .contains("nonzero or mixed")
        );
        ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut host.context(), 0);
        assert!(
            ZeroDmaRoot::QueuedSpriteGraphics
                .resolve(&mut host, &hook)
                .is_err()
        );
        arguments(&mut host, &[1, 0]);
        ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut host.context(), 2);
        assert!(
            ZeroDmaRoot::QueuedSpriteGraphics
                .resolve(&mut host, &hook)
                .is_err()
        );
        arguments(&mut host, &[0, 1]);
        ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut host.context(), 1);
        // Preserve the compiled unchecked odd-bound behavior: index zero still
        // consumes the following run count; do not silently validate it away.
        assert_eq!(
            ZeroDmaRoot::QueuedSpriteGraphics
                .resolve(&mut host, &hook)
                .unwrap()
                .transfers(),
            1
        );
    }
}

fn stream(plan: ZeroDmaPlan) -> AtomicRun {
    AtomicRun {
        stack_reads: Some(
            (0..plan.words())
                .map(|index| {
                    JournalRead::new(
                        0x100,
                        HybridAccess::DmaRead {
                            address: plan.address(index),
                            value: index as u16,
                        },
                        0,
                    )
                    .unwrap()
                })
                .collect(),
        ),
        ..AtomicRun::default()
    }
}
#[test]
fn full_dma_sequence_rejects_alias_value_order_origin_length_and_extra_rom_reads() {
    let plan = ZeroDmaPlan {
        source: 0xffb8da,
        transfers: 1,
    };
    let expected = stream(plan);
    let bytes = BTreeSet::new();
    plan.compare(&expected, &expected, &bytes).unwrap();
    assert_eq!(plan.summary().words_per_execution, 65536);
    for mutation in 0..6 {
        let mut changed = stream(plan);
        let events = changed.stack_reads.as_mut().unwrap();
        match mutation {
            0 => events[7].read.raw_address ^= 0x10000, // same canonical RAM, different raw alias
            1 => events[7].read.value ^= 1,
            2 => events.swap(7, 8),
            3 => events[7].read.origin = "cpu",
            4 => {
                events.pop();
            }
            _ => events.insert(
                0,
                JournalRead::new(
                    0x100,
                    HybridAccess::DmaRead {
                        address: 0x100,
                        value: 0,
                    },
                    0,
                )
                .unwrap(),
            ),
        }
        assert!(plan.compare(&expected, &changed, &bytes).is_err());
    }
    // Agreement between two wrong streams is not a complete source plan.
    let mut wrong = stream(plan);
    wrong.stack_reads.as_mut().unwrap().insert(
        0,
        JournalRead::new(
            0x100,
            HybridAccess::DmaRead {
                address: 0x100,
                value: 0,
            },
            0,
        )
        .unwrap(),
    );
    assert!(plan.compare(&wrong, &wrong, &bytes).is_err());
    let cpu_rom = JournalRead::new(
        0x100,
        HybridAccess::Read {
            address: 0x100,
            word: true,
            value: 0,
            purpose: ReadPurpose::Prefetch,
        },
        0,
    );
    assert!(cpu_rom.is_none());
}

#[test]
fn cpu_residual_control_and_post_store_cpu_reads_do_not_gain_dma_admission() {
    let plan = ZeroDmaPlan {
        source: 0xffb8da,
        transfers: 1,
    };
    let mut host = synthetic_host(&[0x60fe], &[0x4e73]);
    let expected = stream(plan);
    let mut changed = stream(plan);
    let mut cpu = JournalRead::new(
        0x100,
        HybridAccess::Read {
            address: 0xff1000,
            word: true,
            value: 0,
            purpose: ReadPurpose::Semantic,
        },
        3,
    )
    .unwrap();
    changed.stack_reads.as_mut().unwrap().insert(0, cpu.clone());
    assert!(
        plan.reject_feedback(&host.emulator, &changed)
            .unwrap_err()
            .contains("CPU residual")
    );
    changed.stack_reads.as_mut().unwrap().remove(0);
    cpu.uninitialized_mask = 0;
    changed.stack_reads.as_mut().unwrap().push(cpu);
    assert!(plan.reject_feedback(&host.emulator, &changed).is_ok());
    assert!(
        plan.compare(&expected, &changed, &BTreeSet::from([0xff1000, 0xff1001]),)
            .is_err()
    );
    host.emulator.hybrid_bus_mut().hybrid_accesses = Some(vec![HybridAccess::Read {
        address: 0xc00000,
        word: true,
        value: 0,
        purpose: ReadPurpose::Semantic,
    }]);
    assert!(
        plan.reject_feedback(&host.emulator, &expected)
            .unwrap_err()
            .contains("feedback")
    );
}

#[test]
fn host_activation_is_check_only_atomic_and_keeps_machine_and_old_coverage() {
    let (mut host, _) = fixture(Revision::Rev02, ids::DMA_COPY_WORDS_TO_VRAM);
    host.checks.calls = 7;
    host.checks.mismatches = 1;
    let before = bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap();
    host.mode = Mode::Reference;
    assert!(
        host.enable_dependent_equivalence(&[ids::DMA_COPY_WORDS_TO_VRAM], Default::default())
            .is_err()
    );
    host.mode = Mode::Check;
    assert!(
        host.enable_dependent_equivalence(
            &[ids::DMA_COPY_WORDS_TO_VRAM, ids::ABS],
            Default::default()
        )
        .is_err()
    );
    let address = ids::DMA_COPY_WORDS_TO_VRAM
        .metadata()
        .address(host.revision());
    assert!(host.registry.at(address).unwrap().dependent_proof.is_none());
    host.enable_dependent_equivalence(&[ids::DMA_COPY_WORDS_TO_VRAM], Default::default())
        .unwrap();
    assert_eq!((host.checks.calls, host.checks.mismatches), (7, 1));
    assert_eq!(
        before,
        bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap()
    );
}

#[test]
fn all_source_observer_restores_flag_and_trace_after_a_failed_root() {
    let (mut host, _) = fixture(Revision::Rev02, ids::DMA_COPY_WORDS_TO_VRAM);
    host.enable_dependent_equivalence(
        &[ids::DMA_COPY_WORDS_TO_VRAM],
        DependentProofConfig::default(),
    )
    .unwrap();
    let hook = host
        .registry
        .at(ids::DMA_COPY_WORDS_TO_VRAM
            .metadata()
            .address(host.revision()))
        .unwrap()
        .clone();
    let mut fork = host.fork();
    fork.hybrid_bus_mut().hybrid_dma_all_sources = false;
    fork.hybrid_bus_mut().hybrid_cpu_accesses = Some(vec![HybridAccess::DmaRead {
        address: 0x100,
        value: 0x1234,
    }]);
    assert!(
        host.run_check_side(&mut fork, &hook, CheckSide::Rust)
            .is_err()
    );
    assert!(!fork.hybrid_bus().hybrid_dma_all_sources);
    assert_eq!(
        fork.hybrid_bus().hybrid_cpu_accesses,
        Some(vec![HybridAccess::DmaRead {
            address: 0x100,
            value: 0x1234
        }])
    );
}
