//! Speech translation ownership; see docs/PORTING.md.

use crate::{Context, Result};
use tje_mem::ram;

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
];

/// Reviewed original entries that return without clock/interrupt progress.
/// Missing IDs are deliberately not approved for atomic checking or fallback.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    crate::ids::SET_NEXT_SPEECH_LIFETIME,
    // Both queries below have only a four-slot RAM scan and no callees or waits.
    crate::ids::IS_SPEECH_BUBBLE_ENTITY,
    crate::ids::HAS_SPEAKER_SPEECH,
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
