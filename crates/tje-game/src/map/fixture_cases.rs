//! Authored map inputs on a gameplay state; no state or cartridge data is stored.
use crate::{Result, check, fill_map};
use tje_game::ids;
use tje_host::Host;
use tje_mem::{Address, HardwareSprite, ram};

pub fn run(host: &mut Host) -> Result {
    let r = host.revision();
    let map = ram::g_player1_level_map(r);
    let anchors = ram::g_level_anchor_positions(r);
    fill_map(host, 0x3000);
    check(
        host,
        ids::MAP_BUILD_FILLED_TERRAIN_SHAPES,
        &[],
        "full filled-class map through all shape passes",
    )?;
    fill_map(host, 0);
    map.at(5).at(5).write(&mut host.context(), 0x2001);
    map.at(4).at(5).write(&mut host.context(), 0x1100);
    check(
        host,
        ids::MAP_EXPAND_REGIONS_BESIDE_PATHS,
        &[5, 5],
        "upper path rectangle merges global labels",
    )?;
    let mut c = host.context();
    anchors.at(0).x().write(&mut c, 5);
    anchors.at(0).y().write(&mut c, 5);
    anchors.at(1).x().write(&mut c, 5);
    anchors.at(1).y().write(&mut c, 5);
    map.at(5).at(5).write(&mut c, 0x1000);
    check(
        host,
        ids::MAP_IS_ANCHOR_CONNECTION_MISSING,
        &[],
        "matching anchors; visited store precedes terminal hit",
    )?;
    check(
        host,
        ids::MAP_TRACE_TRANSITION_CHAIN,
        &[5, 5, 8, 8],
        "anchor exclusion stops transition recursion",
    )?;
    for player in [0_u32, 1] {
        for (x, y) in [
            (0_i32, 0_i32),
            (128, 64),
            (256, 128),
            (-1, -1),
            (32767, 32767),
        ] {
            check(
                host,
                ids::GET_MAP_TERRAIN_AT_WORLD_POSITION,
                &[player, x as u32, y as u32],
                "terrain boundary and negative-coordinate original bug",
            )?;
        }
        let mut c = host.context();
        let entity = ram::g_player_info(r).at(player as i32);
        ram::g_player_lives(r).at(player as i32).write(&mut c, 0);
        ram::g_player_inventory_open(r)
            .at(player as i32)
            .write(&mut c, 0);
        entity.x().write(&mut c, 512);
        entity.y().write(&mut c, 384);
        entity.level().write(&mut c, 1);
        check(
            host,
            ids::UPDATE_LAND_AROUND_PLAYER,
            &[player],
            "active player 5x5 cache; ROM base/overlay and doorway predicate",
        )?;
    }
    let sprites = Address::<HardwareSprite>::new(ram::g_saved_level_map(r).address());
    for i in 0..80 {
        let mut c = host.context();
        sprites.offset(i).tile().write(&mut c, 0x122 + i as u16);
        sprites.offset(i).y().write(&mut c, 128);
    }
    check(
        host,
        ids::MAP_DRAW_FIXED_BACKGROUND_DECORATIONS,
        &[],
        "functional nine-word bounded VDP writer",
    )?;
    for player in 0..2 {
        let mut c = host.context();
        let entity = ram::g_player_info(r).at(player);
        entity.level().write(&mut c, 3);
        entity.x().write(&mut c, 320);
        entity.y().write(&mut c, 256);
        ram::g_player_elevator_flags(r).at(player).write(&mut c, 0);
    }
    check(
        host,
        ids::MAP_DRAW_PLAYER,
        &[3, sprites.address()],
        "both player icon uploads and map positions",
    )?;
    let mut c = host.context();
    ram::g_earthling_bank_users(r).at(0).write(&mut c, 1);
    ram::g_earthling_bank_levels(r).at(0).write(&mut c, 3);
    let piece = ram::g_earthling_banks(r).at(0).at(0);
    piece.kind().write(&mut c, 0);
    piece.x().write(&mut c, 320);
    piece.y().write(&mut c, 256);
    check(
        host,
        ids::MAP_DRAW_SHIP_PIECE,
        &[3, sprites.address()],
        "first ship piece icon uploads four tiles",
    )?;
    let mut c = host.context();
    let elevator = ram::g_elevator_records(r).at(0).at(0);
    elevator.level().write(&mut c, 3);
    elevator.state().write(&mut c, 0);
    elevator.x().write(&mut c, 320);
    elevator.y().write(&mut c, 256);
    check(
        host,
        ids::MAP_DRAW_ELEVATOR,
        &[3, sprites.address()],
        "unblocked departure elevator icon",
    )?;
    for counter in [0, 24] {
        ram::g_frame_counter(r).write(&mut host.context(), counter);
        check(
            host,
            ids::DRAW_MAP_CHARACTERS,
            &[sprites.address()],
            "real VBlank wait; both marker blink branches",
        )?;
    }
    ram::g_camera_column_upload_height(r).write(&mut host.context(), 28);
    for camera in [0, 1] {
        check(
            host,
            ids::MAP_QUEUE_VISIBLE_METATILE_ROW,
            &[30, 16, 16, 0, camera, 16],
            "real IRQ-cleared visible row transfer",
        )?;
        check(
            host,
            ids::MAP_QUEUE_VISIBLE_METATILE_COLUMN,
            &[30, 16, 16, 0, camera, 16],
            "real IRQ-cleared visible column transfer",
        )?;
    }
    Ok(())
}
