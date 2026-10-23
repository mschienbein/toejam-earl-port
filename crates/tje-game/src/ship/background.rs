use crate::{Context, Result, ids};
use tje_mem::{Address, Ptr, RenderFrame, ram, rom, rom_symbol};

fn decrement_animation_timer(value: i8) -> (i8, bool) {
    // SUBQ.B followed immediately by BGT consumes overflow, not a reloaded byte.
    let difference = i16::from(value) - 1;
    (difference as i8, difference <= 0)
}

fn queue_word(context: &mut dyn Context, value: u16) {
    let revision = context.revision();
    let count = ram::g_sprite_tile_upload_queue_word_count(revision);
    let old = count.read(context);
    let next = count.read(context).wrapping_add(1);
    count.write(context, next);
    ram::g_sprite_tile_upload_runs(revision)
        .at(i32::from(old))
        .write(context, value);
}

fn upload_background(
    context: &mut dyn Context,
    frame: Address<RenderFrame>,
    parts: i32,
    frame_number: i8,
) -> Result<()> {
    let revision = context.revision();
    for part_number in 0..parts {
        let part = frame.piece(part_number);
        let unpack = if part.tile_format().read(context) < 0 {
            ids::SPRITE_UNPACK_INTERLEAVED_RLE
        } else {
            ids::SPRITE_UNPACK_RLE
        };
        crate::call(context, unpack, &[part.address(), 16])?;
        let offset = ram::g_sprite_tile_buffer_offset(revision);
        let next = offset.read(context).wrapping_add(512);
        offset.write(context, next);
    }
    queue_word(context, 1);
    queue_word(context, (parts * 16) as u16);
    crate::call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    crate::call(context, ids::TEXT_ANIMATE_FONT_BANKS, &[8])?;
    crate::call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[1, 1])?;
    ram::g_ship_screen_state(revision)
        .background_frame()
        .write(context, frame_number);
    Ok(())
}

/// G/src/ship/ship_screen_and_collection.c:61 (`AnimateShipScreenBackground`).
/// SUBQ.B/BGT tests the widened signed difference, including -128 overflow.
pub fn animate_ship_screen_background(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let timer = ram::g_ship_background_animation_timer(revision);
    let frame_number = if arguments[0] as i8 != 0 {
        timer.write(context, 12);
        1
    } else {
        let (next, animate) = decrement_animation_timer(timer.read(context));
        timer.write(context, next);
        if animate {
            let frame = if ram::g_ship_screen_state(revision)
                .background_frame()
                .read(context)
                == 1
            {
                2
            } else {
                1
            };
            timer.write(context, 12);
            frame
        } else {
            0
        }
    };
    if frame_number != 0 {
        let frames = rom::intro_and_ship_animation_pointers(revision)
            .at(1)
            .read(context)
            .address();
        let frame = Address::<Ptr<RenderFrame>>::new(frames)
            .offset(i32::from(frame_number) + 9)
            .read(context)
            .target();
        upload_background(context, frame, 8, frame_number)?;
    } else {
        crate::call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        crate::call(context, ids::TEXT_ANIMATE_FONT_BANKS, &[8])?;
    }
    Ok(None)
}

/// G/src/ship/ship_screen_and_collection.c:131 (`AnimateTransitionBackground`).
pub fn animate_transition_background(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let background_number = arguments[0] as i16;
    let timer = ram::g_ship_background_animation_timer(revision);
    let (next, animate) = decrement_animation_timer(timer.read(context));
    timer.write(context, next);
    if animate {
        let frame_number = if ram::g_ship_screen_state(revision)
            .background_frame()
            .read(context)
            == 1
        {
            2
        } else {
            1
        };
        timer.write(context, 12);
        let index = i32::from(background_number) * 2 + i32::from(frame_number) - 1;
        let frame = rom::spr_transition_background(revision)
            .at(index)
            .read(context)
            .target();
        upload_background(context, frame, 4, frame_number)?;
    } else {
        crate::call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        crate::call(context, ids::TEXT_ANIMATE_FONT_BANKS, &[8])?;
    }
    Ok(None)
}

