//! Ship translation ownership; see docs/PORTING.md.

use crate::{Context, Result, execution::ExecutionClass, ids};
use tje_mem::ram;

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[crate::Translation {
    id: crate::ids::FILL_SPRITE_OPAQUE_PIXELS,
    function: fill_sprite_opaque_pixels,
}];

/// Reviewed original Atomic entries, including untranslated originals.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    // G/src/ship/ship_screen_and_collection.c:41-58: signed-word loop over
    // packed RAM pixels; no callees, device accesses, or clock-dependent reads.
    ids::FILL_SPRITE_OPAQUE_PIXELS,
    // G/src/ship/piece_level_assignment.c:9-41: ROM/RAM assignment and RNG-only
    // rejection loops. Invalid prefilled state can loop forever, but no IRQ or
    // hardware readiness can make it progress; do not repair that behavior.
    ids::ASSIGN_SHIP_PIECE_LEVELS,
    // G/src/ship/ship_screen_and_collection.c:614-648: RAM camera/indicator
    // updates and AllocateEntityWithEviction/FreeEntity's RAM-only pool scans.
    ids::SHIP_UPDATE_PENDING_PIECE_INDICATOR,
];

/// Reviewed original non-Atomic classes, independent of translation readiness.
/// Together with ATOMIC_CAPABLE these classify all fifteen ship originals in
/// both revisions; the matching sources have only relocated-address changes.
pub static EXECUTION_CONTRACTS: &[(crate::FunctionId, ExecutionClass)] = &[
    // G/src/ship/ship_screen_and_collection.c:61-105: both branches call
    // WaitForNextVBlank; the upload branch also performs stable-buffer DMA.
    (ids::ANIMATE_SHIP_SCREEN_BACKGROUND, ExecutionClass::TimeAware),
    // :108-128: stable ROM tiles/palettes and completed sprite-shadow DMA;
    // DrawHUD is Functional. No status/raster read or IRQ-dependent observer.
    (ids::RESTORE_GAMEPLAY_GRAPHICS, ExecutionClass::Functional),
    // :131-170: every branch explicitly waits for VBlank before font/upload.
    (ids::ANIMATE_TRANSITION_BACKGROUND, ExecutionClass::TimeAware),
    // :173-256: VBlank waits, IRQ-updated frame/cooldown observations, and
    // PlayEndingSequence/MapEnterFinalLevel's time-progressing scene work.
    (ids::PLAY_SHIP_COMPLETION_SEQUENCE, ExecutionClass::TimeAware),
    // :259-305: finite stable tile/palette writes, then the time-aware
    // AnimateShipScreenBackground(1). The page==4 bug is unreachable as written.
    (
        ids::INITIALIZE_SHIP_SCREEN_TILEMAP_AND_PALETTES,
        ExecutionClass::TimeAware,
    ),
    // :308-333: ninety calls of the VBlank-waiting background animator.
    // The out-of-range uninitialized piece_name is a separate frame obligation.
    (ids::SHOW_SHIP_PIECE_NAME, ExecutionClass::TimeAware),
    // :336-376: bounded RAM scan and stable ROM strings/decimal output through
    // Functional text writers; no status/raster read, wait, or unstable source.
    (ids::SHOW_MISSING_SHIP_PIECE_SUMMARY, ExecutionClass::Functional),
    // :380-416: background animation and explicit VBlank waits between fades.
    // Passing palette[16] requires an emulated call frame, not a narrower class.
    (ids::ANIMATE_SHIP_SCREEN_PALETTE, ExecutionClass::TimeAware),
    // :424-451: IRQ-updated frame timeout plus action press/release polling;
    // both the main loop and release loops call the VBlank-waiting animator.
    (
        ids::WAIT_FOR_SHIP_SCREEN_ACKNOWLEDGEMENT,
        ExecutionClass::TimeAware,
    ),
    // :468-560: background/name/palette/acknowledgement waits and optional
    // completion sequence. Its stack-local DMA sprite array needs a call frame.
    (ids::SHOW_SHIP_PIECES, ExecutionClass::TimeAware),
    // :584-610: ShowShipPieces and speech/dialogue can wait for original IRQs.
    // Its omitted StartDialogue speaker remains a residual call-frame bug.
    (ids::SHIP_PIECE_ABILITY, ExecutionClass::TimeAware),
    // :652-661: calls time-aware ShowShipPieces. Preserve the original endless
    // search when all ten assignments are zero; no synthetic completion exit.
    (ids::SHOW_RANDOM_UNCOLLECTED_SHIP_PIECE, ExecutionClass::TimeAware),
];

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
