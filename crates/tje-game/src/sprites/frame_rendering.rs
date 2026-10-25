//! Metasprite positioning and decoder dispatch, retaining per-piece access order.
use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, ObjectState, RenderFrame, RenderPiece, ram};

fn append_upload_word(context: &mut dyn Context, value: u16) {
    let revision = context.revision();
    let count = ram::g_sprite_tile_upload_queue_word_count(revision);
    let index = count.read(context);
    count.write(context, count.read(context).wrapping_add(1));
    ram::g_sprite_tile_upload_runs(revision)
        .at(i32::from(index))
        .write(context, value);
}

/// G/src/sprites/entity_sprite_rendering.c:271, QueueSpritePieceTiles.
/// The upload gate is restored only after decode and both queued run words.
pub fn queue_sprite_piece_tiles(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let piece = Address::<RenderPiece>::new(arguments[0]);
    let pending = ram::g_sprite_graphics_upload_pending(revision);
    ram::g_v_blank_graphics_paused(revision).write(context, 1);
    let saved = pending.read(context) as i8 as i16;
    pending.write(context, 0);
    ram::g_v_blank_graphics_paused(revision).write(context, 0);
    let tile_count =
        u16::from(piece.width().read(context)) * u16::from(piece.height().read(context));
    let decoder = if piece.tile_format().read(context) < 0 {
        ids::SPRITE_UNPACK_INTERLEAVED_RLE
    } else {
        ids::SPRITE_UNPACK_RLE
    };
    call_void(context, decoder, &[piece.address(), u32::from(tile_count)])?;
    let offset = ram::g_sprite_tile_buffer_offset(revision);
    offset.write(
        context,
        offset
            .read(context)
            .wrapping_add(tile_count.wrapping_shl(5)),
    );
    append_upload_word(context, arguments[1] as u16);
    append_upload_word(context, tile_count);
    pending.write(context, saved as u8);
    Ok(Some(tile_count as i16 as i32 as u32))
}

/// G/src/sprites/entity_sprite_rendering.c:357, PrepareEntitySpriteFrame.
/// Discard remains live between pieces, and the second viewport reuses the first's tiles.
pub fn prepare_entity_sprite_frame(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let entity = Address::<ObjectState>::new(arguments[0]);
    let frame = Address::<RenderFrame>::new(arguments[1]);
    let reuse_tiles = arguments[2] != 0;
    let viewport = arguments[3];
    let mut tile_offset = 0u16;
    let mut discard = 0i16;
    let mut elevation = 0;
    let tile_base = entity.tile().read(context);
    if entity.entity_index().read(context) <= 1 {
        elevation = entity.height().read(context);
    }
    let mut piece_number = 0i16;
    while (piece_number as i32 as u32) < u32::from(frame.piece_count().read(context)) {
        let piece = Address::<RenderPiece>::new(
            frame
                .address()
                .wrapping_add(4)
                .wrapping_add((i32::from(piece_number) * 10) as u32),
        );
        let format = piece.tile_format().read(context);
        let tile_count = u16::from(piece.height().read(context))
            .wrapping_mul(u16::from(piece.width().read(context)));
        if !reuse_tiles {
            let decoder = if format < 0 {
                ids::SPRITE_UNPACK_INTERLEAVED_RLE
            } else if format & 0x40 != 0 {
                ids::SPRITE_UNPACK_RAW
            } else {
                ids::SPRITE_UNPACK_RLE
            };
            call_void(context, decoder, &[piece.address(), u32::from(tile_count)])?;
            if elevation < 0 && entity.animation_group_mask().read(context) as u8 & 0x20 == 0 {
                discard = call(
                    context,
                    ids::CLIP_SPRITE_PIXELS_BELOW_CUTOFF,
                    &[piece.address(), elevation as i32 as u32],
                )? as i16;
            }
            if entity.behavior_flags().read(context) & 0x20 != 0 {
                call_void(
                    context,
                    ids::CLEAR_SPRITE_MASK_PIXELS,
                    &[u32::from(tile_count) << 5],
                )?;
            }
            if discard == 0 {
                let offset = ram::g_sprite_tile_buffer_offset(revision);
                offset.write(
                    context,
                    offset
                        .read(context)
                        .wrapping_add(tile_count.wrapping_shl(5)),
                );
            }
        }
        let sprite_number = if viewport == 0 {
            entity.primary_sprites()
        } else {
            entity.secondary_sprites()
        }
        .at(i32::from(piece_number))
        .read(context);
        let hardware = ram::g_hardware_sprite_table(revision).at(i32::from(sprite_number));
        if discard == 0 {
            let size = (i32::from(piece.width().read(context)) - 1) * 4
                + i32::from(piece.height().read(context))
                - 1;
            hardware.size().write(context, size as u8);
            let palette = ((format as i16 & 3) as u16).wrapping_shl(13);
            hardware.tile().write(
                context,
                palette.wrapping_add(tile_base).wrapping_add(tile_offset),
            );
            tile_offset = tile_offset.wrapping_add(tile_count);
        } else {
            hardware.size().write(context, 0);
            hardware.tile().write(context, 0);
        }
        piece_number = piece_number.wrapping_add(1);
    }
    if !reuse_tiles && tile_offset > 0 {
        append_upload_word(context, tile_base & 0x07ff);
        append_upload_word(context, tile_offset);
    }
    Ok(None)
}

