use crate::{Context, Result, call, ids};
use tje_mem::{arithmetic::expr_rem_u32, ram, rom};

/// G/src/map/land_processing.c:146 (`MapConsumePathCellDirections`).
/// ORIGINAL BUG: when no neighbor supplies an edge, shape retains entry D6.B;
/// matching assembly :463-540 assigns D6 only on the successful neighbor paths.
pub fn map_consume_path_cell_directions(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let bank = args[0] as i16;
    let row = i32::from(args[1] as i16);
    let column = i32::from(args[2] as i16);
    let map = if bank == 0 {
        ram::g_player1_level_map(r)
    } else {
        ram::g_player2_level_map(r)
    };
    let cell_address = map.cast::<u16>().offset(row * 19 + column);
    let mut cell = cell_address.read(c);
    if cell & 0x4000 == 0 {
        return Ok(Some(u32::MAX));
    }
    let slot = ram::g_land_path_traversal_direction(r);
    let mut finished = 0;
    let mut shape = c.data_register(6) as u8;
    let mut directions = call(
        c,
        ids::MAP_GET_BANK_PATH_DIRECTIONS,
        &[i32::from(bank) as u32, row as u32, column as u32],
    )? as u8;
    if directions == 0 {
        if slot.read(c) != -1 {
            finished = 1;
        }
        for (dy, dx, opposite, outgoing, boundary) in [
            (-1, 0, 4, 1, 16),
            (0, 1, 8, 2, 17),
            (1, 0, 1, 4, 18),
            (0, -1, 2, 8, 19),
        ] {
            directions = call(
                c,
                ids::MAP_GET_BANK_PATH_DIRECTIONS,
                &[
                    i32::from(bank) as u32,
                    (row + dy) as u32,
                    (column + dx) as u32,
                ],
            )? as u8;
            if slot.read(c) == opposite || directions & (opposite as u8) != 0 {
                if slot.read(c) == -1 {
                    slot.write(c, outgoing);
                }
                shape = boundary;
                break;
            }
        }
        let tables = rom::map_generation_lookup_tables(r);
        shape = if cell & 0x8000 != 0 {
            tables.edge_alternate().at(i32::from(shape)).read(c)
        } else {
            tables.edge_plain().at(i32::from(shape)).read(c)
        };
        directions = 0;
    } else {
        shape = call(
            c,
            ids::MAP_FIND_PATH_SHAPE_FOR_METATILE,
            &[u32::from(cell as u8)],
        )? as u8;
        let direction = i32::from(slot.read(c)) as u32;
        let opposite = call(c, ids::MAP_REVERSE_PATH_DIRECTION, &[direction])? as i8;
        if opposite != -1 {
            shape |= opposite as u8;
            directions &= !(opposite as u8);
        }
        if slot.read(c) == -1 || (slot.read(c) as u8) & directions == 0 {
            if directions & 1 != 0 {
                slot.write(c, 1);
            } else if directions & 2 != 0 {
                slot.write(c, 2);
            } else if directions & 4 != 0 {
                slot.write(c, 4);
            } else if directions & 8 != 0 {
                slot.write(c, 8);
            } else {
                if slot.read(c) == -1 {
                    return Ok(Some(u32::MAX));
                }
                finished = 1;
            }
        }
        if finished == 0 {
            shape |= slot.read(c) as u8;
        }
        directions &= !shape;
        shape = rom::map_generation_lookup_tables(r)
            .tiles()
            .at(i32::from(shape))
            .read(c);
        let mask = ram::g_map_path_direction_banks(r)
            .at_word_index(row as i16)
            .at_word_index(column as i16);
        if bank == 0 {
            mask.write(c, mask.read(c) & 0xf0);
            mask.write(c, mask.read(c) | directions);
        } else {
            mask.write(c, mask.read(c) & 15);
            mask.write(c, mask.read(c) | (directions << 4));
        }
    }
    let alternatives = rom::tile_alternative_pointers(r)
        .at(i32::from(shape))
        .read(c)
        .target();
    let random = super::random_generation::random(c)?;
    let alternative = expr_rem_u32(random, u32::from(alternatives.read(c)))?.wrapping_add(1) as i16;
    cell &= 0x9f00;
    cell |= u16::from(alternatives.offset(i32::from(alternative)).read(c));
    if directions != 0 {
        cell |= 0x4000;
    }
    cell |= 0x2000;
    cell_address.write(c, cell);
    Ok(Some(finished))
}
