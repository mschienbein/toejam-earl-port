use super::super::animations::wait_upload;
use super::frame as source;
use crate::call_frame::{FrameLayout, LocalFrame};
use crate::{Context, Result, call_void, ids};
use tje_mem::{ram, rom};

/// G/src/ui/hud.c:197 (`HudInitializeHealthBar`), both c_hud.s:442–634.
/// TimeAware: the queued path waits for ISR upload; the immediate path animates.
/// Preserve unsigned buffer-clear/DMA offsets, signed pixel offsets and byte
/// queue postincrements, including their unchecked aliasing and wrap behavior.
pub fn hud_initialize_health_bar(context: &mut dyn Context) -> Result<Option<u32>> {
    // Both c_hud.s:443-446: LINK #-8 / MOVEM D2-D6/A2-A4, then two words.
    // Scalar locals stay real because unchecked signed pixel offsets can alias
    // this frame; no oracle may replace one of their causal stores.
    let frame = LocalFrame::enter(context, FrameLayout::link(8, 0x7c, 0x1c))?;
    let revision = context.revision();
    let player = i32::from(frame.argument_word(0).read(context));
    let upload_now = frame.argument_word(1).read(context);
    let displayed = ram::g_hud_displayed_health(revision).at_long_index(player);
    context.discard_u8(displayed.address())?;
    displayed.write(context, 0);
    let (tile, sprite, base_maximum) = if player == 0 {
        (0x7b4_u32, 15, 23_i16)
    } else {
        (0x7d4, 17, 31_i16)
    };
    let maximum = frame.local::<i16>(-8);
    let byte_offset = frame.local::<i16>(-6);
    let row = frame.local::<i16>(-4);
    let tile_row = frame.local::<i16>(-2);
    maximum.write(context, base_maximum);
    if ram::g_player_lives(revision)
        .at_long_index(player)
        .read(context)
        >= 0
    {
        let rank = ram::g_player_rank(revision)
            .at_long_index(player)
            .read(context);
        let value = maximum
            .read(context)
            .wrapping_add(i16::from(rank).wrapping_mul(4));
        maximum.write(context, value);
    }
    let sprites = ram::g_hardware_sprite_table(revision);
    sprites.at_word_index(sprite).size().write(context, 12);
    sprites
        .at_word_index(sprite)
        .tile()
        .write(context, (tile as u16).wrapping_add(0x8000));
    sprites
        .at_long_index(i32::from(sprite) + 1)
        .size()
        .write(context, 12);
    sprites
        .at_long_index(i32::from(sprite) + 1)
        .tile()
        .write(context, (tile as u16).wrapping_add(0x8004));
    let buffer = ram::g_sprite_tile_upload_buffer(revision).cast::<i8>();
    let offset = ram::g_sprite_tile_buffer_offset(revision);
    row.write(context, 0);
    loop {
        let buffer_offset = i32::from(offset.read(context));
        let index = buffer_offset.wrapping_add(i32::from(row.read(context)));
        let address = buffer.offset_long_index(index);
        context.discard_u8(address.address())?;
        address.write(context, 0);
        let next = row.read(context).wrapping_add(1);
        row.write(context, next);
        if row.read(context) >= 256 {
            break;
        }
    }
    let mut full = false;
    row.write(context, 0);
    while row.read(context) < 8 && !full {
        let mut unit = 0_i16;
        loop {
            let health_unit = row.read(context).wrapping_shl(3).wrapping_add(unit);
            if health_unit >= maximum.read(context) {
                full = true;
                break;
            }
            let pixels = if unit & 1 == 0 { 0xd0_u8 } else { 0x0d };
            let current_offset = offset.read(context) as i16;
            byte_offset.write(context, current_offset);
            let displacement = row.read(context).wrapping_mul(32).wrapping_add(unit / 2);
            let value = byte_offset.read(context).wrapping_add(displacement);
            byte_offset.write(context, value);
            tile_row.write(context, 0);
            loop {
                let index = i32::from(tile_row.read(context))
                    .wrapping_shl(2)
                    .wrapping_add(i32::from(byte_offset.read(context)));
                let address = buffer.offset_long_index(index);
                let value = address.read(context) as u8 | pixels;
                address.write(context, value as i8);
                let next = tile_row.read(context).wrapping_add(1);
                tile_row.write(context, next);
                if tile_row.read(context) >= 3 {
                    break;
                }
            }
            unit = unit.wrapping_add(1);
            if unit >= 8 {
                break;
            }
        }
        let next = row.read(context).wrapping_add(1);
        row.write(context, next);
    }
    if upload_now != 0 {
        // PEA count, MOVE.L destination, then load offset and PEA source.
        source::push_pea(context, 128)?;
        source::push_move(context, tile << 5)?;
        let pointer = buffer
            .offset_long_index(i32::from(offset.read(context)))
            .address();
        source::push_pea(context, pointer)?;
        source::call_void(
            context,
            ids::HUD_INITIALIZE_HEALTH_BAR,
            ids::DMA_COPY_WORDS_TO_VRAM,
            0x186,
        )?;
        source::release(context, 12)?;
        // c_hud.s:595-602 loads actual health before displayed health.
        while ram::g_player_health(revision)
            .at_long_index(player)
            .read(context)
            > displayed.read(context)
        {
            source::push_move(context, player as u32)?;
            source::call_void(
                context,
                ids::HUD_INITIALIZE_HEALTH_BAR,
                ids::HUD_ANIMATE_HEALTH_BAR,
                0x196,
            )?;
            source::release(context, 4)?;
        }
    } else {
        let next = offset.read(context).wrapping_add(256);
        offset.write(context, next);
        let count = ram::g_sprite_tile_upload_queue_word_count(revision);
        let runs = ram::g_sprite_tile_upload_runs(revision);
        let index = count.read(context);
        let incremented = count.read(context).wrapping_add(1);
        count.write(context, incremented);
        runs.at_word_index(i16::from(index))
            .write(context, tile as u16);
        let index = count.read(context);
        let incremented = count.read(context).wrapping_add(1);
        count.write(context, incremented);
        runs.at_word_index(i16::from(index)).write(context, 8);
        ram::g_sprite_graphics_upload_pending(revision).write(context, 1);
        wait_upload(context)?;
        context.discard_u8(displayed.address())?;
        displayed.write(context, 0);
    }
    frame.leave(context)?;
    Ok(None)
}