/// G/src/ship/ship_screen_and_collection.c:259 (`InitializeShipScreenTilemapAndPalettes`).
/// The page==4 && uninitialized-row test is unreachable under the original bound.
pub fn initialize_ship_screen_tilemap_and_palettes(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for page in 0..4 {
        for row in 0..8 {
            context
                .write_vdp_command(0x5000_0003_u32.wrapping_add(((page * 8 + row) as u32) << 23));
            for repeat in 0..5 {
                let first_tile = (((page ^ repeat) & 1) << 6) + 1;
                for column in 0..8 {
                    let offset = rom::transition_background_tile_offsets(revision)
                        .at(row)
                        .at(column)
                        .read(context);
                    let tile = u16::from(offset).wrapping_add(first_tile as u16);
                    context.write_vdp_data(tile.wrapping_add(0xa000));
                }
            }
        }
    }
    for row in 0..28 {
        for column in 0..40 {
            if row == 0 || row == 27 || column == 0 || column == 39 {
                let mut tile = (crate::call(context, ids::GET_RANDOM_NUMBER, &[])? as u16 & 3) + 41;
                if crate::call(context, ids::GET_RANDOM_NUMBER, &[])? & 1 != 0 {
                    tile |= 0x1800;
                }
                context.write_vdp_command(
                    0x5000_0003_u32
                        .wrapping_add((row as u32) << 23)
                        .wrapping_add((column as u32) << 17),
                );
                context.write_vdp_data(tile.wrapping_add(0x8580));
            }
        }
    }
    context.write_vdp_control(0x911e);
    context.write_vdp_control(0x921c);
    for (palette, slot) in [
        (rom::palette_ship_line1(revision).address(), 0),
        (rom::palette_ship_line2(revision).address(), 1),
        (rom::palette_ship_line3(revision).address(), 2),
        (rom::palette_all_black(revision).address(), 3),
    ] {
        crate::call(context, ids::COPY_PALETTE_TO_CRAM_SLOT, &[palette, slot])?;
    }
    context.write_vdp_control(0x8c81);
    crate::call(context, ids::ENABLE_TEXT_FONT_ANIMATION, &[])?;
    crate::call(context, ids::ANIMATE_SHIP_SCREEN_BACKGROUND, &[1])?;
    Ok(None)
}

/// G/src/ship/ship_screen_and_collection.c:308 (`ShowShipPieceName`).
/// The unmatched switch retains incoming D3 as piece_name (both matching compiler
/// listings), preserving the original uninitialized-local bug without inventing a pointer.
pub fn show_ship_piece_name(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let symbol = match arguments[0] as i16 {
        0 => Some("ShipPiecePart0Str"),
        1 => Some("ShipPiecePart1Str"),
        2 => Some("ShipPiecePart2Str"),
        3 => Some("ShipPiecePart3Str"),
        4 => Some("ShipPiecePart4Str"),
        5 => Some("ShipPiecePart5Str"),
        6 => Some("ShipPiecePart6Str"),
        7 => Some("ShipPiecePart7Str"),
        8 => Some("ShipPiecePart8Str"),
        9 => Some("ShipPiecePart9Str"),
        _ => None,
    };
    let piece_name = symbol.map_or_else(
        || context.data_register(3),
        |name| {
            rom_symbol(revision, name)
                .expect("generated ship string symbol")
                .address
        },
    );
    let mut count = 0_i16;
    while context.read_u8(piece_name.wrapping_add(i32::from(count) as u32)) != 0 {
        count = count.wrapping_add(1);
    }
    context.write_vdp_command(
        0x5c00_0003_u32.wrapping_add(((20 - i32::from(count) / 2) as u32) << 17),
    );
    crate::call(context, ids::TEXT_WRITE_STRING_TILES, &[piece_name])?;
    for _ in 0..90 {
        crate::call(context, ids::ANIMATE_SHIP_SCREEN_BACKGROUND, &[0])?;
    }
    Ok(None)
}

/// G/src/ship/ship_screen_and_collection.c:336 (`ShowMissingShipPieceSummary`).
/// String offsets are the matching source literals' even-aligned layout within
/// the generated GameText_0205E8 symbol, not copied cartridge contents.
pub fn show_missing_ship_piece_summary(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player_number = arguments[0] as i32;
    let player = ram::g_player_info(revision).at(player_number);
    let mut missing = 0_i16;
    let mut on_level = false;
    for piece in 0..10 {
        let assignments = ram::g_ship_piece_level_assignments(revision);
        if assignments.at(piece).read(context) != 0 {
            missing = missing.wrapping_add(1);
            if assignments.at(piece).read(context) == player.level().read(context) {
                on_level = true;
            }
        }
    }
    let same_level = ram::g_player_info(revision)
        .at(player_number ^ 1)
        .level()
        .read(context)
        == player.level().read(context);
    let (command, offset) = match (on_level, same_level, player_number == 0) {
        (true, true, _) => (0x518a_0003, 0),
        (true, false, true) => (0x5186_0003, 32),
        (true, false, false) => (0x5188_0003, 68),
        (false, true, _) => (0x5190_0003, 102),
        (false, false, true) => (0x518c_0003, 126),
        (false, false, false) => (0x518e_0003, 154),
    };
    let text = rom_symbol(revision, "GameText_0205E8")
        .expect("generated ship summary symbol")
        .address;
    context.write_vdp_command(command);
    crate::call(context, ids::TEXT_WRITE_STRING_TILES, &[text + offset])?;
    context.write_vdp_command(0x5c0e_0003);
    crate::call(
        context,
        ids::TEXT_WRITE_DECIMAL_TILES,
        &[i32::from(missing) as u32],
    )?;
    crate::call(context, ids::TEXT_WRITE_STRING_TILES, &[text + 180])?;
    Ok(None)
}

