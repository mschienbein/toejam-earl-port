use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, FunctionId, Result, call_at, ids};
use tje_mem::{Address, arithmetic, ram};

/// G/src/camera/camera_follow.c:17 (`GetPlayerCameraVelocity`).
/// Animation accumulators/multiplication narrow to words before division; the
/// final two emulated output-pointer stores retain their original alias order.
pub fn get_player_camera_velocity(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = args[0] as i16;
    let dx = Address::<i16>::new(args[1]);
    let dy = Address::<i16>::new(args[2]);
    let screen_x = args[3] as i16;
    let screen_y = args[4] as i16;
    if ram::g_player_lives(revision)
        .at(i32::from(player))
        .read(context)
        < 0
        || ram::g_player_inventory_open(revision)
            .at(i32::from(player))
            .read(context)
            != 0
    {
        dx.write(context, 0);
        dy.write(context, 0);
        return Ok(None);
    }
    let entity = ram::g_player_info(revision).at(if player == 0 { 0 } else { 1 });
    let camera_x = if player == 0 {
        ram::g_camera1_pending_x(revision)
    } else {
        ram::g_camera2_pending_x(revision)
    }
    .read(context);
    let camera_y = if player == 0 {
        ram::g_camera1_pending_y(revision)
    } else {
        ram::g_camera2_pending_y(revision)
    }
    .read(context);
    let mut velocity_x = 0_i16;
    let mut velocity_y = 0_i16;
    if entity.frame_phase().read(context) != 0
        && entity.animation_motion_axes().read(context) & 3 != 0
    {
        let mut frame_count = 0_i16;
        loop {
            let sequence = entity.sequence().read(context);
            let byte = sequence
                .target()
                .offset(i32::from(frame_count))
                .read(context) as u8;
            if byte & 0xf0 == 0xf0 {
                break;
            }
            let frame = entity
                .frames()
                .read(context)
                .target()
                .offset(i32::from(byte & 127))
                .read(context)
                .target();
            velocity_x = velocity_x.wrapping_add(i16::from(frame.motion_x().read(context)));
            velocity_y = velocity_y.wrapping_add(i16::from(frame.motion_y().read(context)));
            frame_count = frame_count.wrapping_add(1);
        }
        velocity_x = velocity_x.wrapping_mul(entity.velocity_x().read(context));
        velocity_y = velocity_y.wrapping_mul(entity.velocity_y().read(context));
        let divisor = i32::from(entity.frame_phase().read(context)) * i32::from(frame_count);
        velocity_x = arithmetic::div_i32(i32::from(velocity_x), divisor)? as i16;
        let divisor = i32::from(entity.frame_phase().read(context)) * i32::from(frame_count);
        velocity_y = arithmetic::div_i32(i32::from(velocity_y), divisor)? as i16;
    }
    if entity.draw_flags().read(context) & 0x18 != 0
        || entity.animation_group_mask().read(context) & 0x100 != 0
    {
        velocity_x = 0;
    } else if entity.animation_motion_axes().read(context) & 1 == 0
        && entity.x().read(context) > 168
        && entity.x().read(context) < 2512
    {
        velocity_x = entity.velocity_x().read(context);
    }
    if entity.draw_flags().read(context) & 0x28 != 0
        || entity.animation_group_mask().read(context) & 0x100 != 0
        || entity.sprite_state().read(context) == 5
        || entity.sprite_state().read(context) == 6
    {
        velocity_y = 0;
    } else if entity.animation_motion_axes().read(context) & 2 == 0 {
        velocity_y = entity.velocity_y().read(context);
        if ram::g_player_fall_states(revision)
            .at(i32::from(player))
            .read(context)
            == 0
        {
            velocity_y = velocity_y.wrapping_sub(entity.velocity_height().read(context) / 2);
        }
    }
    let (left, right, top, bottom) = if ram::g_player_lives(revision)
        .at(i32::from(player) ^ 1)
        .read(context)
        < 0
    {
        (160, 160, 112, 112)
    } else if ram::g_split_screen_enabled(revision).read(context) == 0 {
        (50, 270, 40, 158)
    } else {
        (160, 160, 56, 56)
    };
    if velocity_x == 0 {
        if screen_x < left && camera_x > 8 {
            velocity_x = -256;
        }
        if screen_x > right && camera_x < 2360 {
            velocity_x = 256;
        }
    }
    if velocity_y == 0 {
        if screen_y < top && camera_y > 8 {
            velocity_y = -256;
        }
        if screen_y > bottom && camera_y < 1560 {
            velocity_y = 256;
        }
    }
    dx.write(context, velocity_x);
    dy.write(context, velocity_y);
    Ok(None)
}

/// G/src/camera/camera_follow.c:97 (`UpdateCameraFollowing`).
/// Matching c_camera_follow.s:247-791 uses LINK A6,#-8 and saves D2-D6/A2-A3.
/// Velocity X lives at A6-8/-6 and Y at A6-4/-2. The two five-slot calls retain
/// their outgoing areas until the original LEA 40(SP),SP, never host pointers.
pub fn update_camera_following(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(8, 0x7c, 0x0c))?;
    let result = following_body(context, &frame);
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

