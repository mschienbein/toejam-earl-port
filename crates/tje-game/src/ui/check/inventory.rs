//! Opt-in proof of explicitly selected inventory candidates; never hooks unrelated
//! pending menu/pause bodies. All natural inputs remain controller-only scripts.
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::Path,
    sync::{Arc, OnceLock},
};
use tje_game::{FunctionId, Translation, ids, ui};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
static CANDIDATES: OnceLock<Vec<Translation>> = OnceLock::new();

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
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 6 {
        return Err("usage: tje-ui-inventory-check <rom-dir> <output-dir> <functions-comma-list> <input-json> <frames> <label>".into());
    }
    let output = Path::new(&args[1]);
    fs::create_dir_all(output)?;
    let mut candidates = Vec::new();
    for name in args[2].split(',') {
        let translation = ui::PENDING_TRANSLATIONS
            .iter()
            .chain(ui::TRANSLATIONS)
            .find(|entry| entry.id.metadata().name == name)
            .copied()
            .ok_or_else(|| format!("no UI body named {name}"))?;
        candidates.push(translation);
    }
    let candidates = CANDIDATES.get_or_init(|| candidates).as_slice();
    let source = fs::read_to_string(&args[3])?;
    let frames: u64 = args[4].parse()?;
    let mut failures = Vec::new();
    let fixtures_enabled = std::env::var_os("TJE_UI_FIXTURES").is_some();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let bytes = fs::read(Path::new(&args[0]).join(match revision {
            Revision::Rev02 => "rev02.bin",
            Revision::Rev00 => "rev00.bin",
        }))?;
        let registry = HookRegistry::proof(revision, candidates)?;
        let script = InputScript::parse(&source)?;
        let mut host = Host::new(&bytes, Mode::Check, registry.clone())?;
        host.set_wait_input_poller(Arc::new(InputScript::parse(&source)?));
        let mut reference = Host::new(&bytes, Mode::Reference, registry)?;
        let mut fixture_counts = BTreeMap::new();
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
            let fixture_result = if fixtures_enabled {
                fixtures(&mut host)
            } else {
                Ok(())
            };
            for (name, coverage) in host.checks.by_function.iter() {
                fixture_counts.insert(name.to_owned(), coverage.calls_checked - before[name]);
            }
            fixture_result?;
            if !host.checks.unexercised.is_empty()
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
                "input":args[3], "whole_session_equal":natural_reference_equal, "error":result.as_ref().err().map(ToString::to_string),
                "fixtures_enabled":fixtures_enabled,
                "fixture_counts":fixture_counts, "checks":host.checks, "measurements":host.measurements,
                "fixture_entry_sr":host.cpu().status_register(), "fixture_entry_pc":host.cpu().pc(),
            }))?,
        )?;
        match result {
            Ok(()) => println!("{revision:?}: {} selected hooks PASS", candidates.len()),
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
