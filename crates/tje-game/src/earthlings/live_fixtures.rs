//! Opt-in host proof target; the private harness manifest is recorded in the lane report.
//! Inputs start at power-on; all synthetic tests below are labeled live-state fixtures.
use tje_game::{Context, FunctionId, earthlings, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Ptr, Revision, ram, rom};

/// Resolve canonical bodies once so proof roots remain stable across promotion.
/// This test-only table includes pending callees rather than bypassing them.
pub(super) fn proof_translations() -> &'static [tje_game::Translation] {
    static TABLE: std::sync::LazyLock<Vec<tje_game::Translation>> =
        std::sync::LazyLock::new(|| {
            earthlings::TRANSLATIONS
                .iter()
                .chain(earthlings::PENDING_TRANSLATIONS)
                .copied()
                .collect()
        });
    &TABLE
}

fn fixture(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| {
            panic!(
                "{:?} {} {arguments:08x?}: {error}",
                host.revision(),
                id.metadata().name
            )
        });
}

fn live_fixtures(host: &mut Host) {
    let revision = host.revision();
    let earthling = ram::g_earthling_banks(revision).at(0).at(0);
    for kind in [-1, 0, 9, 18] {
        for state in [0, 6, 7] {
            earthling.kind().write(&mut host.context(), kind);
            earthling.state().write(&mut host.context(), state);
            earthling.object().write(&mut host.context(), 0);
            for index in [0, 58] {
                ram::g_random_cache_index(revision).write(&mut host.context(), index);
                fixture(
                    host,
                    ids::EARTHLING_SELECT_TURN_STATE,
                    &[earthling.address()],
                );
            }
        }
    }
    for x in [i16::MIN, -340, -339, 0, 339, 340, i16::MAX] {
        for y in [-200_i16, -199, 0, 199, 200] {
            earthling.kind().write(&mut host.context(), 0);
            earthling.x().write(&mut host.context(), 0);
            earthling.y().write(&mut host.context(), 0);
            ram::g_earthling_bank_users(revision)
                .at(0)
                .write(&mut host.context(), 1);
            ram::g_earthling_bank_levels(revision)
                .at(0)
                .write(&mut host.context(), 1);
            fixture(
                host,
                ids::EARTHLING_IS_SPECIAL_RECORD_NEARBY,
                &[x as u32, y as u32, 1],
            );
        }
    }
    let object = ram::g_player_info(revision).at(6);
    let target = ram::g_player_info(revision).at(7);
    for latch in [0, 1, -1] {
        for mask in [0, 0x800, 0x40000, 0x40800, u32::MAX] {
            object
                .attachment_x_or_latch()
                .write(&mut host.context(), latch);
            target
                .animation_group_mask()
                .write(&mut host.context(), mask);
            fixture(
                host,
                ids::ENTITY_TRIGGER_TARGET_STATUS_EVENT_ONCE,
                &[object.address(), target.address()],
            );
        }
    }
    for bank in 0..2 {
        for level in [-1_i16, 0, 1, 2, 3, 4, 12, 25, 26] {
            fixture(
                host,
                ids::EARTHLING_CONFIGURE_MAILBOX_LEVEL_BANK,
                &[bank, level as u32],
            );
        }
    }
    for slot in 0..20 {
        ram::g_earthling_banks(revision)
            .at(0)
            .at(slot)
            .kind()
            .write(&mut host.context(), -1);
    }
    for kind in [0, 1, 14, 18, 24] {
        let definition = rom::earthling_definitions(revision).at(kind);
        for x in [-1_i16, 256, 1024, i16::MAX] {
            fixture(
                host,
                ids::CHECK_EARTHLING_OVERLAP,
                &[definition.address(), 0, x as u32, 500, 1],
            );
        }
    }
    for state in [0, 4, 5, 6, 7, 8] {
        fixture(
            host,
            ids::EARTHLING_PROBE_DIRECTIONAL_TERRAIN,
            &[0, 512, 512, state],
        );
    }
    earthling.kind().write(&mut host.context(), 19);
    earthling.object().write(&mut host.context(), 6);
    earthling.level().write(&mut host.context(), 1);
    object.level().write(&mut host.context(), 1);
    for state in [0, 4, 8, 12] {
        for terrain in [0x10, 0x50, 0x80, 0x90] {
            for duration in [0, 1] {
                earthling.state().write(&mut host.context(), state);
                earthling
                    .saved_x_or_status()
                    .write(&mut host.context(), terrain);
                object.frame_duration().write(&mut host.context(), duration);
                fixture(
                    host,
                    ids::EARTHLING_SELECT_TERRAIN_ANIMATION_STATE,
                    &[earthling.address(), object.address(), 6],
                );
            }
        }
    }
    for x in [-40_i16, 0, 40] {
        for previous in [-256_i16, 0, 256] {
            object.velocity_x().write(&mut host.context(), previous);
            fixture(
                host,
                ids::ENTITY_STEER_WITH_HORIZONTAL_DRIFT,
                &[object.address(), target.address(), x as u32, 20],
            );
        }
    }
    object.x().write(&mut host.context(), 512);
    object.y().write(&mut host.context(), 512);
    fixture(
        host,
        ids::ENTITY_UPDATE_TERRAIN_DRAW_FLAG,
        &[object.address()],
    );
    for slot in 0..6 {
        ram::g_mole_effect_entities(revision)
            .at(slot)
            .write(&mut host.context(), 0);
    }
    fixture(
        host,
        ids::EARTHLING_SPAWN_MOLE_EFFECT,
        &[earthling.address(), object.address()],
    );
    let step = ram::g_player_info(revision)
        .at(0)
        .current_frame()
        .read(&host.context())
        .address();
    assert_ne!(
        step, 0,
        "live player must supply a render-frame ROM pointer"
    );
    object
        .earthling()
        .write(&mut host.context(), Ptr::new(earthling.address()));
    object.behavior_flags().write(&mut host.context(), 0);
    object.animation_motion_axes().write(&mut host.context(), 3);
    earthling.state().write(&mut host.context(), 6);
    earthling
        .health_visibility()
        .at(1)
        .write(&mut host.context(), 6);
    ram::g_player_inventory_open(revision)
        .at(0)
        .write(&mut host.context(), 0);
    ram::g_player2_inventory_open(revision).write(&mut host.context(), 0);
    for kind in [1, 6, 7, 19, 22] {
        earthling.kind().write(&mut host.context(), kind);
        for velocity in [-320_i16, 0, 320] {
            object.velocity_x().write(&mut host.context(), velocity);
            object.velocity_y().write(&mut host.context(), velocity);
            fixture(
                host,
                ids::EARTHLING_MOVE_WITHIN_MAP_BOUNDS,
                &[step, object.address()],
            );
        }
    }
}

