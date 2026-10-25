use super::frame_support::{
    add_word, clear_word, in_frame, random_sample_index, release_arguments, retained_call,
    source_call, subtract_flags,
};
use crate::call_frame::{FrameLayout, OutgoingArguments};
use crate::{Context, FunctionId, Result, ids};
use tje_mem::{Address, ram, rom};

fn update_music(context: &mut dyn Context, caller: FunctionId) -> Result<()> {
    let revision = context.revision();
    if ram::g_requested_music_selection(revision).read(context)
        != ram::g_current_music_selection(revision).read(context)
    {
        let requested = i32::from(ram::g_requested_music_selection(revision).read(context)) as u32;
        source_call(
            context,
            caller,
            0x36,
            ids::MUSIC_SELECT_SONG_TEMPLATE_BANK,
            &[requested],
        )?;
        let requested = ram::g_requested_music_selection(revision).read(context);
        ram::g_current_music_selection(revision).write(context, requested);
    } else {
        source_call(context, caller, 0x4a, ids::MUSIC_UPDATE_SEQUENCER, &[])?;
    }
    Ok(())
}

fn poll_player(context: &mut dyn Context, player: i32, caller: FunctionId) -> Result<()> {
    let revision = context.revision();
    let swapped = ram::g_controller_ports_swapped(revision).read(context) != 0;
    let function = if (player == 0) != swapped {
        ids::GET_CTRL1_INPUT
    } else {
        ids::GET_CTRL2_INPUT
    };
    let offset = match (caller == ids::JAM_OUT_V_BLANK, player, swapped) {
        (true, 0, true) => 0x68,
        (true, 0, false) => 0x70,
        (true, _, true) => 0xaa,
        (true, _, false) => 0xb2,
        (false, 0, true) => 0x5e,
        (false, 0, false) => 0x66,
        (false, _, true) => 0x9e,
        (false, _, false) => 0xa6,
    };
    let input = source_call(context, caller, offset, function, &[])? as u8;
    let input_slot = ram::g_player_processed_input(revision).at(player);
    input_slot.write(context, input);
    let (previous, animation, route) = if player == 0 {
        (
            ram::g_jam_player1_previous_input(revision),
            ram::g_jam_player1_pending_animation_input(revision),
            ram::g_jam_audio_route1_pending_input(revision),
        )
    } else {
        (
            ram::g_jam_player2_previous_input(revision),
            ram::g_jam_player2_pending_animation_input(revision),
            ram::g_jam_audio_route2_pending_input(revision),
        )
    };
    if i16::from(input_slot.read(context)) != previous.read(context) {
        let input = input_slot.read(context);
        animation.write(context, i16::from(input));
        route.write(context, u16::from(input));
        let input = input_slot.read(context);
        previous.write(context, i16::from(input));
    }
    Ok(())
}

fn swap_solo_earl_routes(context: &mut dyn Context) {
    let revision = context.revision();
    if ram::g_player_selection(revision).read(context) == 1 {
        let saved = ram::g_jam_audio_route1_pending_input(revision).read(context);
        let second = ram::g_jam_audio_route2_pending_input(revision).read(context);
        ram::g_jam_audio_route1_pending_input(revision).write(context, second);
        ram::g_jam_audio_route2_pending_input(revision).write(context, saved);
        let counter = ram::g_frame_counter(revision);
        let old = counter.read(context);
        let next = add_word(context, old, 1);
        counter.write(context, next);
    }
}

fn restore_solo_earl_counter(context: &mut dyn Context) {
    let revision = context.revision();
    let selection = ram::g_player_selection(revision).read(context) as u8;
    subtract_flags(context, u32::from(selection), 1, 8, false);
    if selection == 1 {
        let counter = ram::g_frame_counter(revision);
        let old = counter.read(context);
        let next = subtract_flags(context, u32::from(old), 1, 16, true) as u16;
        counter.write(context, next);
    }
}

