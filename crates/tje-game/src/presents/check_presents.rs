//! Opt-in presents ROM proof driver. The private manifest and exact invocations
//! are recorded in scratch/Presents/report.md. Not linked into the game library.
use std::{env, fs, path::Path};
use tje_game::{Context, FunctionId, ids};
use tje_host::{Host, Mode, abi::HookContext, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram};

fn fixture(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| panic!("{} {arguments:08x?}: {error}", id.metadata().name));
}

fn candidate_hook(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let id = tje_game::function_at(context.revision(), context.cpu().pc()).unwrap();
    let translation = tje_game::presents::PENDING
        .iter()
        .find(|candidate| candidate.id == id)
        .unwrap();
    let mut arguments = [0; 16];
    let count = usize::from(id.metadata().abi.argument_count);
    for (index, argument) in arguments[..count].iter_mut().enumerate() {
        *argument = context.argument_u32(index as u32);
    }
    Ok((translation.function)(context, &arguments[..count])?)
}

fn registry(revision: Revision) -> HookRegistry {
    let mut registry = HookRegistry::filtered(revision, &["presents"], &[]).unwrap();
    for candidate in tje_game::presents::PENDING {
        if tje_game::is_atomic_capable(candidate.id)
            && candidate.id != ids::ENTITY_UPDATE_LEVEL_ITEM_VISIBILITY
        {
            registry
                .register_contract(candidate.id.metadata().clone(), true, candidate_hook)
                .unwrap();
        }
    }
    registry
}

fn fixtures(host: &mut Host) {
    let revision = host.revision();
    for level in [0_i16, 1, 2, 26, -1] {
        for anchor in [-128_i8, -1, 0, 19, 127] {
            for index in 0..2 {
                let record = ram::g_level_anchor_positions(revision).at(index);
                record.x().write(&mut host.context(), anchor);
                record.y().write(&mut host.context(), anchor);
            }
            let x = ((i32::from(anchor) + 2) * 128 - 16) as i16;
            let y = ((i32::from(anchor) + 2) * 64) as i16;
            for dx in [-31_i16, -30, -29, 0, 29, 30, 31, i16::MIN] {
                for dy in [-21_i16, -20, -19, 0, 19, 20, 21, i16::MAX] {
                    fixture(
                        host,
                        ids::ELEVATOR_ALLOWS_WORLD_PLACEMENT,
                        &[
                            x.wrapping_add(dx) as u32,
                            y.wrapping_add(dy) as u32,
                            level as u32,
                        ],
                    );
                }
            }
        }
    }
    let projectile = ram::g_player_info(revision).at(5);
    for pass in 0..3 {
        for target_index in 0..2 {
            for width in [0_u8, 1, 9, 16, 31, 32, 255] {
                for height in [-32768_i16, -1, 0, 10, 32767] {
                    for dx in [-32768_i16, -12, -1, 0, 1, 12, 32767] {
                        let mut context = host.context();
                        for index in 0..2 {
                            let player = ram::g_player_info(revision).at(index);
                            player.draw_flags().write(
                                &mut context,
                                if pass == 0 && index == target_index {
                                    1
                                } else {
                                    0
                                },
                            );
                            player.behavior_flags().write(&mut context, 1);
                            player.level().write(&mut context, 1);
                            ram::g_player_decoy_entities(revision).at(index).write(
                                &mut context,
                                if pass == 1 && index == target_index {
                                    6 + index as i8
                                } else {
                                    0
                                },
                            );
                            let decoy = ram::g_player_info(revision).at(6 + index);
                            decoy.draw_flags().write(&mut context, 1);
                            decoy.behavior_flags().write(&mut context, 1);
                            decoy.level().write(&mut context, 1);
                        }
                        let target = ram::g_player_info(revision).at(if pass == 0 {
                            target_index
                        } else {
                            6 + target_index
                        });
                        target.x().write(&mut context, 100);
                        target.y().write(&mut context, 100);
                        target.height().write(&mut context, height);
                        target.width().write(&mut context, width);
                        target.height_pixels().write(&mut context, 32);
                        projectile.x().write(&mut context, 100_i16.wrapping_add(dx));
                        projectile.y().write(&mut context, 100);
                        projectile.height().write(&mut context, 10);
                        projectile.width().write(&mut context, 16);
                        projectile.level().write(&mut context, 1);
                        fixture(
                            host,
                            ids::ENTITY_FIND_PROJECTILE_TARGET_HIT,
                            &[projectile.address()],
                        );
                    }
                }
            }
        }
    }
}

