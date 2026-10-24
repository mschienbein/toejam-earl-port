//! Leaf rendering operations retaining the compiler's signed condition codes.
use crate::{
    Context, Result,
    call_frame::{FrameLayout, LocalFrame},
};
use tje_mem::{Address, ObjectState, Ptr, RenderPiece, arithmetic::lsl_u32, ram, rom};

/// G/src/sprites/entity_visibility.c:12, IsEntityVisibleInViewport.
/// The matching code uses signed BLT/BLE for the bounds after zero-extending dimensions.
pub fn is_entity_visible_in_viewport(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let entity = Address::<ObjectState>::new(arguments[0]);
    let (x, y, viewport_height) = if arguments[1] as i16 == 0 {
        if ram::g_player_inventory_open(revision).at(0).read(context) != 0
            || entity.level().read(context) != ram::g_player1_map_level(revision).read(context)
        {
            return Ok(Some(0));
        }
        let x = entity
            .x()
            .read(context)
            .wrapping_sub(ram::g_camera1_pending_x(revision).read(context));
        let y = entity
            .y()
            .read(context)
            .wrapping_sub(ram::g_camera1_pending_y(revision).read(context));
        (
            x,
            y,
            if ram::g_split_screen_enabled(revision).read(context) == 0 {
                224
            } else {
                112
            },
        )
    } else {
        if ram::g_split_screen_enabled(revision).read(context) == 0
            || ram::g_player2_inventory_open(revision).read(context) != 0
            || entity.level().read(context) != ram::g_player2_map_level(revision).read(context)
        {
            return Ok(Some(0));
        }
        let x = entity
            .x()
            .read(context)
            .wrapping_sub(ram::g_camera2_pending_x(revision).read(context));
        let y = entity
            .y()
            .read(context)
            .wrapping_sub(ram::g_camera2_pending_y(revision).read(context));
        (x, y, 112)
    };
    let visible = i32::from(entity.width().read(context)) + i32::from(x) >= 0
        && 320 >= i32::from(x) - i32::from(entity.width().read(context))
        && i32::from(y) + 10 > 0
        && viewport_height
            >= i32::from(y)
                - i32::from(entity.height_pixels().read(context))
                - i32::from(entity.height().read(context));
    Ok(Some(u32::from(visible)))
}

/// G/src/sprites/entity_sprite_rendering.c:191, SortEntitiesForSpriteDraw.
/// Equal-Y records stay in index order; flag-7 records form a separate final list.
pub fn sort_entities_for_sprite_draw(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut heads = [0u32; 2];
    let mut tails = [0u32; 2];
    let count = if ram::g_player1_map_level(revision).read(context) == 26 {
        40
    } else {
        26
    };
    for index in 0..count {
        let entity = ram::g_player_info(revision).at(index);
        if entity.draw_flags().read(context) & 1 == 0 {
            continue;
        }
        let list = usize::from(entity.draw_flags().read(context) & 0x80 != 0);
        let mut previous = 0;
        let mut current = heads[list];
        while current != 0 {
            let current_entity = Address::<ObjectState>::new(current);
            if entity.y().read(context) > current_entity.y().read(context) {
                if previous == 0 {
                    entity.next().write(context, Ptr::new(heads[list]));
                    heads[list] = entity.address();
                } else {
                    entity.next().write(context, Ptr::new(current));
                    Address::<ObjectState>::new(previous)
                        .next()
                        .write(context, Ptr::new(entity.address()));
                }
                break;
            }
            previous = current;
            current = current_entity.next().read(context).address();
        }
        if current == 0 {
            if heads[list] == 0 {
                heads[list] = entity.address();
            } else {
                Address::<ObjectState>::new(tails[list])
                    .next()
                    .write(context, Ptr::new(entity.address()));
            }
            entity.next().write(context, Ptr::new(0));
            tails[list] = entity.address();
        }
    }
    if heads[0] != 0 {
        ram::g_entity_draw_list_head(revision).write(context, Ptr::new(heads[0]));
        Address::<ObjectState>::new(tails[0])
            .next()
            .write(context, Ptr::new(heads[1]));
    } else {
        ram::g_entity_draw_list_head(revision).write(context, Ptr::new(heads[1]));
    }
    Ok(None)
}

