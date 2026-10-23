//! Sprite-pool leaves; every index remains an unchecked emulated-memory offset.
use crate::{
    Context, Result, call_at,
    call_frame::{CallSite, FrameLayout, LocalFrame},
    ids,
};
use tje_mem::{Address, ram};

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

/// G/src/sprites/sprite_slots.c:10, AllocateSpriteSlots.
/// G/build/rev02/asm/c_sprite_slots.s:3-146 establishes LINK #-32,
/// D2-D6 saves, both unchecked 12-byte locals, and mutable entry-SP+16.
pub fn allocate_sprite_slots(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(32, 0x7c, 0))?;
    let mut count = arguments[0] as i16;
    let mode = arguments[1] as i16;
    let include_secondary = arguments[2] as i8;
    let slot = frame.local::<i16>(-2);
    let free_count = frame.local::<i8>(-3);
    let secondary_count = frame.local::<i8>(-4);
    let index = frame.local::<i16>(-6);
    let limit = frame.local::<i16>(-8);
    let free_slots = frame.local::<i8>(-20);
    let secondary_slots = frame.local::<i8>(-32);
    free_count.write(context, 0);
    secondary_count.write(context, 0);
    let mut flags = 1i8;
    if include_secondary != 0 {
        flags |= 2;
    }
    if mode == 1 {
        flags |= 4;
    }
    let states = ram::g_viewport_sprite_slot_flags(context.revision());
    limit.write(context, 54);
    index.write(context, 0);
    while index.read(context) < limit.read(context) {
        let current = index.read(context);
        slot.write(
            context,
            if mode == 1 {
                53i16.wrapping_sub(current)
            } else {
                current
            },
        );
        let state = states.at(i32::from(slot.read(context))).read(context);
        if state & 1 == 0 {
            let old = free_count.read(context);
            free_count.write(context, old.wrapping_add(1));
            free_slots
                .offset(i32::from(old))
                .write(context, slot.read(context) as i8);
        } else if include_secondary != 0
            && state & 2 == 0
            && i32::from(secondary_count.read(context))
                < i32::from(count) - i32::from(free_count.read(context))
        {
            let old = secondary_count.read(context);
            secondary_count.write(context, old.wrapping_add(1));
            secondary_slots
                .offset(i32::from(old))
                .write(context, slot.read(context) as i8);
        }
        if i16::from(free_count.read(context)) == count {
            break;
        }
        index.write(context, index.read(context).wrapping_add(1));
    }
    let status = if i32::from(count)
        > i32::from(free_count.read(context)) + i32::from(secondary_count.read(context))
    {
        1
    } else {
        index.write(context, 0);
        while index.read(context) < i16::from(free_count.read(context)) {
            let selected = free_slots
                .offset(i32::from(index.read(context)))
                .read(context);
            states.at(i32::from(selected)).write(context, flags);
            let output = frame.argument_slot(3).read(context);
            frame
                .argument_slot(3)
                .write(context, output.wrapping_add(1));
            Address::<i8>::new(output).write(context, selected.wrapping_add(25));
            index.write(context, index.read(context).wrapping_add(1));
        }
        if i16::from(free_count.read(context)) < count {
            count = count.wrapping_sub(i16::from(free_count.read(context)));
            index.write(context, 0);
            while index.read(context) < count {
                let selected = secondary_slots
                    .offset(i32::from(index.read(context)))
                    .read(context);
                states.at(i32::from(selected)).write(context, flags);
                let output = frame.argument_slot(3).read(context);
                frame
                    .argument_slot(3)
                    .write(context, output.wrapping_add(1));
                Address::<i8>::new(output).write(context, selected.wrapping_add(25));
                index.write(context, index.read(context).wrapping_add(1));
            }
        }
        0
    };
    frame.leave(context)?;
    Ok(Some(status))
}

/// G/src/sprites/sprite_slots.c:66, AllocateSpriteSlot.
/// LINK #-4 leaves the output byte at A6-1 uninitialized until the selector writes it.
/// The matching JSR returns at REV02 D5A6 / REV00 D586 (function offset 0x20).
pub fn allocate_sprite_slot(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0, 0))?;
    let output = frame.local::<i8>(-1);
    let site = CallSite::relative(ids::ALLOCATE_SPRITE_SLOT, context.revision(), 0x20);
    let status = call_at(
        context,
        ids::ALLOCATE_SPRITE_SLOTS,
        &[
            1,
            arguments[0] as i16 as i32 as u32,
            arguments[1],
            output.address(),
        ],
        site,
    )?;
    let result = if status != 0 { 0 } else { output.read(context) };
    frame.leave(context)?;
    Ok(Some(result as i32 as u32))
}
