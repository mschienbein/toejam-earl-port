//! Opt-in title/transition ReplayedEnvironment campaign, never linked into tje-game.
//! Private manifest dependencies: tje-game, tje-host, tje-mem, tje-session,
//! genesis-config and serde_json. Arguments: ROM METRICS_JSON SESSION_JSON...
//! Requires the frozen proof_replayed_environment API; there is no old-tier fallback.
//! All ten roots are checked in each single natural campaign pass. Original callee
//! segments are modular evidence, never claimed as Rust composition or replacement.
use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use serde_json::{Value, json};
use std::{collections::BTreeMap, error::Error, fs, sync::Arc};
use tje_game::{FunctionId, ids};
use tje_host::{
    CheckFailurePolicy, Host, HostError, Mode, execution::WaitInputPoller, hooks::HookRegistry,
};
use tje_mem::{Bus, Revision, ram};
use tje_session::{CONNECTED, Session};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

const ROOTS: &[FunctionId] = &[
    ids::ANIMATE_TITLE_TRANSITION_BACKGROUND,
    ids::INITIALIZE_TITLE_TRANSITION_TILEMAP,
    ids::CLEAR_UPPER_VRAM_TILEMAP,
    ids::CLEAR_LOWER_VRAM_TILEMAP,
    ids::TITLE_DRAW_SELECTED_LOGO,
    ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
    ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
    ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
    ids::TITLE_SCREEN_VIDEO,
    ids::SEGA_AND_INTRO_VIDEO_ROUTINE,
];
const FIXTURE_TAIL_FRAMES: u64 = 16_384;

