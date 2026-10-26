use super::*;
use tje_game::{Context, Result, ids};

fn replacement(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    Ok(Some(42))
}
static VALUES: &[Translation] = &[Translation {
    id: ids::ABS,
    function: replacement,
}];

#[test]
fn selection_is_separate_from_proven_dispatch_and_default_off() {
    let profile = EnhancedProfile::new("authored-test-v1", VALUES, &[]);
    assert!(
        profile
            .dispatcher()
            .resolve_entry(ids::ABS)
            .unwrap()
            .is_some()
    );
    assert!(
        profile
            .dispatcher()
            .resolve_entry(ids::LMULT)
            .unwrap()
            .is_none()
    );
    assert!(
        Dispatcher::hybrid()
            .resolve_entry(ids::LMULT)
            .unwrap()
            .is_some()
    );
    assert_eq!(EnhancedMode::default().profile_id(), "faithful");
    for revision in Revision::ALL {
        let mode = EnhancedMode::new(profile, revision).unwrap();
        assert_eq!(mode.driver.entries.len(), 1);
        assert_eq!(
            mode.driver.entries[0].0,
            ids::ABS.metadata().address(revision)
        );
        assert!(!mode.driver.entries[0].1.is_canonical_production());
    }
}

#[test]
fn invalid_and_ambiguous_profiles_reject_at_selection() {
    static DUPLICATE: &[Translation] = &[
        Translation {
            id: ids::ABS,
            function: replacement,
        },
        Translation {
            id: ids::ABS,
            function: replacement,
        },
    ];
    static BLOCKING: &[Translation] = &[Translation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: replacement,
    }];
    for profile in [
        EnhancedProfile::new("", VALUES, &[]),
        EnhancedProfile::new("faithful", VALUES, &[]),
        EnhancedProfile::new("empty-v1", &[], &[]),
        EnhancedProfile::new("duplicate-v1", DUPLICATE, &[]),
        EnhancedProfile::new("blocking-v1", BLOCKING, &[]),
    ] {
        assert!(EnhancedMode::new(profile, Revision::Rev02).is_err());
    }
}

#[test]
fn enhanced_error_keeps_the_owned_game_error_source_chain() {
    let error = super::Error::<std::convert::Infallible>::Override(
        tje_game::Error::EnhancedExecutionFailure {
            entry: ids::ABS,
            cause: Box::new(tje_game::Error::Platform("authored first cause".into())),
        },
    );
    let source = std::error::Error::source(&error).unwrap();
    assert!(source.downcast_ref::<tje_game::Error>().is_some());
    assert_eq!(source.source().unwrap().to_string(), "authored first cause");
}

#[test]
fn resumable_sets_compose_timeaware_entries_but_never_duplicate_addresses() {
    fn resume(_: &mut dyn super::Context, _: &mut Continuation) -> Result<OverrideStep> {
        Ok(OverrideStep::Return(None))
    }
    static WAIT: &[ResumableTranslation] = &[ResumableTranslation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: resume,
    }];
    static ABS: &[ResumableTranslation] = &[ResumableTranslation {
        id: ids::ABS,
        function: resume,
    }];
    static SETS: &[&[ResumableTranslation]] = &[WAIT, ABS];
    let profile = EnhancedProfile::new("composed-resumable-v1", &[], &[]).with_resumable_sets(SETS);
    for revision in Revision::ALL {
        assert_eq!(
            EnhancedMode::new(profile, revision)
                .unwrap()
                .driver
                .resumable
                .len(),
            2
        );
        assert!(EnhancedMode::new(profile.with_resumable(WAIT), revision).is_err());
        assert!(
            EnhancedMode::new(
                EnhancedProfile::new("legacy-conflict-v1", VALUES, &[]).with_resumable(ABS),
                revision,
            )
            .is_err()
        );
    }
}

#[test]
fn immutable_filtered_selection_never_resurrects_disabled_nested_entries() {
    static ENTRIES: &[Translation] = &[
        Translation {
            id: ids::ABS,
            function: replacement,
        },
        Translation {
            id: ids::LMULT,
            function: replacement,
        },
    ];
    let profile = EnhancedProfile::new("filtered-v1", ENTRIES, &[]);
    for revision in Revision::ALL {
        let mode = EnhancedMode::new_selected(profile, revision, |id| id == ids::ABS).unwrap();
        assert_eq!(mode.driver.entries.len(), 1);
        assert_eq!(mode.driver.entries[0].1.id(), ids::ABS);
        assert!(
            mode.driver
                .dispatcher()
                .resolve_entry(ids::LMULT)
                .unwrap()
                .is_none()
        );
        assert!(EnhancedMode::new_selected(profile, revision, |_| false).is_err());
    }
}

#[test]
fn original_entry_fences_cannot_alias_selected_overrides() {
    fn abs(id: tje_game::FunctionId) -> bool {
        id == ids::ABS
    }
    fn check(_: &Sidecars, _: tje_game::FunctionId) -> Result<()> {
        unreachable!("ambiguous profile must reject before execution")
    }
    let profile = EnhancedProfile::new("authored-overlapping-fence-v1", VALUES, &[])
        .with_original_guard(OriginalEntryGuard {
            selected: abs,
            check,
        });
    for revision in Revision::ALL {
        assert!(EnhancedMode::new(profile, revision).is_err());
    }
}