fn wave2_fixtures(host: &mut Host) {
    let revision = host.revision();
    for _ in 0..64 {
        fixture(host, ids::SETUP_PRESENT_IDENTITIES, &[]);
        fixture(host, ids::SELECT_RANDOM_PRESENT, &[]);
    }
    for player_number in 0..2 {
        let player = ram::g_player_info(revision).at(player_number);
        let telephone = ram::g_telephone_states(revision).at(player_number);
        let earthling = ram::g_earthling_banks(revision).at(0).at(0);
        fixture(host, ids::SET_TOMATO_RAIN_TOMATOES, &[player_number as u32]);
        for allocation_full in [false, true] {
            for index in 0..34 {
                ram::g_dynamic_entity_slot_allocated(revision)
                    .at(index)
                    .write(&mut host.context(), i8::from(allocation_full));
            }
            for existing in [0_i8, 10] {
                ram::g_player_transient_effect_entities(revision)
                    .at(player_number)
                    .write(&mut host.context(), existing);
                fixture(
                    host,
                    ids::PRESENT_START_TRANSIENT_EFFECT,
                    &[player_number as u32, 0xffff8000, 0x7fff, 0xffff, 0x81],
                );
                fixture(
                    host,
                    ids::PRESENT_CREATE_TEMPORARY_TERRAIN_EFFECT,
                    &[player.address(), player_number as u32],
                );
            }
            for ticks in [0_i8, 1, 80, -128] {
                for delay in [-128_i8, -1, 0, 1, 2, 127] {
                    ram::g_player_tomato_rain_ticks(revision)
                        .at(player_number)
                        .write(&mut host.context(), ticks);
                    ram::g_player_tomato_rain_spawn_delay(revision)
                        .at(player_number)
                        .write(&mut host.context(), delay);
                    for slot in 4..12 {
                        ram::g_shared_projectile_entities(revision)
                            .at(slot)
                            .write(&mut host.context(), 0);
                    }
                    fixture(host, ids::CHECK_TOMATO_RAIN_TIMER, &[player_number as u32]);
                }
            }
            for state in [8_i8, 9, 10, 11] {
                earthling.state().write(&mut host.context(), state);
                earthling.object().write(&mut host.context(), 3);
                earthling.level().write(&mut host.context(), 1);
                earthling.x().write(&mut host.context(), 200);
                earthling.y().write(&mut host.context(), 200);
                for (x, y) in [
                    (200_i16, 200_i16),
                    (100, 300),
                    (300, 100),
                    (i16::MIN, i16::MAX),
                ] {
                    player.x().write(&mut host.context(), x);
                    player.y().write(&mut host.context(), y);
                    player.velocity_x().write(&mut host.context(), -800);
                    player.velocity_y().write(&mut host.context(), 1600);
                    for pool_full in [false, true] {
                        for slot in 4..12 {
                            ram::g_shared_projectile_entities(revision)
                                .at(slot)
                                .write(&mut host.context(), if pool_full { 10 } else { 0 });
                        }
                        fixture(
                            host,
                            ids::EARTHLING_LAUNCH_AIMED_PROJECTILE,
                            &[player.address(), earthling.address()],
                        );
                        for slot in 0..4 {
                            ram::g_control_remap_projectile_entities(revision)
                                .at(slot)
                                .write(&mut host.context(), if pool_full { 10 } else { 0 });
                        }
                        fixture(
                            host,
                            ids::EARTHLING_LAUNCH_CONTROL_REMAP_PROJECTILE,
                            &[earthling.address(), player.address()],
                        );
                    }
                }
            }
        }
        for object in [0_i8, 10] {
            for departure in [0_i8, 1] {
                telephone.object().write(&mut host.context(), object);
                ram::g_elevator_departure_in_progress(revision)
                    .write(&mut host.context(), departure);
                fixture(
                    host,
                    ids::TELEPHONE_START_RING,
                    &[player_number as u32, telephone.address()],
                );
            }
        }
    }
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let root = Path::new(&args[1]);
    let evidence = Path::new(&args[2]);
    fs::create_dir_all(evidence).unwrap();
    let roots: Vec<_> = tje_game::presents::TRANSLATIONS
        .iter()
        .map(|translation| translation.id)
        .collect();
    let missing = tje_game::Dispatcher::native(&roots)
        .err()
        .unwrap_or_default();
    let names: Vec<_> = missing.iter().map(|id| id.metadata().name).collect();
    fs::write(
        evidence.join("native-closure.json"),
        serde_json::to_vec_pretty(&names).unwrap(),
    )
    .unwrap();
    let script = InputScript::parse(
        &fs::read_to_string(root.join("crates/tje-host/examples/start-game.json")).unwrap(),
    )
    .unwrap();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let name = if revision == Revision::Rev02 {
            "rev02"
        } else {
            "rev00"
        };
        let bytes = fs::read(root.join(format!("roms/{name}.bin"))).unwrap();
        let registry = registry(revision);
        let mut host = Host::new(&bytes, Mode::Check, registry).unwrap();
        let mut reference =
            Host::new(&bytes, Mode::Reference, HookRegistry::empty(revision)).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for frame in 0..10000 {
                let actual = host.run_frame(script.at(frame)).unwrap();
                let expected = reference.run_frame(script.at(frame)).unwrap();
                assert_eq!(actual.pc, expected.pc, "{name} frame {frame} PC");
                assert_eq!(
                    actual.frame_counter, expected.frame_counter,
                    "{name} frame {frame} counter"
                );
                assert_eq!(actual.ram, expected.ram, "{name} frame {frame} RAM");
                assert_eq!(actual.vdp, expected.vdp, "{name} frame {frame} VDP");
                assert_eq!(
                    actual.writes, expected.writes,
                    "{name} frame {frame} hardware"
                );
                assert_eq!(
                    actual.idle_random_draws, expected.idle_random_draws,
                    "{name} frame {frame} idle RNG"
                );
                assert_eq!(
                    actual.lag_events, expected.lag_events,
                    "{name} frame {frame} lag events"
                );
                assert!(
                    actual
                        .audio_samples
                        .iter()
                        .flatten()
                        .map(|value| value.to_bits())
                        .eq(expected
                            .audio_samples
                            .iter()
                            .flatten()
                            .map(|value| value.to_bits())),
                    "{name} frame {frame} audio"
                );
                assert_eq!(
                    host.frame_buffer().0,
                    reference.frame_buffer().0,
                    "{name} frame {frame} pixels"
                );
            }
            assert!(host.measurements.game_frame.is_some());
            fs::write(
                evidence.join(format!("{name}-natural.json")),
                serde_json::to_vec_pretty(&host.checks).unwrap(),
            )
            .unwrap();
            fixtures(&mut host);
            wave2_fixtures(&mut host);
            wave3_fixtures(&mut host);
            later_atomic_fixtures(&mut host);
            wave67_atomic_fixtures(&mut host);
        }));
        fs::write(
            evidence.join(format!("{name}-with-fixtures.json")),
            serde_json::to_vec_pretty(&host.checks).unwrap(),
        )
        .unwrap();
        if let Err(error) = result {
            std::panic::resume_unwind(error);
        }
        println!(
            "{name}: 10000 reference-equal frames; title {:?}, game {:?}; {} checked calls, {} mismatches",
            host.measurements.title_frame,
            host.measurements.game_frame,
            host.checks.calls,
            host.checks.mismatches
        );
    }
}

