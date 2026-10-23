use crate::{Context, Result, call, ids};
use tje_mem::{Address, EarthlingState, ObjectState, ram, rom};

/// G/src/earthlings/generic_enemy.c:20 (`EarthlingProcessSpecialThinkStates`).
/// Atomic: special RAM animation/attack branches and queued sound requests.
pub fn earthling_process_special_think_states(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let object = Address::<ObjectState>::new(arguments[1]);
    if earthling.kind().read(context) == 9
        && (i16::from(earthling.state().read(context)) & 252) != 4
    {
        return Ok(Some(1));
    }
    if earthling.kind().read(context) == 17 && earthling.state_delay().read(context) != 0 {
        earthling.state_delay().write(context, 0);
        return Ok(Some(1));
    }
    if earthling.object().read(context) == 0 {
        return Ok(Some(1));
    }
    if earthling.kind().read(context) == 18 {
        if earthling.state().read(context) <= 1 {
            object
                .behavior_flags()
                .write(context, object.behavior_flags().read(context) | 1);
            object
                .draw_flags()
                .write(context, object.draw_flags().read(context) & !0x80);
            if object.frame_duration().read(context) != 0 {
                return Ok(Some(1));
            }
            earthling.timer().write(context, 0);
        } else if earthling.state().read(context) == 3 {
            object
                .behavior_flags()
                .write(context, object.behavior_flags().read(context) | 1);
            let (stored_timer, timer) = super::predecrement_word(earthling.timer().read(context));
            earthling.timer().write(context, stored_timer);
            if timer > 0 {
                return Ok(Some(1));
            }
        }
    }
    if i16::from(earthling.state().read(context)) & 252 == 8 {
        if earthling.kind().read(context) == 4 || earthling.kind().read(context) == 21 {
            if ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0
                && object.frame_duration().read(context) > 3
                && earthling.saved_x_or_status().read(context) != 0
            {
                call(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_dentist_laugh(revision).address(), 50, 0],
                )?;
                earthling.saved_x_or_status().write(context, 0);
            }
            if object.frame_duration().read(context) != 0 {
                return Ok(Some(1));
            }
            earthling.timer().write(context, 0);
        }
        if earthling.kind().read(context) == 15 {
            call(
                context,
                ids::EARTHLING_UPDATE_AIMED_PROJECTILE_ATTACK,
                &[earthling.address(), object.address()],
            )?;
            return Ok(Some(1));
        }
        if earthling.kind().read(context) == 16 {
            if object.frame_duration().read(context) != 0 {
                return Ok(Some(1));
            }
            earthling.timer().write(context, 0);
        }
    }
    Ok(Some(0))
}

