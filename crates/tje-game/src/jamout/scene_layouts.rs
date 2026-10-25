use super::frame_support::{
    add_word, clear_word, in_frame, release_arguments, retained_call_void, source_call_void,
    subtract_flags, word_flags,
};
use crate::call_frame::FrameLayout;
use crate::{Context, Result, ids};
use tje_mem::{ram, rom};

/// G/src/jamout/jamout.c:609 (`JamOutShowLeadPairLayout`, REV 02 0x03B8CA).
/// Hide nonlead copies, place Earl then ToeJam, run both time-aware tilemap
/// helpers, and read the packed 16-by-22 map from the selected revision's ROM.
pub fn show_lead_pair_layout(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    in_frame(context, FrameLayout::movem(0, 0x0c, 0), |context, _| {
        let revision = context.revision();
        let root = ids::JAM_OUT_SHOW_LEAD_PAIR_LAYOUT;
        for slot in 1..9 {
            retained_call_void(context, root, 0x10, ids::JAM_OUT_HIDE_TOEJAM_COPY, &[slot])?;
            retained_call_void(context, root, 0x1a, ids::JAM_OUT_HIDE_EARL_COPY, &[slot])?;
            release_arguments(context, 2)?;
        }
        retained_call_void(
            context,
            root,
            0x32,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[0, 174, 127],
        )?;
        retained_call_void(
            context,
            root,
            0x40,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[0, 144, 127],
        )?;
        source_call_void(
            context,
            root,
            0x46,
            ids::INITIALIZE_TITLE_TRANSITION_TILEMAP,
            &[],
        )?;
        source_call_void(context, root, 0x4c, ids::CLEAR_UPPER_VRAM_TILEMAP, &[])?;
        release_arguments(context, 6)?;
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
        // The last ADDQ.W/CMPI.W row test leaves X=0, Z=1, NVC=0.
        context.set_status_register((context.status_register() & !0x1f) | 4);
        Ok(None)
    })
}

/// G/src/jamout/jamout.c:654 (`JamOutShowFourPairLayout`, REV 02 0x03BA14).
/// Preserve all hide/position calls before clearing and the four background
/// uploads, including their original top-left, top-right, bottom-left order.
pub fn show_four_pair_layout(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    in_frame(context, FrameLayout::movem(0, 0x04, 0x1c), |context, _| {
        let root = ids::JAM_OUT_SHOW_FOUR_PAIR_LAYOUT;
        for slot in 4..9 {
            retained_call_void(context, root, 0x22, ids::JAM_OUT_HIDE_TOEJAM_COPY, &[slot])?;
            retained_call_void(context, root, 0x2c, ids::JAM_OUT_HIDE_EARL_COPY, &[slot])?;
            release_arguments(context, 2)?;
        }
        retained_call_void(
            context,
            root,
            0x42,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[0, 65, 70],
        )?;
        retained_call_void(
            context,
            root,
            0x4e,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[0, 95, 70],
        )?;
        retained_call_void(
            context,
            root,
            0x5c,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[1, 225, 70],
        )?;
        retained_call_void(
            context,
            root,
            0x6a,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[1, 255, 70],
        )?;
        retained_call_void(
            context,
            root,
            0x78,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[2, 65, 182],
        )?;
        retained_call_void(
            context,
            root,
            0x86,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[2, 95, 182],
        )?;
        retained_call_void(
            context,
            root,
            0x94,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[3, 225, 182],
        )?;
        retained_call_void(
            context,
            root,
            0xa2,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[3, 255, 182],
        )?;
        source_call_void(context, root, 0xa8, ids::CLEAR_UPPER_VRAM_TILEMAP, &[])?;
        retained_call_void(
            context,
            root,
            0xae,
            ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK,
            &[0, 0],
        )?;
        retained_call_void(
            context,
            root,
            0xb6,
            ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK,
            &[20, 0],
        )?;
        retained_call_void(
            context,
            root,
            0xbe,
            ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK,
            &[0, 14],
        )?;
        retained_call_void(
            context,
            root,
            0xc8,
            ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK,
            &[20, 14],
        )?;
        release_arguments(context, 32)?;
        Ok(None)
    })
}

/// G/src/jamout/jamout.c:677 (`JamOutShowNinePairLayout`, REV 02 0x03BAE6).
/// All eighteen position calls precede the clear and transition initialization.
pub fn show_nine_pair_layout(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    in_frame(context, FrameLayout::movem(0, 0, 0x0c), |context, _| {
        let root = ids::JAM_OUT_SHOW_NINE_PAIR_LAYOUT;
        retained_call_void(
            context,
            root,
            0x1c,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[0, 38, 53],
        )?;
        retained_call_void(
            context,
            root,
            0x28,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[0, 68, 53],
        )?;
        retained_call_void(
            context,
            root,
            0x36,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[1, 144, 53],
        )?;
        retained_call_void(
            context,
            root,
            0x44,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[1, 174, 53],
        )?;
        retained_call_void(
            context,
            root,
            0x52,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[2, 250, 53],
        )?;
        retained_call_void(
            context,
            root,
            0x60,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[2, 280, 53],
        )?;
        retained_call_void(
            context,
            root,
            0x6e,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[3, 38, 127],
        )?;
        retained_call_void(
            context,
            root,
            0x7c,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[3, 68, 127],
        )?;
        retained_call_void(
            context,
            root,
            0x8a,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[4, 144, 127],
        )?;
        retained_call_void(
            context,
            root,
            0x98,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[4, 174, 127],
        )?;
        retained_call_void(
            context,
            root,
            0xa6,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[5, 250, 127],
        )?;
        retained_call_void(
            context,
            root,
            0xb4,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[5, 280, 127],
        )?;
        retained_call_void(
            context,
            root,
            0xc2,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[6, 38, 201],
        )?;
        retained_call_void(
            context,
            root,
            0xd0,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[6, 68, 201],
        )?;
        retained_call_void(
            context,
            root,
            0xde,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[7, 144, 201],
        )?;
        retained_call_void(
            context,
            root,
            0xec,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[7, 174, 201],
        )?;
        retained_call_void(
            context,
            root,
            0xfa,
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            &[8, 250, 201],
        )?;
        retained_call_void(
            context,
            root,
            0x108,
            ids::JAM_OUT_POSITION_EARL_COPY,
            &[8, 280, 201],
        )?;
        source_call_void(context, root, 0x10e, ids::CLEAR_UPPER_VRAM_TILEMAP, &[])?;
        source_call_void(
            context,
            root,
            0x114,
            ids::INITIALIZE_TITLE_TRANSITION_TILEMAP,
            &[],
        )?;
        release_arguments(context, 54)?;
        Ok(None)
    })
}

