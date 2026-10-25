use crate::call_frame::{CallSite, FrameLayout, LocalFrame};
use crate::{Context, FunctionId, Result, call_at, call_at_void};

/// Enter and leave the exact emitted frame, including early returns and errors.
/// Non-returning typed exits must retain their live frame for the platform.
pub(super) fn with_frame(
    context: &mut dyn Context,
    layout: FrameLayout,
    body: impl FnOnce(&mut dyn Context, &LocalFrame) -> Result<Option<u32>>,
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, layout)?;
    let result = body(context, &frame);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let leave = frame.leave(context);
    match (result, leave) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

/// The caller supplies the matching instruction's return-PC offset. All player
/// sites using this helper have identical offsets in the two matched revisions.
pub(super) fn call_from(
    context: &mut dyn Context,
    caller: FunctionId,
    callee: FunctionId,
    arguments: &[u32],
    return_offset: u32,
) -> Result<u32> {
    let site = CallSite::relative(caller, context.revision(), return_offset);
    call_at(context, callee, arguments, site)
}

pub(super) fn call_from_void(
    context: &mut dyn Context,
    caller: FunctionId,
    callee: FunctionId,
    arguments: &[u32],
    return_offset: u32,
) -> Result<()> {
    let site = CallSite::relative(caller, context.revision(), return_offset);
    call_at_void(context, callee, arguments, site)
}

/// A 68000 memory CLR performs a read even though it does not use the value.
pub(super) fn clear_byte(context: &mut dyn Context, address: u32) -> Result<()> {
    context.discard_u8(address)?;
    context.write_u8(address, 0);
    logic_flags(context, 0, 8);
    Ok(())
}

pub(super) fn clear_word(context: &mut dyn Context, address: u32) -> Result<()> {
    context.discard_u16(address)?;
    context.write_u16(address, 0);
    logic_flags(context, 0, 16);
    Ok(())
}

/// MOVE.L reads its high word before its low word on the original bus.
pub(super) fn read_long(context: &dyn Context, address: u32) -> u32 {
    (u32::from(context.read_u16(address)) << 16)
        | u32::from(context.read_u16(address.wrapping_add(2)))
}

/// MCC68K's inline NEXT_RANDOM: CMPI.W; ADDQ.W (RMW); MOVE.W reload.
/// The reset arm is a MOVE.W of zero, not CLR.W on the cache index.
pub(super) fn advance_random_index(context: &mut dyn Context) -> u16 {
    let index = tje_mem::ram::g_random_cache_index(context.revision());
    let compared = index.read(context);
    subtraction_flags(context, u32::from(compared), 58, 16, false);
    if compared == 58 {
        logic_flags(context, 0, 16);
        index.write(context, 0);
        0
    } else {
        let old = index.read(context);
        let updated = addition_flags(context, u32::from(old), 1, 16) as u16;
        index.write(context, updated);
        let selected = index.read(context);
        logic_flags(context, u32::from(selected), 16);
        selected
    }
}

/// MOVE/TST/AND leave X unchanged and clear V/C.
pub(super) fn logic_flags(context: &mut dyn Context, value: u32, bits: u32) {
    let mask = u32::MAX >> (32 - bits);
    let value = value & mask;
    let flags = if value == 0 { 4 } else { 0 } | if value & (1 << (bits - 1)) != 0 { 8 } else { 0 };
    context.set_status_register((context.status_register() & !0x000f) | flags);
}

/// CMP preserves X; SUB changes it to the borrow bit.
pub(super) fn subtraction_flags(
    context: &mut dyn Context,
    destination: u32,
    source: u32,
    bits: u32,
    extend: bool,
) -> u32 {
    let mask = u32::MAX >> (32 - bits);
    let destination = destination & mask;
    let source = source & mask;
    let result = destination.wrapping_sub(source) & mask;
    let sign = 1 << (bits - 1);
    let carry = source > destination;
    let overflow = (destination ^ source) & (destination ^ result) & sign != 0;
    let mut flags = u16::from(carry)
        | (u16::from(overflow) << 1)
        | (u16::from(result == 0) << 2)
        | (u16::from(result & sign != 0) << 3);
    let cleared = if extend {
        flags |= u16::from(carry) << 4;
        0x001f
    } else {
        0x000f
    };
    context.set_status_register((context.status_register() & !cleared) | flags);
    result
}

pub(super) fn addition_flags(
    context: &mut dyn Context,
    destination: u32,
    source: u32,
    bits: u32,
) -> u32 {
    let mask = u32::MAX >> (32 - bits);
    let destination = destination & mask;
    let source = source & mask;
    let sum = u64::from(destination) + u64::from(source);
    let result = sum as u32 & mask;
    let sign = 1 << (bits - 1);
    let carry = sum > u64::from(mask);
    let overflow = !(destination ^ source) & (destination ^ result) & sign != 0;
    let flags = (u16::from(carry) * 0x11)
        | (u16::from(overflow) << 1)
        | (u16::from(result == 0) << 2)
        | (u16::from(result & sign != 0) << 3);
    context.set_status_register((context.status_register() & !0x001f) | flags);
    result
}

#[cfg(test)]
mod random_index_tests {
    use super::*;
    use crate::core::test_support::TestContext;
    use tje_mem::{Revision, ram};

    #[test]
    fn every_index_matches_cmp_addq_reload_flags_without_losing_extend() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut context = TestContext::new(revision);
            let index = ram::g_random_cache_index(revision);
            for previous in 0..=u16::MAX {
                index.write(&mut context, previous);
                context.writes.clear();
                context.set_status_register(0x271f);
                let selected = advance_random_index(&mut context);
                let expected = if previous == 58 {
                    0
                } else {
                    previous.wrapping_add(1)
                };
                let flags = if expected == 0 {
                    4
                } else if expected & 0x8000 != 0 {
                    8
                } else {
                    0
                };
                let extend = if previous == 58 || previous == u16::MAX {
                    0x10
                } else {
                    0
                };
                assert_eq!(selected, expected);
                assert_eq!(index.read(&context), expected);
                assert_eq!(context.status_register(), 0x2700 | extend | flags);
            }
        }
    }
}
