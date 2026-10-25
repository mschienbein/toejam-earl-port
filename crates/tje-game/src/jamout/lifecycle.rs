use super::entities::set_data_word;
use super::frame_support::{
    clear_long, clear_word, in_frame, random_sample_index, release_arguments, retained_call,
    retained_call_void, source_call, source_call_void,
};
use crate::call_frame::{CallSite, FrameLayout, OutgoingArguments};
use crate::{Context, Result, call_at_void, ids};
use tje_mem::{Ptr, ram, rom, rom_symbol};

fn pads(context: &mut dyn Context, second_offset: u32, first_offset: u32) -> Result<u32> {
    // MCC68K evaluates the right operand of the source bitwise OR first.
    let second = source_call(
        context,
        ids::START_JAM_OUT,
        second_offset,
        ids::GET_CTRL2_INPUT,
        &[],
    )?;
    context.set_data_register(2, second);
    let first = source_call(
        context,
        ids::START_JAM_OUT,
        first_offset,
        ids::GET_CTRL1_INPUT,
        &[],
    )?;
    let combined = first | second;
    context.set_data_register(2, combined);
    Ok(combined)
}

fn consume_animation_input(context: &mut dyn Context, player: i32) -> Result<()> {
    let revision = context.revision();
    let shift = if player == 0 { 0 } else { 0xb2 };
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
        if pending.byte_offset(1).cast::<u8>().read(context) & 0x40 != 0 {
            let sequence = if source_call(
                context,
                ids::START_JAM_OUT,
                0x27a + shift,
                ids::GET_RANDOM_NUMBER,
                &[],
            )? & 1
                != 0
            {
                rom::jam_out_character_animations(revision).address()
            } else {
                rom::jam_bass_drum_alternate_sequence(revision).address()
            };
            source_call_void(
                context,
                ids::START_JAM_OUT,
                0x29a + shift,
                set_sequence,
                &[sequence, 6],
            )?;
        }
        if pending.byte_offset(1).cast::<u8>().read(context) & 0x10 != 0 {
            let sequence = if source_call(
                context,
                ids::START_JAM_OUT,
                0x2a8 + shift,
                ids::GET_RANDOM_NUMBER,
                &[],
            )? & 1
                != 0
            {
                rom::jam_snare_drum_second_sequence(revision).address()
            } else {
                rom::jam_snare_drum_first_sequence(revision).address()
            };
            source_call_void(
                context,
                ids::START_JAM_OUT,
                0x2c8 + shift,
                set_sequence,
                &[sequence, 6],
            )?;
        }
        if pending.byte_offset(1).cast::<u8>().read(context) & 0x20 != 0 {
            source_call_void(
                context,
                ids::START_JAM_OUT,
                0x2e2 + shift,
                set_sequence,
                &[rom::jam_clap_sequence(revision).address(), 3],
            )?;
            lock.write(context, 1);
        }
        if pending.read(context) & 0x0f != 0
            || pending.byte_offset(1).cast::<u8>().read(context) & 0x80 != 0
        {
            source_call_void(
                context,
                ids::START_JAM_OUT,
                0x30e + shift,
                set_sequence,
                &[rom::jam_vocal_or_scratch_sequence(revision).address(), 2],
            )?;
            lock.write(context, 1);
        }
        clear_word(context, pending.address())?;
    }
    Ok(())
}

