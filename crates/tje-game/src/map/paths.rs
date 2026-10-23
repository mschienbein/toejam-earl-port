use crate::{Context, Result, call, ids};
use tje_mem::{Address, ram, rom};

/// G/src/map/transition_paths.c:10 (`MapTransitionContainsMetatile`).
/// A zero list number keeps the original unsigned-underflow descriptor address.
pub fn map_transition_contains_metatile(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let list_number = u32::from(args[0] as u8).wrapping_sub(1);
    let list = rom::map_transition_descriptors(c.revision())
        .at(list_number as i32)
        .shapes()
        .read(c)
        .target();
    let value = args[1] as u8;
    let mut index = 1_i16;
    while u32::from(index as u16) <= u32::from(list.read(c)) {
        if list.offset(i32::from(index)).read(c) == value {
            return Ok(Some(1));
        }
        index = index.wrapping_add(1);
    }
    Ok(Some(0))
}

/// G/src/map/map_population.c:12 (`MapFindRouteToSecondAnchor`).
/// Visited stores precede the anchor test; traversal is up/down/left/right.
/// Path-class cells deliberately have no coordinate bounds checks.
pub fn map_find_route_to_second_anchor(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let row = args[0] as i16;
    let column = args[1] as i16;
    let revision = c.revision();
    let base = ram::g_player1_level_map(revision).cast::<u16>();
    // Direct short subscripts use MULU/ADDA.W and doubled word indexing.
    let cell = base
        .byte_offset(i32::from(row.wrapping_mul(38)))
        .byte_offset(i32::from(column.wrapping_mul(2)));
    if cell.read(c) & 0x0800 != 0 {
        return Ok(Some(0));
    }
    cell.write(c, cell.read(c) | 0x0800);
    let anchor = ram::g_level_anchor_positions(revision).at(1);
    if i16::from(anchor.y().read(c)) == row && i16::from(anchor.x().read(c)) == column {
        return Ok(Some(1));
    }
    let boundary = cell.read(c) & 0x3000 == 0x1000;
    if !boundary && cell.read(c) & 0x3000 != 0x2000 {
        return Ok(Some(0));
    }
    for (dy, dx, outgoing, incoming, in_bounds) in [
        (-1, 0, 1, 4, row > 0),
        (1, 0, 4, 1, row < 25),
        (0, -1, 8, 2, column > 0),
        (0, 1, 2, 8, column < 18),
    ] {
        // An arithmetic expression subscript is long, unlike the unchanged axis.
        let next_row = i32::from(row) + dy;
        let next_column = i32::from(column) + dx;
        let row_bytes = if dy == 0 {
            i32::from(row.wrapping_mul(38))
        } else {
            next_row * 38
        };
        let column_bytes = if dx == 0 {
            i32::from(column.wrapping_mul(2))
        } else {
            next_column * 2
        };
        let neighbor = base.byte_offset(row_bytes).byte_offset(column_bytes);
        let eligible = if boundary {
            in_bounds
                && (neighbor.read(c) == (cell.read(c) & !0x0800)
                    || neighbor.read(c) & incoming != 0)
        } else {
            cell.read(c) & outgoing != 0
                && (neighbor.read(c) & 0x3000 == 0x1000 || neighbor.read(c) & incoming != 0)
        };
        if eligible
            && call(
                c,
                ids::MAP_FIND_ROUTE_TO_SECOND_ANCHOR,
                &[next_row as u32, next_column as u32],
            )? != 0
        {
            return Ok(Some(1));
        }
    }
    Ok(Some(0))
}

/// G/src/map/land_processing.c:100 (`MapReversePathDirection`).
/// Non-cardinal values other than -1 return up, not a bitwise reversed mask.
pub fn map_reverse_path_direction(_: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let result = match args[0] as i32 {
        -1 => -1_i32,
        2 => 8,
        8 => 2,
        1 => 4,
        _ => 1,
    };
    Ok(Some(result as u32))
}

/// G/src/map/land_processing.c:117 (`MapFindPathShapeForMetatile`).
/// Both lookup banks remain cartridge-backed; 16 is the not-found result.
pub fn map_find_path_shape_for_metatile(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let revision = c.revision();
    let metatile = args[0] as u8;
    for shape in 0..16 {
        let base_shape = rom::map_generation_lookup_tables(revision)
            .tiles()
            .at(shape)
            .read(c);
        let alternatives = rom::tile_alternative_pointers(revision)
            .at(i32::from(base_shape))
            .read(c)
            .target();
        let mut alternative = 1_i16;
        while u32::from(alternative as u16) <= u32::from(alternatives.read(c)) {
            if metatile == alternatives.offset(i32::from(alternative)).read(c) {
                return Ok(Some(shape as u32));
            }
            alternative = alternative.wrapping_add(1);
        }
    }
    Ok(Some(16))
}

/// G/src/map/land_processing.c:135 (`MapGetBankPathDirections`).
/// The emitted word row product is retained even for out-of-range short inputs.
pub fn map_get_bank_path_directions(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let bank = args[0] as i16;
    let row = args[1] as i16;
    let column = args[2] as i16;
    let address = ram::g_map_path_direction_banks(c.revision())
        .cast::<u8>()
        .byte_offset(i32::from(row.wrapping_mul(19)))
        .byte_offset(i32::from(column));
    let directions = address.read(c);
    Ok(Some(u32::from(if bank == 0 {
        directions & 15
    } else {
        directions >> 4
    })))
}

/// G/src/map/land_processing.c:509 (`PlayerIsNearTerrainEdge`).
pub fn player_is_near_terrain_edge(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let terrain: Address<u8> = if args[0] as i16 == 0 {
        ram::g_player1_nearby_terrain(c.revision()).cast()
    } else {
        ram::g_player2_nearby_terrain(c.revision()).cast()
    };
    Ok(Some(u32::from(
        terrain.offset(7).read(c) >= 0x50
            || terrain.offset(11).read(c) >= 0x50
            || terrain.offset(13).read(c) >= 0x50
            || terrain.offset(17).read(c) >= 0x50,
    )))
}
