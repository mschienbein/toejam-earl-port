//! Shared IRQ-progressing upload wait for UI translations.
use crate::{Context, Result};
use tje_mem::ram;

pub(crate) fn wait_upload(context: &mut dyn Context) -> Result<()> {
    let revision = context.revision();
    let counter = ram::g_frame_counter(revision);
    while ram::g_sprite_graphics_upload_pending(revision).read(context) != 0 {
        let previous = counter.read(context);
        context.wait_vblank_frame(counter.address(), previous)?;
    }
    Ok(())
}
