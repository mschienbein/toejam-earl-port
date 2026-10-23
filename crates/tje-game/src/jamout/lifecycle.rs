use super::entities::set_data_word;
use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, call, call_at, ids};
use tje_mem::{Ptr, ram, rom, rom_symbol};

fn pads(context: &mut dyn Context) -> Result<u32> {
    // MCC68K evaluates the right operand of the source bitwise OR first.
    let second = call(context, ids::GET_CTRL2_INPUT, &[])?;
    context.set_data_register(2, second);
    let first = call(context, ids::GET_CTRL1_INPUT, &[])?;
    let combined = first | second;
    context.set_data_register(2, combined);
    Ok(combined)
}

fn consume_animation_input(context: &mut dyn Context, player: i32) -> Result<()> {
    let revision = context.revision();
    let (pending, lock, set_sequence) = if player == 0 {
        (
            ram::g_jam_player1_pending_animation_input(revision),
            ram::g_jam_toejam_animation_locked(revision),
            ids::JAM_OUT_SET_TOEJAM_SEQUENCE,
        )
    } else {
        (
            ram::g_jam_player2_pending_animation_input(revision),
            ram::g_jam_earl_animation_locked(revision),
            ids::JAM_OUT_SET_EARL_SEQUENCE,
        )
    };
    if pending.read(context) != 0 {
        if pending.read(context) & 0x40 != 0 {
            let sequence = if call(context, ids::GET_RANDOM_NUMBER, &[])? & 1 != 0 {
                rom::jam_out_character_animations(revision).address()
            } else {
                rom::jam_bass_drum_alternate_sequence(revision).address()
            };
            call(context, set_sequence, &[sequence, 6])?;
        }
        if pending.read(context) & 0x10 != 0 {
            let sequence = if call(context, ids::GET_RANDOM_NUMBER, &[])? & 1 != 0 {
                rom::jam_snare_drum_second_sequence(revision).address()
            } else {
                rom::jam_snare_drum_first_sequence(revision).address()
            };
            call(context, set_sequence, &[sequence, 6])?;
        }
        if pending.read(context) & 0x20 != 0 {
            call(
                context,
                set_sequence,
                &[rom::jam_clap_sequence(revision).address(), 3],
            )?;
            lock.write(context, 1);
        }
        if pending.read(context) & 0x0f != 0 || pending.read(context) & 0x80 != 0 {
            call(
                context,
                set_sequence,
                &[rom::jam_vocal_or_scratch_sequence(revision).address(), 2],
            )?;
            lock.write(context, 1);
        }
        pending.write(context, 0);
    }
    Ok(())
}

