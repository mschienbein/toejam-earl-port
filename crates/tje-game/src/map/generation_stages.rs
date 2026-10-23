use super::random_generation::random;
use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/map/level_generation_and_changes.c:435 (`MapGenerateSeparatedAnchors`).
pub fn map_generate_separated_anchors(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let anchors = ram::g_level_anchor_positions(r);
    for i in 0..8 {
        anchors.at(i).x().write(c, 0);
        anchors.at(i).y().write(c, 0);
    }
    let (min_x, max_x, min_y, max_y) = if ram::g_requested_map_level(r).read(c) < 4 {
        (5, 13, 8, 17)
    } else {
        (2, 16, 2, 23)
    };
    let count = call(c, ids::MAP_GET_RANDOM_GENERATION_PARAMETER, &[6])? as i16;
    let mut position = 0_i16;
    while position < count {
        let anchor = anchors.at_word_index(position);
        let x = (random(c)? % ((max_x - min_x + 1) as u32) + min_x as u32) as i8;
        anchor.x().write(c, x);
        let y = (random(c)? % ((max_y - min_y + 1) as u32) + min_y as u32) as i8;
        anchor.y().write(c, y);
        let mut previous = position.wrapping_sub(1);
        let mut rejected = false;
        while previous >= 0 {
            let prior = anchors.at_word_index(previous);
            if (i32::from(anchor.x().read(c)) - i32::from(prior.x().read(c))).abs() < 3
                && (i32::from(anchor.y().read(c)) - i32::from(prior.y().read(c))).abs() < 5
            {
                rejected = true;
                break;
            }
            previous = previous.wrapping_sub(1);
        }
        if !rejected {
            position = position.wrapping_add(1);
        }
    }
    Ok(Some(i32::from(count) as u32))
}