fn state_fixtures(host: &mut Host) {
    let revision = host.revision();
    let earthling = ram::g_earthling_banks(revision).at(0).at(0);
    let object = ram::g_player_info(revision).at(6);
    let step = ram::g_player_info(revision)
        .at(0)
        .current_frame()
        .read(&host.context())
        .address();
    for kind in [1, 5, 10, 13, 14, 17, 18] {
        earthling.kind().write(&mut host.context(), kind);
        earthling.state().write(&mut host.context(), 6);
        for velocity in [-320_i16, 0, 320] {
            object.velocity_x().write(&mut host.context(), velocity);
            object.velocity_y().write(&mut host.context(), velocity);
            fixture(
                host,
                ids::EARTHLING_MOVE_ON_ALLOWED_TERRAIN,
                &[step, object.address()],
            );
        }
    }
    earthling.kind().write(&mut host.context(), 1);
    earthling.second_object().write(&mut host.context(), 0);
    for state in [0, 2, 4, 6, 7] {
        for frame in [-1, 0, 3] {
            object.frame().write(&mut host.context(), frame);
            fixture(
                host,
                ids::SET_EARTHLING_STATE,
                &[earthling.address(), state],
            );
        }
    }
    earthling.object().write(&mut host.context(), 0);
    earthling
        .saved_x_or_status()
        .write(&mut host.context(), -32768);
    earthling.saved_y().write(&mut host.context(), 32767);
    fixture(
        host,
        ids::EARTHLING_RESTORE_SAVED_POSITION,
        &[earthling.address()],
    );
    fixture(
        host,
        ids::EARTHLING_REMOVE_FROM_LEVEL,
        &[earthling.address()],
    );
    for bank in 0..2 {
        for slot in 0..20 {
            let record = ram::g_earthling_banks(revision).at(bank).at(slot);
            record.object().write(&mut host.context(), 0);
            record.second_object().write(&mut host.context(), 0);
            record.kind().write(&mut host.context(), -1);
        }
        ram::g_earthling_bank_levels(revision)
            .at(bank)
            .write(&mut host.context(), 1);
    }
    for users in [0, 1, 2] {
        ram::g_earthling_bank_users(revision)
            .at(0)
            .write(&mut host.context(), users);
        fixture(host, ids::EARTHLING_RELEASE_LEVEL_BANK, &[1]);
    }
    ram::g_player_boom_box_entities(revision)
        .at(0)
        .write(&mut host.context(), 6);
    ram::g_player_boom_box_entities(revision)
        .at(1)
        .write(&mut host.context(), 0);
    object.level().write(&mut host.context(), 1);
    earthling.object().write(&mut host.context(), 7);
    let x = object.x().read(&host.context());
    earthling.x().write(&mut host.context(), x);
    let y = object.y().read(&host.context());
    earthling.y().write(&mut host.context(), y);
    for kind in [0, 1, 7, 10, 11, 14, 17, 18, 20] {
        for state in [0, 3, 4, 99] {
            earthling.kind().write(&mut host.context(), kind);
            earthling.state().write(&mut host.context(), state);
            fixture(host, ids::EARTHLING_APPLY_NEARBY_SLEEP, &[]);
        }
    }
    fixture(
        host,
        ids::RELEASE_FINISHED_MOLE_EFFECTS_AND_STOLEN_PRESENTS,
        &[],
    );
    for player in 0..2 {
        fixture(host, ids::PLAYER_CREATE_LIFE_REQUEST_DISPLAY, &[player]);
    }
    for player in 0..2 {
        for state in [0, 5, 6] {
            ram::g_player_info(revision)
                .at(player)
                .sprite_state()
                .write(&mut host.context(), state);
            ram::g_player_interaction_choice_active(revision)
                .at(player)
                .write(&mut host.context(), 1);
            ram::g_player_interaction_choice_entities(revision)
                .at(player)
                .write(&mut host.context(), 6);
            for input in [0, 1, 2, 0x40] {
                ram::g_player_processed_input(revision)
                    .at(player)
                    .write(&mut host.context(), input);
                ram::g_player_previous_processed_input(revision)
                    .at(player)
                    .write(&mut host.context(), 0);
                fixture(host, ids::CHECK_A_BUTTON, &[]);
            }
        }
    }
}

