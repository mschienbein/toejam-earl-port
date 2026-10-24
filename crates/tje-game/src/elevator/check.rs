//! Private-ROM opt-in harness, built with scratch/W0World/check2/Cargo.toml.
use std::{
    env, fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tje_game::{
    Context, FunctionId, Translation, camera, elevator, execution::ExecutionClass, ids,
};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Ptr, Revision, ram};

static FAILURES: AtomicUsize = AtomicUsize::new(0);

// Fixed proof batches, rather than a snapshot of whichever entries happen to
// be globally registered. Proof overlays preserve Rust-to-Rust composition.
const FUNCTIONAL_PROOF: &[Translation] = &[
    Translation {
        id: ids::UPDATE_CAMERA_Y,
        function: camera::update_camera_y,
    },
    Translation {
        id: ids::UPDATE_CAMERA_X,
        function: camera::update_camera_x,
    },
    Translation {
        id: ids::REDRAW_CAMERA_TILE_ROWS,
        function: camera::redraw_camera_tile_rows,
    },
    Translation {
        id: ids::SET_CAMERA_VIEW_ORIGIN,
        function: camera::set_camera_view_origin,
    },
    Translation {
        id: ids::SET_CAMERA_POSITION,
        function: camera::set_camera_position,
    },
    Translation {
        id: ids::LOAD_FINAL_LEVEL_MAP,
        function: elevator::load_final_level_map,
    },
    Translation {
        id: ids::ELEVATOR_LOAD_RIDE_BACKGROUND_TILES,
        function: elevator::elevator_load_ride_background_tiles,
    },
    Translation {
        id: ids::ELEVATOR_ALTERNATE_RIDE_ENTITIES,
        function: elevator::elevator_alternate_ride_entities,
    },
];

const TIME_PROOF: &[Translation] = &[
    Translation {
        id: ids::UPDATE_CAMERA_FOLLOWING,
        function: camera::update_camera_following,
    },
    Translation {
        id: ids::TRY_UPDATE_CAMERA_Y,
        function: camera::try_update_camera_y,
    },
    Translation {
        id: ids::TRY_UPDATE_CAMERA_X,
        function: camera::try_update_camera_x,
    },
    Translation {
        id: ids::MOVE_CAMERA_UP,
        function: camera::move_camera_up,
    },
    Translation {
        id: ids::MOVE_CAMERA_DOWN,
        function: camera::move_camera_down,
    },
    Translation {
        id: ids::MOVE_CAMERA_LEFT,
        function: camera::move_camera_left,
    },
    Translation {
        id: ids::MOVE_CAMERA_RIGHT,
        function: camera::move_camera_right,
    },
    Translation {
        id: ids::ANIMATE_SPLIT_SCREEN_WIPE,
        function: camera::animate_split_screen_wipe,
    },
    Translation {
        id: ids::ANIMATE_PLAYERS_INTO_SPLIT_VIEWS,
        function: camera::animate_players_into_split_views,
    },
    Translation {
        id: ids::MAP_ENTER_FINAL_LEVEL,
        function: elevator::map_enter_final_level,
    },
    Translation {
        id: ids::ELEVATOR_PREPARE_RIDE_DIALOGUE,
        function: elevator::elevator_prepare_ride_dialogue,
    },
    Translation {
        id: ids::ELEVATOR_RIDE_V_BLANK,
        function: elevator::elevator_ride_v_blank,
    },
    Translation {
        id: ids::ELEVATOR_BEGIN_RIDE_SCENE,
        function: elevator::elevator_begin_ride_scene,
    },
    Translation {
        id: ids::ELEVATOR_END_RIDE_SCENE,
        function: elevator::elevator_end_ride_scene,
    },
    Translation {
        id: ids::ELEVATOR_ADVANCE_RIDERS_TO_NEXT_LEVEL,
        function: elevator::elevator_advance_riders_to_next_level,
    },
    Translation {
        id: ids::PROCESS_ELEVATOR_STEP1,
        function: elevator::process_elevator_step1,
    },
    Translation {
        id: ids::PROCESS_ELEVATORS,
        function: elevator::process_elevators,
    },
];

