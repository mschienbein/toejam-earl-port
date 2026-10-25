use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, Revision, ram};

pub(super) fn row_cursor(revision: Revision, camera: i16) -> Address<i16> {
    if camera == 0 {
        ram::g_camera1_tile_row_cursor(revision)
    } else {
        ram::g_camera2_tile_row_cursor(revision)
    }
}
pub(super) fn column_cursor(revision: Revision, camera: i16) -> Address<i16> {
    if camera == 0 {
        ram::g_camera1_tile_column_cursor(revision)
    } else {
        ram::g_camera2_tile_column_cursor(revision)
    }
}

/// G/src/camera/camera_tiles_and_scroll.c:68 (`BuildCameraTileRow`).
/// Preserve wrapped cursors, unchecked negative map coordinates, and the fixed
/// published count even when the world-edge loop terminates before 45 words.
pub fn build_camera_tile_row(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let top_edge = args[0] as i16;
    let camera = args[1] as i16;
    let row_cursor = row_cursor(revision, camera);
    let column_cursor = column_cursor(revision, camera);
    let (screen_row, tile_row, mut map_row) = if top_edge != 0 {
        (
            row_cursor.read(context) & 31,
            row_cursor.read(context) & 7,
            row_cursor.read(context) >> 3,
        )
    } else {
        let row = row_cursor
            .read(context)
            .wrapping_add(ram::g_camera_column_upload_height(revision).read(context))
            .wrapping_sub(3);
        if row >= 224 {
            return Ok(None);
        }
        (row & 31, row & 7, row >> 3)
    };
    let map = if camera == 0 {
        ram::g_player1_level_map(revision)
    } else {
        ram::g_player2_level_map(revision)
    };
    let level = if camera == 0 {
        ram::g_player1_map_level(revision)
    } else {
        ram::g_player2_map_level(revision)
    };
    let wraps = level.read(context) == 1 || level.read(context) == 26;
    let previous = column_cursor.read(context).wrapping_sub(1);
    column_cursor.write(context, previous);
    let metadata_camera = if camera == 0 { 0 } else { 1 };
    ram::g_camera_row_upload_start_column(revision)
        .at(metadata_camera)
        .write(context, (column_cursor.read(context) & 63) as i8);
    let mut world_column = column_cursor.read(context);
    let restored = column_cursor.read(context).wrapping_add(1);
    column_cursor.write(context, restored);
    ram::g_camera_row_upload_vram_row(revision)
        .at(i32::from(camera))
        .write(context, screen_row as i8);
    let blank = map_row == 0 || map_row > 26;
    if blank && !wraps {
        call_void(
            context,
            ids::CLEAR_CAMERA_TILE_ROW,
            &[i32::from(camera) as u32],
        )?;
        return Ok(None);
    }
    map_row = map_row.wrapping_sub(1);
    let mut count = 0_i16;
    while count < 45 && world_column < 336 {
        let tile_column = world_column & 15;
        let mut map_x = world_column >> 4;
        let mut tile = 0_u16;
        let mut metatile;
        if blank {
            metatile = 82;
        } else if map_x == 0 || map_x > 19 {
            if !wraps {
                let attributes = ram::g_camera_tile_attributes(revision).read(context);
                ram::g_camera_row_upload_words(revision)
                    .at(i32::from(camera))
                    .at(i32::from(count))
                    .write(context, attributes);
                count = count.wrapping_add(1);
                world_column = world_column.wrapping_add(1);
                continue;
            }
            metatile = 82;
        } else {
            map_x = map_x.wrapping_sub(1);
            let cell = map
                .cast::<u16>()
                .offset(i32::from(map_row) * 19 + i32::from(map_x))
                .read(context);
            metatile = ((cell & 0x1f00) >> 8) as u8;
            if metatile != 0 {
                tile = call(
                    context,
                    ids::GET_OVERLAY_METATILE_TILE,
                    &[
                        u32::from(metatile),
                        i32::from(tile_row) as u32,
                        i32::from(tile_column) as u32,
                    ],
                )? as u16;
            }
            if tile == 0 {
                metatile = cell as u8;
            }
        }
        if tile == 0 {
            tile = call(
                context,
                ids::GET_LEVEL_METATILE_TILE,
                &[
                    u32::from(metatile),
                    i32::from(tile_row) as u32,
                    i32::from(tile_column) as u32,
                ],
            )? as u16;
        }
        let attributes = ram::g_camera_tile_attributes(revision).read(context);
        ram::g_camera_row_upload_words(revision)
            .at(i32::from(camera))
            .at(i32::from(count))
            .write(context, tile | attributes);
        count = count.wrapping_add(1);
        world_column = world_column.wrapping_add(1);
    }
    ram::g_camera_row_upload_word_count(revision)
        .at(i32::from(camera))
        .write(context, 45);
    ram::g_camera_row_upload_pending(revision)
        .at(i32::from(camera))
        .write(context, 1);
    Ok(None)
}

