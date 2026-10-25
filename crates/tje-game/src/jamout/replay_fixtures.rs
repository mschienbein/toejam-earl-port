//! Opt-in check-only modular replay runner, not a shipping crate target.
//! Args: ROM directory, session directory, output directory, natural|fixtures.
//! Each natural session hooks all seven roots once; nested original execution
//! produces independent root checks, never an all-Rust composition claim.
use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use serde_json::{Value, json};
use std::{fs, path::Path, sync::Arc};
use tje_game::{FunctionId, ids};
use tje_host::{
    CheckFailurePolicy, FrameSnapshot, Host, HostError, Mode, execution::WaitInputPoller,
    fixture::BoundaryAdvance, hooks::HookRegistry, input::InputScript,
};
use tje_mem::{Bus, Revision, ram};
use tje_session::{CONNECTED, Session};

const ROOTS: &[FunctionId] = &[
    ids::START_JAM_OUT,
    ids::JAM_OUT_V_BLANK,
    ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK,
    ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL,
    ids::JAM_OUT_SHOW_LEAD_PAIR_LAYOUT,
    ids::JAM_OUT_SHOW_FOUR_PAIR_LAYOUT,
    ids::JAM_OUT_SHOW_NINE_PAIR_LAYOUT,
];

fn registry(revision: Revision) -> HookRegistry {
    HookRegistry::proof_replayed_environment(
        revision,
        tje_game::jamout::PENDING_TRANSLATIONS,
        tje_game::jamout::PENDING_FRAMED_TRANSLATIONS,
        ROOTS,
    )
    .unwrap()
}

