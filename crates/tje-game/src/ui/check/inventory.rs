//! Opt-in proof of explicitly selected inventory candidates; never hooks unrelated
//! pending menu/pause bodies. All natural inputs remain controller-only scripts.
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::Path,
    sync::{Arc, OnceLock},
};
use tje_game::{FramedTranslation, FunctionId, Translation, ids, ui};
use tje_host::{
    Host, Mode,
    hooks::HookRegistry,
    input::{Button, InputHold, InputScript},
};
use tje_mem::{Revision, ram};

mod replay_fixtures;

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
static CANDIDATES: OnceLock<Vec<Translation>> = OnceLock::new();
static FRAMED_CANDIDATES: OnceLock<Vec<FramedTranslation>> = OnceLock::new();

fn input_script(source: &str) -> Result<InputScript> {
    if let Ok(script) = InputScript::parse(source) {
        return Ok(script);
    }
    let session = tje_session::Session::from_json(source)?;
    if !session.writes().is_empty() {
        return Err("owner natural proof requires a controller-only power-on session; recipe sessions belong to the corpus runner".into());
    }
    let p2_connected = session
        .runs
        .first()
        .is_some_and(|run| run.2 & tje_session::CONNECTED != 0);
    let mut holds = Vec::new();
    let mut start = 0;
    for tje_session::Run(frames, p1, p2) in session.runs {
        if p1 & tje_session::CONNECTED == 0 || (p2 & tje_session::CONNECTED != 0) != p2_connected {
            return Err("InputScript conversion requires constant controller connectivity".into());
        }
        let buttons = |pad: u16| {
            [
                (tje_session::UP, Button::Up),
                (tje_session::DOWN, Button::Down),
                (tje_session::LEFT, Button::Left),
                (tje_session::RIGHT, Button::Right),
                (tje_session::A, Button::A),
                (tje_session::B, Button::B),
                (tje_session::C, Button::C),
                (tje_session::START, Button::Start),
            ]
            .into_iter()
            .filter_map(|(bit, button)| (pad & bit != 0).then_some(button))
            .collect()
        };
        if (p1 | p2) & 0xff != 0 {
            holds.push(InputHold {
                start,
                end: start + frames,
                p1: buttons(p1),
                p2: buttons(p2),
            });
        }
        start += frames;
    }
    Ok(InputScript {
        p2_connected,
        holds,
    })
}

fn check(host: &mut Host, id: FunctionId, args: &[u32]) -> Result<()> {
    if !host.checks.by_function.contains_key(id.metadata().name) {
        return Ok(());
    }
    host.check_fixture(id.metadata().address(host.revision()), args)?;
    Ok(())
}

fn fixtures(host: &mut Host) -> Result<()> {
    let revision = host.revision();
    for player in [0, 1] {
        for suppressed in [0, 1] {
            ram::g_inventory_labels_suppressed(revision).write(&mut host.context(), suppressed);
            for slot in 0..16 {
                ram::g_player_inventory(revision).at(player).at(slot).write(
                    &mut host.context(),
                    if slot % 3 == 0 { -1 } else { (slot % 28) as i8 },
                );
            }
            for present in 0..28 {
                ram::g_present_info(revision)
                    .at(present)
                    .identified()
                    .write(&mut host.context(), (present % 2) as i8);
            }
            ram::g_player_inventory_scroll_row(revision)
                .at(player)
                .write(&mut host.context(), 0);
            check(host, ids::LOAD_PRESENTS_INTO_INVENTORY, &[player as u32])?;
        }
        for split in [0, 1] {
            ram::g_split_screen_enabled(revision).write(&mut host.context(), split);
            check(
                host,
                ids::INVENTORY_DEACTIVATE_DISPLAY_ENTITIES,
                &[player as u32],
            )?;
            check(
                host,
                ids::INVENTORY_INITIALIZE_DISPLAY_ENTITIES,
                &[player as u32],
            )?;
        }
        if host.cpu().status_register() & 0x700 == 0 {
            for banner in [0, 1, 2, 3] {
                check(
                    host,
                    ids::INVENTORY_DRAW_ACTION_BANNER,
                    &[player as u32, banner, 1],
                )?;
            }
        }
    }
    // A frame-local pointer call is exercised without a present-effect wait.
    let opening = ram::g_text_font_buffers(revision).cast::<i16>();
    opening.write(&mut host.context(), 0);
    opening.offset(1).write(&mut host.context(), 0);
    check(host, ids::CHECK_IF_OPEN_FOR_BOTH, &[opening.address()])?;
    // Direct redraw services all phases itself; unlike the IRQ-driven refresh,
    // this source path has no interrupt-dependent return condition.
    check(host, ids::INVENTORY_REDRAW_SYNCHRONOUSLY, &[0])?;
    check(host, ids::INVENTORY_REDRAW_SYNCHRONOUSLY, &[1])?;
    Ok(())
}

