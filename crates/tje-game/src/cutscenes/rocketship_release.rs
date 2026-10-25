use crate::{Context, Result, call_void, ids};
use tje_mem::ram;

/// G/src/cutscenes/rocketship_release.c:6 (`CutsceneReleaseRocketshipPair`).
/// Atomic: RAM-only frees, retaining exhaust-before-ship order and slot clears.
pub fn cutscene_release_rocketship_pair(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let exhaust = ram::g_cutscene_rocketship_exhaust_entity(revision);
    if exhaust.read(context) != 0 {
        let handle = exhaust.read(context);
        call_void(context, ids::FREE_ENTITY, &[u32::from(handle)])?;
    }
    exhaust.write(context, 0);
    let ship = ram::g_cutscene_rocketship_entity(revision);
    if ship.read(context) != 0 {
        let handle = ship.read(context);
        call_void(context, ids::FREE_ENTITY, &[u32::from(handle)])?;
    }
    ship.write(context, 0);
    Ok(None)
}
