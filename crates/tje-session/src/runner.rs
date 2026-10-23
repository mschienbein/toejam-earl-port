use anyhow::{Context, Result, ensure};
use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::Path,
};
use tje_game::FunctionMetadata;
use tje_host::{
    FrameSnapshot, Host, Mode,
    hooks::HookRegistry,
    session_trace::{EntryObservation, FrameObservations, LogicTick},
};
use tje_mem::{Bus, Revision, ram};
use tje_session::{CONNECTED, Session, TOOL_VERSION};

pub fn functions() -> Vec<&'static FunctionMetadata> {
    tje_game::functions().collect()
}
pub fn function_name(revision: Revision, address: u32) -> &'static str {
    tje_game::functions()
        .find(|function| {
            function.address(revision) <= address
                && address < function.address(revision) + function.size(revision)
        })
        .map_or("<uncatalogued>", |function| function.name)
}
pub fn entry_name(revision: Revision, address: u32) -> &'static str {
    tje_game::function_at(revision, address).map_or("<uncatalogued>", |id| id.metadata().name)
}

pub fn inputs(pads: [u16; 2]) -> GenesisInputs {
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

pub struct Ram<'a>(pub &'a [u8]);
impl Bus for Ram<'_> {
    fn read_u8(&self, address: u32) -> u8 {
        self.0[(address & 0xFFFF) as usize]
    }
    fn write_u8(&mut self, _address: u32, _value: u8) {
        panic!("read-only session observation");
    }
}

/// The sample order is a local observation contract, not serialized session data.
fn sample_addresses(revision: Revision) -> Vec<u32> {
    let mut samples = (0..32)
        .map(|index| ram::g_player_inventory(revision).address() + index)
        .collect::<Vec<_>>();
    for base in [
        ram::g_player_inventory_scroll_row(revision).address(),
        ram::g_player_inventory_selection_row(revision).address(),
        ram::g_player_inventory_selection_column(revision).address(),
    ] {
        samples.extend([base, base + 1]);
    }
    samples
}

pub fn host(bytes: &[u8], mode: Mode, revision: Revision, capture_ticks: bool) -> Result<Host> {
    let registry = if mode == Mode::Reference {
        HookRegistry::empty(revision)
    } else {
        HookRegistry::translated(revision)?
    };
    let mut host = Host::new(bytes, mode, registry)?;
    let entries = functions()
        .iter()
        .map(|function| function.address(revision))
        .collect::<Vec<_>>();
    let watched = functions()
        .iter()
        .filter(|function| {
            matches!(
                function.name,
                "MenuWhoHandler"
                    | "MenuWhatHandler"
                    | "MenuJamOutHandler"
                    | "MenuControllerOptionsHandler"
                    | "OpenPresent"
                    | "OpenSelectedPresent"
                    | "SetupSelectedPresent"
                    | "RandomizePresents"
                    | "DrawGameMap"
                    | "ElevatorBeginRideScene"
                    | "ShipPieceAbility"
                    | "StartJamOut"
                    | "PlayEndingSequence"
                    | "EndingPlayScrollingCredits"
                    | "TitleScreenVideo"
            )
        })
        .map(|function| function.address(revision))
        .collect::<Vec<_>>();
    host.enable_session_trace(
        &entries,
        &watched,
        &sample_addresses(revision),
        capture_ticks,
    )?;
    Ok(host)
}

