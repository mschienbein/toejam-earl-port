//! Opt-in world-lane ROM harness. Build through the private manifest recorded in
//! scratch/W0World/report.md; this is not a tje-game runtime module.
use std::{env, fs, path::Path};
use tje_game::{Context, Dispatcher, FunctionId, ids};
use tje_host::{Host, Mode, abi::HookContext, hooks::HookRegistry, input::InputScript};
use tje_mem::{Ptr, Revision, ram};

// This evidence batch owns exactly the original fourteen W0 roots (thirteen
// assigned leaves plus the pre-existing camera-priority pilot), not every
// subsequently translated camera/elevator function and its growing closure.
const W0_ROOTS: &[FunctionId] = &[
    ids::SET_CAMERA_TILE_PRIORITY_ENABLED,
    ids::CLEAR_CAMERA_TILE_ROW,
    ids::GET_LEVEL_METATILE_TILE,
    ids::GET_OVERLAY_METATILE_TILE,
    ids::CLEAR_CAMERA_TILE_COLUMN,
    ids::ELEVATOR_SET_STATE,
    ids::PLAYER_SET_POSITION_FROM_SAVED_MAP_CELL,
    ids::ELEVATOR_INVALIDATE_SECONDARY_RECORDS,
    ids::PROCESS_ELEVATOR,
    ids::ENTITY_FIND_NARROW_COLLISION_AT_POSITION,
    ids::ENTITY_FIND_MENU_AWARE_COLLISION_AT_POSITION,
    ids::ENTITY_FIND_VOLUME_COLLISION_AT_POSITION,
    ids::PROJECTILE_FIND_COLLISION_EXCLUDING_OWNER,
    ids::ENTITY_FIND_HORIZONTAL_COLLISION_AT_X,
];

fn fixture(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| panic!("{} {arguments:08x?}: {error}", id.metadata().name));
}

// R011 entries use the stack-assembly ABI, but their not_reproducible_c metadata
// intentionally permits compiler scratch registers. Strengthen this opt-in
// evidence contract to compare all registers and CCR without changing metadata.
fn strict_collision(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let id = tje_game::function_at(context.revision(), context.cpu().pc()).unwrap();
    let mut arguments = [0; 5];
    let count = usize::from(id.metadata().abi.argument_count);
    for (index, argument) in arguments[..count].iter_mut().enumerate() {
        *argument = context.argument_u32(index as u32);
    }
    Ok(Some(tje_game::call(context, id, &arguments[..count])?))
}

fn registry(revision: Revision) -> HookRegistry {
    let mut registry = HookRegistry::empty(revision);
    for translation in tje_game::camera::TRANSLATIONS
        .iter()
        .chain(tje_game::elevator::TRANSLATIONS)
        .filter(|entry| W0_ROOTS.contains(&entry.id))
    {
        registry
            .register_translation_with_execution(
                *translation,
                tje_game::execution_class(translation.id).expect("reviewed W0 execution class"),
            )
            .unwrap();
    }
    for translation in tje_game::collision::TRANSLATIONS
        .iter()
        .filter(|entry| W0_ROOTS.contains(&entry.id))
    {
        let mut symbol = translation.id.metadata().clone();
        symbol.abi.strict_registers = true;
        registry
            .register_contract(
                symbol,
                tje_game::is_atomic_capable(translation.id),
                strict_collision,
            )
            .unwrap();
    }
    registry
}

