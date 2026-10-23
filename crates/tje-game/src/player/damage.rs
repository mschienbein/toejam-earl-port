use crate::{Context, Result, call, ids};
use tje_mem::{Address, EarthlingDefinition, EarthlingState, ObjectState, ram, rom};

/// G/src/player/earthling_damage_reactions.c:152 (`Player_Squish`).
pub(super) fn player_squish(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let number = i32::from(arguments[0] as i16);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    let revision = context.revision();
    let player = super::indexing::player_by_word_index(revision, number);
    if (earthling.kind().read(context) != 19
        || i32::from(earthling.state().read(context)) & 0xfc == 4)
        && call(
            context,
            ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
            &[player.address(), number as u32],
        )? != 0
    {
        player.animation_motion_axes().write(context, 3);
        call(context, ids::SET_SPRITE, &[player.address(), 2])?;
        player
            .draw_flags()
            .write(context, player.draw_flags().read(context) | 0x80);
        player.height().write(
            context,
            if ram::g_player_terrain_sinking(revision)
                .at(number)
                .read(context)
                != 0
            {
                -3
            } else {
                0
            },
        );
        if earthling.state().read(context) != 0 {
            earthling.timer().write(context, 30);
        }
        let health = ram::g_player_health(revision).at(number);
        health.write(
            context,
            health.read(context).wrapping_sub(
                rom::earthling_definitions(revision)
                    .at(i32::from(earthling.kind().read(context)))
                    .damage()
                    .read(context),
            ),
        );
        call(
            context,
            ids::PLAY_SOUND_EFFECT,
            &[rom::sfx_squish(revision).address(), 50, 0],
        )?;
    }
    Ok(None)
}

/// G/src/player/earthling_damage_reactions.c:315 (`WhirlWindAbility`).
pub(super) fn whirl_wind_ability(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let number = i32::from(arguments[0] as i16);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    if earthling.state().read(context) != 4 {
        return Ok(None);
    }
    let revision = context.revision();
    let player = super::indexing::player_by_word_index(revision, number);
    if call(
        context,
        ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
        &[player.address(), number as u32],
    )? == 0
        && player.animation_group_mask().read(context) & 0x40000 == 0
        && player.sprite_state().read(context) != 77
    {
        return Ok(None);
    }
    call(context, ids::SET_SPRITE, &[player.address(), 79])?;
    player.animation_motion_axes().write(context, 3);
    player.x().write(context, earthling.x().read(context));
    player.y().write(context, earthling.y().read(context));
    player.height().write(context, 0);
    player.velocity_x().write(context, 0);
    player.velocity_y().write(context, 0);
    player.velocity_height().write(context, 0);
    player.acceleration_x().write(context, 0);
    player.acceleration_y().write(context, 0);
    player.acceleration_height().write(context, 0);
    player
        .draw_flags()
        .write(context, player.draw_flags().read(context) | 0x40);
    ram::g_player_whirlwind_timers(revision)
        .at(number)
        .write(context, 225);
    let object = earthling.object().read(context) as u32;
    call(context, ids::FREE_ENTITY, &[object])?;
    earthling.object().write(context, 0);
    earthling.kind().write(context, -1);
    Ok(None)
}

/// G/src/player/earthling_damage_reactions.c:226 (`EarthlingStartHorizontalAttack`).
pub(super) fn earthling_start_horizontal_attack(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let target = Address::<ObjectState>::new(arguments[1]);
    let delta = target
        .x()
        .read(context)
        .wrapping_sub(earthling.x().read(context));
    if i32::from(delta).abs() > 65 {
        call(
            context,
            ids::SET_EARTHLING_STATE,
            &[earthling.address(), if delta > 0 { 11 } else { 10 }],
        )?;
        ram::g_player_info(context.revision())
            .at(i32::from(earthling.object().read(context)))
            .frame()
            .write(context, -1);
        return Ok(Some(1));
    }
    Ok(Some(0))
}

