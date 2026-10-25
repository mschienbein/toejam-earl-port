//! Registry-only admission backed by corpus-d754b68-final-nine.json (83 inputs, both ROMs).
use std::collections::BTreeSet;
use tje_game::{Dispatcher, FunctionId, TranslatedEntry, audio, core, ids, player, sprites};

const VALUES: &[FunctionId] = &[
    ids::AI_BOOGIE_TOUCH,
    ids::LEMONADE_ABILITY,
    ids::BUILD_VIEWPORT_SPRITE_LINKS,
    ids::SELECT_VIEWPORT_SPRITE_CHAIN,
    ids::LOAD_SOUND_DRIVER_TO_Z80,
];
const FRAMED: &[FunctionId] = &[
    ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT,
    ids::SPRITE_CHECKS,
    ids::ALLOCATE_SPRITE_SLOTS,
    ids::ALLOCATE_SPRITE_SLOT,
];

#[test]
fn corpus_nine_use_the_confirmed_canonical_entry_forms() {
    let dispatcher = Dispatcher::hybrid();
    dispatcher.validate_entries().unwrap();
    for &id in VALUES {
        let selected = dispatcher.resolve_entry(id).unwrap().unwrap();
        assert!(selected.is_canonical_production());
        assert!(matches!(selected.entry(), TranslatedEntry::Values(_)));
    }
    for &id in FRAMED {
        let selected = dispatcher.resolve_entry(id).unwrap().unwrap();
        assert!(selected.is_canonical_production());
        assert!(matches!(selected.entry(), TranslatedEntry::Framed(_)));
    }
}

#[test]
fn promoted_owners_have_disjoint_production_and_pending_entries() {
    let production: BTreeSet<_> = tje_game::translations()
        .map(|entry| entry.id)
        .chain(tje_game::framed_translations().map(|entry| entry.id))
        .collect();
    let pending_ids = audio::PENDING
        .iter()
        .chain(core::PENDING)
        .chain(player::PENDING)
        .chain(sprites::PENDING)
        .copied();
    let pending_values = audio::PENDING_TRANSLATIONS
        .iter()
        .chain(core::PENDING_TRANSLATIONS)
        .chain(player::PENDING_TRANSLATIONS)
        .chain(sprites::PENDING_TRANSLATIONS)
        .map(|entry| entry.id);
    let pending_frames = audio::PENDING_FRAMED_TRANSLATIONS
        .iter()
        .chain(core::PENDING_FRAMED_TRANSLATIONS)
        .chain(player::PENDING_FRAMED_TRANSLATIONS)
        .chain(sprites::PENDING_FRAMED_TRANSLATIONS)
        .map(|entry| entry.id);
    for id in pending_ids.chain(pending_values).chain(pending_frames) {
        assert!(
            !production.contains(&id),
            "production entry remains pending: {}",
            id.metadata().inventory_id
        );
    }
}
