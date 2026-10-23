//! Shared MCC68K word-indexed entity addressing for UI translations.
use crate::Context;
use tje_mem::{Address, ObjectState, ram};

/// ASL.W #7 followed by ADDA.W, not a full-width host index product.
pub(crate) fn entity(context: &dyn Context, index: i16) -> Address<ObjectState> {
    ram::g_player_info(context.revision())
        .at(0)
        .byte_offset(i32::from(index.wrapping_shl(7)))
}
