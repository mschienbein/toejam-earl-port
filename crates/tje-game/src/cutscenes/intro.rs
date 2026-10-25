use crate::call_frame::CallSite;
use crate::{Context, Result, call, call_at, call_at_void, call_void, ids};
use tje_mem::{Address, ram, rom};

/// G/src/cutscenes/intro.c:85 (`IntroBeginLargePlayerScene`).
/// TimeAware: the background animation advances through a real VBlank wait.
pub fn intro_begin_large_player_scene(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let entry = context
        .frame_state()
        .ok_or(crate::call_frame::FrameError::Unavailable)?;
    let saved_d2 = context.data_register(2);
    // MOVE.W 10(SP),D2 after the private D2 save: entry SP+6, not the
    // complete four-byte argument slot, and before the first allocation call.
    let background = entry.argument_word(0).read(context);
    context.set_data_register(2, (saved_d2 & 0xffff_0000) | u32::from(background as u16));
    call_void(context, ids::INTRO_ALLOCATE_LARGE_PLAYER_PARTS, &[])?;
    ram::g_scene_background_pair_index(revision).write(context, background as i8);
    call_void(context, ids::INTRO_POSITION_LARGE_PLAYER_PARTS, &[])?;
    framed_intro_call(
        context,
        ids::INTRO_BEGIN_LARGE_PLAYER_SCENE,
        0x22,
        ids::ANIMATE_TITLE_TRANSITION_BACKGROUND,
        &[1],
    )?;
    let mode = ram::g_mode2_reg_softcopy(revision).read(context) | 0x40;
    ram::g_mode2_reg_softcopy(revision).write(context, mode);
    let mode = ram::g_mode2_reg_softcopy(revision).read(context);
    context.write_device_u16(0xc00004, u16::from(mode) | 0x8104);
    context.set_data_register(2, saved_d2);
    // Final MOVE.L (SP)+,D2 restores the entry register and defines N/Z/V/C.
    let flags = if saved_d2 == 0 {
        0x04
    } else if saved_d2 & 0x8000_0000 != 0 {
        0x08
    } else {
        0
    };
    let status = context.status_register();
    context.set_status_register((status & !0x000f) | flags);
    Ok(None)
}

/// Matching c_intro.o return PCs are function-relative and equal in both ROM
/// revisions. Prepared calls supply a real argument area to lazy framed entries.
fn framed_intro_call(
    context: &mut dyn Context,
    caller: crate::FunctionId,
    offset: u32,
    callee: crate::FunctionId,
    arguments: &[u32],
) -> Result<()> {
    let site = CallSite::relative(caller, context.revision(), offset);
    call_at_void(context, callee, arguments, site)
}

fn framed_intro_cadence(
    context: &mut dyn Context,
    caller: crate::FunctionId,
    offset: u32,
    arguments: &[u32],
) -> Result<()> {
    let site = CallSite::relative(caller, context.revision(), offset);
    // Cadence declares an int result even though intro.c does not use it.
    call_at(
        context,
        ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
        arguments,
        site,
    )?;
    Ok(())
}

/// G/src/cutscenes/intro.c:549 and corresponding loop guards. The matching
/// compiler evaluates GetCtrl2Input before GetCtrl1Input in the bitwise OR.
pub(super) fn start_pressed(context: &mut dyn Context) -> Result<bool> {
    let second = call(context, ids::GET_CTRL2_INPUT, &[])?;
    let first = call(context, ids::GET_CTRL1_INPUT, &[])?;
    Ok((second | first) & 0x80 != 0)
}

/// G/src/cutscenes/intro.c:545-547 and repeated display enable/disable blocks.
pub(super) fn set_display(context: &mut dyn Context, enabled: bool) {
    let mode = ram::g_mode2_reg_softcopy(context.revision());
    let value = if enabled {
        mode.read(context) | 0x40
    } else {
        mode.read(context) & 0x3c
    };
    mode.write(context, value);
    context.write_device_u16(0xc00004, u16::from(mode.read(context)) | 0x8104);
}

/// G/src/cutscenes/intro.c:115,330,400,490 and intro_part11.c:41.
/// SUBQ.W followed by BGT consumes V as well as N/Z. Store the truncated word
/// but compare the widened subtraction: -32768 -> stored 32767 still takes
/// the nonpositive path. BNE/BEQ countdown users have the same zero decision.
pub(super) fn decrement<B: tje_mem::Bus + ?Sized>(context: &mut B, word: Address<i16>) -> i32 {
    let comparison = i32::from(word.read(context)) - 1;
    word.write(context, comparison as i16);
    comparison
}

/// G/src/cutscenes/intro.c:560 and repeated signed-word preincrements.
pub(super) fn increment(context: &mut dyn Context, word: Address<i16>) -> i16 {
    let value = word.read(context).wrapping_add(1);
    word.write(context, value);
    value
}

/// G/build/accept/c038-final/a/{rev02,rev00}/obj/c_intro.s: each
/// countdown-driven switch reloads the state after ADDQ.W's memory write.
/// Do not reuse the arithmetic result: the original performs another bus read.
fn increment_and_reload(context: &mut dyn Context, word: Address<i16>) -> i16 {
    increment(context, word);
    word.read(context)
}

/// G accepted c_intro.o: dense switches load a signed PC-relative ROM word
/// before jumping. Rust owns the equivalent match, but must retain that
/// semantic read; out-of-range states take the compiler's bounds branches.
fn read_switch_entry(
    context: &mut dyn Context,
    owner: crate::FunctionId,
    offset: u32,
    upper: i16,
    state: i16,
) {
    if (1..=upper).contains(&state) {
        let address = owner
            .metadata()
            .address(context.revision())
            .wrapping_add(offset)
            .wrapping_add(u32::from((state - 1) as u16) * 2);
        let _ = Address::<i16>::new(address).read(context);
    }
}

