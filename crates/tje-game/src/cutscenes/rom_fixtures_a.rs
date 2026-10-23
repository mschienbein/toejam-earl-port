//! Opt-in CutscenesA differential checks. Synthetic inputs fork live gameplay;
//! they are not claimed to be controller-generated scene events.

use std::{fs, path::PathBuf};
use tje_game::{FunctionId, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram};

const W2_ATOMIC: &[FunctionId] = &[
    ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR,
    ids::CUTSCENE_LOAD_SECOND_TEXT_LINE,
    ids::CUTSCENE_SET_FOREGROUND_ROCKETSHIP_POSE,
    ids::ENDING_CREATE_ACTIVE_PLAYER_ENTITIES,
    ids::ENDING_CREATE_ELEVATED_THREE_PIECE_COMPOSITE,
    ids::ENDING_CREATE_POSE_COMPOSITE,
    ids::ENDING_CREATE_RIGHT_OFFSET_COMPOSITE,
    ids::ENDING_CREATE_SIX_PIECE_COMPOSITE,
    ids::FUNKATRON_CREATE_SCENE_ENTITY,
    ids::INTRO_CREATE_COLLISION_SCENE_ENTITIES,
    ids::INTRO_CREATE_FALLING_ROCKETSHIP,
];

const W3_ATOMIC: &[FunctionId] = &[
    ids::CUTSCENE_RELEASE_FLAGGED_ENTITY_RANGE,
    ids::CUTSCENE_RELEASE_ROCKETSHIP_PAIR,
    ids::CUTSCENE_RELEASE_TEXT_ENTITIES,
    ids::ENDING_RELEASE_ELEVATED_THREE_PIECE_COMPOSITE,
    ids::ENDING_RELEASE_POSE_COMPOSITE,
    ids::ENDING_RELEASE_RIGHT_OFFSET_COMPOSITE,
    ids::ENDING_RELEASE_SIX_PIECE_COMPOSITE,
    ids::INTRO_RELEASE_COLLISION_SCENE_ENTITIES,
    ids::INTRO_RELEASE_FALLING_ROCKETSHIP,
];

fn wave() -> u8 {
    std::env::var("TJE_A_WAVE").map_or(1, |value| value.parse().unwrap())
}

fn selected() -> Vec<FunctionId> {
    let mut ids = vec![
        ids::CUTSCENE_DRAW_METATILE_BLOCK,
        ids::CUTSCENE_QUEUE_CENTERED_ORIGINAL_PALETTE_TEXT_LINE,
        ids::CUTSCENE_QUEUE_CENTERED_REMAPPED_TEXT_LINE,
    ];
    if wave() >= 2 {
        ids.extend_from_slice(W2_ATOMIC);
        ids.extend([
            ids::ENDING_DRAW_NINE_METATILE_BACKGROUND,
            ids::ENDING_FILL_REPEATED_METATILE_BACKGROUND,
        ]);
    }
    if wave() >= 3 {
        ids.extend_from_slice(W3_ATOMIC);
    }
    if wave() >= 4 {
        ids.extend([
            ids::CUTSCENE_LOAD_ORIGINAL_PALETTE_TEXT_LINE,
            ids::CUTSCENE_TEXT_LOAD,
        ]);
    }
    if std::env::var_os("TJE_A_INCLUDE_TIME").is_some() {
        if wave() >= 5 {
            ids.push(ids::FUNKATRON_UPDATE_SCENE_ENTITIES);
        }
        ids.extend([ids::CLEAR_LOWER_VRAM_TILEMAP, ids::CLEAR_UPPER_VRAM_TILEMAP]);
        if wave() >= 2 {
            ids.extend([
                ids::ANIMATE_TITLE_TRANSITION_BACKGROUND,
                ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
                ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
                ids::TITLE_DRAW_SELECTED_LOGO,
            ]);
        }
        if wave() >= 3 {
            ids.extend([
                ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
                ids::INITIALIZE_TITLE_TRANSITION_TILEMAP,
                ids::INTRO_RELEASE_FRAME_ENTITIES_AND_REFRESH,
            ]);
        }
        if wave() >= 4 {
            ids.push(ids::TITLE_SCREEN_VIDEO);
        }
        if wave() >= 5 {
            ids.push(ids::SEGA_LOGO_V_BLANK);
        }
        if wave() >= 7 {
            ids.push(ids::SEGA_AND_INTRO_VIDEO_ROUTINE);
        }
    }
    if std::env::var_os("TJE_A_EXCLUDE_CAPTIONS").is_some() {
        ids.retain(|id| {
            ![
                ids::CUTSCENE_LOAD_SECOND_TEXT_LINE,
                ids::CUTSCENE_LOAD_ORIGINAL_PALETTE_TEXT_LINE,
                ids::CUTSCENE_TEXT_LOAD,
            ]
            .contains(id)
        });
    }
    if std::env::var_os("TJE_A_TIME_ONLY").is_some() {
        ids.retain(|id| {
            tje_game::execution_class(*id) == Some(tje_game::execution::ExecutionClass::TimeAware)
        });
    }
    ids
}

