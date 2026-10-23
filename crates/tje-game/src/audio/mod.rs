//! Audio translation ownership; see docs/PORTING.md.
//!
//! Original execution classes are reviewed for all 22 inventory entries.
//! Seven RAM-only entries are Atomic, including `PlaySoundEffect` and its
//! Atomic request-only callee.
//! Nine fixed-source hardware transaction entries are Functional, not Atomic.
//!
//! Six originals are TimeAware: `AudioRetryPendingPcmMailbox`,
//! `AudioSubmitPendingPcmSample`, and `AudioTrySubmitSequencerPcmSample`
//! (`G/src/asm/sound_pcm.s:10-131`); `AudioRetryPendingPsgMailbox` and
//! `AudioSubmitPendingIndexedCommand` (`G/src/asm/sound_psg.s:9-45`); and their
//! transitive caller `MusicUpdateSequencer` (`G/src/asm/sound.s:762-1053`).
//! Their mailbox-lock retries require a running Z80, exclude Functional, and
//! observe every bounded BUSACK poll and 24/124-iteration released-bus delay.
//! Original TimeAware fallback executes those actual instructions. A Rust
//! translation still needs a reviewed whole-loop primitive; generic waits do
//! not preserve the read/retry counts. Retry fragments are not RTS entry hooks.

use crate::{Context, Result, execution::ExecutionClass, ids};
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
    crate::Translation {
        id: ids::PLAY_SOUND_EFFECT,
        function: play_sound_effect,
    },
];

/// Reviewed original entries that return without clock/interrupt progress.
/// The six W0 leaves have no callees; the copy/clear loops have fixed bounds.
/// `PlaySoundEffect` (`G/src/audio/play_sound_effect.c:15-31`) only reads/writes
/// RAM and calls the Atomic request-only `AudioRequestFmChannelRelease`.
/// This classifies original entries independently of translation availability.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    ids::MUSIC_RESTORE_INITIAL_STATE,
    ids::MUSIC_REQUEST_TRACK_POINTER_RESET,
    ids::MUSIC_CLEAR_TRACK_POINTERS,
    ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE,
    ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT,
    ids::PLAY_SOUND,
    ids::PLAY_SOUND_EFFECT,
];

/// Reviewed original non-atomic contracts, independent of translation status.
///
/// - `SetupYM2612` (`G/src/asm/sound.s:71-664`): fixed patch ROM reads and
///   ordered YM writes; BUSY latency is the Functional model's eventual-ready
///   stutter. No timer/audio elapsed-time observation or mailbox retry occurs.
/// - `AudioWriteFmChannelReleaseMask` (`sound.s:670-704`) and
///   `YM2612UpdateFrequency` (`sound.s:709-756`): the same ordered YM/BUSACK
///   transaction contract, with the DAC-delay byte changed while Z80 is stopped.
/// - `LoadSoundDriverToZ80` (`G/src/asm/z80bus.s:10-39`): stable ROM upload
///   while bus-owned, combinational BUSACK (reset still denies access), and
///   ordered reset/release writes. Its fixed instruction delays are not logic
///   observations; this is not a sound/startup timing-equivalence claim.
/// - `AudioReleaseAllFmChannels` (`G/src/asm/sound_channels.s:9-20`): six
///   Functional key-mask calls, then a RAM request clear.
/// - `AudioLoadPendingChannelTemplateEffect` (`sound.s:9-64`): Functional
///   releases followed by fixed-source channel-template ROM/RAM operations.
/// - `MusicLoadSongChannelTemplates` (`G/src/asm/sound_schedule.s:24-38`):
///   Functional release, Atomic initial-state copy, stable ROM-template copy.
/// - `MusicSelectSongTemplateBank` (`sound_schedule.s:12-18`) and
///   `MusicProcessTemplateRestartHook` (`sound_schedule.s:44-76`): finite RAM/
///   stack decisions and only the reviewed Functional template-loader callees.
/// - The six TimeAware entries documented above require the actual timed
///   mailbox protocol and original IRQ execution; the class permits original
///   fallback, not replacing their loops with a generic condition wait.
pub static EXECUTION_CONTRACTS: &[(crate::FunctionId, ExecutionClass)] = &[
    (ids::SETUP_YM2612, ExecutionClass::Functional),
    (
        ids::AUDIO_WRITE_FM_CHANNEL_RELEASE_MASK,
        ExecutionClass::Functional,
    ),
    (ids::YM2612_UPDATE_FREQUENCY, ExecutionClass::Functional),
    (ids::LOAD_SOUND_DRIVER_TO_Z80, ExecutionClass::Functional),
    (
        ids::AUDIO_RELEASE_ALL_FM_CHANNELS,
        ExecutionClass::Functional,
    ),
    (
        ids::AUDIO_LOAD_PENDING_CHANNEL_TEMPLATE_EFFECT,
        ExecutionClass::Functional,
    ),
    (
        ids::MUSIC_LOAD_SONG_CHANNEL_TEMPLATES,
        ExecutionClass::Functional,
    ),
    (
        ids::MUSIC_SELECT_SONG_TEMPLATE_BANK,
        ExecutionClass::Functional,
    ),
    (
        ids::MUSIC_PROCESS_TEMPLATE_RESTART_HOOK,
        ExecutionClass::Functional,
    ),
    (
        ids::AUDIO_RETRY_PENDING_PCM_MAILBOX,
        ExecutionClass::TimeAware,
    ),
    (
        ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
        ExecutionClass::TimeAware,
    ),
    (
        ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
        ExecutionClass::TimeAware,
    ),
    (
        ids::AUDIO_RETRY_PENDING_PSG_MAILBOX,
        ExecutionClass::TimeAware,
    ),
    (
        ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
        ExecutionClass::TimeAware,
    ),
    (ids::MUSIC_UPDATE_SEQUENCER, ExecutionClass::TimeAware),
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

/// G/src/audio/play_sound_effect.c:15 (`PlaySoundEffect`, REV 02 0x03f5dc).
/// Keep the raw long sample value, signed-short admission, ordered request and
/// priority stores, and the one-way blocking latch. Both sizeof uses are inert.
pub fn play_sound_effect(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let sample = arguments[0] as i32;
    let priority = arguments[1] as i16;
    let stop_channels = arguments[2] as i16;
    if (ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0
        && ram::g_psg_effect_status(revision).read(context) == 0)
        || priority >= ram::g_sound_effect_priority(revision).read(context)
    {
        ram::g_pending_pcm_sample(revision).write(context, sample);
        ram::g_sound_effect_priority(revision).write(context, priority);
        if stop_channels != 0 {
            ram::g_pcm_blocks_sequencer(revision).write(context, 1);
            crate::call(context, ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, &[])?;
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
