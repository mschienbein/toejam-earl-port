//! Opt-in replay-environment campaign; never part of production registration.
//! Compile with the frozen proof_replayed_environment API after Main announces it.
//! One natural campaign checks the selected live roots, then labelled live-state
//! fixtures. This is modular replay evidence, not live replacement/composition.
use std::{env, fs, path::PathBuf, sync::Arc};
use tje_game::{FunctionId, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Address, Ptr, RenderFrame, Revision, ram};

mod check_frames;

fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    if !host.checks.by_function.contains_key(id.metadata().name) {
        return;
    }
    if let Err(error) = host.check_fixture(id.metadata().address(host.revision()), arguments) {
        let evidence = PathBuf::from(env::var_os("TJE_SPRITE_EVIDENCE").unwrap());
        let key = match host.revision() {
            Revision::Rev02 => "rev02",
            Revision::Rev00 => "rev00",
        };
        let failure = serde_json::json!({
            "function":id.metadata().name, "arguments":arguments,
            "proof_envelope":"ReplayedEnvironment", "error":error.to_string(), "checks":host.checks,
        });
        fs::write(
            evidence.join(format!("{key}-replayed-fixture-failed.json")),
            serde_json::to_vec_pretty(&failure).unwrap(),
        )
        .unwrap();
        panic!("{} labelled replay fixture: {error}", id.metadata().name);
    }
}

fn queue_fixture(host: &mut Host, case: &str) {
    let revision = host.revision();
    let buffer = ram::g_sprite_tile_upload_buffer(revision).address();
    let header = Address::<RenderFrame>::new(buffer + 6200);
    let frames = buffer + 6100;
    let sequence = buffer + 6300;
    for number in 0..40 {
        ram::g_player_info(revision)
            .at(number)
            .draw_flags()
            .write(&mut host.context(), 0);
    }
    for slot in 0..4 {
        ram::g_speech_bubble_entities(revision)
            .at(slot)
            .write(&mut host.context(), 0);
    }
    ram::g_split_screen_enabled(revision).write(&mut host.context(), 0);
    ram::g_player_inventory_open(revision)
        .at(0)
        .write(&mut host.context(), 0);
    ram::g_player_inventory_open(revision)
        .at(1)
        .write(&mut host.context(), 0);
    ram::g_shared_tile_sprite_copies_enabled(revision).write(&mut host.context(), 0);
    ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut host.context(), 0);
    ram::g_sprite_tile_buffer_offset(revision).write(&mut host.context(), 0);
    ram::g_entity_draw_list_head(revision).write(&mut host.context(), Ptr::new(0));
    ram::g_inventory_draw_list_head(revision).write(&mut host.context(), Ptr::new(0));
    header.piece_count().write(&mut host.context(), 0);
    header.reuse_tiles().write(&mut host.context(), 0);
    host.context().write_u32(frames, header.address());
    host.context().write_u8(sequence, 0);
    host.context().write_u8(sequence + 1, 0);
    let entity = ram::g_player_info(revision).at(38);
    entity.draw_flags().write(&mut host.context(), 3);
    entity.behavior_flags().write(&mut host.context(), 0);
    entity.frame_duration().write(&mut host.context(), 0);
    entity.frame_phase().write(&mut host.context(), 5);
    entity.frame().write(&mut host.context(), -1);
    entity.sprite_ready().write(&mut host.context(), 0);
    entity.tile_count().write(&mut host.context(), 1);
    entity
        .sequence()
        .write(&mut host.context(), Ptr::new(sequence));
    entity.frames().write(&mut host.context(), Ptr::new(frames));
    entity
        .current_frame()
        .write(&mut host.context(), Ptr::new(header.address()));
    entity.primary_shadow().write(&mut host.context(), 0);
    entity.secondary_shadow().write(&mut host.context(), 0);
    entity
        .primary_sprites()
        .at(0)
        .write(&mut host.context(), 25);
    entity
        .secondary_sprites()
        .at(0)
        .write(&mut host.context(), 26);
    match case {
        "empty" | "busy-upload" => entity.draw_flags().write(&mut host.context(), 0),
        "initial-frame" => {}
        "duration-not-expired" => entity.frame_duration().write(&mut host.context(), 2),
        "duration-expired" | "loop" | "hold" | "reuse" => {
            entity.frame_duration().write(&mut host.context(), 1);
            entity.frame().write(&mut host.context(), 0);
            if case == "loop" {
                host.context().write_u8(sequence + 1, 0xff);
            } else if case == "hold" {
                host.context().write_u8(sequence + 1, 0xfe);
            } else if case == "reuse" {
                header.reuse_tiles().write(&mut host.context(), 1);
                entity.sprite_ready().write(&mut host.context(), 1);
            }
        }
        "both-views" => entity.draw_flags().write(&mut host.context(), 7),
        "shared-entity" => entity.behavior_flags().write(&mut host.context(), 4),
        "speech-entity" => ram::g_speech_bubble_entities(revision)
            .at(0)
            .write(&mut host.context(), 38),
        "inventory" => ram::g_player_inventory_open(revision)
            .at(0)
            .write(&mut host.context(), 1),
        "first-over-budget" => entity.tile_count().write(&mut host.context(), 255),
        "skip-over-budget" => {
            entity.tile_count().write(&mut host.context(), 112);
            let second = ram::g_player_info(revision).at(39);
            second.draw_flags().write(&mut host.context(), 3);
            second.behavior_flags().write(&mut host.context(), 0);
            second.frame_duration().write(&mut host.context(), 0);
            second.frame().write(&mut host.context(), -1);
            second.sprite_ready().write(&mut host.context(), 0);
            second.tile_count().write(&mut host.context(), 1);
            second
                .sequence()
                .write(&mut host.context(), Ptr::new(sequence));
            second.frames().write(&mut host.context(), Ptr::new(frames));
        }
        _ => panic!("unknown authored queue case {case}"),
    }
    // Busy uses the live VBlank/Upload path with a real, empty tile queue.
    // There is no fake wait callback, forced ready return or original-state copy.
    ram::g_sprite_graphics_upload_pending(revision)
        .write(&mut host.context(), u8::from(case == "busy-upload"));
    check(host, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[]);
}

