//! Source-preserving nonreturning paths of the MCC68K divide helpers.
use crate::{
    Context, FunctionId, Result,
    call_frame::{FrameLayout, LocalFrame},
    control::{ExceptionPreparedState, ExceptionRequest},
    exits, ids,
};

/// G/src/asm/runtime.s:29-49,93-114,193-213. The first DIVU faults AFTER
/// these source stores/register operations. Its D2 save belongs to the handler's
/// suspended call frame and must not be restored/unwound on the typed transfer.
pub(super) fn divide_by_zero(context: &mut dyn Context, entry: FunctionId) -> Result<Option<u32>> {
    let status = context.status_register();
    if status & 0x8000 != 0 {
        // A traced entry faults after an EARLIER prefix instruction. Do not
        // clear T or manufacture the later DIVU fault from that different path.
        return Err(crate::Error::Platform(
            "unsupported runtime exception prefix: trace-enabled entry".into(),
        ));
    }
    let saved_d2 = context.data_register(2);
    let signed = entry == ids::LMODT || entry == ids::LDIVT;
    if !signed {
        // ULMODT/ULDIVT's MOVEA selector precedes the shared D2 stack save.
        context.set_address_register(0, u32::from(entry == ids::ULDIVT));
    }
    // One predecrement long, low physical word first. Keep the actual RAM save
    // and frame declaration; there is intentionally no leave on this path.
    let _saved_frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x04, 0))?;
    let mut extend = status & 0x10;
    if signed {
        if entry == ids::LDIVT {
            context.set_address_register(0, context.data_register(0));
            context.set_data_register(0, 0);
            context.set_data_register(2, context.data_register(1));
            context.set_data_register(1, context.address_register(0));
        } else {
            context.set_data_register(2, context.data_register(1));
            context.set_data_register(1, context.data_register(0));
            context.set_data_register(0, 0);
        }
        if (context.data_register(1) as i32) < 0 {
            context.set_data_register(1, context.data_register(1).wrapping_neg());
            context.set_data_register(0, !context.data_register(0));
            // NEG.L of a negative (therefore nonzero) dividend sets X, including
            // MIN_INT's wrapping negate. NOT/CMPI/CLR/SWAP do not change X.
            extend = 0x10;
        }
        context.set_address_register(0, context.data_register(0));
    } else {
        context.set_data_register(2, context.data_register(1));
        context.set_data_register(1, context.data_register(0));
    }
    // CMPI.L #FFFF,D2 selects this native halfword path for divisor zero.
    context.set_address_register(1, context.data_register(1) as i16 as i32 as u32);
    context.set_data_register(1, context.data_register(1) & 0xffff0000);
    context.set_data_register(1, context.data_register(1).rotate_left(16));
    // The original faulting DIVU clears N/Z/V/C and preserves the source X.
    // No arithmetic result is manufactured and D2 remains zero, not restored.
    context.set_status_register((status & !0x1f) | extend);
    let prepared = ExceptionPreparedState {
        data: [
            context.data_register(0),
            context.data_register(1),
            context.data_register(2),
        ],
        address: [context.address_register(0), context.address_register(1)],
        stack_pointer: context.stack_pointer()?,
        stack_banks: context.stack_banks()?,
        status: context.status_register(),
        saved_d2: Some(saved_d2),
    };
    let request = ExceptionRequest::runtime_divide(entry, context.revision(), prepared)
        .ok_or(crate::Error::Untranslated(entry))?;
    exits::exception(entry, request)
}
