use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
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
    caller: crate::FunctionId,
) -> Result<()> {
    let revision = context.revision();
    let dispatcher = context.dispatcher();
    let delta = if caller == ids::ANIMATE_TRANSITION_BACKGROUND {
        8
    } else {
        0
    };
    let site = |offset: u32| CallSite::relative(caller, revision, offset - delta);
    for part_number in 0..parts {
        let part = frame.piece(part_number);
        let (unpack, return_offset) = if part.tile_format().read(context) < 0 {
            (ids::SPRITE_UNPACK_INTERLEAVED_RLE, 0x76)
        } else {
            (ids::SPRITE_UNPACK_RLE, 0x84)
        };
        dispatcher.call_at_void(context, unpack, &[part.address(), 16], site(return_offset))?;
        let offset = ram::g_sprite_tile_buffer_offset(revision);
        let next = offset.read(context).wrapping_add(512);
        offset.write(context, next);
    }
    queue_word(context, 1);
    queue_word(context, (parts * 16) as u16);
    let wait = OutgoingArguments::push(context, &[1])?;
    wait.call_void(context, dispatcher, ids::WAIT_FOR_NEXT_V_BLANK, site(0xd8))?;
    let font = OutgoingArguments::push(context, &[8])?;
    font.call(
        context,
        dispatcher,
        ids::TEXT_ANIMATE_FONT_BANKS,
        site(0xe2),
    )?;
    let upload = OutgoingArguments::push(context, &[1, 1])?;
    upload.call_void(
        context,
        dispatcher,
        ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
        site(0xf0),
    )?;
    upload.release(context)?;
    font.release(context)?;
    wait.release(context)?;
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
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x04))?;
    let result = (|| {
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
            upload_background(
                context,
                frame,
                8,
                frame_number,
                ids::ANIMATE_SHIP_SCREEN_BACKGROUND,
            )?;
        } else {
            let dispatcher = context.dispatcher();
            let wait = OutgoingArguments::push(context, &[1])?;
            wait.call_void(
                context,
                dispatcher,
                ids::WAIT_FOR_NEXT_V_BLANK,
                CallSite::relative(ids::ANIMATE_SHIP_SCREEN_BACKGROUND, revision, 0x106),
            )?;
            let font = OutgoingArguments::push(context, &[8])?;
            font.call(
                context,
                dispatcher,
                ids::TEXT_ANIMATE_FONT_BANKS,
                CallSite::relative(ids::ANIMATE_SHIP_SCREEN_BACKGROUND, revision, 0x110),
            )?;
            font.release(context)?;
            wait.release(context)?;
        }
        Ok(None)
    })();
    super::screen::leave_frame(context, frame, result)
}

/// G/src/ship/ship_screen_and_collection.c:131 (`AnimateTransitionBackground`).
pub fn animate_transition_background(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x04))?;
    let result = (|| {
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
            upload_background(
                context,
                frame,
                4,
                frame_number,
                ids::ANIMATE_TRANSITION_BACKGROUND,
            )?;
        } else {
            let dispatcher = context.dispatcher();
            let wait = OutgoingArguments::push(context, &[1])?;
            wait.call_void(
                context,
                dispatcher,
                ids::WAIT_FOR_NEXT_V_BLANK,
                CallSite::relative(ids::ANIMATE_TRANSITION_BACKGROUND, revision, 0xfe),
            )?;
            let font = OutgoingArguments::push(context, &[8])?;
            font.call(
                context,
                dispatcher,
                ids::TEXT_ANIMATE_FONT_BANKS,
                CallSite::relative(ids::ANIMATE_TRANSITION_BACKGROUND, revision, 0x108),
            )?;
            font.release(context)?;
            wait.release(context)?;
        }
        Ok(None)
    })();
    super::screen::leave_frame(context, frame, result)
}

