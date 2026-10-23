//! 68000 platform services; assembly register and condition-code effects are exact.
//!
//! G/src/asm/dma.s:72 starts memory-to-VRAM DMA, which holds the CPU until the
//! transfer completes. G/src/asm/vdp.s bulk writers can exceed the four-word VDP
//! FIFO: clears write 32,768/40/64 words, palette upload writes 16, and variable
//! DBRA loops include large/zero-count paths. Those ten entries remain original
//! and deliberately absent from both registration lists: neither DMA completion
//! nor FIFO draining can progress inside an atomic call.

use crate::{Context, Result};
use tje_mem::ram;

/// G/src/asm/interrupts.s:33 (`DisableAllInterrupts`).
/// ORI to SR changes only IPL; every D/A register and all CCR bits survive.
pub fn disable_all_interrupts(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    context.set_status_register(context.status_register() | 0x0700);
    Ok(None)
}

/// G/src/asm/interrupts.s:41 (`EnableAllInterrupts`).
/// ANDI to SR clears only IPL, without changing registers or condition codes.
pub fn enable_all_interrupts(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    context.set_status_register(context.status_register() & 0xf8ff);
    Ok(None)
}

/// G/src/asm/controllers.s:12 (`GetCtrl1Input`).
/// The fixed DBRA delays terminate without polling BUSACK or peripheral state.
pub fn get_ctrl1_input(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    read_controller(context, false)
}

/// G/src/asm/controllers.s:40 (`GetCtrl2Input`).
/// Retains the Z80 DAC-delay update, PSG-status byte store, D1 residue and CCR.
pub fn get_ctrl2_input(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    read_controller(context, true)
}

fn read_controller(context: &mut dyn Context, second: bool) -> Result<Option<u32>> {
    context.write_z80_bus_request(0x0100);
    context.write_controller_control(second, 0x40);
    context.write_controller_data(second, 0);
    // Each MOVE.W #10 / DBRA pair finishes with D1's low word equal to 0xffff.
    // The high word is untouched by both delays and the later byte operations.
    let start_a = (context.read_controller_data(second) << 2) & 0xc0;
    context.write_controller_data(second, 0x40);
    let directions_bc = context.read_controller_data(second) & 0x3f;
    let input = !(start_a | directions_bc);
    let d1 = (context.data_register(1) & 0xffff_0000) | 0xff00 | u32::from(directions_bc);
    context.set_data_register(0, u32::from(input));
    context.set_data_register(1, d1);

    let delay = context.read_z80_ram(0x004a);
    context.write_z80_ram(0x004a, delay.wrapping_sub(2));
    let psg_active = context.read_z80_ram(0x004b);
    // This is the high byte of the signed word view, not a whole-word assignment.
    ram::g_psg_effect_status(context.revision())
        .cast::<u8>()
        .write(context, psg_active);
    context.write_z80_bus_request(0);

    // The last MOVE.W #0 clears N/V/C and sets Z. X survives that MOVE and the
    // preceding MOVE.B; its last writer is SUBQ.B #2 on the DAC-delay byte.
    let extend = if delay < 2 { 0x0010 } else { 0 };
    context.set_status_register((context.status_register() & !0x001f) | extend | 0x0004);
    Ok(Some(u32::from(input)))
}

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: crate::ids::DISABLE_ALL_INTERRUPTS,
        function: disable_all_interrupts,
    },
    crate::Translation {
        id: crate::ids::ENABLE_ALL_INTERRUPTS,
        function: enable_all_interrupts,
    },
    crate::Translation {
        id: crate::ids::GET_CTRL1_INPUT,
        function: get_ctrl1_input,
    },
    crate::Translation {
        id: crate::ids::GET_CTRL2_INPUT,
        function: get_ctrl2_input,
    },
];

/// Reviewed entries with no clock/interrupt-dependent return paths or callees.
/// Interrupt masks are straight-line SR operations (interrupts.s:33,41).
/// Pad sampling has only fixed eleven-iteration DBRA delays (controllers.s:12,40);
/// bus acquisition is a request store, not a BUSACK wait.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    crate::ids::DISABLE_ALL_INTERRUPTS,
    crate::ids::ENABLE_ALL_INTERRUPTS,
    crate::ids::GET_CTRL1_INPUT,
    crate::ids::GET_CTRL2_INPUT,
];

#[cfg(test)]
mod tests;
