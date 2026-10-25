use super::decisions::next_cached_random;
use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, EarthlingState, ObjectState, arithmetic, ram, rom};

/// G c_animation_behaviours.s:90–108 and c_terrain_animation.s:230–248,
/// 371–389 sign-extend the chance byte and use signed CMP.L/BLE.
pub(super) fn chance_exceeds_remainder(chance: i8, remainder: u32) -> bool {
    i32::from(chance) > remainder as i32
}

pub(super) fn random_timer(context: &mut dyn Context, minimum: u8, maximum: u8) -> Result<i16> {
    let random = u32::from(next_cached_random(context));
    let range = u32::from(maximum)
        .wrapping_sub(u32::from(minimum))
        .wrapping_add(1);
    Ok(arithmetic::expr_rem_u32(random, range)?.wrapping_add(u32::from(minimum)) as i16)
}

fn object_for(
    context: &dyn Context,
    earthling: Address<EarthlingState>,
) -> Option<Address<ObjectState>> {
    let handle = earthling.object().read(context);
    (handle != 0).then(|| ram::g_player_info(context.revision()).at(i32::from(handle)))
}

/// G/src/earthlings/animation_behaviours.c:16 (`EarthlingThinkHorizontalWander`).
/// Atomic: exact cached-RNG evaluation order and signed timer/velocity updates.
pub fn earthling_think_horizontal_wander(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let Some(object) = object_for(context, earthling) else {
        return Ok(None);
    };
    let definition = rom::earthling_definitions(context.revision())
        .at(i32::from(earthling.kind().read(context)));
    let mut next = -1_i16;
    if object.sprite_state().read(context) == 0
        && earthling.state().read(context) != 0
        && earthling.state().read(context) != 2
    {
        object
            .velocity_x()
            .write(context, object.velocity_x().read(context).wrapping_neg());
        if next_cached_random(context) & 1 != 0 {
            object
                .velocity_y()
                .write(context, object.velocity_y().read(context).wrapping_neg())
        }
        next = i16::from(earthling.state().read(context)) ^ 1;
        earthling.timer().write(context, 3);
        earthling
            .state_delay()
            .write(context, definition.state_delay().read(context));
        call_void(
            context,
            ids::SET_EARTHLING_STATE,
            &[earthling.address(), next as u32],
        )?;
    }
    let (stored_timer, timer) = super::predecrement_word(earthling.timer().read(context));
    earthling.timer().write(context, stored_timer);
    if timer < 0 {
        if earthling.state_delay().read(context) == 0 {
            let random = u32::from(next_cached_random(context)) % 100;
            if chance_exceeds_remainder(definition.target_chance().read(context), random) {
                let previous_x = object.velocity_x().read(context);
                let previous_y = object.velocity_y().read(context);
                let distance = u32::from(definition.target_distance().read(context));
                let kind = earthling.kind().read(context) as u32;
                next = call(
                    context,
                    ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
                    &[object.address(), distance, 0, kind],
                )? as i8 as i16;
                if next != -1 {
                    if earthling.state().read(context) != 2
                        && object.draw_flags().read(context) & 6 != 0
                        && (i16::from(earthling.state().read(context)) ^ next) & 1 != 0
                    {
                        next = -1;
                        object.velocity_x().write(context, previous_x);
                        object.velocity_y().write(context, previous_y);
                    }
                    let minimum = definition.target_time_min().read(context);
                    let maximum = definition.target_time_max().read(context);
                    let timer = random_timer(context, minimum, maximum)?;
                    earthling.timer().write(context, timer);
                }
            }
        } else {
            earthling.state_delay().write(
                context,
                earthling.state_delay().read(context).wrapping_sub(1),
            )
        }
        if next == -1 && earthling.state().read(context) != 2 {
            if earthling.state().read(context) == 0 {
                next = (next_cached_random(context) & 1) as i16 + 6;
                object
                    .velocity_x()
                    .write(context, if next == 6 { -320 } else { 320 });
            }
            let velocity_y = if next_cached_random(context) % 100 < 33 {
                0
            } else if next_cached_random(context) % 100 < 50 {
                -320
            } else {
                320
            };
            object.velocity_y().write(context, velocity_y);
            let minimum = definition.move_time_min().read(context);
            let maximum = definition.move_time_max().read(context);
            let timer = random_timer(context, minimum, maximum)?;
            earthling.timer().write(context, timer);
        }
        if next != -1 && i16::from(earthling.state().read(context)) != next {
            call_void(
                context,
                ids::SET_EARTHLING_STATE,
                &[earthling.address(), next as u32],
            )?;
        }
        if earthling.timer().read(context) <= 0 {
            earthling.timer().write(context, 1)
        }
    }
    Ok(None)
}

