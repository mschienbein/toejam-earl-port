use crate::{Context, Result, call, ids};
use tje_mem::{Address, ObjectState, ram};

/// G/src/ui/hud.c:598 (`CheckPromotionScore`).
/// Points compare as unsigned words; the queued upload is serviced by real IRQs.
pub fn check_promotion_score(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    if ram::g_player_lives(revision).at(player).read(context) < 0 {
        return Ok(Some(0));
    }
    let rank = i16::from(ram::g_player_rank(revision).at(player).read(context));
    if rank < 8 {
        let threshold = call(context, ids::GET_SCORE_FOR_PROMO, &[i32::from(rank) as u32])? as u16;
        if ram::g_player_points(revision).at(player).read(context) >= threshold {
            let rank = ram::g_player_rank(revision).at(player);
            rank.write(context, rank.read(context).wrapping_add(1));
            while ram::g_sprite_graphics_upload_pending(revision).read(context) != 0 {
                context.wait_for_interrupt()?;
            }
            call(context, ids::HUD_INITIALIZE_HEALTH_BAR, &[player as u32, 0])?;
            if ram::g_split_screen_enabled(revision).read(context) == 0 {
                ram::g_hud_redraw_pending(revision).write(context, 1);
            }
            return Ok(Some(1));
        }
    }
    Ok(Some(0))
}

/// G/src/ui/hud.c:665 (`PlayerResetRankAndResources`).
/// Preserve the original missing rank-eight stop, including its endless loop.
pub fn player_reset_rank_and_resources(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    let rank = ram::g_player_rank(revision).at(player);
    rank.write(context, 0);
    loop {
        let current_rank = i32::from(rank.read(context)) as u32;
        let threshold = call(context, ids::GET_SCORE_FOR_PROMO, &[current_rank])? as u16;
        if ram::g_player_points(revision).at(player).read(context) < threshold {
            break;
        }
        rank.write(context, rank.read(context).wrapping_add(1));
    }
    ram::g_player_bucks(revision).at(player).write(context, 0);
    if ram::g_player_selection(revision).read(context) == 2 {
        ram::g_player_lives(revision).at(player).write(context, 0);
    } else {
        ram::g_player_lives(revision).at(player).write(context, 3);
        ram::g_player_bucks(revision).at(player).write(context, 3);
    }
    let health = call(context, ids::PLAYER_GET_MAXIMUM_HEALTH, &[player as u32])? as i8;
    ram::g_player_health(revision)
        .at(player)
        .write(context, health);
    ram::g_player_health_before_swimming(revision)
        .at(player)
        .write(context, -1);
    ram::g_player_powerups(revision)
        .at(player)
        .write(context, -1);
    call(context, ids::HUD_INITIALIZE_HEALTH_BAR, &[player as u32, 1])?;
    call(context, ids::HUD_LOAD_ACTION_ICON_SPRITE, &[player as u32])?;
    call(context, ids::HUD_LOAD_LIVES_SPRITE_TILES, &[player as u32])?;
    Ok(None)
}

/// G/src/ui/hud.c:689 (`SetupTheMainGame`).
/// Absent players retain resources; both players' HUD constructors still run.
pub fn setup_the_main_game(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    if ram::g_player_lives(revision).at(0).read(context) >= 0 {
        ram::g_toejam_level(revision).write(context, 1);
        ram::g_player_health(revision).at(0).write(context, 23);
        ram::g_player_lives(revision).at(0).write(context, 3);
        ram::g_player_bucks(revision).at(0).write(context, 3);
    }
    if ram::g_player_lives(revision).at(1).read(context) >= 0 {
        ram::g_earl_level(revision).write(context, 1);
        ram::g_player_health(revision).at(1).write(context, 31);
        ram::g_player_lives(revision).at(1).write(context, 3);
        ram::g_player_bucks(revision).at(1).write(context, 3);
    }
    ram::g_player_health_before_swimming(revision)
        .at(1)
        .write(context, -1);
    ram::g_player_health_before_swimming(revision)
        .at(0)
        .write(context, -1);
    ram::g_player_powerups(revision).at(0).write(context, -1);
    ram::g_player_powerups(revision).at(1).write(context, -1);
    call(context, ids::HUD_INITIALIZE_HEALTH_BAR, &[0, 1])?;
    call(context, ids::HUD_INITIALIZE_HEALTH_BAR, &[1, 1])?;
    call(context, ids::HUD_LOAD_LEVEL_NUMBER_SPRITES, &[0])?;
    call(context, ids::HUD_LOAD_LEVEL_NUMBER_SPRITES, &[1])?;
    call(context, ids::HUD_LOAD_ACTION_ICON_SPRITE, &[0])?;
    call(context, ids::HUD_LOAD_ACTION_ICON_SPRITE, &[1])?;
    call(context, ids::HUD_LOAD_LIVES_SPRITE_TILES, &[0])?;
    call(context, ids::HUD_LOAD_LIVES_SPRITE_TILES, &[1])?;
    Ok(None)
}

/// G/src/ui/hud.c:724 (`PlayerUpdateHealthAndLives`).
/// c_hud.s:1687-1689: SUBQ.B then BGE tests N==V, not the wrapped sign.
/// At lives=-128 the stored decrement is +127 but the negative branch runs.
pub fn player_update_health_and_lives(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let object = Address::<ObjectState>::new(arguments[0]);
    let terrain = u32::from(arguments[1] as u16);
    let player = i32::from(object.entity_index().read(context));
    if matches!(object.sprite_state().read(context), 5 | 6)
        || object.animation_group_mask().read(context) & 0x20000 != 0
        || ram::g_player1_map_level(revision).read(context) == 26
        || ram::g_player_promotion_display_entities(revision)
            .at(player)
            .read(context)
            != 0
    {
        return Ok(None);
    }
    let health = ram::g_player_health(revision).at(player);
    let saved_health = ram::g_player_health_before_swimming(revision).at(player);
    if ram::g_hud_displayed_health(revision)
        .at(player)
        .read(context)
        == 0
        && health.read(context) <= 0
    {
        call(context, ids::KILL_THE_PLAYER, &[player as u32])?;
        let lives = ram::g_player_lives(revision).at(player);
        let decremented = i16::from(lives.read(context)) - 1;
        lives.write(context, decremented as i8);
        if decremented < 0 {
            lives.write(context, lives.read(context).wrapping_add(1));
            saved_health.write(context, -1);
        } else {
            health.write(context, 100);
            if saved_health.read(context) != -1 {
                saved_health.write(context, 100);
            }
        }
        return Ok(None);
    }
    if object.animation_group_mask().read(context) & 0x800 != 0
        || matches!(object.sprite_state().read(context), 3 | 7)
    {
        if saved_health.read(context) == -1 {
            saved_health.write(context, health.read(context));
        }
        if ram::g_frame_counter(revision).read(context) & 12 == 0 && health.read(context) > 0 {
            health.write(context, health.read(context).wrapping_sub(1));
        }
    } else {
        if saved_health.read(context) != -1 {
            health.write(context, saved_health.read(context));
        }
        saved_health.write(context, -1);
    }
    if object.draw_flags().read(context) & 0x80 != 0 && object.sprite_state().read(context) != 2 {
        object
            .draw_flags()
            .write(context, object.draw_flags().read(context) & 0x7f);
    }
    if call(context, ids::CHECK_PROMOTION_SCORE, &[player as u32])? != 0 {
        call(
            context,
            ids::PERFORM_PROMOTION,
            &[player as u32, 11, terrain],
        )?;
    }
    Ok(None)
}
