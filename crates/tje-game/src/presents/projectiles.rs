use crate::{Context, Result, call, ids};
use tje_mem::{Address, EarthlingState, ObjectState, ram, rom};

/// G/src/presents/projectile_effects.c:338 (`EntityFindProjectileTargetHit`).
/// Atomic: four RAM-only target tests; signed short result is extended in D0.
pub fn entity_find_projectile_target_hit(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let projectile = Address::<ObjectState>::new(arguments[0]);
    let objects = ram::g_player_info(revision);
    let mut target = objects.at(0);
    for pass in 0..2 {
        for index in 0..2 {
            if pass != 0 {
                let decoy = ram::g_player_decoy_entities(revision)
                    .at(index)
                    .read(context);
                if decoy == 0 {
                    target = target.offset(1);
                    continue;
                }
                target = objects.at(i32::from(decoy));
            }
            if target.draw_flags().read(context) & 1 != 0
                && target.behavior_flags().read(context) & 1 != 0
                && projectile.level().read(context) == target.level().read(context)
            {
                let reach = (u32::from(projectile.width().read(context)) / 2
                    + u32::from(target.width().read(context)) / 2)
                    .wrapping_sub(5) as i16;
                if i32::from(reach)
                    > (i32::from(projectile.x().read(context))
                        - i32::from(target.x().read(context)))
                    .wrapping_abs()
                    && (i32::from(projectile.y().read(context))
                        - i32::from(target.y().read(context)))
                    .wrapping_abs()
                        < i32::from(reach) * 3 / 4
                    && (i32::from(target.height().read(context)) as u32)
                        .wrapping_add(u32::from(target.height_pixels().read(context)))
                        .wrapping_sub(10) as i32
                        > i32::from(projectile.height().read(context))
                {
                    let hit = if pass == 0 {
                        index + 1
                    } else {
                        i32::from(
                            ram::g_player_decoy_entities(revision)
                                .at(index)
                                .read(context),
                        ) + 1
                    };
                    return Ok(Some((hit as i16 as i32) as u32));
                }
            }
            target = target.offset(1);
        }
    }
    Ok(Some(0))
}

