use crate::{Context, Result, call, ids};
use tje_mem::{Address, ObjectState, ram, rom, rom_symbol};

fn speech_text(context: &dyn Context, symbol: &str) -> u32 {
    rom_symbol(context.revision(), symbol)
        .expect("source-named speech symbol")
        .address
}

/// G/src/player/active_present_falling_and_respawn.c:134 (`PlayerStartFalling`).
/// TimeAware: the real VBlank handler must service the terrain-priority request.
pub(super) fn player_start_falling(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let number = i32::from(arguments[1] as i16);
    let revision = context.revision();
    let nearby = if number == 0 {
        ram::g_player1_nearby_terrain(revision)
    } else {
        ram::g_player2_nearby_terrain(revision)
    }
    .cast::<u8>();
    ram::g_player_fall_states(revision)
        .at(number)
        .write(context, 1);
    let sprite = (player.sprite_state().read(context) & 3) + 20;
    if matches!(sprite, 20 | 21) {
        if nearby.offset(11).read(context) < 0x80 {
            player
                .x()
                .write(context, player.x().read(context).wrapping_add(8));
        } else if nearby.offset(13).read(context) < 0x80 {
            player
                .x()
                .write(context, player.x().read(context).wrapping_sub(8));
        }
    }
    player.acceleration_height().write(context, -128);
    call(context, ids::SET_SPRITE, &[player.address(), sprite as u32])?;
    super::frames::clear_byte(context, player.animation_motion_axes().address())?;
    player
        .tile()
        .write(context, player.tile().read(context) & !0x8000);
    ram::g_terrain_priority_update_player(revision).write(context, number as i16);
    let pending = ram::g_terrain_priority_update_pending(revision);
    pending.write(context, 1);
    // Matching compiler lines287-288: TST.W/BNE until the complete word is zero.
    // The predicate owns every poll, including the satisfying zero read.
    context.wait_until(crate::execution::WaitRequest::ram_word_equal(
        pending.address(),
        u16::MAX,
        0,
    ))?;
    let text = speech_text(context, "GameText_00F8D6");
    call(context, ids::GENERATE_SPEECH, &[number as u32, text, 0])?;
    call(
        context,
        ids::PLAY_SOUND_EFFECT,
        &[rom::sfx_falling(revision).address(), 50, 0],
    )?;
    Ok(None)
}

/// G/src/player/active_present_falling_and_respawn.c:580 (`PlayerUpdateTerrainSinking`).
/// Preserve the original matching byte store of 256, which truncates to zero.
pub(super) fn player_update_terrain_sinking(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let land = arguments[1] as u16;
    let revision = context.revision();
    let number = i32::from(player.entity_index().read(context));
    if ram::g_player_interaction_choice_active(revision)
        .at(number)
        .read(context)
        != 0
        || super::frames::read_long(context, player.animation_group_mask().address()) & 0x20000 != 0
        // Matching compiler lines1179-1185 keep all four short-circuit loads.
        || player.sprite_state().read(context) == 79
        || player.sprite_state().read(context) == 78
        || player.sprite_state().read(context) == 5
        || player.sprite_state().read(context) == 6
    {
        return Ok(None);
    }
    let previous_height = player.height().read(context);
    let sinking = ram::g_player_terrain_sinking(revision).at(number);
    if land == 0x20
        && player.height().read(context) <= 0
        && super::frames::read_long(context, player.animation_group_mask().address()) & 0x60 == 0
        && ram::g_player_elevator_flags(revision)
            .at(number)
            .read(context)
            == 0
    {
        if sinking.read(context) == 0 {
            sinking.write(context, 1);
            player.height().write(context, -3);
        }
        player
            .draw_flags()
            .write(context, player.draw_flags().read(context) | 0x40);
        if ram::g_frame_counter(revision).read(context) & 60 == 0 {
            if (number == 0 && player.height().read(context) > -16)
                || (number == 1 && player.height().read(context) > -20)
            {
                player
                    .height()
                    .write(context, player.height().read(context).wrapping_sub(1));
            } else if ram::g_sinking_help_speech_delay(revision).read(context) == 0 {
                if ram::g_split_screen_enabled(revision).read(context) == 0
                    && ram::g_player_lives(revision).at(number ^ 1).read(context) >= 0
                    && sinking_help_random_bit(context) != 0
                {
                    let text = speech_text(context, "GameText_010448");
                    call(context, ids::GENERATE_SPEECH, &[number as u32, text, 1])?;
                    ram::g_sinking_help_speech_delay(revision).write(context, 0);
                }
            } else {
                let delay = ram::g_sinking_help_speech_delay(revision);
                delay.write(context, delay.read(context).wrapping_sub(1));
            }
        }
        let other = ram::g_player_info(revision).at(number ^ 1);
        if call(
            context,
            ids::ENTITY_IS_WITHIN_RANGE,
            &[player.address(), other.address(), 18, 7],
        )? != 0
        {
            player.height().write(context, -3);
        }
    } else if sinking.read(context) != 0 {
        super::frames::clear_word(context, player.height().address())?;
        super::frames::clear_byte(context, sinking.address())?;
        player
            .draw_flags()
            .write(context, player.draw_flags().read(context) & !0x40);
    }
    if previous_height != player.height().read(context) {
        super::frames::clear_byte(context, player.sprite_ready().address())?;
    }
    Ok(None)
}

