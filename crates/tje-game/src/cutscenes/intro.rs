use crate::{Context, Result, call, ids};
use tje_mem::{Address, ram, rom};

/// G/src/cutscenes/intro.c:85 (`IntroBeginLargePlayerScene`).
/// TimeAware: the background animation advances through a real VBlank wait.
pub fn intro_begin_large_player_scene(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let background = arguments[0] as i16;
    call(context, ids::INTRO_ALLOCATE_LARGE_PLAYER_PARTS, &[])?;
    ram::g_scene_background_pair_index(revision).write(context, background as i8);
    call(context, ids::INTRO_POSITION_LARGE_PLAYER_PARTS, &[])?;
    call(context, ids::ANIMATE_TITLE_TRANSITION_BACKGROUND, &[1])?;
    let mode = ram::g_mode2_reg_softcopy(revision).read(context) | 0x40;
    ram::g_mode2_reg_softcopy(revision).write(context, mode);
    let mode = ram::g_mode2_reg_softcopy(revision).read(context);
    context.write_device_u16(0xc00004, u16::from(mode) | 0x8104);
    Ok(None)
}

/// G/src/cutscenes/intro.c:549 and corresponding loop guards. The matching
/// compiler evaluates GetCtrl2Input before GetCtrl1Input in the bitwise OR.
fn start_pressed(context: &mut dyn Context) -> Result<bool> {
    let second = call(context, ids::GET_CTRL2_INPUT, &[])?;
    let first = call(context, ids::GET_CTRL1_INPUT, &[])?;
    Ok((second | first) & 0x80 != 0)
}

/// G/src/cutscenes/intro.c:545-547 and repeated display enable/disable blocks.
fn set_display(context: &mut dyn Context, enabled: bool) {
    let mode = ram::g_mode2_reg_softcopy(context.revision());
    let value = if enabled {
        mode.read(context) | 0x40
    } else {
        mode.read(context) & 0x3c
    };
    mode.write(context, value);
    context.write_device_u16(0xc00004, u16::from(mode.read(context)) | 0x8104);
}

/// G/src/cutscenes/intro.c:557 and repeated signed-word predecrements.
fn decrement(context: &mut dyn Context, word: Address<i16>) -> i16 {
    let value = word.read(context).wrapping_sub(1);
    word.write(context, value);
    value
}

/// G/src/cutscenes/intro.c:560 and repeated signed-word preincrements.
fn increment(context: &mut dyn Context, word: Address<i16>) -> i16 {
    let value = word.read(context).wrapping_add(1);
    word.write(context, value);
    value
}

