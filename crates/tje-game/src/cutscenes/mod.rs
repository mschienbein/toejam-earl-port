//! Cutscenes translation ownership; see docs/PORTING.md.

use crate::{Context, Result, ids};
use tje_mem::{Ptr, ram, rom};

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::CUTSCENE_APPEND_REMAPPED_FONT_TILES,
        function: cutscene_append_remapped_font_tiles,
    },
    crate::Translation {
        id: ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
        function: intro_set_large_player_part_sequence,
    },
    crate::Translation {
        id: ids::INTRO_SET_DRIVING_PART_SEQUENCE,
        function: intro_set_driving_part_sequence,
    },
    crate::Translation {
        id: ids::ENDING_GET_FIRST_PLAYER_ANIMATION_FRAME,
        function: ending_get_first_player_animation_frame,
    },
    crate::Translation {
        id: ids::INTRO_REVEAL_FRAME_ENTITIES,
        function: intro_reveal_frame_entities,
    },
    crate::Translation {
        id: ids::CUTSCENE_MOVE_ROCKETSHIP_PAIR_LEFT,
        function: cutscene_move_rocketship_pair_left,
    },
    crate::Translation {
        id: ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
        function: cutscene_position_rocketship_pair,
    },
    crate::Translation {
        id: ids::ENDING_SET_POSE_COMPOSITE,
        function: ending_set_pose_composite,
    },
    crate::Translation {
        id: ids::ENDING_POSITION_SIX_PIECE_COMPOSITE,
        function: ending_position_six_piece_composite,
    },
    crate::Translation {
        id: ids::ENDING_POSITION_RIGHT_OFFSET_COMPOSITE,
        function: ending_position_right_offset_composite,
    },
    crate::Translation {
        id: ids::INTRO_POSITION_FALLING_ROCKETSHIP,
        function: intro_position_falling_rocketship,
    },
    crate::Translation {
        id: ids::ENDING_RESTART_ELEVATED_COMPOSITE_SEQUENCE,
        function: ending_restart_elevated_composite_sequence,
    },
];

/// All entries below are call-free RAM/ROM operations. Their only loops have
/// local finite counters; no return path waits for hardware or interrupts.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    ids::CUTSCENE_APPEND_REMAPPED_FONT_TILES,
    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
    ids::ENDING_GET_FIRST_PLAYER_ANIMATION_FRAME,
    ids::INTRO_REVEAL_FRAME_ENTITIES,
    ids::CUTSCENE_MOVE_ROCKETSHIP_PAIR_LEFT,
    ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
    ids::ENDING_SET_POSE_COMPOSITE,
    ids::ENDING_POSITION_SIX_PIECE_COMPOSITE,
    ids::ENDING_POSITION_RIGHT_OFFSET_COMPOSITE,
    ids::INTRO_POSITION_FALLING_ROCKETSHIP,
    ids::ENDING_RESTART_ELEVATED_COMPOSITE_SEQUENCE,
];

