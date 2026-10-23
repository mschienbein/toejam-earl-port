use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/presents/level_item_setup.c:116 (`ElevatorAllowsWorldPlacement`).
/// Atomic: two RAM anchor records, no device observations or waits.
pub fn elevator_allows_world_placement(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let x = i32::from(arguments[0] as i16);
    let y = i32::from(arguments[1] as i16);
    if arguments[2] as i16 == 1 {
        return Ok(Some(1));
    }
    for index in 0..2 {
        let anchor = ram::g_level_anchor_positions(context.revision()).at(index);
        let elevator_x = ((i32::from(anchor.x().read(context)) + 2) * 128) as i16;
        let elevator_x = elevator_x.wrapping_sub(16);
        let elevator_y = ((i32::from(anchor.y().read(context)) + 2) * 64) as i16;
        let elevator_y = elevator_y.wrapping_sub(5);
        if (x - i32::from(elevator_x)).wrapping_abs() < 30
            && (y - (i32::from(elevator_y) + 5)).wrapping_abs() < 20
        {
            return Ok(Some(0));
        }
    }
    Ok(Some(1))
}

/// G/src/presents/level_item_setup.c:32 (`SetupPresentIdentities`).
/// Atomic: RAM shuffle and bounded RNG calls, without device observations.
pub fn setup_present_identities(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let info = ram::g_present_info(context.revision());
    for index in 0..27 {
        info.at(index).appearance().write(context, -1);
        info.at(index).identified().write(context, 0);
    }
    info.at(26).appearance().write(context, 26);
    info.at(27).appearance().write(context, 27);
    info.at(27).identified().write(context, 1);
    for appearance in 0..26 {
        let mut identity = ((call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 27) as i16;
        while info.at(i32::from(identity)).appearance().read(context) != -1 {
            identity = ((i32::from(identity) + 1) % 27) as i16;
        }
        info.at(i32::from(identity))
            .appearance()
            .write(context, appearance);
    }
    Ok(None)
}

/// G/src/presents/level_item_setup.c:303 (`SelectRandomPresent`).
/// Atomic: ROM weight reads and RNG, including the original zero-draw retry.
pub fn select_random_present(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let weights = rom::present_weight_table(context.revision());
    let mut total = 0_i16;
    for index in 0..27 {
        total = total.wrapping_add(i16::from(weights.at(index).read(context)));
    }
    let selected = loop {
        let random = call(context, ids::GET_RANDOM_NUMBER, &[])? as u16;
        let selected =
            tje_mem::arithmetic::expr_rem_u32(u32::from(random), (i32::from(total) + 1) as u32)?
                as i16;
        if selected != 0 {
            break selected;
        }
    };
    total = 0;
    for index in 0..27 {
        total = total.wrapping_add(i16::from(weights.at(index).read(context)));
        if selected <= total {
            return Ok(Some(index as u32));
        }
    }
    Ok(Some(27))
}

/// G/src/presents/level_item_setup.c:90 (`MapReleaseLevelItemBank`).
/// Atomic: reference-counted RAM bank release and storage-only FreeEntity.
pub fn map_release_level_item_bank(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let level = arguments[0] as i16;
    for bank in 0..2 {
        let users = ram::g_item_bank_users(revision).at(bank);
        if users.read(context) != 0
            && i16::from(ram::g_item_bank_levels(revision).at(bank).read(context)) == level
        {
            users.write(context, users.read(context).wrapping_sub(1));
            if users.read(context) == 0 {
                ram::g_item_bank_levels(revision)
                    .at(bank)
                    .write(context, -1);
                for index in 0..32 {
                    let item = ram::g_object_table(revision).at(bank).at(index);
                    let object = item.object().read(context);
                    item.kind().write(context, -1);
                    if object != 0 {
                        call(context, ids::FREE_ENTITY, &[object as u32])?;
                        item.object().write(context, 0);
                    }
                }
            }
        }
    }
    Ok(None)
}

/// G/src/presents/level_item_setup.c:409 (`DropPresent`).
/// Atomic: cached RNG, two perimeter scans, RAM bank insertion and entity release.
pub fn drop_present(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let index = i32::from(arguments[0] as i16);
    let player = ram::g_player_info(revision).at(index);
    if player.animation_group_mask().read(context) & 0x800 != 0
        || matches!(player.sprite_state().read(context), 3 | 7)
    {
        return Ok(None);
    }
    let land = if index == 0 {
        ram::g_player1_nearby_terrain(revision).cast::<u8>()
    } else {
        ram::g_player2_nearby_terrain(revision).cast::<u8>()
    };
    let destination = ram::g_dropped_present_table(revision).at(i32::from(
        ram::g_dropped_present_table_idx(revision).read(context),
    ));
    if destination.kind().read(context) != -1 && destination.object().read(context) != 0 {
        let number = destination.object().read(context);
        call(context, ids::FREE_ENTITY, &[number as u32])?;
        destination.object().write(context, 0);
    }
    let random_index = ram::g_random_cache_index(revision);
    let next = if random_index.read(context) == 58 {
        0
    } else {
        random_index.read(context).wrapping_add(1)
    };
    random_index.write(context, next);
    let random = ram::g_cached_random_values(revision)
        .at(i32::from(next))
        .read(context);
    for pass in 0..2 {
        for row in 0..5 {
            for column in 0..5 {
                let mut y = if random & 1 != 0 { 4 - row } else { row };
                let mut x = if random & 2 != 0 { 4 - column } else { column };
                if random & 4 != 0 {
                    std::mem::swap(&mut x, &mut y);
                }
                if row != 0 && row != 4 && column != 0 && column != 4 {
                    continue;
                }
                let terrain = land.offset(y * 5 + x).read(context);
                if terrain == 0x90 || terrain == 0x80 {
                    continue;
                }
                let position_x = (x * 8 + i32::from(player.x().read(context)) - 16) as i16;
                let position_y = (y * 8 + i32::from(player.y().read(context)) - 16) as i16;
                if pass == 0
                    && (0..32).any(|slot| {
                        let item = ram::g_dropped_present_table(revision).at(slot);
                        item.kind().read(context) != -1
                            && item.level().read(context) == player.level().read(context)
                            && item.x().read(context) == position_x
                            && item.y().read(context) == position_y
                    })
                {
                    continue;
                }
                destination.kind().write(context, arguments[1] as i8);
                destination
                    .level()
                    .write(context, player.level().read(context));
                destination.x().write(context, position_x);
                destination.y().write(context, position_y);
                let current = ram::g_dropped_present_table_idx(revision).read(context);
                ram::g_dropped_present_table_idx(revision)
                    .write(context, current.wrapping_add(1) & 31);
                return Ok(None);
            }
        }
    }
    Ok(None)
}
