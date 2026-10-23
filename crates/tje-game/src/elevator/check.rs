//! Private-ROM opt-in harness, built with scratch/W0World/check2/Cargo.toml.
use std::{
    env, fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tje_game::{Context, FunctionId, execution::ExecutionClass, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Ptr, Revision, ram};

static FAILURES: AtomicUsize = AtomicUsize::new(0);

fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    // Fixtures follow the explicit execution-class selection; unregistered
    // classes are omitted, never credited as checked. All selected IDs must
    // leave the host's final unexercised set below.
    if !host.checks.by_function.contains_key(id.metadata().name) {
        return;
    }
    if let Err(error) = host.check_fixture(id.metadata().address(host.revision()), arguments) {
        FAILURES.fetch_add(1, Ordering::Relaxed);
        eprintln!(
            "FIXTURE_ERROR {:?} {} {arguments:08x?}: {error}",
            host.revision(),
            id.metadata().name
        );
    }
}

fn tile_fixtures(host: &mut Host) {
    let revision = host.revision();
    for camera in 0..2 {
        for level in [1_i8, 2, 26] {
            for position in [-1_i16, 0, 7, 8, 20, 223, 335, 336, i16::MAX] {
                let mut c = host.context();
                ram::g_player1_map_level(revision).write(&mut c, level);
                ram::g_player2_map_level(revision).write(&mut c, level);
                ram::g_camera_column_upload_height(revision).write(&mut c, 19);
                ram::g_camera1_tile_row_cursor(revision).write(&mut c, position);
                ram::g_camera2_tile_row_cursor(revision).write(&mut c, position);
                ram::g_camera1_tile_column_cursor(revision).write(&mut c, position);
                ram::g_camera2_tile_column_cursor(revision).write(&mut c, position);
                for edge in 0..2 {
                    check(host, ids::BUILD_CAMERA_TILE_ROW, &[edge, camera]);
                    check(host, ids::BUILD_CAMERA_TILE_COLUMN, &[edge, camera]);
                }
            }
        }
    }
    for directions in 0..16_i8 {
        let mut c = host.context();
        ram::g_camera1_pending_tile_directions(revision).write(&mut c, directions);
        ram::g_camera2_pending_tile_directions(revision).write(&mut c, directions);
        ram::g_camera1_pending_tile_row(revision).write(&mut c, 20);
        ram::g_camera2_pending_tile_row(revision).write(&mut c, 20);
        ram::g_camera1_pending_tile_column(revision).write(&mut c, 20);
        ram::g_camera2_pending_tile_column(revision).write(&mut c, 20);
        check(host, ids::BUILD_PENDING_CAMERA_TILES, &[]);
    }
    for player in 0..2 {
        for flags in [0_u8, 8, 16, 32] {
            for screen in [-1_i16, 50, 112, 160, 271] {
                let mut c = host.context();
                ram::g_player_lives(revision).at(player).write(&mut c, 3);
                ram::g_player_inventory_open(revision)
                    .at(player)
                    .write(&mut c, 0);
                let entity = ram::g_player_info(revision).at(player);
                entity.frame_phase().write(&mut c, 0);
                entity.animation_motion_axes().write(&mut c, 0);
                entity.draw_flags().write(&mut c, flags);
                entity.animation_group_mask().write(&mut c, 0);
                check(
                    host,
                    ids::GET_PLAYER_CAMERA_VELOCITY,
                    &[
                        player as u32,
                        0xff9000,
                        0xff9002,
                        screen as u32,
                        screen as u32,
                    ],
                );
            }
        }
    }
}