/// G/src/player/earthling_damage_reactions.c:245 (`CupidAbility`).
pub(super) fn cupid_ability(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let number = i32::from(arguments[0] as i16);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    let revision = context.revision();
    if ram::g_player_interaction_choice_active(revision)
        .at(number)
        .read(context)
        != 0
        || earthling.state().read(context) == 3
    {
        return Ok(None);
    }
    let player = super::indexing::player_by_word_index(revision, number);
    if !(matches!(player.sprite_state().read(context), 0 | 4 | 76 | 77)
        || player.animation_group_mask().read(context) & (0x2040_0000 | 0x1834_c03c) != 0)
    {
        return Ok(None);
    }
    if i32::from(earthling.state().read(context)) & 0xfc != 8
        && player.height().read(context) < 40
        && (i32::from(player.y().read(context)) - i32::from(earthling.y().read(context))).abs() <= 8
    {
        call(
            context,
            ids::EARTHLING_START_HORIZONTAL_ATTACK,
            &[earthling.address(), player.address()],
        )?;
    }
    if player.height().read(context) > 10 && player.height().read(context) < 75 {
        let delta = player
            .x()
            .read(context)
            .wrapping_sub(earthling.x().read(context));
        if i32::from(delta).abs() < 25 {
            call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 3])?;
            let object =
                ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
            object.frame().write(context, -1);
            object.acceleration_height().write(context, -32);
            object.velocity_height().write(context, -512);
            object
                .behavior_flags()
                .write(context, object.behavior_flags().read(context) & !1);
        }
    }
    Ok(None)
}

/// G/src/player/earthling_damage_reactions.c:283 (`EarthlingUpdateBouncingDamageState`).
/// ORIGINAL BUG: when neither loaded map matches, map_number is the entry D1
/// low word (G/build/rev02/asm/c_earthling_damage_reactions.s:651-670).
pub(super) fn earthling_update_bouncing_damage_state(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let revision = context.revision();
    let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    if earthling.object().read(context) == 0 {
        call(
            context,
            ids::EARTHLING_REMOVE_FROM_LEVEL,
            &[earthling.address()],
        )?;
        return Ok(None);
    }
    if object.frame_duration().read(context) == 0 {
        call(context, ids::DAMAGE_EARTHLING, &[earthling.address()])?;
    } else if object.height().read(context) < 3 && object.velocity_height().read(context) < 0 {
        object.acceleration_height().write(context, -64);
        object.velocity_height().write(context, 512);
        object.height().write(context, 0);
        if object.frame().read(context) == 0 {
            object.frame_duration().write(context, 1);
        }
        let map = if object.level().read(context)
            == ram::g_player1_map_level(revision).read(context)
        {
            0
        } else if object.level().read(context) == ram::g_player2_map_level(revision).read(context) {
            1
        } else {
            i32::from(context.data_register(1) as i16)
        };
        let args = [
            map as u32,
            object.x().read(context) as u32,
            object.y().read(context) as u32,
        ];
        if call(context, ids::GET_MAP_TERRAIN_AT_WORLD_POSITION, &args)? as u16 > 0x50 {
            call(context, ids::DAMAGE_EARTHLING, &[earthling.address()])?;
        }
    }
    Ok(None)
}

/// G/src/player/earthling_damage_reactions.c:412 (`EarthlingCheckDecoyAttackRange`).
pub(super) fn earthling_check_decoy_attack_range(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let definition = Address::<EarthlingDefinition>::new(arguments[1]);
    let revision = context.revision();
    if i32::from(earthling.state().read(context)) & 252 == 8 {
        return Ok(None);
    }
    let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    for number in 0..2 {
        let handle = ram::g_player_decoy_entities(revision)
            .at(number)
            .read(context);
        if handle != 0 {
            let decoy = ram::g_player_info(revision).at(i32::from(handle));
            let args = [
                object.address(),
                decoy.address(),
                u32::from(definition.contact_distance_x().read(context)),
                u32::from(definition.contact_distance_y().read(context)),
            ];
            if call(context, ids::ENTITY_IS_WITHIN_RANGE, &args)? != 0
                && call(
                    context,
                    ids::EARTHLING_START_HORIZONTAL_ATTACK,
                    &[earthling.address(), decoy.address()],
                )? != 0
            {
                return Ok(None);
            }
        }
    }
    Ok(None)
}
