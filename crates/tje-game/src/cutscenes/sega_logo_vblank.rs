use crate::{Context, Result, call, ids};
use tje_mem::ram;

/// G/src/cutscenes/sega_logo_vblank.c:11 (`SegaLogoVBlank`).
/// TimeAware: ordered palette/sprite DMA and music YM/Z80 service precede the
/// IRQ-owned frame increment. It must not be registered as an atomic callback.
pub fn sega_logo_vblank(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let pending = ram::g_sega_palette_upload_pending(revision);
    if pending.read(context) != 0 {
        context.discard_u16(pending.address())?;
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
    let next = current.wrapping_add(1);
    counter.write(context, next);
    // c_sega_logo_vblank.s:22: ADDQ.W sets all five CCR bits, including X.
    let carry = u16::from(current == u16::MAX);
    let overflow = u16::from(current == 0x7fff) << 1;
    let negative = (next & 0x8000) >> 12;
    let zero = u16::from(next == 0) << 2;
    let flags = (carry << 4) | negative | zero | overflow | carry;
    context.set_status_register((context.status_register() & !0x1f) | flags);
    Ok(None)
}