/// G/src/cutscenes/intro.c:524 (`IntroPart5`).
/// TimeAware: driving poses, scrolling, input and sound are interleaved with
/// shared VBlank/music service; countdown and local positions wrap as words.
pub fn intro_part5(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    ram::g_scene_background_pair_index(revision).write(context, 8);
    call(context, ids::INITIALIZE_TITLE_TRANSITION_TILEMAP, &[])?;
    call(context, ids::INTRO_ALLOCATE_DRIVING_PARTS, &[])?;
    call(context, ids::INTRO_ALLOCATE_FLYING_ENTITY_SLOTS, &[])?;
    call(
        context,
        ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
        &[11, 7, 30, 20],
    )?;
    call(
        context,
        ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
        &[12, 8, 29, 19],
    )?;
    call(context, ids::INTRO_POSITION_DRIVING_PART, &[0, 104, 151])?;
    call(context, ids::INTRO_POSITION_DRIVING_PART, &[1, 104, 151])?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank2(revision).address(), 2],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_intro_earl_driving_bank3(revision).address(), 3],
    )?;
    state.write(context, 0);
    countdown.write(context, 42);
    let mut driving_pose = 1_i16;
    let mut scroll_direction = 0_i16;
    let mut scroll_position = 0_i16;
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data(scroll_position as u16);
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    set_display(context, true);
    while state.read(context) != -1 && !start_pressed(context)? {
        call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
        call(
            context,
            ids::INTRO_UPDATE_FLYING_ENTITY_MOTION,
            &[driving_pose as i32 as u32],
        )?;
        if scroll_direction != 0 {
            context.write_vdp_command(0x7c02_0003);
            context.write_vdp_data(scroll_position as u16);
            scroll_position = scroll_position.wrapping_add(scroll_direction.wrapping_shl(2));
        }
        if decrement(context, countdown) != 0 {
            continue;
        }
        countdown.write(context, 42);
        let next = increment(context, state);
        match next {
            1 => {
                call(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_held_frame_streams(revision)
                            .at(2)
                            .frame()
                            .address(),
                        6,
                    ],
                )?;
                call(context, ids::INTRO_POSITION_DRIVING_PART, &[2, 104, 151])?;
                countdown.write(context, 17);
            }
            2 => {
                call(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[
                        2,
                        rom::intro_driving_actor_two_loop_animation(revision).address(),
                        3,
                    ],
                )?;
                countdown.write(context, 51);
            }
            3 => {
                call(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[
                        2,
                        rom::intro_held_frame_three(revision).frame().address(),
                        6,
                    ],
                )?;
                countdown.write(context, 17);
            }
            4 => {
                call(context, ids::INTRO_RELEASE_DRIVING_PART_SPRITES, &[2])?;
                call(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_held_frame_streams(revision)
                            .at(1)
                            .frame()
                            .address(),
                        6,
                    ],
                )?;
            }
            5 | 9 => {
                call(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_held_frame_streams(revision)
                            .at(5)
                            .frame()
                            .address(),
                        6,
                    ],
                )?;
                call(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_running(revision).address(), 50, 0],
                )?;
                driving_pose = 2;
                scroll_direction = -1;
            }
            6 | 8 | 10 => {
                call(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_held_frame_streams(revision)
                            .at(1)
                            .frame()
                            .address(),
                        6,
                    ],
                )?;
                driving_pose = 1;
                scroll_direction = 0;
            }
            7 => {
                call(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_held_frame_streams(revision)
                            .at(6)
                            .frame()
                            .address(),
                        6,
                    ],
                )?;
                call(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_running(revision).address(), 50, 0],
                )?;
                driving_pose = 3;
                scroll_direction = 1;
            }
            11 => {
                call(context, ids::INTRO_UPDATE_FLYING_ENTITY_MOTION, &[0])?;
                state.write(context, -1);
            }
            _ => {}
        }
    }
    call(context, ids::INTRO_RELEASE_DRIVING_PARTS, &[])?;
    call(context, ids::INTRO_RELEASE_FLYING_ENTITY_SLOTS, &[])?;
    scroll_position = 0;
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data(scroll_position as u16);
    call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
    Ok(Some(u32::from(state.read(context) != -1)))
}

