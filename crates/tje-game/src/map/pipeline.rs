use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

pub(super) fn corners(c: &dyn Context, row: i32, column: i32) -> u16 {
    let map = ram::g_player1_level_map(c.revision());
    let mut mask = 0;
    for (enabled, cells, bit) in [
        (
            row > 0 && column > 0,
            [(row, column - 1), (row - 1, column - 1), (row - 1, column)],
            1,
        ),
        (
            row > 0 && column < 18,
            [(row - 1, column), (row - 1, column + 1), (row, column + 1)],
            2,
        ),
        (
            row < 25 && column < 18,
            [(row, column + 1), (row + 1, column + 1), (row + 1, column)],
            4,
        ),
        (
            row < 25 && column > 0,
            [(row + 1, column), (row + 1, column - 1), (row, column - 1)],
            8,
        ),
    ] {
        if enabled
            && cells
                .into_iter()
                .all(|(y, x)| map.at(y).at(x).read(c) & 0x800 != 0)
        {
            mask |= bit;
        }
    }
    mask
}

/// G/src/map/level_generation_and_changes.c:86 (`MapBuildFilledTerrainShapes`).
pub fn map_build_filled_terrain_shapes(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let map = ram::g_player1_level_map(r);
    for row in 0..26 {
        for column in 0..19 {
            let cell = map.at(row).at(column);
            let shape = cell.read(c);
            cell.write(
                c,
                if shape & 0x3000 == 0x3000 {
                    cell.read(c) | 0x800
                } else {
                    cell.read(c) & 0xf7ff
                },
            );
            if shape & 0x3000 != 0x2000 {
                let d = ram::g_map_path_direction_banks(r).at(row).at(column);
                d.write(c, d.read(c) & 0xf0);
            }
        }
    }
    for row in 0..26 {
        for column in 0..19 {
            let cell = map.at(row).at(column);
            if cell.read(c) & 0x800 != 0 {
                let mask = corners(c, row, column);
                cell.write(c, cell.read(c) | mask);
            }
        }
    }
    call(c, ids::MAP_REFINE_FILLED_SHAPE_MASKS, &[])?;
    for row in 0..26 {
        for column in 0..19 {
            let cell = map.at(row).at(column);
            let shape = cell.read(c);
            if shape & 0x800 != 0 {
                let index = i32::from(shape & 31);
                cell.write(c, cell.read(c) & 0xff00);
                let bank = rom::map_generation_lookup_tables(r);
                let value = if cell.read(c) & 0x8000 != 0 {
                    bank.filled_alternate().at(index).read(c)
                } else {
                    bank.filled_plain().at(index).read(c)
                };
                cell.write(c, cell.read(c) | u16::from(value));
            }
        }
    }
    Ok(None)
}

/// G/src/map/random_regions_and_cleanup.c:433 (`MapExpandRegionsBesidePaths`).
pub fn map_expand_regions_beside_paths(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let row = i32::from(args[0] as i16);
    let column = i32::from(args[1] as i16);
    let map = ram::g_player1_level_map(c.revision());
    let mut changed = 0;
    for (bit, dy, dx, checks, sy, sx, h, w) in [
        (
            1,
            -1,
            0,
            [(-1, -1), (-1, 1), (-2, -1), (-2, 1)],
            -2,
            -1,
            2,
            3,
        ),
        (4, 1, 0, [(1, -1), (1, 1), (2, -1), (2, 1)], 1, -1, 2, 3),
        (
            8,
            0,
            -1,
            [(-1, -1), (1, -1), (-1, -2), (1, -2)],
            -1,
            -2,
            3,
            2,
        ),
        (2, 0, 1, [(-1, 1), (1, 1), (-1, 2), (1, 2)], -1, 1, 3, 2),
    ] {
        if map
            .at_word_index(row as i16)
            .at_word_index(column as i16)
            .read(c)
            & bit
            != 0
        {
            // The unchanged short axis uses a word displacement; the adjusted
            // axis is promoted and uses the emitted full long displacement.
            let neighbor = if dy == 0 {
                map.at_word_index(row as i16).at(column + dx)
            } else {
                map.at(row + dy).at_word_index(column as i16)
            };
            let terrain = neighbor.read(c);
            if terrain & 0x3000 == 0x1000
                && checks
                    .into_iter()
                    .any(|(y, x)| map.at(row + y).at(column + x).read(c) != terrain)
            {
                call(
                    c,
                    ids::MAP_FILL_RECTANGLE_AND_MERGE_REGIONS,
                    &[
                        (row + sy) as u32,
                        (column + sx) as u32,
                        h,
                        w,
                        i32::from(terrain as i16) as u32,
                    ],
                )?;
                changed = 1;
            }
        }
    }
    Ok(Some(changed))
}

