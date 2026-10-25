use super::frames::{
    advance_random_index, call_from, call_from_void, clear_word, read_long, with_frame,
};
use crate::call_frame::{CallSite, FrameLayout, OutgoingArguments};
use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, EarthlingState, ObjectState, ram, rom, rom_symbol};

/// G/src/player/earthling_damage_reactions.c:40 (`Player_Youch`).
pub(super) fn player_youch(context: &mut dyn Context) -> Result<Option<u32>> {
    // G/build/{rev00,rev02}/asm/c_earthling_damage_reactions.s:34-231.
    with_frame(
        context,
        FrameLayout::link(4, 0xfc, 0x1c),
        |context, stack| {
            let number = i32::from(stack.argument_word(0).read(context));
            let earthling = Address::<EarthlingState>::new(stack.argument_slot(1).read(context));
            let revision = context.revision();
            let player = super::indexing::player_by_word_index(revision, number);
            if earthling.kind().read(context) == 14 {
                if read_long(context, player.animation_group_mask().address()) & 0x40800 == 0
                    && player.sprite_state().read(context) != 3
                    && player.sprite_state().read(context) != 7
                    && player.sprite_state().read(context) != 77
                {
                    return Ok(Some(1));
                }
            } else if call_from(
                context,
                ids::PLAYER_YOUCH,
                ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
                &[player.address(), number as u32],
                0x5e,
            )? == 0
                && read_long(context, player.animation_group_mask().address()) & 0x40000 == 0
            {
                return Ok(Some(1));
            }
            let attacker =
                ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
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
            if call_from(
                context,
                ids::PLAYER_YOUCH,
                ids::ABS,
                &[i32::from(attack) as u32],
                0xa4,
            )? as i32
                > 10
            {
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
            if call_from(
                context,
                ids::PLAYER_YOUCH,
                ids::ELEVATOR_BLOCKS_PLAYER_MOVEMENT,
                &[number as u32, x as u32, y as u32, 1],
                0xf8,
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
            call_from_void(
                context,
                ids::PLAYER_YOUCH,
                ids::SET_SPRITE,
                &[player.address(), sprite],
                0x142,
            )?;
            player.animation_motion_axes().write(context, 3);
            player
                .draw_flags()
                .write(context, player.draw_flags().read(context) | 0x40);
            if ram::g_player_terrain_sinking(revision)
                .at(number)
                .read(context)
                != 0
            {
                player.height().write(context, -3);
            } else {
                clear_word(context, player.height().address())?;
            }
            ram::g_player_pending_hurt_damage(revision)
                .at(number)
                .write(
                    context,
                    rom::earthling_definitions(revision)
                        .at(i32::from(earthling.kind().read(context)))
                        .damage()
                        .read(context),
                );
            if earthling.kind().read(context) == 4 || earthling.kind().read(context) == 21 {
                call_from_void(
                    context,
                    ids::PLAYER_YOUCH,
                    ids::SET_EARTHLING_STATE,
                    &[earthling.address(), attack as u32],
                    0x1a4,
                )?;
                let object =
                    ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
                clear_word(context, object.velocity_x().address())?;
                clear_word(context, object.velocity_y().address())?;
                object.frame().write(context, -1);
                earthling.saved_x_or_status().write(context, 1);
            }
            let text = rom_symbol(revision, "GameText_01B310")
                .expect("source speech symbol")
                .address;
            call_from(
                context,
                ids::PLAYER_YOUCH,
                ids::GENERATE_SPEECH,
                &[number as u32, text, 1],
                0x1e0,
            )?;
            let sample = if number == 0 {
                rom::sfx_toejam_youch(revision).address()
            } else {
                rom::sfx_earl_youch(revision).address()
            };
            call_from_void(
                context,
                ids::PLAYER_YOUCH,
                ids::PLAY_SOUND_EFFECT,
                &[sample, 50, 0],
                0x208,
            )?;
            Ok(Some(0))
        },
    )
}

/// G/src/player/earthling_damage_reactions.c:177 (`HulaAbility`).
pub(super) fn hula_ability(context: &mut dyn Context) -> Result<Option<u32>> {
    // c_earthling_damage_reactions.s:309-477. The first speech argument
    // area stays live while the following PlaySoundEffect arguments are pushed.
    with_frame(
        context,
        FrameLayout::link(4, 0x7c, 0x1c),
        |context, stack| {
            let number = i32::from(stack.argument_word(0).read(context));
            let earthling = Address::<EarthlingState>::new(stack.argument_slot(1).read(context));
            let revision = context.revision();
            let player = super::indexing::player_by_word_index(revision, number);
            let level = player.level().read(context);
            if level == 0 {
                return Ok(None);
            }
            let mut sprite = -1_i8;
            let eligible = call_from(
                context,
                ids::HULA_ABILITY,
                ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
                &[player.address(), number as u32],
                0x48,
            )? != 0
                && read_long(context, player.animation_group_mask().address()) & 0x2000_0010 == 0;
            let mut in_range = false;
            let mut dx = 0_i16;
            let mut dy = 0_i16;
            if eligible {
                let dancer =
                    ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
                let difference =
                    i32::from(dancer.x().read(context)) - i32::from(player.x().read(context));
                dx = call_from(
                    context,
                    ids::HULA_ABILITY,
                    ids::ABS,
                    &[difference as u32],
                    0x82,
                )? as i16;
                let difference =
                    i32::from(dancer.y().read(context)) - i32::from(player.y().read(context));
                dy = call_from(
                    context,
                    ids::HULA_ABILITY,
                    ids::ABS,
                    &[difference as u32],
                    0x9e,
                )? as i16;
                in_range = dx <= 100 && dy <= 75;
            }
            if eligible && in_range {
                let delay = ram::g_player_hula_contact_delay(revision).at(number);
                let previous = delay.read(context);
                let (stored, remaining) = super::counters::predecrement_byte(previous);
                delay.write(context, stored);
                if remaining < 0 {
                    dy = dy.wrapping_add(dy / 3);
                    let distance = if dx > dy { dx } else { dy };
                    let index = advance_random_index(context);
                    if ram::g_cached_random_values(revision)
                        .at(i32::from(index))
                        .read(context)
                        & 3
                        == 3
                    {
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
                                let speech =
                                    OutgoingArguments::push(context, &[number as u32, text, 1])?;
                                let dispatcher = context.dispatcher();
                                speech.call(
                                    context,
                                    dispatcher,
                                    ids::GENERATE_SPEECH,
                                    CallSite::relative(ids::HULA_ABILITY, revision, 0x152),
                                )?;
                                call_from_void(
                                    context,
                                    ids::HULA_ABILITY,
                                    ids::PLAY_SOUND_EFFECT,
                                    &[rom::sfx_aloha(revision).address(), 50, 1],
                                    0x166,
                                )?;
                                speech.release(context)?;
                                cooldown.write(context, 128_u8 as i8);
                            } else {
                                cooldown.write(context, cooldown.read(context).wrapping_sub(1));
                            }
                        }
                    }
                    let next_delay = (i32::from(distance) >> 5) + 4;
                    delay.write(context, next_delay as i8);
                }
            } else if player.sprite_state().read(context) == 89 {
                sprite = call_from(
                    context,
                    ids::HULA_ABILITY,
                    ids::GET_ACTIVE_PRESENT_SPRITE,
                    &[player.address()],
                    0x1a0,
                )? as i8;
                super::frames::clear_byte(
                    context,
                    ram::g_player_hula_contact_delay(revision)
                        .at(number)
                        .address(),
                )?;
            }
            if sprite != -1 {
                let current = player.sprite_state().read(context);
                if sprite != current {
                    player.animation_motion_axes().write(context, 3);
                    call_from_void(
                        context,
                        ids::HULA_ABILITY,
                        ids::SET_SPRITE,
                        &[player.address(), sprite as u32],
                        0x1d0,
                    )?;
                }
            }
            Ok(None)
        },
    )
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
    call_void(
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
    call_void(
        context,
        ids::SET_EARTHLING_STATE,
        &[earthling.address(), next],
    )?;
    call_void(
        context,
        ids::PLAY_SOUND_EFFECT,
        &[rom::sfx_boogie_boogie(revision).address(), 50, 0],
    )?;
    Ok(None)
}

/// G/src/player/earthling_damage_reactions.c:347 (`PlayerUpdateWhirlwindAndRecovery`).
pub(super) fn player_update_whirlwind_and_recovery(
    context: &mut dyn Context,
) -> Result<Option<u32>> {
    // c_earthling_damage_reactions.s:750-1002: source-specific RNG probes
    // read a word for recovery speech/duration, but a byte for direction.
    with_frame(
        context,
        FrameLayout::movem(0, 0x7c, 0x1c),
        |context, stack| {
            let player = Address::<ObjectState>::new(stack.argument_slot(0).read(context));
            let terrain = stack.argument_word(1).cast::<u16>().read(context);
            let revision = context.revision();
            let number = i32::from(player.entity_index().read(context));
            if player.sprite_state().read(context) == 78 {
                let timer = ram::g_player_whirlwind_recovery_timers(revision).at(number);
                let previous = timer.read(context);
                let (stored, remaining) = super::counters::predecrement_byte(previous);
                timer.write(context, stored);
                if remaining <= 0 {
                    let sprite;
                    if ram::g_player_powerups(revision).at(number).read(context) == 5 {
                        sprite = call_from(
                            context,
                            ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY,
                            ids::GET_ACTIVE_PRESENT_SPRITE,
                            &[player.address()],
                            0x58,
                        )? as i8;
                    } else if ram::g_player_powerups(revision).at(number).read(context) == 0 {
                        sprite = 61;
                        player.velocity_height().write(context, -1);
                        super::frames::clear_byte(
                            context,
                            player.animation_motion_axes().address(),
                        )?;
                    } else if terrain >= 0x80 {
                        if terrain == 0x90 {
                            call_from_void(
                                context,
                                ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY,
                                ids::PLAYER_START_FALLING,
                                &[player.address(), number as u32],
                                0x9a,
                            )?;
                            sprite = -1;
                        } else {
                            sprite = 65;
                        }
                    } else {
                        sprite = call_from(
                            context,
                            ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY,
                            ids::GET_ACTIVE_PRESENT_SPRITE,
                            &[player.address()],
                            0xaa,
                        )? as i8;
                        player.animation_motion_axes().write(context, 3);
                        let index = advance_random_index(context);
                        if ram::g_cached_random_values(revision)
                            .at(i32::from(index))
                            .read(context)
                            & 3
                            == 1
                        {
                            let text = rom_symbol(revision, "GameText_01BB56")
                                .expect("source speech symbol")
                                .address;
                            call_from(
                                context,
                                ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY,
                                ids::GENERATE_SPEECH,
                                &[number as u32, text, 1],
                                0x100,
                            )?;
                        }
                    }
                    if sprite != -1 {
                        call_from_void(
                            context,
                            ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY,
                            ids::SET_SPRITE,
                            &[player.address(), sprite as u32],
                            0x11c,
                        )?;
                    }
                } else {
                    let bits = timer.read(context) as u8 & 15;
                    if bits == 0 {
                        let previous = player.frame_phase().read(context);
                        let next = previous.wrapping_add(1);
                        player.frame_phase().write(context, next);
                    }
                }
            } else {
                let current = player.sprite_state().read(context);
                if current != 79 {
                    return Ok(None);
                }
                let timer = ram::g_player_whirlwind_timers(revision).at(number);
                let previous = timer.read(context);
                let (stored, remaining) = super::counters::predecrement_word(previous);
                timer.write(context, stored);
                if remaining <= 0 {
                    clear_word(context, player.velocity_x().address())?;
                    clear_word(context, player.velocity_y().address())?;
                    call_from_void(
                        context,
                        ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY,
                        ids::SET_SPRITE,
                        &[player.address(), 78],
                        0x170,
                    )?;
                    ram::g_player_whirlwind_recovery_timers(revision)
                        .at(number)
                        .write(context, 48);
                    let draw = player.draw_flags().read(context) & !0x40;
                    player.draw_flags().write(context, draw);
                } else {
                    let timer = ram::g_player_whirlwind_direction_timers(revision).at(number);
                    let (stored, remaining) =
                        super::counters::predecrement_byte(timer.read(context));
                    timer.write(context, stored);
                    if remaining <= 0 {
                        let index = advance_random_index(context);
                        let direction = tje_mem::Address::<i8>::new(
                            ram::g_cached_random_values(revision)
                                .at(i32::from(index))
                                .address()
                                + 1,
                        )
                        .read(context)
                            & 7;
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
                        let index = advance_random_index(context);
                        let duration = 20
                            + ram::g_cached_random_values(revision)
                                .at(i32::from(index))
                                .read(context)
                                % 6;
                        timer.write(context, duration as i8);
                    }
                    call_from_void(
                        context,
                        ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY,
                        ids::ENTITY_UPDATE_TERRAIN_DRAW_FLAG,
                        &[player.address()],
                        0x294,
                    )?;
                }
            }
            Ok(None)
        },
    )
}
