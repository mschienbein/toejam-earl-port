use crate::{Context, Result, call, ids};
use tje_mem::{Ptr, ram, rom};

/// G/src/cutscenes/ending_player_entities.c:18 (`EndingCreateActivePlayerEntities`).
/// Atomic: snapshots lives, then performs bounded RAM-only entity allocations.
pub fn ending_create_active_player_entities(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let second_active = ram::g_player2_lives(revision).read(context) >= 0;
    let first_active = ram::g_player_lives(revision).at(0).read(context) >= 0;
    if first_active {
        let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as u8;
        ram::g_ending_player1_entity(revision).write(context, handle);
        if handle != 0 {
            let object = ram::g_player_info(revision).at(i32::from(handle));
            let frames = rom::ending_player1_frames(revision).address();
            object.sprite_count().write(context, 3);
            object.tile_count().write(context, 48);
            object.tile().write(context, 0x8000);
            object.frames().write(context, Ptr::new(frames));
            let sequence = if second_active {
                rom::ending_player1_paired_animation(revision).address()
            } else {
                rom::ending_player1_solo_animation(revision).address()
            };
            object.sequence().write(context, Ptr::new(sequence));
            object.x().write(context, arguments[0] as i16);
            object
                .y()
                .write(context, (arguments[1] as u16).wrapping_add(100) as i16);
            object.height().write(context, 100);
            object.sprite_ready().write(context, 0);
            object.frame_duration().write(context, 1);
            object.frame().write(context, -1);
            object.frame_phase().write(context, 6);
            // Matching caller retains second_active in D4.W at both allocations.
            super::allocate_viewport_with_d4(context, object.address(), u16::from(second_active))?;
        }
    }
    if second_active {
        let handle = call(context, ids::ALLOCATE_ENTITY, &[])? as u8;
        ram::g_ending_player2_entity(revision).write(context, handle);
        if handle != 0 {
            let object = ram::g_player_info(revision).at(i32::from(handle));
            let frames = rom::ending_player2_frames(revision).address();
            object.sprite_count().write(context, 3);
            object.tile_count().write(context, 48);
            object.tile().write(context, 0x8000);
            object.frames().write(context, Ptr::new(frames));
            let sequence = if first_active {
                rom::ending_player2_paired_animation(revision).address()
            } else {
                rom::ending_frame_index_streams(revision).address()
            };
            object.sequence().write(context, Ptr::new(sequence));
            object.x().write(context, arguments[0] as i16);
            object
                .y()
                .write(context, (arguments[1] as u16).wrapping_add(100) as i16);
            object.height().write(context, 100);
            object.sprite_ready().write(context, 0);
            object.frame_duration().write(context, 1);
            object.frame().write(context, -1);
            object.frame_phase().write(context, 6);
            super::allocate_viewport_with_d4(context, object.address(), u16::from(second_active))?;
        }
    }
    Ok(None)
}
