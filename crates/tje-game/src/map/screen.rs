use crate::{Context, Result};
use tje_mem::{Address, HardwareSprite, ram, rom};

/// G/src/map/map_drawing_and_controller.c:135 (`MapCoverBlockContainsTerrain`).
/// Cover coordinates are full int slots; row/column narrow only after arithmetic.
/// In particular, -1 / 2 is zero, not the arithmetic-right-shift result -1.
pub fn map_cover_block_contains_terrain(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let map = Address::<u16>::new(args[0]);
    if ram::g_map_reveal_mode(c.revision()).read(c) != 0 {
        let cover_row = (args[1] as i32).wrapping_mul(4).wrapping_sub(1);
        let cover_column = (args[2] as i32).wrapping_mul(5).wrapping_sub(1);
        for row_offset in 0..4 {
            for column_offset in 0..5 {
                let row = cover_row.wrapping_add(row_offset) as i16;
                let column = (cover_column.wrapping_add(column_offset) / 2) as i16;
                if (0..26).contains(&row)
                    && (0..19).contains(&column)
                    && map.offset(i32::from(row) * 19 + i32::from(column)).read(c) & 0x7f != 34
                {
                    return Ok(Some(1));
                }
            }
        }
    }
    Ok(Some(0))
}

/// G/src/map/map_drawing_and_controller.c:263 (`LoadLevel1Tiles`).
/// Each byte-sized ROM metatile is read separately for each destination plane.
pub fn load_level1_tiles(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = c.revision();
    let data = rom::level1_map_data(revision);
    for row in 0..26 {
        for column in 0..19 {
            ram::g_player1_level_map(revision)
                .at(row)
                .at(column)
                .write(c, u16::from(data.at(row).at(column).read(c)));
            ram::g_player2_level_map(revision)
                .at(row)
                .at(column)
                .write(c, u16::from(data.at(row).at(column).read(c)));
        }
    }
    Ok(None)
}

/// G/src/map/map_drawing_and_controller.c:276 (`MapAppendCoverBlockSprites`).
/// Reload the shared index at every field access, preserving pointer/RAM aliases.
pub fn map_append_cover_block_sprites(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let row = i32::from(args[0] as i16);
    let column = i32::from(args[1] as i16);
    let sprites = Address::<HardwareSprite>::new(args[2]);
    let retain_tiles = args[3] as i16;
    let index = ram::g_map_screen_next_sprite_index(c.revision());
    if index.read(c) > 78 {
        return Ok(Some(1));
    }
    sprites
        .offset(i32::from(index.read(c)))
        .x()
        .write(c, (column * 40 + 128) as i16);
    sprites
        .offset(i32::from(index.read(c)))
        .y()
        .write(c, (row * 32 + 128) as i16);
    if retain_tiles == 0 {
        sprites
            .offset(i32::from(index.read(c)))
            .tile()
            .write(c, 0xc136);
    }
    index.write(c, index.read(c).wrapping_add(1));
    sprites
        .offset(i32::from(index.read(c)))
        .x()
        .write(c, (column * 40 + 160) as i16);
    sprites
        .offset(i32::from(index.read(c)))
        .y()
        .write(c, (row * 32 + 128) as i16);
    if retain_tiles == 0 {
        sprites
            .offset(i32::from(index.read(c)))
            .tile()
            .write(c, 0xc146);
    }
    index.write(c, index.read(c).wrapping_add(1));
    Ok(Some(0))
}

/// G/src/map/map_drawing_and_controller.c:338 (`PlayerHasOddElevatorState`).
pub fn player_has_odd_elevator_state(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let flags = ram::g_player_elevator_flags(c.revision()).at(i32::from(args[0] as i16));
    let result = flags.read(c) != 0 && ((i32::from(flags.read(c)) - 1) & 1) == 0;
    Ok(Some(u32::from(result)))
}
