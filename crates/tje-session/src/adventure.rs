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
    Path { target: (i16, i16) },
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

#[derive(Clone, Copy)]
struct ShipContact {
    level: i8,
    goal: Goal,
    point: (i16, i16),
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
    scene_player: usize,
    last_inventory_action: [u64; 2],
    last_inventory_kind: [i8; 2],
    inventory_since: [u64; 2],
    inventory_kinds: [i8; 2],
    held_action: [u16; 2],
    held_until: [u64; 2],
    ship_contacts: [Option<ShipContact>; 2],
    mobility_needed: [bool; 2],
    doorway_used: [bool; 2],
    bath_since: [Option<u64>; 2],
    arrival_exit: [Option<i16>; 2],
    coop_phase: u8,
    coop_targets: Option<[(i16, i16); 2]>,
    coop_center: (i16, i16),
    final_entry: Option<u64>,
    final_ready: bool,
    maximum_hit_damage: Option<i8>,
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
            scene_player: 0,
            last_inventory_action: [0; 2],
            inventory_since: [0; 2],
            inventory_kinds: [0; 2],
            bath_since: [None; 2],
            arrival_exit: [None; 2],
            ship_contacts: [None; 2],
            held_action: [0; 2],
            held_until: [0; 2],
            mobility_needed: [false; 2],
            doorway_used: [false; 2],
            coop_phase: 0,
            coop_targets: None,
            coop_center: (0, 0),
            last_inventory_kind: [-1; 2],
            final_entry: None,
            final_ready: false,
            maximum_hit_damage: None,
            last_status: 0,
        }
    }
    fn reconsider_routes(&mut self, player: usize, level: i8) {
        self.blocked.retain(|(blocked_level, goal)| {
            *blocked_level != level || !matches!(goal, Goal::Item { .. } | Goal::Doorway)
        });
        self.ship_contacts[player] = None;
    }
    fn interaction_done(
        &self,
        memory: &Ram<'_>,
        revision: Revision,
        content: &Content,
        player: usize,
        kind: i8,
    ) -> bool {
        match kind {
            24 => self.bath_since[player].is_some(),
            23 => {
                ram::g_player_lemonade_reward_states(revision)
                    .at(player as i32)
                    .read(memory)
                    == 2
            }
            _ => friendly_done(content, kind),
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
                    self.scene_player = usize::from(event.stack_slots[0] as u16).min(1);
                }
                "PlayEndingSequence" => {
                    self.scene = Scene::Ending;
                    self.scene_since = event.frame;
                }
                "MapEnterFinalLevel" => {
                    self.scene = Scene::Game;
                    self.final_entry = Some(event.frame);
                    self.final_ready = false;
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
                        self.reconsider_routes(player, self.levels[player]);
                    }
                }
                "MenuWhoHandler" => {
                    self.scene = Scene::Game;
                    self.blocked.clear();
                    self.levels = [-1; 2];
                    self.arrival_exit = [None; 2];
                    self.bath_since = [None; 2];
                    self.final_entry = None;
                    self.final_ready = false;
                }
                "ProcessInput" if self.scene != Scene::Ending => {
                    self.scene = Scene::Game;
                    self.final_ready |= self.final_entry.is_some();
                }
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
                    pads[self.scene_player] |= C;
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
                self.mobility_needed[player] = false;
                self.doorway_used[player] = false;
                self.reconsider_routes(player, level);
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
                if view.sprite_state().read(memory) == 117 {
                    *pad |= [A, UP, RIGHT, DOWN, LEFT][((frame / 4) % 5) as usize];
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
                    && frame - self.last_inventory_action[player] < present_settle_frames(kind)
                {
                    continue;
                }
                self.goals[player] = None;
            }
            let health = ram::g_player_health(revision)
                .at(player as i32)
                .read(memory);
            let position = (view.x().read(memory), view.y().read(memory));
            let terrain = if player == 0 {
                ram::g_player1_terrain_underfoot(revision)
            } else {
                ram::g_player2_terrain_underfoot(revision)
            };
            if self.objective == Objective::Campaign
                && self.goals[player] != Some(Goal::Fall)
                && health > 0
                && view.height().read(memory) == 0
                && terrain.read(memory) & 0xF0 == 0x80
                && !matches!(
                    ram::g_player_powerups(revision)
                        .at(player as i32)
                        .read(memory),
                    0 | 2
                )
                && let Some(input) =
                    self.navigation[player].return_to_shore(memory, rom, revision, player, frame)
            {
                // An accidental splash is not a new swimming campaign route:
                // get back to dry footing before reconsidering the old goal.
                if self.goals[player].is_some() {
                    self.reconsider_routes(player, level);
                }
                self.goals[player] = None;
                self.goal_since[player] = frame;
                self.mobility_needed[player] = false;
                *pad |= input;
                continue;
            }
            if self.objective == Objective::Campaign
                && health > 0
                && self
                    .maximum_hit_damage
                    .is_none_or(|maximum| health <= maximum)
                && view.height().read(memory) <= 0
                && terrain.read(memory) & 0xF0 <= 0x50
                && frame - self.last_inventory_action[player]
                    > present_settle_frames(self.last_inventory_kind[player])
            {
                // Spend held food before another lethal hit; if none remains,
                // real wings can take us out of nearby ground-contact danger.
                let inventory = ram::g_player_inventory(revision).at(player as i32);
                let mut healing = None;
                let mut healing_priority = 0;
                let mut wings = None;
                for slot in 0..16 {
                    let kind = inventory.at(slot as i32).read(memory);
                    // Sundae and rootbeer heal reliably; ordinary food can
                    // instead be yucky, so keep it behind those real options.
                    let priority = match kind {
                        14 => 3,
                        10 => 2,
                        9 => 1,
                        _ => 0,
                    };
                    if priority > healing_priority {
                        healing = Some((slot, kind));
                        healing_priority = priority;
                        if kind == 14 {
                            break;
                        }
                    }
                    if kind == 0 && wings.is_none() {
                        wings = Some((slot, kind));
                    }
                }
                let present = healing.or_else(|| {
                    wings.filter(|_| {
                        level > 0
                            && ram::g_player_powerups(revision)
                                .at(player as i32)
                                .read(memory)
                                != 0
                            && Navigator::nearby_threat(memory, revision, level, position)
                    })
                });
                if let Some((slot, kind)) = present {
                    let maximum_damage = *self.maximum_hit_damage.get_or_insert_with(|| {
                        let definitions = tje_mem::rom::earthling_definitions(revision);
                        (0..25).fold(0, |maximum, kind| {
                            maximum.max(rom[definitions.at(kind).damage().address() as usize] as i8)
                        })
                    });
                    if health <= maximum_damage {
                        self.goals[player] = Some(Goal::Present { slot, kind });
                        self.goal_since[player] = frame;
                    }
                }
            }
            if self.goals[player].is_none() || frame - self.goal_since[player] > 7200 {
                if let Some(goal) = self.goals[player]
                    && !self.blocked.contains(&(level, goal))
                {
                    self.blocked.push((level, goal));
                }
                self.goals[player] =
                    self.select_goal(memory, rom, revision, content, player, health);
                self.goal_since[player] = frame;
            }
            if let Some(goal) = self.goals[player] {
                match goal {
                    Goal::Present { slot: _, kind } => {
                        if frame - self.last_inventory_action[player]
                            > present_settle_frames(self.last_inventory_kind[player])
                            && frame % 32 < 12
                        {
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
                        if matches!(goal, Goal::Earthling { .. })
                            && self.interaction_done(memory, revision, content, player, kind)
                        {
                            if !self.blocked.contains(&(level, goal)) {
                                self.blocked.push((level, goal));
                            }
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
                            let x = earthling.x().read(memory);
                            let y = earthling.y().read(memory);
                            target = (x - 48, y - 4);
                            // Leave the lemonade contact zone horizontally before
                            // walking up; otherwise Drink re-enters indefinitely.
                            if (position.0 - x).abs() < 160 && (position.1 - y).abs() < 80 {
                                *pad |= if position.1 > y + 12 && position.0 > x + 64 {
                                    LEFT
                                } else if (position.1 - (y - 4)).abs() > 3 {
                                    if position.1 < y - 4 { DOWN } else { UP }
                                } else if position.0 < x {
                                    RIGHT
                                } else {
                                    LEFT
                                };
                                continue;
                            }
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
                        let mut ship_range = None;
                        if matches!(goal, Goal::Ship { .. }) {
                            let center = (earthling.x().read(memory), earthling.y().read(memory));
                            let definition = tje_mem::rom::earthling_definitions(revision).at(0);
                            let limits = (
                                i16::from(rom[definition.contact_distance_x().address() as usize]),
                                i16::from(rom[definition.contact_distance_y().address() as usize]),
                            );
                            ship_range = Some((center, limits));
                            let flying = matches!(
                                ram::g_player_powerups(revision)
                                    .at(player as i32)
                                    .read(memory),
                                0 | 2
                            );
                            if flying {
                                target = center;
                            } else if let Some(contact) = self.ship_contacts[player]
                                .filter(|contact| contact.level == level && contact.goal == goal)
                            {
                                target = contact.point;
                            } else {
                                let _ = self.navigation[player]
                                    .steer(memory, rom, revision, player, position, frame);
                                if let Some(point) = self.navigation[player]
                                    .reachable_land_in_range(position, center, limits)
                                {
                                    self.ship_contacts[player] =
                                        Some(ShipContact { level, goal, point });
                                    self.mobility_needed[player] = false;
                                    target = point;
                                } else {
                                    let approach = self.navigation[player].marked_path_approach(
                                        memory, revision, player, position, center,
                                    );
                                    self.mobility_needed[player] = approach.is_none();
                                    self.goals[player] =
                                        approach.map(|target| Goal::Path { target });
                                    self.goal_since[player] = frame;
                                    continue;
                                }
                            }
                        }
                        let dx = (position.0 - target.0).abs();
                        let dy = (position.1 - target.1).abs();
                        let in_contact =
                            ship_range.map_or(dx < 12 && dy < 4, |(center, limits)| {
                                (position.0 - center.0).abs() < limits.0
                                    && (position.1 - center.1).abs() < limits.1
                            });
                        if in_contact {
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
                                if !self.blocked.contains(&(level, goal)) {
                                    self.blocked.push((level, goal));
                                }
                                self.goals[player] = None;
                            }
                        } else if let Some(input) = self.navigation[player]
                            .steer(memory, rom, revision, player, target, frame)
                        {
                            *pad |= input;
                            if matches!(goal, Goal::Ship { .. }) {
                                self.mobility_needed[player] = false;
                            }
                        } else {
                            self.mobility_needed[player] |= matches!(goal, Goal::Ship { .. });
                            if !self.blocked.contains(&(level, goal)) {
                                self.blocked.push((level, goal));
                            }
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
                            if !self.blocked.contains(&(level, goal)) {
                                self.blocked.push((level, goal));
                            }
                            self.goals[player] = None;
                        }
                    }
                    Goal::Elevator { bank } => {
                        let elevator = ram::g_elevator_records(revision).at(bank).at(0);
                        let doorway = (elevator.x().read(memory), elevator.y().read(memory));
                        let target = departure_approach(position, doorway);
                        let state = elevator.state().read(memory);
                        let dy = position.1 - doorway.1;
                        if target.0 == doorway.0
                            && (position.0 - doorway.0).abs() < 8
                            && (0..40).contains(&dy)
                        {
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
                            self.mobility_needed[player] = false;
                        } else {
                            let approach = self.navigation[player]
                                .marked_path_approach(memory, revision, player, position, target);
                            self.mobility_needed[player] = approach.is_none();
                            self.goals[player] = approach.map(|target| Goal::Path { target });
                            self.goal_since[player] = frame;
                        }
                    }
                    Goal::Path { target } => {
                        let map = if player == 1
                            && ram::g_player_lives(revision).at(0).read(memory) >= 0
                            && ram::g_split_screen_enabled(revision).read(memory) != 0
                        {
                            ram::g_player2_level_map(revision)
                        } else {
                            ram::g_player1_level_map(revision)
                        };
                        let cell = map
                            .at(i32::from((target.1 >> 6) - 1))
                            .at(i32::from((target.0 >> 7) - 1))
                            .read(memory);
                        if cell & 0x4000 == 0 {
                            self.goals[player] = None;
                            self.reconsider_routes(player, level);
                        } else if let Some(input) = self.navigation[player].toward(
                            memory,
                            rom,
                            revision,
                            player,
                            (target, Travel::Land),
                            frame,
                        ) {
                            *pad |= input;
                        } else {
                            // A prior path animation may have severed this route.
                            // Do not wait forever for an unreachable old trigger.
                            self.goals[player] = None;
                            self.mobility_needed[player] = true;
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
                            if !self.blocked.contains(&(level, goal)) {
                                self.blocked.push((level, goal));
                            }
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
                            let travel = if ram::g_player_powerups(revision)
                                .at(player as i32)
                                .read(memory)
                                == 0
                            {
                                Travel::Fly
                            } else {
                                Travel::Fall
                            };
                            if let Some(direction) = self.navigation[player].toward(
                                memory,
                                rom,
                                revision,
                                player,
                                (target, travel),
                                frame,
                            ) {
                                *pad |= direction;
                            } else {
                                self.goals[player] = None;
                                self.mobility_needed[player] = true;
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
            if self.objective == Objective::Campaign
                && ram::g_player_powerups(revision)
                    .at(player as i32)
                    .read(memory)
                    == 7
                && *pad & (UP | DOWN | LEFT | RIGHT) != 0
                && self.navigation[player].safe_to_run(position, *pad)
            {
                let target = match self.goals[player] {
                    Some(Goal::Elevator { bank }) => {
                        let elevator = ram::g_elevator_records(revision).at(bank).at(0);
                        Some((elevator.x().read(memory), elevator.y().read(memory)))
                    }
                    Some(Goal::Ship { bank, index } | Goal::Earthling { bank, index }) => {
                        let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                        Some((earthling.x().read(memory), earthling.y().read(memory)))
                    }
                    Some(Goal::Item { bank, index }) => {
                        let item = ram::g_object_table(revision).at(bank).at(index);
                        Some((item.x().read(memory), item.y().read(memory)))
                    }
                    Some(Goal::Path { target }) => Some(target),
                    _ => None,
                };
                if matches!(self.goals[player], Some(Goal::Path { .. }))
                    || target.is_some_and(|target| {
                        (i32::from(position.0) - i32::from(target.0)).abs()
                            + (i32::from(position.1) - i32::from(target.1)).abs()
                            > 64
                    })
                {
                    // A is the original hitops run modifier. Avoid edge hops
                    // around narrow corners and release it for precise contact.
                    *pad |= A;
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
            let inventories: [[i8; 16]; 2] = std::array::from_fn(|player| {
                let inventory = ram::g_player_inventory(revision).at(player as i32);
                std::array::from_fn(|slot| inventory.at(slot as i32).read(memory))
            });
            eprintln!(
                "goals frame={frame} player(level,x,y,hp,lives,elevator)={stats:?} inventory={inventories:?} targets={:?} blocked={}",
                self.goals,
                self.blocked.len()
            );
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
        self.final_ready
            && self.final_entry.is_some_and(|start| frame >= start + 300)
            && ram::g_player1_map_level(revision).read(memory) == 26
            && ram::g_player2_map_level(revision).read(memory) == 26
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
            && self.interaction_done(memory, revision, content, 0, 24)
            && self.interaction_done(memory, revision, content, 0, 23)
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
        let power = ram::g_player_powerups(revision)
            .at(player as i32)
            .read(memory);
        // Actual active wings satisfy the travel demand. Do not cancel them
        // with a spare tube/door/randomizer merely because an old flag remains.
        let mobility_needed = self.mobility_needed[player] && power != 0;
        if matches!(self.objective, Objective::Campaign | Objective::LevelZero)
            && level == 0
            && self.interaction_done(memory, revision, content, player, 23)
            && self.interaction_done(memory, revision, content, player, 24)
        {
            return Some(Goal::Fall);
        }
        if self.objective == Objective::GameOver {
            return Some(Goal::Drown);
        }
        let inventory = ram::g_player_inventory(revision).at(player as i32);
        let doorway = ram::g_player_doorway_entities(revision)
            .at(player as i32)
            .read(memory);
        if doorway > 0
            && power != 0
            && (mobility_needed || !self.doorway_used[player])
            && !self.blocked.contains(&(level, Goal::Doorway))
            && ram::g_player_info(revision)
                .at(i32::from(doorway))
                .level()
                .read(memory)
                == level
        {
            return Some(Goal::Doorway);
        }
        if mobility_needed
            && let Some(slot) = (0..16).find(|&slot| inventory.at(slot as i32).read(memory) == 0)
        {
            return Some(Goal::Present { slot, kind: 0 });
        }
        if mobility_needed
            && let Some(slot) = (0..16).find(|&slot| inventory.at(slot as i32).read(memory) == 8)
        {
            return Some(Goal::Present { slot, kind: 8 });
        }
        if mobility_needed
            && power != 2
            && let Some(slot) = (0..16).find(|&slot| inventory.at(slot as i32).read(memory) == 2)
        {
            return Some(Goal::Present { slot, kind: 2 });
        }
        if mobility_needed
            && let Some(slot) = (0..16).find(|&slot| inventory.at(slot as i32).read(memory) == 26)
        {
            return Some(Goal::Present { slot, kind: 26 });
        }
        let bucks = ram::g_player_bucks(revision).at(player as i32).read(memory);
        let food_health = tje_mem::rom_data_units(revision)
            .iter()
            .find(|unit| unit.name == "PresentAuxiliarySignedLookup")
            .unwrap();
        let mut inventory_counts = [0_u8; 28];
        let mut count = 0;
        for slot in 0..16 {
            let kind = inventory.at(slot).read(memory);
            if kind >= 0 {
                count += 1;
                if let Some(total) = inventory_counts.get_mut(kind as usize) {
                    *total += 1;
                }
            }
        }
        let lives = ram::g_player_lives(revision).at(player as i32).read(memory);
        let rank = ram::g_player_rank(revision).at(player as i32).read(memory);
        if self.objective == Objective::LevelZero && level > 0 {
            let gear = (0..16).find_map(|slot| {
                let kind = inventory.at(slot as i32).read(memory);
                matches!(kind, 0 | 2).then_some(Goal::Present { slot, kind })
            });
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
            let useful = (health < 12 && matches!(kind, 9 | 10 | 14 | 17))
                || (self.objective == Objective::Explore
                    && (0..28).contains(&kind)
                    && content
                        .presents_opened
                        .get(&(kind as u8))
                        .is_some_and(Option::is_none));
            let useful = useful
                || (self.objective == Objective::Campaign && kind == 17 && lives < 8)
                || (self.objective == Objective::Campaign
                    && kind == 18
                    && mobility_needed
                    && inventory_counts[0] + inventory_counts[8] == 0
                    && count >= 4)
                || (self.objective == Objective::Campaign
                    && level >= 5
                    && matches!(kind, 7 | 27)
                    && ram::g_player_powerups(revision)
                        .at(player as i32)
                        .read(memory)
                        < 0)
                || matches!(kind, 20 | 21)
                || (kind == 11 && ram::g_player_rank(revision).at(player as i32).read(memory) < 8)
                || (count >= 14
                    && matches!(kind, 1 | 3 | 4 | 6 | 7 | 9 | 10 | 14 | 17 | 19 | 25 | 27)
                    && (self.objective != Objective::Campaign || !matches!(kind, 9 | 10 | 14)));
            useful.then_some(Goal::Present { slot, kind })
        });
        if present.is_some() {
            return present;
        }
        let ship = (0..2).find_map(|bank| {
            (0..20).find_map(|index| {
                let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                (earthling.kind().read(memory) == 0 && earthling.level().read(memory) == level)
                    .then_some(Goal::Ship { bank, index })
            })
        });
        // An unreachable ship must not starve the real mobility pickup that
        // would make it reachable. Accessible ships still have first priority.
        if ship.is_some() && !mobility_needed {
            return ship;
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
                                && !self.interaction_done(memory, revision, content, player, kind)))
                    {
                        return Some(goal);
                    }
                }
            }
        }
        let view = ram::g_player_info(revision).at(player as i32);
        let position = (view.x().read(memory), view.y().read(memory));
        let departure = (0..2).find_map(|bank| {
            let elevator = ram::g_elevator_records(revision).at(bank).at(0);
            (elevator.level().read(memory) == level && elevator.state().read(memory) != 11)
                .then_some(Goal::Elevator { bank })
        });
        if self.objective == Objective::Campaign
            && level > 0
            && health >= 14
            && ship.is_none()
            && let Some(Goal::Elevator { bank }) = departure
        {
            let elevator = ram::g_elevator_records(revision).at(bank).at(0);
            let distance = (i32::from(position.0) - i32::from(elevator.x().read(memory))).abs()
                + (i32::from(position.1) - i32::from(elevator.y().read(memory))).abs();
            if distance <= 96 {
                // Already at a usable exit: do not abandon it for a distant
                // proactive reserve after this level's real piece is collected.
                return departure;
            }
        }
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
                let campaign_needed = match kind {
                    0 | 8 => inventory_counts[kind as usize] < 2,
                    2 => inventory_counts[2] == 0,
                    7 | 27 => inventory_counts[7] + inventory_counts[27] < 2,
                    9 | 10 | 14 => {
                        inventory_counts[9] + inventory_counts[10] + inventory_counts[14] < 2
                    }
                    11 => rank < 8 && inventory_counts[11] == 0,
                    17 => lives < 8 && inventory_counts[17] == 0,
                    20 | 21 => bucks < 10 && inventory_counts[kind as usize] == 0,
                    26 => {
                        inventory_counts[26] == 0 && inventory_counts[0] + inventory_counts[8] == 0
                    }
                    _ => false,
                };
                let needed =
                    (self.objective == Objective::Campaign && count < 16 && campaign_needed)
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
        if ship.is_some() {
            return ship;
        }
        departure
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
                let level = self.levels[player];
                self.blocked.retain(|(blocked_level, goal)| {
                    *blocked_level != level || *goal != Goal::Doorway
                });
            }
            *pad |= A;
            self.last_inventory_action[player] = frame;
            self.last_inventory_kind[player] = kind;
            self.held_action[player] = A;
            self.held_until[player] = frame + 12;
        }
    }
}

