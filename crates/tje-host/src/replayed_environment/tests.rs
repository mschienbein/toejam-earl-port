//! Entirely authored instruction streams. Metadata supplies only public ids and
//! entry addresses; no cartridge instructions, RAM snapshots or recordings.
use super::*;
use crate::{CheckFailurePolicy, CheckReport, Host, Mode, hooks::HookRegistry, sinks::Inputs};
use genesis_config::GenesisInputs;
use jgenesis_common::frontend::TickEffect;
use m68000_emu::BusInterface;
use std::cell::Cell;
use tje_game::{Context, FramedTranslation, Translation, call_void, execution::WaitRequest, ids};

pub(in crate::replayed_environment) mod causal_inputs;
mod explicit_boundaries;
mod frame_causality;
mod irq_flags;
mod live_routes;
mod panic_cleanup;
mod publication_cleanup;
mod scalar_transport;
mod tail_bounds;

// Larger committed extent for these authored streams; this is NOT the owner's
// readable body, a cartridge proof, promotion, or an extent override.
const ROOT: FunctionId = ids::UPDATE_CAMERA_FOLLOWING;
const CHILD: FunctionId = ids::PROCESS_INPUT;
const VALUE: u32 = 0xff9000;
const FLAG: u32 = 0xff9001;
const RESULT: u32 = 0xff9002;
const IRQ_VISIBLE: u32 = 0xff9004;

fn install(rom: &mut [u8], address: u32, words: &[u16]) {
    for (index, word) in words.iter().enumerate() {
        let at = address as usize + 2 * index;
        rom[at..at + 2].copy_from_slice(&word.to_be_bytes());
    }
}
fn jsr(id: FunctionId, revision: Revision) -> [u16; 3] {
    let address = id.metadata().address(revision);
    [0x4eb9, (address >> 16) as u16, address as u16]
}
fn host(
    revision: Revision,
    pending: &'static [Translation],
    framed: &'static [FramedTranslation],
    selected: &[FunctionId],
    main: &[u16],
    bodies: &[(FunctionId, Vec<u16>)],
) -> Host {
    host_with_handler(
        revision,
        pending,
        framed,
        selected,
        main,
        bodies,
        &[0x13fc, 0, 0x00ff, 0x9001, 0x5239, 0x00ff, 0x9004, 0x4e73],
    )
}
fn host_with_handler(
    revision: Revision,
    pending: &'static [Translation],
    framed: &'static [FramedTranslation],
    selected: &[FunctionId],
    main: &[u16],
    bodies: &[(FunctionId, Vec<u16>)],
    handler: &[u16],
) -> Host {
    let registry =
        HookRegistry::proof_replayed_environment(revision, pending, framed, selected).unwrap();
    let mut rom = vec![0; 0x100000];
    rom[..4].copy_from_slice(&crate::catalogue::STACK_TOP.to_be_bytes());
    rom[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
    for vector in [28, 30] {
        rom[vector * 4..vector * 4 + 4].copy_from_slice(&0x80000_u32.to_be_bytes());
    }
    install(&mut rom, 0x100, main);
    install(&mut rom, 0x80000, handler);
    for (id, body) in bodies {
        install(&mut rom, id.metadata().address(revision), body);
    }
    let mut host = crate::fixture::tests::synthetic_host_from_rom(rom);
    let (state, report) = State::new(&registry, 0x100000, 7).unwrap();
    host.registry = registry;
    host.revision = revision;
    host.symbols = crate::catalogue::RevisionSymbols::for_revision(revision);
    host.mode = Mode::Check;
    host.wait_clock.revision = Some(revision);
    host.checks = CheckReport::registered(&host.registry);
    host.checks.replayed_environment = Some(report);
    host.replayed_state = Some(state);
    host
}
fn step(host: &mut Host) -> crate::Result<()> {
    let effect = host.tick_live_original(&mut Inputs(GenesisInputs::default()))?;
    if effect == TickEffect::FrameRendered {
        host.finish_live_frame()?;
    }
    Ok(())
}
fn checked(host: &Host, id: FunctionId) -> u64 {
    host.checks
        .replayed_environment
        .as_ref()
        .unwrap()
        .by_function[id.metadata().inventory_id]
        .checked
}
fn finish_one(host: &mut Host) -> crate::Result<()> {
    for _ in 0..100000 {
        step(host)?;
        if checked(host, ROOT) != 0 {
            return Ok(());
        }
    }
    panic!("authored root failed to complete its actual call")
}
fn main_call(revision: Revision, unmask: bool) -> Vec<u16> {
    [
        vec![0x46fc, if unmask { 0x2000 } else { 0x2700 }],
        jsr(ROOT, revision).to_vec(),
        vec![0x60fe],
    ]
    .concat()
}
fn literal(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let value = context.read_u8(VALUE);
    context.write_u8(RESULT, value.wrapping_add(1));
    Ok(None)
}
fn wrong_store(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let value = context.read_u8(VALUE);
    context.write_u8(RESULT, value.wrapping_add(2));
    Ok(None)
}
fn omitted(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let _ = context.read_u8(VALUE);
    Ok(None)
}
static LITERAL: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: literal,
}];
static WRONG: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: wrong_store,
}];
static OMITTED: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: omitted,
}];
fn literal_code() -> Vec<u16> {
    vec![
        0x1039, 0x00ff, 0x9000, 0x5200, 0x13c0, 0x00ff, 0x9002, 0x4e75,
    ]
}

fn excluded_fragment_must_not_execute(
    _context: &mut dyn Context,
    _arguments: &[u32],
) -> tje_game::Result<Option<u32>> {
    panic!("unsupported metadata fragment was excluded before execution")
}
static FRAGMENT_CANDIDATE: &[Translation] = &[Translation {
    id: ids::AUDIO_RETRY_PENDING_PCM_MAILBOX,
    function: excluded_fragment_must_not_execute,
}];

#[test]
fn unsupported_fragment_is_typed_but_generic_metadata_stays_fatal_and_supported_root_proves() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let fragment = ids::AUDIO_RETRY_PENDING_PCM_MAILBOX;
        let requested = [ROOT, fragment];
        let mut supported = Vec::new();
        let mut excluded = Vec::new();
        for id in requested {
            match HookRegistry::proof_replayed_environment(
                revision,
                FRAGMENT_CANDIDATE,
                LITERAL,
                &[id],
            ) {
                Ok(_) => supported.push(id),
                Err(crate::HostError::ReplayUnsupported { function, reason }) => {
                    assert_eq!(id, fragment);
                    assert_eq!(function, fragment.metadata().name);
                    assert_eq!(reason.kind, UnsupportedKind::EntryBoundary);
                    assert!(reason.detail.contains(fragment.metadata().inventory_id));
                    assert!(reason.detail.contains("fragment"));
                    excluded.push((id, reason));
                }
                Err(error) => panic!("unexpected invalid constructor metadata: {error}"),
            }
        }
        assert_eq!(supported, [ROOT]);
        assert_eq!(excluded.len(), 1);
        let generic = HookRegistry::proof_filtered_with_frames(
            revision,
            FRAGMENT_CANDIDATE,
            &[],
            &[fragment],
        )
        .err()
        .expect("generic entry-shape validation is unchanged");
        assert!(matches!(&generic, crate::HostError::Metadata(detail)
            if detail.contains("not a function entry")));

        let mut host = host(
            revision,
            FRAGMENT_CANDIDATE,
            LITERAL,
            &supported,
            &main_call(revision, false),
            &[(ROOT, literal_code())],
        );
        host.context().write_u8(VALUE, 7);
        finish_one(&mut host).unwrap();
        host.finalize_replayed_campaign().unwrap();
        let report = host.checks.replayed_environment.as_ref().unwrap();
        assert!(report.complete && report.finalized);
        let row = &report.by_function[ROOT.metadata().inventory_id];
        assert_eq!(
            (
                row.entered,
                row.reference_completed,
                row.checked,
                row.passed
            ),
            (1, 1, 1, 1)
        );
        assert_eq!(
            (row.failed, row.unsupported, row.open, row.provisional),
            (0, 0, 0, 0)
        );
        assert!(
            report
                .by_function
                .get(fragment.metadata().inventory_id)
                .is_none_or(|row| row.entered == 0 && row.checked == 0 && row.passed == 0)
        );
        assert_eq!(host.context().read_u8(RESULT), 8);
        assert!(
            !excluded.is_empty(),
            "the outer requested scope remains incomplete"
        );
    }
}

