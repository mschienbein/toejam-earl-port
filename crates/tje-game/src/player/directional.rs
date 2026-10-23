use crate::{Context, Result, call, ids};
use tje_mem::{Address, ObjectState, ram};

/// G/src/player/sprite_checks_and_movement.c:103 (`PlayerChooseDirectionalMovement`).
pub(super) fn player_choose_directional_movement(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let number = i32::from(arguments[1] as i16);
    let mut input = arguments[2] as i16;
    let land = arguments[3] as u16;
    let revision = context.revision();
    let mut sprite = -1_i16;
    let mut speed_x = 320_i16;
    let mut speed_y = 240_i16;
    if land == 0x30 {
        speed_x = 400;
        speed_y = 300;
        if ram::g_player_terrain_sinking(revision)
            .at(number)
            .read(context)
            != 0
        {
            player.height().write(context, 0);
            player
                .draw_flags()
                .write(context, player.draw_flags().read(context) & !0x40);
        }
    }
    let mut vx = player.height().read(context);
    if ram::g_player_terrain_sinking(revision)
        .at(number)
        .read(context)
        != 0
        && vx < 0
    {
        if vx < -16 {
            vx = -16;
        }
        speed_x = speed_x.wrapping_add(vx.wrapping_mul(10));
        speed_y = speed_y.wrapping_add(vx.wrapping_mul(8));
    }
    let gate = ram::g_player_elevator_approach_input_gate(revision).at(number);
    if gate.read(context) != 0 {
        if input & 1 != 0 {
            input = 1;
        } else {
            gate.write(context, 0);
        }
    }
    let mut vy;
    if player.animation_group_mask().read(context) & 0x8000 == 0 {
        vx = player.velocity_x().read(context);
        vy = player.velocity_y().read(context);
    } else {
        vx = player.acceleration_x().read(context);
        vy = player.acceleration_y().read(context);
    }
    if player.animation_group_mask().read(context) & 0x800_0000 != 0 {
        speed_x = speed_x.wrapping_mul(3);
        speed_y = speed_y.wrapping_mul(3);
    }
    let current = player.sprite_state().read(context);
    let mut mode = i16::from(player.animation_motion_axes().read(context));
    if player.animation_group_mask().read(context) & 0x80 != 0 {
        if (current == 28 && input & 1 != 0)
            || (current == 29 && input & 2 != 0)
            || (current == 30 && input & 4 != 0)
            || (current == 31 && input & 8 != 0)
        {
            return Ok(None);
        }
        sprite = 0;
    } else if input & 0xf == 0 && (vx != 0 || vy != 0) {
        sprite = 0;
        vx = 0;
        vy = 0;
        mode = 0;
    } else if input & 0xc != 0 {
        if (input & 4 != 0 && vx >= 0) || (input & 8 != 0 && vx <= 0) {
            sprite = if input & 4 != 0 { 10 } else { 11 };
            mode = 3;
        }
        if input & 3 != 0 {
            vy = if input & 1 != 0 { -speed_y } else { speed_y };
            vx = if input & 4 != 0 { -speed_y } else { speed_y };
        } else {
            vy = 0;
            vx = if input & 4 != 0 { -speed_x } else { speed_x };
        }
    } else if input & 3 != 0 {
        if vx != 0 || (input & 1 != 0 && vy >= 0) || (input & 2 != 0 && vy <= 0) {
            sprite = if input & 1 != 0 { 8 } else { 9 };
            mode = 2;
            vx = 0;
        }
        vy = if input & 1 != 0 { -speed_x } else { speed_x };
    }
    call(
        context,
        ids::PLAYER_APPLY_DIRECTIONAL_MOVEMENT_ANIMATION,
        &[
            player.address(),
            number as u32,
            input as u32,
            sprite as u32,
            vx as u32,
            vy as u32,
            mode as u32,
        ],
    )?;
    Ok(None)
}

/// G/src/player/sprite_checks_and_movement.c:14 (`SpriteChecks`).
pub(super) fn sprite_checks(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let number = i32::from(arguments[0] as i16);
    let mut input = arguments[1] as i16;
    let revision = context.revision();
    let (land, player) = if number == 0 {
        (
            u16::from(ram::g_player1_terrain_underfoot(revision).read(context)),
            ram::g_player_info(revision).at(0),
        )
    } else {
        (
            u16::from(ram::g_player2_terrain_underfoot(revision).read(context)),
            ram::g_player_info(revision).at(1),
        )
    };
    if player.draw_flags().read(context) & 8 != 0 {
        return Ok(None);
    }
    if player.sprite_state().read(context) == 89 {
        let ticks = ram::g_player_hula_dance_ticks(revision).at(number);
        if ticks.read(context) > 0 {
            ticks.write(context, ticks.read(context).wrapping_sub(1));
        } else {
            call(context, ids::SET_SPRITE, &[player.address(), 0])?;
            player.velocity_x().write(context, 0);
            player.velocity_y().write(context, 0);
        }
        return Ok(None);
    }
    if player.animation_group_mask().read(context) & 0x77d3_3760 != 0
        || matches!(player.sprite_state().read(context), 79 | 78 | 5 | 6 | 2)
        || (player.animation_group_mask().read(context) & 0x8000 != 0
            && player.height().read(context) < 4)
    {
        return Ok(None);
    }
    if ram::g_player_control_remap_entities(revision)
        .at(number)
        .read(context)
        != 0
    {
        input = call(
            context,
            ids::PLAYER_REMAP_DIRECTIONAL_INPUT,
            &[number as u32],
        )? as i16;
    }
    if player.x().read(context) <= 8 {
        input &= !4;
    }
    if i32::from(player.width().read(context)) + i32::from(player.x().read(context)) >= 2672 {
        input &= !8;
    }
    if i32::from(player.y().read(context)) - i32::from(player.height_pixels().read(context)) <= 8 {
        input &= !1;
    }
    if player.y().read(context) >= 1776 {
        input &= !2;
    }
    let elevator = ram::g_player_elevator_flags(revision)
        .at(number)
        .read(context);
    if elevator != 0 {
        if call(context, ids::PROCESS_ELEVATOR, &[elevator as u32])? == 0 {
            input = 0;
        } else {
            input &= !0xc;
        }
    }
    if (i32::from(
        ram::g_doorway_player_contact_flags(revision)
            .at(0)
            .read(context),
    ) == ((number + 1) | 4)
        || i32::from(ram::g_second_doorway_player_contact_flags(revision).read(context))
            == ((number + 1) | 4))
        && input & 1 != 0
    {
        input &= !0xc;
    }
    let sprite = player.sprite_state().read(context);
    if player.animation_group_mask().read(context) & 0x10 != 0 {
        if (sprite == 16 && input & 1 != 0)
            || (sprite == 17 && input & 2 != 0)
            || (sprite == 18 && input & 4 != 0)
            || (sprite == 19 && input & 8 != 0)
        {
            return Ok(None);
        }
        player.velocity_x().write(context, 0);
        player.velocity_y().write(context, 0);
    }
    call(
        context,
        ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT,
        &[
            player.address(),
            number as u32,
            input as u32,
            u32::from(land),
        ],
    )?;
    Ok(None)
}
