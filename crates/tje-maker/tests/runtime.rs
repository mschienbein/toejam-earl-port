#![cfg(feature = "preview")]
//! Ignored private-ROM runs; no ROM, save, decoded resource or output is a public
//! fixture. Position injections below test transactions, not natural clear stamps.
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use sha2::{Digest, Sha256};
use tje_engine_cranelift::{CacheConfig, Engine, load_engine};
use tje_engine_runtime::enhanced::EnhancedMode;
use tje_maker::{
    enhanced::{MakerState, RunStatus, new_mode},
    *,
};
use tje_mem::{Revision, ram};
use tje_rom::Rom;
use tje_session::{CONNECTED, Session};

fn inputs(pads: [u16; 2]) -> GenesisInputs {
    let pads = pads.map(|bits| {
        let mut pad = if bits & CONNECTED != 0 {
            GenesisController::new(GenesisControllerType::ThreeButton)
        } else {
            GenesisController::None
        };
        for (index, button) in [
            GenesisButton::Up,
            GenesisButton::Down,
            GenesisButton::Left,
            GenesisButton::Right,
            GenesisButton::A,
            GenesisButton::B,
            GenesisButton::C,
            GenesisButton::Start,
        ]
        .into_iter()
        .enumerate()
        {
            pad.set_field(button, bits & (1 << index) != 0);
        }
        pad
    });
    GenesisInputs {
        p1: pads[0],
        p2: pads[1],
    }
}
fn idle() -> GenesisInputs {
    inputs([CONNECTED, CONNECTED])
}
fn press_up() -> GenesisInputs {
    inputs([CONNECTED | 1, CONNECTED])
}
fn settle(mode: &mut EnhancedMode, engine: &mut Engine) {
    // A video boundary may interrupt a compiled routine with old coordinates
    // already in its registers. Inject fixtures only before the next world
    // input root, not halfway through an original movement/writeback.
    let target = tje_game::ids::PROCESS_INPUT
        .metadata()
        .address(engine.devices().memory().revision());
    let deadline = engine.frame_count() + 240;
    while engine.cpu().pc != target {
        assert!(
            engine.frame_count() < deadline,
            "world input root not reached"
        );
        mode.step(engine, idle()).unwrap();
    }
}
fn position(mode: &mut EnhancedMode, engine: &mut Engine, x: i16, y: i16) {
    settle(mode, engine);
    let r = engine.devices().memory().revision();
    let memory = engine.devices_mut().memory_mut();
    ram::g_player_info(r).at(0).x().write(memory, x);
    ram::g_player_info(r).at(0).y().write(memory, y);
    ram::g_player_info(r).at(0).height().write(memory, 0);
}
fn travel_to(mode: &mut EnhancedMode, engine: &mut Engine, x: i16, target: i8) {
    let revision = engine.devices().memory().revision();
    // The original polls each physical pad on alternating VBlanks, then maps
    // input in a separate world phase. Do not inject a new fixture mid-travel
    // or assume a one-frame release was sampled by that original protocol.
    for _ in 0..16 {
        mode.play_frame(engine, idle()).unwrap();
    }
    position(mode, engine, x, 800);
    for _ in 0..120 {
        if ram::g_player_info(revision)
            .at(0)
            .level()
            .read(engine.devices().memory())
            == target
        {
            break;
        }
        mode.play_frame(engine, press_up()).unwrap();
    }
    assert_eq!(
        ram::g_player_info(revision)
            .at(0)
            .level()
            .read(engine.devices().memory()),
        target
    );
    for _ in 0..240 {
        if !mode
            .sidecars()
            .get::<MakerState>()
            .unwrap()
            .is_transitioning()
        {
            break;
        }
        mode.play_frame(engine, idle()).unwrap();
    }
    assert!(
        !mode
            .sidecars()
            .get::<MakerState>()
            .unwrap()
            .is_transitioning(),
        "travel did not publish camera/HUD boundary; pc={:#x}",
        engine.cpu().pc
    );
}
fn finish_at(mode: &mut EnhancedMode, engine: &mut Engine, x: i16) {
    for _ in 0..16 {
        mode.play_frame(engine, idle()).unwrap();
    }
    position(mode, engine, x, 800);
    for _ in 0..120 {
        if mode.sidecars().get::<MakerState>().unwrap().status() == RunStatus::Complete {
            break;
        }
        mode.play_frame(engine, press_up()).unwrap();
    }
    let r = engine.devices().memory().revision();
    let memory = engine.devices().memory();
    let player = ram::g_player_info(r).at(0);
    assert_eq!(
        mode.sidecars().get::<MakerState>().unwrap().status(),
        RunStatus::Complete,
        "pc={:#x} xyh={:?} sprite={} velocity_y={} inventory={} processed={:#x}",
        engine.cpu().pc,
        (
            player.x().read(memory),
            player.y().read(memory),
            player.height().read(memory)
        ),
        player.sprite_state().read(memory),
        player.velocity_y().read(memory),
        ram::g_player_inventory_open(r).at(0).read(memory),
        ram::g_player_processed_input(r).at(0).read(memory),
    );
}
fn exhaust_life(mode: &mut EnhancedMode, engine: &mut Engine, hero: i32) {
    settle(mode, engine);
    let r = engine.devices().memory().revision();
    let memory = engine.devices_mut().memory_mut();
    let player = ram::g_player_info(r).at(hero);
    ram::g_player_lives(r).at(hero).write(memory, 0);
    player.sprite_state().write(memory, 5);
    player.height().write(memory, 1000);
    player.velocity_height().write(memory, 0);
    player
        .behavior_flags()
        .write(memory, player.behavior_flags().read(memory) | 0x80);
    for _ in 0..240 {
        if ram::g_player_lives(r)
            .at(hero)
            .read(engine.devices().memory())
            < 0
            && !mode
                .sidecars()
                .get::<MakerState>()
                .unwrap()
                .is_transitioning()
        {
            break;
        }
        mode.play_frame(engine, idle()).unwrap();
    }
    assert_eq!(
        ram::g_player_lives(r)
            .at(hero)
            .read(engine.devices().memory()),
        -1
    );
    assert!(
        !mode
            .sidecars()
            .get::<MakerState>()
            .unwrap()
            .is_transitioning()
    );
}
fn digest_frames(mode: &mut EnhancedMode, engine: &mut Engine, frames: usize) -> (String, String) {
    let (mut video, mut audio) = (Sha256::new(), Sha256::new());
    for _ in 0..frames {
        mode.play_frame(engine, idle()).unwrap();
        let (pixels, size) = engine.frame_buffer();
        video.update(size.width.to_le_bytes());
        video.update(size.height.to_le_bytes());
        for pixel in &pixels[..size.len() as usize] {
            video.update([pixel.r, pixel.g, pixel.b, pixel.a]);
        }
        for sample in engine.devices().audio_samples() {
            for value in sample {
                audio.update(value.to_bits().to_le_bytes());
            }
        }
    }
    (
        format!("{:x}", video.finalize()),
        format!("{:x}", audio.finalize()),
    )
}

