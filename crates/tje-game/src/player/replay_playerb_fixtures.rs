//! Private opt-in PlayerB proof driver. It never alters production tables.
//! Replayed proofs are modular single-pass checks, not Rust-callee composition.
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tje_game::{Context, FramedTranslation, FunctionId, Translation, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::Revision;

mod proof_support;
mod replay_playerb_cases;

static ROOTS: LazyLock<Vec<FunctionId>> = LazyLock::new(|| {
    let names = std::env::var("PLAYERB_FUNCTIONS").unwrap_or_else(|_| {
        "HulaAbility,PlayerUpdateWhirlwindAndRecovery,Player_Youch,HottubAbility,ChickenAbility,PlayerApplyPhasedJumpInput,PlayerUpdatePhasedJumpMotion".into()
    });
    if names == "all" {
        return replay_playerb_cases::FUNCTIONS.to_vec();
    }
    let selected: Vec<_> = replay_playerb_cases::FUNCTIONS
        .iter()
        .copied()
        .filter(|id| names.split(',').any(|name| name == id.metadata().name))
        .collect();
    assert_eq!(
        selected.len(),
        names.split(',').count(),
        "unknown or duplicate PlayerB root"
    );
    selected
});

static VALUES: LazyLock<Vec<Translation>> = LazyLock::new(|| {
    tje_game::player::PENDING_TRANSLATIONS
        .iter()
        .filter(|entry| ROOTS.contains(&entry.id))
        .copied()
        .collect()
});
static FRAMED: LazyLock<Vec<FramedTranslation>> = LazyLock::new(|| {
    tje_game::player::PENDING_FRAMED_TRANSLATIONS
        .iter()
        .filter(|entry| ROOTS.contains(&entry.id))
        .copied()
        .collect()
});

fn registry(revision: Revision) -> HookRegistry {
    assert_eq!(
        VALUES.len() + FRAMED.len(),
        ROOTS.len(),
        "root no longer pending: refresh the private harness selection"
    );
    match std::env::var("PLAYERB_TIER")
        .as_deref()
        .unwrap_or("replayed")
    {
        "replayed" => HookRegistry::proof_replayed_environment(
            revision,
            tje_game::player::PENDING_TRANSLATIONS,
            tje_game::player::PENDING_FRAMED_TRANSLATIONS,
            &ROOTS,
        )
        .unwrap(),
        "strict" => {
            assert!(
                ROOTS.iter().all(|id| matches!(
                    *id,
                    ids::LEMONADE_ABILITY
                        | ids::AI_BOOGIE_TOUCH
                        | ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT
                        | ids::SPRITE_CHECKS
                )),
                "strict harness mode is for the four frozen candidates, not a substitute replay proof"
            );
            HookRegistry::proof_with_frames(revision, &VALUES, &FRAMED).unwrap()
        }
        other => panic!("unknown PLAYERB_TIER {other}"),
    }
}

fn recording(path: &str) -> (InputScript, u64) {
    let session: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(session["start"]["kind"], "power_on");
    let runs = session["runs"].as_array().unwrap();
    let connected = runs[0]["pads"][1].as_u64().unwrap() & 256 != 0;
    let mut frame = 0;
    let mut holds = Vec::new();
    for run in runs {
        let end = frame + run["frames"].as_u64().unwrap();
        let pads = [0, 1].map(|index| {
            let bits = run["pads"][index].as_u64().unwrap();
            assert_eq!(
                bits & 256 != 0,
                index == 0 || connected,
                "connection changes require the session runner"
            );
            ["up", "down", "left", "right", "a", "b", "c", "start"]
                .into_iter()
                .enumerate()
                .filter_map(|(bit, name)| (bits & (1 << bit) != 0).then_some(name))
                .collect::<Vec<_>>()
        });
        holds.push(serde_json::json!({"start":frame,"end":end,"p1":pads[0],"p2":pads[1]}));
        frame = end;
    }
    (
        InputScript::parse(
            &serde_json::json!({"p2_connected":connected,"holds":holds}).to_string(),
        )
        .unwrap(),
        frame,
    )
}

fn check_prepared(host: &mut Host, id: FunctionId, arguments: &[u32]) -> Result<(), String> {
    // Labelled RTS injection uses the real settled caller PC in immutable ROM,
    // not CALL_SENTINEL. This is explicitly not evidence of a natural JSR.
    assert!(host.fixture_boundary_is_settled());
    let caller_pc = host.cpu().pc();
    assert!(caller_pc < 0x100000 && caller_pc & 1 == 0);
    let base = host.context().stack_pointer().unwrap();
    let mut sp = base;
    {
        let mut c = host.context();
        for &argument in arguments.iter().rev() {
            sp = sp.wrapping_sub(4);
            c.write_u32(sp, argument);
        }
        sp = sp.wrapping_sub(4);
        c.write_u32(sp, caller_pc);
        c.set_stack_pointer(sp).unwrap();
    }
    let result = host
        .check_entry_fixture(id.metadata().address(host.revision()))
        .map(|_| ())
        .map_err(|error| error.to_string());
    host.context().set_stack_pointer(base).unwrap();
    result
}

fn fixtures(rom: &[u8], revision: Revision, output: &std::path::Path) -> bool {
    let input = "crates/tje-host/examples/start-game.json";
    let script = Arc::new(InputScript::parse(&fs::read_to_string(input).unwrap()).unwrap());
    let mut host = Host::new(rom, Mode::Reference, registry(revision)).unwrap();
    host.set_wait_input_poller(script.clone());
    for frame in 0..10000 {
        host.run_frame(script.at(frame)).unwrap();
    }
    assert!(host.measurements.game_frame.is_some());
    let boundary = proof_support::settle(&mut host, &script);
    let mut results = Vec::new();
    for &id in ROOTS.iter() {
        for &label in replay_playerb_cases::labels(id) {
            for number in 0..2 {
                let arguments = replay_playerb_cases::configure(&mut host, id, label, number);
                let result = check_prepared(&mut host, id, &arguments);
                results.push(serde_json::json!({"function":id.metadata().name,"label":label,"player":number,"kind":"labelled-real-state","arguments":arguments,"error":result.err()}));
            }
        }
    }
    let ok = results.iter().all(|result| result["error"].is_null()) && host.checks.mismatches == 0;
    fs::write(output, serde_json::to_string_pretty(&serde_json::json!({"tier":std::env::var("PLAYERB_TIER").unwrap_or_else(|_| "replayed".into()),"revision":format!("{revision:?}"),"warmup_frames":10000,"warmup_input":input,"settled_boundary":boundary,"cases":results,"checks":host.checks,"passed":ok})).unwrap()).unwrap();
    ok
}

fn natural(
    rom: &[u8],
    revision: Revision,
    input: &str,
    maximum: Option<u64>,
    output: &std::path::Path,
) -> bool {
    let (script, recorded_frames) = recording(input);
    let script = Arc::new(script);
    let frames = maximum.map_or(recorded_frames, |limit| limit.min(recorded_frames));
    let registry = registry(revision);
    let mut checked = Host::new(rom, Mode::Check, registry.clone()).unwrap();
    let mut reference = Host::new(rom, Mode::Reference, registry).unwrap();
    checked.set_wait_input_poller(script.clone());
    reference.set_wait_input_poller(script.clone());
    let mut completed = 0;
    let outcome = (|| -> Result<(), String> {
        for frame in 0..frames {
            let a = checked
                .run_frame(script.at(frame))
                .map_err(|error| error.to_string())?;
            let b = reference
                .run_frame(script.at(frame))
                .map_err(|error| error.to_string())?;
            if a.frame_counter != b.frame_counter
                || a.ram != b.ram
                || a.vdp != b.vdp
                || a.writes != b.writes
                || a.idle_random_draws != b.idle_random_draws
                || a.lag_events != b.lag_events
            {
                return Err(format!("whole-frame state mismatch at {frame}"));
            }
            if !a
                .audio_samples
                .iter()
                .flatten()
                .map(|value| value.to_bits())
                .eq(b
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|value| value.to_bits()))
            {
                return Err(format!("audio mismatch at {frame}"));
            }
            let (pa, sa) = checked.frame_buffer();
            let (pb, sb) = reference.frame_buffer();
            if sa != sb || pa[..sa.len() as usize] != pb[..sb.len() as usize] {
                return Err(format!("pixel mismatch at {frame}"));
            }
            completed += 1;
        }
        Ok(())
    })();
    let ok = outcome.is_ok() && checked.checks.mismatches == 0;
    fs::write(output, serde_json::to_string_pretty(&serde_json::json!({"kind":"natural-recording","revision":format!("{revision:?}"),"input":input,"requested_frames":frames,"completed_frames":completed,"error":outcome.err(),"checks":checked.checks,"passed":ok})).unwrap()).unwrap();
    ok
}