fn prepare_effect_fixture(host: &mut Host) {
    let revision = host.revision();
    let mut context = host.context();
    for number in 0..40 {
        let object = ram::g_player_info(revision).at(number);
        object.draw_flags().write(&mut context, 1);
        object.behavior_flags().write(&mut context, 0);
        object.entity_index().write(&mut context, number as i8);
        object.sprite_state().write(&mut context, 0);
        object.sprite_count().write(&mut context, 0);
        object.tile_count().write(&mut context, 0);
        object.frame_duration().write(&mut context, 1);
        object.level().write(&mut context, 1);
    }
    for index in 0..2 {
        ram::g_player_lives(revision)
            .at(index)
            .write(&mut context, 3);
        ram::g_player_elevator_flags(revision)
            .at(index)
            .write(&mut context, 0);
        ram::g_player_inventory_open(revision)
            .at(index)
            .write(&mut context, 0);
        ram::g_player_transient_effect_entities(revision)
            .at(index)
            .write(&mut context, 0);
    }
}

fn wave3_fixtures(host: &mut Host) {
    let revision = host.revision();
    prepare_effect_fixture(host);
    for users in [0_i8, 1, 2, -128] {
        for level in [-1_i16, 1, 2] {
            for bank in 0..2 {
                ram::g_item_bank_users(revision)
                    .at(bank)
                    .write(&mut host.context(), users);
                ram::g_item_bank_levels(revision)
                    .at(bank)
                    .write(&mut host.context(), 1);
                for index in 0..32 {
                    ram::g_object_table(revision)
                        .at(bank)
                        .at(index)
                        .object()
                        .write(&mut host.context(), if index % 2 == 0 { 6 } else { 0 });
                }
            }
            fixture(host, ids::MAP_RELEASE_LEVEL_ITEM_BANK, &[level as u32]);
        }
    }
    for occupied in [false, true] {
        for index in 0..32 {
            ram::g_dropped_present_table(revision)
                .at(index)
                .object()
                .write(&mut host.context(), if occupied { 6 } else { 0 });
        }
        fixture(host, ids::PRESENT_RELEASE_DROPPED_ENTITIES, &[]);
    }
    for slot in [-1, 4, 8, 11] {
        for index in 4..12 {
            ram::g_shared_projectile_entities(revision)
                .at(index)
                .write(&mut host.context(), if index == slot { 6 } else { 0 });
        }
        fixture(host, ids::PRESENT_RELEASE_FIRST_SHARED_TOMATO, &[]);
    }
    for index in 0..2 {
        let player = ram::g_player_info(revision).at(index);
        for previous in [0_i8, 6] {
            for active in [0_i8, 7] {
                ram::g_player_one_shot_terrain_effect_entities(revision)
                    .at(index)
                    .write(&mut host.context(), previous);
                ram::g_player_active_terrain_effect_entities(revision)
                    .at(index)
                    .write(&mut host.context(), active);
                fixture(host, ids::PRESENT_RETIRE_TERRAIN_EFFECT, &[index as u32]);
            }
        }
        for number in [0_i8, 6] {
            for duration in [0_i8, 1, 2, -1] {
                for frame in [-128_i8, -1, 0, 2, 3, 127] {
                    ram::g_player_transient_effect_entities(revision)
                        .at(index)
                        .write(&mut host.context(), number);
                    ram::g_player_info(revision)
                        .at(6)
                        .frame_duration()
                        .write(&mut host.context(), duration);
                    ram::g_player_info(revision)
                        .at(6)
                        .frame()
                        .write(&mut host.context(), frame);
                    fixture(host, ids::PRESENT_UPDATE_TRANSIENT_EFFECT, &[index as u32]);
                }
            }
        }
        for number in [0_i8, 6] {
            for timer in [i16::MIN, -1, 0, 1, 2, 240] {
                for duration in [0_i8, 2] {
                    ram::g_player_control_remap_entities(revision)
                        .at(index)
                        .write(&mut host.context(), number);
                    ram::g_player_control_remap_timers(revision)
                        .at(index)
                        .write(&mut host.context(), timer);
                    ram::g_player_info(revision)
                        .at(6)
                        .frame_duration()
                        .write(&mut host.context(), duration);
                    fixture(
                        host,
                        ids::PLAYER_UPDATE_CONTROL_REMAP_EFFECT,
                        &[player.address()],
                    );
                }
            }
        }
        for number in [0_i8, 6] {
            for timer in [i16::MIN, -1, 0, 1, 2, 500] {
                for state in [0_i8, 1] {
                    for paused in [0_i8, 1] {
                        ram::g_player_rain_cloud_states(revision)
                            .at(index)
                            .write(&mut host.context(), number);
                        ram::g_player_rain_cloud_timers(revision)
                            .at(index)
                            .write(&mut host.context(), timer);
                        ram::g_player_info(revision)
                            .at(6)
                            .sprite_state()
                            .write(&mut host.context(), state);
                        ram::g_player_info(revision)
                            .at(6)
                            .frame_duration()
                            .write(&mut host.context(), 0);
                        ram::g_player_elevator_flags(revision)
                            .at(index)
                            .write(&mut host.context(), paused);
                        fixture(host, ids::PRESENT_UPDATE_RAIN_CLOUD, &[index as u32]);
                    }
                }
            }
        }
        for mask in 0_u32..8 {
            for terrain in [0x10_u8, 0x80, 0x90] {
                player.sprite_state().write(&mut host.context(), 0);
                player.animation_group_mask().write(&mut host.context(), 0);
                player.x().write(&mut host.context(), 400);
                player.y().write(&mut host.context(), 400);
                let land = if index == 0 {
                    ram::g_player1_nearby_terrain(revision).cast::<u8>()
                } else {
                    ram::g_player2_nearby_terrain(revision).cast::<u8>()
                };
                for offset in 0..25 {
                    land.offset(offset).write(&mut host.context(), terrain);
                }
                for slot in 0..32 {
                    let item = ram::g_dropped_present_table(revision).at(slot);
                    item.kind().write(&mut host.context(), -1);
                    item.object().write(&mut host.context(), 0);
                }
                ram::g_dropped_present_table_idx(revision).write(&mut host.context(), 31);
                ram::g_random_cache_index(revision).write(&mut host.context(), 58);
                ram::g_cached_random_values(revision)
                    .at(0)
                    .write(&mut host.context(), mask as u16);
                fixture(host, ids::DROP_PRESENT, &[index as u32, 27]);
            }
        }
        for own_level in [0_i8, 1] {
            for other_level in [0_i8, 1] {
                ram::g_telephone_states(revision)
                    .at(index)
                    .level()
                    .write(&mut host.context(), own_level);
                ram::g_telephone_states(revision)
                    .at(index ^ 1)
                    .level()
                    .write(&mut host.context(), other_level);
                ram::g_player_transient_effect_entities(revision)
                    .at(index)
                    .write(&mut host.context(), 8);
                fixture(host, ids::OPEN_TELEPHONE, &[index as u32]);
                fixture(host, ids::TELEPHONE_CLEAR_ALL, &[]);
            }
        }
    }
}

