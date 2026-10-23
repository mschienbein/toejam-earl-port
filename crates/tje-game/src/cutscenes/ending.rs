use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/cutscenes/ending.c:405 (`EndingPlayDescendingRocketshipScene`).
/// TimeAware: the script advances through VBlank-driven background/sprite service.
pub fn ending_play_descending_rocketship_scene(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    call(context, ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER, &[1])?;
    call(context, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
    ram::g_cutscene_script_countdown(revision).write(context, 42);
    let mut character_y = 132_i16;
    let mut character_x = 280_i16;
    let mut move_timer = 20_i16;
    let mut move_period = move_timer;
    call(
        context,
        ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
        &[0, character_x as i32 as u32, character_y as i32 as u32],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::ending_palette_a(revision).address(), 3],
    )?;
    call(
        context,
        ids::ENDING_CREATE_STATIONARY_COMPOSITE,
        &[170, 180],
    )?;
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let current = mode.read(context);
    mode.write(context, current | 64);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    let state = ram::g_cutscene_script_state(revision);
    state.write(context, 0);
    while state.read(context) != -1 {
        call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
        match state.read(context) {
            0 => {
                character_x = character_x.wrapping_sub(1);
                if character_x == 182 {
                    let current = state.read(context);
                    state.write(context, current.wrapping_add(1));
                }
                call(
                    context,
                    ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
                    &[character_x as i32 as u32, character_y as i32 as u32],
                )?;
            }
            1 => {
                character_x = character_x.wrapping_sub(1);
                move_timer = move_timer.wrapping_sub(1);
                if move_timer == 0 {
                    move_period >>= 1;
                    if move_period == 0 {
                        move_period = 1;
                    }
                    move_timer = move_period;
                    character_y = character_y.wrapping_add(1);
                }
                call(
                    context,
                    ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
                    &[character_x as i32 as u32, character_y as i32 as u32],
                )?;
                if character_x == 122 {
                    let current = state.read(context);
                    state.write(context, current.wrapping_add(1));
                }
            }
            2 => {
                if character_y < 185 {
                    character_y = character_y.wrapping_add(1);
                } else {
                    move_period = move_period.wrapping_add(1);
                    if move_period & 1 != 0 {
                        character_y = character_y.wrapping_add(1);
                    }
                }
                call(
                    context,
                    ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
                    &[character_x as i32 as u32, character_y as i32 as u32],
                )?;
                if character_y >= 195 {
                    let current = state.read(context);
                    state.write(context, current.wrapping_add(1));
                }
            }
            3 => state.write(context, -1),
            _ => {}
        }
    }
    let current = mode.read(context);
    mode.write(context, current & 60);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    call(context, ids::ENDING_RELEASE_STATIONARY_COMPOSITE, &[])?;
    call(context, ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR, &[])?;
    call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
    Ok(Some(0))
}

