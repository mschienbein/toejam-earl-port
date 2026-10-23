//! Sprite-pool leaves; every index remains an unchecked emulated-memory offset.
use crate::{Context, Result};
use tje_mem::ram;

/// G/src/sprites/sprite_slots.c:79, FreeSpriteSlots. The subtraction narrows to char.
pub fn free_sprite_slots(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let flags = ram::g_viewport_sprite_slot_flags(context.revision());
    let count = arguments[0] as i16;
    for index in 0..count {
        let handle = context.read_i8(arguments[1].wrapping_add(index as u32));
        let slot = handle.wrapping_sub(25);
        flags.at(i32::from(slot)).write(context, 0);
    }
    Ok(None)
}

/// G/src/sprites/sprite_slots.c:93, FreeSpriteSlot. Preserve byte-width wraparound.
pub fn free_sprite_slot(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let slot = (arguments[0] as i8).wrapping_sub(25);
    ram::g_viewport_sprite_slot_flags(context.revision())
        .at(i32::from(slot))
        .write(context, 0);
    Ok(None)
}

/// G/src/sprites/sprite_slots.c:101, IsSpriteSlotLocked. Only bit one is tested.
pub fn is_sprite_slot_locked(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let slot = (arguments[0] as i32).wrapping_sub(25);
    let state = ram::g_viewport_sprite_slot_flags(context.revision())
        .at(slot)
        .read(context);
    Ok(Some(u32::from(state & 2 != 0)))
}

/// G/src/sprites/sprite_slots.c:111, AllocateSpriteTileRun. Signed division truncates toward zero.
pub fn allocate_sprite_tile_run(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let groups = ((i32::from(arguments[0] as i16) - 1) / 8 + 1) as i16;
    let flags = ram::g_sprite_tile_group_allocated(context.revision());
    let mut run = 0i16;
    for index in 0..80i16 {
        if flags.at(i32::from(index)).read(context) == 0 {
            run = run.wrapping_add(1);
            if run == groups {
                let first = index.wrapping_add(1).wrapping_sub(run);
                let tile = first.wrapping_shl(3).wrapping_add(640);
                for mark in 0..run {
                    flags
                        .at(i32::from(first.wrapping_add(mark)))
                        .write(context, 1);
                }
                return Ok(Some(i32::from(tile) as u32));
            }
        } else {
            run = 0;
        }
    }
    Ok(Some(0))
}

/// G/src/sprites/sprite_slots.c:135, FreeSpriteTileRun. Narrow the base subtraction before division.
pub fn free_sprite_tile_run(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let groups = ((i32::from(arguments[1] as i16) - 1) / 8 + 1) as i16;
    let first = (arguments[0] as i16).wrapping_sub(640) / 8;
    let flags = ram::g_sprite_tile_group_allocated(context.revision());
    for index in 0..groups {
        flags
            .at(i32::from(first) + i32::from(index))
            .write(context, 0);
    }
    Ok(None)
}

/// G/src/sprites/sprite_slots.c:228, ResetEntityRuntimeState. Exactly 47 words, not a whole record.
pub fn reset_entity_runtime_state(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let mut words = arguments[0];
    for _ in 0..47 {
        context.write_u16(words, 0);
        words = words.wrapping_add(2);
    }
    Ok(None)
}
