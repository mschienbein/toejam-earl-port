use crate::{Context, Result};
use tje_mem::{ram, rom};

/// G/src/map/level_generation_and_changes.c:15 (`MapRefineFilledShapeMasks`).
/// The three in-place passes intentionally read across row/map boundaries.
pub fn map_refine_filled_shape_masks(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let map = ram::g_player1_level_map(c.revision());
    for row in 0..26 {
        for column in 0..19 {
            let cell = map.at(row).at(column);
            let mut shape = cell.read(c);
            if shape & 0x0800 != 0 {
                shape &= 0xff;
                if shape == 3 {
                    shape = map.at(row).at(column - 1).read(c) & 0xff;
                    if map.at(row).at(column + 1).read(c) & 0xff != 7
                        && (shape == 11 || shape == 19)
                    {
                        cell.write(c, cell.read(c) | 0x10);
                    }
                }
            }
        }
    }
    for row in 0..26 {
        for column in 0..19 {
            let cell = map.at(row).at(column);
            let mut shape = cell.read(c);
            if shape & 0x0800 != 0 {
                shape &= 0xff;
                if shape == 12 {
                    shape = map.at(row).at(column - 1).read(c) & 0xff;
                    if shape == 4 || shape == 28 {
                        cell.write(c, cell.read(c) | 0x10);
                    }
                }
            }
        }
    }
    for row in 0..26 {
        for column in 0..19 {
            let cell = map.at(row).at(column);
            let mut shape = cell.read(c);
            if shape & 0x0800 != 0 && shape & 15 == 15 {
                shape = map.at(row - 1).at(column).read(c) & 31;
                let upper_left = matches!(shape, 21 | 13 | 12);
                let upper_right = matches!(shape, 14 | 28 | 26);
                shape = map.at(row + 1).at(column).read(c) & 31;
                let lower_left = matches!(shape, 7 | 3);
                let lower_right = matches!(shape, 19 | 11);
                shape = if upper_right {
                    if lower_left {
                        23
                    } else if lower_right {
                        26
                    } else {
                        20
                    }
                } else if upper_left {
                    if lower_right {
                        24
                    } else if lower_left {
                        21
                    } else {
                        20
                    }
                } else {
                    20
                };
                cell.write(c, cell.read(c) & 0xff00);
                cell.write(c, cell.read(c) | shape);
            }
        }
    }
    Ok(None)
}

/// G/src/map/level_generation_and_changes.c:148 (`MapBuildDirectionalPathShapes`).
/// Neighbor priority, separate flag stores, and ROM table views match the source.
pub fn map_build_directional_path_shapes(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = c.revision();
    let map = ram::g_player1_level_map(revision);
    let directions = ram::g_map_path_direction_banks(revision);
    for row in 1..25 {
        for column in 1..18 {
            let cell = map.at(row).at(column);
            if cell.read(c) & 0x3000 == 0x1000 {
                let mut replacement = 0;
                let neighbor = if map.at(row - 1).at(column).read(c) & 0x3004 == 0x2004 {
                    Some((row - 1, column, 4, 16))
                } else if map.at(row).at(column + 1).read(c) & 0x3008 == 0x2008 {
                    Some((row, column + 1, 8, 17))
                } else if map.at(row + 1).at(column).read(c) & 0x3001 == 0x2001 {
                    Some((row + 1, column, 1, 18))
                } else if map.at(row).at(column - 1).read(c) & 0x3002 == 0x2002 {
                    Some((row, column - 1, 2, 19))
                } else {
                    None
                };
                if let Some((neighbor_row, neighbor_column, bit, shape)) = neighbor {
                    if directions.at(neighbor_row).at(neighbor_column).read(c) & bit != 0 {
                        cell.write(c, cell.read(c) | 0x4000);
                    } else {
                        replacement = shape;
                    }
                }
                if replacement != 0 {
                    let upper = cell.read(c) & 0xff00;
                    cell.write(c, upper);
                    let table = if upper & 0x8000 != 0 {
                        rom::map_boundary_shapes_alternate(revision)
                    } else {
                        rom::map_boundary_shapes_plain(revision)
                    };
                    cell.write(c, cell.read(c) | u16::from(table.at(replacement).read(c)));
                }
            }
        }
    }
    for row in 0..26 {
        for column in 0..19 {
            let cell = map.at(row).at(column);
            let mut shape = cell.read(c);
            let mut corners = directions.at(row).at(column).read(c) & 15;
            if ram::g_map_directional_path_mode(revision).read(c) != 0 {
                corners = 0;
            }
            if shape & 0x3000 == 0x2000 {
                shape ^= u16::from(corners);
                let tile = rom::map_generation_lookup_tables(revision)
                    .tiles()
                    .at(i32::from(shape & 15))
                    .read(c);
                cell.write(c, u16::from(tile) | 0x2000);
                if corners != 0 {
                    cell.write(c, cell.read(c) | 0x4000);
                }
            }
        }
    }
    Ok(None)
}

