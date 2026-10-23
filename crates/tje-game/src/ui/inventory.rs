use crate::{Context, Result};
use tje_mem::{ram, rom};

/// G/src/ui/inventory.c:32 (`ForceTextBackgroundColorToBlack`).
/// The long control-port write precedes the word-sized CRAM write.
pub fn force_text_background_color_to_black(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let force_black = arguments[0] as i16;
    context.write_vdp_command(0xc01e0000);
    context.write_vdp_data(if force_black != 0 { 0 } else { 0x200 });
    Ok(None)
}

/// G/src/ui/inventory.c:48 (`TextCopyFontBanksToRam`).
/// Each longword read is immediately followed by its destination write.
pub fn text_copy_font_banks_to_ram(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut destination = ram::g_text_font_buffers(revision).cast::<i32>();
    let mut source = rom::image_text_chars1(revision).cast::<i32>();
    for _ in 0..2 {
        for _ in 0..472 {
            let value = source.read(context);
            destination.write(context, value);
            source = source.offset(1);
            destination = destination.offset(1);
        }
        source = rom::image_text_chars2(revision).cast::<i32>();
    }
    Ok(None)
}

/// G/src/ui/inventory.c:238 (`InventorySetCameraSuspension`).
pub fn inventory_set_camera_suspension(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_split_screen_enabled(revision).read(context) == 0 {
        return Ok(None);
    }
    let suspend = arguments[0] != 0;
    if ram::g_player_inventory_open(revision).at(0).read(context) != 0 {
        if suspend {
            let x = ram::g_camera1_pending_x(revision).read(context);
            ram::g_inventory_saved_camera1_x(revision).write(context, x);
            let y = ram::g_camera1_pending_y(revision).read(context);
            ram::g_inventory_saved_camera1_y(revision).write(context, y);
            ram::g_camera1_pending_x(revision).write(context, 0);
            ram::g_camera1_pending_y(revision).write(context, 0);
        } else {
            let x = ram::g_inventory_saved_camera1_x(revision).read(context);
            ram::g_camera1_pending_x(revision).write(context, x);
            let y = ram::g_inventory_saved_camera1_y(revision).read(context);
            ram::g_camera1_pending_y(revision).write(context, y);
        }
    }
    if ram::g_player2_inventory_open(revision).read(context) != 0 {
        if suspend {
            let x = ram::g_camera2_pending_x(revision).read(context);
            ram::g_inventory_saved_camera2_x(revision).write(context, x);
            let y = ram::g_camera2_pending_y(revision).read(context);
            ram::g_inventory_saved_camera2_y(revision).write(context, y);
            ram::g_camera2_pending_x(revision).write(context, 0);
            ram::g_camera2_pending_y(revision).write(context, 112);
        } else {
            let x = ram::g_inventory_saved_camera2_x(revision).read(context);
            ram::g_camera2_pending_x(revision).write(context, x);
            let y = ram::g_inventory_saved_camera2_y(revision).read(context);
            ram::g_camera2_pending_y(revision).write(context, y);
        }
    }
    Ok(None)
}

/// G/src/ui/inventory.c:531 (`InventoryGetSelectedPresent`).
/// The computed slot narrows to signed char before indexing the inventory.
/// MCC68K explicitly sign-extends the returned char through the whole D0.
pub fn inventory_get_selected_present(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = arguments[0] as i32;
    let revision = context.revision();
    let row = i32::from(
        ram::g_player_inventory_selection_row(revision)
            .at(player)
            .read(context),
    );
    let scroll = i32::from(
        ram::g_player_inventory_scroll_row(revision)
            .at(player)
            .read(context),
    );
    let column = i32::from(
        ram::g_player_inventory_selection_column(revision)
            .at(player)
            .read(context),
    );
    let slot = row
        .wrapping_add(scroll)
        .wrapping_mul(2)
        .wrapping_add(column) as i8;
    let present = ram::g_player_inventory(revision)
        .at(player)
        .at(i32::from(slot))
        .read(context);
    Ok(Some(i32::from(present) as u32))
}