fn registry(revision: Revision) -> HookRegistry {
    static OVERRIDES: std::sync::LazyLock<Vec<tje_game::Translation>> =
        std::sync::LazyLock::new(|| {
            let pending = tje_game::cutscenes::PENDING_TRANSLATIONS;
            let roots = selected();
            pending
                .iter()
                .copied()
                .chain(
                    tje_game::translations()
                        .filter(|entry| roots.contains(&entry.id))
                        .copied(),
                )
                .collect()
        });
    HookRegistry::proof_filtered(revision, &OVERRIDES, &selected()).unwrap()
}

fn emit(host: &Host, natural: &serde_json::Value, failure: Option<String>) {
    let evidence = serde_json::json!({
        "revision": host.revision().name(),
        "frames": host.frame_count(),
        "source": "start-game controller script then labelled live-state fixtures",
        "natural": natural,
        "with_labelled_live_state_fixtures": host.checks,
        "measurements": host.measurements,
        "failure": failure,
    });
    println!("CUTSCENES_A_COVERAGE {evidence}");
    if let Some(directory) = std::env::var_os("TJE_CUTSCENES_A_METRICS_DIR") {
        fs::write(
            PathBuf::from(directory).join(format!(
                "{}-wave{}.json",
                host.revision().name(),
                wave()
            )),
            serde_json::to_string_pretty(&evidence).unwrap(),
        )
        .unwrap();
    }
}

fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) -> Result<(), String> {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .map(|_| ())
        .map_err(|error| format!("{} {arguments:08x?}: {error}", id.metadata().name))
}

