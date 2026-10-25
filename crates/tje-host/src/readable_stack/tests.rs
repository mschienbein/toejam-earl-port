use super::*;
use crate::{
    CheckReport, Mode,
    abi::HookContext,
    fixture::tests::synthetic_host,
    hooks::{HookFn, HookRegistry},
};
use tje_game::{
    Context, FunctionId,
    call_frame::{FrameLayout, LocalFrame},
    execution::{ExecutionClass, ProofEnvelope},
    ids,
    stack_read::{LocalWidth, Reader, UnsupportedReason},
};
use tje_mem::{Revision, ram};
const CODE: u32 = 0xfff000;
const S: u32 = 0xff7f00;

fn production(revision: Revision, id: FunctionId) -> Host {
    let mut host = synthetic_host(&[0x60fe], &[0x4e73]);
    host.revision = revision;
    host.symbols = crate::catalogue::RevisionSymbols::for_revision(revision);
    host.mode = Mode::Check;
    host.registry = HookRegistry::filtered(revision, &[], &[id]).unwrap();
    host.checks = CheckReport::registered(&host.registry);
    host.settle_fixture_boundary(Default::default()).unwrap();
    host
}
fn prepare(host: &mut Host, entry_sp: u32, arguments: usize) {
    host.context()
        .set_stack_pointer(entry_sp + 4 + 4 * arguments as u32)
        .unwrap();
}
fn unsupported(
    host: &mut Host,
    id: FunctionId,
    arguments: &[u32],
) -> tje_game::stack_read::Unsupported {
    let before = (host.checks.calls, host.checks.mismatches);
    let error = host
        .check_fixture(id.metadata().address(host.revision), arguments)
        .unwrap_err();
    assert_eq!(
        (host.checks.calls, host.checks.mismatches),
        (before.0 + 1, before.1 + 1)
    );
    error
        .typed_stack_read()
        .expect("typed readable guard failure, not a comparison/string fallback")
}

#[test]
fn reviewed_rectangle_and_cram_sources_fail_before_serving_saved_word_residue() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for id in [
            ids::UPLOAD_VRAM_WORD_RECTANGLE,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
        ] {
            let mut host = production(revision, id);
            let sp = 0xfffe00;
            let args: Vec<_> = if id == ids::UPLOAD_VRAM_WORD_RECTANGLE {
                vec![0, sp - 8, 1, 1, 0]
            } else {
                vec![sp - 8, 0]
            };
            prepare(&mut host, sp, args.len());
            host.context().write_u16(sp - 8, 0xabcd);
            host.context().set_data_register(2, 0x12345678);
            let failure = unsupported(&mut host, id, &args);
            assert_eq!(failure.root, id);
            assert_eq!(failure.address, sp - 8);
            assert_eq!(failure.width, 2);
            assert_eq!(failure.reader, Reader::RustMemory);
        }
    }
}

#[test]
fn reviewed_sprite_pointer_and_speech_signed_handle_reads_fail_closed() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = production(revision, ids::FREE_SPRITE_SLOTS);
        let sp = 0xffff00;
        prepare(&mut host, sp, 2);
        host.context().write_u8(sp - 12, 26);
        host.context().set_data_register(2, 0x19000000);
        let failure = unsupported(&mut host, ids::FREE_SPRITE_SLOTS, &[1, sp - 12]);
        assert_eq!((failure.address, failure.width), (sp - 12, 1));
        let mut host = production(revision, ids::HAS_SPEAKER_SPEECH);
        prepare(&mut host, 0xff7f0c, 1);
        ram::g_speech_bubble_entities(revision)
            .at(0)
            .write(&mut host.context(), -71);
        let failure = unsupported(&mut host, ids::HAS_SPEAKER_SPEECH, &[0]);
        assert_eq!((failure.address, failure.width), (0xff7f09, 1));
    }
}

#[test]
fn reviewed_ship_and_jam_aliases_never_consume_omitted_save_bytes() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = production(revision, ids::SHIP_UPDATE_PENDING_PIECE_INDICATOR);
        prepare(&mut host, 0xff7ee0, 0);
        ram::g_ship_piece_indicator_entity(revision).write(&mut host.context(), -71);
        let failure = unsupported(&mut host, ids::SHIP_UPDATE_PENDING_PIECE_INDICATOR, &[]);
        assert_eq!((failure.address, failure.width), (0xff7edc, 2));
        let mut host = production(revision, ids::JAM_OUT_UPDATE_LEAD_ANIMATIONS);
        prepare(&mut host, 0xff7f0e, 0);
        ram::g_jam_toejam_entities(revision)
            .at(0)
            .write(&mut host.context(), -71);
        let failure = unsupported(&mut host, ids::JAM_OUT_UPDATE_LEAD_ANIMATIONS, &[]);
        assert_eq!((failure.address, failure.width), (0xff7f02, 1));
        for (id, slot) in [
            (ids::JAM_OUT_POSITION_TOEJAM_COPY, -12945_i32),
            (ids::JAM_OUT_POSITION_EARL_COPY, -12954),
        ] {
            let mut host = production(revision, id);
            prepare(&mut host, S, 3);
            host.context().write_u16(S - 8, 1);
            let failure = unsupported(&mut host, id, &[slot as u32, 123, 456]);
            assert_eq!((failure.address, failure.width), (S - 8, 2));
        }
    }
}