fn run(args: &[String], fixtures_enabled: bool, require_all: bool) -> Result<()> {
    if args.len() != 6 {
        return Err("usage: tje-ui-inventory-check <rom-dir> <output-dir> <inventory|functions-comma-list> <input-or-session-json> <frames> <label>".into());
    }
    let output = Path::new(&args[1]);
    fs::create_dir_all(output)?;
    let mut candidates = Vec::new();
    let mut framed_candidates = Vec::new();
    let names: Vec<_> = if args[2] == "inventory" {
        ui::PENDING_FRAMED_TRANSLATIONS
            .iter()
            .filter(|entry| entry.id.metadata().source == "src/ui/inventory.c")
            .map(|entry| entry.id.metadata().name)
            .collect()
    } else {
        args[2].split(',').collect()
    };
    for name in names {
        if let Some(entry) = ui::PENDING_FRAMED_TRANSLATIONS
            .iter()
            .find(|entry| entry.id.metadata().name == name)
            .copied()
        {
            framed_candidates.push(entry);
            continue;
        }
        let translation = ui::PENDING_TRANSLATIONS
            .iter()
            .chain(ui::TRANSLATIONS)
            .find(|entry| entry.id.metadata().name == name)
            .copied()
            .ok_or_else(|| format!("no UI body named {name}"))?;
        candidates.push(translation);
    }
    let candidates = CANDIDATES.get_or_init(|| candidates).as_slice();
    let framed_candidates = FRAMED_CANDIDATES
        .get_or_init(|| framed_candidates)
        .as_slice();
    let frames: u64 = args[4].parse()?;
    let mut failures = Vec::new();
    let replay = std::env::var_os("TJE_UI_REPLAY").is_some();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let revision_key = match revision {
            Revision::Rev02 => "rev02",
            Revision::Rev00 => "rev00",
        };
        let input_path = args[3].replace("{revision}", revision_key);
        let source = fs::read_to_string(&input_path)?;
        let bytes = fs::read(Path::new(&args[0]).join(match revision {
            Revision::Rev02 => "rev02.bin",
            Revision::Rev00 => "rev00.bin",
        }))?;
        let registry = if replay {
            HookRegistry::proof_replayed_environment(revision, candidates, framed_candidates, &[])?
        } else {
            HookRegistry::proof_with_frames(revision, candidates, framed_candidates)?
        };
        let script = Arc::new(input_script(&source)?);
        let mut host = Host::new(&bytes, Mode::Check, registry.clone())?;
        host.set_wait_input_poller(script.clone());
        let mut reference = Host::new(&bytes, Mode::Reference, registry)?;
        let mut fixture_counts = BTreeMap::new();
        let mut fixture_observations = Vec::new();
        let mut natural_reference_equal = false;
        let result = (|| -> Result<()> {
            for frame in 0..frames {
                let a = host.run_frame(script.at(frame))?;
                let b = reference.run_frame(script.at(frame))?;
                if a.frame_counter != b.frame_counter
                    || a.ram != b.ram
                    || a.vdp != b.vdp
                    || a.writes != b.writes
                    || a.idle_random_draws != b.idle_random_draws
                    || a.lag_events != b.lag_events
                    || !a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
                        .audio_samples
                        .iter()
                        .flatten()
                        .map(|v| v.to_bits()))
                {
                    return Err(format!("whole-session mismatch at {frame}").into());
                }
                let (pixels_a, size_a) = host.frame_buffer();
                let (pixels_b, size_b) = reference.frame_buffer();
                if size_a != size_b
                    || pixels_a[..size_a.len() as usize] != pixels_b[..size_b.len() as usize]
                {
                    return Err(format!("whole-session pixel mismatch at {frame}").into());
                }
            }
            natural_reference_equal = true;
            let before: BTreeMap<_, _> = host
                .checks
                .by_function
                .iter()
                .map(|(name, c)| (name.to_owned(), c.calls_checked))
                .collect();
            let fixture_result = if fixtures_enabled && replay {
                host.settle_fixture_boundary(script.at(host.frame_count()))?;
                fixture_observations = replay_fixtures::run(&mut host)?;
                Ok(())
            } else if fixtures_enabled {
                fixtures(&mut host)
            } else {
                Ok(())
            };
            for (name, coverage) in host.checks.by_function.iter() {
                fixture_counts.insert(name.to_owned(), coverage.calls_checked - before[name]);
            }
            fixture_result?;
            if fixture_observations.iter().any(|observation| {
                observation
                    .get("error")
                    .is_some_and(|error| !error.is_null())
            }) {
                return Err("one or more labelled inventory fixtures failed".into());
            }
            if (require_all && !host.checks.unexercised.is_empty())
                || host.checks.mismatches != 0
                || !host.checks.uninitialized_stack_reads.is_empty()
            {
                return Err("incomplete selected inventory proof".into());
            }
            Ok(())
        })();
        fs::write(
            output.join(format!("{}-{}.json", revision.name(), args[5])),
            serde_json::to_vec_pretty(&serde_json::json!({
                "revision":revision.name(), "frames":host.frame_count(), "selected":args[2],
                "input":input_path, "whole_session_equal":natural_reference_equal, "error":result.as_ref().err().map(ToString::to_string),
                "proof_scope": if replay { "own instructions with authenticated original callee segments; not Rust composition" } else { "ClassDefined" },
                "fixture_observations":fixture_observations,
                "fixtures_enabled":fixtures_enabled,
                "fixture_counts":fixture_counts, "checks":host.checks, "measurements":host.measurements,
                "fixture_entry_sr":host.cpu().status_register(), "fixture_entry_pc":host.cpu().pc(),
            }))?,
        )?;
        match result {
            Ok(()) => {
                let exercised = host
                    .checks
                    .by_function
                    .iter()
                    .filter(|(_, coverage)| coverage.calls_checked != 0)
                    .count();
                println!(
                    "{revision:?}: completed with zero mismatches; {exercised}/{} selected roots exercised",
                    candidates.len() + framed_candidates.len()
                );
            }
            Err(error) => {
                eprintln!("{revision:?}: {error}");
                failures.push(format!("{revision:?}: {error}"));
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; ").into())
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("--campaign") {
        return run(&args, std::env::var_os("TJE_UI_FIXTURES").is_some(), true);
    }
    if args.len() != 4 || std::env::var_os("TJE_UI_REPLAY").is_none() {
        return Err("usage: TJE_UI_REPLAY=1 tje-ui-inventory-check --campaign <rom-dir> <output-dir> <manifest-json>".into());
    }
    let manifest: serde_json::Value = serde_json::from_str(&fs::read_to_string(&args[3])?)?;
    let jobs = manifest["jobs"]
        .as_array()
        .ok_or("campaign jobs must be an array")?;
    let output = Path::new(&args[2]);
    fs::create_dir_all(output)?;
    let mut coverage = BTreeMap::<String, BTreeMap<String, u64>>::new();
    let mut outcomes = Vec::new();
    let mut failures = Vec::new();
    // One sequential campaign, all fourteen roots hooked together. Natural
    // nested original entries are checked by Host, not by per-root reruns.
    for job in jobs {
        let source = job["source"]
            .as_str()
            .ok_or("campaign source must be a path")?;
        let frames = job["frames"]
            .as_u64()
            .ok_or("campaign frames must be an integer")?;
        let label = job["label"]
            .as_str()
            .ok_or("campaign label must be a string")?;
        let fixtures = job["fixtures"].as_bool().unwrap_or(false);
        let invocation = [
            args[1].clone(),
            args[2].clone(),
            "inventory".into(),
            source.into(),
            frames.to_string(),
            label.into(),
        ];
        let result = run(&invocation, fixtures, fixtures);
        if let Err(error) = &result {
            failures.push(format!("{label}: {error}"));
        }
        outcomes.push(
            serde_json::json!({"label":label, "source":source, "frames":frames,
            "fixtures":fixtures, "error":result.err().map(|error| error.to_string())}),
        );
        for revision in [Revision::Rev02, Revision::Rev00] {
            let path = output.join(format!("{}-{label}.json", revision.name()));
            let receipt: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
            let counts = coverage.entry(revision.name().into()).or_default();
            if let Some(functions) = receipt["checks"]["by_function"].as_object() {
                for (name, function) in functions {
                    *counts.entry(name.clone()).or_default() +=
                        function["calls_checked"].as_u64().unwrap_or(0);
                }
            }
        }
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        for entry in ui::PENDING_FRAMED_TRANSLATIONS
            .iter()
            .filter(|entry| entry.id.metadata().source == "src/ui/inventory.c")
        {
            if coverage
                .get(revision.name())
                .and_then(|counts| counts.get(entry.id.metadata().name))
                .copied()
                .unwrap_or(0)
                == 0
            {
                failures.push(format!(
                    "{}: unexercised {}",
                    revision.name(),
                    entry.id.metadata().name
                ));
            }
        }
    }
    fs::write(
        output.join("campaign.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "scope":"modular own-instruction replay; original callee segments are explicit, not Rust composition",
            "manifest":args[3], "coverage":coverage, "jobs":outcomes, "failures":failures,
        }))?,
    )?;
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; ").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_run_conversion_preserves_buttons_and_connectivity() {
        let mut recording =
            tje_session::Session::new(Revision::Rev02, "authored input conversion test");
        recording
            .record([tje_session::CONNECTED, tje_session::CONNECTED])
            .unwrap();
        recording
            .record([
                tje_session::CONNECTED | tje_session::A,
                tje_session::CONNECTED | tje_session::B,
            ])
            .unwrap();
        recording
            .record([tje_session::CONNECTED, tje_session::CONNECTED])
            .unwrap();
        let script = input_script(&recording.to_json().unwrap()).unwrap();
        let expected = InputScript::parse(
            r#"{"p2_connected":true,"holds":[{"start":1,"end":2,"p1":["a"],"p2":["b"]}]}"#,
        )
        .unwrap();
        for frame in 0..3 {
            assert_eq!(script.at(frame), expected.at(frame));
        }
    }
}
