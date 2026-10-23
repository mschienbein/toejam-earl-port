use crate::{Context, Result};
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
