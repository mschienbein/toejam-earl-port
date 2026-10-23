//! Speech translation ownership; see docs/PORTING.md.

use crate::{Context, Result};
use tje_mem::ram;

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[crate::Translation {
    id: crate::ids::SET_NEXT_SPEECH_LIFETIME,
    function: set_next_speech_lifetime,
}];

/// Reviewed original entries that return without clock/interrupt progress.
/// Missing IDs are deliberately not approved for atomic checking or fallback.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[crate::ids::SET_NEXT_SPEECH_LIFETIME];
/// G/src/speech/speech_and_dialogue.c:63 (`SetNextSpeechLifetime`).
pub fn set_next_speech_lifetime(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    ram::g_next_speech_lifetime_override(context.revision()).write(context, arguments[0] as i16);
    Ok(None)
}
