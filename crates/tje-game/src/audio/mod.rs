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
//! All six remaining original entry bodies are written below but remain pending.
//! The five mailbox entries require ClockedStrict's TimingSource to clock every
//! original role, including no-I/O DBRA steps, or a reviewed whole-loop contract.
//! A condition-only wait cannot preserve the read/retry counts. Retry fragments
//! share their owner continuation and are not independent RTS entry hooks.

use crate::{Context, Result, execution::ExecutionClass, ids};
use tje_mem::{Ptr, ram, rom};

mod channels;
mod hardware;
mod mailbox;
pub mod templates;
mod upload;

/// Check-only bodies proven under the recorded environment, never live replacements.
pub static PROVEN_REPLAYED: &[crate::Translation] = &[];
/// Framed counterpart of `PROVEN_REPLAYED`, also excluded from production.
pub static PROVEN_REPLAYED_FRAMED: &[crate::FramedTranslation] = &[];

/// Available Rust entries owned by this subsystem.
/// Upload admission: corpus-d754b68-final-nine.json, 83 inputs in both ROMs.
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
    crate::Translation {
        id: ids::AUDIO_WRITE_FM_CHANNEL_RELEASE_MASK,
        function: hardware::audio_write_fm_channel_release_mask,
    },
    crate::Translation {
        id: ids::YM2612_UPDATE_FREQUENCY,
        function: hardware::ym2612_update_frequency,
    },
    crate::Translation {
        id: ids::SETUP_YM2612,
        function: hardware::setup_ym2612,
    },
    crate::Translation {
        id: ids::AUDIO_RELEASE_ALL_FM_CHANNELS,
        function: channels::audio_release_all_fm_channels,
    },
    crate::Translation {
        id: ids::AUDIO_LOAD_PENDING_CHANNEL_TEMPLATE_EFFECT,
        function: channels::audio_load_pending_channel_template_effect,
    },
    crate::Translation {
        id: ids::MUSIC_LOAD_SONG_CHANNEL_TEMPLATES,
        function: templates::music_load_song_channel_templates,
    },
    crate::Translation {
        id: ids::MUSIC_SELECT_SONG_TEMPLATE_BANK,
        function: templates::music_select_song_template_bank,
    },
    crate::Translation {
        id: ids::MUSIC_PROCESS_TEMPLATE_RESTART_HOOK,
        function: templates::music_process_template_restart_hook,
    },
    crate::Translation {
        id: ids::LOAD_SOUND_DRIVER_TO_Z80,
        function: upload::load_sound_driver_to_z80,
    },
];

/// The VM and mailbox routines wait for ClockedStrict/TimingSource integration:
/// main-thread YM/BUSACK/lock reads, 24/124 DBRA delays and IRQ cadence remain
/// exact obligations. Their readable data/control bodies are not timing proof.
pub static PENDING: &[crate::FunctionId] = &[
    ids::MUSIC_UPDATE_SEQUENCER,
    ids::AUDIO_RETRY_PENDING_PCM_MAILBOX,
    ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
    ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
    ids::AUDIO_RETRY_PENDING_PSG_MAILBOX,
    ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
];

/// Pending bodies never enter the production dispatcher. Fragment identities
/// are retained for catalogue completeness, not approved as ordinary RTS hooks.
pub static PENDING_TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::MUSIC_UPDATE_SEQUENCER,
        function: templates::music_update_sequencer,
    },
    crate::Translation {
        id: ids::AUDIO_RETRY_PENDING_PCM_MAILBOX,
        function: mailbox::audio_retry_pending_pcm_mailbox,
    },
    crate::Translation {
        id: ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
        function: mailbox::audio_submit_pending_pcm_sample,
    },
    crate::Translation {
        id: ids::AUDIO_RETRY_PENDING_PSG_MAILBOX,
        function: mailbox::audio_retry_pending_psg_mailbox,
    },
    crate::Translation {
        id: ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
        function: mailbox::audio_submit_pending_indexed_command,
    },
];

/// Read the immediate PCM argument at its original use site, after all gates.
/// An eager scalar adapter would add a caller-slot read on early-return paths.
pub static PENDING_FRAMED_TRANSLATIONS: &[crate::FramedTranslation] = &[crate::FramedTranslation {
    id: ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
    function: mailbox::audio_try_submit_sequencer_pcm_sample,
}];

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
pub static EXECUTION_CONTRACTS: &[crate::execution::ExecutionContract] = &[
    crate::execution::ExecutionContract {
        function: ids::SETUP_YM2612,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::AUDIO_WRITE_FM_CHANNEL_RELEASE_MASK,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::YM2612_UPDATE_FREQUENCY,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::LOAD_SOUND_DRIVER_TO_Z80,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::AUDIO_RELEASE_ALL_FM_CHANNELS,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::AUDIO_LOAD_PENDING_CHANNEL_TEMPLATE_EFFECT,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::MUSIC_LOAD_SONG_CHANNEL_TEMPLATES,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::MUSIC_SELECT_SONG_TEMPLATE_BANK,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::MUSIC_PROCESS_TEMPLATE_RESTART_HOOK,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::AUDIO_RETRY_PENDING_PCM_MAILBOX,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::AUDIO_RETRY_PENDING_PSG_MAILBOX,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    crate::execution::ExecutionContract {
        function: ids::MUSIC_UPDATE_SEQUENCER,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
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
/// request store before the priority store. The emitted MOVE.W leaves priority
/// in D0.low without changing its upper word; callers can observe the final
/// MOVE.W or rejected CMP.W condition codes (G/build/rev02/asm/c_play_sound.s:4-16,
/// identical in REV 00). Every other register and the entry X flag are preserved.
pub fn play_sound(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let sound_number = arguments[0] as i16;
    let priority = arguments[1] as i16;
    context.set_data_register(
        0,
        (context.data_register(0) & 0xffff_0000) | u32::from(priority as u16),
    );
    if ram::g_pcm_effect_cooldown_ticks(revision).read(context) != 0
        || ram::g_psg_effect_status(revision).read(context) != 0
    {
        let old_priority = ram::g_sound_effect_priority(revision).read(context);
        hardware::compare_word(context, priority as u16, old_priority as u16);
        if priority < old_priority {
            return Ok(None);
        }
    }
    ram::g_pending_psg_effect(revision).write(context, sound_number);
    ram::g_sound_effect_priority(revision).write(context, priority);
    let flags = (u16::from(priority < 0) << 3) | (u16::from(priority == 0) << 2);
    context.set_status_register((context.status_register() & !0x0f) | flags);
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
            crate::call_void(context, ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, &[])?;
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