fn upload_fixtures(host: &mut Host) {
    let revision = host.revision();
    for start in [0_i8, 20, 31, 63] {
        let mut c = host.context();
        for camera in 0..2 {
            ram::g_camera_row_upload_pending(revision)
                .at(camera)
                .write(&mut c, 1);
            ram::g_camera_row_upload_start_column(revision)
                .at(camera)
                .write(&mut c, start);
            ram::g_camera_row_upload_word_count(revision)
                .at(camera)
                .write(&mut c, 45);
            ram::g_camera_row_upload_vram_row(revision)
                .at(camera)
                .write(&mut c, 3);
            ram::g_camera_column_upload_pending(revision)
                .at(camera)
                .write(&mut c, 1);
            ram::g_camera_column_upload_start_row(revision)
                .at(camera)
                .write(&mut c, start & 31);
            ram::g_camera_column_upload_word_count(revision)
                .at(camera)
                .write(&mut c, 19);
            ram::g_camera_column_upload_vram_column(revision)
                .at(camera)
                .write(&mut c, 3);
        }
        check(host, ids::UPDATE_CAMERA_Y, &[]);
        check(host, ids::UPDATE_CAMERA_X, &[]);
    }
    for camera in 0..2 {
        {
            let mut c = host.context();
            ram::g_camera1_tile_row_cursor(revision).write(&mut c, 10);
            ram::g_camera2_tile_row_cursor(revision).write(&mut c, 10);
            ram::g_camera_column_upload_height(revision).write(&mut c, 19);
        }
        check(host, ids::REDRAW_CAMERA_TILE_ROWS, &[camera]);
        for position in [-1_i16, 8, 500, 3000] {
            check(
                host,
                ids::SET_CAMERA_VIEW_ORIGIN,
                &[camera, position as u32, position as u32],
            );
            check(
                host,
                ids::SET_CAMERA_POSITION,
                &[camera, position as u32, position as u32],
            );
        }
    }
}

fn elevator_fixtures(host: &mut Host) {
    let revision = host.revision();
    let elevator = ram::g_elevator_records(revision).at(0).at(0);
    for x in [i16::MIN, -1, 0, 9, 10, 19, 20, i16::MAX] {
        for y in [-1_i16, 0, 1, 5, 7, 8, 10, 12, 20] {
            let mut c = host.context();
            for bank in 0..2 {
                for kind in 0..2 {
                    let record = ram::g_elevator_records(revision).at(bank).at(kind);
                    record.x().write(&mut c, 0);
                    record.y().write(&mut c, 0);
                    record.level().write(&mut c, 1);
                    record.state().write(&mut c, 3);
                    record.entity().write(&mut c, 0);
                }
            }
            for player in 0..2 {
                let entity = ram::g_player_info(revision).at(player);
                entity.x().write(&mut c, x);
                entity.y().write(&mut c, 10);
                entity.height().write(&mut c, 0);
                entity.level().write(&mut c, 1);
                ram::g_player_elevator_flags(revision)
                    .at(player)
                    .write(&mut c, 0);
            }
            check(
                host,
                ids::ELEVATOR_BLOCKS_PLAYER_MOVEMENT,
                &[0, x as u32, y as u32, 0],
            );
            check(
                host,
                ids::ELEVATOR_BLOCKS_ENTITY_MOVEMENT,
                &[
                    ram::g_player_info(revision).at(0).address(),
                    x as u32,
                    y as u32,
                ],
            );
            check(
                host,
                ids::ELEVATOR_CONTAINS_POSITION,
                &[x as u32, y as u32, 1],
            );
            check(
                host,
                ids::ELEVATOR_CHECK_PLAYER_PROXIMITY_AND_INPUT,
                &[elevator.address(), 1],
            );
        }
    }
    for delay in [i8::MIN, -1, 0, 1, 2, i8::MAX] {
        for height in [0_i16, 24, 28, 29, i16::MAX] {
            let mut c = host.context();
            elevator.entity().write(&mut c, 2);
            ram::g_departure_elevator_shake_delay(revision).write(&mut c, delay);
            ram::g_departure_elevator_shake_cycles(revision).write(&mut c, 5);
            ram::g_player_info(revision)
                .at(4)
                .height()
                .write(&mut c, height);
            check(
                host,
                ids::ELEVATOR_STEP_DEPARTURE_HEIGHT_CYCLES,
                &[elevator.address()],
            );
        }
    }
    for occupied in 0..3 {
        let mut c = host.context();
        ram::g_player_info(revision)
            .at(2)
            .draw_flags()
            .write(&mut c, u8::from(occupied != 0));
        ram::g_player_info(revision)
            .at(3)
            .draw_flags()
            .write(&mut c, u8::from(occupied == 2));
        elevator.state().write(&mut c, 0);
        check(
            host,
            ids::ELEVATOR_CREATE_ENTITY_PAIR,
            &[elevator.address()],
        );
    }
    for level in [i8::MIN, -1, 0, 1, 2, 24, 25, 26, i8::MAX] {
        for kind in 0..2 {
            let mut c = host.context();
            for bank in 0..2 {
                for member in 0..2 {
                    let record = ram::g_elevator_records(revision).at(bank).at(member);
                    record.entity().write(&mut c, 0);
                    record.level().write(&mut c, -1);
                }
            }
            check(host, ids::SETUP_ELEVATOR, &[level as u32, kind]);
        }
    }
    check(host, ids::SETUP_INITIAL_ELEVATOR, &[]);
    check(host, ids::ELEVATOR_UPDATE_ENTITY_VISIBILITY, &[]);
    for delay in [i8::MIN, -1, 0, 1, 2, i8::MAX] {
        let mut c = host.context();
        for bank in 0..2 {
            let record = ram::g_elevator_records(revision).at(bank).at(1);
            record.state().write(&mut c, 0);
            record.entity().write(&mut c, 0);
        }
        ram::g_arrival_elevator_close_delay(revision).write(&mut c, delay);
        check(host, ids::PROCESS_ELEVATOR_STEP2, &[]);
    }
    for state in 0_i8..=13 {
        for number in [0_i8, 2] {
            let mut c = host.context();
            for bank in 0..2 {
                let record = ram::g_elevator_records(revision).at(bank).at(1);
                record.state().write(&mut c, state);
                record.entity().write(&mut c, number);
            }
            ram::g_player_info(revision)
                .at(2)
                .frame_duration()
                .write(&mut c, 0);
            ram::g_player_info(revision)
                .at(4)
                .frame_duration()
                .write(&mut c, 0);
            check(host, ids::PROCESS_ELEVATOR_STEP2, &[]);
        }
    }
    {
        let mut c = host.context();
        ram::g_entity_draw_list_head(revision).write(&mut c, Ptr::new(0));
        ram::g_inventory_draw_list_head(revision).write(&mut c, Ptr::new(0));
        ram::g_elevator_ride_entity(revision).write(
            &mut c,
            Ptr::new(ram::g_player_info(revision).at(2).address()),
        );
    }
    for counter in [0_u16, 1, 16] {
        ram::g_frame_counter(revision).write(&mut host.context(), counter);
        check(host, ids::ELEVATOR_ALTERNATE_RIDE_ENTITIES, &[]);
    }
    {
        let mut c = host.context();
        ram::g_player1_map_level(revision).write(&mut c, 1);
        ram::g_sprite_tile_buffer_offset(revision).write(&mut c, 0);
        ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut c, 0);
    }
    check(host, ids::ELEVATOR_LOAD_RIDE_BACKGROUND_TILES, &[]);
    for counter in [0_u16, 60, 120, 180, 300, 360, 419, 420] {
        for number in [0_u8, 6] {
            let mut c = host.context();
            ram::g_frame_counter(revision).write(&mut c, counter);
            for index in 0..4 {
                ram::g_elevator_ride_speech_entities(revision)
                    .at(index)
                    .write(&mut c, number);
            }
            check(host, ids::ELEVATOR_UPDATE_RIDE_DIALOGUE, &[]);
        }
    }
    check(host, ids::LOAD_FINAL_LEVEL_MAP, &[]);
    check(host, ids::MAP_CLEAR_ACTIVE_LEVEL_OBJECTS, &[]);
}

