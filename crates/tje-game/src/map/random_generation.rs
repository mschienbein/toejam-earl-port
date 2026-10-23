use crate::{Context, Result, call, ids};
use tje_mem::{Address, arithmetic::expr_rem_u32, ram, rom};

pub(super) fn random(c: &mut dyn Context) -> Result<u32> {
    call(c, ids::GET_RANDOM_NUMBER, &[])
}

/// G/src/map/level_generation_and_changes.c:418 (`GenerateLevelSeeds`).
pub fn generate_level_seeds(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    for level in 0..26 {
        let value = if ram::g_world_type(r).read(c) != 0 {
            rom::fixed_world_seeds(r).at(level).read(c)
        } else {
            random(c)? as u16
        };
        ram::g_level_seeds(r).at(level).write(c, value);
    }
    ram::g_player1_map_level(r).write(c, 1);
    ram::g_player2_map_level(r).write(c, 1);
    ram::g_saved_map_level(r).write(c, -1);
    ram::g_highest_level(r).write(c, 1);
    Ok(None)
}

/// G/src/map/random_regions_and_cleanup.c:60 (`MapGetRandomGenerationParameter`).
pub fn map_get_random_generation_parameter(
    c: &mut dyn Context,
    args: &[u32],
) -> Result<Option<u32>> {
    let r = c.revision();
    let index = i32::from(args[0] as i16);
    let row = i32::from(
        rom::map_generation_range_row(r)
            .at(i32::from(ram::g_requested_map_level(r).read(c)) - 2)
            .read(c),
    );
    let pair = rom::map_generation_range_pairs(r).at(row).at(index);
    let minimum = u32::from(pair.at(0).read(c));
    let maximum = u32::from(pair.at(1).read(c));
    let value = random(c)? as u16;
    let span = maximum.wrapping_sub(minimum).wrapping_add(1);
    let selected = expr_rem_u32(u32::from(value), span)?.wrapping_add(minimum);
    Ok(Some(u32::from(selected as u16)))
}

/// G/src/map/map_drawing_and_controller.c:21 (`MapChooseBackgroundTileBucket`).
pub fn map_choose_background_tile_bucket(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let value = random(c)? & 0x3ff;
    let mut threshold = 256;
    let mut bucket = 0;
    while bucket < 8 {
        if value > threshold {
            break;
        }
        threshold -= 32;
        bucket += 1;
    }
    Ok(Some(bucket))
}

/// G/src/map/random_regions_and_cleanup.c:89 (`MapCarveDirectionalPath`).
pub fn map_carve_directional_path(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let map = ram::g_player1_level_map(r);
    let masks = ram::g_map_path_direction_banks(r);
    let mut row = args[0] as i16;
    let mut column = args[1] as i16;
    let target_row = args[2] as i16;
    let target_column = args[3] as i16;
    let random_steps = args[4] as i16;
    let mut turn_chance = args[5] as i16;
    let mut direction_steps = 0_i16;
    let mut opposite = -1_i16;
    let mut previous = -1_i16;
    let mut repeated = 0_i16;
    let mut steps = 0_i16;
    let mut copy =
        random(c)? % 100 < (10 + 3 * i32::from(ram::g_requested_map_level(r).read(c))) as u32;
    let mut direction = 1_i16 << (random(c)? & 3);
    // The first iteration necessarily enters direction_steps<=0 before reading next.
    let mut next = direction;
    while row != target_row || column != target_column {
        if direction_steps <= 0 || turn_chance == 0 {
            direction_steps = (random(c)? % 3 + 2) as i16;
            if (i32::from(turn_chance) as u32) > random(c)? % 100 {
                let index = (random(c)? % 3 + 1).wrapping_add(i32::from(direction) as u32);
                next = i16::from(rom::map_path_turn_directions(r).at(index as i32).read(c));
            } else if random(c)? % 100 < 50 {
                next = if i32::from(target_row) - i32::from(row) > 0 {
                    4
                } else if target_row != row || random(c)? % 100 < 50 {
                    1
                } else {
                    4
                };
            } else {
                next = if i32::from(target_column) - i32::from(column) > 0 {
                    2
                } else if column != target_column || random(c)? % 100 < 50 {
                    8
                } else {
                    2
                };
            }
        }
        if turn_chance == 0 || random(c)? % 100 < 80 {
            direction = next;
        } else {
            let index = (random(c)? % 3 + 1).wrapping_add(i32::from(next) as u32);
            direction = i16::from(rom::map_path_turn_directions(r).at(index as i32).read(c));
        }
        if previous == direction {
            if repeated >= direction_steps {
                let index = (random(c)? % 3 + 1).wrapping_add(i32::from(direction) as u32);
                direction = i16::from(rom::map_path_turn_directions(r).at(index as i32).read(c));
            } else {
                repeated = repeated.wrapping_add(1);
            }
        } else {
            repeated = 0;
        }
        if direction != opposite
            && ((direction == 1 && row > 2)
                || (direction == 4 && row < 24)
                || (direction == 8 && column > 2)
                || (direction == 2 && column < 17))
        {
            let cell = map.at(i32::from(row)).at(i32::from(column));
            cell.write(c, cell.read(c) | (direction as u16) | 0x2000);
            if copy {
                let mask = masks.at(i32::from(row)).at(i32::from(column));
                mask.write(c, mask.read(c) | direction as u8);
            }
            match direction {
                1 => {
                    row = row.wrapping_sub(1);
                    opposite = 4;
                }
                4 => {
                    row = row.wrapping_add(1);
                    opposite = 1;
                }
                8 => {
                    column = column.wrapping_sub(1);
                    opposite = 2;
                }
                2 => {
                    column = column.wrapping_add(1);
                    opposite = 8;
                }
                _ => {}
            }
            let cell = map.at(i32::from(row)).at(i32::from(column));
            cell.write(c, cell.read(c) | opposite as u16 | 0x2000);
            if copy {
                let mask = masks.at(i32::from(row)).at(i32::from(column));
                mask.write(c, mask.read(c) | opposite as u8);
            }
            previous = direction;
            direction_steps = direction_steps.wrapping_sub(1);
            steps = steps.wrapping_add(1);
            if (i32::from(ram::g_requested_map_level(r).read(c)) as u32) > random(c)? % 100 {
                copy = true;
            }
        } else {
            direction_steps = 0;
        }
        if steps >= random_steps {
            turn_chance = 0;
        }
    }
    Ok(None)
}

