//! Opt-in composed proof for held Earthlings entries after the host contract bundle.
//! The nearest-target dependent-residual body stays pending and is never a root.
#[path = "live_fixtures.rs"]
mod previous;

use std::sync::Arc;
use tje_game::{FunctionId, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram};

const ROOTS: &[FunctionId] = &[
    ids::AI_GENERIC_ENEMY_THINK,
    ids::AI_OPERA_SINGER_TOUCH,
    ids::AI_WIZARD_TOUCH,
    ids::EARTHLING_DISPATCH_ABILITIES,
    ids::PLAYER_PROCESS_LIFE_REQUEST,
    ids::AI_MAILBOX_TOUCH,
    ids::AI_WISEMAN_TOUCH,
    ids::EARTHLING_DISPATCH_PLAYER_CONTACTS,
    ids::PROCESS_EARTHLINGS,
    ids::FIND_SPAWN_LOCATION_FOR_EARTHLING,
    ids::EARTHLING_RETAIN_LEVEL_BANK,
    ids::OPEN_EARTHLING_PRESENT,
];

fn fixture(host: &mut Host, id: FunctionId, args: &[u32]) {
    host.check_fixture(id.metadata().address(host.revision()), args)
        .unwrap_or_else(|error| {
            panic!(
                "{:?} {} {args:08x?}: {error}",
                host.revision(),
                id.metadata().name
            )
        });
}

fn frame_fixtures(host: &mut Host) {
    let revision = host.revision();
    for bank in 0..2 {
        for slot in 0..20 {
            let record = ram::g_earthling_banks(revision).at(bank).at(slot);
            record.kind().write(&mut host.context(), -1);
            record.object().write(&mut host.context(), 0);
            record.second_object().write(&mut host.context(), 0);
        }
    }
    for kind in [0, 1, 8, 13, 14, 18, 19] {
        fixture(
            host,
            ids::FIND_SPAWN_LOCATION_FOR_EARTHLING,
            &[kind, 0, 0xfff000, 0xfff002, 1],
        );
    }
    for level in [0, 1, 2, 26] {
        ram::g_earthling_bank_users(revision)
            .at(0)
            .write(&mut host.context(), 0);
        ram::g_earthling_bank_users(revision)
            .at(1)
            .write(&mut host.context(), 0);
        for piece in 0..10 {
            ram::g_ship_piece_level_assignments(revision)
                .at(piece)
                .write(&mut host.context(), -1);
        }
        if level < 26 {
            ram::g_earthling_spawn_failure_masks(revision)
                .at(level)
                .write(&mut host.context(), 0);
        }
        fixture(host, ids::EARTHLING_RETAIN_LEVEL_BANK, &[level as u32]);
    }
}