/// G/src/camera/camera_tiles_and_scroll.c:270 (`BuildCameraTileColumn`).
/// The wrap-level check uses camera == 1, not merely a nonzero selector.
pub fn build_camera_tile_column(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let left_edge = args[0] as i16;
    let camera = args[1] as i16;
    let column_cursor = column_cursor(revision, camera);
    let row_cursor = row_cursor(revision, camera);
    let (screen_column, tile_column, mut map_x) = if left_edge != 0 {
        (
            column_cursor.read(context) & 63,
            column_cursor.read(context) & 15,
            column_cursor.read(context) >> 4,
        )
    } else {
        let column = column_cursor.read(context).wrapping_add(42);
        if column >= 336 {
            return Ok(None);
        }
        (column & 63, column & 15, column >> 4)
    };
    let previous = row_cursor.read(context).wrapping_sub(1);
    row_cursor.write(context, previous);
    ram::g_camera_column_upload_start_row(revision)
        .at(i32::from(camera))
        .write(context, (row_cursor.read(context) & 31) as i8);
    let mut world_row = row_cursor.read(context);
    let restored = row_cursor.read(context).wrapping_add(1);
    row_cursor.write(context, restored);
    let map = if camera == 0 {
        ram::g_player1_level_map(revision)
    } else {
        ram::g_player2_level_map(revision)
    };
    ram::g_camera_column_upload_vram_column(revision)
        .at(i32::from(camera))
        .write(context, screen_column as i8);
    let wraps = (camera == 0
        && (ram::g_player1_map_level(revision).read(context) == 1
            || ram::g_player1_map_level(revision).read(context) == 26))
        || (camera == 1
            && (ram::g_player2_map_level(revision).read(context) == 1
                || ram::g_player2_map_level(revision).read(context) == 26));
    let blank = map_x == 0 || map_x > 19;
    if blank && !wraps {
        call_void(
            context,
            ids::CLEAR_CAMERA_TILE_COLUMN,
            &[i32::from(camera) as u32],
        )?;
        return Ok(None);
    }
    map_x = map_x.wrapping_sub(1);
    let mut count = 0_i16;
    while count < ram::g_camera_column_upload_height(revision).read(context) && world_row < 224 {
        let tile_row = world_row & 7;
        let mut map_row = world_row >> 3;
        let mut tile = 0_u16;
        let mut metatile;
        if blank {
            metatile = 82;
        } else if map_row == 0 || map_row > 26 {
            if !wraps {
                let attributes = ram::g_camera_tile_attributes(revision).read(context);
                ram::g_camera_column_upload_words(revision)
                    .at(i32::from(camera))
                    .cast::<u16>()
                    .byte_offset(i32::from(count.wrapping_mul(2)))
                    .write(context, attributes);
                count = count.wrapping_add(1);
                world_row = world_row.wrapping_add(1);
                continue;
            }
            metatile = 82;
        } else {
            map_row = map_row.wrapping_sub(1);
            let cell = map
                .cast::<u16>()
                .offset(i32::from(map_row) * 19 + i32::from(map_x))
                .read(context);
            metatile = ((cell & 0x1f00) >> 8) as u8;
            if metatile != 0 {
                tile = call(
                    context,
                    ids::GET_OVERLAY_METATILE_TILE,
                    &[
                        u32::from(metatile),
                        i32::from(tile_row) as u32,
                        i32::from(tile_column) as u32,
                    ],
                )? as u16;
            }
            if tile == 0 {
                metatile = cell as u8;
            }
        }
        if tile == 0 {
            tile = call(
                context,
                ids::GET_LEVEL_METATILE_TILE,
                &[
                    u32::from(metatile),
                    i32::from(tile_row) as u32,
                    i32::from(tile_column) as u32,
                ],
            )? as u16;
        }
        let attributes = ram::g_camera_tile_attributes(revision).read(context);
        ram::g_camera_column_upload_words(revision)
            .at(i32::from(camera))
            .cast::<u16>()
            .byte_offset(i32::from(count.wrapping_mul(2)))
            .write(context, tile | attributes);
        count = count.wrapping_add(1);
        world_row = world_row.wrapping_add(1);
    }
    let count = ram::g_camera_column_upload_height_low_byte(revision).read(context);
    ram::g_camera_column_upload_word_count(revision)
        .at(i32::from(camera))
        .write(context, count);
    ram::g_camera_column_upload_pending(revision)
        .at(i32::from(camera))
        .write(context, 1);
    Ok(None)
}

/// G/src/camera/camera_tiles_and_scroll.c:522 (`BuildPendingCameraTiles`).
pub fn build_pending_camera_tiles(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for camera in 0..2 {
        let directions = if camera == 0 {
            ram::g_camera1_pending_tile_directions(revision)
        } else {
            ram::g_camera2_pending_tile_directions(revision)
        };
        let pending_row = if camera == 0 {
            ram::g_camera1_pending_tile_row(revision)
        } else {
            ram::g_camera2_pending_tile_row(revision)
        };
        let pending_column = if camera == 0 {
            ram::g_camera1_pending_tile_column(revision)
        } else {
            ram::g_camera2_pending_tile_column(revision)
        };
        let vertical = directions.read(context) & 5;
        if vertical != 0 && vertical != 5 {
            let row = pending_row.read(context);
            row_cursor(revision, camera).write(context, row);
            call_void(
                context,
                ids::BUILD_CAMERA_TILE_ROW,
                &[(vertical & 1) as u32, camera as u32],
            )?;
        }
        let horizontal = directions.read(context) & 10;
        if horizontal != 0 && horizontal != 10 {
            let column = pending_column.read(context);
            column_cursor(revision, camera).write(context, column);
            call_void(
                context,
                ids::BUILD_CAMERA_TILE_COLUMN,
                &[(horizontal & 8) as u32, camera as u32],
            )?;
        }
        directions.write(context, 0);
    }
    Ok(None)
}
