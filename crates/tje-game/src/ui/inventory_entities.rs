use super::entities::entity;
use crate::{Context, Result, call_void, ids};
use tje_mem::ram;

/// G/src/ui/inventory.c:855 (`InventoryDeactivateDisplayEntities`).
pub fn inventory_deactivate_display_entities(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = arguments[0] as i16;
    let mut viewport = 1;
    let first = if player == 0 {
        if ram::g_split_screen_enabled(revision).read(context) != 0 {
            viewport = 0;
        }
        26_i16
    } else {
        33_i16
    };
    for index in first..first + 7 {
        let object = entity(context, index);
        call_void(
            context,
            ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
            &[object.address(), viewport],
        )?;
        object.draw_flags().write(context, 0);
    }
    let object = entity(context, player);
    let flags = object.draw_flags().read(context) & !8;
    object.draw_flags().write(context, flags);
    Ok(None)
}
