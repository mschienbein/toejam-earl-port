//! RAM-only entity update controllers and their reviewed animation callbacks.
use crate::{Context, Error, Result, call, function_at, ids};
use tje_mem::{Address, ObjectState, Ptr, ram, rom};

/// G/src/sprites/sprite_slots.c:259, FreeEntity. Preserve players and inactive records.
pub fn free_entity(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let number = arguments[0] as i16;
    if number <= 1 {
        return Ok(None);
    }
    let entity = ram::g_player_info(revision)
        .byte_offset(i32::from(number.wrapping_shl(7)))
        .cast::<ObjectState>();
    if entity.draw_flags().read(context) & 1 != 0 {
        if entity.draw_flags().read(context) & 2 != 0 {
            call(
                context,
                ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                &[entity.address(), 0],
            )?;
        }
        if entity.draw_flags().read(context) & 4 != 0 {
            call(
                context,
                ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                &[entity.address(), 1],
            )?;
        }
        entity.draw_flags().write(context, 0);
        if number >= 6 {
            ram::g_dynamic_entity_slot_allocated(revision)
                .at(i32::from(number) - 6)
                .write(context, 0);
        }
    }
    Ok(None)
}

/// G/src/sprites/entity_depth.c:10, UpdateEntityShadows.
/// A zero depth deliberately indexes the byte preceding SpriteTileAllocationLookup.
pub fn update_entity_shadows(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let entity = Address::<ObjectState>::new(arguments[0]);
    let table = ram::g_hardware_sprite_table(revision);
    let mut sprite_number = entity.primary_shadow().read(context);
    for viewport in 0..2 {
        let bit = if viewport == 0 { 2 } else { 4 };
        if entity.draw_flags().read(context) & bit != 0 {
            if call(
                context,
                ids::IS_SPRITE_SLOT_LOCKED,
                &[sprite_number as i32 as u32],
            )? != 0
            {
                sprite_number = 0;
            }
            if sprite_number == 0 {
                sprite_number = call(context, ids::ALLOCATE_SPRITE_SLOT, &[viewport, 0])? as i8;
                if sprite_number == 0 {
                    return Ok(None);
                }
                let hardware = table.at(i32::from(sprite_number));
                hardware
                    .size()
                    .write(context, hardware.size().read(context) & 0x8f);
                if viewport == 0 {
                    entity.primary_shadow().write(context, sprite_number);
                } else {
                    entity.secondary_shadow().write(context, sprite_number);
                }
            }
            if sprite_number != 0 {
                let hardware = table.at(i32::from(sprite_number));
                if entity.height().read(context) < 0 && entity.sprite_state().read(context) != -1 {
                    hardware.size().write(context, 12);
                    hardware.tile().write(context, 0x87f9);
                } else {
                    let old_depth = hardware.size().read(context) & 0x70;
                    let mut depth = if entity.height().read(context) >= 128
                        || entity.sprite_state().read(context) == -1
                    {
                        16
                    } else {
                        (127i32 - i32::from(entity.height().read(context))) as u8
                    };
                    depth &= 0x70;
                    if depth != old_depth {
                        hardware.size().write(context, depth);
                        depth >>= 4;
                        hardware
                            .size()
                            .write(context, hardware.size().read(context) | ((depth << 1) & 12));
                        let tile = u16::from(
                            rom::sprite_tile_allocation_lookup(revision)
                                .at(i32::from(depth) - 1)
                                .read(context),
                        )
                        .wrapping_add(0xe7a0);
                        hardware.tile().write(context, tile);
                    }
                }
            }
        }
        sprite_number = entity.secondary_shadow().read(context);
    }
    Ok(None)
}

/// G/src/sprites/entity_depth.c:63, UpdateAllEntityShadows. Always the 26-record prefix.
pub fn update_all_entity_shadows(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..26 {
        let entity = ram::g_player_info(revision).at(index);
        if entity.draw_flags().read(context) & 1 != 0 {
            let function = if entity.draw_flags().read(context) & 0x40 != 0 {
                ids::UPDATE_ENTITY_SHADOWS
            } else {
                ids::RELEASE_ENTITY_SHADOWS
            };
            call(context, function, &[entity.address()])?;
        }
    }
    Ok(None)
}

/// G/src/sprites/entity_visibility.c:45, UpdateEntityViewportSprites.
/// Preserve the original caller's D4 lifetime for the allocation helper's
/// uninitialized-tile bug: CLR.B initially, MOVEQ #1 if secondary allocation is needed.
pub fn update_entity_viewport_sprites(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let saved = context.data_register(4);
    context.set_data_register(4, saved & !0xff);
    let result = update_viewport_inner(context, arguments);
    context.set_data_register(4, saved);
    result
}

