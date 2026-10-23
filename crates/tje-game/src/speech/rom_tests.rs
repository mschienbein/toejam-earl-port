//! Opt-in host harness compiled by the lane's separate private Cargo manifest.
//! No emulator dependency enters the ROM-free tje-game crate.
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tje_game::{Context, FunctionId, Translation, ids, ship, speech};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Platform, Ptr, Revision, ram, rom};

fn registry(revision: Revision) -> HookRegistry {
    let pending = std::env::var("TJE_SHIP_PENDING").unwrap_or_default();
    if pending.is_empty() {
        return HookRegistry::filtered(revision, &["ship", "speech"], &[]).unwrap();
    }
    static SELECTED: LazyLock<Vec<Translation>> = LazyLock::new(|| {
        let pending = std::env::var("TJE_SHIP_PENDING").expect("explicit proof selection");
        ship::TRANSLATIONS
            .iter()
            .chain(speech::TRANSLATIONS)
            .chain(ship::PENDING_TRANSLATIONS)
            .chain(speech::PENDING_TRANSLATIONS)
            .filter(|translation| {
                pending
                    .split(',')
                    .any(|name| name == translation.id.metadata().name)
            })
            .copied()
            .collect()
    });
    let translations = &*SELECTED;
    assert_eq!(
        translations.len(),
        pending.split(',').count(),
        "explicit proof selection"
    );
    let mut registry = HookRegistry::proof(revision, translations).unwrap();
    for &translation in ship::TRANSLATIONS.iter().chain(speech::TRANSLATIONS) {
        if !translations
            .iter()
            .any(|selected| selected.id == translation.id)
        {
            registry
                .register_translation_with_execution(
                    translation,
                    tje_game::execution_class(translation.id).unwrap(),
                )
                .unwrap();
        }
    }
    registry
}