fn collision_fixtures(host: &mut Host) {
    let revision = host.revision();
    let entity = ram::g_player_info(revision).at(6);
    let target = ram::g_player_info(revision).at(7);
    let mut cases = vec![[
        100_i16, 100, 20, 100, 100, 20, 100, 100, 20, 32, 20, 7, 1, 1, 1,
    ]];
    // Each tuple holds current xyz, target xyz, proposed xyz, target width,
    // target pixel height, signed index, active/collidable/menu flags.
    for slot in 0..9 {
        for value in [
            i16::MIN,
            i16::MIN + 100,
            -1,
            0,
            7,
            8,
            10,
            13,
            20,
            21,
            22,
            30,
            31,
            99,
            107,
            108,
            113,
            i16::MAX,
        ] {
            let mut case = cases[0];
            case[slot] = value;
            cases.push(case);
        }
    }
    for slot in 9..15 {
        for value in [-128_i16, -1, 0, 1, 2, 7, 127, 128, 255] {
            let mut case = cases[0];
            case[slot] = value;
            cases.push(case);
        }
    }
    // Deliberate SUB.W overflow cases with successful broad-phase eligibility.
    for slot in [0, 2, 6, 7] {
        let mut case = cases[0];
        case[slot] = case[slot].wrapping_add(i16::MIN);
        cases.push(case);
    }
    for (number, case) in cases.iter().enumerate() {
        let mut context = host.context();
        for (object, start, index) in [(entity, 0, 6), (target, 3, case[11] as i8)] {
            object.x().write(&mut context, case[start]);
            object.y().write(&mut context, case[start + 1]);
            object.height().write(&mut context, case[start + 2]);
            object
                .width()
                .write(&mut context, if start == 0 { 32 } else { case[9] as u8 });
            object
                .height_pixels()
                .write(&mut context, if start == 0 { 20 } else { case[10] as u8 });
            object.entity_index().write(&mut context, index);
            object.level().write(&mut context, 1);
            object
                .draw_flags()
                .write(&mut context, if start == 0 { 1 } else { case[12] as u8 });
            object
                .behavior_flags()
                .write(&mut context, if start == 0 { 1 } else { case[13] as u8 });
            object.next().write(&mut context, Ptr::new(0));
        }
        ram::g_player_inventory_open(revision)
            .at(i32::from(case[11] as i8))
            .write(&mut context, case[14] as i8);
        ram::g_entity_draw_list_head(revision).write(&mut context, Ptr::new(target.address()));
        for register in 0..8 {
            context.set_data_register(
                register,
                0x1357_2468_u32.wrapping_add(number as u32 * 0x10001 + register as u32),
            );
        }
        for register in 0..7 {
            context.set_address_register(register, 0x2468_1357 + register as u32 * 0x10001);
        }
        let sr = context.status_register();
        context.set_status_register((sr & !0x1f) | (number as u16 & 0x1f));
        fixture(
            host,
            ids::ENTITY_FIND_NARROW_COLLISION_AT_POSITION,
            &[
                entity.address(),
                case[6] as u32,
                case[7] as u32,
                (-128_i32) as u32,
            ],
        );
        fixture(
            host,
            ids::ENTITY_FIND_MENU_AWARE_COLLISION_AT_POSITION,
            &[entity.address(), case[6] as u32, case[7] as u32],
        );
        fixture(
            host,
            ids::ENTITY_FIND_VOLUME_COLLISION_AT_POSITION,
            &[
                entity.address(),
                case[6] as u32,
                case[7] as u32,
                case[8] as u32,
                (-128_i32) as u32,
            ],
        );
        fixture(
            host,
            ids::PROJECTILE_FIND_COLLISION_EXCLUDING_OWNER,
            &[entity.address(), 99],
        );
        fixture(
            host,
            ids::ENTITY_FIND_HORIZONTAL_COLLISION_AT_X,
            &[
                entity.address(),
                case[6] as u32,
                case[7] as u32,
                (-128_i32) as u32,
            ],
        );
    }
    for head in [0, entity.address(), target.address()] {
        let mut context = host.context();
        ram::g_entity_draw_list_head(revision).write(&mut context, Ptr::new(head));
        entity.next().write(
            &mut context,
            Ptr::new(if head == entity.address() {
                target.address()
            } else {
                0
            }),
        );
        target.next().write(&mut context, Ptr::new(0));
        for (id, args) in [
            (
                ids::ENTITY_FIND_NARROW_COLLISION_AT_POSITION,
                vec![entity.address(), 100, 100, 0],
            ),
            (
                ids::ENTITY_FIND_MENU_AWARE_COLLISION_AT_POSITION,
                vec![entity.address(), 100, 100],
            ),
            (
                ids::ENTITY_FIND_VOLUME_COLLISION_AT_POSITION,
                vec![entity.address(), 100, 100, 20, 0],
            ),
            (
                ids::PROJECTILE_FIND_COLLISION_EXCLUDING_OWNER,
                vec![entity.address(), 7],
            ),
            (
                ids::ENTITY_FIND_HORIZONTAL_COLLISION_AT_X,
                vec![entity.address(), 100, 0xffff, 0],
            ),
        ] {
            fixture(host, id, &args);
        }
    }
}

fn camera_fixtures(host: &mut Host) {
    let revision = host.revision();
    for camera in [-1_i16, 0, 1, 2] {
        fixture(host, ids::CLEAR_CAMERA_TILE_ROW, &[camera as u32]);
        for height in [-1_i16, 0, 19, 32] {
            ram::g_camera_column_upload_height(revision).write(&mut host.context(), height);
            fixture(host, ids::CLEAR_CAMERA_TILE_COLUMN, &[camera as u32]);
        }
    }
    for metatile in [0_u32, 1, 14, 81, 121, 0x80, 0x81, 0xff, 0x7f01, 0xffff] {
        for row in [0_u32, 7, 8, 0x7ff, 0xffff] {
            for column in [0_u32, 3, 15, 16, 0xffff] {
                fixture(host, ids::GET_LEVEL_METATILE_TILE, &[metatile, row, column]);
            }
        }
    }
    for metatile in [0_u32, 1, 15, 0x10, 0x11, 0x1f, 0xffff] {
        for row in [0_u32, 7, 8, 0x7ff, 0xffff] {
            for column in [0_u32, 3, 15, 16, 0xffff] {
                fixture(
                    host,
                    ids::GET_OVERLAY_METATILE_TILE,
                    &[metatile, row, column],
                );
            }
        }
    }
}

