use crate::{Context, Result, call, ids};
use tje_mem::{Address, EarthlingState, ObjectState, ram, rom, rom_symbol};

/// G/src/player/ability_helpers.c:61 (`LemonadeAbility`).
pub(super) fn lemonade_ability(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let number = i32::from(arguments[0] as i16);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    let revision = context.revision();
    let player = super::indexing::player_by_word_index(revision, number);
    if earthling.state().read(context) == 1
        && ram::g_player_info(revision)
            .at(i32::from(earthling.object().read(context)))
            .frame_duration()
            .read(context)
            == 0
    {
        call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 0])?;
    }
    let reward = ram::g_player_lemonade_reward_states(revision).at(number);
    if reward.read(context) == 1 && ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0 {
        call(context, ids::OPEN_SELECTED_PRESENT, &[number as u32, 17])?;
        reward.write(context, 2);
    }
    if player.sprite_state().read(context) == 118 && player.frame_duration().read(context) == 0 {
        let sprite = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])?;
        call(context, ids::SET_SPRITE, &[player.address(), sprite])?;
        let text = rom_symbol(revision, "GameText_023E04")
            .expect("source speech symbol")
            .address;
        call(context, ids::GENERATE_SPEECH, &[number as u32, text, 1])?;
        call(
            context,
            ids::PLAY_SOUND_EFFECT,
            &[rom::sfx_burp(revision).address(), 50, 0],
        )?;
        if reward.read(context) == 0 {
            reward.write(context, 1);
        }
    } else if player.sprite_state().read(context) == 8
        && player.y().read(context) > earthling.y().read(context)
    {
        call(context, ids::SET_SPRITE, &[player.address(), 118])?;
        call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 1])?;
    }
    Ok(None)
}

/// G/src/player/active_present_falling_and_respawn.c:811 (`PlayerAdvanceToNextLevel`).
pub(super) fn player_advance_to_next_level(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let number = i32::from(arguments[1] as i16);
    let revision = context.revision();
    let args = [
        number as u32,
        player.x().read(context) as u32,
        (i32::from(player.y().read(context)) + 1) as u32,
        (i32::from(player.height().read(context)) + 9) as u32,
        player.level().read(context) as u32,
    ];
    call(context, ids::PRESENT_START_TRANSIENT_EFFECT, &args)?;
    let handle = ram::g_player_transient_effect_entities(revision).at(number);
    let effect = ram::g_player_info(revision).at(i32::from(handle.read(context)));
    let entity = handle.read(context) as u32;
    call(context, ids::UPDATE_ENTITY_VIEWPORT_SPRITES, &[entity])?;
    if effect.draw_flags().read(context) & 2 != 0 {
        while handle.read(context) != 0 {
            if call(
                context,
                ids::PRESENT_UPDATE_TRANSIENT_EFFECT,
                &[number as u32],
            )? != 0
            {
                player
                    .behavior_flags()
                    .write(context, player.behavior_flags().read(context) | 0x40);
            }
            call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[2])?;
            call(context, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[])?;
        }
        player
            .behavior_flags()
            .write(context, player.behavior_flags().read(context) & !0x40);
    }
    let level = player.level().read(context) as u32;
    call(context, ids::EARTHLING_RELEASE_LEVEL_BANK, &[level])?;
    let level = player.level().read(context) as u32;
    call(context, ids::MAP_RELEASE_LEVEL_ITEM_BANK, &[level])?;
    player
        .level()
        .write(context, player.level().read(context).wrapping_add(1));
    let level = player.level().read(context) as u32;
    call(context, ids::MAP_LOAD_PLAYER_LEVEL, &[0, level])?;
    let level = player.level().read(context) as u32;
    call(context, ids::SETUP_ELEVATOR, &[level, 0])?;
    let level = player.level().read(context) as u32;
    call(context, ids::EARTHLING_RETAIN_LEVEL_BANK, &[level])?;
    let level = player.level().read(context) as u32;
    call(context, ids::LOAD_LEVEL_ITEM_BANK, &[level])?;
    if player.level().read(context) > 1 {
        call(
            context,
            ids::PLAYER_SET_POSITION_FROM_SAVED_MAP_CELL,
            &[player.address()],
        )?;
    } else {
        call(context, ids::SET_POSITION_ON_NEW_LEVEL, &[number as u32])?;
    }
    player.height().write(context, 70);
    Ok(None)
}