fn friendly_fixtures(host: &mut Host, selected: Option<&str>) {
    let revision = host.revision();
    // Use an actually allocated original object, so the speech allocation cannot
    // legitimately reuse the synthetic Earthling's otherwise-free handle.
    let handle = host
        .context()
        .call_original(ids::ALLOCATE_ENTITY.metadata().address(revision), &[])
        .unwrap() as i8;
    assert!(handle >= 6);
    let object = ram::g_player_info(revision).at(i32::from(handle));
    let record = ram::g_earthling_banks(revision).at(0).at(0);
    for number in 0..2 {
        let player = ram::g_player_info(revision).at(number);
        player.x().write(&mut host.context(), 512);
        player.y().write(&mut host.context(), 512);
        player.level().write(&mut host.context(), 1);
        player.sprite_state().write(&mut host.context(), 0);
        player.animation_group_mask().write(&mut host.context(), 0);
        ram::g_player_interaction_choice_active(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_wizard_interaction_states(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_wiseman_interaction_states(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_opera_singer_interaction_states(revision)
            .at(number)
            .write(&mut host.context(), 0);
    }
    record.object().write(&mut host.context(), handle);
    record.second_object().write(&mut host.context(), 0);
    record.level().write(&mut host.context(), 1);
    object.x().write(&mut host.context(), 512);
    object.y().write(&mut host.context(), 512);
    object.level().write(&mut host.context(), 1);
    object.draw_flags().write(&mut host.context(), 3);
    object.frame().write(&mut host.context(), 5);
    object.frame_duration().write(&mut host.context(), 0);
    for (kind, id) in [
        (10, ids::AI_WIZARD_TOUCH),
        (17, ids::AI_OPERA_SINGER_TOUCH),
        (11, ids::AI_WISEMAN_TOUCH),
        (9, ids::AI_MAILBOX_TOUCH),
    ] {
        if selected.is_some_and(|name| name != id.metadata().name) {
            continue;
        }
        record.kind().write(&mut host.context(), kind);
        record.state().write(&mut host.context(), 0);
        ram::g_mailbox_level_states(revision)
            .at(0)
            .level()
            .write(&mut host.context(), 1);
        ram::g_mailbox_level_states(revision)
            .at(0)
            .mode()
            .write(&mut host.context(), 0);
        ram::g_mailbox_level_states(revision)
            .at(0)
            .contact()
            .at(0)
            .write(&mut host.context(), 0);
        fixture(host, id, &[0, record.address()]);
    }
    if selected.is_none_or(|name| name == "AI_GenericEnemyThink") {
        let definition = tje_mem::rom::earthling_definitions(revision).at(3);
        assert!(definition.turn_chance().read(&host.context()) > 0);
        record.kind().write(&mut host.context(), 3);
        record.state().write(&mut host.context(), 6);
        record.state_delay().write(&mut host.context(), 1);
        record.timer().write(&mut host.context(), 0);
        object.sprite_state().write(&mut host.context(), 6);
        ram::g_random_cache_index(revision).write(&mut host.context(), 0);
        for (index, value) in [(1, 0), (2, 1), (3, 0)] {
            ram::g_cached_random_values(revision)
                .at(index)
                .write(&mut host.context(), value);
        }
        fixture(host, ids::AI_GENERIC_ENEMY_THINK, &[record.address()]);
    }
    if selected.is_none_or(|name| name == "PlayerProcessLifeRequest") {
        let donor = ram::g_player_info(revision).at(0);
        donor.sprite_state().write(&mut host.context(), 0);
        donor.animation_group_mask().write(&mut host.context(), 0);
        ram::g_player_lives(revision)
            .at(0)
            .write(&mut host.context(), 3);
        ram::g_player_lives(revision)
            .at(1)
            .write(&mut host.context(), -1);
        ram::g_player_inventory_open(revision)
            .at(0)
            .write(&mut host.context(), 0);
        ram::g_player_elevator_flags(revision)
            .at(0)
            .write(&mut host.context(), 0);
        ram::g_player_interaction_choice_active(revision)
            .at(0)
            .write(&mut host.context(), 0);
        ram::g_player_interaction_responses(revision)
            .at(0)
            .write(&mut host.context(), 0);
        ram::g_controller_ports_swapped(revision).write(&mut host.context(), 0);
        ram::g_controller_buttons_ac_swapped(revision).write(&mut host.context(), 0);
        ram::g_controller2_raw_input(revision).write(&mut host.context(), 0x40);
        for selection in [0, 2] {
            ram::g_player_selection(revision).write(&mut host.context(), selection);
            ram::g_life_request_display_entity(revision).write(&mut host.context(), 0);
            ram::g_life_request_pending(revision).write(&mut host.context(), 0);
            fixture(host, ids::PLAYER_PROCESS_LIFE_REQUEST, &[1]);
        }
        ram::g_life_request_display_entity(revision).write(&mut host.context(), i16::from(handle));
        ram::g_life_request_pending(revision).write(&mut host.context(), 1);
        ram::g_player_interaction_responses(revision)
            .at(0)
            .write(&mut host.context(), 1);
        fixture(host, ids::PLAYER_PROCESS_LIFE_REQUEST, &[1]);
    }
}

fn persist(
    host: &Host,
    revision: Revision,
    frames: u64,
    natural: &serde_json::Value,
    error: Option<&str>,
) {
    if let Ok(directory) = std::env::var("TJE_EARTHLINGS_METRICS_DIR") {
        std::fs::create_dir_all(&directory).unwrap();
        let report = serde_json::json!({"revision": format!("{revision:?}"), "frames_requested": frames, "frame": host.frame_count(), "natural": natural, "total": host.checks, "measurements": host.measurements, "error": error});
        std::fs::write(
            format!("{directory}/{revision:?}.json"),
            serde_json::to_string_pretty(&report).unwrap(),
        )
        .unwrap();
    }
}

#[test]
#[ignore = "requires both private ROM revisions"]
fn held_composed_session_and_settled_fixtures_both_revisions() {
    let script = Arc::new(
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap(),
    );
    let frames =
        std::env::var("TJE_EARTHLINGS_FRAMES").map_or(10_000, |value| value.parse().unwrap());
    let filter = std::env::var("TJE_EARTHLINGS_ROOT").ok();
    let roots: Vec<_> = ROOTS
        .iter()
        .copied()
        .filter(|id| {
            filter
                .as_ref()
                .is_none_or(|name| *name == id.metadata().name)
        })
        .collect();
    assert!(!roots.is_empty(), "unknown held proof root");
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = if revision == Revision::Rev02 {
            "TJE_REV02_ROM"
        } else {
            "TJE_REV00_ROM"
        };
        let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
        let registry =
            HookRegistry::proof_filtered(revision, previous::proof_translations(), &roots).unwrap();
        let mode = Mode::Check;
        let mut checked = Host::new(&bytes, mode, registry.clone()).unwrap();
        let mut reference = Host::new(&bytes, Mode::Reference, registry).unwrap();
        checked.set_wait_input_poller(script.clone());
        reference.set_wait_input_poller(script.clone());
        for frame in 0..frames {
            let a = match checked.run_frame(script.at(frame)) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    persist(
                        &checked,
                        revision,
                        frames,
                        &serde_json::Value::Null,
                        Some(&error.to_string()),
                    );
                    panic!("{revision:?} frame {frame}: {error}");
                }
            };
            let b = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(a.ram, b.ram, "{revision:?} RAM frame {frame}");
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
                        .map(|value| value.to_bits()))
            );
        }
        let natural = serde_json::to_value(&checked.checks).unwrap();
        let boundary = checked
            .settle_fixture_boundary(script.at(checked.frame_count()))
            .unwrap();
        println!(
            "HELD_BOUNDARY {revision:?}: ticks={} frames={}..{}",
            boundary.ticks, boundary.start_frame, boundary.end_frame
        );
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if filter
                .as_deref()
                .is_none_or(|name| name == "OpenEarthlingPresent")
            {
                previous::present_fixtures(&mut checked);
            }
            if filter.as_ref().is_none() {
                frame_fixtures(&mut checked);
            }
            friendly_fixtures(&mut checked, filter.as_deref());
            if filter.as_ref().is_none() {
                previous::time_aware_fixtures(&mut checked);
            }
        }));
        persist(
            &checked,
            revision,
            frames,
            &natural,
            outcome.as_ref().err().map(|_| "fixture panic; see stderr"),
        );
        if let Err(payload) = outcome {
            std::panic::resume_unwind(payload);
        }
        for id in &roots {
            assert!(
                checked.checks.by_function[id.metadata().name].calls_checked > 0,
                "unexercised {}",
                id.metadata().name
            );
        }
        assert_eq!(checked.checks.mismatches, 0);
        println!("HELD_PASS {revision:?}: {} checks", checked.checks.calls);
    }
}
