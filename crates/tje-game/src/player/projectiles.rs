use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, ObjectState, ram, rom};

/// G/src/player/projectiles.c:24 (`PlayerLaunchPresentProjectile`).
pub(super) fn player_launch_present_projectile(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let number = i32::from(arguments[1] as i16);
    let powerup = arguments[2] as i16;
    let revision = context.revision();
    let slots = ram::g_shared_projectile_entities(revision);
    let mut slot = 0;
    while slot < 4 && slots.at(slot).read(context) != 0 {
        slot += 1;
    }
    if slot == 4 {
        return Ok(None);
    }
    let allocated = call(context, ids::ALLOCATE_ENTITY, &[])? as i8;
    slots.at(slot).write(context, allocated);
    if allocated != 0 {
        ram::g_shared_projectile_owners(revision)
            .at(slot)
            .write(context, number as i8);
        let projectile = ram::g_player_info(revision).at(i32::from(slots.at(slot).read(context)));
        let mut direction = i16::from(player.sprite_state().read(context) & 3);
        let placement = rom::projectile_placement_triples(revision)
            .at(i32::from(powerup.wrapping_sub(3)))
            .at(number)
            .at(i32::from(direction));
        projectile.x().write(
            context,
            player
                .x()
                .read(context)
                .wrapping_add(i16::from(placement.at(0).read(context))),
        );
        projectile.y().write(
            context,
            player
                .y()
                .read(context)
                .wrapping_add(i16::from(placement.at(1).read(context))),
        );
        projectile.height().write(
            context,
            player
                .height()
                .read(context)
                .wrapping_add(i16::from(placement.at(2).read(context))),
        );
        direction = i16::from(
            ram::g_player_processed_input(revision)
                .at(number)
                .read(context),
        );
        direction &= 0xf;
        let facing = player.sprite_state().read(context) & 3;
        if facing == 0 && direction & 1 == 0 {
            direction = 1;
        } else if facing == 1 && direction & 2 == 0 {
            direction = 2;
        } else if facing == 2 && direction & 4 == 0 {
            direction = 4;
        } else if facing == 3 && direction & 8 == 0 {
            direction = 8;
        }
        let (speed_x, speed_y) = if powerup == 3 {
            projectile.velocity_height().write(context, 160);
            projectile.acceleration_height().write(context, -32);
            (896, 512)
        } else {
            projectile.velocity_height().write(context, 64);
            projectile.acceleration_height().write(context, -8);
            call_void(context, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[11])?;
            (1152, 768)
        };
        if direction & 1 != 0 {
            projectile.velocity_y().write(context, -speed_y);
        } else if direction & 2 != 0 {
            projectile.velocity_y().write(context, speed_y);
        }
        if direction & 4 != 0 {
            projectile.velocity_x().write(context, -speed_x);
        } else if direction & 8 != 0 {
            projectile.velocity_x().write(context, speed_x);
        }
        if projectile.velocity_x().read(context) != 0 && projectile.velocity_y().read(context) != 0
        {
            projectile.velocity_x().write(
                context,
                ((i32::from(projectile.velocity_x().read(context)) >> 2) * 3) as i16,
            );
            projectile.velocity_y().write(
                context,
                ((i32::from(projectile.velocity_y().read(context)) >> 2) * 3) as i16,
            );
        }
        projectile
            .level()
            .write(context, player.level().read(context));
        projectile.width().write(context, 9);
        projectile.height_pixels().write(context, 7);
        projectile.sprite_count().write(context, 1);
        projectile.tile_count().write(context, 16);
        projectile.tile().write(context, 0x8000);
        let animation = rom::tomato_projectile_animations(revision).at(0);
        projectile
            .sequence()
            .write(context, animation.sequence().read(context));
        projectile
            .frames()
            .write(context, animation.frames().read(context));
        projectile.frame_phase().write(context, 0);
        projectile
            .draw_flags()
            .write(context, projectile.draw_flags().read(context) | 0x40);
        projectile.sprite_state().write(context, -1);
    }
    Ok(None)
}

/// G/src/player/projectiles.c:105 (`PlayerUpdatePresentShooting`).
pub(super) fn player_update_present_shooting(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let revision = context.revision();
    let number = i32::from(player.entity_index().read(context));
    if player.animation_group_mask().read(context) & 0x10_0000 != 0 {
        if player.frame().read(context) == 1 {
            call_void(
                context,
                ids::PLAYER_LAUNCH_PRESENT_PROJECTILE,
                &[player.address(), number as u32, 3],
            )?;
            player
                .frame()
                .write(context, player.frame().read(context).wrapping_add(1));
        } else if player.frame_duration().read(context) == 0 {
            let sprite = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])?;
            call_void(context, ids::SET_SPRITE, &[player.address(), sprite])?;
        }
    } else if player.animation_group_mask().read(context) & 0x20_0000 != 0 {
        if player.frame().read(context) == 2 {
            call_void(
                context,
                ids::PLAYER_LAUNCH_PRESENT_PROJECTILE,
                &[player.address(), number as u32, 4],
            )?;
            player
                .frame()
                .write(context, player.frame().read(context).wrapping_add(1));
        } else if player.frame_duration().read(context) == 0 {
            let sprite = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])?;
            call_void(context, ids::SET_SPRITE, &[player.address(), sprite])?;
        }
    }
    if ram::g_player_elevator_flags(revision)
        .at(number)
        .read(context)
        == 0
        && player.animation_group_mask().read(context) & 4 != 0
        && ram::g_player_processed_input(revision)
            .at(number)
            .read(context)
            & 0x40
            != 0
        && ram::g_player_processed_input(revision)
            .at(number)
            .read(context)
            & 0xf
            != 0
    {
        let direction = player.sprite_state().read(context) & 3;
        let mut sprite = -1;
        if ram::g_player_powerups(revision).at(number).read(context) == 3 {
            sprite = direction + 80;
        }
        if ram::g_player_powerups(revision).at(number).read(context) == 4 {
            sprite = direction + 84;
        }
        if sprite != -1 {
            call_void(context, ids::SET_SPRITE, &[player.address(), sprite as u32])?;
        }
    }
    Ok(None)
}
