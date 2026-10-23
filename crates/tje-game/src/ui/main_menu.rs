use crate::{Context, Result};
use tje_mem::ram;

/// G/src/ui/main_menu.c:171 (`AreBothControllersConnected`).
/// The ten-iteration CPU delays do not poll time, BUSACK, or an interrupt.
/// Atomic execution preserves the controller/Z80 access order with clocks frozen.
/// MCC68K emits CLR for zero stores here: its 68000 read-before-write bus
/// transactions are observable, even though the read values are discarded.
pub fn are_both_controllers_connected(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    context.write_z80_bus_request(0x100);
    context.write_controller_control(false, 0x40);
    let _ = context.read_controller_data(false);
    context.write_controller_data(false, 0);
    let first_read = context.read_controller_data(false);
    let _ = context.read_device_u8(0xa10009);
    context.write_controller_control(false, 0);
    let _ = context.read_controller_data(false);
    context.write_controller_data(false, 0);
    let second_read = context.read_controller_data(false);
    let mut present = (second_read & first_read & 15) != 15;
    if present {
        context.write_controller_control(true, 0x40);
        let _ = context.read_controller_data(true);
        context.write_controller_data(true, 0);
        let first_read = context.read_controller_data(true);
        let _ = context.read_device_u8(0xa1000b);
        context.write_controller_control(true, 0);
        let _ = context.read_controller_data(true);
        context.write_controller_data(true, 0);
        let second_read = context.read_controller_data(true);
        if second_read & first_read & 15 == 15 {
            present = false;
        }
    }
    context.discard_z80_bus_request_read()?;
    context.write_z80_bus_request(0);
    Ok(Some(u32::from(present)))
}

/// G/src/ui/main_menu.c:322 (`PositionMenuCursorEntity`).
/// The entity handle and coordinates are signed shorts; shifts narrow on store.
/// The compiler scales the handle with ASL.W then ADDA.W, including word wrap.
pub fn position_menu_cursor_entity(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_menu_cursor_entity(revision).read(context) != 0 {
        let handle = ram::g_menu_cursor_entity(revision).read(context);
        let offset = i32::from(handle.wrapping_shl(7));
        let cursor = ram::g_player_info(revision).at(0).byte_offset(offset);
        let x = i32::from(arguments[0] as i16).wrapping_shl(3) as i16;
        cursor.x().write(context, x);
        let y = i32::from(arguments[1] as i16)
            .wrapping_shl(3)
            .wrapping_add(6) as i16;
        cursor.y().write(context, y);
    }
    Ok(None)
}
