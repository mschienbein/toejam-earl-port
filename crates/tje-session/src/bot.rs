//! A bounded deterministic input planner. It observes named RAM views and the
//! user's runtime ROM terrain resources; it never edits RAM or saves resources.
use crate::runner::{self, Coverage, Ram, ReplayReport};
use crate::{
    adventure::{Adventure, Objective},
    content_bot::ContentBot,
    navigation::Navigator,
};
use anyhow::{Result, ensure};
use clap::ValueEnum;
use std::{fs, path::Path};
use tje_host::{FrameSnapshot, Host, Mode, session_trace::FrameObservations};
use tje_mem::{Revision, ram};
use tje_session::{A, B, C, CONNECTED, DOWN, LEFT, RIGHT, START, Session, UP};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Scenario {
    OnePlayer,
    TwoPlayer,
    JamOut,
    Campaign,
    CampaignTwo,
    Explore,
    Coop,
    LevelZero,
    GameOver,
    JamAll,
    Intro,
    Menus,
    SecretEnding,
    ContentOne,
    ContentTwo,
}

impl Scenario {
    fn players(self) -> usize {
        match self {
            Self::TwoPlayer
            | Self::Coop
            | Self::JamOut
            | Self::JamAll
            | Self::ContentTwo
            | Self::CampaignTwo => 2,
            _ => 1,
        }
    }
}

struct Planner {
    scenario: Scenario,
    menu: &'static str,
    menu_frame: u64,
    game_frame: Option<u64>,
    jam_frame: Option<u64>,
    jam_round: u32,
    navigators: Option<[Navigator; 2]>,
    adventure: Option<Adventure>,
    content_bot: Option<ContentBot>,
    controller_visits: u32,
    intro_seen: bool,
    intro_complete: bool,
    menu_active: bool,
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
            controller_visits: 0,
            intro_seen: false,
            intro_complete: false,
            menu_active: true,
            navigators: matches!(scenario, Scenario::OnePlayer | Scenario::TwoPlayer)
                .then(|| std::array::from_fn(|_| Navigator::new())),
            content_bot: matches!(scenario, Scenario::ContentOne | Scenario::ContentTwo)
                .then(|| ContentBot::new(scenario.players())),
            adventure: match scenario {
                Scenario::Campaign | Scenario::CampaignTwo => {
                    Some(Adventure::new(Objective::Campaign))
                }
                Scenario::SecretEnding => Some(Adventure::new(Objective::Campaign)),
                Scenario::Explore => Some(Adventure::new(Objective::Explore)),
                Scenario::Coop => Some(Adventure::new(Objective::Coop)),
                Scenario::LevelZero => Some(Adventure::new(Objective::LevelZero)),
                Scenario::GameOver => Some(Adventure::new(Objective::GameOver)),
                Scenario::Menus => Some(Adventure::new(Objective::Campaign)),
                _ => None,
            },
        }
    }
    fn observe(&mut self, observations: &FrameObservations, revision: Revision) {
        if let Some(adventure) = &mut self.adventure {
            adventure.observe(observations, revision);
        }
        if let Some(content_bot) = &mut self.content_bot {
            content_bot.observe(observations, revision);
        }
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
                self.menu_active = true;
                if name == "MenuWhoHandler" {
                    self.game_frame = None;
                }
                if name == "MenuControllerOptionsHandler" {
                    self.controller_visits += 1;
                }
                if name == "MenuWhoHandler" && self.intro_seen {
                    self.intro_complete = true;
                }
                if name == "MenuWhoHandler" && self.jam_frame.take().is_some() {
                    self.jam_round += 1;
                }
            } else if name == "StartJamOut" {
                self.jam_frame = Some(entry.frame);
                self.menu = "";
            } else if name == "GameProgram" {
                self.menu_active = true;
                self.menu = "";
                self.game_frame = None;
                self.jam_frame = None;
            } else if name == "IntroVideo"
                && self.scenario == Scenario::Intro
                && self.menu == "MenuWhatHandler"
            {
                self.intro_seen = true;
                self.menu = "IntroVideo";
                self.menu_frame = entry.frame;
            } else if name == "ProcessInput" {
                self.menu_active = false;
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
        content: &runner::Content,
    ) -> [u16; 2] {
        let mut pads = [
            CONNECTED,
            if self.scenario.players() == 2 {
                CONNECTED
            } else {
                0
            },
        ];
        if let Some(start) = self.jam_frame {
            let age = frame - start;
            if age
                >= if self.scenario == Scenario::JamAll {
                    2400
                } else {
                    2000
                }
            {
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
        if host.measurements.game_frame.is_none() || self.menu_active {
            if self.menu == "IntroVideo" {
                return pads;
            }
            let age = frame.saturating_sub(self.menu_frame);
            let down_count = match self.menu {
                "MenuWhoHandler" => u64::from(self.scenario.players() == 1),
                "MenuWhatHandler" => {
                    if matches!(self.scenario, Scenario::JamOut | Scenario::JamAll) {
                        1
                    } else if self.scenario == Scenario::Intro {
                        2
                    } else if self.scenario == Scenario::Menus && self.controller_visits < 2 {
                        3
                    } else {
                        4
                    }
                }
                "MenuJamOutHandler" => {
                    if self.scenario == Scenario::JamAll {
                        u64::from(self.jam_round % 8)
                    } else if self.jam_round == 0 {
                        1
                    } else {
                        7
                    }
                }
                "MenuControllerOptionsHandler" => u64::from(self.controller_visits == 1),
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
        if self.scenario == Scenario::Menus && age < 1200 {
            for (start, button) in [
                (100, START),
                (180, START),
                (260, C),
                (400, C),
                (520, B),
                (600, C),
                (680, A),
                (760, B),
            ] {
                if (start..start + 12).contains(&age) {
                    pads[0] |= button;
                }
            }
            return pads;
        }
        if self.scenario == Scenario::SecretEnding && age < 700 {
            // Original reachable pause sequence, not a RAM intervention. It
            // removes nine assignments; coverage must not count them collected.
            for (start, button) in [
                (80, START),
                (180, UP | A | B | C),
                (260, RIGHT | A),
                (340, DOWN | B),
                (420, LEFT | C),
                (500, START),
            ] {
                if (start..start + 16).contains(&age) {
                    pads[0] |= button;
                }
            }
            return pads;
        }
        if let Some(content_bot) = &mut self.content_bot {
            content_bot.choose(&memory, rom, revision, content, frame, &mut pads);
            logical_controls(&mut pads, &memory, rom, revision, frame);
            return pads;
        }
        if let Some(adventure) = &mut self.adventure {
            adventure.choose(&memory, rom, revision, content, frame, &mut pads);
            logical_controls(&mut pads, &memory, rom, revision, frame);
            return pads;
        }
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
                    *pad |= self
                        .navigators
                        .as_mut()
                        .expect("legacy navigation scenario")[player]
                        .steer(&memory, rom, revision, player, (x, y), frame)
                        .unwrap_or(0);
                }
            }
        }
        pads
    }
}

