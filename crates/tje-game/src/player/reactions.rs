use crate::{Context, Result, call, ids};
use tje_mem::{Address, EarthlingState, ObjectState, ram, rom, rom_symbol};

/// G/src/player/earthling_damage_reactions.c:40 (`Player_Youch`).
pub(super) fn player_youch(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let number = i32::from(arguments[0] as i16);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    let revision = context.revision();
    let player = super::indexing::player_by_word_index(revision, number);
    if earthling.kind().read(context) == 14 {
        if player.animation_group_mask().read(context) & 0x40800 == 0
            && !matches!(player.sprite_state().read(context), 3 | 7 | 77)
        {
            return Ok(Some(1));
        }
    } else if call(
        context,
        ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
        &[player.address(), number as u32],
    )? == 0
        && player.animation_group_mask().read(context) & 0x40000 == 0
    {
        return Ok(Some(1));
    }
    let attacker = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    let mut attack = player
        .x()
        .read(context)
        .wrapping_sub(attacker.x().read(context));
    let dy = player
        .y()
        .read(context)
        .wrapping_sub(attacker.y().read(context));
    let mut x = player.x().read(context);
    let mut y = player.y().read(context);
    let mut sprite;
    if i32::from(attack).abs() > 10 {
        if attack > 0 {
            sprite = 55;
            attack = 11;
            x = x.wrapping_add(16);
        } else {
            sprite = 54;
            attack = 10;
            x = x.wrapping_sub(16);
        }
    } else if dy > 0 {
        sprite = 53;
        attack = 9;
        y = y.wrapping_add(16);
    } else {
        sprite = 52;
        attack = 8;
        y = y.wrapping_sub(16);
    }
    if call(
        context,
        ids::ELEVATOR_BLOCKS_PLAYER_MOVEMENT,
        &[number as u32, x as u32, y as u32, 1],
    )? == 0
    {
        player.x().write(context, x);
        player.y().write(context, y);
    }
    if player.velocity_x().read(context) > 0 {
        sprite = 55;
    } else if player.velocity_x().read(context) < 0 {
        sprite = 54;
    } else if player.velocity_y().read(context) > 0 {
        sprite = 53;
    } else if player.velocity_y().read(context) < 0 {
        sprite = 52;
    }
    call(context, ids::SET_SPRITE, &[player.address(), sprite])?;
    player.animation_motion_axes().write(context, 3);
    player
        .draw_flags()
        .write(context, player.draw_flags().read(context) | 0x40);
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
    ram::g_player_pending_hurt_damage(revision)
        .at(number)
        .write(
            context,
            rom::earthling_definitions(revision)
                .at(i32::from(earthling.kind().read(context)))
                .damage()
                .read(context),
        );
    if matches!(earthling.kind().read(context), 4 | 21) {
        call(
            context,
            ids::SET_EARTHLING_STATE,
            &[earthling.address(), attack as u32],
        )?;
        let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
        object.velocity_x().write(context, 0);
        object.velocity_y().write(context, 0);
        object.frame().write(context, -1);
        earthling.saved_x_or_status().write(context, 1);
    }
    let text = rom_symbol(revision, "GameText_01B310")
        .expect("source speech symbol")
        .address;
    call(context, ids::GENERATE_SPEECH, &[number as u32, text, 1])?;
    let sample = if number == 0 {
        rom::sfx_toejam_youch(revision).address()
    } else {
        rom::sfx_earl_youch(revision).address()
    };
    call(context, ids::PLAY_SOUND_EFFECT, &[sample, 50, 0])?;
    Ok(Some(0))
}

