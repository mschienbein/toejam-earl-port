use crate::ui::animations::wait_upload;
use crate::ui::hud_all::frame as source;
use crate::{
    Context, Result,
    call_frame::{FrameLayout, LocalFrame},
    ids,
};
use tje_mem::{Address, ObjectState, ram};

/// G/src/ui/hud.c:598 (`CheckPromotionScore`).
/// Points compare as unsigned words; the queued upload is serviced by real IRQs.
pub fn check_promotion_score(context: &mut dyn Context) -> Result<Option<u32>> {
    // c_hud.s:1407 is a single MOVE.L save, not MOVEM's extra terminal read.
    let frame = LocalFrame::enter(context, FrameLayout::movem(4, 0, 0))?;
    let saved_d2 = frame.local::<u32>(-4);
    let entry_d2 = context.data_register(2);
    saved_d2.write(context, entry_d2);
    let revision = context.revision();
    let player = i32::from(frame.argument_word(0).read(context));
    let mut promoted = 0;
    if ram::g_player_lives(revision)
        .at_long_index(player)
        .read(context)
        >= 0
    {
        let rank = i16::from(
            ram::g_player_rank(revision)
                .at_long_index(player)
                .read(context),
        );
        if rank < 8 {
            source::push_move(context, i32::from(rank) as u32)?;
            let threshold = source::call(
                context,
                ids::CHECK_PROMOTION_SCORE,
                ids::GET_SCORE_FOR_PROMO,
                0x3a,
            )? as u16;
            source::release(context, 4)?;
            if ram::g_player_points(revision)
                .at_long_index(player)
                .read(context)
                >= threshold
            {
                let rank = ram::g_player_rank(revision).at_long_index(player);
                let next = rank.read(context).wrapping_add(1);
                rank.write(context, next);
                wait_upload(context)?;
                source::push_clear(context)?;
                source::push_move(context, player as u32)?;
                source::call(
                    context,
                    ids::CHECK_PROMOTION_SCORE,
                    ids::HUD_INITIALIZE_HEALTH_BAR,
                    0x72,
                )?;
                let split = ram::g_split_screen_enabled(revision).read(context);
                source::release(context, 8)?;
                if split == 0 {
                    ram::g_hud_redraw_pending(revision).write(context, 1);
                }
                promoted = 1;
            }
        }
    }
    let old_d2 = saved_d2.read(context);
    context.set_data_register(2, old_d2);
    frame.leave(context)?;
    Ok(Some(promoted))
}

/// G/src/ui/hud.c:665 (`PlayerResetRankAndResources`).
/// Preserve the original missing rank-eight stop, including its endless loop.
pub fn player_reset_rank_and_resources(context: &mut dyn Context) -> Result<Option<u32>> {
    // Both c_hud.s:1519-1520: MOVEM D2/D3/A2/A3 then a signed argument word.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x0c))?;
    let revision = context.revision();
    let player = i32::from(frame.argument_word(0).read(context));
    let rank = ram::g_player_rank(revision).at_long_index(player);
    context.discard_u8(rank.address())?;
    rank.write(context, 0);
    loop {
        let current_rank = i32::from(rank.read(context)) as u32;
        source::push_move(context, current_rank)?;
        let threshold = source::call(
            context,
            ids::PLAYER_RESET_RANK_AND_RESOURCES,
            ids::GET_SCORE_FOR_PROMO,
            0x32,
        )? as u16;
        source::release(context, 4)?;
        if ram::g_player_points(revision)
            .at_long_index(player)
            .read(context)
            < threshold
        {
            break;
        }
        let next = rank.read(context).wrapping_add(1);
        rank.write(context, next);
    }
    let bucks = ram::g_player_bucks(revision).at_long_index(player);
    context.discard_u8(bucks.address())?;
    bucks.write(context, 0);
    let lives = ram::g_player_lives(revision).at_long_index(player);
    if ram::g_player_selection(revision).read(context) == 2 {
        context.discard_u8(lives.address())?;
        lives.write(context, 0);
    } else {
        lives.write(context, 3);
        bucks.write(context, 3);
    }
    source::push_move(context, player as u32)?;
    let health = source::call(
        context,
        ids::PLAYER_RESET_RANK_AND_RESOURCES,
        ids::PLAYER_GET_MAXIMUM_HEALTH,
        0xa4,
    )? as i8;
    source::release(context, 4)?;
    ram::g_player_health(revision)
        .at_long_index(player)
        .write(context, health);
    ram::g_player_health_before_swimming(revision)
        .at_long_index(player)
        .write(context, -1);
    ram::g_player_powerups(revision)
        .at_long_index(player)
        .write(context, -1);
    // The compiler retains all three call areas until LEA 16(SP),SP.
    source::push_pea(context, 1)?;
    source::push_move(context, player as u32)?;
    source::call(
        context,
        ids::PLAYER_RESET_RANK_AND_RESOURCES,
        ids::HUD_INITIALIZE_HEALTH_BAR,
        0xd8,
    )?;
    source::push_move(context, player as u32)?;
    source::call(
        context,
        ids::PLAYER_RESET_RANK_AND_RESOURCES,
        ids::HUD_LOAD_ACTION_ICON_SPRITE,
        0xe2,
    )?;
    source::push_move(context, player as u32)?;
    source::call(
        context,
        ids::PLAYER_RESET_RANK_AND_RESOURCES,
        ids::HUD_LOAD_LIVES_SPRITE_TILES,
        0xec,
    )?;
    source::release(context, 16)?;
    frame.leave(context)?;
    Ok(None)
}

