//! Opt-in selected pending-player natural checks. Each revision still runs its
//! recorded original timeline; all comparison failures are retained, not waived.
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tje_game::{FramedTranslation, Translation, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::Revision;

static SELECTED: LazyLock<Vec<Translation>> = LazyLock::new(|| {
    let names = std::env::var("PLAYER_HELD_FUNCTIONS").ok();
    let selected: Vec<_> = tje_game::player::PENDING_TRANSLATIONS
        .iter()
        .filter(|translation| {
            names.as_ref().map_or(
                translation.id != ids::PLAYER_FINALIZE_LAST_LIFE_EXIT,
                |names| {
                    names
                        .split(',')
                        .any(|name| name == translation.id.metadata().name)
                },
            )
        })
        .copied()
        .collect();
    selected
});

static SELECTED_FRAMED: LazyLock<Vec<FramedTranslation>> = LazyLock::new(|| {
    let names = std::env::var("PLAYER_HELD_FUNCTIONS").ok();
    let selected: Vec<_> = tje_game::player::PENDING_FRAMED_TRANSLATIONS
        .iter()
        .filter(|translation| {
            names.as_ref().is_none_or(|names| {
                names
                    .split(',')
                    .any(|name| name == translation.id.metadata().name)
            })
        })
        .copied()
        .collect();
    if let Some(names) = names {
        assert_eq!(
            SELECTED.len() + selected.len(),
            names.split(',').count(),
            "unknown or duplicate held function"
        );
    }
    assert!(!SELECTED.is_empty() || !selected.is_empty());
    selected
});

fn script(path: &str) -> InputScript {
    let recording: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(recording["start"]["kind"], "power_on");
    let mut start = 0;
    let mut holds = Vec::new();
    for run in recording["runs"].as_array().unwrap() {
        let end = start + run["frames"].as_u64().unwrap();
        assert_eq!(run["pads"][0].as_u64().unwrap() & 256, 256);
        assert_eq!(run["pads"][1].as_u64().unwrap() & 256, 0);
        let bits = run["pads"][0].as_u64().unwrap();
        let buttons: Vec<_> = ["up", "down", "left", "right", "a", "b", "c", "start"]
            .into_iter()
            .enumerate()
            .filter_map(|(bit, name)| (bits & (1 << bit) != 0).then_some(name))
            .collect();
        holds.push(serde_json::json!({"start":start,"end":end,"p1":buttons}));
        start = end;
    }
    InputScript::parse(&serde_json::json!({"p2_connected":false,"holds":holds}).to_string())
        .unwrap()
}

fn main() {
    let output = PathBuf::from(std::env::args_os().nth(1).expect("evidence directory"));
    fs::create_dir_all(&output).unwrap();
    let mut failures = 0;
    for (revision, name) in [(Revision::Rev02, "rev02"), (Revision::Rev00, "rev00")] {
        let path = format!("sessions/{name}-one-player.tje.json");
        let inputs = Arc::new(script(&path));
        let rom = fs::read(format!("roms/{name}.bin")).unwrap();
        let registry =
            HookRegistry::proof_with_frames(revision, &SELECTED, &SELECTED_FRAMED).unwrap();
        let mut checked = Host::new(&rom, Mode::Check, registry.clone()).unwrap();
        let mut reference = Host::new(&rom, Mode::Reference, registry).unwrap();
        checked.set_wait_input_poller(inputs.clone());
        reference.set_wait_input_poller(inputs.clone());
        let mut completed = 0;
        let result = (|| -> Result<(), String> {
            for frame in 0..10000 {
                let a = checked
                    .run_frame(inputs.at(frame))
                    .map_err(|error| error.to_string())?;
                let b = reference
                    .run_frame(inputs.at(frame))
                    .map_err(|error| error.to_string())?;
                if a.ram != b.ram
                    || a.vdp != b.vdp
                    || a.writes != b.writes
                    || a.frame_counter != b.frame_counter
                    || a.idle_random_draws != b.idle_random_draws
                    || a.lag_events != b.lag_events
                {
                    return Err(format!("whole-frame comparison differs at {frame}"));
                }
                if !a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits()))
                {
                    return Err(format!("audio differs at {frame}"));
                }
                let (pa, sa) = checked.frame_buffer();
                let (pb, sb) = reference.frame_buffer();
                if sa != sb || pa[..sa.len() as usize] != pb[..sb.len() as usize] {
                    return Err(format!("pixels differ at {frame}"));
                }
                completed += 1;
            }
            Ok(())
        })();
        if result.is_err() {
            failures += 1;
        }
        fs::write(output.join(format!("{name}-held-natural.json")),serde_json::to_string_pretty(&serde_json::json!({"revision":name,"input":path,"requested_frames":10000,"completed_frames":completed,"error":result.as_ref().err(),"game_frame":checked.measurements.game_frame,"checks":checked.checks})).unwrap()).unwrap();
        println!(
            "PLAYER_HELD_NATURAL {name} frames={completed} checked={} mismatches={} result={result:?}",
            checked.checks.calls, checked.checks.mismatches
        );
    }
    assert_eq!(
        failures, 0,
        "held natural proof failed; inspect retained reports"
    );
}
