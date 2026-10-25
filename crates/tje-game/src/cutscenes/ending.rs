use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, call, call_void, ids};
use tje_mem::{ram, rom};

/// G/src/cutscenes/ending.c:405 (`EndingPlayDescendingRocketshipScene`).
/// TimeAware: the script advances through VBlank-driven background/sprite service.
pub fn ending_play_descending_rocketship_scene(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    // Its saved A2 low word becomes the following bouncing scene's residual
    // -2(A6) input in PlayEndingSequence; keep this predecessor frame as well.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x3c, 0x0c))?;
    let result = descending_body(context);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    if result.is_err() {
        context.set_stack_pointer(frame.save_base())?;
    }
    frame.leave(context)?;
    result
}

fn descending_body(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::ENDING_PLAY_DESCENDING_ROCKETSHIP_SCENE;
    context.set_address_register(2, ram::g_cutscene_script_state(revision).address());
    context.set_address_register(3, ram::g_mode2_reg_softcopy(revision).address());
    // c_ending.o:c64 returns at c6a; its argument remains through c6a's call.
    let border = retained_ending_call(
        context,
        caller,
        0xc6a - 0xc50,
        ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
        &[1],
    )?;
    call_void(context, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
    border.release(context)?;
    ram::g_cutscene_script_countdown(revision).write(context, 42);
    let mut character_y = 132_i16;
    let mut character_x = 280_i16;
    let mut move_timer = 20_i16;
    let mut move_period = move_timer;
    call_void(
        context,
        ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
        &[0, character_x as i32 as u32, character_y as i32 as u32],
    )?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::ending_palette_a(revision).address(), 3],
    )?;
    call_void(
        context,
        ids::ENDING_CREATE_STATIONARY_COMPOSITE,
        &[170, 180],
    )?;
    call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let current = mode.read(context);
    mode.write(context, current | 64);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    let state = ram::g_cutscene_script_state(revision);
    clear_word_instruction(context, state.address())?;
    while state.read(context) != -1 {
        let cadence = retained_ending_call(
            context,
            caller,
            0xcea - 0xc50,
            ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
            &[0, 0],
        )?;
        let phase = state.read(context);
        cadence.release(context)?;
        match phase {
            0 => {
                character_x = character_x.wrapping_sub(1);
                if character_x == 182 {
                    let current = state.read(context);
                    state.write(context, current.wrapping_add(1));
                }
                call_void(
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
                call_void(
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
                call_void(
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
    call_void(context, ids::ENDING_RELEASE_STATIONARY_COMPOSITE, &[])?;
    call_void(context, ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR, &[])?;
    framed_ending_call(
        context,
        caller,
        0xdcc - 0xc50,
        ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
        &[0, 0],
    )?;
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
    // Source MOVEM D2-D5/A2-A3 positions the prepared outgoing arguments.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x3c, 0x0c))?;
    let result = upward_body(context);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    if result.is_err() {
        context.set_stack_pointer(frame.save_base())?;
    }
    frame.leave(context)?;
    result
}

fn upward_body(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::ENDING_PLAY_UPWARD_ROCKETSHIP_DEPARTURE;
    let literal_base = ids::ENDING_PLAY_UPWARD_ROCKETSHIP_DEPARTURE
        .metadata()
        .address(revision);
    let border = retained_ending_call(
        context,
        caller,
        0x5ac - 0x592,
        ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
        &[1],
    )?;
    call_void(context, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
    border.release(context)?;
    clear_byte_instruction(
        context,
        ram::g_requested_music_selection(revision).address(),
    )?;
    let mut character_y = 195_i16;
    let mut character_x = 112_i16;
    let mut move_timer = 15_i16;
    let mut move_period = move_timer;
    call_void(
        context,
        ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
        &[0, character_x as i32 as u32, character_y as i32 as u32],
    )?;
    call_void(context, ids::CUTSCENE_CREATE_PLANET_ENTITY, &[160, 180])?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank2(revision).address(), 2],
    )?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank3(revision).address(), 3],
    )?;
    call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let current = mode.read(context);
    mode.write(context, current | 64);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    clear_word_instruction(context, state.address())?;
    countdown.write(context, 17);
    while state.read(context) != -1 {
        let cadence = retained_ending_call(
            context,
            caller,
            0x640 - 0x592,
            ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
            &[0, 0],
        )?;
        let phase = state.read(context);
        cadence.release(context)?;
        match phase {
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
                        call_void(
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
                call_void(
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
                call_void(
                    context,
                    ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
                    &[character_x as i32 as u32, character_y as i32 as u32],
                )?;
                if character_x == 57 && ram::g_player_selection(revision).read(context) == 2 {
                    call_void(
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
    call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    call_void(context, ids::CUTSCENE_RELEASE_PLANET_ENTITY, &[])?;
    call_void(context, ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR, &[])?;
    framed_ending_call(
        context,
        caller,
        0x770 - 0x592,
        ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
        &[0, 0],
    )?;
    Ok(Some(0))
}

/// G/src/cutscenes/ending.c:287 (`EndingPlayScrollingCredits`).
/// TimeAware: two frame waits per iteration, sprite service and credit pacing.
/// Literal addresses are label metadata only: _L141.._L155 offsets relative to
/// EndingPlayScrollingCredits in the matching REV 02 and REV 00 c_ending.o.
pub fn ending_play_scrolling_credits(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    // Source MOVEM D2-D3/A2-A4 positions the border/cadence entry slots.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x1c))?;
    let result = credits_body(context);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    if result.is_err() {
        context.set_stack_pointer(frame.save_base())?;
    }
    frame.leave(context)?;
    result
}

fn credits_body(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::ENDING_PLAY_SCROLLING_CREDITS;
    let base = ids::ENDING_PLAY_SCROLLING_CREDITS
        .metadata()
        .address(revision);
    call_void(context, ids::CUTSCENE_CREATE_DRIFTING_ENTITIES, &[])?;
    call_void(context, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
    call_void(context, ids::DRAW_STARFIELD_PLANE, &[])?;
    let border = retained_ending_call(
        context,
        caller,
        0x7c2 - 0x790,
        ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
        &[1],
    )?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank2(revision).address(), 2],
    )?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank3(revision).address(), 3],
    )?;
    call_void(
        context,
        ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
        &[0, 280, 160],
    )?;
    call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let current = mode.read(context);
    mode.write(context, current | 64);
    let control = u16::from(mode.read(context)) | 0x8104;
    border.release(context)?;
    context.write_vdp_control(control);
    let countdown = ram::g_cutscene_script_countdown(revision);
    let state = ram::g_cutscene_script_state(revision);
    countdown.write(context, 85);
    clear_word_instruction(context, state.address())?;
    let mut scroll = 0_i16;
    let mut animation_phase = 0_i16;
    while state.read(context) != -1 {
        call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        context.write_vdp_command(0x7c02_0003);
        context.write_vdp_data((i32::from(scroll) / 4) as u16);
        call_void(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
        call_void(context, ids::SELECT_VIEWPORT_SPRITE_CHAIN, &[0])?;
        call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        call_void(
            context,
            ids::CUTSCENE_UPDATE_DRIFTING_ENTITIES,
            &[scroll as i32 as u32],
        )?;
        animation_phase = ((i32::from(animation_phase) + 1) % 3) as i16;
        if animation_phase == 0 {
            call_void(context, ids::CUTSCENE_MOVE_ROCKETSHIP_PAIR_LEFT, &[1])?;
        }
        call_void(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
        scroll = scroll.wrapping_add(1);
        let next = countdown.read(context).wrapping_sub(1);
        countdown.write(context, next);
        if next != 0 {
            continue;
        }
        countdown.write(context, 102);
        let next = increment_word_and_reload(context, state);
        match next {
            1 => {
                credit_page(context, base + 0x308, base + 0x324, 48)?;
                countdown.write(context, 136);
            }
            2 | 4 | 6 | 8 | 10 | 12 | 14 | 16 => {
                call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
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
                call_void(context, ids::CUTSCENE_TEXT_LOAD, &[base + 0x4a0, 160, 35])?;
                countdown.write(context, 136);
            }
            18 => state.write(context, -1),
            _ => {}
        }
    }
    let current = mode.read(context);
    mode.write(context, current & 60);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    call_void(context, ids::CUTSCENE_RELEASE_DRIFTING_ENTITIES, &[])?;
    call_void(context, ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR, &[])?;
    framed_ending_call(
        context,
        caller,
        0xa8a - 0x790,
        ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
        &[0, 0],
    )?;
    Ok(Some(0))
}

fn credit_page(context: &mut dyn Context, first: u32, second: u32, second_y: u32) -> Result<()> {
    call_void(context, ids::CUTSCENE_TEXT_LOAD, &[first, 160, 35])?;
    call_void(
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
    // Source MOVEM D2-D6/A2-A4 positions the retained final cadence area.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x7c, 0x1c))?;
    let result = dialogue_body(context);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    if result.is_err() {
        context.set_stack_pointer(frame.save_base())?;
    }
    frame.leave(context)?;
    result
}

fn dialogue_body(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::ENDING_PLAY_DIALOGUE_AND_DEPARTURE;
    let base = ids::ENDING_PLAY_DIALOGUE_AND_DEPARTURE
        .metadata()
        .address(revision);
    call_void(context, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
    call_void(context, ids::ENDING_CREATE_POSE_COMPOSITE, &[])?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_game_line1(revision).address(), 0],
    )?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_game_line2(revision).address(), 1],
    )?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank2(revision).address(), 2],
    )?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_game_line4(revision).address(), 3],
    )?;
    call_void(context, ids::CLEAR_LOWER_VRAM_TILEMAP, &[])?;
    call_void(context, ids::ENDING_DRAW_NINE_METATILE_BACKGROUND, &[])?;
    context.write_vdp_command(0x4000_0010);
    context.write_vdp_data((-30_i16) as u16);
    let mut character_y = 127_i16;
    let mut character_x = 92_i16;
    let mut speed = 0_i16;
    let mut clap_played = false;
    call_void(
        context,
        ids::CUTSCENE_SET_FOREGROUND_ROCKETSHIP_POSE,
        &[3, character_x as i32 as u32, character_y as i32 as u32],
    )?;
    call_void(
        context,
        ids::ENDING_SET_POSE_COMPOSITE,
        &[0, character_x as i32 as u32, 127],
    )?;
    call_void(
        context,
        ids::ENDING_CREATE_ACTIVE_PLAYER_ENTITIES,
        &[character_x as i32 as u32, 127],
    )?;
    call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let current = mode.read(context);
    mode.write(context, current | 64);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    let mut mouth_timer = 0_i16;
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    clear_word_instruction(context, state.address())?;
    countdown.write(context, 34);
    while state.read(context) != -1 {
        call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        call_void(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
        call_void(context, ids::SELECT_VIEWPORT_SPRITE_CHAIN, &[0])?;
        call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        call_void(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
        match state.read(context) {
            0 => {
                if decrement(context, countdown) == 0 {
                    let text = base
                        + if ram::g_player_selection(revision).read(context) == 2 {
                            0x40a
                        } else {
                            0x414
                        };
                    call_void(
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
                    call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
                    call_void(
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
                    call_void(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_jam_clap(revision).address(), 90, 0],
                    )?;
                }
                if decrement(context, countdown) == 0 {
                    increment(context, state);
                    call_void(context, ids::ENDING_RELEASE_ACTIVE_PLAYER_ENTITIES, &[])?;
                    call_void(
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
                    call_void(
                        context,
                        ids::ENDING_SET_POSE_COMPOSITE,
                        &[1, character_x as i32 as u32, 127],
                    )?;
                }
                call_void(
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
                    call_void(
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
                call_void(
                    context,
                    ids::ENDING_RELEASE_ELEVATED_THREE_PIECE_COMPOSITE,
                    &[],
                )?;
                call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
                call_void(context, ids::ENDING_CREATE_SIX_PIECE_COMPOSITE, &[])?;
                call_void(
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
                    call_void(
                        context,
                        ids::ENDING_POSITION_SIX_PIECE_COMPOSITE,
                        &[character_x as i32 as u32, character_y as i32 as u32],
                    )?;
                    mouth_timer = 4;
                    clear_byte_instruction(context, event.address())?;
                }
                mouth_timer = mouth_timer.wrapping_sub(1);
                if mouth_timer == 0 {
                    call_void(
                        context,
                        ids::ENDING_POSITION_SIX_PIECE_COMPOSITE,
                        &[character_x as i32 as u32, (-50_i32) as u32],
                    )?;
                }
                if decrement(context, countdown) == 0 {
                    increment(context, state);
                    call_void(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_running(revision).address(), 50, 0],
                    )?;
                    call_void(context, ids::ENDING_RELEASE_SIX_PIECE_COMPOSITE, &[])?;
                }
            }
            11 => {
                character_x = character_x.wrapping_sub(speed);
                if character_x < -30 {
                    increment(context, state);
                }
                speed = speed.wrapping_add(1);
                call_void(
                    context,
                    ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
                    &[character_x as i32 as u32, character_y as i32 as u32],
                )?;
                call_void(
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
    call_void(
        context,
        ids::ENDING_RELEASE_ELEVATED_THREE_PIECE_COMPOSITE,
        &[],
    )?;
    call_void(context, ids::ENDING_RELEASE_ACTIVE_PLAYER_ENTITIES, &[])?;
    call_void(context, ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR, &[])?;
    call_void(context, ids::ENDING_RELEASE_POSE_COMPOSITE, &[])?;
    call_void(context, ids::ENDING_RELEASE_SIX_PIECE_COMPOSITE, &[])?;
    call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    let cadence = retained_ending_call(
        context,
        caller,
        0x522 - 0x158,
        ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
        &[0, 0],
    )?;
    framed_ending_call_void(
        context,
        caller,
        0x528 - 0x158,
        ids::CLEAR_LOWER_VRAM_TILEMAP,
        &[],
    )?;
    cadence.release(context)?;
    speed = 0;
    context.write_vdp_command(0x4000_0010);
    context.write_vdp_data(speed as u16);
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank0(revision).address(), 0],
    )?;
    call_void(
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

/// Matching c_ending.s:694-705,1184-1188: ADDQ.W followed by
/// a separate MOVE.W consumes a fresh word read, not a cached RMW result.
fn increment_word_and_reload<B: tje_mem::Bus + ?Sized>(
    context: &mut B,
    word: tje_mem::Address<i16>,
) -> i16 {
    let next = word.read(context).wrapping_add(1);
    word.write(context, next);
    word.read(context)
}

/// G/src/cutscenes/ending.c:505-508; matching c_ending.s:1193-1194.
/// Only the first ADDQ.W reads uninitialized -2(A6), i.e. entrySP-6.
/// The real store initializes that word; its reload and later RMWs are ordinary.
fn increment_movement_frame(
    context: &mut dyn Context,
    word: tje_mem::Address<i16>,
    first_update: &mut bool,
) -> Result<i16> {
    let previous = if *first_update {
        let frame = context
            .frame_state()
            .ok_or(crate::call_frame::FrameError::Unavailable)?;
        let read = frame.uninitialized_local(
            ids::ENDING_PLAY_BOUNCING_ROCKETSHIP_SCENE,
            -6,
            crate::stack_read::LocalWidth::Word,
            "G/src/cutscenes/ending.c:505-508; c_ending.s:1116-1117,1193-1194: uninitialized WORD -2(A6)=entrySP-6, first ADDQ.W RMW read",
        );
        context.read_uninitialized_local(read)? as i16
    } else {
        word.read(context)
    };
    word.write(context, previous.wrapping_add(1));
    *first_update = false;
    Ok(word.read(context))
}

/// Matching c_ending.s:8,334,440,1171 and c_funkotron_scene.s:277,289:
/// CLR.B performs a discarded byte read before the zero store.
pub(super) fn clear_byte_instruction<B: tje_mem::Bus + crate::Platform + ?Sized>(
    context: &mut B,
    address: u32,
) -> Result<()> {
    context.discard_u8(address)?;
    context.write_u8(address, 0);
    Ok(())
}

/// Matching c_ending.s:18-19,134,470,642,1000,1167: CLR.W uses one
/// discarded physical word read, never a semantic read or two byte reads.
fn clear_word_instruction<B: tje_mem::Bus + crate::Platform + ?Sized>(
    context: &mut B,
    address: u32,
) -> Result<()> {
    context.discard_u16(address)?;
    context.write_u16(address, 0);
    Ok(())
}

/// G/src/cutscenes/ending.c:466 (`EndingPlayBouncingRocketshipScene`).
/// TimeAware. Matching c_ending.s:1116-1117 is LINK #-4 then MOVEM
/// D2-D6/A2-A4. The ORIGINAL movement_frame word at -2(A6) is never initialized.
/// Return offsets below come from matching c_ending.o, whose REV00/02 assembly
/// sources are identical; the function starts at object-section offset 0xdda.
pub fn ending_play_bouncing_rocketship_scene(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0x7c, 0x1c))?;
    let result = bouncing_body(context, &frame);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    if result.is_err() {
        context.set_stack_pointer(frame.save_base())?;
    }
    frame.leave(context)?;
    result
}

fn bouncing_body(context: &mut dyn Context, frame: &LocalFrame) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::ENDING_PLAY_BOUNCING_ROCKETSHIP_SCENE;
    context.set_address_register(
        3,
        ids::COPY_PALETTE_TO_CRAM_SLOT.metadata().address(revision),
    );
    context.set_address_register(4, ram::g_mode2_reg_softcopy(revision).address());
    context.set_address_register(
        2,
        ids::ENDING_SET_POSE_COMPOSITE.metadata().address(revision),
    );
    framed_ending_call_void(
        context,
        caller,
        0xdfa - 0xdda,
        ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR,
        &[],
    )?;
    framed_ending_call_void(
        context,
        caller,
        0xe00 - 0xdda,
        ids::ENDING_CREATE_POSE_COMPOSITE,
        &[],
    )?;
    framed_ending_call_void(
        context,
        caller,
        0xe06 - 0xdda,
        ids::ENDING_CREATE_RIGHT_OFFSET_COMPOSITE,
        &[],
    )?;
    let palettes = [
        retained_ending_call(
            context,
            caller,
            0xe10 - 0xdda,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            &[rom::palette_game_line1(revision).address(), 0],
        )?,
        retained_ending_call(
            context,
            caller,
            0xe1c - 0xdda,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            &[rom::palette_game_line2(revision).address(), 1],
        )?,
        retained_ending_call(
            context,
            caller,
            0xe28 - 0xdda,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            &[rom::palette_space_screen_bank2(revision).address(), 2],
        )?,
        retained_ending_call(
            context,
            caller,
            0xe34 - 0xdda,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            &[rom::ending_palette_b(revision).address(), 3],
        )?,
    ];
    framed_ending_call_void(
        context,
        caller,
        0xe3a - 0xdda,
        ids::ENDING_FILL_REPEATED_METATILE_BACKGROUND,
        &[],
    )?;
    for arguments in palettes.into_iter().rev() {
        arguments.release(context)?;
    }
    let mut character_y = 72_i16;
    let mut character_x = 152_i16;
    let mut scroll = 0_i16;
    context.set_data_register(2, 72);
    set_word_register(context, 3, character_x);
    set_word_register(context, 6, scroll);
    context.write_vdp_command(0x7c00_0003);
    context.write_vdp_data(scroll as u16);
    let mut velocity_y = 0_i16;
    let mut acceleration_y = 1_i16;
    set_word_register(context, 4, velocity_y);
    context.set_data_register(5, 1);
    let ship = retained_ending_call(
        context,
        caller,
        0xe70 - 0xdda,
        ids::CUTSCENE_SET_FOREGROUND_ROCKETSHIP_POSE,
        &[3, character_x as i32 as u32, character_y as i32 as u32],
    )?;
    let pose = retained_ending_call(
        context,
        caller,
        0xe80 - 0xdda,
        ids::ENDING_SET_POSE_COMPOSITE,
        &[1, character_x as i32 as u32, 127],
    )?;
    let wait = retained_ending_call(
        context,
        caller,
        0xe8a - 0xdda,
        ids::WAIT_FOR_NEXT_V_BLANK,
        &[1],
    )?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let current = mode.read(context);
    mode.write(context, current | 64);
    wait.release(context)?;
    pose.release(context)?;
    ship.release(context)?;
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    let state = ram::g_cutscene_script_state(revision);
    clear_word_instruction(context, state.address())?;
    ram::g_cutscene_script_countdown(revision).write(context, 17);
    let movement_frame = frame.local::<i16>(-2);
    let mut first_movement_update = true;
    while state.read(context) != -1 {
        clear_byte_instruction(
            context,
            ram::g_sprite_graphics_upload_pending(revision).address(),
        )?;
        framed_ending_call_void(
            context,
            caller,
            0xec0 - 0xdda,
            ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
            &[],
        )?;
        let wait = retained_ending_call(
            context,
            caller,
            0xeca - 0xdda,
            ids::WAIT_FOR_NEXT_V_BLANK,
            &[1],
        )?;
        let upload = retained_ending_call(
            context,
            caller,
            0xed4 - 0xdda,
            ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
            &[0, 0],
        )?;
        upload.release(context)?;
        wait.release(context)?;
        context.write_vdp_command(0x7c00_0003);
        context.write_vdp_data(scroll as u16);
        let viewport = retained_ending_call(
            context,
            caller,
            0xef0 - 0xdda,
            ids::SELECT_VIEWPORT_SPRITE_CHAIN,
            &[0],
        )?;
        scroll = scroll.wrapping_add(5);
        set_word_register(context, 6, scroll);
        let next_state = increment_word_and_reload(context, state);
        viewport.release(context)?;
        if next_state & 1 != 0 {
            character_x = character_x.wrapping_sub(1);
            set_word_register(context, 3, character_x);
        }
        character_y = character_y.wrapping_add(velocity_y);
        set_word_register(context, 2, character_y);
        // Only the first RMW input is residual; never initialize or waive it.
        let phase = increment_movement_frame(context, movement_frame, &mut first_movement_update)?;
        if i32::from(phase) % 9 == 0 {
            velocity_y = velocity_y.wrapping_add(acceleration_y);
            set_word_register(context, 4, velocity_y);
        }
        if character_y >= 137 {
            if velocity_y == 0 {
                acceleration_y = 0;
                set_word_register(context, 5, acceleration_y);
            } else {
                velocity_y = (1_i32 - i32::from(velocity_y)) as i16;
                set_word_register(context, 4, velocity_y);
            }
        }
        if character_y >= 127 {
            let ship = retained_ending_call(
                context,
                caller,
                0xf5a - 0xdda,
                ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
                &[character_x as i32 as u32, 127],
            )?;
            let companion = retained_ending_call(
                context,
                caller,
                0xf6a - 0xdda,
                ids::ENDING_POSITION_RIGHT_OFFSET_COMPOSITE,
                &[character_x as i32 as u32, 127],
            )?;
            let pose = retained_ending_call(
                context,
                caller,
                0xf78 - 0xdda,
                ids::ENDING_SET_POSE_COMPOSITE,
                &[0, character_x as i32 as u32, 127],
            )?;
            pose.release(context)?;
            companion.release(context)?;
            ship.release(context)?;
        } else {
            let ship = retained_ending_call(
                context,
                caller,
                0xf90 - 0xdda,
                ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR,
                &[character_x as i32 as u32, character_y as i32 as u32],
            )?;
            let companion = retained_ending_call(
                context,
                caller,
                0xfa0 - 0xdda,
                ids::ENDING_POSITION_RIGHT_OFFSET_COMPOSITE,
                &[character_x as i32 as u32, (-50_i32) as u32],
            )?;
            companion.release(context)?;
            ship.release(context)?;
            framed_ending_call_void(
                context,
                caller,
                0xfc8 - 0xdda,
                ids::ENDING_SET_POSE_COMPOSITE,
                &[u32::from(character_y < 107), character_x as i32 as u32, 127],
            )?;
        }
        if character_x < -30 {
            state.write(context, -1);
        }
    }
    let current = mode.read(context);
    mode.write(context, current & 60);
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    framed_ending_call_void(
        context,
        caller,
        0x1000 - 0xdda,
        ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR,
        &[],
    )?;
    framed_ending_call_void(
        context,
        caller,
        0x1006 - 0xdda,
        ids::ENDING_RELEASE_POSE_COMPOSITE,
        &[],
    )?;
    framed_ending_call_void(
        context,
        caller,
        0x100c - 0xdda,
        ids::ENDING_RELEASE_RIGHT_OFFSET_COMPOSITE,
        &[],
    )?;
    let cadence = retained_ending_call(
        context,
        caller,
        0x1016 - 0xdda,
        ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
        &[0, 0],
    )?;
    framed_ending_call_void(
        context,
        caller,
        0x101c - 0xdda,
        ids::CLEAR_LOWER_VRAM_TILEMAP,
        &[],
    )?;
    cadence.release(context)?;
    scroll = 0;
    set_word_register(context, 6, scroll);
    context.write_vdp_command(0x7c00_0003);
    context.write_vdp_data(scroll as u16);
    Ok(Some(0))
}

/// G/src/cutscenes/ending.c:28 (`PlayEndingSequence`).
/// TimeAware; exact MOVEM A2-A4 frame and retained 40-byte outgoing area.
/// Callsite offsets are matching c_ending.o return PCs, identical in both revisions.
pub fn play_ending_sequence(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0, 0x1c))?;
    let result = ending_sequence_body(context);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    if result.is_err() {
        context.set_stack_pointer(frame.save_base())?;
    }
    frame.leave(context)?;
    result
}

fn ending_sequence_body(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::PLAY_ENDING_SEQUENCE;
    let x = ram::g_camera1_pending_x(revision);
    let y = ram::g_camera1_pending_y(revision);
    context.set_address_register(
        2,
        ids::COPY_PALETTE_TO_CRAM_SLOT.metadata().address(revision),
    );
    context.set_address_register(3, y.address());
    context.set_address_register(4, x.address());
    clear_byte_instruction(
        context,
        ram::g_requested_music_selection(revision).address(),
    )?;
    let wait = retained_ending_call(context, caller, 0x26, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let current = mode.read(context);
    mode.write(context, current & 60);
    wait.release(context)?;
    context.write_vdp_control(u16::from(mode.read(context)) | 0x8104);
    framed_ending_call_void(context, caller, 0x4a, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
    clear_word_instruction(context, x.address())?;
    clear_word_instruction(context, y.address())?;
    context.write_vdp_control(0x8228);
    context.write_vdp_command(0x4000_0010);
    context.write_vdp_data(y.read(context) as u16);
    context.write_vdp_command(0x7c00_0003);
    context.write_vdp_data(x.read(context) as u16);
    context.write_vdp_command(0x4002_0010);
    context.write_vdp_data(y.read(context) as u16);
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data(x.read(context) as u16);
    framed_ending_call_void(context, caller, 0x9c, ids::CLEAR_UPPER_VRAM_TILEMAP, &[])?;
    ram::g_camera_tile_attributes(revision).write(context, 0x8000);
    context.write_vdp_command(0x7000_0002);
    let words = i32::from(rom::font_data_size(revision).read(context)).wrapping_shl(4) as u32;
    framed_ending_call_void(
        context,
        caller,
        0xc6,
        ids::MOVE_DATA_TO_VRAM,
        &[rom::image_text_base(revision).address(), words],
    )?;
    context.write_vdp_command(0x4000_0000);
    let words = i32::from(rom::level_tiles_size(revision).read(context)).wrapping_shl(4) as u32;
    let level = retained_ending_call(
        context,
        caller,
        0xea,
        ids::MOVE_DATA_TO_VRAM,
        &[rom::level_tiles(revision).address(), words],
    )?;
    let palettes = [
        retained_ending_call(
            context,
            caller,
            0xf4,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            &[rom::palette_title_screen_bank0(revision).address(), 0],
        )?,
        retained_ending_call(
            context,
            caller,
            0x100,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            &[rom::palette_title_screen_bank1(revision).address(), 1],
        )?,
        retained_ending_call(
            context,
            caller,
            0x10c,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            &[rom::palette_title_screen_bank2(revision).address(), 2],
        )?,
        retained_ending_call(
            context,
            caller,
            0x118,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            &[rom::palette_title_screen_bank3(revision).address(), 3],
        )?,
    ];
    framed_ending_call_void(context, caller, 0x11e, ids::DRAW_STARFIELD_PLANE, &[])?;
    framed_ending_call(
        context,
        caller,
        0x122,
        ids::ENDING_PLAY_DIALOGUE_AND_DEPARTURE,
        &[],
    )?;
    framed_ending_call(
        context,
        caller,
        0x128,
        ids::ENDING_PLAY_UPWARD_ROCKETSHIP_DEPARTURE,
        &[],
    )?;
    framed_ending_call(
        context,
        caller,
        0x12e,
        ids::ENDING_PLAY_SCROLLING_CREDITS,
        &[],
    )?;
    framed_ending_call(
        context,
        caller,
        0x134,
        ids::ENDING_PLAY_DESCENDING_ROCKETSHIP_SCENE,
        &[],
    )?;
    framed_ending_call(
        context,
        caller,
        0x13a,
        ids::ENDING_PLAY_BOUNCING_ROCKETSHIP_SCENE,
        &[],
    )?;
    framed_ending_call_void(
        context,
        caller,
        0x142,
        ids::CUTSCENE_RELEASE_TEXT_ENTITIES,
        &[],
    )?;
    framed_ending_call_void(context, caller, 0x148, ids::CLEAR_LOWER_VRAM_TILEMAP, &[])?;
    framed_ending_call_void(
        context,
        caller,
        0x14e,
        ids::CUTSCENE_RELEASE_FLAGGED_ENTITY_RANGE,
        &[],
    )?;
    for arguments in palettes.into_iter().rev() {
        arguments.release(context)?;
    }
    level.release(context)?;
    Ok(None)
}

fn set_word_register(context: &mut dyn Context, index: usize, value: i16) {
    let old = context.data_register(index);
    context.set_data_register(index, (old & 0xffff_0000) | u32::from(value as u16));
}

fn framed_ending_call(
    context: &mut dyn Context,
    caller: crate::FunctionId,
    offset: u32,
    callee: crate::FunctionId,
    arguments: &[u32],
) -> Result<u32> {
    let site = CallSite::relative(caller, context.revision(), offset);
    crate::call_at(context, callee, arguments, site)
}

fn framed_ending_call_void(
    context: &mut dyn Context,
    caller: crate::FunctionId,
    offset: u32,
    callee: crate::FunctionId,
    arguments: &[u32],
) -> Result<()> {
    let site = CallSite::relative(caller, context.revision(), offset);
    crate::call_at_void(context, callee, arguments, site)
}

fn retained_ending_call(
    context: &mut dyn Context,
    caller: crate::FunctionId,
    offset: u32,
    callee: crate::FunctionId,
    arguments: &[u32],
) -> Result<OutgoingArguments> {
    let site = CallSite::relative(caller, context.revision(), offset);
    let area = OutgoingArguments::push(context, arguments)?;
    let dispatcher = context.dispatcher();
    if callee.metadata().abi.returns_value {
        area.call(context, dispatcher, callee, site)?;
    } else {
        area.call_void(context, dispatcher, callee, site)?;
    }
    Ok(area)
}

#[cfg(test)]
#[path = "ending_access_tests.rs"]
mod access_tests;

#[cfg(test)]
#[path = "ending_call_tests.rs"]
mod call_tests;