/// G/src/map/level_generation_and_changes.c:335 (`MapSwapPlayerBanks`).
pub fn map_swap_player_banks(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = c.revision();
    let first = ram::g_player1_level_map(revision);
    let second = ram::g_player2_level_map(revision);
    let directions = ram::g_map_path_direction_banks(revision);
    for row in 0..26 {
        for column in 0..19 {
            let a = first.at(row).at(column);
            let b = second.at(row).at(column);
            let saved = a.read(c);
            a.write(c, b.read(c));
            b.write(c, saved);
            let mask = directions.at(row).at(column);
            let saved = mask.read(c);
            mask.write(c, mask.read(c).wrapping_shl(4));
            mask.write(c, mask.read(c) | (saved >> 4));
        }
    }
    Ok(None)
}

/// G/src/map/level_generation_and_changes.c:360 (`CopyLevelBetweenPlayers`).
pub fn copy_level_between_players(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = c.revision();
    let player = args[0] as i16;
    let first = ram::g_player1_level_map(revision);
    let second = ram::g_player2_level_map(revision);
    let (source, destination) = if player == 0 {
        (first, second)
    } else {
        (second, first)
    };
    let directions = ram::g_map_path_direction_banks(revision);
    for row in 0..26 {
        for column in 0..19 {
            destination
                .at(row)
                .at(column)
                .write(c, source.at(row).at(column).read(c));
            let address = directions.at(row).at(column);
            let mask = address.read(c);
            let result = if player == 0 {
                (mask << 4) | (mask & 15)
            } else {
                (mask >> 4) | (mask & 0xf0)
            };
            address.write(c, result);
        }
    }
    Ok(None)
}

/// G/src/map/level_generation_and_changes.c:387 (`MapSwapPlayerWithSavedBank`).
/// Saved directions alias the second half of the direction-bank allocation.
pub fn map_swap_player_with_saved_bank(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = c.revision();
    let player = args[0] as i16;
    let (map, level) = if player == 0 {
        (
            ram::g_player1_level_map(revision),
            ram::g_player1_map_level(revision),
        )
    } else {
        (
            ram::g_player2_level_map(revision),
            ram::g_player2_map_level(revision),
        )
    };
    ram::g_saved_map_level(revision).write(c, level.read(c));
    let saved_map = ram::g_saved_level_map(revision);
    let directions = ram::g_map_path_direction_banks(revision);
    let saved_directions = ram::g_saved_map_path_directions(revision);
    for row in 0..26 {
        for column in 0..19 {
            let active = map.at(row).at(column);
            let saved = saved_map.at(row).at(column);
            let cell = active.read(c);
            active.write(c, saved.read(c));
            saved.write(c, cell);
            let active = directions.at(row).at(column);
            let saved = saved_directions.at(row).at(column);
            let mask = active.read(c);
            if player == 0 {
                active.write(c, (mask & 0xf0) | (saved.read(c) & 15));
                saved.write(c, mask);
            } else {
                active.write(c, (saved.read(c) << 4) | (mask & 15));
                saved.write(c, mask >> 4);
            }
        }
    }
    Ok(None)
}

/// G/src/map/level_generation_and_changes.c:640 (`LoadFinalLevel`).
/// Both ROM reads and the repeated per-cell level-byte stores are preserved.
pub fn load_final_level(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = c.revision();
    let level = args[0] as i16;
    let data = rom::funkatron_map_data(revision);
    for row in 0..26 {
        for column in 0..19 {
            ram::g_player1_level_map(revision)
                .at(row)
                .at(column)
                .write(c, data.at(row).at(column).read(c));
            ram::g_player2_level_map(revision)
                .at(row)
                .at(column)
                .write(c, data.at(row).at(column).read(c));
            ram::g_player1_map_level(revision).write(c, level as i8);
            ram::g_player2_map_level(revision).write(c, level as i8);
        }
    }
    Ok(None)
}
