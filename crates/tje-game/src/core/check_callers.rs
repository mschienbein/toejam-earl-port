//! Opt-in caller proof executable. Arguments: ROM root, output root, Jam input,
//! natural|fixtures, canonical function name|all, frames. Failed attempts remain
//! failed in the JSON and exit status; fixtures never erase natural failures.

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, LazyLock},
};
use tje_game::{Context, FunctionId, Translation, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Platform, Ptr, Revision, ram};

const ROOTS: &[FunctionId] = &[
    ids::RELOAD_ACTIVE_CAMERA_VIEWS,
    ids::WAIT_FOR_NEXT_V_BLANK,
    ids::ENTER_SPLIT_SCREEN,
    ids::ENTER_SHARED_SCREEN,
    ids::CENTER_SHARED_CAMERA_ON_PLAYER,
    ids::UPDATE_SCREEN_SPLIT_MODE,
    ids::CHECK_START_BUTTON,
    ids::PROCESS_INPUT,
    ids::HANDLE_V_BLANK,
    ids::JAM_OUT_V_BLANK,
    ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK,
    ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL,
    ids::JAM_OUT_SHOW_LEAD_PAIR_LAYOUT,
    ids::JAM_OUT_SHOW_FOUR_PAIR_LAYOUT,
    ids::JAM_OUT_SHOW_NINE_PAIR_LAYOUT,
    ids::JAM_OUT_CREATE_CHARACTER_COPIES,
    ids::START_JAM_OUT,
];

static BODIES: LazyLock<Vec<Translation>> = LazyLock::new(|| {
    ROOTS
        .iter()
        .map(|&id| {
            *tje_game::translations()
                .chain(tje_game::core::PENDING_TRANSLATIONS)
                .chain(tje_game::jamout::PENDING_TRANSLATIONS)
                .find(|entry| entry.id == id)
                .expect("complete caller body")
        })
        .collect()
});
static SINGLE_BODIES: LazyLock<BTreeMap<FunctionId, [Translation; 1]>> =
    LazyLock::new(|| BODIES.iter().map(|&body| (body.id, [body])).collect());

fn registry(revision: Revision, single: Option<FunctionId>) -> HookRegistry {
    let bodies = if let Some(id) = single {
        &SINGLE_BODIES[&id][..]
    } else {
        BODIES.as_slice()
    };
    HookRegistry::proof(revision, bodies).unwrap()
}

fn quiet(host: &mut Host) {
    let revision = host.revision();
    let mut c = host.context();
    let sr = c.status_register() | 0x0700;
    c.set_status_register(sr);
    ram::g_mode2_reg_softcopy(revision).write(&mut c, 0x20);
    c.write_vdp_control(0x8124);
    ram::g_v_blank_func(revision).write(&mut c, Ptr::new(0));
    ram::g_v_blank_graphics_enabled(revision).write(&mut c, 0);
    ram::g_h_blank_graphics_enabled(revision).write(&mut c, 0);
    ram::g_v_blank_graphics_paused(revision).write(&mut c, 1);
    ram::g_v_blank_audio_input_override(revision).write(&mut c, 0);
    ram::g_player_inventory_open(revision)
        .at(0)
        .write(&mut c, 0);
    ram::g_player_inventory_open(revision)
        .at(1)
        .write(&mut c, 0);
}