/// G/src/earthlings/generic_enemy.c:71 (`AI_GenericEnemyThink`).
/// TimeAware: the shopper speech branch waits for interrupt-owned sprite uploads.
/// The literal remains in the user's ROM (compiler `_L139`, final ten bytes).
pub fn ai_generic_enemy_think(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    if call(
        context,
        ids::EARTHLING_PROCESS_SPECIAL_THINK_STATES,
        &[earthling.address(), object.address()],
    )? != 0
    {
        return Ok(None);
    }
    let definition =
        rom::earthling_definitions(revision).at(i32::from(earthling.kind().read(context)));
    let mut next = -1_i16;
    let mut search_bias = 0;
    if object.sprite_state().read(context) == 0
        && earthling.state().read(context) != 0
        && earthling.state().read(context) != 2
    {
        earthling.timer().write(context, 0);
        earthling
            .state_delay()
            .write(context, definition.state_delay().read(context));
        object
            .sprite_state()
            .write(context, earthling.state().read(context));
    }
    let (stored_timer, timer) = super::predecrement_word(earthling.timer().read(context));
    earthling.timer().write(context, stored_timer);
    if timer < 0 {
        object.velocity_x().write(context, 0);
        object.velocity_y().write(context, 0);
        if earthling.state_delay().read(context) == 0 {
            let random = u32::from(super::decisions::next_cached_random(context)) % 100;
            if random < (i32::from(definition.target_chance().read(context)) as u32) {
                if earthling.kind().read(context) == 15 {
                    search_bias = 100
                }
                let distance = u32::from(definition.target_distance().read(context));
                let kind = earthling.kind().read(context) as u32;
                next = call(
                    context,
                    ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
                    &[object.address(), distance, search_bias, kind],
                )? as i8 as i16;
                if next != -1 {
                    let minimum = definition.target_time_min().read(context);
                    let maximum = definition.target_time_max().read(context);
                    let timer = super::behaviors::random_timer(context, minimum, maximum)?;
                    earthling.timer().write(context, timer);
                }
                if earthling.kind().read(context) == 15
                    && i16::from(earthling.state().read(context)) != next
                    && earthling.state().read(context) & 4 != 0
                    && next & 4 != 0
                {
                    next = 0;
                    earthling.timer().write(context, 2);
                }
            }
        } else {
            earthling.state_delay().write(
                context,
                earthling.state_delay().read(context).wrapping_sub(1),
            )
        }
        if next == -1 && earthling.state().read(context) != 2 {
            let random = u32::from(super::decisions::next_cached_random(context)) % 100;
            if random < (i32::from(definition.turn_chance().read(context)) as u32) {
                next = call(
                    context,
                    ids::EARTHLING_SELECT_TURN_STATE,
                    &[earthling.address()],
                )? as i8 as i16;
                if earthling.kind().read(context) == 3
                    && next == 0
                    && super::decisions::next_cached_random(context) & 1 != 0
                {
                    let speaker = earthling.object().read(context) as u32;
                    let metadata = ids::AI_GENERIC_ENEMY_THINK.metadata();
                    let text = metadata.address(revision) + metadata.size(revision) - 10;
                    call(context, ids::GENERATE_SPEECH, &[speaker, text, 1])?;
                }
                if i16::from(earthling.state().read(context)) != next {
                    object.frame().write(context, -1)
                } else {
                    next = -1
                }
                let minimum = definition.turn_time_min().read(context);
                let maximum = definition.turn_time_max().read(context);
                let timer = super::behaviors::random_timer(context, minimum, maximum)?;
                earthling.timer().write(context, timer);
            } else {
                next = (super::decisions::next_cached_random(context) & 3) as i16 + 4;
                let minimum = definition.move_time_min().read(context);
                let maximum = definition.move_time_max().read(context);
                let timer = super::behaviors::random_timer(context, minimum, maximum)?;
                earthling.timer().write(context, timer);
            }
        }
        if definition.behavior_flags().read(context) & 2 != 0
            && next >= 4
            && i16::from(earthling.state().read(context)) != next
            && (next & 2) == (i16::from(earthling.state().read(context)) & 2)
        {
            next = (next ^ 2) & 254;
            next |= (super::decisions::next_cached_random(context) & 1) as i16;
            object.velocity_x().write(context, 0);
            object.velocity_y().write(context, 0);
        }
        if object.velocity_x().read(context) == 0 && object.velocity_y().read(context) == 0 {
            match next {
                4 => object.velocity_y().write(context, -320),
                5 => object.velocity_y().write(context, 320),
                6 => object.velocity_x().write(context, -320),
                7 => object.velocity_x().write(context, 320),
                _ => {}
            }
        }
        if next != -1 && i16::from(earthling.state().read(context)) != next {
            call(
                context,
                ids::SET_EARTHLING_STATE,
                &[earthling.address(), next as u32],
            )?;
            if earthling.kind().read(context) == 4 || earthling.kind().read(context) == 21 {
                object.frame().write(context, -1)
            }
            if earthling.kind().read(context) == 18 {
                let flags = object.draw_flags().read(context);
                object.draw_flags().write(
                    context,
                    if earthling.state().read(context) > 1 {
                        flags | 0x80
                    } else {
                        flags & !0x80
                    },
                );
            }
        }
        if earthling.timer().read(context) <= 0 {
            earthling.timer().write(context, 1)
        }
    }
    if earthling.kind().read(context) == 18 && i16::from(earthling.state().read(context)) & 252 == 4
    {
        call(
            context,
            ids::EARTHLING_SPAWN_MOLE_EFFECT,
            &[earthling.address(), object.address()],
        )?;
        object
            .behavior_flags()
            .write(context, object.behavior_flags().read(context) & !1);
    }
    Ok(None)
}