#[test]
fn own_accesses_and_return_are_checked_without_replacing_the_live_original_both_revisions() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut checked = host(
            revision,
            &[],
            LITERAL,
            &[ROOT],
            &main_call(revision, false),
            &[(ROOT, literal_code())],
        );
        checked.context().write_u8(VALUE, 8);
        finish_one(&mut checked).unwrap();
        let before = checked.emulator.hybrid_cpu().hybrid_state_words();
        checked.finalize_replayed_campaign().unwrap();
        assert_eq!(
            checked.emulator.hybrid_cpu().hybrid_state_words(),
            before,
            "finalization cannot advance/rewrite the live CPU"
        );
        assert_eq!(checked.context().read_u8(RESULT), 9);
        let report = checked.checks.replayed_environment.as_ref().unwrap();
        assert!(report.complete && report.finalized);
        assert_eq!(
            (
                report.attempted,
                report.own_matches,
                report.completed_proofs
            ),
            (1, 1, 1)
        );
        assert_eq!(report.by_function[ROOT.metadata().inventory_id].passed, 1);
    }
}
#[test]
fn changed_and_dropped_own_writes_fail_even_if_public_success_counts_are_forged() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for body in [WRONG, OMITTED] {
            let mut host = host(
                revision,
                &[],
                body,
                &[ROOT],
                &main_call(revision, false),
                &[(ROOT, literal_code())],
            );
            host.context().write_u8(VALUE, 8);
            assert!(finish_one(&mut host).is_err());
            let report = host.checks.replayed_environment.as_mut().unwrap();
            assert!(!report.failures.is_empty());
            assert_eq!(report.failures[0].witness.return_frame, Some(0));
            report.complete = true;
            report.mismatches = 0;
            report.unsupported = 0;
            report.completed_proofs = 999;
            host.checks.mismatches = 0;
            assert!(host.finalize_replayed_campaign().is_err());
            assert!(!host.checks.replayed_environment.as_ref().unwrap().complete);
            assert_eq!(
                host.context().read_u8(RESULT),
                9,
                "failed Rust analysis never becomes live"
            );
        }
    }
}
thread_local! { static CHILD_RUNS: Cell<usize> = const { Cell::new(0) }; }
fn child(context: &mut dyn Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
    CHILD_RUNS.with(|runs| runs.set(runs.get() + 1));
    context.write_u8(VALUE, 17);
    Ok(None)
}
fn parent(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    call_void(context, CHILD, &[])?;
    context.write_u8(RESULT, context.read_u8(VALUE));
    Ok(None)
}
static NESTED_VALUES: &[Translation] = &[Translation {
    id: CHILD,
    function: child,
}];
static PARENT: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: parent,
}];
#[test]
fn one_actual_pass_checks_nested_roots_and_never_executes_rust_callees_during_substitution() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        CHILD_RUNS.with(|runs| runs.set(0));
        let parent_code = [
            jsr(CHILD, revision).to_vec(),
            vec![0x1039, 0x00ff, 0x9000, 0x13c0, 0x00ff, 0x9002, 0x4e75],
        ]
        .concat();
        let child_code = vec![0x13fc, 17, 0x00ff, 0x9000, 0x4e75];
        let mut host = host(
            revision,
            NESTED_VALUES,
            PARENT,
            &[ROOT, CHILD],
            &main_call(revision, false),
            &[(ROOT, parent_code), (CHILD, child_code)],
        );
        finish_one(&mut host).unwrap();
        host.finalize_replayed_campaign().unwrap();
        assert_eq!(
            CHILD_RUNS.with(Cell::get),
            3,
            "only the independent child baseline and two causal probes execute Rust child code"
        );
        let report = host.checks.replayed_environment.as_ref().unwrap();
        assert_eq!(
            (
                report.attempted,
                report.completed_proofs,
                report.substitutions
            ),
            (2, 2, 1)
        );
        assert_eq!(report.obligations.len(), 1);
        assert!(matches!(
            report.obligations[0].status,
            ObligationStatus::Discharged
        ));
        assert_eq!(
            report.obligations[0].callee_witness.invocation_id,
            report.obligations[0]
                .discharged_by
                .as_ref()
                .unwrap()
                .witness
                .invocation_id
        );
    }
}
fn wait(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.wait_until(WaitRequest::ram_byte_equal(FLAG, 255, 0))?;
    context.write_u8(RESULT, 7);
    Ok(None)
}
fn wrong_wait(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.wait_until(WaitRequest::ram_byte_not_equal(FLAG, 255, 0))?;
    context.write_u8(RESULT, 7);
    Ok(None)
}
static WAIT: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: wait,
}];
static WRONG_WAIT: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: wrong_wait,
}];
#[test]
fn real_irq_polls_are_consumed_and_wrong_wait_relations_are_not_accepted_by_readiness_coincidence()
{
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (body, succeeds) in [(WAIT, true), (WRONG_WAIT, false)] {
            let code = vec![
                0x4a39, 0x00ff, 0x9001, 0x66f8, 0x13fc, 7, 0x00ff, 0x9002, 0x4e75,
            ];
            let mut host = host(
                revision,
                &[],
                body,
                &[ROOT],
                &main_call(revision, true),
                &[(ROOT, code)],
            );
            host.context().write_u8(FLAG, 1);
            {
                let bus = host.emulator.hybrid_bus_mut();
                bus.write_word(0xc00004, 0x8010);
                bus.write_word(0xc00004, 0x8a00);
                bus.apply_pending_writes();
            }
            let result = finish_one(&mut host);
            assert_eq!(result.is_ok(), succeeds, "{revision:?}: {result:?}");
            assert_ne!(
                host.context().read_u8(IRQ_VISIBLE),
                0,
                "only a real H interrupt changes the observation flag"
            );
            if succeeds {
                host.finalize_replayed_campaign().unwrap();
                assert_eq!(host.context().read_u8(RESULT), 7);
            } else {
                assert!(host.finalize_replayed_campaign().is_err());
                assert!(
                    host.checks.replayed_environment.as_ref().unwrap().failures[0]
                        .check
                        .as_ref()
                        .unwrap()
                        .detail
                        .contains("wrong wait predicate")
                );
            }
        }
    }
}
#[test]
fn zero_checked_and_open_segments_never_finalize_and_finalizer_does_not_fast_forward() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut zero = host(
            revision,
            &[],
            LITERAL,
            &[ROOT],
            &[0x60fe],
            &[(ROOT, literal_code())],
        );
        assert!(zero.finalize_replayed_campaign().is_err());
        let mut open = host(
            revision,
            &[],
            LITERAL,
            &[ROOT],
            &main_call(revision, false),
            &[(ROOT, vec![0x60fe])],
        );
        for _ in 0..100 {
            step(&mut open).unwrap();
        }
        let before = open.emulator.hybrid_cpu().hybrid_state_words();
        assert!(open.finalize_replayed_campaign().is_err());
        assert_eq!(open.emulator.hybrid_cpu().hybrid_state_words(), before);
        let report = open.checks.replayed_environment.as_ref().unwrap();
        assert!(report.finalized && !report.complete && report.open_segments != 0);
    }
}