fn quiet_music(host: &mut Host) {
    let revision = host.revision();
    let mut c = host.context();
    ram::g_fm_channel_release_requested(revision).write(&mut c, 0);
    ram::g_music_track_reset_requested(revision).write(&mut c, 0);
    ram::g_pending_fm_effect(revision).write(&mut c, -1);
    ram::g_pending_psg_effect(revision).write(&mut c, -1);
    ram::g_pending_pcm_sample(revision).write(&mut c, 0);
    ram::g_psg_effect_status(revision).write(&mut c, 0);
    ram::g_pcm_effect_cooldown_ticks(revision).write(&mut c, 0);
    ram::g_pcm_blocks_sequencer(revision).write(&mut c, 0);
    ram::g_current_music_selection(revision).write(&mut c, 0);
    ram::g_requested_music_selection(revision).write(&mut c, 0);
    // The assembly's opaque scalar is explicitly a word, not the legacy byte view.
    ram::g_music_channel_scan_count_minus_one(revision)
        .cast::<i16>()
        .write(&mut c, 0);
    for channel in 0..6 {
        ram::g_music_channels(revision)
            .at(channel)
            .current_stream()
            .write(&mut c, Ptr::new(0));
        ram::g_music_fm_channel_owners(revision)
            .cast::<u32>()
            .offset(channel)
            .write(&mut c, 0);
    }
    // A real initial bus take keeps the existing idle Z80 state stable up to
    // the first controller read. No BUSACK or YM value is fabricated.
    tje_game::Platform::write_z80_bus_request(&mut c, 0x100);
}

fn attempt(
    host: &mut Host,
    rows: &mut Vec<serde_json::Value>,
    id: FunctionId,
    label: &str,
    arguments: &[u32],
) {
    let entry = id.metadata().address(host.revision());
    println!("FIXTURE {}: {label}", id.metadata().name);
    let result = host.check_fixture(entry, arguments);
    rows.push(serde_json::json!({"function":id.metadata().name,"coverage_source":"live-state fixture","label":label,"entry_pc":entry,"arguments":arguments,"ok":result.is_ok(),"error":result.err().map(|error|error.to_string())}));
}

