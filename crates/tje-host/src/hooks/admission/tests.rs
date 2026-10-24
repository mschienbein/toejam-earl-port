use super::*;
use crate::{Host, Mode, hooks::HookRegistry};
use tje_game::{Context, execution::ExecutionClass, ids};
use tje_rom::Revision;

fn changed_body(context: &mut dyn Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
    context.write_u8(0xff9000, 1);
    Ok(None)
}
fn changed_frame(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    changed_body(context, &[])
}
fn instrumentation(context: &mut crate::abi::HookContext<'_>) -> Result<Option<u32>> {
    context.write_u8(0xff9000, 1);
    Ok(None)
}
static CHANGED_VALUES: &[Translation] = &[Translation {
    id: ids::ABS,
    function: changed_body,
}];
static CHANGED_FRAMES: &[FramedTranslation] = &[FramedTranslation {
    id: ids::ABS,
    function: changed_frame,
}];
static CANONICAL_VALUES: &[Translation] = &[Translation {
    id: ids::ABS,
    function: tje_game::runtime::abs,
}];
static MIXED: &[Translation] = &[
    Translation {
        id: ids::ABS,
        function: tje_game::runtime::abs,
    },
    Translation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: tje_game::core::wait_for_next_vblank,
    },
];

fn rejects_before_rom_import(registry: HookRegistry) {
    assert!(registry.ensure_replacement_admission().is_err());
    // Invalid bytes deliberately demonstrate that live admission is rejected
    // before ROM parsing, device creation, timing preparation or guest effects.
    let error = Host::new(&[], Mode::Replace, registry)
        .err()
        .expect("rejected registry");
    assert!(matches!(error, HostError::Metadata(_)), "{error}");
    assert!(
        error.to_string().contains("canonical production bodies"),
        "{error}"
    );
}

#[test]
fn canonical_pending_tables_are_check_only_through_every_proof_constructor() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let values = tje_game::core::PENDING_TRANSLATIONS;
        let framed = tje_game::sprites::PENDING_FRAMED_TRANSLATIONS;
        assert!(!values.is_empty() && !framed.is_empty());
        let selected = [values[0].id];
        for registry in [
            HookRegistry::proof(revision, values).unwrap(),
            HookRegistry::proof_with_frames(revision, &[], framed).unwrap(),
            HookRegistry::proof_filtered(revision, values, &selected).unwrap(),
            HookRegistry::proof_filtered_with_frames(revision, values, framed, &selected).unwrap(),
            HookRegistry::proof_clocked_strict(revision, values, framed, &selected).unwrap(),
        ] {
            assert!(registry.admission.proof_only);
            assert!(registry.admission.noncanonical_dispatcher);
            assert!(registry.iter().all(|hook| hook.proof_only_admission()));
            rejects_before_rom_import(registry.clone());
            rejects_before_rom_import(registry);
        }
    }
}

#[test]
fn unhooked_pending_callee_overlays_keep_independent_noncanonical_provenance() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut registry = HookRegistry::proof_filtered(revision, MIXED, &[ids::ABS]).unwrap();
        assert_eq!(registry.len(), 1);
        let hook = registry.at(ids::ABS.metadata().address(revision)).unwrap();
        assert_eq!(hook.body_provenance(), BodyProvenance::CanonicalProduction);
        assert!(
            registry
                .at(ids::WAIT_FOR_NEXT_V_BLANK.metadata().address(revision))
                .is_none()
        );
        assert!(registry.admission.noncanonical_dispatcher);
        rejects_before_rom_import(registry.clone());
        // This private mutation isolates the independent dispatcher barrier:
        // neither a canonical selected root nor clearing a proof tag launders
        // its still-effective unhooked pending callee.
        registry.admission.proof_only = false;
        for hook in registry.hooks.values_mut() {
            hook.proof_only_admission = false;
        }
        assert!(!registry.admission.noncanonical_hooks);
        rejects_before_rom_import(registry);
    }
}

#[test]
fn bounded_entries_and_manual_value_registration_cannot_launder_bodies() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for registry in [
            HookRegistry::from_translations(revision, tje_game::core::PENDING_TRANSLATIONS)
                .unwrap(),
            HookRegistry::from_entries(
                revision,
                &[],
                tje_game::sprites::PENDING_FRAMED_TRANSLATIONS,
            )
            .unwrap(),
            HookRegistry::from_entries(revision, CHANGED_VALUES, &[]).unwrap(),
            HookRegistry::from_entries(revision, &[], CHANGED_FRAMES).unwrap(),
        ] {
            assert!(!registry.admission.proof_only);
            assert!(registry.admission.noncanonical_dispatcher);
            assert!(
                registry
                    .iter()
                    .all(|hook| hook.body_provenance() == BodyProvenance::CandidateOverride)
            );
            rejects_before_rom_import(registry);
        }
        let mut registry = HookRegistry::empty(revision);
        registry
            .register_translation_with_execution(CHANGED_VALUES[0], ExecutionClass::Atomic)
            .unwrap();
        assert!(!registry.admission.proof_only);
        assert!(registry.admission.noncanonical_hooks);
        rejects_before_rom_import(registry);
        let mut registry = HookRegistry::empty(revision);
        let pending = *tje_game::core::PENDING_TRANSLATIONS
            .iter()
            .find(|entry| entry.id == ids::WAIT_FOR_NEXT_V_BLANK)
            .unwrap();
        registry
            .register_translation_with_execution(pending, ExecutionClass::TimeAware)
            .unwrap();
        rejects_before_rom_import(registry);
    }
}