/// G/src/ui/inventory.c:645 (`TextMapCharacterToGlyph`).
/// Embedded controls write their preceding words before returning the final glyph.
/// The question-mark path increments a wrapping unsigned-word cache index.
pub fn text_map_character_to_glyph(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let character = arguments[0] as i8;
    let glyph = match character {
        97..=122 => character.wrapping_add(-96),
        65..=90 => character.wrapping_add(-64),
        48..=57 => character.wrapping_add(-12),
        63 => {
            let revision = context.revision();
            let index = ram::g_random_cache_index(revision);
            let next = if index.read(context) == 58 {
                index.write(context, 0);
                0
            } else {
                let incremented = index.read(context).wrapping_add(1);
                index.write(context, incremented);
                index.read(context)
            };
            // The matching compiler reads only the low byte used by the mask.
            let random = ram::g_cached_random_values(revision)
                .at(i32::from(next))
                .cast::<u8>()
                .byte_offset(1)
                .read(context);
            ((random & 3) + 27) as i8
        }
        46 => 31,
        44 => 32,
        58 | 39 => 33,
        45 => 34,
        33 => 35,
        1 => 46,
        2 => {
            context.write_vdp_data(0x85e3);
            context.write_vdp_data(0x85e4);
            49
        }
        3 => {
            context.write_vdp_data(0x85e6);
            51
        }
        4 => {
            context.write_vdp_data(0x85e8);
            53
        }
        5 => {
            context.write_vdp_data(0x85ea);
            context.write_vdp_data(0x85eb);
            56
        }
        6 => {
            context.write_vdp_data(0x85ed);
            58
        }
        _ => 0,
    };
    Ok(Some(i32::from(glyph) as u32))
}

/// G/src/ui/inventory.c:784 (`InventoryUpdateSelectorEntity`).
/// Unlike InventoryGetSelectedPresent, this expression keeps its full-width slot.
/// The retreat loop mutates the original signed-byte row/column storage in order.
pub fn inventory_update_selector_entity(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = i32::from(arguments[0] as i16);
    let revision = context.revision();
    let rows = ram::g_player_inventory_selection_row(revision).at(player);
    let columns = ram::g_player_inventory_selection_column(revision).at(player);
    let mut exhausted = false;
    if ram::g_player_inventory_open(revision)
        .at(player)
        .read(context)
        != 2
    {
        loop {
            let row = i32::from(rows.read(context));
            let scroll = i32::from(
                ram::g_player_inventory_scroll_row(revision)
                    .at(player)
                    .read(context),
            );
            let column = i32::from(columns.read(context));
            let slot = row
                .wrapping_add(scroll)
                .wrapping_mul(2)
                .wrapping_add(column);
            if ram::g_player_inventory(revision)
                .at(player)
                .at(slot)
                .read(context)
                != -1
            {
                break;
            }
            if rows.read(context) == 0 && columns.read(context) == 0 {
                exhausted = true;
                break;
            }
            if columns.read(context) > 0 {
                let column = columns.read(context).wrapping_sub(1);
                columns.write(context, column);
            } else {
                let row = rows.read(context).wrapping_sub(1);
                rows.write(context, row);
                let column = columns.read(context).wrapping_add(1);
                columns.write(context, column);
            }
        }
    }
    let selector = if player == 0 {
        ram::g_player1_inventory_selector_entity(revision)
    } else {
        ram::g_player2_inventory_selector_entity(revision)
    };
    if !exhausted {
        let x = i32::from(columns.read(context))
            .wrapping_mul(160)
            .wrapping_add(7) as i16;
        selector.x().write(context, x);
        let y = i32::from(rows.read(context))
            .wrapping_mul(16)
            .wrapping_add(63) as i16;
        selector.y().write(context, y);
        let flags = selector.behavior_flags().read(context);
        selector.behavior_flags().write(context, flags & !0x40);
        if player == 1 {
            let y = selector.y().read(context).wrapping_add(104);
            selector.y().write(context, y);
        }
    } else {
        let flags = selector.behavior_flags().read(context);
        selector.behavior_flags().write(context, flags | 0x40);
    }
    Ok(None)
}