// Dependency-closed timed batch which does not contain the known blocked
// raster/cycle-charged scene paths or their departure-state-machine callers.
// `time-all` remains the explicit diagnostic for all seventeen candidates.
// The player-center animation also remains pending: at the 10,000-frame state,
// the completed ISR observes Z80 byte A0004A as 4 versus 3, not a YM BUSY spin.
const TIME_READY_PROOF: &[Translation] = &[
    Translation {
        id: ids::UPDATE_CAMERA_FOLLOWING,
        function: camera::update_camera_following,
    },
    Translation {
        id: ids::TRY_UPDATE_CAMERA_Y,
        function: camera::try_update_camera_y,
    },
    Translation {
        id: ids::TRY_UPDATE_CAMERA_X,
        function: camera::try_update_camera_x,
    },
    Translation {
        id: ids::MOVE_CAMERA_UP,
        function: camera::move_camera_up,
    },
    Translation {
        id: ids::MOVE_CAMERA_DOWN,
        function: camera::move_camera_down,
    },
    Translation {
        id: ids::MOVE_CAMERA_LEFT,
        function: camera::move_camera_left,
    },
    Translation {
        id: ids::MOVE_CAMERA_RIGHT,
        function: camera::move_camera_right,
    },
    Translation {
        id: ids::ELEVATOR_RIDE_V_BLANK,
        function: elevator::elevator_ride_v_blank,
    },
];

