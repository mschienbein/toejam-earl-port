//! Ship translation ownership; see docs/PORTING.md.

use crate::{Context, Result};
use tje_mem::ram;

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[crate::Translation {
    id: crate::ids::FILL_SPRITE_OPAQUE_PIXELS,
    function: fill_sprite_opaque_pixels,
}];

/// The original has no callees or hardware accesses. Its signed word loop is
/// bounded by the narrowed tile byte count and requires no clock progress.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[crate::ids::FILL_SPRITE_OPAQUE_PIXELS];

/// G/src/ship/ship_screen_and_collection.c:41 (`FillSpriteOpaquePixels`).
/// Shift/narrow the signed tile count before comparing the signed byte counter.
pub fn fill_sprite_opaque_pixels(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let byte_count = (arguments[0] as i16).wrapping_shl(5);
    let offset = ram::g_sprite_tile_buffer_offset(revision).read(context);
    let mut pixels = ram::g_sprite_tile_upload_buffer(revision)
        .at(i32::from(offset))
        .address();
    let mut byte_number = 0_i16;
    while byte_number < byte_count {
        let mut packed_pixels = context.read_u8(pixels);
        if packed_pixels & 15 != 0 {
            packed_pixels |= 15;
        }
        if packed_pixels & 240 != 0 {
            packed_pixels |= 240;
        }
        context.write_u8(pixels, packed_pixels);
        pixels = pixels.wrapping_add(1);
        byte_number = byte_number.wrapping_add(1);
    }
    Ok(None)
}
