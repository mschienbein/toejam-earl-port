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
        assert_eq!(mode.entries.len(), 1);
        assert_eq!(mode.entries[0].0, ids::ABS.metadata().address(revision));
        assert!(!mode.entries[0].1.is_canonical_production());
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
