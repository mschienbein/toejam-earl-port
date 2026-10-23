use crate::{Context, Result, call, ids};
use tje_mem::{Address, ObjectState, Revision, ram};

/// G/src/presents/present_dispatch.c:117 (`SetupSelectedPresent`).
/// TimeAware: the action-icon upload and speech paths require graphics IRQ progress.
/// REV 02's fall-state guard and additional hitops refresh mask remain explicit.
pub fn setup_selected_present(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = Address::<ObjectState>::new(arguments[0]);
    let index = i32::from(arguments[1] as i16);
    let present = arguments[2] as i16;
    let terrain = arguments[3] as u16;
    if present < 8 && present != 5 && player.height().read(context) > 0 {
        if ram::g_player_powerups(revision).at(index).read(context) == 0
            && player.animation_group_mask().read(context) & 0x8000 != 0
            && present != 0
        {
            call(
                context,
                ids::PRESENT_TRY_END_ACTIVE_POWERUP,
                &[index as u32],
            )?;
        } else if ram::g_player_powerups(revision).at(index).read(context) == 1
            && ram::g_player_fall_states(revision).at(index).read(context) == 0
            && player.animation_group_mask().read(context) & 0x20 != 0
            && present != 1
        {
            call(context, ids::STOP_PLAYER, &[player.address()])?;
            player.acceleration_height().write(context, -32);
            ram::g_player_landing_counter_seed(revision)
                .at(index)
                .write(context, 2);
        }
    }
    let points = ram::g_player_points(revision).at(index);
    points.write(context, points.read(context).wrapping_add(2));
    if present >= 8 {
        return Ok(None);
    }
    ram::g_player_powerups(revision)
        .at(index)
        .write(context, present as i8);
    call(context, ids::HUD_LOAD_ACTION_ICON_SPRITE, &[index as u32])?;
    let timer = ram::g_player_powerup_timers(revision).at(index);
    timer.write(context, 1000);
    match present {
        5 => {
            timer.write(context, timer.read(context) >> 2);
            if revision == Revision::Rev00
                || ram::g_player_fall_states(revision).at(index).read(context) == 0
            {
                call(context, ids::PLAY_SOUND, &[5, 94])?;
                call(
                    context,
                    ids::OPEN_ROCKET_SKATES,
                    &[player.address(), index as u32, u32::from(terrain)],
                )?;
            }
        }
        0 => {
            timer.write(context, timer.read(context) >> 1);
            call(
                context,
                ids::PLAYER_INITIALIZE_PHASED_JUMP_MOTION,
                &[index as u32],
            )?;
        }
        3 => {
            call(context, ids::START_DIALOGUE, &[26, index as u32])?;
        }
        4 => {
            call(context, ids::START_DIALOGUE, &[27, index as u32])?;
        }
        6 => {
            let text = tje_mem::rom_symbol(revision, "RosebushesText")
                .expect("catalogued present text")
                .address;
            call(context, ids::GENERATE_SPEECH, &[index as u32, text, 1])?;
        }
        7 => {
            timer.write(context, 750);
            call(context, ids::START_DIALOGUE, &[28, index as u32])?;
        }
        _ => {}
    }
    let sprite = if present != 5 {
        call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])? as i8
    } else {
        -1
    };
    let groups = player.animation_group_mask().read(context);
    let refresh = if revision == Revision::Rev02 {
        0x0004c81c | 0x8000000
    } else {
        0x0004c81c
    };
    if sprite != -1
        && groups & 0x20000000 == 0
        && (matches!(player.sprite_state().read(context), 0 | 4 | 76 | 3 | 7)
            || groups & (0x400000 | 0x20000000 | refresh) != 0)
    {
        call(context, ids::SET_SPRITE, &[player.address(), sprite as u32])?;
        player.velocity_x().write(context, 0);
        player.velocity_y().write(context, 0);
    }
    Ok(None)
}