fn main() {
    let root = PathBuf::from(env::var_os("TJE_PORT_ROOT").expect("TJE_PORT_ROOT"));
    let evidence = PathBuf::from(env::var_os("TJE_SPRITE_EVIDENCE").expect("TJE_SPRITE_EVIDENCE"));
    let frames: u64 = env::var("TJE_SPRITE_FRAMES")
        .map(|value| value.parse().unwrap())
        .unwrap_or(10_000);
    let include_confirmed = env::var_os("TJE_SPRITE_REPLAY_CONFIRMED_FRAMES").is_some();
    // Default excludes the three already-passed frame candidates: no duplicate
    // confirmation/corpus work. The opt-in exists only for an explicit new request.
    let functions: &[FunctionId] = if include_confirmed {
        &[
            ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
            ids::BUILD_VIEWPORT_SPRITE_LINKS,
            ids::ALLOCATE_SPRITE_SLOTS,
            ids::ALLOCATE_SPRITE_SLOT,
        ]
    } else {
        &[ids::UPDATE_AND_QUEUE_ENTITY_SPRITES]
    };
    let script = Arc::new(
        InputScript::parse(include_str!(
            "../../../tje-host/examples/moving-player.json"
        ))
        .unwrap(),
    );
    fs::create_dir_all(&evidence).unwrap();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = match revision {
            Revision::Rev02 => "rev02",
            Revision::Rev00 => "rev00",
        };
        let bytes = fs::read(root.join("roms").join(format!("{key}.bin"))).unwrap();
        let registry = HookRegistry::proof_replayed_environment(
            revision,
            tje_game::sprites::PENDING_TRANSLATIONS,
            tje_game::sprites::PENDING_FRAMED_TRANSLATIONS,
            functions,
        )
        .unwrap();
        let mut host = Host::new(&bytes, Mode::Check, registry).unwrap();
        host.set_wait_input_poller(script.clone());
        for frame in 0..frames {
            if let Err(error) = host.run_frame(script.at(frame)) {
                fs::write(evidence.join(format!("{key}-replayed-natural-failed.json")),
                    serde_json::to_vec_pretty(&serde_json::json!({"frame":frame,"error":error.to_string(),"checks":host.checks})).unwrap()).unwrap();
                panic!("{key} replayed natural frame{frame}: {error}");
            }
        }
        let natural = serde_json::to_value(&host.checks).unwrap();
        let mut fixtures = Vec::new();
        for case in [
            "empty",
            "busy-upload",
            "initial-frame",
            "duration-not-expired",
            "duration-expired",
            "loop",
            "hold",
            "reuse",
            "both-views",
            "shared-entity",
            "speech-entity",
            "inventory",
            "first-over-budget",
            "skip-over-budget",
        ] {
            let boundary = host
                .settle_fixture_boundary(script.at(host.frame_count()))
                .unwrap();
            let before = host.checks.by_function
                [ids::UPDATE_AND_QUEUE_ENTITY_SPRITES.metadata().name]
                .calls_checked;
            queue_fixture(&mut host, case);
            fixtures.push(serde_json::json!({"case":case,"calls_before":before,
                "calls_after":host.checks.by_function[ids::UPDATE_AND_QUEUE_ENTITY_SPRITES.metadata().name].calls_checked,
                "original_settle_ticks":boundary.ticks,"original_settle_master_clocks":boundary.master_clocks}));
            fs::write(
                evidence.join(format!("{key}-replayed-fixtures.json")),
                serde_json::to_vec_pretty(&fixtures).unwrap(),
            )
            .unwrap();
        }
        if include_confirmed {
            check_frames::fixtures(&mut host);
        }
        assert_eq!(host.checks.mismatches, 0);
        for id in functions {
            assert!(host.checks.by_function[id.metadata().name].calls_checked > 0);
        }
        let result = serde_json::json!({"revision":key,"requested_natural_frames":frames,
            "proof_envelope":"ReplayedEnvironment","live_replacement_eligibility":false,
            "composition_proof":false,"natural":natural,"labelled_fixtures":fixtures,"checks":host.checks});
        fs::write(
            evidence.join(format!("{key}-replayed-coverage.json")),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        println!("SPRITES_REPLAYED {result}");
    }
}
