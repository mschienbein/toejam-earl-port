use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, ids};
use tje_mem::{Address, HardwareSprite, Ptr, RenderFrame, ram, rom};

pub(super) fn leave_frame(
    context: &mut dyn Context,
    frame: LocalFrame,
    result: Result<Option<u32>>,
) -> Result<Option<u32>> {
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

/// G/src/ship/ship_screen_and_collection.c:380 (`AnimateShipScreenPalette`).
/// Matching LINK -36 / MOVEM D2-D6,A2-A3; palette starts at A6-36, the
/// high/low component words are A6-4/-2. CallSite offsets are post-JSR PCs
/// in the matching linked compiler output, relative to this function in each ROM.
pub fn animate_ship_screen_palette(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let dispatcher = context.dispatcher();
    let site = |offset| CallSite::relative(ids::ANIMATE_SHIP_SCREEN_PALETTE, revision, offset);
    let frame = LocalFrame::enter(context, FrameLayout::link(36, 0x7c, 0x0c))?;
    let result = (|| {
        let palette = frame.local::<u16>(-36);
        let high_component = frame.local::<i16>(-4);
        let low_component = frame.local::<i16>(-2);
        let source = rom::palette_ship_line3(revision);
        let lookup = rom::palette_fade_lookup(revision);
        let mut fade_step = 0_i16;
        for color in 0..16 {
            context.discard_u16(palette.offset(color).address())?;
            palette.offset(color).write(context, 0);
            dispatcher.call_at(
                context,
                ids::ANIMATE_SHIP_SCREEN_BACKGROUND,
                &[0],
                site(0x26),
            )?;
        }
        dispatcher.call_at(context, ids::PLAY_SOUND, &[12, 90], site(0x3e))?;
        for tick in 1..61 {
            if matches!(tick, 8 | 16 | 22 | 28 | 32 | 36 | 38 | 40) {
                fade_step = fade_step.wrapping_add(1);
            } else if matches!(tick, 44 | 48 | 54 | 60) {
                fade_step = fade_step.wrapping_sub(1);
            } else {
                dispatcher.call_at(
                    context,
                    ids::ANIMATE_SHIP_SCREEN_BACKGROUND,
                    &[0],
                    site(0x94),
                )?;
                continue;
            }
            for color in 1..16 {
                let value = source.at(color).read(context) as i16;
                let high = i16::from(
                    lookup
                        .at(i32::from(fade_step) - 1)
                        .at((i32::from(value) >> 9) & 7)
                        .read(context),
                );
                high_component.write(context, high);
                let value = source.at(color).read(context) as i16;
                let middle = i16::from(
                    lookup
                        .at(i32::from(fade_step) - 1)
                        .at(i32::from((i32::from(value) >> 5) as i16) & 7)
                        .read(context),
                );
                let value = source.at(color).read(context) as i16;
                let low = i16::from(
                    lookup
                        .at(i32::from(fade_step) - 1)
                        .at(i32::from((i32::from(value) >> 1) as i16) & 7)
                        .read(context),
                );
                low_component.write(context, low);
                let value = ((i32::from(high_component.read(context)) << 9)
                    | (i32::from(middle) << 5)
                    | (i32::from(low_component.read(context)) << 1))
                    as u16;
                palette.offset(color).write(context, value);
            }
            // The compiler retains Wait's argument while pushing Copy's two
            // arguments, then cleans twelve bytes only after the palette call.
            let wait = OutgoingArguments::push(context, &[1])?;
            wait.call(context, dispatcher, ids::WAIT_FOR_NEXT_V_BLANK, site(0x162))?;
            let copy = OutgoingArguments::push(context, &[palette.address(), 3])?;
            copy.call(
                context,
                dispatcher,
                ids::COPY_PALETTE_TO_CRAM_SLOT,
                site(0x170),
            )?;
            copy.release(context)?;
            wait.release(context)?;
        }
        Ok(None)
    })();
    leave_frame(context, frame, result)
}

/// G/src/ship/ship_screen_and_collection.c:468 (`ShowShipPieces`).
/// Matching LINK -420 / MOVEM D2-D7,A2-A4. All ten local scalar/pointer
/// slots and the 50-sprite array retain their real emulated addresses. The
/// original 640-byte DMA intentionally reads past that 400-byte array: unwritten
/// bytes and the caller tail are not initialized, shortened, or replaced.
pub fn show_ship_pieces(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let dispatcher = context.dispatcher();
    let site = |offset| CallSite::relative(ids::SHOW_SHIP_PIECES, revision, offset);
    let frame = LocalFrame::enter(context, FrameLayout::link(420, 0xfc, 0x1c))?;
    let result = (|| {
        let selected_piece = arguments[0] as i16;
        let player_number = arguments[1] as i16;
        let ship_number = frame.local::<i16>(-2);
        let tile_offset = frame.local::<i16>(-4);
        let sprite_number = frame.local::<i16>(-6);
        let part_number = frame.local::<i16>(-8);
        let base_tile = frame.local::<u16>(-10);
        let pass = frame.local::<i16>(-12);
        let mode_pointer = frame.local::<u32>(-16);
        let frames_pointer = frame.local::<u32>(-20);
        let sprites = frame.local::<HardwareSprite>(-420);
        let levels = ram::g_ship_piece_level_assignments(revision);
        let mode = ram::g_mode2_reg_softcopy(revision);
        // Original word loads retain the upper register halves; D3 later
        // supplies ShowShipPieceName's out-of-range uninitialized-pointer input.
        let d3 = (context.data_register(3) & 0xffff_0000) | u32::from(selected_piece as u16);
        let d4 = (context.data_register(4) & 0xffff_0000) | u32::from(player_number as u16);
        context.set_data_register(3, d3);
        context.set_data_register(4, d4);
        context.set_address_register(3, levels.address());
        base_tile.write(context, 129);
        sprite_number.write(context, 0);
        mode_pointer.write(context, mode.address());
        dispatcher.call_at(context, ids::DISABLE_BLANKING_GRAPHICS, &[], site(0x2e))?;
        let blanked = mode.read(context) & 0x3c;
        mode.write(context, blanked);
        context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
        let initial_upload = OutgoingArguments::push(context, &[1, 1])?;
        initial_upload.call(
            context,
            dispatcher,
            ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
            site(0x5e),
        )?;
        // The upload's two slots remain allocated during this no-argument call.
        dispatcher.call_at(
            context,
            ids::INITIALIZE_SHIP_SCREEN_TILEMAP_AND_PALETTES,
            &[],
            site(0x62),
        )?;
        initial_upload.release(context)?;
        if selected_piece >= 0 {
            context.discard_u8(levels.at(i32::from(selected_piece)).address())?;
            levels.at(i32::from(selected_piece)).write(context, 0);
        }
        let frames = rom::intro_and_ship_animation_pointers(revision)
            .at(1)
            .read(context)
            .address();
        frames_pointer.write(context, frames);
        pass.write(context, 0);
        loop {
            ship_number.write(context, 0);
            loop {
                if (pass.read(context) == 0
                    && levels
                        .at(i32::from(ship_number.read(context)))
                        .read(context)
                        != 0)
                    || (pass.read(context) == 1
                        && levels
                            .at(i32::from(ship_number.read(context)))
                            .read(context)
                            == 0)
                {
                    let next = ship_number.read(context).wrapping_add(1);
                    ship_number.write(context, next);
                    if ship_number.read(context) >= 10 {
                        break;
                    }
                    continue;
                }
                let ship = ship_number.read(context);
                let frames = Address::<Ptr<RenderFrame>>::new(frames_pointer.read(context));
                let picture = frames.offset(i32::from(ship)).read(context).target();
                context.set_address_register(4, picture.address());
                tile_offset.write(context, 0);
                part_number.write(context, 0);
                while u32::from(picture.piece_count().read(context))
                    > (i32::from(part_number.read(context)) as u32)
                {
                    // A byte piece_count bounds the signed word index to 0..254;
                    // its emitted word-times-ten displacement therefore fits.
                    let part = picture.piece(i32::from(part_number.read(context)));
                    context.set_address_register(2, part.address());
                    let mut format = part.tile_format().read(context);
                    let tiles = (u32::from(part.height().read(context))
                        * u32::from(part.width().read(context)))
                        as i16;
                    let (unpack, return_offset) = if format < 0 {
                        (ids::SPRITE_UNPACK_INTERLEAVED_RLE, 0x100)
                    } else {
                        (ids::SPRITE_UNPACK_RLE, 0x110)
                    };
                    dispatcher.call_at(
                        context,
                        unpack,
                        &[part.address(), i32::from(tiles) as u32],
                        site(return_offset),
                    )?;
                    if levels
                        .at(i32::from(ship_number.read(context)))
                        .read(context)
                        != 0
                    {
                        dispatcher.call_at(
                            context,
                            ids::FILL_SPRITE_OPAQUE_PIXELS,
                            &[i32::from(tiles) as u32],
                            site(0x126),
                        )?;
                    }
                    let offset = ram::g_sprite_tile_buffer_offset(revision);
                    let next = offset
                        .read(context)
                        .wrapping_add((i32::from(tiles) << 5) as u16);
                    offset.write(context, next);
                    if ship_number.read(context) == selected_piece {
                        format = 3;
                    }
                    let size = ((u32::from(part.width().read(context)).wrapping_sub(1)) << 2) as u8;
                    let size = size.wrapping_add(part.height().read(context).wrapping_sub(1));
                    let sprite =
                        sprites.byte_offset(i32::from(sprite_number.read(context).wrapping_shl(3)));
                    sprite.size().write(context, size);
                    let sprite =
                        sprites.byte_offset(i32::from(sprite_number.read(context).wrapping_shl(3)));
                    let x = (i32::from(part.x().read(context)) + 298) as i16;
                    sprite.x().write(context, x);
                    let sprite =
                        sprites.byte_offset(i32::from(sprite_number.read(context).wrapping_shl(3)));
                    let y = (i32::from(part.y().read(context)) + 288) as i16;
                    sprite.y().write(context, y);
                    let sprite =
                        sprites.byte_offset(i32::from(sprite_number.read(context).wrapping_shl(3)));
                    sprite
                        .link()
                        .write(context, (i32::from(sprite_number.read(context)) + 1) as u8);
                    let sprite =
                        sprites.byte_offset(i32::from(sprite_number.read(context).wrapping_shl(3)));
                    let tile = (i32::from((format as i16) & 3) << 13)
                        + 0x8000
                        + i32::from(base_tile.read(context))
                        + i32::from(tile_offset.read(context));
                    sprite.tile().write(context, tile as u16);
                    let next = tile_offset.read(context).wrapping_add(tiles);
                    tile_offset.write(context, next);
                    let next = sprite_number.read(context).wrapping_add(1);
                    sprite_number.write(context, next);
                    let next = part_number.read(context).wrapping_add(1);
                    part_number.write(context, next);
                }
                for local_offset in [-10, -4] {
                    let count = ram::g_sprite_tile_upload_queue_word_count(revision);
                    let index = count.read(context);
                    let next = count.read(context).wrapping_add(1);
                    count.write(context, next);
                    let value = frame.local::<u16>(local_offset).read(context);
                    ram::g_sprite_tile_upload_runs(revision)
                        .at(i32::from(index))
                        .write(context, value);
                }
                let tiles = tile_offset.read(context) as u16;
                let next = base_tile.read(context).wrapping_add(tiles);
                base_tile.write(context, next);
                dispatcher.call_at(
                    context,
                    ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
                    &[1, 1],
                    site(0x254),
                )?;
                let next = ship_number.read(context).wrapping_add(1);
                ship_number.write(context, next);
                if ship_number.read(context) >= 10 {
                    break;
                }
            }
            let next = pass.read(context).wrapping_add(1);
            pass.write(context, next);
            if pass.read(context) >= 2 {
                break;
            }
        }
        // This final array subscript is long arithmetic in the matching output.
        let last = sprites.offset(i32::from(sprite_number.read(context)) - 1);
        context.discard_u8(last.link().address())?;
        last.link().write(context, 0);
        let dma = OutgoingArguments::push(context, &[sprites.address(), 0xf800, 320])?;
        dma.call(
            context,
            dispatcher,
            ids::DMA_COPY_WORDS_TO_VRAM,
            site(0x29e),
        )?;
        let enabled = mode.read(context) | 0x40;
        mode.write(context, enabled);
        let command = 0x8104 | u16::from(mode.read(context));
        dma.release(context)?;
        context.write_vdp_control(command);
        if selected_piece >= 0 {
            let selected = OutgoingArguments::push(context, &[i32::from(selected_piece) as u32])?;
            selected.call(context, dispatcher, ids::SHOW_SHIP_PIECE_NAME, site(0x2d2))?;
            dispatcher.call_at(context, ids::ANIMATE_SHIP_SCREEN_PALETTE, &[], site(0x2d6))?;
            selected.release(context)?;
        } else if player_number != 2 {
            dispatcher.call_at(
                context,
                ids::SHOW_MISSING_SHIP_PIECE_SUMMARY,
                &[i32::from(player_number) as u32],
                site(0x2e8),
            )?;
        }
        ship_number.write(context, 0);
        loop {
            if levels
                .at(i32::from(ship_number.read(context)))
                .read(context)
                != 0
            {
                break;
            }
            let next = ship_number.read(context).wrapping_add(1);
            ship_number.write(context, next);
            if ship_number.read(context) >= 10 {
                break;
            }
        }
        if ship_number.read(context) < 10 {
            let acknowledgement =
                OutgoingArguments::push(context, &[i32::from(player_number) as u32])?;
            acknowledgement.call(
                context,
                dispatcher,
                ids::WAIT_FOR_SHIP_SCREEN_ACKNOWLEDGEMENT,
                site(0x318),
            )?;
            let blanked = mode.read(context) & 0x3c;
            mode.write(context, blanked);
            let command = 0x8104 | u16::from(mode.read(context));
            acknowledgement.release(context)?;
            context.write_vdp_control(command);
            dispatcher.call_at(context, ids::RESTORE_GAMEPLAY_GRAPHICS, &[], site(0x340))?;
            if ram::g_player_inventory_open(revision).at(0).read(context) != 0 {
                dispatcher.call_at(
                    context,
                    ids::INVENTORY_REDRAW_SYNCHRONOUSLY,
                    &[0],
                    site(0x350),
                )?;
            }
            if ram::g_player2_inventory_open(revision).read(context) != 0 {
                dispatcher.call_at(
                    context,
                    ids::INVENTORY_REDRAW_SYNCHRONOUSLY,
                    &[1],
                    site(0x364),
                )?;
            }
            if selected_piece >= 0 {
                let enabled = mode.read(context) | 0x40;
                mode.write(context, enabled);
                context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
                dispatcher.call_at(context, ids::ENABLE_BLANKING_GRAPHICS, &[], site(0x392))?;
            }
        } else {
            dispatcher.call_at(
                context,
                ids::PLAY_SHIP_COMPLETION_SEQUENCE,
                &[i32::from(player_number) as u32],
                site(0x39e),
            )?;
        }
        Ok(None)
    })();
    leave_frame(context, frame, result)
}
