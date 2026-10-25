use crate::call_frame::{CallSite, OutgoingArguments};
use crate::{Context, Result, call_at_void, ids};
use tje_mem::ram;

/// G/src/cutscenes/intro_frame_release.c:8 (`IntroReleaseFrameEntitiesAndRefresh`).
/// TimeAware: RAM releases/queuing precede a real VBlank and graphics upload.
pub fn intro_release_frame_entities_and_refresh(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    // c_intro_frame_release.s:4,33: one MOVE.L save/restore, not MOVEM.
    let entry_sp = context.stack_pointer()?;
    let save_sp = entry_sp.wrapping_sub(4);
    let saved = context.data_register(2);
    context.write_u16(save_sp.wrapping_add(2), saved as u16);
    context.write_u16(save_sp, (saved >> 16) as u16);
    context.set_stack_pointer(save_sp)?;
    let result =
        release_frame_entities_body(context).map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    if result.is_err() {
        context.set_stack_pointer(save_sp)?;
    }
    let restore_sp = context.stack_pointer()?;
    let restored = context.read_u32(restore_sp);
    context.set_data_register(2, restored);
    context.set_stack_pointer(restore_sp.wrapping_add(4))?;
    let flags = (u16::from(restored == 0) << 2) | ((restored >> 28) as u16 & 8);
    context.set_status_register((context.status_register() & !0xf) | flags);
    result
}

fn release_frame_entities_body(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let owner = ids::INTRO_RELEASE_FRAME_ENTITIES_AND_REFRESH;
    for index in 0..10 {
        let slot = ram::g_intro_component_entities(revision).at(index);
        let object_number = i16::from(slot.read(context));
        if object_number != 0 {
            call_at_void(
                context,
                ids::FREE_ENTITY,
                &[object_number as i32 as u32],
                CallSite::relative(owner, revision, 0x22),
            )?;
        }
        context.discard_u8(slot.address())?;
        slot.write(context, 0);
    }
    context.discard_u8(ram::g_sprite_graphics_upload_pending(revision).address())?;
    ram::g_sprite_graphics_upload_pending(revision).write(context, 0);
    call_at_void(
        context,
        ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
        &[],
        CallSite::relative(owner, revision, 0x44),
    )?;
    let dispatcher = context.dispatcher();
    let wait = OutgoingArguments::push(context, &[1])?;
    wait.call_void(
        context,
        dispatcher,
        ids::WAIT_FOR_NEXT_V_BLANK,
        CallSite::relative(owner, revision, 0x4e),
    )?;
    let upload = OutgoingArguments::push(context, &[0, 0])?;
    upload.call_void(
        context,
        dispatcher,
        ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
        CallSite::relative(owner, revision, 0x58),
    )?;
    upload.release(context)?;
    wait.release(context)?;
    Ok(None)
}
