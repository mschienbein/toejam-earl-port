use crate::call_frame::{CallSite, FrameLayout, LocalFrame};
use crate::{Context, FunctionId, Result, call, call_at, ids};
use tje_mem::{Address, ram, rom};

fn update_music(context: &mut dyn Context, caller: FunctionId) -> Result<()> {
    let revision = context.revision();
    if ram::g_current_music_selection(revision).read(context)
        != ram::g_requested_music_selection(revision).read(context)
    {
        let requested = i32::from(ram::g_requested_music_selection(revision).read(context)) as u32;
        call_at(
            context,
            ids::MUSIC_SELECT_SONG_TEMPLATE_BANK,
            &[requested],
            CallSite::relative(caller, revision, 0x36),
        )?;
        let requested = ram::g_requested_music_selection(revision).read(context);
        ram::g_current_music_selection(revision).write(context, requested);
    } else {
        call(context, ids::MUSIC_UPDATE_SEQUENCER, &[])?;
    }
    Ok(())
}

fn poll_player(context: &mut dyn Context, player: i32) -> Result<()> {
    let revision = context.revision();
    let swapped = ram::g_controller_ports_swapped(revision).read(context) != 0;
    let function = if (player == 0) != swapped {
        ids::GET_CTRL1_INPUT
    } else {
        ids::GET_CTRL2_INPUT
    };
    let input = call(context, function, &[])? as u8;
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
        let next = counter.read(context).wrapping_add(1);
        counter.write(context, next);
    }
}

fn restore_solo_earl_counter(context: &mut dyn Context) {
    let revision = context.revision();
    if ram::g_player_selection(revision).read(context) == 1 {
        let counter = ram::g_frame_counter(revision);
        let next = counter.read(context).wrapping_sub(1);
        counter.write(context, next);
    }
}

fn play(context: &mut dyn Context, sample: u32) -> Result<()> {
    call(context, ids::PLAY_SOUND_EFFECT, &[sample, 90, 0])?;
    Ok(())
}

/// G/src/jamout/jamout.c:149 (`JamOutVBlank`, REV 02 0x03AC1C).
/// Keep music before counter/input service, full-byte change detection, route
/// swapping, source-ordered effect requests, and the final counter correction.
fn vblank_inner(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    update_music(context, ids::JAM_OUT_V_BLANK)?;
    let counter = ram::g_frame_counter(revision);
    let next = counter.read(context).wrapping_add(1);
    counter.write(context, next);
    if ram::g_frame_counter_low_byte(revision).read(context) & 1 != 0 {
        poll_player(context, 0)?;
    } else {
        poll_player(context, 1)?;
    }
    swap_solo_earl_routes(context);
    if ram::g_frame_counter_low_byte(revision).read(context) & 1 != 0 {
        let pending = ram::g_jam_audio_route1_pending_input(revision);
        if pending.read(context) != 0 {
            if pending.read(context) & 0x40 != 0 {
                if ram::g_jam_selected_song(revision).read(context) == 0 {
                    ram::g_music_animation_event(revision).write(context, 1);
                }
                play(context, rom::sfx_jam_bass_drum(revision).address())?;
            }
            if pending.read(context) & 0x10 != 0 {
                play(context, rom::sfx_jam_snare_drum(revision).address())?;
            }
            if pending.read(context) & 0x20 != 0 {
                play(context, rom::sfx_jam_clap(revision).address())?;
            }
            if pending.read(context) & 0x0f != 0 {
                if pending.read(context) & 1 != 0 {
                    play(context, rom::sfx_jam_toejam(revision).address())?;
                }
                if pending.read(context) & 2 != 0 {
                    play(context, rom::sfx_jam_big_earl(revision).address())?;
                }
                if pending.read(context) & 4 != 0 {
                    play(context, rom::sfx_jam_rap(revision).address())?;
                }
                if pending.read(context) & 8 != 0 {
                    play(context, rom::sfx_jammin(revision).address())?;
                }
            }
            if pending.read(context) & 0x80 != 0 {
                play(context, rom::sfx_jam_record_scratch(revision).address())?;
            }
            pending.write(context, 0);
        }
    } else {
        let pending = ram::g_jam_audio_route2_pending_input(revision);
        if pending.read(context) != 0 {
            if pending.read(context) & 0x40 != 0 {
                call(context, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[22])?;
                if ram::g_jam_selected_song(revision).read(context) == 0 {
                    ram::g_music_animation_event(revision).write(context, 1);
                }
            }
            if pending.read(context) & 0x10 != 0 {
                call(context, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[20])?;
            }
            if pending.read(context) & 0x20 != 0 {
                call(context, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[19])?;
            }
            if pending.read(context) & 0x0f != 0 || pending.read(context) & 0x80 != 0 {
                call(context, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[21])?;
            }
            pending.write(context, 0);
        }
    }
    restore_solo_earl_counter(context);
    Ok(None)
}