// Preserve the actual recording's connection bits and recipe semantics, including
// wait-time requests beyond the frame currently driven by the outer runner.
struct Timeline {
    session: Session,
    ends: Vec<u64>,
}
impl Timeline {
    fn load(path: &Path) -> Self {
        let session = Session::from_json(&fs::read_to_string(path).unwrap()).unwrap();
        let mut sum = 0;
        let ends = session
            .runs
            .iter()
            .map(|run| {
                sum += run.0;
                sum
            })
            .collect();
        Self { session, ends }
    }
    fn sample(&self, frame: u64) -> tje_host::Result<GenesisInputs> {
        let index = self.ends.partition_point(|end| *end <= frame);
        let run = self
            .session
            .runs
            .get(index)
            .ok_or_else(|| HostError::Input(format!("recording has no frame {frame}")))?;
        Ok(inputs([run.1, run.2]))
    }
}
impl WaitInputPoller for Timeline {
    fn at(&self, frame: u64) -> tje_host::Result<GenesisInputs> {
        self.sample(frame)
    }
    fn before_frame(&self, frame: u64, bus: &mut dyn Bus) -> tje_host::Result<()> {
        for write in self
            .session
            .writes()
            .iter()
            .filter(|write| write.before_frame == frame)
        {
            match write.width {
                1 => bus.write_u8(write.address, write.value as u8),
                2 => bus.write_u16(write.address, write.value as u16),
                4 => bus.write_u32(write.address, write.value),
                _ => unreachable!("validated session write"),
            }
        }
        Ok(())
    }
}
fn inputs(pads: [u16; 2]) -> GenesisInputs {
    use GenesisButton::*;
    let controllers = pads.map(|pad| {
        let mut controller = if pad & CONNECTED == 0 {
            GenesisController::None
        } else {
            GenesisController::new(GenesisControllerType::ThreeButton)
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
        p1: controllers[0],
        p2: controllers[1],
    }
}
fn assert_same(a: &FrameSnapshot, b: &FrameSnapshot) {
    assert_eq!(a.ram, b.ram);
    assert_eq!(a.vdp, b.vdp);
    assert_eq!(a.writes, b.writes);
    assert_eq!(a.frame_counter, b.frame_counter);
    assert_eq!(a.idle_random_draws, b.idle_random_draws);
    assert_eq!(a.lag_events, b.lag_events);
    assert!(
        a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
            .audio_samples
            .iter()
            .flatten()
            .map(|v| v.to_bits()))
    );
}
fn boundary_json(advance: BoundaryAdvance) -> Value {
    let frames = advance.completed_frames.iter().map(|frame| json!({
        "frame":frame.frame,"frame_counter":frame.frame_counter,"pc":frame.pc,
        "ram":frame.ram.as_ref(),"vdp":{"vram":frame.vdp.vram.as_ref(),
        "cram":frame.vdp.cram.as_ref(),"vsram":frame.vdp.vsram.as_ref(),"registers":frame.vdp.registers},
        "writes":frame.writes,"idle_random_draws":frame.idle_random_draws,"lag_events":frame.lag_events,
        "audio_sample_bits":frame.audio_samples.iter().map(|s| [s[0].to_bits(),s[1].to_bits()]).collect::<Vec<_>>(),
    })).collect::<Vec<_>>();
    json!({"start_pc":advance.start_pc,"end_pc":advance.end_pc,"start_frame":advance.start_frame,
        "end_frame":advance.end_frame,"ticks":advance.ticks,"master_clocks":advance.master_clocks,
        "handlers_completed":advance.handlers_completed,"completed_frames":frames})
}
fn natural(bytes: &[u8], revision: Revision, timeline: Arc<Timeline>) -> Value {
    let mut checked = Host::new(bytes, Mode::Check, registry(revision)).unwrap();
    checked
        .set_check_failure_policy(CheckFailurePolicy::ContinueOriginal)
        .unwrap();
    let mut original = Host::new(bytes, Mode::Reference, HookRegistry::empty(revision)).unwrap();
    checked.set_wait_input_poller(timeline.clone());
    original.set_wait_input_poller(timeline.clone());
    let mut completed = 0;
    let mut failure = None;
    for frame in 0..timeline.session.frames {
        timeline
            .before_frame(frame, &mut checked.context())
            .unwrap();
        timeline
            .before_frame(frame, &mut original.context())
            .unwrap();
        let input = timeline.sample(frame).unwrap();
        let a = match checked.run_frame(input) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                failure = Some(json!({"frame":frame,"pc":checked.cpu().pc(),
                "sp":checked.cpu().stack_pointer(),"error":error.to_string()}));
                break;
            }
        };
        let b = original.run_frame(input).unwrap();
        assert_same(&a, &b);
        let (a, sa) = checked.frame_buffer();
        let (b, sb) = original.frame_buffer();
        assert_eq!(sa, sb);
        assert_eq!(&a[..sa.len() as usize], &b[..sb.len() as usize]);
        completed += 1;
    }
    json!({"coverage_source":"natural recorded session","completed_frames":completed,
        "requested_frames":timeline.session.frames,"failure":failure,"checks":checked.checks,
        "measurements":checked.measurements})
}
fn check(
    host: &mut Host,
    root: FunctionId,
    label: Value,
    arguments: &[u32],
    rows: &mut Vec<Value>,
) {
    let result = host.check_fixture(root.metadata().address(host.revision()), arguments);
    rows.push(
        json!({"coverage_source":"labelled live-state fixture","label":label,
        "function":root.metadata().name,"arguments":arguments,"ok":result.is_ok(),
        "error":result.err().map(|error| error.to_string())}),
    );
}
fn fixtures(bytes: &[u8], revision: Revision, timeline: Arc<Timeline>, root: FunctionId) -> Value {
    let mut host = Host::new(bytes, Mode::Reference, registry(revision)).unwrap();
    host.set_wait_input_poller(timeline.clone());
    // A menu state before Jam Out allocation for Start; all other cases retain
    // the natural scene's real copies, driver, controller and graphics state.
    let seed_frame = if root == ids::START_JAM_OUT {
        700
    } else {
        1200
    };
    for frame in 0..seed_frame {
        timeline.before_frame(frame, &mut host.context()).unwrap();
        host.run_frame(timeline.sample(frame).unwrap()).unwrap();
    }
    let advance = host
        .settle_fixture_boundary(timeline.sample(host.frame_count()).unwrap())
        .unwrap();
    let settling = boundary_json(advance);
    let mut rows = Vec::new();
    if root == ids::START_JAM_OUT {
        let start = host.frame_count();
        let script = InputScript::parse(
            &json!({"holds":[{"start":start+8,"end":start+608,"p1":["start"]}]}).to_string(),
        )
        .unwrap();
        host.set_wait_input_poller(Arc::new(script));
        for song in [0, 1, 7] {
            check(
                &mut host,
                root,
                json!({"scene":"whole modal return","song":song,
                "future_start_hold":[start+8,start+608],"no_forced_exit":true}),
                &[song],
                &mut rows,
            );
        }
    } else if [ids::JAM_OUT_V_BLANK, ids::JAM_OUT_RANDOM_SAMPLE_V_BLANK].contains(&root) {
        for (selection, swapped, counter, pending, song_cycle, template_cycle) in [
            (0, 0, 0, 0, 0, 0),
            (0, 1, 1, 0x80, 6, 0),
            (1, 0, u16::MAX, 0xff, 7, 2),
            (1, 1, 0, 0x70, 0, 1),
            (0, 0, 1, 0x0f, 1, 2),
            (0, 1, 0, 0x80, 7, 1),
        ] {
            let mut context = host.context();
            ram::g_player_selection(revision).write(&mut context, selection);
            ram::g_controller_ports_swapped(revision).write(&mut context, swapped);
            ram::g_frame_counter(revision).write(&mut context, counter);
            ram::g_jam_audio_route1_pending_input(revision).write(&mut context, pending);
            ram::g_jam_audio_route2_pending_input(revision).write(&mut context, pending);
            ram::g_jam_song_cycle_index(revision).write(&mut context, song_cycle);
            ram::g_jam_template_bank_cycle_index(revision).write(&mut context, template_cycle);
            check(
                &mut host,
                root,
                json!({"entry":"prepared RTS callback, not an IRQ/RTE fixture",
                "selection":selection,"swapped":swapped,"counter":counter,"pending":pending,
                "song_cycle":song_cycle,"template_cycle":template_cycle,
                "device_state":"natural live driver, no BUSACK/lock/YM readiness fabrication"}),
                &[],
                &mut rows,
            );
        }
    } else if root == ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL {
        for phase in [i16::MIN, -1, 0, 1, 2, 3, 4, 5, i16::MAX] {
            for countdown in [0, 1, 2] {
                for step in [-1, 0, 1] {
                    let mut context = host.context();
                    ram::g_jam_layout_phase(revision).write(&mut context, phase);
                    ram::g_jam_layout_countdown(revision).write(&mut context, countdown);
                    ram::g_jam_scroll_step(revision).write(&mut context, step);
                    ram::g_jam_horizontal_scroll(revision).write(&mut context, i16::MAX);
                    ram::g_jam_vertical_scroll(revision).write(&mut context, i16::MIN);
                    check(
                        &mut host,
                        root,
                        json!({"display":"natural on","phase":phase,
                        "countdown":countdown,"step":step,"scroll":"word overflow boundaries"}),
                        &[0],
                        &mut rows,
                    );
                }
            }
        }
        check(
            &mut host,
            root,
            json!({"display":"natural on","path":"full reset"}),
            &[1],
            &mut rows,
        );
    } else {
        check(
            &mut host,
            root,
            json!({"display":"natural on","copies":"natural live allocated handles"}),
            &[],
            &mut rows,
        );
        for slot in 0..9 {
            ram::g_jam_toejam_entities(revision)
                .at(slot)
                .write(&mut host.context(), 0);
            ram::g_jam_earl_entities(revision)
                .at(slot)
                .write(&mut host.context(), 0);
        }
        check(
            &mut host,
            root,
            json!({"display":"natural on","copies":"authored absent handles"}),
            &[],
            &mut rows,
        );
    }
    json!({"coverage_source":"labelled fixtures after original-only recorded prefix",
        "seed_frame":seed_frame,"settling":settling,"attempts":rows,"checks":host.checks,
        "measurements":host.measurements})
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert_eq!(
        args.len(),
        4,
        "ROM directory, session directory, output directory, natural|fixtures"
    );
    fs::create_dir_all(&args[2]).unwrap();
    let mut ok = true;
    for (tag, revision) in [("rev02", Revision::Rev02), ("rev00", Revision::Rev00)] {
        let bytes = fs::read(Path::new(&args[0]).join(format!("{tag}.bin"))).unwrap();
        let session_path = Path::new(&args[1]).join(format!("{tag}-jam-all.tje.json"));
        let timeline = Arc::new(Timeline::load(&session_path));
        let (evidence, successful) = match args[3].as_str() {
            "natural" => {
                let evidence = natural(&bytes, revision, timeline);
                let hit_all = ROOTS.iter().all(|id| {
                    evidence["checks"]["by_function"][id.metadata().name]["calls_checked"]
                        .as_u64()
                        .unwrap_or(0)
                        != 0
                });
                let successful = evidence["failure"].is_null()
                    && evidence["completed_frames"] == evidence["requested_frames"]
                    && hit_all
                    && evidence["checks"]["mismatches"].as_u64() == Some(0);
                (evidence, successful)
            }
            "fixtures" => {
                let cases = ROOTS
                    .iter()
                    .map(|&root| fixtures(&bytes, revision, timeline.clone(), root))
                    .collect::<Vec<_>>();
                let successful = cases.iter().all(|case| {
                    case["attempts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|row| row["ok"] == true)
                });
                (json!(cases), successful)
            }
            _ => panic!("expected natural|fixtures"),
        };
        ok &= successful;
        let report = json!({"purpose":"check-only modular replay evidence; not live replacement or status authority",
            "revision":revision.name(),"functions":ROOTS.iter().map(|id| id.metadata().name).collect::<Vec<_>>(),
            "session":session_path,"successful":successful,"evidence":evidence});
        fs::write(
            Path::new(&args[2]).join(format!("{}-jamout-{tag}.json", args[3])),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
    if !ok {
        std::process::exit(1);
    }
}
