//! Goal-directed inputs; every action still travels through the original pad I/O.
use crate::{
    navigation::{Navigator, Travel},
    runner::{Content, Ram, entry_name},
};
use tje_host::session_trace::FrameObservations;
use tje_mem::{Revision, ram};
use tje_session::{A, B, C, DOWN, LEFT, RIGHT, UP};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Objective {
    Campaign,
    Explore,
    Coop,
    LevelZero,
    GameOver,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Goal {
    Ship { bank: i32, index: i32 },
    Item { bank: i32, index: i32 },
    Earthling { bank: i32, index: i32 },
    Elevator { bank: i32 },
    Present { slot: usize, kind: i8 },
    Fall,
    Drown,
    Doorway,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scene {
    Game,
    Acknowledge,
    Map,
    Ending,
    GameOver,
}

pub struct Adventure {
    objective: Objective,
    navigation: [Navigator; 2],
    goals: [Option<Goal>; 2],
    goal_since: [u64; 2],
    levels: [i8; 2],
    level_since: [u64; 2],
    blocked: Vec<(i8, Goal)>,
    scene: Scene,
    scene_since: u64,
    last_inventory_action: [u64; 2],
    last_inventory_kind: [i8; 2],
    inventory_since: [u64; 2],
    inventory_kinds: [i8; 2],
    held_action: [u16; 2],
    held_until: [u64; 2],
    mobility_needed: [bool; 2],
    doorway_used: [bool; 2],
    bath_since: [Option<u64>; 2],
    arrival_exit: [Option<i16>; 2],
    coop_phase: u8,
    coop_targets: Option<[(i16, i16); 2]>,
    coop_center: (i16, i16),
    final_entry: Option<u64>,
    last_status: u64,
}
impl Adventure {
    pub fn new(objective: Objective) -> Self {
        Self {
            objective,
            navigation: std::array::from_fn(|_| Navigator::new()),
            goals: [None; 2],
            goal_since: [0; 2],
            levels: [-1; 2],
            level_since: [0; 2],
            blocked: Vec::new(),
            scene: Scene::Game,
            scene_since: 0,
            last_inventory_action: [0; 2],
            inventory_since: [0; 2],
            inventory_kinds: [0; 2],
            bath_since: [None; 2],
            arrival_exit: [None; 2],
            held_action: [0; 2],
            held_until: [0; 2],
            mobility_needed: [false; 2],
            doorway_used: [false; 2],
            coop_phase: 0,
            coop_targets: None,
            coop_center: (0, 0),
            last_inventory_kind: [-1; 2],
            final_entry: None,
            last_status: 0,
        }
    }
    pub fn observe(&mut self, observations: &FrameObservations, revision: Revision) {
        for event in &observations.entries {
            match entry_name(revision, event.address) {
                "WaitForShipScreenAcknowledgement" => {
                    self.scene = Scene::Acknowledge;
                    self.scene_since = event.frame;
                }
                "DrawGameMap" => {
                    self.scene = Scene::Map;
                    self.scene_since = event.frame;
                }
                "PlayEndingSequence" => {
                    self.scene = Scene::Ending;
                    self.scene_since = event.frame;
                }
                "MapEnterFinalLevel" => {
                    self.scene = Scene::Game;
                    self.final_entry = Some(event.frame);
                }
                "ShowGameOverAndReset" => {
                    self.scene = Scene::GameOver;
                    self.scene_since = event.frame;
                }
                "PlayerTeleportThroughDoorway" => {
                    let player = event.stack_slots[0] as u16 as usize;
                    if player < 2 {
                        self.doorway_used[player] = true;
                        self.mobility_needed[player] = false;
                        self.goals[player] = None;
                    }
                }
                "MenuWhoHandler" => {
                    self.scene = Scene::Game;
                    self.blocked.clear();
                    self.levels = [-1; 2];
                    self.arrival_exit = [None; 2];
                }
                "ProcessInput" if self.scene != Scene::Ending => self.scene = Scene::Game,
                _ => {}
            }
        }
    }
    pub fn choose(
        &mut self,
        memory: &Ram<'_>,
        rom: &[u8],
        revision: Revision,
        content: &Content,
        frame: u64,
        pads: &mut [u16; 2],
    ) {
        match self.scene {
            Scene::Acknowledge => {
                return;
            }
            Scene::Map => {
                if frame - self.scene_since > 100 && (frame - self.scene_since) % 48 < 12 {
                    pads[0] |= C;
                }
                return;
            }
            Scene::Ending => return,
            Scene::GameOver => {
                return;
            }
            Scene::Game => {}
        }
        if self.coop_inputs(memory, rom, revision, frame, pads) {
            return;
        }
        for (player, pad) in pads.iter_mut().enumerate() {
            if *pad == 0 {
                continue;
            }
            if frame < self.held_until[player] {
                *pad |= self.held_action[player];
                continue;
            }
            let view = ram::g_player_info(revision).at(player as i32);
            let level = view.level().read(memory);
            if self.levels[player] != level {
                self.levels[player] = level;
                self.level_since[player] = frame;
                self.goals[player] = None;
            }
            if ram::g_player_lives(revision).at(player as i32).read(memory) < 0 {
                self.mobility_needed[player] = false;
                continue;
            }
            if view.sprite_state().read(memory) == 119 {
                let since = *self.bath_since[player].get_or_insert(frame);
                if frame - since > 2400 {
                    *pad |= if player == 0 { LEFT } else { RIGHT };
                }
                continue;
            }
            if matches!(view.sprite_state().read(memory), 116 | 117) {
                if frame % 48 < 12 {
                    *pad |= A;
                }
                continue;
            }
            if ram::g_player_fall_states(revision)
                .at(player as i32)
                .read(memory)
                != 0
            {
                continue;
            }
            let elevator = ram::g_player_elevator_flags(revision)
                .at(player as i32)
                .read(memory);
            if elevator != 0 {
                if elevator & 1 == 0 {
                    let arrival = ram::g_elevator_records(revision)
                        .at((i32::from(elevator) - 1) / 2)
                        .at(1);
                    self.arrival_exit[player] = Some(arrival.y().read(memory) + 40);
                    *pad |= DOWN;
                }
                continue;
            }
            if let Some(exit_y) = self.arrival_exit[player] {
                if view.y().read(memory) < exit_y {
                    *pad |= DOWN;
                    continue;
                }
                self.arrival_exit[player] = None;
            }
            if ram::g_player_interaction_choice_active(revision)
                .at(player as i32)
                .read(memory)
                != 0
            {
                if frame % 48 < 12 {
                    *pad |= UP;
                } else if frame % 48 >= 24 && frame % 48 < 36 {
                    *pad |= A;
                }
                continue;
            }
            let inventory = ram::g_player_inventory_open(revision)
                .at(player as i32)
                .read(memory);
            if inventory != self.inventory_kinds[player] {
                self.inventory_kinds[player] = inventory;
                self.inventory_since[player] = frame;
            }
            if inventory != 0 {
                if matches!(self.goals[player], Some(Goal::Present { .. })) {
                    self.inventory_input(memory, revision, player, frame, pad);
                } else {
                    let age = frame - self.inventory_since[player];
                    if inventory == 1 {
                        if age % 64 < 12 {
                            *pad |= B;
                        }
                    } else if (64..76).contains(&age) {
                        *pad |= A;
                    } else if age >= 128 && age % 64 < 12 {
                        *pad |= B;
                    }
                }
                continue;
            }
            if let Some(Goal::Present { kind, .. }) = self.goals[player] {
                if self.last_inventory_kind[player] == kind
                    && frame - self.last_inventory_action[player] < 140
                {
                    continue;
                }
                self.goals[player] = None;
            }
            let health = ram::g_player_health(revision)
                .at(player as i32)
                .read(memory);
            let position = (view.x().read(memory), view.y().read(memory));
            if self.goals[player].is_none() || frame - self.goal_since[player] > 7200 {
                if let Some(goal) = self.goals[player] {
                    self.blocked.push((level, goal));
                }
                self.goals[player] =
                    self.select_goal(memory, rom, revision, content, player, health);
                self.goal_since[player] = frame;
            }
            if let Some(goal) = self.goals[player] {
                match goal {
                    Goal::Present { slot: _, kind } => {
                        if frame - self.last_inventory_action[player] > 120 && frame % 32 < 12 {
                            *pad |= B;
                            self.last_inventory_kind[player] = kind;
                        }
                    }
                    Goal::Ship { bank, index } | Goal::Earthling { bank, index } => {
                        let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                        if earthling.kind().read(memory) < 0
                            || earthling.level().read(memory) != level
                        {
                            self.goals[player] = None;
                            continue;
                        }
                        let kind = earthling.kind().read(memory);
                        if matches!(goal, Goal::Earthling { .. }) && friendly_done(content, kind) {
                            self.blocked.push((level, goal));
                            self.goals[player] = None;
                            continue;
                        }
                        if kind == 23 {
                            let unit = tje_mem::rom_data_units(revision)
                                .iter()
                                .find(|unit| unit.name == "EARTHLING_DATA")
                                .unwrap();
                            let definition =
                                tje_mem::Address::<tje_mem::EarthlingDefinition>::new(unit.address)
                                    .offset(i32::from(kind));
                            let range_x =
                                i16::from(rom[definition.contact_distance_x().address() as usize]);
                            let range_y =
                                i16::from(rom[definition.contact_distance_y().address() as usize]);
                            if (position.0 - earthling.x().read(memory)).abs() < range_x
                                && position.1 > earthling.y().read(memory)
                                && position.1 - earthling.y().read(memory) < range_y
                            {
                                *pad |= UP;
                                continue;
                            }
                        }
                        let mut target =
                            (earthling.x().read(memory), earthling.y().read(memory) + 6);
                        if matches!(goal, Goal::Earthling { .. }) && matches!(kind, 9..=11 | 17) {
                            target.1 = earthling.y().read(memory) + 12;
                        }
                        if matches!(goal, Goal::Earthling { .. }) && kind == 24 {
                            target = (
                                earthling.x().read(memory) - 48,
                                earthling.y().read(memory) - 4,
                            );
                        }
                        if matches!(goal, Goal::Earthling { .. }) && kind == 23 {
                            let x = earthling.x().read(memory);
                            let y = earthling.y().read(memory);
                            target = if position.1 < y + 20 {
                                if position.0 > x - 60 {
                                    (x - 64, position.1)
                                } else {
                                    (position.0, y + 24)
                                }
                            } else {
                                (x, y + 16)
                            };
                            if (position.0 - x).abs() < 160 && (position.1 - y).abs() < 128 {
                                *pad |= if position.1 < y + 16 {
                                    if position.0 > x - 60 { LEFT } else { DOWN }
                                } else if (position.0 - x).abs() > 4 {
                                    if position.0 < x { RIGHT } else { LEFT }
                                } else {
                                    UP
                                };
                                continue;
                            }
                        }
                        if matches!(goal, Goal::Ship { .. }) {
                            let approaches = [
                                (0, 6),
                                (0, 12),
                                (-10, 0),
                                (10, 0),
                                (0, -6),
                                (-18, 4),
                                (18, 4),
                                (0, 20),
                            ];
                            let attempt = self
                                .blocked
                                .iter()
                                .filter(|&&(old_level, old)| old_level == level && old == goal)
                                .count();
                            let offset = approaches[attempt % approaches.len()];
                            target = (
                                earthling.x().read(memory) + offset.0,
                                earthling.y().read(memory) + offset.1,
                            );
                        }
                        let dx = (position.0 - target.0).abs();
                        let dy = (position.1 - target.1).abs();
                        if dx < 12 && dy < 4 {
                            if kind == 24 {
                                *pad |= RIGHT;
                            }
                            if kind == 23 {
                                *pad |= UP;
                            }
                            if matches!(goal, Goal::Earthling { .. }) && frame % 64 < 12 {
                                *pad |= A;
                            }
                            if frame - self.goal_since[player] > 180
                                && content
                                    .earthlings_encountered
                                    .get(&(earthling.kind().read(memory) as u8))
                                    .is_some_and(Option::is_some)
                                && !matches!(earthling.kind().read(memory), 0 | 9..=11 | 17 | 23..=24)
                            {
                                self.blocked.push((level, goal));
                                self.goals[player] = None;
                            }
                        } else if let Some(input) = self.navigation[player]
                            .steer(memory, rom, revision, player, target, frame)
                        {
                            *pad |= input;
                        } else {
                            self.mobility_needed[player] |= matches!(goal, Goal::Ship { .. });
                            self.blocked.push((level, goal));
                            self.goals[player] = None;
                        }
                    }
                    Goal::Item { bank, index } => {
                        let item = ram::g_object_table(revision).at(bank).at(index);
                        if item.kind().read(memory) < 0 || item.level().read(memory) != level {
                            self.goals[player] = None;
                            continue;
                        }
                        let target = (item.x().read(memory), item.y().read(memory));
                        if let Some(input) = self.navigation[player]
                            .steer(memory, rom, revision, player, target, frame)
                        {
                            *pad |= input;
                        } else {
                            self.blocked.push((level, goal));
                            self.goals[player] = None;
                        }
                    }
                    Goal::Elevator { bank } => {
                        let elevator = ram::g_elevator_records(revision).at(bank).at(0);
                        let target = (elevator.x().read(memory), elevator.y().read(memory) + 22);
                        let state = elevator.state().read(memory);
                        let dy = position.1 - elevator.y().read(memory);
                        if (position.0 - target.0).abs() < 8 && (0..40).contains(&dy) {
                            if state == 3 {
                                *pad |= UP;
                            } else if state <= 1 {
                                if dy < 12 {
                                    *pad |= DOWN;
                                } else if dy > 18 {
                                    *pad |= UP;
                                }
                                // Stay in the 10..20 approach strip until the
                                // original elevator update opens the doorway.
                            }
                        } else if let Some(input) = self.navigation[player]
                            .steer(memory, rom, revision, player, target, frame)
                        {
                            *pad |= input;
                        } else {
                            self.mobility_needed[player] = true;
                            self.goals[player] = None;
                        }
                    }
                    Goal::Doorway => {
                        let handle = ram::g_player_doorway_entities(revision)
                            .at(player as i32)
                            .read(memory);
                        if handle <= 0 {
                            self.goals[player] = None;
                            continue;
                        }
                        let door = ram::g_player_info(revision).at(i32::from(handle));
                        let target = (door.x().read(memory), door.y().read(memory) + 12);
                        let approach = position.1 - door.y().read(memory);
                        if (position.0 - target.0).abs() < 5 && (0..32).contains(&approach) {
                            // Keep walking into the centered contact footprint:
                            // its owner handler opens even the initially closed door.
                            *pad |= UP;
                        } else if let Some(input) = self.navigation[player]
                            .steer(memory, rom, revision, player, target, frame)
                        {
                            *pad |= input;
                        } else {
                            self.goals[player] = None;
                        }
                    }
                    Goal::Fall => {
                        // Refresh the map first, then select the closest actual
                        // void tile rather than inventing a destination/snapshot.
                        let _ = self.navigation[player]
                            .steer(memory, rom, revision, player, position, frame);
                        if let Some(target) =
                            self.navigation[player].nearest_terrain(position, 0x90)
                        {
                            let direction = self.navigation[player].toward(
                                memory,
                                rom,
                                revision,
                                player,
                                (target, Travel::Fall),
                                frame,
                            );
                            if ram::g_player_powerups(revision)
                                .at(player as i32)
                                .read(memory)
                                == 0
                            {
                                let distance = (i32::from(position.0) - i32::from(target.0)).abs()
                                    + (i32::from(position.1) - i32::from(target.1)).abs();
                                let flight = (60..64).contains(&view.sprite_state().read(memory));
                                if !flight && frame % 24 < 12 {
                                    *pad |= A;
                                } else if let Some(direction) = direction {
                                    *pad |= direction;
                                    if distance > 16 && frame % 24 < 12 {
                                        *pad |= A;
                                    }
                                }
                            } else if let Some(direction) = direction {
                                *pad |= direction;
                            }
                        }
                    }
                    Goal::Drown => {
                        let _ = self.navigation[player]
                            .steer(memory, rom, revision, player, position, frame);
                        if self.navigation[player].terrain_at(position) != 0x80
                            && let Some(target) =
                                self.navigation[player].nearest_terrain(position, 0x80)
                            && let Some(input) = self.navigation[player].toward(
                                memory,
                                rom,
                                revision,
                                player,
                                (target, Travel::Swim),
                                frame,
                            )
                        {
                            *pad |= input;
                        }
                    }
                }
            }
        }
        if frame - self.last_status >= 1000 {
            self.last_status = frame;
            let stats: [(i8, i16, i16, i8, i8, i8); 2] = std::array::from_fn(|player| {
                let view = ram::g_player_info(revision).at(player as i32);
                (
                    view.level().read(memory),
                    view.x().read(memory),
                    view.y().read(memory),
                    ram::g_player_health(revision)
                        .at(player as i32)
                        .read(memory),
                    ram::g_player_lives(revision).at(player as i32).read(memory),
                    ram::g_player_elevator_flags(revision)
                        .at(player as i32)
                        .read(memory),
                )
            });
            eprintln!(
                "goals frame={frame} players={stats:?} targets={:?} blocked={}",
                self.goals,
                self.blocked.len()
            );
            let details: [[i32; 12]; 2] = std::array::from_fn(|player| {
                let index = player as i32;
                let view = ram::g_player_info(revision).at(index);
                [
                    i32::from(
                        ram::g_player_inventory_open(revision)
                            .at(index)
                            .read(memory),
                    ),
                    i32::from(
                        ram::g_player_interaction_choice_active(revision)
                            .at(index)
                            .read(memory),
                    ),
                    i32::from(
                        ram::g_player_interaction_responses(revision)
                            .at(index)
                            .read(memory),
                    ),
                    i32::from(view.sprite_state().read(memory)),
                    i32::from(view.draw_flags().read(memory)),
                    i32::from(view.behavior_flags().read(memory)),
                    i32::from(ram::g_player_fall_states(revision).at(index).read(memory)),
                    i32::from(ram::g_player_powerups(revision).at(index).read(memory)),
                    i32::from(ram::g_player_rank(revision).at(index).read(memory)),
                    i32::from(ram::g_player_points(revision).at(index).read(memory)),
                    i32::from(
                        ram::g_player_processed_input(revision)
                            .at(index)
                            .read(memory),
                    ),
                    i32::from(
                        ram::g_player_previous_processed_input(revision)
                            .at(index)
                            .read(memory),
                    ),
                ]
            });
            eprintln!(
                "state inv/choice/response/sprite/draw/behavior/fall/power/rank/points/input/previous={details:?}"
            );
            let elevators: [[i16; 4]; 4] = std::array::from_fn(|index| {
                let elevator = ram::g_elevator_records(revision)
                    .at((index / 2) as i32)
                    .at((index % 2) as i32);
                [
                    i16::from(elevator.level().read(memory)),
                    i16::from(elevator.state().read(memory)),
                    elevator.x().read(memory),
                    elevator.y().read(memory),
                ]
            });
            eprintln!("elevators level/state/x/y={elevators:?}");
            let terrain: [u8; 5] = [7, 11, 12, 13, 17].map(|index| {
                ram::g_player1_nearby_terrain(revision)
                    .at(index / 5)
                    .at(index % 5)
                    .read(memory)
            });
            eprintln!("terrain up/left/center/right/down={terrain:?}");
            for bank in 0..2 {
                for index in 0..20 {
                    let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                    let kind = earthling.kind().read(memory);
                    if matches!(kind,9..=11|17|23..=24)
                        && earthling.level().read(memory) == stats[0].0
                    {
                        let handle = earthling.object().read(memory);
                        let object = ram::g_player_info(revision).at(i32::from(handle));
                        let unit = tje_mem::rom_data_units(revision)
                            .iter()
                            .find(|unit| unit.name == "EARTHLING_DATA")
                            .unwrap();
                        let definition =
                            tje_mem::Address::<tje_mem::EarthlingDefinition>::new(unit.address)
                                .offset(i32::from(kind));
                        eprintln!(
                            "friendly kind={kind} handle={handle} pos={:?} size={:?} contact={:?} sprite={}",
                            (object.x().read(memory), object.y().read(memory)),
                            (
                                object.width().read(memory),
                                object.height_pixels().read(memory)
                            ),
                            (
                                rom[definition.contact_distance_x().address() as usize],
                                rom[definition.contact_distance_y().address() as usize]
                            ),
                            object.sprite_state().read(memory)
                        );
                    }
                }
            }
            if let Some(Goal::Elevator { bank }) =
                self.select_goal(memory, rom, revision, content, 0, stats[0].3)
            {
                let view = ram::g_elevator_records(revision).at(bank).at(0);
                let target = (view.x().read(memory), view.y().read(memory) + 22);
                let position = (stats[0].1, stats[0].2);
                let nearest = self.navigation[0].reachable_land_near(position, target);
                eprintln!(
                    "route target={target:?} terrain={} reachable-nearest={nearest:?}",
                    self.navigation[0].terrain_at(target)
                );
            }
        }
    }

    fn coop_inputs(
        &mut self,
        memory: &Ram<'_>,
        rom: &[u8],
        revision: Revision,
        frame: u64,
        pads: &mut [u16; 2],
    ) -> bool {
        if self.objective != Objective::Coop || self.coop_phase >= 3 {
            return false;
        }
        let views = [
            ram::g_player_info(revision).at(0),
            ram::g_player_info(revision).at(1),
        ];
        if views.iter().any(|view| view.level().read(memory) != 1) {
            return false;
        }
        let positions = views.map(|view| (view.x().read(memory), view.y().read(memory)));
        if self.coop_targets.is_none() {
            for (player, position) in positions.iter().copied().enumerate() {
                let _ =
                    self.navigation[player].steer(memory, rom, revision, player, position, frame);
            }
            self.coop_center = (
                (positions[0].0 + positions[1].0) / 2,
                (positions[0].1 + positions[1].1) / 2,
            );
            let desired = [
                (positions[0].0 - 180, positions[0].1 - 160),
                (positions[1].0 + 180, positions[1].1 + 160),
            ];
            let targets: [Option<(i16, i16)>; 2] = std::array::from_fn(|player| {
                self.navigation[player].reachable_land_near(positions[player], desired[player])
            });
            let [Some(first), Some(second)] = targets else {
                return true;
            };
            self.coop_targets = Some([first, second]);
            eprintln!(
                "coop positions={positions:?} targets={:?} center={:?} terrain={:?}",
                self.coop_targets,
                self.coop_center,
                positions.map(|position| self.navigation[0].terrain_at(position))
            );
        }
        if self.coop_phase == 0 {
            let targets = self.coop_targets.unwrap();
            for (player, pad) in pads.iter_mut().enumerate() {
                let input = self.navigation[player].steer(
                    memory,
                    rom,
                    revision,
                    player,
                    targets[player],
                    frame,
                );
                if let Some(input) = input {
                    *pad |= input;
                } else {
                    let desired = if player == 0 {
                        (self.coop_center.0 - 200, self.coop_center.1 - 160)
                    } else {
                        (self.coop_center.0 + 200, self.coop_center.1 + 160)
                    };
                    if let Some(target) =
                        self.navigation[player].reachable_land_near(positions[player], desired)
                    {
                        self.coop_targets.as_mut().unwrap()[player] = target;
                    }
                }
            }
            if ram::g_split_screen_enabled(revision).read(memory) != 0 {
                self.coop_phase = 1;
            }
            return true;
        }
        if self.coop_phase == 1 {
            if views
                .iter()
                .any(|view| matches!(view.sprite_state().read(memory), 90 | 91))
            {
                self.coop_phase = 2;
                return true;
            }
            let dx = positions[1].0 - positions[0].0;
            let dy = positions[1].1 - positions[0].1;
            if dx.abs() < 80 && dy.abs() < 24 {
                if dy.abs() > 3 {
                    pads[1] |= if dy < 0 { DOWN } else { UP };
                } else {
                    pads[0] |= if dx > 0 { RIGHT } else { LEFT };
                    pads[1] |= if dx > 0 { LEFT } else { RIGHT };
                }
            } else {
                for (player, pad) in pads.iter_mut().enumerate() {
                    if let Some(input) = self.navigation[player].steer(
                        memory,
                        rom,
                        revision,
                        player,
                        self.coop_center,
                        frame,
                    ) {
                        *pad |= input;
                    }
                }
            }
            return true;
        }
        if views
            .iter()
            .all(|view| !matches!(view.sprite_state().read(memory), 90 | 91))
        {
            self.coop_phase = 3;
        }
        true
    }

    pub fn campaign_complete(&self, memory: &Ram<'_>, revision: Revision, frame: u64) -> bool {
        self.final_entry.is_some_and(|start| frame >= start + 300)
            && ram::g_player_info(revision).at(0).level().read(memory) == 26
            && ram::g_v_blank_graphics_enabled(revision).read(memory) != 0
    }

    pub fn hidden_level_complete(
        &self,
        memory: &Ram<'_>,
        revision: Revision,
        content: &Content,
        frame: u64,
    ) -> bool {
        self.objective == Objective::LevelZero
            && friendly_done(content, 23)
            && friendly_done(content, 24)
            && ram::g_player_info(revision).at(0).level().read(memory) > 0
            && ram::g_player_fall_states(revision).at(0).read(memory) == 0
            && frame - self.level_since[0] > 180
    }

    pub fn coop_complete(&self, memory: &Ram<'_>, revision: Revision, frame: u64) -> bool {
        self.objective == Objective::Coop
            && self.coop_phase >= 3
            && (0..2).all(|player| {
                ram::g_player_info(revision).at(player).level().read(memory) == 2
                    && ram::g_player_elevator_flags(revision)
                        .at(player)
                        .read(memory)
                        == 0
                    && self.arrival_exit[player as usize].is_none()
                    && frame - self.level_since[player as usize] > 300
            })
    }

    fn select_goal(
        &self,
        memory: &Ram<'_>,
        rom: &[u8],
        revision: Revision,
        content: &Content,
        player: usize,
        health: i8,
    ) -> Option<Goal> {
        let level = self.levels[player];
        if self.objective == Objective::LevelZero
            && level == 0
            && friendly_done(content, 23)
            && friendly_done(content, 24)
        {
            return Some(Goal::Fall);
        }
        if self.objective == Objective::GameOver {
            return Some(Goal::Drown);
        }
        let inventory = ram::g_player_inventory(revision).at(player as i32);
        if !self.doorway_used[player]
            && self.last_inventory_kind[player] == 8
            && ram::g_player_doorway_entities(revision)
                .at(player as i32)
                .read(memory)
                > 0
        {
            return Some(Goal::Doorway);
        }
        if self.mobility_needed[player]
            && ram::g_player_powerups(revision)
                .at(player as i32)
                .read(memory)
                != 0
            && let Some(slot) = (0..16).find(|&slot| inventory.at(slot as i32).read(memory) == 0)
        {
            return Some(Goal::Present { slot, kind: 0 });
        }
        if self.mobility_needed[player]
            && ram::g_player_powerups(revision)
                .at(player as i32)
                .read(memory)
                != 0
            && let Some(slot) = (0..16).find(|&slot| inventory.at(slot as i32).read(memory) == 8)
        {
            return Some(Goal::Present { slot, kind: 8 });
        }
        let bucks = ram::g_player_bucks(revision).at(player as i32).read(memory);
        let food_health = tje_mem::rom_data_units(revision)
            .iter()
            .find(|unit| unit.name == "PresentAuxiliarySignedLookup")
            .unwrap();
        let count = (0..16)
            .filter(|&slot| inventory.at(slot).read(memory) >= 0)
            .count();
        let gear = (0..16).find_map(|slot| {
            let kind = inventory.at(slot as i32).read(memory);
            matches!(kind, 0 | 2).then_some(Goal::Present { slot, kind })
        });
        let reserve_gear = (0..16).any(|slot| matches!(inventory.at(slot).read(memory), 0 | 8))
            || ram::g_player_powerups(revision)
                .at(player as i32)
                .read(memory)
                == 0;
        if self.objective == Objective::LevelZero && level > 0 {
            let active_gear = matches!(
                ram::g_player_powerups(revision)
                    .at(player as i32)
                    .read(memory),
                0 | 2
            );
            if level > 1 && (gear.is_some() || active_gear) {
                return Some(Goal::Fall);
            }
            if level == 1 {
                if active_gear {
                    return Some(Goal::Fall);
                }
                if let Some(gear) = gear {
                    return Some(gear);
                }
            }
            for bank in 0..2 {
                for index in 0..28 {
                    let item = ram::g_object_table(revision).at(bank).at(index);
                    if item.level().read(memory) == level
                        && matches!(item.kind().read(memory), 0 | 2)
                        && !self.blocked.contains(&(level, Goal::Item { bank, index }))
                    {
                        return Some(Goal::Item { bank, index });
                    }
                }
            }
        }
        let present = (0..16).find_map(|slot| {
            let kind = inventory.at(slot as i32).read(memory);
            let useful = (health < 12 && matches!(kind, 9 | 14 | 17))
                || (self.objective == Objective::Explore
                    && (0..28).contains(&kind)
                    && content
                        .presents_opened
                        .get(&(kind as u8))
                        .is_some_and(Option::is_none));
            let useful = useful
                || matches!(kind, 20 | 21)
                || (kind == 11 && ram::g_player_rank(revision).at(player as i32).read(memory) < 8)
                || (count >= 14
                    && matches!(kind, 1 | 3 | 4 | 6 | 7 | 9 | 10 | 14 | 17 | 19 | 25 | 27));
            useful.then_some(Goal::Present { slot, kind })
        });
        if present.is_some() {
            return present;
        }
        for bank in 0..2 {
            for index in 0..20 {
                let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                let goal = Goal::Ship { bank, index };
                if earthling.kind().read(memory) == 0 && earthling.level().read(memory) == level {
                    return Some(goal);
                }
            }
        }
        if self.objective == Objective::Explore || level == 0 || health < 8 {
            for bank in 0..2 {
                for index in 0..20 {
                    let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                    let kind = earthling.kind().read(memory);
                    let goal = Goal::Earthling { bank, index };
                    if kind >= 0
                        && (self.objective == Objective::Explore || level == 0 || kind == 10)
                        && match kind {
                            9 => count < 16 && bucks >= 3,
                            10 => bucks > 0,
                            11 => bucks >= 2,
                            17 => bucks >= 3,
                            _ => true,
                        }
                        && earthling.level().read(memory) == level
                        && !self.blocked.contains(&(level, goal))
                        && (content
                            .earthlings_encountered
                            .get(&(kind as u8))
                            .is_some_and(Option::is_none)
                            || (matches!(kind, 9..=11 | 17 | 23..=24)
                                && !friendly_done(content, kind)))
                    {
                        return Some(goal);
                    }
                }
            }
        }
        let view = ram::g_player_info(revision).at(player as i32);
        let position = (view.x().read(memory), view.y().read(memory));
        let mut best_item = None;
        for bank in 0..2 {
            for index in 0..28 {
                let item = ram::g_object_table(revision).at(bank).at(index);
                let kind = item.kind().read(memory);
                let goal = Goal::Item { bank, index };
                if item.level().read(memory) != level
                    || kind < 0
                    || self.blocked.contains(&(level, goal))
                {
                    continue;
                }
                let needed = (self.objective == Objective::Campaign
                    && !reserve_gear
                    && matches!(kind, 0 | 8))
                    || (self.objective == Objective::Explore
                        && kind < 28
                        && content
                            .presents_opened
                            .get(&(kind as u8))
                            .is_some_and(Option::is_none))
                    || (health < 14
                        && (64..80).contains(&kind)
                        && rom[food_health.address as usize + (kind - 64) as usize] as i8 > 0)
                    || (self.objective == Objective::Explore
                        && kind == 80
                        && ram::g_player_bucks(revision).at(player as i32).read(memory) < 5);
                if !needed {
                    continue;
                }
                let distance = (i32::from(position.0) - i32::from(item.x().read(memory))).abs()
                    + (i32::from(position.1) - i32::from(item.y().read(memory))).abs();
                if best_item.is_none_or(|(best, _)| distance < best) {
                    best_item = Some((distance, goal));
                }
            }
        }
        if let Some((_, goal)) = best_item {
            return Some(goal);
        }
        for bank in 0..2 {
            let elevator = ram::g_elevator_records(revision).at(bank).at(0);
            if elevator.level().read(memory) == level && elevator.state().read(memory) != 11 {
                return Some(Goal::Elevator { bank });
            }
        }
        None
    }

    fn inventory_input(
        &mut self,
        memory: &Ram<'_>,
        revision: Revision,
        player: usize,
        frame: u64,
        pad: &mut u16,
    ) {
        let Some(Goal::Present { slot, kind }) = self.goals[player] else {
            return;
        };
        if frame - self.last_inventory_action[player] < 32 {
            return;
        }
        let row = ram::g_player_inventory_scroll_row(revision)
            .at(player as i32)
            .read(memory)
            + ram::g_player_inventory_selection_row(revision)
                .at(player as i32)
                .read(memory);
        let column = ram::g_player_inventory_selection_column(revision)
            .at(player as i32)
            .read(memory);
        if frame % 32 >= 12 {
            return;
        }
        if row > (slot / 2) as i8 {
            *pad |= UP;
        } else if row < (slot / 2) as i8 {
            *pad |= DOWN;
        } else if column != (slot % 2) as i8 {
            *pad |= if column == 0 { RIGHT } else { LEFT };
        } else if ram::g_inventory_banner_mode(revision)
            .at(player as i32)
            .read(memory)
            != 0
        {
            *pad |= C;
            self.held_action[player] = C;
            self.held_until[player] = frame + 12;
        } else {
            if kind == 8 {
                self.doorway_used[player] = false;
            }
            *pad |= A;
            self.last_inventory_action[player] = frame;
            self.last_inventory_kind[player] = kind;
            self.held_action[player] = A;
            self.held_until[player] = frame + 12;
        }
    }
}

fn friendly_done(content: &Content, kind: i8) -> bool {
    let key = match kind {
        9 => "mail_order_purchase",
        10 => "wizard_paid",
        11 => "wiseman_identification",
        17 => "opera_paid",
        23 => "lemonade_reward",
        24 => "hottub_bath",
        _ => {
            return content
                .earthlings_encountered
                .get(&(kind as u8))
                .is_some_and(Option::is_some);
        }
    };
    content
        .friendly_interactions
        .get(key)
        .is_some_and(Option::is_some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tje_session::CONNECTED;

    #[test]
    fn inventory_action_survives_both_controller_poll_parities() {
        let revision = Revision::Rev02;
        let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
        ram::g_player_inventory_open(revision)
            .at(0)
            .write(&mut memory, 1);
        let mut planner = Adventure::new(Objective::Explore);
        planner.goals[0] = Some(Goal::Present { slot: 0, kind: 27 });
        let mut pressed = CONNECTED;
        planner.inventory_input(&Ram(memory.ram()), revision, 0, 64, &mut pressed);
        assert_eq!(pressed, CONNECTED | A);
        for frame in 65..76 {
            let mut pads = [CONNECTED, 0];
            planner.choose(
                &Ram(memory.ram()),
                &[],
                revision,
                &Content::new(),
                frame,
                &mut pads,
            );
            assert_eq!(pads[0], CONNECTED | A, "frame {frame}");
        }
        assert_eq!(planner.held_until[0], 76);
    }

    #[test]
    fn campaign_collects_mobility_without_chasing_unrelated_hostile_encounters() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 5;
            ram::g_player_info(revision)
                .at(0)
                .level()
                .write(&mut memory, 5);
            ram::g_player_powerups(revision)
                .at(0)
                .write(&mut memory, -1);
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                inventory.at(slot).write(&mut memory, -1);
            }
            let hostile = ram::g_earthling_banks(revision).at(0).at(0);
            hostile.kind().write(&mut memory, 1);
            hostile.level().write(&mut memory, 5);
            let wings = ram::g_object_table(revision).at(0).at(0);
            wings.kind().write(&mut memory, 0);
            wings.level().write(&mut memory, 5);
            let elevator = ram::g_elevator_records(revision).at(0).at(0);
            elevator.level().write(&mut memory, 5);
            assert!(matches!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 7),
                Some(Goal::Item { bank: 0, index: 0 })
            ));
            inventory.at(0).write(&mut memory, 0);
            assert!(matches!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 7),
                Some(Goal::Elevator { bank: 0 })
            ));
            planner.mobility_needed[0] = true;
            assert!(matches!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 7),
                Some(Goal::Present { slot: 0, kind: 0 })
            ));
        }
    }
}
