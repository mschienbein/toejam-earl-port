use crate::{Context, Result, call, ids};
use tje_mem::{Address, ram};

/// G/src/ui/inventory.c:278 (`InventoryRefreshContentsAndWait`).
/// Both waits poll only the source byte and resume after actual interrupt service.
pub fn inventory_refresh_contents_and_wait(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = context
        .frame_state()
        .ok_or(crate::call_frame::FrameError::Unavailable)?
        .argument_word(0)
        .read(context);
    let phase = ram::g_inventory_redraw_phase(revision);
    wait_for_redraw(context, phase)?;
    ram::g_inventory_redraw_player(revision).write(context, player as i8);
    phase.write(context, 1);
    wait_for_redraw(context, phase)?;
    Ok(None)
}

fn wait_for_redraw(context: &mut dyn Context, phase: Address<i8>) -> Result<()> {
    while phase.read(context) != 0 {
        context.wait_for_interrupt()?;
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