fn elevator_fixtures(host: &mut Host) {
    let revision = host.revision();
    let record = ram::g_elevator_records(revision).at(0).at(0);
    for entity in [0_i8, 2, 3, -1] {
        record.entity().write(&mut host.context(), entity);
        for state in (0_i8..=13).chain([i8::MIN, -1, i8::MAX]) {
            fixture(
                host,
                ids::ELEVATOR_SET_STATE,
                &[record.address(), state as u32],
            );
        }
    }
    for x in [-128_i8, -1, 0, 1, 127] {
        for y in [-128_i8, -1, 0, 1, 127] {
            let mut context = host.context();
            ram::g_arrival_elevator_map_x(revision).write(&mut context, x);
            ram::g_arrival_elevator_map_y(revision).write(&mut context, y);
            fixture(
                host,
                ids::PLAYER_SET_POSITION_FROM_SAVED_MAP_CELL,
                &[ram::g_player_info(revision).at(0).address()],
            );
        }
    }
    fixture(
        host,
        ids::PLAYER_SET_POSITION_FROM_SAVED_MAP_CELL,
        &[ram::g_arrival_elevator_map_x(revision).address()],
    );
    fixture(host, ids::ELEVATOR_INVALIDATE_SECONDARY_RECORDS, &[]);
    for handle in 0..=255_u32 {
        let zero_based = (handle as i8).wrapping_sub(1);
        let record = ram::g_elevator_records(revision)
            .at(i32::from(zero_based >> 1))
            .at(i32::from(zero_based & 1));
        for state in [3_i8, 2] {
            record.state().write(&mut host.context(), state);
            fixture(host, ids::PROCESS_ELEVATOR, &[handle]);
        }
    }
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let root = Path::new(&args[1]);
    let evidence = Path::new(&args[2]);
    fs::create_dir_all(evidence).unwrap();
    let script = InputScript::parse(
        &fs::read_to_string(root.join("crates/tje-host/examples/start-game.json")).unwrap(),
    )
    .unwrap();
    let roots = W0_ROOTS;
    assert!(
        Dispatcher::native(roots).is_ok(),
        "world native closure has missing translations"
    );
    for revision in [Revision::Rev02, Revision::Rev00] {
        let name = if revision == Revision::Rev02 {
            "rev02"
        } else {
            "rev00"
        };
        let bytes = fs::read(root.join(format!("roms/{name}.bin"))).unwrap();
        let mut host = Host::new(&bytes, Mode::Check, registry(revision)).unwrap();
        let mut reference =
            Host::new(&bytes, Mode::Reference, HookRegistry::empty(revision)).unwrap();
        for frame in 0..10000 {
            let actual = host.run_frame(script.at(frame)).unwrap();
            let expected = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(actual.pc, expected.pc, "{name} frame {frame} PC");
            assert_eq!(
                actual.frame_counter, expected.frame_counter,
                "{name} frame {frame} counter"
            );
            assert_eq!(actual.ram, expected.ram, "{name} frame {frame} whole RAM");
            assert_eq!(actual.vdp, expected.vdp, "{name} frame {frame} VDP");
            assert_eq!(
                actual.writes, expected.writes,
                "{name} frame {frame} hardware trace"
            );
            assert_eq!(
                actual.idle_random_draws, expected.idle_random_draws,
                "{name} frame {frame} idle draws"
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
                "{name} frame {frame} audio bits"
            );
            assert_eq!(
                host.frame_buffer().0,
                reference.frame_buffer().0,
                "{name} frame {frame} pixels"
            );
        }
        assert!(
            host.measurements.game_frame.is_some(),
            "{name} never reached gameplay"
        );
        let natural = serde_json::to_value(&host.checks).unwrap();
        let measurements = serde_json::to_value(&host.measurements).unwrap();
        fs::write(evidence.join(format!("{name}-natural.json")), serde_json::to_vec_pretty(&serde_json::json!({"checks": natural, "measurements": measurements, "whole_session_reference_equal_frames": 10000})).unwrap()).unwrap();
        collision_fixtures(&mut host);
        camera_fixtures(&mut host);
        elevator_fixtures(&mut host);
        assert_eq!(host.checks.mismatches, 0);
        assert!(host.checks.uninitialized_stack_reads.is_empty());
        for id in roots {
            if *id != ids::SET_CAMERA_TILE_PRIORITY_ENABLED {
                assert!(
                    host.checks.by_function[id.metadata().name].calls_checked > 0,
                    "unexercised {}",
                    id.metadata().name
                );
            }
        }
        fs::write(
            evidence.join(format!("{name}-with-fixtures.json")),
            serde_json::to_vec_pretty(&host.checks).unwrap(),
        )
        .unwrap();
        println!(
            "{name}: 10000 reference-equal frames; title {:?}, gameplay {:?}; {} checked calls; zero mismatches, zero uninitialized stack reads",
            host.measurements.title_frame, host.measurements.game_frame, host.checks.calls
        );
        println!("{}", serde_json::to_string_pretty(&host.checks).unwrap());
    }
}
