//! Camera translation ownership; see docs/PORTING.md.

use crate::{Context, Result, execution::ExecutionClass, ids};
use tje_mem::{Bus, Revision, ram, rom};

mod animations;
mod follow;
mod movement;
mod tiles;
mod uploads;

pub use animations::{animate_players_into_split_views, animate_split_screen_wipe};
pub use follow::{get_player_camera_velocity, update_camera_following};
pub use movement::{
    move_camera_down, move_camera_left, move_camera_right, move_camera_up, try_update_camera_x,
    try_update_camera_y,
};
pub use tiles::{build_camera_tile_column, build_camera_tile_row, build_pending_camera_tiles};
pub use uploads::{
    redraw_camera_tile_rows, set_camera_position, set_camera_view_origin, update_camera_x,
    update_camera_y,
};

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::SET_CAMERA_TILE_PRIORITY_ENABLED,
        function: set_camera_tile_priority_enabled,
    },
    crate::Translation {
        id: ids::CLEAR_CAMERA_TILE_ROW,
        function: clear_camera_tile_row_hook,
    },
    crate::Translation {
        id: ids::GET_LEVEL_METATILE_TILE,
        function: get_level_metatile_tile_hook,
    },
    crate::Translation {
        id: ids::GET_OVERLAY_METATILE_TILE,
        function: get_overlay_metatile_tile_hook,
    },
    crate::Translation {
        id: ids::CLEAR_CAMERA_TILE_COLUMN,
        function: clear_camera_tile_column_hook,
    },
    crate::Translation {
        id: ids::BUILD_CAMERA_TILE_ROW,
        function: build_camera_tile_row,
    },
    crate::Translation {
        id: ids::BUILD_CAMERA_TILE_COLUMN,
        function: build_camera_tile_column,
    },
    crate::Translation {
        id: ids::BUILD_PENDING_CAMERA_TILES,
        function: build_pending_camera_tiles,
    },
    crate::Translation {
        id: ids::GET_PLAYER_CAMERA_VELOCITY,
        function: get_player_camera_velocity,
    },
    crate::Translation {
        id: ids::UPDATE_CAMERA_Y,
        function: update_camera_y,
    },
    crate::Translation {
        id: ids::UPDATE_CAMERA_X,
        function: update_camera_x,
    },
    crate::Translation {
        id: ids::REDRAW_CAMERA_TILE_ROWS,
        function: redraw_camera_tile_rows,
    },
    crate::Translation {
        id: ids::SET_CAMERA_VIEW_ORIGIN,
        function: set_camera_view_origin,
    },
    crate::Translation {
        id: ids::SET_CAMERA_POSITION,
        function: set_camera_position,
    },
    crate::Translation {
        id: ids::TRY_UPDATE_CAMERA_Y,
        function: try_update_camera_y,
    },
];

/// Complete TimeAware bodies whose real-ISR timing proofs remain blocked.
/// This table is deliberately not part of the production dispatcher or registry.
pub static PENDING_TRANSLATIONS: &[crate::Translation] = &[
    // REV 00 cooperation frame 47366: semantic Z80 RAM read A0004A is 4 in
    // the original and 255 in Rust (trace 194). Await the Z80 timing schedule;
    // this phase-dependent read is not an eligible YM BUSY stutter.
    crate::Translation {
        id: ids::TRY_UPDATE_CAMERA_X,
        function: try_update_camera_x,
    },
    // Both-revision frame-10000 caller-envelope fixtures disagree on a semantic
    // Z80 A0004A read (1 versus 3), even with equal completed ISR counts.
    crate::Translation {
        id: ids::ANIMATE_SPLIT_SCREEN_WIPE,
        function: animate_split_screen_wipe,
    },
    // Same live caller envelope: completed ISR reads A0004A as 4 versus 3.
    // Neither this read nor the wipe's read is an eligible YM BUSY stutter.
    crate::Translation {
        id: ids::ANIMATE_PLAYERS_INTO_SPLIT_VIEWS,
        function: animate_players_into_split_views,
    },
    // Full cooperation corpus: interrupt timing inside these calls admits an
    // extra original HBlank IRQ. Await the timing schedule/commutation projection
    // before re-proving them; short start-game fixtures are not sufficient.
    crate::Translation {
        id: ids::MOVE_CAMERA_UP,
        function: move_camera_up,
    },
    crate::Translation {
        id: ids::MOVE_CAMERA_DOWN,
        function: move_camera_down,
    },
    crate::Translation {
        id: ids::MOVE_CAMERA_LEFT,
        function: move_camera_left,
    },
    crate::Translation {
        id: ids::MOVE_CAMERA_RIGHT,
        function: move_camera_right,
    },
    crate::Translation {
        id: ids::UPDATE_CAMERA_FOLLOWING,
        function: update_camera_following,
    },
];

