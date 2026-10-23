use crate::{Context, Result, call, ids};
use tje_mem::{RenderFrameFourPieceView, ram, rom};

/// G/src/cutscenes/title_and_transition.c:99 (`ClearLowerVramTilemap`).
/// TimeAware: waits for the IRQ-owned frame counter before the 2048-word fill.
pub fn clear_lower_vram_tilemap(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    clear_plane(context, 0x6000_0002)
}

/// G/src/cutscenes/title_and_transition.c:87 (`ClearUpperVramTilemap`).
/// TimeAware: waits for the IRQ-owned frame counter before the 2048-word fill.
pub fn clear_upper_vram_tilemap(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    clear_plane(context, 0x6000_0003)
}

fn clear_plane(context: &mut dyn Context, base: u32) -> Result<Option<u32>> {
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    for row in 0..32_u32 {
        context.write_vdp_command(base.wrapping_add(row << 23));
        for _ in 0..64 {
            context.write_vdp_data(0x8000);
        }
    }
    Ok(None)
}

/// G/src/cutscenes/title_and_transition.c:200 (`CutsceneDrawMetatileBlock`).
/// Functional: fixed ROM lookup and ordered writes, with no status/raster reads.
pub fn cutscene_draw_metatile_block(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let metatile = i32::from(arguments[0] as i16) as u32;
    let left = i32::from(arguments[1] as i16) as u32;
    let top = i32::from(arguments[2] as i16) as u32;
    for row in 0..8_u32 {
        context.write_vdp_command(
            0x6000_0002_u32
                .wrapping_add(top.wrapping_add(row).wrapping_shl(23))
                .wrapping_add(left.wrapping_shl(17)),
        );
        for column in 0..16 {
            let tile = call(
                context,
                ids::GET_LEVEL_METATILE_TILE,
                &[metatile, row, column],
            )?;
            context.write_vdp_data(tile.wrapping_add(0x8000) as u16);
        }
    }
    Ok(None)
}

/// G/src/cutscenes/title_and_transition.c:24 (`AnimateTitleTransitionBackground`).
/// TimeAware: even an unchanged animation waits for one real VBlank.
pub fn animate_title_transition_background(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut frame_phase = 0_i16;
    let timer = ram::g_scene_background_frame_timer(revision);
    if arguments[0] as i8 != 0 {
        frame_phase = 1;
        timer.write(context, 8);
    } else {
        let (next, expired) = decrement_background_timer(timer.read(context));
        timer.write(context, next);
        if expired {
            frame_phase = if ram::g_scene_background_frame_phase(revision).read(context) == 1 {
                2
            } else {
                1
            };
            timer.write(context, 8);
        }
    }
    if frame_phase != 0 {
        let mut frame_number =
            i16::from(ram::g_scene_background_pair_index(revision).read(context)).wrapping_mul(2);
        frame_number = frame_number.wrapping_add(frame_phase).wrapping_sub(1);
        let frame = rom::spr_transition_background(revision)
            .at(i32::from(frame_number))
            .read(context)
            .target()
            .cast::<RenderFrameFourPieceView>();
        for number in 0..4 {
            let part = frame.pieces().at(number);
            let decoder = if part.tile_format().read(context) < 0 {
                ids::SPRITE_UNPACK_INTERLEAVED_RLE
            } else {
                ids::SPRITE_UNPACK_RLE
            };
            call(context, decoder, &[part.address(), 16])?;
            let offset = ram::g_sprite_tile_buffer_offset(revision);
            let current = offset.read(context);
            offset.write(context, current.wrapping_add(512));
        }
        let count = ram::g_sprite_tile_upload_queue_word_count(revision);
        let slot = count.read(context);
        count.write(context, slot.wrapping_add(1));
        ram::g_sprite_tile_upload_runs(revision)
            .at(i32::from(slot))
            .write(context, 1);
        let slot = count.read(context);
        count.write(context, slot.wrapping_add(1));
        ram::g_sprite_tile_upload_runs(revision)
            .at(i32::from(slot))
            .write(context, 64);
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[1, 1])?;
        ram::g_scene_background_frame_phase(revision).write(context, frame_phase as i8);
        context.write_vdp_control(0x8228);
    } else {
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    }
    Ok(None)
}

// The matching SUBQ.B/BGT pair consumes V: -128 stores 127 but still expires.
fn decrement_background_timer(value: i8) -> (i8, bool) {
    let widened = i16::from(value) - 1;
    (widened as i8, widened <= 0)
}