// Prove the original caller's real split-transition envelope independently of
// the longer ride scenes. Untranslated scene callees keep their original body.
const TRANSITION_PROOF: &[Translation] = &[
    Translation {
        id: ids::ANIMATE_SPLIT_SCREEN_WIPE,
        function: camera::animate_split_screen_wipe,
    },
    Translation {
        id: ids::ANIMATE_PLAYERS_INTO_SPLIT_VIEWS,
        function: camera::animate_players_into_split_views,
    },
    Translation {
        id: ids::PROCESS_ELEVATOR_STEP1,
        function: elevator::process_elevator_step1,
    },
    Translation {
        id: ids::PROCESS_ELEVATORS,
        function: elevator::process_elevators,
    },
];

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
    // ASL.W #7 + ADDA.W: player 512 wraps to slot zero, not ROM at
    // gPlayerInfo + 65536. Negative indexes preserve the same word arithmetic.
    for player in [-512_i16, -1, 512] {
        ram::g_player_elevator_flags(revision)
            .at(i32::from(player))
            .write(&mut host.context(), 0);
        check(
            host,
            ids::ELEVATOR_BLOCKS_PLAYER_MOVEMENT,
            &[player as u32, 0, (-1_i32) as u32, 0],
        );
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
    check(host, ids::UPDATE_CAMERA_FOLLOWING, &[]);
    for split in [0_i8, 1] {
        for lives in [(3_i8, 3_i8), (3, -1), (-1, 3)] {
            for velocity in [-512_i16, 0, 512] {
                let mut c = host.context();
                ram::g_split_screen_enabled(revision).write(&mut c, split);
                for (player, life) in [lives.0, lives.1].into_iter().enumerate() {
                    let player = player as i32;
                    ram::g_player_lives(revision).at(player).write(&mut c, life);
                    ram::g_player_inventory_open(revision)
                        .at(player)
                        .write(&mut c, 0);
                    ram::g_player_fall_states(revision)
                        .at(player)
                        .write(&mut c, 0);
                    let entity = ram::g_player_info(revision).at(player);
                    entity.x().write(&mut c, 260);
                    entity.y().write(&mut c, 200);
                    entity.height().write(&mut c, 0);
                    entity.height_pixels().write(&mut c, 32);
                    entity.frame_phase().write(&mut c, 0);
                    entity.animation_motion_axes().write(&mut c, 0);
                    entity.draw_flags().write(&mut c, 1);
                    entity.animation_group_mask().write(&mut c, 0);
                    entity.sprite_state().write(&mut c, 0);
                    entity.velocity_x().write(&mut c, velocity);
                    entity.velocity_y().write(&mut c, velocity);
                    entity.velocity_height().write(&mut c, 0);
                    ram::g_camera_row_upload_pending(revision)
                        .at(player)
                        .write(&mut c, 0);
                    ram::g_camera_column_upload_pending(revision)
                        .at(player)
                        .write(&mut c, 0);
                }
                for position in [
                    ram::g_camera1_pending_x(revision),
                    ram::g_camera1_pending_y(revision),
                    ram::g_camera2_pending_x(revision),
                    ram::g_camera2_pending_y(revision),
                ] {
                    position.write(&mut c, 100);
                }
                for fixed in [
                    ram::g_camera1_follow_x_fixed(revision),
                    ram::g_camera1_follow_y_fixed(revision),
                    ram::g_camera2_follow_x_fixed(revision),
                    ram::g_camera2_follow_y_fixed(revision),
                ] {
                    fixed.write(&mut c, 100 << 8);
                }
                for cursor in [
                    ram::g_camera1_tile_row_cursor(revision),
                    ram::g_camera2_tile_row_cursor(revision),
                    ram::g_camera1_tile_column_cursor(revision),
                    ram::g_camera2_tile_column_cursor(revision),
                ] {
                    cursor.write(&mut c, 11);
                }
                check(host, ids::UPDATE_CAMERA_FOLLOWING, &[]);
            }
        }
    }
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
        // The only original caller, EnterSplitScreen (startup_and_main_loop.c:
        // 380-388), drains sprite uploads and disables BOTH blanking graphics
        // handlers before either animation. Keep the display and IRQs enabled.
        ram::g_v_blank_graphics_enabled(revision).write(&mut c, 0);
        ram::g_h_blank_graphics_enabled(revision).write(&mut c, 0);
        tje_mem::Platform::write_vdp_control(&mut c, 0x9200);
        ram::g_entity_draw_list_head(revision).write(&mut c, Ptr::new(0));
        ram::g_inventory_draw_list_head(revision).write(&mut c, Ptr::new(0));
        ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut c, 0);
        ram::g_sprite_graphics_upload_pending(revision).write(&mut c, 0);
        ram::g_camera1_pending_x(revision).write(&mut c, 100);
        ram::g_camera1_pending_y(revision).write(&mut c, 100);
        for player in 0..2 {
            ram::g_player_lives(revision).at(player).write(&mut c, 3);
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
    {
        // The state machines are called by the normal gameplay main loop.
        let mut c = host.context();
        ram::g_v_blank_graphics_enabled(revision).write(&mut c, 1);
        ram::g_h_blank_graphics_enabled(revision).write(&mut c, 1);
    }
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
        // ElevatorBeginRideScene:602 disables both graphics handlers after
        // draining its sprite uploads, immediately before preparing dialogue.
        ram::g_v_blank_graphics_enabled(revision).write(&mut c, 0);
        ram::g_h_blank_graphics_enabled(revision).write(&mut c, 0);
        ram::g_sprite_graphics_upload_pending(revision).write(&mut c, 0);
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
    for counter in [1_u16, 0, 16] {
        ram::g_frame_counter(revision).write(&mut host.context(), counter);
        check(host, ids::ELEVATOR_RIDE_V_BLANK, &[]);
    }
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
        // BeginRideScene must still have the normal IRQ upload service while
        // its newly selected elevator frames become resident (scene.c:595-602).
        ram::g_v_blank_graphics_enabled(revision).write(&mut c, 1);
        ram::g_h_blank_graphics_enabled(revision).write(&mut c, 1);
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
    {
        // The only caller is PlayShipCompletionSequence:254-255, after Ending.
        // PlayEndingSequence:34 leaves blanking graphics disabled, and its final
        // bouncing scene clears display enable at ending.c:530-531. Reproduce
        // this actual caller envelope, not an artificial no-wait workaround.
        let mut c = host.context();
        ram::g_v_blank_graphics_enabled(revision).write(&mut c, 0);
        ram::g_h_blank_graphics_enabled(revision).write(&mut c, 0);
        let mode = ram::g_mode2_reg_softcopy(revision).read(&c) & 0x3c;
        ram::g_mode2_reg_softcopy(revision).write(&mut c, mode);
        tje_mem::Platform::write_vdp_control(&mut c, 0x8104 | u16::from(mode));
    }
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
    let selection = args.get(4).map_or("functional", String::as_str);
    assert!(matches!(
        selection,
        "atomic" | "functional" | "time" | "time-all" | "transition" | "closure"
    ));
    // Optional named roots isolate a failing entry without hiding its Rust
    // candidate callees. This is a diagnostic filter, not an approval waiver.
    let selected_roots: Vec<_> = args
        .get(5)
        .map(|names| {
            assert_eq!(selection, "time-all");
            names
                .split(',')
                .map(|name| {
                    TIME_PROOF
                        .iter()
                        .find(|entry| entry.id.metadata().name == name)
                        .unwrap_or_else(|| panic!("unknown timed proof root: {name}"))
                        .id
                })
                .collect()
        })
        .unwrap_or_default();
    if selection == "closure" {
        let roots: Vec<_> = tje_game::translations()
            .filter(|entry| {
                ["camera", "elevator"].contains(&entry.id.metadata().subsystem)
                    && entry.id.metadata().wave > 0
            })
            .map(|entry| entry.id)
            .collect();
        match tje_game::Dispatcher::native(&roots) {
            Ok(_) => println!(
                "Native closure complete for {} delivered roots",
                roots.len()
            ),
            Err(missing) => {
                println!(
                    "Native closure incomplete for {} delivered roots; {} untranslated dependencies:",
                    roots.len(),
                    missing.len()
                );
                for id in missing {
                    println!("{} ({})", id.metadata().name, id.metadata().inventory_id);
                }
            }
        }
        return;
    }
    let timed = matches!(selection, "time" | "time-all" | "transition");
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
        let registry = if selection == "time-all" {
            HookRegistry::proof_filtered(revision, TIME_PROOF, &selected_roots).unwrap()
        } else if selection == "transition" {
            HookRegistry::proof(revision, TRANSITION_PROOF).unwrap()
        } else if timed {
            HookRegistry::proof(revision, TIME_READY_PROOF).unwrap()
        } else if selection == "functional" {
            HookRegistry::proof(revision, FUNCTIONAL_PROOF).unwrap()
        } else {
            let mut registry = HookRegistry::empty(revision);
            for translation in tje_game::translations().filter(|entry| {
                ["camera", "elevator"].contains(&entry.id.metadata().subsystem)
                    && entry.id.metadata().wave > 0
                    && tje_game::execution_class(entry.id) == Some(ExecutionClass::Atomic)
            }) {
                registry
                    .register_translation_with_execution(
                        tje_game::Dispatcher::hybrid().resolve_entry(translation.id).unwrap().unwrap(),
                        ExecutionClass::Atomic,
                    )
                    .unwrap();
            }
            registry
        };
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
        // A frame boundary can be inside a pending/active IRQ. Finish original
        // work before changing any fixture inputs; never lower SR to fake a
        // main-call context or manufacture the counter/ready state being waited on.
        let boundary = host
            .settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        fs::write(
            evidence.join(format!("{name}-boundary.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "ticks": boundary.ticks, "master_clocks": boundary.master_clocks,
                "start_pc": boundary.start_pc, "end_pc": boundary.end_pc,
                "start_frame": boundary.start_frame, "end_frame": boundary.end_frame,
                "handlers_completed": boundary.handlers_completed,
                "completed_frames": boundary.completed_frames.len(),
                "entry_sr": host.cpu().status_register(),
            }))
            .unwrap(),
        )
        .unwrap();
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
