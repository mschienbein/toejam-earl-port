use crate::{Context, Result};
use tje_mem::{Address, ram};

/// G/src/map/random_regions_and_cleanup.c:234 (`MapFindInteriorGrowthOrigin`).
/// A failed search leaves both output words untouched; all three anchors block growth.
pub fn map_find_interior_growth_origin(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let row_position = Address::<i16>::new(args[0]);
    let column_position = Address::<i16>::new(args[1]);
    let direction = args[2] as i16;
    let region = args[3] as i16;
    let mut row = row_position.read(c);
    let mut column = column_position.read(c);
    let map = ram::g_player1_level_map(c.revision());
    let anchors = ram::g_level_anchor_positions(c.revision());
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
        if row < 1 || column < 1 || row >= 24 || column >= 17 {
            return Ok(Some(0));
        }
        for index in 0..3 {
            let anchor = anchors.at(index);
            if i32::from(anchor.x().read(c)) <= i32::from(column) + 1
                && i16::from(anchor.x().read(c)) >= column
                && i32::from(anchor.y().read(c)) <= i32::from(row) + 1
                && i16::from(anchor.y().read(c)) >= row
            {
                return Ok(Some(0));
            }
        }
        for row_offset in -1..3 {
            for column_offset in -1..3 {
                let tile = map
                    .at(i32::from(row) + row_offset)
                    .at(i32::from(column) + column_offset)
                    .read(c) as i16;
                if tile != region && i32::from(tile) != ((i32::from(region) & 0x0700) | 0x1000) {
                    return Ok(Some(0));
                }
            }
        }
        for row_offset in 0..2 {
            for column_offset in 0..2 {
                if map
                    .at(i32::from(row) + row_offset)
                    .at(i32::from(column) + column_offset)
                    .read(c)
                    != region as u16
                {
                    row_position.write(c, row);
                    column_position.write(c, column);
                    return Ok(Some(1));
                }
            }
        }
    }
}

/// MCC68K's word row product and doubled word column for a direct short subscript.
/// G/build/rev02/asm/c_random_regions_and_cleanup.s:1302-1308.
pub(super) fn short_cell(base: Address<[[u16; 19]; 26]>, row: i16, column: i16) -> Address<u16> {
    base.at_word_index(row).at_word_index(column)
}

/// G/src/map/random_regions_and_cleanup.c:413 (`MapFillRectangleAndMergeRegions`).
/// Short loop variables wrap, but their comparison bounds are promoted sums.
pub fn map_fill_rectangle_and_merge_regions(
    c: &mut dyn Context,
    args: &[u32],
) -> Result<Option<u32>> {
    let start_row = args[0] as i16;
    let start_column = args[1] as i16;
    let height = args[2] as i16;
    let width = args[3] as i16;
    let replacement = args[4] as i16;
    let map = ram::g_player1_level_map(c.revision());
    let mut row = start_row;
    while i32::from(row) < i32::from(start_row) + i32::from(height) {
        let mut column = start_column;
        while i32::from(column) < i32::from(start_column) + i32::from(width) {
            let cell = short_cell(map, row, column);
            let tile = cell.read(c) as i16;
            if tile & 0x3000 == 0x1000 && tile != replacement {
                for map_row in 0..26 {
                    for map_column in 0..19 {
                        let other = map.at(map_row).at(map_column);
                        if other.read(c) == tile as u16 {
                            other.write(c, replacement as u16);
                        }
                    }
                }
            } else {
                cell.write(c, replacement as u16);
            }
            column = column.wrapping_add(1);
        }
        row = row.wrapping_add(1);
    }
    Ok(None)
}

