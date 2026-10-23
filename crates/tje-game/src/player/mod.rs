//! Player translation ownership; see docs/PORTING.md.

use crate::{Context, Result, ids};
use tje_mem::{Address, ObjectState, ram};

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::STOP_PLAYER,
        function: stop_player,
    },
    crate::Translation {
        id: ids::PLAYER_INITIALIZE_PHASED_JUMP_MOTION,
        function: player_initialize_phased_jump_motion,
    },
    crate::Translation {
        id: ids::ENTITY_HAS_FREE_TIMED_EFFECT_SLOT,
        function: entity_has_free_timed_effect_slot,
    },
];

/// Reviewed original entries that return without clock/interrupt progress.
/// Missing IDs are deliberately not approved for atomic checking or fallback.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    ids::STOP_PLAYER,
    ids::PLAYER_INITIALIZE_PHASED_JUMP_MOTION,
    ids::ENTITY_HAS_FREE_TIMED_EFFECT_SLOT,
];
/// G/src/player/entity_motion_terrain.c:303 (`StopPlayer`). Preserve the original
/// interleaved velocity/acceleration store order, even on aliased addresses.
pub fn stop_player(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    state.velocity_x().write(context, 0);
    state.acceleration_x().write(context, 0);
    state.velocity_y().write(context, 0);
    state.acceleration_y().write(context, 0);
    state.velocity_height().write(context, 0);
    state.acceleration_height().write(context, 0);
    Ok(None)
}
/// G/src/player/start_motion_mode.c:5 (`PlayerInitializePhasedJumpMotion`).
pub fn player_initialize_phased_jump_motion(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    ram::g_player_phased_jump_phase(revision)
        .at(player)
        .write(context, 1);
    ram::g_player_phased_jump_phase_ticks(revision)
        .at(player)
        .write(context, 16);
    ram::g_player_phased_jump_armed(revision)
        .at(player)
        .write(context, 1);
    ram::g_player_phased_jump_speech_used(revision)
        .at(player)
        .write(context, 0);
    Ok(None)
}
/// G/src/player/effect_slots.c:17 (`EntityHasFreeTimedEffectSlot`). Signed slot
/// indices use C truth, not a positive-only host predicate.
pub fn entity_has_free_timed_effect_slot(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let slots = ram::g_timed_effect_entities(context.revision());
    Ok(Some(u32::from(
        slots.at(0).read(context) == 0 || slots.at(1).read(context) == 0,
    )))
}