fn departure_approach(position: (i16, i16), elevator: (i16, i16)) -> (i16, i16) {
    let side = elevator.0 + if position.0 < elevator.0 { -32 } else { 32 };
    let dx = (position.0 - elevator.0).abs();
    if position.1 <= elevator.1 && dx < 24 {
        // Original ElevatorBlocksPlayerMovement rejects crossing the roof
        // line within20 pixels. Leave it sideways before approaching the door.
        (side, elevator.1 - 8)
    } else if position.1 < elevator.1 + 18 && dx >= 24 {
        // Stay on that flank until below the doorway. Turning inward as soon
        // as X clears the roof re-enters the sideways stage on the next step.
        (side, elevator.1 + 22)
    } else {
        (elevator.0, elevator.1 + 22)
    }
}

fn present_settle_frames(kind: i8) -> u64 {
    if matches!(kind, 0 | 9 | 10 | 14) {
        32
    } else {
        140
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
    fn sleeping_campaigns_change_real_inputs_in_each_poll_window() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let player = ram::g_player_info(revision).at(0);
            player.level().write(&mut memory, 6);
            player.sprite_state().write(&mut memory, 117);
            ram::g_player_lives(revision).at(0).write(&mut memory, 4);
            let mut planner = Adventure::new(Objective::Campaign);
            for (step, expected) in [A, UP, RIGHT, DOWN, LEFT].into_iter().enumerate() {
                let mut pads = [CONNECTED, 0];
                planner.choose(
                    &Ram(memory.ram()),
                    &[],
                    revision,
                    &Content::new(),
                    3200 + step as u64 * 4,
                    &mut pads,
                );
                assert_eq!(pads[0], CONNECTED | expected);
            }
            player.sprite_state().write(&mut memory, 116);
            let mut pads = [CONNECTED, 0];
            planner.choose(
                &Ram(memory.ram()),
                &[],
                revision,
                &Content::new(),
                3224,
                &mut pads,
            );
            assert_eq!(pads[0], CONNECTED);
        }
    }

    #[test]
    fn a_campaign_leaves_level_zero_after_its_real_services() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            ram::g_player_lemonade_reward_states(revision)
                .at(0)
                .write(&mut memory, 2);
            let mut content = Content::new();
            content
                .friendly_interactions
                .insert("lemonade_reward".into(), Some(1));
            content
                .friendly_interactions
                .insert("hottub_bath".into(), Some(2));
            for objective in [Objective::Campaign, Objective::LevelZero] {
                let mut planner = Adventure::new(objective);
                planner.levels[0] = 0;
                planner.bath_since[0] = Some(2);
                assert_eq!(
                    planner.select_goal(&Ram(memory.ram()), &[], revision, &content, 0, 27),
                    Some(Goal::Fall)
                );
            }
        }
    }

    #[test]
    fn current_island_witnesses_keep_lemonade_23_and_hot_tub_24_distinct() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let mut planner = Adventure::new(Objective::LevelZero);
            let content = Content::new();
            ram::g_player_lemonade_reward_states(revision)
                .at(0)
                .write(&mut memory, 2);
            assert!(planner.interaction_done(&Ram(memory.ram()), revision, &content, 0, 23));
            assert!(!planner.interaction_done(&Ram(memory.ram()), revision, &content, 0, 24));
            ram::g_player_lemonade_reward_states(revision)
                .at(0)
                .write(&mut memory, 0);
            planner.bath_since[0] = Some(1);
            assert!(!planner.interaction_done(&Ram(memory.ram()), revision, &content, 0, 23));
            assert!(planner.interaction_done(&Ram(memory.ram()), revision, &content, 0, 24));
        }
    }

    #[test]
    fn elevator_roof_approach_goes_around_the_original_blocking_line() {
        assert_eq!(departure_approach((302, 298), (300, 300)), (332, 292));
        assert_eq!(departure_approach((298, 290), (300, 300)), (268, 292));
        assert_eq!(departure_approach((350, 290), (300, 300)), (332, 322));
        assert_eq!(departure_approach((300, 312), (300, 300)), (300, 322));
        assert_eq!(departure_approach((330, 292), (300, 300)), (332, 322));
        assert_eq!(departure_approach((332, 314), (300, 300)), (332, 322));
        assert_eq!(departure_approach((332, 320), (300, 300)), (300, 322));
        assert_eq!(departure_approach((268, 292), (300, 300)), (268, 322));
        assert_eq!(departure_approach((268, 314), (300, 300)), (268, 322));
        assert_eq!(departure_approach((268, 320), (300, 300)), (300, 322));
    }

    #[test]
    fn elevator_flank_staging_is_not_mistaken_for_a_door_approach() {
        for revision in Revision::ALL {
            for x in [268, 332] {
                let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
                let player = ram::g_player_info(revision).at(0);
                player.level().write(&mut memory, 5);
                player.x().write(&mut memory, x);
                player.y().write(&mut memory, 312);
                ram::g_player_health(revision).at(0).write(&mut memory, 27);
                ram::g_player_lives(revision).at(0).write(&mut memory, 2);
                ram::g_player_powerups(revision)
                    .at(0)
                    .write(&mut memory, -1);
                ram::g_player1_terrain_underfoot(revision).write(&mut memory, 0x10);
                for slot in 0..16 {
                    ram::g_player_inventory(revision)
                        .at(0)
                        .at(slot)
                        .write(&mut memory, -1);
                }
                let elevator = ram::g_elevator_records(revision).at(0).at(0);
                elevator.level().write(&mut memory, 5);
                elevator.x().write(&mut memory, 300);
                elevator.y().write(&mut memory, 300);
                let mut rom = vec![0; revision.rom_size()];
                rom[tje_mem::rom::player_tile_property_lookup(revision).address() as usize] = 0x10;
                let mut planner = Adventure::new(Objective::Campaign);
                planner.levels[0] = 5;
                planner.goals[0] = Some(Goal::Elevator { bank: 0 });
                let mut pads = [CONNECTED, 0];
                planner.choose(
                    &Ram(memory.ram()),
                    &rom,
                    revision,
                    &Content::new(),
                    3200,
                    &mut pads,
                );
                assert_eq!(pads[0], CONNECTED | DOWN);
                assert_eq!(planner.goals[0], Some(Goal::Elevator { bank: 0 }));
            }
        }
    }

    #[test]
    fn an_explicit_campaign_fall_does_not_turn_back_toward_shore() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let player = ram::g_player_info(revision).at(0);
            player.level().write(&mut memory, 5);
            player.x().write(&mut memory, 300);
            player.y().write(&mut memory, 68);
            ram::g_player_health(revision).at(0).write(&mut memory, 27);
            ram::g_player_lives(revision).at(0).write(&mut memory, 2);
            ram::g_player_powerups(revision)
                .at(0)
                .write(&mut memory, -1);
            ram::g_player1_terrain_underfoot(revision).write(&mut memory, 0x80);
            let mut rom = vec![0; revision.rom_size()];
            rom[tje_mem::rom::player_tile_property_lookup(revision).address() as usize] = 0x10;
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 5;
            planner.goals[0] = Some(Goal::Fall);
            let mut pads = [CONNECTED, 0];
            planner.choose(
                &Ram(memory.ram()),
                &rom,
                revision,
                &Content::new(),
                3200,
                &mut pads,
            );
            assert_eq!(pads[0], CONNECTED | UP);
            assert_eq!(planner.goals[0], Some(Goal::Fall));
        }
    }

    #[test]
    fn old_island_coverage_cannot_complete_a_new_level_zero_excursion() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            ram::g_player_info(revision)
                .at(0)
                .level()
                .write(&mut memory, 12);
            let mut content = Content::new();
            content
                .friendly_interactions
                .insert("lemonade_reward".into(), Some(1));
            content
                .friendly_interactions
                .insert("hottub_bath".into(), Some(2));
            let mut planner = Adventure::new(Objective::LevelZero);
            assert!(!planner.hidden_level_complete(&Ram(memory.ram()), revision, &content, 1000));
            planner.bath_since[0] = Some(500);
            assert!(!planner.hidden_level_complete(&Ram(memory.ram()), revision, &content, 1000));
            ram::g_player_lemonade_reward_states(revision)
                .at(0)
                .write(&mut memory, 2);
            assert!(planner.hidden_level_complete(&Ram(memory.ram()), revision, &content, 1000));
        }
    }

    #[test]
    fn revealed_roads_reconsider_items_and_invalidate_the_old_ship_contact() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            ram::g_player_info(revision)
                .at(0)
                .level()
                .write(&mut memory, 5);
            ram::g_player_health(revision).at(0).write(&mut memory, 27);
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 5;
            planner.goals[0] = Some(Goal::Path { target: (164, 84) });
            planner.ship_contacts[0] = Some(ShipContact {
                level: 5,
                goal: Goal::Ship { bank: 0, index: 1 },
                point: (100, 100),
            });
            planner.blocked = vec![
                (5, Goal::Item { bank: 0, index: 0 }),
                (5, Goal::Earthling { bank: 0, index: 2 }),
                (6, Goal::Item { bank: 0, index: 0 }),
            ];
            let mut pads = [CONNECTED, 0];
            planner.choose(
                &Ram(memory.ram()),
                &[],
                revision,
                &Content::new(),
                1000,
                &mut pads,
            );
            assert!(planner.goals[0].is_none());
            assert!(planner.ship_contacts[0].is_none());
            assert_eq!(planner.blocked.len(), 2);
            assert!(
                planner
                    .blocked
                    .contains(&(5, Goal::Earthling { bank: 0, index: 2 }))
            );
            assert!(
                planner
                    .blocked
                    .contains(&(6, Goal::Item { bank: 0, index: 0 }))
            );
        }
    }

    #[test]
    fn an_accidentally_swimming_campaign_returns_to_shore_before_its_old_goal() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let player = ram::g_player_info(revision).at(0);
            player.level().write(&mut memory, 5);
            player.x().write(&mut memory, 300);
            player.y().write(&mut memory, 300);
            ram::g_player_health(revision).at(0).write(&mut memory, 20);
            ram::g_player_lives(revision).at(0).write(&mut memory, 2);
            ram::g_player_powerups(revision).at(0).write(&mut memory, 7);
            ram::g_player1_terrain_underfoot(revision).write(&mut memory, 0x80);
            for slot in 0..16 {
                ram::g_player_inventory(revision)
                    .at(0)
                    .at(slot)
                    .write(&mut memory, -1);
            }
            ram::g_player_inventory(revision)
                .at(0)
                .at(0)
                .write(&mut memory, 14);
            let mut rom = vec![0; revision.rom_size()];
            rom[tje_mem::rom::player_tile_property_lookup(revision).address() as usize] = 0x10;
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 5;
            planner.goals[0] = Some(Goal::Elevator { bank: 0 });
            planner.blocked.push((5, Goal::Item { bank: 0, index: 0 }));
            planner.mobility_needed[0] = true;
            let mut pads = [CONNECTED, 0];
            planner.choose(
                &Ram(memory.ram()),
                &rom,
                revision,
                &Content::new(),
                3200,
                &mut pads,
            );
            assert_ne!(pads[0] & (UP | DOWN | LEFT | RIGHT), 0);
            assert_eq!(pads[0] & (A | B), 0);
            assert_eq!(planner.goals[0], None);
            assert!(!planner.mobility_needed[0]);
            assert!(
                !planner
                    .blocked
                    .contains(&(5, Goal::Item { bank: 0, index: 0 }))
            );
            assert_eq!(
                ram::g_player_inventory(revision).at(0).at(0).read(&memory),
                14
            );
        }
    }

    #[test]
    fn a_held_healing_present_preempts_a_long_route_at_critical_health() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let player = ram::g_player_info(revision).at(0);
            player.level().write(&mut memory, 5);
            ram::g_player_health(revision).at(0).write(&mut memory, 15);
            ram::g_player_lives(revision).at(0).write(&mut memory, 2);
            ram::g_player1_terrain_underfoot(revision).write(&mut memory, 0x10);
            for slot in 0..16 {
                ram::g_player_inventory(revision)
                    .at(0)
                    .at(slot)
                    .write(&mut memory, -1);
            }
            ram::g_player_inventory(revision)
                .at(0)
                .at(0)
                .write(&mut memory, 14);
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 5;
            planner.goals[0] = Some(Goal::Elevator { bank: 0 });
            let mut pads = [CONNECTED, 0];
            let mut rom = vec![0; revision.rom_size()];
            let damage = tje_mem::rom::earthling_definitions(revision).at(1).damage();
            rom[damage.address() as usize] = 24;
            planner.choose(
                &Ram(memory.ram()),
                &rom,
                revision,
                &Content::new(),
                3200,
                &mut pads,
            );
            assert_eq!(planner.goals[0], Some(Goal::Present { slot: 0, kind: 14 }));
            assert_eq!(pads[0], CONNECTED | B);
            assert_eq!(planner.maximum_hit_damage, Some(24));
            assert_eq!(
                ram::g_player_inventory(revision).at(0).at(0).read(&memory),
                14
            );
            planner.goals[0] = Some(Goal::Present { slot: 0, kind: 14 });
            planner.last_inventory_kind[0] = 14;
            planner.last_inventory_action[0] = 3200;
            ram::g_player_inventory(revision)
                .at(0)
                .at(0)
                .write(&mut memory, 9);
            player.height().write(&mut memory, -6);
            ram::g_player1_terrain_underfoot(revision).write(&mut memory, 0x20);
            let mut pads = [CONNECTED, 0];
            planner.choose(
                &Ram(memory.ram()),
                &rom,
                revision,
                &Content::new(),
                3233,
                &mut pads,
            );
            assert_eq!(planner.goals[0], Some(Goal::Present { slot: 0, kind: 9 }));
            assert_eq!(pads[0], CONNECTED | B);
        }
    }

    #[test]
    fn guaranteed_heals_precede_risky_food_regardless_of_inventory_order() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            ram::g_player_info(revision)
                .at(0)
                .level()
                .write(&mut memory, 5);
            ram::g_player_health(revision).at(0).write(&mut memory, 15);
            ram::g_player_lives(revision).at(0).write(&mut memory, 2);
            ram::g_player_powerups(revision)
                .at(0)
                .write(&mut memory, -1);
            ram::g_player1_terrain_underfoot(revision).write(&mut memory, 0x10);
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                inventory.at(slot).write(&mut memory, -1);
            }
            inventory.at(0).write(&mut memory, 9);
            inventory.at(1).write(&mut memory, 10);
            inventory.at(2).write(&mut memory, 14);
            let mut rom = vec![0; revision.rom_size()];
            let damage = tje_mem::rom::earthling_definitions(revision).at(1).damage();
            rom[damage.address() as usize] = 24;
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 5;
            for (step, (slot, kind)) in [(2, 14), (1, 10), (0, 9)].into_iter().enumerate() {
                planner.goals[0] = Some(Goal::Elevator { bank: 0 });
                let mut pads = [CONNECTED, 0];
                planner.choose(
                    &Ram(memory.ram()),
                    &rom,
                    revision,
                    &Content::new(),
                    3200 + step as u64 * 256,
                    &mut pads,
                );
                assert_eq!(planner.goals[0], Some(Goal::Present { slot, kind }));
                assert_eq!(pads[0], CONNECTED | B);
                assert_eq!(inventory.at(slot as i32).read(&memory), kind);
                inventory.at(slot as i32).write(&mut memory, -1);
            }
            assert_eq!(present_settle_frames(10), 32);
        }
    }

    #[test]
    fn critical_ground_danger_spends_real_wings_when_no_food_remains() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let player = ram::g_player_info(revision).at(0);
            player.level().write(&mut memory, 5);
            player.x().write(&mut memory, 164);
            player.y().write(&mut memory, 84);
            ram::g_player_health(revision).at(0).write(&mut memory, 15);
            ram::g_player_lives(revision).at(0).write(&mut memory, 2);
            ram::g_player_powerups(revision)
                .at(0)
                .write(&mut memory, -1);
            ram::g_player1_terrain_underfoot(revision).write(&mut memory, 0x10);
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                inventory.at(slot).write(&mut memory, -1);
            }
            inventory.at(0).write(&mut memory, 0);
            let enemy = ram::g_earthling_banks(revision).at(0).at(0);
            enemy.kind().write(&mut memory, 19);
            enemy.level().write(&mut memory, 5);
            enemy.object().write(&mut memory, 6);
            let object = ram::g_player_info(revision).at(6);
            object.level().write(&mut memory, 5);
            object.x().write(&mut memory, 200);
            object.y().write(&mut memory, 84);
            let mut rom = vec![0; revision.rom_size()];
            let damage = tje_mem::rom::earthling_definitions(revision).at(1).damage();
            rom[damage.address() as usize] = 24;
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 5;
            planner.goals[0] = Some(Goal::Elevator { bank: 0 });
            let mut pads = [CONNECTED, 0];
            planner.choose(
                &Ram(memory.ram()),
                &rom,
                revision,
                &Content::new(),
                3200,
                &mut pads,
            );
            assert_eq!(planner.goals[0], Some(Goal::Present { slot: 0, kind: 0 }));
            assert_eq!(pads[0], CONNECTED | B);
            assert_eq!(inventory.at(0).read(&memory), 0);
            inventory.at(1).write(&mut memory, 14);
            planner.goals[0] = Some(Goal::Elevator { bank: 0 });
            let mut pads = [CONNECTED, 0];
            planner.choose(
                &Ram(memory.ram()),
                &rom,
                revision,
                &Content::new(),
                3232,
                &mut pads,
            );
            assert_eq!(planner.goals[0], Some(Goal::Present { slot: 1, kind: 14 }));
            assert_eq!(pads[0], CONNECTED | B);
            assert_eq!(inventory.at(0).read(&memory), 0);
        }
    }

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
    fn a_nearby_departure_precedes_a_healthy_campaigns_reserve_detour() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let player = ram::g_player_info(revision).at(0);
            player.level().write(&mut memory, 5);
            player.x().write(&mut memory, 300);
            player.y().write(&mut memory, 300);
            ram::g_player_powerups(revision)
                .at(0)
                .write(&mut memory, -1);
            ram::g_player_rank(revision).at(0).write(&mut memory, 8);
            for slot in 0..16 {
                ram::g_player_inventory(revision)
                    .at(0)
                    .at(slot)
                    .write(&mut memory, -1);
            }
            let wings = ram::g_object_table(revision).at(0).at(0);
            wings.kind().write(&mut memory, 0);
            wings.level().write(&mut memory, 5);
            wings.x().write(&mut memory, 1200);
            wings.y().write(&mut memory, 800);
            let elevator = ram::g_elevator_records(revision).at(0).at(0);
            elevator.level().write(&mut memory, 5);
            elevator.x().write(&mut memory, 332);
            elevator.y().write(&mut memory, 312);
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 5;
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 55),
                Some(Goal::Elevator { bank: 0 })
            );
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 13),
                Some(Goal::Item { bank: 0, index: 0 })
            );
            let ship = ram::g_earthling_banks(revision).at(0).at(0);
            ship.kind().write(&mut memory, 0);
            ship.level().write(&mut memory, 5);
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 55),
                Some(Goal::Ship { bank: 0, index: 0 })
            );
        }
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
                Some(Goal::Item { bank: 0, index: 0 })
            ));
            inventory.at(1).write(&mut memory, 0);
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

    #[test]
    fn a_campaign_keeps_a_bounded_food_reserve_before_health_becomes_critical() {
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
            let item = ram::g_object_table(revision).at(0).at(0);
            item.kind().write(&mut memory, 10);
            item.level().write(&mut memory, 5);
            let elevator = ram::g_elevator_records(revision).at(0).at(0);
            elevator.level().write(&mut memory, 5);
            elevator.x().write(&mut memory, 1200);
            elevator.y().write(&mut memory, 800);
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 27),
                Some(Goal::Item { bank: 0, index: 0 })
            );
            inventory.at(0).write(&mut memory, 10);
            inventory.at(1).write(&mut memory, 14);
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 27),
                Some(Goal::Elevator { bank: 0 })
            );
            for slot in 0..14 {
                inventory.at(slot).write(&mut memory, 10);
            }
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 27),
                Some(Goal::Elevator { bank: 0 })
            );
        }
    }

    #[test]
    fn tub_approach_leaves_the_drink_contact_zone_before_walking_up() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let player = ram::g_player_info(revision).at(0);
            player.level().write(&mut memory, 0);
            player.x().write(&mut memory, 1534);
            player.y().write(&mut memory, 1034);
            let tub = ram::g_earthling_banks(revision).at(0).at(0);
            tub.kind().write(&mut memory, 24);
            tub.level().write(&mut memory, 0);
            tub.x().write(&mut memory, 1398);
            tub.y().write(&mut memory, 980);
            let mut planner = Adventure::new(Objective::LevelZero);
            planner.levels[0] = 0;
            planner.goals[0] = Some(Goal::Earthling { bank: 0, index: 0 });
            let mut pads = [CONNECTED, 0];
            planner.choose(
                &Ram(memory.ram()),
                &[],
                revision,
                &Content::new(),
                100,
                &mut pads,
            );
            assert_eq!(pads[0], CONNECTED | LEFT);
        }
    }

    #[test]
    fn map_exit_press_belongs_to_the_requesting_controller() {
        let revision = Revision::Rev02;
        let memory = tje_mem::AddressSpace::new(Vec::new(), revision);
        let mut planner = Adventure::new(Objective::Coop);
        planner.scene = Scene::Map;
        planner.scene_player = 1;
        let mut pads = [CONNECTED, CONNECTED];
        planner.choose(
            &Ram(memory.ram()),
            &[],
            revision,
            &Content::new(),
            144,
            &mut pads,
        );
        assert_eq!(pads, [CONNECTED, CONNECTED | C]);
    }

    #[test]
    fn campaign_uses_existing_hitops_once_without_fabricating_resources() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 5;
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                inventory.at(slot).write(&mut memory, -1);
            }
            inventory.at(0).write(&mut memory, 27);
            ram::g_player_powerups(revision)
                .at(0)
                .write(&mut memory, -1);
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 23),
                Some(Goal::Present { slot: 0, kind: 27 })
            );
            ram::g_player_powerups(revision).at(0).write(&mut memory, 7);
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 23),
                None
            );
            assert_eq!(inventory.at(0).read(&memory), 27);
        }
    }

    #[test]
    fn severed_hidden_road_routes_do_not_wait_forever() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let player = ram::g_player_info(revision).at(0);
            player.level().write(&mut memory, 5);
            player.x().write(&mut memory, 164);
            player.y().write(&mut memory, 84);
            ram::g_player_powerups(revision)
                .at(0)
                .write(&mut memory, -1);
            ram::g_player1_level_map(revision)
                .at(0)
                .at(0)
                .write(&mut memory, 0x4000);
            let mut rom = vec![0; revision.rom_size()];
            rom[tje_mem::rom::player_tile_property_lookup(revision).address() as usize] = 0x90;
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 5;
            planner.goals[0] = Some(Goal::Path { target: (188, 84) });
            let mut pads = [CONNECTED, 0];
            planner.choose(
                &Ram(memory.ram()),
                &rom,
                revision,
                &Content::new(),
                100,
                &mut pads,
            );
            assert_eq!(planner.goals[0], None);
            assert!(planner.mobility_needed[0]);
            assert_eq!(pads[0], CONNECTED);
        }
    }

    #[test]
    fn unreachable_ship_does_not_starve_wings_while_an_innertube_is_active() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 7;
            planner.mobility_needed[0] = true;
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                inventory.at(slot).write(&mut memory, -1);
            }
            inventory.at(0).write(&mut memory, 2);
            ram::g_player_powerups(revision).at(0).write(&mut memory, 2);
            let ship = ram::g_earthling_banks(revision).at(0).at(0);
            ship.kind().write(&mut memory, 0);
            ship.level().write(&mut memory, 7);
            let wings = ram::g_object_table(revision).at(0).at(0);
            wings.kind().write(&mut memory, 0);
            wings.level().write(&mut memory, 7);
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 23),
                Some(Goal::Item { bank: 0, index: 0 })
            );
            planner.mobility_needed[0] = false;
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 23),
                Some(Goal::Ship { bank: 0, index: 0 })
            );
        }
    }

    #[test]
    fn campaign_uses_real_extra_lives_even_when_health_is_full() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 7;
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                inventory.at(slot).write(&mut memory, -1);
            }
            inventory.at(0).write(&mut memory, 17);
            for lives in [1, 5, 7] {
                ram::g_player_lives(revision)
                    .at(0)
                    .write(&mut memory, lives);
                assert_eq!(
                    planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 23),
                    Some(Goal::Present { slot: 0, kind: 17 })
                );
            }
            ram::g_player_lives(revision).at(0).write(&mut memory, 8);
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 23),
                None
            );
            assert_eq!(inventory.at(0).read(&memory), 17);
        }
    }

    #[test]
    fn a_still_live_doorway_can_be_reused_only_when_mobility_is_needed_again() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 7;
            planner.doorway_used[0] = true;
            planner.mobility_needed[0] = true;
            ram::g_player_powerups(revision)
                .at(0)
                .write(&mut memory, -1);
            ram::g_player_doorway_entities(revision)
                .at(0)
                .write(&mut memory, 6);
            ram::g_player_info(revision)
                .at(6)
                .level()
                .write(&mut memory, 7);
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                inventory.at(slot).write(&mut memory, -1);
            }
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 23),
                Some(Goal::Doorway)
            );
            planner.blocked.push((7, Goal::Doorway));
            inventory.at(0).write(&mut memory, 8);
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 23),
                Some(Goal::Present { slot: 0, kind: 8 })
            );
            planner.goals[0] = Some(Goal::Present { slot: 0, kind: 8 });
            let mut pad = CONNECTED;
            planner.inventory_input(&Ram(memory.ram()), revision, 0, 64, &mut pad);
            assert_eq!(pad, CONNECTED | A);
            assert!(!planner.blocked.contains(&(7, Goal::Doorway)));
            inventory.at(0).write(&mut memory, -1);
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 23),
                Some(Goal::Doorway)
            );
            planner.doorway_used[0] = true;
            planner.mobility_needed[0] = false;
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 23),
                None
            );
        }
    }

    #[test]
    fn active_wings_are_not_replaced_by_spare_mobility_or_a_stale_need_flag() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let mut planner = Adventure::new(Objective::Campaign);
            planner.levels[0] = 7;
            planner.mobility_needed[0] = true;
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                inventory.at(slot).write(&mut memory, -1);
            }
            inventory.at(0).write(&mut memory, 2);
            inventory.at(1).write(&mut memory, 26);
            ram::g_player_powerups(revision).at(0).write(&mut memory, 0);
            ram::g_player_doorway_entities(revision)
                .at(0)
                .write(&mut memory, 6);
            ram::g_player_info(revision)
                .at(6)
                .level()
                .write(&mut memory, 7);
            let ship = ram::g_earthling_banks(revision).at(0).at(0);
            ship.kind().write(&mut memory, 0);
            ship.level().write(&mut memory, 7);
            assert_eq!(
                planner.select_goal(&Ram(memory.ram()), &[], revision, &Content::new(), 0, 23),
                Some(Goal::Ship { bank: 0, index: 0 })
            );
            assert_eq!(ram::g_player_powerups(revision).at(0).read(&memory), 0);
            assert_eq!(inventory.at(0).read(&memory), 2);
        }
    }

    #[test]
    fn final_completion_waits_for_main_loop_return_and_both_loaded_maps() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let mut planner = Adventure::new(Objective::Campaign);
            planner.final_entry = Some(1);
            ram::g_v_blank_graphics_enabled(revision).write(&mut memory, 1);
            ram::g_player_info(revision)
                .at(0)
                .level()
                .write(&mut memory, 26);
            ram::g_player1_map_level(revision).write(&mut memory, 26);
            ram::g_player_info(revision)
                .at(1)
                .level()
                .write(&mut memory, 2);
            assert!(!planner.campaign_complete(&Ram(memory.ram()), revision, 400));
            planner.final_ready = true;
            assert!(!planner.campaign_complete(&Ram(memory.ram()), revision, 400));
            ram::g_player2_map_level(revision).write(&mut memory, 26);
            assert!(planner.campaign_complete(&Ram(memory.ram()), revision, 400));
            planner.final_ready = false;
            assert!(!planner.campaign_complete(&Ram(memory.ram()), revision, 400));
        }
    }

    #[test]
    fn final_readiness_requires_a_later_normal_input_entry_and_resets_at_menus() {
        for revision in Revision::ALL {
            let mut planner = Adventure::new(Objective::Campaign);
            let observe = |planner: &mut Adventure, id: tje_game::FunctionId, frame| {
                planner.observe(
                    &FrameObservations {
                        ticks: Vec::new(),
                        entries: vec![tje_host::session_trace::EntryObservation {
                            address: id.metadata().address(revision),
                            frame,
                            stack_slots: [0; 4],
                            ram_samples: Vec::new(),
                        }],
                    },
                    revision,
                );
            };
            observe(&mut planner, tje_game::ids::PROCESS_INPUT, 1);
            assert!(!planner.final_ready);
            observe(&mut planner, tje_game::ids::MAP_ENTER_FINAL_LEVEL, 2);
            assert!(!planner.final_ready);
            observe(&mut planner, tje_game::ids::PROCESS_INPUT, 3);
            assert!(planner.final_ready);
            observe(&mut planner, tje_game::ids::MENU_WHO_HANDLER, 4);
            assert!(!planner.final_ready);
            assert_eq!(planner.final_entry, None);
        }
    }
}
