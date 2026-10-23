use super::animations::wait_upload;
use crate::{Context, Result, call, ids};
use tje_mem::ram;

/// G/src/ui/inventory.c:100 (`EntityClearViewportSprites`).
pub fn entity_clear_viewport_sprites(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let viewport = arguments[0];
    wait_upload(context)?;
    let mask = if viewport == 0 { 2 } else { 4 };
    for index in 0..26 {
        let object = ram::g_player_info(revision).at(index);
        if object.draw_flags().read(context) & 1 != 0
            && object.draw_flags().read(context) & mask != 0
        {
            call(
                context,
                ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                &[object.address(), viewport],
            )?;
        }
    }
    let head = ram::g_entity_draw_list_head(revision)
        .read(context)
        .address();
    call(context, ids::POSITION_ENTITY_SPRITES, &[head])?;
    call(context, ids::BUILD_VIEWPORT_SPRITE_LINKS, &[])?;
    ram::g_sprite_graphics_upload_pending(revision).write(context, 1);
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    Ok(None)
}
