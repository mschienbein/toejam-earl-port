//! Opt-in host harness, compiled as a separate scratch Cargo binary (not a game dependency).
//! Runs the standard moving-player session and explicitly labelled synthetic live-state fixtures.
use std::{env, fs, path::PathBuf};
use tje_game::{Context, FunctionId, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Address, Ptr, RenderPiece, Revision, ram};

fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    if !host.checks.by_function.contains_key(id.metadata().name) {
        return; // Explicit function filtering also filters synthetic fixture calls.
    }
    let result = host.check_fixture(id.metadata().address(host.revision()), arguments);
    if let Err(error) = result {
        let evidence =
            PathBuf::from(env::var_os("TJE_SPRITE_EVIDENCE").expect("TJE_SPRITE_EVIDENCE"));
        let key = match host.revision() {
            Revision::Rev02 => "rev02",
            Revision::Rev00 => "rev00",
        };
        let failure = serde_json::json!({"function":id.metadata().name, "arguments":arguments, "error":error.to_string(), "checks":host.checks});
        fs::write(
            evidence.join(format!("{key}-fixture-failed.json")),
            serde_json::to_vec_pretty(&failure).unwrap(),
        )
        .unwrap();
        panic!(
            "live-state fixture {} {arguments:08x?}: {error}",
            id.metadata().name
        );
    }
}