fn local(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let sp = context.stack_pointer()?;
    context.set_stack_pointer(sp - 4)?;
    context.write_u8(sp - 4, 9);
    let value = context.read_u8(sp - 4);
    context.write_u8(RESULT, value);
    context.set_stack_pointer(sp)?;
    Ok(None)
}
fn missing_local(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let sp = context.stack_pointer()?;
    context.set_stack_pointer(sp - 4)?;
    let value = context.read_u8(sp - 4);
    context.write_u8(RESULT, value);
    context.set_stack_pointer(sp)?;
    Ok(None)
}
static LOCAL: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: local,
}];
static MISSING_LOCAL: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: missing_local,
}];
#[test]
fn private_locals_are_causal_and_a_missing_rust_store_is_never_oracle_repaired() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (body, succeeds) in [(LOCAL, true), (MISSING_LOCAL, false)] {
            let code = vec![
                0x598f, 0x1ebc, 9, 0x1017, 0x13c0, 0x00ff, 0x9002, 0x588f, 0x4e75,
            ];
            let mut host = host(
                revision,
                &[],
                body,
                &[ROOT],
                &main_call(revision, false),
                &[(ROOT, code)],
            );
            assert_eq!(finish_one(&mut host).is_ok(), succeeds);
            assert_eq!(host.finalize_replayed_campaign().is_ok(), succeeds);
        }
    }
}
fn abs_parent(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let value = tje_game::call(context, ids::ABS, &[9])?;
    context.write_u8(RESULT, value as u8);
    Ok(None)
}
fn wrong_abs_argument(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let value = tje_game::call(context, ids::ABS, &[10])?;
    context.write_u8(RESULT, value as u8);
    Ok(None)
}
fn imported_scratch(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let _ = tje_game::call(context, ids::IS_SPEECH_BUBBLE_ENTITY, &[0])?;
    context.write_u8(RESULT, context.data_register(1) as u8);
    Ok(None)
}
static ABS_PARENT: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: abs_parent,
}];
static WRONG_ABS_ARGUMENT: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: wrong_abs_argument,
}];
static IMPORTED_SCRATCH: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: imported_scratch,
}];
fn abs_parent_code(revision: Revision, scratch: bool) -> Vec<u16> {
    [
        vec![0x2f3c, 0, 9],
        jsr(ids::ABS, revision).to_vec(),
        vec![0x588f],
        if scratch { vec![0x2001] } else { Vec::new() },
        vec![0x13c0, 0x00ff, 0x9002, 0x4e75],
    ]
    .concat()
}
#[test]
fn canonical_strict_child_uses_the_same_actual_case_and_wrong_consumed_arguments_fail() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (body, succeeds) in [(ABS_PARENT, true), (WRONG_ABS_ARGUMENT, false)] {
            let mut host = host(
                revision,
                &[],
                body,
                &[ROOT],
                &main_call(revision, false),
                &[
                    (ROOT, abs_parent_code(revision, false)),
                    (ids::ABS, vec![0x202f, 4, 0x4e75]),
                ],
            );
            assert_eq!(finish_one(&mut host).is_ok(), succeeds);
            assert_eq!(host.finalize_replayed_campaign().is_ok(), succeeds);
            if succeeds {
                let report = host.checks.replayed_environment.as_ref().unwrap();
                assert_eq!(
                    report.by_function[ids::ABS.metadata().inventory_id].passed,
                    1
                );
                assert!(
                    report.obligations[0]
                        .discharged_by
                        .as_ref()
                        .unwrap()
                        .checked_outputs
                        .registers
                        .iter()
                        .any(|state| state.field == "D0" && state.bits == u32::MAX)
                );
            }
        }
    }
}
pub(super) fn actual_strict_child(revision: Revision) -> (Host, lifecycle::CompletedRoot) {
    let mut host = host(
        revision,
        &[],
        ABS_PARENT,
        &[ROOT],
        &main_call(revision, false),
        &[
            (ROOT, abs_parent_code(revision, false)),
            (ids::ABS, vec![0x202f, 4, 0x4e75]),
        ],
    );
    let mut state = host.replayed_state.take().unwrap();
    let registry = host.registry.clone();
    for _ in 0..10000 {
        state
            .before_tick(
                &mut host.emulator,
                host.frame,
                host.checks.replayed_environment.as_mut().unwrap(),
            )
            .unwrap();
        let effect = host.tick(&mut Inputs(GenesisInputs::default())).unwrap();
        let rendered = effect == TickEffect::FrameRendered;
        state.after_tick(
            &mut host.emulator,
            &registry,
            &std::collections::BTreeSet::new(),
            rendered,
            GenesisInputs::default(),
            host.checks.replayed_environment.as_mut().unwrap(),
        );
        if rendered {
            host.frame += 1;
        }
        if let Some(error) = state.take_error() {
            panic!("actual strict child admission failed: {error}");
        }
        if let Some(completed) = state.next_completed() {
            assert_eq!(
                completed.root.admission.identity().kind,
                RootKind::StrictCanonical
            );
            assert_eq!(completed.original.entry.function, Some(ids::ABS));
            host.replayed_state = Some(state);
            return (host, completed);
        }
    }
    panic!("actual authored child did not complete");
}
#[test]
fn ordinary_abi_success_does_not_certify_different_scratch_registers() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        // Use a C ABI with a genuinely caller-clobbered D1, not the strict
        // compiler-runtime abs ABI. Argument zero needs no speech-slot reads.
        let root = [
            vec![0x2f3c, 0, 0],
            jsr(ids::IS_SPEECH_BUBBLE_ENTITY, revision).to_vec(),
            vec![0x588f, 0x2001, 0x13c0, 0x00ff, 0x9002, 0x4e75],
        ]
        .concat();
        let mut host = host(
            revision,
            &[],
            IMPORTED_SCRATCH,
            &[ROOT],
            &main_call(revision, false),
            &[
                (ROOT, root),
                (
                    ids::IS_SPEECH_BUBBLE_ENTITY,
                    vec![0x302f, 6, 0x7000, 0x7255, 0x4e75],
                ),
            ],
        );
        finish_one(&mut host).unwrap();
        assert!(host.finalize_replayed_campaign().is_err());
        let report = host.checks.replayed_environment.as_ref().unwrap();
        assert_eq!(
            report.by_function[ids::IS_SPEECH_BUBBLE_ENTITY.metadata().inventory_id].passed,
            1,
            "the canonical ordinary ABI check itself passes"
        );
        assert_eq!(
            report.by_function[ROOT.metadata().inventory_id].own_matches,
            1
        );
        assert_eq!(report.by_function[ROOT.metadata().inventory_id].passed, 0);
        assert!(
            report.obligations[0]
                .required_outputs
                .registers
                .iter()
                .any(|state| state.field == "D1" && state.bits == u32::MAX)
        );
        assert!(report.obligations[0].discharged_by.is_none());
    }
}
#[test]
fn equal_aggregate_callee_visits_never_discharge_a_different_actual_invocation() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let main = [
            vec![0x46fc, 0x2700, 0x4280],
            jsr(ROOT, revision).to_vec(),
            vec![0x60f6],
        ]
        .concat();
        let mut host = host(
            revision,
            &[],
            ABS_PARENT,
            &[ROOT],
            &main,
            &[
                (ROOT, abs_parent_code(revision, false)),
                (ids::ABS, vec![0x202f, 4, 0x4e75]),
            ],
        );
        let mut disabled = false;
        for _ in 0..10000 {
            step(&mut host).unwrap();
            if !disabled && checked(&host, ids::ABS) == 1 {
                host.disabled_check_roots
                    .insert(ids::ABS.metadata().address(revision));
                disabled = true;
            }
            if checked(&host, ROOT) == 2 {
                break;
            }
        }
        assert_eq!(checked(&host, ROOT), 2);
        assert_eq!(checked(&host, ids::ABS), 1);
        assert!(host.finalize_replayed_campaign().is_err());
        let report = host.checks.replayed_environment.as_ref().unwrap();
        assert_eq!(
            report.by_function[ROOT.metadata().inventory_id].own_matches,
            2
        );
        assert_eq!(report.by_function[ROOT.metadata().inventory_id].passed, 1);
        assert!(
            report
                .obligations
                .iter()
                .any(|obligation| obligation.discharged_by.is_none())
        );
    }
}
fn empty_child(_: &mut dyn Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
    Ok(None)
}
fn residue_parent(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    call_void(context, CHILD, &[])?;
    let value = context.read_u8(context.stack_pointer()? - 8);
    context.write_u8(RESULT, value);
    Ok(None)
}
static EMPTY_CHILD: &[Translation] = &[Translation {
    id: CHILD,
    function: empty_child,
}];
static RESIDUE_PARENT: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: residue_parent,
}];
#[test]
fn substituted_compiler_private_residue_is_not_a_fabricated_caller_input() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let parent_code = [
            jsr(CHILD, revision).to_vec(),
            vec![0x102f, 0xfff8, 0x13c0, 0x00ff, 0x9002, 0x4e75],
        ]
        .concat();
        let child_code = vec![0x598f, 0x1ebc, 9, 0x588f, 0x4e75];
        let mut host = host(
            revision,
            EMPTY_CHILD,
            RESIDUE_PARENT,
            &[ROOT, CHILD],
            &main_call(revision, false),
            &[(ROOT, parent_code), (CHILD, child_code)],
        );
        assert!(finish_one(&mut host).is_err());
        assert!(host.finalize_replayed_campaign().is_err());
        assert!(
            host.checks
                .replayed_environment
                .as_ref()
                .unwrap()
                .failures
                .iter()
                .any(|failure| failure
                    .unsupported
                    .as_ref()
                    .is_some_and(|reason| reason.detail.contains("private residue")))
        );
    }
}
fn foreign_dispatch(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let _ = tje_game::Dispatcher::hybrid().call_void(context, CHILD, &[]);
    Ok(None)
}
static FOREIGN: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: foreign_dispatch,
}];
#[test]
fn ignoring_a_foreign_dispatch_error_cannot_enable_original_or_rust_callee_fallback() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = host(
            revision,
            NESTED_VALUES,
            FOREIGN,
            &[ROOT, CHILD],
            &main_call(revision, false),
            &[
                (ROOT, [jsr(CHILD, revision).to_vec(), vec![0x4e75]].concat()),
                (CHILD, vec![0x13fc, 17, 0x00ff, 0x9000, 0x4e75]),
            ],
        );
        assert!(finish_one(&mut host).is_err());
        assert!(host.finalize_replayed_campaign().is_err());
        assert!(
            host.checks
                .replayed_environment
                .as_ref()
                .unwrap()
                .failures
                .iter()
                .any(|failure| failure
                    .unsupported
                    .as_ref()
                    .is_some_and(|reason| reason.detail.contains("dispatcher configuration")))
        );
    }
}

