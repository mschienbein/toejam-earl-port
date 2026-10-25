use crate::{Result, check, fill_map};
use tje_game::{Context, FunctionId, call, call_void, ids};
use tje_host::Host;
use tje_mem::{Address, HardwareSprite, ram, rom};

fn selected(host: &Host, id: FunctionId) -> bool {
    host.checks.by_function.contains_key(id.metadata().name)
        && std::env::var("TJE_MAP_FIXTURE_FILTER").map_or(true, |filter| {
            filter.split(',').any(|name| name == id.metadata().name)
        })
}

fn case(name: &str) -> bool {
    std::env::var("TJE_MAP_FIXTURE_CASE").map_or(true, |selected| selected == name)
}

// Set up real transient effects through the retail/proven allocators, not
// invented handles, slots, or animation bytes. This setup is not hook evidence.
fn prepare_effects(host: &mut Host) -> Result {
    let r = host.revision();
    let mut c = host.context();
    for animation in 0..8 {
        let number = call(&mut c, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[0])? as i16;
        ram::g_land_path_animation_entities(r)
            .at(animation)
            .write(&mut c, number as i8);
        let object = ram::g_player_info(r).at(i32::from(number));
        object.level().write(&mut c, 1);
        object.width().write(&mut c, 32);
        object.height_pixels().write(&mut c, 32);
        object.sprite_count().write(&mut c, 1);
        object.tile_count().write(&mut c, 16);
        let tile = object.tile().read(&c) | 0x8000;
        object.tile().write(&mut c, tile);
        let data = rom::transient_effect_animation(r);
        let sequence = data.sequence().read(&c);
        let frames = data.frames().read(&c);
        object.sequence().write(&mut c, sequence);
        object.frames().write(&mut c, frames);
        object.frame_phase().write(&mut c, 2);
        let d4 = c.data_register(4);
        c.set_data_register(4, (d4 & 0xffff_0000) | animation as u32);
        let result = call_void(
                    &mut c,
                    ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
                    &[object.address(), 0],
                );
        c.set_data_register(4, d4);
        result?;
        if object.draw_flags().read(&c) & 6 == 0 {
            return Err("fixture effect allocation did not obtain a viewport slot".into());
        }
        let draw = object.draw_flags().read(&c) | 0x80;
        let behavior = object.behavior_flags().read(&c) | 0x40;
        object.draw_flags().write(&mut c, draw);
        object.behavior_flags().write(&mut c, behavior);
    }
    Ok(())
}

fn release_effects(host: &mut Host) -> Result {
    let r = host.revision();
    let mut c = host.context();
    for animation in 0..8 {
        let slot = ram::g_land_path_animation_entities(r).at(animation);
        let object = slot.read(&c);
        if object != 0 {
            call_void(&mut c, ids::FREE_ENTITY, &[i32::from(object) as u32])?;
        }
        slot.write(&mut c, 0);
    }
    Ok(())
}

fn level_state(host: &mut Host, requested: i8, first: i8, second: i8, saved: i8) {
    let r = host.revision();
    let mut c = host.context();
    ram::g_requested_map_level(r).write(&mut c, requested);
    ram::g_player1_map_level(r).write(&mut c, first);
    ram::g_player2_map_level(r).write(&mut c, second);
    ram::g_saved_map_level(r).write(&mut c, saved);
    ram::g_level_seeds(r)
        .at(i32::from(requested))
        .write(&mut c, 12345);
}

