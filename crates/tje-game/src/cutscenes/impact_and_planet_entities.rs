use super::large_player_parts::entity_at;
use crate::{Context, Result, call, ids};
use tje_mem::{Ptr, ram, rom};

/// G/src/cutscenes/impact_and_planet_entities.c:15 (`IntroCreateRocketshipImpact`).
/// Atomic: fixed RAM stores and RAM-only viewport allocation.
pub fn intro_create_rocketship_impact(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_intro_rocketship_impact_entity(revision);
    let allocated = call(context, ids::ALLOCATE_ENTITY, &[])? as u16;
    selector.write(context, allocated);
    if selector.read(context) != 0 {
        let object = entity_at(context, selector.read(context) as i16);
        object.tile().write(context, 0);
        let frame = rom::spr_intro_rocketship_impact(revision)
            .at(0)
            .read(context);
        object.current_frame().write(context, frame);
        object.frames().write(
            context,
            Ptr::new(rom::spr_intro_rocketship_impact(revision).address()),
        );
        object.tile_count().write(context, 8);
        object.sprite_count().write(context, 1);
        object.x().write(context, arguments[0] as i16);
        object.y().write(context, arguments[1] as i16);
        object.sprite_ready().write(context, 0);
        object.sequence().write(
            context,
            Ptr::new(rom::intro_frame_index_lists(revision).address()),
        );
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        object.frame_phase().write(context, 6);
        call(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/impact_and_planet_entities.c:46 (`CutsceneCreatePlanetEntity`).
/// Atomic: fixed RAM stores and RAM-only viewport allocation.
pub fn cutscene_create_planet_entity(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_cutscene_planet_entity(revision);
    let allocated = call(context, ids::ALLOCATE_ENTITY, &[])? as u16;
    selector.write(context, allocated);
    if selector.read(context) != 0 {
        let object = entity_at(context, selector.read(context) as i16);
        object.tile().write(context, 0);
        object.current_frame().write(
            context,
            Ptr::new(rom::spr_planet_frame00(revision).address()),
        );
        object
            .frames()
            .write(context, Ptr::new(rom::spr_planet(revision).address()));
        object.tile_count().write(context, 72);
        object.sprite_count().write(context, 6);
        object.x().write(context, arguments[0] as i16);
        object.y().write(context, arguments[1] as i16);
        object.sprite_ready().write(context, 0);
        object.sequence().write(
            context,
            Ptr::new(
                rom::intro_held_frame_streams(revision)
                    .at(0)
                    .frame()
                    .address(),
            ),
        );
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        object.frame_phase().write(context, 6);
        call(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/impact_and_planet_entities.c:77 (`EndingCreateStationaryComposite`).
/// Atomic. ORIGINAL BUG: current_frame stores the frame-table address, not its
/// first entry. Preserve the bad pointer and its RAM-only sprite reservation.
pub fn ending_create_stationary_composite(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_ending_stationary_composite_entity(revision);
    let allocated = call(context, ids::ALLOCATE_ENTITY, &[])? as u16;
    selector.write(context, allocated);
    if selector.read(context) != 0 {
        let object = entity_at(context, selector.read(context) as i16);
        object.tile().write(context, 0);
        object.current_frame().write(
            context,
            Ptr::new(rom::ending_stationary_composite_frames(revision).address()),
        );
        object.frames().write(
            context,
            Ptr::new(rom::ending_stationary_composite_frames(revision).address()),
        );
        object.tile_count().write(context, 72);
        object.sprite_count().write(context, 6);
        object.x().write(context, arguments[0] as i16);
        object.y().write(context, arguments[1] as i16);
        object.sprite_ready().write(context, 0);
        object.sequence().write(
            context,
            Ptr::new(
                rom::intro_held_frame_streams(revision)
                    .at(0)
                    .frame()
                    .address(),
            ),
        );
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        object.frame_phase().write(context, 6);
        call(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/impact_and_planet_entities.c:39 (`IntroReleaseRocketshipImpact`).
/// Atomic. The original intentionally does not clear the released selector.
pub fn intro_release_rocketship_impact(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let slot = ram::g_intro_rocketship_impact_entity(context.revision()).read(context);
    if slot != 0 {
        call(context, ids::FREE_ENTITY, &[u32::from(slot)])?;
    }
    Ok(None)
}

/// G/src/cutscenes/impact_and_planet_entities.c:70 (`CutsceneReleasePlanetEntity`).
/// Atomic. The original intentionally does not clear the released selector.
pub fn cutscene_release_planet_entity(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let slot = ram::g_cutscene_planet_entity(context.revision()).read(context);
    if slot != 0 {
        call(context, ids::FREE_ENTITY, &[u32::from(slot)])?;
    }
    Ok(None)
}

/// G/src/cutscenes/impact_and_planet_entities.c:103 (`EndingReleaseStationaryComposite`).
/// Atomic. The original intentionally does not clear the released selector.
pub fn ending_release_stationary_composite(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let slot = ram::g_ending_stationary_composite_entity(context.revision()).read(context);
    if slot != 0 {
        call(context, ids::FREE_ENTITY, &[u32::from(slot)])?;
    }
    Ok(None)
}