fn clear_exact(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.discard_u8(RESULT)?;
    context.write_u8(RESULT, 0);
    Ok(None)
}
fn clear_without_discard(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.write_u8(RESULT, 0);
    Ok(None)
}
fn split_word(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.write_u8(RESULT, 0x12);
    context.write_u8(RESULT + 1, 0x34);
    Ok(None)
}
static CLEAR: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: clear_exact,
}];
static CLEAR_WITHOUT_DISCARD: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: clear_without_discard,
}];
static SPLIT_WORD: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: split_word,
}];
#[test]
fn discarded_reads_and_physical_word_width_remain_mandatory_even_when_final_ram_matches() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (body, succeeds) in [(CLEAR, true), (CLEAR_WITHOUT_DISCARD, false)] {
            let mut host = host(
                revision,
                &[],
                body,
                &[ROOT],
                &main_call(revision, false),
                &[(ROOT, vec![0x4239, 0x00ff, 0x9002, 0x4e75])],
            );
            assert_eq!(finish_one(&mut host).is_ok(), succeeds);
            assert_eq!(host.finalize_replayed_campaign().is_ok(), succeeds);
        }
        let mut split = host(
            revision,
            &[],
            SPLIT_WORD,
            &[ROOT],
            &main_call(revision, false),
            &[(ROOT, vec![0x33fc, 0x1234, 0x00ff, 0x9002, 0x4e75])],
        );
        assert!(finish_one(&mut split).is_err());
        assert_eq!(split.context().read_u16(RESULT), 0x1234);
        assert!(split.finalize_replayed_campaign().is_err());
    }
}
fn changed_irq_visible(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.write_u8(IRQ_VISIBLE, 2);
    wait(context)
}
static CHANGED_IRQ_RAM: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: changed_irq_visible,
}];
#[test]
fn changed_main_ram_observed_by_an_actual_isr_is_not_excluded_as_interrupt_timing() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let code = vec![
            0x13fc, 1, 0x00ff, 0x9004, 0x4a39, 0x00ff, 0x9001, 0x66f8, 0x13fc, 7, 0x00ff, 0x9002,
            0x4e75,
        ];
        let mut host = host(
            revision,
            &[],
            CHANGED_IRQ_RAM,
            &[ROOT],
            &main_call(revision, true),
            &[(ROOT, code)],
        );
        host.context().write_u8(FLAG, 1);
        {
            let bus = host.emulator.hybrid_bus_mut();
            bus.write_word(0xc00004, 0x8010);
            bus.write_word(0xc00004, 0x8a00);
            bus.apply_pending_writes();
        }
        assert!(finish_one(&mut host).is_err());
        assert!(host.context().read_u8(IRQ_VISIBLE) >= 2);
        assert!(host.finalize_replayed_campaign().is_err());
        assert!(
            host.checks.replayed_environment.as_ref().unwrap().failures[0]
                .check
                .as_ref()
                .unwrap()
                .detail
                .contains("own store")
        );
    }
}
#[test]
fn continue_original_executes_each_live_instruction_once_without_installing_failed_rust_effects() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let main = [
            vec![0x46fc, 0x2700],
            jsr(ROOT, revision).to_vec(),
            vec![0x60f8],
        ]
        .concat();
        let mut checked = host(
            revision,
            &[],
            WRONG,
            &[ROOT],
            &main,
            &[(ROOT, literal_code())],
        );
        let mut reference = host(
            revision,
            &[],
            WRONG,
            &[ROOT],
            &main,
            &[(ROOT, literal_code())],
        );
        checked
            .set_check_failure_policy(CheckFailurePolicy::ContinueOriginal)
            .unwrap();
        reference.mode = Mode::Reference;
        reference.replayed_state = None;
        for _ in 0..10000 {
            step(&mut checked).unwrap();
            step(&mut reference).unwrap();
        }
        assert_eq!(
            checked.cpu().hybrid_state_words(),
            reference.cpu().hybrid_state_words()
        );
        assert_eq!(
            checked.emulator.hybrid_bus().memory.hybrid_raw_main_ram(),
            reference.emulator.hybrid_bus().memory.hybrid_raw_main_ram()
        );
        assert_eq!(
            bincode::encode_to_vec(
                checked.emulator.hybrid_bus().cycles,
                bincode::config::standard()
            )
            .unwrap(),
            bincode::encode_to_vec(
                reference.emulator.hybrid_bus().cycles,
                bincode::config::standard()
            )
            .unwrap()
        );
        assert_eq!(
            bincode::encode_to_vec(
                &checked.emulator.hybrid_bus().vdp,
                bincode::config::standard()
            )
            .unwrap(),
            bincode::encode_to_vec(
                &reference.emulator.hybrid_bus().vdp,
                bincode::config::standard()
            )
            .unwrap()
        );
        assert!(checked.finalize_replayed_campaign().is_err());
    }
}

const IRQ_COPY: u32 = 0xff9006;
fn irq_consumer(
    context: &mut dyn Context,
    value: u32,
    residue: bool,
) -> tje_game::Result<Option<u32>> {
    context.set_data_register(0, value);
    context.write_u8(FLAG, 1);
    context.wait_until(WaitRequest::ram_byte_equal(FLAG, 255, 0))?;
    let result = if residue {
        context.read_u8(context.stack_pointer()? - 7)
    } else {
        context.read_u8(IRQ_COPY)
    };
    context.write_u8(RESULT, result);
    Ok(None)
}
fn irq_seven(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    irq_consumer(context, 7, false)
}
fn irq_nine(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    irq_consumer(context, 9, false)
}
fn irq_residue(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    irq_consumer(context, 7, true)
}
static IRQ_SEVEN: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: irq_seven,
}];
static IRQ_NINE: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: irq_nine,
}];
static IRQ_RESIDUE: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: irq_residue,
}];
fn irq_case(
    revision: Revision,
    body: &'static [FramedTranslation],
    algorithmic: bool,
    residue: bool,
) -> Host {
    // Save/restore is real transport in both controls. Only the algorithmic
    // variant consumes entry D0 to publish its global byte.
    let handler = [
        vec![0x48e7, 0x8000],
        if algorithmic {
            vec![0x13c0, 0x00ff, 0x9006]
        } else {
            vec![0x13fc, 7, 0x00ff, 0x9006]
        },
        vec![0x13fc, 0, 0x00ff, 0x9001, 0x4cdf, 1, 0x4e73],
    ]
    .concat();
    irq_case_with_handler(revision, body, residue, &handler, Vec::new(), None)
}
fn irq_case_with_handler(
    revision: Revision,
    body: &'static [FramedTranslation],
    residue: bool,
    handler: &[u16],
    mut extra_code: Vec<(FunctionId, Vec<u16>)>,
    initial_d3: Option<u8>,
) -> Host {
    let root = [
        vec![0x13fc, 1, 0x00ff, 0x9001, 0x4a39, 0x00ff, 0x9001, 0x66f8],
        if residue {
            vec![0x102f, 0xfff9]
        } else {
            vec![0x1039, 0x00ff, 0x9006]
        },
        vec![0x13c0, 0x00ff, 0x9002, 0x4e75],
    ]
    .concat();
    extra_code.push((ROOT, root));
    let main = [
        vec![0x46fc, 0x2700, 0x7007],
        initial_d3.map_or_else(Vec::new, |value| vec![0x7600 | u16::from(value)]),
        vec![0x46fc, 0x2000],
        jsr(ROOT, revision).to_vec(),
        vec![0x60fe],
    ]
    .concat();
    let mut host = host_with_handler(revision, &[], body, &[ROOT], &main, &extra_code, handler);
    let bus = host.emulator.hybrid_bus_mut();
    bus.write_word(0xc00004, 0x8010);
    bus.write_word(0xc00004, 0x8a00);
    bus.apply_pending_writes();
    host
}
#[test]
fn isr_semantic_register_inputs_are_authenticated_before_effects_even_with_real_save_restore() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (body, succeeds) in [(IRQ_SEVEN, true), (IRQ_NINE, false)] {
            let mut host = irq_case(revision, body, true, false);
            let result = finish_one(&mut host);
            assert_eq!(result.is_ok(), succeeds, "{revision:?}: {result:?}");
            assert_eq!(
                host.context().read_u8(IRQ_COPY),
                7,
                "only actual original ISR effects reach live RAM"
            );
            assert_eq!(host.finalize_replayed_campaign().is_ok(), succeeds);
            assert!(
                host.replayed_state
                    .as_ref()
                    .unwrap()
                    .handler_inputs
                    .iter()
                    .any(|(&(root, vector, handler), mask)| root == ROOT
                        && vector == 28
                        && handler == 0x80000
                        && mask.get(tje_engine_core::ir::StateField::Data(0)) & 0xff == 0xff),
                "finite-path input receipts include masks that later fail validation, without granting proof"
            );
            if !succeeds {
                assert!(
                    host.checks.replayed_environment.as_ref().unwrap().failures[0]
                        .check
                        .as_ref()
                        .unwrap()
                        .detail
                        .contains("consumes Data(0)")
                );
            }
        }
        let mut pure = irq_case(revision, IRQ_NINE, false, false);
        finish_one(&mut pure).unwrap();
        pure.finalize_replayed_campaign().unwrap();
        assert!(
            pure.replayed_state
                .as_ref()
                .unwrap()
                .handler_inputs
                .values()
                .all(|mask| mask.get(tje_engine_core::ir::StateField::Data(0)) == 0),
            "diagnostic masks must not turn a pure save/restore into an algorithmic input"
        );
        assert_eq!(
            pure.context().read_u8(RESULT),
            7,
            "pure preservation transport cannot demand an unrelated algorithmic input"
        );
    }
}
#[test]
fn justified_isr_internal_restores_do_not_turn_private_residue_into_caller_data() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = irq_case(revision, IRQ_RESIDUE, false, true);
        assert!(finish_one(&mut host).is_err());
        assert!(host.finalize_replayed_campaign().is_err());
        assert!(
            host.checks
                .replayed_environment
                .as_ref()
                .unwrap()
                .failures
                .iter()
                .any(|failure| failure
                    .unsupported
                    .as_ref()
                    .is_some_and(|reason| reason.detail.contains("ISR-private residue")))
        );
    }
}

