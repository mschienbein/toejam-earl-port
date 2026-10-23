use crate::{Context, Result, call, ids};
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
                call(context, ids::GAME_PAUSED, &[0])?;
            }
        } else if ram::g_player2_previous_processed_input(revision).read(context) & 0x80 == 0 {
            call(context, ids::GAME_PAUSED, &[1])?;
        }
        return Ok(Some(1));
    }
    Ok(Some(0))
}

/// G/src/core/startup_and_main_loop.c:321 (`ProcessInput`, REV 02 0x000A6E).
/// Preserve A/Start/B/map dispatch order and the pre-B input snapshots. Map
/// exploration uses narrowed word coordinates and the original unchecked level.
pub fn process_input(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    call(context, ids::GET_PLAYER_INPUT, &[])?;
    call(context, ids::CHECK_A_BUTTON, &[])?;
    let first = ram::g_player_processed_input(revision).at(0).read(context);
    let second = ram::g_player_processed_input(revision).at(1).read(context);
    if call(context, ids::CHECK_START_BUTTON, &[])? != 0 {
        return Ok(None);
    }
    if ram::g_player1_map_level(revision).read(context) < 26 {
        call(context, ids::CHECK_B_BUTTON, &[])?;
        if (first | second) & 0x20 != 0 {
            if first & 0x20 != 0 {
                if ram::g_player_inventory_open(revision).at(0).read(context) == 0 {
                    call(context, ids::DRAW_GAME_MAP, &[0])?;
                    return Ok(None);
                }
            } else if ram::g_player_inventory_open(revision).at(1).read(context) == 0 {
                call(context, ids::DRAW_GAME_MAP, &[1])?;
                return Ok(None);
            }
        }
    }
    for player in 0..2 {
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
            call(context, ids::SPRITE_CHECKS, &[player as u32, input])?;
            let entity = ram::g_player_info(revision).at(player);
            let x = entity.x().read(context).wrapping_sub(72);
            let y = entity.y().read(context).wrapping_sub(8);
            if (0..2560).contains(&x) && (0..1792).contains(&y) {
                let level = i32::from(entity.level().read(context));
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
    }
    Ok(None)
}