/// G accepted c_intro.o: each part ends with CMPI.W/SNE/NEG.B/EXT.W/EXT.L.
/// NEG.B produces X=1 only for an interrupted scene; the final EXT clears
/// N/V/C and sets Z for D0=0. SR control bits are never changed.
fn scene_result(context: &mut dyn Context, state: Address<i16>) -> Result<Option<u32>> {
    let interrupted = state.read(context) != -1;
    let status = context.status_register();
    context.set_status_register((status & !0x001f) | if interrupted { 0x10 } else { 0x04 });
    Ok(Some(u32::from(interrupted)))
}

/// G accepted c_intro.s: CLR reads its memory operand before writing zero.
/// The pre-read is discarded, not an extra semantic input to the Rust body.
fn clear_word(context: &mut dyn Context, word: Address<i16>) -> Result<()> {
    context.discard_u16(word.address())?;
    word.write(context, 0);
    Ok(())
}

fn clear_byte(context: &mut dyn Context, address: u32) -> Result<()> {
    context.discard_u8(address)?;
    context.write_u8(address, 0);
    Ok(())
}

/// G/src/cutscenes/intro.c:524 (`IntroPart5`).
/// TimeAware: driving poses, scrolling, input and sound are interleaved with
/// shared VBlank/music service; countdown and local positions wrap as words.
pub fn intro_part5(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    ram::g_scene_background_pair_index(revision).write(context, 8);
    call_void(context, ids::INITIALIZE_TITLE_TRANSITION_TILEMAP, &[])?;
    call_void(context, ids::INTRO_ALLOCATE_DRIVING_PARTS, &[])?;
    call_void(context, ids::INTRO_ALLOCATE_FLYING_ENTITY_SLOTS, &[])?;
    framed_intro_call(
        context,
        ids::INTRO_PART5,
        0x46,
        ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
        &[11, 7, 30, 20],
    )?;
    framed_intro_call(
        context,
        ids::INTRO_PART5,
        0x5c,
        ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
        &[12, 8, 29, 19],
    )?;
    call_void(context, ids::INTRO_POSITION_DRIVING_PART, &[0, 104, 151])?;
    call_void(context, ids::INTRO_POSITION_DRIVING_PART, &[1, 104, 151])?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank2(revision).address(), 2],
    )?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_intro_earl_driving_bank3(revision).address(), 3],
    )?;
    clear_word(context, state)?;
    countdown.write(context, 42);
    let mut driving_pose = 1_i16;
    let mut scroll_direction = 0_i16;
    let mut scroll_position = 0_i16;
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data(scroll_position as u16);
    call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    set_display(context, true);
    while state.read(context) != -1 && !start_pressed(context)? {
        framed_intro_cadence(context, ids::INTRO_PART5, 0xf8, &[0, 0])?;
        call_void(
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
        let next = increment_and_reload(context, state);
        read_switch_entry(context, ids::INTRO_PART5, 0x156, 11, next);
        match next {
            1 => {
                call_void(
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
                call_void(context, ids::INTRO_POSITION_DRIVING_PART, &[2, 104, 151])?;
                countdown.write(context, 17);
            }
            2 => {
                call_void(
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
                call_void(
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
                call_void(context, ids::INTRO_RELEASE_DRIVING_PART_SPRITES, &[2])?;
                call_void(
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
                call_void(
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
                call_void(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_running(revision).address(), 50, 0],
                )?;
                driving_pose = 2;
                scroll_direction = -1;
            }
            6 | 8 | 10 => {
                call_void(
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
                call_void(
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
                call_void(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_running(revision).address(), 50, 0],
                )?;
                driving_pose = 3;
                scroll_direction = 1;
            }
            11 => {
                call_void(context, ids::INTRO_UPDATE_FLYING_ENTITY_MOTION, &[0])?;
                state.write(context, -1);
            }
            _ => {}
        }
    }
    call_void(context, ids::INTRO_RELEASE_DRIVING_PARTS, &[])?;
    call_void(context, ids::INTRO_RELEASE_FLYING_ENTITY_SLOTS, &[])?;
    scroll_position = 0;
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data(scroll_position as u16);
    framed_intro_cadence(context, ids::INTRO_PART5, 0x2b8, &[0, 0])?;
    scene_result(context, state)
}

/// G/src/cutscenes/intro.c:628 (`IntroPart6`).
/// TimeAware: timed dodging poses and controller/PSG observations use real frames.
pub fn intro_part6(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    call_void(context, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
    call_void(context, ids::INTRO_ALLOCATE_FLYING_ENTITY_SLOTS, &[])?;
    clear_word(context, state)?;
    countdown.write(context, 42);
    let mut scroll_position = 0_i16;
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data((scroll_position / 4) as u16);
    ram::g_scene_background_pair_index(revision).write(context, 9);
    framed_intro_call(
        context,
        ids::INTRO_PART6,
        0x5c,
        ids::ANIMATE_TITLE_TRANSITION_BACKGROUND,
        &[1],
    )?;
    call_void(
        context,
        ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
        &[0, 104, 151],
    )?;
    let mut pose_changed = 0_i16;
    let mut driving_pose = 0_i16;
    while state.read(context) != -1 && !start_pressed(context)? {
        call_void(
            context,
            ids::INTRO_UPDATE_RIGHTWARD_FLYING_ENTITY,
            &[pose_changed as i32 as u32, driving_pose as i32 as u32],
        )?;
        pose_changed = 0;
        framed_intro_cadence(context, ids::INTRO_PART6, 0x94, &[0, 0])?;
        context.write_vdp_command(0x7c02_0003);
        context.write_vdp_data((scroll_position / 4) as u16);
        scroll_position = scroll_position.wrapping_add(1);
        if decrement(context, countdown) != 0 {
            continue;
        }
        countdown.write(context, 4);
        let next = increment_and_reload(context, state);
        read_switch_entry(context, ids::INTRO_PART6, 0xe0, 10, next);
        match next {
            1 | 7 => {
                call_void(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[1, 104, 151],
                )?;
            }
            2 => {
                call_void(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[2, 104, 151],
                )?;
                call_void(context, ids::PLAY_SOUND, &[21, 5])?;
            }
            3 => {
                call_void(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[3, 104, 151],
                )?;
                countdown.write(context, 51);
                pose_changed = 1;
                driving_pose = 1;
            }
            4 => {
                call_void(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[2, 104, 151],
                )?;
            }
            5 => {
                call_void(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[1, 104, 151],
                )?;
                pose_changed = 1;
                driving_pose = 2;
                call_void(context, ids::PLAY_SOUND, &[22, 5])?;
            }
            6 => {
                call_void(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[0, 104, 151],
                )?;
                countdown.write(context, 51);
            }
            8 => {
                call_void(
                    context,
                    ids::CUTSCENE_SET_ROCKETSHIP_PAIR_POSE,
                    &[2, 104, 151],
                )?;
                pose_changed = 1;
                driving_pose = 1;
                call_void(context, ids::PLAY_SOUND, &[21, 5])?;
            }
            9 => {
                call_void(
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
    call_void(context, ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR, &[])?;
    call_void(context, ids::INTRO_RELEASE_FLYING_ENTITY_SLOTS, &[])?;
    scroll_position = 0;
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data(scroll_position as u16);
    framed_intro_cadence(context, ids::INTRO_PART6, 0x212, &[0, 0])?;
    scene_result(context, state)
}

/// G/src/cutscenes/intro.c:711 (`IntroPart7`).
/// TimeAware: alternate five-pixel camera shake, upload cadence and timed audio.
pub fn intro_part7(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    let camera_x = ram::g_camera1_pending_x(revision);
    let camera_y = ram::g_camera1_pending_y(revision);
    call_void(context, ids::INTRO_ALLOCATE_DRIVING_PARTS, &[])?;
    call_void(context, ids::INTRO_ALLOCATE_FLYING_ENTITY_SLOTS, &[])?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_intro_earl_driving_bank0(revision).address(), 0],
    )?;
    let mut shake_phase = 0_i16;
    clear_word(context, state)?;
    countdown.write(context, 34);
    let mut driving_pose = 1_i16;
    let mut scroll_position = 0_i16;
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data((scroll_position / 4) as u16);
    let mut scroll_direction = 0_i16;
    ram::g_scene_background_pair_index(revision).write(context, 10);
    framed_intro_call(
        context,
        ids::INTRO_PART7,
        0x76,
        ids::ANIMATE_TITLE_TRANSITION_BACKGROUND,
        &[1],
    )?;
    call_void(context, ids::INTRO_POSITION_DRIVING_PART, &[0, 104, 151])?;
    call_void(context, ids::INTRO_POSITION_DRIVING_PART, &[1, 104, 151])?;
    while state.read(context) != -1 && !start_pressed(context)? {
        framed_intro_call(
            context,
            ids::INTRO_PART7,
            0xa8,
            ids::ANIMATE_TITLE_TRANSITION_BACKGROUND,
            &[0],
        )?;
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
        clear_byte(
            context,
            ram::g_sprite_graphics_upload_pending(revision).address(),
        )?;
        call_void(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
        call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        if shake_phase != 0 {
            context.write_vdp_command(0x4000_0010);
            context.write_vdp_data(camera_y.read(context) as u16);
            context.write_vdp_command(0x7c00_0003);
            context.write_vdp_data(camera_x.read(context).wrapping_neg() as u16);
        }
        call_void(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
        framed_intro_call(
            context,
            ids::INTRO_PART7,
            0x11c,
            ids::SELECT_VIEWPORT_SPRITE_CHAIN,
            &[0],
        )?;
        call_void(
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
        let next = increment_and_reload(context, state);
        read_switch_entry(context, ids::INTRO_PART7, 0x18a, 7, next);
        match next {
            1 => {
                call_void(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[1, rom::intro_held_frame_five(revision).frame().address(), 6],
                )?;
                call_void(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_running(revision).address(), 50, 0],
                )?;
                scroll_direction = -1;
                driving_pose = 2;
            }
            3 => {
                call_void(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[1, rom::intro_held_frame_one(revision).frame().address(), 6],
                )?;
                driving_pose = 1;
                scroll_direction = 0;
            }
            4 => {
                call_void(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[1, rom::intro_held_frame_six(revision).frame().address(), 6],
                )?;
                call_void(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_running(revision).address(), 50, 0],
                )?;
                scroll_direction = 1;
                driving_pose = 3;
            }
            5 => {
                call_void(
                    context,
                    ids::INTRO_SET_DRIVING_PART_SEQUENCE,
                    &[1, rom::intro_held_frame_one(revision).frame().address(), 6],
                )?;
                driving_pose = 4;
                scroll_direction = 0;
            }
            6 => {
                shake_phase = 1;
                call_void(context, ids::INTRO_CREATE_ASTEROID_ENTITY, &[])?;
                call_void(context, ids::PLAY_SOUND, &[10, 5])?;
            }
            7 => {
                state.write(context, -1);
                shake_phase = 0;
            }
            _ => {}
        }
    }
    set_display(context, false);
    call_void(context, ids::INTRO_RELEASE_ASTEROID_ENTITY, &[])?;
    call_void(context, ids::INTRO_RELEASE_FLYING_ENTITY_SLOTS, &[])?;
    call_void(context, ids::INTRO_RELEASE_DRIVING_PARTS, &[])?;
    clear_word(context, camera_x)?;
    clear_word(context, camera_y)?;
    context.write_vdp_command(0x4000_0010);
    context.write_vdp_data(camera_y.read(context) as u16);
    context.write_vdp_command(0x7c00_0003);
    context.write_vdp_data(camera_x.read(context) as u16);
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data(camera_x.read(context) as u16);
    framed_intro_cadence(context, ids::INTRO_PART7, 0x2de, &[0, 0])?;
    scene_result(context, state)
}

/// G/src/cutscenes/intro.c:819 (`IntroPart8`).
/// TimeAware: crash-screen transfer and shared-tile actors cross real VBlanks.
pub fn intro_part8(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    clear_word(context, state)?;
    countdown.write(context, 4);
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank0(revision).address(), 0],
    )?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_intro_collision_bank2(revision).address(), 2],
    )?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank3(revision).address(), 3],
    )?;
    context.write_vdp_command(0x4660_0001);
    let size = i32::from(rom::crash_screen_size(revision).read(context)).wrapping_shl(4);
    call_void(
        context,
        ids::MOVE_DATA_TO_VRAM,
        &[rom::crash_screen(revision).address(), size as u32],
    )?;
    ram::g_shared_tile_sprite_copies_enabled(revision).write(context, 1);
    call_void(context, ids::CLEAR_UPPER_VRAM_TILEMAP, &[])?;
    call_void(context, ids::CLEAR_LOWER_VRAM_TILEMAP, &[])?;
    call_void(context, ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK, &[0, 0])?;
    call_void(context, ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK, &[20, 0])?;
    call_void(context, ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK, &[0, 14])?;
    call_void(
        context,
        ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK,
        &[20, 14],
    )?;
    call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    set_display(context, true);
    while state.read(context) != -1 && !start_pressed(context)? {
        framed_intro_cadence(context, ids::INTRO_PART8, 0xd8, &[0, 0])?;
        if decrement(context, countdown) == 0 {
            match increment_and_reload(context, state) {
                1 => {
                    // G's preserved A3 holds the background-draw function; the
                    // collision constructor consumes it on allocation failure.
                    let saved_a3 = context.address_register(3);
                    let residue = ids::JAM_OUT_DRAW_PACKED_BACKGROUND_BLOCK
                        .metadata()
                        .address(revision);
                    context.set_address_register(3, residue);
                    let result =
                        call_void(context, ids::INTRO_CREATE_COLLISION_SCENE_ENTITIES, &[]);
                    context.set_address_register(3, saved_a3);
                    result?;
                    call_void(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_falling(revision).address(), 50, 0],
                    )?;
                    countdown.write(context, 59);
                }
                2 => {
                    call_void(context, ids::INTRO_RELEASE_COLLISION_SCENE_ENTITIES, &[])?;
                    state.write(context, -1);
                }
                _ => {}
            }
        }
    }
    set_display(context, false);
    framed_intro_cadence(context, ids::INTRO_PART8, 0x160, &[0, 0])?;
    clear_word(context, ram::g_shared_tile_sprite_copies_enabled(revision))?;
    call_void(context, ids::DRAW_STARFIELD_PLANE, &[])?;
    call_void(context, ids::INITIALIZE_TITLE_TRANSITION_TILEMAP, &[])?;
    scene_result(context, state)
}

/// G/src/cutscenes/intro.c:866 (`IntroPart9`).
/// TimeAware: timed falling-ship scene with second-plane vertical scroll.
pub fn intro_part9(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    ram::g_scene_background_pair_index(revision).write(context, 12);
    framed_intro_call(
        context,
        ids::INTRO_PART9,
        0x28,
        ids::ANIMATE_TITLE_TRANSITION_BACKGROUND,
        &[1],
    )?;
    framed_intro_call(
        context,
        ids::INTRO_PART9,
        0x3e,
        ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
        &[11, 7, 30, 20],
    )?;
    framed_intro_call(
        context,
        ids::INTRO_PART9,
        0x54,
        ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
        &[12, 8, 29, 19],
    )?;
    call_void(context, ids::INTRO_CREATE_FALLING_ROCKETSHIP, &[])?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank2(revision).address(), 2],
    )?;
    clear_word(context, state)?;
    let mut scroll_position = 0_i16;
    countdown.write(context, 42);
    call_void(context, ids::INTRO_POSITION_FALLING_ROCKETSHIP, &[170, 141])?;
    call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    set_display(context, true);
    while state.read(context) != -1 && !start_pressed(context)? {
        framed_intro_cadence(context, ids::INTRO_PART9, 0xb2, &[0, 0])?;
        context.write_vdp_command(0x4002_0010);
        context.write_vdp_data(scroll_position as u16);
        scroll_position = scroll_position.wrapping_add(1);
        if decrement(context, countdown) != 0 {
            continue;
        }
        countdown.write(context, 85);
        if increment_and_reload(context, state) == 2 {
            state.write(context, -1);
        }
    }
    set_display(context, false);
    call_void(context, ids::INTRO_RELEASE_FALLING_ROCKETSHIP, &[])?;
    scroll_position = 0;
    context.write_vdp_command(0x4002_0010);
    context.write_vdp_data(scroll_position as u16);
    framed_intro_cadence(context, ids::INTRO_PART9, 0x13a, &[0, 0])?;
    scene_result(context, state)
}

