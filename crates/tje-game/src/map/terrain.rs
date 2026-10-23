use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/map/terrain_at_position.c:17 (`GetMapTerrainAtWorldPosition`).
/// Negative map indices deliberately bypass the original upper/zero bounds test.
pub fn get_map_terrain_at_world_position(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let player = args[0] as i16;
    let x = args[1] as i16;
    let y = args[2] as i16;
    let (map, level) = if player == 0 {
        (ram::g_player1_level_map(r), ram::g_player1_map_level(r))
    } else {
        (ram::g_player2_level_map(r), ram::g_player2_map_level(r))
    };
    let special = matches!(level.read(c), 1 | 26);
    let tile_y = (y >> 3) & 7;
    let tile_x = (x >> 3) & 15;
    let mut map_y = y >> 6;
    let mut map_x = x >> 7;
    if map_x == 0 || map_x > 19 || map_y == 0 || map_y > 26 {
        return Ok(Some(if special { 0x80 } else { 0x90 }));
    }
    map_x = map_x.wrapping_sub(1);
    map_y = map_y.wrapping_sub(1);
    let cell = map
        .cast::<u16>()
        .offset(i32::from(map_y) * 19 + i32::from(map_x))
        .read(c);
    ram::g_terrain_query_metatile_word(r).write(c, cell);
    let base = u32::from(ram::g_terrain_query_metatile_low_byte(r).read(c));
    let tile = call(
        c,
        ids::GET_LEVEL_METATILE_TILE,
        &[base, i32::from(tile_y) as u32, i32::from(tile_x) as u32],
    )? as u16;
    Ok(Some(u32::from(
        rom::player_tile_property_lookup(r)
            .at(i32::from(tile & 0x1ff))
            .read(c)
            & 0xf0,
    )))
}

/// G/src/map/terrain_at_position.c:54 (`UpdateLandAroundPlayer`).
pub fn update_land_around_player(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let p = i32::from(args[0] as i16);
    if ram::g_player_lives(r).at(p).read(c) < 0
        || ram::g_player_inventory_open(r).at(p).read(c) != 0
    {
        return Ok(None);
    }
    let mut map = ram::g_player1_level_map(r);
    let (entity, mut land) = if p == 0 {
        (
            ram::g_player_info(r).at(0),
            ram::g_player1_nearby_terrain(r).cast::<u8>(),
        )
    } else {
        if ram::g_player_lives(r).at(0).read(c) >= 0 && ram::g_split_screen_enabled(r).read(c) != 0
        {
            map = ram::g_player2_level_map(r);
        }
        (
            ram::g_player2_entity(r),
            ram::g_player2_nearby_terrain(r).cast::<u8>(),
        )
    };
    let cy = entity.y().read(c) >> 3;
    let cx = entity.x().read(c) >> 3;
    let mut sy = cy.wrapping_sub(2);
    while i32::from(sy) < i32::from(cy) + 3 {
        let mut sx = cx.wrapping_sub(2);
        while i32::from(sx) < i32::from(cx) + 3 {
            let terrain = if call(
                c,
                ids::PRESENT_FIND_DOORWAY_OWNER_AT_TILE,
                &[entity.address(), i32::from(sy) as u32, i32::from(sx) as u32],
            )? != 0
            {
                0x10
            } else {
                let ty = sy & 7;
                let tx = sx & 15;
                let mut my = sy >> 3;
                let mut mx = sx >> 4;
                if mx == 0 || mx > 19 || my == 0 || my > 26 {
                    if matches!(entity.level().read(c), 1 | 26) {
                        0x80
                    } else {
                        0x90
                    }
                } else {
                    mx = mx.wrapping_sub(1);
                    my = my.wrapping_sub(1);
                    let cell = map
                        .cast::<u16>()
                        .offset(i32::from(my) * 19 + i32::from(mx))
                        .read(c);
                    let overlay = (cell & 0x1f00) >> 8;
                    let mut tile = 0;
                    if overlay != 0 {
                        tile = call(
                            c,
                            ids::GET_OVERLAY_METATILE_TILE,
                            &[
                                u32::from(overlay),
                                i32::from(ty) as u32,
                                i32::from(tx) as u32,
                            ],
                        )? as u16;
                    }
                    if tile == 0 {
                        tile = call(
                            c,
                            ids::GET_LEVEL_METATILE_TILE,
                            &[
                                u32::from(cell as u8),
                                i32::from(ty) as u32,
                                i32::from(tx) as u32,
                            ],
                        )? as u16;
                    }
                    rom::player_tile_property_lookup(r)
                        .at(i32::from(tile & 0x7ff))
                        .read(c)
                }
            };
            land.write(c, terrain);
            land = land.offset(1);
            sx = sx.wrapping_add(1);
        }
        sy = sy.wrapping_add(1);
    }
    Ok(None)
}