fn update_viewport_inner(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let number = arguments[0] as i32;
    let entity = ram::g_player_info(context.revision()).at(number);
    let mut add_primary = false;
    let mut add_secondary = false;
    if entity.draw_flags().read(context) & 1 == 0 {
        return Ok(None);
    }
    if call(
        context,
        ids::IS_ENTITY_VISIBLE_IN_VIEWPORT,
        &[entity.address(), 0],
    )? as i16
        != 0
    {
        if call(context, ids::IS_SPEECH_BUBBLE_ENTITY, &[number as u32])? == 0
            && entity.draw_flags().read(context) & 2 == 0
        {
            add_primary = true;
        }
    } else if entity.draw_flags().read(context) & 2 != 0 {
        call(
            context,
            ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
            &[entity.address(), 0],
        )?;
    }
    if call(
        context,
        ids::IS_ENTITY_VISIBLE_IN_VIEWPORT,
        &[entity.address(), 1],
    )? as i16
        != 0
    {
        if call(context, ids::IS_SPEECH_BUBBLE_ENTITY, &[number as u32])? == 0
            && entity.draw_flags().read(context) & 4 == 0
        {
            add_secondary = true;
            context.set_data_register(4, 1);
        }
    } else if entity.draw_flags().read(context) & 4 != 0 {
        call(
            context,
            ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
            &[entity.address(), 1],
        )?;
    }
    if add_primary {
        if number == 0 && entity.draw_flags().read(context) & 4 != 0 {
            call(
                context,
                ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                &[entity.address(), 1],
            )?;
        }
        if number != 1 || entity.draw_flags().read(context) & 4 == 0 {
            entity.sprite_ready().write(context, 0);
            call(
                context,
                ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
                &[entity.address(), 0],
            )?;
        }
    }
    if add_secondary {
        if number == 1 && entity.draw_flags().read(context) & 2 != 0 {
            call(
                context,
                ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                &[entity.address(), 0],
            )?;
        }
        if number != 0 || entity.draw_flags().read(context) & 2 == 0 {
            entity.sprite_ready().write(context, 0);
            call(
                context,
                ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
                &[entity.address(), 1],
            )?;
        }
    }
    Ok(None)
}

/// G/src/sprites/entity_visibility.c:83, UpdateAllEntityViewportSprites.
pub fn update_all_entity_viewport_sprites(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let count = if ram::g_player1_map_level(revision).read(context) == 26 {
        ram::g_inventory_draw_list_head(revision).write(context, Ptr::new(0));
        40
    } else {
        26
    };
    for index in 0..count {
        call(context, ids::UPDATE_ENTITY_VIEWPORT_SPRITES, &[index])?;
    }
    Ok(None)
}

/// G/src/sprites/entity_sprite_rendering.c:444, ProcessEntityAnimationMotion.
/// ORIGINAL BUG: the not-ready branch adds tiles only after the total exceeds 112.
pub fn process_entity_animation_motion(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let count = if ram::g_player1_map_level(revision).read(context) == 26 {
        40
    } else {
        26
    };
    let mut tile_total = 0i16;
    for index in 0..count {
        let entity = ram::g_player_info(revision).at(index);
        if entity.draw_flags().read(context) & 1 == 0
            || call(context, ids::IS_SPEECH_BUBBLE_ENTITY, &[index as u32])? != 0
        {
            continue;
        }
        if i32::from(entity.frame_duration().read(context)) - 1 == 0 {
            if i32::from(entity.tile_count().read(context)) + i32::from(tile_total) > 112 {
                continue;
            }
            let mut next = entity.frame().read(context).wrapping_add(1);
            let entry = entity
                .sequence()
                .read(context)
                .target()
                .offset(i32::from(next))
                .read(context) as u8;
            if entry == 255 {
                next = 0;
            } else if entry == 254 {
                continue;
            }
            let frame_number = entity
                .sequence()
                .read(context)
                .target()
                .offset(i32::from(next))
                .read(context) as u8
                & 0x7f;
            let frame = entity
                .frames()
                .read(context)
                .target()
                .offset(i32::from(frame_number))
                .read(context)
                .target();
            let needs_tiles =
                frame.reuse_tiles().read(context) == 0 || entity.sprite_ready().read(context) == 0;
            if entity.animation_motion_axes().read(context) & 3 != 0 {
                let address = entity.process().read(context).address();
                let callback = function_at(revision, address).ok_or_else(|| {
                    Error::Platform(format!("unknown sprite motion callback {address:#010x}"))
                })?;
                call(context, callback, &[frame.address(), entity.address()])?;
            }
            if entity.draw_flags().read(context) & 6 != 0 && needs_tiles {
                tile_total = tile_total.wrapping_add(i16::from(entity.tile_count().read(context)));
            }
        } else if entity.sprite_ready().read(context) == 0
            && entity.draw_flags().read(context) & 6 != 0
            && i32::from(entity.tile_count().read(context)) + i32::from(tile_total) > 112
        {
            tile_total = tile_total.wrapping_add(i16::from(entity.tile_count().read(context)));
        }
    }
    Ok(None)
}
