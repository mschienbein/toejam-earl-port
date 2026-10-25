use super::intro::{action_buttons, caption, decrement, increment, set_display, start_pressed};
use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, call_at, call_at_void, call_void, ids};
use tje_mem::{ram, rom};

/// G/src/cutscenes/intro_part11.c:16 (`IntroPart11`).
/// TimeAware: timed/released-button dialogue with real VBlank/music service.
/// Caption pointers are matching compiler labels _L49.._L61, relative to this
/// function, with identical offsets in both revision c_intro_part11.o objects.
pub fn intro_part11(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    // c_intro_part11.s:4: MOVEM.L D2/D3/A2/A3/A4,-(SP).
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x1c))?;
    let result = intro_part11_body(context).map_err(|error| context.protect_timing_failure(error));
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

fn intro_part11_body(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let dispatcher = context.dispatcher();
    let state = ram::g_cutscene_script_state(revision);
    let countdown = ram::g_cutscene_script_countdown(revision);
    let actor = ram::g_cutscene_music_actor_animation_enabled(revision);
    let advance_mask = ram::g_cutscene_advance_button_mask(revision);
    let owner = ids::INTRO_PART11;
    context.set_address_register(2, countdown.address());
    context.set_address_register(
        3,
        ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE
            .metadata()
            .address(revision),
    );
    context.set_address_register(4, ids::CUTSCENE_TEXT_LOAD.metadata().address(revision));
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
    call_void(context, ids::INTRO_ALLOCATE_LARGE_PLAYER_PARTS, &[])?;
    ram::g_scene_background_pair_index(revision).write(context, 7);
    call_at_void(
        context,
        ids::INITIALIZE_TITLE_TRANSITION_TILEMAP,
        &[],
        CallSite::relative(owner, revision, 0x4e),
    )?;
    // The border's slot remains below Wait's slot through the mode publication.
    let border = OutgoingArguments::push(context, &[0])?;
    border.call_void(
        context,
        dispatcher,
        ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
        CallSite::relative(owner, revision, 0x56),
    )?;
    call_void(context, ids::INTRO_POSITION_LARGE_PLAYER_PARTS, &[])?;
    let wait = OutgoingArguments::push(context, &[1])?;
    wait.call_void(
        context,
        dispatcher,
        ids::WAIT_FOR_NEXT_V_BLANK,
        CallSite::relative(owner, revision, 0x66),
    )?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    let enabled_mode = mode.read(context) | 0x40;
    mode.write(context, enabled_mode);
    let mode_value = mode.read(context);
    wait.release(context)?;
    border.release(context)?;
    context.write_device_u16(0xc00004, u16::from(mode_value) | 0x8104);
    let mut advance_pressed = false;
    context.discard_u16(state.address())?;
    state.write(context, 0);
    countdown.write(context, 51);
    actor.write(context, 1);
    while state.read(context) != -1 && !start_pressed(context)? {
        loop {
            let enabled = actor.read(context) as i32 as u32;
            let cadence = OutgoingArguments::push(context, &[0, enabled])?;
            cadence.call(
                context,
                dispatcher,
                ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
                CallSite::relative(owner, revision, 0xae),
            )?;
            let mask = action_buttons(context)?;
            // c_intro_part11.s:40-47 retains both slots through both controller calls.
            cadence.release(context)?;
            advance_mask.write(context, mask);
            if mask != 0 {
                advance_pressed = true;
            }
            if advance_mask.read(context) == 0 {
                break;
            }
        }
        if !advance_pressed && decrement(context, countdown) > 0 {
            continue;
        }
        increment(context, state);
        actor.write(context, 1);
        match state.read(context) {
            1 => {
                advance_pressed = false;
                caption(context, owner, 0x556, 220, 80, false)?;
                countdown.write(context, 42);
            }
            2 => {
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
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        5,
                        rom::intro_large_player_part_five_frame_pulse(revision).address(),
                        3,
                    ],
                )?;
                countdown.write(context, 25);
            }
            4 => {
                advance_pressed = false;
                caption(context, owner, 0x56a, 220, 72, false)?;
                caption(context, owner, 0x57a, 220, 84, true)?;
                countdown.write(context, 34);
            }
            5 => {
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
                        rom::intro_large_player_part_seven_held_frame_eighteen(revision).address(),
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
                countdown.write(context, 34);
            }
            7 => {
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
            8 => {
                advance_pressed = false;
                caption(context, owner, 0x592, 220, 80, false)?;
                countdown.write(context, 68);
            }
            9 => {
                advance_pressed = false;
                caption(context, owner, 0x5a2, 220, 80, false)?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_seventeen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 51);
            }
            10 => {
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
                countdown.write(context, 34);
            }
            11 => {
                advance_pressed = false;
                caption(context, owner, 0x5b0, 220, 80, false)?;
                countdown.write(context, 34);
            }
            12 => {
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
            13 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_large_player_part_one_held_frame_two(revision).address(),
                        6,
                    ],
                )?;
                call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
                countdown.write(context, 17);
            }
            14 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_large_player_part_one_held_frame_nineteen(revision).address(),
                        6,
                    ],
                )?;
                call_void(
                    context,
                    ids::INTRO_POSITION_LARGE_PLAYER_PART,
                    &[3, 148, 112],
                )?;
                countdown.write(context, 8);
            }
            15 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        3,
                        rom::intro_large_player_part_three_loop_animation(revision).address(),
                        3,
                    ],
                )?;
                context.discard_u16(actor.address())?;
                actor.write(context, 0);
                countdown.write(context, 42);
            }
            16 => {
                call_void(context, ids::INTRO_RELEASE_LARGE_PLAYER_PART_SPRITES, &[3])?;
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
            17 => {
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
            18 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        5,
                        rom::intro_large_player_part_five_descending_frames(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 17);
            }
            19 => {
                advance_pressed = false;
                caption(context, owner, 0x5c6, 220, 80, false)?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_eighteen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 85);
            }
            20 => {
                advance_pressed = false;
                caption(context, owner, 0x5de, 220, 72, false)?;
                caption(context, owner, 0x5f4, 220, 84, true)?;
                countdown.write(context, 25);
            }
            21 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_seventeen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 25);
            }
            22 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        6,
                        rom::intro_large_player_part_six_held_frame_fifteen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 17);
            }
            23 => {
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
                        rom::intro_large_player_part_seven_held_frame_eighteen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 17);
            }
            24 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        7,
                        rom::intro_large_player_part_seven_held_frame_seventeen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 25);
            }
            25 => {
                call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
                countdown.write(context, 17);
            }
            26 => {
                advance_pressed = false;
                caption(context, owner, 0x608, 220, 72, false)?;
                caption(context, owner, 0x614, 220, 84, true)?;
                countdown.write(context, 34);
            }
            27 => {
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        5,
                        rom::intro_large_player_part_five_frame_pulse(revision).address(),
                        3,
                    ],
                )?;
                countdown.write(context, 68);
            }
            28 => {
                advance_pressed = false;
                caption(context, owner, 0x624, 220, 80, false)?;
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
                        rom::intro_large_player_part_seven_held_frame_eighteen(revision).address(),
                        6,
                    ],
                )?;
                countdown.write(context, 17);
            }
            29 => {
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
                countdown.write(context, 34);
            }
            30 => {
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
            31 => {
                advance_pressed = false;
                caption(context, owner, 0x630, 78, 54, false)?;
                call_void(
                    context,
                    ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE,
                    &[
                        1,
                        rom::intro_large_player_part_one_held_frame_three(revision).address(),
                        6,
                    ],
                )?;
                context.discard_u16(actor.address())?;
                actor.write(context, 0);
                countdown.write(context, 85);
            }
            32 => {
                state.write(context, -1);
            }
            _ => {}
        }
    }
    set_display(context, false);
    call_void(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    call_at_void(
        context,
        ids::INTRO_RELEASE_LARGE_PLAYER_SPRITES_AND_REFRESH,
        &[],
        CallSite::relative(owner, revision, 0x528),
    )?;
    call_void(context, ids::INTRO_RELEASE_LARGE_PLAYER_PARTS, &[])?;
    call_at(
        context,
        ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
        &[0, 0],
        CallSite::relative(owner, revision, 0x538),
    )?;
    let interrupted = state.read(context) != -1;
    // c_intro_part11.s:451-458: SNE; NEG.B; EXT.W/L leaves X on an abort,
    // or Z on completion. The final EXT clears V/C and retains that X.
    let flags = if interrupted { 0x10 } else { 4 };
    context.set_status_register((context.status_register() & !0x1f) | flags);
    Ok(Some(u32::from(interrupted)))
}
