use crate::call_frame::{CallSite, FrameLayout, LocalFrame};
use crate::{Context, Result, call_at, ids};
use tje_mem::ram;

/// G/src/core/startup_and_main_loop.c:307 (`CheckStartButton`, REV 02 0x000A24).
/// ToeJam wins simultaneous Start presses. A newly pressed Start enters the
/// original pause flow; holding Start still returns int one after it resumes.
pub fn check_start_button(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let input = ram::g_player_processed_input(revision);
    let second = input.at(1).read(context);
    let first = input.at(0).read(context);
    if (first | second) & 0x80 != 0 {
        if input.at(0).read(context) & 0x80 != 0 {
            if ram::g_player_previous_processed_input(revision)
                .at(0)
                .read(context)
                & 0x80
                == 0
            {
                call_at(
                    context,
                    ids::GAME_PAUSED,
                    &[0],
                    CallSite::relative(ids::CHECK_START_BUTTON, revision, 0x40),
                )?;
            }
        } else if ram::g_player2_previous_processed_input(revision).read(context) & 0x80 == 0 {
            call_at(
                context,
                ids::GAME_PAUSED,
                &[1],
                CallSite::relative(ids::CHECK_START_BUTTON, revision, 0x40),
            )?;
        }
        return Ok(Some(1));
    }
    Ok(Some(0))
}

/// G/src/core/startup_and_main_loop.c:321 (`ProcessInput`, REV 02 0x000A6E).
/// Preserve A/Start/B/map dispatch order and the pre-B input snapshots. Map
/// exploration uses narrowed word coordinates and the original unchecked level.
fn process_input_inner(context: &mut dyn Context) -> Result<Option<u32>> {
    let revision = context.revision();
    context.set_address_register(2, ram::g_player_inventory_open(revision).address());
    context.set_address_register(3, ram::g_player_processed_input(revision).address());
    call_at(
        context,
        ids::GET_PLAYER_INPUT,
        &[],
        CallSite::relative(ids::PROCESS_INPUT, revision, 0x14),
    )?;
    call_at(
        context,
        ids::CHECK_A_BUTTON,
        &[],
        CallSite::relative(ids::PROCESS_INPUT, revision, 0x1a),
    )?;
    let first = ram::g_player_processed_input(revision).at(0).read(context);
    let second = ram::g_player_processed_input(revision).at(1).read(context);
    let previous = context.data_register(3);
    context.set_data_register(3, (previous & 0xffffff00) | u32::from(first));
    let previous = context.data_register(4);
    context.set_data_register(4, (previous & 0xffffff00) | u32::from(second));
    if call_at(
        context,
        ids::CHECK_START_BUTTON,
        &[],
        CallSite::relative(ids::PROCESS_INPUT, revision, 0x24),
    )? != 0
    {
        return Ok(None);
    }
    if ram::g_player1_map_level(revision).read(context) < 26 {
        call_at(
            context,
            ids::CHECK_B_BUTTON,
            &[],
            CallSite::relative(ids::PROCESS_INPUT, revision, 0x3c),
        )?;
        if (first | second) & 0x20 != 0 {
            if first & 0x20 != 0 {
                if ram::g_player_inventory_open(revision).at(0).read(context) == 0 {
                    call_at(
                        context,
                        ids::DRAW_GAME_MAP,
                        &[0],
                        CallSite::relative(ids::PROCESS_INPUT, revision, 0x60),
                    )?;
                    return Ok(None);
                }
            } else if ram::g_player_inventory_open(revision).at(1).read(context) == 0 {
                call_at(
                    context,
                    ids::DRAW_GAME_MAP,
                    &[1],
                    CallSite::relative(ids::PROCESS_INPUT, revision, 0x60),
                )?;
                return Ok(None);
            }
        }
    }
    for player in 0..2 {
        let previous = context.data_register(4);
        context.set_data_register(4, (previous & 0xffff0000) | player as u32);
        if ram::g_player_lives(revision).at(player).read(context) >= 0
            && ram::g_player_inventory_open(revision)
                .at(player)
                .read(context)
                == 0
        {
            let input = u32::from(
                ram::g_player_processed_input(revision)
                    .at(player)
                    .read(context),
            );
            context.set_data_register(2, player as u32);
            call_at(
                context,
                ids::SPRITE_CHECKS,
                &[player as u32, input],
                CallSite::relative(ids::PROCESS_INPUT, revision, 0xaa),
            )?;
            let entity = ram::g_player_info(revision).at(player);
            let x = entity.x().read(context).wrapping_sub(72);
            let y = entity.y().read(context).wrapping_sub(8);
            let previous = context.data_register(5);
            context.set_data_register(5, (previous & 0xffff0000) | u32::from(y as u16));
            if (0..2560).contains(&x) && (0..1792).contains(&y) {
                let level = i32::from(entity.level().read(context));
                let previous = context.data_register(3);
                context.set_data_register(
                    3,
                    (previous & 0xffff0000) | u32::from((level as i16).wrapping_mul(2) as u16),
                );
                let cell = ram::g_map_visited_cover_bits(revision)
                    .at(level)
                    .at(i32::from(y >> 8));
                let mask = 0x80_u8 >> (x / 320) as u32;
                if cell.read(context) & mask == 0 {
                    let next = cell.read(context) | mask;
                    cell.write(context, next);
                    let points = ram::g_player_points(revision).at(player);
                    let next = points.read(context).wrapping_add(1);
                    points.write(context, next);
                    if ram::g_split_screen_enabled(revision).read(context) == 0
                        && ram::g_player_lives(revision).at(player ^ 1).read(context) >= 0
                    {
                        let points = ram::g_player_points(revision).at(player ^ 1);
                        let next = points.read(context).wrapping_add(1);
                        points.write(context, next);
                    }
                }
            }
        }
        let previous = context.data_register(4);
        context.set_data_register(4, (previous & 0xffff0000) | (player + 1) as u32);
    }
    Ok(None)
}

/// G/src/core/startup_and_main_loop.c:321 (`ProcessInput`), matching MOVEM
/// D2-D5/A2-A3 prologue. Framed calls preserve the incoming slots/local addresses
/// used by SpriteChecks and the menu/map callees instead of synthetic sentinels.
pub fn process_input(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x3c, 0x0c))?;
    let result = process_input_inner(context);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let cleanup = frame.leave(context);
    match (result, cleanup) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}