/// G/src/jamout/jamout.c:352 (`JamOutUpdateLayoutsAndScroll`, REV 02 0x03B39E).
/// Preserve word-sized updates and the shared scroll-reset tail after layouts
/// zero/one. Invalid negative phases retain the original no-switch-action path.
pub fn update_layouts_and_scroll(context: &mut dyn Context) -> Result<Option<u32>> {
    in_frame(context, FrameLayout::movem(0, 0, 0x0c), |context, frame| {
        let reset = frame.argument_word(0).read(context);
        update_layouts(context, reset)
    })
}

fn update_layouts(context: &mut dyn Context, reset: i16) -> Result<Option<u32>> {
    let revision = context.revision();
    let phase = ram::g_jam_layout_phase(revision);
    let countdown = ram::g_jam_layout_countdown(revision);
    let horizontal = ram::g_jam_horizontal_scroll(revision);
    let vertical = ram::g_jam_vertical_scroll(revision);
    let step = ram::g_jam_scroll_step(revision);
    let root = ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL;
    if reset != 0 {
        clear_word(context, phase.address())?;
        countdown.write(context, 170);
        clear_word(context, horizontal.address())?;
        clear_word(context, vertical.address())?;
        clear_word(context, step.address())?;
        context.write_vdp_command(0x7c000003);
        context.write_vdp_data(horizontal.read(context) as u16);
        context.write_vdp_command(0x40000010);
        context.write_vdp_data(vertical.read(context) as u16);
        source_call_void(context, root, 0x5a, ids::JAM_OUT_SHOW_LEAD_PAIR_LAYOUT, &[])?;
    } else {
        if step.read(context) != 0 {
            let delta = step.read(context);
            let next = horizontal.read(context).wrapping_sub(delta);
            horizontal.write(context, next);
            let delta = step.read(context);
            let next = vertical.read(context).wrapping_add(delta);
            vertical.write(context, next);
        } else {
            clear_word(context, horizontal.address())?;
            clear_word(context, vertical.address())?;
        }
        context.write_vdp_command(0x7c000003);
        context.write_vdp_data(horizontal.read(context) as u16);
        context.write_vdp_command(0x40000010);
        context.write_vdp_data(vertical.read(context) as u16);
        let old = countdown.read(context);
        let next = subtract_flags(context, old as u16 as u32, 1, 16, true) as i16;
        countdown.write(context, next);
        if next == 0 {
            countdown.write(context, 170);
            let old = phase.read(context);
            let next = add_word(context, old as u16, 1) as i16;
            phase.write(context, next);
            if phase.read(context) >= 6 {
                clear_word(context, phase.address())?;
            }
            clear_word(
                context,
                ram::g_jam_toejam_animation_locked(revision).address(),
            )?;
            clear_word(
                context,
                ram::g_jam_earl_animation_locked(revision).address(),
            )?;
            let selected = phase.read(context);
            word_flags(context, selected as u16);
            if selected >= 0 {
                subtract_flags(context, selected as u16 as u32, 5, 16, false);
            }
            if (0..=5).contains(&selected) {
                add_word(context, selected as u16, selected as u16);
                // The compiled switch fetches one signed ROM table word.
                let table = root.metadata().address(revision) + 0xe2;
                let displacement = context.read_i16(table + 2 * selected as u32);
                word_flags(context, displacement as u16);
            }
            match selected {
                0 => {
                    source_call_void(context, root, 0xf2, ids::JAM_OUT_SHOW_LEAD_PAIR_LAYOUT, &[])?;
                    clear_word(context, step.address())?;
                }
                1 => {
                    source_call_void(context, root, 0xfa, ids::JAM_OUT_SHOW_FOUR_PAIR_LAYOUT, &[])?;
                    clear_word(context, step.address())?;
                }
                2 => {
                    source_call_void(
                        context,
                        root,
                        0x106,
                        ids::CUTSCENE_RELEASE_TEXT_ENTITIES,
                        &[],
                    )?;
                    source_call_void(
                        context,
                        root,
                        0x10a,
                        ids::JAM_OUT_SHOW_NINE_PAIR_LAYOUT,
                        &[],
                    )?;
                    countdown.write(context, 68);
                    word_flags(context, 68);
                }
                3 => {
                    step.write(context, 1);
                    countdown.write(context, 68);
                    word_flags(context, 68);
                }
                4 => {
                    step.write(context, -1);
                    countdown.write(context, 68);
                    word_flags(context, 68);
                }
                5 => {
                    clear_word(context, step.address())?;
                    countdown.write(context, 68);
                    word_flags(context, 68);
                }
                _ => {}
            }
        }
    }
    Ok(None)
}