fn main() {
    let arguments: Vec<_> = std::env::args().collect();
    let output = PathBuf::from(arguments.get(1).expect("evidence directory"));
    let mode = arguments.get(2).map_or("fixtures", String::as_str);
    fs::create_dir_all(&output).unwrap();
    let rom_root = std::env::var("PLAYERB_ROM_ROOT").unwrap_or_else(|_| "roms".into());
    let mut failures = 0;
    for (revision, name) in [(Revision::Rev02, "rev02"), (Revision::Rev00, "rev00")] {
        let rom = fs::read(format!("{rom_root}/{name}.bin")).unwrap();
        let evidence = output.join(format!("{name}-{mode}.json"));
        let ok = match mode {
            "fixtures" => fixtures(&rom, revision, &evidence),
            "natural" => {
                let input = arguments.get(3).map_or_else(
                    || format!("sessions/{name}-one-player.tje.json"),
                    |path| path.replace("{revision}", name),
                );
                let maximum = arguments
                    .get(4)
                    .map(|value| value.parse().expect("maximum frames"));
                natural(&rom, revision, &input, maximum, &evidence)
            }
            _ => panic!("mode must be fixtures or natural"),
        };
        failures += usize::from(!ok);
        println!(
            "PLAYERB_PROOF {name} {mode} passed={ok} evidence={}",
            evidence.display()
        );
    }
    assert_eq!(failures, 0, "proof failed; retain both-revision reports");
}