/// G/src/sprites/entity_sprite_rendering.c:301, ClipSpritePixelsBelowCutoff.
/// Offset arithmetic here zero-extends the buffer word, unlike the ADDA.W copy helpers.
pub fn clip_sprite_pixels_below_cutoff(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let piece = Address::<RenderPiece>::new(arguments[0]);
    let row_width = i16::from(piece.height().read(context)) * 8;
    let row_before_cutoff = i16::from(piece.y().read(context)).wrapping_sub(2);
    let cutoff = arguments[1] as i16;
    let row = row_before_cutoff.wrapping_sub(cutoff);
    // BLT immediately consumes SUB.W's N xor V, not a fresh TST of the narrowed row.
    if row_before_cutoff >= cutoff {
        return Ok(Some(1));
    }
    if i32::from(row_width) + i32::from(row) <= 0 {
        return Ok(Some(0));
    }
    let first_hidden = row.wrapping_neg();
    let mut row = 0i16;
    while (row as i32 as u32) < u32::from(piece.width().read(context)) {
        let displacement = i32::from(row)
            .wrapping_mul(i32::from(row_width))
            .wrapping_mul(4)
            .wrapping_add(i32::from(
                ram::g_sprite_tile_buffer_offset(revision).read(context),
            ))
            .wrapping_add(i32::from(first_hidden).wrapping_mul(4));
        let mut pixels = ram::g_sprite_tile_upload_buffer(revision)
            .address()
            .wrapping_add(displacement as u32);
        let value = context.read_u32(pixels) & 0x0f00_0ff0;
        context.write_u32(pixels, value);
        pixels = pixels.wrapping_add(4);
        let mut column = first_hidden.wrapping_add(1);
        while column < row_width {
            context.write_u32(pixels, 0);
            pixels = pixels.wrapping_add(4);
            column = column.wrapping_add(1);
        }
        row = row.wrapping_add(1);
    }
    Ok(Some(0))
}

/// G/src/sprites/entity_sprite_rendering.c:326, ClearSpriteMaskPixels.
pub fn clear_sprite_mask_pixels(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let offset = ram::g_sprite_tile_buffer_offset(revision).read(context) as i16;
    let mut pixels = ram::g_sprite_tile_upload_buffer(revision)
        .address()
        .wrapping_add(offset as i32 as u32);
    for _ in 0..arguments[0] as u16 {
        let packed = context.read_u8(pixels);
        context.write_u8(
            pixels,
            if matches!(packed, 0xff | 0xf0 | 0x0f) {
                0
            } else {
                packed
            },
        );
        pixels = pixels.wrapping_add(1);
    }
    Ok(None)
}

/// G/src/sprites/entity_sprite_rendering.c:427, AppendSpriteTilesToBuffer.
/// Copy forward in interleaved longword accesses, including overlapping source/destination.
pub fn append_sprite_tiles_to_buffer(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let offset = ram::g_sprite_tile_buffer_offset(revision);
    let mut destination = ram::g_sprite_tile_upload_buffer(revision)
        .address()
        .wrapping_add(offset.read(context) as i16 as i32 as u32);
    let tile_count = arguments[1] as i16;
    let mut source = arguments[0];
    let count = tile_count.wrapping_shl(3);
    for _ in 0..count {
        let value = context.read_u32(source);
        context.write_u32(destination, value);
        source = source.wrapping_add(4);
        destination = destination.wrapping_add(4);
    }
    offset.write(
        context,
        offset
            .read(context)
            .wrapping_add(tile_count.wrapping_shl(5) as u16),
    );
    Ok(None)
}