/// G/src/player/active_present_falling_and_respawn.c:636 (`PlayerUpdateHurtRecoveryAndIdle`).
pub(super) fn player_update_hurt_recovery_and_idle(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let land = arguments[1] as u16;
    let revision = context.revision();
    let number = i32::from(player.entity_index().read(context));
    let nearby = if number == 0 {
        ram::g_player1_nearby_terrain(revision)
    } else {
        ram::g_player2_nearby_terrain(revision)
    }
    .cast::<u8>();
    if (super::frames::read_long(context, player.animation_group_mask().address()) & 0x2000 != 0
        || player.sprite_state().read(context) == 68)
        && player.frame_duration().read(context) == 0
    {
        player
            .draw_flags()
            .write(context, player.draw_flags().read(context) & !0x40);
        let health = ram::g_player_health(revision).at(number);
        // Matching compiler lines1343-1345 load damage before the health RMW.
        let damage = ram::g_player_pending_hurt_damage(revision)
            .at(number)
            .read(context);
        health.write(context, health.read(context).wrapping_sub(damage));
        let mut direction = player.sprite_state().read(context) & 3;
        if ram::g_player_powerups(revision).at(number).read(context) == 5 {
            call(
                context,
                ids::OPEN_ROCKET_SKATES,
                &[player.address(), number as u32, u32::from(land)],
            )?;
        } else if player.height().read(context) > 4 {
            call(context, ids::STOP_PLAYER, &[player.address()])?;
            super::frames::clear_byte(context, player.animation_motion_axes().address())?;
            player.acceleration_height().write(context, -32);
            ram::g_player_landing_counter_seed(revision)
                .at(number)
                .write(context, 2);
            if player.sprite_state().read(context) == 68 {
                direction = 1;
            }
            call(
                context,
                ids::SET_SPRITE,
                &[player.address(), (direction + 20) as u32],
            )?;
        } else if nearby.offset(12).read(context) < 0x80 {
            let sprite = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])?;
            call(context, ids::SET_SPRITE, &[player.address(), sprite])?;
        } else {
            if nearby.offset(11).read(context) < 0x80 {
                player
                    .x()
                    .write(context, player.x().read(context).wrapping_add(16));
            }
            if nearby.offset(13).read(context) < 0x80 {
                player
                    .x()
                    .write(context, player.x().read(context).wrapping_sub(16));
            }
            if nearby.offset(12).read(context) == 0x80 {
                if player.sprite_state().read(context) == 68 {
                    direction = 1;
                } else if direction & 2 != 0 {
                    direction ^= 1;
                }
                call(
                    context,
                    ids::SET_SPRITE,
                    &[player.address(), (direction + 64) as u32],
                )?;
                super::frames::clear_word(context, player.height().address())?;
                call(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_splash(revision).address(), 50, 0],
                )?;
            } else {
                if health.read(context) <= 0 {
                    health.write(context, 1);
                }
                call(context, ids::STOP_PLAYER, &[player.address()])?;
                call(
                    context,
                    ids::PLAYER_START_FALLING,
                    &[player.address(), number as u32],
                )?;
            }
        }
    }
    if number == 1 {
        if player.sprite_state().read(context) == 0 && player.frame().read(context) != 0 {
            if call(context, ids::GET_RANDOM_NUMBER, &[])? & 0x7ff == 1 {
                call(context, ids::SET_SPRITE, &[player.address(), 88])?;
            }
        } else if player.sprite_state().read(context) == 88
            && player.frame_duration().read(context) == 0
        {
            call(context, ids::SET_SPRITE, &[player.address(), 0])?;
            player.frame().write(context, 1);
        }
    }
    Ok(None)
}