fn later_atomic_fixtures(host: &mut Host) {
    let revision = host.revision();
    prepare_effect_fixture(host);
    let earthling = ram::g_earthling_banks(revision).at(0).at(0);
    let source = ram::g_player_info(revision).at(3);
    earthling.kind().write(&mut host.context(), 15);
    earthling.object().write(&mut host.context(), 3);
    for frame in [-128_i8, -2, -1, 0, 1, 2, 6, 14, 16, 18] {
        for duration in [0_i8, 1] {
            for state in [8_i8, 9, 10, 11] {
                source.frame().write(&mut host.context(), frame);
                source.frame_duration().write(&mut host.context(), duration);
                earthling.state().write(&mut host.context(), state);
                fixture(
                    host,
                    ids::EARTHLING_UPDATE_CONTROL_REMAP_ATTACK,
                    &[earthling.address(), source.address()],
                );
                fixture(
                    host,
                    ids::EARTHLING_UPDATE_AIMED_PROJECTILE_ATTACK,
                    &[earthling.address(), source.address()],
                );
            }
        }
    }
    for index in 0..2 {
        let player = ram::g_player_info(revision).at(index);
        player.x().write(&mut host.context(), 400);
        player.y().write(&mut host.context(), 400);
        player.level().write(&mut host.context(), 1);
        ram::g_player_transient_effect_entities(revision)
            .at(index)
            .write(&mut host.context(), 9);
        ram::g_player_info(revision)
            .at(9)
            .frame()
            .write(&mut host.context(), 4);
        ram::g_player_info(revision)
            .at(9)
            .frame_duration()
            .write(&mut host.context(), 1);
        for existing in [0_i8, 6] {
            for full in [false, true] {
                for slot in 0..34 {
                    ram::g_dynamic_entity_slot_allocated(revision)
                        .at(slot)
                        .write(&mut host.context(), i8::from(full));
                }
                ram::g_player_decoy_entities(revision)
                    .at(index)
                    .write(&mut host.context(), existing);
                fixture(host, ids::OPEN_DECOY, &[index as u32]);
                ram::g_player_doorway_entities(revision)
                    .at(index)
                    .write(&mut host.context(), existing);
                fixture(host, ids::PRESENT_IS_DOORWAY, &[index as u32]);
                ram::g_player_rain_cloud_states(revision)
                    .at(index)
                    .write(&mut host.context(), existing);
                fixture(host, ids::PRESENT_IS_RAIN_CLOUD, &[index as u32]);
                ram::g_player_boom_box_entities(revision)
                    .at(index)
                    .write(&mut host.context(), existing);
                fixture(host, ids::OPEN_BOOM_BOX, &[index as u32]);
                ram::g_player_control_remap_entities(revision)
                    .at(index)
                    .write(&mut host.context(), existing);
                fixture(
                    host,
                    ids::PLAYER_APPLY_CONTROL_REMAP_EFFECT,
                    &[index as u32],
                );
                fixture(
                    host,
                    ids::PRESENT_CREATE_ACTIVE_TERRAIN_EFFECT,
                    &[player.address(), index as u32],
                );
                let telephone = ram::g_telephone_states(revision).at(index);
                telephone.x().write(&mut host.context(), 400);
                telephone.y().write(&mut host.context(), 400);
                telephone.level().write(&mut host.context(), 1);
                fixture(
                    host,
                    ids::TELEPHONE_CREATE_DISPLAY_ENTITY,
                    &[telephone.address()],
                );
            }
        }
        for number in [0_i8, 6] {
            for hits in [-128_i8, 0, 2, 3, 127] {
                for level in [1_i8, 2] {
                    ram::g_player_decoy_entities(revision)
                        .at(index)
                        .write(&mut host.context(), number);
                    ram::g_player_decoy_hit_counts(revision)
                        .at(index)
                        .write(&mut host.context(), hits);
                    ram::g_player_info(revision)
                        .at(6)
                        .level()
                        .write(&mut host.context(), level);
                    fixture(host, ids::PRESENT_UPDATE_DECOY, &[index as u32]);
                }
            }
        }
        for number in [0_i8, 6] {
            for timer in [i16::MIN, -1, 0, 1, 2, 300] {
                for event in [0_i8, 1] {
                    ram::g_player_boom_box_entities(revision)
                        .at(index)
                        .write(&mut host.context(), number);
                    ram::g_player_boom_box_entities(revision)
                        .at(index ^ 1)
                        .write(&mut host.context(), 0);
                    ram::g_player_boom_box_timers(revision)
                        .at(index)
                        .write(&mut host.context(), timer);
                    ram::g_music_animation_event(revision).write(&mut host.context(), event);
                    fixture(host, ids::PRESENT_UPDATE_BOOM_BOX, &[index as u32]);
                }
            }
        }
    }
}

