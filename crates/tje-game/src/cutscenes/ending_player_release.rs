use crate::{Context, Result, call_void, ids};
use tje_mem::ram;

/// G/src/cutscenes/ending_player_release.c:8 (`EndingReleaseActivePlayerEntities`).
/// Atomic: release and clear player one before reading player two's selector.
pub fn ending_release_active_player_entities(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let first = ram::g_ending_player1_entity(revision);
    let slot = first.read(context);
    if slot != 0 {
        call_void(context, ids::FREE_ENTITY, &[u32::from(slot)])?;
    }
    first.write(context, 0);
    let second = ram::g_ending_player2_entity(revision);
    let slot = second.read(context);
    if slot != 0 {
        call_void(context, ids::FREE_ENTITY, &[u32::from(slot)])?;
    }
    second.write(context, 0);
    Ok(None)
}
