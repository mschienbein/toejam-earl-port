//! Opt-in W0 actor differential acceptance binary, outside the game dependency graph.
//!
//! Build this file as an external harness binary with tje-game, tje-host, tje-mem,
//! and serde_json dependencies. Run from the port root with an evidence-directory
//! argument. It uses the standard start-game input, a timed reference, then labelled
//! synthetic fixtures on the reached live gameplay state. No ROM data is embedded.

use std::{fs, path::PathBuf};
use tje_game::{Dispatcher, FunctionId, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram};

mod wave1_fixtures;
mod wave2_fixtures;
mod wave3_fixtures;
mod wave4_fixtures;
mod wave5_fixtures;

const ROOTS: &[FunctionId] = &[
    ids::MAP_GET_SHIP_PIECE_WORLD_POSITION,
    ids::EARTHLING_CLEAR_MOLE_PLAYER_CONTACT_FLAGS,
    ids::EARTHLING_IS_UPDATE_BLOCKED,
    ids::EARTHLING_CAN_TARGET_PLAYER_STATE,
    ids::ENTITY_SET_VELOCITY_TOWARD_DELTA,
    ids::ENTITY_STEER_WITH_VERTICAL_DEAD_ZONE,
    ids::EARTHLING_IS_MAIL_ORDER_MAILBOX,
    ids::NICE_EARTHLING_CHECK,
    ids::MAP_IS_LARGE_PLACEMENT_TERRAIN_EXCLUDED,
    ids::PRESENT_CLEAR_WORLD_ENTITY_READINESS,
    ids::PRESENT_REGISTER_DECOY_HIT,
    ids::PRESENT_FIND_DOORWAY_OWNER_AT_TILE,
    ids::PLAYER_REMAP_DIRECTIONAL_INPUT,
    ids::NO_OP,
    ids::SETUP_PLAYER,
    ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
    ids::INITIALIZE_LEVEL_ZERO_FIXED_ITEMS,
    ids::GET_LEVEL_ZERO_EARTHLING_POSITION,
];

fn short_slot(value: i16) -> u32 {
    // Deliberately unrelated upper bits verify callee-side short narrowing.
    0xa55a_0000 | u32::from(value as u16)
}

fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| {
            panic!(
                "live-state fixture {} {arguments:08x?}: {error}",
                id.metadata().name
            )
        });
}

