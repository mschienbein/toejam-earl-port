use crate::{Context, Result, call, ids};
use tje_mem::{Address, ObjectState, ram};

/// G/src/player/sleep.c:21 (`OpenSchoolBook`).
pub(super) fn open_school_book(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let state = Address::<ObjectState>::new(arguments[0]);
    call(context, ids::SET_SPRITE, &[state.address(), 116])?;
    let player = i32::from(state.entity_index().read(context));
    ram::g_player_book_sleep_delay(context.revision())
        .at(player)
        .write(context, 60);
    call(context, ids::STOP_PLAYER, &[state.address()])?;
    state.animation_motion_axes().write(context, 0);
    state.height().write(context, 0);
    Ok(None)
}

/// G/src/player/active_present_falling_and_respawn.c:165 (`PlayerStartTeeteringFall`).
pub(super) fn player_start_teetering_fall(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let player = Address::<ObjectState>::new(arguments[0]);
    let number = i32::from(player.entity_index().read(context));
    player.height().write(context, 5);
    player.velocity_height().write(context, 512);
    player.acceleration_height().write(context, -80);
    player.animation_motion_axes().write(context, 0);
    let sprite = (player.sprite_state().read(context) & 3) + 20;
    ram::g_player_landing_counter_seed(context.revision())
        .at(number)
        .write(context, 1);
    player.velocity_x().write(context, 0);
    player.velocity_y().write(context, 0);
    match sprite {
        20 => player.velocity_y().write(context, -256),
        21 => player.velocity_y().write(context, 256),
        22 => player.velocity_x().write(context, -512),
        23 => player.velocity_x().write(context, 512),
        _ => unreachable!(),
    }
    call(context, ids::SET_SPRITE, &[player.address(), sprite as u32])?;
    Ok(None)
}

/// G/src/player/active_present_falling_and_respawn.c:923 (`KillThePlayer`).
/// TimeAware: cancelling skates calls the IRQ-gated HUD icon upload.
pub(super) fn kill_the_player(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let number = i32::from(arguments[0] as i16);
    let revision = context.revision();
    if ram::g_player_inventory_open(revision)
        .at(number)
        .read(context)
        != 0
    {
        return Ok(None);
    }
    let player = super::indexing::player_by_word_index(revision, number);
    call(context, ids::STOP_PLAYER, &[player.address()])?;
    player.velocity_height().write(context, 256);
    super::frames::clear_byte(context, player.animation_motion_axes().address())?;
    if ram::g_player_powerups(revision).at(number).read(context) == 5 {
        call(context, ids::PLAY_SOUND, &[6, 99])?;
        call(context, ids::PLAY_SOUND, &[0, 98])?;
        ram::g_player_powerups(revision)
            .at(number)
            .write(context, -1);
        call(context, ids::HUD_LOAD_ACTION_ICON_SPRITE, &[number as u32])?;
        call(
            context,
            ids::HUD_POSITION_ACTION_ICON_SPRITE,
            &[number as u32],
        )?;
    }
    player.height().write(
        context,
        // c_active_present_falling_and_respawn.s:2163-2165: distinct CMPI.B reads.
        if player.sprite_state().read(context) == 3
            || player.sprite_state().read(context) == 7
            || super::frames::read_long(context, player.animation_group_mask().address()) & 0x800
                != 0
        {
            -58
        } else {
            -16
        },
    );
    player
        .draw_flags()
        .write(context, player.draw_flags().read(context) & !0xc0);
    ram::g_player_pre_death_sprite_state(revision)
        .at(number)
        .write(context, player.sprite_state().read(context));
    call(context, ids::SET_SPRITE, &[player.address(), 5])?;
    if ram::g_player_lives(revision).at(number).read(context) == 0 {
        player
            .behavior_flags()
            .write(context, player.behavior_flags().read(context) | 0x80);
    }
    super::frames::clear_byte(
        context,
        ram::g_player_jump_counters(revision).at(number).address(),
    )?;
    super::frames::clear_byte(
        context,
        ram::g_player_terrain_sinking(revision).at(number).address(),
    )?;
    player
        .behavior_flags()
        .write(context, player.behavior_flags().read(context) & !1);
    Ok(None)
}
