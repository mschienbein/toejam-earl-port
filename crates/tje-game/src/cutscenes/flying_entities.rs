use super::large_player_parts::entity_at;
use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Ptr, ram, rom};

/// G/src/cutscenes/flying_entities.c:15 (`IntroUpdateRightwardFlyingEntity`).
/// Atomic: RAM-only motion, RNG and viewport sprite lifecycle. Preserve the
/// first scene's superfluous GetRandomNumber(0) argument and low-bit flag test.
pub fn intro_update_rightward_flying_entity(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let restart = arguments[0] as i16;
    let scene = arguments[1] as i16;
    if scene == 0 {
        return Ok(None);
    }
    let slot = ram::g_intro_flying_entities(revision).at(0).read(context);
    let object = entity_at(context, i16::from(slot));
    if restart != 0 {
        if object.draw_flags().read(context) & 3 != 0 {
            call_void(
                context,
                ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                &[object.address(), 0],
            )?;
        }
        if scene == 1 {
            object.y().write(context, 94);
            let variant = (call(context, ids::GET_RANDOM_NUMBER, &[0])? as i32) % 3;
            object.sequence().write(
                context,
                Ptr::new(
                    rom::intro_held_frame_streams(revision)
                        .at(variant)
                        .frame()
                        .address(),
                ),
            );
            object.height().write(context, 0);
        } else if scene == 2 {
            object.y().write(context, 192);
            object.height().write(context, 80);
            let variant = (call(context, ids::GET_RANDOM_NUMBER, &[])? as i32 & 1) + 3;
            object.sequence().write(
                context,
                Ptr::new(
                    rom::intro_held_frame_streams(revision)
                        .at(variant)
                        .frame()
                        .address(),
                ),
            );
        }
        object.frames().write(
            context,
            Ptr::new(rom::intro_flying_entity_frames(revision).address()),
        );
        object.frame_phase().write(context, 6);
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        object.sprite_ready().write(context, 0);
        object.x().write(context, 96);
        call_void(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), 0],
        )?;
    }
    let x = object.x().read(context).wrapping_add(10);
    object.x().write(context, x);
    if object.x().read(context) > 232 && object.draw_flags().read(context) & 3 != 0 {
        call_void(
            context,
            ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/flying_entities.c:50 (`IntroUpdateFlyingEntityMotion`).
/// Atomic: RAM-only motion, RNG and viewport sprite lifecycle. ORIGINAL BUG:
/// variant's &1 mask prevents the alternate-frame-table branches (2 and 3).
pub fn intro_update_flying_entity_motion(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let scene = arguments[0] as i16;
    let (spawn_x, drift_x): (i16, i16) = match scene {
        0 => {
            for index in 0..4 {
                let slot = ram::g_intro_flying_entities(revision)
                    .at(index)
                    .read(context);
                if slot != 0 {
                    let object = entity_at(context, i16::from(slot));
                    call_void(
                        context,
                        ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                        &[object.address(), 0],
                    )?;
                }
            }
            return Ok(None);
        }
        2 => (248, -12),
        3 => (88, 12),
        _ => (168, 0),
    };
    // G's scene 2/3 MOVEQ writes all of D4; the default CLR.W preserves its
    // upper word. D4 is drift_x when the allocator's uninitialized-tile bug runs.
    let saved_d4 = context.data_register(4);
    let allocator_d4 = match scene {
        2 => (-12_i32) as u32,
        3 => 12,
        _ => saved_d4 & 0xffff_0000,
    };
    for index in 0..4 {
        let slot = ram::g_intro_flying_entities(revision)
            .at(index)
            .read(context);
        if slot == 0 {
            continue;
        }
        let object = entity_at(context, i16::from(slot));
        if object.x().read(context) < 50
            || object.x().read(context) > 270
            || object.height().read(context) > -18
            || object.height().read(context) < -198
        {
            if object.draw_flags().read(context) & 2 != 0 {
                call_void(
                    context,
                    ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                    &[object.address(), 0],
                )?;
            }
            object.x().write(context, spawn_x);
            object.y().write(context, 0);
            object.height().write(context, -100);
            if scene == 4 {
                object.sequence().write(
                    context,
                    Ptr::new(rom::intro_flying_scene_four_animation(revision).address()),
                );
                object.frames().write(
                    context,
                    Ptr::new(rom::intro_flying_entity_frames(revision).address()),
                );
                object.velocity_x().write(context, 0);
                object.velocity_y().write(context, 0);
                object.velocity_height().write(context, -1);
            } else {
                let velocity = ((call(context, ids::GET_RANDOM_NUMBER, &[])? as i32) % 4) as i16;
                object.velocity_x().write(context, velocity);
                let velocity = 4_i16.wrapping_sub(object.velocity_x().read(context));
                object.velocity_y().write(context, velocity);
                if call(context, ids::GET_RANDOM_NUMBER, &[])? as i32 & 1 != 0 {
                    let velocity = object.velocity_x().read(context).wrapping_neg();
                    object.velocity_x().write(context, velocity);
                }
                if call(context, ids::GET_RANDOM_NUMBER, &[])? as i32 & 1 != 0 {
                    let velocity = object.velocity_y().read(context).wrapping_neg();
                    object.velocity_y().write(context, velocity);
                }
                let velocity =
                    (call(context, ids::GET_RANDOM_NUMBER, &[])? as i32).wrapping_neg() % 12;
                object.velocity_height().write(context, velocity as i16);
                let variant = (call(context, ids::GET_RANDOM_NUMBER, &[])? as i32 & 1) as i16;
                if variant == 0 {
                    object.sequence().write(
                        context,
                        Ptr::new(rom::intro_flying_forward_frames_animation(revision).address()),
                    );
                    object.frames().write(
                        context,
                        Ptr::new(rom::intro_flying_entity_frames(revision).address()),
                    );
                } else if variant == 1 {
                    object.sequence().write(
                        context,
                        Ptr::new(rom::intro_flying_flipped_frames_animation(revision).address()),
                    );
                    object.frames().write(
                        context,
                        Ptr::new(rom::intro_flying_entity_frames(revision).address()),
                    );
                } else if variant == 2 {
                    object.sequence().write(
                        context,
                        Ptr::new(rom::intro_flying_flipped_frames_animation(revision).address()),
                    );
                    object.frames().write(
                        context,
                        Ptr::new(rom::intro_flying_alternate_entity_frames(revision).address()),
                    );
                } else if variant == 3 {
                    object.sequence().write(
                        context,
                        Ptr::new(rom::intro_flying_forward_frames_animation(revision).address()),
                    );
                    object.frames().write(
                        context,
                        Ptr::new(rom::intro_flying_alternate_entity_frames(revision).address()),
                    );
                }
            }
            let frame = object.frames().read(context).target().read(context);
            object.current_frame().write(context, frame);
            object.frame_phase().write(context, 5);
            object.frame_duration().write(context, 1);
            object.frame().write(context, -1);
            object.sprite_ready().write(context, 0);
        } else {
            let y = object.y().read(context).wrapping_add(1);
            object.y().write(context, y);
            let height = object.height().read(context).wrapping_add(1);
            object.height().write(context, height);
        }
        let x = object
            .x()
            .read(context)
            .wrapping_add(object.velocity_x().read(context))
            .wrapping_add(drift_x);
        object.x().write(context, x);
        let height = object
            .height()
            .read(context)
            .wrapping_add(object.velocity_y().read(context));
        object.height().write(context, height);
        let velocity = object.velocity_height().read(context).wrapping_add(1);
        object.velocity_height().write(context, velocity);
        if object.velocity_height().read(context) == 0 {
            context.set_data_register(4, allocator_d4);
            let result = call_void(
                context,
                ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
                &[object.address(), 0],
            );
            context.set_data_register(4, saved_d4);
            result?;
            object.sprite_ready().write(context, 0);
        }
    }
    Ok(None)
}