fn generic_read(context: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    Ok(Some(u32::from(context.read_u16(S - 8))))
}
fn authored(revision: Revision, body: HookFn, words: &[u16]) -> Host {
    authored_class(revision, body, words, ExecutionClass::Atomic)
}
fn authored_class(revision: Revision, body: HookFn, words: &[u16], class: ExecutionClass) -> Host {
    let mut host = synthetic_host(&[0x60fe], &[0x4e73]);
    host.revision = revision;
    host.symbols = crate::catalogue::RevisionSymbols::for_revision(revision);
    host.mode = Mode::Check;
    let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
    symbol.name = "AuthoredReadableStackRoot";
    symbol.addresses = [CODE; 2];
    symbol.sizes = [128; 2];
    symbol.abi.argument_count = 0;
    let mut registry = HookRegistry::empty(revision);
    registry
        .register_execution_contract(symbol, class, body)
        .unwrap();
    host.checks = CheckReport::registered(&registry);
    host.registry = registry;
    host.settle_fixture_boundary(Default::default()).unwrap();
    for (index, word) in words.iter().copied().enumerate() {
        host.context().write_u16(CODE + 2 * index as u32, word);
    }
    prepare(&mut host, S, 0);
    host
}

fn cited(
    context: &mut HookContext<'_>,
    width: LocalWidth,
    mixed: bool,
    generic_again: bool,
) -> crate::Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(8, 0, 0))?;
    if mixed {
        frame.local::<u8>(-3).write(context, 0x7f);
    }
    let request = context.frame_state().unwrap().uninitialized_local(
        ids::GET_RANDOM_NUMBER,
        -8,
        width,
        "authored LINK -8; exact local entry-8 read width",
    );
    let value = context.read_uninitialized_local(request)?;
    if generic_again {
        let _ = context.read_u16(S - 8);
    }
    let mask = if width == LocalWidth::Long {
        0x80000000
    } else {
        0x8000
    };
    let sr = context.status_register();
    context.set_status_register(
        (sr & !15)
            | if value == 0 {
                4
            } else if value & mask != 0 {
                8
            } else {
                0
            },
    );
    frame.leave(context)?;
    Ok(Some(value))
}
fn cited_word(c: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    cited(c, LocalWidth::Word, false, false)
}
fn cited_long(c: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    cited(c, LocalWidth::Long, false, false)
}
fn cited_mixed(c: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    cited(c, LocalWidth::Word, true, false)
}
fn uncited_again(c: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    cited(c, LocalWidth::Word, false, true)
}

#[test]
fn explicit_root_inputs_pair_real_word_long_and_mixed_original_occurrences() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (body, words, value, expected_mask, beats) in [
            (
                cited_word as HookFn,
                vec![0x4e56, 0xfff8, 0x302e, 0xfffc, 0x4e5e, 0x4e75],
                0x4234,
                3,
                1,
            ),
            (
                cited_long as HookFn,
                vec![0x4e56, 0xfff8, 0x202e, 0xfffc, 0x4e5e, 0x4e75],
                0x12345678,
                15,
                2,
            ),
            (
                cited_mixed as HookFn,
                vec![
                    0x4e56, 0xfff8, 0x1d7c, 0x007f, 0xfffd, 0x302e, 0xfffc, 0x4e5e, 0x4e75,
                ],
                0x4234,
                1,
                1,
            ),
        ] {
            for class in [
                ExecutionClass::Atomic,
                ExecutionClass::Functional,
                ExecutionClass::TimeAware,
            ] {
                let mut host = authored_class(revision, body, &words, class);
                if beats == 2 {
                    host.context().write_u32(S - 8, value);
                } else {
                    host.context().write_u16(S - 8, value as u16);
                }
                host.check_fixture(CODE, &[]).unwrap();
                assert_eq!(host.checks.mismatches, 0);
                let records = &host.checks.declared_stack_inputs;
                assert_eq!(records.len(), beats);
                assert_eq!(records[0].unwritten_mask, expected_mask & 3);
                assert!(
                    records.iter().all(|record| record.physical_width == 2
                        && record.citation.contains("exact local"))
                );
            }
        }
    }
}