/// G/src/player/earthling_damage_reactions.c:177 (`HulaAbility`).
pub(super) fn hula_ability(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let number = i32::from(arguments[0] as i16);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    let revision = context.revision();
    let player = super::indexing::player_by_word_index(revision, number);
    if player.level().read(context) == 0 {
        return Ok(None);
    }
    let mut sprite = -1_i8;
    let eligible = call(
        context,
        ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
        &[player.address(), number as u32],
    )? != 0
        && player.animation_group_mask().read(context) & 0x2000_0010 == 0;
    let mut in_range = false;
    let mut dx = 0_i16;
    let mut dy = 0_i16;
    if eligible {
        let dancer = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
        dx = (i32::from(dancer.x().read(context)) - i32::from(player.x().read(context))).abs()
            as i16;
        dy = (i32::from(dancer.y().read(context)) - i32::from(player.y().read(context))).abs()
            as i16;
        in_range = dx <= 100 && dy <= 75;
    }
    if eligible && in_range {
        let delay = ram::g_player_hula_contact_delay(revision).at(number);
        let (stored, remaining) = super::counters::predecrement_byte(delay.read(context));
        delay.write(context, stored);
        if remaining < 0 {
            dy = dy.wrapping_add(dy / 3);
            let distance = if dx > dy { dx } else { dy };
            if super::poses::next_random(context) & 3 == 3 {
                sprite = 89;
                ram::g_player_hula_dance_ticks(revision)
                    .at(number)
                    .write(context, 28);
                if dx < 20 && dy < 15 {
                    let cooldown = ram::g_hula_speech_cooldown(revision);
                    if cooldown.read(context) == 0 {
                        let text = rom_symbol(revision, "GameText_01B5BE")
                            .expect("source speech symbol")
                            .address;
                        call(context, ids::GENERATE_SPEECH, &[number as u32, text, 1])?;
                        call(
                            context,
                            ids::PLAY_SOUND_EFFECT,
                            &[rom::sfx_aloha(revision).address(), 50, 1],
                        )?;
                        cooldown.write(context, 128_u8 as i8);
                    } else {
                        cooldown.write(context, cooldown.read(context).wrapping_sub(1));
                    }
                }
            }
            delay.write(context, ((i32::from(distance) >> 5) + 4) as i8);
        }
    } else if player.sprite_state().read(context) == 89 {
        sprite = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])? as i8;
        ram::g_player_hula_contact_delay(revision)
            .at(number)
            .write(context, 0);
    }
    if sprite != -1 && sprite != player.sprite_state().read(context) {
        player.animation_motion_axes().write(context, 3);
        call(context, ids::SET_SPRITE, &[player.address(), sprite as u32])?;
    }
    Ok(None)
}

/// G/src/player/earthling_damage_reactions.c:435 (`ChickenAbility`).
pub(super) fn chicken_ability(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let number = i32::from(arguments[0] as i16);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    let player = super::indexing::player_by_word_index(context.revision(), number);
    let dx =
        (i32::from(player.x().read(context)) - i32::from(earthling.x().read(context))).abs() as i16;
    let dy =
        (i32::from(player.y().read(context)) - i32::from(earthling.y().read(context))).abs() as i16;
    if dx < 25 && dy < 15 {
        call(
            context,
            ids::PLAYER_YOUCH,
            &[number as u32, earthling.address()],
        )?;
    }
    if u32::from(super::poses::next_random(context)) % 100 < 92
        || i32::from(earthling.state().read(context)) & 252 == 8
        || (dy < 50 && dx < 75)
        || call(
            context,
            ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
            &[player.address(), number as u32],
        )? == 0
    {
        return Ok(None);
    }
    let next = if dx < dy {
        if player.y().read(context) < earthling.y().read(context) {
            8
        } else {
            9
        }
    } else if player.x().read(context) < earthling.x().read(context) {
        10
    } else {
        11
    };
    call(
        context,
        ids::SET_EARTHLING_STATE,
        &[earthling.address(), next],
    )?;
    Ok(None)
}

/// G/src/player/earthling_damage_reactions.c:467 (`AI_BoogieTouch`).
pub(super) fn ai_boogie_touch(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let number = i32::from(arguments[0] as i16);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    let revision = context.revision();
    if i32::from(earthling.state().read(context)) & 252 == 8
        || call(
            context,
            ids::PLAYER_YOUCH,
            &[number as u32, earthling.address()],
        )? != 0
    {
        return Ok(None);
    }
    let player = super::indexing::player_by_word_index(revision, number);
    let dx =
        (i32::from(player.x().read(context)) - i32::from(earthling.x().read(context))).abs() as i16;
    let next = if dx > 8 {
        if player.x().read(context) > earthling.x().read(context) {
            11
        } else {
            10
        }
    } else if player.y().read(context) > earthling.y().read(context) {
        9
    } else {
        8
    };
    call(
        context,
        ids::SET_EARTHLING_STATE,
        &[earthling.address(), next],
    )?;
    call(
        context,
        ids::PLAY_SOUND_EFFECT,
        &[rom::sfx_boogie_boogie(revision).address(), 50, 0],
    )?;
    Ok(None)
}

