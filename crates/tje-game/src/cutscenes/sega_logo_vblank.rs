use crate::{Context, Result, call, ids};
use tje_mem::ram;

/// G/src/cutscenes/sega_logo_vblank.c:11 (`SegaLogoVBlank`).
/// TimeAware: ordered palette/sprite DMA and music YM/Z80 service precede the
/// IRQ-owned frame increment. It must not be registered as an atomic callback.
pub fn sega_logo_vblank(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let pending = ram::g_sega_palette_upload_pending(revision);
    if pending.read(context) != 0 {
        pending.write(context, 0);
        let palette = ram::g_sega_palette_upload_source(revision).read(context);
        call(
            context,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            &[palette.address(), 3],
        )?;
    }
    if ram::g_sega_v_blank_sprite_updates_enabled(revision).read(context) != 0 {
        call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
        call(context, ids::SELECT_VIEWPORT_SPRITE_CHAIN, &[0])?;
    }
    call(context, ids::MUSIC_UPDATE_SEQUENCER, &[])?;
    let counter = ram::g_frame_counter(revision);
    let current = counter.read(context);
    counter.write(context, current.wrapping_add(1));
    Ok(None)
}