fn logical_controls(
    pads: &mut [u16; 2],
    memory: &Ram<'_>,
    rom: &[u8],
    revision: Revision,
    frame: u64,
) {
    for (player, pad) in pads.iter_mut().enumerate() {
        if *pad == 0 {
            continue;
        }
        if *pad & 15 != 0
            && ram::g_player_powerups(revision)
                .at(player as i32)
                .read(memory)
                == 0
            && ram::g_player_inventory_open(revision)
                .at(player as i32)
                .read(memory)
                == 0
            && ram::g_player_interaction_choice_active(revision)
                .at(player as i32)
                .read(memory)
                == 0
        {
            let view = ram::g_player_info(revision).at(player as i32);
            let near_elevator = (0..2).any(|bank| {
                let elevator = ram::g_elevator_records(revision).at(bank).at(0);
                elevator.level().read(memory) == view.level().read(memory)
                    && (elevator.x().read(memory) - view.x().read(memory)).abs() < 24
                    && (elevator.y().read(memory) - view.y().read(memory)).abs() < 48
            });
            let near_pickup = (0..2).any(|bank| {
                (0..28).any(|index| {
                    let item = ram::g_object_table(revision).at(bank).at(index);
                    item.kind().read(memory) >= 0
                        && item.level().read(memory) == view.level().read(memory)
                        && (item.x().read(memory) - view.x().read(memory)).abs() < 20
                        && (item.y().read(memory) - view.y().read(memory)).abs() < 16
                })
            });
            let near_service = (0..2).any(|bank| {
                (0..20).any(|index| {
                    let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                    matches!(earthling.kind().read(memory),0 | 9..=11 | 17 | 23..=24)
                        && earthling.level().read(memory) == view.level().read(memory)
                        && (earthling.x().read(memory) - view.x().read(memory)).abs() < 40
                        && (earthling.y().read(memory) - view.y().read(memory)).abs() < 24
                })
            });
            if !near_elevator && !near_pickup && !near_service && frame % 24 < 12 {
                if !(60..64).contains(&view.sprite_state().read(memory)) {
                    *pad &= !15;
                }
                *pad |= A;
            }
        }
        let direction = *pad & 15;
        if direction != 0
            && ram::g_player_control_remap_entities(revision)
                .at(player as i32)
                .read(memory)
                != 0
            && ram::g_player_inventory_open(revision)
                .at(player as i32)
                .read(memory)
                == 0
            && ram::g_player_interaction_choice_active(revision)
                .at(player as i32)
                .read(memory)
                == 0
        {
            let resource = tje_mem::rom_data_units(revision)
                .iter()
                .find(|unit| unit.name == "MapGenerationLookupTables")
                .unwrap();
            let mapping = tje_mem::Address::<tje_mem::MapGenerationTables>::new(resource.address)
                .input_directions()
                .at(i32::from(
                    ram::g_player_control_remap_indices(revision)
                        .at(player as i32)
                        .read(memory),
                ));
            if let Some(physical) = [
                UP,
                DOWN,
                LEFT,
                RIGHT,
                UP | LEFT,
                UP | RIGHT,
                DOWN | LEFT,
                DOWN | RIGHT,
            ]
            .into_iter()
            .find(|physical| {
                u16::from(rom[mapping.at(i32::from(*physical) - 1).address() as usize]) == direction
            }) {
                *pad = (*pad & !15) | physical;
            }
        }
        if ram::g_controller_buttons_ac_swapped(revision).read(memory) != 0 {
            *pad = (*pad & !(A | C)) | ((*pad & A) << 2) | ((*pad & C) >> 2);
        }
    }
}

