use crate::{Context, Result, call_void, ids};
use tje_mem::ram;

fn plane(camera: i32) -> u32 {
    if camera == 0 { 0x60000002 } else { 0x40000003 }
}

/// G/src/camera/camera_tiles_and_scroll.c:209 (`UpdateCameraY`).
/// Functional: stable RAM row transfers; the second run wraps at VRAM column 64.
pub fn update_camera_y(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for camera in 0..2 {
        let pending = ram::g_camera_row_upload_pending(revision).at(camera);
        if pending.read(context) != 0 {
            let start = i16::from(
                ram::g_camera_row_upload_start_column(revision)
                    .at(camera)
                    .read(context),
            );
            let count = i16::from(
                ram::g_camera_row_upload_word_count(revision)
                    .at(camera)
                    .read(context),
            );
            let row = ram::g_camera_row_upload_vram_row(revision)
                .at(camera)
                .read(context);
            let command = plane(camera)
                .wrapping_add((i32::from(row) as u32).wrapping_shl(23))
                .wrapping_add((i32::from(start) as u32).wrapping_shl(17));
            context.write_vdp_command(command);
            let source = ram::g_camera_row_upload_words(revision)
                .at(camera)
                .cast::<u16>();
            if i32::from(start) + i32::from(count) <= 64 {
                call_void(
                    context,
                    ids::MOVE_DATA_TO_VRAM,
                    &[source.address(), i32::from(count) as u32],
                )?;
            } else {
                let first = 64_i16.wrapping_sub(start);
                call_void(
                    context,
                    ids::MOVE_DATA_TO_VRAM,
                    &[source.address(), i32::from(first) as u32],
                )?;
                let row = ram::g_camera_row_upload_vram_row(revision)
                    .at(camera)
                    .read(context);
                context.write_vdp_command(
                    plane(camera).wrapping_add((i32::from(row) as u32).wrapping_shl(23)),
                );
                call_void(
                    context,
                    ids::MOVE_DATA_TO_VRAM,
                    &[
                        source.offset(i32::from(first)).address(),
                        (i32::from(count) - i32::from(first)) as u32,
                    ],
                )?;
            }
            pending.write(context, 0);
        }
    }
    Ok(None)
}

/// G/src/camera/camera_tiles_and_scroll.c:412 (`UpdateCameraX`).
/// Functional: column writes temporarily select auto-increment 128, then restore 2.
pub fn update_camera_x(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    context.write_vdp_control(0x8f80);
    for camera in 0..2 {
        let pending = ram::g_camera_column_upload_pending(revision).at(camera);
        if pending.read(context) != 0 {
            let start = i16::from(
                ram::g_camera_column_upload_start_row(revision)
                    .at(camera)
                    .read(context),
            );
            let count = ram::g_camera_column_upload_word_count(revision)
                .at(camera)
                .read(context);
            let column = ram::g_camera_column_upload_vram_column(revision)
                .at(camera)
                .read(context);
            context.write_vdp_command(
                plane(camera)
                    .wrapping_add((i32::from(start) as u32).wrapping_shl(23))
                    .wrapping_add((i32::from(column) as u32).wrapping_shl(17)),
            );
            let source = ram::g_camera_column_upload_words(revision)
                .at(camera)
                .cast::<u16>();
            if i32::from(start) + i32::from(count) <= 32 {
                call_void(
                    context,
                    ids::MOVE_DATA_TO_VRAM,
                    &[source.address(), i32::from(count) as u32],
                )?;
            } else {
                let first = 32_i16.wrapping_sub(start);
                call_void(
                    context,
                    ids::MOVE_DATA_TO_VRAM,
                    &[source.address(), i32::from(first) as u32],
                )?;
                let column = ram::g_camera_column_upload_vram_column(revision)
                    .at(camera)
                    .read(context);
                context.write_vdp_command(
                    plane(camera).wrapping_add((i32::from(column) as u32).wrapping_shl(17)),
                );
                call_void(
                    context,
                    ids::MOVE_DATA_TO_VRAM,
                    &[
                        source.offset(i32::from(first)).address(),
                        (i32::from(count) - i32::from(first)) as u32,
                    ],
                )?;
            }
            pending.write(context, 0);
        }
    }
    context.write_vdp_control(0x8f02);
    Ok(None)
}