#[test]
fn actual_reset_cannot_clear_an_open_replay_or_reuse_its_trajectory_as_approval() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = host(
            revision,
            &[],
            LITERAL,
            &[ROOT],
            &main_call(revision, false),
            &[(ROOT, vec![0x60fe])],
        );
        for _ in 0..100 {
            step(&mut host).unwrap();
        }
        host.reset();
        assert!(host.finalize_replayed_campaign().is_err());
        let report = host.checks.replayed_environment.as_ref().unwrap();
        assert!(!report.complete && report.finalized);
        assert!(report.open_segments != 0);
        assert!(host.run_frame(GenesisInputs::default()).is_err());
    }
}

fn busack_operand(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let value = context.read_u16(0x1a11100);
    context.write_u16(RESULT, value);
    Ok(None)
}
fn busack_device_operand(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let value = context.read_device_u16(0xa11100);
    context.write_u16(RESULT, value);
    Ok(None)
}
fn status_even_operand(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let value = context.read_u8(0x1c00004);
    context.write_u8(RESULT, value);
    Ok(None)
}
fn status_odd_operand(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let value = context.read_device_u8(0x1c00005);
    context.write_u8(RESULT, value);
    Ok(None)
}
fn z80_operand(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let value = context.read_u8(0x1a00000);
    context.write_u8(RESULT, value);
    Ok(None)
}
static BUSACK_OPERAND: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: busack_operand,
}];
static BUSACK_DEVICE: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: busack_device_operand,
}];
static STATUS_EVEN: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: status_even_operand,
}];
static STATUS_ODD: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: status_odd_operand,
}];
static Z80_OPERAND: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: z80_operand,
}];
fn bus_granted_main(revision: Revision) -> Vec<u16> {
    [
        vec![
            0x46fc, 0x2700, 0x33fc, 0x0100, 0x00a1, 0x1100, 0x33fc, 0x0100, 0x00a1, 0x1200,
        ],
        jsr(ROOT, revision).to_vec(),
        vec![0x60fe],
    ]
    .concat()
}
#[test]
fn undefined_recorded_bits_are_not_generic_operands_through_bus_device_or_alias_paths() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for body in [BUSACK_OPERAND, BUSACK_DEVICE] {
            let mut host = host(
                revision,
                &[],
                body,
                &[ROOT],
                &main_call(revision, false),
                &[(
                    ROOT,
                    vec![0x3039, 0x00a1, 0x1100, 0x33c0, 0x00ff, 0x9002, 0x4e75],
                )],
            );
            assert!(finish_one(&mut host).is_err());
            assert!(host.finalize_replayed_campaign().is_err());
            assert!(
                host.checks.replayed_environment.as_ref().unwrap().failures[0]
                    .unsupported
                    .as_ref()
                    .unwrap()
                    .detail
                    .contains("undefined")
            );
        }
        for (body, address, succeeds) in [(STATUS_EVEN, 4, false), (STATUS_ODD, 5, true)] {
            let mut host = host(
                revision,
                &[],
                body,
                &[ROOT],
                &main_call(revision, false),
                &[(
                    ROOT,
                    vec![0x1039, 0x00c0, address, 0x13c0, 0x00ff, 0x9002, 0x4e75],
                )],
            );
            assert_eq!(finish_one(&mut host).is_ok(), succeeds);
            assert_eq!(host.finalize_replayed_campaign().is_ok(), succeeds);
        }
        for granted in [false, true] {
            let main = if granted {
                bus_granted_main(revision)
            } else {
                main_call(revision, false)
            };
            let mut host = host(
                revision,
                &[],
                Z80_OPERAND,
                &[ROOT],
                &main,
                &[(
                    ROOT,
                    vec![0x1039, 0x00a0, 0, 0x13c0, 0x00ff, 0x9002, 0x4e75],
                )],
            );
            assert_eq!(
                finish_one(&mut host).is_ok(),
                granted,
                "actual denied/owned Z80 read mask must be retained"
            );
            assert_eq!(host.finalize_replayed_campaign().is_ok(), granted);
        }
    }
}
fn driven_busack_wait(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.wait_until(WaitRequest::z80_busack())?;
    context.write_u8(RESULT, 7);
    Ok(None)
}
fn undefined_busack_wait(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.wait_until(WaitRequest::new(
        tje_game::execution::WaitCondition::DeviceWord {
            address: 0xa11100,
            mask: 0x0101,
            value: 0,
        },
    ))?;
    context.write_u8(RESULT, 7);
    Ok(None)
}
static DRIVEN_BUSACK: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: driven_busack_wait,
}];
static UNDEFINED_BUSACK: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: undefined_busack_wait,
}];
#[test]
fn driven_wait_predicate_can_use_an_original_full_beat_without_exposing_undefined_bits() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (body, succeeds) in [(DRIVEN_BUSACK, true), (UNDEFINED_BUSACK, false)] {
            let code = vec![
                0x3039,
                0x00a1,
                0x1100,
                0x0240,
                if succeeds { 0x0100 } else { 0x0101 },
                0x0c40,
                0,
                0x66f0,
                0x13fc,
                7,
                0x00ff,
                0x9002,
                0x4e75,
            ];
            let mut host = host(
                revision,
                &[],
                body,
                &[ROOT],
                &bus_granted_main(revision),
                &[(ROOT, code)],
            );
            assert_eq!(finish_one(&mut host).is_ok(), succeeds);
            assert_eq!(host.finalize_replayed_campaign().is_ok(), succeeds);
            if !succeeds {
                assert!(
                    host.checks.replayed_environment.as_ref().unwrap().failures[0]
                        .unsupported
                        .as_ref()
                        .unwrap()
                        .detail
                        .contains("undefined")
                );
            }
        }
    }
}
#[test]
fn reset_ends_the_private_trajectory_even_when_its_public_report_was_removed() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = host(
            revision,
            &[],
            LITERAL,
            &[ROOT],
            &main_call(revision, false),
            &[(ROOT, vec![0x60fe])],
        );
        for _ in 0..100 {
            step(&mut host).unwrap();
        }
        host.checks.replayed_environment = None;
        host.reset();
        let before = host.cpu().hybrid_state_words();
        let clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
        assert!(step(&mut host).is_err());
        assert_eq!(host.cpu().hybrid_state_words(), before);
        assert_eq!(host.emulator.hybrid_bus().cycles.m68k_mclk_cycles, clock);
        assert!(host.finalize_replayed_campaign().is_err());
    }
}

