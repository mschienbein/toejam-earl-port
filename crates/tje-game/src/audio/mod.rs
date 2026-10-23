//! Audio translation ownership; see docs/PORTING.md.
//!
//! The W0 RAM-only leaves below have no original callees or device waits.
//! YM writers, Z80 uploads, and PCM/PSG mailbox submissions are deliberately not
//! atomic-capable: their busy/BUSACK polling and mailbox retries require clocks.
//! Retry fragments fall through into their waiting owner and are not RTS hooks.

use crate::{Context, Result, ids};
use tje_mem::{Ptr, ram, rom};

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::MUSIC_RESTORE_INITIAL_STATE,
        function: music_restore_initial_state,
    },
    crate::Translation {
        id: ids::MUSIC_REQUEST_TRACK_POINTER_RESET,
        function: music_request_track_pointer_reset,
    },
    crate::Translation {
        id: ids::MUSIC_CLEAR_TRACK_POINTERS,
        function: music_clear_track_pointers,
    },
    crate::Translation {
        id: ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE,
        function: audio_request_fm_channel_release,
    },
    crate::Translation {
        id: ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT,
        function: audio_request_channel_template_effect,
    },
    crate::Translation {
        id: ids::PLAY_SOUND,
        function: play_sound,
    },
];

/// Reviewed original entries that return without clock/interrupt progress.
/// Each entry is a RAM-only leaf; the two loops have fixed 276-byte/six-record
/// bounds. Missing IDs are not approved for atomic checking or fallback.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    ids::MUSIC_RESTORE_INITIAL_STATE,
    ids::MUSIC_REQUEST_TRACK_POINTER_RESET,
    ids::MUSIC_CLEAR_TRACK_POINTERS,
    ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE,
    ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT,
    ids::PLAY_SOUND,
];

/// G/src/asm/sound_init.s:12 (`MusicRestoreInitialState`, REV 02 0x03e3f4).
/// Copy the revision's initial-state resource byte by byte, preserving the DBRA
/// residue, postincremented addresses, and the last MOVE.B's condition codes.
pub fn music_restore_initial_state(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut source = rom::music_initial_state(revision).at(0).cast::<u8>();
    let mut destination = ram::g_music_fm_channel_owners(revision).at(0).cast::<u8>();
    let mut counter = 0x0113_u16;
    loop {
        let value = source.read(context);
        source = source.offset(1);
        destination.write(context, value);
        destination = destination.offset(1);
        counter = counter.wrapping_sub(1);
        if counter == u16::MAX {
            context.set_data_register(0, u32::from(counter));
            context.set_address_register(0, source.address());
            context.set_address_register(1, destination.address());
            // SUBQ.W #1 from 0x114 clears X. MOVE.B clears V/C and supplies N/Z;
            // DBRA and the address postincrements do not change condition codes.
            let nz = if value == 0 {
                0x04
            } else if value & 0x80 != 0 {
                0x08
            } else {
                0
            };
            context.set_status_register((context.status_register() & !0x1f) | nz);
            return Ok(None);
        }
    }
}

/// G/src/audio/request_track_reset.c:5 (`MusicRequestTrackPointerReset`).
pub fn music_request_track_pointer_reset(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    ram::g_music_track_reset_requested(context.revision()).write(context, 1);
    Ok(None)
}

/// G/src/asm/sound_clear.s:9 (`MusicClearTrackPointers`, REV 02 0x03e418).
/// Alternate each owner clear with its stream clear, then clear the request.
/// The final CLR.W preserves X, sets Z, and clears N/V/C.
pub fn music_clear_track_pointers(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let owners = ram::g_music_fm_channel_owners(revision).at(0).cast::<u32>();
    let channels = ram::g_music_channels(revision);
    for index in 0..6 {
        owners.offset(index).write(context, 0);
        channels
            .at(index)
            .current_stream()
            .write(context, Ptr::new(0));
    }
    ram::g_music_track_reset_requested(revision).write(context, 0);
    context.set_data_register(0, 0x0000_ffff);
    context.set_address_register(0, owners.offset(6).address());
    context.set_address_register(1, channels.at(6).address());
    context.set_status_register((context.status_register() & !0x0f) | 0x04);
    Ok(None)
}

/// G/src/audio/request_channel_release.c:5 (`AudioRequestFmChannelRelease`).
pub fn audio_request_fm_channel_release(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    ram::g_fm_channel_release_requested(context.revision()).write(context, 1);
    Ok(None)
}

/// G/src/audio/request_template_effect.c:5 (`AudioRequestChannelTemplateEffect`).
/// MCC68K reads the low signed word from the incoming four-byte argument slot.
pub fn audio_request_channel_template_effect(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    ram::g_pending_fm_effect(context.revision()).write(context, arguments[0] as i16);
    Ok(None)
}

/// G/src/audio/play_sound.c:13 (`PlaySound`, REV 02 0x03f770).
/// Retain signed-short priority admission, short-circuit reads, and the pending
/// request store before the priority store. The discarded sizeof has no effect.
pub fn play_sound(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let sound_number = arguments[0] as i16;
    let priority = arguments[1] as i16;
    if (ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0
        && ram::g_psg_effect_status(revision).read(context) == 0)
        || priority >= ram::g_sound_effect_priority(revision).read(context)
    {
        ram::g_pending_psg_effect(revision).write(context, sound_number);
        ram::g_sound_effect_priority(revision).write(context, priority);
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