/// G/src/camera/camera_tiles_and_scroll.c:448 (`RedrawCameraTileRows`).
pub fn redraw_camera_tile_rows(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let camera = args[0] as i16;
    let selected = if camera == 0 { 0 } else { 1 };
    let cursor = super::tiles::row_cursor(revision, camera);
    let end = cursor
        .read(context)
        .wrapping_add(ram::g_camera_column_upload_height(revision).read(context));
    while end > cursor.read(context) {
        call_void(context, ids::BUILD_CAMERA_TILE_ROW, &[1, selected])?;
        call_void(context, ids::UPDATE_CAMERA_Y, &[])?;
        let next = cursor.read(context).wrapping_add(1);
        cursor.write(context, next);
    }
    let restored = end.wrapping_sub(ram::g_camera_column_upload_height(revision).read(context));
    cursor.write(context, restored);
    let directions = if camera == 0 {
        ram::g_camera1_pending_tile_directions(revision)
    } else {
        ram::g_camera2_pending_tile_directions(revision)
    };
    directions.write(context, 0);
    ram::g_camera_row_upload_pending(revision)
        .at(i32::from(camera))
        .write(context, 0);
    ram::g_camera_column_upload_pending(revision)
        .at(i32::from(camera))
        .write(context, 0);
    Ok(None)
}

/// G/src/camera/camera_tiles_and_scroll.c:477 (`SetCameraViewOrigin`).
pub fn set_camera_view_origin(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut camera = args[0] as i16;
    let mut x = args[1] as i16;
    let mut y = args[2] as i16;
    if ram::g_player_lives(revision).at(0).read(context) < 0 {
        camera = 0;
    }
    x = x.clamp(8, 2360);
    if y < 8 {
        y = 8;
    }
    if ram::g_split_screen_enabled(revision).read(context) != 0 {
        if y > 1672 {
            y = 1672;
        }
        ram::g_camera_column_upload_height(revision).write(context, 19);
    } else {
        if y > 1560 {
            y = 1560;
        }
        ram::g_camera_column_upload_height(revision).write(context, 32);
    }
    let pending_x = if camera == 0 {
        ram::g_camera1_pending_x(revision)
    } else {
        ram::g_camera2_pending_x(revision)
    };
    let pending_y = if camera == 0 {
        ram::g_camera1_pending_y(revision)
    } else {
        ram::g_camera2_pending_y(revision)
    };
    pending_x.write(context, x);
    pending_y.write(context, y);
    super::tiles::column_cursor(revision, camera).write(context, (x >> 3).wrapping_sub(1));
    super::tiles::row_cursor(revision, camera).write(context, (y >> 3).wrapping_sub(1));
    call_void(
        context,
        ids::REDRAW_CAMERA_TILE_ROWS,
        &[i32::from(camera) as u32],
    )?;
    if ram::g_player_lives(revision).at(0).read(context) < 0 {
        let x = ram::g_camera1_pending_x(revision).read(context);
        ram::g_camera2_pending_x(revision).write(context, x);
        let y = ram::g_camera1_pending_y(revision).read(context);
        ram::g_camera2_pending_y(revision).write(context, y);
    }
    Ok(None)
}

/// G/src/camera/set_camera_position.c:8 (`SetCameraPosition`).
pub fn set_camera_position(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut player = args[0] as i32;
    if ram::g_player_lives(revision).at(0).read(context) < 0 {
        player = 0;
    }
    call_void(
        context,
        ids::SET_CAMERA_VIEW_ORIGIN,
        &[player as u32, args[1], args[2]],
    )?;
    let (x, y, fixed_x, fixed_y) = if player == 0 {
        (
            ram::g_camera1_pending_x(revision),
            ram::g_camera1_pending_y(revision),
            ram::g_camera1_follow_x_fixed(revision),
            ram::g_camera1_follow_y_fixed(revision),
        )
    } else {
        (
            ram::g_camera2_pending_x(revision),
            ram::g_camera2_pending_y(revision),
            ram::g_camera2_follow_x_fixed(revision),
            ram::g_camera2_follow_y_fixed(revision),
        )
    };
    fixed_x.write(context, i32::from(x.read(context)).wrapping_shl(8));
    fixed_y.write(context, i32::from(y.read(context)).wrapping_shl(8));
    Ok(None)
}
