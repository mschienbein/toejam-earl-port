//! Ship translation ownership; see docs/PORTING.md.

mod background;
mod collection;
mod screen;
pub use background::{
    animate_ship_screen_background, animate_transition_background,
    initialize_ship_screen_tilemap_and_palettes, restore_gameplay_graphics,
    show_missing_ship_piece_summary, show_ship_piece_name, wait_for_ship_screen_acknowledgement,
};
pub use collection::{
    play_ship_completion_sequence, ship_piece_ability, ship_update_pending_piece_indicator,
    show_random_uncollected_ship_piece,
};
pub use screen::{animate_ship_screen_palette, show_ship_pieces};

use crate::{Context, Result, execution::ExecutionClass, ids};
use tje_mem::{ram, rom};

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::FILL_SPRITE_OPAQUE_PIXELS,
        function: fill_sprite_opaque_pixels,
    },
    crate::Translation {
        id: ids::ASSIGN_SHIP_PIECE_LEVELS,
        function: assign_ship_piece_levels,
    },
    crate::Translation {
        id: ids::SHIP_UPDATE_PENDING_PIECE_INDICATOR,
        function: ship_update_pending_piece_indicator,
    },
    crate::Translation {
        id: ids::SHOW_MISSING_SHIP_PIECE_SUMMARY,
        function: show_missing_ship_piece_summary,
    },
    crate::Translation {
        id: ids::RESTORE_GAMEPLAY_GRAPHICS,
        function: restore_gameplay_graphics,
    },
];

/// Complete bodies awaiting their explicitly reviewed check-mode proofs.
/// Never registered by production or replacement mode.
pub static PENDING_TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::ANIMATE_SHIP_SCREEN_BACKGROUND,
        function: animate_ship_screen_background,
    },
    crate::Translation {
        id: ids::ANIMATE_TRANSITION_BACKGROUND,
        function: animate_transition_background,
    },
    crate::Translation {
        id: ids::INITIALIZE_SHIP_SCREEN_TILEMAP_AND_PALETTES,
        function: initialize_ship_screen_tilemap_and_palettes,
    },
    crate::Translation {
        id: ids::SHOW_SHIP_PIECE_NAME,
        function: show_ship_piece_name,
    },
    crate::Translation {
        id: ids::WAIT_FOR_SHIP_SCREEN_ACKNOWLEDGEMENT,
        function: wait_for_ship_screen_acknowledgement,
    },
    crate::Translation {
        id: ids::PLAY_SHIP_COMPLETION_SEQUENCE,
        function: play_ship_completion_sequence,
    },
    crate::Translation {
        id: ids::SHIP_PIECE_ABILITY,
        function: ship_piece_ability,
    },
    crate::Translation {
        id: ids::SHOW_RANDOM_UNCOLLECTED_SHIP_PIECE,
        function: show_random_uncollected_ship_piece,
    },
    crate::Translation {
        id: ids::ANIMATE_SHIP_SCREEN_PALETTE,
        function: animate_ship_screen_palette,
    },
    crate::Translation {
        id: ids::SHOW_SHIP_PIECES,
        function: show_ship_pieces,
    },
];