/// G/src/cutscenes/intro.c:112 and corresponding release loops; the compiler
/// evaluates the second pad first before masking the three face buttons.
pub(super) fn action_buttons(context: &mut dyn Context) -> Result<i16> {
    let second = call(context, ids::GET_CTRL2_INPUT, &[])?;
    let first = call(context, ids::GET_CTRL1_INPUT, &[])?;
    Ok(((second | first) & 0x70) as i16)
}

/// G/src/cutscenes/intro.c and intro_part11.c: compiler-local caption objects
/// are addressed relative to their owning function. Offsets below come from
/// matching compiler labels in both revision objects, never copied text bytes.
pub(super) fn caption(
    context: &mut dyn Context,
    owner: crate::FunctionId,
    offset: u32,
    x: i16,
    y: i16,
    second: bool,
) -> Result<()> {
    let text = Address::<i8>::new(
        owner
            .metadata()
            .address(context.revision())
            .wrapping_add(offset),
    );
    let function = if second {
        ids::CUTSCENE_LOAD_SECOND_TEXT_LINE
    } else {
        ids::CUTSCENE_TEXT_LOAD
    };
    call_void(
        context,
        function,
        &[text.address(), x as i32 as u32, y as i32 as u32],
    )?;
    Ok(())
}

/// G/src/cutscenes/intro.c:97 (`IntroPart1`).
/// TimeAware: released face-button progression, music and real VBlank service.
/// Caption offsets are compiler labels _L80.._L94 relative to IntroPart1 in
/// G/build/{rev02,rev00}/obj/c_intro.o; both revision offsets are identical.
pub fn intro_part1(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    let actor = ram::g_cutscene_music_actor_animation_enabled(revision);
    let advance_mask = ram::g_cutscene_advance_button_mask(revision);
    let owner = ids::INTRO_PART1;
    framed_intro_call(
        context,
        owner,
        0x1e,
        ids::INTRO_BEGIN_LARGE_PLAYER_SCENE,
        &[7],
    )?;
    let mut advance_scene = false;
    clear_word(context, state)?;
    countdown.write(context, 51);
    actor.write(context, 1);
    while start_pressed(context)? {
        let enabled = actor.read(context) as i32 as u32;
        framed_intro_cadence(context, owner, 0x48, &[0, enabled])?;
    }
    while state.read(context) != -1 && !start_pressed(context)? {
        loop {
            let enabled = actor.read(context) as i32 as u32;
            framed_intro_cadence(context, owner, 0x76, &[0, enabled])?;
            let mask = action_buttons(context)?;
            advance_mask.write(context, mask);
            if mask != 0 {
                advance_scene = true;
            }
            if advance_mask.read(context) == 0 {
                break;
            }
        }
        if !advance_scene && decrement(context, countdown) > 0 {
            continue;
        }
        increment(context, state);
        actor.write(context, 1);
        let next = state.read(context);
        read_switch_entry(context, owner, 0xd6, 39, next);
        match next {
            1 | 3 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        5,
                        rom::intro_large_player_part_five_frame_pulse(revision).address(),
                        3,
                    ],
                )?;
                countdown.write(context, if next == 1 { 25 } else { 34 });
            }
            2 => {
                advance_scene = false;
                caption(context, owner, 0x56c, 220, 80, false)?;
                countdown.write(context, 34);
            }
            4 => {
                advance_scene = false;
                caption(context, owner, 0x578, 220, 80, false)?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        6,
                        rom::intro_large_player_part_six_held_frame_fifteen(revision).address(),
                        6,
                    ],
                )?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_seventeen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 59);
            }
            5 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        6,
                        rom::intro_large_player_part_six_held_frame_fourteen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 34);
            }
            6 => {
                advance_scene = false;
                caption(context, owner, 0x58c, 220, 72, false)?;
                caption(context, owner, 0x5a4, 220, 84, true)?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_sixteen(revision).address(),
                        6,
                    ],
                )?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        5,
                        rom::intro_large_player_part_five_ascending_frames(revision).address(),
                        3,
                    ],
                )?;
                countdown.write(context, 51);
            }
            7 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_large_player_part_one_held_frame_two(revision).address(),
                        6,
                    ],
                )?;
                clear_word(context, actor)?;
                countdown.write(context, 51);
            }
            8 => {
                call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
                countdown.write(context, 8);
            }
            9 => {
                advance_scene = false;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_large_player_part_one_held_frame_three(revision).address(),
                        6,
                    ],
                )?;
                caption(context, owner, 0x5b6, 78, 54, false)?;
                countdown.write(context, 34);
                clear_word(context, actor)?;
            }
            10 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_large_player_part_one_held_frame_one(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 8);
            }
            11 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        5,
                        rom::intro_large_player_part_five_descending_frames(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 8);
                call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
            }
            12 | 18 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        5,
                        rom::intro_large_player_part_five_frame_pulse(revision).address(),
                        3,
                    ],
                )?;
                countdown.write(context, 8);
            }
            13 => {
                advance_scene = false;
                caption(context, owner, 0x5c4, 220, 72, false)?;
                caption(context, owner, 0x5d8, 220, 84, true)?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        6,
                        rom::intro_large_player_part_six_held_frame_fifteen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 8);
            }
            14 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_seventeen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 4);
            }
            15 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_eighteen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 17);
            }
            16 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_seventeen(revision).address(),
                        6,
                    ],
                )?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        6,
                        rom::intro_large_player_part_six_held_frame_fourteen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 51);
            }
            17 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_sixteen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 17);
            }
            19 => {
                advance_scene = false;
                caption(context, owner, 0x5ec, 220, 80, false)?;
                countdown.write(context, 59);
            }
            20 | 29 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_large_player_part_one_held_frame_two(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 8);
            }
            21 => {
                advance_scene = false;
                caption(context, owner, 0x5fe, 78, 54, false)?;
                countdown.write(context, 42);
            }
            22 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_large_player_part_one_held_frame_one(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 34);
            }
            23 => {
                countdown.write(context, 8);
                call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
            }
            24 => {
                advance_scene = false;
                caption(context, owner, 0x60c, 220, 80, false)?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_seventeen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 34);
            }
            25 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        6,
                        rom::intro_large_player_part_six_held_frame_fifteen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 8);
            }
            26 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        6,
                        rom::intro_large_player_part_six_held_frame_fourteen(revision).address(),
                        6,
                    ],
                )?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_sixteen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 34);
            }
            27 => {
                advance_scene = false;
                caption(context, owner, 0x620, 220, 72, false)?;
                caption(context, owner, 0x62c, 220, 84, true)?;
                countdown.write(context, 85);
            }
            28 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        5,
                        rom::intro_large_player_part_five_frame_pulse(revision).address(),
                        3,
                    ],
                )?;
                countdown.write(context, 17);
            }
            30 => {
                advance_scene = false;
                caption(context, owner, 0x642, 78, 54, false)?;
                countdown.write(context, 17);
            }
            31 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        5,
                        rom::intro_large_player_part_five_ascending_frames(revision).address(),
                        3,
                    ],
                )?;
                countdown.write(context, 34);
            }
            32 => {
                call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
                countdown.write(context, 8);
            }
            33 => {
                advance_scene = false;
                caption(context, owner, 0x64e, 220, 80, false)?;
                countdown.write(context, 34);
            }
            34 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_large_player_part_one_held_frame_three(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 34);
                clear_word(context, actor)?;
            }
            35 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        5,
                        rom::intro_large_player_part_five_descending_frames(revision).address(),
                        6,
                    ],
                )?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_large_player_part_one_held_frame_two(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 17);
            }
            36 => {
                advance_scene = false;
                caption(context, owner, 0x65e, 220, 72, false)?;
                countdown.write(context, 34);
            }
            37 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_seventeen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 17);
            }
            38 => {
                call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        5,
                        rom::intro_large_player_part_five_frame_pulse(revision).address(),
                        3,
                    ],
                )?;
                countdown.write(context, 17);
            }
            39 => {
                state.write(context, -1);
            }
            _ => {}
        }
    }
    call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    framed_intro_call(
        context,
        owner,
        0x53e,
        ids::INTRO_RELEASE_LARGE_PLAYER_SPRITES_AND_REFRESH,
        &[],
    )?;
    call_void(context, ids::INTRO_RELEASE_LARGE_PLAYER_PARTS, &[])?;
    framed_intro_cadence(context, owner, 0x54e, &[0, 0])?;
    scene_result(context, state)
}

