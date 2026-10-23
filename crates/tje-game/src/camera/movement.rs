use crate::{Context, Result, call, ids};
use tje_mem::ram;

/// G/src/camera/camera_tiles_and_scroll.c:551 (`TryUpdateCameraY`).
pub fn try_update_camera_y(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    flush(context, args[0] as i32, true)
}
/// G/src/camera/camera_tiles_and_scroll.c:570 (`TryUpdateCameraX`).
pub fn try_update_camera_x(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    flush(context, args[0] as i32, false)
}
fn flush(context: &mut dyn Context, camera: i32, vertical: bool) -> Result<Option<u32>> {
    let revision = context.revision();
    let pending = if vertical {
        ram::g_camera_row_upload_pending(revision)
    } else {
        ram::g_camera_column_upload_pending(revision)
    };
    let other = pending.at(camera ^ 1);
    let saved = other.read(context);
    other.write(context, 0);
    ram::g_v_blank_audio_input_override(revision).write(context, 1);
    ram::g_v_blank_graphics_paused(revision).write(context, 1);
    call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    call(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
    call(
        context,
        if vertical {
            ids::UPDATE_CAMERA_Y
        } else {
            ids::UPDATE_CAMERA_X
        },
        &[],
    )?;
    other.write(context, saved);
    call(context, ids::ENABLE_BLANKING_GRAPHICS, &[])?;
    ram::g_v_blank_graphics_paused(revision).write(context, 0);
    ram::g_v_blank_audio_input_override(revision).write(context, 0);
    Ok(None)
}

#[derive(Clone, Copy)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// G/src/camera/camera_tiles_and_scroll.c:589 (`MoveCameraUp`).
pub fn move_camera_up(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    move_pixel(context, args[0] as i16, Direction::Up)
}
/// G/src/camera/camera_tiles_and_scroll.c:627 (`MoveCameraDown`).
pub fn move_camera_down(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    move_pixel(context, args[0] as i16, Direction::Down)
}
/// G/src/camera/camera_tiles_and_scroll.c:665 (`MoveCameraLeft`).
pub fn move_camera_left(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    move_pixel(context, args[0] as i16, Direction::Left)
}
/// G/src/camera/camera_tiles_and_scroll.c:703 (`MoveCameraRight`).
pub fn move_camera_right(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    move_pixel(context, args[0] as i16, Direction::Right)
}
fn move_pixel(
    context: &mut dyn Context,
    mut camera: i16,
    direction: Direction,
) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_player_lives(revision).at(0).read(context) < 0 {
        camera = 0;
    }
    let selected = if camera == 0 { 0 } else { 1 };
    let vertical = matches!(direction, Direction::Up | Direction::Down);
    let position = match (selected, vertical) {
        (0, true) => ram::g_camera1_pending_y(revision),
        (0, false) => ram::g_camera1_pending_x(revision),
        (_, true) => ram::g_camera2_pending_y(revision),
        (_, false) => ram::g_camera2_pending_x(revision),
    };
    let allowed = match direction {
        Direction::Up | Direction::Left => position.read(context) > 8,
        Direction::Down => {
            (ram::g_split_screen_enabled(revision).read(context) != 0
                && position.read(context) < 1672)
                || position.read(context) < 1560
        }
        Direction::Right => position.read(context) < 2360,
    };
    if !allowed {
        return Ok(Some(1));
    }
    let delta = if matches!(direction, Direction::Up | Direction::Left) {
        -1
    } else {
        1
    };
    let next = position.read(context).wrapping_add(delta);
    position.write(context, next);
    let new_cursor = (position.read(context) >> 3).wrapping_sub(1);
    let cursor = if vertical {
        super::tiles::row_cursor(revision, camera)
    } else {
        super::tiles::column_cursor(revision, camera)
    };
    if new_cursor != cursor.read(context) {
        let pending = if vertical {
            ram::g_camera_row_upload_pending(revision)
        } else {
            ram::g_camera_column_upload_pending(revision)
        };
        if pending.at(selected).read(context) != 0 {
            call(
                context,
                if vertical {
                    ids::TRY_UPDATE_CAMERA_Y
                } else {
                    ids::TRY_UPDATE_CAMERA_X
                },
                &[selected as u32],
            )?;
        }
        let directions = if selected == 0 {
            ram::g_camera1_pending_tile_directions(revision)
        } else {
            ram::g_camera2_pending_tile_directions(revision)
        };
        let bit = match direction {
            Direction::Up => 1,
            Direction::Down => 4,
            Direction::Left => 8,
            Direction::Right => 2,
        };
        let flags = directions.read(context) | bit;
        directions.write(context, flags);
        let pending_cursor = match (selected, vertical) {
            (0, true) => ram::g_camera1_pending_tile_row(revision),
            (0, false) => ram::g_camera1_pending_tile_column(revision),
            (_, true) => ram::g_camera2_pending_tile_row(revision),
            (_, false) => ram::g_camera2_pending_tile_column(revision),
        };
        pending_cursor.write(context, new_cursor);
    }
    if ram::g_player_lives(revision).at(0).read(context) < 0 {
        let (first, second) = if vertical {
            (
                ram::g_camera1_pending_y(revision),
                ram::g_camera2_pending_y(revision),
            )
        } else {
            (
                ram::g_camera1_pending_x(revision),
                ram::g_camera2_pending_x(revision),
            )
        };
        let value = first.read(context);
        second.write(context, value);
    }
    Ok(Some(0))
}