fn fixture_coverage(host: &mut Host) {
    let revision = host.revision();
    let entity = ram::g_player_info(revision).at(39);
    let buffer = ram::g_sprite_tile_upload_buffer(revision).address();
    let piece = Address::<RenderPiece>::new(buffer + 7680);
    let stream = buffer + 7800;

    for index in 0..40 {
        ram::g_player_info(revision)
            .at(index)
            .draw_flags()
            .write(&mut host.context(), 0);
    }
    for level in [1, 26] {
        ram::g_player1_map_level(revision).write(&mut host.context(), level);
        check(host, ids::SORT_ENTITIES_FOR_SPRITE_DRAW, &[]);
        for (index, y, flags) in [
            (0, 20, 1),
            (1, 20, 1),
            (2, -32768, 1),
            (25, 32767, 0x81),
            (26, 30, 1),
            (39, 32767, 0x81),
        ] {
            let object = ram::g_player_info(revision).at(index);
            object.y().write(&mut host.context(), y);
            object.draw_flags().write(&mut host.context(), flags);
        }
        check(host, ids::SORT_ENTITIES_FOR_SPRITE_DRAW, &[]);
    }
    check(host, ids::SET_RAM_INDEXES, &[]);
    entity.level().write(&mut host.context(), 1);
    entity.height().write(&mut host.context(), 0);
    ram::g_player1_map_level(revision).write(&mut host.context(), 1);
    ram::g_player2_map_level(revision).write(&mut host.context(), 1);
    ram::g_camera1_pending_x(revision).write(&mut host.context(), 0);
    ram::g_camera1_pending_y(revision).write(&mut host.context(), 0);
    ram::g_camera2_pending_x(revision).write(&mut host.context(), 0);
    ram::g_camera2_pending_y(revision).write(&mut host.context(), 0);
    ram::g_player_inventory_open(revision)
        .at(0)
        .write(&mut host.context(), 0);
    ram::g_player2_inventory_open(revision).write(&mut host.context(), 0);
    for split in [0, 1] {
        ram::g_split_screen_enabled(revision).write(&mut host.context(), split);
        for (x, y, width, height) in [
            (-32768, -32768, 255, 255),
            (-11, 1, 10, 10),
            (-10, 1, 10, 10),
            (0, -10, 0, 0),
            (0, -9, 0, 0),
            (0, 112, 0, 0),
            (0, 224, 0, 0),
            (330, 100, 10, 10),
            (331, 100, 10, 10),
            (32767, 32767, 255, 255),
        ] {
            entity.x().write(&mut host.context(), x);
            entity.y().write(&mut host.context(), y);
            entity.width().write(&mut host.context(), width);
            entity.height_pixels().write(&mut host.context(), height);
            for viewport in [0, 1, 0xffff] {
                check(
                    host,
                    ids::IS_ENTITY_VISIBLE_IN_VIEWPORT,
                    &[entity.address(), viewport],
                );
            }
        }
    }
    ram::g_player_inventory_open(revision)
        .at(0)
        .write(&mut host.context(), 1);
    check(
        host,
        ids::IS_ENTITY_VISIBLE_IN_VIEWPORT,
        &[entity.address(), 0],
    );
    ram::g_player2_inventory_open(revision).write(&mut host.context(), 1);
    check(
        host,
        ids::IS_ENTITY_VISIBLE_IN_VIEWPORT,
        &[entity.address(), 1],
    );
    for player in [0, 1] {
        let object = ram::g_player_info(revision).at(player);
        object
            .entity_index()
            .write(&mut host.context(), player as i8);
        for state in [-128i8, -1, 0, 1, 120, 127] {
            check(
                host,
                ids::SET_SPRITE,
                &[object.address(), state as i32 as u32],
            );
        }
    }

    for handle in [-128i8, -1, 0, 24, 25, 78, 127] {
        check(host, ids::FREE_SPRITE_SLOT, &[handle as i32 as u32]);
    }
    for index in 0..12 {
        entity
            .primary_sprites()
            .at(index)
            .write(&mut host.context(), (25 + index) as i8);
    }
    for count in [-1i16, 0, 1, 12] {
        check(
            host,
            ids::FREE_SPRITE_SLOTS,
            &[count as i32 as u32, entity.primary_sprites().address()],
        );
    }
    for flags in [0, 1, 2, 3, 4, 7, -1] {
        ram::g_viewport_sprite_slot_flags(revision)
            .at(0)
            .write(&mut host.context(), flags);
        check(host, ids::IS_SPRITE_SLOT_LOCKED, &[25]);
    }
    for pattern in [0, 1, 2] {
        for index in 0..80 {
            let allocated = match pattern {
                0 => 0,
                1 => 1,
                _ => (index % 3 != 0) as i8,
            };
            ram::g_sprite_tile_group_allocated(revision)
                .at(index)
                .write(&mut host.context(), allocated);
        }
        for count in [i16::MIN, -9, -1, 0, 1, 8, 9, 640, 641, i16::MAX] {
            check(host, ids::ALLOCATE_SPRITE_TILE_RUN, &[count as i32 as u32]);
        }
    }
    for count in [i16::MIN, -9, -1, 0, 1, 8, 9, 640, 641, i16::MAX] {
        for tile in [640i16, 647, 648, 0, i16::MIN, i16::MAX] {
            check(
                host,
                ids::FREE_SPRITE_TILE_RUN,
                &[tile as i32 as u32, count as i32 as u32],
            );
        }
    }
    check(host, ids::RESET_ENTITY_RUNTIME_STATE, &[entity.address()]);

    ram::g_sprite_tile_buffer_offset(revision).write(&mut host.context(), 0);
    piece.width().write(&mut host.context(), 2);
    piece.height().write(&mut host.context(), 2);
    for y in [-128, -10, 0, 127] {
        piece.y().write(&mut host.context(), y);
        // A maximal positive cutoff can make the original zero its own call stack;
        // keep differential return fixtures within returning states, including SUB.W overflow.
        for cutoff in [i16::MIN, -200, -10, -2, -1, 0, 10, 200] {
            for index in 0..128 {
                host.context().write_u8(buffer + index, 0xff);
            }
            check(
                host,
                ids::CLIP_SPRITE_PIXELS_BELOW_CUTOFF,
                &[piece.address(), cutoff as i32 as u32],
            );
        }
    }
    for count in [0, 1, 7, 64] {
        for index in 0..64 {
            host.context().write_u8(
                buffer + index,
                [0xff, 0xf0, 0x0f, 0, 0x12, 1, 0xfe][index as usize % 7],
            );
        }
        check(host, ids::CLEAR_SPRITE_MASK_PIXELS, &[count]);
    }
    for count in [-1i16, 0, 1, 2, 4096] {
        for index in 0..128 {
            host.context().write_u8(buffer + index, index as u8);
        }
        ram::g_sprite_tile_buffer_offset(revision).write(&mut host.context(), 4);
        check(
            host,
            ids::APPEND_SPRITE_TILES_TO_BUFFER,
            &[buffer, count as i32 as u32],
        );
    }

    piece
        .tile_data()
        .write(&mut host.context(), Ptr::new(stream));
    ram::g_sprite_tile_buffer_offset(revision).write(&mut host.context(), 0);
    let literal: Vec<u8> = std::iter::once(32).chain(0..32).collect();
    let mixed: Vec<u8> = [0xf8, 0x12, 0xb8, 0x48, 8]
        .into_iter()
        .chain(1..9)
        .collect();
    for data in [
        vec![0xe0, 0x12],
        vec![0xa0],
        vec![0x60],
        literal,
        mixed,
        vec![0xdf, 0x98],
    ] {
        for (index, value) in data.into_iter().enumerate() {
            host.context().write_u8(stream + index as u32, value);
        }
        seed_registers(host);
        check(host, ids::SPRITE_UNPACK_RLE, &[piece.address(), 1]);
    }
    for count in [0, 1024, 2048] {
        seed_registers(host);
        check(host, ids::SPRITE_UNPACK_RLE, &[piece.address(), count]);
    }
    for lane in [
        vec![0xf8, 0x12],
        vec![0xb8],
        vec![0x48],
        vec![0x68, 0x12],
        vec![8, 1, 2, 3, 4, 5, 6, 7, 8],
        vec![0x63, 0x12, 0xfb, 0x34],
        vec![0xf7, 0x98],
    ] {
        let data = lane.repeat(4);
        for (index, value) in data.into_iter().enumerate() {
            host.context().write_u8(stream + index as u32, value);
        }
        seed_registers(host);
        check(
            host,
            ids::SPRITE_UNPACK_INTERLEAVED_RLE,
            &[piece.address(), 1],
        );
    }
    for count in [0, 4096, 8192] {
        seed_registers(host);
        check(
            host,
            ids::SPRITE_UNPACK_INTERLEAVED_RLE,
            &[piece.address(), count],
        );
    }
    for last in [0, 0x1234_5678, 0x8000_0001] {
        for index in 0..16 {
            host.context().write_u32(stream + index * 4, last);
        }
        for count in [1, 2] {
            seed_registers(host);
            check(host, ids::SPRITE_UNPACK_RAW, &[piece.address(), count]);
        }
    }

    for id in [
        ids::TERMINATE_SPRITE_CHAIN_AT_HEAD,
        ids::TERMINATE_PRIMARY_SPRITE_CHAIN,
        ids::SELECT_PRIMARY_VIEWPORT_SPRITES,
        ids::SELECT_SECONDARY_VIEWPORT_SPRITES,
    ] {
        for link in [0, 79, 127, 128, 255] {
            ram::g_primary_viewport_uploaded_link(revision).write(&mut host.context(), link);
            ram::g_secondary_viewport_uploaded_link(revision).write(&mut host.context(), link);
            check(host, id, &[]);
        }
    }
    for flags in [0, 2, 4, 6] {
        let player = ram::g_player_info(revision).at(0);
        player.draw_flags().write(&mut host.context(), flags);
        player.behavior_flags().write(&mut host.context(), 0x10);
        player.sprite_ready().write(&mut host.context(), 1);
        player.sprite_count().write(&mut host.context(), 1);
        player
            .primary_sprites()
            .at(0)
            .write(&mut host.context(), 25);
        player
            .secondary_sprites()
            .at(0)
            .write(&mut host.context(), 78);
        check(host, ids::HIDE_FLAGGED_PLAYER_SPRITES, &[0]);
    }
}