fn following_body(context: &mut dyn Context, frame: &LocalFrame) -> Result<()> {
    let revision = context.revision();
    let caller = ids::UPDATE_CAMERA_FOLLOWING.metadata().address(revision);
    let first = ram::g_player_info(revision).at(0);
    let second = ram::g_player_info(revision).at(1);
    let first_x = ram::g_camera1_pending_x(revision);
    let first_y = ram::g_camera1_pending_y(revision);
    let second_x = ram::g_camera2_pending_x(revision);
    let second_y = ram::g_camera2_pending_y(revision);
    context.set_address_register(2, first_x.address());
    context.set_address_register(3, first_y.address());
    let primary_x = first.x().read(context).wrapping_sub(first_x.read(context));
    let mut primary_y = first
        .y()
        .read(context)
        .wrapping_sub(first_y.read(context))
        .wrapping_sub(i16::from(first.height_pixels().read(context) / 2));
    let secondary_x = second
        .x()
        .read(context)
        .wrapping_sub(second_x.read(context));
    let mut secondary_y = second
        .y()
        .read(context)
        .wrapping_sub(second_y.read(context))
        .wrapping_sub(i16::from(second.height_pixels().read(context) / 2));
    if first.height().read(context) > 0
        && first.animation_group_mask().read(context) & 0x04008000 != 0
    {
        if ram::g_split_screen_enabled(revision).read(context) != 0 {
            primary_y = primary_y.wrapping_sub(first.height().read(context) / 2);
        } else if first.y().read(context) < second.y().read(context) {
            primary_y = primary_y.wrapping_sub(first.height().read(context));
        }
    }
    if second.height().read(context) > 0
        && second.animation_group_mask().read(context) & 0x04008000 != 0
    {
        if ram::g_split_screen_enabled(revision).read(context) != 0 {
            secondary_y = secondary_y.wrapping_sub(second.height().read(context) / 2);
        } else if second.y().read(context) < first.y().read(context) {
            secondary_y = secondary_y.wrapping_sub(second.height().read(context));
        }
    }
    let vx = [frame.local::<i16>(-8), frame.local::<i16>(-6)];
    let vy = [frame.local::<i16>(-4), frame.local::<i16>(-2)];
    let dispatcher = context.dispatcher();
    let primary = OutgoingArguments::push(
        context,
        &[
            0,
            vx[0].address(),
            vy[0].address(),
            i32::from(primary_x) as u32,
            i32::from(primary_y) as u32,
        ],
    )?;
    // JSR d16(PC) at 0x29d2/0x29ee, four-byte instructions in both revisions.
    primary.call(
        context,
        dispatcher,
        ids::GET_PLAYER_CAMERA_VELOCITY,
        CallSite::new(caller + 0xe0),
    )?;
    let secondary = OutgoingArguments::push(
        context,
        &[
            1,
            vx[1].address(),
            vy[1].address(),
            i32::from(secondary_x) as u32,
            i32::from(secondary_y) as u32,
        ],
    )?;
    secondary.call(
        context,
        dispatcher,
        ids::GET_PLAYER_CAMERA_VELOCITY,
        CallSite::new(caller + 0xfc),
    )?;
    secondary.release(context)?;
    primary.release(context)?;

    if ram::g_split_screen_enabled(revision).read(context) == 0
        && ram::g_player_inventory_open(revision).at(0).read(context) == 0
        && ram::g_player_inventory_open(revision).at(1).read(context) == 0
        && ram::g_player_lives(revision).at(0).read(context) >= 0
        && ram::g_player_lives(revision).at(1).read(context) >= 0
    {
        let v0 = vx[0].read(context);
        let v1 = vx[1].read(context);
        if v0 != 0 || v1 != 0 {
            let velocity = combined_velocity(v0, v1, primary_x, secondary_x, 50, 270);
            follow_axis(
                context,
                first_x,
                ram::g_camera1_follow_x_fixed(revision),
                velocity,
                0,
                caller,
                &[
                    (ids::MOVE_CAMERA_RIGHT, 0x1fe),
                    (ids::MOVE_CAMERA_LEFT, 0x212),
                ],
            )?;
        }
        let v0 = vy[0].read(context);
        let v1 = vy[1].read(context);
        if v0 != 0 || v1 != 0 {
            let velocity = combined_velocity(v0, v1, primary_y, secondary_y, 40, 158);
            follow_axis(
                context,
                first_y,
                ram::g_camera1_follow_y_fixed(revision),
                velocity,
                0,
                caller,
                &[(ids::MOVE_CAMERA_DOWN, 0x2fa), (ids::MOVE_CAMERA_UP, 0x30e)],
            )?;
        }
        let x = first_x.read(context);
        second_x.write(context, x);
        let y = first_y.read(context);
        second_y.write(context, y);
    } else if ram::g_player_lives(revision).at(0).read(context) >= 0 {
        let velocity = vx[0].read(context);
        if velocity != 0 {
            follow_axis(
                context,
                first_x,
                ram::g_camera1_follow_x_fixed(revision),
                velocity,
                0,
                caller,
                &[
                    (ids::MOVE_CAMERA_RIGHT, 0x368),
                    (ids::MOVE_CAMERA_LEFT, 0x37c),
                ],
            )?;
        }
        let velocity = vy[0].read(context);
        if velocity != 0 {
            follow_axis(
                context,
                first_y,
                ram::g_camera1_follow_y_fixed(revision),
                velocity,
                0,
                caller,
                &[(ids::MOVE_CAMERA_DOWN, 0x3be), (ids::MOVE_CAMERA_UP, 0x3d2)],
            )?;
        }
        let velocity = vx[1].read(context);
        if velocity != 0 {
            follow_axis(
                context,
                second_x,
                ram::g_camera2_follow_x_fixed(revision),
                velocity,
                1,
                caller,
                &[
                    (ids::MOVE_CAMERA_RIGHT, 0x416),
                    (ids::MOVE_CAMERA_LEFT, 0x430),
                ],
            )?;
        }
        let velocity = vy[1].read(context);
        if velocity != 0 {
            follow_axis(
                context,
                second_y,
                ram::g_camera2_follow_y_fixed(revision),
                velocity,
                1,
                caller,
                &[(ids::MOVE_CAMERA_DOWN, 0x47e), (ids::MOVE_CAMERA_UP, 0x498)],
            )?;
        }
    } else {
        let velocity = vx[1].read(context);
        if velocity != 0 {
            follow_axis(
                context,
                first_x,
                ram::g_camera1_follow_x_fixed(revision),
                velocity,
                0,
                caller,
                &[
                    (ids::MOVE_CAMERA_RIGHT, 0x4e6),
                    (ids::MOVE_CAMERA_LEFT, 0x4fa),
                ],
            )?;
        }
        let velocity = vy[1].read(context);
        if velocity != 0 {
            follow_axis(
                context,
                first_y,
                ram::g_camera1_follow_y_fixed(revision),
                velocity,
                0,
                caller,
                &[(ids::MOVE_CAMERA_DOWN, 0x53c), (ids::MOVE_CAMERA_UP, 0x550)],
            )?;
        }
    }
    Ok(())
}