/// G/src/ui/hud.c:689 (`SetupTheMainGame`).
/// Absent players retain resources; both players' HUD constructors still run.
pub fn setup_the_main_game(context: &mut dyn Context) -> Result<Option<u32>> {
    // No register-save prologue; keep the original caller-owned outgoing stack.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0, 0))?;
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
    // Linked c_hud.s:1623-1641, same return offsets in both revisions.
    source::push_pea(context, 1)?;
    source::push_clear(context)?;
    source::call(
        context,
        ids::SETUP_THE_MAIN_GAME,
        ids::HUD_INITIALIZE_HEALTH_BAR,
        0x74,
    )?;
    source::push_pea(context, 1)?;
    source::push_pea(context, 1)?;
    source::call(
        context,
        ids::SETUP_THE_MAIN_GAME,
        ids::HUD_INITIALIZE_HEALTH_BAR,
        0x80,
    )?;
    source::push_clear(context)?;
    source::call(
        context,
        ids::SETUP_THE_MAIN_GAME,
        ids::HUD_LOAD_LEVEL_NUMBER_SPRITES,
        0x86,
    )?;
    source::push_pea(context, 1)?;
    source::call(
        context,
        ids::SETUP_THE_MAIN_GAME,
        ids::HUD_LOAD_LEVEL_NUMBER_SPRITES,
        0x8e,
    )?;
    source::push_clear(context)?;
    source::call(
        context,
        ids::SETUP_THE_MAIN_GAME,
        ids::HUD_LOAD_ACTION_ICON_SPRITE,
        0x94,
    )?;
    source::push_pea(context, 1)?;
    source::call(
        context,
        ids::SETUP_THE_MAIN_GAME,
        ids::HUD_LOAD_ACTION_ICON_SPRITE,
        0x9c,
    )?;
    source::push_clear(context)?;
    source::call(
        context,
        ids::SETUP_THE_MAIN_GAME,
        ids::HUD_LOAD_LIVES_SPRITE_TILES,
        0xa2,
    )?;
    source::push_pea(context, 1)?;
    source::call(
        context,
        ids::SETUP_THE_MAIN_GAME,
        ids::HUD_LOAD_LIVES_SPRITE_TILES,
        0xaa,
    )?;
    source::release(context, 40)?;
    frame.leave(context)?;
    Ok(None)
}

/// G/src/ui/hud.c:724 (`PlayerUpdateHealthAndLives`).
/// c_hud.s:1687-1689: SUBQ.B then BGE tests N==V, not the wrapped sign.
/// At lives=-128 the stored decrement is +127 but the negative branch runs.
pub fn player_update_health_and_lives(context: &mut dyn Context) -> Result<Option<u32>> {
    // Both c_hud.s:1647-1649: MOVEM D2/D3/A2-A4, object long then terrain word.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x1c))?;
    let revision = context.revision();
    let object = Address::<ObjectState>::new(frame.argument_slot(0).read(context));
    let terrain = u32::from(frame.argument_word(1).cast::<u16>().read(context));
    let player = i32::from(object.entity_index().read(context));
    if object.sprite_state().read(context) == 5
        || object.sprite_state().read(context) == 6
        || object.animation_group_mask().read(context) & 0x20000 != 0
        || ram::g_player1_map_level(revision).read(context) == 26
        || ram::g_player_promotion_display_entities(revision)
            .at(player)
            .read(context)
            != 0
    {
        frame.leave(context)?;
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
        source::push_move(context, player as u32)?;
        source::call(
            context,
            ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
            ids::KILL_THE_PLAYER,
            0x7e,
        )?;
        let lives = ram::g_player_lives(revision).at(player);
        let decremented = i16::from(lives.read(context)) - 1;
        lives.write(context, decremented as i8);
        source::release(context, 4)?;
        if decremented < 0 {
            lives.write(context, lives.read(context).wrapping_add(1));
            saved_health.write(context, -1);
        } else {
            health.write(context, 100);
            if saved_health.read(context) != -1 {
                saved_health.write(context, 100);
            }
        }
        frame.leave(context)?;
        return Ok(None);
    }
    if object.animation_group_mask().read(context) & 0x800 != 0
        || object.sprite_state().read(context) == 3
        || object.sprite_state().read(context) == 7
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
    source::push_move(context, player as u32)?;
    let promoted = source::call(
        context,
        ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
        ids::CHECK_PROMOTION_SCORE,
        0x16a,
    )?;
    source::release(context, 4)?;
    if promoted != 0 {
        source::push_move(context, terrain)?;
        source::push_pea(context, 11)?;
        source::push_move(context, player as u32)?;
        source::call(
            context,
            ids::PLAYER_UPDATE_HEALTH_AND_LIVES,
            ids::PERFORM_PROMOTION,
            0x186,
        )?;
        source::release(context, 12)?;
    }
    frame.leave(context)?;
    Ok(None)
}
