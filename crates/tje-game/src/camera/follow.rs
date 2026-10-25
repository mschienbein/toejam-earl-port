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
    set_word(context, 2, 0);
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
    set_word(context, 3, primary_x);
    set_word(context, 4, primary_y);
    set_word(context, 6, secondary_x);
    set_word(context, 5, secondary_y);
    if first.height().read(context) > 0
        && first.animation_group_mask().read(context) & 0x04008000 != 0
    {
        if ram::g_split_screen_enabled(revision).read(context) != 0 {
            primary_y = primary_y.wrapping_sub(first.height().read(context) / 2);
        } else {
            // MOVE.W partner.y,D1; CMP.W player.y,D1 reads the right C
            // operand first. Do not rely on Rust's left-to-right comparison.
            let partner_y = second.y().read(context);
            let player_y = first.y().read(context);
            if partner_y > player_y {
                primary_y = primary_y.wrapping_sub(first.height().read(context));
            }
        }
    }
    set_word(context, 4, primary_y);
    if second.height().read(context) > 0
        && second.animation_group_mask().read(context) & 0x04008000 != 0
    {
        if ram::g_split_screen_enabled(revision).read(context) != 0 {
            secondary_y = secondary_y.wrapping_sub(second.height().read(context) / 2);
        } else {
            let partner_y = first.y().read(context);
            let player_y = second.y().read(context);
            if partner_y > player_y {
                secondary_y = secondary_y.wrapping_sub(second.height().read(context));
            }
        }
    }
    set_word(context, 5, secondary_y);
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
    // The original TST.B precedes LEA 40(SP),SP.
    let shared = ram::g_split_screen_enabled(revision).read(context) == 0;
    secondary.release(context)?;
    primary.release(context)?;

    if shared
        && ram::g_player_inventory_open(revision).at(0).read(context) == 0
        && ram::g_player_inventory_open(revision).at(1).read(context) == 0
        && ram::g_player_lives(revision).at(0).read(context) >= 0
        && ram::g_player_lives(revision).at(1).read(context) >= 0
    {
        if vx[0].read(context) != 0 || vx[1].read(context) != 0 {
            let velocity = combined_velocity(
                |index| vx[index].read(context),
                primary_x,
                secondary_x,
                50,
                270,
            );
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
        if vy[0].read(context) != 0 || vy[1].read(context) != 0 {
            let velocity = combined_velocity(
                |index| vy[index].read(context),
                primary_y,
                secondary_y,
                40,
                158,
            );
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
            let velocity = vx[0].read(context);
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
            let velocity = vy[0].read(context);
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
            let velocity = vx[1].read(context);
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
            let velocity = vy[1].read(context);
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
            let velocity = vx[1].read(context);
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
            let velocity = vy[1].read(context);
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
    mut read: impl FnMut(usize) -> i16,
    primary: i16,
    secondary: i16,
    lower: i16,
    upper: i16,
) -> i16 {
    if read(0) < 0 && read(1) < 0 {
        // The source MOVE of operand 1 precedes CMP's read of operand 0.
        if read(1) > read(0) && secondary < upper {
            read(0)
        } else if primary < upper {
            read(1)
        } else {
            0
        }
    } else if read(0) > 0 && read(1) > 0 {
        if read(1) < read(0) && secondary > lower {
            read(0)
        } else if primary > lower {
            read(1)
        } else {
            0
        }
    } else {
        let mut combined = 0_i16;
        if read(0) < 0 && secondary < upper {
            combined = read(0);
        } else if read(1) < 0 && primary < upper {
            combined = read(1);
        }
        if read(0) > 0 && secondary > lower {
            combined = combined.wrapping_add(read(0));
        } else if read(1) > 0 && primary > lower {
            combined = combined.wrapping_add(read(1));
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
    set_word(context, 3, target);
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
        set_word(context, 2, i16::from(blocked));
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
        set_word(context, 2, i16::from(blocked));
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

fn set_word(context: &mut dyn Context, register: usize, value: i16) {
    let high = context.data_register(register) & 0xffff0000;
    context.set_data_register(register, high | u32::from(value as u16));
}

#[cfg(test)]
mod tests {
    fn combined_velocity(
        first: i16,
        second: i16,
        primary: i16,
        secondary: i16,
        lower: i16,
        upper: i16,
    ) -> i16 {
        super::combined_velocity(
            |index| [first, second][index],
            primary,
            secondary,
            lower,
            upper,
        )
    }

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

    #[test]
    fn combined_velocity_preserves_short_circuit_and_cmp_operand_reads() {
        for (values, expected_reads, expected) in [
            ([-200, -100], &[0, 1, 1, 0, 0][..], -200),
            ([-100, -200], &[0, 1, 1, 0, 1][..], -200),
            ([100, 200], &[0, 0, 1, 1, 0, 1][..], 200),
            ([-300, 200], &[0, 1, 0, 0, 0, 0, 1, 1][..], -100),
            ([300, -200], &[0, 0, 1, 0, 1, 1, 0, 0][..], 100),
        ] {
            let mut reads = Vec::new();
            let actual = super::combined_velocity(
                |index| {
                    reads.push(index);
                    values[index]
                },
                160,
                160,
                50,
                270,
            );
            assert_eq!(actual, expected);
            assert_eq!(reads, expected_reads);
        }
    }

    #[test]
    fn flight_comparisons_read_partner_y_before_player_y() {
        use super::super::movement::tests::{Access, Machine};
        use tje_mem::{Revision, ram};
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut machine = Machine::new(revision);
            let first = ram::g_player_info(revision).at(0);
            let second = ram::g_player_info(revision).at(1);
            for (entity, y) in [(first, 100), (second, 200)] {
                entity.x().write(&mut machine.memory, 200);
                entity.y().write(&mut machine.memory, y);
                entity.height().write(&mut machine.memory, 16);
                entity.height_pixels().write(&mut machine.memory, 32);
                entity
                    .animation_group_mask()
                    .write(&mut machine.memory, 0x04000000);
            }
            for player in 0..2 {
                ram::g_player_lives(revision)
                    .at(player)
                    .write(&mut machine.memory, 3);
            }
            super::update_camera_following(&mut machine, &[]).unwrap();
            let accesses = machine.main_accesses();
            let expected = [
                Access::Read(first.x().address(), 2),
                Access::Read(ram::g_camera1_pending_x(revision).address(), 2),
                Access::Read(first.y().address(), 2),
                Access::Read(ram::g_camera1_pending_y(revision).address(), 2),
                Access::Read(first.height_pixels().address(), 1),
                Access::Read(second.x().address(), 2),
                Access::Read(ram::g_camera2_pending_x(revision).address(), 2),
                Access::Read(second.y().address(), 2),
                Access::Read(ram::g_camera2_pending_y(revision).address(), 2),
                Access::Read(second.height_pixels().address(), 1),
                Access::Read(first.height().address(), 2),
                Access::Read(first.animation_group_mask().address(), 4),
                Access::Read(ram::g_split_screen_enabled(revision).address(), 1),
                Access::Read(second.y().address(), 2),
                Access::Read(first.y().address(), 2),
                Access::Read(first.height().address(), 2),
                Access::Read(second.height().address(), 2),
                Access::Read(second.animation_group_mask().address(), 4),
                Access::Read(ram::g_split_screen_enabled(revision).address(), 1),
                Access::Read(first.y().address(), 2),
                Access::Read(second.y().address(), 2),
            ];
            assert_eq!(&accesses[..expected.len()], expected);
        }
    }
}
