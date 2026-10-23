//! 68000 platform services; assembly register and condition-code effects are exact.
//!
//! Bulk writers are Functional, not Atomic: immediate completion preserves their
//! ordered transactions and stable ROM/RAM sources, but removes FIFO/DMA latency.
//! Reset/exception paths are TimeAware and retain their distinct control-flow ABI.

use crate::{Context, Result, execution::ExecutionClass};
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

/// Original execution contracts for every non-atomic platform catalogue entry.
///
/// All ten bulk writers have no callees, no VDP status/HV/busy readback, and no
/// Z80 retries. Their ROM/work-RAM source stream is stable while the 68000 is
/// stalled; Functional completion preserves every ordered write and CPU result.
/// This is logic equivalence only, never scanline/interrupt timing equivalence.
pub static EXECUTION_CONTRACTS: &[(crate::FunctionId, ExecutionClass)] = &[
    // dma.s:15-75: enables DMA and starts a stable-source memory transfer; the
    // implicit bus halt is transaction latency, not an observed busy operand.
    (
        crate::ids::DMA_COPY_WORDS_TO_VRAM,
        ExecutionClass::Functional,
    ),
    // vdp.s:20-26: 32,768 zero words; only FIFO-space stalls require time.
    (crate::ids::CLEAR_VIDEO_RAM, ExecutionClass::Functional),
    // vdp.s:32-38: 40 zero words to VSRAM, no raster/status observation.
    (
        crate::ids::CLEAR_VERTICAL_SCROLL_RAM,
        ExecutionClass::Functional,
    ),
    // vdp.s:44-50: 64 zero words to CRAM, no raster/status observation.
    (crate::ids::CLEAR_COLOUR_RAM, ExecutionClass::Functional),
    // vdp.s:58-71: ordered stable-source longwords; inclusive low-word DBRA.
    (
        crate::ids::COPY_LONGWORD_RUN_TO_VRAM_TILE,
        ExecutionClass::Functional,
    ),
    // vdp.s:77-84: repeated low-word value; FIFO draining has no readback.
    (crate::ids::FILL_VDP_DATA_WORDS, ExecutionClass::Functional),
    // vdp.s:90-97: byte-width data writes retain their order and FIFO semantics.
    (crate::ids::FILL_VDP_DATA_BYTES, ExecutionClass::Functional),
    // vdp.s:103-110: forward stable ROM/RAM word stream, no busy/raster reads.
    (crate::ids::MOVE_DATA_TO_VRAM, ExecutionClass::Functional),
    // vdp.s:117-147: row commands and stable-source words, no status observation.
    (
        crate::ids::UPLOAD_VRAM_WORD_RECTANGLE,
        ExecutionClass::Functional,
    ),
    // vdp.s:153-165: fixed sixteen palette words, only FIFO latency is removed.
    (
        crate::ids::COPY_PALETTE_TO_CRAM_SLOT,
        ExecutionClass::Functional,
    ),
    // interrupts.s:49-53 + core/startup_and_main_loop.c:175-255: real VBlank
    // exception frame; dispatches graphics/audio/input callbacks and returns RTE.
    (crate::ids::LEVEL6_ISR, ExecutionClass::TimeAware),
    // interrupts.s:59-64 + core/startup_and_main_loop.c:258-279: real HBlank
    // exception frame and raster-relative lower-viewport programming, then RTE.
    (crate::ids::LEVEL4_ISR, ExecutionClass::TimeAware),
    // interrupts.s:69-73: non-returning reset transfer reloads SP and enters
    // startup.s's BUSACK/status/bootstrap path, then the interrupt-driven loop.
    (
        crate::ids::RESTART_FROM_RESET_ENTRY,
        ExecutionClass::TimeAware,
    ),
    // interrupts.s:14-27: startup fragment clears RAM and enters non-returning
    // VDPInitialisation/FurtherInitialisation (core:58-62,607-706).
    (crate::ids::GAME_PROGRAM, ExecutionClass::TimeAware),
    // startup.s:23-102: cold/warm reset, observed VDP status and Z80 bus polling,
    // peripheral bootstrap, then GameProgram; never an ordinary RTS call.
    (crate::ids::START, ExecutionClass::TimeAware),
];

#[cfg(test)]
mod tests;