/// G/src/sprites/set_sprite.c:7, SetSprite. Read descriptors through revision-selected ROM views.
/// Matching c_set_sprite.s:4-41 also defines D0.B, D1, A0/A1 and final CCR.
/// Preserve D0's upper24 bits and the ASR.B X carried through a zero-count ASL.L.
pub fn set_sprite(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let state = arguments[1] as i8;
    context.set_address_register(0, arguments[0]);
    context.set_data_register(
        0,
        (context.data_register(0) & !0xff) | u32::from(state as u8),
    );
    if state < 0 {
        // TST.B sets N and clears Z/V/C while retaining entry X.
        context.set_status_register((context.status_register() & !0x0f) | 8);
        return Ok(None);
    }
    let entity = Address::<ObjectState>::new(arguments[0]);
    let revision = context.revision();
    let definitions = if entity.entity_index().read(context) == 0 {
        rom::player_animation_descriptors(revision)
    } else {
        rom::player2_animation_descriptors(revision)
    };
    let definition = definitions.at(i32::from(state));
    context.set_address_register(1, definition.address());
    entity
        .sequence()
        .write(context, definition.sequence().read(context));
    entity
        .frames()
        .write(context, definition.frames().read(context));
    entity
        .frame_phase()
        .write(context, definition.period().read(context));
    entity.frame_duration().write(context, 1);
    entity.frame().write(context, -1);
    entity.sprite_state().write(context, state);
    let stored_state = entity.sprite_state().read(context);
    let shifted = stored_state >> 2;
    context.set_data_register(
        0,
        (context.data_register(0) & !0xff) | u32::from(shifted as u8),
    );
    let count = u32::from(shifted as u8) & 63;
    let mask = lsl_u32(1, count);
    context.set_data_register(1, mask);
    entity.animation_group_mask().write(context, mask);
    // ASR.B #2 supplies X if ASL.L has count0; otherwise only count32 shifts
    // the initial one out last. MOVE.L to the field then clears V/C, sets N/Z.
    let x = if count == 0 {
        stored_state as u8 & 2 != 0
    } else {
        count == 32
    };
    let nz = if mask == 0 {
        4
    } else if mask & 0x8000_0000 != 0 {
        8
    } else {
        0
    };
    context
        .set_status_register((context.status_register() & !0x1f) | nz | if x { 0x10 } else { 0 });
    Ok(None)
}

/// G/src/sprites/entity_indexes.c:8, SetRAMIndexes. Initialize 40 indices without clearing records.
/// Matching c_entity_indexes.s:4-15 also leaves D0.W=40, A0 just past record39,
/// and Z-only CCR; D0's upper word, D1 and A1 retain their entry values.
pub fn set_ram_indexes(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let table = ram::g_hardware_sprite_table(revision);
    table.at(0).link().write(context, 23);
    table.at(23).link().write(context, 79);
    table.at(24).link().write(context, 79);
    for index in 0..40 {
        ram::g_player_info(revision)
            .at(index)
            .entity_index()
            .write(context, index as i8);
    }
    context.set_data_register(0, (context.data_register(0) & 0xffff_0000) | 40);
    context.set_address_register(0, ram::g_player_info(revision).at(40).address());
    context.set_status_register((context.status_register() & !0x1f) | 4);
    Ok(None)
}

fn append_link_index(context: &mut dyn Context, slots: Address<i16>, count: &mut i16, value: i16) {
    // G's ADD.W doubles the short array index before sign-extended addressing.
    let destination = slots.byte_offset(i32::from(count.wrapping_shl(1)));
    *count = count.wrapping_add(1);
    destination.write(context, value);
}

fn sprite_link(table: u32, number: i16) -> Address<u8> {
    // The short sprite number is ASL.W #3, not a host/full-long product.
    Address::new(
        table
            .wrapping_add(number.wrapping_shl(3) as i32 as u32)
            .wrapping_add(3),
    )
}

