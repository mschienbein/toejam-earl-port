//! Stock MCC68K runtime entries. These are not stack-argument C functions.
use crate::{Context, Result};
use tje_mem::arithmetic;

/// G/src/asm/runtime.s:16 (`abs`): one stack long, wrapping NEG.L, D0 result.
pub fn abs(_: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    Ok(Some(arithmetic::abs_i32(arguments[0] as i32) as u32))
}
/// G/src/asm/runtime.s:28 (`lmodt`): signed D0 % D1, dividend-signed remainder.
pub fn lmodt(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    Ok(Some(arithmetic::rem_i32(
        context.data_register(0) as i32,
        context.data_register(1) as i32,
    )? as u32))
}
/// G/src/asm/runtime.s:92 (`ldivt`): signed D0 / D1, truncation toward zero.
pub fn ldivt(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    Ok(Some(arithmetic::div_i32(
        context.data_register(0) as i32,
        context.data_register(1) as i32,
    )? as u32))
}
/// G/src/asm/runtime.s:157 (`lmult`): low 32 bits of D0 * D1, D2 preserved.
pub fn lmult(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    Ok(Some(
        context
            .data_register(0)
            .wrapping_mul(context.data_register(1)),
    ))
}
/// G/src/asm/runtime.s:192 (`ulmodt`): select remainder from the shared UDIV body.
pub fn ulmodt(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    unsigned_divmod(context, false)
}
/// G/src/asm/runtime.s:200 (`uldivt`): select quotient from the shared UDIV body.
pub fn uldivt(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    unsigned_divmod(context, true)
}
fn unsigned_divmod(context: &mut dyn Context, quotient: bool) -> Result<Option<u32>> {
    let dividend = context.data_register(0);
    let divisor = context.data_register(1);
    Ok(Some(if quotient {
        arithmetic::div_u32(dividend, divisor)?
    } else {
        arithmetic::rem_u32(dividend, divisor)?
    }))
}

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: crate::ids::ABS,
        function: abs,
    },
    crate::Translation {
        id: crate::ids::LMODT,
        function: lmodt,
    },
    crate::Translation {
        id: crate::ids::LDIVT,
        function: ldivt,
    },
    crate::Translation {
        id: crate::ids::LMULT,
        function: lmult,
    },
    crate::Translation {
        id: crate::ids::ULMODT,
        function: ulmodt,
    },
    crate::Translation {
        id: crate::ids::ULDIVT,
        function: uldivt,
    },
];

/// Runtime entries need no non-atomic execution contract.
pub static EXECUTION_CONTRACTS: &[(crate::FunctionId, crate::execution::ExecutionClass)] = &[];

/// Reviewed original entries that return without clock/interrupt progress.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    crate::ids::ABS,
    crate::ids::LMODT,
    crate::ids::LDIVT,
    crate::ids::LMULT,
    crate::ids::ULMODT,
    crate::ids::ULDIVT,
];