/// Reviewed original Atomic entries, including untranslated callees.
/// Tile builders and pending-bit dispatch only touch RAM/ROM; camera velocity
/// only reads animation data and calls the no-wait division runtime.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    ids::SET_CAMERA_TILE_PRIORITY_ENABLED,
    ids::CLEAR_CAMERA_TILE_ROW,
    ids::GET_LEVEL_METATILE_TILE,
    ids::GET_OVERLAY_METATILE_TILE,
    ids::CLEAR_CAMERA_TILE_COLUMN,
    // camera_tiles_and_scroll.c:68,270,522 — RAM/ROM builders and bit dispatch.
    ids::BUILD_CAMERA_TILE_ROW,
    ids::BUILD_CAMERA_TILE_COLUMN,
    ids::BUILD_PENDING_CAMERA_TILES,
    // camera_follow.c:17 — animation scan and scalar division, no hardware.
    ids::GET_PLAYER_CAMERA_VELOCITY,
];

/// Reviewed non-Atomic ORIGINAL contracts, translated or not.
/// Sources are G/src/camera/; no raster/status observations occur in the
/// Functional streams. TimeAware entries include every conditional wait path.
pub static EXECUTION_CONTRACTS: &[(crate::FunctionId, ExecutionClass)] = &[
    // camera_tiles_and_scroll.c:209,412 — stable RAM tile buffers to VDP.
    (ids::UPDATE_CAMERA_Y, ExecutionClass::Functional),
    (ids::UPDATE_CAMERA_X, ExecutionClass::Functional),
    // camera_tiles_and_scroll.c:448,477 — build/upload loops, no clock reads.
    (ids::REDRAW_CAMERA_TILE_ROWS, ExecutionClass::Functional),
    (ids::SET_CAMERA_VIEW_ORIGIN, ExecutionClass::Functional),
    // set_camera_position.c:8 — redraw, then copy fixed-point coordinates.
    (ids::SET_CAMERA_POSITION, ExecutionClass::Functional),
    // camera_tiles_and_scroll.c:551,570 — explicitly wait one VBlank.
    (ids::TRY_UPDATE_CAMERA_Y, ExecutionClass::TimeAware),
    (ids::TRY_UPDATE_CAMERA_X, ExecutionClass::TimeAware),
    // camera_tiles_and_scroll.c:589,627,665,703 — may flush via the waits above.
    (ids::MOVE_CAMERA_UP, ExecutionClass::TimeAware),
    (ids::MOVE_CAMERA_DOWN, ExecutionClass::TimeAware),
    (ids::MOVE_CAMERA_LEFT, ExecutionClass::TimeAware),
    (ids::MOVE_CAMERA_RIGHT, ExecutionClass::TimeAware),
    // camera_follow.c:97 — may call any of the four time-aware pixel movers.
    (ids::UPDATE_CAMERA_FOLLOWING, ExecutionClass::TimeAware),
    // split_screen_wipe.c:12 and split_views.c:12 — VBlank-driven animations.
    (ids::ANIMATE_SPLIT_SCREEN_WIPE, ExecutionClass::TimeAware),
    (
        ids::ANIMATE_PLAYERS_INTO_SPLIT_VIEWS,
        ExecutionClass::TimeAware,
    ),
];
/// G/src/camera/toggle_camera_flag.c:6 (`SetCameraTilePriorityEnabled`).
pub fn set_camera_tile_priority_enabled(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    ram::g_camera_tile_attributes(context.revision())
        .write(context, if arguments[0] as i16 != 0 { 0x8000 } else { 0 });
    Ok(None)
}

/// G/src/camera/camera_tiles_and_scroll.c:8 (`ClearCameraTileRow`).
/// RAM-only fixed row fill; count is stored before the pending flag.
pub fn clear_camera_tile_row(bus: &mut (impl Bus + ?Sized), revision: Revision, camera: i16) {
    let row = ram::g_camera_row_upload_words(revision).at(i32::from(camera));
    for column in 0..45 {
        let tile = ram::g_camera_tile_attributes(revision).read(bus);
        row.at(column).write(bus, tile);
    }
    ram::g_camera_row_upload_word_count(revision)
        .at(i32::from(camera))
        .write(bus, 45);
    ram::g_camera_row_upload_pending(revision)
        .at(i32::from(camera))
        .write(bus, 1);
}