fn fixtures(host: &mut Host) {
    let revision = host.revision();
    let player = ram::g_player_info(revision).at(0);
    let target = ram::g_player_info(revision).at(1);
    let earthling = ram::g_earthling_banks(revision).at(0).at(0);
    let outputs = ram::g_object_table(revision).at(0).at(0);
    let x = outputs.x().address();
    let y = outputs.y().address();

    // Signed map bytes, pre-read ordering, and aliased output pointers.
    for map_x in [i8::MIN, -1, 0, 19, i8::MAX] {
        for map_y in [i8::MIN, -1, 0, 25, i8::MAX] {
            ram::g_ship_piece_spawn_map_x(revision).write(&mut host.context(), map_x);
            ram::g_ship_piece_spawn_map_y(revision).write(&mut host.context(), map_y);
            for (output_x, output_y) in [
                (x, y),
                (x, x),
                (ram::g_ship_piece_spawn_map_x(revision).address(), y),
            ] {
                check(
                    host,
                    ids::MAP_GET_SHIP_PIECE_WORLD_POSITION,
                    &[output_x, output_y],
                );
            }
        }
    }

    // Bank selection, both player's latches, last slot and missing object handles.
    for selected_bank in 0..2 {
        for entity_index in [0, 1, -1] {
            for users in [0, 1, -1] {
                for bank in 0..2 {
                    for slot in 0..20 {
                        let record = ram::g_earthling_banks(revision).at(bank).at(slot);
                        record.kind().write(&mut host.context(), -1);
                        record.object().write(&mut host.context(), 0);
                    }
                    for (slot, kind, object) in [(0, 18, 2), (1, 18, 0), (2, 9, 3), (19, 18, 4)] {
                        let record = ram::g_earthling_banks(revision).at(bank).at(slot);
                        record.kind().write(&mut host.context(), kind);
                        record.object().write(&mut host.context(), object);
                    }
                }
                player.level().write(&mut host.context(), 7);
                player
                    .entity_index()
                    .write(&mut host.context(), entity_index);
                ram::g_earthling_bank_users(revision)
                    .at(0)
                    .write(&mut host.context(), users);
                ram::g_earthling_bank_levels(revision)
                    .at(0)
                    .write(&mut host.context(), if selected_bank == 0 { 7 } else { 8 });
                for object in 2..5 {
                    let object = ram::g_player_info(revision).at(object);
                    object.attachment_x_or_latch().write(&mut host.context(), 9);
                    object
                        .attachment_y_or_latch()
                        .write(&mut host.context(), -7);
                }
                check(
                    host,
                    ids::EARTHLING_CLEAR_MOLE_PLAYER_CONTACT_FLAGS,
                    &[player.address()],
                );
            }
        }
    }

    check(host, ids::EARTHLING_IS_UPDATE_BLOCKED, &[0]);
    for visible in [0, 2, 4, 6, -1] {
        earthling
            .health_visibility()
            .at(1)
            .write(&mut host.context(), visible);
        for first in [0, 1, -1] {
            for second in [0, 1, -1] {
                ram::g_player_inventory_open(revision)
                    .at(0)
                    .write(&mut host.context(), first);
                ram::g_player2_inventory_open(revision).write(&mut host.context(), second);
                check(
                    host,
                    ids::EARTHLING_IS_UPDATE_BLOCKED,
                    &[earthling.address()],
                );
            }
        }
    }

    for source_state in [0, 2] {
        player
            .sprite_state()
            .write(&mut host.context(), source_state);
        for target_state in [i8::MIN, -1, 0, 2, 3, 4, 7, 76, 77, 78, i8::MAX] {
            target
                .sprite_state()
                .write(&mut host.context(), target_state);
            for mask in std::iter::once(0).chain((0..32).map(|bit| 1_u32 << bit)) {
                target
                    .animation_group_mask()
                    .write(&mut host.context(), mask);
                check(
                    host,
                    ids::EARTHLING_CAN_TARGET_PLAYER_STATE,
                    &[player.address(), target.address()],
                );
            }
        }
    }

    let deltas = [
        i16::MIN,
        -21,
        -20,
        -19,
        -6,
        -5,
        -4,
        -2,
        -1,
        0,
        1,
        2,
        4,
        5,
        6,
        19,
        20,
        21,
        i16::MAX,
    ];
    for delta_x in deltas {
        for delta_y in deltas {
            for function in [
                ids::ENTITY_SET_VELOCITY_TOWARD_DELTA,
                ids::ENTITY_STEER_WITH_VERTICAL_DEAD_ZONE,
            ] {
                check(
                    host,
                    function,
                    &[player.address(), short_slot(delta_x), short_slot(delta_y)],
                );
            }
        }
    }

    for kind in [-1, 9, 10] {
        earthling.kind().write(&mut host.context(), kind);
        earthling.level().write(&mut host.context(), -1);
        for first_level in [-1, 0] {
            for second_level in [-1, 0] {
                for first_mode in [-1, 0, 1, 2] {
                    for second_mode in [-1, 0, 1, 2] {
                        let banks = ram::g_mailbox_level_states(revision);
                        banks.at(0).level().write(&mut host.context(), first_level);
                        banks.at(1).level().write(&mut host.context(), second_level);
                        banks.at(0).mode().write(&mut host.context(), first_mode);
                        banks.at(1).mode().write(&mut host.context(), second_mode);
                        check(
                            host,
                            ids::EARTHLING_IS_MAIL_ORDER_MAILBOX,
                            &[earthling.address()],
                        );
                    }
                }
            }
        }
    }

    for kind in [-1, 10, 11, 17, 24] {
        earthling.kind().write(&mut host.context(), kind);
        for state in [i8::MIN, -1, 0, 1, 2, 3, 4, i8::MAX] {
            earthling.state().write(&mut host.context(), state);
            for first in [-1, 0, 2, 3] {
                for second in [-1, 0, 2, 3] {
                    ram::g_player_wiseman_interaction_states(revision)
                        .at(0)
                        .write(&mut host.context(), first);
                    ram::g_player2_wiseman_interaction_state(revision)
                        .write(&mut host.context(), second);
                    check(host, ids::NICE_EARTHLING_CHECK, &[earthling.address()]);
                }
            }
        }
    }

    for terrain in (0..256).chain([i16::MIN, -1, i16::MAX]) {
        check(
            host,
            ids::MAP_IS_LARGE_PLACEMENT_TERRAIN_EXCLUDED,
            &[short_slot(terrain)],
        );
    }

    for first_active in [0, 1] {
        for second_active in [0, -1] {
            ram::g_item_bank_users(revision)
                .at(0)
                .write(&mut host.context(), first_active);
            ram::g_item_bank_users(revision)
                .at(1)
                .write(&mut host.context(), second_active);
            for bank in 0..2 {
                for slot in 0..32 {
                    let item = ram::g_object_table(revision).at(bank).at(slot);
                    item.kind().write(
                        &mut host.context(),
                        [-128, -1, 0, 63, 64, 127][slot as usize % 6],
                    );
                    item.object()
                        .write(&mut host.context(), [0, 2, 3, -1][slot as usize % 4]);
                }
            }
            for slot in 0..32 {
                let item = ram::g_dropped_present_table(revision).at(slot);
                item.kind()
                    .write(&mut host.context(), if slot & 1 == 0 { -1 } else { 80 });
                item.object()
                    .write(&mut host.context(), if slot & 2 == 0 { 0 } else { 4 });
            }
            for object in -1..5 {
                ram::g_player_info(revision)
                    .at(object)
                    .sprite_ready()
                    .write(&mut host.context(), -1);
            }
            check(host, ids::PRESENT_CLEAR_WORLD_ENTITY_READINESS, &[]);
        }
    }

    for (first, second) in [(0, 0), (2, 3), (2, 2), (-1, 2)] {
        ram::g_player_decoy_entities(revision)
            .at(0)
            .write(&mut host.context(), first);
        ram::g_player_decoy_entities(revision)
            .at(1)
            .write(&mut host.context(), second);
        for hits in [i8::MIN, -1, 0, i8::MAX] {
            ram::g_player_decoy_hit_counts(revision)
                .at(0)
                .write(&mut host.context(), hits);
            ram::g_player_decoy_hit_counts(revision)
                .at(1)
                .write(&mut host.context(), hits);
            for object in [i16::MIN, -1, 0, 1, 2, 3, i16::MAX] {
                check(host, ids::PRESENT_REGISTER_DECOY_HIT, &[short_slot(object)]);
            }
        }
    }

    // Boundary tile areas, signed shifts, both owners, level and facing gates.
    ram::g_player_doorway_entities(revision)
        .at(0)
        .write(&mut host.context(), 2);
    ram::g_player_doorway_entities(revision)
        .at(1)
        .write(&mut host.context(), 3);
    for owner in 0..2 {
        for (world_x, world_y) in [
            (i16::MIN, i16::MIN),
            (-17, -11),
            (0, 0),
            (16, 10),
            (i16::MAX, i16::MAX),
        ] {
            for state in [0, 1, -1] {
                for same_level in [false, true] {
                    for above in [false, true] {
                        for door in 2..4 {
                            ram::g_player_info(revision)
                                .at(door)
                                .level()
                                .write(&mut host.context(), 8);
                        }
                        let doorway = ram::g_player_info(revision).at(owner + 2);
                        doorway.x().write(&mut host.context(), world_x);
                        doorway.y().write(&mut host.context(), world_y);
                        doorway.level().write(&mut host.context(), 7);
                        doorway.sprite_state().write(&mut host.context(), state);
                        player
                            .level()
                            .write(&mut host.context(), if same_level { 7 } else { 6 });
                        player.y().write(
                            &mut host.context(),
                            if above {
                                world_y.wrapping_sub(1)
                            } else {
                                world_y
                            },
                        );
                        let row = ((i32::from(world_y) - 10) >> 3) as i16;
                        let column = ((i32::from(world_x) - 16) >> 3) as i16;
                        for row_offset in [-1, 0, 2, 3] {
                            for column_offset in [-1, 0, 3, 4] {
                                check(
                                    host,
                                    ids::PRESENT_FIND_DOORWAY_OWNER_AT_TILE,
                                    &[
                                        player.address(),
                                        short_slot(row + row_offset),
                                        short_slot(column + column_offset),
                                    ],
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    ram::g_player_doorway_entities(revision)
        .at(0)
        .write(&mut host.context(), 0);
    ram::g_player_doorway_entities(revision)
        .at(1)
        .write(&mut host.context(), 0);
    check(
        host,
        ids::PRESENT_FIND_DOORWAY_OWNER_AT_TILE,
        &[player.address(), 0, 0],
    );

    // All controller byte combinations, including impossible direction nibbles.
    for player_number in 0..2 {
        for index in [0, 1, 2, 3] {
            ram::g_player_control_remap_indices(revision)
                .at(player_number)
                .write(&mut host.context(), index);
            for input in 0..=u8::MAX {
                ram::g_player_processed_input(revision)
                    .at(player_number)
                    .write(&mut host.context(), input);
                check(
                    host,
                    ids::PLAYER_REMAP_DIRECTIONAL_INPUT,
                    &[short_slot(player_number as i16)],
                );
            }
        }
        for index in [i8::MIN, -1, 4, i8::MAX] {
            ram::g_player_control_remap_indices(revision)
                .at(player_number)
                .write(&mut host.context(), index);
            for input in [0, 1, 10, 11, 15, 0xf1, 0xff] {
                ram::g_player_processed_input(revision)
                    .at(player_number)
                    .write(&mut host.context(), input);
                check(
                    host,
                    ids::PLAYER_REMAP_DIRECTIONAL_INPUT,
                    &[short_slot(player_number as i16)],
                );
            }
        }
    }
    check(host, ids::NO_OP, &[]);
    check(host, ids::SETUP_PLAYER, &[]);

    for player_number in 0..2 {
        for choice in [0, 1, -1] {
            ram::g_player_interaction_choice_active(revision)
                .at(player_number)
                .write(&mut host.context(), choice);
            for state in [i8::MIN, -1, 0, 3, 4, 76, 77, i8::MAX] {
                player.sprite_state().write(&mut host.context(), state);
                for mask in std::iter::once(0).chain((0..32).map(|bit| 1_u32 << bit)) {
                    player
                        .animation_group_mask()
                        .write(&mut host.context(), mask);
                    check(
                        host,
                        ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
                        &[player.address(), short_slot(player_number as i16)],
                    );
                }
            }
        }
    }

    for offset in 0..24 {
        host.context().write_u8(outputs.address() + offset, 0xa5);
    }
    check(
        host,
        ids::INITIALIZE_LEVEL_ZERO_FIXED_ITEMS,
        &[outputs.address()],
    );
    for kind in (-1..26).chain([i16::MIN, i16::MAX]) {
        for output_y in [y, x] {
            check(
                host,
                ids::GET_LEVEL_ZERO_EARTHLING_POSITION,
                &[short_slot(kind), x, output_y],
            );
        }
    }

    // Pre-existing player pilot hooks remain selected by the subsystem filter.
    check(host, ids::STOP_PLAYER, &[player.address()]);
    check(host, ids::PLAYER_INITIALIZE_PHASED_JUMP_MOTION, &[0]);
    check(host, ids::ENTITY_HAS_FREE_TIMED_EFFECT_SLOT, &[]);
}

fn main() {
    let evidence = PathBuf::from(std::env::args_os().nth(1).expect("evidence directory"));
    let frames: u64 = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "10000".into())
        .parse()
        .unwrap();
    fs::create_dir_all(&evidence).unwrap();
    assert!(
        Dispatcher::native(ROOTS).is_ok(),
        "actor native dependency closure"
    );
    let script = InputScript::parse(
        &fs::read_to_string("crates/tje-host/examples/start-game.json").unwrap(),
    )
    .unwrap();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let name = match revision {
            Revision::Rev02 => "rev02",
            Revision::Rev00 => "rev00",
        };
        let rom = fs::read(format!("roms/{name}.bin")).unwrap();
        let registry =
            HookRegistry::filtered(revision, &["earthlings", "presents", "player"], &[]).unwrap();
        let mut checked = Host::new(&rom, Mode::Check, registry.clone()).unwrap();
        let mut reference = Host::new(&rom, Mode::Reference, registry).unwrap();
        for frame in 0..frames {
            let a = checked.run_frame(script.at(frame)).unwrap();
            let b = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(a.frame_counter, b.frame_counter, "{name} frame {frame}");
            assert_eq!(a.ram, b.ram, "{name} whole RAM frame {frame}");
            assert_eq!(a.vdp, b.vdp, "{name} VDP frame {frame}");
            assert_eq!(a.writes, b.writes, "{name} device writes frame {frame}");
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits())),
                "{name} audio frame {frame}"
            );
            let (pa, sa) = checked.frame_buffer();
            let (pb, sb) = reference.frame_buffer();
            assert_eq!(sa, sb);
            assert_eq!(
                &pa[..sa.len() as usize],
                &pb[..sb.len() as usize],
                "{name} pixels frame {frame}"
            );
        }
        assert!(checked.measurements.title_frame.is_some());
        assert!(checked.measurements.menu_frame.is_some());
        assert!(checked.measurements.game_frame.is_some());
        let natural = serde_json::to_value(&checked.checks).unwrap();
        let fixture_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fixtures(&mut checked);
            wave1_fixtures::run(&mut checked);
            wave2_fixtures::run(&mut checked);
            wave3_fixtures::run(&mut checked);
            wave4_fixtures::run(&mut checked);
            wave5_fixtures::run(&mut checked);
        }));
        let metrics = serde_json::json!({
            "revision": name,
            "frames": frames,
            "input": "crates/tje-host/examples/start-game.json",
            "filter": ["earthlings", "presents", "player"],
            "title_frame": checked.measurements.title_frame,
            "menu_frame": checked.measurements.menu_frame,
            "game_frame": checked.measurements.game_frame,
            "logic_ticks": checked.measurements.logic_ticks,
            "whole_session_matches_reference": true,
            "native_actor_closure": true,
            "natural": natural,
            "with_live_fixtures": checked.checks,
            "fixtures_completed": fixture_result.is_ok(),
        });
        fs::write(
            evidence.join(format!("{name}-proof.json")),
            serde_json::to_string_pretty(&metrics).unwrap(),
        )
        .unwrap();
        if let Err(error) = fixture_result {
            std::panic::resume_unwind(error);
        }
        assert_eq!(checked.checks.mismatches, 0);
        assert!(checked.checks.uninitialized_stack_reads.is_empty());
        for translation in tje_game::player::TRANSLATIONS {
            let name = translation.id.metadata().name;
            let coverage = checked
                .checks
                .by_function
                .get(name)
                .expect("registered player hook");
            assert!(
                coverage.calls_checked > 0,
                "{name}: unexercised player hook in {revision:?}"
            );
            assert_eq!(
                coverage.mismatches, 0,
                "{name}: player mismatch in {revision:?}"
            );
        }
        println!(
            "ACTOR_PROOF {name} frames={frames} calls={} mismatches={} nested={} dead_stack_calls={} dead_stack_bytes={}",
            checked.checks.calls,
            checked.checks.mismatches,
            checked.checks.nested_original_calls,
            checked.checks.dead_stack_differing_calls,
            checked.checks.dead_stack_differing_bytes
        );
    }
}