fn seed_registers(host: &mut Host) {
    for index in 0..8 {
        host.context()
            .set_data_register(index, 0xa5a5_1234u32.wrapping_add(index as u32));
    }
    host.context().set_status_register(0x271f);
}

fn main() {
    let root = PathBuf::from(env::var_os("TJE_PORT_ROOT").expect("TJE_PORT_ROOT"));
    let evidence = PathBuf::from(env::var_os("TJE_SPRITE_EVIDENCE").expect("TJE_SPRITE_EVIDENCE"));
    let frames = env::var("TJE_SPRITE_FRAMES")
        .map(|value| value.parse().unwrap())
        .unwrap_or(10_000u64);
    let script = InputScript::parse(include_str!(
        "../../../tje-host/examples/moving-player.json"
    ))
    .unwrap();
    let selected: Vec<_> = env::var("TJE_SPRITE_FUNCTIONS")
        .map(|names| {
            names
                .split(',')
                .map(|name| {
                    tje_game::functions()
                        .find(|function| function.subsystem == "sprites" && function.name == name)
                        .expect("known sprite function")
                        .id
                })
                .collect()
        })
        .unwrap_or_default();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = match revision {
            Revision::Rev02 => "rev02",
            Revision::Rev00 => "rev00",
        };
        let bytes = fs::read(root.join("roms").join(format!("{key}.bin"))).unwrap();
        let subsystems: &[&str] = if selected.is_empty() {
            &["sprites"]
        } else {
            &[]
        };
        let registry = HookRegistry::filtered(revision, subsystems, &selected).unwrap();
        let mut checked = Host::new(&bytes, Mode::Check, registry.clone()).unwrap();
        let mut reference = Host::new(&bytes, Mode::Reference, registry).unwrap();
        for frame in 0..frames {
            let a = match checked.run_frame(script.at(frame)) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    fs::write(
                        evidence.join(format!("{key}-failed.json")),
                        serde_json::to_vec_pretty(&checked.checks).unwrap(),
                    )
                    .unwrap();
                    panic!("{revision:?} frame {frame}: {error}");
                }
            };
            let b = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(
                a.frame_counter, b.frame_counter,
                "{revision:?} frame {frame}"
            );
            assert_eq!(a.ram, b.ram, "{revision:?} whole RAM frame {frame}");
            assert_eq!(a.vdp, b.vdp, "{revision:?} VDP frame {frame}");
            assert_eq!(a.writes, b.writes, "{revision:?} writes frame {frame}");
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples
                    .iter()
                    .flatten()
                    .map(|value| value.to_bits())
                    .eq(b
                        .audio_samples
                        .iter()
                        .flatten()
                        .map(|value| value.to_bits())),
                "{revision:?} audio frame {frame}"
            );
            let (pa, sa) = checked.frame_buffer();
            let (pb, sb) = reference.frame_buffer();
            assert_eq!(sa, sb);
            assert_eq!(
                &pa[..sa.len() as usize],
                &pb[..sb.len() as usize],
                "{revision:?} pixels frame {frame}"
            );
        }
        assert!(checked.measurements.title_frame.is_some());
        assert!(checked.measurements.game_frame.is_some());
        let natural = serde_json::to_value(&checked.checks).unwrap();
        fs::write(
            evidence.join(format!("{key}-natural.json")),
            serde_json::to_vec_pretty(&natural).unwrap(),
        )
        .unwrap();
        println!(
            "NATURAL {} {}",
            revision.name(),
            serde_json::to_string(&checked.checks).unwrap()
        );
        fixture_coverage(&mut checked);
        assert_eq!(checked.checks.mismatches, 0);
        assert!(checked.checks.uninitialized_stack_reads.is_empty());
        // The two pre-existing pilot hooks are outside this batch and may need their existing fixtures.
        for translation in tje_game::sprites::TRANSLATIONS {
            if !checked
                .checks
                .by_function
                .contains_key(translation.id.metadata().name)
            {
                continue;
            }
            if [
                ids::UPDATE_ATTACHED_ENTITY_POSITION,
                ids::SELECT_SPRITE_CHAIN_START,
            ]
            .contains(&translation.id)
            {
                continue;
            }
            assert!(
                checked.checks.by_function[translation.id.metadata().name].calls_checked > 0,
                "unexercised {}",
                translation.id.metadata().name
            );
        }
        let result = serde_json::json!({"revision":revision.name(), "frames":frames, "title_frame":checked.measurements.title_frame,"menu_frame":checked.measurements.menu_frame,"game_frame":checked.measurements.game_frame,"logic_ticks":checked.measurements.logic_ticks,"whole_session_comparison":"RAM, VDP, writes, RNG draws, lag events, bitwise audio, pixels: exact every frame", "natural":natural,"with_live_fixtures":checked.checks});
        fs::write(
            evidence.join(format!("{key}-coverage.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        println!(
            "WITH_LIVE_FIXTURES {} {}",
            revision.name(),
            serde_json::to_string(&result).unwrap()
        );
    }
    let roots: Vec<_> = tje_game::sprites::TRANSLATIONS
        .iter()
        .map(|translation| translation.id)
        .collect();
    assert!(tje_game::Dispatcher::native(&roots).is_ok());
}