/// G/src/ui/hud.c:535 (`DrawHUD`), both c_hud.s:1263–1371.
/// Functional: stable background reads and finite ordered writes/sprite DMA.
pub fn draw_hud(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_player1_map_level(revision).read(context) == 26 {
        ram::g_hud_redraw_pending(revision).write(context, 0);
        return Ok(None);
    }
    let inventories = ram::g_player_inventory_open(revision);
    if ram::g_split_screen_enabled(revision).read(context) == 0
        && inventories.at(0).read(context) == 0
        && inventories.at(1).read(context) == 0
    {
        for row in 0_i16..5 {
            context.write_vdp_command(
                0x50000003_u32.wrapping_add(((i32::from(row) + 23) as u32) << 23),
            );
            if row < 4 {
                for column in 0_i16..40 {
                    let mut tile = u16::from(
                        rom::combined_hud_background_byte_map(revision)
                            .at_word_index(row)
                            .at_word_index(column)
                            .read(context),
                    );
                    if tile & 0x40 != 0 {
                        tile |= 0x800;
                    }
                    if tile & 0x80 != 0 {
                        tile |= 0x1000;
                    }
                    tile &= 0xff3f;
                    context.write_vdp_data(tile.wrapping_add(0x8580));
                }
            } else {
                for _ in 0..40 {
                    context.write_vdp_data(0xc580);
                }
            }
        }
        context.write_vdp_control(0x9297);
        call_void(context, ids::HUD_DRAW_RANK_LABELS, &[])?;
    } else {
        if inventories.at(0).read(context) == 0 && inventories.at(1).read(context) == 0 {
            context.write_vdp_control(0x9200);
        }
        call_void(context, ids::HUD_POSITION_LEVEL_NUMBER_SPRITES, &[1])?;
    }
    let player = if ram::g_player_lives(revision).at(0).read(context) >= 0 {
        0
    } else {
        1
    };
    call_void(context, ids::HUD_POSITION_LEVEL_NUMBER_SPRITES, &[player])?;
    call_void(context, ids::HUD_POSITION_ACTION_ICON_SPRITE, &[0])?;
    call_void(context, ids::HUD_POSITION_ACTION_ICON_SPRITE, &[1])?;
    call_void(context, ids::HUD_POSITION_HEALTH_BAR_SPRITES, &[0])?;
    call_void(context, ids::HUD_POSITION_HEALTH_BAR_SPRITES, &[1])?;
    call_void(context, ids::HUD_POSITION_LIVES_SPRITE, &[0])?;
    call_void(context, ids::HUD_POSITION_LIVES_SPRITE, &[1])?;
    call_void(
        context,
        ids::DMA_COPY_WORDS_TO_VRAM,
        &[
            ram::g_hud_hardware_sprite_bytes(revision).address(),
            0xf858,
            48,
        ],
    )?;
    ram::g_hud_redraw_pending(revision).write(context, 0);
    Ok(None)
}
