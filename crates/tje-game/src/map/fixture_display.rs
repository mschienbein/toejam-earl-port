use crate::{Result, check, fill_map};
use tje_game::ids;
use tje_host::Host;
use tje_mem::{Address, HardwareSprite, ram};

pub fn run(host: &mut Host) -> Result {
    let r = host.revision();
    let sprites = Address::<HardwareSprite>::new(ram::g_saved_level_map(r).address());
    for i in 0..80 {
        let mut c = host.context();
        sprites.offset(i).tile().write(&mut c, 0xe122);
        sprites.offset(i).y().write(&mut c, 1);
    }
    for reveal in [0, 1] {
        fill_map(host, 30);
        let mut c = host.context();
        ram::g_map_reveal_mode(r).write(&mut c, reveal);
        ram::g_map_directional_path_mode(r).write(&mut c, 0);
        ram::g_map_screen_next_sprite_index(r).write(&mut c, 7);
        ram::g_random_cache_index(r).write(&mut c, 0);
        ram::g_rng_state(r).write(&mut c, 12345);
        for row in 0..7 {
            ram::g_map_visited_cover_bits(r)
                .at(1)
                .at(row)
                .write(&mut c, 0);
            ram::g_map_revealed_cover_bits(r)
                .at(1)
                .at(row)
                .write(&mut c, 0);
        }
        ram::g_map_revealed_cover_bits(r)
            .at(1)
            .at(0)
            .write(&mut c, 0x80);
        for index in 0..59 {
            ram::g_cached_random_values(r)
                .at(index)
                .write(&mut c, ((index * 37) % 100) as u16);
        }
        check(
            host,
            ids::DISPLAY_MAP,
            &[ram::g_player1_level_map(r).address(), sprites.address(), 1],
            "functional cover writer, cached reveals, retained sprite tiles",
        )?;
    }
    let mut c = host.context();
    ram::g_player1_map_level(r).write(&mut c, 1);
    ram::g_map_directional_path_mode(r).write(&mut c, 1);
    for row in 0..26 {
        for column in 0..19 {
            ram::g_player1_level_map(r)
                .at(row)
                .at(column)
                .write(&mut c, 30 | ((((row * 19 + column) % 32) as u16) << 8));
        }
    }
    check(
        host,
        ids::LOAD_MAP_INTO_VRAM,
        &[0, sprites.address()],
        "all overlay selectors and mirror flag; border and DisplayMap mode gate",
    )?;
    for (cell, mode) in [(34, 0), (30, 0), (0x9e, 1)] {
        fill_map(host, cell);
        let mut c = host.context();
        ram::g_map_directional_path_mode(r).write(&mut c, mode);
        ram::g_rng_state(r).write(&mut c, 12345);
        for row in 0..26 {
            for column in 0..19 {
                ram::g_map_path_direction_banks(r)
                    .at(row)
                    .at(column)
                    .write(&mut c, 1);
            }
        }
        check(
            host,
            ids::MAP_DRAW_BACKGROUND_AND_TERRAIN,
            &[0],
            "functional font/background and terrain; water RNG, mirror and path palette",
        )?;
    }
    Ok(())
}
