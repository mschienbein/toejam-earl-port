use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/jamout/jamout.c:609 (`JamOutShowLeadPairLayout`, REV 02 0x03B8CA).
/// Hide nonlead copies, place Earl then ToeJam, run both time-aware tilemap
/// helpers, and read the packed 16-by-22 map from the selected revision's ROM.
pub fn show_lead_pair_layout(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for slot in 1..9 {
        call(context, ids::JAM_OUT_HIDE_TOEJAM_COPY, &[slot])?;
        call(context, ids::JAM_OUT_HIDE_EARL_COPY, &[slot])?;
    }
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[0, 174, 127])?;
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[0, 144, 127])?;
    call(context, ids::INITIALIZE_TITLE_TRANSITION_TILEMAP, &[])?;
    call(context, ids::CLEAR_UPPER_VRAM_TILEMAP, &[])?;
    for row in 0..16 {
        context.write_vdp_command(0x60120003_u32.wrapping_add(((row + 6) as u32) << 23));
        for column in 0..22 {
            let mut tile = i16::from(
                rom::crash_packed_tilemap_a(revision)
                    .at(row)
                    .at(column)
                    .read(context),
            );
            if tile & 0x40 != 0 {
                tile |= 0x0800;
            }
            if tile & 0x80 != 0 {
                tile |= 0x1000;
            }
            tile = (tile as u16 & 0xff3f) as i16;
            context.write_vdp_data((i32::from(tile) + 0xa233) as u16);
        }
    }
    Ok(None)
}

/// G/src/jamout/jamout.c:654 (`JamOutShowFourPairLayout`, REV 02 0x03BA14).
/// Preserve all hide/position calls before clearing and the four background
/// uploads, including their original top-left, top-right, bottom-left order.
pub fn show_four_pair_layout(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    for slot in 4..9 {
        call(context, ids::JAM_OUT_HIDE_TOEJAM_COPY, &[slot])?;
        call(context, ids::JAM_OUT_HIDE_EARL_COPY, &[slot])?;
    }
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[0, 65, 70])?;
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[0, 95, 70])?;
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[1, 225, 70])?;
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[1, 255, 70])?;
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[2, 65, 182])?;
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[2, 95, 182])?;
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[3, 225, 182])?;
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[3, 255, 182])?;
    call(context, ids::CLEAR_UPPER_VRAM_TILEMAP, &[])?;
    call(context, ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK, &[0, 0])?;
    call(context, ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK, &[20, 0])?;
    call(context, ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK, &[0, 14])?;
    call(
        context,
        ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK,
        &[20, 14],
    )?;
    Ok(None)
}

/// G/src/jamout/jamout.c:677 (`JamOutShowNinePairLayout`, REV 02 0x03BAE6).
/// All eighteen position calls precede the clear and transition initialization.
pub fn show_nine_pair_layout(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[0, 38, 53])?;
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[0, 68, 53])?;
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[1, 144, 53])?;
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[1, 174, 53])?;
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[2, 250, 53])?;
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[2, 280, 53])?;
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[3, 38, 127])?;
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[3, 68, 127])?;
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[4, 144, 127])?;
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[4, 174, 127])?;
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[5, 250, 127])?;
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[5, 280, 127])?;
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[6, 38, 201])?;
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[6, 68, 201])?;
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[7, 144, 201])?;
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[7, 174, 201])?;
    call(context, ids::JAM_OUT_POSITION_TOEJAM_COPY, &[8, 250, 201])?;
    call(context, ids::JAM_OUT_POSITION_EARL_COPY, &[8, 280, 201])?;
    call(context, ids::CLEAR_UPPER_VRAM_TILEMAP, &[])?;
    call(context, ids::INITIALIZE_TITLE_TRANSITION_TILEMAP, &[])?;
    Ok(None)
}

/// G/src/jamout/jamout.c:352 (`JamOutUpdateLayoutsAndScroll`, REV 02 0x03B39E).
/// Preserve word-sized updates and the shared scroll-reset tail after layouts
/// zero/one. Invalid negative phases retain the original no-switch-action path.
pub fn update_layouts_and_scroll(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let phase = ram::g_jam_layout_phase(revision);
    let countdown = ram::g_jam_layout_countdown(revision);
    let horizontal = ram::g_jam_horizontal_scroll(revision);
    let vertical = ram::g_jam_vertical_scroll(revision);
    let step = ram::g_jam_scroll_step(revision);
    if arguments[0] as i16 != 0 {
        phase.write(context, 0);
        countdown.write(context, 170);
        horizontal.write(context, 0);
        vertical.write(context, 0);
        step.write(context, 0);
        context.write_vdp_command(0x7c000003);
        context.write_vdp_data(horizontal.read(context) as u16);
        context.write_vdp_command(0x40000010);
        context.write_vdp_data(vertical.read(context) as u16);
        call(context, ids::JAM_OUT_SHOW_LEAD_PAIR_LAYOUT, &[])?;
    } else {
        if step.read(context) != 0 {
            let next = horizontal.read(context).wrapping_sub(step.read(context));
            horizontal.write(context, next);
            let next = vertical.read(context).wrapping_add(step.read(context));
            vertical.write(context, next);
        } else {
            horizontal.write(context, 0);
            vertical.write(context, 0);
        }
        context.write_vdp_command(0x7c000003);
        context.write_vdp_data(horizontal.read(context) as u16);
        context.write_vdp_command(0x40000010);
        context.write_vdp_data(vertical.read(context) as u16);
        let next = countdown.read(context).wrapping_sub(1);
        countdown.write(context, next);
        if next == 0 {
            countdown.write(context, 170);
            let next = phase.read(context).wrapping_add(1);
            phase.write(context, next);
            if next >= 6 {
                phase.write(context, 0);
            }
            ram::g_jam_toejam_animation_locked(revision).write(context, 0);
            ram::g_jam_earl_animation_locked(revision).write(context, 0);
            match phase.read(context) {
                0 => {
                    call(context, ids::JAM_OUT_SHOW_LEAD_PAIR_LAYOUT, &[])?;
                    step.write(context, 0);
                }
                1 => {
                    call(context, ids::JAM_OUT_SHOW_FOUR_PAIR_LAYOUT, &[])?;
                    step.write(context, 0);
                }
                2 => {
                    call(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
                    call(context, ids::JAM_OUT_SHOW_NINE_PAIR_LAYOUT, &[])?;
                    countdown.write(context, 68);
                }
                3 => {
                    step.write(context, 1);
                    countdown.write(context, 68);
                }
                4 => {
                    step.write(context, -1);
                    countdown.write(context, 68);
                }
                5 => {
                    step.write(context, 0);
                    countdown.write(context, 68);
                }
                _ => {}
            }
        }
    }
    Ok(None)
}
