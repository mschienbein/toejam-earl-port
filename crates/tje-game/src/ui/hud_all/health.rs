use super::super::animations::wait_upload;
use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/ui/hud.c:197 (`HudInitializeHealthBar`), both c_hud.s:442–634.
/// TimeAware: the queued path waits for ISR upload; the immediate path animates.
/// Preserve unsigned buffer-clear/DMA offsets, signed pixel offsets and byte
/// queue postincrements, including their unchecked aliasing and wrap behavior.
pub fn hud_initialize_health_bar(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    let upload_now = arguments[1] as i16;
    ram::g_hud_displayed_health(revision)
        .at_long_index(player)
        .write(context, 0);
    let (tile, sprite, mut maximum) = if player == 0 {
        (0x7b4_u32, 15, 23_i16)
    } else {
        (0x7d4, 17, 31_i16)
    };
    if ram::g_player_lives(revision)
        .at_long_index(player)
        .read(context)
        >= 0
    {
        maximum = maximum.wrapping_add(
            i16::from(
                ram::g_player_rank(revision)
                    .at_long_index(player)
                    .read(context),
            )
            .wrapping_mul(4),
        );
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
    for row in 0..256 {
        let index = i32::from(offset.read(context)).wrapping_add(row);
        buffer.offset_long_index(index).write(context, 0);
    }
    let mut full = false;
    for row in 0_i16..8 {
        if full {
            break;
        }
        for unit in 0_i16..8 {
            let health_unit = row.wrapping_shl(3).wrapping_add(unit);
            if health_unit >= maximum {
                full = true;
                break;
            }
            let pixels = if unit & 1 == 0 { 0xd0_u8 } else { 0x0d };
            let byte_offset = (offset.read(context) as i16)
                .wrapping_add(row.wrapping_mul(32))
                .wrapping_add(unit / 2);
            for tile_row in 0_i32..3 {
                let address =
                    buffer.offset_long_index((tile_row << 2).wrapping_add(i32::from(byte_offset)));
                let value = address.read(context) as u8 | pixels;
                address.write(context, value as i8);
            }
        }
    }
    if upload_now != 0 {
        let source = buffer
            .offset_long_index(i32::from(offset.read(context)))
            .address();
        call(
            context,
            ids::DMA_COPY_WORDS_TO_VRAM,
            &[source, tile << 5, 128],
        )?;
        while ram::g_hud_displayed_health(revision)
            .at_long_index(player)
            .read(context)
            < ram::g_player_health(revision)
                .at_long_index(player)
                .read(context)
        {
            call(context, ids::HUD_ANIMATE_HEALTH_BAR, &[player as u32])?;
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
        ram::g_hud_displayed_health(revision)
            .at_long_index(player)
            .write(context, 0);
    }
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
        call(context, ids::HUD_DRAW_RANK_LABELS, &[])?;
    } else {
        if inventories.at(0).read(context) == 0 && inventories.at(1).read(context) == 0 {
            context.write_vdp_control(0x9200);
        }
        call(context, ids::HUD_POSITION_LEVEL_NUMBER_SPRITES, &[1])?;
    }
    let player = if ram::g_player_lives(revision).at(0).read(context) >= 0 {
        0
    } else {
        1
    };
    call(context, ids::HUD_POSITION_LEVEL_NUMBER_SPRITES, &[player])?;
    call(context, ids::HUD_POSITION_ACTION_ICON_SPRITE, &[0])?;
    call(context, ids::HUD_POSITION_ACTION_ICON_SPRITE, &[1])?;
    call(context, ids::HUD_POSITION_HEALTH_BAR_SPRITES, &[0])?;
    call(context, ids::HUD_POSITION_HEALTH_BAR_SPRITES, &[1])?;
    call(context, ids::HUD_POSITION_LIVES_SPRITE, &[0])?;
    call(context, ids::HUD_POSITION_LIVES_SPRITE, &[1])?;
    call(
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
