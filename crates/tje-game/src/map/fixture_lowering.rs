//! Labelled broad-index live fixtures for emitted word/long lowering.
use crate::{Result, check};
use tje_game::ids;
use tje_host::Host;
use tje_mem::{Address, HardwareSprite, ram};

fn selected(host: &Host, id: tje_game::FunctionId) -> bool {
    host.checks.by_function.contains_key(id.metadata().name)
}

pub fn run(host: &mut Host) -> Result {
    let r = host.revision();
    // Like ReviewHazards' authored wide-index fixture, use a real writable
    // stack below the tested RAM, not a waiver of residual-stack auditing.
    let mut c = host.context();
    let registers = c.cpu().address_registers();
    c.cpu().set_address_registers(registers, 0xff0400, 0xff0400);
    if selected(host, ids::MAP_GET_RANDOM_GENERATION_PARAMETER) {
        ram::g_requested_map_level(r).write(&mut host.context(), 2);
        for index in [i16::MIN, i16::MAX, -1, -16384, 16384] {
            ram::g_rng_state(r).write(&mut host.context(), 12345);
            check(
                host,
                ids::MAP_GET_RANDOM_GENERATION_PARAMETER,
                &[i32::from(index) as u32],
                "word-scaled pair index, negative and wide",
            )?;
        }
    }
    if selected(host, ids::MAP_CONSUME_PATH_CELL_DIRECTIONS) {
        for bank in [0, 1] {
            for row in [-8000_i16, -2000, -1000] {
                let map = if bank == 0 {
                    ram::g_player1_level_map(r)
                } else {
                    ram::g_player2_level_map(r)
                };
                let cell = map.cast::<u16>().offset(i32::from(row) * 19);
                let mask = ram::g_map_path_direction_banks(r).at_word_index(row).at(0);
                cell.write(&mut host.context(), 0x4021);
                mask.write(&mut host.context(), if bank == 0 { 0xa1 } else { 0x1a });
                ram::g_rng_state(r).write(&mut host.context(), 12345);
                ram::g_land_path_traversal_direction(r).write(&mut host.context(), -1);
                check(
                    host,
                    ids::MAP_CONSUME_PATH_CELL_DIRECTIONS,
                    &[bank, i32::from(row) as u32, 0],
                    "long flat map but word-scaled byte direction bank",
                )?;
            }
        }
    }
    if selected(host, ids::DISPLAY_MAP) {
        for level in [10000_i16, -10000] {
            for row in 0..7 {
                ram::g_map_visited_cover_bits(r)
                    .at_word_index(level)
                    .at(row)
                    .write(&mut host.context(), 0xff);
            }
            check(
                host,
                ids::DISPLAY_MAP,
                &[
                    ram::g_player1_level_map(r).address(),
                    ram::g_saved_level_map(r).address(),
                    i32::from(level) as u32,
                ],
                "level-times-seven wraps as signed word displacement",
            )?;
        }
    }
    if selected(host, ids::MAP_UPLOAD_COVER_ANIMATION_FRAME) {
        let sprites = Address::<HardwareSprite>::new(ram::g_saved_level_map(r).address());
        for frame in [16384_i16, -16384, 16391] {
            for index in 0..80 {
                sprites
                    .offset(index)
                    .tile()
                    .write(&mut host.context(), 0x0122);
            }
            ram::g_sprite_tile_buffer_offset(r).write(&mut host.context(), 0);
            ram::g_sprite_tile_upload_queue_word_count(r).write(&mut host.context(), 0);
            check(
                host,
                ids::MAP_UPLOAD_COVER_ANIMATION_FRAME,
                &[sprites.address(), i32::from(frame) as u32, 0],
                "frame pointer index wraps independently of literal frame-seven palette branch",
            )?;
        }
    }
    if selected(host, ids::TERRAIN_VALIDATION) {
        let x = ram::g_saved_level_map(r).cast::<i16>();
        let y = x.offset(1);
        for (width, height) in [(1, 0), (1, -1)] {
            ram::g_rng_state(r).write(&mut host.context(), 1);
            check(
                host,
                ids::TERRAIN_VALIDATION,
                &[30, width, height as u32, x.address(), y.address(), 1],
                "empty or negative unchecked rectangle follows original output path",
            )?;
        }
    }
    Ok(())
}