pub fn generate(
    bytes: &[u8],
    scenario: Scenario,
    frames: u64,
    path: &Path,
    prefix: Option<&Path>,
) -> Result<ReplayReport> {
    ensure!(frames > 0, "bot needs at least one frame");
    let rom = tje_rom::Rom::import(bytes)?;
    let revision = rom.revision();
    let prefix = prefix
        .map(fs::read_to_string)
        .transpose()?
        .map(|json| Session::from_json(&json))
        .transpose()?;
    if let Some(prefix) = &prefix {
        ensure!(
            Revision::from(prefix.revision) == revision,
            "prefix cartridge revision differs"
        );
        ensure!(
            matches!(prefix.start, tje_session::Start::PowerOn),
            "bot prefixes must be input-only power-on sessions"
        );
        ensure!(
            prefix.frames <= frames,
            "frame limit is shorter than the prefix"
        );
    }
    let mut prefix_inputs = prefix.as_ref().map(Session::inputs);
    let mut host = runner::host(rom.bytes(), Mode::Reference, revision, false)?;
    let mut planner = Planner::new(scenario);
    let mut coverage = Coverage::new();
    let mode = match scenario {
        Scenario::JamOut => "ordinary and random-sample Jam Out",
        Scenario::JamAll => "all eight Jam Out modes",
        Scenario::Intro => "full natural intro selected from the menu",
        Scenario::Menus => "controller options, pause, map and inventory",
        Scenario::SecretEnding => {
            "original hidden pause sequence clears nine assignments; not a ten-piece campaign"
        }
        _ => "Fixed World gameplay",
    };
    let mut session = Session::new(
        revision,
        format!("RAM-observing {scenario:?} bot; power-on, no RAM writes; {mode}"),
    );
    let mut previous = None;
    for frame in 0..frames {
        let pads = prefix_inputs
            .as_mut()
            .and_then(Iterator::next)
            .unwrap_or_else(|| {
                planner.choose(
                    frame,
                    previous.as_ref(),
                    &host,
                    rom.bytes(),
                    revision,
                    &coverage.content,
                )
            });
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
        if planner.content_bot.as_ref().is_some_and(|content_bot| {
            content_bot.complete(
                &Ram(&previous.as_ref().unwrap().ram),
                revision,
                &coverage.content,
                frame,
            )
        }) {
            break;
        }
        if matches!(
            scenario,
            Scenario::Campaign | Scenario::CampaignTwo | Scenario::SecretEnding
        ) && (scenario == Scenario::SecretEnding
            || coverage
                .content
                .ship_pieces_collected
                .values()
                .all(Option::is_some))
            && planner.adventure.as_ref().is_some_and(|adventure| {
                adventure.campaign_complete(&Ram(&previous.as_ref().unwrap().ram), revision, frame)
            })
            && prefix.as_ref().is_none_or(|prefix| frame >= prefix.frames)
        {
            break;
        }
        if (scenario == Scenario::JamAll && planner.jam_round >= 8)
            || (scenario == Scenario::Intro && planner.intro_complete)
            || (scenario == Scenario::Menus
                && planner
                    .game_frame
                    .is_some_and(|start| frame - start >= 1200))
            || (scenario == Scenario::GameOver
                && coverage
                    .content
                    .milestones
                    .get("game_over")
                    .and_then(|frame| *frame)
                    .is_some_and(|start| frame >= start + 300)
                && planner.menu_active
                && planner.menu == "MenuWhoHandler")
            || (scenario == Scenario::Coop
                && planner.adventure.as_ref().is_some_and(|adventure| {
                    adventure.coop_complete(&Ram(&previous.as_ref().unwrap().ram), revision, frame)
                }))
            || (scenario == Scenario::LevelZero
                && planner.adventure.as_ref().is_some_and(|adventure| {
                    adventure.hidden_level_complete(
                        &Ram(&previous.as_ref().unwrap().ram),
                        revision,
                        &coverage.content,
                        frame,
                    )
                }))
        {
            break;
        }
    }
    fs::write(path, session.to_json()?)?;
    runner::replay_report(path, &session, &host, coverage.content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_two_player_campaign_uses_both_pads_and_the_real_adventure_policy() {
        let planner = Planner::new(Scenario::CampaignTwo);
        assert_eq!(planner.scenario.players(), 2);
        assert!(planner.adventure.is_some());
        assert!(planner.content_bot.is_none());
        assert!(planner.navigators.is_none());
    }
}
