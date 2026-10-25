//! Opt-in external host harness; never linked into tje-game.
//! Build with a private manifest whose bin path names this file and whose
//! dependencies are tje-game, tje-host, tje-mem, tje-rom, and serde_json. Arguments:
//! ROM INPUT_JSON METRICS_JSON FRAMES. No ROM-derived state is written to disk.
//! Optional TJE_MAP_FIXTURE_FILTER selects comma-separated canonical map names;
//! only selected hooks contribute coverage, and unexercised selections still fail.
mod fixture_cases;
mod fixture_display;
mod fixture_generation;
mod fixture_lowering;
mod fixture_time;
use std::{error::Error, fs};
use tje_game::{Dispatcher, FunctionId, ids, map};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{ram, rom};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

fn check(host: &mut Host, id: FunctionId, args: &[u32], label: &str) -> Result {
    if !host.checks.by_function.contains_key(id.metadata().name) {
        return Ok(());
    }
    eprintln!("fixture {}: {label}; args={args:?}", id.metadata().name);
    host.check_fixture(id.metadata().address(host.revision()), args)
        .map_err(|error| {
            format!(
                "live-state fixture {label}, {}: {error}",
                id.metadata().name
            )
        })?;
    Ok(())
}

fn fill_map(host: &mut Host, value: u16) {
    let map = ram::g_player1_level_map(host.revision());
    let mut c = host.context();
    for row in 0..26 {
        for column in 0..19 {
            map.at(row).at(column).write(&mut c, value);
        }
    }
}