#[derive(Clone, Copy)]
enum Tail {
    Released,
    TitlePressRelease,
    AttractPressRelease,
}
struct Recording {
    session: Session,
    run_ends: Vec<u64>,
    tail_connections: [u16; 2],
}
struct Timeline {
    recording: Arc<Recording>,
    tail: Tail,
}
impl Timeline {
    fn new(session: Session) -> Self {
        let mut end = 0;
        let run_ends = session
            .runs
            .iter()
            .map(|run| {
                end += run.0;
                end
            })
            .collect();
        let tail_connections = session
            .runs
            .last()
            .map_or([CONNECTED, 0], |run| [run.1 & CONNECTED, run.2 & CONNECTED]);
        Self {
            recording: Arc::new(Recording {
                session,
                run_ends,
                tail_connections,
            }),
            tail: Tail::Released,
        }
    }
    fn with_tail(&self, tail: Tail) -> Self {
        Self {
            recording: Arc::clone(&self.recording),
            tail,
        }
    }
    fn pads(&self, frame: u64) -> tje_host::Result<[u16; 2]> {
        let recording = &self.recording;
        if frame < recording.session.frames {
            let index = recording.run_ends.partition_point(|&end| end <= frame);
            let run = recording.session.runs[index];
            return Ok([run.1, run.2]);
        }
        let offset = frame - recording.session.frames;
        if offset >= FIXTURE_TAIL_FRAMES {
            return Err(HostError::Input(format!(
                "no authored title fixture input for frame {frame}"
            )));
        }
        // Explicit authored extension, not implicit reuse of the recording's last buttons.
        let mut pads = recording.tail_connections;
        let start = match self.tail {
            Tail::Released => false,
            Tail::TitlePressRelease => (20..40).contains(&offset),
            Tail::AttractPressRelease => (64..72).contains(&(offset % 128)),
        };
        if start && pads[0] & CONNECTED != 0 {
            pads[0] |= tje_session::START;
        }
        Ok(pads)
    }
}
fn inputs(pads: [u16; 2]) -> GenesisInputs {
    use GenesisButton::*;
    let controllers = pads.map(|pad| {
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
        p1: controllers[0],
        p2: controllers[1],
    }
}
impl WaitInputPoller for Timeline {
    fn at(&self, frame: u64) -> tje_host::Result<GenesisInputs> {
        Ok(inputs(self.pads(frame)?))
    }
    fn before_frame(&self, frame: u64, memory: &mut dyn Bus) -> tje_host::Result<()> {
        let writes = self.recording.session.writes();
        let first = writes.partition_point(|write| write.before_frame < frame);
        for write in &writes[first..] {
            if write.before_frame != frame {
                break;
            }
            match write.width {
                1 => memory.write_u8(write.address, write.value as u8),
                2 => memory.write_u16(write.address, write.value as u16),
                4 => memory.write_u32(write.address, write.value),
                _ => unreachable!("Session::from_json validated recipe widths"),
            }
        }
        Ok(())
    }
}
fn registry(revision: Revision) -> tje_host::Result<HookRegistry> {
    HookRegistry::proof_replayed_environment(
        revision,
        tje_game::cutscenes::PENDING_TRANSLATIONS,
        tje_game::cutscenes::PENDING_FRAMED_TRANSLATIONS,
        ROOTS,
    )
}
fn check(host: &mut Host, id: FunctionId, args: &[u32], label: &str, cases: &mut Vec<Value>) {
    let result = host.check_fixture(id.metadata().address(host.revision()), args);
    let (d0, error) = match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error.to_string())),
    };
    cases.push(
        json!({ "function": id.metadata().name, "label": label, "arguments": args,
        "d0": d0, "error": error }),
    );
}
fn fixtures(host: &mut Host, timeline: &Arc<Timeline>) -> Result<Value> {
    let revision = host.revision();
    let boundary = host.settle_fixture_boundary(timeline.at(host.frame_count())?)?;
    // The full boundary snapshots remain owned here through all fixture checks.
    // Export metadata only: no RAM, VDP, pixels, PCM or raw trace is published.
    let mut cases = Vec::new();
    check(
        host,
        ids::CLEAR_UPPER_VRAM_TILEMAP,
        &[],
        "settled real-state plane-B clear",
        &mut cases,
    );
    check(
        host,
        ids::CLEAR_LOWER_VRAM_TILEMAP,
        &[],
        "settled real-state plane-A clear; timed source preserved",
        &mut cases,
    );
    for (restart, timer, phase) in [
        (1_u32, 8_i8, 1_i8),
        (0, 3, 1),
        (0, 1, 1),
        (0, 1, 2),
        (0, i8::MIN, 1),
    ] {
        let mut context = host.context();
        ram::g_scene_background_frame_timer(revision).write(&mut context, timer);
        ram::g_scene_background_frame_phase(revision).write(&mut context, phase);
        ram::g_scene_background_pair_index(revision).write(&mut context, 0);
        ram::g_sprite_tile_buffer_offset(revision).write(&mut context, 0);
        ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut context, 0);
        check(
            host,
            ids::ANIMATE_TITLE_TRANSITION_BACKGROUND,
            &[restart],
            "restart/no-change/expiry/byte-overflow animation paths",
            &mut cases,
        );
    }
    for attributes in [0_u16, 0x8000] {
        ram::g_camera_tile_attributes(revision).write(&mut host.context(), attributes);
        ram::g_scene_background_pair_index(revision).write(&mut host.context(), 0);
        check(
            host,
            ids::INITIALIZE_TITLE_TRANSITION_TILEMAP,
            &[],
            "both authored camera-priority attributes",
            &mut cases,
        );
    }
    for selector in [0, 1] {
        ram::g_title_logo_selector(revision).write(&mut host.context(), selector);
        check(
            host,
            ids::TITLE_DRAW_SELECTED_LOGO,
            &[],
            "both ROM-backed logo branches",
            &mut cases,
        );
    }
    for clear in [0_i32, 1, -1] {
        ram::g_rng_state(revision).write(&mut host.context(), 12345);
        check(
            host,
            ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
            &[clear as u32],
            "border-only and clearing modes, including signed truth",
            &mut cases,
        );
    }
    for rectangle in [
        [11_i32, 7, 30, 20],
        [4, 5, 4, 5],
        [2, 3, 1, 4],
        [0, 4, 3, 2],
        [-2, -1, 1, 1],
    ] {
        ram::g_rng_state(revision).write(&mut host.context(), 12345);
        check(
            host,
            ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER,
            &rectangle.map(|value| value as u32),
            "normal/one-cell/empty-column/empty-row/negative rectangle",
            &mut cases,
        );
    }
    for (event, updates, period, advance, animate, lookback) in [
        (0_i8, 0_i16, 0_u16, 0_u16, 0_i16, 100_u16),
        (1, 0, 0, 0, 0, 0),
        (1, 0, 0, 0, 0, 100),
        (1, 51, 0, 0, 0, 100),
        (1, 0, 2, 0, 1, 100),
        (1, 0, 1, 0, 0, 100),
    ] {
        let counter = ram::g_frame_counter(revision).read(&host.context());
        let mut context = host.context();
        ram::g_music_animation_event(revision).write(&mut context, event);
        ram::g_scene_update_counter(revision).write(&mut context, updates);
        ram::g_scene_background_advance_counter(revision).write(&mut context, advance);
        ram::g_scene_last_music_event_time(revision)
            .write(&mut context, counter.wrapping_sub(lookback));
        ram::g_scene_background_pair_index(revision).write(&mut context, 23);
        ram::g_intro_large_player_part_entities(revision)
            .at(2)
            .write(&mut context, 6);
        check(
            host,
            ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
            &[u32::from(period), animate as i32 as u32],
            "authored event/update/advance/actor inputs; environment may update event before its actual read",
            &mut cases,
        );
    }
    host.set_wait_input_poller(Arc::new(timeline.with_tail(Tail::Released)));
    check(
        host,
        ids::TITLE_SCREEN_VIDEO,
        &[],
        "explicit released-input title timeout",
        &mut cases,
    );
    host.set_wait_input_poller(Arc::new(timeline.with_tail(Tail::TitlePressRelease)));
    check(
        host,
        ids::TITLE_SCREEN_VIDEO,
        &[],
        "explicit Start press/release title exit",
        &mut cases,
    );
    host.set_wait_input_poller(Arc::new(timeline.with_tail(Tail::AttractPressRelease)));
    check(
        host,
        ids::SEGA_AND_INTRO_VIDEO_ROUTINE,
        &[],
        "finite periodic Start schedule across original scene segments",
        &mut cases,
    );
    let completed_frames: Vec<_> = boundary
        .completed_frames
        .iter()
        .map(|frame| {
            json!({
                "frame": frame.frame, "frame_counter": frame.frame_counter,
                "writes": frame.writes.len(), "audio_samples": frame.audio_samples.len(),
                "idle_random_draws": frame.idle_random_draws, "lag_events": frame.lag_events,
            })
        })
        .collect();
    Ok(json!({ "boundary": {
        "start_pc": boundary.start_pc, "end_pc": boundary.end_pc,
        "start_frame": boundary.start_frame, "end_frame": boundary.end_frame,
        "ticks": boundary.ticks, "master_clocks": boundary.master_clocks,
        "handlers_completed": boundary.handlers_completed, "completed_frames": completed_frames,
    }, "cases": cases }))
}
fn main() -> Result {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 4 {
        return Err("usage: ROM METRICS_JSON SESSION_JSON...".into());
    }
    let bytes = fs::read(&args[1])?;
    let mut reports = Vec::new();
    let mut totals: BTreeMap<&str, (u64, u64)> = ROOTS
        .iter()
        .map(|id| (id.metadata().name, (0, 0)))
        .collect();
    let mut fixture_done = false;
    let mut failed = false;
    for path in &args[3..] {
        let session = Session::from_json(&fs::read_to_string(path)?)?;
        let revision: Revision = session.revision.into();
        let frames = session.frames;
        let timeline = Arc::new(Timeline::new(session));
        let mut host = Host::new(&bytes, Mode::Check, registry(revision)?)?;
        host.set_check_failure_policy(CheckFailurePolicy::ContinueOriginal)?;
        host.set_wait_input_poller(timeline.clone());
        let mut natural_error = None;
        while host.frame_count() < frames {
            let frame = host.frame_count();
            timeline.before_frame(frame, &mut host.context())?;
            if let Err(error) = host.run_frame(timeline.at(frame)?) {
                natural_error = Some(error.to_string());
                break;
            }
        }
        let natural = serde_json::to_value(&host.checks)?;
        let mut fixture_report = None;
        let mut fixture_error = None;
        if !fixture_done && host.measurements.game_frame.is_some() {
            fixture_done = true;
            match fixtures(&mut host, &timeline) {
                Ok(report) => {
                    failed |= report["cases"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|case| !case["error"].is_null());
                    fixture_report = Some(report);
                }
                Err(error) => fixture_error = Some(error.to_string()),
            }
        }
        for (name, coverage) in host.checks.by_function.iter() {
            let total = totals
                .get_mut(name)
                .expect("registry is scoped to the ten title roots");
            total.0 += coverage.calls_checked;
            total.1 += coverage.mismatches;
        }
        failed |= natural_error.is_some() || fixture_error.is_some() || host.checks.mismatches != 0;
        reports.push(json!({ "session": path, "revision": revision.name(),
            "recorded_frames": frames, "completed_frames": host.frame_count(),
            "natural_error": natural_error, "natural": natural,
            "fixture_report": fixture_report, "fixture_error": fixture_error,
            "checks": host.checks, "measurements": host.measurements,
        }));
    }
    let unexercised: Vec<_> = totals
        .iter()
        .filter(|(_, (calls, _))| *calls == 0)
        .map(|(&name, _)| name)
        .collect();
    failed |= !fixture_done || !unexercised.is_empty();
    fs::write(
        &args[2],
        serde_json::to_string_pretty(&json!({
            "tier": "ReplayedEnvironment; modular caller proof; no composition or live replacement eligibility",
            "authored_fixture_tail": "released extension at recorded connectivity; title Start20..40; attract Start64..72 modulo128; finite16384frames",
            "fixtures_exercised": fixture_done, "totals": totals, "unexercised": unexercised,
            "failed": failed, "sessions": reports,
        }))?,
    )?;
    if failed {
        Err("replay campaign has failed or unexercised title roots; see metrics".into())
    } else {
        Ok(())
    }
}
