use crate::{Context, Result, call, ids};
use tje_mem::{Address, ObjectState, ram};

/// G/src/player/entity_motion_terrain.c:22 (`EntityIsNearPosition`).
pub(super) fn entity_is_near_position(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let entity = Address::<ObjectState>::new(arguments[0]);
    let x = i32::from(arguments[1] as i16);
    let y = i32::from(arguments[2] as i16);
    let level = arguments[3] as i8;
    Ok(Some(u32::from(
        entity.level().read(context) == level
            && (i32::from(entity.x().read(context)) - x).abs() < 16
            && (i32::from(entity.y().read(context)) - y).abs() < 5,
    )))
}

/// G/src/player/entity_motion_terrain.c:35 (`EntityIsWithinRange`).
pub(super) fn entity_is_within_range(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let entity = Address::<ObjectState>::new(arguments[0]);
    let other = Address::<ObjectState>::new(arguments[1]);
    let dx = i32::from(arguments[2] as i16);
    let dy = i32::from(arguments[3] as i16);
    Ok(Some(u32::from(
        entity.level().read(context) == other.level().read(context)
            && (i32::from(entity.x().read(context)) - i32::from(other.x().read(context))).abs()
                < dx
            && (i32::from(entity.y().read(context)) - i32::from(other.y().read(context))).abs()
                < dy,
    )))
}

/// G/src/player/entity_motion_terrain.c:196 (`PlayerApplyTerrainDestination`).
/// The caller's 12-byte packet is emulated storage, including writes back into it.
pub(super) fn player_apply_terrain_destination(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = i32::from(arguments[0] as i16);
    let entity = Address::<ObjectState>::new(arguments[1]);
    let packet = arguments[2];
    let x = Address::<i16>::new(packet);
    let y = Address::<i16>::new(packet.wrapping_add(2));
    let tx = Address::<i16>::new(packet.wrapping_add(6));
    let ty = Address::<i16>::new(packet.wrapping_add(8));
    let fx = Address::<u8>::new(packet.wrapping_add(10));
    let fy = Address::<u8>::new(packet.wrapping_add(11));
    let revision = context.revision();
    let terrain = if player == 0 {
        ram::g_player1_nearby_terrain(revision)
    } else {
        ram::g_player2_nearby_terrain(revision)
    }
    .cast::<u8>();
    let mut alternate = false;
    let mut sprite = -1_i16;
    if ram::g_player1_map_level(revision).read(context) == 26
        && (entity.x().read(context) != x.read(context)
            || entity.y().read(context) != y.read(context))
    {
        let saved_x = entity.x().read(context);
        let saved_y = entity.y().read(context);
        entity.x().write(context, x.read(context));
        entity.y().write(context, y.read(context));
        if call(context, ids::PLAYERS_REQUIRE_SEPARATE_VIEWS, &[])? != 0 {
            entity.x().write(context, saved_x);
            entity.y().write(context, saved_y);
            entity
                .draw_flags()
                .write(context, entity.draw_flags().read(context) | 0x30);
            return Ok(None);
        }
        entity.x().write(context, saved_x);
        entity.y().write(context, saved_y);
    }
    let mut property = terrain
        .offset(i32::from(ty.read(context)) * 5 + i32::from(tx.read(context)))
        .read(context)
        & 0xf0;
    if entity.animation_group_mask().read(context) & 0x40800 != 0 {
        if matches!(entity.level().read(context), 1 | 26) {
            if x.read(context) <= 28 || x.read(context) >= 2652 {
                x.write(context, entity.x().read(context));
                entity
                    .draw_flags()
                    .write(context, entity.draw_flags().read(context) | 0x10);
            }
            if y.read(context) <= 48 || y.read(context) >= 1736 {
                y.write(context, entity.y().read(context));
                entity
                    .draw_flags()
                    .write(context, entity.draw_flags().read(context) | 0x20);
            }
        }
        if property != 0x80 {
            sprite = i16::from(entity.sprite_state().read(context) & 3) + 48;
            call(context, ids::SET_SPRITE, &[entity.address(), sprite as u32])?;
            entity.velocity_x().write(context, 0);
            entity.velocity_y().write(context, 0);
            entity.animation_motion_axes().write(context, 0);
        }
        property = 0x50;
    }
    if property <= 0x50 || entity.height().read(context) > 0 {
        entity.x().write(context, x.read(context));
        entity.fraction_x().write(context, fx.read(context));
        entity.y().write(context, y.read(context));
        entity.fraction_y().write(context, fy.read(context));
    } else if property >= 0x80 {
        if property == 0x80 {
            alternate = true;
        }
        if tx.read(context) == 0 {
            x.write(context, x.read(context).wrapping_add(8));
            tx.write(context, tx.read(context).wrapping_add(1));
        } else if tx.read(context) == 4 {
            x.write(context, x.read(context).wrapping_sub(8));
            tx.write(context, tx.read(context).wrapping_sub(1));
        }
        if ty.read(context) == 0 {
            y.write(context, y.read(context).wrapping_add(8));
            ty.write(context, ty.read(context).wrapping_add(1));
        } else if ty.read(context) == 4 {
            y.write(context, y.read(context).wrapping_sub(8));
            ty.write(context, ty.read(context).wrapping_sub(1));
        }
        property = terrain
            .offset(i32::from(ty.read(context)) * 5 + i32::from(tx.read(context)))
            .read(context)
            & 0xf0;
        if property <= 0x50 {
            entity.x().write(context, x.read(context));
            entity.fraction_x().write(context, 0);
            entity.y().write(context, y.read(context));
            entity.fraction_y().write(context, 0);
        }
        if entity.velocity_x().read(context) > 0 {
            sprite = 19;
        }
        if entity.velocity_x().read(context) < 0 {
            sprite = 18;
        }
        if entity.velocity_y().read(context) > 0
            && (terrain.offset(17).read(context) >= 0x80
                || terrain.offset(22).read(context) >= 0x80)
        {
            sprite = 17;
        }
        if entity.velocity_y().read(context) < 0
            && (terrain.offset(2).read(context) >= 0x80 || terrain.offset(7).read(context) >= 0x80)
        {
            sprite = 16;
        }
        if sprite != -1 {
            let powerup = ram::g_player_powerups(revision).at(player);
            if alternate || powerup.read(context) == 1 {
                sprite += 12;
                if powerup.read(context) != 1 {
                    if sprite & 2 != 0 {
                        entity.velocity_y().write(context, 0);
                    } else {
                        entity.velocity_x().write(context, 0);
                    }
                }
            }
            call(context, ids::SET_SPRITE, &[entity.address(), sprite as u32])?;
            entity
                .draw_flags()
                .write(context, entity.draw_flags().read(context) | 0x30);
            ram::g_player_jump_counters(revision)
                .at(player)
                .write(context, 0);
        }
    }
    Ok(None)
}

/// G/src/player/entity_motion_terrain.c:315 (`Player1ProcessTerrainMotion`).
pub(super) fn player1_process_terrain_motion(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    call(
        context,
        ids::PLAYER_PROCESS_TERRAIN_MOTION,
        &[0, arguments[0]],
    )?;
    Ok(None)
}

/// G/src/player/entity_motion_terrain.c:322 (`Player2ProcessTerrainMotion`).
pub(super) fn player2_process_terrain_motion(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    call(
        context,
        ids::PLAYER_PROCESS_TERRAIN_MOTION,
        &[1, arguments[0]],
    )?;
    Ok(None)
}