/// G/src/map/terrain_placement.c:13 (`TerrainValidation`).
pub fn terrain_validation(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    use tje_mem::{
        Address,
        arithmetic::{expr_rem_u32, rem_i32},
    };
    let r = c.revision();
    let mut metatile = args[0] as u16;
    let width = args[1] as i16;
    let height = args[2] as i16;
    let px = Address::<i16>::new(args[3]);
    let py = Address::<i16>::new(args[4]);
    let flags = args[5] as u8;
    if metatile & 0x1f00 != 0 && flags & 4 == 0 {
        return Ok(Some(0));
    }
    let mirrored = metatile & 0x80 != 0;
    metatile &= 0x7f;
    if matches!(metatile, 34 | 82 | 84) {
        return Ok(Some(0));
    }
    if width >= 16 && height >= 8 {
        if (flags & 1 != 0 && matches!(metatile, 30 | 33))
            || (flags & 8 != 0 && matches!(metatile, 75 | 76 | 80))
            || (flags & 2 != 0 && (metatile == 53 || (56..=58).contains(&metatile)))
        {
            px.write(c, 63);
            py.write(c, 31);
            return Ok(Some(1));
        }
        return Ok(Some(0));
    }
    let sy = expr_rem_u32(
        super::random_generation::random(c)?,
        (9 - i32::from(height)) as u32,
    )? as i16;
    let sx = expr_rem_u32(
        super::random_generation::random(c)?,
        (17 - i32::from(width)) as u32,
    )? as i16;
    let mut oy = sy;
    loop {
        let mut ox = sx;
        loop {
            let mut valid = true;
            let mut row = oy;
            while i32::from(row) < i32::from(oy) + i32::from(height) {
                let mut column = ox;
                while i32::from(column) < i32::from(ox) + i32::from(width) {
                    let tile = rom::level_metatile_maps(r)
                        .at(i32::from(metatile))
                        .at(i32::from(row))
                        .at(i32::from(column))
                        .read(c);
                    let property = rom::player_tile_property_lookup(r)
                        .at(i32::from(tile & 0x7ff))
                        .read(c);
                    if !((flags & 1 != 0 && property == 0x10)
                        || (flags & 2 != 0 && property == 0x20)
                        || (flags & 8 != 0 && property == 0x80))
                    {
                        valid = false;
                        break;
                    }
                    column = column.wrapping_add(1);
                }
                if !valid {
                    break;
                }
                row = row.wrapping_add(1);
            }
            if valid {
                py.write(c, (i32::from(oy) * 8 + i32::from(height) * 8 - 2) as i16);
                px.write(c, (i32::from(ox) * 8 + i32::from(width) * 4) as i16);
                if mirrored {
                    px.write(c, (128 - i32::from(px.read(c))) as i16);
                }
                return Ok(Some(1));
            }
            ox = rem_i32(i32::from(ox) + 1, 17 - i32::from(width))? as i16;
            if ox == sx {
                break;
            }
        }
        oy = rem_i32(i32::from(oy) + 1, 9 - i32::from(height))? as i16;
        if oy == sy {
            break;
        }
    }
    Ok(Some(0))
}