/// G/src/cutscenes/intro.c:912 (`IntroPart10`).
/// TimeAware: timed descent, impact and caption. The caption pointer is compiler
/// label _L380, IntroPart10+0x194, identical in both matching revision objects.
pub fn intro_part10(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    framed_intro_call(
        context,
        ids::INTRO_PART10,
        0x20,
        ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
        &[1],
    )?;
    call_void(context, ids::INTRO_CREATE_FALLING_ROCKETSHIP, &[])?;
    countdown.write(context, 42);
    let mut scene_y = 20_i16;
    call_void(context, ids::CUTSCENE_CREATE_PLANET_ENTITY, &[160, 180])?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_space_screen_bank3(revision).address(), 3],
    )?;
    call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    set_display(context, true);
    state.write(context, 1);
    while state.read(context) != -1 && !start_pressed(context)? {
        framed_intro_cadence(context, ids::INTRO_PART10, 0x80, &[0, 0])?;
        let next = state.read(context);
        read_switch_entry(context, ids::INTRO_PART10, 0x9e, 6, next);
        match next {
            1 => {
                scene_y = scene_y.wrapping_add(1);
                call_void(
                    context,
                    ids::INTRO_POSITION_FALLING_ROCKETSHIP,
                    &[160, scene_y as i32 as u32],
                )?;
                if scene_y == 160 {
                    increment(context, state);
                    call_void(context, ids::INTRO_RELEASE_FALLING_ROCKETSHIP, &[])?;
                }
            }
            2 => {
                call_void(context, ids::INTRO_CREATE_ROCKETSHIP_IMPACT, &[160, 180])?;
                call_void(context, ids::PLAY_SOUND, &[8, 50])?;
                countdown.write(context, 85);
                increment(context, state);
            }
            3 | 5 => {
                if decrement(context, countdown) == 0 {
                    increment(context, state);
                }
            }
            4 => {
                call_void(context, ids::INTRO_RELEASE_ROCKETSHIP_IMPACT, &[])?;
                caption(context, ids::INTRO_PART10, 0x194, 160, 30, false)?;
                countdown.write(context, 119);
                increment(context, state);
            }
            6 => {
                state.write(context, -1);
            }
            _ => {}
        }
    }
    set_display(context, false);
    call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    call_void(context, ids::CUTSCENE_RELEASE_PLANET_ENTITY, &[])?;
    call_void(context, ids::INTRO_RELEASE_FALLING_ROCKETSHIP, &[])?;
    framed_intro_cadence(context, ids::INTRO_PART10, 0x17a, &[0, 0])?;
    scene_result(context, state)
}

