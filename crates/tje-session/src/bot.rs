//! A bounded deterministic input planner. It observes named RAM views and the
//! user's runtime ROM terrain resources; it never edits RAM or saves resources.
use crate::runner::{self, Coverage, Ram, ReplayReport};
use anyhow::{Result, ensure};
use clap::ValueEnum;
use std::{collections::VecDeque, fs, path::Path};
use tje_host::{FrameSnapshot, Host, Mode, session_trace::FrameObservations};
use tje_mem::{Revision, ram};
use tje_session::{A, B, C, CONNECTED, DOWN, LEFT, RIGHT, START, Session, UP};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Scenario {
    OnePlayer,
    TwoPlayer,
    JamOut,
}

struct Planner {
    scenario: Scenario,
    menu: &'static str,
    menu_frame: u64,
    game_frame: Option<u64>,
    jam_frame: Option<u64>,
    jam_round: u32,
    navigators: [Navigator; 2],
}
impl Planner {
    fn new(scenario: Scenario) -> Self {
        Self {
            scenario,
            menu: "",
            menu_frame: 0,
            game_frame: None,
            jam_frame: None,
            jam_round: 0,
            navigators: std::array::from_fn(|_| Navigator::new()),
        }
    }
    fn observe(&mut self, observations: &FrameObservations, revision: Revision) {
        for entry in &observations.entries {
            let name = runner::entry_name(revision, entry.address);
            if matches!(
                name,
                "MenuWhoHandler"
                    | "MenuWhatHandler"
                    | "MenuJamOutHandler"
                    | "MenuControllerOptionsHandler"
            ) {
                self.menu = name;
                self.menu_frame = entry.frame;
                if name == "MenuWhoHandler" && self.jam_frame.take().is_some() {
                    self.jam_round += 1;
                }
            } else if name == "StartJamOut" {
                self.jam_frame = Some(entry.frame);
                self.menu = "";
            }
        }
    }
    fn choose(
        &mut self,
        frame: u64,
        previous: Option<&FrameSnapshot>,
        host: &Host,
        rom: &[u8],
        revision: Revision,
    ) -> [u16; 2] {
        let mut pads = [
            CONNECTED,
            if self.scenario == Scenario::OnePlayer {
                0
            } else {
                CONNECTED
            },
        ];
        if let Some(start) = self.jam_frame {
            let age = frame - start;
            if age >= 2000 {
                pads[0] |= START;
            } else if age > 90 {
                let pulse = (age / 16) % 8;
                if age % 16 < 8 {
                    pads[0] |= [A, B, C, UP, DOWN, LEFT, RIGHT, START][pulse as usize];
                    pads[1] |= [C, A, B, RIGHT, LEFT, UP, DOWN, START][pulse as usize];
                }
            }
            return pads;
        }
        if host.measurements.game_frame.is_none() {
            let age = frame.saturating_sub(self.menu_frame);
            let down_count = match self.menu {
                "MenuWhoHandler" => u64::from(self.scenario == Scenario::OnePlayer),
                "MenuWhatHandler" => {
                    if self.scenario == Scenario::JamOut {
                        1
                    } else {
                        4
                    }
                }
                "MenuJamOutHandler" => {
                    if self.jam_round == 0 {
                        1
                    } else {
                        7
                    }
                }
                _ => 0,
            };
            if !self.menu.is_empty() {
                // Menu setup blocks until graphics uploads finish. Use generous
                // release gaps; edges, not held repeat, move its selection.
                for step in 0..down_count {
                    if (80 + step * 24..88 + step * 24).contains(&age) {
                        pads[0] |= DOWN;
                    }
                }
                let confirm = 104 + down_count * 24;
                if (confirm..confirm + 8).contains(&age) {
                    pads[0] |= START;
                }
            } else if frame >= 300 && frame % 120 < 8 {
                pads[0] |= START;
            }
            return pads;
        }
        let start = *self.game_frame.get_or_insert(frame);
        let age = frame - start;
        let Some(snapshot) = previous else {
            return pads;
        };
        let memory = Ram(&snapshot.ram);
        // Four initial inventory attempts exercise the real selector/open path.
        // They are controller actions, not fabricated grants or forced effects.
        for round in 0..4 {
            let open = 80 + round * 180;
            let action = open + 70;
            if (open..open + 8).contains(&age) {
                pads[0] |= B;
            }
            if (action..action + 8).contains(&age) {
                pads[0] |= A;
            }
        }
        if (900..908).contains(&age) || (1050..1058).contains(&age) {
            pads[0] |= C;
        }
        if age < 1160 {
            return pads;
        }
        for (player, pad) in pads.iter_mut().enumerate() {
            if *pad == 0
                || ram::g_player_lives(revision)
                    .at(player as i32)
                    .read(&memory)
                    < 0
            {
                continue;
            }
            if ram::g_player_inventory_open(revision)
                .at(player as i32)
                .read(&memory)
                != 0
            {
                if age % 120 < 8 {
                    *pad |= B;
                }
                continue;
            }
            let view = ram::g_player_info(revision).at(player as i32);
            let level = view.level().read(&memory);
            let position = (view.x().read(&memory), view.y().read(&memory));
            let elevator_id = ram::g_player_elevator_flags(revision)
                .at(player as i32)
                .read(&memory);
            if elevator_id != 0 {
                // Arrival IDs are 2/4. A rider must actually walk out through
                // their open doorway before another elevator can be targeted.
                if elevator_id & 1 == 0 {
                    *pad |= DOWN;
                }
                continue;
            }
            let mut target = None;
            // A visible ship piece is preferred over advancing past its level.
            for bank in 0..2 {
                for index in 0..20 {
                    let entity = ram::g_earthling_banks(revision).at(bank).at(index);
                    if entity.kind().read(&memory) == 0 && entity.level().read(&memory) == level {
                        target = Some((entity.x().read(&memory), entity.y().read(&memory), true));
                    }
                }
            }
            if target.is_none() {
                for bank in 0..2 {
                    let elevator = ram::g_elevator_records(revision).at(bank).at(0);
                    if elevator.level().read(&memory) == level
                        && elevator.state().read(&memory) != 11
                    {
                        target = Some((
                            elevator.x().read(&memory),
                            elevator.y().read(&memory) + 22,
                            false,
                        ));
                        break;
                    }
                }
            }
            // Keeping P2 initially stationary naturally exercises split-screen;
            // it joins P1 after that separation has been observable for a while.
            if player == 1 && age < 1700 {
                continue;
            }
            if let Some((x, y, ship)) = target {
                if (position.0 - x).abs() < 8 && (position.1 - y).abs() < 10 {
                    if ship {
                        if age % 48 < 8 {
                            *pad |= A;
                        }
                    } else {
                        *pad |= UP;
                    }
                } else {
                    *pad |= self.navigators[player].steer(
                        &memory,
                        rom,
                        revision,
                        player,
                        (x, y),
                        frame,
                    );
                }
            }
        }
        pads
    }
}