fn hole_operand(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let value = context.read_u16(0x1180000);
    context.write_u16(RESULT, value);
    Ok(None)
}
fn void_scalar(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.write_u8(RESULT, 7);
    Ok(Some(0))
}
static HOLE_OPERAND: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: hole_operand,
}];
static VOID_SCALAR: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: void_scalar,
}];
#[test]
fn unmapped_rom_samples_never_become_software_operands() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = host(
            revision,
            &[],
            HOLE_OPERAND,
            &[ROOT],
            &main_call(revision, false),
            &[(
                ROOT,
                vec![0x3039, 0x0018, 0, 0x33c0, 0x00ff, 0x9002, 0x4e75],
            )],
        );
        assert!(finish_one(&mut host).is_err());
        assert!(host.finalize_replayed_campaign().is_err());
        assert!(
            host.checks
                .replayed_environment
                .as_ref()
                .unwrap()
                .failures
                .iter()
                .any(|failure| failure
                    .unsupported
                    .as_ref()
                    .is_some_and(|reason| reason.detail.contains("undefined")))
        );
    }
}
#[test]
fn a_void_roots_scalar_return_is_not_a_scratch_register_observation() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = host(
            revision,
            &[],
            VOID_SCALAR,
            &[ROOT],
            &main_call(revision, false),
            &[(ROOT, vec![0x13fc, 7, 0x00ff, 0x9002, 0x4e75])],
        );
        assert!(finish_one(&mut host).is_err());
        assert!(host.finalize_replayed_campaign().is_err());
        assert!(
            host.checks
                .replayed_environment
                .as_ref()
                .unwrap()
                .failures
                .iter()
                .any(|failure| failure
                    .comparison
                    .as_ref()
                    .is_some_and(|comparison| comparison.point == "abi_return_shape"))
        );
    }
}

#[test]
fn genuine_isr_stack_value_inputs_are_not_discarded_with_alignment_transport() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let handler = [
            0x48e7, 0x8000, 0x200f, 0x13c0, 0x00ff, 0x9006, 0x13fc, 0, 0x00ff, 0x9001, 0x4cdf, 1,
            0x4e73,
        ];
        let root = vec![
            0x13fc, 1, 0x00ff, 0x9001, 0x4a39, 0x00ff, 0x9001, 0x66f8, 0x1039, 0x00ff, 0x9006,
            0x13c0, 0x00ff, 0x9002, 0x4e75,
        ];
        let main = [
            vec![0x46fc, 0x2700, 0x7007, 0x46fc, 0x2000],
            jsr(ROOT, revision).to_vec(),
            vec![0x60fe],
        ]
        .concat();
        let mut host = host_with_handler(
            revision,
            &[],
            IRQ_SEVEN,
            &[ROOT],
            &main,
            &[(ROOT, root)],
            &handler,
        );
        let bus = host.emulator.hybrid_bus_mut();
        bus.write_word(0xc00004, 0x8010);
        bus.write_word(0xc00004, 0x8a00);
        bus.apply_pending_writes();
        assert!(finish_one(&mut host).is_err());
        assert!(host.finalize_replayed_campaign().is_err());
        assert!(
            host.checks
                .replayed_environment
                .as_ref()
                .unwrap()
                .failures
                .iter()
                .any(|failure| failure
                    .unsupported
                    .as_ref()
                    .is_some_and(|reason| reason.kind == UnsupportedKind::TypedControlProfile
                        && reason.detail.contains("hardware-adjusted stack value")))
        );
        assert!(
            host.replayed_state
                .as_ref()
                .unwrap()
                .handler_inputs
                .values()
                .any(|mask| mask.get(tje_engine_core::ir::StateField::Ssp) & !1 != 0),
            "a rejected genuine stack-value input remains visible in the diagnostic union"
        );
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResidueCase {
    Word,
    Generic,
    MixedWord,
    WrongMixedWord,
    Long,
    WrongLong,
    SecondCited,
    SecondGeneric,
    WrongWidth,
    WrongOffset,
    WrongOwner,
    WrongFrame,
    ReleasedFrame,
    EmptyCitation,
}
fn residue_body(context: &mut dyn Context, case: ResidueCase) -> tje_game::Result<Option<u32>> {
    use tje_game::{call_frame::FrameState, stack_read::LocalWidth};
    let frame = context
        .frame_state()
        .ok_or(tje_game::call_frame::FrameError::Unavailable)?;
    let sp = context.stack_pointer()?;
    let long = matches!(case, ResidueCase::Long | ResidueCase::WrongLong);
    let bytes = if long { 8 } else { 4 };
    let address = sp - bytes;
    context.set_stack_pointer(address)?;
    if matches!(case, ResidueCase::MixedWord | ResidueCase::WrongMixedWord) {
        context.write_u8(
            address,
            if case == ResidueCase::WrongMixedWord {
                0xab
            } else {
                0xaa
            },
        );
    }
    if long {
        context.write_u16(
            address,
            if case == ResidueCase::WrongLong {
                0xabce
            } else {
                0xabcd
            },
        );
    }
    let descriptor_frame = if case == ResidueCase::WrongFrame {
        FrameState::new(sp + 4)
    } else {
        frame
    };
    let owner = if case == ResidueCase::WrongOwner {
        CHILD
    } else {
        ROOT
    };
    let offset = if case == ResidueCase::WrongOffset {
        -2
    } else {
        -(bytes as i32)
    };
    let width = if case == ResidueCase::WrongWidth {
        LocalWidth::Byte
    } else if long {
        LocalWidth::Long
    } else {
        LocalWidth::Word
    };
    // The authored original below reads this local without defining every lane.
    // This explicit declaration is the intent, not a generic-read address waiver.
    let citation = if case == ResidueCase::EmptyCitation {
        ""
    } else {
        "authored residue_code: current frame local read before complete initialization"
    };
    if case == ResidueCase::ReleasedFrame {
        context.set_stack_pointer(sp)?;
    }
    let mut value = if case == ResidueCase::Generic {
        u32::from(context.read_u16(address))
    } else {
        context.read_uninitialized_local(
            descriptor_frame.uninitialized_local(owner, offset, width, citation),
        )?
    };
    if case == ResidueCase::SecondCited {
        value = context.read_uninitialized_local(frame.uninitialized_local(
            ROOT,
            -4,
            LocalWidth::Word,
            citation,
        ))?;
    } else if case == ResidueCase::SecondGeneric {
        value = u32::from(context.read_u16(address));
    }
    if long {
        context.write_u32(RESULT, value);
    } else {
        context.write_u16(RESULT, value as u16);
    }
    context.set_stack_pointer(sp)?;
    Ok(None)
}
static RESIDUE_CASES: &[(ResidueCase, &[FramedTranslation], bool)] = &[
    (
        ResidueCase::Word,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::Word),
        }],
        true,
    ),
    (
        ResidueCase::Generic,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::Generic),
        }],
        false,
    ),
    (
        ResidueCase::MixedWord,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::MixedWord),
        }],
        true,
    ),
    (
        ResidueCase::WrongMixedWord,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::WrongMixedWord),
        }],
        false,
    ),
    (
        ResidueCase::Long,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::Long),
        }],
        true,
    ),
    (
        ResidueCase::WrongLong,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::WrongLong),
        }],
        false,
    ),
    (
        ResidueCase::SecondCited,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::SecondCited),
        }],
        true,
    ),
    (
        ResidueCase::SecondGeneric,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::SecondGeneric),
        }],
        false,
    ),
    (
        ResidueCase::WrongWidth,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::WrongWidth),
        }],
        false,
    ),
    (
        ResidueCase::WrongOffset,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::WrongOffset),
        }],
        false,
    ),
    (
        ResidueCase::WrongOwner,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::WrongOwner),
        }],
        false,
    ),
    (
        ResidueCase::WrongFrame,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::WrongFrame),
        }],
        false,
    ),
    (
        ResidueCase::ReleasedFrame,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::ReleasedFrame),
        }],
        false,
    ),
    (
        ResidueCase::EmptyCitation,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| residue_body(ctx, ResidueCase::EmptyCitation),
        }],
        false,
    ),
];
fn residue_code(case: ResidueCase) -> Vec<u16> {
    let long = matches!(case, ResidueCase::Long | ResidueCase::WrongLong);
    let mut code = vec![if long { 0x518f } else { 0x598f }];
    if matches!(case, ResidueCase::MixedWord | ResidueCase::WrongMixedWord) {
        code.extend([0x1ebc, 0x00aa]);
    } else if long {
        code.extend([0x3ebc, 0xabcd]);
    }
    code.push(if long { 0x2017 } else { 0x3017 });
    if matches!(case, ResidueCase::SecondCited | ResidueCase::SecondGeneric) {
        code.push(0x3217);
    }
    code.extend([
        if long { 0x23c0 } else { 0x33c0 },
        0x00ff,
        0x9002,
        if long { 0x508f } else { 0x588f },
        0x4e75,
    ]);
    code
}
#[test]
fn deliberate_uninitialized_locals_require_exact_explicit_occurrences_without_blessing_or_repair() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for &(case, body, succeeds) in RESIDUE_CASES {
            let long = matches!(case, ResidueCase::Long | ResidueCase::WrongLong);
            // Genuine authored pre-root CPU writes establish the entry residue;
            // no fixture RAM/CPU snapshot is installed after the call begins.
            let seed = if long {
                vec![0x23fc, 0x1234, 0x5678, 0x00ff, 0x7ff4]
            } else {
                vec![0x33fc, 0x1234, 0x00ff, 0x7ff8]
            };
            let main = [
                vec![0x46fc, 0x2700],
                seed,
                jsr(ROOT, revision).to_vec(),
                vec![0x60fe],
            ]
            .concat();
            let mut host = host(
                revision,
                &[],
                body,
                &[ROOT],
                &main,
                &[(ROOT, residue_code(case))],
            );
            let result = finish_one(&mut host);
            assert_eq!(
                result.is_ok(),
                succeeds,
                "{revision:?}/{case:?}: {result:?}"
            );
            assert_eq!(
                host.finalize_replayed_campaign().is_ok(),
                succeeds,
                "{revision:?}/{case:?}"
            );
            if succeeds {
                let context = host.context();
                let value = if long {
                    context.read_u32(RESULT)
                } else {
                    u32::from(context.read_u16(RESULT))
                };
                let expected = if long {
                    0xabcd5678
                } else if case == ResidueCase::MixedWord {
                    0xaa34
                } else {
                    0x1234
                };
                assert_eq!(value, expected);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AbiCase {
    Mirrored,
    Elided,
    Duplicate,
    WrongPurpose,
    WrongAddress,
    WrongWidth,
    AfterObservable,
}
fn abi_restore_body(context: &mut dyn Context, case: AbiCase) -> tje_game::Result<Option<u32>> {
    use tje_game::call_frame::{FrameLayout, LocalFrame};
    if case == AbiCase::Elided {
        context.write_u8(RESULT, 7);
        return Ok(None);
    }
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x04, 0))?;
    let entry_sp = context.frame_state().unwrap().entry_sp;
    context.write_u8(RESULT, 7);
    if case == AbiCase::Mirrored || case == AbiCase::Duplicate {
        frame.leave(context)?;
        if case == AbiCase::Duplicate {
            context.discard_u16(entry_sp)?;
        }
        return Ok(None);
    }
    // This initialized private restore read advances over the actual MOVEM
    // memory traffic but must retain its exact optional terminal occurrence.
    let saved = context.read_u32(entry_sp - 4);
    match case {
        AbiCase::WrongPurpose => {
            let _ = context.read_u16(entry_sp);
        }
        AbiCase::WrongAddress => context.discard_u16(entry_sp + 2)?,
        AbiCase::WrongWidth => context.discard_u8(entry_sp)?,
        AbiCase::AfterObservable => {
            context.set_data_register(2, saved);
            context.set_stack_pointer(entry_sp)?;
            context.write_u8(VALUE, 1);
            context.discard_u16(entry_sp)?;
        }
        _ => unreachable!(),
    }
    Ok(None)
}
static ABI_CASES: &[(AbiCase, &[FramedTranslation], bool)] = &[
    (
        AbiCase::Mirrored,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| abi_restore_body(ctx, AbiCase::Mirrored),
        }],
        true,
    ),
    (
        AbiCase::Elided,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| abi_restore_body(ctx, AbiCase::Elided),
        }],
        true,
    ),
    (
        AbiCase::Duplicate,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| abi_restore_body(ctx, AbiCase::Duplicate),
        }],
        false,
    ),
    (
        AbiCase::WrongPurpose,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| abi_restore_body(ctx, AbiCase::WrongPurpose),
        }],
        false,
    ),
    (
        AbiCase::WrongAddress,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| abi_restore_body(ctx, AbiCase::WrongAddress),
        }],
        false,
    ),
    (
        AbiCase::WrongWidth,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| abi_restore_body(ctx, AbiCase::WrongWidth),
        }],
        false,
    ),
    (
        AbiCase::AfterObservable,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| abi_restore_body(ctx, AbiCase::AfterObservable),
        }],
        false,
    ),
];
#[test]
fn optional_movem_terminal_occurrences_are_structural_but_never_reusable_read_waivers() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for &(case, body, succeeds) in ABI_CASES {
            let mut original = vec![0x48e7, 0x2000, 0x13fc, 7, 0x00ff, 0x9002, 0x4cdf, 4];
            if case == AbiCase::AfterObservable {
                original.extend([0x13fc, 1, 0x00ff, 0x9000]);
            }
            original.push(0x4e75);
            let mut host = host(
                revision,
                &[],
                body,
                &[ROOT],
                &main_call(revision, false),
                &[(ROOT, original)],
            );
            let result = finish_one(&mut host);
            assert_eq!(
                result.is_ok(),
                succeeds,
                "{revision:?}/{case:?}: {result:?}"
            );
            assert_eq!(
                host.finalize_replayed_campaign().is_ok(),
                succeeds,
                "{revision:?}/{case:?}"
            );
        }
    }
}

