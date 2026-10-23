use crate::{Context, Result, call, ids};
use tje_mem::{Address, EarthlingState, ObjectState, ram, rom};

/// G/src/earthlings/terrain_animation.c:16 (`EarthlingSelectTerrainAnimationState`).
/// Atomic: terrain/state/range RAM and a queued sound request, no device wait.
pub fn earthling_select_terrain_animation_state(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let object = Address::<ObjectState>::new(arguments[1]);
    let mut next = arguments[2] as i16;
    let animation = i16::from(earthling.state().read(context)) & 252;
    if animation == 4 {
        if earthling.saved_x_or_status().read(context) >= 0x80 {
            next = next.wrapping_add(4)
        }
    } else if animation == 8 {
        if object.frame_duration().read(context) == 0 {
            if earthling.saved_x_or_status().read(context) >= 0x80 {
                next = next.wrapping_add(8)
            } else {
                call(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_ice_cream_honk(revision).address(), 50, 0],
                )?;
            }
        } else {
            next = next.wrapping_add(4)
        }
    } else if animation == 12 {
        if earthling.saved_x_or_status().read(context) >= 0x50
            || call(
                context,
                ids::ENTITY_IS_WITHIN_RANGE,
                &[
                    object.address(),
                    ram::g_player_info(revision).at(0).address(),
                    96,
                    48,
                ],
            )? != 0
            || call(
                context,
                ids::ENTITY_IS_WITHIN_RANGE,
                &[
                    object.address(),
                    ram::g_player2_entity(revision).address(),
                    96,
                    48,
                ],
            )? != 0
        {
            next = next.wrapping_add(8);
        } else {
            next = next.wrapping_add(4)
        }
    }
    if animation != (next & 252) {
        object.frame().write(context, -1)
    }
    Ok(Some(next as i8 as u32))
}

/// G/src/earthlings/terrain_animation.c:49 (`EarthlingUpdateTerrainAnimationState`).
/// Atomic: ROM animation and sound-request RAM; keep the collidable-bit ordering.
pub fn earthling_update_terrain_animation_state(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let object = Address::<ObjectState>::new(arguments[1]);
    let mut next = -1_i16;
    let animation = i16::from(earthling.state().read(context)) & 252;
    if animation == 4 {
        if earthling.saved_x_or_status().read(context) >= 0x80 {
            next = i16::from(earthling.state().read(context)) + 4
        }
    } else if animation == 8 {
        if object.frame_duration().read(context) == 0 {
            if earthling.saved_x_or_status().read(context) >= 0x80 {
                next = i16::from(earthling.state().read(context)) + 4
            } else {
                next = i16::from(earthling.state().read(context)) - 4;
                call(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_ice_cream_honk(revision).address(), 50, 0],
                )?;
            }
        }
    } else if animation == 12
        && earthling.saved_x_or_status().read(context) < 0x50
        && call(
            context,
            ids::ENTITY_IS_WITHIN_RANGE,
            &[
                object.address(),
                ram::g_player_info(revision).at(0).address(),
                128,
                64,
            ],
        )? == 0
        && call(
            context,
            ids::ENTITY_IS_WITHIN_RANGE,
            &[
                object.address(),
                ram::g_player2_entity(revision).address(),
                128,
                64,
            ],
        )? == 0
    {
        next = i16::from(earthling.state().read(context)) - 4;
    }
    if next >= 0 {
        call(
            context,
            ids::SET_EARTHLING_STATE,
            &[earthling.address(), next as u32],
        )?;
        object.frame().write(context, -1);
        let flags = object.behavior_flags().read(context);
        object.behavior_flags().write(
            context,
            if next & 252 == 4 {
                flags | 1
            } else {
                flags & !1
            },
        );
    }
    Ok(None)
}