/// G/src/ship/ship_screen_and_collection.c:259 (`InitializeShipScreenTilemapAndPalettes`).
/// The page==4 && uninitialized-row test is unreachable under the original bound.
pub fn initialize_ship_screen_tilemap_and_palettes(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0x7c, 0x04))?;
    let result = (|| {
        let revision = context.revision();
        let dispatcher = context.dispatcher();
        let site = |offset| {
            CallSite::relative(
                ids::INITIALIZE_SHIP_SCREEN_TILEMAP_AND_PALETTES,
                revision,
                offset,
            )
        };
        for page in 0..4 {
            for row in 0..8 {
                context.write_vdp_command(
                    0x5000_0003_u32.wrapping_add(((page * 8 + row) as u32) << 23),
                );
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
                    let mut tile =
                        (dispatcher.call_at(context, ids::GET_RANDOM_NUMBER, &[], site(0xca))?
                            as u16
                            & 3)
                            + 41;
                    if dispatcher.call_at(context, ids::GET_RANDOM_NUMBER, &[], site(0xda))? & 1
                        != 0
                    {
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
        let first =
            OutgoingArguments::push(context, &[rom::palette_ship_line1(revision).address(), 0])?;
        first.call_void(
            context,
            dispatcher,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            site(0x144),
        )?;
        let second =
            OutgoingArguments::push(context, &[rom::palette_ship_line2(revision).address(), 1])?;
        second.call_void(
            context,
            dispatcher,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            site(0x150),
        )?;
        let third =
            OutgoingArguments::push(context, &[rom::palette_ship_line3(revision).address(), 2])?;
        third.call_void(
            context,
            dispatcher,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            site(0x15c),
        )?;
        let black =
            OutgoingArguments::push(context, &[rom::palette_all_black(revision).address(), 3])?;
        black.call_void(
            context,
            dispatcher,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            site(0x168),
        )?;
        black.release(context)?;
        third.release(context)?;
        second.release(context)?;
        first.release(context)?;
        context.write_vdp_control(0x8c81);
        dispatcher.call_at_void(context, ids::ENABLE_TEXT_FONT_ANIMATION, &[], site(0x17a))?;
        dispatcher.call_at_void(
            context,
            ids::ANIMATE_SHIP_SCREEN_BACKGROUND,
            &[1],
            site(0x182),
        )?;
        Ok(None)
    })();
    super::screen::leave_frame(context, frame, result)
}

/// G/src/ship/ship_screen_and_collection.c:308 (`ShowShipPieceName`).
/// The unmatched switch retains incoming D3 as piece_name (both matching compiler
/// listings), preserving the original uninitialized-local bug without inventing a pointer.
pub fn show_ship_piece_name(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0))?;
    let result = (|| {
        let revision = context.revision();
        let dispatcher = context.dispatcher();
        let site = |offset| CallSite::relative(ids::SHOW_SHIP_PIECE_NAME, revision, offset);
        let piece = arguments[0] as i16;
        if (0..10).contains(&piece) {
            // Both matching listings: MOVE.W jtbl(PC,D0.W),D0 before JMP.
            // The switch itself remains Rust-owned; cartridge code is immutable.
            let table = ids::SHOW_SHIP_PIECE_NAME.metadata().address(revision) + 0x20;
            let _ = context.read_u16(table + u32::from(piece as u16) * 2);
        }
        let symbol = match piece {
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
        dispatcher.call_at_void(
            context,
            ids::TEXT_WRITE_STRING_TILES,
            &[piece_name],
            site(0xc4),
        )?;
        for _ in 0..90 {
            dispatcher.call_at_void(
                context,
                ids::ANIMATE_SHIP_SCREEN_BACKGROUND,
                &[0],
                site(0xce),
            )?;
        }
        Ok(None)
    })();
    super::screen::leave_frame(context, frame, result)
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
    crate::call_void(context, ids::TEXT_WRITE_STRING_TILES, &[text + offset])?;
    context.write_vdp_command(0x5c0e_0003);
    crate::call_void(
        context,
        ids::TEXT_WRITE_DECIMAL_TILES,
        &[i32::from(missing) as u32],
    )?;
    crate::call_void(context, ids::TEXT_WRITE_STRING_TILES, &[text + 180])?;
    Ok(None)
}

/// G/src/ship/ship_screen_and_collection.c:424 (`WaitForShipScreenAcknowledgement`).
/// Input release and the 240-tick timeout rely on real original IRQ/input progress.
pub fn wait_for_ship_screen_acknowledgement(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x04))?;
    let result = (|| {
        let revision = context.revision();
        let player_number = arguments[0] as i16;
        let dispatcher = context.dispatcher();
        let site = |offset| {
            CallSite::relative(ids::WAIT_FOR_SHIP_SCREEN_ACKNOWLEDGEMENT, revision, offset)
        };
        context.discard_u16(ram::g_frame_counter(revision).address())?;
        ram::g_frame_counter(revision).write(context, 0);
        'timeout: while ram::g_frame_counter(revision).read(context) < 240 {
            for player in 0..2 {
                if player_number == player || player_number == 2 {
                    let pressed = dispatcher.call_at(
                        context,
                        ids::READ_MAPPED_PLAYER_INPUT,
                        &[player as u32],
                        site(if player == 0 { 0x26 } else { 0x60 }),
                    )? as i16
                        & 0x70;
                    if pressed != 0 {
                        while dispatcher.call_at(
                            context,
                            ids::READ_MAPPED_PLAYER_INPUT,
                            &[player as u32],
                            site(if player == 0 { 0x3e } else { 0x7a }),
                        )? & 0x70
                            & (i32::from(pressed) as u32)
                            != 0
                        {
                            dispatcher.call_at_void(
                                context,
                                ids::ANIMATE_SHIP_SCREEN_BACKGROUND,
                                &[0],
                                site(if player == 0 { 0x38 } else { 0x72 }),
                            )?;
                            context.poll_yield()?;
                        }
                        break 'timeout;
                    }
                }
            }
            dispatcher.call_at_void(
                context,
                ids::ANIMATE_SHIP_SCREEN_BACKGROUND,
                &[0],
                site(0x90),
            )?;
            context.poll_yield()?;
        }
        Ok(None)
    })();
    super::screen::leave_frame(context, frame, result)
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
        crate::call_void(context, ids::MOVE_DATA_TO_VRAM, &[source, words])?;
    }
    crate::call_void(context, ids::DRAW_HUD, &[])?;
    crate::call_void(
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
        crate::call_void(context, ids::COPY_PALETTE_TO_CRAM_SLOT, &[palette, slot])?;
    }
    crate::call_void(
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
