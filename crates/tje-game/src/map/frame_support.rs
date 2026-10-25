use crate::{
    Context, Result,
    call_frame::{FrameLayout, LocalFrame},
};

/// Exact source frames must also unwind on ordinary diagnostic errors. Explicit
/// CPU exits retain their live frame according to the shared boundary contract.
pub(super) fn in_frame(
    c: &mut dyn Context,
    layout: FrameLayout,
    body: impl FnOnce(&mut dyn Context, &LocalFrame) -> Result<Option<u32>>,
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(c, layout)?;
    let result = body(c, &frame);
    let result = result.map_err(|error| c.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let leave = frame.leave(c);
    match (result, leave) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}

pub(super) fn release_arguments(
    c: &mut dyn Context,
    outgoing: crate::call_frame::OutgoingArguments,
    result: Result<()>,
) -> Result<()> {
    let result = result.map_err(|error| c.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let release = outgoing.release(c);
    match (result, release) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}
