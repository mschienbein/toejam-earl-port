//! Opt-in ROM-backed executable, built by a private scratch Cargo manifest with
//! tje-game, tje-host, tje-mem and serde_json dependencies. Not part of tje-game's
//! dependency graph. Arguments: ROM directory, evidence directory, Jam Out input.
//! Every fixture starts from a completed live session; no recorded state is saved.

#[path = "../jamout/check_fixtures.rs"]
mod jamout;

use std::{fs, path::Path};
use tje_game::{FunctionId, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram};

// This executable proves the original W0 delivery, not later subsystem batches.
const W0_ROOTS: &[FunctionId] = &[
    ids::DISABLE_BLANKING_GRAPHICS,
    ids::ENABLE_BLANKING_GRAPHICS,
    ids::GET_V_BLANK_GRAPHICS_ENABLED,
    ids::READ_MAPPED_PLAYER_INPUT,
    ids::GET_LEVEL_MUSIC_INDEX,
    ids::RESET_VDP,
    ids::DRAW_LEVEL_GRAPHICS,
    ids::PLAYERS_REQUIRE_SEPARATE_VIEWS,
    ids::DELAY_ITERATIONS,
    ids::JAM_OUT_UPDATE_LEAD_ANIMATIONS,
    ids::JAM_OUT_SYNCHRONIZE_ENTITY_COPIES,
    ids::JAM_OUT_POSITION_TOEJAM_COPY,
    ids::JAM_OUT_POSITION_EARL_COPY,
    ids::JAM_OUT_HIDE_TOEJAM_COPY,
    ids::JAM_OUT_HIDE_EARL_COPY,
    ids::JAM_OUT_SET_TOEJAM_SEQUENCE,
    ids::JAM_OUT_SET_EARL_SEQUENCE,
];

fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) -> u32 {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| {
            panic!(
                "live-state fixture {} {arguments:?}: {error}",
                id.metadata().name
            )
        })
}

fn core_fixtures(host: &mut Host) {
    let revision = host.revision();
    for count in [0, 1, 10, i16::MAX as u32, 0x8000, 0xffff, 0x12340001] {
        check(host, ids::DELAY_ITERATIONS, &[count]);
    }
    let toejam = ram::g_player_info(revision).at(0);
    let earl = ram::g_player_info(revision).at(1);
    for lives in [(-1, 0), (0, -1), (0, 0)] {
        ram::g_player_lives(revision)
            .at(0)
            .write(&mut host.context(), lives.0);
        ram::g_player2_lives(revision).write(&mut host.context(), lives.1);
        for (first_level, second_level) in [(0, 0), (0, 1)] {
            toejam.level().write(&mut host.context(), first_level);
            earl.level().write(&mut host.context(), second_level);
            check(host, ids::PLAYERS_REQUIRE_SEPARATE_VIEWS, &[]);
        }
    }
    earl.level().write(&mut host.context(), 0);
    for (a, b) in [
        (0, 0),
        (0, 124),
        (0, 125),
        (124, 0),
        (125, 0),
        (i16::MIN, 1),
        (1, i16::MIN),
        (i16::MAX, -1),
        (-1, i16::MAX),
        (i16::MIN, 0),
        (0, i16::MIN),
        (i16::MIN, i16::MAX),
    ] {
        for (x_a, x_b, y_a, y_b) in [(0, 0, a, b), (a, b, 0, 0)] {
            toejam.x().write(&mut host.context(), x_a);
            earl.x().write(&mut host.context(), x_b);
            toejam.y().write(&mut host.context(), y_a);
            earl.y().write(&mut host.context(), y_b);
            toejam.height().write(&mut host.context(), 0);
            earl.height().write(&mut host.context(), 0);
            check(host, ids::PLAYERS_REQUIRE_SEPARATE_VIEWS, &[]);
        }
    }
    for (first_y, second_y) in [(0, 124), (124, 0)] {
        toejam.x().write(&mut host.context(), 0);
        earl.x().write(&mut host.context(), 0);
        toejam.y().write(&mut host.context(), first_y);
        earl.y().write(&mut host.context(), second_y);
        for mask in [0, 0x8000, 0x04000000, u32::MAX] {
            for height in [-1, 0, 1, i16::MAX] {
                toejam.height().write(&mut host.context(), height);
                earl.height().write(&mut host.context(), height);
                toejam
                    .animation_group_mask()
                    .write(&mut host.context(), mask);
                earl.animation_group_mask().write(&mut host.context(), mask);
                check(host, ids::PLAYERS_REQUIRE_SEPARATE_VIEWS, &[]);
            }
        }
    }
    for level in [-1, 25, 26, 127] {
        ram::g_player1_map_level(revision).write(&mut host.context(), level);
        for delay in [i8::MIN, -1, 0, 1, 127] {
            for frame in [-128, -1, 0, 3, 127] {
                ram::g_level_palette_animation_delay(revision).write(&mut host.context(), delay);
                ram::g_level_palette_animation_frame(revision).write(&mut host.context(), frame);
                check(host, ids::DRAW_LEVEL_GRAPHICS, &[]);
            }
        }
    }
    check(host, ids::RESET_VDP, &[]);
    for id in [
        ids::DISABLE_BLANKING_GRAPHICS,
        ids::ENABLE_BLANKING_GRAPHICS,
        ids::GET_V_BLANK_GRAPHICS_ENABLED,
    ] {
        check(host, id, &[]);
    }
    check(host, ids::GET_LEVEL_MUSIC_INDEX, &[0xffff]);
    check(host, ids::READ_MAPPED_PLAYER_INPUT, &[0]);
}