/// G/src/cutscenes/intro.c:628 (`IntroPart6`).
/// TimeAware: timed dodging poses and controller/PSG observations use real frames.
pub fn intro_part6(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    call(context, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
    call(context, ids::INTRO_ALLOCATE_FLYING_ENTITY_SLOTS, &[])?;
    state.write(context, 0);
    countdown.write(context, 42);
    let mut scroll_position = 0_i16;
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data((scroll_position / 4) as u16);
    ram::g_scene_background_pair_index(revision).write(context, 9);
    call(context, ids::ANIMATE_TITLE_TRANSITION_BACKGROUND, &[1])?;
    call(
        context,
        ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
        &[0, 104, 151],
    )?;
    let mut pose_changed = 0_i16;
    let mut driving_pose = 0_i16;
    while state.read(context) != -1 && !start_pressed(context)? {
        call(
            context,
            ids::INTRO_UPDATE_RIGHTWARD_FLYING_ENTITY,
            &[pose_changed as i32 as u32, driving_pose as i32 as u32],
        )?;
        pose_changed = 0;
        call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
        context.write_vdp_command(0x7c02_0003);
        context.write_vdp_data((scroll_position / 4) as u16);
        scroll_position = scroll_position.wrapping_add(1);
        if decrement(context, countdown) != 0 {
            continue;
        }
        countdown.write(context, 4);
        match increment(context, state) {
            1 | 7 => {
                call(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[1, 104, 151],
                )?;
            }
            2 => {
                call(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[2, 104, 151],
                )?;
                call(context, ids::PLAY_SOUND, &[21, 5])?;
            }
            3 => {
                call(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[3, 104, 151],
                )?;
                countdown.write(context, 51);
                pose_changed = 1;
                driving_pose = 1;
            }
            4 => {
                call(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[2, 104, 151],
                )?;
            }
            5 => {
                call(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[1, 104, 151],
                )?;
                pose_changed = 1;
                driving_pose = 2;
                call(context, ids::PLAY_SOUND, &[22, 5])?;
            }
            6 => {
                call(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[0, 104, 151],
                )?;
                countdown.write(context, 51);
            }
            8 => {
                call(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[2, 104, 151],
                )?;
                pose_changed = 1;
                driving_pose = 1;
                call(context, ids::PLAY_SOUND, &[21, 5])?;
            }
            9 => {
                call(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[3, 104, 151],
                )?;
                countdown.write(context, 51);
            }
            10 => {
                state.write(context, -1);
            }
            _ => {}
        }
    }
    call(context, ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR, &[])?;
    call(context, ids::INTRO_RELEASE_FLYING_ENTITY_SLOTS, &[])?;
    scroll_position = 0;
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data(scroll_position as u16);
    call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
    Ok(Some(u32::from(state.read(context) != -1)))
}

/// G/src/cutscenes/intro.c:711 (`IntroPart7`).
/// TimeAware: alternate five-pixel camera shake, upload cadence and timed audio.
pub fn intro_part7(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    let camera_x = ram::g_camera1_pending_x(revision);
    let camera_y = ram::g_camera1_pending_y(revision);
    call(context, ids::INTRO_ALLOCATE_DRIVING_PARTS, &[])?;
    call(context, ids::INTRO_ALLOCATE_FLYING_ENTITY_SLOTS, &[])?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_intro_earl_driving_bank0(revision).address(), 0],
    )?;
    let mut shake_phase = 0_i16;
    state.write(context, 0);
    countdown.write(context, 34);
    let mut driving_pose = 1_i16;
    let mut scroll_position = 0_i16;
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data((scroll_position / 4) as u16);
    let mut scroll_direction = 0_i16;
    ram::g_scene_background_pair_index(revision).write(context, 10);
    call(context, ids::ANIMATE_TITLE_TRANSITION_BACKGROUND, &[1])?;
    call(context, ids::INTRO_POSITION_DRIVING_PART, &[0, 104, 151])?;
    call(context, ids::INTRO_POSITION_DRIVING_PART, &[1, 104, 151])?;
    while state.read(context) != -1 && !start_pressed(context)? {
        call(context, ids::ANIMATE_TITLE_TRANSITION_BACKGROUND, &[0])?;
        if shake_phase == 1 {
            let value = camera_x.read(context).wrapping_add(5);
            camera_x.write(context, value);
            let value = camera_y.read(context).wrapping_add(5);
            camera_y.write(context, value);
            shake_phase = 2;
        } else if shake_phase == 2 {
            let value = camera_x.read(context).wrapping_sub(5);
            camera_x.write(context, value);
            let value = camera_y.read(context).wrapping_sub(5);
            camera_y.write(context, value);
            shake_phase = 1;
        }
        ram::g_sprite_graphics_upload_pending(revision).write(context, 0);
        call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        if shake_phase != 0 {
            context.write_vdp_command(0x4000_0010);
            context.write_vdp_data(camera_y.read(context) as u16);
            context.write_vdp_command(0x7c00_0003);
            context.write_vdp_data(camera_x.read(context).wrapping_neg() as u16);
        }
        call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
        call(context, ids::SELECT_VIEWPORT_SPRITE_CHAIN, &[0])?;
        call(
            context,
            ids::INTRO_UPDATE_FLYING_ENTITY_MOTION,
            &[driving_pose as i32 as u32],
        )?;
        if scroll_direction != 0 {
            context.write_vdp_command(0x7c02_0003);
            context.write_vdp_data((scroll_position / 4) as u16);
            scroll_position = scroll_position.wrapping_add(scroll_direction.wrapping_shl(3));
        }
        if decrement(context, countdown) != 0 {
            continue;
        }
        countdown.write(context, 34);
        match increment(context, state) {
            1 => {
                call(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[1, rom::intro_held_frame_five(revision).frame().address(), 6],
                )?;
                call(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_running(revision).address(), 50, 0],
                )?;
                scroll_direction = -1;
                driving_pose = 2;
            }
            3 => {
                call(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[1, rom::intro_held_frame_one(revision).frame().address(), 6],
                )?;
                driving_pose = 1;
                scroll_direction = 0;
            }
            4 => {
                call(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[1, rom::intro_held_frame_six(revision).frame().address(), 6],
                )?;
                call(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_running(revision).address(), 50, 0],
                )?;
                scroll_direction = 1;
                driving_pose = 3;
            }
            5 => {
                call(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[1, rom::intro_held_frame_one(revision).frame().address(), 6],
                )?;
                driving_pose = 4;
                scroll_direction = 0;
            }
            6 => {
                shake_phase = 1;
                call(context, ids::INTRO_CREATE_ASTEROID_ENTITY, &[])?;
                call(context, ids::PLAY_SOUND, &[10, 5])?;
            }
            7 => {
                state.write(context, -1);
                shake_phase = 0;
            }
            _ => {}
        }
    }
    set_display(context, false);
    call(context, ids::INTRO_RELEASE_ASTEROID_ENTITY, &[])?;
    call(context, ids::INTRO_RELEASE_FLYING_ENTITY_SLOTS, &[])?;
    call(context, ids::INTRO_RELEASE_DRIVING_PARTS, &[])?;
    camera_x.write(context, 0);
    camera_y.write(context, 0);
    context.write_vdp_command(0x4000_0010);
    context.write_vdp_data(camera_y.read(context) as u16);
    context.write_vdp_command(0x7c00_0003);
    context.write_vdp_data(camera_x.read(context) as u16);
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data(camera_x.read(context) as u16);
    call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
    Ok(Some(u32::from(state.read(context) != -1)))
}