/// G/src/earthlings/terrain_animation.c:88 (`EarthlingThinkTimedMovement`).
/// Atomic: preserve cached-RNG order, timer truncations and type-specific attacks.
/// The early Cupid/state-three call forwards entry D1 unchanged: G's compiled
/// caller (c_terrain_animation.s:167–179) never writes it before the call. The
/// callee's unmatched-map original bug therefore retains that exact residue.
pub fn earthling_think_timed_movement(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let mut next = -1_i16;
    let mut separation = 0_u32;
    if earthling.kind().read(context) == 7 && earthling.state().read(context) == 3 {
        call(
            context,
            ids::EARTHLING_UPDATE_BOUNCING_DAMAGE_STATE,
            &[earthling.address()],
        )?;
        return Ok(None);
    }
    if earthling.object().read(context) == 0 {
        return Ok(None);
    }
    let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    let definition =
        rom::earthling_definitions(revision).at(i32::from(earthling.kind().read(context)));
    if object.sprite_state().read(context) == 0
        && earthling.state().read(context) != 0
        && earthling.state().read(context) != 2
    {
        earthling.timer().write(context, 0);
        earthling
            .state_delay()
            .write(context, definition.state_delay().read(context));
    }
    let (stored_timer, timer) = super::predecrement_word(earthling.timer().read(context));
    earthling.timer().write(context, stored_timer);
    if timer < 0 {
        object.velocity_x().write(context, 0);
        object.velocity_y().write(context, 0);
        if earthling.state_delay().read(context) == 0 {
            let random = u32::from(super::decisions::next_cached_random(context)) % 100;
            if (i32::from(definition.target_chance().read(context)) as u32) > random {
                if earthling.kind().read(context) == 7 {
                    separation = 85
                }
                let distance = u32::from(definition.target_distance().read(context));
                let kind = earthling.kind().read(context) as u32;
                next = call(
                    context,
                    ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
                    &[object.address(), distance, separation, kind],
                )? as i8 as i16;
                if next != -1 {
                    let minimum = definition.target_time_min().read(context);
                    let maximum = definition.target_time_max().read(context);
                    let timer = super::behaviors::random_timer(context, minimum, maximum)?;
                    earthling.timer().write(context, timer);
                }
                if (earthling.kind().read(context) == 6 || earthling.kind().read(context) == 22)
                    && i16::from(earthling.state().read(context)) != next
                    && earthling.state().read(context) >= 4
                    && next >= 4
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
        if earthling.kind().read(context) == 19 && next == 0 {
            next = -1
        }
        if earthling.kind().read(context) == 7
            && i16::from(earthling.state().read(context)) & 252 == 8
        {
            next = -1
        } else if next == -1 && earthling.state().read(context) != 2 {
            let random = u32::from(super::decisions::next_cached_random(context)) % 100;
            if (i32::from(definition.turn_chance().read(context)) as u32) > random {
                next = call(
                    context,
                    ids::EARTHLING_SELECT_TURN_STATE,
                    &[earthling.address()],
                )? as i8 as i16;
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
        if object.velocity_x().read(context) == 0 && object.velocity_y().read(context) == 0 {
            match next {
                4 => object.velocity_y().write(context, -320),
                5 => object.velocity_y().write(context, 320),
                6 => object.velocity_x().write(context, -320),
                7 => object.velocity_x().write(context, 320),
                _ => {}
            }
        }
        if earthling.kind().read(context) == 19 && next != -1 {
            next = call(
                context,
                ids::EARTHLING_SELECT_TERRAIN_ANIMATION_STATE,
                &[earthling.address(), object.address(), next as u32],
            )? as i8 as i16;
        }
        if next != -1 && i16::from(earthling.state().read(context)) != next {
            call(
                context,
                ids::SET_EARTHLING_STATE,
                &[earthling.address(), next as u32],
            )?;
            if earthling.kind().read(context) == 7
                && earthling.saved_x_or_status().read(context) != 0
                && next == 0
                && object.draw_flags().read(context) & 6 != 0
            {
                call(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_cupid_sing(revision).address(), 40, 0],
                )?;
                earthling.saved_x_or_status().write(context, 0);
            } else if earthling.kind().read(context) == 19 {
                let flags = object.behavior_flags().read(context);
                object.behavior_flags().write(
                    context,
                    if next & 252 == 4 {
                        flags | 1
                    } else {
                        flags & !1
                    },
                );
            }
        }
        if earthling.timer().read(context) <= 0 {
            earthling.timer().write(context, 1)
        }
    }
    if earthling.kind().read(context) == 19
        && (next == -1 || i16::from(earthling.state().read(context)) != next)
    {
        call(
            context,
            ids::EARTHLING_UPDATE_TERRAIN_ANIMATION_STATE,
            &[earthling.address(), object.address()],
        )?;
    }
    if earthling.kind().read(context) == 7 && i16::from(earthling.state().read(context)) & 252 == 8
    {
        call(
            context,
            ids::EARTHLING_UPDATE_CONTROL_REMAP_ATTACK,
            &[earthling.address(), object.address()],
        )?;
    }
    Ok(None)
}