const WIDTH: usize = 320;
const HEIGHT: usize = 216;
const CELLS: usize = WIDTH * HEIGHT;
struct Navigator {
    level: i8,
    map: Vec<u16>,
    terrain: Vec<u8>,
    parents: Vec<usize>,
    queue: VecDeque<usize>,
    path: Vec<usize>,
    path_index: usize,
    target: Option<(i16, i16)>,
    last_plan: u64,
    previous: (i16, i16),
    stuck: u64,
}
impl Navigator {
    fn new() -> Self {
        Self {
            level: -1,
            map: vec![0; 26 * 19],
            terrain: vec![0x90; CELLS],
            parents: vec![usize::MAX; CELLS],
            queue: VecDeque::with_capacity(CELLS),
            path: Vec::new(),
            path_index: 0,
            target: None,
            last_plan: 0,
            previous: (0, 0),
            stuck: 0,
        }
    }
    fn steer(
        &mut self,
        memory: &Ram<'_>,
        rom: &[u8],
        revision: Revision,
        player: usize,
        target: (i16, i16),
        frame: u64,
    ) -> u16 {
        let view = ram::g_player_info(revision).at(player as i32);
        let level = view.level().read(memory);
        let position = (view.x().read(memory), view.y().read(memory));
        let map_view = if player == 1 && ram::g_split_screen_enabled(revision).read(memory) != 0 {
            ram::g_player2_level_map(revision)
        } else {
            ram::g_player1_level_map(revision)
        };
        let changed = self.level != level
            || (0..26).any(|row| {
                (0..19).any(|col| {
                    self.map[(row * 19 + col) as usize] != map_view.at(row).at(col).read(memory)
                })
            });
        if changed {
            self.level = level;
            for row in 0..26 {
                for col in 0..19 {
                    self.map[(row * 19 + col) as usize] = map_view.at(row).at(col).read(memory);
                }
            }
            self.decode_terrain(rom, revision);
            self.path.clear();
        }
        if position == self.previous {
            self.stuck += 1;
        } else {
            self.stuck = 0;
        }
        self.previous = position;
        if self.path_index >= self.path.len()
            || self.target != Some(target)
            || self.stuck > 90
            || frame - self.last_plan > 360
        {
            self.plan(position, target);
            self.target = Some(target);
            self.last_plan = frame;
            self.stuck = 0;
        }
        while let Some(&cell) = self.path.get(self.path_index) {
            let point = (
                ((cell % WIDTH) * 8 + 4) as i16,
                ((cell / WIDTH) * 8 + 4) as i16,
            );
            if (position.0 - point.0).abs() < 5 && (position.1 - point.1).abs() < 5 {
                self.path_index += 1;
                continue;
            }
            let dx = point.0 - position.0;
            let dy = point.1 - position.1;
            return if dx.abs() >= dy.abs() {
                if dx < 0 { LEFT } else { RIGHT }
            } else if dy < 0 {
                UP
            } else {
                DOWN
            };
        }
        // If the goal itself occupies a special collision cell, approach from
        // the nearest walkable endpoint then let the real elevator gate decide.
        if (position.0 - target.0).abs() > 4 {
            if position.0 < target.0 { RIGHT } else { LEFT }
        } else if position.1 < target.1 {
            DOWN
        } else {
            UP
        }
    }
    fn decode_terrain(&mut self, rom: &[u8], revision: Revision) {
        let resource = |name: &str| {
            tje_mem::rom_data_units(revision)
                .iter()
                .find(|unit| unit.name == name)
                .unwrap()
                .address as usize
        };
        let base = resource("LevelMetatileMaps");
        let overlay = resource("OverlayMetatileMaps");
        let lookup = resource("PlayerTilePropertyLookup");
        let word = |address: usize| u16::from_be_bytes([rom[address], rom[address + 1]]);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let cell = y * WIDTH + x;
                if x < 16 || y < 8 {
                    self.terrain[cell] = 0x90;
                    continue;
                }
                let map = self.map[(y / 8 - 1) * 19 + x / 16 - 1];
                let mut tile = 0;
                let over = usize::from((map & 0x1F00) >> 8);
                if over & 15 != 0 {
                    let col = if over & 16 != 0 { 15 - x % 16 } else { x % 16 };
                    tile = word(overlay + (((over & 15) - 1) * 128 + (y % 8) * 16 + col) * 2);
                }
                if tile == 0 {
                    let kind = usize::from(map as u8);
                    let col = if kind & 128 != 0 { 15 - x % 16 } else { x % 16 };
                    tile = word(base + ((kind & 127) * 128 + (y % 8) * 16 + col) * 2);
                }
                self.terrain[cell] = rom[lookup + usize::from(tile & 0x7FF)] & 0xF0;
            }
        }
    }
    fn plan(&mut self, start: (i16, i16), goal: (i16, i16)) {
        self.path.clear();
        self.path_index = 0;
        if start.0 < 0 || start.1 < 0 {
            return;
        }
        let start = (start.1 as usize / 8) * WIDTH + start.0 as usize / 8;
        if start >= CELLS {
            return;
        }
        self.parents.fill(usize::MAX);
        self.queue.clear();
        self.queue.push_back(start);
        self.parents[start] = start;
        let mut best = start;
        let distance = |cell: usize| {
            ((cell % WIDTH) as i32 * 8 + 4 - i32::from(goal.0)).abs()
                + ((cell / WIDTH) as i32 * 8 + 4 - i32::from(goal.1)).abs()
        };
        while let Some(cell) = self.queue.pop_front() {
            if distance(cell) < distance(best) {
                best = cell;
            }
            if distance(cell) <= 8 {
                best = cell;
                break;
            }
            for next in [
                cell.checked_sub(WIDTH),
                cell.checked_add(WIDTH).filter(|&next| next < CELLS),
                (cell % WIDTH > 0).then(|| cell - 1),
                (cell % WIDTH + 1 < WIDTH).then_some(cell + 1),
            ]
            .into_iter()
            .flatten()
            {
                if self.parents[next] == usize::MAX && self.terrain[next] <= 0x50 {
                    self.parents[next] = cell;
                    self.queue.push_back(next);
                }
            }
        }
        let mut cell = best;
        while cell != start {
            self.path.push(cell);
            cell = self.parents[cell];
        }
        self.path.reverse();
    }
}