fn fixtures(host: &mut Host) -> Result {
    let revision = host.revision();
    let map = ram::g_player1_level_map(revision);
    let directions = ram::g_map_path_direction_banks(revision);
    let anchors = ram::g_level_anchor_positions(revision);
    // Start each bank operation from authored distinguishable cells and nibbles.
    for id in [
        ids::MAP_SWAP_PLAYER_BANKS,
        ids::COPY_LEVEL_BETWEEN_PLAYERS,
        ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK,
    ] {
        for player in [0_i32, 1, -1] {
            let mut c = host.context();
            ram::g_player1_map_level(revision).write(&mut c, 3);
            ram::g_player2_map_level(revision).write(&mut c, 7);
            for row in 0..26 {
                for column in 0..19 {
                    let index = row * 19 + column;
                    map.at(row).at(column).write(&mut c, (index * 37) as u16);
                    ram::g_player2_level_map(revision)
                        .at(row)
                        .at(column)
                        .write(&mut c, !((index * 37) as u16));
                    ram::g_saved_level_map(revision)
                        .at(row)
                        .at(column)
                        .write(&mut c, (index as u16) ^ 0x55aa);
                    directions
                        .at(row)
                        .at(column)
                        .write(&mut c, (index as u8) ^ 0x6d);
                    ram::g_saved_map_path_directions(revision)
                        .at(row)
                        .at(column)
                        .write(&mut c, (index as u8) ^ 0xa7);
                }
            }
            let argument = [player as u32];
            check(
                host,
                id,
                if id == ids::MAP_SWAP_PLAYER_BANKS {
                    &[]
                } else {
                    &argument
                },
                "distinct bank words and nibbles; zero/nonzero player",
            )?;
        }
    }
    check(
        host,
        ids::LOAD_LEVEL1_TILES,
        &[],
        "live gameplay; both level-one planes",
    )?;
    for level in [26_i32, -1, 0x1234, i32::from(i16::MIN)] {
        check(
            host,
            ids::LOAD_FINAL_LEVEL,
            &[level as u32],
            "level-byte narrowing",
        )?;
    }
    for upper in [0_u16, 21, 14] {
        for lower in [0_u16, 7, 19] {
            fill_map(host, 0);
            let mut c = host.context();
            map.at(5).at(5).write(&mut c, 0x880f);
            map.at(4).at(5).write(&mut c, upper);
            map.at(6).at(5).write(&mut c, lower);
            // First and second pass, including their reads into adjacent rows.
            map.at(0).at(0).write(&mut c, 0x0803);
            map.at(0).at(-1).write(&mut c, 11);
            map.at(0).at(1).write(&mut c, 0);
            map.at(9).at(4).write(&mut c, 0x080c);
            map.at(9).at(3).write(&mut c, 28);
            check(
                host,
                ids::MAP_REFINE_FILLED_SHAPE_MASKS,
                &[],
                "three masks passes and all corner decisions",
            )?;
        }
    }
    for mode in [0, 1] {
        for pending in [0_u8, 15] {
            for alternate in [0_u16, 0x8000] {
                fill_map(host, 0);
                let mut c = host.context();
                ram::g_map_directional_path_mode(revision).write(&mut c, mode);
                for row in 0..26 {
                    for column in 0..19 {
                        directions.at(row).at(column).write(&mut c, pending);
                    }
                }
                for (row, column, dy, dx, bit) in [
                    (4, 4, -1, 0, 4),
                    (4, 10, 0, 1, 8),
                    (10, 4, 1, 0, 1),
                    (10, 10, 0, -1, 2),
                ] {
                    map.at(row).at(column).write(&mut c, alternate | 0x1000);
                    map.at(row + dy).at(column + dx).write(&mut c, 0x2000 | bit);
                }
                for shape in 0..16 {
                    map.at(20).at(shape).write(&mut c, 0x2000 | shape as u16);
                }
                check(
                    host,
                    ids::MAP_BUILD_DIRECTIONAL_PATH_SHAPES,
                    &[],
                    "neighbor priority; both ROM shape banks; pending and mode",
                )?;
            }
        }
    }
    let row_position = ram::g_saved_level_map(revision).cast::<i16>();
    let column_position = row_position.offset(1);
    for direction in [1_u32, 2, 4, 8] {
        for contents in [0x1100_u16, 0x3100, 0x0000] {
            fill_map(host, contents);
            let mut c = host.context();
            for index in 0..3 {
                anchors.at(index).x().write(&mut c, -1);
                anchors.at(index).y().write(&mut c, -1);
            }
            row_position.write(&mut c, 5);
            column_position.write(&mut c, 5);
            check(
                host,
                ids::MAP_FIND_INTERIOR_GROWTH_ORIGIN,
                &[
                    row_position.address(),
                    column_position.address(),
                    direction,
                    0x3100,
                ],
                "each direction: base succeeds, filled reaches bounds, foreign cell fails",
            )?;
        }
    }
    for anchor_index in 0..3 {
        fill_map(host, 0x1100);
        let mut c = host.context();
        for index in 0..3 {
            anchors.at(index).x().write(&mut c, -1);
            anchors.at(index).y().write(&mut c, -1);
        }
        anchors.at(anchor_index).x().write(&mut c, 5);
        anchors.at(anchor_index).y().write(&mut c, 4);
        row_position.write(&mut c, 5);
        column_position.write(&mut c, 5);
        check(
            host,
            ids::MAP_FIND_INTERIOR_GROWTH_ORIGIN,
            &[row_position.address(), column_position.address(), 1, 0x3100],
            "each anchor blocks growth",
        )?;
    }
    for (row, column, height, width, replacement) in [
        (5_i32, 5_i32, 2_i32, 3_i32, 0x1200_u32),
        (0, 0, 0, 4, 0x1200),
        (0, 0, 4, -1, 0x1200),
        (-1, -1, 2, 2, 0x8100),
        (4000, 0, 1, 1, 0x1200),
    ] {
        fill_map(host, 0x1100);
        map.at(7).at(12).write(&mut host.context(), 0x1100);
        // Word-wrapped row 4000 addresses live RAM above SP. Row 2000 instead
        // addresses old stack bytes and is deliberately not an approvable fixture.
        map.cast::<u16>()
            .byte_offset(i32::from((4000_i16).wrapping_mul(38)))
            .write(&mut host.context(), 0x1100);
        check(
            host,
            ids::MAP_FILL_RECTANGLE_AND_MERGE_REGIONS,
            &[
                row as u32,
                column as u32,
                height as u32,
                width as u32,
                replacement,
            ],
            "global label merge; empty rectangles; signed/word-wrapped subscripts",
        )?;
    }
    for id in [
        ids::MAP_WIDEN_HORIZONTAL_REGION_EDGES,
        ids::MAP_WIDEN_VERTICAL_REGION_EDGES,
    ] {
        for transpose in [false, true] {
            fill_map(host, 0);
            let mut c = host.context();
            for fixed in [0, 8, 18, 25] {
                for moving in [1, 3, 4, 5, 6, 7, 8, 9] {
                    let (row, column) = if transpose {
                        (moving, fixed)
                    } else {
                        (fixed, moving)
                    };
                    if column < 19 {
                        map.at(row).at(column).write(&mut c, 0x1100);
                    }
                }
            }
            check(
                host,
                id,
                &[],
                "boundary clear; interior extension; gap retains remembered label",
            )?;
        }
    }
    for list in 1..=16 {
        let list_address = rom::map_transition_descriptors(revision)
            .at(list - 1)
            .shapes()
            .read(&host.context())
            .target();
        let count = list_address.read(&host.context());
        for value in [
            0,
            0xff,
            list_address.offset(1).read(&host.context()),
            list_address.offset(i32::from(count)).read(&host.context()),
        ] {
            check(
                host,
                ids::MAP_TRANSITION_CONTAINS_METATILE,
                &[list as u32, u32::from(value)],
                "each real descriptor, first/last entries and absent values",
            )?;
        }
    }
    for value in 0..=255 {
        check(
            host,
            ids::MAP_FIND_PATH_SHAPE_FOR_METATILE,
            &[value],
            "all unsigned metatiles, including not found",
        )?;
    }
    for value in [i32::MIN, -1, 0, 1, 2, 4, 8, 15, 255, i32::MAX] {
        check(
            host,
            ids::MAP_REVERSE_PATH_DIRECTION,
            &[value as u32],
            "sentinel, cardinal and non-cardinal full int",
        )?;
    }
    for bank in [0_i32, 1, -1] {
        for (row, column) in [
            (0_i32, 0_i32),
            (25, 18),
            (-1, -1),
            (4000, 0),
            (i32::from(i16::MAX), 0),
        ] {
            directions
                .cast::<u8>()
                .byte_offset(i32::from((4000_i16).wrapping_mul(19)))
                .write(&mut host.context(), 0xa7);
            check(
                host,
                ids::MAP_GET_BANK_PATH_DIRECTIONS,
                &[bank as u32, row as u32, column as u32],
                "both nibbles and word-row wrap",
            )?;
        }
    }
    for player in [0, 1] {
        for flag in [i8::MIN, -2, -1, 0, 1, 2, 3, i8::MAX] {
            ram::g_player_elevator_flags(revision)
                .at(player)
                .write(&mut host.context(), flag);
            check(
                host,
                ids::PLAYER_HAS_ODD_ELEVATOR_STATE,
                &[player as u32],
                "signed elevator-state parity",
            )?;
        }
        let terrain = if player == 0 {
            ram::g_player1_nearby_terrain(revision)
        } else {
            ram::g_player2_nearby_terrain(revision)
        }
        .cast::<u8>();
        for edge in [-1, 7, 11, 13, 17] {
            for value in [0x4f, 0x50, 0xff] {
                let mut c = host.context();
                for offset in 0..25 {
                    terrain.offset(offset).write(&mut c, 0);
                }
                if edge >= 0 {
                    terrain.offset(edge).write(&mut c, value);
                }
                check(
                    host,
                    ids::PLAYER_IS_NEAR_TERRAIN_EDGE,
                    &[player as u32],
                    "each adjacent cell and unsigned threshold",
                )?;
            }
        }
    }
    for reveal in [0, 1] {
        ram::g_map_reveal_mode(revision).write(&mut host.context(), reveal);
        for value in [34_u16, 0xa2, 0] {
            fill_map(host, value);
            for (row, column) in [
                (0_i32, 0_i32),
                (6, 7),
                (-1, -1),
                (7, 8),
                (0x4000, 0x4000),
                (i32::MIN, i32::MAX),
            ] {
                check(
                    host,
                    ids::MAP_COVER_BLOCK_CONTAINS_TERRAIN,
                    &[map.address(), row as u32, column as u32],
                    "reveal gate; negative truncation; clipped and wrapped cover coordinates",
                )?;
            }
        }
    }
    let sprites = ram::g_saved_level_map(revision).address();
    for index in [7_i16, 78, 79, -1, i16::MIN] {
        for retain in [0_i32, 1, -1] {
            ram::g_map_screen_next_sprite_index(revision).write(&mut host.context(), index);
            check(
                host,
                ids::MAP_APPEND_COVER_BLOCK_SPRITES,
                &[6, 7, sprites, retain as u32],
                "last permitted pair, exhausted table, retain gate, negative index",
            )?;
        }
    }
    ram::g_map_screen_next_sprite_index(revision).write(&mut host.context(), 0);
    check(
        host,
        ids::MAP_APPEND_COVER_BLOCK_SPRITES,
        &[
            0,
            (-3_i32) as u32,
            ram::g_map_screen_next_sprite_index(revision).address() - 6,
            0,
        ],
        "sprite x aliases shared index; later accesses must reload",
    )?;
    // Authored graphs exercise the visited check, anchor hit, all edge types,
    // reciprocal checks, failed recursion, and the unbounded path-class branch.
    for direction in 0..4 {
        for boundary in [false, true] {
            fill_map(host, 0);
            let (dy, dx, outgoing, incoming) =
                [(-1, 0, 1, 4), (1, 0, 4, 1), (0, -1, 8, 2), (0, 1, 2, 8)][direction];
            let mut c = host.context();
            anchors.at(1).x().write(&mut c, (5 + dx) as i8);
            anchors.at(1).y().write(&mut c, (5 + dy) as i8);
            map.at(5)
                .at(5)
                .write(&mut c, if boundary { 0x1100 } else { 0x2000 | outgoing });
            map.at(5 + dy)
                .at(5 + dx)
                .write(&mut c, if boundary { 0x2000 | incoming } else { 0x1100 });
            check(
                host,
                ids::MAP_FIND_ROUTE_TO_SECOND_ANCHOR,
                &[5, 5],
                "each axis; boundary/path reciprocal traversal",
            )?;
        }
    }
    for (start, neighbor, anchor_y) in [
        (0x0800_u16, 0_u16, 5_i8),
        (0x1100, 0, 5),
        (0x2001, 0x1000, -1),
    ] {
        fill_map(host, 0);
        let mut c = host.context();
        map.at(0).at(0).write(&mut c, start);
        map.at(-1).at(0).write(&mut c, neighbor);
        anchors.at(1).x().write(&mut c, 0);
        anchors.at(1).y().write(&mut c, anchor_y);
        check(
            host,
            ids::MAP_FIND_ROUTE_TO_SECOND_ANCHOR,
            &[0, 0],
            "visited, isolated, or out-of-map path",
        )?;
    }
    fixture_cases::run(host)?;
    fixture_generation::run(host)?;
    fixture_display::run(host)?;
    fixture_time::run(host)?;
    fixture_lowering::run(host)?;
    Ok(())
}