fn save(host: &Host, natural: &serde_json::Value) {
    let report = serde_json::json!({
        "revision": host.revision().name(),
        "frames": host.frame_count(),
        "title_frame": host.measurements.title_frame,
        "menu_frame": host.measurements.menu_frame,
        "game_frame": host.measurements.game_frame,
        "natural": natural,
        "fixture_irq_mode": std::env::var("TJE_SHIP_IRQ").unwrap_or_else(|_| "counter-only".into()),
        "with_live_fixtures": host.checks,
    });
    let dir = PathBuf::from(std::env::var_os("TJE_SHIP_SPEECH_EVIDENCE").unwrap());
    fs::write(
        dir.join(format!(
            "{}-{}.json",
            host.revision().name(),
            std::env::var("TJE_SHIP_LABEL").unwrap_or_else(|_| "proof".into())
        )),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

fn fixture(host: &mut Host, natural: &serde_json::Value, id: FunctionId, args: &[u32]) {
    host.check_fixture(id.metadata().address(host.revision()), args)
        .unwrap_or_else(|error| {
            save(host, natural);
            panic!(
                "{:?} {} {args:?}: {error}",
                host.revision(),
                id.metadata().name
            )
        });
}

fn fixtures(host: &mut Host, natural: &serde_json::Value) {
    let revision = host.revision();
    // Authored text and register/queue boundaries on live post-gameplay forks.
    // Each check_fixture restores its entry state; these are not natural calls.
    for text in [
        b"".as_slice(),
        b"abc",
        b"Hello world!",
        b"?\x01\x02\x03\x04\x05\x06",
    ] {
        let address = 0xfff000;
        {
            let mut c = host.context();
            for (index, &byte) in text.iter().chain([0].iter()).enumerate() {
                c.write_u8(address + index as u32, byte);
            }
            ram::g_sprite_tile_buffer_offset(revision).write(&mut c, 0);
            ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut c, 0);
        }
        fixture(
            host,
            natural,
            ids::QUEUE_SPEECH_TEXT_TILES,
            &[address, text.len() as u32, 0x8123],
        );
    }
    for world_type in [0, 1, -1] {
        let mut c = host.context();
        ram::g_world_type(revision).write(&mut c, world_type);
        for piece in 0..10 {
            ram::g_ship_piece_level_assignments(revision)
                .at(piece)
                .write(&mut c, 0);
        }
        drop(c);
        fixture(host, natural, ids::ASSIGN_SHIP_PIECE_LEVELS, &[]);
    }
    fixture(host, natural, ids::FILL_SPRITE_OPAQUE_PIXELS, &[1]);
    fixture(host, natural, ids::SET_NEXT_SPEECH_LIFETIME, &[0xff80]);
    fixture(host, natural, ids::IS_SPEECH_BUBBLE_ENTITY, &[6]);
    fixture(host, natural, ids::HAS_SPEAKER_SPEECH, &[0]);
}

#[test]
#[ignore = "requires private cartridges and evidence directory"]
fn ship_speech_live_session_and_fixtures_both_revisions() {
    let script = Arc::new(
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap(),
    );
    for (revision, key) in [
        (Revision::Rev02, "TJE_REV02_ROM"),
        (Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let rom = fs::read(std::env::var_os(key).unwrap()).unwrap();
        let mut checked = Host::new(&rom, Mode::Check, registry(revision)).unwrap();
        let mut reference = Host::new(&rom, Mode::Reference, registry(revision)).unwrap();
        checked.set_wait_input_poller(script.clone());
        for frame in 0..10_000 {
            let a = checked.run_frame(script.at(frame)).unwrap_or_else(|error| {
                let mut natural = serde_json::to_value(&checked.checks).unwrap();
                natural["failure"] = serde_json::json!(error.to_string());
                save(&checked, &natural);
                panic!("{revision:?} frame {frame}: {error}")
            });
            let b = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(a.ram, b.ram, "{revision:?} whole RAM frame {frame}");
            assert_eq!(a.vdp, b.vdp, "{revision:?} VDP frame {frame}");
            assert_eq!(
                a.writes, b.writes,
                "{revision:?} hardware writes frame {frame}"
            );
            assert_eq!(a.frame_counter, b.frame_counter);
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits())),
                "{revision:?} audio frame {frame}"
            );
            let (pa, sa) = checked.frame_buffer();
            let (pb, sb) = reference.frame_buffer();
            assert_eq!(sa, sb);
            assert_eq!(&pa[..sa.len() as usize], &pb[..sb.len() as usize]);
        }
        assert!(checked.measurements.game_frame.is_some());
        let mut natural = serde_json::to_value(&checked.checks).unwrap();
        natural["whole_session_reference_equal_frames"] = serde_json::json!(10_000);
        fixtures(&mut checked, &natural);
        for id in [
            ids::GENERATE_SPEECH,
            ids::CLEAR_SPEECH_BUBBLES_AND_REPLIES,
            ids::SHIP_UPDATE_PENDING_PIECE_INDICATOR,
            ids::SHOW_MISSING_SHIP_PIECE_SUMMARY,
            ids::RESTORE_GAMEPLAY_GRAPHICS,
            ids::ANIMATE_TRANSITION_BACKGROUND,
            ids::ANIMATE_SHIP_SCREEN_BACKGROUND,
            ids::INITIALIZE_SHIP_SCREEN_TILEMAP_AND_PALETTES,
            ids::SHOW_SHIP_PIECE_NAME,
            ids::WAIT_FOR_SHIP_SCREEN_ACKNOWLEDGEMENT,
            ids::START_DIALOGUE,
            ids::UPDATE_SPEECH_AND_DIALOGUE,
        ] {
            if !tje_game::translations().any(|translation| translation.id == id) {
                continue;
            }
            candidate_fixtures(&mut checked, id).unwrap_or_else(|error| {
                save(&checked, &natural);
                panic!("{revision:?} {}: {error}", id.metadata().name);
            });
        }
        save(&checked, &natural);
        assert_eq!(checked.checks.mismatches, 0);
        assert!(
            checked.checks.unexercised.is_empty(),
            "{:?}",
            checked.checks.unexercised
        );
    }
}

