//! Shared IRQ-progressing upload wait for UI translations.
use crate::{Context, Result, execution::WaitRequest};
use tje_mem::ram;

/// G c_hud.s:277-278,626-627 and c_inventory.s: TST.B pending / BNE.
/// The predicate owns every byte poll, including the final zero observation.
/// No caller-side flag, frame-counter or invented aligned-word read is added.
pub(crate) fn wait_upload(context: &mut dyn Context) -> Result<()> {
    let pending = ram::g_sprite_graphics_upload_pending(context.revision());
    context.wait_until(WaitRequest::ram_byte_equal(pending.address(), u8::MAX, 0))
}
