use crate::execution::WaitRequest;
use crate::{Context, Result, call_void, ids};
use tje_mem::{Address, ram};

/// G/src/ui/inventory.c:278 (`InventoryRefreshContentsAndWait`).
/// Each source loop delegates every byte poll, including the final zero read.
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
    context.wait_until(WaitRequest::ram_byte_equal(phase.address(), u8::MAX, 0))
}

/// G/src/ui/inventory.c:92 (`WaitForVBlankWithGraphicsPaused`).
pub fn wait_for_vblank_with_graphics_paused(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let paused = ram::g_v_blank_graphics_paused(context.revision());
    paused.write(context, 1);
    call_void(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    paused.write(context, 0);
    Ok(None)
}