/// G/src/cutscenes/title_and_transition.c:142 (`CutsceneDrawRandomizedScreenBorder`).
/// TimeAware: VBlank first, then source-ordered RNG calls and VDP writes.
pub fn cutscene_draw_randomized_screen_border(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    if arguments[0] as i16 != 0 {
        for row in 0..28_u32 {
            context.write_vdp_command(0x6000_0002_u32.wrapping_add(row << 23));
            for column in 0..40 {
                if row == 0 || row == 27 || column == 0 || column == 39 {
                    let mut tile = (call(context, ids::GET_RANDOM_NUMBER, &[])? & 3) as u16 + 41;
                    if call(context, ids::GET_RANDOM_NUMBER, &[])? & 1 != 0 {
                        tile |= 0x1800;
                    }
                    context.write_vdp_data(0x8580_u16.wrapping_add(tile));
                } else {
                    context.write_vdp_data(0x8000);
                }
            }
        }
    } else {
        for row in 0..28_u32 {
            for column in 0..40_u32 {
                if row == 0 || row == 27 || column == 0 || column == 39 {
                    let mut tile = (call(context, ids::GET_RANDOM_NUMBER, &[])? & 3) as u16 + 41;
                    context.write_vdp_command(
                        0x6000_0002_u32
                            .wrapping_add(row << 23)
                            .wrapping_add(column << 17),
                    );
                    if call(context, ids::GET_RANDOM_NUMBER, &[])? & 1 != 0 {
                        tile |= 0x1800;
                    }
                    context.write_vdp_data(0x8580_u16.wrapping_add(tile));
                }
            }
        }
    }
    Ok(None)
}

/// G/src/cutscenes/title_and_transition.c:178 (`CutsceneDrawRandomizedRectangleBorder`).
/// TimeAware: the inclusive signed-word loops follow a real VBlank wait.
pub fn cutscene_draw_randomized_rectangle_border(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let left = arguments[0] as i16;
    let top = arguments[1] as i16;
    let right = arguments[2] as i16;
    let bottom = arguments[3] as i16;
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let mut row = top;
    while row <= bottom {
        context.write_vdp_command(
            0x6000_0002_u32
                .wrapping_add((row as i32 as u32).wrapping_shl(23))
                .wrapping_add((left as i32 as u32).wrapping_shl(17)),
        );
        let mut column = left;
        while column <= right {
            if row == top || row == bottom || column == left || column == right {
                let mut tile = (call(context, ids::GET_RANDOM_NUMBER, &[])? & 3) as u16 + 41;
                if call(context, ids::GET_RANDOM_NUMBER, &[])? & 1 != 0 {
                    tile |= 0x1800;
                }
                context.write_vdp_data(0x8580_u16.wrapping_add(tile));
            } else {
                context.write_vdp_data(0x8244);
            }
            column = column.wrapping_add(1);
        }
        row = row.wrapping_add(1);
    }
    Ok(None)
}

/// G/src/cutscenes/title_and_transition.c:212 (`EndingDrawNineMetatileBackground`).
/// Functional: nine fixed stable-ROM metatile transaction streams.
pub fn ending_draw_nine_metatile_background(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    call(context, ids::CUTSCENE_DRAW_METATILE_BLOCK, &[1, 0, 0])?;
    call(context, ids::CUTSCENE_DRAW_METATILE_BLOCK, &[7, 16, 0])?;
    call(context, ids::CUTSCENE_DRAW_METATILE_BLOCK, &[2, 32, 0])?;
    call(context, ids::CUTSCENE_DRAW_METATILE_BLOCK, &[6, 0, 8])?;
    call(context, ids::CUTSCENE_DRAW_METATILE_BLOCK, &[30, 16, 8])?;
    call(context, ids::CUTSCENE_DRAW_METATILE_BLOCK, &[4, 32, 8])?;
    call(context, ids::CUTSCENE_DRAW_METATILE_BLOCK, &[0, 0, 16])?;
    call(context, ids::CUTSCENE_DRAW_METATILE_BLOCK, &[5, 16, 16])?;
    call(context, ids::CUTSCENE_DRAW_METATILE_BLOCK, &[3, 32, 16])?;
    Ok(None)
}

/// G/src/cutscenes/title_and_transition.c:226 (`EndingFillRepeatedMetatileBackground`).
/// Functional: fixed four-by-four stable-ROM metatile writes; column outermost.
pub fn ending_fill_repeated_metatile_background(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    for column in 0..4 {
        for row in 0..4 {
            call(
                context,
                ids::CUTSCENE_DRAW_METATILE_BLOCK,
                &[82, column * 16, row * 8],
            )?;
        }
    }
    Ok(None)
}