fn play(context: &mut dyn Context, sample: u32, offset: u32) -> Result<()> {
    source_call(
        context,
        ids::JAM_OUT_V_BLANK,
        offset,
        ids::PLAY_SOUND_EFFECT,
        &[sample, 90, 0],
    )?;
    Ok(())
}

/// G/src/jamout/jamout.c:149 (`JamOutVBlank`, REV 02 0x03AC1C).
/// Keep music before counter/input service, full-byte change detection, route
/// swapping, source-ordered effect requests, and the final counter correction.
fn vblank_inner(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    update_music(context, ids::JAM_OUT_V_BLANK)?;
    let counter = ram::g_frame_counter(revision);
    let old = counter.read(context);
    let next = add_word(context, old, 1);
    counter.write(context, next);
    if ram::g_frame_counter_low_byte(revision).read(context) & 1 != 0 {
        poll_player(context, 0, ids::JAM_OUT_V_BLANK)?;
    } else {
        poll_player(context, 1, ids::JAM_OUT_V_BLANK)?;
    }
    swap_solo_earl_routes(context);
    if ram::g_frame_counter_low_byte(revision).read(context) & 1 != 0 {
        let pending = ram::g_jam_audio_route1_pending_input(revision);
        if pending.read(context) != 0 {
            if pending.byte_offset(1).cast::<u8>().read(context) & 0x40 != 0 {
                if ram::g_jam_selected_song(revision).read(context) == 0 {
                    ram::g_music_animation_event(revision).write(context, 1);
                }
                play(context, rom::sfx_jam_bass_drum(revision).address(), 0x136)?;
            }
            if pending.byte_offset(1).cast::<u8>().read(context) & 0x10 != 0 {
                play(context, rom::sfx_jam_snare_drum(revision).address(), 0x154)?;
            }
            if pending.byte_offset(1).cast::<u8>().read(context) & 0x20 != 0 {
                play(context, rom::sfx_jam_clap(revision).address(), 0x172)?;
            }
            if pending.read(context) & 0x0f != 0 {
                if pending.byte_offset(1).cast::<u8>().read(context) & 1 != 0 {
                    play(context, rom::sfx_jam_toejam(revision).address(), 0x19c)?;
                }
                if pending.byte_offset(1).cast::<u8>().read(context) & 2 != 0 {
                    play(context, rom::sfx_jam_big_earl(revision).address(), 0x1ba)?;
                }
                if pending.byte_offset(1).cast::<u8>().read(context) & 4 != 0 {
                    play(context, rom::sfx_jam_rap(revision).address(), 0x1d8)?;
                }
                if pending.byte_offset(1).cast::<u8>().read(context) & 8 != 0 {
                    play(context, rom::sfx_jammin(revision).address(), 0x1f6)?;
                }
            }
            if pending.byte_offset(1).cast::<u8>().read(context) & 0x80 != 0 {
                play(
                    context,
                    rom::sfx_jam_record_scratch(revision).address(),
                    0x214,
                )?;
            }
            clear_word(context, pending.address())?;
        }
    } else {
        let pending = ram::g_jam_audio_route2_pending_input(revision);
        if pending.read(context) != 0 {
            if pending.byte_offset(1).cast::<u8>().read(context) & 0x40 != 0 {
                source_call(
                    context,
                    ids::JAM_OUT_V_BLANK,
                    0x236,
                    ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT,
                    &[22],
                )?;
                if ram::g_jam_selected_song(revision).read(context) == 0 {
                    ram::g_music_animation_event(revision).write(context, 1);
                }
            }
            if pending.byte_offset(1).cast::<u8>().read(context) & 0x10 != 0 {
                source_call(
                    context,
                    ids::JAM_OUT_V_BLANK,
                    0x25a,
                    ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT,
                    &[20],
                )?;
            }
            if pending.byte_offset(1).cast::<u8>().read(context) & 0x20 != 0 {
                source_call(
                    context,
                    ids::JAM_OUT_V_BLANK,
                    0x26e,
                    ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT,
                    &[19],
                )?;
            }
            if pending.read(context) & 0x0f != 0
                || pending.byte_offset(1).cast::<u8>().read(context) & 0x80 != 0
            {
                source_call(
                    context,
                    ids::JAM_OUT_V_BLANK,
                    0x28c,
                    ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT,
                    &[21],
                )?;
            }
            clear_word(context, pending.address())?;
        }
    }
    restore_solo_earl_counter(context);
    Ok(None)
}