fn prepare_fixture_irqs(host: &mut Host) {
    // The original HandleVBlank runs in both modes. Counter-only explicitly
    // gates its graphics/audio/input callbacks; audio-input enables the real
    // sequencer/controller service. Neither mode is described as natural play.
    let revision = host.revision();
    let mut c = host.context();
    ram::g_mode2_reg_softcopy(revision).write(&mut c, 0x40);
    ram::g_v_blank_func(revision).write(&mut c, Ptr::new(0));
    ram::g_v_blank_graphics_enabled(revision).write(&mut c, 0);
    ram::g_h_blank_graphics_enabled(revision).write(&mut c, 0);
    ram::g_v_blank_graphics_paused(revision).write(&mut c, 1);
    ram::g_v_blank_audio_input_override(revision).write(&mut c, 0);
    if std::env::var("TJE_SHIP_IRQ").as_deref() == Ok("audio-input") {
        ram::g_v_blank_graphics_paused(revision).write(&mut c, 0);
    }
    ram::g_sprite_graphics_upload_pending(revision).write(&mut c, 0);
    let sr = c.status_register() & !0x0700;
    c.set_status_register(sr);
    c.write_vdp_control(0x8164);
}

fn candidate_fixtures(host: &mut Host, id: FunctionId) -> tje_host::Result<()> {
    let revision = host.revision();
    let check = |host: &mut Host, args: &[u32]| {
        host.check_fixture(id.metadata().address(revision), args)
            .map(|_| ())
    };
    match id {
        ids::GENERATE_SPEECH => {
            for (flags, state, replace) in [(1, 0, 0), (3, 117, 1), (3, 0, 0), (3, 0, 1), (5, 0, 1)]
            {
                let mut c = host.context();
                let player = ram::g_player_info(revision).at(0);
                player.draw_flags().write(&mut c, flags);
                player.sprite_state().write(&mut c, state);
                player.x().write(&mut c, 32760);
                player.y().write(&mut c, 32760);
                player.height().write(&mut c, -32760);
                ram::g_next_speech_lifetime_override(revision).write(&mut c, 0x01fe);
                ram::g_sprite_graphics_upload_pending(revision).write(&mut c, 0);
                ram::g_sprite_tile_buffer_offset(revision).write(&mut c, 0);
                ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut c, 0);
                c.write_u8(0xfff000, b'H');
                c.write_u8(0xfff001, b'i');
                c.write_u8(0xfff002, 0);
                drop(c);
                check(host, &[0, 0xfff000, replace])?;
            }
            // Explicitly exercise the real pending-upload wait with the
            // original graphics/audio/input VBlank service, not a ready flag.
            prepare_fixture_irqs(host);
            let mut c = host.context();
            ram::g_v_blank_graphics_enabled(revision).write(&mut c, 1);
            ram::g_v_blank_graphics_paused(revision).write(&mut c, 0);
            ram::g_inventory_redraw_phase(revision).write(&mut c, 0);
            ram::g_terrain_priority_update_pending(revision).write(&mut c, 0);
            ram::g_sprite_graphics_upload_pending(revision).write(&mut c, 1);
            ram::g_player_info(revision)
                .at(0)
                .draw_flags()
                .write(&mut c, 3);
            ram::g_player_info(revision)
                .at(0)
                .sprite_state()
                .write(&mut c, 0);
            drop(c);
            check(host, &[0, 0xfff000, 1])?;
        }
        ids::CLEAR_SPEECH_BUBBLES_AND_REPLIES => {
            check(host, &[])?;
            for slot in 0..4 {
                let mut c = host.context();
                ram::g_speech_bubble_entities(revision)
                    .at(slot)
                    .write(&mut c, (slot + 6) as i8);
                let bubble = ram::g_player_info(revision).at(slot + 6);
                bubble.draw_flags().write(&mut c, 1);
            }
            check(host, &[])?;
        }
        ids::SHIP_UPDATE_PENDING_PIECE_INDICATOR => {
            for (active, pending, split, y) in [
                (0, 0, 0, 224),
                (0, 1, 1, 224),
                (0, 1, 0, 224),
                (6, 0, 0, 224),
                (6, 0, 0, 223),
            ] {
                let mut c = host.context();
                ram::g_ship_piece_indicator_entity(revision).write(&mut c, active);
                ram::g_ship_screen_state(revision)
                    .pending_piece_notice_level()
                    .write(&mut c, pending);
                ram::g_split_screen_enabled(revision).write(&mut c, split);
                ram::g_camera1_pending_x(revision).write(&mut c, 32760);
                ram::g_camera1_pending_y(revision).write(&mut c, 0);
                ram::g_player_info(revision).at(6).y().write(&mut c, y);
                drop(c);
                check(host, &[])?;
            }
        }
        ids::ANIMATE_SHIP_SCREEN_BACKGROUND | ids::ANIMATE_TRANSITION_BACKGROUND => {
            prepare_fixture_irqs(host);
            for bits in 0..=255_u8 {
                let timer = bits as i8;
                let mut c = host.context();
                ram::g_ship_background_animation_timer(revision).write(&mut c, timer);
                ram::g_ship_screen_state(revision)
                    .background_frame()
                    .write(&mut c, 1);
                ram::g_sprite_tile_buffer_offset(revision).write(&mut c, 0);
                ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut c, 0);
                drop(c);
                check(
                    host,
                    &[if id == ids::ANIMATE_TRANSITION_BACKGROUND {
                        20
                    } else {
                        0
                    }],
                )?;
            }
            if id == ids::ANIMATE_SHIP_SCREEN_BACKGROUND {
                check(host, &[1])?;
            }
        }
        ids::INITIALIZE_SHIP_SCREEN_TILEMAP_AND_PALETTES => {
            prepare_fixture_irqs(host);
            // Initialization is called with the display blanked by ShowShipPieces.
            ram::g_mode2_reg_softcopy(revision).write(&mut host.context(), 0);
            check(host, &[])?;
        }
        ids::SHOW_SHIP_PIECE_NAME => {
            prepare_fixture_irqs(host);
            for piece in 0..10 {
                check(host, &[piece])?;
            }
            // Out-of-range switch uses incoming D3 as its pointer, not zero.
            let mut c = host.context();
            c.set_data_register(3, 0xfff000);
            c.write_u8(0xfff000, b'X');
            c.write_u8(0xfff001, 0);
            drop(c);
            check(host, &[u32::MAX])?;
            check(host, &[10])?;
        }
        ids::SHOW_MISSING_SHIP_PIECE_SUMMARY => {
            for player in 0..2 {
                for same_level in [false, true] {
                    for on_level in [false, true] {
                        let mut c = host.context();
                        ram::g_player_info(revision)
                            .at(player)
                            .level()
                            .write(&mut c, 2);
                        ram::g_player_info(revision)
                            .at(player ^ 1)
                            .level()
                            .write(&mut c, if same_level { 2 } else { 3 });
                        for piece in 0..10 {
                            ram::g_ship_piece_level_assignments(revision)
                                .at(piece)
                                .write(&mut c, if on_level { 2 } else { 0 });
                        }
                        drop(c);
                        check(host, &[player as u32])?;
                    }
                }
            }
        }
        ids::RESTORE_GAMEPLAY_GRAPHICS => {
            check(host, &[])?;
        }
        ids::WAIT_FOR_SHIP_SCREEN_ACKNOWLEDGEMENT => {
            prepare_fixture_irqs(host);
            for player in [0, 1, 2] {
                check(host, &[player])?;
            }
        }
        ids::START_DIALOGUE => {
            prepare_fixture_irqs(host);
            for dialogue in 0..58 {
                for speaker in [0, 1, 6] {
                    let mut c = host.context();
                    for slot in 0..2 {
                        ram::g_delayed_speech_reply_timers(revision)
                            .at(slot)
                            .write(&mut c, 0);
                    }
                    ram::g_player_inventory_open(revision)
                        .at(0)
                        .write(&mut c, 0);
                    ram::g_player_inventory_open(revision)
                        .at(1)
                        .write(&mut c, 0);
                    drop(c);
                    check(host, &[dialogue, speaker])?;
                }
            }
        }
        ids::UPDATE_SPEECH_AND_DIALOGUE => {
            prepare_fixture_irqs(host);
            for residual in [0, 1, 0x1234_5678] {
                prepare_speech_tick(host, residual);
                check(host, &[])?;
            }
            // Force each original omitted-speaker reunion call using source
            // RNG arithmetic, not a patched branch or a corrected argument.
            let reunion_seed = (1..100_000)
                .find(|&seed| {
                    let first = tje_game::map::park_miller(seed);
                    first as u16 & 0x1fff != 1 && tje_game::map::park_miller(first) as u16 & 3 == 0
                })
                .unwrap();
            for elevator_player in [-1, 0, 1] {
                for residual in [0, 1, 0x1234_5678] {
                    prepare_speech_tick(host, residual);
                    let mut c = host.context();
                    ram::g_rng_state(revision).write(&mut c, reunion_seed);
                    ram::g_reunion_dialogue_pending(revision).write(&mut c, 1);
                    if elevator_player >= 0 {
                        ram::g_player_elevator_flags(revision)
                            .at(elevator_player)
                            .write(&mut c, 1);
                    }
                    drop(c);
                    check(host, &[])?;
                }
            }
            let remap_seed = (1..100_000)
                .find(|&seed| {
                    let first = tje_game::map::park_miller(seed);
                    first as u16 & 0x1fff != 1
                        && tje_game::map::park_miller(first) as u16 & 0xff == 2
                })
                .unwrap();
            prepare_speech_tick(host, 1);
            ram::g_rng_state(revision).write(&mut host.context(), remap_seed);
            ram::g_player_control_remap_entities(revision)
                .at(0)
                .write(&mut host.context(), 6);
            check(host, &[])?;
            // Expiration and a delayed reply, including signed-byte lifetime
            // wrap and the unsigned frame's existing entity allocation.
            prepare_speech_tick(host, 0);
            for slot in 0..4 {
                let mut c = host.context();
                ram::g_speech_bubble_entities(revision)
                    .at(slot)
                    .write(&mut c, (slot + 6) as i8);
                ram::g_speech_bubble_lifetimes(revision)
                    .at(slot)
                    .write(&mut c, if slot == 0 { -128 } else { 0 });
                ram::g_player_info(revision)
                    .at(slot + 6)
                    .draw_flags()
                    .write(&mut c, 1);
            }
            ram::g_delayed_speech_reply_timers(revision)
                .at(0)
                .write(&mut host.context(), 1);
            ram::g_delayed_speech_reply_texts(revision)
                .at(0)
                .write(&mut host.context(), Ptr::new(0xfff000));
            ram::g_delayed_speech_reply_speakers(revision)
                .at(0)
                .write(&mut host.context(), 0);
            check(host, &[])?;
            let sneeze_seed = (1..100_000)
                .find(|&seed| tje_game::map::park_miller(seed) as u16 & 0x1fff == 1)
                .unwrap();
            prepare_speech_tick(host, 0);
            let mut c = host.context();
            let kind = (0..25)
                .find(|&kind| {
                    rom::earthling_definitions(revision)
                        .at(kind)
                        .behavior_flags()
                        .read(&c)
                        & 1
                        != 0
                })
                .unwrap();
            ram::g_rng_state(revision).write(&mut c, sneeze_seed);
            ram::g_earthling_bank_users(revision).at(0).write(&mut c, 1);
            let earthling = ram::g_earthling_banks(revision).at(0).at(0);
            earthling.kind().write(&mut c, kind as i8);
            earthling.object().write(&mut c, 6);
            earthling.state().write(&mut c, 2);
            ram::g_player_info(revision)
                .at(6)
                .draw_flags()
                .write(&mut c, 3);
            ram::g_dynamic_entity_slot_allocated(revision)
                .at(0)
                .write(&mut c, 1);
            drop(c);
            check(host, &[])?;
            for bits in 0..=255_u8 {
                prepare_speech_tick(host, 0);
                let mut c = host.context();
                ram::g_speech_bubble_entities(revision)
                    .at(0)
                    .write(&mut c, 6);
                ram::g_speech_bubble_lifetimes(revision)
                    .at(0)
                    .write(&mut c, bits as i8);
                let bubble = ram::g_player_info(revision).at(6);
                bubble.draw_flags().write(&mut c, 3);
                bubble.sprite_count().write(&mut c, 0);
                bubble.tile_count().write(&mut c, 0);
                drop(c);
                check(host, &[])?;
            }
        }
        _ => {
            check(host, &[])?;
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires private cartridges, pending selection, and evidence directory"]
fn ship_speech_live_state_candidates_both_revisions() {
    let selected = std::env::var("TJE_SHIP_PENDING").expect("explicit pending function selection");
    let script = Arc::new(
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap(),
    );
    let mut failures = Vec::new();
    for (revision, key) in [
        (Revision::Rev02, "TJE_REV02_ROM"),
        (Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let bytes = fs::read(std::env::var_os(key).unwrap()).unwrap();
        let mut host = Host::new(&bytes, Mode::Reference, registry(revision)).unwrap();
        host.set_wait_input_poller(script.clone());
        let frames = std::env::var("TJE_SHIP_FRAMES")
            .map(|value| value.parse::<u64>().unwrap())
            .unwrap_or(2000);
        for frame in 0..frames {
            host.run_frame(script.at(frame)).unwrap();
        }
        assert!(host.measurements.game_frame.is_some());
        for name in selected.split(',') {
            let id = tje_game::functions()
                .find(|function| function.name == name)
                .unwrap()
                .id;
            if let Err(error) = candidate_fixtures(&mut host, id) {
                eprintln!("{revision:?} {name}: {error}");
                failures.push(format!("{revision:?} {name}: {error}"));
            }
        }
        save(
            &host,
            &serde_json::json!({"kind":"reference-to-gameplay-then-labeled-live-state-fixtures"}),
        );
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

fn prepare_speech_tick(host: &mut Host, residual: u32) {
    let revision = host.revision();
    let mut c = host.context();
    c.set_data_register(2, residual);
    c.write_u8(0xfff000, b'X');
    c.write_u8(0xfff001, 0);
    ram::g_reunion_dialogue_pending(revision).write(&mut c, 0);
    ram::g_split_screen_enabled(revision).write(&mut c, 0);
    ram::g_rng_state(revision).write(&mut c, 1);
    for slot in 0..4 {
        ram::g_speech_bubble_entities(revision)
            .at(slot)
            .write(&mut c, 0);
    }
    for player in 0..2 {
        ram::g_delayed_speech_reply_timers(revision)
            .at(player)
            .write(&mut c, 0);
        ram::g_player_control_remap_entities(revision)
            .at(player)
            .write(&mut c, 0);
        ram::g_player_elevator_flags(revision)
            .at(player)
            .write(&mut c, 0);
        ram::g_player_inventory_open(revision)
            .at(player)
            .write(&mut c, 0);
        ram::g_player_lives(revision).at(player).write(&mut c, 1);
        let object = ram::g_player_info(revision).at(player);
        object.draw_flags().write(&mut c, 3);
        object.sprite_state().write(&mut c, 0);
        object.level().write(&mut c, 2);
        object.x().write(&mut c, 0);
        object.y().write(&mut c, 0);
    }
}