pub fn apply_recipe(host: &mut Host, session: &Session, frame: u64) {
    for write in session
        .writes()
        .iter()
        .filter(|write| write.before_frame == frame)
    {
        let mut context = host.context();
        match write.width {
            1 => context.write_u8(write.address, write.value as u8),
            2 => context.write_u16(write.address, write.value as u16),
            4 => context.write_u32(write.address, write.value),
            _ => unreachable!("validated recipe width"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Content {
    pub milestones: BTreeMap<String, Option<u64>>,
    /// Storage identities 0..27: mystery and bonus-hi-tops are tracked separately
    /// from the resolved/normalized effects. A successful setup is required.
    pub presents_opened: BTreeMap<u8, Option<u64>>,
    /// Requires allocated entity plus visible-in-either-viewport bits, not spawn.
    pub earthlings_encountered: BTreeMap<u8, Option<u64>>,
    pub ship_pieces_collected: BTreeMap<u8, Option<u64>>,
    pub levels_visited: BTreeSet<i8>,
}
impl Default for Content {
    fn default() -> Self {
        Self::new()
    }
}
impl Content {
    pub fn new() -> Self {
        Self {
            milestones: [
                "title",
                "menus",
                "fixed_world",
                "one_player",
                "walking",
                "two_player_split_screen",
                "inventory",
                "elevator_ride",
                "map_screen",
                "jam_out",
                "ending_started",
                "ending_credits",
            ]
            .into_iter()
            .map(|name| (name.into(), None))
            .collect(),
            presents_opened: (0..28).map(|kind| (kind, None)).collect(),
            earthlings_encountered: (0..25).map(|kind| (kind, None)).collect(),
            ship_pieces_collected: (0..10).map(|kind| (kind, None)).collect(),
            levels_visited: BTreeSet::new(),
        }
    }
    pub fn mark(&mut self, name: &str, frame: u64) {
        self.milestones
            .entry(name.into())
            .or_default()
            .get_or_insert(frame);
    }
    pub fn merge(&mut self, other: &Self) {
        fn merge_map<K: Ord + Clone>(
            target: &mut BTreeMap<K, Option<u64>>,
            source: &BTreeMap<K, Option<u64>>,
        ) {
            for (key, &frame) in source {
                let entry = target.entry(key.clone()).or_default();
                if entry.is_none() {
                    *entry = frame;
                }
            }
        }
        merge_map(&mut self.milestones, &other.milestones);
        merge_map(&mut self.presents_opened, &other.presents_opened);
        merge_map(
            &mut self.earthlings_encountered,
            &other.earthlings_encountered,
        );
        merge_map(
            &mut self.ship_pieces_collected,
            &other.ship_pieces_collected,
        );
        self.levels_visited.extend(&other.levels_visited);
    }
}

pub struct Coverage {
    pub content: Content,
    pending_present: [Option<i8>; 2],
    previous_ship: Option<[i8; 10]>,
    collecting_ship: bool,
    initial_positions: [Option<(i16, i16)>; 2],
}
impl Coverage {
    pub fn new() -> Self {
        Self {
            content: Content::new(),
            pending_present: [None; 2],
            previous_ship: None,
            collecting_ship: false,
            initial_positions: [None; 2],
        }
    }
    pub fn observe(
        &mut self,
        host: &Host,
        snapshot: &FrameSnapshot,
        observations: &FrameObservations,
        revision: Revision,
    ) {
        let frame = snapshot.frame;
        if host.measurements.title_frame.is_some() {
            self.content.mark("title", frame);
        }
        if host.measurements.menu_frame.is_some() {
            self.content.mark("menus", frame);
        }
        for event in &observations.entries {
            self.entry(event, revision);
        }
        if host.measurements.game_frame.is_none() {
            return;
        }
        let memory = Ram(&snapshot.ram);
        if ram::g_world_type(revision).read(&memory) != 0 {
            self.content.mark("fixed_world", frame);
        }
        if ram::g_player_selection(revision).read(&memory) != 2 {
            self.content.mark("one_player", frame);
        }
        if ram::g_player_selection(revision).read(&memory) == 2
            && ram::g_split_screen_enabled(revision).read(&memory) != 0
        {
            self.content.mark("two_player_split_screen", frame);
        }
        for player in 0..2 {
            if ram::g_player_lives(revision)
                .at(player as i32)
                .read(&memory)
                < 0
            {
                continue;
            }
            let view = ram::g_player_info(revision).at(player as i32);
            self.content
                .levels_visited
                .insert(view.level().read(&memory));
            let position = (view.x().read(&memory), view.y().read(&memory));
            if let Some(initial) = self.initial_positions[player] {
                if (i32::from(initial.0) - i32::from(position.0)).abs()
                    + (i32::from(initial.1) - i32::from(position.1)).abs()
                    > 24
                {
                    self.content.mark("walking", frame);
                }
            } else {
                self.initial_positions[player] = Some(position);
            }
            if ram::g_player_inventory_open(revision)
                .at(player as i32)
                .read(&memory)
                != 0
            {
                self.content.mark("inventory", frame);
            }
        }
        for bank in 0..2 {
            for index in 0..20 {
                let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                let kind = earthling.kind().read(&memory);
                if (0..25).contains(&kind)
                    && earthling.object().read(&memory) != 0
                    && earthling.health_visibility().at(1).read(&memory) & 6 != 0
                {
                    self.content
                        .earthlings_encountered
                        .get_mut(&(kind as u8))
                        .unwrap()
                        .get_or_insert(frame);
                }
            }
        }
        let ship = std::array::from_fn(|index| {
            ram::g_ship_piece_level_assignments(revision)
                .at(index as i32)
                .read(&memory)
        });
        if let Some(previous) = self.previous_ship {
            let mut changed = (0..10).filter(|&index| previous[index] > 0 && ship[index] == 0);
            if let Some(index) = changed.next() {
                if self.collecting_ship && changed.next().is_none() {
                    self.content
                        .ship_pieces_collected
                        .get_mut(&(index as u8))
                        .unwrap()
                        .get_or_insert(frame);
                }
                self.collecting_ship = false;
            }
        }
        self.previous_ship = Some(ship);
    }
    fn entry(&mut self, event: &EntryObservation, revision: Revision) {
        let frame = event.frame;
        match entry_name(revision, event.address) {
            "OpenPresent" => {
                let player = event.stack_slots[0] as u16 as usize;
                if player < 2 && event.ram_samples.len() == 38 {
                    let samples = &event.ram_samples;
                    let slot =
                        (usize::from(samples[32 + player]) + usize::from(samples[34 + player])) * 2
                            + usize::from(samples[36 + player]);
                    self.pending_present[player] = samples
                        .get(player * 16 + slot)
                        .copied()
                        .map(|value| value as i8);
                }
            }
            "SetupSelectedPresent" => {
                let player = event.stack_slots[1] as u16 as usize;
                let kind = event.stack_slots[2] as u16;
                if kind < 28 {
                    self.content
                        .presents_opened
                        .get_mut(&(kind as u8))
                        .unwrap()
                        .get_or_insert(frame);
                }
                if player < 2
                    && let Some(original) = self.pending_present[player].take()
                    && (0..28).contains(&original)
                {
                    self.content
                        .presents_opened
                        .get_mut(&(original as u8))
                        .unwrap()
                        .get_or_insert(frame);
                }
            }
            "RandomizePresents" => {
                self.content
                    .presents_opened
                    .get_mut(&18)
                    .unwrap()
                    .get_or_insert(frame);
            }
            "DrawGameMap" => self.content.mark("map_screen", frame),
            "ElevatorBeginRideScene" => self.content.mark("elevator_ride", frame),
            "ShipPieceAbility" => self.collecting_ship = true,
            "StartJamOut" => self.content.mark("jam_out", frame),
            "PlayEndingSequence" => self.content.mark("ending_started", frame),
            "EndingPlayScrollingCredits" => self.content.mark("ending_credits", frame),
            _ => {}
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReplayReport {
    pub session: String,
    pub revision: String,
    pub mode: String,
    pub frames: u64,
    pub logic_ticks: u64,
    pub lag_frames: Vec<u64>,
    pub idle_random_draws: u64,
    pub checked_calls: u64,
    pub replaced_calls: u64,
    pub mismatches: u64,
    pub entries_hit: usize,
    pub inventory_entries: usize,
    pub entry_counts: BTreeMap<String, u64>,
    pub content: Content,
}
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub tool: String,
    pub sessions: Vec<ReplayReport>,
    pub union_entries_hit: usize,
    pub union_entry_counts: BTreeMap<String, u64>,
    pub content_by_revision: BTreeMap<String, Content>,
    #[serde(default)]
    pub entry_counts_by_revision: BTreeMap<String, BTreeMap<String, u64>>,
    #[serde(default)]
    pub entries_hit_by_revision: BTreeMap<String, usize>,
}
impl Report {
    pub fn from_sessions(sessions: Vec<ReplayReport>) -> Self {
        let mut report = Self {
            tool: TOOL_VERSION.into(),
            sessions,
            ..Self::default()
        };
        for session in &report.sessions {
            for (name, count) in &session.entry_counts {
                *report.union_entry_counts.entry(name.clone()).or_default() += count;
                *report
                    .entry_counts_by_revision
                    .entry(session.revision.clone())
                    .or_default()
                    .entry(name.clone())
                    .or_default() += count;
            }
            report
                .content_by_revision
                .entry(session.revision.clone())
                .or_default()
                .merge(&session.content);
        }
        report.union_entries_hit = report
            .union_entry_counts
            .values()
            .filter(|&&count| count != 0)
            .count();
        report.entries_hit_by_revision = report
            .entry_counts_by_revision
            .iter()
            .map(|(revision, counts)| {
                (
                    revision.clone(),
                    counts.values().filter(|&&count| count != 0).count(),
                )
            })
            .collect();
        report
    }
}

pub fn replay_report(
    path: &Path,
    session: &Session,
    host: &Host,
    content: Content,
) -> ReplayReport {
    let entry_counts = functions()
        .into_iter()
        .zip(host.session_entry_counts().unwrap())
        .map(|(function, &count)| {
            (
                format!("{} {}", function.inventory_id, function.name),
                count,
            )
        })
        .collect::<BTreeMap<_, _>>();
    ReplayReport {
        session: path.display().to_string(),
        revision: format!("{:?}", Revision::from(session.revision)).to_lowercase(),
        mode: format!("{:?}", host.mode()).to_lowercase(),
        frames: session.frames,
        logic_ticks: host.measurements.logic_ticks,
        lag_frames: host
            .measurements
            .lag_events
            .iter()
            .map(|event| event.frame)
            .collect(),
        idle_random_draws: host.measurements.idle_draws_per_frame.iter().sum(),
        checked_calls: host
            .checks
            .by_function
            .values()
            .map(|count| count.calls_checked)
            .sum(),
        replaced_calls: host
            .checks
            .by_function
            .values()
            .map(|count| count.calls_replaced)
            .sum(),
        mismatches: host.checks.mismatches,
        entries_hit: entry_counts.values().filter(|&&count| count > 0).count(),
        inventory_entries: entry_counts.len(),
        entry_counts,
        content,
    }
}

pub fn replay(bytes: &[u8], path: &Path, mode: Mode) -> Result<ReplayReport> {
    let session = Session::from_json(&fs::read_to_string(path)?)?;
    let revision = Revision::from(session.revision);
    ensure!(
        tje_rom::Rom::import(bytes)?.revision() == revision,
        "ROM revision does not match session"
    );
    let mut host = host(bytes, mode, revision, false)?;
    let mut coverage = Coverage::new();
    for (frame, pads) in session.inputs().enumerate() {
        apply_recipe(&mut host, &session, frame as u64);
        let snapshot = host
            .run_frame(inputs(pads))
            .with_context(|| format!("session {} frame {frame}", path.display()))?;
        let observations = host.take_session_observations();
        coverage.observe(&host, &snapshot, &observations, revision);
    }
    Ok(replay_report(path, &session, &host, coverage.content))
}

#[derive(Debug, Serialize)]
pub struct Divergence {
    pub tick: Option<u64>,
    pub reference_frame: u64,
    pub second_frame: u64,
    pub reference_pc: u32,
    pub second_pc: u32,
    pub reference_function: String,
    pub second_function: String,
    pub ram_start: Option<u32>,
    pub ram_end_exclusive: Option<u32>,
    pub detail: String,
}
#[derive(Debug, Serialize)]
pub struct Comparison {
    pub session: String,
    pub second_mode: String,
    pub boundary: String,
    pub compared: u64,
    pub skipped_lag_ticks: Vec<(u64, u64, u64)>,
    pub divergence: Option<Divergence>,
}
fn ram_difference(a: &[u8], b: &[u8]) -> Option<(u32, u32)> {
    let start = a.iter().zip(b).position(|(a, b)| a != b)?;
    let end = a[start..]
        .iter()
        .zip(&b[start..])
        .position(|(a, b)| a == b)
        .map_or(a.len(), |count| start + count);
    Some((0xFF0000 + start as u32, 0xFF0000 + end as u32))
}
fn divergence(
    revision: Revision,
    tick: Option<u64>,
    frames: [u64; 2],
    pcs: [u32; 2],
    rams: [&[u8]; 2],
    detail: &str,
) -> Divergence {
    let range = ram_difference(rams[0], rams[1]);
    Divergence {
        tick,
        reference_frame: frames[0],
        second_frame: frames[1],
        reference_pc: pcs[0],
        second_pc: pcs[1],
        reference_function: function_name(revision, pcs[0]).into(),
        second_function: function_name(revision, pcs[1]).into(),
        ram_start: range.map(|range| range.0),
        ram_end_exclusive: range.map(|range| range.1),
        detail: detail.into(),
    }
}

/// Frame comparison checks all RAM (including dead stack), video/VDP, ordered
/// hardware writes and bit-exact PCM; no hash-only or ignored-byte shortcuts.
pub fn compare(bytes: &[u8], path: &Path, second: Mode, ticks: bool) -> Result<Comparison> {
    let session = Session::from_json(&fs::read_to_string(path)?)?;
    let revision = Revision::from(session.revision);
    ensure!(
        tje_rom::Rom::import(bytes)?.revision() == revision,
        "ROM revision does not match session"
    );
    let mut reference = host(bytes, Mode::Reference, revision, ticks)?;
    let mut candidate = host(bytes, second, revision, ticks)?;
    let mut result = Comparison {
        session: path.display().to_string(),
        second_mode: format!("{second:?}").to_lowercase(),
        boundary: if ticks { "logic_tick" } else { "video_frame" }.into(),
        compared: 0,
        skipped_lag_ticks: Vec::new(),
        divergence: None,
    };
    let mut queues: [VecDeque<LogicTick>; 2] = std::array::from_fn(|_| VecDeque::new());
    for (frame, pads) in session.inputs().enumerate() {
        apply_recipe(&mut reference, &session, frame as u64);
        apply_recipe(&mut candidate, &session, frame as u64);
        let a = reference.run_frame(inputs(pads))?;
        let b = candidate.run_frame(inputs(pads))?;
        let observations = [
            reference.take_session_observations(),
            candidate.take_session_observations(),
        ];
        if ticks {
            for (queue, observations) in queues.iter_mut().zip(observations) {
                queue.extend(observations.ticks);
            }
            while !queues[0].is_empty() && !queues[1].is_empty() {
                let a = queues[0].pop_front().unwrap();
                let b = queues[1].pop_front().unwrap();
                if a.lagged {
                    result.skipped_lag_ticks.push((a.ordinal, a.frame, b.frame));
                    continue;
                }
                result.compared += 1;
                if a.ordinal != b.ordinal || a.ram != b.ram {
                    let mut difference = divergence(
                        revision,
                        Some(a.ordinal),
                        [a.frame, b.frame],
                        [a.pc, b.pc],
                        [&a.ram, &b.ram],
                        "work RAM differs at completed logic tick",
                    );
                    // Last entered function is execution context, not a claim of
                    // causality for a later write; precise hook errors name the callee.
                    difference.reference_function = a
                        .last_entry
                        .map_or("<none>", |pc| entry_name(revision, pc))
                        .into();
                    difference.second_function = b
                        .last_entry
                        .map_or("<none>", |pc| entry_name(revision, pc))
                        .into();
                    result.divergence = Some(difference);
                    return Ok(result);
                }
            }
            ensure!(
                queues[0].len() < 120 && queues[1].len() < 120,
                "runners differ by more than 120 logic ticks; refused unbounded backlog"
            );
        } else {
            result.compared += 1;
            let detail = if a.ram != b.ram {
                Some("work RAM differs")
            } else if a.vdp != b.vdp {
                Some("VDP memories/registers differ")
            } else if reference.frame_buffer() != candidate.frame_buffer() {
                Some("rendered pixels/frame size differ")
            } else if a.writes != b.writes {
                Some("ordered VDP/YM2612/PSG writes differ")
            } else if a.audio_samples.len() != b.audio_samples.len()
                || a.audio_samples
                    .iter()
                    .flatten()
                    .zip(b.audio_samples.iter().flatten())
                    .any(|(a, b)| a.to_bits() != b.to_bits())
            {
                Some("bit-exact stereo PCM differs")
            } else if a.idle_random_draws != b.idle_random_draws || a.lag_events != b.lag_events {
                Some("idle random draws or lag events differ")
            } else {
                None
            };
            if let Some(detail) = detail {
                result.divergence = Some(divergence(
                    revision,
                    None,
                    [a.frame, b.frame],
                    [a.pc, b.pc],
                    [&a.ram, &b.ram],
                    detail,
                ));
                return Ok(result);
            }
        }
    }
    if ticks && (!queues[0].is_empty() || !queues[1].is_empty()) {
        result.divergence = Some(Divergence {
            tick: None,
            reference_frame: session.frames,
            second_frame: session.frames,
            reference_pc: reference.cpu().pc(),
            second_pc: candidate.cpu().pc(),
            reference_function: function_name(revision, reference.cpu().pc()).into(),
            second_function: function_name(revision, candidate.cpu().pc()).into(),
            ram_start: None,
            ram_end_exclusive: None,
            detail: format!(
                "unpaired completed ticks at session end: reference={}, second={}",
                queues[0].len(),
                queues[1].len()
            ),
        });
    }
    Ok(result)
}

pub fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(value)? + "\n")
        .with_context(|| format!("write report {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_first_contiguous_range() {
        assert_eq!(
            ram_difference(&[0, 1, 2, 3, 4], &[0, 7, 8, 3, 9]),
            Some((0xFF0001, 0xFF0003))
        );
        assert_eq!(ram_difference(&[0, 1], &[0, 1]), None);
    }
    #[test]
    fn inventory_denominator_is_exact() {
        assert_eq!(functions().len(), 648);
        for revision in Revision::ALL {
            let entries = functions()
                .iter()
                .map(|function| function.address(revision))
                .collect::<BTreeSet<_>>();
            assert_eq!(entries.len(), 648);
        }
    }
}