/// G/src/player/active_present_falling_and_respawn.c:750 (`PlayerResolveDescendingTerrain`).
pub(super) fn player_resolve_descending_terrain(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let land = arguments[1] as u16;
    let revision = context.revision();
    let number = i32::from(player.entity_index().read(context));
    if ram::g_player_fall_states(revision).at(number).read(context) != 0 {
        return Ok(None);
    }
    // Matching compiler lines1627-1631: a byte BTST, then a separate long mask
    // load only when the byte test is clear. Do not combine these accesses.
    if player
        .animation_group_mask()
        .byte_offset(3)
        .cast::<u8>()
        .read(context)
        & 0x20
        != 0
        || super::frames::read_long(context, player.animation_group_mask().address()) & 0x1000_0000
            != 0
    {
        let mut sprite = -1_i8;
        let mut direction = player.sprite_state().read(context) & 3;
        player.draw_flags().write(
            context,
            if land == 0x90 {
                player.draw_flags().read(context) & !0x40
            } else {
                player.draw_flags().read(context) | 0x40
            },
        );
        if ram::g_player_powerups(revision).at(number).read(context) == 1
            && player.velocity_height().read(context) > 0
        {
            if player.height().read(context) > 60
                || ram::g_player_processed_input(revision)
                    .at(number)
                    .read(context)
                    & 0x40
                    == 0
            {
                super::frames::clear_word(context, player.velocity_height().address())?;
                player.acceleration_height().write(context, -64);
            }
        } else if player.height().read(context) < 4 {
            if land >= 0x80 {
                if land == 0x90 {
                    call(
                        context,
                        ids::PLAYER_START_FALLING,
                        &[player.address(), number as u32],
                    )?;
                } else {
                    super::frames::clear_word(context, player.height().address())?;
                    super::frames::clear_word(context, player.acceleration_height().address())?;
                    super::frames::clear_word(context, player.velocity_height().address())?;
                    if direction & 2 != 0 {
                        direction ^= 1;
                    }
                    sprite = direction + 64;
                    player
                        .draw_flags()
                        .write(context, player.draw_flags().read(context) & !0x40);
                    call(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_splash(revision).address(), 50, 0],
                    )?;
                }
            } else if ram::g_player_powerups(revision).at(number).read(context) == 7 {
                super::frames::clear_word(context, player.height().address())?;
                super::frames::clear_word(context, player.acceleration_height().address())?;
                super::frames::clear_word(context, player.velocity_height().address())?;
                sprite = if ram::g_player_processed_input(revision)
                    .at(number)
                    .read(context)
                    & 0xf
                    != 0
                {
                    direction + 108
                } else {
                    call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])? as i8
                };
                player
                    .draw_flags()
                    .write(context, player.draw_flags().read(context) & !0x40);
            } else {
                ram::g_player_jump_counters(revision).at(number).write(
                    context,
                    ram::g_player_landing_counter_seed(revision)
                        .at(number)
                        .read(context),
                );
                player.acceleration_height().write(context, -16);
                ram::g_player_fall_states(revision)
                    .at(number)
                    .write(context, 2);
            }
            if i32::from(sprite) & 0xfc != 108 {
                super::frames::clear_word(context, player.velocity_x().address())?;
                super::frames::clear_word(context, player.velocity_y().address())?;
            }
        }
        if sprite != -1 {
            call(context, ids::SET_SPRITE, &[player.address(), sprite as u32])?;
        }
    }
    Ok(None)
}

/// c_active_present_falling_and_respawn.s:1244-1259 in both revisions.
/// ADDQ.W is followed by a real index reload; BTST reads only the cache low byte.
pub(super) fn sinking_help_random_bit(context: &mut dyn Context) -> u8 {
    let revision = context.revision();
    let selected = super::frames::advance_random_index(context);
    ram::g_cached_random_values(revision)
        .at(i32::from(selected))
        .byte_offset(1)
        .cast::<u8>()
        .read(context)
        & 1
}