fn behavior_fixtures(host: &mut Host) {
    let revision = host.revision();
    let earthling = ram::g_earthling_banks(revision).at(0).at(0);
    let object = ram::g_player_info(revision).at(6);
    for player in 0..2 {
        ram::g_player_interaction_choice_active(revision)
            .at(player)
            .write(&mut host.context(), 0);
        ram::g_player_info(revision)
            .at(player)
            .sprite_state()
            .write(&mut host.context(), 0);
        ram::g_player_info(revision)
            .at(player)
            .level()
            .write(&mut host.context(), 1);
        ram::g_player_inventory_open(revision)
            .at(player)
            .write(&mut host.context(), 0);
    }
    earthling.object().write(&mut host.context(), 6);
    earthling.second_object().write(&mut host.context(), 0);
    earthling.level().write(&mut host.context(), 1);
    object.level().write(&mut host.context(), 1);
    object.x().write(&mut host.context(), 500);
    object.y().write(&mut host.context(), 500);
    for (kind, id) in [
        (12, ids::EARTHLING_THINK_HORIZONTAL_WANDER),
        (10, ids::EARTHLING_ADVANCE_HORIZONTAL_TURN_ANIMATION),
        (11, ids::EARTHLING_THINK_HORIZONTAL_REVERSAL),
        (8, ids::EARTHLING_THINK_TARGETED_RESET_CYCLE),
    ] {
        for state in [0, 2, 3, 4, 6, 7] {
            for duration in [0, 1] {
                earthling.kind().write(&mut host.context(), kind);
                earthling.state().write(&mut host.context(), state);
                earthling.timer().write(&mut host.context(), 0);
                earthling.state_delay().write(&mut host.context(), 0);
                earthling
                    .health_visibility()
                    .at(0)
                    .write(&mut host.context(), 1);
                object.sprite_state().write(&mut host.context(), 0);
                object.frame_duration().write(&mut host.context(), duration);
                object.behavior_flags().write(&mut host.context(), 0);
                fixture(host, id, &[earthling.address()]);
            }
        }
    }
    earthling.kind().write(&mut host.context(), 19);
    for state in [0, 4, 8, 12] {
        for terrain in [0x10, 0x50, 0x80, 0x90] {
            earthling.state().write(&mut host.context(), state);
            earthling
                .saved_x_or_status()
                .write(&mut host.context(), terrain);
            object.frame_duration().write(&mut host.context(), 0);
            fixture(
                host,
                ids::EARTHLING_UPDATE_TERRAIN_ANIMATION_STATE,
                &[earthling.address(), object.address()],
            );
        }
    }
    earthling.kind().write(&mut host.context(), 1);
    earthling.state().write(&mut host.context(), 99);
    fixture(host, ids::EARTHLING_WAKE_LEVEL, &[1]);
    for mask in [0, 2, 4, 6, 0xff] {
        earthling.state().write(&mut host.context(), 2);
        fixture(host, ids::EARTHLING_RESET_STATE_TWO_BY_DRAW_MASK, &[mask]);
    }
    for kind in [0, 1, 9, 10, 11, 17, 20, 23, 24] {
        for health in [0, 1, 2] {
            earthling.kind().write(&mut host.context(), kind);
            earthling
                .health_visibility()
                .at(0)
                .write(&mut host.context(), health);
            earthling.object().write(&mut host.context(), 0);
            fixture(host, ids::DAMAGE_EARTHLING, &[earthling.address()]);
        }
    }
    earthling.kind().write(&mut host.context(), 1);
    earthling.object().write(&mut host.context(), 0);
    fixture(host, ids::EARTHLING_RELOCATE_RECORD, &[earthling.address()]);
    earthling.kind().write(&mut host.context(), 20);
    earthling.object().write(&mut host.context(), 6);
    earthling.state().write(&mut host.context(), 0);
    fixture(host, ids::DROP_PRESENTS_ABILITY, &[0, earthling.address()]);
}

