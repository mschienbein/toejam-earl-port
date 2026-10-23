//! Private-ROM differential proof. Inputs are committed controller sessions;
//! labelled fixtures change live gameplay state without publishing state bytes.

use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::Path,
    sync::{Arc, OnceLock},
};
use tje_game::{FunctionId, Translation, ids, ui::hud_all};
use tje_host::{Host, Mode, execution::WaitInputPoller, hooks::HookRegistry};
use tje_mem::{Revision, ram};
use tje_session::{CONNECTED, Session, Start};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

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

    fn inputs(&self, frame: u64) -> tje_host::Result<GenesisInputs> {
        let index = self.ends.partition_point(|&end| end <= frame);
        let run = self.session.runs.get(index).ok_or_else(|| {
            tje_host::HostError::Input(format!("no recorded controller sample at frame {frame}"))
        })?;
        let pads = [run.1, run.2].map(|pad| {
            let mut controller = if pad & CONNECTED != 0 {
                GenesisController::new(GenesisControllerType::ThreeButton)
            } else {
                GenesisController::None
            };
            for (index, button) in [
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
                controller.set_field(button, pad & (1 << index) != 0);
            }
            controller
        });
        Ok(GenesisInputs {
            p1: pads[0],
            p2: pads[1],
        })
    }
}

impl WaitInputPoller for Timeline {
    fn at(&self, frame: u64) -> tje_host::Result<GenesisInputs> {
        self.inputs(frame)
    }
}

