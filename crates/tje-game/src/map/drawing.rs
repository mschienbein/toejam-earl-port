use crate::{Context, Result, call, ids};
use tje_mem::{Address, HardwareSprite, ram, rom};

/// G/src/map/map_drawing_and_controller.c:36 (`MapDrawFixedBackgroundDecorations`).
/// Functional contract: stable ordered writes, FIFO latency only, no status reads.
pub fn map_draw_fixed_background_decorations(
    c: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    for (command, value) in [
        (0x65820003, 0x458f),
        (0x6c020003, 0x458f),
        (0x6d820003, 0x458f),
        (0x65d00003, 0x4d8f),
        (0x6c500003, 0x4d8f),
        (0x6dd00003, 0x4d8f),
        (0x6e140003, 0x459b),
        (0x6e380003, 0x459b),
        (0x6e3e0003, 0x459b),
    ] {
        c.write_vdp_command(command);
        c.write_vdp_data(value);
    }
    Ok(None)
}
fn icon_command(tile: u16) -> u32 {
    let bytes = u32::from(tile) << 5;
    0x40000000 + ((bytes & 0x3fff) << 16) + ((bytes & 0xc000) >> 14)
}

/// G/src/map/map_drawing_and_controller.c:349 (`Map_DrawPlayer`).
pub fn map_draw_player(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let level = args[0] as i32;
    let sprites = Address::<HardwareSprite>::new(args[1]);
    for p in 0..2 {
        let position = ram::g_player_info(r).at(p);
        if call(c, ids::PLAYER_HAS_ODD_ELEVATOR_STATE, &[p as u32])? == 0
            && i32::from(position.level().read(c)) == level
        {
            let sprite = sprites.offset(p);
            let tile = sprite.tile().read(c);
            c.write_vdp_command(icon_command(tile));
            let icon = if p == 0 {
                rom::toejam_map_icon(r)
            } else {
                rom::earl_map_icon(r)
            };
            call(c, ids::MOVE_DATA_TO_VRAM, &[icon.address(), 16])?;
            sprite.tile().write(c, sprite.tile().read(c) | 0xe000);
            sprite.x().write(
                c,
                ((i32::from(position.x().read(c)) - 128) / 8 + 130) as i16,
            );
            sprite
                .y()
                .write(c, ((i32::from(position.y().read(c)) - 64) / 8 + 128) as i16);
        }
    }
    Ok(None)
}

/// G/src/map/map_drawing_and_controller.c:375 (`Map_DrawShipPiece`).
pub fn map_draw_ship_piece(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let level = args[0] as i16;
    let sprites = Address::<HardwareSprite>::new(args[1]);
    for bank in 0..2 {
        if ram::g_earthling_bank_users(r).at(bank).read(c) == 0
            || i16::from(ram::g_earthling_bank_levels(r).at(bank).read(c)) != level
        {
            continue;
        }
        let records = ram::g_earthling_banks(r).at(bank);
        let piece = if records.at(0).kind().read(c) == 0 {
            0
        } else if records.at(1).kind().read(c) == 0 {
            1
        } else {
            continue;
        };
        let x = records.at(piece).x().read(c);
        let y = records.at(piece).y().read(c);
        let tile = sprites.tile().read(c) & 0x7ff;
        c.write_vdp_command(icon_command(tile));
        call(
            c,
            ids::MOVE_DATA_TO_VRAM,
            &[rom::ship_piece_map_icon(r).address(), 64],
        )?;
        let x = ((i32::from(x) - 128) / 8 + 126) as i16;
        let y = ((i32::from(y) - 64) / 8 + 128) as i16;
        for piece in [0, 2] {
            let sprite = sprites.offset(piece);
            sprite.tile().write(c, sprite.tile().read(c) | 0x4000);
            sprite.size().write(c, 4);
            sprite
                .link()
                .write(c, sprite.link().read(c).wrapping_add(1));
            sprite.x().write(c, x);
            sprite.y().write(c, y);
        }
        return Ok(None);
    }
    Ok(None)
}

/// G/src/map/elevator_icon.c:12 (`Map_DrawElevator`).
pub fn map_draw_elevator(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let level = args[0] as i16;
    let sprite = Address::<HardwareSprite>::new(args[1]);
    for bank in 0..2 {
        let elevator = ram::g_elevator_records(r).at(bank).at(0);
        if i16::from(elevator.level().read(c)) != level || elevator.state().read(c) == 13 {
            continue;
        }
        let x = elevator.x().read(c).wrapping_sub(128);
        let y = elevator.y().read(c).wrapping_sub(64);
        c.write_vdp_command(icon_command(sprite.tile().read(c) & 0x7ff));
        call(
            c,
            ids::MOVE_DATA_TO_VRAM,
            &[rom::elevator_map_icon(r).address(), 16],
        )?;
        sprite.tile().write(c, sprite.tile().read(c) | 0x6000);
        sprite.x().write(c, (i32::from(x) / 8 + 130) as i16);
        sprite.y().write(c, (i32::from(y) / 8 + 128) as i16);
        return Ok(None);
    }
    Ok(None)
}

/// G/src/map/map_drawing_and_controller.c:410 (`DrawMapCharacters`).
pub fn draw_map_characters(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let sprites = Address::<HardwareSprite>::new(args[0]);
    call(c, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    if ram::g_frame_counter(c.revision()).read(c) & 24 != 0 {
        for p in 0..3 {
            c.write_vdp_command(0x78000003 + (p as u32) * 0x80000);
            c.write_vdp_data(sprites.offset(p).y().read(c) as u16);
        }
    } else {
        for p in 0..3 {
            c.write_vdp_command(0x78000003 + p * 0x80000);
            c.write_vdp_data(1);
        }
    }
    Ok(None)
}

/// G/src/map/map_drawing_and_controller.c:443 (`MapUploadCoverAnimationFrame`).
pub fn map_upload_cover_animation_frame(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let sprites = Address::<HardwareSprite>::new(args[0]);
    let number = i32::from(args[1] as i16);
    let first = args[2] as i16;
    let frame = rom::spr_map_tile_uncover(r)
        .at_word_index(number as i16)
        .read(c)
        .target();
    if number == 7 {
        for i in 7..80 {
            let sprite = sprites.offset(i);
            sprite.tile().write(c, sprite.tile().read(c) | 0x6000);
        }
    }
    for i in 0..2 {
        let piece = frame.piece(i);
        let flags = piece.tile_format().read(c);
        // Both matching listings load height (+1) before width (+0).
        let height = u32::from(piece.height().read(c));
        let count = (height * u32::from(piece.width().read(c))) as i16;
        call(
            c,
            if flags < 0 {
                ids::SPRITE_UNPACK_INTERLEAVED_RLE
            } else {
                ids::SPRITE_UNPACK_RLE
            },
            &[piece.address(), i32::from(count) as u32],
        )?;
        let offset = ram::g_sprite_tile_buffer_offset(r);
        offset.write(
            c,
            offset.read(c).wrapping_add((i32::from(count) << 5) as u16),
        );
    }
    let queue_index = ram::g_sprite_tile_upload_queue_word_count(r);
    for value in [if first != 0 { 0x122 } else { 0x136 }, 20] {
        let old = queue_index.read(c);
        queue_index.write(c, queue_index.read(c).wrapping_add(1));
        ram::g_sprite_tile_upload_runs(r)
            .at(i32::from(old))
            .write(c, value);
    }
    call(c, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    call(
        c,
        ids::DMA_COPY_WORDS_TO_VRAM,
        &[sprites.address(), 0xf800, 320],
    )?;
    call(c, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[1, 1])?;
    Ok(None)
}
