use crate::{Context, Result, call, call_void, ids};
use tje_mem::{ram, rom};

/// G/src/map/map_drawing_and_controller.c:158 (`DisplayMap`).
/// Reversed traversal changes visit order without changing the two-pass reveal rules.
pub fn display_map(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let map = args[0];
    let sprites = args[1];
    let level = i32::from(args[2] as i16);
    if level == 0 || ram::g_map_directional_path_mode(r).read(c) != 0 {
        return Ok(None);
    }
    let traversal = super::random_generation::random(c)? & 3;
    let mut revealed = 0_u32;
    let visited = ram::g_map_visited_cover_bits(r).at_word_index(level as i16);
    let marks = ram::g_map_revealed_cover_bits(r).at_word_index(level as i16);
    for outer_row in 0..7 {
        let row = if traversal & 1 != 0 {
            6 - outer_row
        } else {
            outer_row
        };
        for outer_column in 0..8 {
            let column = if traversal & 2 != 0 {
                7 - outer_column
            } else {
                outer_column
            };
            let bit = 0x80_u8 >> column;
            if visited.at(row).read(c) & bit == 0 {
                if marks.at(row).read(c) & bit != 0 {
                    call(
                        c,
                        ids::MAP_APPEND_COVER_BLOCK_SPRITES,
                        &[row as u32, column as u32, sprites, 1],
                    )?;
                } else if revealed < 8
                    && call(
                        c,
                        ids::MAP_COVER_BLOCK_CONTAINS_TERRAIN,
                        &[map, row as u32, column as u32],
                    )? != 0
                {
                    let index = ram::g_random_cache_index(r);
                    let next = if index.read(c) == 58 {
                        0
                    } else {
                        index.read(c).wrapping_add(1)
                    };
                    index.write(c, next);
                    if u32::from(ram::g_cached_random_values(r).at(i32::from(next)).read(c)) % 100
                        < 25 - revealed
                    {
                        call(
                            c,
                            ids::MAP_APPEND_COVER_BLOCK_SPRITES,
                            &[row as u32, column as u32, sprites, 0],
                        )?;
                        marks.at(row).write(c, marks.at(row).read(c) | bit);
                        revealed += 1;
                    }
                }
            }
        }
    }
    for row in 0..7 {
        for column in 0..8 {
            let bit = 0x80_u8 >> column;
            if visited.at(row).read(c) & bit == 0 && marks.at(row).read(c) & bit == 0 {
                if revealed < 2
                    && call(
                        c,
                        ids::MAP_COVER_BLOCK_CONTAINS_TERRAIN,
                        &[map, row as u32, column as u32],
                    )? != 0
                {
                    call(
                        c,
                        ids::MAP_APPEND_COVER_BLOCK_SPRITES,
                        &[row as u32, column as u32, sprites, 0],
                    )?;
                    marks.at(row).write(c, marks.at(row).read(c) | bit);
                    revealed += 1;
                } else {
                    for cover_row in 0..4 {
                        c.write_vdp_command(
                            0x50000003
                                + (((row * 4 + cover_row) as u32) << 23)
                                + (((column * 5) as u32) << 17),
                        );
                        for cover_column in 0..5 {
                            let tile = rom::map_cover_layout(r)
                                .at(cover_row)
                                .at(cover_column)
                                .read(c);
                            c.write_vdp_data(u16::from(tile).wrapping_add(0xc112));
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

/// G/src/map/map_drawing_and_controller.c:214 (`LoadMapIntoVRAM`).
pub fn load_map_into_vram(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let player = args[0] as i16;
    let sprites = args[1];
    let (map, level) = if player == 0 {
        (ram::g_player1_level_map(r), ram::g_player1_map_level(r))
    } else {
        (ram::g_player2_level_map(r), ram::g_player2_map_level(r))
    };
    let level = level.read(c);
    c.write_vdp_command(0x50000003);
    for _ in 0..40 {
        c.write_vdp_data(0x8000);
    }
    for row in 0..26 {
        c.write_vdp_command(0x50000003 + (((row + 1) as u32) << 23));
        c.write_vdp_data(0x8000);
        for column in 0..19 {
            let overlay = (map.at(row).at(column).read(c) & 0x1f00) >> 8;
            if overlay != 0 {
                let number = overlay & 15;
                let left = (number + 121) * 2 + 1;
                let right = (number + 121) * 2 + 2;
                if overlay & 16 != 0 {
                    c.write_vdp_data(right | 0x8800);
                    c.write_vdp_data(left | 0x8800);
                } else {
                    c.write_vdp_data(left | 0x8000);
                    c.write_vdp_data(right | 0x8000);
                }
            } else {
                c.write_vdp_data(0x8000);
                c.write_vdp_data(0x8000);
            }
        }
        c.write_vdp_data(0x8000);
    }
    c.write_vdp_command(0x5d800003);
    for _ in 0..40 {
        c.write_vdp_data(0x8000);
    }
    call_void(
        c,
        ids::DISPLAY_MAP,
        &[map.address(), sprites, i32::from(level) as u32],
    )?;
    Ok(None)
}

/// G/src/map/map_drawing_and_controller.c:59 (`MapDrawBackgroundAndTerrain`).
pub fn map_draw_background_and_terrain(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let player = args[0] as i16;
    let map = if player == 0 {
        let _level = ram::g_player1_map_level(r).read(c);
        ram::g_player1_level_map(r)
    } else {
        let _level = ram::g_player2_map_level(r).read(c);
        ram::g_player2_level_map(r)
    };
    for row in 0..28 {
        c.write_vdp_command(0x60020003 + (((row + 1) as u32) << 23));
        for column in 0..40 {
            let mut tile = u16::from(
                rom::font_background_byte_map(r)
                    .at(if row <= 26 { row } else { row - 1 })
                    .at(column)
                    .read(c),
            );
            if tile & 0x40 != 0 {
                tile |= 0x800;
            }
            if tile & 0x80 != 0 {
                tile |= 0x1000;
            }
            tile &= 0xff3f;
            c.write_vdp_data(tile.wrapping_add(0x4580));
        }
    }
    call_void(c, ids::MAP_DRAW_FIXED_BACKGROUND_DECORATIONS, &[])?;
    for row in 0..26 {
        c.write_vdp_command(0x60040003 + (((row + 2) as u32) << 23));
        for column in 0..19 {
            let cell = map.at(row).at(column).read(c) & 0xff;
            if cell != 34 {
                let number = cell & 0x7f;
                let mut left = number * 2 + 1;
                let mut right = number * 2 + 2;
                let mut palette = if number < 64 {
                    1
                } else if number > 101 {
                    u16::from(
                        rom::tile_lookup_auxiliary(r)
                            .at(i32::from(number) - 102)
                            .read(c),
                    )
                } else {
                    0
                };
                if ram::g_map_directional_path_mode(r).read(c) != 0
                    && ram::g_map_path_direction_banks(r)
                        .at(row)
                        .at(column)
                        .read(c)
                        & 15
                        != 0
                {
                    palette = 3;
                }
                left |= palette << 13;
                right |= palette << 13;
                if cell & 0x80 != 0 {
                    c.write_vdp_data(right | 0x800);
                    c.write_vdp_data(left | 0x800);
                } else {
                    c.write_vdp_data(left);
                    c.write_vdp_data(right);
                }
            } else {
                for _half in 0..2 {
                    let bucket = call(c, ids::MAP_CHOOSE_BACKGROUND_TILE_BUCKET, &[])? as i16;
                    if bucket == 0 {
                        c.write_vdp_data(0x2045);
                    } else {
                        let flips = super::random_generation::random(c)? & 0x1800;
                        c.write_vdp_data(
                            flips
                                .wrapping_add(0x422b)
                                .wrapping_add(i32::from(bucket) as u32)
                                .wrapping_sub(1) as u16,
                        );
                    }
                }
            }
        }
    }
    Ok(None)
}

/// G/src/map/map_drawing_and_controller.c:297 (`MapInitializeScreenSprites`).
/// ORIGINAL BUG: the second failed allocation is not retried after stronger recovery.
pub fn map_initialize_screen_sprites(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    use tje_mem::{Address, HardwareSprite};
    let r = c.revision();
    let sprites = Address::<HardwareSprite>::new(args[0]);
    let mut first = call(c, ids::ALLOCATE_SPRITE_TILE_RUN, &[8])? as u16;
    if first == 0 {
        call_void(c, ids::ENTITY_CLEAR_VIEWPORT_SPRITES, &[0])?;
        first = call(c, ids::ALLOCATE_SPRITE_TILE_RUN, &[8])? as u16;
        if first == 0 {
            call_void(c, ids::ENTITY_CLEAR_VIEWPORT_SPRITES, &[1])?;
        }
    }
    call_void(c, ids::FREE_SPRITE_TILE_RUN, &[u32::from(first), 8])?;
    ram::g_map_screen_next_sprite_index(r).write(c, 7);
    for slot in 0..80 {
        let sprite = sprites.offset(slot);
        if slot < i32::from(ram::g_map_screen_next_sprite_index(r).read(c)) {
            sprite.tile().write(c, first.wrapping_add(slot as u16));
            // G c_map_drawing_and_controller.s:1023, CLR.B pre-read.
            c.discard_u8(sprite.size().address())?;
            sprite.size().write(c, 0);
        } else if slot & 1 != 0 {
            sprite.tile().write(c, 0xe122);
            sprite.size().write(c, 15);
        } else {
            sprite.tile().write(c, 0xe132);
            sprite.size().write(c, 3);
        }
        sprite.link().write(c, (slot + 1) as u8);
        sprite.y().write(c, 1);
    }
    // Same listing:1083, CLR.B of the terminating link.
    c.discard_u8(sprites.offset(79).link().address())?;
    sprites.offset(79).link().write(c, 0);
    Ok(None)
}
