use super::animations::wait_upload;
use super::control::finish_inventory_frame;
use super::hud_all::frame;
use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, ids};
use tje_mem::ram;

/// G/src/ui/inventory.c:100 (`EntityClearViewportSprites`).
/// c_inventory.s:112-155 has no LINK or A6 save. In particular, retain the
/// draw-list argument through BuildViewportSpriteLinks and propagate its A6.
pub fn entity_clear_viewport_sprites(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x1c, 0x04))?;
    let viewport = frame.argument_slot(0).read(context);
    context.set_data_register(2, viewport);
    let result = entity_clear_viewport_sprites_inner(context, viewport);
    finish_inventory_frame(context, frame, result)
}

fn entity_clear_viewport_sprites_inner(
    context: &mut dyn Context,
    viewport: u32,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::ENTITY_CLEAR_VIEWPORT_SPRITES;
    let dispatcher = context.dispatcher();
    wait_upload(context)?;
    let mask = if viewport == 0 { 2 } else { 4 };
    context.set_data_register(4, u32::from(mask));
    context.set_data_register(3, context.data_register(3) & 0xffff0000);
    for index in 0..26 {
        let object = ram::g_player_info(revision).at(index);
        context.set_address_register(2, object.address());
        if object.draw_flags().read(context) & 1 != 0
            && object.draw_flags().read(context) & mask != 0
        {
            {
                let second = viewport;
                frame::push_move(context, second)?;
                let first = object.address();
                frame::push_move(context, first)?;
                frame::call_void(context, caller, ids::RELEASE_ENTITY_VIEWPORT_SPRITES, 0x44)?;
                frame::release(context, 8)?;
            };
        }
        context.set_data_register(
            3,
            (context.data_register(3) & 0xffff0000) | (index + 1) as u32,
        );
        context.set_address_register(2, object.offset(1).address());
    }
    let head = ram::g_entity_draw_list_head(revision)
        .read(context)
        .address();
    let head_argument = {
        let argument = head;
        frame::push_move(context, argument)?;
        OutgoingArguments::existing(context, 1)?
    };
    head_argument.call_void(
        context,
        dispatcher,
        ids::POSITION_ENTITY_SPRITES,
        CallSite::relative(caller, revision, 0x5e),
    )?;
    OutgoingArguments::existing(context, 0)?.call_void(
        context,
        dispatcher,
        ids::BUILD_VIEWPORT_SPRITE_LINKS,
        CallSite::relative(caller, revision, 0x64),
    )?;
    head_argument.release(context)?;
    ram::g_sprite_graphics_upload_pending(revision).write(context, 1);
    {
        let argument = 1;
        frame::push_pea(context, argument)?;
        frame::call_void(context, caller, ids::WAIT_FOR_NEXT_V_BLANK, 0x78)?;
        frame::release(context, 4)?;
    };
    Ok(None)
}
