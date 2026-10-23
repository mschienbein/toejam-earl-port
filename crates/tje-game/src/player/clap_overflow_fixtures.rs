//! Opt-in RH01 original-versus-Rust regression binary, built by the lane's
//! private manifest. Extreme positions are labelled live-state fixtures, not
//! naturally reached gameplay coordinates or serialized machine snapshots.
use std::{fs, path::PathBuf};
use tje_game::ids;
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram};

fn check(host: &mut Host, toejam_y: i16, earl_y: i16, reversed: bool, label: &str) {
    let revision = host.revision();
    let toejam = ram::g_player_info(revision).at(0);
    let earl = ram::g_player_info(revision).at(1);
    for player in [toejam, earl] {
        player.level().write(&mut host.context(), 1);
        player.height().write(&mut host.context(), 20);
        player.animation_motion_axes().write(&mut host.context(), 3);
        player.frame_duration().write(&mut host.context(), 1);
        player.frame().write(&mut host.context(), 0);
        player.draw_flags().write(&mut host.context(), 1);
    }
    toejam
        .x()
        .write(&mut host.context(), if reversed { 124 } else { 100 });
    earl.x()
        .write(&mut host.context(), if reversed { 100 } else { 124 });
    toejam.y().write(&mut host.context(), toejam_y);
    earl.y().write(&mut host.context(), earl_y);
    toejam
        .sprite_state()
        .write(&mut host.context(), if reversed { 10 } else { 11 });
    earl.sprite_state()
        .write(&mut host.context(), if reversed { 11 } else { 10 });
    toejam.animation_group_mask().write(&mut host.context(), 4);
    earl.animation_group_mask().write(&mut host.context(), 4);
    host.check_fixture(
        ids::PLAYER_UPDATE_HEALTH_SHARING_CLAP
            .metadata()
            .address(revision),
        &[],
    )
    .unwrap_or_else(|error| {
        panic!(
            "{label}: {revision:?} toejam_y={toejam_y} earl_y={earl_y} reversed={reversed}: {error}"
        )
    });
}

fn main() {
    let output = PathBuf::from(std::env::args_os().nth(1).expect("evidence directory"));
    fs::create_dir_all(&output).unwrap();
    let script = InputScript::parse(
        &fs::read_to_string("crates/tje-host/examples/start-game.json").unwrap(),
    )
    .unwrap();
    for (revision, name) in [(Revision::Rev02, "rev02"), (Revision::Rev00, "rev00")] {
        let registry =
            HookRegistry::filtered(revision, &[], &[ids::PLAYER_UPDATE_HEALTH_SHARING_CLAP])
                .unwrap();
        let mut host = Host::new(
            &fs::read(format!("roms/{name}.bin")).unwrap(),
            Mode::Reference,
            registry,
        )
        .unwrap();
        for frame in 0..10000 {
            host.run_frame(script.at(frame)).unwrap();
        }
        assert!(host.measurements.game_frame.is_some());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            check(
                &mut host,
                i16::MAX,
                i16::MIN,
                false,
                "RH01-clap-subw-overflow",
            );
            check(
                &mut host,
                i16::MIN,
                i16::MAX,
                false,
                "RH01-clap-positive-overflow",
            );
            let anchors = [
                i16::MIN,
                i16::MIN + 1,
                -7,
                -1,
                0,
                1,
                6,
                7,
                i16::MAX - 1,
                i16::MAX,
            ];
            for toejam in anchors {
                for earl in anchors {
                    for reversed in [false, true] {
                        check(&mut host, toejam, earl, reversed, "RH01-boundary-matrix");
                    }
                }
            }
        }));
        fs::write(output.join(format!("{name}-rh01-proof.json")), serde_json::to_string_pretty(&serde_json::json!({
            "revision": name, "input": "crates/tje-host/examples/start-game.json", "frames": 10000,
            "kind": "labelled live-state RH01 fixtures; natural reachability of extreme Y not established",
            "review_counterexamples": ["RH01-clap-subw-overflow", "RH01-clap-positive-overflow"],
            "checks": host.checks, "completed": result.is_ok()
        })).unwrap()).unwrap();
        if let Err(error) = result {
            std::panic::resume_unwind(error);
        }
        assert_eq!(host.checks.calls, 202);
        assert_eq!(host.checks.mismatches, 0);
        assert!(host.checks.uninitialized_stack_reads.is_empty());
        println!(
            "PLAYER_RH01_PROOF {name} calls={} mismatches=0",
            host.checks.calls
        );
    }
}