fn compare_frame(checked: &mut Host, reference: &mut Host, script: &InputScript, frame: u64) {
    let a = checked.run_frame(script.at(frame)).unwrap();
    let b = reference.run_frame(script.at(frame)).unwrap();
    assert_eq!(a.frame_counter, b.frame_counter, "frame counter {frame}");
    assert_eq!(a.ram, b.ram, "whole RAM including stack {frame}");
    assert_eq!(a.vdp, b.vdp, "VDP {frame}");
    assert_eq!(a.writes, b.writes, "ordered hardware writes {frame}");
    assert_eq!(a.idle_random_draws, b.idle_random_draws, "idle RNG {frame}");
    assert_eq!(a.lag_events, b.lag_events, "lag events {frame}");
    assert!(
        a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
            .audio_samples
            .iter()
            .flatten()
            .map(|v| v.to_bits())),
        "audio bits {frame}"
    );
    let (a, sa) = checked.frame_buffer();
    let (b, sb) = reference.frame_buffer();
    assert_eq!(sa, sb, "frame dimensions {frame}");
    assert_eq!(
        &a[..sa.len() as usize],
        &b[..sb.len() as usize],
        "pixels {frame}"
    );
}

fn session(
    revision: Revision,
    rom_root: &Path,
    output_root: &Path,
    label: &str,
    script: &InputScript,
) {
    let revision_file = match revision {
        Revision::Rev02 => "rev02",
        Revision::Rev00 => "rev00",
    };
    let bytes = fs::read(rom_root.join(format!("{revision_file}.bin"))).unwrap();
    let registry = HookRegistry::filtered(revision, &[], W0_ROOTS).unwrap();
    let mut host = Host::new(&bytes, Mode::Check, registry.clone()).unwrap();
    let mut reference = Host::new(&bytes, Mode::Reference, registry).unwrap();
    for frame in 0..10_000 {
        compare_frame(&mut host, &mut reference, script, frame);
    }
    assert!(host.measurements.title_frame.is_some());
    assert!(host.measurements.menu_frame.is_some());
    if label == "gameplay" {
        assert!(host.measurements.game_frame.is_some());
    } else {
        assert!(host.checks.by_function["JamOutUpdateLeadAnimations"].calls_checked > 0);
    }
    let natural = serde_json::to_value(&host.checks).unwrap();
    if label == "gameplay" {
        core_fixtures(&mut host);
        jamout::fixtures(&mut host);
        assert!(
            host.checks.unexercised.is_empty(),
            "{:?}",
            host.checks.unexercised
        );
    }
    assert_eq!(host.checks.mismatches, 0);
    assert!(host.checks.uninitialized_stack_reads.is_empty());
    let report = serde_json::json!({
        "revision": revision.name(), "coverage_source": label,
        "frames": host.frame_count(), "whole_session_equal": true,
        "filter": "W0_ROOTS (17 explicit function IDs)", "natural": natural,
        "with_live_fixtures": host.checks, "measurements": host.measurements,
    });
    let path = output_root.join(format!("{label}-{revision_file}.json"));
    fs::write(&path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    println!("proof {}", path.display());
}

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    assert_eq!(
        arguments.len(),
        3,
        "ROM directory, evidence directory, Jam Out input"
    );
    let rom_root = Path::new(&arguments[0]);
    let output_root = Path::new(&arguments[1]);
    let gameplay =
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap();
    let jam = InputScript::parse(&fs::read_to_string(&arguments[2]).unwrap()).unwrap();
    assert!(tje_game::Dispatcher::native(W0_ROOTS).is_ok());
    for revision in [Revision::Rev02, Revision::Rev00] {
        session(revision, rom_root, output_root, "gameplay", &gameplay);
        session(revision, rom_root, output_root, "jamout", &jam);
    }
}
