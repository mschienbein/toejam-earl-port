//! Availability metadata must not grant production/native admission.
use std::collections::BTreeSet;
use tje_game::{Dispatcher, FunctionId};

#[test]
fn canonical_availability_is_unique_owned_and_completes_the_catalogue() {
    let mut available = BTreeSet::new();
    let mut pending = BTreeSet::new();
    let mut replayed = BTreeSet::new();
    for owner in tje_game::SUBSYSTEMS {
        for id in owner
            .translations
            .iter()
            .map(|body| body.id)
            .chain(owner.framed_translations.iter().map(|body| body.id))
        {
            assert_eq!(id.metadata().subsystem, owner.name);
            assert!(available.insert(id), "duplicate production body: {id:?}");
        }
        for id in owner
            .replayed_translations
            .iter()
            .map(|body| body.id)
            .chain(
                owner
                    .replayed_framed_translations
                    .iter()
                    .map(|body| body.id),
            )
        {
            assert_eq!(id.metadata().subsystem, owner.name);
            assert!(available.insert(id), "ambiguous replayed body form: {id:?}");
            assert!(replayed.insert(id));
            assert!(Dispatcher::hybrid().resolve_entry(id).unwrap().is_none());
            assert!(Dispatcher::native(&[id]).is_err());
        }
        for id in owner
            .pending_translations
            .iter()
            .map(|body| body.id)
            .chain(owner.pending_framed_translations.iter().map(|body| body.id))
        {
            assert_eq!(id.metadata().subsystem, owner.name);
            assert!(
                available.insert(id),
                "ambiguous canonical body form: {id:?}"
            );
            assert!(pending.insert(id));
            assert!(Dispatcher::hybrid().resolve_entry(id).unwrap().is_none());
            assert!(Dispatcher::native(&[id]).is_err());
        }
    }
    let catalogue: BTreeSet<FunctionId> = tje_game::functions().map(|entry| entry.id).collect();
    assert_eq!(
        available, catalogue,
        "a canonical body is missing from shared availability"
    );
    let getter_values: BTreeSet<_> = tje_game::pending_translations()
        .map(|body| body.id)
        .collect();
    let getter_frames: BTreeSet<_> = tje_game::pending_framed_translations()
        .map(|body| body.id)
        .collect();
    assert!(getter_values.is_disjoint(&getter_frames));
    assert_eq!(
        getter_values
            .union(&getter_frames)
            .copied()
            .collect::<BTreeSet<_>>(),
        pending
    );
    let values = tje_game::pending_translations();
    assert_eq!(values.clone().count(), values.count());
    let frames = tje_game::pending_framed_translations();
    assert_eq!(frames.clone().count(), frames.count());
    let replayed_values: BTreeSet<_> = tje_game::replayed_translations()
        .map(|body| body.id)
        .collect();
    let replayed_frames: BTreeSet<_> = tje_game::replayed_framed_translations()
        .map(|body| body.id)
        .collect();
    assert!(replayed_values.is_disjoint(&replayed_frames));
    assert_eq!(
        replayed_values
            .union(&replayed_frames)
            .copied()
            .collect::<BTreeSet<_>>(),
        replayed
    );
}