/// G/src/cutscenes/ending.c:221 (`EndingPlayUpwardRocketshipDeparture`).
/// TimeAware: frame-driven script, paired-player text and accelerating motion.
/// Local literal pointers use _L105/_L106 relative to the matching exported
/// function label in c_ending.o; both revisions have offsets 0x1ec/0x1fa.
pub fn ending_play_upward_rocketship_departure(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let literal_base = ids::ENDING_PLAY_UPWARD_ROCKETSHIP_DEPARTURE
        .metadata()
        .address(revision);
    call(context, ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER, &[1])?;
    call(context, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
    ram::g_requested_music_selection(revision).write(context, 0);
    let mut character_y = 195_i16;
    let mut character_x = 112_i16;
    let mut move_timer = 15_i16;
    let mut move_period = move_timer;
    call(
        context,
        ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
        &[0, character_x as i32 as u32, character_y as i32 as u32],
    )?;
    call(context, ids::CUTSCENE_CREATE_PLANET_ENTITY, &[160, 180])?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank2(revision).address(), 2],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank3(revision).address(), 3],
    )?;
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let current = mode.read(context);
    mode.write(context, current | 64);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    state.write(context, 0);
    countdown.write(context, 17);
    while state.read(context) != -1 {
        call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
        match state.read(context) {
            0 => {
                let next = countdown.read(context).wrapping_sub(1);
                countdown.write(context, next);
                if next == 0 {
                    let current = state.read(context);
                    state.write(context, current.wrapping_add(1));
                }
            }
            1 => {
                if move_timer & 1 != 0 {
                    character_y = character_y.wrapping_sub(1);
                }
                move_timer = move_timer.wrapping_sub(1);
                if move_timer == 0 {
                    move_period >>= 1;
                    if move_period == 0 {
                        move_period = 1;
                    }
                    move_timer = move_period;
                    character_x = character_x.wrapping_sub(1);
                    if character_x == 107 && ram::g_player_selection(revision).read(context) == 2 {
                        call(
                            context,
                            ids::CUTSCENE_TEXT_LOAD,
                            &[
                                literal_base + 0x1ec,
                                (i32::from(character_x) + 70) as u32,
                                (i32::from(character_y) - 85) as u32,
                            ],
                        )?;
                    }
                }
                call(
                    context,
                    ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
                    &[character_x as i32 as u32, character_y as i32 as u32],
                )?;
                if character_y <= 132 {
                    let current = state.read(context);
                    state.write(context, current.wrapping_add(1));
                }
            }
            2 => {
                character_x = character_x.wrapping_sub(1);
                call(
                    context,
                    ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
                    &[character_x as i32 as u32, character_y as i32 as u32],
                )?;
                if character_x == 57 && ram::g_player_selection(revision).read(context) == 2 {
                    call(
                        context,
                        ids::CUTSCENE_TEXT_LOAD,
                        &[
                            literal_base + 0x1fa,
                            (i32::from(character_x) + 35) as u32,
                            (i32::from(character_y) - 80) as u32,
                        ],
                    )?;
                }
                if character_x <= -30 {
                    let current = state.read(context);
                    state.write(context, current.wrapping_add(1));
                }
            }
            3 => state.write(context, -1),
            _ => {}
        }
    }
    let current = mode.read(context);
    mode.write(context, current & 60);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    call(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    call(context, ids::CUTSCENE_RELEASE_PLANET_ENTITY, &[])?;
    call(context, ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR, &[])?;
    call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
    Ok(Some(0))
}

