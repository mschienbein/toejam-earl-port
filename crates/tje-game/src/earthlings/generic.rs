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
/// The ROM `_L139` literal follows RTS, outside the inventoried executable size.
pub fn ai_generic_enemy_think(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = crate::call_frame::LocalFrame::enter(
        context,
        crate::call_frame::FrameLayout::link(4, 0x7c, 0x3c),
    )?;
    let earthling = Address::<EarthlingState>::new(frame.argument_slot(0).read(context));
    // c_generic_enemy.s:117: nearest-target's no-candidate shark bug can
    // consume incoming A3, not merely its four declared stack arguments.
    context.set_address_register(3, ram::g_random_cache_index(context.revision()).address());
    let result = generic_enemy_think(context, earthling);
    super::framed::finish(context, frame, result)
}

fn generic_enemy_think(
    context: &mut dyn Context,
    earthling: Address<EarthlingState>,
) -> Result<Option<u32>> {
    use super::framed::{invoke, invoke_on};
    use super::pending_access::next_cached_index;
    let revision = context.revision();
    let caller = ids::AI_GENERIC_ENEMY_THINK;
    let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    let special = invoke(
        context,
        caller,
        ids::EARTHLING_PROCESS_SPECIAL_THINK_STATES,
        &[earthling.address(), object.address()],
        0x0198ac,
    )?;
    if special != 0 {
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
        context.discard_u16(earthling.timer().address())?;
        earthling.timer().write(context, 0);
        earthling
            .state_delay()
            .write(context, definition.state_delay().read(context));
        object
            .sprite_state()
            .write(context, earthling.state().read(context));
    }
    let old_timer = earthling.timer().read(context);
    let (stored_timer, timer) = super::predecrement_word(old_timer);

    earthling.timer().write(context, stored_timer);
    if timer < 0 {
        context.discard_u16(object.velocity_x().address())?;
        object.velocity_x().write(context, 0);
        context.discard_u16(object.velocity_y().address())?;
        object.velocity_y().write(context, 0);
        if earthling.state_delay().read(context) == 0 {
            let index = next_cached_index(context);
            let chance = definition.target_chance().read(context);
            let random = u32::from(
                ram::g_cached_random_values(revision)
                    .at(i32::from(index))
                    .read(context),
            ) % 100;
            if super::behaviors::chance_exceeds_remainder(chance, random) {
                if earthling.kind().read(context) == 15 {
                    search_bias = 100
                }
                let kind = earthling.kind().read(context) as u32;
                let distance = u32::from(definition.target_distance().read(context));
                next = invoke(
                    context,
                    caller,
                    ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
                    &[object.address(), distance, search_bias, kind],
                    0x019978,
                )? as i8 as i16;
                if next != -1 {
                    let minimum = definition.target_time_min().read(context);
                    let maximum = definition.target_time_max().read(context);
                    let timer = pending_random_timer(context, minimum, maximum)?;
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
            let old = earthling.state_delay().read(context);
            let next = old.wrapping_sub(1);
            earthling.state_delay().write(context, next);
        }
        if next == -1 && earthling.state().read(context) != 2 {
            let index = next_cached_index(context);
            let chance = definition.turn_chance().read(context);
            let random = u32::from(
                ram::g_cached_random_values(revision)
                    .at(i32::from(index))
                    .read(context),
            ) % 100;
            if super::behaviors::chance_exceeds_remainder(chance, random) {
                next = invoke(
                    context,
                    caller,
                    ids::EARTHLING_SELECT_TURN_STATE,
                    &[earthling.address()],
                    0x019a80,
                )? as i8 as i16;
                if earthling.kind().read(context) == 3
                    && next == 0
                    && pending_random_byte(context) & 1 != 0
                {
                    let speaker = earthling.object().read(context) as u32;
                    let metadata = ids::AI_GENERIC_ENEMY_THINK.metadata();
                    let text = metadata.address(revision) + metadata.size(revision);
                    invoke(
                        context,
                        caller,
                        ids::GENERATE_SPEECH,
                        &[speaker, text, 1],
                        0x019ad0,
                    )?;
                }
                if i16::from(earthling.state().read(context)) != next {
                    object.frame().write(context, -1)
                } else {
                    next = -1
                }
                let minimum = definition.turn_time_min().read(context);
                let maximum = definition.turn_time_max().read(context);
                let timer = pending_random_timer(context, minimum, maximum)?;
                earthling.timer().write(context, timer);
            } else {
                next = (pending_random_word(context) & 3) as i16 + 4;
                let minimum = definition.move_time_min().read(context);
                let maximum = definition.move_time_max().read(context);
                let timer = pending_random_timer(context, minimum, maximum)?;
                earthling.timer().write(context, timer);
            }
        }
        if definition.behavior_flags().read(context) & 2 != 0
            && next >= 4
            && i16::from(earthling.state().read(context)) != next
            && (next & 2) == (i16::from(earthling.state().read(context)) & 2)
        {
            next = (next ^ 2) & 254;
            next |= (pending_random_byte(context) & 1) as i16;
            context.discard_u16(object.velocity_x().address())?;
            object.velocity_x().write(context, 0);
            context.discard_u16(object.velocity_y().address())?;
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
            invoke(
                context,
                caller,
                ids::SET_EARTHLING_STATE,
                &[earthling.address(), next as u32],
                0x019cc2,
            )?;
            if earthling.kind().read(context) == 4 || earthling.kind().read(context) == 21 {
                object.frame().write(context, -1)
            }
            if earthling.kind().read(context) == 18 {
                let draw_late = earthling.state().read(context) > 1;
                let flags = object.draw_flags().read(context);
                object.draw_flags().write(
                    context,
                    if draw_late {
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
    let kind = earthling.kind().read(context);

    if kind == 18 {
        let state = i32::from(earthling.state().read(context)) & 252;

        if state == 4 {
            let arguments = crate::call_frame::OutgoingArguments::push(
                context,
                &[earthling.address(), object.address()],
            )?;
            invoke_on(
                context,
                caller,
                ids::EARTHLING_SPAWN_MOLE_EFFECT,
                &arguments,
                0x019d22,
            )?;
            let flags = object.behavior_flags().read(context) & !1;
            object.behavior_flags().write(context, flags);

            arguments.release(context)?;
        }
    }
    Ok(None)
}

fn pending_random_word(context: &mut dyn Context) -> u16 {
    let index = super::pending_access::next_cached_index(context);
    ram::g_cached_random_values(context.revision())
        .at(i32::from(index))
        .read(context)
}

fn pending_random_byte(context: &mut dyn Context) -> u8 {
    let index = super::pending_access::next_cached_index(context);
    // c_generic_enemy.s:332 / 502: BTST or MOVE.B of the low byte, not MOVE.W.
    ram::g_cached_random_values(context.revision())
        .at(i32::from(index))
        .cast::<u8>()
        .byte_offset(1)
        .read(context)
}

fn pending_random_timer(context: &mut dyn Context, minimum: u8, maximum: u8) -> Result<i16> {
    let random = u32::from(pending_random_word(context));
    let range = u32::from(maximum)
        .wrapping_sub(u32::from(minimum))
        .wrapping_add(1);
    let result = tje_mem::arithmetic::expr_rem_u32(random, range)?.wrapping_add(u32::from(minimum));
    Ok(result as i16)
}

#[cfg(test)]
mod signed_chance_tests {
    use super::ai_generic_enemy_think;
    use crate::earthlings::access_tests::ObservedContext;
    use tje_mem::{Revision, ram, rom};

    #[test]
    fn negative_target_and_turn_chances_follow_signed_ble_in_the_actual_body() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            for target_only in [true, false] {
                // Authored non-cartridge fixture: only the compared chance and
                // distinct timer constants are populated, never game ROM data.
                let definition = rom::earthling_definitions(revision).at(1);
                let mut image = vec![0_u8; 0x100000];
                image[definition.target_chance().address() as usize] = 0xff;
                image[definition.turn_chance().address() as usize] = 0xff;
                image[definition.move_time_min().address() as usize] = 3;
                image[definition.move_time_max().address() as usize] = 3;
                image[definition.turn_time_min().address() as usize] = 7;
                image[definition.turn_time_max().address() as usize] = 7;
                let mut context = ObservedContext::with_rom(revision, image);
                let record = ram::g_earthling_banks(revision).at(0).at(0);
                let object = ram::g_player_info(revision).at(6);
                record.kind().write(&mut context, 1);
                record.object().write(&mut context, 6);
                record
                    .state()
                    .write(&mut context, if target_only { 2 } else { 0 });
                record
                    .state_delay()
                    .write(&mut context, i8::from(!target_only));
                record.timer().write(&mut context, 0);
                object.sprite_state().write(&mut context, 6);
                object.frame().write(&mut context, -1);
                ram::g_cached_random_values(revision)
                    .at(1)
                    .write(&mut context, 50);
                context.prepare(&[record.address()]);
                ai_generic_enemy_think(&mut context).unwrap();
                assert_eq!(
                    record.state().read(&context),
                    if target_only { 2 } else { 4 }
                );
                assert_eq!(
                    record.timer().read(&context),
                    if target_only { 1 } else { 3 }
                );
                assert_eq!(
                    ram::g_random_cache_index(revision).read(&context),
                    if target_only { 1 } else { 3 },
                );
            }
        }
    }
}
