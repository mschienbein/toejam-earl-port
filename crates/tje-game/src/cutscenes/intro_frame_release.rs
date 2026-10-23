use crate::{Context, Result, call, ids};
use tje_mem::ram;

/// G/src/cutscenes/intro_frame_release.c:8 (`IntroReleaseFrameEntitiesAndRefresh`).
/// TimeAware: RAM releases/queuing precede a real VBlank and graphics upload.
pub fn intro_release_frame_entities_and_refresh(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for index in 0..10 {
        let slot = ram::g_intro_component_entities(revision).at(index);
        let object_number = i16::from(slot.read(context));
        if object_number != 0 {
            call(context, ids::FREE_ENTITY, &[object_number as i32 as u32])?;
        }
        slot.write(context, 0);
    }
    ram::g_sprite_graphics_upload_pending(revision).write(context, 0);
    call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    call(context, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 0])?;
    Ok(None)
}
