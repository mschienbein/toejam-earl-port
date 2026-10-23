use crate::call_frame::{CallSite, FrameLayout, LocalFrame};
use crate::{Context, Result, call_at, ids};
use tje_mem::{Address, EarthlingState, LevelItem, ObjectState, ram};

/// G/src/presents/item_updates.c:230 (`PlayerCheckLevelItemHazards`).
/// TimeAware: the hurt handler can enter speech. LINK -20; MOVEM D2-D4/A2-A3;
/// only the original source fields at A6-20, -19, -14 and -12 are initialized.
/// Player_Youch's return PC is object offset 0x666 - function start 0x5c0.
pub fn player_check_level_item_hazards(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(20, 0x1c, 0x0c))?;
    let result = level_hazards_body(context, arguments, &frame);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    frame.leave(context)?;
    result
}

fn level_hazards_body(
    context: &mut dyn Context,
    arguments: &[u32],
    frame: &LocalFrame,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let items = Address::<LevelItem>::new(arguments[1]);
    let player = super::word_indexed_player(revision, index);
    if player.height().read(context) > 0 {
        return Ok(None);
    }
    let source = frame.local::<EarthlingState>(-20);
    for slot in 0..4 {
        let item = items.offset(slot);
        if item.kind().read(context) == 83
            && item.object().read(context) != 0
            && (i32::from(item.x().read(context)) - i32::from(player.x().read(context)))
                .wrapping_abs()
                < 24
            && (i32::from(item.y().read(context)) - i32::from(player.y().read(context)))
                .wrapping_abs()
                < 12
        {
            source.kind().write(context, 1);
            source.x().write(context, item.x().read(context));
            source.y().write(context, item.y().read(context));
            source.object().write(context, item.object().read(context));
            call_at(
                context,
                ids::PLAYER_YOUCH,
                &[index as u32, source.address()],
                CallSite::relative(ids::PLAYER_CHECK_LEVEL_ITEM_HAZARDS, revision, 0xa6),
            )?;
        }
    }
    Ok(None)
}

/// G/src/presents/projectile_effects.c:568 (`PresentUpdateTerrainEffects`).
/// TimeAware: a grown effect can call Player_Youch with its partial stack source.
/// LINK -20; MOVEM D2-D4/A2-A4. Call-PC offsets are from object start 0xde2.
pub fn present_update_terrain_effects(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(20, 0x1c, 0x1c))?;
    let result = terrain_effects_body(context, arguments, &frame);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    frame.leave(context)?;
    result
}

fn terrain_effects_body(
    context: &mut dyn Context,
    arguments: &[u32],
    frame: &LocalFrame,
) -> Result<Option<u32>> {
    let revision = context.revision();
    let caller = ids::PRESENT_UPDATE_TERRAIN_EFFECTS;
    let player = Address::<ObjectState>::new(arguments[0]);
    let terrain = arguments[1] as u16;
    let index = i32::from(player.entity_index().read(context));
    let active = ram::g_player_active_terrain_effect_entities(revision).at(index);
    let one_shot = ram::g_player_one_shot_terrain_effect_entities(revision).at(index);
    if ram::g_player_powerups(revision).at(index).read(context) != 6
        || ram::g_player_elevator_flags(revision)
            .at(index)
            .read(context)
            != 0
        || matches!(player.sprite_state().read(context), 5 | 6)
    {
        if active.read(context) != 0 {
            call_at(
                context,
                ids::PRESENT_RETIRE_TERRAIN_EFFECT,
                &[index as u32],
                CallSite::relative(caller, revision, 0x6a),
            )?;
        }
    } else if ram::g_player_previous_processed_input(revision)
        .at(index)
        .read(context)
        & 0x40
        == 0
        && ram::g_player_processed_input(revision)
            .at(index)
            .read(context)
            & 0x40
            != 0
    {
        if terrain == 0x10 || terrain == 0x50 {
            if active.read(context) != 0 {
                call_at(
                    context,
                    ids::PRESENT_RETIRE_TERRAIN_EFFECT,
                    &[index as u32],
                    CallSite::relative(caller, revision, 0xb6),
                )?;
            }
            call_at(
                context,
                ids::PRESENT_CREATE_ACTIVE_TERRAIN_EFFECT,
                &[player.address(), index as u32],
                CallSite::relative(caller, revision, 0xc4),
            )?;
        } else if terrain < 0x80 && one_shot.read(context) == 0 {
            call_at(
                context,
                ids::PRESENT_CREATE_TEMPORARY_TERRAIN_EFFECT,
                &[player.address(), index as u32],
                CallSite::relative(caller, revision, 0xee),
            )?;
        }
    } else {
        let number = active.read(context);
        if number != 0 {
            let effect = ram::g_player_info(revision).at(i32::from(number));
            if effect.frame_duration().read(context) == 0
                && effect.draw_flags().read(context) & 6 != 0
            {
                let source = frame.local::<EarthlingState>(-20);
                source.kind().write(context, 1);
                source.x().write(context, effect.x().read(context));
                source.y().write(context, effect.y().read(context));
                source.object().write(context, number);
                if ram::g_player_elevator_flags(revision)
                    .at(index)
                    .read(context)
                    == 0
                    && call_at(
                        context,
                        ids::ENTITY_IS_WITHIN_RANGE,
                        &[effect.address(), player.address(), 25, 10],
                        CallSite::relative(caller, revision, 0x154),
                    )? != 0
                {
                    call_at(
                        context,
                        ids::PLAYER_YOUCH,
                        &[index as u32, source.address()],
                        CallSite::relative(caller, revision, 0x16c),
                    )?;
                }
                if ram::g_player_elevator_flags(revision)
                    .at(index ^ 1)
                    .read(context)
                    == 0
                {
                    // The XOR expression is widened before its ASL.L #7.
                    let partner = ram::g_player_info(revision).at(index ^ 1);
                    if call_at(
                        context,
                        ids::ENTITY_IS_WITHIN_RANGE,
                        &[effect.address(), partner.address(), 25, 10],
                        CallSite::relative(caller, revision, 0x19c),
                    )? != 0
                    {
                        call_at(
                            context,
                            ids::PLAYER_YOUCH,
                            &[(index ^ 1) as u32, source.address()],
                            CallSite::relative(caller, revision, 0x1b8),
                        )?;
                    }
                }
            }
        }
    }
    let number = one_shot.read(context);
    if number != 0
        && ram::g_player_info(revision)
            .at(i32::from(number))
            .frame_duration()
            .read(context)
            == 0
    {
        call_at(
            context,
            ids::FREE_ENTITY,
            &[number as u32],
            CallSite::relative(caller, revision, 0x1ea),
        )?;
        one_shot.write(context, 0);
    }
    Ok(None)
}