#[test]
fn second_uncited_original_or_generic_rust_read_does_not_inherit_permission() {
    let original_twice = [
        0x4e56, 0xfff8, 0x302e, 0xfffc, 0x4a6e, 0xfffc, 0x4e5e, 0x4e75,
    ];
    let mut host = authored(Revision::Rev02, cited_word, &original_twice);
    let error = host
        .check_fixture(CODE, &[])
        .unwrap_err()
        .typed_stack_read()
        .unwrap();
    assert_eq!(error.reason, UnsupportedReason::UncitedInputDependence);
    let mut host = authored(Revision::Rev02, uncited_again, &original_twice);
    let error = host
        .check_fixture(CODE, &[])
        .unwrap_err()
        .typed_stack_read()
        .unwrap();
    assert_eq!(error.reason, UnsupportedReason::Unwritten);
    assert_eq!(error.reader, Reader::RustMemory);
}

#[test]
fn original_store_cannot_be_repaired_by_declaring_a_bug_read() {
    let words = [
        0x4e56, 0xfff8, 0x3d7c, 0, 0xfffc, 0x302e, 0xfffc, 0x4e5e, 0x4e75,
    ];
    let mut host = authored(Revision::Rev02, cited_word, &words);
    let error = host
        .check_fixture(CODE, &[])
        .unwrap_err()
        .typed_stack_read()
        .unwrap();
    assert_eq!(error.reason, UnsupportedReason::PairedInputMismatch);
}

#[test]
fn generic_guard_applies_to_strict_and_each_transitional_execution_class() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for class in [
            ExecutionClass::Atomic,
            ExecutionClass::Functional,
            ExecutionClass::TimeAware,
        ] {
            for envelope in [ProofEnvelope::ClassDefined, ProofEnvelope::ClockedStrict] {
                let mut host = authored(revision, generic_read, &[0x4e75]);
                let mut hook = host.registry.at(CODE).unwrap().clone();
                hook.execution_class = class;
                hook.proof_envelope = envelope;
                let mut fork = host.fork();
                let error = host
                    .run_check_side(
                        &mut fork,
                        &hook,
                        crate::stack_dependency::CheckSide::Rust,
                        crate::call_clock::ClockRunPurpose::Baseline,
                        crate::future_frame_watch::ObservationOrigin::CheckFork,
                    )
                    .err()
                    .expect("the guard must reject before a Rust operand is served")
                    .typed_stack_read()
                    .unwrap();
                assert_eq!(
                    (error.address, error.width, error.reader),
                    (S - 8, 2, Reader::RustMemory)
                );
            }
        }
    }
}

// Authored issuing scope; the actual VDP reads only when subsequent ticks run.
fn retained_dma(host: &mut Host, source: u32) {
    use m68000_emu::BusInterface;
    host.mode = Mode::Replace;
    let sp = host.cpu().stack_pointer();
    let observer = host
        .readable_stack_for_root(ids::DMA_COPY_WORDS_TO_VRAM, sp)
        .unwrap();
    let previous = host
        .emulator
        .hybrid_bus_mut()
        .replace_ram_dma_observer(Some(observer.clone()));
    observer
        .guard
        .run_root(ids::DMA_COPY_WORDS_TO_VRAM, sp, || {
            for value in [
                0x8114,
                0x8f02,
                0x9301,
                0x9400,
                0x9500 | ((source >> 1) & 255) as u16,
                0x9600 | ((source >> 9) & 255) as u16,
                0x9700 | ((source >> 17) & 127) as u16,
                0x4000,
                0x0080,
            ] {
                let bus = host.emulator.hybrid_bus_mut();
                bus.write_word(0xc00004, value);
                bus.apply_pending_writes();
            }
        })
        .unwrap();
    host.emulator
        .hybrid_bus_mut()
        .restore_ram_dma_observer(previous);
    assert_eq!(
        host.readable_stack
            .as_ref()
            .unwrap()
            .namespace
            .metrics()
            .active,
        1
    );
}

