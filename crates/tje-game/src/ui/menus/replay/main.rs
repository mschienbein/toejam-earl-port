//! Check-only replay-environment campaign. Usage: ROM_DIRECTORY OUTPUT_DIRECTORY
//! [comma-separated recording suffixes | all | fixtures-only]. No live replacement.
use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use std::{error::Error, fs, path::Path, sync::Arc};
use tje_game::{FunctionId, ids, ui::menus};
use tje_host::{
    CheckFailurePolicy, Host, Mode, execution::WaitInputPoller, hooks::HookRegistry,
    input::InputScript,
};
use tje_mem::{Bus, Revision, rom};
use tje_session::{CONNECTED, Session};
type Result<T = (), E = Box<dyn Error + Send + Sync>> = std::result::Result<T, E>;

const ROOTS: [FunctionId; 9] = [
    ids::NEW_GAME_MAIN_MENU,
    ids::SHOW_GAME_OVER_AND_RESET,
    ids::MENU_WHO_HANDLER,
    ids::MENU_CONTROLLER_OPTIONS_HANDLER,
    ids::MENU_WHAT_HANDLER,
    ids::MENU_JAM_OUT_HANDLER,
    ids::GENERAL_MENU_HANDLER,
    ids::RUN_FINAL_LEVEL_PAUSE_MENU,
    ids::GAME_PAUSED,
];

