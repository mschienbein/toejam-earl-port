//! Opt-in player-lane proof binary, built by a private external Cargo manifest.
//! Inputs start from power-on; synthetic fixtures are labelled separately.
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, OnceLock},
};
use tje_game::FunctionId;
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::Revision;

mod wave1_fixtures;
mod wave2_fixtures;
mod wave3_fixtures;
mod wave4_fixtures;
mod wave5_fixtures;

fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| panic!("{} {arguments:08x?}: {error}", id.metadata().name));
}

fn session_script(path: &str) -> InputScript {
    let session: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(session["start"]["kind"], "power_on");
    let runs = session["runs"].as_array().unwrap();
    let connected = runs[0]["pads"][1].as_u64().unwrap() & 256 != 0;
    let mut frame = 0;
    let mut holds = Vec::new();
    for run in runs {
        let end = frame + run["frames"].as_u64().unwrap();
        let mut pads = [Vec::new(), Vec::new()];
        for (index, pad) in pads.iter_mut().enumerate() {
            let bits = run["pads"][index].as_u64().unwrap();
            assert_eq!(bits & 256 != 0, index == 0 || connected);
            for (bit, button) in ["up", "down", "left", "right", "a", "b", "c", "start"]
                .into_iter()
                .enumerate()
            {
                if bits & (1 << bit) != 0 {
                    pad.push(button);
                }
            }
        }
        holds.push(serde_json::json!({"start":frame,"end":end,"p1":pads[0],"p2":pads[1]}));
        frame = end;
    }
    InputScript::parse(&serde_json::json!({"p2_connected":connected,"holds":holds}).to_string())
        .unwrap()
}

fn main() {
    let evidence = PathBuf::from(std::env::args_os().nth(1).expect("evidence directory"));
    let frames: u64 = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "10000".into())
        .parse()
        .unwrap();
    fs::create_dir_all(&evidence).unwrap();
    let wave: u8 = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "2".into())
        .parse()
        .unwrap();
    let include_time = std::env::args().nth(4).as_deref() == Some("time");
    static PROOF_PENDING: OnceLock<Vec<tje_game::Translation>> = OnceLock::new();
    let pending: &'static [tje_game::Translation] = PROOF_PENDING.get_or_init(|| {
        tje_game::player::PENDING_TRANSLATIONS
            .iter()
            .copied()
            .filter(|translation| {
                translation.id.metadata().wave <= wave
                    && (include_time || tje_game::is_atomic_capable(translation.id))
                    && !tje_game::player::FRAME_BLOCKED.contains(&translation.id)
            })
            .collect()
    });
    for revision in [Revision::Rev02, Revision::Rev00] {
        let name = if revision == Revision::Rev02 {
            "rev02"
        } else {
            "rev00"
        };
        let input_path = format!("sessions/{name}-one-player.tje.json");
        let script = Arc::new(session_script(&input_path));
        let rom = fs::read(format!("roms/{name}.bin")).unwrap();
        let mut registry = HookRegistry::proof(revision, pending).unwrap();
        for &translation in tje_game::player::TRANSLATIONS {
            let class = tje_game::execution_class(translation.id).unwrap();
            registry
                .register_translation_with_execution(translation, class)
                .unwrap();
        }
        let mut checked = Host::new(&rom, Mode::Check, registry.clone()).unwrap();
        let mut reference = Host::new(&rom, Mode::Reference, registry.clone()).unwrap();
        checked.set_wait_input_poller(script.clone());
        reference.set_wait_input_poller(script.clone());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for frame in 0..frames {
                let a = checked.run_frame(script.at(frame)).unwrap();
                let b = reference.run_frame(script.at(frame)).unwrap();
                assert_eq!(a.frame_counter, b.frame_counter, "{name} {frame}");
                assert_eq!(a.ram, b.ram, "{name} RAM {frame}");
                assert_eq!(a.vdp, b.vdp, "{name} VDP {frame}");
                assert_eq!(a.writes, b.writes, "{name} writes {frame}");
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
        // Keep authored state changes isolated from the naturally compared run.
        // This separately reached live state is the already exercised W1 fixture
        // boundary, not a snapshot or a power-on synthetic machine.
        let mut fixture_host = Host::new(&rom, Mode::Reference, registry).unwrap();
        let fixture_script = Arc::new(
            InputScript::parse(
                &fs::read_to_string("crates/tje-host/examples/start-game.json").unwrap(),
            )
            .unwrap(),
        );
        fixture_host.set_wait_input_poller(fixture_script.clone());
        for frame in 0..10000 {
            fixture_host.run_frame(fixture_script.at(frame)).unwrap();
        }
        assert!(fixture_host.measurements.game_frame.is_some());
        let fixture_result = if result.is_ok() {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                wave1_fixtures::run(&mut fixture_host);
                wave2_fixtures::run(&mut fixture_host);
                if wave >= 3 {
                    wave3_fixtures::run(&mut fixture_host);
                }
                if wave >= 4 {
                    wave4_fixtures::run(&mut fixture_host);
                }
                if wave >= 5 {
                    wave5_fixtures::run(&mut fixture_host);
                }
            }))
        } else {
            result
        };
        let metrics = serde_json::json!({"revision":name,"frames":frames,"input":input_path,"natural":natural,"fixtures":fixture_host.checks,"fixture_frames":10000,"fixture_input":"crates/tje-host/examples/start-game.json","fixtures_completed":fixture_result.is_ok(),"game_frame":checked.measurements.game_frame});
        fs::write(
            evidence.join(format!("{name}-proof.json")),
            serde_json::to_string_pretty(&metrics).unwrap(),
        )
        .unwrap();
        if let Err(error) = fixture_result {
            std::panic::resume_unwind(error);
        }
        assert_eq!(checked.checks.mismatches, 0);
        assert!(checked.measurements.game_frame.is_some());
        println!(
            "PLAYER_PROOF {name} frames={frames} calls={} mismatches={} nested={} dead_stack_calls={} dead_stack_bytes={}",
            checked.checks.calls,
            checked.checks.mismatches,
            checked.checks.nested_original_calls,
            checked.checks.dead_stack_differing_calls,
            checked.checks.dead_stack_differing_bytes
        );
    }
}