fn special_fixtures(host: &mut Host) {
    let revision = host.revision();
    let earthling = ram::g_earthling_banks(revision).at(0).at(0);
    let object = ram::g_player_info(revision).at(6);
    earthling.object().write(&mut host.context(), 6);
    earthling.second_object().write(&mut host.context(), 0);
    for kind in [1, 4, 9, 15, 16, 17, 18, 21] {
        for state in [0, 3, 4, 8] {
            earthling.kind().write(&mut host.context(), kind);
            earthling.state().write(&mut host.context(), state);
            earthling.state_delay().write(&mut host.context(), 1);
            earthling.timer().write(&mut host.context(), 1);
            earthling.saved_x_or_status().write(&mut host.context(), 1);
            for duration in [0, 1, 4] {
                object.frame_duration().write(&mut host.context(), duration);
                fixture(
                    host,
                    ids::EARTHLING_PROCESS_SPECIAL_THINK_STATES,
                    &[earthling.address(), object.address()],
                );
            }
        }
    }
    earthling.kind().write(&mut host.context(), 20);
    object.draw_flags().write(&mut host.context(), 6);
    for state in [0, 1, 2, 3, 4, 6, 7] {
        for duration in [0, 1] {
            earthling.state().write(&mut host.context(), state);
            object.frame_duration().write(&mut host.context(), duration);
            object.height().write(&mut host.context(), 40);
            object.velocity_x().write(&mut host.context(), 0);
            object.velocity_y().write(&mut host.context(), 0);
            fixture(host, ids::SANTA_LOGIC, &[earthling.address()]);
        }
    }
    for player in 0..2 {
        fixture(
            host,
            ids::PLAYER_CREATE_INTERACTION_CHOICE_DISPLAY,
            &[player],
        );
    }
    for kind in [0, 1, 6, 7, 14, 17, 20, 22] {
        earthling.kind().write(&mut host.context(), kind);
        earthling
            .health_visibility()
            .at(0)
            .write(&mut host.context(), 2);
        for mask in [0, 2, 4, 6] {
            fixture(host, ids::DO_VIKING_ABILITY, &[mask]);
        }
        fixture(host, ids::EARTHLING_DAMAGE_ON_HAZARD_TERRAIN, &[0, 1]);
    }
    // AssignShipPieceLevels requires the same empty assignment array as new game.
    for piece in 0..10 {
        ram::g_ship_piece_level_assignments(revision)
            .at(piece)
            .write(&mut host.context(), 0);
    }
    fixture(host, ids::SETUP_INITIAL_EARTHLINGS, &[]);
}

fn allocation_and_timed_fixtures(host: &mut Host) {
    let revision = host.revision();
    let earthling = ram::g_earthling_banks(revision).at(0).at(0);
    for kind in 0..25 {
        for random in [0, 1] {
            earthling.kind().write(&mut host.context(), kind);
            earthling.object().write(&mut host.context(), 0);
            earthling.second_object().write(&mut host.context(), 0);
            earthling.level().write(&mut host.context(), 1);
            earthling.x().write(&mut host.context(), 512);
            earthling.y().write(&mut host.context(), 512);
            ram::g_random_cache_index(revision).write(&mut host.context(), 0);
            ram::g_cached_random_values(revision)
                .at(1)
                .write(&mut host.context(), random);
            fixture(
                host,
                ids::EARTHLING_ALLOCATE_ENTITIES,
                &[earthling.address()],
            );
        }
    }
    let object = ram::g_player_info(revision).at(6);
    earthling.object().write(&mut host.context(), 6);
    earthling.second_object().write(&mut host.context(), 0);
    object.level().write(&mut host.context(), 1);
    object.x().write(&mut host.context(), 512);
    object.y().write(&mut host.context(), 512);
    for kind in [6, 7, 19, 22] {
        for state in [0, 2, 4, 8] {
            for timer in [-1_i16, 0, 1] {
                earthling.kind().write(&mut host.context(), kind);
                earthling.state().write(&mut host.context(), state);
                earthling.timer().write(&mut host.context(), timer);
                earthling.state_delay().write(&mut host.context(), 0);
                object.sprite_state().write(&mut host.context(), state);
                object.frame_duration().write(&mut host.context(), 1);
                fixture(
                    host,
                    ids::EARTHLING_THINK_TIMED_MOVEMENT,
                    &[earthling.address()],
                );
            }
        }
    }
}

fn visibility_fixtures(host: &mut Host) {
    let revision = host.revision();
    let records = ram::g_earthling_banks(revision).at(0);
    for slot in 0..20 {
        let record = records.at(slot);
        record.kind().write(&mut host.context(), -1);
        record.object().write(&mut host.context(), 0);
        record.second_object().write(&mut host.context(), 0);
    }
    let record = records.at(0);
    record.kind().write(&mut host.context(), 1);
    record.level().write(&mut host.context(), 1);
    record.y().write(&mut host.context(), 512);
    for x in [-32768_i16, 512, 32767] {
        record.x().write(&mut host.context(), x);
        for parity in [0, 1] {
            ram::g_earthling_update_parity(revision).write(&mut host.context(), parity);
            fixture(host, ids::LOAD_IN_EARTHLINGS, &[records.address()]);
        }
    }
}

