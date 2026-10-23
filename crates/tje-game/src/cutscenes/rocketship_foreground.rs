use crate::{Context, Result, call, ids};
use tje_mem::{Ptr, ram, rom};

/// G/src/cutscenes/rocketship_foreground.c:11 (`CutsceneSetForegroundRocketshipPose`).
/// Atomic: release/allocation updates RAM bookkeeping without device transfers.
pub fn cutscene_set_foreground_rocketship_pose(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let animation = arguments[0] as i16;
    let handle = ram::g_cutscene_rocketship_entity(revision).read(context);
    let object = ram::g_player_info(revision).at(i32::from(handle));
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
    let tile = object.tile().read(context);
    object.tile().write(context, tile | 0x8000);
    // ASL.W/ADDA.W retain word-wrapped indexing even for an invalid selector.
    let sequence = rom::intro_held_frame_streams(revision)
        .cast::<i8>()
        .byte_offset(i32::from(animation.wrapping_mul(2)));
    object
        .sequence()
        .write(context, Ptr::new(sequence.address()));
    object.frame_duration().write(context, 1);
    object.frame().write(context, -1);
    // Matching caller loads Y into D4.W; the allocator's ORIGINAL BUG can read it.
    super::allocate_viewport_with_d4(context, object.address(), arguments[2] as u16)?;
    let handle = ram::g_cutscene_rocketship_exhaust_entity(revision).read(context);
    let object = ram::g_player_info(revision).at(i32::from(handle));
    if object.draw_flags().read(context) & 2 != 0 {
        call(
            context,
            ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), 0],
        )?;
    }
    Ok(None)
}