/// G/src/map/map_population.c:71 (`MapIsAnchorConnectionMissing`).
pub fn map_is_anchor_connection_missing(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let a = ram::g_level_anchor_positions(c.revision()).at(0);
    let x = i32::from(a.x().read(c)) as u32;
    let y = i32::from(a.y().read(c)) as u32;
    Ok(Some(u32::from(
        call(c, ids::MAP_FIND_ROUTE_TO_SECOND_ANCHOR, &[y, x])? == 0,
    )))
}

/// G/src/map/transition_paths.c:26 (`MapTraceTransitionChain`).
/// Transitions are installed only on successful unwind; failed recursion decrements visits.
pub fn map_trace_transition_chain(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let row = i32::from(args[0] as i16);
    let column = i32::from(args[1] as i16);
    let incoming = args[2] as i16;
    let outgoing = args[3] as i16;
    let cell = ram::g_player1_level_map(r)
        .at_word_index(row as i16)
        .at_word_index(column as i16);
    if cell.read(c) & 0x1f00 != 0 {
        return Ok(Some(0));
    }
    for i in 0..3 {
        let a = ram::g_level_anchor_positions(r).at(i);
        if i32::from(a.x().read(c)) == column && i32::from(a.y().read(c)) == row {
            return Ok(Some(0));
        }
    }
    let visits = ram::g_transition_path_visit_count(r);
    visits.write(c, visits.read(c).wrapping_add(1));
    let rules: &[(u32, i32, i32, i16, i16, bool)] = match (incoming, outgoing) {
        (8, 8) => &[
            (3, 0, -1, 8, 8, false),
            (4, 1, 0, 4, 8, false),
            (2, 0, 0, 0, 0, true),
        ],
        (2, 2) => &[
            (5, 0, 1, 2, 2, false),
            (6, 1, 0, 4, 2, false),
            (1, 0, 0, 0, 0, true),
        ],
        (4, 8) => &[(7, 0, -1, 8, 4, false), (8, 1, 0, 4, 2, false)],
        (4, 2) => &[(9, 0, 1, 2, 4, false), (10, 1, 0, 4, 8, false)],
        (8, 4) => &[
            (11, 0, -1, 8, 8, false),
            (12, 1, 0, 4, 8, false),
            (13, 0, 0, 0, 0, true),
        ],
        (2, 4) => &[
            (14, 0, 1, 2, 2, false),
            (15, 1, 0, 4, 2, false),
            (16, 0, 0, 0, 0, true),
        ],
        _ => &[],
    };
    for &(descriptor, dy, dx, next_in, next_out, terminal) in rules {
        let value = u32::from(cell.read(c));
        if call(
            c,
            ids::MAP_TRANSITION_CONTAINS_METATILE,
            &[descriptor, value],
        )? != 0
        {
            let found = if terminal {
                visits.read(c) >= ram::g_transition_path_minimum_visits(r).read(c)
            } else {
                call(
                    c,
                    ids::MAP_TRACE_TRANSITION_CHAIN,
                    &[
                        (row + dy) as u32,
                        (column + dx) as u32,
                        i32::from(next_in) as u32,
                        i32::from(next_out) as u32,
                    ],
                )? != 0
            };
            if found {
                cell.write(c, cell.read(c) | ((descriptor as u16) << 8));
                return Ok(Some(1));
            }
        }
    }
    visits.write(c, visits.read(c).wrapping_sub(1));
    Ok(Some(0))
}
