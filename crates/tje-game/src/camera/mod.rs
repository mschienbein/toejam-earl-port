//! Camera translation ownership; see docs/PORTING.md.

use crate::{Context, Result, ids};
use tje_mem::ram;

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[crate::Translation {
    id: ids::SET_CAMERA_TILE_PRIORITY_ENABLED,
    function: set_camera_tile_priority_enabled,
}];

/// Reviewed original entries that return without clock/interrupt progress.
/// Missing IDs are deliberately not approved for atomic checking or fallback.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[ids::SET_CAMERA_TILE_PRIORITY_ENABLED];
/// G/src/camera/toggle_camera_flag.c:6 (`SetCameraTilePriorityEnabled`).
pub fn set_camera_tile_priority_enabled(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    ram::g_camera_tile_attributes(context.revision())
        .write(context, if arguments[0] as i16 != 0 { 0x8000 } else { 0 });
    Ok(None)
}