/// G/src/jamout/jamout.c:30 (`StartJamOut`, REV 02 0x03A7A4).
/// Preserve controller-release waiting, source-ordered resource setup, callback
/// ownership, six independent random draws, 51 consecutive Start updates, input
/// consumption order, and every exit-side teardown store/call. Text stays in ROM.
fn start_jam_out_inner(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut song = arguments[0] as i16;
    context.set_address_register(
        2,
        ram::g_jam_player1_pending_animation_input(revision).address(),
    );
    context.set_address_register(
        3,
        ram::g_jam_player2_pending_animation_input(revision).address(),
    );
    context.set_address_register(4, ids::GET_RANDOM_NUMBER.metadata().address(revision));
    set_data_word(context, 3, song);
    while pads(context)? != 0 {
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    }
    let mode = ram::g_mode2_reg_softcopy(revision).read(context) & 0x3c;
    ram::g_mode2_reg_softcopy(revision).write(context, mode);
    context.write_vdp_control(0x8104 | u16::from(mode));
    call(
        context,
        ids::PLAY_SOUND_EFFECT,
        &[rom::sfx_jammin(revision).address(), 90, 0],
    )?;
    ram::g_jam_selected_song(revision).write(context, song);
    ram::g_v_blank_func(revision).write(
        context,
        Ptr::new(ids::JAM_OUT_V_BLANK.metadata().address(revision)),
    );
    ram::g_music_alternate_template_bank_enabled(revision).write(context, 1);
    context.write_vdp_command(0x46600001);
    let length = (i32::from(rom::crash_screen_size(revision).read(context)) as u32) << 4;
    context.set_data_register(2, length);
    // The original retains all three outgoing pairs until one LEA 24(SP),SP.
    // In particular the palette-one index leaves the word later reused by the
    // creator's uninitialized -2(A6) local; never erase or invent that residue.
    let dispatcher = context.dispatcher();
    let image = OutgoingArguments::push(context, &[rom::crash_screen(revision).address(), length])?;
    image.call(
        context,
        dispatcher,
        ids::MOVE_DATA_TO_VRAM,
        CallSite::relative(ids::START_JAM_OUT, revision, 0xaa),
    )?;
    let first_palette = OutgoingArguments::push(
        context,
        &[rom::palette_title_screen_bank1(revision).address(), 1],
    )?;
    first_palette.call(
        context,
        dispatcher,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        CallSite::relative(ids::START_JAM_OUT, revision, 0xba),
    )?;
    let last_palette =
        OutgoingArguments::push(context, &[rom::palette_game_line4(revision).address(), 3])?;
    last_palette.call(
        context,
        dispatcher,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        CallSite::relative(ids::START_JAM_OUT, revision, 0xca),
    )?;
    last_palette.release(context)?;
    first_palette.release(context)?;
    image.release(context)?;
    ram::g_shared_tile_sprite_copies_enabled(revision).write(context, 1);
    call_at(
        context,
        ids::JAM_OUT_CREATE_CHARACTER_COPIES,
        &[],
        CallSite::relative(ids::START_JAM_OUT, revision, 0xda),
    )?;
    context.write_vdp_control(0x8c89);
    ram::g_camera_tile_attributes(revision).write(context, 0);
    call(context, ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL, &[1])?;
    let mut start_hold = 0_i16;
    set_data_word(context, 4, start_hold);
    if song == 7 {
        for index in [
            ram::g_jam_audio_route1_button_a_sample_index(revision),
            ram::g_jam_audio_route1_button_b_sample_index(revision),
            ram::g_jam_audio_route1_button_c_sample_index(revision),
            ram::g_jam_audio_route2_button_a_sample_index(revision),
            ram::g_jam_audio_route2_button_b_sample_index(revision),
            ram::g_jam_audio_route2_button_c_sample_index(revision),
        ] {
            let value = call(context, ids::GET_RANDOM_NUMBER, &[])? as i32;
            let remainder = value % 42;
            context.set_data_register(2, remainder as u32);
            index.write(context, remainder as u16);
        }
        ram::g_v_blank_func(revision).write(
            context,
            Ptr::new(
                ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK
                    .metadata()
                    .address(revision),
            ),
        );
    }
    if song != 0 {
        song = song.wrapping_sub(1);
        set_data_word(context, 3, song);
        call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
        ram::g_current_music_selection(revision).write(context, i8::from(song == 0));
        ram::g_requested_music_selection(revision).write(context, song as i8);
    } else {
        call(context, ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, &[])?;
        call(context, ids::MUSIC_REQUEST_TRACK_POINTER_RESET, &[])?;
    }
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let mode = ram::g_mode2_reg_softcopy(revision).read(context) | 0x40;
    ram::g_mode2_reg_softcopy(revision).write(context, mode);
    context.write_vdp_control(0x8104 | u16::from(mode));
    let text = rom_symbol(revision, "PressAndHoldStartToExitStr")
        .expect("generated Jam Out exit-text symbol")
        .address;
    call(context, ids::CUTSCENE_TEXT_LOAD, &[text, 160, 30])?;
    while start_hold < 51 {
        call(context, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[3, 0])?;
        call(context, ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL, &[0])?;
        call(context, ids::JAM_OUT_UPDATE_LEAD_ANIMATIONS, &[])?;
        if pads(context)? & 0x80 != 0 {
            start_hold = start_hold.wrapping_add(1);
        } else {
            start_hold = 0;
        }
        set_data_word(context, 4, start_hold);
        consume_animation_input(context, 0)?;
        consume_animation_input(context, 1)?;
    }
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let mode = ram::g_mode2_reg_softcopy(revision).read(context) & 0x3c;
    ram::g_mode2_reg_softcopy(revision).write(context, mode);
    context.write_vdp_control(0x8104 | u16::from(mode));
    ram::g_v_blank_func(revision).write(context, Ptr::new(0));
    ram::g_music_alternate_template_bank_enabled(revision).write(context, 0);
    call(context, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[])?;
    call(context, ids::CLEAR_UPPER_VRAM_TILEMAP, &[])?;
    call(context, ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL, &[1])?;
    call(context, ids::CLEAR_LOWER_VRAM_TILEMAP, &[])?;
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    ram::g_current_music_selection(revision).write(context, 1);
    ram::g_requested_music_selection(revision).write(context, 0);
    call(context, ids::JAM_OUT_RELEASE_CHARACTER_COPIES, &[])?;
    ram::g_shared_tile_sprite_copies_enabled(revision).write(context, 0);
    call(context, ids::CUTSCENE_RELEASE_FLAGGED_ENTITY_RANGE, &[])?;
    Ok(None)
}

/// G/src/jamout/jamout.c:30 (`StartJamOut`) and matching prologue:
/// MOVEM D2-D4/A2-A4, no LINK. Keep the actual caller frame/outgoing residue
/// needed by the character constructor and restore it on the normal RTS path.
pub fn start_jam_out(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x1c, 0x1c))?;
    let result = start_jam_out_inner(context, arguments);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let cleanup = frame.leave(context);
    match (result, cleanup) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}