fn time_fixtures(host: &mut Host) {
    let revision = host.revision();
    for camera in 0..2 {
        {
            let mut c = host.context();
            for viewport in 0..2 {
                ram::g_camera_row_upload_pending(revision)
                    .at(viewport)
                    .write(&mut c, 0);
                ram::g_camera_column_upload_pending(revision)
                    .at(viewport)
                    .write(&mut c, 0);
            }
        }
        check(host, ids::TRY_UPDATE_CAMERA_Y, &[camera]);
        check(host, ids::TRY_UPDATE_CAMERA_X, &[camera]);
        for position in [8_i16, 9, 100, 1560, 1672, 2360] {
            let mut c = host.context();
            ram::g_camera1_pending_x(revision).write(&mut c, position);
            ram::g_camera2_pending_x(revision).write(&mut c, position);
            ram::g_camera1_pending_y(revision).write(&mut c, position);
            ram::g_camera2_pending_y(revision).write(&mut c, position);
            for id in [
                ids::MOVE_CAMERA_UP,
                ids::MOVE_CAMERA_DOWN,
                ids::MOVE_CAMERA_LEFT,
                ids::MOVE_CAMERA_RIGHT,
            ] {
                check(host, id, &[camera]);
            }
        }
    }
    {
        let mut c = host.context();
        ram::g_entity_draw_list_head(revision).write(&mut c, Ptr::new(0));
        ram::g_inventory_draw_list_head(revision).write(&mut c, Ptr::new(0));
        ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut c, 0);
        ram::g_sprite_graphics_upload_pending(revision).write(&mut c, 0);
        ram::g_camera1_pending_x(revision).write(&mut c, 100);
        ram::g_camera1_pending_y(revision).write(&mut c, 100);
        for player in 0..2 {
            let entity = ram::g_player_info(revision).at(player);
            entity.x().write(&mut c, 200 + player as i16 * 50);
            entity.y().write(&mut c, 200 + player as i16 * 50);
            entity.height().write(&mut c, 0);
            entity.height_pixels().write(&mut c, 32);
            entity.draw_flags().write(&mut c, 3);
            entity.sprite_ready().write(&mut c, 1);
        }
    }
    check(host, ids::ANIMATE_PLAYERS_INTO_SPLIT_VIEWS, &[]);
    check(host, ids::ANIMATE_SPLIT_SCREEN_WIPE, &[]);
    let record = ram::g_elevator_records(revision).at(0).at(0);
    for state in [0_i8, 1, 2, 4, 5, 6, 10, 11, 13] {
        let mut c = host.context();
        for bank in 0..2 {
            for kind in 0..2 {
                let elevator = ram::g_elevator_records(revision).at(bank).at(kind);
                elevator.state().write(&mut c, 11);
                elevator.entity().write(&mut c, 0);
            }
        }
        record.state().write(&mut c, state);
        for player in 0..2 {
            ram::g_player_elevator_flags(revision)
                .at(player)
                .write(&mut c, 0);
        }
        check(host, ids::PROCESS_ELEVATOR_STEP1, &[]);
        check(host, ids::PROCESS_ELEVATORS, &[]);
    }
    {
        let mut c = host.context();
        ram::g_player_lives(revision).at(1).write(&mut c, -1);
    }
    check(
        host,
        ids::ELEVATOR_PREPARE_RIDE_DIALOGUE,
        &[ram::g_player_info(revision).at(0).address()],
    );
    {
        let mut c = host.context();
        ram::g_player_lives(revision).at(0).write(&mut c, 3);
        ram::g_player_lives(revision).at(1).write(&mut c, 3);
        c.write_u32(tje_host::catalogue::RNG_STATE, 1);
        ram::g_elevator_ride_used_dialogue_mask(revision).write(&mut c, 0);
        ram::g_sprite_graphics_upload_pending(revision).write(&mut c, 0);
    }
    check(
        host,
        ids::ELEVATOR_PREPARE_RIDE_DIALOGUE,
        &[ram::g_player_info(revision).at(0).address()],
    );
    {
        let mut c = host.context();
        ram::g_elevator_ride_entity(revision).write(
            &mut c,
            Ptr::new(ram::g_player_info(revision).at(2).address()),
        );
        ram::g_elevator_ride_scroll_acceleration(revision).write(&mut c, 1024);
        ram::g_elevator_ride_scroll_velocity(revision).write(&mut c, 128);
        ram::g_elevator_ride_scroll_y_fixed(revision).write(&mut c, 0);
        ram::g_frame_counter(revision).write(&mut c, 1);
        let sr = c.status_register();
        c.set_status_register((sr & !0x700) | 0x600);
    }
    check(host, ids::ELEVATOR_RIDE_V_BLANK, &[]);
    {
        let mut c = host.context();
        let sr = c.status_register();
        c.set_status_register(sr & !0x700);
        ram::g_v_blank_func(revision).write(
            &mut c,
            Ptr::new(ids::ELEVATOR_RIDE_V_BLANK.metadata().address(revision)),
        );
        ram::g_elevator_ride_scroll_velocity(revision).write(&mut c, 2048);
        ram::g_elevator_ride_scroll_acceleration(revision).write(&mut c, 4096);
    }
    check(host, ids::ELEVATOR_END_RIDE_SCENE, &[]);
    {
        let mut c = host.context();
        ram::g_v_blank_func(revision).write(&mut c, Ptr::new(0));
        record.entity().write(&mut c, 2);
        record.state().write(&mut c, 1);
        for slot in [2, 4] {
            let entity = ram::g_player_info(revision).at(slot);
            entity.draw_flags().write(&mut c, 3);
            entity.level().write(&mut c, 1);
        }
        ram::g_player_lives(revision).at(1).write(&mut c, -1);
    }
    check(host, ids::ELEVATOR_BEGIN_RIDE_SCENE, &[record.address()]);
    check(host, ids::MAP_ENTER_FINAL_LEVEL, &[]);
    {
        let mut c = host.context();
        c.set_data_register(3, 26);
        ram::g_v_blank_func(revision).write(&mut c, Ptr::new(0));
        for player in 0..2 {
            ram::g_player_lives(revision).at(player).write(&mut c, 3);
            ram::g_player_elevator_flags(revision)
                .at(player)
                .write(&mut c, 0);
            let entity = ram::g_player_info(revision).at(player);
            entity.level().write(&mut c, 26);
            entity.x().write(&mut c, 500);
            entity.y().write(&mut c, 1400);
        }
    }
    // Explicitly exercise the original no-rider residual-D3 destination path.
    check(host, ids::ELEVATOR_ADVANCE_RIDERS_TO_NEXT_LEVEL, &[1]);
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let root = Path::new(&args[1]);
    let evidence = Path::new(&args[2]);
    let frames: u64 = args.get(3).map_or(10000, |value| value.parse().unwrap());
    let selection = args.get(4).map_or("logic", String::as_str);
    assert!(matches!(selection, "atomic" | "logic" | "time"));
    let timed = selection == "time";
    let atomic_only = selection == "atomic";
    fs::create_dir_all(evidence).unwrap();
    let script = Arc::new(
        InputScript::parse(
            &fs::read_to_string(root.join("crates/tje-host/examples/start-game.json")).unwrap(),
        )
        .unwrap(),
    );
    for revision in [Revision::Rev02, Revision::Rev00] {
        let name = if revision == Revision::Rev02 {
            "rev02"
        } else {
            "rev00"
        };
        let mut registry = HookRegistry::empty(revision);
        for translation in tje_game::translations()
            .filter(|entry| {
                ["camera", "elevator"].contains(&entry.id.metadata().subsystem)
                    && entry.id.metadata().wave > 0
            })
            .chain(tje_game::camera::PENDING_TRANSLATIONS)
            .chain(tje_game::elevator::PENDING_TRANSLATIONS)
        {
            let class = tje_game::camera::EXECUTION_CONTRACTS
                .iter()
                .chain(tje_game::elevator::EXECUTION_CONTRACTS)
                .find(|(id, _)| *id == translation.id)
                .map_or(ExecutionClass::Atomic, |(_, class)| *class);
            if (class == ExecutionClass::TimeAware) != timed
                || (atomic_only && class != ExecutionClass::Atomic)
            {
                continue;
            }
            registry
                .register_translation_with_execution(*translation, class)
                .unwrap();
        }
        let mut host = Host::new(
            &fs::read(root.join(format!("roms/{name}.bin"))).unwrap(),
            Mode::Check,
            registry,
        )
        .unwrap();
        host.set_wait_input_poller(script.clone());
        for frame in 0..frames {
            host.run_frame(script.at(frame)).unwrap();
        }
        assert!(host.measurements.game_frame.is_some());
        fs::write(evidence.join(format!("{name}-natural.json")), serde_json::to_vec_pretty(&serde_json::json!({"frames":frames,"checks":host.checks,"measurements":host.measurements})).unwrap()).unwrap();
        if timed {
            time_fixtures(&mut host);
        } else {
            tile_fixtures(&mut host);
            upload_fixtures(&mut host);
            elevator_fixtures(&mut host);
        }
        fs::write(
            evidence.join(format!("{name}-fixtures.json")),
            serde_json::to_vec_pretty(&host.checks).unwrap(),
        )
        .unwrap();
        if !host.checks.unexercised.is_empty() || !host.checks.uninitialized_stack_reads.is_empty()
        {
            FAILURES.fetch_add(1, Ordering::Relaxed);
        }
        println!(
            "{name}: {} calls, {} mismatches; fixture errors remain fatal",
            host.checks.calls, host.checks.mismatches
        );
        println!("{}", serde_json::to_string_pretty(&host.checks).unwrap());
    }
    assert_eq!(
        FAILURES.load(Ordering::Relaxed),
        0,
        "failed fixtures; evidence retained for both revisions"
    );
}