/// G/src/cutscenes/intro.c:314 (`IntroPart2`).
/// TimeAware: revealed ship components and released-button caption states.
/// Caption labels _L150.._L157 have identical function-relative offsets in the
/// two matching c_intro.o objects; only those address offsets are recorded here.
pub fn intro_part2(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    let advance_mask = ram::g_cutscene_advance_button_mask(revision);
    let owner = ids::INTRO_PART2;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_ship_line3(revision).address(), 2],
    )?;
    call_void(context, ids::INTRO_CREATE_HIDDEN_FRAME_ENTITIES, &[])?;
    clear_word(context, state)?;
    countdown.write(context, 17);
    let mut advance_period = 0_i16;
    while state.read(context) != -1 && !start_pressed(context)? {
        loop {
            framed_intro_cadence(context, owner, 0x46, &[advance_period as i32 as u32, 0])?;
            let mask = action_buttons(context)?;
            advance_mask.write(context, mask);
            if mask != 0 {
                clear_word(context, countdown)?;
            }
            if advance_mask.read(context) == 0 {
                break;
            }
        }
        if decrement(context, countdown) > 0 {
            continue;
        }
        countdown.write(context, 85);
        let next = increment_and_reload(context, state);
        read_switch_entry(context, owner, 0x96, 10, next);
        match next {
            1 => {
                caption(context, owner, 0x1cc, 160, 30, false)?;
                call_void(context, ids::INTRO_REVEAL_FRAME_ENTITIES, &[])?;
            }
            2 => {
                caption(context, owner, 0x1e6, 160, 30, false)?;
                countdown.write(context, 51);
            }
            3 => {
                caption(context, owner, 0x1f2, 160, 30, false)?;
                countdown.write(context, 102);
            }
            4 => {
                caption(context, owner, 0x212, 160, 30, false)?;
            }
            5 => {
                caption(context, owner, 0x230, 160, 30, false)?;
            }
            6 => {
                caption(context, owner, 0x246, 160, 30, false)?;
            }
            7 => {
                caption(context, owner, 0x264, 160, 30, false)?;
                countdown.write(context, 102);
            }
            8 => {
                caption(context, owner, 0x280, 160, 30, false)?;
                countdown.write(context, 51);
            }
            9 => {
                call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
                ram::g_requested_music_selection(revision).write(context, 6);
                advance_period = 1;
                countdown.write(context, 306);
            }
            10 => {
                state.write(context, -1);
                advance_period = 0;
            }
            _ => {}
        }
    }
    call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    framed_intro_call(
        context,
        owner,
        0x196,
        ids::INTRO_RELEASE_FRAME_ENTITIES_AND_REFRESH,
        &[],
    )?;
    framed_intro_cadence(context, owner, 0x1a0, &[0, 0])?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank2(revision).address(), 2],
    )?;
    scene_result(context, state)
}

