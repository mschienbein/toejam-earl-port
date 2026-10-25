use crate::{Context, Result, call_void, ids};
use tje_mem::ram;

/// G/src/cutscenes/intro_frame_release.c:8 (`IntroReleaseFrameEntitiesAndRefresh`).
/// TimeAware: RAM releases/queuing precede a real VBlank and graphics upload.
pub fn intro_release_frame_entities_and_refresh(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let restored_d2 = context.data_register(2);
    for index in 0..10 {
        let slot = ram::g_intro_component_entities(revision).at(index);
        let object_number = i16::from(slot.read(context));
        if object_number != 0 {
            call_void(context, ids::FREE_ENTITY, &[object_number as i32 as u32])?;
        }
        context.discard_u8(slot.address())?;
        slot.write(context, 0);
    }
    context.discard_u8(ram::g_sprite_graphics_upload_pending(revision).address())?;
    ram::g_sprite_graphics_upload_pending(revision).write(context, 0);
    call_void(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
    call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    call_void(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
    // c_intro_frame_release.s:33 restores D2 with MOVE.L, not MOVEM: NZVC
    // reflect the full saved register, while X remains the upload callee's.
    let flags = (u16::from(restored_d2 == 0) << 2) | ((restored_d2 >> 28) as u16 & 8);
    context.set_status_register((context.status_register() & !0xf) | flags);
    Ok(None)
}