/// G/src/player/earthling_damage_reactions.c:347 (`PlayerUpdateWhirlwindAndRecovery`).
pub(super) fn player_update_whirlwind_and_recovery(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let terrain = arguments[1] as u16;
    let revision = context.revision();
    let number = i32::from(player.entity_index().read(context));
    if player.sprite_state().read(context) == 78 {
        let timer = ram::g_player_whirlwind_recovery_timers(revision).at(number);
        let (stored, remaining) = super::counters::predecrement_byte(timer.read(context));
        timer.write(context, stored);
        if remaining <= 0 {
            let sprite;
            if ram::g_player_powerups(revision).at(number).read(context) == 5 {
                sprite = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])? as i8;
            } else if ram::g_player_powerups(revision).at(number).read(context) == 0 {
                sprite = 61;
                player.velocity_height().write(context, -1);
                player.animation_motion_axes().write(context, 0);
            } else if terrain >= 0x80 {
                if terrain == 0x90 {
                    call(
                        context,
                        ids::PLAYER_START_FALLING,
                        &[player.address(), number as u32],
                    )?;
                    sprite = -1;
                } else {
                    sprite = 65;
                }
            } else {
                sprite = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])? as i8;
                player.animation_motion_axes().write(context, 3);
                if super::poses::next_random(context) & 3 == 1 {
                    let text = rom_symbol(revision, "GameText_01BB56")
                        .expect("source speech symbol")
                        .address;
                    call(context, ids::GENERATE_SPEECH, &[number as u32, text, 1])?;
                }
            }
            if sprite != -1 {
                call(context, ids::SET_SPRITE, &[player.address(), sprite as u32])?;
            }
        } else if timer.read(context) & 15 == 0 {
            player
                .frame_phase()
                .write(context, player.frame_phase().read(context).wrapping_add(1));
        }
    } else if player.sprite_state().read(context) == 79 {
        let timer = ram::g_player_whirlwind_timers(revision).at(number);
        let (stored, remaining) = super::counters::predecrement_word(timer.read(context));
        timer.write(context, stored);
        if remaining <= 0 {
            player.velocity_x().write(context, 0);
            player.velocity_y().write(context, 0);
            call(context, ids::SET_SPRITE, &[player.address(), 78])?;
            ram::g_player_whirlwind_recovery_timers(revision)
                .at(number)
                .write(context, 48);
            player
                .draw_flags()
                .write(context, player.draw_flags().read(context) & !0x40);
        } else {
            let timer = ram::g_player_whirlwind_direction_timers(revision).at(number);
            let (stored, remaining) = super::counters::predecrement_byte(timer.read(context));
            timer.write(context, stored);
            if remaining <= 0 {
                let direction = (super::poses::next_random(context) & 7) as i8;
                if !(2..=7).contains(&direction) {
                    player.velocity_y().write(context, -256);
                } else if direction > 2 && direction < 6 {
                    player.velocity_y().write(context, 256);
                }
                if direction > 0 && direction < 4 {
                    player.velocity_x().write(context, 256);
                } else if direction > 4 {
                    player.velocity_x().write(context, -256);
                }
                if direction & 1 != 0 {
                    player.velocity_x().write(
                        context,
                        ((i32::from(player.velocity_x().read(context)) >> 2) * 3) as i16,
                    );
                    player.velocity_y().write(
                        context,
                        ((i32::from(player.velocity_y().read(context)) >> 2) * 3) as i16,
                    );
                }
                let duration = 20 + super::poses::next_random(context) % 6;
                timer.write(context, duration as i8);
            }
            call(
                context,
                ids::ENTITY_UPDATE_TERRAIN_DRAW_FLAG,
                &[player.address()],
            )?;
        }
    }
    Ok(None)
}
