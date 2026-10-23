//! Opt-in source-backed player-frame proof binary; no ROM data embedded.
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tje_game::{FunctionId, Translation, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Ptr, Revision, ram, rom};

static FRAME_PROOF: LazyLock<Vec<Translation>> = LazyLock::new(|| {
    let selection = std::env::var("PLAYER_FRAME_FUNCTIONS").unwrap_or_else(|_| {
        "PlayerProcessTerrainMotion,PlayerApplyDirectionalMovementAnimation".into()
    });
    let names: Vec<_> = selection.split(',').collect();
    let selected: Vec<_> = tje_game::player::PENDING_TRANSLATIONS
        .iter()
        .chain(tje_game::player::TRANSLATIONS.iter())
        .filter(|translation| names.contains(&translation.id.metadata().name))
        .copied()
        .collect();
    assert_eq!(
        selected.len(),
        names.len(),
        "unknown/duplicate frame proof selection"
    );
    selected
});

fn selected(id: FunctionId) -> bool {
    FRAME_PROOF.iter().any(|translation| translation.id == id)
}

fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| panic!("{} {arguments:08x?}: {error}", id.metadata().name));
}

fn input_script(path: &str) -> InputScript {
    let session: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(session["start"]["kind"], "power_on");
    let runs = session["runs"].as_array().unwrap();
    let connected = runs[0]["pads"][1].as_u64().unwrap() & 256 != 0;
    let mut start = 0;
    let mut holds = Vec::new();
    for run in runs {
        let end = start + run["frames"].as_u64().unwrap();
        let pads = [0, 1].map(|index| {
            let bits = run["pads"][index].as_u64().unwrap();
            assert_eq!(bits & 256 != 0, index == 0 || connected);
            ["up", "down", "left", "right", "a", "b", "c", "start"]
                .into_iter()
                .enumerate()
                .filter_map(|(bit, name)| (bits & (1 << bit) != 0).then_some(name))
                .collect::<Vec<_>>()
        });
        holds.push(serde_json::json!({"start":start,"end":end,"p1":pads[0],"p2":pads[1]}));
        start = end;
    }
    InputScript::parse(&serde_json::json!({"p2_connected":connected,"holds":holds}).to_string())
        .unwrap()
}

fn prepare(host: &mut Host, number: i32) {
    let r = host.revision();
    let p = ram::g_player_info(r).at(number);
    p.entity_index().write(&mut host.context(), number as i8);
    p.level().write(&mut host.context(), 1);
    p.x().write(&mut host.context(), 512);
    p.y().write(&mut host.context(), 512);
    p.height().write(&mut host.context(), 0);
    p.draw_flags().write(&mut host.context(), 0x31);
    p.behavior_flags().write(&mut host.context(), 1);
    p.velocity_x().write(&mut host.context(), 320);
    p.velocity_y().write(&mut host.context(), 240);
    p.acceleration_x().write(&mut host.context(), 0);
    p.acceleration_y().write(&mut host.context(), 0);
    p.sprite_state().write(&mut host.context(), 11);
    p.animation_group_mask().write(&mut host.context(), 4);
    p.frame().write(&mut host.context(), 1);
    p.frame_duration().write(&mut host.context(), 4);
    p.fraction_x().write(&mut host.context(), 250);
    p.fraction_y().write(&mut host.context(), 199);
    p.animation_motion_axes().write(&mut host.context(), 3);
    ram::g_player_lives(r)
        .at(number)
        .write(&mut host.context(), 2);
    ram::g_player_inventory_open(r)
        .at(number)
        .write(&mut host.context(), 0);
    ram::g_player_powerups(r)
        .at(number)
        .write(&mut host.context(), -1);
    ram::g_player_elevator_flags(r)
        .at(number)
        .write(&mut host.context(), 0);
    ram::g_player_terrain_sinking(r)
        .at(number)
        .write(&mut host.context(), 0);
    ram::g_player_control_remap_entities(r)
        .at(number)
        .write(&mut host.context(), 0);
    ram::g_player_interaction_choice_active(r)
        .at(number)
        .write(&mut host.context(), 0);
    ram::g_player_fall_states(r)
        .at(number)
        .write(&mut host.context(), 0);
    ram::g_player1_map_level(r).write(&mut host.context(), 1);
    ram::g_entity_draw_list_head(r).write(&mut host.context(), Ptr::new(0));
    let nearby = if number == 0 {
        ram::g_player1_nearby_terrain(r)
    } else {
        ram::g_player2_nearby_terrain(r)
    }
    .cast::<u8>();
    for cell in 0..25 {
        nearby.offset(cell).write(&mut host.context(), 0x10);
    }
}

