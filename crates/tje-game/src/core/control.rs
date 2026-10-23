use crate::call_frame::CallSite;
use crate::{Context, Result, call_at, exits, ids};

/// G/src/core/error_trap.c:5 (`ErrorTrap`, REV 02 0x000200).
/// The original self-branch has no RTS. Stop game logic through the explicit
/// terminal contract; do not consume the exception/caller frame or fake a return.
pub fn error_trap(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    exits::terminal(ids::ERROR_TRAP)
}

/// G/src/core/startup_and_main_loop.c:58 (`VDPInitialisation`, REV 02 0x0003FE).
/// ResetVDP returns normally; the final JSR enters FurtherInitialisation with
/// the real return PC at the following NOP (entry + 8), not a fabricated RTS.
pub fn vdp_initialisation(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let entry = ids::VDP_INITIALISATION
        .metadata()
        .address(context.revision());
    call_at(
        context,
        ids::RESET_VDP,
        &[],
        CallSite {
            return_pc: entry.wrapping_add(4),
        },
    )?;
    exits::call_transfer(
        ids::VDP_INITIALISATION,
        ids::FURTHER_INITIALISATION,
        entry.wrapping_add(8),
    )
}