#[test]
fn manual_framed_registration_keeps_global_canonical_identity_separate_from_overlay_identity() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let pending = tje_game::sprites::PENDING_FRAMED_TRANSLATIONS;
        let mut registry = HookRegistry::proof_filtered_with_frames(
            revision,
            CANONICAL_VALUES,
            pending,
            &[ids::ABS],
        )
        .unwrap();
        let entry = pending[0];
        registry
            .register_framed_translation_with_execution(
                entry,
                tje_game::execution_class(entry.id).unwrap(),
            )
            .unwrap();
        assert_eq!(
            registry
                .at(entry.id.metadata().address(revision))
                .unwrap()
                .body_provenance(),
            BodyProvenance::CandidateOverride
        );
        rejects_before_rom_import(registry);
        let mut registry = HookRegistry::empty(revision);
        assert!(
            registry
                .register_framed_translation_with_execution(
                    CHANGED_FRAMES[0],
                    ExecutionClass::Atomic
                )
                .is_err()
        );
        assert!(registry.is_empty());
    }
}

#[test]
fn every_manual_host_instrumentation_route_is_rejected_for_live_replacement() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut named = HookRegistry::empty(revision);
        named
            .register(
                &crate::catalogue::FunctionCatalogue::new(revision),
                "abs",
                instrumentation,
            )
            .unwrap();
        let mut authored = HookRegistry::empty(revision);
        authored
            .register_contract(ids::ABS.metadata().clone(), true, instrumentation)
            .unwrap();
        let mut functional = HookRegistry::empty(revision);
        functional
            .register_execution_contract(
                ids::ABS.metadata().clone(),
                ExecutionClass::Functional,
                instrumentation,
            )
            .unwrap();
        let mut timed = HookRegistry::empty(revision);
        timed
            .register_clock_ceiling(
                ids::ABS.metadata().clone(),
                tje_game::execution::CallCeiling::new(10, 1).unwrap(),
                instrumentation,
            )
            .unwrap();
        let mut exit = HookRegistry::empty(revision);
        exit.register_exit_contract(
            ids::ABS.metadata().clone(),
            ExecutionClass::Atomic,
            tje_game::exits::ExitContract::Rts,
            instrumentation,
        )
        .unwrap();
        for registry in [named, authored, functional, timed, exit] {
            assert!(
                registry
                    .iter()
                    .all(|hook| hook.body_provenance() == BodyProvenance::HostInstrumentation)
            );
            assert!(registry.admission.noncanonical_hooks);
            rejects_before_rom_import(registry);
        }
    }
}

#[test]
fn canonical_bounded_manual_and_default_bodies_remain_eligible_without_class_or_policy_inference() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut manual = HookRegistry::empty(revision);
        manual
            .register_translation_with_execution(CANONICAL_VALUES[0], ExecutionClass::Atomic)
            .unwrap();
        for registry in [
            HookRegistry::translated(revision).unwrap(),
            HookRegistry::filtered(revision, &[], &[ids::ABS]).unwrap(),
            HookRegistry::from_entries(revision, CANONICAL_VALUES, &[]).unwrap(),
            HookRegistry::from_entries(revision, &[], &[]).unwrap(),
            manual,
        ] {
            registry.ensure_replacement_admission().unwrap();
            assert!(registry.iter().all(|hook| !hook.proof_only_admission()
                && hook.body_provenance() == BodyProvenance::CanonicalProduction));
        }
        // A proof constructor remains diagnostic even if every supplied body
        // happens to be canonical; provenance is not inferred from its policy.
        rejects_before_rom_import(HookRegistry::proof(revision, CANONICAL_VALUES).unwrap());
        rejects_before_rom_import(HookRegistry::proof(revision, &[]).unwrap());
    }
}

fn encoded(host: &Host) -> Vec<u8> {
    bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap()
}

