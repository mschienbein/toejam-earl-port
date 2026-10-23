use super::random_generation::random;
use crate::{Context, Result, call, call_frame::FrameLayout, ids};
use tje_mem::{ram, rom};

/// G/src/map/random_regions_and_cleanup.c:293 (`MapGrowTerrainRegion`).
/// Matching LINK -16, MOVEM D2-D6/A2-A3. In particular row/column are writable
/// incoming argument words, and their exact addresses escape to the searches.
pub fn map_grow_terrain_region(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    super::frame_support::in_frame(c, FrameLayout::link(16, 0x7c, 0x0c), |c, frame| {
        let r = c.revision();
        let map = ram::g_player1_level_map(r);
        let kind = args[0] as i16;
        let number = args[1] as i16;
        let minimum = args[2] as i16;
        let row = frame.argument_word(3);
        let column = frame.argument_word(4);
        let region = frame.local::<u16>(-4);
        let remaining = frame.local::<i16>(-6);
        let direction = frame.local::<i16>(-8);
        let preferred = frame.local::<i16>(-10);
        let join = frame.local::<u16>(-12);
        let stop = frame.local::<u16>(-14);
        let row_offset = frame.local::<i16>(-2);
        let count = frame.local::<i16>(-16);
        let initial = 1_i16 << (random(c)? & 3);
        direction.write(c, initial);
        region.write(c, (i32::from(number) << 8) as u16);
        region.write(c, region.read(c) | kind as u16);
        if kind == 0x1000 {
            let value = call(c, ids::MAP_GET_RANDOM_GENERATION_PARAMETER, &[0])?;
            stop.write(c, value as u16);
            let value = call(c, ids::MAP_GET_RANDOM_GENERATION_PARAMETER, &[2])?;
            join.write(c, value as u16);
        } else if kind == 0x3000 {
            let value = call(c, ids::MAP_GET_RANDOM_GENERATION_PARAMETER, &[1])?;
            stop.write(c, value as u16);
        }
        remaining.write(c, 0);
        loop {
            if kind == 0x1000 {
                row_offset.write(c, 0);
                while row_offset.read(c) < 2 {
                    let mut col_offset = 0_i16;
                    while col_offset < 2 {
                        let tile = map
                            .at(i32::from(row.read(c)) + i32::from(row_offset.read(c)))
                            .at(i32::from(column.read(c)) + i32::from(col_offset))
                            .read(c);
                        if tile & 0x3000 == 0x1000 && tile != region.read(c) {
                            row_offset.write(c, 0);
                            while row_offset.read(c) < 26 {
                                col_offset = 0;
                                while col_offset < 19 {
                                    let cell = map
                                        .at_word_index(row_offset.read(c))
                                        .at_word_index(col_offset);
                                    if tile == cell.read(c) {
                                        cell.write(c, region.read(c));
                                    }
                                    col_offset = col_offset.wrapping_add(1);
                                }
                                row_offset.write(c, row_offset.read(c).wrapping_add(1));
                            }
                            col_offset = 2;
                            row_offset.write(c, 2);
                        }
                        col_offset = col_offset.wrapping_add(1);
                    }
                    row_offset.write(c, row_offset.read(c).wrapping_add(1));
                }
            }
            map.at_word_index(row.read(c))
                .at_word_index(column.read(c))
                .write(c, region.read(c));
            map.at_word_index(row.read(c))
                .at(i32::from(column.read(c)) + 1)
                .write(c, region.read(c));
            map.at(i32::from(row.read(c)) + 1)
                .at_word_index(column.read(c))
                .write(c, region.read(c));
            map.at(i32::from(row.read(c)) + 1)
                .at(i32::from(column.read(c)) + 1)
                .write(c, region.read(c));
            if remaining.read(c) <= 0 {
                let value = (random(c)? & 1) as i16;
                remaining.write(c, value);
                if kind == 0x3000 {
                    remaining.write(c, remaining.read(c).wrapping_add(1));
                }
                let index = (random(c)? % 3 + 1).wrapping_add(i32::from(direction.read(c)) as u32);
                preferred.write(
                    c,
                    i16::from(rom::map_path_turn_directions(r).at(index as i32).read(c)),
                );
            }
            if random(c)? % 100 < 80 {
                direction.write(c, preferred.read(c));
            }
            let mut tried = 0_i16;
            loop {
                let found = if kind == 0x1000 {
                    let values = [
                        row.address(),
                        column.address(),
                        i32::from(direction.read(c)) as u32,
                        u32::from(region.read(c)),
                        u32::from(join.read(c)),
                    ];
                    call(c, ids::MAP_FIND_REGION_GROWTH_ORIGIN, &values)?
                } else {
                    let values = [
                        row.address(),
                        column.address(),
                        i32::from(direction.read(c)) as u32,
                        u32::from(region.read(c)),
                    ];
                    call(c, ids::MAP_FIND_INTERIOR_GROWTH_ORIGIN, &values)?
                };
                if found != 0 {
                    break;
                }
                tried |= direction.read(c);
                if tried == 15 {
                    return Ok(None);
                }
                let next = loop {
                    let index =
                        (random(c)? % 3 + 1).wrapping_add(i32::from(direction.read(c)) as u32);
                    let next = i16::from(rom::map_path_turn_directions(r).at(index as i32).read(c));
                    if tried & next == 0 {
                        break next;
                    }
                };
                direction.write(c, next);
            }
            remaining.write(c, remaining.read(c).wrapping_sub(1));
            let chance = u32::from(stop.read(c));
            if chance > random(c)? % 100 {
                count.write(c, 0);
                if minimum > 0 {
                    row_offset.write(c, 0);
                    while row_offset.read(c) < 26 {
                        for col_offset in 0..19 {
                            if kind as u16
                                == map
                                    .at_word_index(row_offset.read(c))
                                    .at_word_index(col_offset)
                                    .read(c)
                                    & 0x3000
                            {
                                count.write(c, count.read(c).wrapping_add(1));
                            }
                        }
                        row_offset.write(c, row_offset.read(c).wrapping_add(1));
                    }
                }
                if count.read(c) >= minimum {
                    break;
                }
            }
        }
        Ok(None)
    })
}