fn inputs(pads: [u16; 2]) -> GenesisInputs {
    use GenesisButton::*;
    let pads = pads.map(|pad| {
        let mut controller = if pad & CONNECTED != 0 {
            GenesisController::new(GenesisControllerType::ThreeButton)
        } else {
            GenesisController::None
        };
        for (index, button) in [Up, Down, Left, Right, A, B, C, Start]
            .into_iter()
            .enumerate()
        {
            controller.set_field(button, pad & (1 << index) != 0);
        }
        controller
    });
    GenesisInputs {
        p1: pads[0],
        p2: pads[1],
    }
}
struct Timeline {
    session: Session,
    ends: Vec<u64>,
}
impl Timeline {
    fn load(path: &Path) -> Result<Self> {
        let session = Session::from_json(&fs::read_to_string(path)?)?;
        let mut end = 0;
        let ends = session
            .runs
            .iter()
            .map(|run| {
                end += run.0;
                end
            })
            .collect();
        Ok(Self { session, ends })
    }
}
impl WaitInputPoller for Timeline {
    fn at(&self, frame: u64) -> tje_host::Result<GenesisInputs> {
        let run = self
            .session
            .runs
            .get(self.ends.partition_point(|end| *end <= frame))
            .ok_or_else(|| {
                tje_host::HostError::Input(format!("no recorded input at frame {frame}"))
            })?;
        Ok(inputs([run.1, run.2]))
    }
    fn before_frame(&self, frame: u64, bus: &mut dyn Bus) -> tje_host::Result<()> {
        let writes = self.session.writes();
        for write in writes[writes.partition_point(|write| write.before_frame < frame)..]
            .iter()
            .take_while(|write| write.before_frame == frame)
        {
            match write.width {
                1 => bus.write_u8(write.address, write.value as u8),
                2 => bus.write_u16(write.address, write.value as u16),
                4 => bus.write_u32(write.address, write.value),
                _ => unreachable!("validated recipe width"),
            }
        }
        Ok(())
    }
}
fn registry(revision: Revision) -> tje_host::Result<HookRegistry> {
    HookRegistry::proof_replayed_environment(revision, &[], menus::MENU_FRAME_CANDIDATES, &ROOTS)
}
fn save(path: &Path, value: &serde_json::Value) -> Result {
    let temporary = path.with_extension("json.incomplete");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    fs::File::open(&temporary)?.sync_all()?;
    fs::rename(&temporary, path)?;
    fs::File::open(path.parent().ok_or("receipt parent")?)?.sync_all()?;
    Ok(())
}
fn replay(bytes: &[u8], revision: Revision, path: &Path, output: &Path) -> Result<bool> {
    let timeline = Arc::new(Timeline::load(path)?);
    let mut host = Host::new(bytes, Mode::Check, registry(revision)?)?;
    host.set_wait_input_poller(timeline.clone());
    host.set_check_failure_policy(CheckFailurePolicy::ContinueOriginal)?;
    let result = (|| -> Result {
        for frame in 0..timeline.session.frames {
            timeline.before_frame(frame, &mut host.context())?;
            host.play_frame(timeline.at(frame)?)?;
        }
        Ok(())
    })();
    let provenance = host.finish_future_frame_provenance();
    let eligibility = provenance.ensure_proof_eligible();
    let passed = result.is_ok()
        && eligibility.is_ok()
        && host.checks.mismatches == 0
        && host.checks.failures.is_empty()
        && host.checks.uninitialized_stack_reads.is_empty();
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or("session name")?;
    save(
        &output.join(format!("{}-{name}.json", revision.name())),
        &serde_json::json!({
        "tier":"replayed_environment_check_only", "modular_callees":"actual original JSR-to-RTS segments; not all-Rust composition",
        "session":path,"revision":revision.name(),"expected_frames":timeline.session.frames,
        "frames":host.frame_count(),"passed":passed,"error":result.err().map(|error|error.to_string()),
        "eligibility_error":eligibility.err().map(|error|error.to_string()),"provenance":provenance,"checks":host.checks,
        "failure_policy":"ContinueOriginal retains every failure; completing more frames never grants approval"}),
    )?;
    Ok(passed)
}
fn fixtures(bytes: &[u8], revision: Revision, root: &Path, output: &Path) -> Result<bool> {
    let file = if revision == Revision::Rev02 {
        "rev02"
    } else {
        "rev00"
    };
    let source = Arc::new(Timeline::load(
        &root.join(format!("sessions/{file}-one-player.tje.json")),
    )?);
    let mut host = Host::new(bytes, Mode::Reference, registry(revision)?)?;
    host.set_wait_input_poller(source.clone());
    for frame in 0..1600 {
        source.before_frame(frame, &mut host.context())?;
        host.play_frame(source.at(frame)?)?;
    }
    // Settle by genuine original progress before the first fixture mutation.
    host.settle_fixture_boundary(source.at(host.frame_count())?)?;
    let first = host.frame_count();
    let script = InputScript::parse(
        &serde_json::json!({"p2_connected":true,"holds":[
        {"start":first+8,"end":first+16,"p1":["start"]},
        {"start":first+24,"end":first+32,"p1":["start"]},
        {"start":first+40,"end":first+48,"p1":["start"]},
        {"start":first+56,"end":first+64,"p1":["start"]}]})
        .to_string(),
    )?;
    host.set_wait_input_poller(Arc::new(script));
    let mut cases = Vec::new();
    for id in [
        ids::MENU_WHO_HANDLER,
        ids::MENU_CONTROLLER_OPTIONS_HANDLER,
        ids::MENU_WHAT_HANDLER,
        ids::MENU_JAM_OUT_HANDLER,
        ids::NEW_GAME_MAIN_MENU,
        ids::RUN_FINAL_LEVEL_PAUSE_MENU,
        ids::GAME_PAUSED,
    ] {
        let args: &[u32] = if matches!(
            id,
            ids::NEW_GAME_MAIN_MENU | ids::RUN_FINAL_LEVEL_PAUSE_MENU | ids::GAME_PAUSED
        ) {
            &[0]
        } else {
            &[]
        };
        let result = host.check_fixture(id.metadata().address(revision), args);
        cases.push(serde_json::json!({"function":id.metadata().name,"arguments":args,
            "label":"settled original gameplay state; real future Start edges; no IRQ/SR/readiness edits",
            "passed":result.is_ok(),"error":result.err().map(|error|error.to_string())}));
    }
    let count = i32::from(rom::menu_what_item_count(revision).read(&host.context())) as u32;
    let args = [2, rom::menu_what_items(revision).address(), count];
    let result = host.check_fixture(
        ids::GENERAL_MENU_HANDLER.metadata().address(revision),
        &args,
    );
    cases.push(
        serde_json::json!({"function":"GeneralMenuHandler","arguments":args,
        "label":"settled retail What menu records and real future Start",
        "passed":result.is_ok(),"error":result.err().map(|error|error.to_string())}),
    );
    // Reset is not fabricated as RTS. The typed-entry diagnostic records the
    // genuine stronger-tier rejection or typed exit, never a replay promotion.
    let result =
        host.check_entry_fixture(ids::SHOW_GAME_OVER_AND_RESET.metadata().address(revision));
    cases.push(serde_json::json!({"function":"ShowGameOverAndReset","label":"settled live reset entry; hardware reset requires stronger evidence",
        "replay_coverable":false,"entry_outcome":result.as_ref().ok().map(|outcome|format!("{outcome:?}")),
        "error":result.err().map(|error|error.to_string())}));
    let provenance = host.finish_future_frame_provenance();
    let eligibility = provenance.ensure_proof_eligible();
    let passed = cases
        .iter()
        .filter(|case| case["replay_coverable"] != false)
        .all(|case| case["passed"] == true)
        && eligibility.is_ok()
        && host.checks.mismatches == 0
        && host.checks.failures.is_empty()
        && host.checks.uninitialized_stack_reads.is_empty();
    save(
        &output.join(format!("{}-fixtures.json", revision.name())),
        &serde_json::json!({
        "tier":"replayed_environment_check_only","revision":revision.name(),"settled_frame":first,
        "passed":passed,"cases":cases,"provenance":provenance,"checks":host.checks,
        "eligibility_error":eligibility.err().map(|error|error.to_string())}),
    )?;
    Ok(passed)
}
fn main() -> Result {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let roms = Path::new(args.first().ok_or("missing ROM directory")?);
    let output = Path::new(args.get(1).ok_or("missing output directory")?);
    let selector = args
        .get(2)
        .map(String::as_str)
        .unwrap_or("menus,jam-all,jam-out,one-player,two-player,final-menus,game-over,intro");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../../..");
    fs::create_dir_all(output)?;
    let mut sessions = Vec::new();
    if selector != "fixtures-only" {
        for entry in fs::read_dir(root.join("sessions"))? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.ends_with(".tje.json")
                && (selector == "all"
                    || selector
                        .split(',')
                        .any(|suffix| name.ends_with(&format!("-{suffix}.tje.json"))))
            {
                sessions.push(path);
            }
        }
        sessions.sort();
        if sessions.is_empty() {
            return Err("no matching committed sessions".into());
        }
    }
    let mut failures = 0;
    for (revision, file) in [(Revision::Rev02, "rev02"), (Revision::Rev00, "rev00")] {
        let bytes = fs::read(roms.join(format!("{file}.bin")))?;
        for path in &sessions {
            if !replay(&bytes, revision, path, output)? {
                failures += 1;
            }
        }
        if !fixtures(&bytes, revision, &root, output)? {
            failures += 1;
        }
    }
    if failures != 0 {
        return Err(format!(
            "{failures} replay/fixture receipts failed; unsupported reset is not approval"
        )
        .into());
    }
    Ok(())
}
