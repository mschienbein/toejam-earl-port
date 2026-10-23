use super::draws::literal;
use crate::{Context, Result, call, ids};
use tje_mem::{Ptr, ram, rom};

/// G/src/ui/inventory.c:972 (`InventoryDrawActionBanner`).
pub fn inventory_draw_action_banner(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = arguments[0] as i16;
    let menu = arguments[1] as i16;
    let already_blanked = arguments[2] as i16;
    let base_row = if player == 0 { 1_i32 } else { 14_i32 };
    if already_blanked == 0 {
        call(context, ids::WAIT_FOR_V_BLANK_WITH_GRAPHICS_PAUSED, &[])?;
        call(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
        for row in 0_i32..3 {
            context.write_vdp_command(
                0x501c0003_u32.wrapping_add((base_row.wrapping_add(row) as u32) << 23),
            );
            for _ in 0..13 {
                context.write_vdp_data(0x824e);
            }
        }
    }
    let (column, width, source, tile_base) = match menu {
        0 => (
            14,
            13,
            rom::inventory_compact_map_a(revision).cast::<u8>(),
            0x8233_u16,
        ),
        1 => (
            14,
            13,
            rom::inventory_compact_map_b(revision).cast::<u8>(),
            0xe24e,
        ),
        2 => (
            15,
            11,
            rom::inventory_compact_map_c(revision).cast::<u8>(),
            0x8267,
        ),
        _ => (
            9,
            22,
            rom::inventory_compact_map_d(revision).cast::<u8>(),
            0xc1cd,
        ),
    };
    for row in 0_i32..3 {
        context.write_vdp_command(
            0x50000003_u32
                .wrapping_add((base_row.wrapping_add(row) as u32) << 23)
                .wrapping_add((column as u32) << 17),
        );
        for character in 0..width {
            let mut tile = u16::from(source.offset(row * width + character).read(context));
            if tile & 0x40 != 0 {
                tile |= 0x800;
            }
            tile &= !0xc0;
            context.write_vdp_data(tile_base.wrapping_add(tile));
        }
    }
    if already_blanked == 0 {
        call(context, ids::ENABLE_BLANKING_GRAPHICS, &[])?;
    }
    ram::g_inventory_banner_mode(revision)
        .at(i32::from(player))
        .write(context, menu as i8);
    Ok(None)
}

/// G/src/ui/inventory.c:878 (`LoadPresentsIntoInventory`).
pub fn load_presents_into_inventory(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    let (base_row, mut object) = if player == 0 {
        (5, ram::g_player1_inventory_item_entities(revision).at(0))
    } else {
        (18, ram::g_player2_inventory_item_entities(revision).at(0))
    };
    for row in 0_i32..3 {
        for column in 0_i32..2 {
            let scroll = i32::from(
                ram::g_player_inventory_scroll_row(revision)
                    .at(player)
                    .read(context),
            );
            let slot = (scroll
                .wrapping_mul(2)
                .wrapping_add(row * 2)
                .wrapping_add(column)) as i8;
            let present = ram::g_player_inventory(revision)
                .at(player)
                .at(i32::from(slot))
                .read(context);
            if present != -1 {
                let sprite = i32::from(
                    ram::g_present_info(revision)
                        .at(i32::from(present))
                        .appearance()
                        .read(context),
                );
                object.frames().write(
                    context,
                    Ptr::new(rom::present_sprite_table(revision).at(sprite).address()),
                );
                object.sprite_ready().write(context, 0);
                object
                    .y()
                    .write(context, ((row * 2 + base_row + 1) * 8) as i16);
                object.x().write(context, (column * 160 + 21) as i16);
                let flags = object.behavior_flags().read(context) & !0x40;
                object.behavior_flags().write(context, flags);
                object.frame().write(context, 0);
            }
            if ram::g_inventory_labels_suppressed(revision).read(context) == 0 {
                context.write_vdp_command(
                    0x50000003_u32
                        .wrapping_add(((row * 2 + base_row) as u32) << 23)
                        .wrapping_add(((column * 20 + 5) as u32) << 17),
                );
                if present != -1 {
                    if ram::g_present_info(revision)
                        .at(i32::from(present))
                        .identified()
                        .read(context)
                        != 0
                    {
                        let mut name = if present == 12
                            && ram::g_player_lives(revision).at(player ^ 1).read(context) < 0
                        {
                            rom::inventory_chars_un_fall(revision).at(0)
                        } else {
                            rom::inventory_chars_table(revision)
                                .at(i32::from(present))
                                .read(context)
                                .target()
                        };
                        for _ in 0..13 {
                            let glyph = i32::from(name.read(context));
                            name = name.offset(1);
                            context.write_vdp_data(0x85b4_i32.wrapping_add(glyph) as u16);
                        }
                    } else {
                        for _ in 0..5 {
                            let index = ram::g_random_cache_index(revision);
                            let next = if index.read(context) == 58 {
                                0
                            } else {
                                index.read(context).wrapping_add(1)
                            };
                            index.write(context, next);
                            let random = ram::g_cached_random_values(revision)
                                .at(i32::from(next))
                                .cast::<u8>()
                                .byte_offset(1)
                                .read(context);
                            let glyph = ((random & 3) + 27) as i8;
                            context
                                .write_vdp_data(0x85b4_i32.wrapping_add(i32::from(glyph)) as u16);
                        }
                        for _ in 5..13 {
                            context.write_vdp_data(0x85b4);
                        }
                    }
                } else {
                    let flags = object.behavior_flags().read(context) | 0x40;
                    object.behavior_flags().write(context, flags);
                    let text = literal(context, "GameText_00A606")?;
                    call(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
                }
            }
            object = object.offset(1);
        }
    }
    Ok(None)
}
