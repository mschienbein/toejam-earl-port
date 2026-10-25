use crate::{
    Context, FunctionId, Result,
    call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments},
};

// Same ordinary-error/typed-exit discipline as map/frame_support.rs. Retained
// outgoing areas are source-owned until the corresponding ADDQ/LEA below.
pub(super) fn in_frame(
    context: &mut dyn Context,
    layout: FrameLayout,
    body: impl FnOnce(&mut dyn Context, &LocalFrame) -> Result<Option<u32>>,
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, layout)?;
    let result = body(context, &frame).map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    if result.is_err() {
        context.set_stack_pointer(frame.save_base())?;
    }
    let leave = frame.leave(context);
    match (result, leave) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

pub(super) fn retained_call(
    context: &mut dyn Context,
    caller: FunctionId,
    return_offset: u32,
    callee: FunctionId,
    arguments: &[u32],
) -> Result<u32> {
    let site = CallSite::relative(caller, context.revision(), return_offset);
    let dispatcher = context.dispatcher();
    let outgoing = OutgoingArguments::push(context, arguments)?;
    outgoing.call(context, dispatcher, callee, site)
}

pub(super) fn retained_call_void(
    context: &mut dyn Context,
    caller: FunctionId,
    return_offset: u32,
    callee: FunctionId,
    arguments: &[u32],
) -> Result<()> {
    let site = CallSite::relative(caller, context.revision(), return_offset);
    let dispatcher = context.dispatcher();
    let outgoing = OutgoingArguments::push(context, arguments)?;
    outgoing.call_void(context, dispatcher, callee, site)
}

pub(super) fn release_arguments(context: &mut dyn Context, slots: u32) -> Result<()> {
    OutgoingArguments::existing(context, slots)?.release(context)
}

pub(super) fn source_call(
    context: &mut dyn Context,
    caller: FunctionId,
    return_offset: u32,
    callee: FunctionId,
    arguments: &[u32],
) -> Result<u32> {
    let site = CallSite::relative(caller, context.revision(), return_offset);
    crate::call_at(context, callee, arguments, site)
}

pub(super) fn source_call_void(
    context: &mut dyn Context,
    caller: FunctionId,
    return_offset: u32,
    callee: FunctionId,
    arguments: &[u32],
) -> Result<()> {
    let site = CallSite::relative(caller, context.revision(), return_offset);
    crate::call_at_void(context, callee, arguments, site)
}

// MCC68K tries DIVS.W first and calls lmodt only if the signed quotient does
// not fit a word. The random result is the complete original D0, not i16.
pub(super) fn random_sample_index(
    context: &mut dyn Context,
    caller: FunctionId,
    random_return_offset: u32,
) -> Result<u16> {
    let value = source_call(
        context,
        caller,
        random_return_offset,
        crate::ids::GET_RANDOM_NUMBER,
        &[],
    )? as i32;
    context.set_data_register(0, value as u32);
    context.set_data_register(1, 42);
    let quotient = value / 42;
    let remainder = if i16::try_from(quotient).is_ok() {
        let remainder = value % 42;
        context.set_data_register(0, remainder as u32);
        remainder
    } else {
        source_call(
            context,
            caller,
            random_return_offset + 0x0c,
            crate::ids::LMODT,
            &[],
        )? as i32
    };
    Ok(remainder as u16)
}

// Non-frame CLR operands are discarded physical reads, never semantic inputs.
pub(super) fn clear_word(context: &mut dyn Context, address: u32) -> Result<()> {
    context.discard_u16(address)?;
    context.write_u16(address, 0);
    word_flags(context, 0);
    Ok(())
}

pub(super) fn clear_long(context: &mut dyn Context, address: u32) -> Result<()> {
    context.discard_u16(address)?;
    context.discard_u16(address.wrapping_add(2))?;
    context.write_u16(address, 0);
    context.write_u16(address.wrapping_add(2), 0);
    word_flags(context, 0);
    Ok(())
}

pub(super) fn word_flags(context: &mut dyn Context, value: u16) {
    let flags = if value == 0 {
        4
    } else if value & 0x8000 != 0 {
        8
    } else {
        0
    };
    context.set_status_register((context.status_register() & !0x0f) | flags);
}

pub(super) fn subtract_flags(
    context: &mut dyn Context,
    left: u32,
    right: u32,
    bits: u32,
    extend: bool,
) -> u32 {
    let mask = u32::MAX >> (32 - bits);
    let sign = 1_u32 << (bits - 1);
    let result = left.wrapping_sub(right) & mask;
    let borrow = left < right;
    let overflow = ((left ^ right) & (left ^ result) & sign) != 0;
    let flags = (u16::from(result & sign != 0) << 3)
        | (u16::from(result == 0) << 2)
        | (u16::from(overflow) << 1)
        | u16::from(borrow);
    let status = if extend {
        (context.status_register() & !0x1f) | (u16::from(borrow) << 4)
    } else {
        context.status_register() & !0x0f
    };
    context.set_status_register(status | flags);
    result
}

pub(super) fn add_word(context: &mut dyn Context, left: u16, right: u16) -> u16 {
    let (value, carry) = left.overflowing_add(right);
    let overflow = (!(left ^ right) & (left ^ value) & 0x8000) != 0;
    let flags = (u16::from(carry) << 4)
        | (u16::from(value & 0x8000 != 0) << 3)
        | (u16::from(value == 0) << 2)
        | (u16::from(overflow) << 1)
        | u16::from(carry);
    context.set_status_register((context.status_register() & !0x1f) | flags);
    value
}
