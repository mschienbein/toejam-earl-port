//! Opt-in natural-session half of the presents proof harness.
//! Arguments: ROOT EVIDENCE session REVISION FUNCTION SESSION [END_FRAME].
use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use std::{env, fs, path::Path, sync::Arc};
use tje_host::{Host, Mode, execution::WaitInputPoller, hooks::HookRegistry};
use tje_mem::Revision;
use tje_session::{CONNECTED, Session, Start};

struct SessionInputs {
    session: Session,
    ends: Vec<u64>,
}
impl SessionInputs {
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
impl WaitInputPoller for SessionInputs {
    fn at(&self, frame: u64) -> tje_host::Result<GenesisInputs> {
        let index = self.ends.partition_point(|&end| end <= frame);
        let run = self.session.runs.get(index).ok_or_else(|| {
            tje_host::HostError::Input(format!("session has no input for frame {frame}"))
        })?;
        let pads = [run.1, run.2].map(|pad| {
            let mut controller = if pad & CONNECTED != 0 {
                GenesisController::new(GenesisControllerType::ThreeButton)
            } else {
                GenesisController::None
            };
            for (bit, button) in [
                GenesisButton::Up,
                GenesisButton::Down,
                GenesisButton::Left,
                GenesisButton::Right,
                GenesisButton::A,
                GenesisButton::B,
                GenesisButton::C,
                GenesisButton::Start,
            ]
            .into_iter()
            .enumerate()
            {
                controller.set_field(button, pad & (1 << bit) != 0);
            }
            controller
        });
        Ok(GenesisInputs {
            p1: pads[0],
            p2: pads[1],
        })
    }
}

pub fn run(args: &[String]) {
    let root = Path::new(&args[1]);
    let evidence = Path::new(&args[2]);
    let requested = args.get(5).expect("session mode needs a function name");
    let source = root.join(args.get(6).expect("session mode needs a session path"));
    let session = Session::from_json(&fs::read_to_string(&source).unwrap()).unwrap();
    assert!(
        matches!(session.start, Start::PowerOn),
        "natural-session mode rejects RAM recipes"
    );
    let end = args
        .get(7)
        .map(|value| value.parse::<u64>().unwrap())
        .unwrap_or(session.frames);
    assert!(
        end > 0 && end <= session.frames,
        "end frame outside the actual timeline"
    );
    let timeline = Arc::new(SessionInputs::new(session));
    for revision in [Revision::Rev02, Revision::Rev00] {
        let revision_name = if revision == Revision::Rev02 {
            "rev02"
        } else {
            "rev00"
        };
        if args
            .get(4)
            .is_some_and(|requested| requested != "both" && requested != revision_name)
        {
            continue;
        }
        let bytes = fs::read(root.join(format!("roms/{revision_name}.bin"))).unwrap();
        let registry = crate::replay_registry(revision, requested);
        let mut host = Host::new(&bytes, Mode::Check, registry).unwrap();
        let mut reference =
            Host::new(&bytes, Mode::Reference, HookRegistry::empty(revision)).unwrap();
        host.set_wait_input_poller(timeline.clone());
        reference.set_wait_input_poller(timeline.clone());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for frame in 0..end {
                let inputs = timeline.at(frame).unwrap();
                let actual = host.run_frame(inputs).unwrap();
                let expected = reference.run_frame(inputs).unwrap();
                assert_eq!(actual.pc, expected.pc, "{revision_name} frame {frame} PC");
                assert_eq!(
                    actual.frame_counter, expected.frame_counter,
                    "{revision_name} frame {frame} game counter"
                );
                assert_eq!(
                    actual.ram, expected.ram,
                    "{revision_name} frame {frame} RAM"
                );
                assert_eq!(
                    actual.vdp, expected.vdp,
                    "{revision_name} frame {frame} VDP"
                );
                assert_eq!(
                    actual.writes, expected.writes,
                    "{revision_name} frame {frame} devices"
                );
                assert_eq!(
                    actual.idle_random_draws, expected.idle_random_draws,
                    "{revision_name} frame {frame} RNG"
                );
                assert_eq!(
                    actual.lag_events, expected.lag_events,
                    "{revision_name} frame {frame} lag"
                );
                assert!(
                    actual
                        .audio_samples
                        .iter()
                        .flatten()
                        .map(|sample| sample.to_bits())
                        .eq(expected
                            .audio_samples
                            .iter()
                            .flatten()
                            .map(|sample| sample.to_bits())),
                    "{revision_name} frame {frame} audio"
                );
                assert_eq!(
                    host.frame_buffer().0,
                    reference.frame_buffer().0,
                    "{revision_name} frame {frame} pixels"
                );
            }
            assert!(
                host.checks.calls > 0,
                "zero-hit session is not a root proof"
            );
            assert_eq!(host.checks.mismatches, 0);
        }));
        fs::create_dir_all(evidence).unwrap();
        fs::write(
            evidence.join(format!("{revision_name}-{requested}-natural-session.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "session": source, "source_revision": timeline.session.revision,
                "executed_revision": revision_name, "requested_end_frame": end,
                "natural_power_on": true, "ram_recipes": false,
                "checks": host.checks, "completed": result.is_ok(),
            }))
            .unwrap(),
        )
        .unwrap();
        if let Err(error) = result {
            std::panic::resume_unwind(error);
        }
    }
    // Keep the invocation reconstructible without storing any machine state.
    fs::write(
        evidence.join("invocation.json"),
        serde_json::to_vec_pretty(&env::args().collect::<Vec<_>>()).unwrap(),
    )
    .unwrap();
}