fn play_selected(context: &mut dyn Context, index: Address<u16>) -> Result<()> {
    let revision = context.revision();
    let selected = i32::from(index.read(context));
    let sample = rom::pcm_playback_pointers(revision)
        .at(selected)
        .read(context)
        .address();
    play(context, sample)
}

fn reroll(context: &mut dyn Context, indices: [Address<u16>; 3]) -> Result<()> {
    for index in indices {
        // G's explicit int result view uses the whole original D0, not an i16.
        let value = call(context, ids::GET_RANDOM_NUMBER, &[])? as i32;
        index.write(context, (value % 42) as u16);
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
    let next = counter.read(context).wrapping_add(1);
    counter.write(context, next);
    poll_player(context, 0)?;
    poll_player(context, 1)?;
    swap_solo_earl_routes(context);
    let first = ram::g_jam_audio_route1_pending_input(revision);
    if first.read(context) != 0 {
        if first.read(context) & 0x80 != 0 {
            let cycle = ram::g_jam_song_cycle_index(revision);
            if cycle.read(context) == 7 {
                call(context, ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, &[])?;
                call(context, ids::MUSIC_REQUEST_TRACK_POINTER_RESET, &[])?;
                cycle.write(context, 0);
            } else {
                let previous = cycle.read(context);
                let next = cycle.read(context).wrapping_add(1);
                cycle.write(context, next);
                ram::g_requested_music_selection(revision).write(context, previous as i8);
            }
        }
        if first.read(context) & 0x40 != 0 {
            if ram::g_jam_song_cycle_index(revision).read(context) == 0 {
                ram::g_music_animation_event(revision).write(context, 1);
            }
            play_selected(
                context,
                ram::g_jam_audio_route1_button_a_sample_index(revision),
            )?;
        }
        if first.read(context) & 0x10 != 0 {
            play_selected(
                context,
                ram::g_jam_audio_route1_button_b_sample_index(revision),
            )?;
        }
        if first.read(context) & 0x20 != 0 {
            play_selected(
                context,
                ram::g_jam_audio_route1_button_c_sample_index(revision),
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
            )?;
        }
        first.write(context, 0);
    }
    let second = ram::g_jam_audio_route2_pending_input(revision);
    if second.read(context) != 0 {
        if second.read(context) & 0x80 != 0 {
            let cycle = ram::g_jam_template_bank_cycle_index(revision);
            let next = cycle.read(context).wrapping_add(1);
            cycle.write(context, next);
            if next == 3 {
                cycle.write(context, 0);
            }
            if cycle.read(context) == 0 {
                call(context, ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, &[])?;
                call(context, ids::MUSIC_REQUEST_TRACK_POINTER_RESET, &[])?;
            } else {
                if cycle.read(context) == 1 {
                    ram::g_music_alternate_template_bank_enabled(revision).write(context, 0);
                } else if cycle.read(context) == 2 {
                    ram::g_music_alternate_template_bank_enabled(revision).write(context, 1);
                }
                let current = ram::g_current_music_selection(revision).read(context);
                ram::g_requested_music_selection(revision).write(context, current);
                let requested = ram::g_requested_music_selection(revision).read(context);
                ram::g_current_music_selection(revision).write(context, i8::from(requested == 0));
            }
        }
        if second.read(context) & 0x40 != 0 {
            if ram::g_jam_song_cycle_index(revision).read(context) == 0 {
                ram::g_music_animation_event(revision).write(context, 1);
            }
            play_selected(
                context,
                ram::g_jam_audio_route2_button_a_sample_index(revision),
            )?;
        }
        if second.read(context) & 0x10 != 0 {
            play_selected(
                context,
                ram::g_jam_audio_route2_button_b_sample_index(revision),
            )?;
        }
        if second.read(context) & 0x20 != 0 {
            play_selected(
                context,
                ram::g_jam_audio_route2_button_c_sample_index(revision),
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
            )?;
        }
        second.write(context, 0);
    }
    restore_solo_earl_counter(context);
    Ok(None)
}

/// G/src/jamout/jamout.c:149 (`JamOutVBlank`), matching MOVEM A2-A4.
/// Preserve the real caller frame for the music selector's mutable stack slot.
pub fn vblank(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0, 0x1c))?;
    context.set_address_register(2, ram::g_jam_audio_route1_pending_input(revision).address());
    context.set_address_register(3, ram::g_jam_audio_route2_pending_input(revision).address());
    context.set_address_register(4, ram::g_player_processed_input(revision).address());
    let result = vblank_inner(context, &[]);
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

/// G/src/jamout/jamout.c:223 (`JamOutRandomSampleVBlank`), matching MOVEM A2-A4.
/// The selector's source-backed JSR return is entry+0x36 in both callbacks.
pub fn random_sample_vblank(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0, 0x1c))?;
    context.set_address_register(2, ram::g_jam_audio_route1_pending_input(revision).address());
    context.set_address_register(3, ram::g_jam_audio_route2_pending_input(revision).address());
    context.set_address_register(4, ram::g_player_processed_input(revision).address());
    let result = random_sample_vblank_inner(context, &[]);
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