fn play_selected(context: &mut dyn Context, index: Address<u16>, offset: u32) -> Result<()> {
    let revision = context.revision();
    // The zero/volume pushes precede the index and pointer-table source reads.
    let _tail = OutgoingArguments::push(context, &[90, 0])?;
    let selected = i32::from(index.read(context));
    let sample = rom::pcm_playback_pointers(revision)
        .at(selected)
        .read(context)
        .address();
    retained_call(
        context,
        ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK,
        offset,
        ids::PLAY_SOUND_EFFECT,
        &[sample],
    )?;
    release_arguments(context, 3)
}

fn reroll(context: &mut dyn Context, indices: [Address<u16>; 3], first_offset: u32) -> Result<()> {
    for (position, index) in indices.into_iter().enumerate() {
        let value = random_sample_index(
            context,
            ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK,
            first_offset + 0x20 * position as u32,
        )?;
        context.set_data_register(1, i32::from(value as i16) as u32);
        index.write(context, value);
    }
    Ok(())
}

/// G/src/jamout/jamout.c:223 (`JamOutRandomSampleVBlank`, REV 02 0x03AEC2).
/// Poll both pads, preserve postincrement versus preincrement song/template
/// actions, request the selected samples before rerolling, and retain route order.
fn random_sample_vblank_inner(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    update_music(context, ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK)?;
    let counter = ram::g_frame_counter(revision);
    let old = counter.read(context);
    let next = add_word(context, old, 1);
    counter.write(context, next);
    poll_player(context, 0, ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK)?;
    poll_player(context, 1, ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK)?;
    swap_solo_earl_routes(context);
    let first = ram::g_jam_audio_route1_pending_input(revision);
    if first.read(context) != 0 {
        if first.byte_offset(1).cast::<u8>().read(context) & 0x80 != 0 {
            let cycle = ram::g_jam_song_cycle_index(revision);
            if cycle.read(context) == 7 {
                source_call(
                    context,
                    ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK,
                    0x10c,
                    ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE,
                    &[],
                )?;
                source_call(
                    context,
                    ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK,
                    0x112,
                    ids::MUSIC_REQUEST_TRACK_POINTER_RESET,
                    &[],
                )?;
                clear_word(context, cycle.address())?;
            } else {
                let previous = cycle.read(context);
                let old = cycle.read(context);
                let next = add_word(context, old as u16, 1) as i16;
                cycle.write(context, next);
                ram::g_requested_music_selection(revision).write(context, previous as i8);
            }
        }
        if first.byte_offset(1).cast::<u8>().read(context) & 0x40 != 0 {
            if ram::g_jam_song_cycle_index(revision).read(context) == 0 {
                ram::g_music_animation_event(revision).write(context, 1);
            }
            play_selected(
                context,
                ram::g_jam_audio_route1_button_a_sample_index(revision),
                0x162,
            )?;
        }
        if first.byte_offset(1).cast::<u8>().read(context) & 0x10 != 0 {
            play_selected(
                context,
                ram::g_jam_audio_route1_button_b_sample_index(revision),
                0x18e,
            )?;
        }
        if first.byte_offset(1).cast::<u8>().read(context) & 0x20 != 0 {
            play_selected(
                context,
                ram::g_jam_audio_route1_button_c_sample_index(revision),
                0x1ba,
            )?;
        }
        if first.read(context) & 0x0f != 0 {
            reroll(
                context,
                [
                    ram::g_jam_audio_route1_button_a_sample_index(revision),
                    ram::g_jam_audio_route1_button_b_sample_index(revision),
                    ram::g_jam_audio_route1_button_c_sample_index(revision),
                ],
                0x1ce,
            )?;
        }
        clear_word(context, first.address())?;
    }
    let second = ram::g_jam_audio_route2_pending_input(revision);
    if second.read(context) != 0 {
        if second.byte_offset(1).cast::<u8>().read(context) & 0x80 != 0 {
            let cycle = ram::g_jam_template_bank_cycle_index(revision);
            let old = cycle.read(context);
            let next = add_word(context, old as u16, 1) as i16;
            cycle.write(context, next);
            if cycle.read(context) == 3 {
                clear_word(context, cycle.address())?;
            }
            if cycle.read(context) == 0 {
                source_call(
                    context,
                    ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK,
                    0x25e,
                    ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE,
                    &[],
                )?;
                source_call(
                    context,
                    ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK,
                    0x264,
                    ids::MUSIC_REQUEST_TRACK_POINTER_RESET,
                    &[],
                )?;
            } else {
                if cycle.read(context) == 1 {
                    clear_word(
                        context,
                        ram::g_music_alternate_template_bank_enabled(revision).address(),
                    )?;
                } else if cycle.read(context) == 2 {
                    ram::g_music_alternate_template_bank_enabled(revision).write(context, 1);
                }
                let current = ram::g_current_music_selection(revision).read(context);
                ram::g_requested_music_selection(revision).write(context, current);
                // MOVE.B current,requested supplies Z directly to SEQ.
                ram::g_current_music_selection(revision).write(context, i8::from(current == 0));
            }
        }
        if second.byte_offset(1).cast::<u8>().read(context) & 0x40 != 0 {
            if ram::g_jam_song_cycle_index(revision).read(context) == 0 {
                ram::g_music_animation_event(revision).write(context, 1);
            }
            play_selected(
                context,
                ram::g_jam_audio_route2_button_a_sample_index(revision),
                0x2da,
            )?;
        }
        if second.byte_offset(1).cast::<u8>().read(context) & 0x10 != 0 {
            play_selected(
                context,
                ram::g_jam_audio_route2_button_b_sample_index(revision),
                0x306,
            )?;
        }
        if second.byte_offset(1).cast::<u8>().read(context) & 0x20 != 0 {
            play_selected(
                context,
                ram::g_jam_audio_route2_button_c_sample_index(revision),
                0x332,
            )?;
        }
        if second.read(context) & 0x0f != 0 {
            reroll(
                context,
                [
                    ram::g_jam_audio_route2_button_a_sample_index(revision),
                    ram::g_jam_audio_route2_button_b_sample_index(revision),
                    ram::g_jam_audio_route2_button_c_sample_index(revision),
                ],
                0x346,
            )?;
        }
        clear_word(context, second.address())?;
    }
    restore_solo_earl_counter(context);
    Ok(None)
}