pub static PENDING: &[crate::FunctionId] = &[
    // Real-clock counter-only proof differs by one IRQ frame in both revisions.
    ids::ANIMATE_SHIP_SCREEN_BACKGROUND,
    // All 256 bytes pass at frame 2000; real audio IRQ writes differ at frame 10000.
    ids::ANIMATE_TRANSITION_BACKGROUND,
    // The original plane fill consumes eight additional IRQ frames at frame 2000.
    ids::INITIALIZE_SHIP_SCREEN_TILEMAP_AND_PALETTES,
    // The original's fixed 1,000,000-tick check bound expires before 90 frames.
    ids::SHOW_SHIP_PIECE_NAME,
    // The same timed bound expires before the 240-frame acknowledgement timeout.
    ids::WAIT_FOR_SHIP_SCREEN_ACKNOWLEDGEMENT,
    // Frame-backed palette array and retained outgoing slots; timed proof pending.
    ids::ANIMATE_SHIP_SCREEN_PALETTE,
    // dependent residual: awaiting dependent-equivalence proof
    ids::SHOW_SHIP_PIECES,
    // PlayEndingSequence transitively reads EndingPlayBouncingRocketshipScene's
    // uninitialized movement_frame stack word; residual call-frame API required.
    ids::PLAY_SHIP_COMPLETION_SEQUENCE,
    // Source-backed ShowShipPieces callers retain their original stack layout;
    // the callee's residual DMA and scene timing remain unapproved.
    ids::SHIP_PIECE_ABILITY,
    ids::SHOW_RANDOM_UNCOLLECTED_SHIP_PIECE,
];

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
    (
        ids::ANIMATE_SHIP_SCREEN_BACKGROUND,
        ExecutionClass::TimeAware,
    ),
    // :108-128: stable ROM tiles/palettes and completed sprite-shadow DMA;
    // DrawHUD is Functional. No status/raster read or IRQ-dependent observer.
    (ids::RESTORE_GAMEPLAY_GRAPHICS, ExecutionClass::Functional),
    // :131-170: every branch explicitly waits for VBlank before font/upload.
    (
        ids::ANIMATE_TRANSITION_BACKGROUND,
        ExecutionClass::TimeAware,
    ),
    // :173-256: VBlank waits, IRQ-updated frame/cooldown observations, and
    // PlayEndingSequence/MapEnterFinalLevel's time-progressing scene work.
    (
        ids::PLAY_SHIP_COMPLETION_SEQUENCE,
        ExecutionClass::TimeAware,
    ),
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
    (
        ids::SHOW_MISSING_SHIP_PIECE_SUMMARY,
        ExecutionClass::Functional,
    ),
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
    (
        ids::SHOW_RANDOM_UNCOLLECTED_SHIP_PIECE,
        ExecutionClass::TimeAware,
    ),
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

/// G/src/ship/piece_level_assignment.c:9 (`AssignShipPieceLevels`).
/// RNG results are unsigned words; every divisor here fits native DIVU.
pub fn assign_ship_piece_levels(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let assignments = ram::g_ship_piece_level_assignments(revision);
    if ram::g_world_type(revision).read(context) != 0 {
        for piece in 0..10 {
            let level = rom::fixed_world_ship_pieces(revision)
                .at(piece)
                .read(context);
            assignments.at(piece).write(context, level);
        }
        return Ok(None);
    }
    for group in 0..5 {
        let piece = loop {
            let candidate =
                i32::from(crate::call(context, ids::GET_RANDOM_NUMBER, &[])? as u16 % 10);
            if assignments.at(candidate).read(context) == 0 {
                break candidate;
            }
        };
        loop {
            let level = (u32::from(crate::call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 5
                + (group * 5 + 1)) as i8;
            assignments.at(piece).write(context, level);
            if level > 1 && level < 25 {
                break;
            }
        }
    }
    for _group in 0..5 {
        let piece = loop {
            let candidate =
                i32::from(crate::call(context, ids::GET_RANDOM_NUMBER, &[])? as u16 % 10);
            if assignments.at(candidate).read(context) == 0 {
                break candidate;
            }
        };
        let level = loop {
            let candidate =
                (crate::call(context, ids::GET_RANDOM_NUMBER, &[])? as u16 % 23 + 2) as i16;
            if !(0..10)
                .any(|previous| i16::from(assignments.at(previous).read(context)) == candidate)
            {
                break candidate;
            }
        };
        assignments.at(piece).write(context, level as i8);
    }
    let piece = i32::from(crate::call(context, ids::GET_RANDOM_NUMBER, &[])? as u16 % 10);
    assignments.at(piece).write(context, 25);
    Ok(None)
}