fn terrain_fixtures(host: &mut Host) {
    let r = host.revision();
    for number in 0..2 {
        for state in [8_i8, 11, 16, 79, 78] {
            for axes in [0, 1, 2, 3] {
                prepare(host, number);
                let p = ram::g_player_info(r).at(number);
                let render = rom::player_animation_descriptors(r)
                    .at(11)
                    .frames()
                    .read(&host.context())
                    .target()
                    .read(&host.context());
                p.sprite_state().write(&mut host.context(), state);
                p.animation_group_mask()
                    .write(&mut host.context(), 1_u32 << ((state as u32) >> 2));
                p.animation_motion_axes().write(&mut host.context(), axes);
                check(
                    host,
                    ids::PLAYER_PROCESS_TERRAIN_MOTION,
                    &[number as u32, render.address()],
                );
            }
        }
        for terrain in [0x10, 0x80, 0x90] {
            prepare(host, number);
            let p = ram::g_player_info(r).at(number);
            let nearby = if number == 0 {
                ram::g_player1_nearby_terrain(r)
            } else {
                ram::g_player2_nearby_terrain(r)
            }
            .cast::<u8>();
            for cell in 0..25 {
                nearby.offset(cell).write(&mut host.context(), terrain);
            }
            let render = rom::player_animation_descriptors(r)
                .at(8)
                .frames()
                .read(&host.context())
                .target()
                .read(&host.context());
            p.velocity_x().write(&mut host.context(), -768);
            p.velocity_y().write(&mut host.context(), -576);
            check(
                host,
                ids::PLAYER_PROCESS_TERRAIN_MOTION,
                &[number as u32, render.address()],
            );
        }
        prepare(host, number);
        ram::g_player_inventory_open(r)
            .at(number)
            .write(&mut host.context(), 1);
        check(
            host,
            ids::PLAYER_PROCESS_TERRAIN_MOTION,
            &[number as u32, 0],
        );
    }
}

fn animation_fixtures(host: &mut Host) {
    let r = host.revision();
    for number in 0..2 {
        for state in [0_i8, 8, 12, 44, 60, 108] {
            for sprite in [-1_i16, 0, 8, 11] {
                for input in [0, 0x40, 0x48] {
                    prepare(host, number);
                    let p = ram::g_player_info(r).at(number);
                    p.sprite_state().write(&mut host.context(), state);
                    p.animation_group_mask()
                        .write(&mut host.context(), 1_u32 << ((state as u32) >> 2));
                    let raw_sprite = 0xa55a_0000 | u32::from(sprite as u16);
                    check(
                        host,
                        ids::PLAYER_APPLY_DIRECTIONAL_MOVEMENT_ANIMATION,
                        &[
                            p.address(),
                            number as u32,
                            input,
                            raw_sprite,
                            (-320_i16) as u32,
                            240,
                            3,
                        ],
                    );
                    p.frame_duration().write(&mut host.context(), 1);
                    check(
                        host,
                        ids::PLAYER_APPLY_DIRECTIONAL_MOVEMENT_ANIMATION,
                        &[
                            p.address(),
                            number as u32,
                            input,
                            raw_sprite,
                            (-320_i16) as u32,
                            240,
                            3,
                        ],
                    );
                }
            }
        }
    }
}

fn falling_fixtures(host: &mut Host) {
    let r = host.revision();
    for number in 0..2 {
        prepare(host, number);
    }
    for state in [0, 1, 2] {
        for number in 0..2 {
            ram::g_player_fall_states(r)
                .at(number)
                .write(&mut host.context(), state);
            ram::g_player_info(r)
                .at(number)
                .height()
                .write(&mut host.context(), 10);
            ram::g_player_info(r)
                .at(number)
                .velocity_height()
                .write(&mut host.context(), -1);
            ram::g_player_jump_counters(r)
                .at(number)
                .write(&mut host.context(), 2);
        }
        check(host, ids::CHECK_PLAYER_FALLING, &[]);
    }
}