/// G/src/cutscenes/intro.c:819 (`IntroPart8`).
/// TimeAware: crash-screen transfer and shared-tile actors cross real VBlanks.
pub fn intro_part8(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    state.write(context, 0);
    countdown.write(context, 4);
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank0(revision).address(), 0],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_intro_collision_bank2(revision).address(), 2],
    )?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank3(revision).address(), 3],
    )?;
    context.write_vdp_command(0x4660_0001);
    let size = i32::from(rom::crash_screen_size(revision).read(context)).wrapping_shl(4);
    call(
        context,
        ids::MOVE_DATA_TO_VRAM,
        &[rom::crash_screen(revision).address(), size as u32],
    )?;
    ram::g_shared_tile_sprite_copies_enabled(revision).write(context, 1);
    call(context, ids::CLEAR_UPPER_VRAM_TILEMAP, &[])?;
    call(context, ids::CLEAR_LOWER_VRAM_TILEMAP, &[])?;
    call(context, ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK, &[0, 0])?;
    call(context, ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK, &[20, 0])?;
    call(context, ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK, &[0, 14])?;
    call(
        context,
        ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK,
        &[20, 14],
    )?;
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    set_display(context, true);
    while state.read(context) != -1 && !start_pressed(context)? {
        call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
        if decrement(context, countdown) == 0 {
            match increment(context, state) {
                1 => {
                    call(context, ids::INTRO_CREATE_COLLISION_SCENE_ENTITIES, &[])?;
                    call(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_falling(revision).address(), 50, 0],
                    )?;
                    countdown.write(context, 59);
                }
                2 => {
                    call(context, ids::INTRO_RELEASE_COLLISION_SCENE_ENTITIES, &[])?;
                    state.write(context, -1);
                }
                _ => {}
            }
        }
    }
    set_display(context, false);
    call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
    ram::g_shared_tile_sprite_copies_enabled(revision).write(context, 0);
    call(context, ids::DRAW_STARFIELD_PLANE, &[])?;
    call(context, ids::INITIALIZE_TITLE_TRANSITION_TILEMAP, &[])?;
    Ok(Some(u32::from(state.read(context) != -1)))
}

/// G/src/cutscenes/intro.c:866 (`IntroPart9`).
/// TimeAware: timed falling-ship scene with second-plane vertical scroll.
pub fn intro_part9(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    ram::g_scene_background_pair_index(revision).write(context, 12);
    call(context, ids::ANIMATE_TITLE_TRANSITION_BACKGROUND, &[1])?;
    call(
        context,
        ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
        &[11, 7, 30, 20],
    )?;
    call(
        context,
        ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
        &[12, 8, 29, 19],
    )?;
    call(context, ids::INTRO_CREATE_FALLING_ROCKETSHIP, &[])?;
    call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank2(revision).address(), 2],
    )?;
    state.write(context, 0);
    let mut scroll_position = 0_i16;
    countdown.write(context, 42);
    call(context, ids::INTRO_POSITION_FALLING_ROCKETSHIP, &[170, 141])?;
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    set_display(context, true);
    while state.read(context) != -1 && !start_pressed(context)? {
        call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
        context.write_vdp_command(0x4002_0010);
        context.write_vdp_data(scroll_position as u16);
        scroll_position = scroll_position.wrapping_add(1);
        if decrement(context, countdown) != 0 {
            continue;
        }
        countdown.write(context, 85);
        if increment(context, state) == 2 {
            state.write(context, -1);
        }
    }
    set_display(context, false);
    call(context, ids::INTRO_RELEASE_FALLING_ROCKETSHIP, &[])?;
    scroll_position = 0;
    context.write_vdp_command(0x4002_0010);
    context.write_vdp_data(scroll_position as u16);
    call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[0, 0])?;
    Ok(Some(u32::from(state.read(context) != -1)))
}
