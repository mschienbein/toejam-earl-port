use crate::{Context, Result};
use tje_mem::{ram, rom};

/// G/src/player/active_present_falling_and_respawn.c:71 (`MapClearTilePriorityNearPlayer`).
/// Functional: bounded stable-source VRAM transactions; no raster/status observation.
pub(super) fn map_clear_tile_priority_near_player(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let number = ram::g_terrain_priority_update_player(revision).read(context);
    let player = super::indexing::player_by_word_index(revision, i32::from(number));
    let player_x = player.x().read(context);
    let player_y = player.y().read(context);
    let center_row = player_y >> 3;
    let top_row = center_row.wrapping_sub(6);
    let bottom_row = center_row.wrapping_add(6);
    let first_column = (player_x >> 3).wrapping_sub(4);
    let right_column = first_column.wrapping_add(9);
    let secondary = number == 1 && ram::g_split_screen_enabled(revision).read(context) != 0;
    context.write_vdp_control(0x8c81);
    let mut column = first_column;
    while column <= right_column {
        let mut row = top_row;
        while row <= bottom_row {
            let wrapped_row = (row & 31) as u32;
            let wrapped_column = (column & 63) as u32;
            let command = (wrapped_row << 23)
                .wrapping_add(if secondary { 3 } else { 0x2000_0002 })
                .wrapping_add(wrapped_column << 17);
            context.write_vdp_command(command);
            let mut tile = context.read_vdp_data();
            if row > center_row {
                let property = rom::player_tile_property_lookup(revision)
                    .at(i32::from(tile & 0x7ff))
                    .read(context);
                if property < 0x90 {
                    break;
                }
            }
            tile &= !0x8000;
            let command = (wrapped_row << 23)
                .wrapping_add(if secondary { 0x4000_0003 } else { 0x6000_0002 })
                .wrapping_add(wrapped_column << 17);
            context.write_vdp_command(command);
            context.write_vdp_data(tile);
            row = row.wrapping_add(1);
        }
        column = column.wrapping_add(1);
    }
    ram::g_terrain_priority_update_pending(revision).write(context, 0);
    Ok(None)
}
