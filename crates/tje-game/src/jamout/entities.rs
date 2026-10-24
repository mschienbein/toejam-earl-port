use crate::call_frame::{CallSite, FrameLayout, LocalFrame};
use crate::{Context, Result, call, call_at, ids};
use tje_mem::{Ptr, ram, rom};

/// G/src/jamout/jamout.c:509 (`JamOutReleaseCharacterCopies`, REV 02 0x03B6D2).
/// Keep the redundant original handle reload before FreeEntity; the arrays are
/// not cleared, and each ToeJam release precedes Earl's test for the same slot.
pub fn release_character_copies(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for slot in 0..9 {
        let toejam = ram::g_jam_toejam_entities(revision).at(slot);
        if toejam.read(context) != 0 {
            let _object_number = toejam.read(context);
            let handle = i32::from(toejam.read(context)) as u32;
            call(context, ids::FREE_ENTITY, &[handle])?;
        }
        let earl = ram::g_jam_earl_entities(revision).at(slot);
        if earl.read(context) != 0 {
            let _object_number = earl.read(context);
            let handle = i32::from(earl.read(context)) as u32;
            call(context, ids::FREE_ENTITY, &[handle])?;
        }
    }
    Ok(None)
}

pub(super) fn set_data_word(context: &mut dyn Context, index: usize, value: i16) {
    let previous = context.data_register(index);
    context.set_data_register(index, (previous & 0xffff0000) | u32::from(value as u16));
}

/// G/src/jamout/jamout.c:442 (`JamOutCreateCharacterCopies`, REV 02 0x03B564).
/// Materialize LINK A6,#-4 and MOVEM D2-D6/A2-A4. The shared lead_tile word is
/// really -2(A6); parent_number is D3.B. Neither is zero-initialized on a failed
/// lead allocation, and the second character can inherit the first one's state.
pub fn create_character_copies(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0x7c, 0x1c))?;
    let result: Result<Option<u32>> = (|| {
        context.set_address_register(3, ram::g_jam_toejam_entities(revision).address());
        context.set_address_register(4, ram::g_jam_earl_entities(revision).address());
        for earl in [false, true] {
            let (handles, tile_count, sprite_count, frames, allocate_return, viewport_return) =
                if earl {
                    (
                        ram::g_jam_earl_entities(revision),
                        19,
                        2,
                        rom::spr_jamout_earl(revision).address(),
                        0xcc,
                        0x13a,
                    )
                } else {
                    (
                        ram::g_jam_toejam_entities(revision),
                        16,
                        1,
                        rom::spr_jamout_toejam(revision).address(),
                        0x24,
                        0x92,
                    )
                };
            context.set_data_register(5, tile_count);
            context.set_data_register(6, sprite_count);
            set_data_word(context, 4, 0);
            for slot in 0..9 {
                set_data_word(context, 2, (slot * 2) as i16);
                let allocated = call_at(
                    context,
                    ids::ALLOCATE_ENTITY,
                    &[],
                    CallSite::relative(
                        ids::JAM_OUT_CREATE_CHARACTER_COPIES,
                        revision,
                        allocate_return,
                    ),
                )?;
                handles.at(slot).write(context, allocated as i16);
                if handles.at(slot).read(context) != 0 {
                    let object = super::object_from_handle(context, handles.at(slot).read(context));
                    context.set_address_register(2, object.address());
                    object.tile().write(context, 0x8000);
                    object.tile_count().write(context, tile_count as u8);
                    object.sprite_count().write(context, sprite_count as u8);
                    object.frames().write(context, Ptr::new(frames));
                    object.x().write(context, 160);
                    object.y().write(context, -50);
                    object.sequence().write(
                        context,
                        Ptr::new(rom::jam_lead_idle_loop_sequence(revision).address()),
                    );
                    object.frame_phase().write(context, 6);
                    object.frame_duration().write(context, 1);
                    object.frame().write(context, -1);
                    if slot != 0 {
                        let flags = object.behavior_flags().read(context) | 4;
                        object.behavior_flags().write(context, flags);
                    }
                    object.sprite_ready().write(context, 0);
                    // D4.W is the slot number here. The callee's original shared
                    // tile failure path consumes entry D4.W as its unassigned tile.
                    call_at(
                        context,
                        ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
                        &[object.address(), 0],
                        CallSite::relative(
                            ids::JAM_OUT_CREATE_CHARACTER_COPIES,
                            revision,
                            viewport_return,
                        ),
                    )?;
                    if slot == 0 {
                        let tile = object.tile().read(context);
                        frame.local::<u16>(-2).write(context, tile);
                        let parent = handles.at(slot).read(context) as u8;
                        let previous = context.data_register(3);
                        context.set_data_register(3, (previous & 0xffffff00) | u32::from(parent));
                    } else {
                        let tile = frame.local::<u16>(-2).read(context);
                        object.tile().write(context, tile);
                        let parent = context.data_register(3) as i8;
                        object.owner().write(context, parent);
                    }
                }
                set_data_word(context, 4, (slot + 1) as i16);
            }
        }
        Ok(None)
    })();
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let cleanup = frame.leave(context);
    match (result, cleanup) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}
