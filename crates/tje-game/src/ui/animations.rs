//! Shared IRQ-progressing upload wait for UI translations.
use crate::{Context, Result};
use tje_mem::ram;

/// G c_hud.s:277-278,626-627 and c_inventory.s: TST.B pending / BNE.
/// Only the pending byte is a source operand; no frame-counter word is read.
pub(crate) fn wait_upload(context: &mut dyn Context) -> Result<()> {
    let revision = context.revision();
    while ram::g_sprite_graphics_upload_pending(revision).read(context) != 0 {
        context.wait_for_interrupt()?;
    }
    Ok(())
}