/// G/src/map/level_generation_and_changes.c:482 (`MapConnectAnchorPositions`).
/// Matching MOVEM D2-D6/A2-A3; the first two six-slot call areas remain
/// allocated together until the source's LEA 48(SP),SP.
pub fn map_connect_anchor_positions(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    use super::frame_support::{in_frame, release_arguments};
    use crate::call_frame::{CallSite, FrameLayout, OutgoingArguments};
    in_frame(c, FrameLayout::movem(0, 0x7c, 0x0c), |c, _frame| {
        let r = c.revision();
        let count = args[0] as i16;
        let anchors = ram::g_level_anchor_positions(r);
        let dispatcher = c.dispatcher();
        c.set_data_register(
            2,
            (c.data_register(2) & 0xffff0000) | u32::from(count as u16),
        );
        c.set_address_register(2, anchors.address());
        c.set_address_register(3, ids::GET_RANDOM_NUMBER.metadata().address(r));
        let a = anchors.at(0);
        let b = anchors.at(1);
        let values = [
            i32::from(a.y().read(c)) as u32,
            i32::from(a.x().read(c)) as u32,
            i32::from(b.y().read(c)) as u32,
            i32::from(b.x().read(c)) as u32,
            20,
            60,
        ];
        let first = OutgoingArguments::push(c, &values)?;
        let result = first.call(
            c,
            dispatcher,
            ids::MAP_CARVE_DIRECTIONAL_PATH,
            CallSite::relative(ids::MAP_CONNECT_ANCHOR_POSITIONS, r, 0x48),
        );
        if let Err(error) = result {
            return release_arguments(c, first, Err(error)).map(|_| None);
        }
        let a = anchors.at(1);
        let b = anchors.at(2);
        let values = [
            i32::from(a.y().read(c)) as u32,
            i32::from(a.x().read(c)) as u32,
            i32::from(b.y().read(c)) as u32,
            i32::from(b.x().read(c)) as u32,
            20,
            60,
        ];
        let second = match OutgoingArguments::push(c, &values) {
            Ok(area) => area,
            Err(error) => return release_arguments(c, first, Err(error)).map(|_| None),
        };
        let result = second.call(
            c,
            dispatcher,
            ids::MAP_CARVE_DIRECTIONAL_PATH,
            CallSite::relative(ids::MAP_CONNECT_ANCHOR_POSITIONS, r, 0x7e),
        );
        let result = release_arguments(c, second, result);
        release_arguments(c, first, result)?;
        let mut position = 0_i16;
        c.set_data_register(5, c.data_register(5) & 0xffff0000);
        while position < count {
            let anchor = anchors.at_word_index(position);
            let mut tx = ((random(c)? as i16) & 3) + 4;
            if i32::from(anchor.x().read(c)) + i32::from(tx) < 17 && random(c)? % 100 < 50 {
                tx = tx.wrapping_add(i16::from(anchor.x().read(c)));
            } else {
                if i32::from(anchor.x().read(c)) - i32::from(tx) <= 2 {
                    continue;
                }
                tx = (i32::from(anchor.x().read(c)) - i32::from(tx)) as i16;
            }
            let mut ty = ((random(c)? as i16) & 3) + 5;
            let right = if i32::from(anchor.y().read(c)) + i32::from(ty) < 24 {
                let value = random(c)? as u16;
                c.set_data_register(6, (c.data_register(6) & 0xffff0000) | u32::from(value));
                u32::from(value) % 100 < 50
            } else {
                false
            };
            if right {
                ty = ty.wrapping_add(i16::from(anchor.y().read(c)));
            } else {
                if i32::from(anchor.y().read(c)) - i32::from(ty) <= 2 {
                    continue;
                }
                ty = (i32::from(anchor.y().read(c)) - i32::from(ty)) as i16;
            }
            c.set_data_register(3, i32::from(ty) as u32);
            c.set_data_register(4, (c.data_register(4) & 0xffff0000) | u32::from(tx as u16));
            c.set_data_register(
                5,
                (c.data_register(5) & 0xffff0000) | u32::from(position as u16),
            );
            let values = [
                i32::from(anchor.y().read(c)) as u32,
                i32::from(anchor.x().read(c)) as u32,
                i32::from(ty) as u32,
                i32::from(tx) as u32,
                1,
                0,
            ];
            crate::call_at(
                c,
                ids::MAP_CARVE_DIRECTIONAL_PATH,
                &values,
                CallSite::relative(ids::MAP_CONNECT_ANCHOR_POSITIONS, r, 0x1c6),
            )?;
            position = position.wrapping_add(1);
            c.set_data_register(
                5,
                (c.data_register(5) & 0xffff0000) | u32::from(position as u16),
            );
        }
        Ok(None)
    })
}

/// G/src/map/random_regions_and_cleanup.c:674 (`MapBuildRegionEdgeShapes`).
pub fn map_build_region_edge_shapes(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let count = args[0] as i16;
    let map = ram::g_player1_level_map(r);
    for region in 0..i32::from(count) {
        let chance = call(c, ids::MAP_GET_RANDOM_GENERATION_PARAMETER, &[5])? as i16;
        // Matching asm :2583-2584 consumes signed CMP.L/SGT flags.
        let upper = i32::from(chance) > (random(c)? % 100) as i32;
        for row in 0..26 {
            for column in 0..19 {
                let cell = map.at(row).at(column);
                let value = cell.read(c);
                if (value & 0x3000 == 0x1000 || value & 0x3000 == 0x3000)
                    && region == i32::from((value >> 8) & 7)
                {
                    cell.write(c, cell.read(c) | 0x800);
                    if upper {
                        cell.write(c, cell.read(c) | 0x8000);
                    }
                } else {
                    cell.write(c, cell.read(c) & 0xf7ff);
                }
            }
        }
        for row in 0..26 {
            for column in 0..19 {
                let cell = map.at(row).at(column);
                let value = cell.read(c);
                if value & 0x800 != 0 {
                    let mask = super::pipeline::corners(c, row, column);
                    if value & 0x3000 == 0x1000 {
                        let tables = rom::map_generation_lookup_tables(r);
                        let tile = if upper {
                            tables.edge_alternate().at(i32::from(mask)).read(c)
                        } else {
                            tables.edge_plain().at(i32::from(mask)).read(c)
                        };
                        cell.write(c, cell.read(c) | u16::from(tile));
                    }
                }
            }
        }
    }
    Ok(None)
}

