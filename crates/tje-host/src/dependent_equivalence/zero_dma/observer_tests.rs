use super::tests::fixture;
use super::*;
use crate::{
    CheckReport, Mode, abi::HookContext, catalogue, dependent_equivalence::DependentProofConfig,
    fixture::tests::synthetic_host, hooks::HookRegistry, stack_dependency::CheckSide,
};
use m68000_emu::BusInterface;
use tje_mem::Revision;

fn configure_rom_dma(host: &mut Host) {
    let bus = host.emulator.hybrid_bus_mut();
    for word in [
        0x8114, 0x8f02, 0x9301, 0x9400, 0x9580, 0x9600, 0x9700, 0x4000, 0x0080,
    ] {
        bus.write_word(0xc00004, word);
        bus.apply_pending_writes();
    }
}
#[test]
fn all_source_logging_keeps_actual_rom_reads_without_changing_device_results() {
    for all_sources in [false, true] {
        let mut host = synthetic_host(&[0x60fe], &[0x4e73]);
        let bus = host.emulator.hybrid_bus_mut();
        assert!(!bus.hybrid_dma_all_sources);
        bus.hybrid_dma_all_sources = all_sources;
        bus.hybrid_cpu_accesses = Some(Vec::new());
        bus.hybrid_set_functional_mode(true);
        configure_rom_dma(&mut host);
        let bus = host.emulator.hybrid_bus();
        let words: Vec<_> = bus
            .hybrid_cpu_accesses
            .as_ref()
            .unwrap()
            .iter()
            .filter_map(|access| {
                if let HybridAccess::DmaRead { address, value } = access {
                    Some((*address, *value))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            words,
            if all_sources {
                vec![(0x100, 0x60fe)]
            } else {
                Vec::new()
            }
        );
        let (vram, _, _) = bus.vdp.hybrid_memories();
        assert_eq!(&vram[..2], &[0x60, 0xfe]);
    }
}

fn no_body(_: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    Ok(None)
}
#[test]
fn complete_source_scope_covers_pre_entry_normalization_and_restores_success() {
    let (mut host, _) = fixture(Revision::Rev02, ids::DMA_COPY_WORDS_TO_VRAM);
    let mut registry = HookRegistry::empty(host.revision());
    registry
        .register_execution_contract(
            ids::DMA_COPY_WORDS_TO_VRAM.metadata().clone(),
            ExecutionClass::Functional,
            no_body,
        )
        .unwrap();
    registry
        .enable_dependent_equivalence(
            &[ids::DMA_COPY_WORDS_TO_VRAM],
            DependentProofConfig::default(),
        )
        .unwrap();
    host.registry = registry;
    let hook = host
        .registry
        .at(ids::DMA_COPY_WORDS_TO_VRAM
            .metadata()
            .address(host.revision()))
        .unwrap()
        .clone();
    configure_rom_dma(&mut host); // pending real ROM-source transfer, still timed
    let mut fork = host.fork();
    fork.hybrid_bus_mut().hybrid_cpu_accesses = Some(vec![HybridAccess::DmaRead {
        address: 0x200,
        value: 0x4e73,
    }]);
    let run = host
        .run_check_side(
            &mut fork,
            &hook,
            CheckSide::Rust,
            crate::call_clock::ClockRunPurpose::Baseline,
        )
        .unwrap();
    assert!(
        run.audit
            .stack_reads
            .as_ref()
            .unwrap()
            .iter()
            .any(|event| event.read.origin == "vdp_dma"
                && event.read.raw_address == 0x100
                && event.read.value == 0x60fe)
    );
    assert!(!fork.hybrid_bus().hybrid_dma_all_sources);
    assert_eq!(
        fork.hybrid_bus().hybrid_cpu_accesses,
        Some(vec![HybridAccess::DmaRead {
            address: 0x200,
            value: 0x4e73
        }])
    );
}

const SOURCE: u32 = 0xff1000;
const PRODUCER: u32 = 0xfff000;
const COMMANDS: [u16; 9] = [
    0x8114, 0x8f02, 0x9301, 0x9400, 0x9500, 0x9688, 0x977f, 0x4200, 0x0080,
];
fn omitted(_: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    Ok(None)
}
fn consumer(c: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    for value in COMMANDS {
        c.bus().write_word(0xc00004, value);
    }
    Ok(None)
}
fn cr01(revision: Revision, class: ExecutionClass, marked: bool) -> Host {
    let target = ids::DMA_COPY_WORDS_TO_VRAM.metadata().address(revision);
    // Authored JSR producer; three actual argument slots; JSR consumer; cleanup;
    // BRA.S *. The consumer programs one word, not the admitted zero encoding.
    let mut main = vec![0x4e71; ((target - 0x100) / 2) as usize];
    let program = [
        0x4eb9,
        0x00ff,
        0xf000,
        0x2f3c,
        0,
        1,
        0x2f3c,
        0,
        0x0200,
        0x2f3c,
        0x00ff,
        0x1000,
        0x4eb9,
        (target >> 16) as u16,
        target as u16,
        0x4fef,
        12,
        0x60fe,
    ];
    main[..program.len()].copy_from_slice(&program);
    for value in COMMANDS {
        main.extend([0x33fc, value, 0x00c0, 4]);
    }
    main.push(0x4e75);
    let mut host = synthetic_host(&main, &[0x4e73]);
    host.revision = revision;
    host.symbols = catalogue::RevisionSymbols::for_revision(revision);
    host.mode = Mode::Check;
    let mut producer = ids::GET_RANDOM_NUMBER.metadata().clone();
    producer.name = "OmittedDmaProducer";
    producer.addresses = [PRODUCER; 2];
    producer.abi.argument_count = 0;
    producer.abi.returns_value = false;
    let mut registry = HookRegistry::empty(revision);
    registry.register_contract(producer, true, omitted).unwrap();
    registry
        .register_execution_contract(
            ids::DMA_COPY_WORDS_TO_VRAM.metadata().clone(),
            class,
            consumer,
        )
        .unwrap();
    host.checks = CheckReport::registered(&registry);
    host.registry = registry;
    for (index, word) in [0x33fc, 0x1234, 0x00ff, 0x1000, 0x4e75]
        .into_iter()
        .enumerate()
    {
        host.context().write_u16(PRODUCER + 2 * index as u32, word);
    }
    if marked {
        host.enable_dependent_equivalence(&[ids::DMA_COPY_WORDS_TO_VRAM], Default::default())
            .unwrap();
    }
    host
}
#[test]
fn cr01_omitted_store_never_gains_admission_from_the_zero_only_descriptor() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for class in [ExecutionClass::Functional, ExecutionClass::TimeAware] {
            let mut strict = cr01(revision, class, false);
            let error = strict.run_frame(Default::default()).unwrap_err();
            assert!(
                error.to_string().contains("uninitialized stack read"),
                "{revision:?} {class:?}: {error}"
            );
            assert_eq!(strict.checks.calls, 2);
            assert_eq!(strict.checks.mismatches, 1);
            let mut marked = cr01(revision, class, true);
            let error = marked.run_frame(Default::default()).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(if class == ExecutionClass::Functional {
                        "nonzero encoded"
                    } else {
                        "frozen Functional"
                    }),
                "{error}"
            );
            assert_eq!(marked.checks.calls, 1); // only A checked, B explicitly refused
            assert!(marked.checks.dependent_equivalence.is_empty());
        }
    }
}

#[test]
fn nonzero_live_attempt_is_rejected_before_any_side_executes() {
    let (mut host, _) = fixture(Revision::Rev02, ids::DMA_COPY_WORDS_TO_VRAM);
    host.enable_dependent_equivalence(&[ids::DMA_COPY_WORDS_TO_VRAM], Default::default())
        .unwrap();
    let error = host
        .check_fixture(
            ids::DMA_COPY_WORDS_TO_VRAM
                .metadata()
                .address(host.revision()),
            &[SOURCE, 0x200, 1],
        )
        .unwrap_err();
    assert!(error.to_string().contains("nonzero encoded"), "{error}");
    assert_eq!(host.checks.calls, 0);
    assert_eq!(host.checks.stack_dependency_reruns, 0);
    assert!(host.checks.dependent_equivalence.is_empty());
    // No altered argument area or caller was left behind by the refused fixture.
    assert_eq!(host.cpu().stack_pointer(), catalogue::STACK_TOP);
    assert_eq!(host.cpu().pc(), 0x100);
}
