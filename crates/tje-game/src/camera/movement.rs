use crate::{
    Context, Result, call_at_void,
    call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments},
    ids,
};
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
    // c_camera_tiles_and_scroll.s:1208/1238: D2-D3/A2 survive the flush,
    // including its outgoing Wait argument through both following calls.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x04))?;
    let result = flush_body(context, camera, vertical);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result.map(|()| None);
    }
    let leave = frame.leave(context);
    result?;
    leave?;
    Ok(None)
}

fn flush_body(context: &mut dyn Context, camera: i32, vertical: bool) -> Result<()> {
    let revision = context.revision();
    let pending = if vertical {
        ram::g_camera_row_upload_pending(revision)
    } else {
        ram::g_camera_column_upload_pending(revision)
    };
    context.set_data_register(2, camera as u32);
    context.set_address_register(2, pending.address());
    let other = pending.at(camera ^ 1);
    let saved = other.read(context);
    let d3 = context.data_register(3);
    context.set_data_register(3, (d3 & !0xff) | u32::from(saved as u8));
    context.discard_u8(other.address())?;
    other.write(context, 0);
    ram::g_v_blank_audio_input_override(revision).write(context, 1);
    ram::g_v_blank_graphics_paused(revision).write(context, 1);
    let caller = if vertical {
        ids::TRY_UPDATE_CAMERA_Y
    } else {
        ids::TRY_UPDATE_CAMERA_X
    };
    let dispatcher = context.dispatcher();
    let wait = OutgoingArguments::push(context, &[1])?;
    wait.call_void(
        context,
        dispatcher,
        ids::WAIT_FOR_NEXT_V_BLANK,
        CallSite::relative(caller, revision, 0x3a),
    )?;
    call_at_void(
        context,
        ids::DISABLE_BLANKING_GRAPHICS,
        &[],
        CallSite::relative(caller, revision, 0x40),
    )?;
    call_at_void(
        context,
        if vertical {
            ids::UPDATE_CAMERA_Y
        } else {
            ids::UPDATE_CAMERA_X
        },
        &[],
        CallSite::relative(caller, revision, 0x44),
    )?;
    wait.release(context)?;
    other.write(context, saved);
    call_at_void(
        context,
        ids::ENABLE_BLANKING_GRAPHICS,
        &[],
        CallSite::relative(caller, revision, 0x56),
    )?;
    context.discard_u8(ram::g_v_blank_graphics_paused(revision).address())?;
    ram::g_v_blank_graphics_paused(revision).write(context, 0);
    context.discard_u8(ram::g_v_blank_audio_input_override(revision).address())?;
    ram::g_v_blank_audio_input_override(revision).write(context, 0);
    Ok(())
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
fn move_pixel(context: &mut dyn Context, camera: i16, direction: Direction) -> Result<Option<u32>> {
    // All four compiled movers save D2/A2/A3 before consulting either player.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x04, 0x0c))?;
    let result = move_pixel_body(context, camera, direction);
    let result = result.map_err(|error| context.protect_timing_failure(error));
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let leave = frame.leave(context);
    let value = result?;
    leave?;
    Ok(value)
}

fn move_pixel_body(
    context: &mut dyn Context,
    mut camera: i16,
    direction: Direction,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let vertical = matches!(direction, Direction::Up | Direction::Down);
    let (primary, secondary) = if vertical {
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
    context.set_address_register(2, primary.address());
    context.set_address_register(3, secondary.address());
    if ram::g_player_lives(revision).at(0).read(context) < 0 {
        camera = 0;
    }
    let selected = if camera == 0 { 0 } else { 1 };
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
    let d2 = context.data_register(2);
    context.set_data_register(2, (d2 & 0xffff0000) | u32::from(new_cursor as u16));
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
            let (caller, return_offsets) = match direction {
                Direction::Up => (ids::MOVE_CAMERA_UP, [0x46, 0x82]),
                Direction::Down => (ids::MOVE_CAMERA_DOWN, [0x56, 0xa0]),
                Direction::Left => (ids::MOVE_CAMERA_LEFT, [0x46, 0x82]),
                Direction::Right => (ids::MOVE_CAMERA_RIGHT, [0x46, 0x82]),
            };
            call_at_void(
                context,
                if vertical {
                    ids::TRY_UPDATE_CAMERA_Y
                } else {
                    ids::TRY_UPDATE_CAMERA_X
                },
                &[selected as u32],
                CallSite::relative(caller, revision, return_offsets[selected as usize]),
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

#[cfg(test)]
#[path = "movement_tests.rs"]
pub(super) mod tests;