pub fn run(host: &mut Host) -> Result {
    let r = host.revision();
    let sprites = Address::<HardwareSprite>::new(ram::g_saved_level_map(r).address());
    // The harness must settle the original machine before this first mutation.
    // Never lift SR masks or clear IRQs to manufacture a call-ready boundary.
    if selected(host, ids::ENTITY_RESERVE_LAND_PATH_ANIMATIONS) {
        for pending in [0, 1] {
            let mut c = host.context();
            ram::g_camera_column_upload_height(r).write(&mut c, 28);
            ram::g_camera_row_upload_words(r)
                .at(0)
                .at(0)
                .write(&mut c, 0x8000);
            ram::g_camera_row_upload_word_count(r)
                .at(0)
                .write(&mut c, 1);
            ram::g_camera_row_upload_vram_row(r).at(0).write(&mut c, 16);
            ram::g_camera_row_upload_start_column(r)
                .at(0)
                .write(&mut c, 16);
            ram::g_camera_row_upload_pending(r)
                .at(0)
                .write(&mut c, pending);
            ram::g_camera_column_upload_pending(r)
                .at(0)
                .write(&mut c, 0);
            check(
                host,
                ids::ENTITY_RESERVE_LAND_PATH_ANIMATIONS,
                &[0, 0, 0],
                "idle and real IRQ-cleared camera queue; source D4 animation residue",
            )?;
        }
    }
    if selected(host, ids::ENTITY_START_LAND_PATH_ANIMATION)
        || selected(host, ids::MAP_ANIMATE_METATILE_PATH_CHANGE)
    {
        prepare_effects(host)?;
        ram::g_sprite_graphics_upload_pending(r).write(&mut host.context(), 0);
        if selected(host, ids::ENTITY_START_LAND_PATH_ANIMATION) {
            for direction in [1, 8] {
                ram::g_land_path_traversal_direction(r).write(&mut host.context(), direction);
                check(
                    host,
                    ids::ENTITY_START_LAND_PATH_ANIMATION,
                    &[0, 768, 384],
                    "allocated transient effects; both jitter axes, sound and sprite updates",
                )?;
            }
        }
        if selected(host, ids::MAP_ANIMATE_METATILE_PATH_CHANGE) {
            let mut c = host.context();
            ram::g_camera1_tile_row_cursor(r).write(&mut c, 48);
            ram::g_camera1_tile_column_cursor(r).write(&mut c, 96);
            ram::g_camera_column_upload_height(r).write(&mut c, 28);
            ram::g_player1_level_map(r).at(5).at(5).write(&mut c, 30);
            for direction in [1, 2, 4, 8] {
                ram::g_land_path_traversal_direction(r).write(&mut host.context(), direction);
                check(
                    host,
                    ids::MAP_ANIMATE_METATILE_PATH_CHANGE,
                    &[0, 5, 5],
                    "visible cardinal sweep with real effect updates and IRQ tile transfers",
                )?;
            }
        }
        release_effects(host)?;
    }
    if selected(host, ids::MAP_INITIALIZE_SCREEN_SPRITES) {
        for occupied in [0, 1] {
            for index in 0..80 {
                ram::g_sprite_tile_group_allocated(r)
                    .at(index)
                    .write(&mut host.context(), occupied);
            }
            check(
                host,
                ids::MAP_INITIALIZE_SCREEN_SPRITES,
                &[sprites.address()],
                "free pool and exhaustion/recovery; original missing third-allocation bug",
            )?;
        }
    }
    if selected(host, ids::MAP_UPLOAD_COVER_ANIMATION_FRAME) {
        for (frame, first) in [(0, 0), (7, 0), (11, 1)] {
            let mut c = host.context();
            ram::g_sprite_tile_buffer_offset(r).write(&mut c, 0);
            ram::g_sprite_tile_upload_queue_word_count(r).write(&mut c, 0);
            ram::g_sprite_graphics_upload_pending(r).write(&mut c, 0);
            check(
                host,
                ids::MAP_UPLOAD_COVER_ANIMATION_FRAME,
                &[sprites.address(), frame, first],
                "real two-piece decode, palette-seven branch, VBlank, DMA and upload",
            )?;
        }
    }
    if selected(host, ids::CHANGE_LEVEL) {
        if case("cached") {
            for (player, requested, first, second, saved) in [
                (0, 1, 1, 1, -1),
                (0, 0, 1, 1, -1),
                (1, 1, 0, 1, -1),
                (0, 1, 0, 1, -1),
                (1, 1, 1, 0, -1),
                (0, 2, 1, 1, 2),
            ] {
                level_state(host, requested, first, second, saved);
                check(
                    host,
                    ids::CHANGE_LEVEL,
                    &[player],
                    "cached, partner/saved bank and fixed map branches; exchanged RNG restored",
                )?;
            }
        }
        if case("procedural") {
            for player in [0, 1] {
                level_state(host, 2, 1, 1, -1);
                ram::g_sprite_graphics_upload_pending(r).write(&mut host.context(), 0);
                check(
                    host,
                    ids::CHANGE_LEVEL,
                    &[player],
                    "procedural map with source-backed Grow and ChooseAlt call frames",
                )?;
            }
        }
    }
    if selected(host, ids::MAP_LOAD_PLAYER_LEVEL) {
        let levels: &[u32] = if case("cached") { &[1, 26] } else { &[2] };
        for &level in levels {
            level_state(host, level as i8, 1, 1, -1);
            ram::g_player_lives(r).at(0).write(&mut host.context(), 0);
            check(
                host,
                ids::MAP_LOAD_PLAYER_LEVEL,
                &[0, level],
                "ordinary/final maps, framed ChangeLevel and seed retry path",
            )?;
        }
    }
    if selected(host, ids::PROCESS_LAND) {
        for player in 0..2 {
            ram::g_player_lives(r)
                .at(player)
                .write(&mut host.context(), -1);
        }
        if case("inactive") {
            check(host, ids::PROCESS_LAND, &[], "absent-player branches")?;
        }
        fill_map(host, 30);
        let mut c = host.context();
        let player = ram::g_player_info(r).at(0);
        ram::g_player_lives(r).at(0).write(&mut c, 0);
        player.level().write(&mut c, 2);
        player.height().write(&mut c, 0);
        player.x().write(&mut c, 768);
        player.y().write(&mut c, 384);
        ram::g_player_elevator_flags(r).at(0).write(&mut c, 0);
        if case("unmarked") {
            check(
                host,
                ids::PROCESS_LAND,
                &[],
                "grounded player without a marked metatile",
            )?;
        }
        if case("marked") {
            ram::g_player1_level_map(r)
                .at(5)
                .at(5)
                .write(&mut host.context(), 0x4000 | 30);
            ram::g_player1_nearby_terrain(r)
                .cast::<u8>()
                .offset(7)
                .write(&mut host.context(), 0x90);
            check(
                host,
                ids::PROCESS_LAND,
                &[],
                "marked edge reaches residual-sensitive path animation; no suppressed stack reads",
            )?;
        }
    }
    if selected(host, ids::SET_POSITION_ON_NEW_LEVEL) {
        let failed =
            std::env::var("TJE_MAP_FIXTURE_CASE").is_ok_and(|name| name == "failed-placement");
        fill_map(host, if failed { 34 } else { 30 });
        let mut c = host.context();
        for player in 0..2 {
            ram::g_player_lives(r).at(player).write(&mut c, 0);
            ram::g_player_info(r).at(player).level().write(&mut c, 1);
        }
        for player in [0, 1, 2] {
            check(
                host,
                ids::SET_POSITION_ON_NEW_LEVEL,
                &[player],
                "actual A6-relative validation outputs; single and paired positions",
            )?;
        }
    }
    if selected(host, ids::MAP_ANIMATE_MARKED_LAND_PATHS) {
        fill_map(host, 30);
        ram::g_player1_level_map(r)
            .at(5)
            .at(5)
            .write(&mut host.context(), 0x4000 | 30);
        check(
            host,
            ids::MAP_ANIMATE_MARKED_LAND_PATHS,
            &[0, 5, 5],
            "saved durations and original first-call residual coordinates",
        )?;
    }
    if selected(host, ids::DRAW_GAME_MAP) {
        check(
            host,
            ids::DRAW_GAME_MAP,
            &[0],
            "live local 80-sprite table; actual VBlank and controller press/release",
        )?;
    }
    Ok(())
}
