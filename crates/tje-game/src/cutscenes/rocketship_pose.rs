use super::large_player_parts::{allocate_viewport_with_d4_word, entity_at};
use crate::{Context, Result, call, ids};
use tje_mem::{Ptr, ram, rom};

/// G/src/cutscenes/rocketship_pose.c:10 (`CutsceneSetRocketshipPairPose`).
/// Atomic: finish each object's RAM-only viewport release/stores/allocation before
/// reading the next selector; ROM table and held-stream offsets use word shifts.
pub fn cutscene_set_rocketship_pair_pose(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let animation = arguments[0] as i16;
    let slot = ram::g_cutscene_rocketship_entity(revision).read(context);
    let object = entity_at(context, i16::from(slot));
    if object.draw_flags().read(context) & 2 != 0 {
        call(
            context,
            ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), 0],
        )?;
    }
    object.x().write(context, arguments[1] as i16);
    object.y().write(context, arguments[2] as i16);
    object.sprite_ready().write(context, 0);
    let sequence = rom::intro_held_frame_streams(revision)
        .address()
        .wrapping_add(i32::from(animation.wrapping_mul(2)) as u32);
    object.sequence().write(context, Ptr::new(sequence));
    object.frame_duration().write(context, 1);
    object.frame().write(context, -1);
    allocate_viewport_with_d4_word(context, object, arguments[2] as i16)?;
    let slot = ram::g_cutscene_rocketship_exhaust_entity(revision).read(context);
    let object = entity_at(context, i16::from(slot));
    if object.draw_flags().read(context) & 2 != 0 {
        call(
            context,
            ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), 0],
        )?;
    }
    object.x().write(context, arguments[1] as i16);
    object.y().write(context, arguments[2] as i16);
    object.sprite_ready().write(context, 0);
    let sequence_address = rom::intro_animation_pointers(revision)
        .address()
        .wrapping_add(i32::from(animation.wrapping_shl(2)) as u32);
    let sequence = context.read_u32(sequence_address);
    object.sequence().write(context, Ptr::new(sequence));
    object.frame_duration().write(context, 1);
    object.frame().write(context, -1);
    allocate_viewport_with_d4_word(context, object, arguments[2] as i16)?;
    Ok(None)
}