fn wave67_atomic_fixtures(host: &mut Host) {
    let revision = host.revision();
    prepare_effect_fixture(host);
    for lives0 in [-1_i8, 3] {
        for lives1 in [-1_i8, 3] {
            ram::g_player_lives(revision)
                .at(0)
                .write(&mut host.context(), lives0);
            ram::g_player_lives(revision)
                .at(1)
                .write(&mut host.context(), lives1);
            for bank in 0..2 {
                ram::g_item_bank_users(revision)
                    .at(bank)
                    .write(&mut host.context(), 0);
            }
            fixture(host, ids::SETUP_INITIAL_INVENTORY, &[]);
        }
    }
    prepare_effect_fixture(host);
    let projectile = ram::g_player_info(revision).at(6);
    let player = ram::g_player_info(revision).at(0);
    player.x().write(&mut host.context(), 400);
    player.y().write(&mut host.context(), 400);
    player.height().write(&mut host.context(), 0);
    player.width().write(&mut host.context(), 32);
    player.height_pixels().write(&mut host.context(), 32);
    player.behavior_flags().write(&mut host.context(), 1);
    player
        .next()
        .write(&mut host.context(), tje_mem::Ptr::new(0));
    ram::g_entity_draw_list_head(revision)
        .write(&mut host.context(), tje_mem::Ptr::new(player.address()));
    for flags in [1_u8, 3, 5] {
        for height in [-1_i16, 0, 1, 5, 10, 25, 26] {
            for state in [-1_i8, 0, 21] {
                for slot in 0..4 {
                    ram::g_control_remap_projectile_entities(revision)
                        .at(slot)
                        .write(&mut host.context(), 0);
                }
                ram::g_control_remap_projectile_entities(revision)
                    .at(0)
                    .write(&mut host.context(), 6);
                projectile.draw_flags().write(&mut host.context(), flags);
                projectile.sprite_state().write(&mut host.context(), state);
                projectile.height().write(&mut host.context(), height);
                projectile.width().write(&mut host.context(), 9);
                projectile.x().write(&mut host.context(), 400);
                projectile.y().write(&mut host.context(), 400);
                projectile.tile().write(&mut host.context(), 0x8000);
                projectile.frame_duration().write(&mut host.context(), 0);
                fixture(host, ids::EARTHLING_UPDATE_CONTROL_REMAP_PROJECTILES, &[]);
                for slot in 0..4 {
                    ram::g_control_remap_projectile_entities(revision)
                        .at(slot)
                        .write(&mut host.context(), 0);
                }
                for slot in 0..12 {
                    ram::g_shared_projectile_entities(revision)
                        .at(slot)
                        .write(&mut host.context(), 0);
                }
                for owner in [-1_i8, 0, 3] {
                    ram::g_shared_projectile_entities(revision)
                        .at(0)
                        .write(&mut host.context(), 6);
                    ram::g_shared_projectile_owners(revision)
                        .at(0)
                        .write(&mut host.context(), owner);
                    fixture(host, ids::PROCESS_TOMATO_RAIN, &[]);
                }
            }
        }
    }
    for terrain in [0_u32, 0x10, 0x7f, 0x80, 0xffff] {
        fixture(
            host,
            ids::MAP_IS_LARGE_PLACEMENT_TERRAIN_EXCLUDED,
            &[terrain],
        );
    }
    fixture(host, ids::PRESENT_CLEAR_WORLD_ENTITY_READINESS, &[]);
    ram::g_player_decoy_entities(revision)
        .at(0)
        .write(&mut host.context(), 6);
    fixture(host, ids::PRESENT_REGISTER_DECOY_HIT, &[6]);
    for input in 0_u8..=255 {
        ram::g_player_processed_input(revision)
            .at(0)
            .write(&mut host.context(), input);
        ram::g_player_control_remap_indices(revision)
            .at(0)
            .write(&mut host.context(), (input & 3) as i8);
        fixture(host, ids::PLAYER_REMAP_DIRECTIONAL_INPUT, &[0]);
    }
    fixture(host, ids::NO_OP, &[]);
}
