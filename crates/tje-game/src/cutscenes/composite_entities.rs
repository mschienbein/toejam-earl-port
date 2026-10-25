use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Ptr, ram, rom};

/// G/src/cutscenes/composite_entities.c:33 (`EndingCreatePoseComposite`).
/// Atomic: pool/viewport allocation and held-frame setup touch RAM/ROM only.
pub fn ending_create_pose_composite(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as u8;
    ram::g_ending_pose_composite_entity(revision).write(context, handle);
    if handle != 0 {
        let object = ram::g_player_info(revision).at(i32::from(handle));
        object.tile().write(context, 0);
        let frame = rom::ending_pose_composite_frames(revision)
            .at(0)
            .read(context);
        object.current_frame().write(context, frame);
        object.frames().write(
            context,
            Ptr::new(rom::ending_pose_composite_frames(revision).address()),
        );
        object.tile_count().write(context, 30);
        object.sprite_count().write(context, 3);
        object.x().write(context, 104);
        object.y().write(context, -50);
        object.sprite_ready().write(context, 0);
        let tile = object.tile().read(context);
        object.tile().write(context, tile | 0x8000);
        object.sequence().write(
            context,
            Ptr::new(rom::intro_held_frame_streams(revision).address()),
        );
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        object.frame_phase().write(context, 6);
        call_void(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:85 (`EndingCreateSixPieceComposite`).
/// Atomic: bounded pool/viewport RAM allocation; preserves the existing tile bits.
pub fn ending_create_six_piece_composite(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as u8;
    ram::g_ending_six_piece_composite_entity(revision).write(context, handle);
    if handle != 0 {
        let object = ram::g_player_info(revision).at(i32::from(handle));
        let frame = rom::ending_six_piece_composite_frames(revision)
            .at(0)
            .read(context);
        object.current_frame().write(context, frame);
        object.frames().write(
            context,
            Ptr::new(rom::ending_six_piece_composite_frames(revision).address()),
        );
        object.tile_count().write(context, 69);
        object.sprite_count().write(context, 6);
        object.x().write(context, 104);
        object.y().write(context, -50);
        object.sprite_ready().write(context, 0);
        let tile = object.tile().read(context);
        object.tile().write(context, tile | 0x8000);
        object.sequence().write(
            context,
            Ptr::new(rom::intro_held_frame_streams(revision).address()),
        );
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        object.frame_phase().write(context, 6);
        call_void(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:130 (`EndingCreateRightOffsetComposite`).
/// Atomic: pool/viewport allocation with a ROM-resident animation stream.
pub fn ending_create_right_offset_composite(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as u8;
    ram::g_ending_right_offset_composite_entity(revision).write(context, handle);
    if handle != 0 {
        let object = ram::g_player_info(revision).at(i32::from(handle));
        object.tile().write(context, 0);
        let frame = rom::ending_right_offset_composite_frames(revision)
            .at(0)
            .read(context);
        object.current_frame().write(context, frame);
        object.frames().write(
            context,
            Ptr::new(rom::ending_right_offset_composite_frames(revision).address()),
        );
        object.tile_count().write(context, 43);
        object.sprite_count().write(context, 4);
        object.x().write(context, 104);
        object.y().write(context, -50);
        object.sprite_ready().write(context, 0);
        let tile = object.tile().read(context);
        object.tile().write(context, tile | 0x8000);
        object.sequence().write(
            context,
            Ptr::new(rom::ending_frame_index_streams(revision).at(242).address()),
        );
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        object.frame_phase().write(context, 6);
        call_void(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:175 (`IntroCreateFallingRocketship`).
/// Atomic: bounded RAM allocation and ROM frame/sequence selection.
pub fn intro_create_falling_rocketship(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as u8;
    ram::g_intro_falling_rocketship_entity(revision).write(context, handle);
    if handle != 0 {
        let object = ram::g_player_info(revision).at(i32::from(handle));
        object.tile().write(context, 0);
        let frame = rom::spr_intro_rocketship_falling(revision)
            .at(0)
            .read(context);
        object.current_frame().write(context, frame);
        object.frames().write(
            context,
            Ptr::new(rom::spr_intro_rocketship_falling(revision).address()),
        );
        object.tile_count().write(context, 12);
        object.sprite_count().write(context, 2);
        object.x().write(context, 104);
        object.y().write(context, -50);
        object.sprite_ready().write(context, 0);
        object.sequence().write(
            context,
            Ptr::new(rom::intro_frame_index_lists(revision).at(7).address()),
        );
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        object.frame_phase().write(context, 6);
        call_void(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:219 (`EndingCreateElevatedThreePieceComposite`).
/// Atomic: unsigned word coordinates and bounded pool/viewport RAM allocation.
pub fn ending_create_elevated_three_piece_composite(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as u8;
    ram::g_ending_elevated_three_piece_composite_entity(revision).write(context, handle);
    if handle != 0 {
        let object = ram::g_player_info(revision).at(i32::from(handle));
        object.tile().write(context, 0);
        let frame = rom::ending_elevated_composite_frames(revision)
            .at(0)
            .read(context);
        object.current_frame().write(context, frame);
        object.frames().write(
            context,
            Ptr::new(rom::ending_elevated_composite_frames(revision).address()),
        );
        object.tile().write(context, 0x8000);
        object.tile_count().write(context, 30);
        object.sprite_count().write(context, 3);
        object.x().write(context, arguments[0] as i16);
        object
            .y()
            .write(context, (arguments[1] as u16).wrapping_add(50) as i16);
        object.height().write(context, 50);
        object.sprite_ready().write(context, 0);
        object.sequence().write(
            context,
            Ptr::new(rom::ending_frame_index_streams(revision).at(245).address()),
        );
        object.frame_duration().write(context, 1);
        object.frame().write(context, -1);
        object.frame_phase().write(context, 4);
        call_void(
            context,
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), 0],
        )?;
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:77 (`EndingReleasePoseComposite`).
/// Atomic: conditional RAM-only release, followed by an unconditional slot clear.
pub fn ending_release_pose_composite(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let slot = ram::g_ending_pose_composite_entity(context.revision());
    release_composite(context, slot)
}

/// G/src/cutscenes/composite_entities.c:122 (`EndingReleaseSixPieceComposite`).
/// Atomic: conditional RAM-only release, followed by an unconditional slot clear.
pub fn ending_release_six_piece_composite(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let slot = ram::g_ending_six_piece_composite_entity(context.revision());
    release_composite(context, slot)
}

/// G/src/cutscenes/composite_entities.c:167 (`EndingReleaseRightOffsetComposite`).
/// Atomic: conditional RAM-only release, followed by an unconditional slot clear.
pub fn ending_release_right_offset_composite(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let slot = ram::g_ending_right_offset_composite_entity(context.revision());
    release_composite(context, slot)
}

/// G/src/cutscenes/composite_entities.c:198 (`IntroReleaseFallingRocketship`).
/// Atomic: conditional RAM-only release, followed by an unconditional slot clear.
pub fn intro_release_falling_rocketship(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let slot = ram::g_intro_falling_rocketship_entity(context.revision());
    release_composite(context, slot)
}

/// G/src/cutscenes/composite_entities.c:245 (`EndingReleaseElevatedThreePieceComposite`).
/// Atomic: conditional RAM-only release, followed by an unconditional slot clear.
pub fn ending_release_elevated_three_piece_composite(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let slot = ram::g_ending_elevated_three_piece_composite_entity(context.revision());
    release_composite(context, slot)
}

fn release_composite(context: &mut dyn Context, slot: tje_mem::Address<u8>) -> Result<Option<u32>> {
    if slot.read(context) != 0 {
        let handle = slot.read(context);
        call_void(context, ids::FREE_ENTITY, &[u32::from(handle)])?;
    }
    slot.write(context, 0);
    Ok(None)
}
