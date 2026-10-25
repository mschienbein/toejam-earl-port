use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, call_at, call_at_void, ids};
use tje_mem::{Address, LevelItem, Ptr, ram, rom};

/// G/src/presents/item_updates.c:40 (`EntityCreateLevelItem`).
/// Atomic: the original single saved A2 is materialized. On allocation failure
/// the three pushed placement arguments intentionally expose saved entry A2 as
/// the fourth argument; it is neither zero-filled nor replaced with a guessed flag.
/// JSR return offsets are 0x0c and 0x112 in both matching item_updates objects.
pub fn entity_create_level_item(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0, 0x04))?;
    let result = create_body(context, arguments);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    frame.leave(context)?;
    result
}

fn create_body(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::ENTITY_CREATE_LEVEL_ITEM;
    let item = Address::<LevelItem>::new(arguments[0]);
    context.set_address_register(2, item.address());
    let allocated = call_at(
        context,
        ids::ALLOCATE_ENTITY,
        &[],
        CallSite::relative(caller, revision, 0x0c),
    )? as i8;
    item.object().write(context, allocated);
    if allocated != 0 {
        // Reload the stored handle, as the compiler does (including ROM writes).
        let object = ram::g_player_info(revision).at(i32::from(item.object().read(context)));
        if item.kind().read(context) < 64 {
            let appearance = ram::g_present_info(revision)
                .at(i32::from(item.kind().read(context)))
                .appearance()
                .read(context);
            object.frames().write(
                context,
                Ptr::new(
                    rom::present_sprite_table(revision)
                        .at(i32::from(appearance))
                        .address(),
                ),
            );
        } else if item.kind().read(context) >= 81 {
            object.frames().write(
                context,
                Ptr::new(
                    rom::large_ground_item_frames(revision)
                        .at(i32::from(item.kind().read(context)) - 81)
                        .address(),
                ),
            );
        } else {
            object.frames().write(
                context,
                Ptr::new(
                    rom::spr_ground_item(revision)
                        .at(i32::from(item.kind().read(context)) - 64)
                        .address(),
                ),
            );
            object
                .draw_flags()
                .write(context, object.draw_flags().read(context) | 0x80);
        }
        object.sequence().write(
            context,
            Ptr::new(rom::single_frame_hold_animation(revision).address()),
        );
        object.frame_phase().write(context, 0);
        object.frame_duration().write(context, 1);
        object.x().write(context, item.x().read(context));
        object.y().write(context, item.y().read(context));
        object.level().write(context, item.level().read(context));
        if item.kind().read(context) < 81 {
            object.width().write(context, 24);
            object.height_pixels().write(context, 16);
            object.sprite_count().write(context, 1);
            object.tile_count().write(context, 12);
        } else {
            object.width().write(context, 32);
            object.height_pixels().write(context, 64);
            object.sprite_count().write(context, 2);
            object.tile_count().write(context, 32);
            object
                .behavior_flags()
                .write(context, object.behavior_flags().read(context) | 1);
        }
        object.tile().write(context, 0x8000);
        object
            .draw_flags()
            .write(context, object.draw_flags().read(context) | 8);
    } else if item.kind().read(context) < 81 {
        let values = [
            item.x().address(),
            item.y().address(),
            item.level().read(context) as u32,
        ];
        let outgoing = OutgoingArguments::push(context, &values)?;
        let dispatcher = context.dispatcher();
        outgoing.call_void(
            context,
            dispatcher,
            ids::MAP_FIND_ITEM_PLACEMENT_POSITION,
            CallSite::relative(caller, revision, 0x112),
        )?;
        outgoing.release(context)?;
    }
    Ok(None)
}

/// G/src/presents/item_updates.c:84 (`EntityUpdateLevelItemVisibility`).
/// Atomic: LINK -4; MOVEM D2-D7/A2. The index remains in the actual A6-2
/// word and A2 is the current item, including the constructor's omitted-argument
/// path. Call offsets derive from object start 0x11a (returns 0x224/0x23e).
pub fn entity_update_level_item_visibility(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0xfc, 0x04))?;
    let result = visibility_body(context, arguments, &frame);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    frame.leave(context)?;
    result
}

fn visibility_body(
    context: &mut dyn Context,
    arguments: &[u32],
    frame: &LocalFrame,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::ENTITY_UPDATE_LEVEL_ITEM_VISIBILITY;
    let items = Address::<LevelItem>::new(arguments[0]);
    let count = arguments[1] as i16;
    let index = frame.local::<i16>(-2);
    index.write(context, 0);
    while index.read(context) < count {
        let item = items.offset(i32::from(index.read(context)));
        context.set_address_register(2, item.address());
        let first_x = item
            .x()
            .read(context)
            .wrapping_sub(ram::g_camera1_pending_x(revision).read(context));
        let first_y = item
            .y()
            .read(context)
            .wrapping_sub(ram::g_camera1_pending_y(revision).read(context));
        let second_x = item
            .x()
            .read(context)
            .wrapping_sub(ram::g_camera2_pending_x(revision).read(context));
        let second_y = item
            .y()
            .read(context)
            .wrapping_sub(ram::g_camera2_pending_y(revision).read(context));
        if item.kind().read(context) != -1
            && ((item.level().read(context) == ram::g_player1_map_level(revision).read(context)
                && i32::from(first_x) + 20 > 0
                && first_x < 340
                && i32::from(first_y) + 20 > 0
                && ((item.kind().read(context) < 81 && first_y < 244)
                    || (item.kind().read(context) >= 81 && first_y < 288)))
                || (item.level().read(context) == ram::g_player2_map_level(revision).read(context)
                    && ram::g_split_screen_enabled(revision).read(context) != 0
                    && i32::from(second_x) + 20 > 0
                    && second_x < 340
                    && i32::from(second_y) + 20 > 0
                    && ((item.kind().read(context) < 81 && second_y < 132)
                        || (item.kind().read(context) >= 81 && second_y < 176))))
        {
            if item.object().read(context) == 0 {
                call_at_void(
                    context,
                    ids::ENTITY_CREATE_LEVEL_ITEM,
                    &[item.address()],
                    CallSite::relative(caller, revision, 0x10a),
                )?;
            }
        } else {
            let number = item.object().read(context);
            if number != 0 {
                call_at_void(
                    context,
                    ids::FREE_ENTITY,
                    &[number as u32],
                    CallSite::relative(caller, revision, 0x124),
                )?;
                item.object().write(context, 0);
            }
        }
        index.write(context, index.read(context).wrapping_add(1));
    }
    Ok(None)
}