#[test]
#[ignore = "requires TJE_REV02_ROM and TJE_REV00_ROM; one game process, sequential engines"]
fn samples_load_authored_terrain_population_and_snapshot_transactions_in_both_revisions() {
    for (revision, key, catalog, session_file) in [
        (
            Revision::Rev02,
            "TJE_REV02_ROM",
            Catalog::Rev02,
            "rev02-one-player.tje.json",
        ),
        (
            Revision::Rev00,
            "TJE_REV00_ROM",
            Catalog::Rev00,
            "rev00-one-player.tje.json",
        ),
    ] {
        let bytes = std::fs::read(std::env::var(key).expect("private ROM path")).unwrap();
        let session = Session::from_json(
            &std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../sessions")
                    .join(session_file),
            )
            .unwrap(),
        )
        .unwrap();
        let pads: Vec<_> = session.inputs().collect();
        for (name, json) in [
            (
                "meadow",
                include_bytes!("../../../courses/meadow.tjemaker.json").as_slice(),
            ),
            (
                "two-gardens",
                include_bytes!("../../../courses/two-gardens.tjemaker.json").as_slice(),
            ),
            (
                "vanishing-square",
                include_bytes!("../../../courses/vanishing-square.tjemaker.json").as_slice(),
            ),
        ] {
            let mut authored = ValidatedCourse::from_json(json).unwrap().course().clone();
            // Explicit test retargeting, never an importer fallback. Each catalog
            // keeps a different body digest and must pass its own ROM admission.
            authored.catalog = catalog;
            let document = ValidatedCourse::new(authored).unwrap();
            let cache = CacheConfig {
                data_dir: std::env::temp_dir().join("tje-maker-private-cache"),
            };
            let mut engine = load_engine(
                Rom::import(&bytes).unwrap(),
                &GenesisEmulatorConfig::default(),
                &cache,
                |_| {},
            )
            .unwrap();
            let mut mode = new_mode(&document, engine.devices().memory()).unwrap();
            while mode.sidecars().get::<MakerState>().unwrap().status() != RunStatus::Playing {
                assert!(engine.frame_count() < 1200, "course did not finish setup");
                let index = engine.frame_count() as usize;
                mode.step(&mut engine, inputs(pads[index])).unwrap();
            }
            let memory = engine.devices().memory();
            let level = &document.course().levels[0];
            for y in 0..26 {
                for x in 0..19 {
                    assert_eq!(
                        ram::g_player1_level_map(revision).at(y).at(x).read(memory),
                        level.cell(x as usize, y as usize).unwrap()
                    );
                }
            }
            assert_eq!(
                ram::g_item_bank_users(revision).at(0).read(memory),
                document.course().rules.heroes as i8
            );
            let authored_items: Vec<_> = level
                .objects
                .iter()
                .filter(|object| matches!(object, Object::Item { .. }))
                .collect();
            for slot in 0..32 {
                let item = ram::g_object_table(revision).at(0).at(slot);
                if let Some(expected) = authored_items.get(slot as usize) {
                    assert_eq!(item.x().read(memory), expected.position().x as i16);
                    assert_eq!(item.y().read(memory), expected.position().y as i16);
                } else {
                    assert_eq!(item.kind().read(memory), -1, "stock item injection");
                }
            }
            let authored_actors: Vec<_> = level
                .objects
                .iter()
                .filter(|object| {
                    matches!(
                        object,
                        Object::Earthling { .. }
                            | Object::Service { .. }
                            | Object::ShipPiece { .. }
                    )
                })
                .collect();
            for slot in 0..20 {
                let actor = ram::g_earthling_banks(revision).at(0).at(slot);
                if let Some(expected) = authored_actors.get(slot as usize) {
                    assert_eq!(actor.x().read(memory), expected.position().x as i16);
                    assert_eq!(actor.y().read(memory), expected.position().y as i16);
                } else {
                    assert_eq!(
                        actor.kind().read(memory),
                        -1,
                        "stock Earthling/ship injection"
                    );
                }
            }
            let checkpoint = mode.snapshot(&engine).unwrap();
            let hashes = digest_frames(&mut mode, &mut engine, 120);
            let expected = mode.snapshot(&engine).unwrap();
            mode.restore(&mut engine, &checkpoint).unwrap();
            assert_eq!(digest_frames(&mut mode, &mut engine, 120), hashes);
            assert_eq!(
                mode.snapshot(&engine).unwrap().as_bytes(),
                expected.as_bytes()
            );
            let mut changed = document.course().clone();
            changed.title.push('!');
            let mut wrong_course = new_mode(
                &ValidatedCourse::new(changed).unwrap(),
                engine.devices().memory(),
            )
            .unwrap();
            let before = mode.snapshot(&engine).unwrap();
            assert!(wrong_course.restore(&mut engine, &checkpoint).is_err());
            assert_eq!(
                mode.snapshot(&engine).unwrap().as_bytes(),
                before.as_bytes()
            );
            if name == "two-gardens" {
                assert_eq!(
                    ram::g_telephone_states(revision)
                        .at(0)
                        .x()
                        .read(engine.devices().memory()),
                    1536
                );
                // Simulate the original phone's spent state, then prove a
                // revisit does not repopulate its static authored identity.
                settle(&mut mode, &mut engine);
                ram::g_telephone_states(revision)
                    .at(0)
                    .level()
                    .write(engine.devices_mut().memory_mut(), 0);
                travel_to(&mut mode, &mut engine, 896, 3);
                assert_eq!(
                    ram::g_player1_level_map(revision)
                        .at(0)
                        .at(0)
                        .read(engine.devices().memory()),
                    33
                );
                travel_to(&mut mode, &mut engine, 1280, 2);
                assert_eq!(
                    ram::g_telephone_states(revision)
                        .at(0)
                        .level()
                        .read(engine.devices().memory()),
                    0
                );
                travel_to(&mut mode, &mut engine, 896, 3);
                finish_at(&mut mode, &mut engine, 1792);
            }
            if name == "meadow" {
                position(&mut mode, &mut engine, 704, 800);
                for _ in 0..48 {
                    mode.play_frame(&mut engine, idle()).unwrap();
                }
                assert_ne!(
                    ram::g_collected_level_item_bits(revision)
                        .at(2)
                        .read(engine.devices().memory())
                        & 0x8000_0000,
                    0
                );
                finish_at(&mut mode, &mut engine, 1664);
                mode.restore(&mut engine, &checkpoint).unwrap();
                exhaust_life(&mut mode, &mut engine, 0);
                assert_eq!(
                    mode.sidecars().get::<MakerState>().unwrap().status(),
                    RunStatus::Playing
                );
                assert_eq!(
                    ram::g_earthling_bank_users(revision)
                        .at(0)
                        .read(engine.devices().memory()),
                    1
                );
                exhaust_life(&mut mode, &mut engine, 1);
                assert_eq!(
                    mode.sidecars().get::<MakerState>().unwrap().status(),
                    RunStatus::Failed
                );
            }
            if name == "vanishing-square" {
                position(&mut mode, &mut engine, 1088, 800);
                for _ in 0..150 {
                    mode.play_frame(&mut engine, idle()).unwrap();
                }
                assert_eq!(
                    ram::g_player1_level_map(revision)
                        .at(11)
                        .at(7)
                        .read(engine.devices().memory()),
                    34
                );
            }
            eprintln!(
                "{revision:?} sample={name} loaded_exact=true stock_population=false snapshot_resimulation=true video_sha256={} audio_sha256={}",
                hashes.0, hashes.1
            );
        }
    }
}