/// G/src/cutscenes/title_and_transition.c:111 (`TitleDrawSelectedLogo`).
/// TimeAware: first clears the upper plane after VBlank, then uploads the logo.
pub fn title_draw_selected_logo(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    call(context, ids::CLEAR_UPPER_VRAM_TILEMAP, &[])?;
    if ram::g_title_logo_selector(revision).read(context) == 0 {
        context.write_vdp_command(0x4880_0001);
        let count = i32::from(rom::logo_screen_size(revision).read(context)).wrapping_shl(4) as u32;
        call(
            context,
            ids::MOVE_DATA_TO_VRAM,
            &[rom::logo_screen(revision).address(), count],
        )?;
        for row in 1..26 {
            context.write_vdp_command(0x6000_0003_u32.wrapping_add((row as u32) << 23));
            for column in 0..40 {
                let tile = rom::text_mapping(revision)
                    .at(row - 1)
                    .at(column)
                    .read(context);
                context.write_vdp_data(tile.wrapping_add(0x8244));
            }
        }
    } else {
        context.write_vdp_command(0x4880_0001);
        let count =
            i32::from(rom::t_and_e_logo_screen_size(revision).read(context)).wrapping_shl(4) as u32;
        call(
            context,
            ids::MOVE_DATA_TO_VRAM,
            &[rom::t_and_e_logo_screen(revision).address(), count],
        )?;
        for row in 4..21 {
            context.write_vdp_command(0x6000_0003_u32.wrapping_add((row as u32) << 23));
            for column in 0..40 {
                let tile = rom::title_logo_tilemap(revision)
                    .at(row - 4)
                    .at(column)
                    .read(context);
                context.write_vdp_data(tile.wrapping_add(0x8244));
            }
        }
    }
    Ok(None)
}

/// G/src/cutscenes/title_and_transition.c:235 (`CutsceneReleaseFlaggedEntityRange`).
/// Atomic: preserves the ORIGINAL first-34-record versus handles-6..39 mismatch.
pub fn cutscene_release_flagged_entity_range(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for number in 6..40 {
        let object = ram::g_player_info(revision).at(number - 6);
        if object.draw_flags().read(context) & 1 != 0 {
            call(context, ids::FREE_ENTITY, &[number as u32])?;
        }
    }
    Ok(None)
}

/// G/src/cutscenes/title_and_transition.c:69 (`InitializeTitleTransitionTilemap`).
/// TimeAware: ordered tilemap writes precede the background's VBlank wait/upload.
pub fn initialize_title_transition_tilemap(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for band in 0..4_u32 {
        for row in 0..8_u32 {
            context.write_vdp_command(0x6000_0002_u32.wrapping_add((band * 8 + row) << 23));
            for _ in 0..8 {
                for column in 0..8 {
                    let tile = u16::from(
                        rom::transition_background_tile_offsets(revision)
                            .at(row as i32)
                            .at(column)
                            .read(context),
                    )
                    .wrapping_add(1);
                    let attributes = ram::g_camera_tile_attributes(revision).read(context);
                    context.write_vdp_data(attributes.wrapping_add(0x2000).wrapping_add(tile));
                }
            }
        }
    }
    call(context, ids::ANIMATE_TITLE_TRANSITION_BACKGROUND, &[1])?;
    Ok(None)
}

/// G/src/cutscenes/title_and_transition.c:245 (`CutsceneStepFrameAndMusicCadence`).
/// TimeAware: background, IRQ frame waits, sprite upload and music-event cadence.
pub fn cutscene_step_frame_and_music_cadence(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let advance_period = arguments[0] as u16;
    let animate_objects = arguments[1] as i16;
    call(context, ids::ANIMATE_TITLE_TRANSITION_BACKGROUND, &[0])?;
    ram::g_sprite_graphics_upload_pending(revision).write(context, 0);
    call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
    call(context, ids::SELECT_VIEWPORT_SPRITE_CHAIN, &[0])?;
    let updates = ram::g_scene_update_counter(revision);
    let current = updates.read(context);
    updates.write(context, current.wrapping_add(1));
    let event = ram::g_music_animation_event(revision);
    if event.read(context) == 0 {
        return Ok(Some(1));
    }
    event.write(context, 0);
    let counter = ram::g_frame_counter(revision);
    let last = ram::g_scene_last_music_event_time(revision);
    if u32::from(counter.read(context)).wrapping_sub(u32::from(last.read(context))) < 10 {
        return Ok(Some(1));
    }
    let current = counter.read(context);
    last.write(context, current);
    if animate_objects != 0 {
        call(
            context,
            ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
            &[
                2,
                rom::intro_frame_index_lists(revision).at(67).address(),
                6,
            ],
        )?;
    }
    if updates.read(context) > 51 {
        updates.write(context, 0);
        return Ok(Some(0));
    }
    if advance_period != 0 {
        let advance = ram::g_scene_background_advance_counter(revision);
        let current = advance.read(context);
        advance.write(context, current.wrapping_add(1));
        if advance_period > advance.read(context) {
            return Ok(Some(1));
        }
        advance.write(context, 0);
        let pair = ram::g_scene_background_pair_index(revision);
        let current = pair.read(context);
        pair.write(context, ((i32::from(current) + 1) % 24) as i8);
        call(context, ids::ANIMATE_TITLE_TRANSITION_BACKGROUND, &[1])?;
    }
    Ok(Some(1))
}