/// G/src/sprites/entity_sprite_rendering.c:134, BuildViewportSpriteLinks.
/// Matching c_entity_sprite_rendering.s:348-526 uses LINK #-216 and saves
/// D2-D5/A2-A3; both 54-word arrays remain unchecked emulated local storage.
pub fn build_viewport_sprite_links(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let frame = LocalFrame::enter(context, FrameLayout::link(216, 0x3c, 0x0c))?;
    let primary = frame.local::<i16>(-108);
    let secondary = frame.local::<i16>(-216);
    let mut primary_count = 0i16;
    let mut secondary_count = 0i16;
    let mut pointer = ram::g_entity_draw_list_head(revision)
        .read(context)
        .address();
    while pointer != 0 {
        let entity = Address::<ObjectState>::new(pointer);
        if entity.sprite_ready().read(context) != 0 {
            let count = i16::from(
                entity
                    .current_frame()
                    .read(context)
                    .target()
                    .piece_count()
                    .read(context),
            );
            if entity.draw_flags().read(context) & 2 != 0 {
                for piece in 0..count {
                    let handle = entity.primary_sprites().at(i32::from(piece)).read(context);
                    append_link_index(context, primary, &mut primary_count, i16::from(handle));
                }
            }
            if entity.draw_flags().read(context) & 4 != 0 {
                for piece in 0..count {
                    let handle = entity
                        .secondary_sprites()
                        .at(i32::from(piece))
                        .read(context);
                    append_link_index(context, secondary, &mut secondary_count, i16::from(handle));
                }
            }
        }
        pointer = entity.next().read(context).address();
    }
    pointer = ram::g_entity_draw_list_head(revision)
        .read(context)
        .address();
    while pointer != 0 {
        let entity = Address::<ObjectState>::new(pointer);
        if entity.sprite_ready().read(context) != 0 && entity.draw_flags().read(context) & 0x40 != 0
        {
            if entity.primary_shadow().read(context) != 0 {
                let handle = entity.primary_shadow().read(context);
                append_link_index(context, primary, &mut primary_count, i16::from(handle));
            }
            if entity.secondary_shadow().read(context) != 0 {
                let handle = entity.secondary_shadow().read(context);
                append_link_index(context, secondary, &mut secondary_count, i16::from(handle));
            }
        }
        pointer = entity.next().read(context).address();
    }
    pointer = ram::g_inventory_draw_list_head(revision)
        .read(context)
        .address();
    while pointer != 0 {
        let entity = Address::<ObjectState>::new(pointer);
        if entity.draw_flags().read(context) & 1 != 0 && entity.sprite_ready().read(context) != 0 {
            let count = i16::from(
                entity
                    .current_frame()
                    .read(context)
                    .target()
                    .piece_count()
                    .read(context),
            );
            if entity.draw_flags().read(context) & 2 != 0 {
                for piece in 0..count {
                    let handle = entity.primary_sprites().at(i32::from(piece)).read(context);
                    append_link_index(context, primary, &mut primary_count, i16::from(handle));
                }
            }
            if entity.draw_flags().read(context) & 4 != 0 {
                for piece in 0..count {
                    let handle = entity
                        .secondary_sprites()
                        .at(i32::from(piece))
                        .read(context);
                    append_link_index(context, secondary, &mut secondary_count, i16::from(handle));
                }
            }
        }
        pointer = entity.next().read(context).address();
    }
    let table = ram::g_hardware_sprite_table(revision).address();
    sprite_link(table, 23).write(context, 79);
    let mut previous = 23;
    for piece in 0..primary_count {
        let number = primary
            .byte_offset(i32::from(piece.wrapping_shl(1)))
            .read(context);
        sprite_link(table, number).write(context, 79);
        sprite_link(table, previous).write(context, number as u8);
        previous = number;
    }
    sprite_link(table, 24).write(context, 79);
    sprite_link(table, previous).write(context, 24);
    previous = 24;
    for piece in 0..secondary_count {
        let number = secondary
            .byte_offset(i32::from(piece.wrapping_shl(1)))
            .read(context);
        sprite_link(table, number).write(context, 79);
        sprite_link(table, previous).write(context, number as u8);
        previous = number;
    }
    frame.leave(context)?;
    Ok(None)
}