fn fixtures(host: &mut Host) -> Result<(), String> {
    let revision = host.revision();
    let selected = selected();
    if std::env::var_os("TJE_A_TIME_ONLY").is_some() {
        return time_fixtures(host, &selected);
    }
    let mut failures = Vec::new();
    macro_rules! case {
        ($host:expr, $id:expr, $arguments:expr $(,)?) => {
            if selected.contains(&$id) {
                if let Err(error) = check($host, $id, $arguments) {
                    failures.push(error);
                }
            }
        };
    }
    let text = ram::g_sprite_tile_upload_buffer(revision).address() + 0x1800;
    // Authored bytes include all signs, punctuation, glyph 27's remap, and '?'.
    let symbols = b"A?09z!.,- ";
    for index in 0..40_u32 {
        let byte = match index {
            10 => 0xff,
            11 => 0x80,
            _ => symbols[index as usize % symbols.len()],
        };
        host.context().write_u8(text + index, byte);
    }
    for length in [-1_i16, 0, 1, 2, 15, 31, 32, 33] {
        for id in [
            ids::CUTSCENE_QUEUE_CENTERED_ORIGINAL_PALETTE_TEXT_LINE,
            ids::CUTSCENE_QUEUE_CENTERED_REMAPPED_TEXT_LINE,
        ] {
            ram::g_sprite_tile_buffer_offset(revision).write(&mut host.context(), 0);
            ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut host.context(), 0);
            case!(host, id, &[text, length as i32 as u32, 0x1234]);
        }
    }
    // Authored control-code regressions: repeated controls 2/5 each emit two
    // VDP words per glyph and exceed an Atomic frozen FIFO. Functional must
    // retain every ordered write; controls are never stripped or clamped.
    let controls: &[&[u8]] = &[&[2, 2, 2, 2], &[5, 5, 5, 5], &[2, 3, 4, 5, 6, 2]];
    for pattern in controls {
        for (index, &byte) in pattern.iter().enumerate() {
            host.context().write_u8(text + index as u32, byte);
        }
        for id in [
            ids::CUTSCENE_QUEUE_CENTERED_ORIGINAL_PALETTE_TEXT_LINE,
            ids::CUTSCENE_QUEUE_CENTERED_REMAPPED_TEXT_LINE,
        ] {
            ram::g_sprite_tile_buffer_offset(revision).write(&mut host.context(), 0);
            ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut host.context(), 0);
            case!(host, id, &[text, pattern.len() as u32, 0x1234]);
        }
    }
    if wave() >= 2 {
        // Explicit pool flags exercise successful, partial and exhausted
        // allocation on live-state forks, never a mocked allocator.
        host.context().write_u8(text + 8, 0);
        for pool in [0, 1, 2] {
            prepare_pool(host, pool);
            for &id in W2_ATOMIC {
                if id == ids::CUTSCENE_SET_FOREGROUND_ROCKETSHIP_POSE {
                    continue;
                }
                let args: &[u32] = if id == ids::CUTSCENE_LOAD_SECOND_TEXT_LINE {
                    &[text, 0x8000, 0x7fff]
                } else if id == ids::FUNKATRON_CREATE_SCENE_ENTITY {
                    &[1, 6]
                } else {
                    &[0x8000, 0x7fff]
                };
                case!(host, id, args);
            }
            for (first, second) in [(-1, -1), (0, -1), (-1, 0), (0, 0)] {
                ram::g_player_lives(revision)
                    .at(0)
                    .write(&mut host.context(), first);
                ram::g_player2_lives(revision).write(&mut host.context(), second);
                case!(
                    host,
                    ids::ENDING_CREATE_ACTIVE_PLAYER_ENTITIES,
                    &[0xffff, 0x8000]
                );
            }
            for descriptor in [0, 6, 13, 29] {
                case!(host, ids::FUNKATRON_CREATE_SCENE_ENTITY, &[0, descriptor]);
            }
        }
        prepare_pool(host, 0);
        // Both descriptor*18 and placement*6 use word-wrapped effective offsets.
        for index in [i16::MIN, -1, 0x3fff, 0x4000, i16::MAX] {
            case!(
                host,
                ids::FUNKATRON_CREATE_SCENE_ENTITY,
                &[0, index as i32 as u32]
            );
            case!(
                host,
                ids::FUNKATRON_CREATE_SCENE_ENTITY,
                &[index as i32 as u32, 0]
            );
        }
        ram::g_cutscene_rocketship_entity(revision).write(&mut host.context(), 6);
        ram::g_cutscene_rocketship_exhaust_entity(revision).write(&mut host.context(), 7);
        for flags in [1, 3, 5, 7] {
            for (handle, sprites, tiles) in [(6, 8, 101), (7, 2, 15)] {
                let object = ram::g_player_info(revision).at(handle);
                object.draw_flags().write(&mut host.context(), flags);
                object.behavior_flags().write(&mut host.context(), 0);
                object.tile().write(&mut host.context(), 640);
                object.tile_count().write(&mut host.context(), tiles);
                object.sprite_count().write(&mut host.context(), sprites);
                object.primary_shadow().write(&mut host.context(), 0);
                object.secondary_shadow().write(&mut host.context(), 0);
                for slot in 0..12 {
                    object
                        .primary_sprites()
                        .at(slot)
                        .write(&mut host.context(), 25 + slot as i8);
                    object
                        .secondary_sprites()
                        .at(slot)
                        .write(&mut host.context(), 25 + slot as i8);
                }
            }
            for animation in [-1_i16, 0, 3, i16::MIN, i16::MAX] {
                case!(
                    host,
                    ids::CUTSCENE_SET_FOREGROUND_ROCKETSHIP_POSE,
                    &[animation as i32 as u32, 0x8000, 0xffff]
                );
            }
        }
        // Allocation-failure ORIGINAL BUG: a shared-tile entity with no view
        // frees the tile carried in the compiled caller's D4 low word.
        prepare_pool(host, 0);
        for index in 0..54 {
            ram::g_viewport_sprite_slot_flags(revision)
                .at(index)
                .write(&mut host.context(), -1);
        }
        let ship = ram::g_player_info(revision).at(6);
        ship.draw_flags().write(&mut host.context(), 1);
        ship.behavior_flags().write(&mut host.context(), 4);
        ship.sprite_count().write(&mut host.context(), 1);
        ship.tile_count().write(&mut host.context(), 8);
        ship.tile().write(&mut host.context(), 640);
        ram::g_player_info(revision)
            .at(7)
            .draw_flags()
            .write(&mut host.context(), 0);
        ram::g_sprite_tile_group_allocated(revision)
            .at(0)
            .write(&mut host.context(), 1);
        tje_game::Context::set_data_register(&mut host.context(), 4, 0x1234_0000);
        case!(
            host,
            ids::CUTSCENE_SET_FOREGROUND_ROCKETSHIP_POSE,
            &[0, 0, 640]
        );

        // Exactly the parent's twelve slots are free; each shared attached
        // allocation then takes the residual-D4 failure path.
        prepare_pool(host, 0);
        for index in 12..54 {
            ram::g_viewport_sprite_slot_flags(revision)
                .at(index)
                .write(&mut host.context(), -1);
        }
        tje_game::Context::set_data_register(&mut host.context(), 4, 0x5678_0000);
        case!(host, ids::INTRO_CREATE_COLLISION_SCENE_ENTITIES, &[]);
        case!(host, ids::ENDING_DRAW_NINE_METATILE_BACKGROUND, &[]);
        case!(host, ids::ENDING_FILL_REPEATED_METATILE_BACKGROUND, &[]);
    }
    if wave() >= 3 {
        for flags in [0, 1, 3, 5, 7] {
            prepare_pool(host, 1);
            for number in 0..40 {
                let object = ram::g_player_info(revision).at(number);
                object.draw_flags().write(&mut host.context(), flags);
                object.behavior_flags().write(&mut host.context(), 0);
                object.sprite_count().write(&mut host.context(), 1);
                object.tile_count().write(&mut host.context(), 8);
                object.tile().write(&mut host.context(), 640);
                object.primary_shadow().write(&mut host.context(), 0);
                object.secondary_shadow().write(&mut host.context(), 0);
                object
                    .primary_sprites()
                    .at(0)
                    .write(&mut host.context(), 25);
                object
                    .secondary_sprites()
                    .at(0)
                    .write(&mut host.context(), 25);
            }
            for handle in [0, 6, 7] {
                for slot in [
                    ram::g_ending_pose_composite_entity(revision),
                    ram::g_ending_six_piece_composite_entity(revision),
                    ram::g_ending_right_offset_composite_entity(revision),
                    ram::g_ending_elevated_three_piece_composite_entity(revision),
                    ram::g_intro_falling_rocketship_entity(revision),
                    ram::g_cutscene_rocketship_entity(revision),
                    ram::g_cutscene_rocketship_exhaust_entity(revision),
                    ram::g_intro_collision_parent_entity(revision),
                ] {
                    slot.write(&mut host.context(), handle);
                }
                ram::g_cutscene_first_text_entity(revision)
                    .write(&mut host.context(), handle as i8);
                ram::g_cutscene_second_text_entity(revision)
                    .write(&mut host.context(), handle as i8);
                for index in 0..3 {
                    ram::g_intro_collision_auxiliary_entities(revision)
                        .at(index)
                        .write(&mut host.context(), handle);
                    ram::g_intro_collision_attached_entities(revision)
                        .at(index)
                        .write(&mut host.context(), handle);
                }
                for &id in W3_ATOMIC {
                    case!(host, id, &[]);
                }
            }
            ram::g_cutscene_first_text_entity(revision).write(&mut host.context(), -1);
            ram::g_cutscene_second_text_entity(revision).write(&mut host.context(), 7);
            case!(host, ids::CUTSCENE_RELEASE_TEXT_ENTITIES, &[]);
        }
    }
    if wave() >= 4 {
        for pool in [0, 1, 2] {
            prepare_pool(host, pool);
            ram::g_cutscene_first_text_entity(revision).write(&mut host.context(), 0);
            ram::g_cutscene_second_text_entity(revision).write(&mut host.context(), 0);
            for id in [
                ids::CUTSCENE_LOAD_ORIGINAL_PALETTE_TEXT_LINE,
                ids::CUTSCENE_TEXT_LOAD,
            ] {
                case!(host, id, &[text, 0xffff, 0x8000]);
            }
        }
    }
    if wave() >= 5 {
        prepare_pool(host, 0);
        ram::g_player1_map_level(revision).write(&mut host.context(), 26);
        for index in 0..277 {
            ram::g_funkatron_placement_entities(revision)
                .at(index)
                .write(&mut host.context(), 0);
        }
        for phase in 0..4 {
            ram::g_funkatron_update_phase(revision).write(&mut host.context(), phase);
            for selection in [0, 1, 2] {
                ram::g_player_selection(revision).write(&mut host.context(), selection);
                for (x, y) in [(0, 0), (300, 200), (1600, 800), (i16::MIN, i16::MAX)] {
                    ram::g_camera1_pending_x(revision).write(&mut host.context(), x);
                    ram::g_camera1_pending_y(revision).write(&mut host.context(), y);
                    case!(host, ids::FUNKATRON_UPDATE_SCENE_ENTITIES, &[]);
                }
            }
        }
    }
    for metatile in [0, 1, 7, 30, 82, 0x80, 0xffff] {
        case!(host, ids::CUTSCENE_DRAW_METATILE_BLOCK, &[metatile, 1, 2]);
    }
    if std::env::var_os("TJE_A_INCLUDE_TIME").is_some() {
        case!(host, ids::CLEAR_LOWER_VRAM_TILEMAP, &[]);
        case!(host, ids::CLEAR_UPPER_VRAM_TILEMAP, &[]);
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn time_fixtures(host: &mut Host, selected: &[FunctionId]) -> Result<(), String> {
    let revision = host.revision();
    let mut failures = Vec::new();
    for &id in selected {
        ram::g_sprite_tile_buffer_offset(revision).write(&mut host.context(), 0);
        ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut host.context(), 0);
        ram::g_scene_background_pair_index(revision).write(&mut host.context(), 0);
        ram::g_scene_background_frame_timer(revision).write(&mut host.context(), 8);
        ram::g_scene_background_frame_phase(revision).write(&mut host.context(), 1);
        let arguments: &[u32] = if id == ids::ANIMATE_TITLE_TRANSITION_BACKGROUND {
            &[0]
        } else if id == ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER {
            &[1, 1, 6, 4]
        } else if id == ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER {
            &[0]
        } else if id == ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE {
            &[0, 0]
        } else {
            &[]
        };
        if let Err(error) = check(host, id, arguments) {
            failures.push(error);
        }
        if id == ids::ANIMATE_TITLE_TRANSITION_BACKGROUND {
            if let Err(error) = check(host, id, &[1]) {
                failures.push(error);
            }
        }
        if id == ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER {
            if let Err(error) = check(host, id, &[1]) {
                failures.push(error);
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn prepare_pool(host: &mut Host, state: u8) {
    let revision = host.revision();
    ram::g_player1_map_level(revision).write(&mut host.context(), 26);
    for index in 0..34 {
        let allocated = state == 1 || (state == 2 && index != 0);
        ram::g_dynamic_entity_slot_allocated(revision)
            .at(index)
            .write(&mut host.context(), i8::from(allocated));
    }
    for index in 0..54 {
        ram::g_viewport_sprite_slot_flags(revision)
            .at(index)
            .write(&mut host.context(), 0);
    }
    for index in 0..80 {
        ram::g_sprite_tile_group_allocated(revision)
            .at(index)
            .write(&mut host.context(), 0);
    }
    ram::g_sprite_tile_buffer_offset(revision).write(&mut host.context(), 0);
    ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut host.context(), 0);
}

#[test]
#[ignore = "requires both private retail ROMs; 10,000 frames and live-state fixtures"]
fn lane_a_both_revisions() {
    let mut script =
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap();
    if std::env::var_os("TJE_A_INCLUDE_TIME").is_some() {
        // Labelled future controller inputs let an attract/title invocation
        // eventually exit; they do not alter the first 10,000 natural frames.
        for start in (10_020..20_000).step_by(160) {
            script.holds.push(tje_host::input::InputHold {
                start,
                end: start + 8,
                p1: vec![tje_host::input::Button::Start],
                p2: Vec::new(),
            });
        }
    }
    let script = std::sync::Arc::new(script);
    let mut failures = Vec::new();
    'revisions: for (revision, key) in [
        (Revision::Rev02, "TJE_REV02_ROM"),
        (Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let rom = fs::read(std::env::var_os(key).unwrap_or_else(|| panic!("set {key}"))).unwrap();
        let mode = if std::env::var_os("TJE_A_REFERENCE_ONLY").is_some() {
            Mode::Reference
        } else {
            Mode::Check
        };
        let mut checked = Host::new(&rom, mode, registry(revision)).unwrap();
        checked.set_wait_input_poller(script.clone());
        let mut reference =
            Host::new(&rom, Mode::Reference, HookRegistry::empty(revision)).unwrap();
        reference.set_wait_input_poller(script.clone());
        for frame in 0..10_000 {
            let a = match checked.run_frame(script.at(frame)) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    emit(&checked, &serde_json::Value::Null, Some(error.to_string()));
                    failures.push(format!("{revision:?} frame {frame}: {error}"));
                    continue 'revisions;
                }
            };
            let b = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(a.ram, b.ram, "{revision:?} RAM frame {frame}");
            assert_eq!(a.vdp, b.vdp, "{revision:?} VDP frame {frame}");
            assert_eq!(a.writes, b.writes, "{revision:?} writes frame {frame}");
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits()))
            );
        }
        assert!(checked.measurements.game_frame.is_some());
        let mut natural = serde_json::to_value(&checked.checks).unwrap();
        let boundary = checked
            .settle_fixture_boundary(script.at(checked.frame_count()))
            .unwrap();
        let reference_boundary = reference
            .settle_fixture_boundary(script.at(reference.frame_count()))
            .unwrap();
        assert_eq!(boundary.ticks, reference_boundary.ticks);
        assert_eq!(boundary.master_clocks, reference_boundary.master_clocks);
        assert_eq!(boundary.end_pc, reference_boundary.end_pc);
        assert_eq!(boundary.end_frame, reference_boundary.end_frame);
        assert_eq!(
            boundary.handlers_completed,
            reference_boundary.handlers_completed
        );
        assert_eq!(
            boundary.completed_frames.len(),
            reference_boundary.completed_frames.len()
        );
        for (a, b) in boundary
            .completed_frames
            .iter()
            .zip(&reference_boundary.completed_frames)
        {
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
        }
        natural["settled_boundary"] = serde_json::json!({
            "ticks": boundary.ticks, "master_clocks": boundary.master_clocks,
            "start_pc": boundary.start_pc, "end_pc": boundary.end_pc,
            "start_frame": boundary.start_frame, "end_frame": boundary.end_frame,
            "handlers_completed": boundary.handlers_completed,
            "completed_snapshots_compared": boundary.completed_frames.len(),
        });
        let result = fixtures(&mut checked);
        emit(&checked, &natural, result.as_ref().err().cloned());
        if let Err(error) = result {
            failures.push(format!("{revision:?}: {error}"));
            continue;
        }
        assert_eq!(checked.checks.mismatches, 0);
        assert!(checked.checks.uninitialized_stack_reads.is_empty());
        assert!(checked.checks.unexercised.is_empty());
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