/// G/src/cutscenes/title_and_transition.c:282 (`TitleScreenVideo`).
/// TimeAware: real VBlank waits, music-event state and controller input timeline.
pub fn title_screen_video(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let mode = ram::g_mode2_reg_softcopy(revision);
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let current = mode.read(context);
    mode.write(context, current & 0x3c);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank0(revision).address(), 0],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank1(revision).address(), 1],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank2(revision).address(), 2],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank3(revision).address(), 3],
    )?;
    context.write_vdp_control(0x8c81);
    ram::g_camera_tile_attributes(revision).write(context, 0);
    call(context, ids::INITIALIZE_TITLE_TRANSITION_TILEMAP, &[])?;
    call(context, ids::TITLE_DRAW_SELECTED_LOGO, &[])?;
    ram::g_music_animation_channel(revision).write(context, 2);
    ram::g_scene_background_advance_counter(revision).write(context, 0);
    let mut start_pressed = 0_u16;
    let current = mode.read(context);
    mode.write(context, current | 0x40);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    let counter = ram::g_frame_counter(revision);
    let last_event = ram::g_scene_last_music_event_time(revision);
    let start_time = match revision {
        tje_mem::Revision::Rev02 => {
            counter.write(context, 0);
            last_event.write(context, 0);
            0
        }
        tje_mem::Revision::Rev00 => {
            let start = counter.read(context);
            let current = counter.read(context);
            last_event.write(context, current);
            start
        }
    };
    while match revision {
        tje_mem::Revision::Rev02 => counter.read(context) < 1200,
        tje_mem::Revision::Rev00 => {
            u32::from(counter.read(context)).wrapping_sub(u32::from(start_time)) < 1200
        }
    } {
        // The original retains MOVE.W D0,D0 for the otherwise-unused random word.
        call(context, ids::GET_RANDOM_NUMBER, &[])?;
        if call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[3, 0])? == 0 {
            let selector = ram::g_title_logo_selector(revision);
            let current = selector.read(context);
            selector.write(context, current ^ 1);
            call(context, ids::TITLE_DRAW_SELECTED_LOGO, &[])?;
        }
        // MCC68K evaluates this bitwise OR right-to-left in both revisions.
        let second = call(context, ids::GET_CTRL2_INPUT, &[])?;
        let buttons = call(context, ids::GET_CTRL1_INPUT, &[])? | second;
        if buttons & 0x80 != 0 {
            loop {
                let second = call(context, ids::GET_CTRL2_INPUT, &[])?;
                if (call(context, ids::GET_CTRL1_INPUT, &[])? | second) & 0x80 == 0 {
                    break;
                }
                call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
            }
            start_pressed = 1;
            break;
        }
    }
    let current = mode.read(context);
    mode.write(context, current & 0x3c);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    Ok(Some(u32::from(start_pressed)))
}

/// G/src/cutscenes/title_and_transition.c:337 (`SEGAAndIntroVideoRoutine`).
/// TimeAware: runs complete Sega/title/intro scenes against a controller timeline.
pub fn sega_and_intro_video_routine(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut show_intro = true;
    let service = ram::g_v_blank_audio_input_override(revision);
    service.write(context, 1);
    loop {
        call(context, ids::SEGA_VIDEO, &[])?;
        if call(context, ids::TITLE_SCREEN_VIDEO, &[])? as u16 != 0 {
            break;
        }
        if call(context, ids::INTRO_VIDEO, &[])? as i16 != 0 {
            show_intro = false;
            break;
        }
    }
    if show_intro {
        call(context, ids::INTRO_VIDEO, &[])?;
    }
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_game_line1(revision).address(), 0],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_game_line2(revision).address(), 1],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_game_line3(revision).address(), 2],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_game_line4(revision).address(), 3],
    )?;
    service.write(context, 0);
    context.write_vdp_control(0x8c89);
    Ok(None)
}

#[cfg(test)]
mod timer_tests {
    #[test]
    fn every_timer_byte_matches_subq_signed_condition_flags() {
        for old in 0..=255_u16 {
            let stored = (old as u8).wrapping_sub(1);
            let zero = stored == 0;
            let negative = stored & 0x80 != 0;
            let overflow = old == 0x80;
            let expired = zero || (negative != overflow);
            assert_eq!(
                super::decrement_background_timer(old as u8 as i8),
                (stored as i8, expired),
                "SUBQ.B/BGT input {old:#04x}",
            );
        }
    }
}