/// G/src/cutscenes/intro.c:383 (`IntroPart3`).
/// TimeAware: shared frame/music service and released-button conversation.
/// Caption labels _L191.._L193 have the same offsets in both revision objects.
pub fn intro_part3(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    let actor = ram::g_cutscene_music_actor_animation_enabled(revision);
    let advance_mask = ram::g_cutscene_advance_button_mask(revision);
    let owner = ids::INTRO_PART3;
    framed_intro_call(
        context,
        owner,
        0x1e,
        ids::INTRO_BEGIN_LARGE_PLAYER_SCENE,
        &[7],
    )?;
    clear_byte(
        context,
        ram::g_requested_music_selection(revision).address(),
    )?;
    countdown.write(context, 17);
    actor.write(context, 1);
    let mut advance_scene = false;
    clear_word(context, state)?;
    while state.read(context) != -1 && !start_pressed(context)? {
        loop {
            let enabled = actor.read(context) as i32 as u32;
            framed_intro_cadence(context, owner, 0x4c, &[0, enabled])?;
            let mask = action_buttons(context)?;
            advance_mask.write(context, mask);
            if mask != 0 {
                advance_scene = true;
            }
            if advance_mask.read(context) == 0 {
                break;
            }
        }
        if !advance_scene && decrement(context, countdown) > 0 {
            continue;
        }
        increment(context, state);
        actor.write(context, 1);
        let next = state.read(context);
        read_switch_entry(context, owner, 0xa4, 8, next);
        match next {
            1 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        5,
                        rom::intro_large_player_part_five_frame_pulse(revision).address(),
                        3,
                    ],
                )?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        6,
                        rom::intro_large_player_part_six_held_frame_fifteen(revision).address(),
                        6,
                    ],
                )?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_ascending_frames(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 17);
            }
            2 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        6,
                        rom::intro_large_player_part_six_held_frame_fourteen(revision).address(),
                        6,
                    ],
                )?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_seventeen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 17);
            }
            3 => {
                advance_scene = false;
                caption(context, owner, 0x232, 220, 72, false)?;
                caption(context, owner, 0x248, 220, 84, true)?;
                countdown.write(context, 85);
            }
            4 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        5,
                        rom::intro_large_player_part_five_ascending_frames(revision).address(),
                        3,
                    ],
                )?;
                countdown.write(context, 34);
            }
            5 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_large_player_part_one_held_frame_two(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 17);
            }
            6 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_large_player_part_one_held_frame_three(revision).address(),
                        6,
                    ],
                )?;
                advance_scene = false;
                caption(context, owner, 0x258, 78, 54, false)?;
                countdown.write(context, 59);
            }
            7 => {
                call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
                countdown.write(context, 8);
            }
            8 => {
                state.write(context, -1);
            }
            _ => {}
        }
    }
    set_display(context, false);
    call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    framed_intro_call(
        context,
        owner,
        0x208,
        ids::INTRO_RELEASE_LARGE_PLAYER_SPRITES_AND_REFRESH,
        &[],
    )?;
    call_void(context, ids::INTRO_RELEASE_LARGE_PLAYER_PARTS, &[])?;
    framed_intro_cadence(context, owner, 0x218, &[0, 0])?;
    scene_result(context, state)
}