/// G/src/earthlings/animation_behaviours.c:87 (`EarthlingAdvanceHorizontalTurnAnimation`).
/// Atomic: a one-call delay, animation/facing transitions and RAM velocity stores.
pub fn earthling_advance_horizontal_turn_animation(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let Some(object) = object_for(context, earthling) else {
        return Ok(None);
    };
    if earthling.state_delay().read(context) != 0 {
        earthling.state_delay().write(context, 0);
        return Ok(None);
    }
    let mut next = -1_i16;
    if object.sprite_state().read(context) == 0 && earthling.state().read(context) >= 6 {
        next = call(
            context,
            ids::EARTHLING_SELECT_TURN_STATE,
            &[earthling.address()],
        )? as i8 as i16;
    } else if object.frame_duration().read(context) == 0 {
        let state = earthling.state().read(context);
        if state == 2 {
            next = 3
        } else if state <= 1 || state == 3 {
            if object.velocity_x().read(context) > 0 {
                next = 4;
                object.velocity_x().write(context, -320)
            } else {
                next = 5;
                object.velocity_x().write(context, 320)
            }
        } else if state == 4 {
            next = 6;
            object.velocity_x().write(context, -320)
        } else if state == 5 {
            next = 7;
            object.velocity_x().write(context, 320)
        }
    }
    if next != -1 && i16::from(earthling.state().read(context)) != next {
        call_void(
            context,
            ids::SET_EARTHLING_STATE,
            &[earthling.address(), next as u32],
        )?;
        object.frame().write(context, -1);
    }
    Ok(None)
}

/// G/src/earthlings/animation_behaviours.c:129 (`EarthlingThinkHorizontalReversal`).
/// Atomic: menu-state gating and horizontal animation/velocity reversal.
pub fn earthling_think_horizontal_reversal(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let Some(object) = object_for(context, earthling) else {
        return Ok(None);
    };
    if earthling.state_delay().read(context) != 0 {
        earthling.state_delay().write(context, 0);
        return Ok(None);
    }
    let mut next = -1_i16;
    if object.sprite_state().read(context) == 0 && earthling.state().read(context) >= 2 {
        next = call(
            context,
            ids::EARTHLING_SELECT_TURN_STATE,
            &[earthling.address()],
        )? as i8 as i16;
    } else if earthling.state().read(context) <= 1
        && ram::g_player_inventory_open(revision).at(0).read(context) != 3
        && ram::g_player2_inventory_open(revision).read(context) != 3
    {
        if object.velocity_x().read(context) > 0 {
            next = 2;
            object.velocity_x().write(context, -320)
        } else {
            next = 3;
            object.velocity_x().write(context, 320)
        }
    }
    if next != -1 && i16::from(earthling.state().read(context)) != next {
        call_void(
            context,
            ids::SET_EARTHLING_STATE,
            &[earthling.address(), next as u32],
        )?;
        object.frame().write(context, -1);
    }
    Ok(None)
}

/// G/src/earthlings/animation_behaviours.c:161 (`EarthlingThinkTargetedResetCycle`).
/// Atomic: target helper, signed byte phase countdown and object reset.
pub fn earthling_think_targeted_reset_cycle(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let earthling = Address::<EarthlingState>::new(arguments[0]);
    let Some(object) = object_for(context, earthling) else {
        return Ok(None);
    };
    let definition = rom::earthling_definitions(context.revision())
        .at(i32::from(earthling.kind().read(context)));
    if earthling.state().read(context) == 2 {
        if object.frame_duration().read(context) == 0 {
            call_void(
                context,
                ids::EARTHLING_RESTORE_SAVED_POSITION,
                &[earthling.address()],
            )?;
        }
        return Ok(None);
    }
    let (stored_timer, timer) = super::predecrement_word(earthling.timer().read(context));
    earthling.timer().write(context, stored_timer);
    if timer < 0 {
        object.velocity_x().write(context, 0);
        object.velocity_y().write(context, 0);
        let distance = u32::from(definition.target_distance().read(context));
        let next = call(
            context,
            ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
            &[object.address(), distance, 0, 8],
        )? as i8 as i16;
        if next != -1 {
            let minimum = definition.target_time_min().read(context);
            let maximum = definition.target_time_max().read(context);
            let timer = random_timer(context, minimum, maximum)?;
            earthling.timer().write(context, timer);
        }
        if earthling.state().read(context) == 3 {
            if object.behavior_flags().read(context) & 0x40 != 0 {
                if next != -1 {
                    object.frame().write(context, -1);
                    object.frame_duration().write(context, 1);
                    object.sprite_ready().write(context, 0);
                    object
                        .behavior_flags()
                        .write(context, object.behavior_flags().read(context) & !0x40);
                }
                earthling.timer().write(context, 3);
            } else if object.frame_duration().read(context) == 0 {
                call_void(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 4])?;
                earthling.timer().write(context, 0);
                earthling.health_visibility().at(0).write(context, 7);
            } else {
                earthling.timer().write(context, 1)
            }
        } else {
            let (stored_phase, phase) =
                super::predecrement_byte(earthling.health_visibility().at(0).read(context));
            earthling
                .health_visibility()
                .at(0)
                .write(context, stored_phase);
            if phase < 0 || next == -1 {
                call_void(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 2])?;
                object.frame().write(context, -1);
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod chance_tests {
    #[test]
    fn every_signed_chance_and_unsigned_draw_matches_signed_cmp_flags() {
        for revision in [tje_mem::Revision::Rev02, tje_mem::Revision::Rev00] {
            for chance in i8::MIN..=i8::MAX {
                for draw in u16::MIN..=u16::MAX {
                    let remainder = u32::from(draw) % 100;
                    let left = i32::from(chance);
                    let right = remainder as i32;
                    let difference = left.wrapping_sub(right);
                    let overflow = ((left ^ right) & (left ^ difference)) < 0;
                    let ble = difference == 0 || ((difference < 0) != overflow);
                    assert_eq!(
                        super::chance_exceeds_remainder(chance, remainder),
                        !ble,
                        "{revision:?} chance={chance} draw={draw}"
                    );
                }
            }
        }
    }
}
