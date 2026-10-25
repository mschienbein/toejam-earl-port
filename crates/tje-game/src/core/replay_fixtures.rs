//! Opt-in ReplayedEnvironment campaign. Arguments: ROM root, evidence root,
//! natural input JSON, frame count. Run only after the shared envelope lands.
//! All eight eligible core roots are selected together, never per-root replays.
//! Wait uses its readable word-framed body here and independently retains the
//! stronger canonical timed harness. FurtherInitialisation's finite-stage
//! harness is separate; the frozen selector is deliberately not rechecked.
use std::{fs, path::Path, sync::Arc};
use tje_game::{Context, FunctionId, ids};
use tje_host::{Host, Mode, fixture::BoundaryAdvance, hooks::HookRegistry, input::InputScript};
use tje_mem::{Platform, Ptr, Revision, ram};

const ROOTS: &[FunctionId] = &[
    ids::WAIT_FOR_NEXT_V_BLANK,
    ids::CHECK_START_BUTTON,
    ids::PROCESS_INPUT,
    ids::HANDLE_V_BLANK,
    ids::ENTER_SPLIT_SCREEN,
    ids::ENTER_SHARED_SCREEN,
    ids::CENTER_SHARED_CAMERA_ON_PLAYER,
    ids::UPDATE_SCREEN_SPLIT_MODE,
];
fn registry(revision: Revision) -> HookRegistry {
    HookRegistry::proof_replayed_environment(
        revision,
        tje_game::core::PENDING_TRANSLATIONS,
        tje_game::core::PENDING_FRAMED_TRANSLATIONS,
        ROOTS,
    )
    .unwrap()
}
fn quiet(host: &mut Host) {
    let revision = host.revision();
    let mut context = host.context();
    let sr = context.status_register() | 0x0700;
    context.set_status_register(sr);
    ram::g_mode2_reg_softcopy(revision).write(&mut context, 0x20);
    context.write_vdp_control(0x8124);
    ram::g_v_blank_func(revision).write(&mut context, Ptr::new(0));
    ram::g_v_blank_graphics_enabled(revision).write(&mut context, 0);
    ram::g_h_blank_graphics_enabled(revision).write(&mut context, 0);
    ram::g_v_blank_graphics_paused(revision).write(&mut context, 1);
    ram::g_v_blank_audio_input_override(revision).write(&mut context, 0);
    ram::g_player_inventory_open(revision)
        .at(0)
        .write(&mut context, 0);
    ram::g_player_inventory_open(revision)
        .at(1)
        .write(&mut context, 0);
}
fn settling(advance: &BoundaryAdvance) -> serde_json::Value {
    let frames = advance.completed_frames.iter().map(|frame| serde_json::json!({
        "frame":frame.frame,"frame_counter":frame.frame_counter,"pc":frame.pc,
        "ram":frame.ram.as_ref(),
        "vdp":{"vram":frame.vdp.vram.as_ref(),"cram":frame.vdp.cram.as_ref(),
            "vsram":frame.vdp.vsram.as_ref(),"registers":frame.vdp.registers},
        "writes":frame.writes,"idle_draws":frame.idle_random_draws,"lag":frame.lag_events,
        "sample_bits":frame.audio_samples.iter().map(|sample| [sample[0].to_bits(),sample[1].to_bits()]).collect::<Vec<_>>()
    })).collect::<Vec<_>>();
    serde_json::json!({"start_pc":advance.start_pc,"end_pc":advance.end_pc,
        "start_frame":advance.start_frame,"end_frame":advance.end_frame,"ticks":advance.ticks,
        "master_clocks":advance.master_clocks,"handlers_completed":advance.handlers_completed,
        "completed_frames":frames})
}
fn attempt(
    host: &mut Host,
    rows: &mut Vec<serde_json::Value>,
    id: FunctionId,
    label: &str,
    arguments: &[u32],
    output: &Path,
) {
    let result = host.check_fixture(id.metadata().address(host.revision()), arguments);
    rows.push(
        serde_json::json!({"function":id.metadata().name,"label":label,
        "coverage":"labelled real-state fixture","arguments":arguments,
        "matched":result.is_ok(),"error":result.err().map(|error|error.to_string())}),
    );
    fs::write(
        output,
        serde_json::to_vec_pretty(&serde_json::json!({
            "revision":host.revision().name(),"phase":"fixture progress; failures retained",
            "attempts":rows,"checks":host.checks,
        }))
        .unwrap(),
    )
    .unwrap();
}
fn fixtures(host: &mut Host, output: &Path) -> Vec<serde_json::Value> {
    let revision = host.revision();
    let mut rows = Vec::new();
    quiet(host);
    for count in [0x12340001, 0x12340000, 0x1234ffff, 0x12348000] {
        attempt(
            host,
            &mut rows,
            ids::WAIT_FOR_NEXT_V_BLANK,
            "display-off lazy word argument, upper slot word ignored",
            &[count],
            output,
        );
    }
    ram::g_mode2_reg_softcopy(revision).write(&mut host.context(), 0x40);
    for count in [0x12340000, 0x1234ffff, 0x12348000] {
        attempt(
            host,
            &mut rows,
            ids::WAIT_FOR_NEXT_V_BLANK,
            "display-on nonpositive word count without fabricated clock",
            &[count],
            output,
        );
    }
    ram::g_mode2_reg_softcopy(revision).write(&mut host.context(), 0x20);
    for (first, second, old_first, old_second) in [
        (0, 0, 0, 0),
        (0x80, 0, 0x80, 0),
        (0, 0x80, 0, 0x80),
        (0x80, 0x80, 0x80, 0),
    ] {
        let mut context = host.context();
        ram::g_player_processed_input(revision)
            .at(0)
            .write(&mut context, first);
        ram::g_player_processed_input(revision)
            .at(1)
            .write(&mut context, second);
        ram::g_player_previous_processed_input(revision)
            .at(0)
            .write(&mut context, old_first);
        ram::g_player_previous_processed_input(revision)
            .at(1)
            .write(&mut context, old_second);
        attempt(
            host,
            &mut rows,
            ids::CHECK_START_BUTTON,
            "held/no-new-press and simultaneous priority",
            &[],
            output,
        );
    }
    ram::g_controller1_raw_input(revision).write(&mut host.context(), 0);
    ram::g_controller2_raw_input(revision).write(&mut host.context(), 0);
    attempt(
        host,
        &mut rows,
        ids::PROCESS_INPUT,
        "live living players, no held buttons",
        &[],
        output,
    );
    let first_lives = ram::g_player_lives(revision).at(0).read(&host.context());
    let second_lives = ram::g_player_lives(revision).at(1).read(&host.context());
    let level = ram::g_player1_map_level(revision).read(&host.context());
    ram::g_player_lives(revision)
        .at(0)
        .write(&mut host.context(), -1);
    ram::g_player_lives(revision)
        .at(1)
        .write(&mut host.context(), -1);
    ram::g_player1_map_level(revision).write(&mut host.context(), 26);
    attempt(
        host,
        &mut rows,
        ids::PROCESS_INPUT,
        "both absent, final-level menu gate",
        &[],
        output,
    );
    ram::g_player_lives(revision)
        .at(0)
        .write(&mut host.context(), first_lives);
    ram::g_player_lives(revision)
        .at(1)
        .write(&mut host.context(), second_lives);
    ram::g_player1_map_level(revision).write(&mut host.context(), level);
    for player in 0..2 {
        ram::g_player_lives(revision)
            .at(player)
            .write(&mut host.context(), 3);
        let entity = ram::g_player_info(revision).at(player);
        entity.level().write(&mut host.context(), 1);
        entity
            .x()
            .write(&mut host.context(), (1200 + player * 40) as i16);
        entity
            .y()
            .write(&mut host.context(), (800 + player * 40) as i16);
        entity.height_pixels().write(&mut host.context(), 32);
        entity.height().write(&mut host.context(), 0);
    }
    for player in [0_i16, 1, -1, 512, 513] {
        attempt(
            host,
            &mut rows,
            ids::CENTER_SHARED_CAMERA_ON_PLAYER,
            "word-scaled selector and negative/wrapped selectors, display off",
            &[i32::from(player) as u32],
            output,
        );
    }
    attempt(
        host,
        &mut rows,
        ids::ENTER_SPLIT_SCREEN,
        "display-off nonanimated split",
        &[0],
        output,
    );
    attempt(
        host,
        &mut rows,
        ids::ENTER_SHARED_SCREEN,
        "display-off midpoint and shared mirrors",
        &[40, 40],
        output,
    );
    for force in [0, 1, 0x100] {
        attempt(
            host,
            &mut rows,
            ids::UPDATE_SCREEN_SPLIT_MODE,
            "close living players and char-slot narrowing",
            &[force],
            output,
        );
    }
    for counter in [0_u16, i16::MAX as u16, 0xffff] {
        quiet(host);
        ram::g_frame_counter(revision).write(&mut host.context(), counter);
        attempt(
            host,
            &mut rows,
            ids::HANDLE_V_BLANK,
            "quiet counter wrap/overflow without callback",
            &[],
            output,
        );
    }
    rows
}
fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    assert_eq!(
        arguments.len(),
        4,
        "ROM root, output root, natural input JSON, frames"
    );
    let script = Arc::new(InputScript::parse(&fs::read_to_string(&arguments[2]).unwrap()).unwrap());
    let frames: u64 = arguments[3].parse().unwrap();
    assert!(
        frames >= 10000,
        "retain the complete standard natural campaign"
    );
    let mut passed = true;
    for (file, revision) in [("rev02", Revision::Rev02), ("rev00", Revision::Rev00)] {
        let bytes = fs::read(Path::new(&arguments[0]).join(format!("{file}.bin"))).unwrap();
        let output = Path::new(&arguments[1]).join(format!("core-replay-prep-{file}.json"));
        let mut host = Host::new(&bytes, Mode::Check, registry(revision)).unwrap();
        let mut reference =
            Host::new(&bytes, Mode::Reference, HookRegistry::empty(revision)).unwrap();
        host.set_wait_input_poller(script.clone());
        reference.set_wait_input_poller(script.clone());
        let mut failure = None;
        let mut completed = 0;
        for frame in 0..frames {
            let actual = match host.run_frame(script.at(frame)) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    failure = Some(serde_json::json!({"frame":frame,"error":error.to_string()}));
                    break;
                }
            };
            let expected = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(actual.ram, expected.ram);
            assert_eq!(actual.vdp, expected.vdp);
            assert_eq!(actual.writes, expected.writes);
            assert_eq!(actual.frame_counter, expected.frame_counter);
            assert_eq!(actual.idle_random_draws, expected.idle_random_draws);
            assert_eq!(actual.lag_events, expected.lag_events);
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
                        .map(|sample| sample.to_bits()))
            );
            let (actual, size) = host.frame_buffer();
            let (expected, expected_size) = reference.frame_buffer();
            assert_eq!(size, expected_size);
            assert_eq!(
                &actual[..size.len() as usize],
                &expected[..expected_size.len() as usize]
            );
            completed += 1;
        }
        let natural = serde_json::to_value(&host.checks).unwrap();
        let mut boundary = None;
        let rows = if failure.is_none() {
            assert!(host.measurements.game_frame.is_some());
            let advance = host
                .settle_fixture_boundary(script.at(host.frame_count()))
                .unwrap();
            boundary = Some(settling(&advance));
            fixtures(&mut host, &output)
        } else {
            Vec::new()
        };
        let ok = failure.is_none()
            && rows.iter().all(|row| row["matched"] == true)
            && host.checks.mismatches == 0
            && host.checks.unexercised.is_empty();
        passed &= ok;
        fs::write(&output, serde_json::to_vec_pretty(&serde_json::json!({
            "revision":revision.name(),"envelope":"ReplayedEnvironment","production_eligible":false,
            "campaign":"one pass selecting all eight replay-compatible roots; nested live calls are checked naturally",
            "modularity":"catalogued callees use actual original JSR/RTS segments, not an all-Rust composition proof",
            "input":arguments[2],"requested_frames":frames,"completed_frames":completed,
            "failure":failure,"natural_checks":natural,"settling":boundary,"fixtures":rows,
            "checks":host.checks,"measurements":host.measurements,"passed":ok,
        })).unwrap()).unwrap();
        println!(
            "{} {file} one-pass core replay campaign",
            if ok { "PASS" } else { "BLOCKED" }
        );
    }
    assert!(passed, "failures retained in both-revision evidence");
}