#[test]
fn replacement_tick_converts_only_the_retained_issuing_scope_failure() {
    let mut host = authored(Revision::Rev02, generic_read, &[0x4e75]);
    retained_dma(&mut host, S - 64);
    let mut failure = None;
    let mut inputs = crate::sinks::Inputs(genesis_core::GenesisInputs::default());
    for _ in 0..1024 {
        if let Err(error) = host.tick(&mut inputs) {
            failure = Some(error);
            break;
        }
    }
    let failure = failure
        .expect("actual late DMA source read must fail")
        .typed_stack_read()
        .unwrap();
    assert_eq!(
        (failure.root, failure.address, failure.reader),
        (ids::DMA_COPY_WORDS_TO_VRAM, S - 64, Reader::VdpDma)
    );
    assert_eq!(failure.reason, UnsupportedReason::Unwritten);
    let repeated = host
        .tick(&mut inputs)
        .unwrap_err()
        .typed_stack_read()
        .unwrap();
    assert_eq!(failure, repeated);
}

#[test]
fn pending_replacement_dma_rejects_fixture_before_clone_or_any_effect() {
    let mut host = authored(Revision::Rev02, generic_read, &[0x4e75]);
    retained_dma(&mut host, S - 64);
    let before_cpu = host.cpu().hybrid_state_words();
    let before_clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
    let before =
        bincode::encode_to_vec(&host.emulator.hybrid_bus().vdp, bincode::config::standard())
            .unwrap();
    let mut ran = false;
    let error = host
        .isolated_fixture(|_| {
            ran = true;
            Ok(())
        })
        .unwrap_err()
        .typed_stack_read()
        .unwrap();
    assert!(!ran);
    assert_eq!(error.root, ids::DMA_COPY_WORDS_TO_VRAM);
    assert_eq!(error.reason, UnsupportedReason::PendingOwnedDma);
    assert_eq!(host.cpu().hybrid_state_words(), before_cpu);
    assert_eq!(
        host.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
        before_clock
    );
    assert_eq!(
        bincode::encode_to_vec(&host.emulator.hybrid_bus().vdp, bincode::config::standard())
            .unwrap(),
        before
    );
    let metrics = host.readable_stack.as_ref().unwrap().namespace.metrics();
    assert_eq!(metrics.active, 1);
    assert_eq!(metrics.discarded, 0);
    assert_eq!(metrics.completed, 0);
    assert_eq!(metrics.pending_proof, 1);
}

#[test]
fn fixture_is_available_after_actual_transfer_completion_or_reset_invalidation() {
    for reset in [false, true] {
        let mut host = authored(Revision::Rev02, generic_read, &[0x4e75]);
        retained_dma(&mut host, 0xfff000);
        if reset {
            host.reset();
        } else {
            let mut inputs = crate::sinks::Inputs(genesis_core::GenesisInputs::default());
            for _ in 0..1024 {
                host.tick(&mut inputs).unwrap();
                if !host
                    .readable_stack
                    .as_ref()
                    .unwrap()
                    .namespace
                    .has_retained_dma()
                {
                    break;
                }
            }
        }
        let metrics = host.readable_stack.as_ref().unwrap().namespace.metrics();
        assert_eq!(metrics.active, 0);
        assert_eq!(metrics.completed, u64::from(!reset));
        assert_eq!(metrics.reset_invalidated, u64::from(reset));
        assert_eq!(host.isolated_fixture(|_| Ok(7)).unwrap(), 7);
    }
}

#[test]
fn direct_and_continued_early_guard_failures_count_once_even_when_abort_is_caught() {
    fn caught(context: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
        let _ignored =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| context.read_u16(S - 8)));
        Ok(Some(0))
    }
    for body in [generic_read as HookFn, caught] {
        let mut direct = authored(Revision::Rev02, body, &[0x4e75]);
        let error = direct
            .check_fixture(CODE, &[])
            .unwrap_err()
            .typed_stack_read()
            .unwrap();
        assert_eq!(error.reason, UnsupportedReason::Unwritten);
        assert_eq!((direct.checks.calls, direct.checks.mismatches), (1, 1));
        assert_eq!(
            direct.checks.by_function["AuthoredReadableStackRoot"].mismatches,
            1
        );
        let mut continued = authored(Revision::Rev02, body, &[0x4e75]);
        continued
            .set_check_failure_policy(crate::CheckFailurePolicy::ContinueOriginal)
            .unwrap();
        let hook = continued.registry.at(CODE).unwrap().clone();
        let before = continued.cpu().hybrid_state_words();
        continued.check_live_hook(&hook).unwrap();
        assert_eq!(
            (continued.checks.calls, continued.checks.mismatches),
            (1, 1)
        );
        assert_eq!(continued.checks.failures.len(), 1);
        assert_eq!(
            continued.checks.failures[0]
                .unsupported_stack_read
                .unwrap()
                .reason,
            UnsupportedReason::Unwritten.as_str()
        );
        assert_eq!(continued.cpu().hybrid_state_words(), before);
        assert!(continued.disabled_check_roots.contains(&CODE));
    }
}

