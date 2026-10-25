use super::super::draws::literal;
use crate::{Context, Result, call, call_void, ids};
use tje_mem::{ram, rom};

/// G/src/ui/window_panels.c:107 (`DrawMenuWindowFrame`), both
/// c_window_panels.s:348–403. Functional: fixed border RNG draws and plane writes.
pub fn draw_menu_window_frame(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let filled = arguments[0] as i16;
    for row in 0_u32..28 {
        context.write_vdp_command(0x50000003_u32.wrapping_add(row << 23));
        for column in 0..40 {
            let tile = if row == 0 || row == 27 || column == 0 || column == 39 {
                let mut tile =
                    ((call(context, ids::GET_RANDOM_NUMBER, &[])? & 3) as u16).wrapping_add(41);
                if call(context, ids::GET_RANDOM_NUMBER, &[])? & 1 != 0 {
                    tile |= 0x1800;
                }
                0x8580_u16.wrapping_add(tile)
            } else if filled != 0 {
                0x85b4
            } else {
                0x8000
            };
            context.write_vdp_data(tile);
        }
    }
    context.write_vdp_control(0x911e);
    context.write_vdp_control(0x921c);
    Ok(None)
}

/// G/src/ui/hud.c:509 (`HudDrawRankLabels`), both c_hud.s:1181–1245.
/// Functional: stable ROM strings and sequential ordered glyph writes.
/// Embedded literals remain revision-resolved ROM views, never copied game text.
pub fn hud_draw_rank_labels(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let selection = ram::g_player_selection(revision).read(context);
    if selection == 2 {
        context.write_vdp_command(0x5c160003);
        let rank = i32::from(ram::g_player_rank(revision).at(0).read(context));
        let text = rom::rank_chars_table(revision)
            .at_long_index(rank)
            .read(context)
            .address();
        call_void(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
        context.write_vdp_command(0x5c400003);
        let rank = i32::from(ram::g_player_rank(revision).at(1).read(context));
        let text = rom::rank_chars_table(revision)
            .at_long_index(rank)
            .read(context)
            .address();
        call_void(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
    } else if selection == 0 {
        context.write_vdp_command(0x5c160003);
        let rank = i32::from(ram::g_player_rank(revision).at(0).read(context));
        let text = rom::rank_chars_table(revision)
            .at_long_index(rank)
            .read(context)
            .address();
        call_void(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
        context.write_vdp_command(0x5c3c0003);
        let text = literal(context, "OnVacationLine1Str1")?;
        call_void(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
        context.write_vdp_command(0x5cba0003);
        let text = literal(context, "OnVacationLine2Str1")?;
        call_void(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
    } else {
        context.write_vdp_command(0x5c400003);
        let rank = i32::from(ram::g_player_rank(revision).at(1).read(context));
        let text = rom::rank_chars_table(revision)
            .at_long_index(rank)
            .read(context)
            .address();
        call_void(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
        context.write_vdp_command(0x5c120003);
        let text = literal(context, "OnVacationLine1Str2")?;
        call_void(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
        context.write_vdp_command(0x5c900003);
        let text = literal(context, "OnVacationLine2Str2")?;
        call_void(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
    }
    Ok(None)
}