/// G/src/cutscenes/cutscene_text.c:26 (`CutsceneAppendRemappedFontTiles`).
/// The signed word byte count can wrap independently of the final offset update.
pub fn cutscene_append_remapped_font_tiles(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut source = arguments[0];
    let tile_count = arguments[1] as i16;
    let offset = ram::g_sprite_tile_buffer_offset(revision);
    let mut destination = ram::g_sprite_tile_upload_buffer(revision)
        .at(i32::from(offset.read(context)))
        .address();
    let byte_count = (i32::from(tile_count) << 5) as i16;
    let mut remaining = byte_count;
    while remaining > 0 {
        let mut pixels = match context.read_u8(source) & 15 {
            11 => 1,
            10 | 9 => 2,
            8 => 15,
            _ => 0,
        };
        // The matching code reads the source again before writing the destination.
        pixels |= match context.read_u8(source) & 0xf0 {
            0xb0 => 0x10,
            0xa0 | 0x90 => 0x20,
            0x80 => 0xf0,
            _ => 0,
        };
        context.write_u8(destination, pixels);
        destination = destination.wrapping_add(1);
        source = source.wrapping_add(1);
        remaining = remaining.wrapping_sub(1);
    }
    let current_offset = offset.read(context);
    offset.write(context, current_offset.wrapping_add(byte_count as u16));
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:85 (`IntroSetLargePlayerPartSequence`).
/// Selector indexing is deliberately unchecked; period uses the low signed byte.
pub fn intro_set_large_player_part_sequence(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = arguments[0] as i16;
    let handle = ram::g_intro_large_player_part_entities(revision)
        .at(i32::from(index))
        .read(context);
    let entity = ram::g_player_info(revision).at(i32::from(handle));
    entity.sequence().write(context, Ptr::new(arguments[1]));
    entity.frame_phase().write(context, arguments[2] as i8);
    entity.frame_duration().write(context, 1);
    entity.frame().write(context, -1);
    Ok(None)
}

/// G/src/cutscenes/large_player_parts.c:212 (`IntroSetDrivingPartSequence`).
pub fn intro_set_driving_part_sequence(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = arguments[0] as i16;
    let handle = ram::g_intro_driving_part_entities(revision)
        .at(i32::from(index))
        .read(context);
    let entity = ram::g_player_info(revision).at(i32::from(handle));
    entity.sequence().write(context, Ptr::new(arguments[1]));
    entity.frame_phase().write(context, arguments[2] as i8);
    entity.frame_duration().write(context, 1);
    entity.frame().write(context, -1);
    Ok(None)
}

/// G/src/cutscenes/ending_player_release.c:22 (`EndingGetFirstPlayerAnimationFrame`).
/// Preserve the original first-handle lookup even when only player two exists.
pub fn ending_get_first_player_animation_frame(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let first = ram::g_ending_player1_entity(revision);
    if first.read(context) == 0 && ram::g_ending_player2_entity(revision).read(context) == 0 {
        return Ok(Some(0));
    }
    let entity = ram::g_player_info(revision).at(i32::from(first.read(context)));
    Ok(Some(i32::from(entity.frame().read(context)) as u32))
}

/// G/src/cutscenes/intro_frame_reveal.c:7 (`IntroRevealFrameEntities`).
/// Zero selectors still address entity zero, as in the original ten-slot loop.
pub fn intro_reveal_frame_entities(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..10 {
        let handle = ram::g_intro_component_entities(revision)
            .at(index)
            .read(context);
        let entity = ram::g_player_info(revision).at(i32::from(handle));
        let flags = entity.behavior_flags().read(context);
        entity.behavior_flags().write(context, flags & !0x40);
    }
    Ok(None)
}

/// G/src/cutscenes/rocketship_move.c:7 (`CutsceneMoveRocketshipPairLeft`).
/// The ship uses distance, the exhaust always one, and each word wraps before testing.
pub fn cutscene_move_rocketship_pair_left(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let handle = ram::g_cutscene_rocketship_entity(revision).read(context);
    let entity = ram::g_player_info(revision).at(i32::from(handle));
    let x = entity.x().read(context).wrapping_sub(arguments[0] as i16);
    entity.x().write(context, x);
    if entity.x().read(context) < -60 {
        entity.x().write(context, 320);
    }
    let handle = ram::g_cutscene_rocketship_exhaust_entity(revision).read(context);
    let entity = ram::g_player_info(revision).at(i32::from(handle));
    let x = entity.x().read(context).wrapping_sub(1);
    entity.x().write(context, x);
    if entity.x().read(context) < -60 {
        entity.x().write(context, 320);
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:19 (`CutscenePositionRocketshipPair`).
/// Complete the ship stores before reading the exhaust selector (which may alias).
pub fn cutscene_position_rocketship_pair(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let handle = ram::g_cutscene_rocketship_entity(revision).read(context);
    let entity = ram::g_player_info(revision).at(i32::from(handle));
    entity.x().write(context, arguments[0] as i16);
    entity.y().write(context, arguments[1] as i16);
    let handle = ram::g_cutscene_rocketship_exhaust_entity(revision).read(context);
    let entity = ram::g_player_info(revision).at(i32::from(handle));
    entity.x().write(context, arguments[0] as i16);
    entity.y().write(context, arguments[1] as i16);
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:58 (`EndingSetPoseComposite`).
/// MCC68K doubles the frame index as a word before signed address addition.
pub fn ending_set_pose_composite(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_ending_pose_composite_entity(revision);
    if selector.read(context) != 0 {
        let entity = ram::g_player_info(revision).at(i32::from(selector.read(context)));
        entity
            .x()
            .write(context, (i32::from(arguments[1] as i16) - 5) as i16);
        entity
            .y()
            .write(context, (i32::from(arguments[2] as i16) - 5) as i16);
        let tile = entity.tile().read(context);
        entity.tile().write(context, tile | 0x8000);
        let flags = entity.draw_flags().read(context);
        entity.draw_flags().write(context, flags | 0x80);
        let frame_offset = (arguments[0] as i16).wrapping_mul(2);
        let sequence = rom::intro_held_frame_streams(revision)
            .byte_offset(i32::from(frame_offset))
            .address();
        entity.sequence().write(context, Ptr::new(sequence));
        entity.frame_duration().write(context, 1);
        entity.frame().write(context, -1);
        entity.frame_phase().write(context, 6);
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:108 (`EndingPositionSixPieceComposite`).
pub fn ending_position_six_piece_composite(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_ending_six_piece_composite_entity(revision);
    if selector.read(context) != 0 {
        let entity = ram::g_player_info(revision).at(i32::from(selector.read(context)));
        entity.x().write(context, arguments[0] as i16);
        entity
            .y()
            .write(context, (i32::from(arguments[1] as i16) + 100) as i16);
        entity.height().write(context, 100);
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:154 (`EndingPositionRightOffsetComposite`).
pub fn ending_position_right_offset_composite(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_ending_right_offset_composite_entity(revision);
    if selector.read(context) != 0 {
        let entity = ram::g_player_info(revision).at(i32::from(selector.read(context)));
        entity
            .x()
            .write(context, (i32::from(arguments[0] as i16) + 50) as i16);
        entity.y().write(context, arguments[1] as i16);
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:206 (`IntroPositionFallingRocketship`).
pub fn intro_position_falling_rocketship(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_intro_falling_rocketship_entity(revision);
    if selector.read(context) != 0 {
        let entity = ram::g_player_info(revision).at(i32::from(selector.read(context)));
        entity.x().write(context, arguments[0] as i16);
        entity.y().write(context, arguments[1] as i16);
    }
    Ok(None)
}

/// G/src/cutscenes/composite_entities.c:253 (`EndingRestartElevatedCompositeSequence`).
pub fn ending_restart_elevated_composite_sequence(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let selector = ram::g_ending_elevated_three_piece_composite_entity(revision);
    if selector.read(context) != 0 {
        let entity = ram::g_player_info(revision).at(i32::from(selector.read(context)));
        entity.sprite_ready().write(context, 0);
        let sequence = rom::ending_frame_index_streams(revision).at(249).address();
        entity.sequence().write(context, Ptr::new(sequence));
        entity.frame_duration().write(context, 1);
        entity.frame().write(context, -1);
        entity.frame_phase().write(context, 4);
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