fn save(path: &Path, value: &serde_json::Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn run_session(
    roms: &Path,
    path: &Path,
    output: &Path,
    entries: &'static [Translation],
) -> Result<Host> {
    let session = Session::from_json(&fs::read_to_string(path)?)?;
    if !matches!(session.start, Start::PowerOn) {
        return Err("natural-session proof rejects recipe writes".into());
    }
    let revision: Revision = session.revision.into();
    let bytes = fs::read(roms.join(match revision {
        Revision::Rev02 => "rev02.bin",
        Revision::Rev00 => "rev00.bin",
    }))?;
    let frames = session.frames;
    let timeline = Arc::new(Timeline::new(session));
    let registry = HookRegistry::proof(revision, entries)?;
    let mut checked = Host::new(&bytes, Mode::Check, registry.clone())?;
    checked.set_wait_input_poller(timeline.clone());
    let mut reference = Host::new(&bytes, Mode::Reference, registry)?;
    let mut inventory_frames = [0_u64; 2];
    let mut mail_order_frames = [0_u64; 2];
    let mut split_frames = 0_u64;
    let result = (|| -> Result<()> {
        for frame in 0..frames {
            let a = checked.run_frame(timeline.inputs(frame)?)?;
            let b = reference.run_frame(timeline.inputs(frame)?)?;
            let equal = a.frame_counter == b.frame_counter
                && a.ram == b.ram
                && a.vdp == b.vdp
                && a.writes == b.writes
                && a.idle_random_draws == b.idle_random_draws
                && a.lag_events == b.lag_events
                && a.audio_samples
                    .iter()
                    .flatten()
                    .map(|sample| sample.to_bits())
                    .eq(b
                        .audio_samples
                        .iter()
                        .flatten()
                        .map(|sample| sample.to_bits()));
            let (a_pixels, a_size) = checked.frame_buffer();
            let (b_pixels, b_size) = reference.frame_buffer();
            if !equal
                || a_size != b_size
                || a_pixels[..a_size.len() as usize] != b_pixels[..b_size.len() as usize]
            {
                return Err(format!("whole-reference state differs at frame {frame}").into());
            }
            let context = checked.context();
            for player in [0, 1] {
                let open = ram::g_player_inventory_open(revision)
                    .at(player)
                    .read(&context);
                inventory_frames[player as usize] += u64::from(open != 0);
                mail_order_frames[player as usize] += u64::from(open == 2);
            }
            split_frames += u64::from(ram::g_split_screen_enabled(revision).read(&context) != 0);
        }
        if checked.checks.mismatches != 0 || !checked.checks.uninitialized_stack_reads.is_empty() {
            return Err("mismatch or dependent stack read recorded".into());
        }
        Ok(())
    })();
    let name = path
        .file_stem()
        .ok_or("session has no filename")?
        .to_string_lossy();
    save(
        &output.join(format!("{name}.json")),
        &serde_json::json!({
            "session": path, "revision": revision.name(), "frames": checked.frame_count(),
            "whole_session_reference_equal": result.is_ok(),
            "error": result.as_ref().err().map(ToString::to_string),
            "inventory_open_frames": inventory_frames, "mail_order_open_frames": mail_order_frames,
            "split_screen_frames": split_frames, "checks": checked.checks,
            "measurements": checked.measurements,
        }),
    )?;
    result?;
    println!(
        "{name}: {frames} frames, {} checked calls, mailbox={mail_order_frames:?}",
        checked.checks.calls
    );
    Ok(checked)
}

fn fixture(
    host: &mut Host,
    entries: &[Translation],
    records: &mut Vec<serde_json::Value>,
    label: &str,
    id: FunctionId,
    arguments: &[u32],
) {
    if !entries.iter().any(|entry| entry.id == id) {
        return;
    }
    let result = host.check_fixture(id.metadata().address(host.revision()), arguments);
    records.push(serde_json::json!({
        "label": label, "function": id.metadata().name, "arguments": arguments,
        "passed": result.is_ok(), "error": result.err().map(|error| error.to_string()),
    }));
}

fn drive_fixtures(host: &mut Host, entries: &[Translation], output: &Path) -> Result<()> {
    let revision = host.revision();
    let before: BTreeMap<_, _> = host
        .checks
        .by_function
        .iter()
        .map(|(name, coverage)| (name.to_owned(), coverage.calls_checked))
        .collect();
    let mut records = Vec::new();
    // Every currency byte is meaningful: bucks are signed bytes, not a 999 cap.
    for player in [0, 1] {
        for bucks in i8::MIN..=i8::MAX {
            ram::g_player_bucks(revision)
                .at(player)
                .write(&mut host.context(), bucks);
            fixture(
                host,
                entries,
                &mut records,
                "all signed-byte currency values",
                ids::INVENTORY_DRAW_BUCKS,
                &[player as u32],
            );
        }
        for points in [0, 9, 10, 99, 100, 999, 1000, 32767, 32768, 65535] {
            for rank in 0..=8 {
                let mut context = host.context();
                ram::g_player_bucks(revision)
                    .at(player)
                    .write(&mut context, 127);
                ram::g_player_points(revision)
                    .at(player)
                    .write(&mut context, points);
                ram::g_player_rank(revision)
                    .at(player)
                    .write(&mut context, rank);
                fixture(
                    host,
                    entries,
                    &mut records,
                    "points width and rank transitions",
                    ids::INVENTORY_DRAW_PLAYER_STATS,
                    &[player as u32],
                );
            }
        }
        // Saved swimming health has its own upper clamp, not a lower clamp.
        for rank in [-128, -1, 0, 1, 8, 127] {
            for actual in [-128, -1, 0, 1, 23, 31, 63, 127] {
                for displayed in [-128, -1, 0, 1, 3, 4, 7, 8, 22, 23, 31, 63, 126, 127] {
                    let mut context = host.context();
                    ram::g_player1_map_level(revision).write(&mut context, 1);
                    ram::g_player_lives(revision)
                        .at(player)
                        .write(&mut context, 3);
                    ram::g_hud_displayed_lives(revision)
                        .at(player)
                        .write(&mut context, 3);
                    ram::g_player_info(revision)
                        .at(player)
                        .sprite_state()
                        .write(&mut context, 0);
                    ram::g_player_rank(revision)
                        .at(player)
                        .write(&mut context, rank);
                    ram::g_player_health(revision)
                        .at(player)
                        .write(&mut context, actual);
                    ram::g_player_health_before_swimming(revision)
                        .at(player)
                        .write(&mut context, 127);
                    ram::g_hud_displayed_health(revision)
                        .at(player)
                        .write(&mut context, displayed);
                    fixture(
                        host,
                        entries,
                        &mut records,
                        "health clamp, byte limits and nibble transitions",
                        ids::HUD_ANIMATE_HEALTH_BAR,
                        &[player as u32],
                    );
                }
                let mut context = host.context();
                ram::g_sprite_tile_buffer_offset(revision).write(&mut context, 0);
                ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut context, 0);
                ram::g_sprite_graphics_upload_pending(revision).write(&mut context, 0);
                ram::g_v_blank_graphics_enabled(revision).write(&mut context, 0);
                ram::g_player_health(revision)
                    .at(player)
                    .write(&mut context, actual);
                ram::g_hud_displayed_health(revision)
                    .at(player)
                    .write(&mut context, 0);
                fixture(
                    host,
                    entries,
                    &mut records,
                    "immediate health initialization extremes",
                    ids::HUD_INITIALIZE_HEALTH_BAR,
                    &[player as u32, 1],
                );
            }
        }
        for lives in [0, 4, 5, 127] {
            let mut context = host.context();
            ram::g_player1_map_level(revision).write(&mut context, 1);
            ram::g_player_lives(revision)
                .at(player)
                .write(&mut context, lives);
            ram::g_hud_displayed_lives(revision)
                .at(player)
                .write(&mut context, -1);
            ram::g_player_info(revision)
                .at(player)
                .sprite_state()
                .write(&mut context, 0);
            ram::g_player_rank(revision)
                .at(player)
                .write(&mut context, 0);
            ram::g_player_health(revision)
                .at(player)
                .write(&mut context, 1);
            ram::g_hud_displayed_health(revision)
                .at(player)
                .write(&mut context, 0);
            fixture(
                host,
                entries,
                &mut records,
                "life icon refresh and four/five-life boundary",
                ids::HUD_ANIMATE_HEALTH_BAR,
                &[player as u32],
            );
        }
        for (lives, state, level) in [(-1, 0, 1), (3, 5, 1), (3, 6, 1), (3, 0, 26)] {
            let mut context = host.context();
            ram::g_player_lives(revision)
                .at(player)
                .write(&mut context, lives);
            ram::g_hud_displayed_lives(revision)
                .at(player)
                .write(&mut context, lives);
            ram::g_player_info(revision)
                .at(player)
                .sprite_state()
                .write(&mut context, state);
            ram::g_player1_map_level(revision).write(&mut context, level);
            fixture(
                host,
                entries,
                &mut records,
                "absent, ghost and final-level early returns",
                ids::HUD_ANIMATE_HEALTH_BAR,
                &[player as u32],
            );
        }
        for powerup in -1..=7 {
            let mut context = host.context();
            ram::g_player_powerups(revision)
                .at(player)
                .write(&mut context, powerup);
            ram::g_v_blank_graphics_enabled(revision).write(&mut context, 0);
            ram::g_sprite_graphics_upload_pending(revision).write(&mut context, 0);
            ram::g_sprite_tile_buffer_offset(revision).write(&mut context, 0);
            ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut context, 0);
            fixture(
                host,
                entries,
                &mut records,
                "default and every action-icon descriptor",
                ids::HUD_LOAD_ACTION_ICON_SPRITE,
                &[player as u32],
            );
        }
        for suppressed in [0, 1] {
            for identified in [0, 1] {
                for partner_lives in [-1, 3] {
                    let mut context = host.context();
                    ram::g_inventory_labels_suppressed(revision).write(&mut context, suppressed);
                    ram::g_player_lives(revision)
                        .at(player ^ 1)
                        .write(&mut context, partner_lives);
                    for (row, present) in [(0, 12), (1, 0), (2, 26)] {
                        ram::g_player_mailbox_items(revision)
                            .at(player)
                            .at(row)
                            .write(&mut context, present);
                        ram::g_player_mailbox_prices(revision)
                            .at(player)
                            .at(row)
                            .write(&mut context, [-128, 0, 127][row as usize]);
                        ram::g_present_info(revision)
                            .at(i32::from(present))
                            .identified()
                            .write(&mut context, identified);
                    }
                    fixture(
                        host,
                        entries,
                        &mut records,
                        "mailbox labels, togetherness and signed prices",
                        ids::INVENTORY_DRAW_MAIL_ORDER_OFFERS,
                        &[player as u32],
                    );
                }
            }
        }
    }
    for selection in [-1, 0, 1, 2] {
        for rank in 0..=8 {
            let mut context = host.context();
            ram::g_player_selection(revision).write(&mut context, selection);
            ram::g_player_rank(revision).at(0).write(&mut context, rank);
            ram::g_player_rank(revision)
                .at(1)
                .write(&mut context, 8 - rank);
            fixture(
                host,
                entries,
                &mut records,
                "active/absent player ranks and rank changes",
                ids::HUD_DRAW_RANK_LABELS,
                &[],
            );
        }
    }
    for split in [0, 1] {
        for (open_a, open_b) in [(0, 0), (1, 0), (0, 2), (1, 2)] {
            for lives in [-1, 3] {
                for level in [1, 26] {
                    let mut context = host.context();
                    ram::g_player1_map_level(revision).write(&mut context, level);
                    ram::g_split_screen_enabled(revision).write(&mut context, split);
                    ram::g_player_inventory_open(revision)
                        .at(0)
                        .write(&mut context, open_a);
                    ram::g_player_inventory_open(revision)
                        .at(1)
                        .write(&mut context, open_b);
                    ram::g_player_lives(revision)
                        .at(0)
                        .write(&mut context, lives);
                    ram::g_player_lives(revision).at(1).write(&mut context, 3);
                    ram::g_player_powerups(revision)
                        .at(0)
                        .write(&mut context, -1);
                    ram::g_player_powerups(revision)
                        .at(1)
                        .write(&mut context, -1);
                    fixture(
                        host,
                        entries,
                        &mut records,
                        "combined, split, inventory and final-level HUD",
                        ids::DRAW_HUD,
                        &[],
                    );
                }
            }
        }
    }
    for filled in [0, 1, u32::MAX, 0x10000, i16::MAX as u32] {
        fixture(
            host,
            entries,
            &mut records,
            "border RNG and short truth narrowing",
            ids::DRAW_MENU_WINDOW_FRAME,
            &[filled],
        );
    }
    let counts: BTreeMap<_, _> = host
        .checks
        .by_function
        .iter()
        .map(|(name, coverage)| {
            (
                name,
                coverage.calls_checked - before.get(name).copied().unwrap_or(0),
            )
        })
        .collect();
    let failures = records
        .iter()
        .filter(|record| record["passed"] != true)
        .count();
    save(
        &output.join(format!("{}-live-fixtures.json", revision.name())),
        &serde_json::json!({
            "revision": revision.name(), "coverage_source": "labelled synthetic inputs on live gameplay state",
            "fixture_calls": counts, "fixture_failures": failures, "records": records,
            "checks_including_session": host.checks,
        }),
    )?;
    println!(
        "{revision:?} fixtures: {} cases, {failures} failures",
        records.len()
    );
    if failures != 0 {
        return Err(format!("{failures} labelled fixtures failed").into());
    }
    Ok(())
}

fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() < 3 {
        return Err("usage: tje-ui-hud-check <ROM-directory> <evidence-directory> <session.tje.json>...; TJE_HUD_FUNCTIONS optionally selects comma-separated names".into());
    }
    let roms = Path::new(&arguments[0]);
    let output = Path::new(&arguments[1]);
    fs::create_dir_all(output)?;
    let selection = std::env::var("TJE_HUD_FUNCTIONS").ok();
    let names: Vec<_> = selection
        .as_deref()
        .map_or_else(Vec::new, |text| text.split(',').collect());
    let candidates: Vec<_> = hud_all::PROVEN_ENTRIES
        .iter()
        .chain(hud_all::CANDIDATES)
        .copied()
        .collect();
    for name in &names {
        if !candidates
            .iter()
            .any(|entry| entry.id.metadata().name == *name)
        {
            return Err(format!("unknown HUD function {name}").into());
        }
    }
    static ENTRIES: OnceLock<Vec<Translation>> = OnceLock::new();
    let entries = ENTRIES.get_or_init(|| {
        candidates
            .into_iter()
            .filter(|entry| {
                if names.is_empty() {
                    hud_all::PROVEN_ENTRIES
                        .iter()
                        .any(|proven| proven.id == entry.id)
                } else {
                    names.contains(&entry.id.metadata().name)
                }
            })
            .collect()
    });
    let mut hosts = BTreeMap::new();
    let mut failures = Vec::new();
    for path in &arguments[2..] {
        match run_session(roms, Path::new(path), output, entries) {
            Ok(host) => {
                hosts.insert(host.revision().name(), host);
            }
            Err(error) => {
                eprintln!("{}: {error}", Path::new(path).display());
                failures.push(error.to_string());
            }
        }
    }
    if std::env::var_os("TJE_HUD_SKIP_FIXTURES").is_none() {
        for (_, mut host) in hosts {
            if host.measurements.game_frame.is_some()
                && let Err(error) = drive_fixtures(&mut host, entries, output)
            {
                failures.push(error.to_string());
            }
        }
    }
    if !failures.is_empty() {
        return Err(failures.join("; ").into());
    }
    Ok(())
}