/// G/src/sprites/entity_sprite_rendering.c:43, PositionEntitySprites.
/// A frame's unsigned piece count is narrowed to signed char before either loop.
pub fn position_entity_sprites(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let table = ram::g_hardware_sprite_table(revision);
    let mut pointer = arguments[0];
    while pointer != 0 {
        let entity = Address::<ObjectState>::new(pointer);
        if entity.draw_flags().read(context) & 1 != 0 {
            if entity.behavior_flags().read(context) & 8 != 0 {
                call_void(
                    context,
                    ids::UPDATE_ATTACHED_ENTITY_POSITION,
                    &[entity.address()],
                )?;
            }
            if entity.sprite_ready().read(context) != 0 {
                let frame = entity.current_frame().read(context).target();
                let piece_count = frame.piece_count().read(context) as i8;
                let elevation = entity.height().read(context);
                for viewport in 0..2 {
                    let (view_bit, camera_x, camera_y, bias, sprites, shadow) = if viewport == 0 {
                        (
                            2,
                            ram::g_camera1_pending_x(revision),
                            ram::g_camera1_pending_y(revision),
                            128,
                            entity.primary_sprites(),
                            entity.primary_shadow(),
                        )
                    } else {
                        (
                            4,
                            ram::g_camera2_pending_x(revision),
                            ram::g_camera2_pending_y(revision),
                            240,
                            entity.secondary_sprites(),
                            entity.secondary_shadow(),
                        )
                    };
                    if entity.draw_flags().read(context) & view_bit == 0 {
                        continue;
                    }
                    let screen_x = entity
                        .x()
                        .read(context)
                        .wrapping_add(128)
                        .wrapping_sub(camera_x.read(context));
                    let mut screen_y = entity
                        .y()
                        .read(context)
                        .wrapping_add(bias)
                        .wrapping_sub(camera_y.read(context));
                    for piece_number in 0..i16::from(piece_count) {
                        let sprite_number = sprites.at(i32::from(piece_number)).read(context);
                        let hardware = table.at(i32::from(sprite_number));
                        if entity.behavior_flags().read(context) & 0x40 != 0 {
                            hardware.y().write(context, 1);
                        } else {
                            let piece = Address::<RenderPiece>::new(
                                frame
                                    .address()
                                    .wrapping_add(4)
                                    .wrapping_add((i32::from(piece_number) * 10) as u32),
                            );
                            let offset = if entity.tile().read(context) & 0x0800 != 0 {
                                piece.flipped_x().read(context)
                            } else {
                                piece.x().read(context)
                            };
                            hardware
                                .x()
                                .write(context, i16::from(offset).wrapping_add(screen_x));
                            if !(-224..=224).contains(&elevation) {
                                hardware.y().write(context, 0);
                            } else {
                                hardware.y().write(
                                    context,
                                    i16::from(piece.y().read(context))
                                        .wrapping_add(screen_y)
                                        .wrapping_sub(elevation),
                                );
                            }
                        }
                    }
                    let sprite_number = shadow.read(context);
                    if sprite_number != 0 && entity.behavior_flags().read(context) & 0x40 == 0 {
                        if call(
                            context,
                            ids::IS_SPRITE_SLOT_LOCKED,
                            &[sprite_number as i32 as u32],
                        )? == 0
                        {
                            let hardware = table.at(i32::from(sprite_number));
                            let width = i16::from(hardware.size().read(context) & 12) + 4;
                            hardware.x().write(context, screen_x.wrapping_sub(width));
                            screen_y = screen_y.wrapping_sub(if elevation < 0 { 2 } else { 4 });
                            hardware.y().write(context, screen_y);
                        } else {
                            shadow.write(context, 0);
                        }
                    }
                }
            }
        }
        pointer = entity.next().read(context).address();
    }
    Ok(None)
}
