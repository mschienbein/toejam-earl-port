//! Main-thread sprite preparation; waits retain real accepted-interrupt progress.
use crate::{
    Context, Result, call_at,
    call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments},
    execution::WaitRequest,
    ids,
};
use tje_mem::{Address, ObjectState, ram};

/// G/src/sprites/entity_sprite_rendering.c:526; matching c_entity_sprite_rendering.s:1205-1208.
/// TST.B and SUBQ.B are separate reads: IRQ work may change the byte between them.
/// The branch consumes the subtraction's Z, not the value from the earlier guard.
pub(super) fn tick_frame_duration(context: &mut dyn Context, entity: Address<ObjectState>) -> bool {
    if entity.frame_duration().read(context) == 0 {
        return false;
    }
    let operand = entity.frame_duration().read(context);
    let decremented = operand.wrapping_sub(1);
    entity.frame_duration().write(context, decremented);
    decremented == 0
}

/// G/src/sprites/entity_sprite_rendering.c:503, UpdateAndQueueEntitySprites.
/// The exact pending-byte guard is rechecked after each real interrupt. While the
/// main thread is suspended, only accepted original IRQ work can clear this byte.
/// This is TimeAware, not an atomic busy loop or a fabricated ready condition.
pub fn update_and_queue_entity_sprites(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    // G c_entity_sprite_rendering.s:1149: save D2-D4/A2-A4 before any wait.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x1c, 0x1c))?;
    let result = update_queue_inner(context);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let leave = frame.leave(context);
    match (result, leave) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

fn update_queue_inner(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let dispatcher = context.dispatcher();
    let pending = ram::g_sprite_graphics_upload_pending(revision);
    let mut tile_total = 0u16;
    let mut d2 = context.data_register(2);
    let d3_high = context.data_register(3) & 0xffff_0000;
    let mut last_frame = context.address_register(3);
    context.set_address_register(4, ram::g_player_inventory_open(revision).address());
    call_at(
        context,
        ids::INVENTORY_SET_CAMERA_SUSPENSION,
        &[1],
        CallSite::relative(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, revision, 0x16),
    )?;
    while pending.read(context) != 0 {
        context.wait_until(WaitRequest::interrupt())?;
    }
    let first = if ram::g_split_screen_enabled(revision).read(context) == 0
        && (ram::g_player_inventory_open(revision).at(0).read(context) != 0
            || ram::g_player_inventory_open(revision).at(1).read(context) != 0)
    {
        26
    } else {
        0
    };
    let d4_high = if first == 26 {
        0
    } else {
        context.data_register(4) & 0xffff_0000
    };
    for number in first..40 {
        let entity = ram::g_player_info(revision).at(number);
        if entity.draw_flags().read(context) & 1 == 0
            || entity.behavior_flags().read(context) & 4 != 0
            || call_at(
                context,
                ids::IS_SPEECH_BUBBLE_ENTITY,
                &[number as u32],
                CallSite::relative(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, revision, 0x6a),
            )? != 0
            || entity.frame_duration().read(context) == 0
                && entity.sprite_ready().read(context) != 0
        {
            continue;
        }
        if tile_total != 0
            && u32::from(entity.tile_count().read(context)) + u32::from(tile_total) > 112
        {
            continue;
        }
        let next = if tick_frame_duration(context, entity) {
            entity
                .frame_duration()
                .write(context, entity.frame_phase().read(context));
            let mut next = entity.frame().read(context).wrapping_add(1);
            d2 = (d2 & !0xff) | u32::from(next as u8);
            let entry = entity
                .sequence()
                .read(context)
                .target()
                .offset(i32::from(next))
                .read(context);
            if entry == -1 {
                next = 0;
                d2 &= !0xff;
            } else if entry == -2 {
                // G CD56/CD36 CLR.B performs a discarded byte read before zero.
                context.discard_u8(entity.frame_duration().address())?;
                entity.frame_duration().write(context, 0);
                continue;
            }
            Some(next)
        } else if entity.sprite_ready().read(context) == 0 {
            let current = entity.frame().read(context);
            let next = if current == -1 {
                current.wrapping_add(1)
            } else {
                current
            };
            d2 = (d2 & !0xff) | u32::from(next as u8);
            Some(next)
        } else {
            None
        };
        if let Some(next) = next {
            entity.frame().write(context, next);
            let entry = entity
                .sequence()
                .read(context)
                .target()
                .offset(i32::from(next))
                .read(context);
            if entry as u8 & 0x80 != 0 {
                entity
                    .tile()
                    .write(context, entity.tile().read(context) | 0x0800);
            } else {
                entity
                    .tile()
                    .write(context, entity.tile().read(context) & !0x0800);
            }
            let frame_number = entry as u8 & 0x7f;
            let frame_pointer = entity
                .frames()
                .read(context)
                .target()
                .offset(i32::from(frame_number))
                .read(context);
            last_frame = frame_pointer.address();
            let frame = frame_pointer.target();
            let mut reuse_tiles = false;
            let mut already_uploaded = false;
            if frame.reuse_tiles().read(context) != 0 && entity.sprite_ready().read(context) != 0 {
                reuse_tiles = true;
                already_uploaded = true;
            }
            d2 = if already_uploaded {
                1
            } else {
                d2 & 0xffff_0000
            };
            entity.current_frame().write(context, frame_pointer);
            if entity.draw_flags().read(context) & 2 != 0 {
                call_at(
                    context,
                    ids::PREPARE_ENTITY_SPRITE_FRAME,
                    &[entity.address(), frame.address(), u32::from(reuse_tiles), 0],
                    CallSite::relative(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, revision, 0x148),
                )?;
                reuse_tiles = true;
                entity.sprite_ready().write(context, 1);
            }
            if entity.draw_flags().read(context) & 4 != 0 {
                call_at(
                    context,
                    ids::PREPARE_ENTITY_SPRITE_FRAME,
                    &[entity.address(), frame.address(), u32::from(reuse_tiles), 1],
                    CallSite::relative(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, revision, 0x16e),
                )?;
                reuse_tiles = true;
                entity.sprite_ready().write(context, 1);
            }
            if reuse_tiles && !already_uploaded {
                tile_total = tile_total.wrapping_add(u16::from(entity.tile_count().read(context)));
            }
        }
    }
    if ram::g_shared_tile_sprite_copies_enabled(revision).read(context) != 0 {
        call_at(
            context,
            ids::JAM_OUT_SYNCHRONIZE_ENTITY_COPIES,
            &[],
            CallSite::relative(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, revision, 0x1c0),
        )?;
    }
    call_at(
        context,
        ids::SORT_ENTITIES_FOR_SPRITE_DRAW,
        &[],
        CallSite::relative(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, revision, 0x1c4),
    )?;
    let head = ram::g_entity_draw_list_head(revision)
        .read(context)
        .address();
    let arguments = OutgoingArguments::push(context, &[head])?;
    arguments.call(
        context,
        dispatcher,
        ids::POSITION_ENTITY_SPRITES,
        CallSite::relative(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, revision, 0x1ce),
    )?;
    // G CE50/CE52: test player0 before releasing the world-list argument.
    let first_inventory_open = ram::g_player_inventory_open(revision).at(0).read(context) != 0;
    arguments.release(context)?;
    if first_inventory_open || ram::g_player_inventory_open(revision).at(1).read(context) != 0 {
        let head = ram::g_inventory_draw_list_head(revision)
            .read(context)
            .address();
        let inventory = OutgoingArguments::push(context, &[head])?;
        inventory.call(
            context,
            dispatcher,
            ids::POSITION_ENTITY_SPRITES,
            CallSite::relative(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, revision, 0x1e4),
        )?;
        // G CE66-CE6E retains that head while pushing/calling suspension(0),
        // then removes both actual slots together (ADDQ.L #8,SP).
        let suspension = OutgoingArguments::push(context, &[0])?;
        suspension.call(
            context,
            dispatcher,
            ids::INVENTORY_SET_CAMERA_SUSPENSION,
            CallSite::relative(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, revision, 0x1ec),
        )?;
        OutgoingArguments::existing(context, 2)?.release(context)?;
    }
    context.set_data_register(2, d2);
    context.set_data_register(3, d3_high | u32::from(tile_total));
    context.set_data_register(4, d4_high | 40);
    context.set_address_register(2, ram::g_player_info(revision).at(40).address());
    context.set_address_register(3, last_frame);
    // REV02 CE74 / REV00 CE54, offset 0x1f2: actual zero-argument JSR return.
    let site = CallSite::relative(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, revision, 0x1f2);
    call_at(context, ids::BUILD_VIEWPORT_SPRITE_LINKS, &[], site)?;
    pending.write(context, 1);
    // G c_entity_sprite_rendering.s:1328: final MOVE.B #1 sets N/Z/V/C=0.
    context.set_status_register(context.status_register() & !0x0f);
    Ok(None)
}