fn main() -> Result {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        return Err("expected ROM INPUT_JSON METRICS_JSON FRAMES".into());
    }
    let frames: u64 = args[4].parse()?;
    let script = std::sync::Arc::new(InputScript::parse(&fs::read_to_string(&args[2])?)?);
    let bytes = fs::read(&args[1])?;
    let revision = tje_rom::Rom::import(&bytes)?.revision();
    let filter = std::env::var("TJE_MAP_FIXTURE_FILTER").ok();
    let selected = filter
        .as_ref()
        .map(|filter| {
            if filter == "production" {
                return Ok(map::TRANSLATIONS.iter().map(|entry| entry.id).collect());
            }
            filter
                .split(',')
                .map(|name| {
                    tje_game::functions()
                        .find(|function| function.name == name && function.subsystem == "map")
                        .map(|function| function.id)
                        .ok_or_else(|| format!("unknown map function: {name}"))
                })
                .collect::<std::result::Result<Vec<_>, _>>()
        })
        .transpose()?;
    // Hook selection never changes Rust callee composition: retain every ready
    // pending body ahead of the production dispatcher, even for a single root.
    static BODIES: std::sync::LazyLock<Vec<tje_game::Translation>> =
        std::sync::LazyLock::new(|| {
            map::TRANSLATIONS
                .iter()
                .chain(map::PENDING_TRANSLATIONS)
                .copied()
                .collect()
        });
    let registry = HookRegistry::proof_filtered_with_frames(
        revision,
        BODIES.as_slice(),
        map::PENDING_FRAMED_TRANSLATIONS,
        selected.as_deref().unwrap_or_default(),
    )?;
    let mut checked = Host::new(&bytes, Mode::Check, registry.clone())?;
    let mut reference = Host::new(&bytes, Mode::Reference, registry)?;
    checked.set_wait_input_poller(script.clone());
    reference.set_wait_input_poller(script.clone());
    let result = (|| -> Result {
        for frame in 0..frames {
            let a = checked.run_frame(script.at(frame))?;
            let b = reference.run_frame(script.at(frame))?;
            assert_eq!(a.frame_counter, b.frame_counter, "frame {frame} counter");
            assert_eq!(a.ram, b.ram, "frame {frame} whole RAM");
            assert_eq!(a.vdp, b.vdp, "frame {frame} VDP");
            assert_eq!(a.writes, b.writes, "frame {frame} hardware writes");
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits())),
                "frame {frame} audio"
            );
            let (a, sa) = checked.frame_buffer();
            let (b, sb) = reference.frame_buffer();
            assert_eq!(sa, sb);
            assert_eq!(
                &a[..sa.len() as usize],
                &b[..sb.len() as usize],
                "frame {frame} pixels"
            );
        }
        assert!(checked.measurements.title_frame.is_some());
        assert!(checked.measurements.game_frame.is_some());
        Ok(())
    })();
    let natural = serde_json::to_value(&checked.checks)?;
    let mut boundary = None;
    let result = result.and_then(|()| {
        let advance = checked.settle_fixture_boundary(script.at(checked.frame_count()))?;
        boundary = Some(serde_json::json!({
            "start_pc": advance.start_pc, "end_pc": advance.end_pc,
            "start_frame": advance.start_frame, "end_frame": advance.end_frame,
            "ticks": advance.ticks, "master_clocks": advance.master_clocks,
            "handlers_completed": advance.handlers_completed,
            "completed_frames": advance.completed_frames.iter().map(|frame| {
                serde_json::json!({
                    "frame": frame.frame, "frame_counter": frame.frame_counter, "pc": frame.pc,
                    "ram": frame.ram, "writes": frame.writes, "lag_events": frame.lag_events,
                    "idle_random_draws": frame.idle_random_draws,
                    "audio_sample_bits": frame.audio_samples.iter()
                        .map(|sample| sample.map(f64::to_bits)).collect::<Vec<_>>(),
                    "vdp": { "vram": frame.vdp.vram, "cram": frame.vdp.cram,
                        "vsram": frame.vdp.vsram, "registers": frame.vdp.registers },
                })
            }).collect::<Vec<_>>(),
        }));
        fixtures(&mut checked)
    });
    let roots: Vec<_> = map::TRANSLATIONS.iter().map(|entry| entry.id).collect();
    let missing: Vec<_> = Dispatcher::native(&roots)
        .err()
        .unwrap_or_default()
        .iter()
        .map(|id| id.metadata().name)
        .collect();
    fs::write(
        &args[3],
        serde_json::to_string_pretty(&serde_json::json!({
            "revision": revision.name(), "frames": checked.frame_count(),
            "filter": filter,
            "title_frame": checked.measurements.title_frame,
            "menu_frame": checked.measurements.menu_frame,
            "game_frame": checked.measurements.game_frame,
            "logic_ticks": checked.measurements.logic_ticks,
            "whole_session_equal": natural["mismatches"] == 0 && checked.frame_count() == frames,
            "native_missing": missing, "natural": natural,
            "fixture_boundary": boundary,
            "with_live_fixtures": checked.checks, "error": result.as_ref().err().map(ToString::to_string),
        }))?,
    )?;
    result?;
    assert_eq!(checked.checks.mismatches, 0);
    assert!(checked.checks.uninitialized_stack_reads.is_empty());
    assert!(checked.checks.unexercised.is_empty());
    println!(
        "{}: {frames} frames, all selected map hooks exercised, zero mismatches",
        revision.name()
    );
    Ok(())
}