/// G/src/player/active_present_falling_and_respawn.c:844 (`PlayerRelocateToPartnerOrNextLevel`).
pub(super) fn player_relocate_to_partner_or_next_level(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let number = i32::from(arguments[0] as i16);
    let revision = context.revision();
    let player = super::indexing::player_by_word_index(revision, number);
    ram::g_player_terrain_sinking(revision)
        .at(number)
        .write(context, 0);
    let alone = ram::g_player_lives(revision).at(number ^ 1).read(context) < 0;
    let advance =
        alone && player.level().read(context) != ram::g_highest_level(revision).read(context);
    if advance {
        call(
            context,
            ids::PLAYER_ADVANCE_TO_NEXT_LEVEL,
            &[player.address(), number as u32],
        )?;
    } else {
        let other = if alone {
            player
        } else {
            ram::g_player_info(revision).at(number ^ 1)
        };
        player.x().write(context, other.x().read(context));
        player.y().write(context, other.y().read(context));
        if other.animation_group_mask().read(context) & 0x2000_0000 != 0 {
            player
                .y()
                .write(context, player.y().read(context).wrapping_add(10));
        }
        if ram::g_elevator_departure_in_progress(revision).read(context) == 0 {
            if ram::g_player_elevator_flags(revision)
                .at(number ^ 1)
                .read(context)
                != 0
            {
                player
                    .y()
                    .write(context, player.y().read(context).wrapping_add(15));
            }
            player.height().write(context, 70);
        } else {
            player.height().write(context, 0);
        }
        if ram::g_split_screen_enabled(revision).read(context) != 0
            && player.level().read(context) != other.level().read(context)
        {
            let level = player.level().read(context) as u32;
            call(context, ids::EARTHLING_RELEASE_LEVEL_BANK, &[level])?;
            let level = player.level().read(context) as u32;
            call(context, ids::MAP_RELEASE_LEVEL_ITEM_BANK, &[level])?;
            player.level().write(context, other.level().read(context));
            let source = ram::g_player_info(revision).at(if number == 0 { 0 } else { 1 });
            let level = source.level().read(context) as u32;
            call(
                context,
                ids::MAP_LOAD_PLAYER_LEVEL,
                &[if number == 0 { 0 } else { 1 }, level],
            )?;
            let level = source.level().read(context) as u32;
            call(context, ids::SETUP_ELEVATOR, &[level, 0])?;
            let level = source.level().read(context) as u32;
            call(context, ids::EARTHLING_RETAIN_LEVEL_BANK, &[level])?;
            let level = source.level().read(context) as u32;
            call(context, ids::LOAD_LEVEL_ITEM_BANK, &[level])?;
        }
    }
    call(context, ids::WAIT_FOR_V_BLANK_WITH_GRAPHICS_PAUSED, &[])?;
    call(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    mode.write(context, mode.read(context) & 0x3c);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    call(
        context,
        ids::HUD_LOAD_LEVEL_NUMBER_SPRITES,
        &[number as u32],
    )?;
    call(context, ids::UPDATE_SCREEN_SPLIT_MODE, &[1])?;
    mode.write(context, mode.read(context) | 0x40);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    call(context, ids::ENABLE_BLANKING_GRAPHICS, &[])?;
    call(context, ids::UPDATE_LAND_AROUND_PLAYER, &[number as u32])?;
    if ram::g_elevator_departure_in_progress(revision).read(context) == 0 {
        call(context, ids::SET_SPRITE, &[player.address(), 21])?;
        player.velocity_x().write(context, 0);
        player.velocity_y().write(context, 0);
        player.velocity_height().write(context, -128);
        player.acceleration_height().write(context, -16);
        player
            .draw_flags()
            .write(context, player.draw_flags().read(context) | 0x40);
        let args = [
            0,
            player.x().read(context) as u32,
            player.y().read(context) as u32,
        ];
        let terrain = call(context, ids::GET_MAP_TERRAIN_AT_WORLD_POSITION, &args)? as u8;
        if terrain != 0x90 && terrain != 0x80 {
            ram::g_player_fall_states(revision)
                .at(number)
                .write(context, 2);
            ram::g_player_jump_counters(revision)
                .at(number)
                .write(context, 3);
        } else {
            player
                .y()
                .write(context, player.y().read(context).wrapping_add(20));
        }
        let args = [
            number as u32,
            player.x().read(context) as u32,
            (i32::from(player.y().read(context)) + 1) as u32,
            player.height().read(context) as u32,
            player.level().read(context) as u32,
        ];
        call(context, ids::PRESENT_START_TRANSIENT_EFFECT, &args)?;
    } else {
        call(context, ids::STOP_PLAYER, &[player.address()])?;
        ram::g_player_elevator_flags(revision).at(number).write(
            context,
            ram::g_player_elevator_flags(revision)
                .at(number ^ 1)
                .read(context),
        );
        player
            .behavior_flags()
            .write(context, player.behavior_flags().read(context) | 0x40);
        call(context, ids::SET_SPRITE, &[player.address(), 0])?;
    }
    Ok(None)
}

/// G/src/player/active_present_falling_and_respawn.c:407 (`MakePlayerFall`).
/// The level pre-decrement consumes SUBQ.B's overflow flag before BGE.
pub(super) fn make_player_fall(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let falling = Address::<i8>::new(arguments[0]);
    let revision = context.revision();
    if !((falling.read(context) != 0 && falling.offset(1).read(context) != 0)
        || (falling.read(context) != 0
            && ram::g_player_fall_states(revision).at(1).read(context) == 0)
        || (falling.offset(1).read(context) != 0
            && ram::g_player_fall_states(revision).at(0).read(context) == 0))
    {
        return Ok(None);
    }
    for number in 0..2 {
        let player = ram::g_player_info(revision).at(number);
        if falling.offset(number).read(context) != 0 {
            player
                .tile()
                .write(context, player.tile().read(context) | 0x8000);
            player.height().write(context, 600);
            player.velocity_height().write(context, -640);
            player.acceleration_height().write(context, -16);
            player
                .draw_flags()
                .write(context, player.draw_flags().read(context) | 0x40);
            let level = player.level().read(context) as u32;
            call(context, ids::EARTHLING_RELEASE_LEVEL_BANK, &[level])?;
            let level = player.level().read(context) as u32;
            call(context, ids::MAP_RELEASE_LEVEL_ITEM_BANK, &[level])?;
            let (stored, remaining) =
                super::counters::predecrement_byte(player.level().read(context));
            player.level().write(context, stored);
            if remaining < 0 {
                player
                    .level()
                    .write(context, ram::g_highest_level(revision).read(context));
            }
            ram::g_player_fall_states(revision)
                .at(number)
                .write(context, 2);
        }
        if ram::g_player_lives(revision).at(number).read(context) < 0 {
            call(context, ids::ELEVATOR_INVALIDATE_SECONDARY_RECORDS, &[])?;
        }
    }
    for number in 0..2 {
        if falling.offset(number).read(context) != 0 {
            let player = ram::g_player_info(revision).at(number);
            let level = player.level().read(context) as u32;
            call(context, ids::MAP_LOAD_PLAYER_LEVEL, &[number as u32, level])?;
            let level = player.level().read(context) as u32;
            call(context, ids::SETUP_ELEVATOR, &[level, 0])?;
            let level = player.level().read(context) as u32;
            call(context, ids::EARTHLING_RETAIN_LEVEL_BANK, &[level])?;
            let level = player.level().read(context) as u32;
            call(context, ids::LOAD_LEVEL_ITEM_BANK, &[level])?;
        }
    }
    call(context, ids::DISABLE_BLANKING_GRAPHICS, &[])?;
    let mode = ram::g_mode2_reg_softcopy(revision);
    mode.write(context, mode.read(context) & 0x3c);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    let first = ram::g_player_info(revision).at(0);
    let second = ram::g_player_info(revision).at(1);
    if first.level().read(context) == second.level().read(context)
        || ram::g_player_lives(revision).at(0).read(context) < 0
        || ram::g_player2_lives(revision).read(context) < 0
    {
        if falling.read(context) != 0 && falling.offset(1).read(context) != 0 {
            call(context, ids::SET_POSITION_ON_NEW_LEVEL, &[2])?;
        } else {
            let (number, other) = if falling.read(context) != 0 {
                (0, second)
            } else {
                (1, first)
            };
            let terrain = if number == 0 {
                ram::g_player2_terrain_underfoot(revision).read(context)
            } else {
                ram::g_player1_terrain_underfoot(revision).read(context)
            };
            if terrain >= 0x80
                || other.animation_group_mask().read(context) & 0x2000_0000 != 0
                || matches!(other.sprite_state().read(context), 5 | 6)
                || ram::g_player_lives(revision).at(number ^ 1).read(context) < 0
            {
                call(context, ids::SET_POSITION_ON_NEW_LEVEL, &[number as u32])?;
            } else {
                let player = ram::g_player_info(revision).at(number);
                player.x().write(context, other.x().read(context));
                player.y().write(context, other.y().read(context));
                if ram::g_player_elevator_flags(revision)
                    .at(number ^ 1)
                    .read(context)
                    != 0
                {
                    player
                        .y()
                        .write(context, player.y().read(context).wrapping_add(12));
                }
            }
        }
    } else {
        for number in 0..2 {
            if falling.offset(number).read(context) != 0 {
                call(context, ids::SET_POSITION_ON_NEW_LEVEL, &[number as u32])?;
            }
        }
    }
    call(context, ids::UPDATE_SCREEN_SPLIT_MODE, &[1])?;
    call(context, ids::HUD_LOAD_LEVEL_NUMBER_SPRITES, &[0])?;
    call(context, ids::HUD_POSITION_LEVEL_NUMBER_SPRITES, &[0])?;
    call(context, ids::HUD_LOAD_LEVEL_NUMBER_SPRITES, &[1])?;
    call(context, ids::HUD_POSITION_LEVEL_NUMBER_SPRITES, &[1])?;
    context.write_vdp_control(0x8c89);
    mode.write(context, mode.read(context) | 0x40);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    call(context, ids::ENABLE_BLANKING_GRAPHICS, &[])?;
    Ok(None)
}
