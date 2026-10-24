//! Speech translation ownership; see docs/PORTING.md.

mod dialogue;
pub use dialogue::{
    clear_speech_bubbles_and_replies, generate_speech, start_dialogue, update_speech_and_dialogue,
};

use crate::{Context, Result, execution::ExecutionClass, ids};
use tje_mem::{ram, rom};

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: crate::ids::SET_NEXT_SPEECH_LIFETIME,
        function: set_next_speech_lifetime,
    },
    crate::Translation {
        id: crate::ids::IS_SPEECH_BUBBLE_ENTITY,
        function: is_speech_bubble_entity,
    },
    crate::Translation {
        id: crate::ids::HAS_SPEAKER_SPEECH,
        function: has_speaker_speech,
    },
    crate::Translation {
        id: ids::QUEUE_SPEECH_TEXT_TILES,
        function: queue_speech_text_tiles,
    },
    crate::Translation {
        id: ids::CLEAR_SPEECH_BUBBLES_AND_REPLIES,
        function: clear_speech_bubbles_and_replies,
    },
];

/// Complete bodies excluded from production until their timing proofs pass.
pub static PENDING_TRANSLATIONS: &[crate::Translation] = &[crate::Translation {
    id: ids::UPDATE_SPEECH_AND_DIALOGUE,
    function: update_speech_and_dialogue,
}];

/// Pending entries retain the compiler's word/long/byte argument read widths.
pub static PENDING_FRAMED_TRANSLATIONS: &[crate::FramedTranslation] = &[
    crate::FramedTranslation {
        id: ids::GENERATE_SPEECH,
        function: generate_speech_framed,
    },
    crate::FramedTranslation {
        id: ids::START_DIALOGUE,
        function: start_dialogue_framed,
    },
];

fn generate_speech_framed(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = context
        .frame_state()
        .ok_or(crate::call_frame::FrameError::Unavailable)?;
    let speaker = i32::from(frame.argument_word(0).read(context)) as u32;
    let text = frame.argument_slot(1).read(context);
    let replace = i32::from(frame.argument_byte(2).read(context)) as u32;
    generate_speech(context, &[speaker, text, replace])
}

fn start_dialogue_framed(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = context
        .frame_state()
        .ok_or(crate::call_frame::FrameError::Unavailable)?;
    let dialogue = i32::from(frame.argument_word(0).read(context)) as u32;
    let speaker = i32::from(frame.argument_word(1).read(context)) as u32;
    start_dialogue(context, &[dialogue, speaker])
}

pub static PENDING: &[crate::FunctionId] = &[
    // Strict owned Z80 mailbox read A0004A differs (FF/04) during the real
    // upload-wait IRQ at frame 10000; no mailbox value/count waiver applies.
    ids::GENERATE_SPEECH,
    // Awake/allocating dialogue rejects the same strict Z80 mailbox operand.
    ids::START_DIALOGUE,
    // Strict VDP mismatch at REV 02 frame 7353: the original accepts IRQ4
    // immediately before RTS; the zero-time no-speech Rust path does not.
    // Needs a TimeAware return split/clock contract, not an IRQ or VDP mask.
    ids::UPDATE_SPEECH_AND_DIALOGUE,
];

/// Reviewed original Atomic entries, including untranslated originals.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    // G/src/speech/speech_and_dialogue.c:63-67: one RAM-word store.
    ids::SET_NEXT_SPEECH_LIFETIME,
    // G/src/speech/speech_bubble_lookup.c:6-17: four RAM-byte comparisons.
    ids::IS_SPEECH_BUBBLE_ENTITY,
    // G/src/speech/speech_and_dialogue.c:173-190: four-slot RAM scan and
    // two delayed-reply RAM queries; no callees, hardware, or waits.
    ids::HAS_SPEAKER_SPEECH,
    // :157-170: four RAM slots and FreeEntity's RAM-only pool release.
    ids::CLEAR_SPEECH_BUBBLES_AND_REPLIES,
];

