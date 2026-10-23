use crate::{Context, Result, call, ids};
use tje_mem::{Address, ram};

/// G/src/ui/inventory.c:278 (`InventoryRefreshContentsAndWait`).
/// Both waits resume only after real VBlank handlers have serviced the redraw.
pub fn inventory_refresh_contents_and_wait(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let phase = ram::g_inventory_redraw_phase(revision);
    wait_for_redraw(context, phase)?;
    ram::g_inventory_redraw_player(revision).write(context, arguments[0] as i16 as i8);
    phase.write(context, 1);
    wait_for_redraw(context, phase)?;
    Ok(None)
}

fn wait_for_redraw(context: &mut dyn Context, phase: Address<i8>) -> Result<()> {
    let counter = ram::g_frame_counter(context.revision());
    while phase.read(context) != 0 {
        let previous = counter.read(context);
        context.wait_vblank_frame(counter.address(), previous)?;
    }
    Ok(())
}

/// G/src/ui/inventory.c:92 (`WaitForVBlankWithGraphicsPaused`).
pub fn wait_for_vblank_with_graphics_paused(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let paused = ram::g_v_blank_graphics_paused(context.revision());
    paused.write(context, 1);
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    paused.write(context, 0);
    Ok(None)
}