#[test]
fn partial_byte_load_and_extensions_kill_the_full_incoming_register_dependency() {
    // Save/restore is genuine, but the published value is loaded from the
    // pending byte and fully extended. Entry D0's preserved merge bits are dead.
    let handler = [
        0x48e7, 0x8000, 0x1039, 0x00ff, 0x9001, 0x4880, 0x48c0, 0x13c0, 0x00ff, 0x9006, 0x13fc, 0,
        0x00ff, 0x9001, 0x4cdf, 1, 0x4e73,
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = irq_case_with_handler(revision, IRQ_NINE, false, &handler, Vec::new(), None);
        finish_one(&mut host).unwrap();
        host.finalize_replayed_campaign().unwrap();
        assert_eq!(host.context().read_u8(RESULT), 1);
        assert!(
            host.replayed_state
                .as_ref()
                .unwrap()
                .handler_inputs
                .values()
                .all(|mask| mask.get(tje_engine_core::ir::StateField::Data(0)) == 0)
        );
    }
}

fn irq_d3_seven(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.set_data_register(3, 7);
    irq_consumer(context, 7, false)
}
fn irq_d3_nine(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.set_data_register(3, 9);
    irq_consumer(context, 7, false)
}
static IRQ_D3_SEVEN: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: irq_d3_seven,
}];
static IRQ_D3_NINE: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: irq_d3_nine,
}];
fn irq_call_d3(revision: Revision) -> Vec<u16> {
    [
        vec![0x48e7, 0x9000],
        jsr(CHILD, revision).to_vec(),
        vec![
            0x13c3, 0x00ff, 0x9006, 0x13fc, 0, 0x00ff, 0x9001, 0x4cdf, 9, 0x4e73,
        ],
    ]
    .concat()
}

#[test]
fn nested_declared_preservation_retains_the_later_algorithmic_entry_use() {
    // A genuine nested callee saves/overwrites/restores D2-D4. The subsequent
    // D3 byte use still authenticates entry D3, never unrelated saved D2/D4.
    let child = vec![
        0x48e7, 0x3800, 0x7400, 0x7600, 0x7800, 0x4cdf, 0x001c, 0x4e75,
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (body, succeeds) in [(IRQ_D3_SEVEN, true), (IRQ_D3_NINE, false)] {
            let mut host = irq_case_with_handler(
                revision,
                body,
                false,
                &irq_call_d3(revision),
                vec![(CHILD, child.clone())],
                Some(7),
            );
            let result = finish_one(&mut host);
            assert_eq!(result.is_ok(), succeeds, "{revision:?}: {result:?}");
            assert_eq!(host.finalize_replayed_campaign().is_ok(), succeeds);
            let inputs = &host.replayed_state.as_ref().unwrap().handler_inputs;
            assert!(
                inputs
                    .values()
                    .any(|mask| mask.get(tje_engine_core::ir::StateField::Data(3)) & 0xff != 0)
            );
            assert!(
                inputs.values().all(
                    |mask| mask.get(tje_engine_core::ir::StateField::Data(2)) == 0
                        && mask.get(tje_engine_core::ir::StateField::Data(4)) == 0
                )
            );
            if !succeeds {
                assert!(result.unwrap_err().to_string().contains("consumes Data(3)"));
            }
        }
    }
}