fn main() {
    let out = PathBuf::from(std::env::args_os().nth(1).expect("evidence directory"));
    let frames: u64 = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "10000".into())
        .parse()
        .unwrap();
    fs::create_dir_all(&out).unwrap();
    for (revision, name) in [(Revision::Rev02, "rev02"), (Revision::Rev00, "rev00")] {
        let bytes = fs::read(format!("roms/{name}.bin")).unwrap();
        let path = format!("sessions/{name}-one-player.tje.json");
        let script = Arc::new(input_script(&path));
        let registry = HookRegistry::proof(revision, &FRAME_PROOF).unwrap();
        let mut checked = Host::new(&bytes, Mode::Check, registry.clone()).unwrap();
        let mut reference = Host::new(&bytes, Mode::Reference, registry.clone()).unwrap();
        checked.set_wait_input_poller(script.clone());
        reference.set_wait_input_poller(script.clone());
        let natural_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for frame in 0..frames {
                let a = checked.run_frame(script.at(frame)).unwrap();
                let b = reference.run_frame(script.at(frame)).unwrap();
                assert_eq!(a.ram, b.ram, "{name} RAM {frame}");
                assert_eq!(a.vdp, b.vdp);
                assert_eq!(a.writes, b.writes);
                assert_eq!(a.frame_counter, b.frame_counter);
                assert_eq!(a.idle_random_draws, b.idle_random_draws);
                assert_eq!(a.lag_events, b.lag_events);
                assert!(
                    a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
                        .audio_samples
                        .iter()
                        .flatten()
                        .map(|v| v.to_bits()))
                );
                let (pa, sa) = checked.frame_buffer();
                let (pb, sb) = reference.frame_buffer();
                assert_eq!(sa, sb);
                assert_eq!(&pa[..sa.len() as usize], &pb[..sb.len() as usize]);
            }
        }));
        let natural = serde_json::to_value(&checked.checks).unwrap();
        let mut live = Host::new(&bytes, Mode::Reference, registry).unwrap();
        let fixture_script = Arc::new(
            InputScript::parse(
                &fs::read_to_string("crates/tje-host/examples/start-game.json").unwrap(),
            )
            .unwrap(),
        );
        live.set_wait_input_poller(fixture_script.clone());
        for frame in 0..10000 {
            live.run_frame(fixture_script.at(frame)).unwrap();
        }
        let fixture_result = if natural_result.is_ok() {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if selected(ids::PLAYER_PROCESS_TERRAIN_MOTION) {
                    terrain_fixtures(&mut live);
                }
                if selected(ids::PLAYER_APPLY_DIRECTIONAL_MOVEMENT_ANIMATION) {
                    animation_fixtures(&mut live);
                }
                if selected(ids::CHECK_PLAYER_FALLING) {
                    falling_fixtures(&mut live);
                }
                for (id, number) in [
                    (ids::PLAYER1_PROCESS_TERRAIN_MOTION, 0),
                    (ids::PLAYER2_PROCESS_TERRAIN_MOTION, 1),
                ] {
                    if selected(id) {
                        prepare(&mut live, number);
                        let render = rom::player_animation_descriptors(revision)
                            .at(11)
                            .frames()
                            .read(&live.context())
                            .target()
                            .read(&live.context());
                        check(&mut live, id, &[render.address()]);
                    }
                }
            }))
        } else {
            natural_result
        };
        fs::write(out.join(format!("{name}-frame-proof.json")), serde_json::to_string_pretty(&serde_json::json!({"revision":name,"input":path,"frames":frames,"natural":natural,"fixtures":live.checks,"completed":fixture_result.is_ok()})).unwrap()).unwrap();
        if let Err(error) = fixture_result {
            std::panic::resume_unwind(error);
        }
        assert_eq!(checked.checks.mismatches, 0);
        assert_eq!(live.checks.mismatches, 0);
        for translation in FRAME_PROOF.iter() {
            let name = translation.id.metadata().name;
            assert!(
                checked.checks.by_function.get(name).unwrap().calls_checked
                    + live.checks.by_function.get(name).unwrap().calls_checked
                    > 0,
                "{name}: unexercised"
            );
        }
        println!(
            "PLAYER_FRAME_PROOF {name} natural={} fixtures={} mismatches=0",
            checked.checks.calls, live.checks.calls
        );
    }
}