/// G/src/jamout/jamout.c:149 (`JamOutVBlank`), matching MOVEM A2-A4.
/// Preserve the real caller frame for the music selector's mutable stack slot.
pub fn vblank(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    in_frame(context, FrameLayout::movem(0, 0, 0x1c), |context, _| {
        let revision = context.revision();
        context.set_address_register(2, ram::g_jam_audio_route1_pending_input(revision).address());
        context.set_address_register(3, ram::g_jam_audio_route2_pending_input(revision).address());
        context.set_address_register(4, ram::g_player_processed_input(revision).address());
        vblank_inner(context, &[])
    })
}

/// G/src/jamout/jamout.c:223 (`JamOutRandomSampleVBlank`), matching MOVEM A2-A4.
/// The selector's source-backed JSR return is entry+0x36 in both callbacks.
pub fn random_sample_vblank(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    in_frame(context, FrameLayout::movem(0, 0, 0x1c), |context, _| {
        let revision = context.revision();
        context.set_address_register(2, ram::g_jam_audio_route1_pending_input(revision).address());
        context.set_address_register(3, ram::g_jam_audio_route2_pending_input(revision).address());
        context.set_address_register(4, ram::g_player_processed_input(revision).address());
        random_sample_vblank_inner(context, &[])
    })
}
