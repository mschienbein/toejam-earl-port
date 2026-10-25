//! Source-backed argument pushes shared by pending UI entries.
//!
//! G/build/{rev02,rev00}/asm/c_hud.s distinguishes MOVE.L Dn/An,-(SP)
//! (low word first), PEA (high word first), and CLR.L -(SP) (discarded reads
//! before high/low writes). Ordinary OutgoingArguments::push is PEA order.

use crate::{
    Context, FunctionId, Result,
    call_frame::{CallSite, OutgoingArguments},
};

pub(crate) fn push_move(context: &mut dyn Context, value: u32) -> Result<()> {
    let low = context.stack_pointer()?.wrapping_sub(2);
    context.set_stack_pointer(low)?;
    context.write_u16(low, value as u16);
    let high = low.wrapping_sub(2);
    context.set_stack_pointer(high)?;
    context.write_u16(high, (value >> 16) as u16);
    Ok(())
}

pub(crate) fn push_pea(context: &mut dyn Context, value: u32) -> Result<()> {
    let sp = context.stack_pointer()?.wrapping_sub(4);
    context.set_stack_pointer(sp)?;
    context.write_u16(sp, (value >> 16) as u16);
    context.write_u16(sp.wrapping_add(2), value as u16);
    Ok(())
}

pub(crate) fn push_clear(context: &mut dyn Context) -> Result<()> {
    let sp = context.stack_pointer()?.wrapping_sub(4);
    context.set_stack_pointer(sp)?;
    context.discard_u16(sp)?;
    context.discard_u16(sp.wrapping_add(2))?;
    context.write_u16(sp, 0);
    context.write_u16(sp.wrapping_add(2), 0);
    Ok(())
}

/// The compiler keeps each outgoing area until its explicit ADDQ/LEA cleanup.
/// No argument snapshot, extra push, made-up return address or native fallback.
pub(crate) fn call(
    context: &mut dyn Context,
    caller: FunctionId,
    callee: FunctionId,
    return_offset: u32,
) -> Result<u32> {
    let outgoing = OutgoingArguments::existing(context, 0)?;
    let site = CallSite::relative(caller, context.revision(), return_offset);
    let dispatcher = context.dispatcher();
    outgoing.call(context, dispatcher, callee, site)
}

pub(crate) fn release(context: &mut dyn Context, bytes: u32) -> Result<()> {
    let sp = context.stack_pointer()?.wrapping_add(bytes);
    context.set_stack_pointer(sp)
}