/// Reviewed original non-Atomic classes. Together with ATOMIC_CAPABLE these
/// classify all eight speech originals in both relocated-address revisions.
pub static EXECUTION_CONTRACTS: &[crate::execution::ExecutionContract] = &[
    // G/src/speech/speech_and_dialogue.c:19-45: stable ROM font/RAM text
    // copies and queue writes. TextMapCharacterToGlyph's embedded controls can
    // emit repeated VDP writes, so transaction completion is required; there
    // are no status/raster reads, readiness polls, or concurrent observations.
    crate::execution::ExecutionContract {
        function: ids::QUEUE_SPEECH_TEXT_TILES,
        class: ExecutionClass::Functional,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // :71-154, specifically :135-136: gSpriteGraphicsUploadPending is cleared
    // by UploadQueuedSpriteGraphics via VBlankHandler (startup_and_main_loop.c:
    // 219-220; sprites/load_sprites.c:30). Successful allocation can require
    // actual IRQ progress, even when a particular live entry sees zero.
    crate::execution::ExecutionContract {
        function: ids::GENERATE_SPEECH,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // :200-247: the first-line path calls the time-aware GenerateSpeech.
    crate::execution::ExecutionContract {
        function: ids::START_DIALOGUE,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
    // :264-320: replies and random/reunion dialogues call GenerateSpeech and
    // StartDialogue; those paths can wait for the upload IRQ. The omitted
    // speaker is recovered from the original caller's saved entry-D2 slot.
    crate::execution::ExecutionContract {
        function: ids::UPDATE_SPEECH_AND_DIALOGUE,
        class: ExecutionClass::TimeAware,
        proof_envelope: crate::execution::ProofEnvelope::ClassDefined,
        operational_ceiling: None,
        evidence: None,
    },
];
/// G/src/speech/speech_and_dialogue.c:63 (`SetNextSpeechLifetime`).
pub fn set_next_speech_lifetime(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    ram::g_next_speech_lifetime_override(context.revision()).write(context, arguments[0] as i16);
    Ok(None)
}

/// G/src/speech/speech_bubble_lookup.c:6 (`IsSpeechBubbleEntity`).
/// Bubble handles are signed bytes; the argument is a signed word.
pub fn is_speech_bubble_entity(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let object_number = arguments[0] as i16;
    if object_number >= 6 {
        let slots = ram::g_speech_bubble_entities(context.revision());
        for slot in 0..4 {
            if i16::from(slots.at(slot).read(context)) == object_number {
                return Ok(Some(1));
            }
        }
    }
    Ok(Some(0))
}

/// G/src/speech/speech_and_dialogue.c:173 (`HasSpeakerSpeech`).
/// Preserve the skip-physics flag test and both signed delayed-speaker aliases.
pub fn has_speaker_speech(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let speaker = arguments[0] as i16;
    let slots = ram::g_speech_bubble_entities(revision);
    for slot in 0..4 {
        if slots.at(slot).read(context) != 0 {
            let bubble = ram::g_player_info(revision).at(i32::from(slots.at(slot).read(context)));
            if bubble.draw_flags().read(context) & 0x08 != 0
                && i16::from(bubble.owner().read(context)) == speaker
            {
                return Ok(Some(1));
            }
        }
    }
    if (ram::g_delayed_speech_reply_timers(revision)
        .at(0)
        .read(context)
        != 0
        && i16::from(
            ram::g_delayed_speech_reply_speakers(revision)
                .at(0)
                .read(context),
        ) == speaker)
        || (ram::g_second_speech_reply_timer(revision).read(context) != 0
            && i16::from(ram::g_second_delayed_speech_reply_speaker(revision).read(context))
                == speaker)
    {
        return Ok(Some(1));
    }
    Ok(Some(0))
}

/// G/src/speech/speech_and_dialogue.c:19 (`QueueSpeechTextTiles`).
/// Keep the signed word counters and unsigned byte queue-index wrap.
pub fn queue_speech_text_tiles(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let text = arguments[0];
    let length = arguments[1] as i16;
    let tile = arguments[2] as i16;
    let font = rom::runtime_text_font_tiles(revision).address();
    crate::call(
        context,
        ids::APPEND_SPRITE_TILES_TO_BUFFER,
        &[font.wrapping_add(27 * 32), 1],
    )?;
    let mut padding = ((12 - i32::from(length)) / 2) as i16;
    let mut index = 0_i16;
    while index < padding {
        crate::call(context, ids::APPEND_SPRITE_TILES_TO_BUFFER, &[font, 1])?;
        index = index.wrapping_add(1);
    }
    index = 0;
    while index < length {
        let character =
            i32::from(context.read_u8(text.wrapping_add(i32::from(index) as u32)) as i8);
        let mut glyph = crate::call(
            context,
            ids::TEXT_MAP_CHARACTER_TO_GLYPH,
            &[character as u32],
        )? as i8;
        if glyph == 27 {
            glyph = glyph.wrapping_add(1);
        }
        let source = font.wrapping_add((i32::from(glyph) << 5) as u32);
        crate::call(context, ids::APPEND_SPRITE_TILES_TO_BUFFER, &[source, 1])?;
        index = index.wrapping_add(1);
    }
    if length & 1 != 0 {
        padding = padding.wrapping_add(1);
    }
    index = 0;
    while index < padding {
        crate::call(context, ids::APPEND_SPRITE_TILES_TO_BUFFER, &[font, 1])?;
        index = index.wrapping_add(1);
    }
    for value in [tile, 13] {
        let count = ram::g_sprite_tile_upload_queue_word_count(revision);
        let old = count.read(context);
        let next = count.read(context).wrapping_add(1);
        count.write(context, next);
        ram::g_sprite_tile_upload_runs(revision)
            .at(i32::from(old))
            .write(context, value as u16);
    }
    Ok(None)
}