/// G/src/ship/ship_screen_and_collection.c:424 (`WaitForShipScreenAcknowledgement`).
/// Input release and the 240-tick timeout rely on real original IRQ/input progress.
pub fn wait_for_ship_screen_acknowledgement(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player_number = arguments[0] as i16;
    ram::g_frame_counter(revision).write(context, 0);
    'timeout: while ram::g_frame_counter(revision).read(context) < 240 {
        for player in 0..2 {
            if player_number == player || player_number == 2 {
                let pressed = crate::call(context, ids::READ_MAPPED_PLAYER_INPUT, &[player as u32])?
                    as i16
                    & 0x70;
                if pressed != 0 {
                    while crate::call(context, ids::READ_MAPPED_PLAYER_INPUT, &[player as u32])?
                        & 0x70
                        & (i32::from(pressed) as u32)
                        != 0
                    {
                        crate::call(context, ids::ANIMATE_SHIP_SCREEN_BACKGROUND, &[0])?;
                    }
                    break 'timeout;
                }
            }
        }
        crate::call(context, ids::ANIMATE_SHIP_SCREEN_BACKGROUND, &[0])?;
    }
    Ok(None)
}

/// G/src/ship/ship_screen_and_collection.c:108 (`RestoreGameplayGraphics`).
pub fn restore_gameplay_graphics(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    context.write_vdp_control(0x9100);
    context.write_vdp_control(0x9200);
    for (command, source, size) in [
        (
            0x4000_0000,
            rom::level_tiles(revision).address(),
            rom::level_tiles_size(revision),
        ),
        (
            0x4000_0001,
            rom::img_starfield_large(revision).address(),
            rom::starry_background_size(revision),
        ),
        (
            0x4660_0001,
            rom::inventory_label_open(revision).address(),
            rom::inventory_label_open_size(revision),
        ),
        (
            0x79a0_0000,
            rom::inventory_label_identify(revision).address(),
            rom::inventory_label_identify_size(revision),
        ),
    ] {
        context.write_vdp_command(command);
        let words = (i32::from(size.read(context)) << 4) as u32;
        crate::call(context, ids::MOVE_DATA_TO_VRAM, &[source, words])?;
    }
    crate::call(context, ids::DRAW_HUD, &[])?;
    crate::call(
        context,
        ids::DMA_COPY_WORDS_TO_VRAM,
        &[
            ram::g_hardware_sprite_table(revision).address(),
            0xf800,
            320,
        ],
    )?;
    for (palette, slot) in [
        (rom::palette_game_line1(revision).address(), 0),
        (rom::palette_game_line2(revision).address(), 1),
        (rom::palette_game_line3(revision).address(), 2),
        (rom::palette_game_line4(revision).address(), 3),
    ] {
        crate::call(context, ids::COPY_PALETTE_TO_CRAM_SLOT, &[palette, slot])?;
    }
    crate::call(
        context,
        ids::DISABLE_TEXT_FONT_ANIMATION_IF_MENUS_CLOSED,
        &[],
    )?;
    context.write_vdp_control(0x8c89);
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::decrement_animation_timer;

    #[test]
    fn every_animation_timer_byte_preserves_subq_bgt_overflow() {
        for bits in 0..=255_u8 {
            let (stored, animate) = decrement_animation_timer(bits as i8);
            let result = bits.wrapping_sub(1);
            let negative = result & 0x80 != 0;
            let overflow = bits == 0x80;
            let zero = result == 0;
            assert_eq!(stored as u8, result);
            assert_eq!(animate, zero || negative != overflow, "timer {bits:#04x}");
        }
    }
}