/// G/src/presents/present_dispatch.c:41 (`OpenSelectedPresent`).
/// TimeAware: accepted effects can call speech, dialogue and the HUD upload wait.
pub fn open_selected_present(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let mut present = arguments[1] as i16;
    let player = super::word_indexed_player(revision, index);
    if ram::g_player_elevator_flags(revision)
        .at(index)
        .read(context)
        != 0
        || matches!(player.sprite_state().read(context), 5 | 6)
    {
        return Ok(Some(1));
    }
    let terrain = u16::from(if index == 0 {
        ram::g_player1_terrain_underfoot(revision).read(context)
    } else {
        ram::g_player2_terrain_underfoot(revision).read(context)
    });
    match present {
        27 => present = 7,
        15 => {
            if terrain >= 0x80 {
                return Ok(Some(1));
            }
            call(context, ids::OPEN_DECOY, &[index as u32])?;
        }
        8 => {
            call(context, ids::PRESENT_IS_DOORWAY, &[index as u32])?;
        }
        17 | 16 => {
            call(
                context,
                ids::PERFORM_PROMOTION,
                &[index as u32, present as u32, u32::from(terrain)],
            )?;
        }
        11 => {
            let rank = ram::g_player_rank(revision).at(index).read(context);
            let score = call(context, ids::GET_SCORE_FOR_PROMO, &[rank as u32])?;
            ram::g_player_points(revision)
                .at(index)
                .write(context, score.wrapping_sub(2) as u16);
        }
        9 => {
            let food = if (call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 100 < 70 {
                ids::EAT_YUMMY_FOOD
            } else {
                ids::EAT_YUCKY_FOOD
            };
            call(context, food, &[index as u32])?;
        }
        10 => {
            ram::g_player_burp_timers(revision)
                .at(index)
                .write(context, -1);
            let extra = (call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 7 + 12;
            let burps = ram::g_player_burps_left(revision).at(index);
            burps.write(context, burps.read(context).wrapping_add(extra as i8));
            let health = ram::g_player_health(revision).at(index);
            health.write(context, health.read(context).wrapping_add(4));
        }
        22 => {
            call(context, ids::SET_TOMATO_RAIN_TOMATOES, &[index as u32])?;
            let text = tje_mem::rom_symbol(revision, "GameText_01724A")
                .expect("catalogued rain text")
                .address;
            call(context, ids::GENERATE_SPEECH, &[index as u32, text, 1])?;
            if matches!(player.sprite_state().read(context), 0 | 4 | 76)
                || player.animation_group_mask().read(context)
                    & (0x400000 | 0x20000000 | 0x0830400c)
                    != 0
            {
                call(context, ids::SET_SPRITE, &[player.address(), 1])?;
                call(context, ids::STOP_PLAYER, &[player.address()])?;
                player.animation_motion_axes().write(context, 0);
            }
        }
        12 => {
            if ram::g_pending_unfall_player(revision).read(context) != 0 {
                return Ok(Some(1));
            }
            ram::g_pending_unfall_player(revision).write(context, (index + 1) as i8);
        }
        13 => {
            call(context, ids::PRESENT_IS_RAIN_CLOUD, &[index as u32])?;
        }
        14 => {
            call(context, ids::EAT_FUDGE_SUNDAE, &[index as u32])?;
        }
        19 => {
            if terrain >= 0x80 {
                return Ok(Some(1));
            }
            call(context, ids::OPEN_TELEPHONE, &[index as u32])?;
        }
        20 | 21 => {
            let bucks = ram::g_player_bucks(revision).at(index);
            bucks.write(
                context,
                bucks
                    .read(context)
                    .wrapping_add(if present == 20 { 1 } else { 5 }),
            );
            call(
                context,
                ids::START_DIALOGUE,
                &[if present == 20 { 29 } else { 30 }, index as u32],
            )?;
        }
        23 => {
            call(
                context,
                ids::OPEN_EARTHLING_PRESENT,
                &[player.address(), u32::from(terrain)],
            )?;
        }
        24 => {
            if terrain >= 0x80 || player.height().read(context) > 0 {
                return Ok(Some(1));
            }
            call(context, ids::OPEN_SCHOOL_BOOK, &[player.address()])?;
        }
        25 => {
            if terrain >= 0x80 {
                return Ok(Some(1));
            }
            call(context, ids::OPEN_BOOM_BOX, &[index as u32])?;
        }
        1 if player.level().read(context) == 0 => return Ok(Some(1)),
        _ => {}
    }
    call(
        context,
        ids::SETUP_SELECTED_PRESENT,
        &[
            player.address(),
            index as u32,
            present as u32,
            u32::from(terrain),
        ],
    )?;
    Ok(Some(0))
}

/// G/src/presents/present_dispatch.c:23 (`ProcessPresents`).
/// TimeAware: doorway, speech and telephone map-input paths can suspend.
pub fn process_presents(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    for index in 0..2 {
        for function in [
            ids::PRESENT_UPDATE_DECOY,
            ids::PRESENT_UPDATE_BOOM_BOX,
            ids::PRESENT_UPDATE_DOORWAY,
            ids::PRESENT_UPDATE_BURPS,
            ids::PRESENT_UPDATE_RAIN_CLOUD,
            ids::PRESENT_UPDATE_TRANSIENT_EFFECT,
            ids::ENTITY_RELEASE_FINISHED_TIMED_EFFECT,
            ids::CHECK_FOR_TELEPHONE,
            ids::CHECK_TOMATO_RAIN_TIMER,
        ] {
            call(context, function, &[index])?;
        }
    }
    Ok(None)
}