fn fixtures(host: &mut Host) -> Vec<serde_json::Value> {
    let revision = host.revision();
    let mut rows = Vec::new();
    quiet(host);
    attempt(
        host,
        &mut rows,
        ids::WAIT_FOR_NEXT_V_BLANK,
        "display-off positive count",
        &[2],
    );
    ram::g_mode2_reg_softcopy(revision).write(&mut host.context(), 0x60);
    host.context().write_vdp_control(0x8164);
    let sr = host.context().status_register() & !0x0700;
    host.context().set_status_register(sr);
    for count in [0, 0xffff, 1, 2] {
        attempt(
            host,
            &mut rows,
            ids::WAIT_FOR_NEXT_V_BLANK,
            "counter-only real IRQ path after settled boundary",
            &[count],
        );
    }
    quiet(host);
    for (first, second, previous_first, previous_second) in [
        (0, 0, 0, 0),
        (0x80, 0, 0x80, 0),
        (0, 0x80, 0, 0x80),
        (0x80, 0x80, 0x80, 0),
    ] {
        let mut c = host.context();
        ram::g_player_processed_input(revision)
            .at(0)
            .write(&mut c, first);
        ram::g_player_processed_input(revision)
            .at(1)
            .write(&mut c, second);
        ram::g_player_previous_processed_input(revision)
            .at(0)
            .write(&mut c, previous_first);
        ram::g_player_previous_processed_input(revision)
            .at(1)
            .write(&mut c, previous_second);
        attempt(
            host,
            &mut rows,
            ids::CHECK_START_BUTTON,
            "no-new-press or held Start; IRQs masked",
            &[],
        );
    }
    ram::g_controller1_raw_input(revision).write(&mut host.context(), 0);
    ram::g_controller2_raw_input(revision).write(&mut host.context(), 0);
    attempt(
        host,
        &mut rows,
        ids::PROCESS_INPUT,
        "live living-player state, no controller buttons; display off and IRQs masked",
        &[],
    );
    // Exact framed constructor from the unmodified live allocation state.
    attempt(
        host,
        &mut rows,
        ids::JAM_OUT_CREATE_CHARACTER_COPIES,
        "LINK/MOVEM and true call sites; original live allocator state",
        &[],
    );
    let first_lives = ram::g_player_lives(revision).at(0).read(&host.context());
    let second_lives = ram::g_player2_lives(revision).read(&host.context());
    let level = ram::g_player1_map_level(revision).read(&host.context());
    ram::g_player_lives(revision)
        .at(0)
        .write(&mut host.context(), -1);
    ram::g_player2_lives(revision).write(&mut host.context(), -1);
    ram::g_player1_map_level(revision).write(&mut host.context(), 26);
    attempt(
        host,
        &mut rows,
        ids::PROCESS_INPUT,
        "both absent, final-level menu gate; IRQs masked",
        &[],
    );
    ram::g_player_lives(revision)
        .at(0)
        .write(&mut host.context(), first_lives);
    ram::g_player2_lives(revision).write(&mut host.context(), second_lives);
    ram::g_player1_map_level(revision).write(&mut host.context(), level);

    for split in [0, 1] {
        ram::g_split_screen_enabled(revision).write(&mut host.context(), split);
        attempt(
            host,
            &mut rows,
            ids::RELOAD_ACTIVE_CAMERA_VIEWS,
            "live pending camera coordinates; display off",
            &[],
        );
    }
    for player in 0..2 {
        ram::g_player_lives(revision)
            .at(player)
            .write(&mut host.context(), 3);
        let entity = ram::g_player_info(revision).at(player);
        entity.level().write(&mut host.context(), 1);
        entity
            .x()
            .write(&mut host.context(), (1200 + 40 * player) as i16);
        entity
            .y()
            .write(&mut host.context(), (800 + 40 * player) as i16);
        entity.height_pixels().write(&mut host.context(), 32);
        entity.height().write(&mut host.context(), 0);
    }
    for player in [0_i16, 1, -1, 512, 513] {
        attempt(
            host,
            &mut rows,
            ids::CENTER_SHARED_CAMERA_ON_PLAYER,
            "ASL.W object index, including negative and wrapped selectors; IRQs masked",
            &[i32::from(player) as u32],
        );
    }
    attempt(
        host,
        &mut rows,
        ids::ENTER_SPLIT_SCREEN,
        "display-off nonanimated split; IRQs masked",
        &[0],
    );
    attempt(
        host,
        &mut rows,
        ids::ENTER_SHARED_SCREEN,
        "display-off shared midpoint; IRQs masked",
        &[40, 40],
    );
    for force in [0, 1, 0x100] {
        attempt(
            host,
            &mut rows,
            ids::UPDATE_SCREEN_SPLIT_MODE,
            "close living players, display off; IRQs masked",
            &[force],
        );
    }

    for graphics in [0, 1] {
        ram::g_v_blank_graphics_enabled(revision).write(&mut host.context(), graphics);
        ram::g_v_blank_graphics_paused(revision).write(&mut host.context(), 1);
        ram::g_v_blank_func(revision).write(&mut host.context(), Ptr::new(0));
        ram::g_frame_counter(revision).write(&mut host.context(), u16::MAX);
        attempt(
            host,
            &mut rows,
            ids::HANDLE_V_BLANK,
            "ordinary counter/paused-graphics path, no audio; IRQs masked",
            &[],
        );
    }
    quiet(host);
    for slot in 0..9 {
        // Missing copies intentionally cover layout calls' zero-handle branch;
        // their ROM/tilemap upload still executes in full.
        ram::g_jam_toejam_entities(revision)
            .at(slot)
            .write(&mut host.context(), 0);
        ram::g_jam_earl_entities(revision)
            .at(slot)
            .write(&mut host.context(), 0);
    }
    for id in [
        ids::JAM_OUT_SHOW_LEAD_PAIR_LAYOUT,
        ids::JAM_OUT_SHOW_FOUR_PAIR_LAYOUT,
        ids::JAM_OUT_SHOW_NINE_PAIR_LAYOUT,
    ] {
        attempt(
            host,
            &mut rows,
            id,
            "full tilemap work with display off and IRQs masked; absent copy handles",
            &[],
        );
    }
    for phase in [i16::MIN, -1, 0, 1, 2, 3, 4, 5, i16::MAX] {
        for countdown in [0, 1, 2] {
            ram::g_jam_layout_phase(revision).write(&mut host.context(), phase);
            ram::g_jam_layout_countdown(revision).write(&mut host.context(), countdown);
            ram::g_jam_scroll_step(revision).write(&mut host.context(), -1);
            attempt(
                host,
                &mut rows,
                ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL,
                "phase/countdown boundary; display off and IRQs masked",
                &[0],
            );
        }
    }
    attempt(
        host,
        &mut rows,
        ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL,
        "full reset path, display off and IRQs masked",
        &[1],
    );

    for id in [ids::JAM_OUT_V_BLANK, ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK] {
        for player_selection in [0, 1] {
            for counter in [0, 1, u16::MAX] {
                quiet(host);
                quiet_music(host);
                let mut c = host.context();
                ram::g_player_selection(revision).write(&mut c, player_selection);
                ram::g_frame_counter(revision).write(&mut c, counter);
                ram::g_jam_player1_previous_input(revision).write(&mut c, 0);
                ram::g_jam_player2_previous_input(revision).write(&mut c, 0);
                ram::g_jam_audio_route1_pending_input(revision).write(&mut c, 0xff);
                ram::g_jam_audio_route2_pending_input(revision).write(&mut c, 0xff);
                ram::g_jam_selected_song(revision).write(&mut c, 0);
                ram::g_jam_song_cycle_index(revision).write(&mut c, 7);
                ram::g_jam_template_bank_cycle_index(revision).write(&mut c, 2);
                for index in [
                    ram::g_jam_audio_route1_button_a_sample_index(revision),
                    ram::g_jam_audio_route1_button_b_sample_index(revision),
                    ram::g_jam_audio_route1_button_c_sample_index(revision),
                    ram::g_jam_audio_route2_button_a_sample_index(revision),
                    ram::g_jam_audio_route2_button_b_sample_index(revision),
                    ram::g_jam_audio_route2_button_c_sample_index(revision),
                ] {
                    index.write(&mut c, 0);
                }
                attempt(
                    host,
                    &mut rows,
                    id,
                    "empty music streams, retained pending instruments, wrap/solo routing; IRQs masked",
                    &[],
                );
            }
        }
    }
    // Actual input timeline for the complete modal scene, not forced exit state.
    quiet(host);
    quiet_music(host);
    let frame = host.frame_count();
    let input = format!(
        "{{\"holds\":[{{\"start\":{},\"end\":{},\"p1\":[\"start\"]}}]}}",
        frame + 8,
        frame + 608
    );
    host.set_wait_input_poller(Arc::new(InputScript::parse(&input).unwrap()));
    let sr = host.context().status_register() & !0x0700;
    host.context().set_status_register(sr);
    attempt(
        host,
        &mut rows,
        ids::START_JAM_OUT,
        "complete no-track scene with recorded future Start hold; IRQs enabled",
        &[0],
    );
    rows
}