/// G/src/presents/projectile_effects.c:221 (`SetTomatoRainTomatoes`).
/// Atomic: RNG and two byte stores.
pub fn set_tomato_rain_tomatoes(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let index = arguments[0] as i32;
    let revision = context.revision();
    let count = (call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 31 + 50;
    ram::g_player_tomato_rain_ticks(revision)
        .at(index)
        .write(context, count as i8);
    ram::g_player_tomato_rain_spawn_delay(revision)
        .at(index)
        .write(context, 0);
    Ok(None)
}

/// G/src/presents/projectile_effects.c:230 (`CheckTomatoRainTimer`).
/// Atomic: RAM pool allocation, RNG and ROM animation reads.
pub fn check_tomato_rain_timer(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let index = arguments[0] as i32;
    let revision = context.revision();
    let ticks = ram::g_player_tomato_rain_ticks(revision).at(index);
    let delay = ram::g_player_tomato_rain_spawn_delay(revision).at(index);
    if ticks.read(context) == 0 {
        return Ok(None);
    }
    // SUBQ.B followed directly by BLE consumes its overflow flag. At -128 the
    // stored byte wraps to 127, but the signed branch still takes the spawn path.
    let remaining = super::predecrement_i8(context, delay);
    if remaining > 0 {
        return Ok(None);
    }
    let next = (call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 5 + 4;
    delay.write(context, next as i8);
    let handles = ram::g_shared_projectile_entities(revision);
    let Some(slot) = (4..12).find(|&slot| handles.at(slot).read(context) == 0) else {
        return Ok(None);
    };
    let number = call(context, ids::ALLOCATE_ENTITY, &[])? as i8;
    handles.at(slot).write(context, number);
    if number != 0 {
        ram::g_shared_projectile_owners(revision)
            .at(slot)
            .write(context, -1);
        let player = ram::g_player_info(revision).at(index);
        let projectile = ram::g_player_info(revision).at(i32::from(number));
        let x = (call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 201;
        projectile.x().write(
            context,
            (u32::from(x)
                .wrapping_sub(100)
                .wrapping_add(i32::from(player.x().read(context)) as u32)) as i16,
        );
        let y = (call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 101;
        projectile.y().write(
            context,
            (u32::from(y)
                .wrapping_sub(50)
                .wrapping_add(i32::from(player.y().read(context)) as u32)) as i16,
        );
        projectile.height().write(context, 224);
        projectile.velocity_height().write(context, -384);
        projectile.acceleration_height().write(context, -18);
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
        ticks.write(context, ticks.read(context).wrapping_sub(1));
    }
    Ok(None)
}

/// G/src/presents/projectile_effects.c:48 (`EarthlingLaunchAimedProjectile`).
/// Atomic: RAM allocation/kinematics, ROM vectors and a queued PCM request.
pub fn earthling_launch_aimed_projectile(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let target = Address::<ObjectState>::new(arguments[0]);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    let handles = ram::g_shared_projectile_entities(revision);
    let Some(slot) = (4..12).find(|&slot| handles.at(slot).read(context) == 0) else {
        return Ok(None);
    };
    let number = call(context, ids::ALLOCATE_ENTITY, &[])? as i8;
    handles.at(slot).write(context, number);
    if number == 0 {
        return Ok(None);
    }
    let mut dx = target
        .x()
        .read(context)
        .wrapping_sub(earthling.x().read(context));
    let mut dy = target
        .y()
        .read(context)
        .wrapping_sub(earthling.y().read(context));
    dx = dx.wrapping_add(target.velocity_x().read(context) >> 3);
    dy = dy.wrapping_add(target.velocity_y().read(context) >> 4);
    let (mut sx, mut sy) = if dx == 0 {
        (0_i16, 800_i16)
    } else {
        let ratio = ((i32::from(dy) * 100) / i32::from(dx)).wrapping_abs() as i16;
        let mut direction = 0;
        while direction < 8
            && ratio
                >= rom::earthling_thresholds(revision)
                    .at(direction)
                    .read(context)
        {
            direction += 1;
        }
        let pair = rom::earthling_vector_pairs(revision).at(direction);
        (pair.at(0).read(context), pair.at(1).read(context))
    };
    if dx < 0 {
        sx = sx.wrapping_neg();
    }
    if dy < 0 {
        sy = sy.wrapping_neg();
    }
    ram::g_shared_projectile_owners(revision)
        .at(slot)
        .write(context, earthling.object().read(context));
    let projectile = ram::g_player_info(revision).at(i32::from(number));
    projectile.x().write(context, earthling.x().read(context));
    projectile.y().write(context, earthling.y().read(context));
    if earthling.state().read(context) == 8 {
        projectile.height().write(context, 10);
    } else if earthling.state().read(context) == 9 {
        projectile.height().write(context, 15);
    } else {
        projectile.height().write(context, 27);
        let delta = if earthling.state().read(context) == 10 {
            -10
        } else {
            10
        };
        projectile
            .x()
            .write(context, projectile.x().read(context).wrapping_add(delta));
    }
    let random = call(context, ids::GET_RANDOM_NUMBER, &[])? as u16;
    projectile.velocity_x().write(
        context,
        (u32::from(random % 101)
            .wrapping_sub(50)
            .wrapping_add(i32::from(sx) as u32)) as i16,
    );
    let random = call(context, ids::GET_RANDOM_NUMBER, &[])? as u16;
    projectile.velocity_y().write(
        context,
        (u32::from(random % 61)
            .wrapping_sub(30)
            .wrapping_add(i32::from(sy) as u32)) as i16,
    );
    projectile.velocity_height().write(context, 768);
    dx = i32::from(dx).wrapping_abs() as i16;
    dy = i32::from(dy).wrapping_abs() as i16;
    if dx > dy {
        dx = dx.wrapping_add(dy >> 1);
    } else {
        dx >>= 1;
        dx = dx.wrapping_add(dy);
    }
    dx = (((i32::from(dx) - 40) >> 3) - 64) as i16;
    dx = dx.clamp(-64, -16);
    projectile.acceleration_height().write(context, dx);
    projectile
        .level()
        .write(context, earthling.level().read(context));
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
    if ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0 {
        call(
            context,
            ids::PLAY_SOUND_EFFECT,
            &[rom::sfx_chicken_cannon_fire(revision).address(), 40, 0],
        )?;
    }
    Ok(None)
}

/// G/src/presents/projectile_effects.c:289 (`EarthlingLaunchControlRemapProjectile`).
/// Atomic: a four-slot RAM scan, allocation, animation reads and FM request.
pub fn earthling_launch_control_remap_projectile(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let source = Address::<ObjectState>::new(arguments[1]);
    let handles = ram::g_control_remap_projectile_entities(revision);
    let Some(slot) = (0..4).find(|&slot| handles.at(slot).read(context) == 0) else {
        return Ok(None);
    };
    let number = call(context, ids::ALLOCATE_ENTITY, &[])? as i8;
    handles.at(slot).write(context, number);
    if number == 0 {
        return Ok(None);
    }
    let projectile = ram::g_player_info(revision).at(i32::from(number));
    projectile.x().write(context, source.x().read(context));
    projectile
        .y()
        .write(context, source.y().read(context).wrapping_sub(5));
    projectile
        .height()
        .write(context, source.height().read(context));
    let animation = if earthling.state().read(context) == 10 {
        projectile
            .x()
            .write(context, projectile.x().read(context).wrapping_sub(22));
        projectile.velocity_x().write(context, -896);
        rom::control_remap_projectile_left_animation(revision)
    } else {
        projectile
            .x()
            .write(context, projectile.x().read(context).wrapping_add(22));
        projectile.velocity_x().write(context, 896);
        rom::control_remap_projectile_right_animation(revision)
    };
    projectile.acceleration_height().write(context, -32);
    projectile
        .level()
        .write(context, source.level().read(context));
    projectile.width().write(context, 16);
    projectile.height_pixels().write(context, 8);
    projectile.sprite_count().write(context, 1);
    projectile.tile_count().write(context, 2);
    projectile.tile().write(context, 0x8000);
    projectile
        .sequence()
        .write(context, animation.sequence().read(context));
    projectile
        .frames()
        .write(context, animation.frames().read(context));
    projectile.frame_phase().write(context, 9);
    projectile.frame().write(context, -1);
    projectile.frame_duration().write(context, 1);
    projectile
        .draw_flags()
        .write(context, projectile.draw_flags().read(context) | 0x40);
    projectile.sprite_state().write(context, -1);
    call(context, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[4])?;
    Ok(None)
}

/// G/src/presents/projectile_effects.c:537 (`PresentCreateTemporaryTerrainEffect`).
/// Atomic: storage-only entity allocation, ROM animation and FM request.
pub fn present_create_temporary_terrain_effect(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = Address::<ObjectState>::new(arguments[0]);
    let index = i32::from(arguments[1] as i16);
    let number = call(context, ids::ALLOCATE_ENTITY, &[])? as i8;
    if number != 0 {
        ram::g_player_one_shot_terrain_effect_entities(revision)
            .at(index)
            .write(context, number);
        let effect = ram::g_player_info(revision).at(i32::from(number));
        effect.x().write(context, player.x().read(context));
        effect.y().write(context, player.y().read(context));
        effect.level().write(context, player.level().read(context));
        effect
            .tile()
            .write(context, effect.tile().read(context) | 0x8000);
        effect.width().write(context, 24);
        effect.height_pixels().write(context, 20);
        effect.sprite_count().write(context, 1);
        effect.tile_count().write(context, 16);
        let animation = rom::temporary_terrain_effect_animation(revision);
        effect
            .sequence()
            .write(context, animation.sequence().read(context));
        effect
            .frames()
            .write(context, animation.frames().read(context));
        effect
            .frame_phase()
            .write(context, animation.period().read(context));
        effect.frame().write(context, -1);
        effect.frame_duration().write(context, 1);
        effect
            .behavior_flags()
            .write(context, effect.behavior_flags().read(context) | 1);
        call(context, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[13])?;
    }
    Ok(None)
}

/// G/src/presents/projectile_effects.c:274 (`PresentReleaseFirstSharedTomato`).
/// Atomic: at most eight handles and one storage-only entity release.
pub fn present_release_first_shared_tomato(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let handles = ram::g_shared_projectile_entities(context.revision());
    for slot in 4..12 {
        let number = handles.at(slot).read(context);
        if number != 0 {
            call(context, ids::FREE_ENTITY, &[number as u32])?;
            handles.at(slot).write(context, 0);
            return Ok(Some(1));
        }
    }
    Ok(Some(0))
}

/// G/src/presents/projectile_effects.c:417 (`PlayerUpdateControlRemapEffect`).
/// Atomic: timer/entity RAM, RNG and PSG request; no animation polling loop.
pub fn player_update_control_remap_effect(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = Address::<ObjectState>::new(arguments[0]);
    let index = i32::from(player.entity_index().read(context));
    let handle = ram::g_player_control_remap_entities(revision).at(index);
    if handle.read(context) == 0 {
        return Ok(None);
    }
    let effect = ram::g_player_info(revision).at(i32::from(handle.read(context)));
    effect.level().write(context, player.level().read(context));
    let timer = ram::g_player_control_remap_timers(revision).at(index);
    let remaining = super::predecrement_i16(context, timer);
    if remaining <= 0
        || ram::g_player_lives(revision).at(index).read(context) < 0
        || matches!(player.sprite_state().read(context), 5 | 6)
    {
        let number = handle.read(context);
        call(context, ids::FREE_ENTITY, &[number as u32])?;
        handle.write(context, 0);
    } else if ram::g_player_elevator_flags(revision)
        .at(index)
        .read(context)
        == 0
        && effect.frame_duration().read(context) & 2 != 0
        && (call(context, ids::GET_RANDOM_NUMBER, &[])? as i16) & 1 != 0
    {
        call(context, ids::PLAY_SOUND, &[2, 20])?;
    }
    Ok(None)
}

/// G/src/presents/projectile_effects.c:515 (`PresentRetireTerrainEffect`).
/// Atomic: release previous object and replace its RAM animation.
/// The original intentionally still addresses object zero when the active handle is zero.
pub fn present_retire_terrain_effect(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let retired = ram::g_player_one_shot_terrain_effect_entities(revision).at(index);
    let number = retired.read(context);
    if number != 0 {
        call(context, ids::FREE_ENTITY, &[number as u32])?;
    }
    let active = ram::g_player_active_terrain_effect_entities(revision).at(index);
    let number = active.read(context);
    retired.write(context, number);
    active.write(context, 0);
    let effect = ram::g_player_info(revision).at(i32::from(number));
    let animation = rom::retiring_terrain_effect_animation(revision);
    effect
        .sequence()
        .write(context, animation.sequence().read(context));
    effect
        .frames()
        .write(context, animation.frames().read(context));
    effect
        .frame_phase()
        .write(context, animation.period().read(context));
    effect.frame().write(context, -1);
    effect.frame_duration().write(context, 1);
    Ok(None)
}

/// G/src/presents/projectile_effects.c:615 (`EarthlingUpdateControlRemapAttack`).
/// Atomic: one frame transition, projectile launch or RAM state selection.
pub fn earthling_update_control_remap_attack(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let object = Address::<ObjectState>::new(arguments[1]);
    if object.frame().read(context) == 2 {
        object
            .frame()
            .write(context, object.frame().read(context).wrapping_add(1));
        call(
            context,
            ids::EARTHLING_LAUNCH_CONTROL_REMAP_PROJECTILE,
            arguments,
        )?;
    } else if object.frame_duration().read(context) == 0 {
        let next = if earthling.state().read(context) == 10 {
            0
        } else {
            1
        };
        call(
            context,
            ids::SET_EARTHLING_STATE,
            &[earthling.address(), next],
        )?;
    }
    Ok(None)
}

/// G/src/presents/projectile_effects.c:634 (`EarthlingUpdateAimedProjectileAttack`).
/// Atomic: bounded player/decoy scan, RAM frame transition and queued sound.
pub fn earthling_update_aimed_projectile_attack(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let object = Address::<ObjectState>::new(arguments[1]);
    if object.frame().read(context) < 16 && object.frame().read(context) & 3 == 2 {
        object
            .frame()
            .write(context, object.frame().read(context).wrapping_add(1));
        let mut index = (call(context, ids::GET_RANDOM_NUMBER, &[])? & 1) as i32;
        for _ in 0..2 {
            let decoy = i32::from(
                ram::g_player_decoy_entities(revision)
                    .at(index)
                    .read(context),
            );
            let target = if decoy != 0
                && ram::g_player_info(revision)
                    .at(decoy)
                    .draw_flags()
                    .read(context)
                    & 6
                    != 0
            {
                ram::g_player_info(revision).at(decoy)
            } else {
                ram::g_player_info(revision).at(index)
            };
            if ram::g_player_lives(revision).at(index).read(context) >= 0 {
                let (distance, minimum, maximum) = match earthling.state().read(context) {
                    8 => (
                        earthling
                            .y()
                            .read(context)
                            .wrapping_sub(target.y().read(context)),
                        40,
                        200,
                    ),
                    9 => (
                        target
                            .y()
                            .read(context)
                            .wrapping_sub(earthling.y().read(context)),
                        40,
                        200,
                    ),
                    10 => (
                        earthling
                            .x()
                            .read(context)
                            .wrapping_sub(target.x().read(context)),
                        50,
                        320,
                    ),
                    _ => (
                        target
                            .x()
                            .read(context)
                            .wrapping_sub(earthling.x().read(context)),
                        50,
                        320,
                    ),
                };
                if distance > minimum && distance < maximum {
                    call(
                        context,
                        ids::EARTHLING_LAUNCH_AIMED_PROJECTILE,
                        &[target.address(), earthling.address()],
                    )?;
                    break;
                }
            }
            index ^= 1;
        }
    } else if object.frame_duration().read(context) == 0 {
        call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 0])?;
        call(
            context,
            ids::PLAY_SOUND_EFFECT,
            &[rom::sfx_chicken_cluck(revision).address(), 50, 0],
        )?;
    }
    Ok(None)
}

/// G/src/presents/projectile_effects.c:370 (`PlayerApplyControlRemapEffect`).
/// Atomic: RAM allocation/attachment, ROM animation and RNG mapping selection.
pub fn player_apply_control_remap_effect(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let handle = ram::g_player_control_remap_entities(revision).at(index);
    if handle.read(context) == 0 {
        let number = call(context, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[0])? as i8;
        handle.write(context, number);
        if number != 0 {
            let effect = ram::g_player_info(revision).at(i32::from(number));
            let player = super::word_indexed_player(revision, index);
            effect
                .behavior_flags()
                .write(context, effect.behavior_flags().read(context) | 8);
            effect.owner().write(context, index as i8);
            effect.level().write(context, player.level().read(context));
            effect
                .tile()
                .write(context, effect.tile().read(context) | 0x8000);
            effect.width().write(context, 32);
            effect.height_pixels().write(context, 32);
            effect.sprite_count().write(context, 1);
            effect.tile_count().write(context, 16);
            let animation = rom::player_control_remap_animations(revision).at(0);
            effect
                .sequence()
                .write(context, animation.sequence().read(context));
            effect
                .frames()
                .write(context, animation.frames().read(context));
            effect
                .frame_phase()
                .write(context, animation.period().read(context));
            effect.frame().write(context, -1);
            effect.frame_duration().write(context, 1);
        }
    }
    if handle.read(context) != 0 {
        ram::g_player_control_remap_timers(revision)
            .at(index)
            .write(context, 240);
        let mapping = call(context, ids::GET_RANDOM_NUMBER, &[])? & 3;
        ram::g_player_control_remap_indices(revision)
            .at(index)
            .write(context, mapping as i8);
    }
    Ok(None)
}

/// G/src/presents/projectile_effects.c:484 (`PresentCreateActiveTerrainEffect`).
/// Atomic: RAM entity allocation and ROM descriptor, then a queued FM effect.
pub fn present_create_active_terrain_effect(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = Address::<ObjectState>::new(arguments[0]);
    let index = i32::from(arguments[1] as i16);
    let number = call(context, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[0])? as i8;
    if number != 0 {
        ram::g_player_active_terrain_effect_entities(revision)
            .at(index)
            .write(context, number);
        let effect = ram::g_player_info(revision).at(i32::from(number));
        effect.x().write(context, player.x().read(context));
        effect
            .y()
            .write(context, player.y().read(context).wrapping_sub(1));
        effect.level().write(context, player.level().read(context));
        effect
            .tile()
            .write(context, effect.tile().read(context) | 0x8000);
        effect.width().write(context, 24);
        effect.height_pixels().write(context, 20);
        effect.sprite_count().write(context, 1);
        effect.tile_count().write(context, 16);
        let animation = rom::active_terrain_effect_animations(revision).at(0);
        effect
            .sequence()
            .write(context, animation.sequence().read(context));
        effect
            .frames()
            .write(context, animation.frames().read(context));
        effect
            .frame_phase()
            .write(context, animation.period().read(context));
        effect.frame().write(context, -1);
        effect.frame_duration().write(context, 1);
        effect
            .behavior_flags()
            .write(context, effect.behavior_flags().read(context) | 1);
        call(context, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[13])?;
    }
    Ok(None)
}

/// G/src/presents/projectile_effects.c:439 (`EarthlingUpdateControlRemapProjectiles`).
/// Atomic: four RAM handles, terrain/collision queries and entity/remap helpers.
/// The terrain query and decoy-hit result retain this TU's mismatched int view.
pub fn earthling_update_control_remap_projectiles(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let handles = ram::g_control_remap_projectile_entities(revision);
    for slot in 0..4 {
        let number = handles.at(slot).read(context);
        if number == 0 {
            continue;
        }
        let projectile = ram::g_player_info(revision).at(i32::from(number));
        if projectile.draw_flags().read(context) & 6 == 0 {
            if projectile.height().read(context) < 5 {
                call(context, ids::FREE_ENTITY, &[number as u32])?;
                handles.at(slot).write(context, 0);
            }
        } else if projectile.sprite_state().read(context) == -1 {
            if projectile.height().read(context) <= 0 {
                if projectile.tile().read(context) & 0x8000 != 0 {
                    projectile
                        .draw_flags()
                        .write(context, projectile.draw_flags().read(context) & !0x40);
                    let view = if projectile.draw_flags().read(context) & 4 != 0 {
                        1
                    } else {
                        0
                    };
                    let args = [
                        view,
                        projectile.x().read(context) as u32,
                        projectile.y().read(context) as u32,
                    ];
                    let hit = call(context, ids::GET_MAP_TERRAIN_AT_WORLD_POSITION, &args)? as i16;
                    if hit == 0x90 {
                        projectile
                            .tile()
                            .write(context, projectile.tile().read(context) & !0x8000);
                        projectile.sprite_ready().write(context, 0);
                    } else {
                        call(context, ids::FREE_ENTITY, &[number as u32])?;
                        handles.at(slot).write(context, 0);
                    }
                }
            } else if projectile.height().read(context) <= 25 {
                let hit = call(
                    context,
                    ids::ENTITY_FIND_PROJECTILE_TARGET_HIT,
                    &[projectile.address()],
                )? as i16;
                if hit != 0 {
                    let hit = hit.wrapping_sub(1);
                    let index = i32::from(hit);
                    if hit <= 1
                        && ram::g_player_elevator_flags(revision)
                            .at(index)
                            .read(context)
                            == 0
                        && ram::g_player_inventory_open(revision)
                            .at(index)
                            .read(context)
                            == 0
                    {
                        call(
                            context,
                            ids::PLAYER_APPLY_CONTROL_REMAP_EFFECT,
                            &[hit as u32],
                        )?;
                    } else {
                        call(context, ids::PRESENT_REGISTER_DECOY_HIT, &[hit as u32])?;
                    }
                    call(context, ids::FREE_ENTITY, &[number as u32])?;
                    handles.at(slot).write(context, 0);
                }
            }
        }
    }
    Ok(None)
}

/// G/src/presents/projectile_effects.c:141 (`ProcessTomatoRain`).
/// Atomic: bounded shared-pool updates, RAM/ROM collision/animation and sound requests.
/// Preserve signed owner comparisons and this TU's mismatched terrain/decoy D0 views.
pub fn process_tomato_rain(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let handles = ram::g_shared_projectile_entities(revision);
    for slot in 0..12 {
        let number = i16::from(handles.at(slot).read(context));
        if number == 0 {
            continue;
        }
        let projectile = ram::g_player_info(revision).at(i32::from(number));
        if projectile.height().read(context) < 10 && projectile.draw_flags().read(context) & 6 == 0
        {
            call(context, ids::FREE_ENTITY, &[number as u32])?;
            handles.at(slot).write(context, 0);
        } else if projectile.sprite_state().read(context) == -1 {
            if projectile.height().read(context) <= 0 {
                if projectile.tile().read(context) & 0x8000 != 0 {
                    projectile
                        .draw_flags()
                        .write(context, projectile.draw_flags().read(context) & !0x40);
                    let view = if projectile.draw_flags().read(context) & 4 != 0 {
                        1
                    } else {
                        0
                    };
                    let args = [
                        view,
                        projectile.x().read(context) as u32,
                        projectile.y().read(context) as u32,
                    ];
                    let terrain =
                        call(context, ids::GET_MAP_TERRAIN_AT_WORLD_POSITION, &args)? as i16;
                    if terrain == 0x90 {
                        projectile
                            .tile()
                            .write(context, projectile.tile().read(context) & !0x8000);
                        projectile.sprite_ready().write(context, 0);
                        projectile.sprite_state().write(context, 21);
                    } else {
                        projectile.velocity_x().write(context, 0);
                        projectile.velocity_y().write(context, 0);
                        projectile.height().write(context, 0);
                        projectile.velocity_height().write(context, 0);
                        projectile.acceleration_height().write(context, 0);
                        let animation = if terrain != 0x80 {
                            rom::tomato_ground_impact_animation(revision)
                        } else {
                            rom::tomato_terrain128_impact_animation(revision)
                        };
                        projectile
                            .sequence()
                            .write(context, animation.sequence().read(context));
                        projectile
                            .frames()
                            .write(context, animation.frames().read(context));
                        projectile
                            .frame_phase()
                            .write(context, animation.period().read(context));
                        projectile.sprite_state().write(context, 0);
                        projectile.frame().write(context, -1);
                        projectile.frame_duration().write(context, 1);
                        if ram::g_fm_effect_template_status(revision).read(context) == 0 {
                            call(context, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[15])?;
                        }
                    }
                }
            } else if projectile.height().read(context) <= 25 {
                let owner = ram::g_shared_projectile_owners(revision)
                    .at(slot)
                    .read(context);
                let hit = call(
                    context,
                    ids::PROJECTILE_FIND_COLLISION_EXCLUDING_OWNER,
                    &[projectile.address(), owner as u32],
                )? as i16;
                if hit != 0 {
                    let hit = hit.wrapping_sub(1);
                    let index = i32::from(hit);
                    if hit <= 1
                        && ram::g_player_elevator_flags(revision)
                            .at(index)
                            .read(context)
                            == 0
                        && ram::g_player_inventory_open(revision)
                            .at(index)
                            .read(context)
                            == 0
                        && ram::g_player_interaction_choice_active(revision)
                            .at(index)
                            .read(context)
                            == 0
                    {
                        let health = ram::g_player_health(revision).at(index);
                        let damage = if ram::g_shared_projectile_owners(revision)
                            .at(slot)
                            .read(context)
                            <= 1
                        {
                            4
                        } else {
                            6
                        };
                        health.write(context, health.read(context).wrapping_sub(damage));
                    } else if call(context, ids::PRESENT_REGISTER_DECOY_HIT, &[hit as u32])? == 0 {
                        let earthling = ram::g_player_info(revision)
                            .at(index)
                            .earthling()
                            .read(context);
                        call(context, ids::DAMAGE_EARTHLING, &[earthling.address()])?;
                    }
                    projectile.velocity_x().write(context, 0);
                    projectile.velocity_y().write(context, 0);
                    projectile.velocity_height().write(context, 0);
                    projectile.acceleration_height().write(context, 0);
                    projectile
                        .draw_flags()
                        .write(context, projectile.draw_flags().read(context) & !0x40);
                    let animation = rom::tomato_target_impact_animation(revision);
                    projectile
                        .sequence()
                        .write(context, animation.sequence().read(context));
                    projectile
                        .frames()
                        .write(context, animation.frames().read(context));
                    projectile
                        .frame_phase()
                        .write(context, animation.period().read(context));
                    projectile.frame().write(context, -1);
                    projectile.frame_duration().write(context, 1);
                    projectile.sprite_state().write(context, 0);
                    call(context, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[2])?;
                }
            }
        } else if projectile.frame_duration().read(context) == 0
            && projectile.sprite_state().read(context) != 21
        {
            call(context, ids::FREE_ENTITY, &[number as u32])?;
            handles.at(slot).write(context, 0);
        }
    }
    call(
        context,
        ids::EARTHLING_UPDATE_CONTROL_REMAP_PROJECTILES,
        &[],
    )?;
    Ok(None)
}