#[test]
fn dispatch_and_frame_entries_reject_before_any_guest_or_clock_effect() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for use_play in [false, true] {
            let mut host = crate::fixture::tests::synthetic_host(&[0x60fe], &[0x4e73]);
            host.revision = revision;
            host.registry = HookRegistry::proof_filtered(revision, MIXED, &[ids::ABS]).unwrap();
            host.mode = Mode::Replace;
            let before = encoded(&host);
            let result = if use_play {
                host.play_frame(Default::default()).map(|_| ())
            } else {
                host.run_frame(Default::default()).map(|_| ())
            };
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("canonical production bodies")
            );
            assert_eq!(encoded(&host), before);
            assert_eq!(host.frame_count(), 0);
            assert_eq!(host.checks.calls, 0);
        }
        let mut host = crate::fixture::tests::synthetic_host(&[0x60fe], &[0x4e73]);
        host.mode = Mode::Replace;
        let registry = HookRegistry::from_entries(revision, CHANGED_VALUES, &[]).unwrap();
        let hook = registry.at(ids::ABS.metadata().address(revision)).unwrap();
        let before = encoded(&host);
        assert!(host.replace_hook(hook).is_err());
        assert_eq!(encoded(&host), before);
    }
}

#[test]
fn canonical_index_matches_all_and_only_production_entry_forms() {
    for entry in tje_game::translations()
        .copied()
        .map(TranslatedEntry::Values)
        .chain(
            tje_game::framed_translations()
                .copied()
                .map(TranslatedEntry::Framed),
        )
    {
        assert!(is_canonical(entry));
    }
    for entry in tje_game::core::PENDING_TRANSLATIONS {
        assert!(!is_canonical(TranslatedEntry::Values(*entry)));
    }
    for entry in tje_game::sprites::PENDING_FRAMED_TRANSLATIONS {
        assert!(!is_canonical(TranslatedEntry::Framed(*entry)));
    }
    assert!(!is_canonical(TranslatedEntry::Values(CHANGED_VALUES[0])));
    assert!(!is_canonical(TranslatedEntry::Framed(CHANGED_FRAMES[0])));
}

#[test]
fn canonical_body_retains_live_eligibility_under_unit_only_owner_policy_variations() {
    use tje_game::execution::ProofEnvelope;

    let canonical = tje_game::translations()
        .find(|entry| entry.id == ids::GET_RANDOM_NUMBER)
        .unwrap();
    for revision in [Revision::Rev02, Revision::Rev00] {
        for envelope in [
            ProofEnvelope::ClockedStrict,
            ProofEnvelope::ClockedCommutingInterrupts,
        ] {
            let mut registry =
                HookRegistry::from_translations(revision, std::slice::from_ref(canonical)).unwrap();
            let target = canonical.id.metadata().address(revision);
            // Explicit cfg(test)-only owner-policy variation on a REAL canonical
            // body. It neither promotes a pending id nor bypasses an origin guard.
            let hook = registry.hooks.get_mut(&target).unwrap();
            hook.proof_envelope = envelope;
            assert_eq!(hook.execution_class, ExecutionClass::Atomic);
            assert_eq!(hook.body_provenance(), BodyProvenance::CanonicalProduction);
            assert!(!hook.proof_only_admission());
            registry.ensure_replacement_admission().unwrap();
            let mut host = crate::fixture::tests::synthetic_host(
                &[0x4eb9, (target >> 16) as u16, target as u16, 0x60fe],
                &[0x4e73],
            );
            host.revision = revision;
            host.mode = Mode::Replace;
            host.registry = registry;
            host.checks = crate::CheckReport::registered(&host.registry);
            let state = tje_mem::ram::g_rng_state(revision);
            state.write(&mut host.context(), 1);
            host.play_frame(Default::default()).unwrap();
            assert_ne!(state.read(&host.context()), 1);
            assert_eq!(host.checks.by_function["GetRandomNumber"].calls_replaced, 1);
            assert_eq!(host.registry.at(target).unwrap().proof_envelope, envelope);
        }
    }
}

#[test]
fn canonical_index_conflicts_and_overflow_never_approve_a_body() {
    let canonical = TranslatedEntry::Values(CANONICAL_VALUES[0]);
    for entries in [
        [canonical, canonical],
        [canonical, TranslatedEntry::Framed(CHANGED_FRAMES[0])],
    ] {
        let bodies = index_canonical_bodies(entries);
        let index = bodies
            .binary_search_by_key(&Some(ids::ABS), CanonicalBody::id)
            .unwrap();
        assert!(matches!(bodies[index], CanonicalBody::Conflict(id) if id == ids::ABS));
        assert_eq!(
            bodies
                .iter()
                .filter(|slot| matches!(slot, CanonicalBody::Conflict(_)))
                .count(),
            2,
        );
    }
    let overflow = index_canonical_bodies(std::iter::repeat_n(canonical, 649));
    assert!(
        overflow
            .iter()
            .all(|slot| matches!(slot, CanonicalBody::Conflict(_)))
    );
}