/// G/src/jamout/jamout.c:30 (`StartJamOut`, REV 02 0x03A7A4).
/// Preserve controller-release waiting, source-ordered resource setup, callback
/// ownership, six independent random draws, 51 consecutive Start updates, input
/// consumption order, and every exit-side teardown store/call. Text stays in ROM.
fn start_jam_out_inner(context: &mut dyn Context, mut song: i16) -> Result<Option<u32>> {
    let revision = context.revision();
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
    while pads(context, 0x2e, 0x36)? != 0 {
        source_call_void(
            context,
            ids::START_JAM_OUT,
            0x26,
            ids::WAIT_FOR_NEXT_V_BLANK,
            &[1],
        )?;
    }
    let mode = ram::g_mode2_reg_softcopy(revision).read(context) & 0x3c;
    ram::g_mode2_reg_softcopy(revision).write(context, mode);
    let mode = ram::g_mode2_reg_softcopy(revision).read(context);
    context.write_vdp_control(0x8104 | u16::from(mode));
    source_call_void(
        context,
        ids::START_JAM_OUT,
        0x6a,
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
    image.call_void(
        context,
        dispatcher,
        ids::MOVE_DATA_TO_VRAM,
        CallSite::relative(ids::START_JAM_OUT, revision, 0xaa),
    )?;
    let first_palette = OutgoingArguments::push(
        context,
        &[rom::palette_title_screen_bank1(revision).address(), 1],
    )?;
    first_palette.call_void(
        context,
        dispatcher,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        CallSite::relative(ids::START_JAM_OUT, revision, 0xba),
    )?;
    let last_palette =
        OutgoingArguments::push(context, &[rom::palette_game_line4(revision).address(), 3])?;
    last_palette.call_void(
        context,
        dispatcher,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        CallSite::relative(ids::START_JAM_OUT, revision, 0xca),
    )?;
    last_palette.release(context)?;
    first_palette.release(context)?;
    image.release(context)?;
    ram::g_shared_tile_sprite_copies_enabled(revision).write(context, 1);
    call_at_void(
        context,
        ids::JAM_OUT_CREATE_CHARACTER_COPIES,
        &[],
        CallSite::relative(ids::START_JAM_OUT, revision, 0xda),
    )?;
    context.write_vdp_control(0x8c89);
    clear_word(context, ram::g_camera_tile_attributes(revision).address())?;
    source_call_void(
        context,
        ids::START_JAM_OUT,
        0xf2,
        ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL,
        &[1],
    )?;
    let mut start_hold = 0_i16;
    set_data_word(context, 4, start_hold);
    if song == 7 {
        for (position, index) in [
            ram::g_jam_audio_route1_button_a_sample_index(revision),
            ram::g_jam_audio_route1_button_b_sample_index(revision),
            ram::g_jam_audio_route1_button_c_sample_index(revision),
            ram::g_jam_audio_route2_button_a_sample_index(revision),
            ram::g_jam_audio_route2_button_b_sample_index(revision),
            ram::g_jam_audio_route2_button_c_sample_index(revision),
        ]
        .into_iter()
        .enumerate()
        {
            let remainder =
                random_sample_index(context, ids::START_JAM_OUT, 0x102 + 0x1c * position as u32)?;
            context.set_data_register(2, i32::from(remainder as i16) as u32);
            index.write(context, remainder);
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
        source_call_void(
            context,
            ids::START_JAM_OUT,
            0x1c4,
            ids::WAIT_FOR_NEXT_V_BLANK,
            &[1],
        )?;
        ram::g_current_music_selection(revision).write(context, i8::from(song == 0));
        ram::g_requested_music_selection(revision).write(context, song as i8);
    } else {
        source_call_void(
            context,
            ids::START_JAM_OUT,
            0x1e4,
            ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE,
            &[],
        )?;
        source_call_void(
            context,
            ids::START_JAM_OUT,
            0x1ea,
            ids::MUSIC_REQUEST_TRACK_POINTER_RESET,
            &[],
        )?;
    }
    retained_call_void(
        context,
        ids::START_JAM_OUT,
        0x1f4,
        ids::WAIT_FOR_NEXT_V_BLANK,
        &[1],
    )?;
    let mode = ram::g_mode2_reg_softcopy(revision).read(context) | 0x40;
    ram::g_mode2_reg_softcopy(revision).write(context, mode);
    let mode = ram::g_mode2_reg_softcopy(revision).read(context);
    release_arguments(context, 1)?;
    context.write_vdp_control(0x8104 | u16::from(mode));
    let text = rom_symbol(revision, "PressAndHoldStartToExitStr")
        .expect("generated Jam Out exit-text symbol")
        .address;
    source_call_void(
        context,
        ids::START_JAM_OUT,
        0x228,
        ids::CUTSCENE_TEXT_LOAD,
        &[text, 160, 30],
    )?;
    while start_hold < 51 {
        retained_call(
            context,
            ids::START_JAM_OUT,
            0x23c,
            ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
            &[3, 0],
        )?;
        retained_call_void(
            context,
            ids::START_JAM_OUT,
            0x242,
            ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL,
            &[0],
        )?;
        source_call_void(
            context,
            ids::START_JAM_OUT,
            0x248,
            ids::JAM_OUT_UPDATE_LEAD_ANIMATIONS,
            &[],
        )?;
        let input = pads(context, 0x250, 0x258)?;
        release_arguments(context, 3)?;
        if input & 0x80 != 0 {
            start_hold = start_hold.wrapping_add(1);
        } else {
            start_hold = 0;
        }
        set_data_word(context, 4, start_hold);
        consume_animation_input(context, 0)?;
        consume_animation_input(context, 1)?;
    }
    retained_call_void(
        context,
        ids::START_JAM_OUT,
        0x3e0,
        ids::WAIT_FOR_NEXT_V_BLANK,
        &[1],
    )?;
    let mode = ram::g_mode2_reg_softcopy(revision).read(context) & 0x3c;
    ram::g_mode2_reg_softcopy(revision).write(context, mode);
    let mode = ram::g_mode2_reg_softcopy(revision).read(context);
    release_arguments(context, 1)?;
    context.write_vdp_control(0x8104 | u16::from(mode));
    clear_long(context, ram::g_v_blank_func(revision).address())?;
    clear_word(
        context,
        ram::g_music_alternate_template_bank_enabled(revision).address(),
    )?;
    source_call_void(
        context,
        ids::START_JAM_OUT,
        0x412,
        ids::CUTSCENE_RELEASE_TEXT_ENTITIES,
        &[],
    )?;
    source_call_void(
        context,
        ids::START_JAM_OUT,
        0x418,
        ids::CLEAR_UPPER_VRAM_TILEMAP,
        &[],
    )?;
    retained_call_void(
        context,
        ids::START_JAM_OUT,
        0x420,
        ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL,
        &[1],
    )?;
    source_call_void(
        context,
        ids::START_JAM_OUT,
        0x428,
        ids::CLEAR_LOWER_VRAM_TILEMAP,
        &[],
    )?;
    retained_call_void(
        context,
        ids::START_JAM_OUT,
        0x432,
        ids::WAIT_FOR_NEXT_V_BLANK,
        &[1],
    )?;
    release_arguments(context, 2)?;
    ram::g_current_music_selection(revision).write(context, 1);
    context.discard_u8(ram::g_requested_music_selection(revision).address())?;
    ram::g_requested_music_selection(revision).write(context, 0);
    source_call_void(
        context,
        ids::START_JAM_OUT,
        0x446,
        ids::JAM_OUT_RELEASE_CHARACTER_COPIES,
        &[],
    )?;
    clear_word(
        context,
        ram::g_shared_tile_sprite_copies_enabled(revision).address(),
    )?;
    source_call_void(
        context,
        ids::START_JAM_OUT,
        0x454,
        ids::CUTSCENE_RELEASE_FLAGGED_ENTITY_RANGE,
        &[],
    )?;
    Ok(None)
}

/// G/src/jamout/jamout.c:30 (`StartJamOut`) and matching prologue:
/// MOVEM D2-D4/A2-A4, no LINK. Keep the actual caller frame/outgoing residue
/// needed by the character constructor and restore it on the normal RTS path.
pub fn start_jam_out(context: &mut dyn Context) -> Result<Option<u32>> {
    in_frame(
        context,
        FrameLayout::movem(0, 0x1c, 0x1c),
        |context, frame| {
            let song = frame.argument_word(0).read(context);
            start_jam_out_inner(context, song)
        },
    )
}