/// G/src/map/random_regions_and_cleanup.c:182 (`MapFindRegionGrowthOrigin`).
pub fn map_find_region_growth_origin(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let rp = Address::<i16>::new(args[0]);
    let cp = Address::<i16>::new(args[1]);
    let direction = args[2] as i16;
    let region = args[3] as i16;
    let join = args[4] as i16;
    let mut row = rp.read(c);
    let mut column = cp.read(c);
    let map = ram::g_player1_level_map(c.revision());
    loop {
        if direction == 1 {
            row = row.wrapping_sub(1);
        }
        if direction == 4 {
            row = row.wrapping_add(1);
        }
        if direction == 8 {
            column = column.wrapping_sub(1);
        }
        if direction == 2 {
            column = column.wrapping_add(1);
        }
        if row < 0 || column < 0 || row >= 25 || column >= 18 {
            return Ok(Some(0));
        }
        if (row == 0 || row == 24) && direction & 10 != 0 {
            return Ok(Some(0));
        }
        if (column == 0 || column == 17) && direction & 5 != 0 {
            return Ok(Some(0));
        }
        let mut overlaps = 0;
        for dy in 0..2 {
            for dx in 0..2 {
                let tile = map
                    .at(i32::from(row) + dy)
                    .at(i32::from(column) + dx)
                    .read(c) as i16;
                if tile & 0x3000 == 0x1000 && tile != region {
                    overlaps += 1;
                    if overlaps >= 2 && (i32::from(join) as u32) > random(c)? % 100 {
                        rp.write(c, row);
                        cp.write(c, column);
                        return Ok(Some(1));
                    }
                }
            }
        }
        if overlaps != 0 {
            return Ok(Some(0));
        }
        for dy in 0..2 {
            for dx in 0..2 {
                if map
                    .at(i32::from(row) + dy)
                    .at(i32::from(column) + dx)
                    .read(c)
                    != region as u16
                {
                    rp.write(c, row);
                    cp.write(c, column);
                    return Ok(Some(1));
                }
            }
        }
    }
}

/// G/src/map/random_regions_and_cleanup.c:609 (`MapStabilizeRegionConnections`).
pub fn map_stabilize_region_connections(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    call(c, ids::MAP_WIDEN_HORIZONTAL_REGION_EDGES, &[])?;
    call(c, ids::MAP_WIDEN_VERTICAL_REGION_EDGES, &[])?;
    let map = ram::g_player1_level_map(c.revision());
    let mut changed = true;
    while changed {
        changed = false;
        for row in 0..26 {
            for column in 0..19 {
                if map.at(row).at(column).read(c) & 0x3000 == 0x2000
                    && call(
                        c,
                        ids::MAP_EXPAND_REGIONS_BESIDE_PATHS,
                        &[row as u32, column as u32],
                    )? != 0
                {
                    changed = true;
                }
            }
        }
    }
    Ok(None)
}

/// G/src/map/transition_paths.c:128 (`MapAssignTransitionChain`).
pub fn map_assign_transition_chain(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let map = ram::g_player1_level_map(r);
    for row in 0..26 {
        for column in 0..19 {
            let cell = map.at(row).at(column);
            cell.write(c, cell.read(c) & 0xe0ff);
        }
    }
    ram::g_transition_path_minimum_visits(r).write(c, 25);
    for _pass in 0..2 {
        for row in 0..26 {
            for column in 0..19 {
                ram::g_transition_path_visit_count(r).write(c, 0);
                let cell = map.at(row).at(column);
                let value = u32::from(cell.read(c));
                if call(c, ids::MAP_TRANSITION_CONTAINS_METATILE, &[1, value])? != 0 {
                    if call(
                        c,
                        ids::MAP_TRACE_TRANSITION_CHAIN,
                        &[row as u32, (column - 1) as u32, 8, 8],
                    )? != 0
                    {
                        cell.write(c, cell.read(c) | 0x100);
                        return Ok(None);
                    }
                    if call(
                        c,
                        ids::MAP_TRACE_TRANSITION_CHAIN,
                        &[row as u32, (column + 1) as u32, 2, 2],
                    )? != 0
                    {
                        cell.write(c, cell.read(c) | 0x200);
                        return Ok(None);
                    }
                }
            }
        }
        ram::g_transition_path_minimum_visits(r).write(c, 16);
    }
    Ok(None)
}
