//! One check-only replay campaign for all twelve falling/respawn player roots.
//! Natural recordings and authored real-state fixtures remain separate evidence.
use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use std::{fs, path::PathBuf, sync::Arc};
use tje_game::ids;
use tje_host::{
    CheckFailurePolicy, Host, HostError, Mode, execution::WaitInputPoller, hooks::HookRegistry,
    input::InputScript,
};
use tje_mem::{Bus, Revision};
use tje_session::{CONNECTED, Session};
mod proof_support;
mod replay_fixture_cases;

struct Timeline {
    session: Session,
    ends: Vec<u64>,
}
impl Timeline {
    fn new(session: Session) -> Self {
        let mut end = 0;
        let ends = session
            .runs
            .iter()
            .map(|run| {
                end += run.0;
                end
            })
            .collect();
        Self { session, ends }
    }
}
fn inputs(pads: [u16; 2]) -> GenesisInputs {
    use GenesisButton::*;
    let pads = pads.map(|pad| {
        let mut controller = if pad & CONNECTED != 0 {
            GenesisController::new(GenesisControllerType::ThreeButton)
        } else {
            GenesisController::None
        };
        for (bit, button) in [Up, Down, Left, Right, A, B, C, Start]
            .into_iter()
            .enumerate()
        {
            controller.set_field(button, pad & (1 << bit) != 0);
        }
        controller
    });
    GenesisInputs {
        p1: pads[0],
        p2: pads[1],
    }
}
impl WaitInputPoller for Timeline {
    fn at(&self, frame: u64) -> tje_host::Result<GenesisInputs> {
        let index = self.ends.partition_point(|end| *end <= frame);
        let run = self.session.runs.get(index).ok_or_else(|| {
            HostError::Input(format!("recording has no pad sample for frame {frame}"))
        })?;
        Ok(inputs([run.1, run.2]))
    }
    fn before_frame(&self, frame: u64, memory: &mut dyn Bus) -> tje_host::Result<()> {
        for write in self
            .session
            .writes()
            .iter()
            .filter(|write| write.before_frame == frame)
        {
            match write.width {
                1 => memory.write_u8(write.address, write.value as u8),
                2 => memory.write_u16(write.address, write.value as u16),
                4 => memory.write_u32(write.address, write.value),
                _ => unreachable!("validated recipe"),
            }
        }
        Ok(())
    }
}
fn registry(revision: Revision) -> HookRegistry {
    HookRegistry::proof_replayed_environment(
        revision,
        tje_game::player::PENDING_TRANSLATIONS,
        tje_game::player::PENDING_FRAMED_TRANSLATIONS,
        replay_fixture_cases::ROOTS,
    )
    .expect("frozen check-only replay registry")
}
fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let output = PathBuf::from(arguments.next().expect("evidence directory"));
    fs::create_dir_all(&output).unwrap();
    let mut sessions: Vec<PathBuf> = arguments.map(PathBuf::from).collect();
    if sessions.is_empty() {
        sessions.extend(
            [
                "sessions/rev02-one-player.tje.json",
                "sessions/rev00-one-player.tje.json",
            ]
            .map(PathBuf::from),
        );
    }
    let recordings: Vec<_> = sessions
        .into_iter()
        .map(|path| {
            let session = Session::from_json(&fs::read_to_string(&path).unwrap()).unwrap();
            (path, Arc::new(Timeline::new(session)))
        })
        .collect();
    let mut failures = 0;
    for (revision, name) in [(Revision::Rev02, "rev02"), (Revision::Rev00, "rev00")] {
        let rom = fs::read(format!("roms/{name}.bin")).unwrap();
        let mut coverage = std::collections::BTreeMap::<String, u64>::new();
        for (path, timeline) in recordings
            .iter()
            .filter(|(_, timeline)| Revision::from(timeline.session.revision) == revision)
        {
            let mut host = Host::new(&rom, Mode::Check, registry(revision)).unwrap();
            host.set_check_failure_policy(CheckFailurePolicy::ContinueOriginal)
                .unwrap();
            host.set_wait_input_poller(timeline.clone());
            let mut error = None;
            for frame in 0..timeline.session.frames {
                if let Err(failure) = timeline
                    .before_frame(frame, &mut host.context())
                    .and_then(|()| timeline.at(frame))
                    .and_then(|pad| host.run_frame(pad).map(|_| ()))
                {
                    error = Some(failure.to_string());
                    break;
                }
            }
            let provenance = host.finish_future_frame_provenance();
            let complete = error.is_none()
                && host.checks.mismatches == 0
                && host.checks.failures.is_empty()
                && provenance.ensure_proof_eligible().is_ok();
            failures += u32::from(!complete);
            for id in replay_fixture_cases::ROOTS {
                let count = host.checks.by_function[id.metadata().name].calls_checked;
                *coverage.entry(id.metadata().name.into()).or_default() += count;
            }
            let report = serde_json::json!({"revision":name,"kind":"recorded input timeline; every selected live root, including naturally nested calls, checked in one campaign pass","session":path,"start":timeline.session.start,"requested_frames":timeline.session.frames,"completed_frames":host.frame_count(),"error":error,"checks":host.checks,"future_frame_provenance":provenance,"complete":complete,"composition_proof":false,"live_replacement_eligible":false});
            let stem = path.file_stem().unwrap().to_string_lossy();
            fs::write(
                output.join(format!("{name}-{stem}-replayed.json")),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
        }
        let script = Arc::new(
            InputScript::parse(
                &fs::read_to_string("crates/tje-host/examples/start-game.json").unwrap(),
            )
            .unwrap(),
        );
        let mut host = Host::new(&rom, Mode::Reference, registry(revision)).unwrap();
        host.set_wait_input_poller(script.clone());
        for frame in 0..10000 {
            host.run_frame(script.at(frame)).unwrap();
        }
        let boundary = proof_support::settle(&mut host, &script);
        let cases = replay_fixture_cases::run(&mut host);
        let provenance = host.finish_future_frame_provenance();
        let complete = cases.iter().all(|case| {
            case["error"].is_null() && case["checked"].as_u64().is_some_and(|calls| calls > 0)
        }) && host.checks.mismatches == 0
            && provenance.ensure_proof_eligible().is_ok();
        failures += u32::from(!complete);
        for id in replay_fixture_cases::ROOTS {
            let count = host.checks.by_function[id.metadata().name].calls_checked;
            *coverage.entry(id.metadata().name.into()).or_default() += count;
        }
        let unexercised: Vec<_> = replay_fixture_cases::ROOTS
            .iter()
            .filter(|id| coverage[id.metadata().name] == 0)
            .map(|id| id.metadata().name)
            .collect();
        failures += u32::from(!unexercised.is_empty());
        let report = serde_json::json!({"revision":name,"kind":"labelled real-state fixtures after original-only advance and settled boundary; not natural or composition coverage","fixture_input":"crates/tje-host/examples/start-game.json","fixture_frames":10000,"settled_boundary":boundary,"cases":cases,"checks":host.checks,"future_frame_provenance":provenance,"complete":complete,"coverage_union":coverage,"unexercised":unexercised,"stronger_control_paths":[{"function":ids::PLAYER_FINALIZE_LAST_LIFE_EXIT.metadata().name,"reason":"game-over/reset does not return; the returning survivor fixture is not full-function approval"},{"function":ids::PLAYER_UPDATE_DEATH_ANIMATION.metadata().name,"reason":"a last-life terminal callee path requires the stronger exit tier"},{"function":ids::PROCESS_PLAYERS.metadata().name,"reason":"the dispatched death/final-life terminal path requires the stronger exit tier"}],"live_replacement_eligible":false});
        fs::write(
            output.join(format!("{name}-replayed-fixtures.json")),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        println!(
            "PLAYER_REPLAY {name} fixtures={} mismatches={} complete={complete} unexercised={unexercised:?}",
            host.checks.calls, host.checks.mismatches
        );
    }
    assert_eq!(
        failures, 0,
        "replayed-environment campaign is not complete; both revision receipts retained"
    );
}
