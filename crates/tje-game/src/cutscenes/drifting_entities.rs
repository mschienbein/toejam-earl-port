use super::large_player_parts::{allocate_viewport_with_d4_word, entity_at, reserve_frame_pieces};
use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Ptr, ram, rom};

/// G/src/cutscenes/drifting_entities.c:17 (`CutsceneCreateDriftingEntities`).
/// Atomic: ROM dimensions, RAM allocation and RAM-seeded random draws only.
pub fn cutscene_create_drifting_entities(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..8 {
        let frame = rom::cutscene_drifting_entity_frames(revision)
            .at(index)
            .read(context)
            .target();
        let slot = call(context, ids::ALLOCATE_ENTITY, &[])? as i16;
        if slot != 0 {
            let object = entity_at(context, slot);
            let piece_count = reserve_frame_pieces(context, object, frame);
            object.tile().write(context, 0);
            object
                .current_frame()
                .write(context, Ptr::new(frame.address()));
            let x = ((call(context, ids::GET_RANDOM_NUMBER, &[])? as i32) % 320 - 70) as i16;
            object.x().write(context, x);
            let velocity = ((call(context, ids::GET_RANDOM_NUMBER, &[])? as i32 & 1) + 1) as i16;
            object.velocity_x().write(context, velocity);
            loop {
                let y = ((call(context, ids::GET_RANDOM_NUMBER, &[])? as i32) % 148 + 60) as i16;
                object.y().write(context, y);
                if object.y().read(context) <= 150 || object.y().read(context) >= 170 {
                    break;
                }
            }
            object.sequence().write(
                context,
                Ptr::new(
                    rom::intro_held_frame_streams(revision)
                        .at(index)
                        .frame()
                        .address(),
                ),
            );
            object.frames().write(
                context,
                Ptr::new(rom::cutscene_drifting_entity_frames(revision).address()),
            );
            object.frame_phase().write(context, 1);
            object.frame_duration().write(context, 1);
            object.frame().write(context, -1);
            // The compiler's D4 piece counter survives all intervening RNG calls.
            allocate_viewport_with_d4_word(context, object, piece_count)?;
            object.sprite_ready().write(context, 0);
        }
        ram::g_cutscene_drifting_entities(revision)
            .at(index)
            .write(context, slot as u8);
    }
    Ok(None)
}

/// G/src/cutscenes/drifting_entities.c:68 (`CutsceneUpdateDriftingEntities`).
/// Atomic: fraction byte is promoted unsigned, including comparison to a negative
/// velocity word; `scene` is intentionally unused, as in the matching source.
pub fn cutscene_update_drifting_entities(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..8 {
        let slot = ram::g_cutscene_drifting_entities(revision)
            .at(index)
            .read(context);
        if slot != 0 {
            let object = entity_at(context, i16::from(slot));
            let fraction = object.fraction_x().read(context).wrapping_add(1);
            object.fraction_x().write(context, fraction);
            if u32::from(object.fraction_x().read(context))
                > i32::from(object.velocity_x().read(context)) as u32
            {
                let x = object.x().read(context).wrapping_add(1);
                object.x().write(context, x);
                object.fraction_x().write(context, 0);
                if object.x().read(context) > 370 {
                    let x = ((call(context, ids::GET_RANDOM_NUMBER, &[])? as i32) % 30 - 70) as i16;
                    object.x().write(context, x);
                    let velocity =
                        ((call(context, ids::GET_RANDOM_NUMBER, &[])? as i32 & 1) + 1) as i16;
                    object.velocity_x().write(context, velocity);
                    loop {
                        let y = ((call(context, ids::GET_RANDOM_NUMBER, &[])? as i32) % 148 + 60)
                            as i16;
                        object.y().write(context, y);
                        if object.y().read(context) <= 150 || object.y().read(context) >= 170 {
                            break;
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

/// G/src/cutscenes/drifting_entities.c:94 (`IntroAllocateFlyingEntitySlots`).
/// Atomic: four RAM-only allocations; failed allocations still clear selectors.
pub fn intro_allocate_flying_entity_slots(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..4 {
        let slot = call(context, ids::ALLOCATE_ENTITY, &[])? as i16;
        if slot != 0 {
            let object = entity_at(context, slot);
            object.sprite_count().write(context, 4);
            object.tile_count().write(context, 40);
            object.tile().write(context, 0);
        }
        ram::g_intro_flying_entities(revision)
            .at(index)
            .write(context, slot as u8);
    }
    Ok(None)
}

/// G/src/cutscenes/drifting_entities.c:55 (`CutsceneReleaseDriftingEntities`).
/// Atomic: eight ordered RAM-only releases and selector clears.
pub fn cutscene_release_drifting_entities(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..8 {
        let selector = ram::g_cutscene_drifting_entities(revision).at(index);
        let slot = selector.read(context);
        if slot != 0 {
            call_void(context, ids::FREE_ENTITY, &[u32::from(slot)])?;
        }
        selector.write(context, 0);
    }
    Ok(None)
}

/// G/src/cutscenes/drifting_entities.c:111 (`IntroReleaseFlyingEntitySlots`).
/// Atomic: four ordered RAM-only releases and selector clears.
pub fn intro_release_flying_entity_slots(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..4 {
        let selector = ram::g_intro_flying_entities(revision).at(index);
        let slot = selector.read(context);
        if slot != 0 {
            call_void(context, ids::FREE_ENTITY, &[u32::from(slot)])?;
        }
        selector.write(context, 0);
    }
    Ok(None)
}