fn natural(
    revision: Revision,
    file: &str,
    bytes: &[u8],
    id: FunctionId,
    script: Arc<InputScript>,
    output: &Path,
    frames: u64,
) -> bool {
    let mut host = Host::new(bytes, Mode::Check, registry(revision, Some(id))).unwrap();
    let mut reference = Host::new(bytes, Mode::Reference, HookRegistry::empty(revision)).unwrap();
    host.set_wait_input_poller(script.clone());
    let mut failure = None;
    let mut completed = 0;
    for frame in 0..frames {
        let a = match host.run_frame(script.at(frame)) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let pc = host.cpu().pc();
                let sp = host.cpu().stack_pointer();
                let arguments = (0..u32::from(id.metadata().abi.argument_count))
                    .map(|index| host.context().read_u32(sp + 4 + index * 4))
                    .collect::<Vec<_>>();
                failure = Some(
                    serde_json::json!({"frame":frame,"entry_pc":pc,"entry_sp":sp,"arguments":arguments,"error":error.to_string()}),
                );
                break;
            }
        };
        let b = reference.run_frame(script.at(frame)).unwrap();
        assert_eq!(a.ram, b.ram);
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
        let (a, sa) = host.frame_buffer();
        let (b, sb) = reference.frame_buffer();
        assert_eq!(sa, sb);
        assert_eq!(&a[..sa.len() as usize], &b[..sb.len() as usize]);
        completed += 1;
    }
    let ok = failure.is_none();
    fs::write(output.join(format!("batch2-natural-{}-{file}.json", id.metadata().name)), serde_json::to_string_pretty(&serde_json::json!({"revision":revision.name(),"function":id.metadata().name,"completed_frames":completed,"requested_frames":frames,"failure":failure,"checks":host.checks,"measurements":host.measurements})).unwrap()).unwrap();
    println!(
        "{} {} {file} frames={completed}",
        if ok { "COMPLETED" } else { "BLOCKED" },
        id.metadata().name
    );
    ok
}

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    assert_eq!(arguments.len(), 6);
    let output = Path::new(&arguments[1]);
    let game = include_str!("../../../tje-host/examples/start-game.json");
    let split = include_str!("../../../tje-host/examples/timing-split.json");
    let jam = fs::read_to_string(&arguments[2]).unwrap();
    let selected = ROOTS
        .iter()
        .copied()
        .filter(|id| arguments[4] == "all" || id.metadata().name == arguments[4])
        .collect::<Vec<_>>();
    assert!(!selected.is_empty());
    let frames: u64 = arguments[5].parse().unwrap();
    let mut passed = true;
    for (file, revision) in [("rev02", Revision::Rev02), ("rev00", Revision::Rev00)] {
        let bytes = fs::read(Path::new(&arguments[0]).join(format!("{file}.bin"))).unwrap();
        if arguments[3] == "natural" {
            for &id in &selected {
                let script = if id.metadata().subsystem == "jamout" {
                    &jam
                } else if [
                    ids::ENTER_SPLIT_SCREEN,
                    ids::ENTER_SHARED_SCREEN,
                    ids::UPDATE_SCREEN_SPLIT_MODE,
                ]
                .contains(&id)
                {
                    split
                } else {
                    game
                };
                passed &= natural(
                    revision,
                    file,
                    &bytes,
                    id,
                    Arc::new(InputScript::parse(script).unwrap()),
                    output,
                    frames,
                );
            }
        } else {
            assert_eq!(arguments[3], "fixtures");
            assert_eq!(arguments[4], "all");
            let mut host = Host::new(&bytes, Mode::Reference, registry(revision, None)).unwrap();
            let script = Arc::new(InputScript::parse(game).unwrap());
            host.set_wait_input_poller(script.clone());
            for frame in 0..frames {
                host.run_frame(script.at(frame)).unwrap();
            }
            assert!(host.measurements.game_frame.is_some());
            let advance = host
                .settle_fixture_boundary(script.at(host.frame_count()))
                .unwrap();
            let rows = fixtures(&mut host);
            let ok = rows.iter().all(|row| row["ok"] == true);
            passed &= ok;
            fs::write(output.join(format!("batch2-fixtures-{file}.json")), serde_json::to_string_pretty(&serde_json::json!({"revision":revision.name(),"frames":frames,"settling_ticks":advance.ticks,"settling_master_clocks":advance.master_clocks,"coverage_source":"labeled live-state fixtures after standard gameplay and original settling","attempts":rows,"checks":host.checks,"measurements":host.measurements})).unwrap()).unwrap();
            println!(
                "{} caller fixtures {file}",
                if ok { "COMPLETED" } else { "BLOCKED" }
            );
        }
    }
    if !passed {
        std::process::exit(1);
    }
}
