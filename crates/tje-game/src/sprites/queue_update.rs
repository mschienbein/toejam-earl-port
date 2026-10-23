//! Main-thread sprite preparation; waits retain real accepted-interrupt progress.
use crate::{Context, Result, call, execution::WaitRequest, ids};
use tje_mem::{Address, ObjectState, ram};

/// The guarded byte pre-decrement in G/src/sprites/entity_sprite_rendering.c:526.
/// Equality consumes Z only; the widened predicate equals the emitted SUBQ.B zero test.
pub(super) fn tick_frame_duration(context: &mut dyn Context, entity: Address<ObjectState>) -> bool {
    let old = entity.frame_duration().read(context);
    if old == 0 {
        return false;
    }
    let decremented = i32::from(old) - 1;
    entity.frame_duration().write(context, decremented as i8);
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
    let revision = context.revision();
    let pending = ram::g_sprite_graphics_upload_pending(revision);
    let mut tile_total = 0u16;
    call(context, ids::INVENTORY_SET_CAMERA_SUSPENSION, &[1])?;
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
    for number in first..40 {
        let entity = ram::g_player_info(revision).at(number);
        if entity.draw_flags().read(context) & 1 == 0
            || entity.behavior_flags().read(context) & 4 != 0
            || call(context, ids::IS_SPEECH_BUBBLE_ENTITY, &[number as u32])? != 0
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
            let entry = entity
                .sequence()
                .read(context)
                .target()
                .offset(i32::from(next))
                .read(context);
            if entry == -1 {
                next = 0;
            } else if entry == -2 {
                entity.frame_duration().write(context, 0);
                continue;
            }
            Some(next)
        } else if entity.sprite_ready().read(context) == 0 {
            let current = entity.frame().read(context);
            Some(if current == -1 {
                current.wrapping_add(1)
            } else {
                current
            })
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
            let frame = frame_pointer.target();
            let mut reuse_tiles = false;
            let mut already_uploaded = false;
            if frame.reuse_tiles().read(context) != 0 && entity.sprite_ready().read(context) != 0 {
                reuse_tiles = true;
                already_uploaded = true;
            }
            entity.current_frame().write(context, frame_pointer);
            if entity.draw_flags().read(context) & 2 != 0 {
                call(
                    context,
                    ids::PREPARE_ENTITY_SPRITE_FRAME,
                    &[entity.address(), frame.address(), u32::from(reuse_tiles), 0],
                )?;
                reuse_tiles = true;
                entity.sprite_ready().write(context, 1);
            }
            if entity.draw_flags().read(context) & 4 != 0 {
                call(
                    context,
                    ids::PREPARE_ENTITY_SPRITE_FRAME,
                    &[entity.address(), frame.address(), u32::from(reuse_tiles), 1],
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
        call(context, ids::JAM_OUT_SYNCHRONIZE_ENTITY_COPIES, &[])?;
    }
    call(context, ids::SORT_ENTITIES_FOR_SPRITE_DRAW, &[])?;
    let head = ram::g_entity_draw_list_head(revision)
        .read(context)
        .address();
    call(context, ids::POSITION_ENTITY_SPRITES, &[head])?;
    if ram::g_player_inventory_open(revision).at(0).read(context) != 0
        || ram::g_player_inventory_open(revision).at(1).read(context) != 0
    {
        let head = ram::g_inventory_draw_list_head(revision)
            .read(context)
            .address();
        call(context, ids::POSITION_ENTITY_SPRITES, &[head])?;
        call(context, ids::INVENTORY_SET_CAMERA_SUSPENSION, &[0])?;
    }
    call(context, ids::BUILD_VIEWPORT_SPRITE_LINKS, &[])?;
    pending.write(context, 1);
    Ok(None)
}