#[test]
fn actual_violation_of_a_declared_preserved_register_is_unsupported() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        // The outer IRQ restores D3, so its raw entry/exit are preserved. Its
        // nested callee nevertheless violates its own declared D3 contract.
        let mut host = irq_case_with_handler(
            revision,
            IRQ_D3_SEVEN,
            false,
            &irq_call_d3(revision),
            vec![(CHILD, vec![0x7600, 0x4e75])],
            Some(7),
        );
        let error = finish_one(&mut host).unwrap_err();
        assert!(
            error.to_string().contains("ABI-declared preservation"),
            "{error}"
        );
        assert!(host.finalize_replayed_campaign().is_err());
        assert!(!host.checks.replayed_environment.as_ref().unwrap().complete);
    }
}

#[test]
fn equal_clobbered_register_values_do_not_restore_an_entry_dependency() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let handler = [
            vec![0x48e7, 0x8000],
            jsr(CHILD, revision).to_vec(),
            vec![
                0x13c0, 0x00ff, 0x9006, 0x13fc, 0, 0x00ff, 0x9001, 0x4cdf, 1, 0x4e73,
            ],
        ]
        .concat();
        // Original D0 happens to be seven at entry and exit. MOVEQ produces
        // that value independently, and this callee does not preserve D0.
        let mut host = irq_case_with_handler(
            revision,
            IRQ_NINE,
            false,
            &handler,
            vec![(CHILD, vec![0x7007, 0x4e75])],
            None,
        );
        finish_one(&mut host).unwrap();
        host.finalize_replayed_campaign().unwrap();
        assert_eq!(host.context().read_u8(RESULT), 7);
        assert!(
            host.replayed_state
                .as_ref()
                .unwrap()
                .handler_inputs
                .values()
                .all(|mask| mask.get(tje_engine_core::ir::StateField::Data(0)) == 0)
        );
    }
}

#[derive(Clone, Copy)]
enum ComputedCase {
    Good,
    WrongRegister,
    WrongSite,
    WrongId,
    MissingSite,
    Stack,
    WrongStack,
}
fn computed_call(context: &mut dyn Context, case: ComputedCase) -> tje_game::Result<Option<u32>> {
    use tje_game::call_frame::CallSite;
    let revision = context.revision();
    let target = ids::ABS.metadata().address(revision);
    let stack = matches!(case, ComputedCase::Stack | ComputedCase::WrongStack);
    if stack {
        let pre_call =
            context
                .stack_pointer()?
                .wrapping_sub(if matches!(case, ComputedCase::WrongStack) {
                    0
                } else {
                    4
                });
        context.set_data_register(0, target.wrapping_sub(pre_call));
    } else {
        context.set_address_register(
            0,
            target.wrapping_add(if matches!(case, ComputedCase::WrongRegister) {
                2
            } else {
                0
            }),
        );
    }
    let id = if matches!(case, ComputedCase::WrongId) {
        ids::GET_LEVEL_MUSIC_INDEX
    } else {
        ids::ABS
    };
    let site = CallSite::relative(
        ROOT,
        revision,
        if stack {
            18
        } else if matches!(case, ComputedCase::WrongSite) {
            14
        } else {
            12
        },
    );
    let value = if matches!(case, ComputedCase::MissingSite) {
        tje_game::call(context, id, &[7])?
    } else {
        tje_game::call_at(context, id, &[7], site)?
    };
    context.write_u16(RESULT, value as u16);
    Ok(None)
}
fn computed_good(c: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    computed_call(c, ComputedCase::Good)
}
fn computed_wrong_register(c: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    computed_call(c, ComputedCase::WrongRegister)
}
fn computed_wrong_site(c: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    computed_call(c, ComputedCase::WrongSite)
}
fn computed_wrong_id(c: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    computed_call(c, ComputedCase::WrongId)
}
fn computed_missing_site(c: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    computed_call(c, ComputedCase::MissingSite)
}
fn computed_stack(c: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    computed_call(c, ComputedCase::Stack)
}
fn computed_wrong_stack(c: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    computed_call(c, ComputedCase::WrongStack)
}
static COMPUTED_GOOD: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: computed_good,
}];
static COMPUTED_WRONG_REGISTER: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: computed_wrong_register,
}];
static COMPUTED_WRONG_SITE: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: computed_wrong_site,
}];
static COMPUTED_WRONG_ID: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: computed_wrong_id,
}];
static COMPUTED_MISSING_SITE: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: computed_missing_site,
}];
static COMPUTED_STACK: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: computed_stack,
}];
static COMPUTED_WRONG_STACK: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: computed_wrong_stack,
}];

fn computed_host(
    revision: Revision,
    body: &'static [FramedTranslation],
    stack: bool,
    unknown: bool,
) -> Host {
    let target = if unknown {
        0x80000
    } else {
        ids::ABS.metadata().address(revision)
    };
    let mut root = vec![0x4878, 7]; // PEA seven: real outgoing argument.
    if stack {
        root.extend_from_slice(&[
            0x200f,
            0x4480,
            0x0680,
            (target >> 16) as u16,
            target as u16,
            0x4eb7,
            0x0800,
        ]); // target = pre-call SP + D0.L.
    } else {
        root.extend_from_slice(&[0x41f9, (target >> 16) as u16, target as u16, 0x4e90]);
    }
    root.extend_from_slice(&[0x588f, 0x33c0, 0x00ff, 0x9002, 0x4e75]);
    let main = [jsr(ROOT, revision).to_vec(), vec![0x60fe]].concat();
    let absolute = vec![0x202f, 4, 0x6a02, 0x4480, 0x4e75];
    host_with_handler(
        revision,
        &[],
        body,
        &[ROOT],
        &main,
        &[(ROOT, root), (ids::ABS, absolute)],
        &[0x7007, 0x4e75],
    )
}

#[test]
fn catalogued_computed_calls_require_the_actual_target_register_and_source_site() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (body, succeeds) in [
            (COMPUTED_GOOD, true),
            (COMPUTED_WRONG_REGISTER, false),
            (COMPUTED_WRONG_SITE, false),
            (COMPUTED_WRONG_ID, false),
            (COMPUTED_MISSING_SITE, false),
        ] {
            let mut host = computed_host(revision, body, false, false);
            let result = finish_one(&mut host);
            assert_eq!(result.is_ok(), succeeds, "{revision:?}: {result:?}");
            assert_eq!(host.finalize_replayed_campaign().is_ok(), succeeds);
            assert_eq!(
                host.context().read_u16(RESULT),
                7,
                "only the actual original ran live"
            );
            if succeeds {
                assert!(host.checks.replayed_environment.as_ref().unwrap().complete);
            } else {
                assert!(!host.checks.replayed_environment.as_ref().unwrap().complete);
            }
        }
    }
}

#[test]
fn stack_indexed_call_target_uses_rust_pre_call_sp_not_the_post_push_sp() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (body, succeeds) in [(COMPUTED_STACK, true), (COMPUTED_WRONG_STACK, false)] {
            let mut host = computed_host(revision, body, true, false);
            let result = finish_one(&mut host);
            assert_eq!(result.is_ok(), succeeds, "{revision:?}: {result:?}");
            assert_eq!(host.finalize_replayed_campaign().is_ok(), succeeds);
            assert_eq!(host.context().read_u16(RESULT), 7);
            if !succeeds {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("computed call target consumes")
                );
            }
        }
    }
}

#[test]
fn an_uncatalogued_computed_call_never_creates_an_approved_substitution() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = computed_host(revision, COMPUTED_GOOD, false, true);
        let error = finish_one(&mut host).unwrap_err();
        assert!(error.to_string().contains("uncatalogued"), "{error}");
        assert!(host.finalize_replayed_campaign().is_err());
        assert!(!host.checks.replayed_environment.as_ref().unwrap().complete);
    }
}