/// G/src/map/random_regions_and_cleanup.c:484 (`MapWidenHorizontalRegionEdges`).
/// Nonqualifying cells reset the counter to one without clearing the previous label.
pub fn map_widen_horizontal_region_edges(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let map = ram::g_player1_level_map(c.revision());
    for row in 0..25 {
        let mut run_length = 0_i16;
        let mut previous_terrain = 0xffff;
        for column in 0..19 {
            let terrain = map.at(row).at(column).read(c);
            if terrain & 0x3000 == 0x1000
                && (row == 0 || map.at(row - 1).at(column).read(c) & 0x3000 != 0x1000)
            {
                if terrain == previous_terrain {
                    run_length = run_length.wrapping_add(1);
                } else {
                    previous_terrain = terrain;
                    run_length = 1;
                }
            } else {
                run_length = 1;
            }
            if run_length >= 4 {
                if row > 0 {
                    map.at(row - 1).at(column - 2).write(c, terrain);
                    map.at(row - 1).at(column - 1).write(c, terrain);
                    run_length = 1;
                } else if run_length >= 5 {
                    map.at(row).at(column - 2).write(c, 0);
                    run_length = 2;
                }
            }
        }
    }
    for row in 1..26 {
        let mut run_length = 0_i16;
        let mut previous_terrain = 0xffff;
        for column in 0..19 {
            let terrain = map.at(row).at(column).read(c);
            if terrain & 0x3000 == 0x1000
                && (row == 25 || map.at(row + 1).at(column).read(c) & 0x3000 != 0x1000)
            {
                if terrain == previous_terrain {
                    run_length = run_length.wrapping_add(1);
                } else {
                    previous_terrain = terrain;
                    run_length = 1;
                }
            } else {
                run_length = 1;
            }
            if run_length >= 4 {
                if row < 25 {
                    map.at(row + 1).at(column - 2).write(c, terrain);
                    map.at(row + 1).at(column - 1).write(c, terrain);
                    run_length = 1;
                } else if run_length >= 5 {
                    map.at(row).at(column - 2).write(c, 0);
                    run_length = 2;
                }
            }
        }
    }
    Ok(None)
}

/// G/src/map/random_regions_and_cleanup.c:547 (`MapWidenVerticalRegionEdges`).
/// The two passes retain the same reset-to-one quirk as the horizontal function.
pub fn map_widen_vertical_region_edges(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let map = ram::g_player1_level_map(c.revision());
    for column in 0..18 {
        let mut run_length = 0_i16;
        let mut previous_terrain = 0xffff;
        for row in 0..26 {
            let terrain = map.at(row).at(column).read(c);
            if terrain & 0x3000 == 0x1000
                && (column == 0 || map.at(row).at(column - 1).read(c) & 0x3000 != 0x1000)
            {
                if terrain == previous_terrain {
                    run_length = run_length.wrapping_add(1);
                } else {
                    previous_terrain = terrain;
                    run_length = 1;
                }
            } else {
                run_length = 1;
            }
            if run_length >= 4 {
                if column > 0 {
                    map.at(row - 2).at(column - 1).write(c, terrain);
                    map.at(row - 1).at(column - 1).write(c, terrain);
                    run_length = 1;
                } else if run_length >= 5 {
                    map.at(row - 2).at(column).write(c, 0);
                    run_length = 2;
                }
            }
        }
    }
    for column in 1..19 {
        let mut run_length = 0_i16;
        let mut previous_terrain = 0xffff;
        for row in 0..26 {
            let terrain = map.at(row).at(column).read(c);
            if terrain & 0x3000 == 0x1000
                && (column == 18 || map.at(row).at(column + 1).read(c) & 0x3000 != 0x1000)
            {
                if terrain == previous_terrain {
                    run_length = run_length.wrapping_add(1);
                } else {
                    previous_terrain = terrain;
                    run_length = 1;
                }
            } else {
                run_length = 1;
            }
            if run_length >= 4 {
                if column < 18 {
                    map.at(row - 2).at(column + 1).write(c, terrain);
                    map.at(row - 1).at(column + 1).write(c, terrain);
                    run_length = 1;
                } else if run_length >= 5 {
                    map.at(row - 2).at(column).write(c, 0);
                    run_length = 2;
                }
            }
        }
    }
    Ok(None)
}