/// G/src/cutscenes/intro.c:455 (`IntroPart4`).
/// TimeAware: two VBlank waits per inner step, parallax, input and music.
/// Captions use _L221.._L225's function-relative compiler-label offsets, equal
/// in both matching revisions; all text remains in the user's ROM.
pub fn intro_part4(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    let advance_mask = ram::g_cutscene_advance_button_mask(revision);
    let owner = ids::INTRO_PART4;
    call_void(context, ids::CUTSCENE_CREATE_DRIFTING_ENTITIES, &[])?;
    call_void(context, ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR, &[])?;
    call_void(context, ids::DRAW_STARFIELD_PLANE, &[])?;
    framed_intro_call(
        context,
        owner,
        0x32,
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
    set_display(context, true);
    countdown.write(context, 85);
    clear_word(context, state)?;
    let mut scroll_position = 0_i16;
    let mut exhaust_phase = 0_i16;
    while state.read(context) != -1 && !start_pressed(context)? {
        loop {
            call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
            context.write_vdp_command(0x7c02_0003);
            context.write_vdp_data((scroll_position / 4) as u16);
            call_void(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
            framed_intro_call(
                context,
                owner,
                0xd0,
                ids::SELECT_VIEWPORT_SPRITE_CHAIN,
                &[0],
            )?;
            call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
            call_void(
                context,
                ids::CUTSCENE_UPDATE_DRIFTING_ENTITIES,
                &[scroll_position as i32 as u32],
            )?;
            exhaust_phase ^= 1;
            if exhaust_phase != 0 {
                call_void(
                    context,
                    ids::CUTSCENE_MOVE_ROCKETSHIP_PAIR_LEFT,
                    &[exhaust_phase as i32 as u32],
                )?;
            }
            call_void(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
            scroll_position = scroll_position.wrapping_add(1);
            let mask = action_buttons(context)?;
            advance_mask.write(context, mask);
            if mask != 0 {
                clear_word(context, countdown)?;
            }
            if advance_mask.read(context) == 0 {
                break;
            }
        }
        if decrement(context, countdown) > 0 {
            continue;
        }
        countdown.write(context, 85);
        let next = increment_and_reload(context, state);
        read_switch_entry(context, owner, 0x156, 5, next);
        match next {
            1 => {
                caption(context, owner, 0x238, 160, 30, false)?;
            }
            2 => {
                caption(context, owner, 0x258, 160, 30, false)?;
            }
            3 => {
                caption(context, owner, 0x272, 160, 30, false)?;
                caption(context, owner, 0x286, 160, 40, true)?;
                countdown.write(context, 119);
            }
            4 => {
                caption(context, owner, 0x298, 160, 30, false)?;
            }
            5 => {
                state.write(context, -1);
            }
            _ => {}
        }
    }
    set_display(context, false);
    call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    call_void(context, ids::CUTSCENE_RELEASE_DRIFTING_ENTITIES, &[])?;
    call_void(context, ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR, &[])?;
    framed_intro_cadence(context, owner, 0x21e, &[0, 0])?;
    scene_result(context, state)
}

/// G/src/cutscenes/intro.c:35-44,71-80: zero scroll mirrors before each port
/// transaction, keeping X/Y read order and both planes' distinct command words.
fn reset_scroll(context: &mut dyn Context) -> Result<()> {
    let revision = context.revision();
    let x = ram::g_camera1_pending_x(revision);
    let y = ram::g_camera1_pending_y(revision);
    clear_word(context, x)?;
    clear_word(context, y)?;
    context.write_vdp_command(0x4000_0010);
    context.write_vdp_data(y.read(context) as u16);
    context.write_vdp_command(0x7c00_0003);
    context.write_vdp_data(x.read(context) as u16);
    context.write_vdp_command(0x4002_0010);
    context.write_vdp_data(y.read(context) as u16);
    context.write_vdp_command(0x7c02_0003);
    context.write_vdp_data(x.read(context) as u16);
    Ok(())
}

/// G/src/cutscenes/intro.c:26 (`IntroVideo`).
/// TimeAware: ordered scene calls short-circuit on interruption, with real waits
/// before display transitions and final cleanup. Font/palette data stays in ROM.
pub fn intro_video(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    clear_byte(
        context,
        ram::g_requested_music_selection(revision).address(),
    )?;
    call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    set_display(context, false);
    clear_byte(context, ram::g_music_animation_channel(revision).address())?;
    reset_scroll(context)?;
    call_void(context, ids::CLEAR_UPPER_VRAM_TILEMAP, &[])?;
    ram::g_camera_tile_attributes(revision).write(context, 0x8000);
    context.write_vdp_command(0x7000_0002);
    let size = i32::from(rom::font_data_size(revision).read(context)).wrapping_shl(4);
    call_void(
        context,
        ids::MOVE_DATA_TO_VRAM,
        &[rom::image_text_base(revision).address(), size as u32],
    )?;
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
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank2(revision).address(), 2],
    )?;
    call_void(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank3(revision).address(), 3],
    )?;
    call_void(context, ids::INITIALIZE_TITLE_TRANSITION_TILEMAP, &[])?;
    framed_intro_call(
        context,
        ids::INTRO_VIDEO,
        0x102,
        ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
        &[0],
    )?;
    call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    set_display(context, true);
    let mut interrupted = false;
    for (part, offset) in [
        (ids::INTRO_PART1, 0x128),
        (ids::INTRO_PART2, 0x134),
        (ids::INTRO_PART3, 0x13e),
        (ids::INTRO_PART4, 0x148),
        (ids::INTRO_PART5, 0x152),
        (ids::INTRO_PART6, 0x15c),
        (ids::INTRO_PART7, 0x166),
        (ids::INTRO_PART8, 0x170),
        (ids::INTRO_PART9, 0x17a),
        (ids::INTRO_PART10, 0x184),
        (ids::INTRO_PART11, 0x190),
    ] {
        let site = CallSite::relative(ids::INTRO_VIDEO, revision, offset);
        if call_at(context, part, &[], site)? as i16 != 0 {
            interrupted = true;
            break;
        }
    }
    call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    set_display(context, false);
    call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    call_void(context, ids::CLEAR_UPPER_VRAM_TILEMAP, &[])?;
    call_void(context, ids::CLEAR_LOWER_VRAM_TILEMAP, &[])?;
    call_void(context, ids::CUTSCENE_RELEASE_FLAGGED_ENTITY_RANGE, &[])?;
    reset_scroll(context)?;
    let status = context.status_register();
    context.set_status_register((status & !0x000f) | if interrupted { 0 } else { 0x04 });
    Ok(Some(u32::from(interrupted)))
}

#[cfg(test)]
#[path = "intro_access_tests.rs"]
mod access_tests;

#[cfg(test)]
mod tests {
    use super::decrement;
    use tje_mem::{Address, AddressSpace, Revision};

    #[test]
    fn countdown_predecrement_matches_subq_word_flags_for_every_value() {
        let counter = Address::<i16>::new(0xff0100);
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = AddressSpace::new(Vec::new(), revision);
            for bits in 0..=u16::MAX {
                let old = bits as i16;
                counter.write(&mut memory, old);
                let comparison = decrement(&mut memory, counter);
                let stored = old.wrapping_sub(1);
                let negative = stored < 0;
                let overflow = old == i16::MIN;
                let zero = stored == 0;
                let bgt = !zero && negative == overflow;
                assert_eq!(counter.read(&memory), stored);
                assert_eq!(comparison > 0, bgt, "{revision:?} {old}");
                assert_eq!(comparison == 0, zero, "{revision:?} {old}");
                assert_eq!(comparison != 0, !zero, "{revision:?} {old}");
            }
        }
    }
}