/// G/src/map/random_regions_and_cleanup.c:630 (`MapSeedFilledTerrainRegions`).
/// Matching LINK -8, MOVEM D2-D6/A2-A3; Grow receives its true outgoing area.
pub fn map_seed_filled_terrain_regions(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    use crate::call_frame::{CallSite, FrameLayout};
    super::frame_support::in_frame(c, FrameLayout::link(8, 0x7c, 0x0c), |c, frame| {
        let r = c.revision();
        let map = ram::g_player1_level_map(r);
        let anchors = ram::g_level_anchor_positions(r);
        let row = frame.local::<i16>(-2);
        let cell_value = frame.local::<i16>(-6);
        let eligible = frame.local::<i16>(-4);
        let probability = frame.local::<u16>(-8);
        c.set_address_register(2, anchors.address());
        c.set_address_register(3, map.address());
        row.write(c, 0);
        while row.read(c) < 26 {
            for column in 0..19 {
                let cell = map.at_word_index(row.read(c)).at_word_index(column);
                cell.write(c, cell.read(c) & 0xf7ff);
            }
            row.write(c, row.read(c).wrapping_add(1));
        }
        let value = call(c, ids::MAP_GET_RANDOM_GENERATION_PARAMETER, &[4])? as u16;
        probability.write(c, value);
        row.write(c, 0);
        while row.read(c) < 23 {
            let mut column = 0_i16;
            while column < 16 {
                let value = map.at_word_index(row.read(c)).at_word_index(column).read(c) as i16;
                cell_value.write(c, value);
                if value & 0x3000 == 0x1000 {
                    eligible.write(c, 1);
                    let replacement = (value & 0x700) | 0x3000;
                    let mut sy = row.read(c);
                    let mut sx;
                    while i32::from(sy) < i32::from(row.read(c)) + 4 {
                        sx = column;
                        while i32::from(sx) < i32::from(column) + 4 {
                            let tile = map.at_word_index(sy).at_word_index(sx).read(c) as i16;
                            let mut rejected = tile != cell_value.read(c) && tile != replacement;
                            if !rejected {
                                for i in 0..3 {
                                    let anchor = anchors.at(i);
                                    if i16::from(anchor.y().read(c)) == sy
                                        && i16::from(anchor.x().read(c)) == sx
                                    {
                                        rejected = true;
                                        break;
                                    }
                                }
                            }
                            if rejected {
                                eligible.write(c, 0);
                                sy = (i32::from(row.read(c)) + 4) as i16;
                                sx = (i32::from(column) + 4) as i16;
                            }
                            sx = sx.wrapping_add(1);
                        }
                        sy = sy.wrapping_add(1);
                    }
                    if eligible.read(c) != 0 {
                        let chance = u32::from(probability.read(c));
                        c.set_data_register(2, chance);
                        if chance > random(c)? % 100 {
                            c.set_data_register(
                                3,
                                (c.data_register(3) & 0xffff0000)
                                    | u32::from(column.wrapping_add(4) as u16),
                            );
                            c.set_data_register(
                                4,
                                (c.data_register(4) & 0xffff0000) | u32::from(sy as u16),
                            );
                            c.set_data_register(
                                5,
                                (c.data_register(5) & 0xffff0000) | u32::from(column as u16),
                            );
                            c.set_data_register(
                                6,
                                (c.data_register(6) & 0xffff0000) | u32::from(replacement as u16),
                            );
                            let values = [
                                0x3000,
                                i32::from((cell_value.read(c) >> 8) & 7) as u32,
                                0,
                                (i32::from(row.read(c)) + 1) as u32,
                                (i32::from(column) + 1) as u32,
                            ];
                            crate::call_at(
                                c,
                                ids::MAP_GROW_TERRAIN_REGION,
                                &values,
                                CallSite::relative(ids::MAP_SEED_FILLED_TERRAIN_REGIONS, r, 0x19c),
                            )?;
                        }
                    }
                }
                column = column.wrapping_add(1);
            }
            row.write(c, row.read(c).wrapping_add(1));
        }
        Ok(None)
    })
}