#[test]
fn recursive_and_shared_code_children_cannot_supply_a_root_input_witness() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for class in [
            ExecutionClass::Atomic,
            ExecutionClass::Functional,
            ExecutionClass::TimeAware,
        ] {
            for child in [CODE, CODE + 14] {
                // Root calls itself or a shared entry inside its symbol range.
                // The child reads entry SP-4, which aliases root entry SP-8.
                let words = [
                    0x4a40,
                    0x660a,
                    0x7001,
                    0x4eb9,
                    (child >> 16) as u16,
                    child as u16,
                    0x4e75,
                    0x302f,
                    0xfffc,
                    0x4e75,
                ];
                let mut host = authored_class(revision, cited_word, &words, class);
                host.context().write_u16(S - 8, 0x4234);
                host.context().set_data_register(0, 0);
                let error = host
                    .check_fixture(CODE, &[])
                    .unwrap_err()
                    .typed_stack_read()
                    .unwrap();
                assert_eq!(error.reason, UnsupportedReason::MissingOriginalFrameWitness);
                assert!(host.checks.declared_stack_inputs.is_empty());
                assert_eq!((host.checks.calls, host.checks.mismatches), (1, 1));
            }
        }
    }
}

#[test]
fn vendor_odd_hardware_word_checks_actual_bytes_before_fetch() {
    fn bad(context: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
        context.write_u8(S - 7, 0xcd);
        context.write_u8(S - 6, 0xef);
        let _ = tje_game::Platform::read_device_u16(context, S - 7);
        Ok(None)
    }
    fn valid(context: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
        context.write_u16(S - 8, 0xabcd);
        assert_eq!(tje_game::Platform::read_device_u16(context, S - 7), 0xabcd);
        Ok(None)
    }
    let mut host = authored(Revision::Rev02, bad, &[0x4e75]);
    let error = host
        .check_fixture(CODE, &[])
        .unwrap_err()
        .typed_stack_read()
        .unwrap();
    assert_eq!(
        (error.raw_address, error.address, error.width),
        (S - 7, S - 8, 2)
    );
    assert_eq!(error.reason, UnsupportedReason::Unwritten);
    let mut host = authored(Revision::Rev02, valid, &[0x4e75]);
    host.check_fixture(CODE, &[]).unwrap();
}

#[test]
fn m68k_forwarded_z80_window_dma_keeps_the_actual_issuing_root() {
    use m68000_emu::BusInterface;
    let mut host = authored(Revision::Rev02, generic_read, &[0x4e75]);
    host.mode = Mode::Replace;
    let source = S - 64;
    let sp = host.cpu().stack_pointer();
    let observer = host
        .readable_stack_for_root(ids::DMA_COPY_WORDS_TO_VRAM, sp)
        .unwrap();
    let previous = host
        .emulator
        .hybrid_bus_mut()
        .replace_ram_dma_observer(Some(observer.clone()));
    observer
        .guard
        .run_root(ids::DMA_COPY_WORDS_TO_VRAM, sp, || {
            let bus = host.emulator.hybrid_bus_mut();
            for value in [
                0x8114,
                0x8f02,
                0x9301,
                0x9400,
                0x9500 | ((source >> 1) & 255) as u16,
                0x9600 | ((source >> 9) & 255) as u16,
                0x9700 | ((source >> 17) & 127) as u16,
                0x4000,
            ] {
                bus.write_word(0xc00004, value);
                bus.apply_pending_writes();
            }
            bus.write_word(0xa11200, 0x0100);
            bus.write_word(0xa11100, 0x0100);
            bus.apply_pending_writes();
            bus.write_byte(0xa07f04, 0x80);
            bus.apply_pending_writes();
        })
        .unwrap();
    host.emulator
        .hybrid_bus_mut()
        .restore_ram_dma_observer(previous);
    assert_eq!(
        host.readable_stack
            .as_ref()
            .unwrap()
            .namespace
            .metrics()
            .active,
        1
    );
    let mut failure = None;
    for _ in 0..1024 {
        if let Err(error) = host.tick(&mut crate::sinks::Inputs(Default::default())) {
            failure = Some(error);
            break;
        }
    }
    let error = failure
        .expect("forwarded memory DMA source must be guarded")
        .typed_stack_read()
        .unwrap();
    assert_eq!(
        (error.root, error.address, error.reader),
        (ids::DMA_COPY_WORDS_TO_VRAM, source, Reader::VdpDma)
    );
}