pub fn generate(
    bytes: &[u8],
    scenario: Scenario,
    frames: u64,
    path: &Path,
) -> Result<ReplayReport> {
    ensure!(frames > 0, "bot needs at least one frame");
    let rom = tje_rom::Rom::import(bytes)?;
    let revision = rom.revision();
    let mut host = runner::host(rom.bytes(), Mode::Reference, revision, false)?;
    let mut planner = Planner::new(scenario);
    let mut coverage = Coverage::new();
    let mode = if scenario == Scenario::JamOut {
        "ordinary and random-sample Jam Out"
    } else {
        "Fixed World gameplay"
    };
    let mut session = Session::new(
        revision,
        format!("RAM-observing {scenario:?} bot; power-on, no RAM writes; {mode}"),
    );
    let mut previous = None;
    for frame in 0..frames {
        let pads = planner.choose(frame, previous.as_ref(), &host, rom.bytes(), revision);
        session.record(pads)?;
        let snapshot = host.run_frame(runner::inputs(pads))?;
        let observations = host.take_session_observations();
        planner.observe(&observations, revision);
        coverage.observe(&host, &snapshot, &observations, revision);
        if frame.is_multiple_of(1000) {
            let memory = Ram(&snapshot.ram);
            let players: [(i16, i16, i8); 2] = std::array::from_fn(|index| {
                let player = ram::g_player_info(revision).at(index as i32);
                (
                    player.x().read(&memory),
                    player.y().read(&memory),
                    player.level().read(&memory),
                )
            });
            eprintln!(
                "bot frame={frame} menu={} players={players:?}",
                planner.menu
            );
        }
        previous = Some(snapshot);
    }
    fs::write(path, session.to_json()?)?;
    Ok(runner::replay_report(
        path,
        &session,
        &host,
        coverage.content,
    ))
}