/// G/src/map/level_generation_and_changes.c:208 (`MapScatterTerrainFeatures`).
/// The parameter-7 call precedes the comparison's RNG draw, as emitted at
/// G/build/rev02/asm/c_level_generation_and_changes.s:1360-1364.
pub fn map_scatter_terrain_features(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let map = ram::g_player1_level_map(r);
    let directions = ram::g_map_path_direction_banks(r);
    for row in 0..26 {
        for column in 0..19 {
            if directions.at(row).at(column).read(c) & 15 != 0 {
                continue;
            }
            let shape = map.at(row).at(column).read(c) & 0xff;
            if shape == 0 {
                if row == 0 {
                    continue;
                }
                let upper = map.at(row - 1).at(column).read(c);
                if matches!(upper, 27 | 45) || column == 0 {
                    continue;
                }
                let left = map.at(row).at(column - 1).read(c);
                if matches!(left, 27 | 45) {
                    continue;
                }
                let mut plain = false;
                let mut alternate = false;
                let mut feature = false;
                for sy in row - 3..row + 3 {
                    for sx in column - 2..column + 2 {
                        if (0..26).contains(&sy) && (0..19).contains(&sx) {
                            let tile = map.at(sy).at(sx).read(c);
                            if tile & 0x3000 == 0x1000 {
                                if tile & 0x8000 != 0 {
                                    alternate = true;
                                } else {
                                    plain = true;
                                }
                            }
                            if matches!(tile, 27 | 45) {
                                feature = true;
                            }
                        }
                    }
                }
                if (alternate || plain)
                    && ((feature && random(c)? % 100 < 18) || random(c)? % 100 < 7)
                {
                    let chance = if alternate { 70 } else { 20 };
                    let block = row < 25
                        && column < 18
                        && random(c)? % 100 < 50
                        && map.at(row).at(column + 1).read(c) == 0
                        && map.at(row + 1).at(column).read(c) == 0
                        && map.at(row + 1).at(column + 1).read(c) == 0;
                    if chance > random(c)? % 100 {
                        if block {
                            map.at(row).at(column).write(c, 0x801e);
                            map.at(row).at(column + 1).write(c, 0x801f);
                            map.at(row + 1).at(column).write(c, 0x801d);
                            map.at(row + 1).at(column + 1).write(c, 0x8020);
                        } else {
                            map.at(row).at(column).write(c, 45);
                        }
                    } else if block {
                        map.at(row).at(column).write(c, 2);
                        map.at(row).at(column + 1).write(c, 3);
                        map.at(row + 1).at(column).write(c, 1);
                        map.at(row + 1).at(column + 1).write(c, 4);
                    } else {
                        map.at(row).at(column).write(c, 27);
                    }
                }
            } else if shape == 46 {
                if random(c)? % 100 < 10 {
                    map.at(row).at(column).write(c, 0x902f);
                }
            } else {
                let probability = call(c, ids::MAP_GET_RANDOM_GENERATION_PARAMETER, &[7])?;
                if random(c)? % 100 < probability {
                    match shape {
                        75 => map.at(row).at(column).write(c, 73),
                        76 => map.at(row).at(column).write(c, 74),
                        80 => map.at(row).at(column).write(c, 77),
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(None)
}
