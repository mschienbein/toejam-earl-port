use crate::call_frame::{CallSite, FrameLayout, LocalFrame};
use crate::{Context, FunctionId, Result, call_at};

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