fn combined_velocity(
    first: i16,
    second: i16,
    primary: i16,
    secondary: i16,
    lower: i16,
    upper: i16,
) -> i16 {
    if first < 0 && second < 0 {
        if first < second && secondary < upper {
            first
        } else if primary < upper {
            second
        } else {
            0
        }
    } else if first > 0 && second > 0 {
        if first > second && secondary > lower {
            first
        } else if primary > lower {
            second
        } else {
            0
        }
    } else {
        let mut combined = 0_i16;
        if first < 0 && secondary < upper {
            combined = first;
        } else if second < 0 && primary < upper {
            combined = second;
        }
        if first > 0 && secondary > lower {
            combined = combined.wrapping_add(first);
        } else if second > 0 && primary > lower {
            combined = combined.wrapping_add(second);
        }
        combined
    }
}

fn follow_axis(
    context: &mut dyn Context,
    position: Address<i16>,
    fixed: Address<i32>,
    velocity: i16,
    camera: u32,
    caller: u32,
    sites: &[(FunctionId, u32); 2],
) -> Result<()> {
    let next = fixed.read(context).wrapping_add(i32::from(velocity));
    fixed.write(context, next);
    let target = (fixed.read(context) >> 8) as i16;
    let mut blocked = false;
    while target > position.read(context) {
        // All movement sites are absolute-long JSR (six bytes); offsets above
        // are the source-backed return PCs from the two revision call graphs.
        blocked = call_at(
            context,
            sites[0].0,
            &[camera],
            CallSite::new(caller + sites[0].1),
        )? as i16
            != 0;
        if blocked {
            break;
        }
    }
    while target < position.read(context) {
        blocked = call_at(
            context,
            sites[1].0,
            &[camera],
            CallSite::new(caller + sites[1].1),
        )? as i16
            != 0;
        if blocked {
            break;
        }
    }
    if blocked {
        let reset = i32::from(position.read(context)).wrapping_shl(8);
        fixed.write(context, reset);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::combined_velocity;

    #[test]
    fn shared_velocity_preserves_sign_gates_and_edge_asymmetry() {
        assert_eq!(combined_velocity(-200, -100, 160, 269, 50, 270), -200);
        assert_eq!(combined_velocity(-200, -100, 160, 270, 50, 270), -100);
        assert_eq!(combined_velocity(200, 100, 160, 51, 50, 270), 200);
        assert_eq!(combined_velocity(200, 100, 160, 50, 50, 270), 100);
        assert_eq!(combined_velocity(-300, 200, 160, 160, 50, 270), -100);
        assert_eq!(combined_velocity(300, -200, 160, 160, 50, 270), 100);
        assert_eq!(combined_velocity(-300, 200, 50, 270, 50, 270), 0);
    }
}
