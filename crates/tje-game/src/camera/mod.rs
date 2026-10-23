//! Camera translation ownership; see docs/PORTING.md.

use crate::{Context, Result, ids};
use tje_mem::{Bus, Revision, ram, rom};

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
];

/// Reviewed original entries that return without clock/interrupt progress.
/// Missing IDs are deliberately not approved for atomic checking or fallback.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    ids::SET_CAMERA_TILE_PRIORITY_ENABLED,
    ids::CLEAR_CAMERA_TILE_ROW,
    ids::GET_LEVEL_METATILE_TILE,
    ids::GET_OVERLAY_METATILE_TILE,
    ids::CLEAR_CAMERA_TILE_COLUMN,
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