pub(super) fn time_aware_fixtures(host: &mut Host) {
    let revision = host.revision();
    let record = ram::g_earthling_banks(revision).at(0).at(0);
    let object = ram::g_player_info(revision).at(6);
    for number in 0..2 {
        let player = ram::g_player_info(revision).at(number);
        player.x().write(&mut host.context(), 512);
        player.y().write(&mut host.context(), 512);
        player.level().write(&mut host.context(), 1);
        player.sprite_state().write(&mut host.context(), 0);
        player.animation_group_mask().write(&mut host.context(), 0);
        player.behavior_flags().write(&mut host.context(), 1);
        ram::g_player_interaction_choice_active(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_inventory_open(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_elevator_flags(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_wizard_interaction_states(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_opera_singer_interaction_states(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_wiseman_interaction_states(revision)
            .at(number)
            .write(&mut host.context(), 0);
    }
    record.object().write(&mut host.context(), 6);
    record.second_object().write(&mut host.context(), 0);
    record.level().write(&mut host.context(), 1);
    record.x().write(&mut host.context(), 512);
    record.y().write(&mut host.context(), 512);
    object.x().write(&mut host.context(), 512);
    object.y().write(&mut host.context(), 512);
    object.level().write(&mut host.context(), 1);
    object.draw_flags().write(&mut host.context(), 3);
    object.frame().write(&mut host.context(), 5);
    object.frame_duration().write(&mut host.context(), 0);
    for (kind, id) in [
        (10, ids::AI_WIZARD_TOUCH),
        (17, ids::AI_OPERA_SINGER_TOUCH),
        (11, ids::AI_WISEMAN_TOUCH),
        (9, ids::AI_MAILBOX_TOUCH),
    ] {
        record.kind().write(&mut host.context(), kind);
        record.state().write(&mut host.context(), 0);
        ram::g_mailbox_level_states(revision)
            .at(0)
            .level()
            .write(&mut host.context(), 1);
        ram::g_mailbox_level_states(revision)
            .at(0)
            .mode()
            .write(&mut host.context(), 0);
        ram::g_mailbox_level_states(revision)
            .at(0)
            .contact()
            .at(0)
            .write(&mut host.context(), 0);
        fixture(host, id, &[0, record.address()]);
    }
    record.kind().write(&mut host.context(), 1);
    record.state().write(&mut host.context(), 0);
    record.timer().write(&mut host.context(), 0);
    record.state_delay().write(&mut host.context(), 0);
    fixture(host, ids::AI_GENERIC_ENEMY_THINK, &[record.address()]);
    for id in [
        ids::EARTHLING_DISPATCH_ABILITIES,
        ids::EARTHLING_DISPATCH_PLAYER_CONTACTS,
        ids::PROCESS_EARTHLINGS,
    ] {
        fixture(host, id, &[]);
    }
    ram::g_life_request_pending(revision).write(&mut host.context(), 0);
    fixture(host, ids::PLAYER_PROCESS_LIFE_REQUEST, &[1]);
}

fn decrement_boundary_fixtures(host: &mut Host) {
    let revision = host.revision();
    let record = ram::g_earthling_banks(revision).at(0).at(0);
    let object = ram::g_player_info(revision).at(6);
    record.kind().write(&mut host.context(), 1);
    record.object().write(&mut host.context(), 0);
    record.second_object().write(&mut host.context(), 0);
    for health in i8::MIN..=i8::MAX {
        record
            .health_visibility()
            .at(0)
            .write(&mut host.context(), health);
        fixture(host, ids::DAMAGE_EARTHLING, &[record.address()]);
    }
    record.object().write(&mut host.context(), 6);
    record.level().write(&mut host.context(), 1);
    object.level().write(&mut host.context(), 1);
    object.x().write(&mut host.context(), 512);
    object.y().write(&mut host.context(), 512);
    object.frame_duration().write(&mut host.context(), 1);
    for (kind, id) in [
        (12, ids::EARTHLING_THINK_HORIZONTAL_WANDER),
        (8, ids::EARTHLING_THINK_TARGETED_RESET_CYCLE),
        (6, ids::EARTHLING_THINK_TIMED_MOVEMENT),
    ] {
        record.kind().write(&mut host.context(), kind);
        record.state().write(&mut host.context(), 4);
        object.sprite_state().write(&mut host.context(), 4);
        for timer in [i16::MIN, i16::MIN + 1, -1, 0, 1, i16::MAX] {
            record.timer().write(&mut host.context(), timer);
            record.state_delay().write(&mut host.context(), 0);
            fixture(host, id, &[record.address()]);
        }
    }
    record.kind().write(&mut host.context(), 7);
    record.state().write(&mut host.context(), 3);
    object.level().write(&mut host.context(), 2);
    for map_number in [0, 1] {
        host.context().set_data_register(1, map_number);
        fixture(
            host,
            ids::EARTHLING_THINK_TIMED_MOVEMENT,
            &[record.address()],
        );
    }
    object.level().write(&mut host.context(), 1);
    record.kind().write(&mut host.context(), 18);
    record.state().write(&mut host.context(), 3);
    for timer in [i16::MIN, i16::MIN + 1, -1, 0, 1, i16::MAX] {
        record.timer().write(&mut host.context(), timer);
        fixture(
            host,
            ids::EARTHLING_PROCESS_SPECIAL_THINK_STATES,
            &[record.address(), object.address()],
        );
    }
    record.kind().write(&mut host.context(), 8);
    record.state().write(&mut host.context(), 4);
    for phase in i8::MIN..=i8::MAX {
        record.timer().write(&mut host.context(), 0);
        record
            .health_visibility()
            .at(0)
            .write(&mut host.context(), phase);
        fixture(
            host,
            ids::EARTHLING_THINK_TARGETED_RESET_CYCLE,
            &[record.address()],
        );
    }
}

pub(super) fn present_fixtures(host: &mut Host) {
    let revision = host.revision();
    // Source-reviewed exhausted scans deliberately reach bank zero's one-past
    // record, which is bank one's first record; no bounds guard is added.
    let player = ram::g_player_info(revision).at(0);
    player.level().write(&mut host.context(), 0);
    player.entity_index().write(&mut host.context(), 0);
    player.x().write(&mut host.context(), 512);
    player.y().write(&mut host.context(), 512);
    player.draw_flags().write(&mut host.context(), 3);
    ram::g_earthling_bank_users(revision)
        .at(0)
        .write(&mut host.context(), 1);
    ram::g_earthling_bank_levels(revision)
        .at(0)
        .write(&mut host.context(), 0);
    for kind in [-1, 1, 0] {
        for slot in 0..=20 {
            let entry = ram::g_earthling_banks(revision).at(0).at(0).offset(slot);
            entry.kind().write(&mut host.context(), kind);
            entry.object().write(&mut host.context(), 0);
            entry.second_object().write(&mut host.context(), 0);
        }
        fixture(host, ids::OPEN_EARTHLING_PRESENT, &[player.address(), 0x10]);
    }
    player.level().write(&mut host.context(), 1);
    ram::g_earthling_bank_levels(revision)
        .at(0)
        .write(&mut host.context(), 1);
    for seed in [1, 7, 12345, 127773, i32::MAX] {
        for slot in 0..20 {
            let entry = ram::g_earthling_banks(revision).at(0).at(slot);
            entry.kind().write(&mut host.context(), -1);
            entry.object().write(&mut host.context(), 0);
        }
        ram::g_rng_state(revision).write(&mut host.context(), seed);
        fixture(host, ids::OPEN_EARTHLING_PRESENT, &[player.address(), 0x10]);
    }
}

fn index_and_present_fixtures(host: &mut Host) {
    let revision = host.revision();
    let record = ram::g_earthling_banks(revision).at(0).at(0);
    let object = ram::g_player_info(revision).at(6);
    record.kind().write(&mut host.context(), 1);
    record.object().write(&mut host.context(), 6);
    record.second_object().write(&mut host.context(), 0);
    for state in [-4096_i16, -512, -1, 0, 1, 512, 4096] {
        object.frame().write(&mut host.context(), -1);
        fixture(
            host,
            ids::SET_EARTHLING_STATE,
            &[record.address(), state as u32],
        );
    }
    for bank in [-6000_i16, -1, 0, 1, 6000] {
        fixture(
            host,
            ids::EARTHLING_CONFIGURE_MAILBOX_LEVEL_BANK,
            &[bank as u32, 2],
        );
    }
    // ±256 wrap into dead stack; the separate rejection test covers them.
    for number in [-512_i16, -1, 0, 1, 512] {
        let aliased = ram::g_player_info(revision)
            .at(0)
            .byte_offset(i32::from(number.wrapping_mul(128)));
        aliased.level().write(&mut host.context(), 17);
        fixture(
            host,
            ids::PLAYER_CREATE_INTERACTION_CHOICE_DISPLAY,
            &[number as u32],
        );
        record.state().write(&mut host.context(), 0);
        record.kind().write(&mut host.context(), 20);
        fixture(
            host,
            ids::DROP_PRESENTS_ABILITY,
            &[number as u32, record.address()],
        );
    }
    // An explicit flat RAM table exercises negative short parity without
    // accidentally depending on live objects outside the twenty-record bank.
    let table = tje_mem::Address::<tje_mem::EarthlingState>::new(0xfff000);
    for index in -2..20 {
        let entry = table.offset(index);
        entry.kind().write(&mut host.context(), -1);
        entry.object().write(&mut host.context(), 0);
        entry.second_object().write(&mut host.context(), 0);
    }
    for parity in [-2, 0, 1, 4096] {
        ram::g_earthling_update_parity(revision).write(&mut host.context(), parity);
        fixture(host, ids::LOAD_IN_EARTHLINGS, &[table.address()]);
    }
    if earthlings::TRANSLATIONS
        .iter()
        .any(|entry| entry.id == ids::OPEN_EARTHLING_PRESENT)
    {
        present_fixtures(host);
    }
    record.kind().write(&mut host.context(), 1);
    record.object().write(&mut host.context(), 0);
    record.state().write(&mut host.context(), 0);
    for index in [255, 256, 32767, 32768, 65534, 65535] {
        ram::g_random_cache_index(revision).write(&mut host.context(), index);
        fixture(host, ids::EARTHLING_SELECT_TURN_STATE, &[record.address()]);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn short_index_widths_on_live_states() {
    let roots = [
        ids::AI_OPERA_SINGER_TOUCH,
        ids::AI_WIZARD_TOUCH,
        ids::PLAYER_PROCESS_LIFE_REQUEST,
        ids::AI_MAILBOX_TOUCH,
        ids::AI_WISEMAN_TOUCH,
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = if revision == Revision::Rev02 {
            "TJE_REV02_ROM"
        } else {
            "TJE_REV00_ROM"
        };
        let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
        let registry =
            HookRegistry::proof_filtered(revision, proof_translations(), &roots).unwrap();
        // Reference replay has no entry replacements; check_fixture explicitly
        // checks candidates only after the live-state boundary is reached.
        let mut host = Host::new(&bytes, Mode::Reference, registry).unwrap();
        let script = std::sync::Arc::new(
            InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap(),
        );
        host.set_wait_input_poller(script.clone());
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        host.settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        let record = ram::g_earthling_banks(revision).at(0).at(0);
        let object = ram::g_player_info(revision).at(6);
        record.object().write(&mut host.context(), 6);
        record.level().write(&mut host.context(), 1);
        object.x().write(&mut host.context(), 512);
        object.y().write(&mut host.context(), 512);
        object.level().write(&mut host.context(), 1);
        for number in [-512_i16, -1, 0, 1, 512] {
            let player = ram::g_player_info(revision)
                .at(0)
                .byte_offset(i32::from(number.wrapping_mul(128)));
            player.x().write(&mut host.context(), 512);
            player.y().write(&mut host.context(), 512);
            player.level().write(&mut host.context(), 1);
            player.sprite_state().write(&mut host.context(), 0);
            player.animation_group_mask().write(&mut host.context(), 0);
            ram::g_player_interaction_choice_active(revision)
                .at(i32::from(number))
                .write(&mut host.context(), 0);
            for (kind, id, interaction) in [
                (
                    10,
                    ids::AI_WIZARD_TOUCH,
                    ram::g_player_wizard_interaction_states(revision).at(i32::from(number)),
                ),
                (
                    17,
                    ids::AI_OPERA_SINGER_TOUCH,
                    ram::g_player_opera_singer_interaction_states(revision).at(i32::from(number)),
                ),
                (
                    11,
                    ids::AI_WISEMAN_TOUCH,
                    ram::g_player_wiseman_interaction_states(revision).at(i32::from(number)),
                ),
            ] {
                record.kind().write(&mut host.context(), kind);
                interaction.write(&mut host.context(), 2);
                fixture(&mut host, id, &[number as u32, record.address()]);
            }
            record.kind().write(&mut host.context(), 9);
            let mailbox = ram::g_mailbox_level_states(revision).at(0);
            mailbox.level().write(&mut host.context(), 1);
            mailbox.mode().write(&mut host.context(), 0);
            mailbox
                .contact()
                .at(i32::from(number))
                .write(&mut host.context(), 2);
            fixture(
                &mut host,
                ids::AI_MAILBOX_TOUCH,
                &[number as u32, record.address()],
            );
            ram::g_life_request_pending(revision).write(&mut host.context(), 0);
            ram::g_life_request_display_entity(revision).write(&mut host.context(), 1);
            fixture(
                &mut host,
                ids::PLAYER_PROCESS_LIFE_REQUEST,
                &[number as u32],
            );
        }
        assert_eq!(host.checks.mismatches, 0);
        println!(
            "WIDTH_PENDING {revision:?} {}",
            serde_json::to_string(&host.checks).unwrap()
        );
    }
}

#[test]
#[ignore = "requires private ROMs; malformed stack aliases must fail closed"]
fn word_index_stack_alias_is_a_dependency_audit_rejection() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = if revision == Revision::Rev02 {
            "TJE_REV02_ROM"
        } else {
            "TJE_REV00_ROM"
        };
        let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
        let registry = HookRegistry::filtered(revision, &["earthlings"], &[]).unwrap();
        let mut host = Host::new(&bytes, Mode::Reference, registry).unwrap();
        let script =
            InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap();
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        host.settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        for number in [-256_i16, 256] {
            let player = ram::g_player_info(revision)
                .at(0)
                .byte_offset(i32::from(number.wrapping_mul(128)));
            player.level().write(&mut host.context(), 17);
            let error = host
                .check_fixture(
                    ids::PLAYER_CREATE_INTERACTION_CHOICE_DISPLAY
                        .metadata()
                        .address(revision),
                    &[number as u32],
                )
                .expect_err("stack-alias read must not be approved");
            assert!(error.to_string().contains("dependency"), "{error}");
        }
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn open_present_branches_on_live_states() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = if revision == Revision::Rev02 {
            "TJE_REV02_ROM"
        } else {
            "TJE_REV00_ROM"
        };
        let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
        let registry = HookRegistry::proof_filtered(
            revision,
            proof_translations(),
            &[ids::OPEN_EARTHLING_PRESENT],
        )
        .unwrap();
        let mut host = Host::new(&bytes, Mode::Reference, registry).unwrap();
        let script =
            InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap();
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        host.settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        present_fixtures(&mut host);
        assert_eq!(host.checks.mismatches, 0);
        println!(
            "OPEN_PRESENT {revision:?} {}",
            serde_json::to_string(&host.checks).unwrap()
        );
    }
}

#[test]
#[ignore = "requires both private retail ROMs; frame candidates are opt-in only"]
fn explicit_earthling_frames_on_live_states() {
    let roots = [
        ids::FIND_SPAWN_LOCATION_FOR_EARTHLING,
        ids::EARTHLING_RETAIN_LEVEL_BANK,
        ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = if revision == Revision::Rev02 {
            "TJE_REV02_ROM"
        } else {
            "TJE_REV00_ROM"
        };
        let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
        let registry =
            HookRegistry::proof_filtered(revision, proof_translations(), &roots).unwrap();
        let mut host = Host::new(&bytes, Mode::Reference, registry).unwrap();
        let script =
            InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap();
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        host.settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        for bank in 0..2 {
            for slot in 0..20 {
                let entry = ram::g_earthling_banks(revision).at(bank).at(slot);
                entry.kind().write(&mut host.context(), -1);
                entry.object().write(&mut host.context(), 0);
                entry.second_object().write(&mut host.context(), 0);
            }
        }
        for kind in [0, 1, 8, 13, 14, 18, 19] {
            fixture(
                &mut host,
                ids::FIND_SPAWN_LOCATION_FOR_EARTHLING,
                &[kind, 0, 0xfff000, 0xfff002, 1],
            );
        }
        for level in [0, 1, 2, 26] {
            ram::g_earthling_bank_users(revision)
                .at(0)
                .write(&mut host.context(), 0);
            ram::g_earthling_bank_users(revision)
                .at(1)
                .write(&mut host.context(), 0);
            for piece in 0..10 {
                ram::g_ship_piece_level_assignments(revision)
                    .at(piece)
                    .write(&mut host.context(), -1);
            }
            if level < 26 {
                ram::g_earthling_spawn_failure_masks(revision)
                    .at(level)
                    .write(&mut host.context(), 0);
            }
            fixture(&mut host, ids::EARTHLING_RETAIN_LEVEL_BANK, &[level as u32]);
        }
        let state = ram::g_player_info(revision).at(6);
        state.x().write(&mut host.context(), 500);
        state.y().write(&mut host.context(), 500);
        state.level().write(&mut host.context(), 1);
        state.sprite_state().write(&mut host.context(), 0);
        state.attachment_x_or_latch().write(&mut host.context(), 0);
        state.attachment_y_or_latch().write(&mut host.context(), 0);
        for player in 0..2 {
            let target = ram::g_player_info(revision).at(player);
            target.level().write(&mut host.context(), 1);
            target.x().write(&mut host.context(), 512);
            target.y().write(&mut host.context(), 512);
            target.sprite_state().write(&mut host.context(), 0);
            target.animation_group_mask().write(&mut host.context(), 0);
            ram::g_player_elevator_flags(revision)
                .at(player)
                .write(&mut host.context(), 0);
            ram::g_player_decoy_entities(revision)
                .at(player)
                .write(&mut host.context(), 0);
            ram::g_player_control_remap_entities(revision)
                .at(player)
                .write(&mut host.context(), 0);
            ram::g_player_inventory(revision)
                .at(player)
                .at(0)
                .write(&mut host.context(), 0);
        }
        for mode in [1, 7, 12, 14, 18] {
            fixture(
                &mut host,
                ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
                &[state.address(), 20, 0, mode],
            );
        }
        println!(
            "FRAMED_NORMAL {revision:?} {}",
            serde_json::to_string(&host.checks).unwrap()
        );
        state.level().write(&mut host.context(), 3);
        let error = host
            .check_fixture(
                ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET
                    .metadata()
                    .address(revision),
                &[state.address(), 255, 0, 1],
            )
            .expect_err("dependent residual path must stay unapproved");
        assert!(error.to_string().contains("dependency"), "{error}");
        println!(
            "DEPENDENT_RESIDUAL {revision:?}: {error}; {:?}",
            host.checks.uninitialized_stack_reads
        );
    }
}

#[test]
fn native_startup_reports_the_untranslated_dependency_closure() {
    let roots: Vec<_> = earthlings::TRANSLATIONS
        .iter()
        .map(|entry| entry.id)
        .collect();
    match tje_game::Dispatcher::native(&roots) {
        Ok(_) => panic!("dependent nearest-target entry must fail native startup"),
        Err(missing) => {
            let required = ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET;
            assert!(missing.contains(&required));
            let names: Vec<_> = missing.iter().map(|id| id.metadata().name).collect();
            println!("NATIVE_MISSING {}", serde_json::to_string(&names).unwrap());
        }
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn earthlings_sessions_and_labeled_live_fixtures() {
    let script = std::sync::Arc::new(
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap(),
    );
    let frames =
        std::env::var("TJE_EARTHLINGS_FRAMES").map_or(10_000, |value| value.parse().unwrap());
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = if revision == Revision::Rev02 {
            "TJE_REV02_ROM"
        } else {
            "TJE_REV00_ROM"
        };
        let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
        let pending = std::env::var("TJE_EARTHLINGS_PENDING").unwrap_or_default();
        let registry = if pending == "all" {
            HookRegistry::proof(revision, proof_translations()).unwrap()
        } else {
            HookRegistry::filtered(revision, &["earthlings"], &[]).unwrap()
        };
        let mut checked = Host::new(&bytes, Mode::Check, registry.clone()).unwrap();
        let mut reference = Host::new(&bytes, Mode::Reference, registry).unwrap();
        checked.set_wait_input_poller(script.clone());
        reference.set_wait_input_poller(script.clone());
        for frame in 0..frames {
            let a = checked
                .run_frame(script.at(frame))
                .unwrap_or_else(|error| panic!("{revision:?} frame {frame}: {error}"));
            let b = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(a.ram, b.ram, "{revision:?} RAM frame {frame}");
            assert_eq!(a.vdp, b.vdp, "{revision:?} VDP frame {frame}");
            assert_eq!(a.writes, b.writes, "{revision:?} writes frame {frame}");
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits()))
            );
        }
        let natural = serde_json::to_value(&checked.checks).unwrap();
        checked
            .settle_fixture_boundary(script.at(checked.frame_count()))
            .unwrap();
        live_fixtures(&mut checked);
        state_fixtures(&mut checked);
        behavior_fixtures(&mut checked);
        special_fixtures(&mut checked);
        allocation_and_timed_fixtures(&mut checked);
        visibility_fixtures(&mut checked);
        decrement_boundary_fixtures(&mut checked);
        index_and_present_fixtures(&mut checked);
        if pending == "all" {
            time_aware_fixtures(&mut checked);
        }
        let total = serde_json::to_value(&checked.checks).unwrap();
        if let Ok(directory) = std::env::var("TJE_EARTHLINGS_METRICS_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            let report = serde_json::json!({"revision": format!("{revision:?}"), "frames": frames, "natural": natural, "total": total, "measurements": checked.measurements});
            std::fs::write(
                format!("{directory}/{revision:?}.json"),
                serde_json::to_string_pretty(&report).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(checked.checks.mismatches, 0);
    }
}