/// G/src/cutscenes/ending.c:287 (`EndingPlayScrollingCredits`).
/// TimeAware: two frame waits per iteration, sprite service and credit pacing.
/// Literal addresses are label metadata only: _L141.._L155 offsets relative to
/// EndingPlayScrollingCredits in the matching REV 02 and REV 00 c_ending.o.
pub fn ending_play_scrolling_credits(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let base = ids::ENDING_PLAY_SCROLLING_CREDITS
        .metadata()
        .address(revision);
    call(context, ids::CUTSCENE_CREATE_DRIFTING_ENTITIES, &[])?;
    call(context, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
    call(context, ids::DRAW_STARFIELD_PLANE, &[])?;
    call(context, ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER, &[1])?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank2(revision).address(), 2],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank3(revision).address(), 3],
    )?;
    call(
        context,
        ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
        &[0, 280, 160],
    )?;
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let current = mode.read(context);
    mode.write(context, current | 64);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    let countdown = ram::g_cutscene_script_countdown(revision);
    let state = ram::g_cutscene_script_state(revision);
    countdown.write(context, 85);
    state.write(context, 0);
    let mut scroll = 0_i16;
    let mut animation_phase = 0_i16;
    while state.read(context) != -1 {
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        context.write_vdp_command(0x7c02_0003);
        context.write_vdp_data((i32::from(scroll) / 4) as u16);
        call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
        call(context, ids::SELECT_VIEWPORT_SPRITE_CHAIN, &[0])?;
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        call(
            context,
            ids::CUTSCENE_UPDATE_DRIFTING_ENTITIES,
            &[scroll as i32 as u32],
        )?;
        animation_phase = ((i32::from(animation_phase) + 1) % 3) as i16;
        if animation_phase == 0 {
            call(context, ids::CUTSCENE_MOVE_ROCKETSHIP_PAIR_LEFT, &[1])?;
        }
        call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
        scroll = scroll.wrapping_add(1);
        let next = countdown.read(context).wrapping_sub(1);
        countdown.write(context, next);
        if next != 0 {
            continue;
        }
        countdown.write(context, 102);
        let next = state.read(context).wrapping_add(1);
        state.write(context, next);
        match next {
            1 => {
                credit_page(context, base + 0x308, base + 0x324, 48)?;
                countdown.write(context, 136);
            }
            2 | 4 | 6 | 8 | 10 | 12 | 14 | 16 => {
                call(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
                countdown.write(context, 8);
            }
            3 => credit_page(context, base + 0x344, base + 0x364, 45)?,
            5 => credit_page(context, base + 0x384, base + 0x3a4, 48)?,
            7 => credit_page(context, base + 0x3c4, base + 0x3dc, 45)?,
            9 => credit_page(context, base + 0x3f6, base + 0x412, 45)?,
            11 => credit_page(context, base + 0x42c, base + 0x44a, 45)?,
            13 => countdown.write(context, 8),
            15 => credit_page(context, base + 0x466, base + 0x482, 48)?,
            17 => {
                call(context, ids::CUTSCENE_TEXT_LOAD, &[base + 0x4a0, 160, 35])?;
                countdown.write(context, 136);
            }
            18 => state.write(context, -1),
            _ => {}
        }
    }
    let current = mode.read(context);
    mode.write(context, current & 60);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    call(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    call(context, ids::CUTSCENE_RELEASE_DRIFTING_ENTITIES, &[])?;
    call(context, ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR, &[])?;
    call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
    Ok(Some(0))
}

fn credit_page(context: &mut dyn Context, first: u32, second: u32, second_y: u32) -> Result<()> {
    call(context, ids::CUTSCENE_TEXT_LOAD, &[first, 160, 35])?;
    call(
        context,
        ids::CUTSCENE_LOAD_SECOND_TEXT_LINE,
        &[second, 160, second_y],
    )?;
    Ok(())
}

/// G/src/cutscenes/ending.c:68 (`EndingPlayDialogueAndDeparture`).
/// TimeAware: frame-paced dialogue, music events and accelerating ship motion.
/// _L61.._L64 in both matching c_ending.o files are function-relative
/// 0x40a/0x414/0x41e/0x42e; captions are read from the user's ROM, never copied.
pub fn ending_play_dialogue_and_departure(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let base = ids::ENDING_PLAY_DIALOGUE_AND_DEPARTURE
        .metadata()
        .address(revision);
    call(context, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
    call(context, ids::ENDING_CREATE_POSE_COMPOSITE, &[])?;
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
        &[rom::palette_space_screen_bank2(revision).address(), 2],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_game_line4(revision).address(), 3],
    )?;
    call(context, ids::CLEAR_LOWER_VRAM_TILEMAP, &[])?;
    call(context, ids::ENDING_DRAW_NINE_METATILE_BACKGROUND, &[])?;
    context.write_vdp_command(0x4000_0010);
    context.write_vdp_data((-30_i16) as u16);
    let mut character_y = 127_i16;
    let mut character_x = 92_i16;
    let mut speed = 0_i16;
    let mut clap_played = false;
    call(
        context,
        ids::CUTSCENE_SET_FOREGROUND_ROCKETSHIP_POSE,
        &[3, character_x as i32 as u32, character_y as i32 as u32],
    )?;
    call(
        context,
        ids::ENDING_SET_POSE_COMPOSITE,
        &[0, character_x as i32 as u32, 127],
    )?;
    call(
        context,
        ids::ENDING_CREATE_ACTIVE_PLAYER_ENTITIES,
        &[character_x as i32 as u32, 127],
    )?;
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let current = mode.read(context);
    mode.write(context, current | 64);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    let mut mouth_timer = 0_i16;
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    state.write(context, 0);
    countdown.write(context, 34);
    while state.read(context) != -1 {
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
        call(context, ids::SELECT_VIEWPORT_SPRITE_CHAIN, &[0])?;
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
        match state.read(context) {
            0 => {
                if decrement(context, countdown) == 0 {
                    let text = base
                        + if ram::g_player_selection(revision).read(context) == 2 {
                            0x40a
                        } else {
                            0x414
                        };
                    call(
                        context,
                        ids::CUTSCENE_LOAD_ORIGINAL_PALETTE_TEXT_LINE,
                        &[text, 90, 95],
                    )?;
                    countdown.write(context, 51);
                    increment(context, state);
                }
            }
            1 => {
                if decrement(context, countdown) == 0 {
                    call(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
                    call(
                        context,
                        ids::ENDING_CREATE_ELEVATED_THREE_PIECE_COMPOSITE,
                        &[character_x as i32 as u32, 127],
                    )?;
                    countdown.write(context, 323);
                    increment(context, state);
                }
            }
            2 => {
                if ram::g_player_selection(revision).read(context) == 2
                    && !clap_played
                    && call(context, ids::ENDING_GET_FIRST_PLAYER_ANIMATION_FRAME, &[])? == 28
                {
                    clap_played = true;
                    call(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_jam_clap(revision).address(), 90, 0],
                    )?;
                }
                if decrement(context, countdown) == 0 {
                    increment(context, state);
                    call(context, ids::ENDING_RELEASE_ACTIVE_PLAYER_ENTITIES, &[])?;
                    call(
                        context,
                        ids::ENDING_RESTART_ELEVATED_COMPOSITE_SEQUENCE,
                        &[],
                    )?;
                    countdown.write(context, 34);
                }
            }
            3 => {
                if decrement(context, countdown) == 0 {
                    increment(context, state);
                    countdown.write(context, 51);
                }
            }
            4 | 6 => {
                if decrement(context, countdown) == 0 {
                    increment(context, state);
                }
            }
            5 => {
                character_y = character_y.wrapping_sub(1);
                if character_y < 107 {
                    countdown.write(context, 17);
                    increment(context, state);
                }
                if character_y == 112 {
                    call(
                        context,
                        ids::ENDING_SET_POSE_COMPOSITE,
                        &[1, character_x as i32 as u32, 127],
                    )?;
                }
                call(
                    context,
                    ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
                    &[character_x as i32 as u32, character_y as i32 as u32],
                )?;
            }
            script_state @ (7 | 8) => {
                if script_state == 7 {
                    let text = base
                        + if ram::g_player_selection(revision).read(context) == 2 {
                            0x41e
                        } else {
                            0x42e
                        };
                    call(
                        context,
                        ids::CUTSCENE_LOAD_ORIGINAL_PALETTE_TEXT_LINE,
                        &[text, 160, 30],
                    )?;
                    countdown.write(context, 51);
                    increment(context, state);
                }
                // Case seven deliberately falls through and decrements immediately.
                if decrement(context, countdown) == 0 {
                    increment(context, state);
                }
            }
            9 => {
                call(
                    context,
                    ids::ENDING_RELEASE_ELEVATED_THREE_PIECE_COMPOSITE,
                    &[],
                )?;
                call(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
                call(context, ids::ENDING_CREATE_SIX_PIECE_COMPOSITE, &[])?;
                call(
                    context,
                    ids::ENDING_POSITION_SIX_PIECE_COMPOSITE,
                    &[character_x as i32 as u32, (-50_i32) as u32],
                )?;
                ram::g_requested_music_selection(revision).write(context, 6);
                countdown.write(context, 221);
                increment(context, state);
            }
            10 => {
                let event = ram::g_music_animation_event(revision);
                if event.read(context) != 0 {
                    call(
                        context,
                        ids::ENDING_POSITION_SIX_PIECE_COMPOSITE,
                        &[character_x as i32 as u32, character_y as i32 as u32],
                    )?;
                    mouth_timer = 4;
                    event.write(context, 0);
                }
                mouth_timer = mouth_timer.wrapping_sub(1);
                if mouth_timer == 0 {
                    call(
                        context,
                        ids::ENDING_POSITION_SIX_PIECE_COMPOSITE,
                        &[character_x as i32 as u32, (-50_i32) as u32],
                    )?;
                }
                if decrement(context, countdown) == 0 {
                    increment(context, state);
                    call(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_running(revision).address(), 50, 0],
                    )?;
                    call(context, ids::ENDING_RELEASE_SIX_PIECE_COMPOSITE, &[])?;
                }
            }
            11 => {
                character_x = character_x.wrapping_sub(speed);
                if character_x < -30 {
                    increment(context, state);
                }
                speed = speed.wrapping_add(1);
                call(
                    context,
                    ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
                    &[character_x as i32 as u32, character_y as i32 as u32],
                )?;
                call(
                    context,
                    ids::ENDING_SET_POSE_COMPOSITE,
                    &[1, character_x as i32 as u32, 127],
                )?;
            }
            12 => state.write(context, -1),
            _ => {}
        }
    }
    let current = mode.read(context);
    mode.write(context, current & 60);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    call(
        context,
        ids::ENDING_RELEASE_ELEVATED_THREE_PIECE_COMPOSITE,
        &[],
    )?;
    call(context, ids::ENDING_RELEASE_ACTIVE_PLAYER_ENTITIES, &[])?;
    call(context, ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR, &[])?;
    call(context, ids::ENDING_RELEASE_POSE_COMPOSITE, &[])?;
    call(context, ids::ENDING_RELEASE_SIX_PIECE_COMPOSITE, &[])?;
    call(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
    call(context, ids::CLEAR_LOWER_VRAM_TILEMAP, &[])?;
    speed = 0;
    context.write_vdp_command(0x4000_0010);
    context.write_vdp_data(speed as u16);
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
    Ok(Some(0))
}

fn decrement(context: &mut dyn Context, word: tje_mem::Address<i16>) -> i16 {
    let next = word.read(context).wrapping_sub(1);
    word.write(context, next);
    next
}

fn increment(context: &mut dyn Context, word: tje_mem::Address<i16>) {
    let next = word.read(context).wrapping_add(1);
    word.write(context, next);
}