/// G/src/camera/camera_tiles_and_scroll.c:23 (`GetLevelMetatileTile`).
/// Reproduce ASL.W/ADDA.W indexing, including wrapping out-of-range indices.
pub fn get_level_metatile_tile(
    bus: &(impl Bus + ?Sized),
    revision: Revision,
    metatile: u16,
    row: u16,
    column: u16,
) -> u16 {
    let mirrored = metatile & 0x80 != 0;
    let metatile = if mirrored { metatile & 0x7f } else { metatile };
    let base = rom::level_metatile_maps(revision)
        .cast::<u16>()
        .byte_offset(i32::from(metatile.wrapping_shl(8) as i16))
        .byte_offset(i32::from(row.wrapping_shl(5) as i16));
    let offset = if mirrored {
        15_u32.wrapping_sub(u32::from(column)).wrapping_mul(2) as i32
    } else {
        i32::from(column.wrapping_mul(2) as i16)
    };
    let tile = base.byte_offset(offset).read(bus);
    if mirrored { tile ^ 0x0800 } else { tile }
}

/// G/src/camera/camera_tiles_and_scroll.c:37 (`GetOverlayMetatileTile`).
/// One-based IDs, word-wrap indexing, and an unflipped empty mirrored tile.
pub fn get_overlay_metatile_tile(
    bus: &(impl Bus + ?Sized),
    revision: Revision,
    metatile: u16,
    row: u16,
    column: u16,
) -> u32 {
    let mirrored = metatile & 0x10 != 0;
    let metatile = if mirrored { metatile & 0x0f } else { metatile };
    let metatile = metatile.wrapping_sub(1);
    let base = rom::overlay_metatile_maps(revision)
        .cast::<u16>()
        .byte_offset(i32::from(metatile.wrapping_shl(8) as i16))
        .byte_offset(i32::from(row.wrapping_shl(5) as i16));
    let offset = if mirrored {
        15_u32.wrapping_sub(u32::from(column)).wrapping_mul(2) as i32
    } else {
        i32::from(column.wrapping_mul(2) as i16)
    };
    let tile = base.byte_offset(offset).read(bus);
    u32::from(if mirrored && tile != 0 {
        tile ^ 0x0800
    } else {
        tile
    })
}

/// G/src/camera/camera_tiles_and_scroll.c:242 (`ClearCameraTileColumn`).
/// Re-read the signed height every iteration; aliased writes may change it.
pub fn clear_camera_tile_column(bus: &mut (impl Bus + ?Sized), revision: Revision, camera: i16) {
    let column = ram::g_camera_column_upload_words(revision)
        .at(i32::from(camera))
        .cast::<u16>();
    let mut row = 0_i16;
    while row < ram::g_camera_column_upload_height(revision).read(bus) {
        let tile = ram::g_camera_tile_attributes(revision).read(bus);
        column
            .byte_offset(i32::from(row.wrapping_mul(2)))
            .write(bus, tile);
        row = row.wrapping_add(1);
    }
    ram::g_camera_column_upload_pending(revision)
        .at(i32::from(camera))
        .write(bus, 1);
    let count = ram::g_camera_column_upload_height_low_byte(revision).read(bus);
    ram::g_camera_column_upload_word_count(revision)
        .at(i32::from(camera))
        .write(bus, count);
}

/// G/src/camera/camera_tiles_and_scroll.c:8 (`ClearCameraTileRow`) stack bridge.
pub fn clear_camera_tile_row_hook(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    clear_camera_tile_row(context, context.revision(), arguments[0] as i16);
    Ok(None)
}

/// G/src/camera/camera_tiles_and_scroll.c:23 (`GetLevelMetatileTile`) stack bridge.
pub fn get_level_metatile_tile_hook(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    Ok(Some(u32::from(get_level_metatile_tile(
        context,
        context.revision(),
        arguments[0] as u16,
        arguments[1] as u16,
        arguments[2] as u16,
    ))))
}

/// G/src/camera/camera_tiles_and_scroll.c:37 (`GetOverlayMetatileTile`) stack bridge.
pub fn get_overlay_metatile_tile_hook(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    Ok(Some(get_overlay_metatile_tile(
        context,
        context.revision(),
        arguments[0] as u16,
        arguments[1] as u16,
        arguments[2] as u16,
    )))
}

/// G/src/camera/camera_tiles_and_scroll.c:242 (`ClearCameraTileColumn`) stack bridge.
pub fn clear_camera_tile_column_hook(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    clear_camera_tile_column(context, context.revision(), arguments[0] as i16);
    Ok(None)
}

#[cfg(test)]
mod tests;
