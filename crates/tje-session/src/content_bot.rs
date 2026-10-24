//! Content-directed controller inputs over live game state, never RAM recipes.
use crate::{
    navigation::{Navigator, Travel},
    runner::{Content, Ram, entry_name},
};
use tje_host::session_trace::FrameObservations;
use tje_mem::{Revision, ram};
use tje_session::{A, B, C, DOWN, LEFT, RIGHT, UP};

const EARTHLING_CUPID: i8 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Goal {
    Item {
        bank: i32,
        index: i32,
        dropped: bool,
    },
    Earthling {
        bank: i32,
        index: i32,
    },
    Elevator {
        bank: i32,
    },
    Telephone {
        index: i32,
    },
    Present {
        kind: i8,
        drop: bool,
    },
    Survey {
        column: i16,
        row: i16,
        point: (i16, i16),
    },
    Path {
        point: (i16, i16),
    },
    FinalContact {
        point: (i16, i16),
    },
    Doorway,
}

pub struct ContentBot {
    players: usize,
    navigation: Vec<Navigator>,
    goals: [Option<Goal>; 2],
    goal_since: [u64; 2],
    levels: [i8; 2],
    blocked: Vec<(i8, Goal, u64)>,
    next_action: [u64; 2],
    held: [u16; 2],
    held_until: [u64; 2],
    inventory_modes: [i8; 2],
    inventory_actions: [u8; 2],
    close_inventory: [bool; 2],
    arrival_exit: [Option<i16>; 2],
    bath_since: [Option<u64>; 2],
    mobility_needed: [bool; 2],
    doorway_used: [bool; 2],
    service_contacts: [Option<(Goal, (i16, i16))>; 2],
    previous_health: [i8; 2],
    whirlwind_contact: bool,
    cupid_bounced: bool,
    full_inventory_reaction: bool,
    nice_earthling_checked: bool,
    map_restored: bool,
    rose_abilities: u8,
    initial_life_setup: Option<bool>,
    stockpiling: bool,
    scene: &'static str,
    scene_player: usize,
    scene_since: u64,
    last_status: u64,
}

impl ContentBot {
    pub fn new(players: usize) -> Self {
        Self {
            players,
            navigation: (0..players).map(|_| Navigator::new()).collect(),
            goals: [None; 2],
            goal_since: [0; 2],
            levels: [-1; 2],
            blocked: Vec::new(),
            next_action: [0; 2],
            held: [0; 2],
            held_until: [0; 2],
            inventory_modes: [0; 2],
            inventory_actions: [0; 2],
            close_inventory: [false; 2],
            arrival_exit: [None; 2],
            bath_since: [None; 2],
            mobility_needed: [false; 2],
            doorway_used: [false; 2],
            service_contacts: [None; 2],
            previous_health: [i8::MAX; 2],
            whirlwind_contact: false,
            cupid_bounced: false,
            full_inventory_reaction: false,
            nice_earthling_checked: false,
            map_restored: false,
            rose_abilities: 0,
            initial_life_setup: None,
            stockpiling: false,
            scene: "game",
            scene_player: 0,
            scene_since: 0,
            last_status: 0,
        }
    }

    pub fn stockpile(players: usize) -> Self {
        let mut bot = Self::new(players);
        bot.stockpiling = true;
        bot
    }

    pub fn observe(&mut self, observations: &FrameObservations, revision: Revision) {
        for event in &observations.entries {
            match entry_name(revision, event.address) {
                "WaitForShipScreenAcknowledgement" => {
                    self.scene = "ship";
                    self.scene_since = event.frame;
                    self.scene_player = (event.stack_slots[0] as u16 as usize).min(1);
                }
                "DrawGameMap" => {
                    self.scene = "map";
                    self.scene_since = event.frame;
                    self.scene_player = (event.stack_slots[0] as u16 as usize).min(1);
                }
                "PlayEndingSequence" => self.scene = "ending",
                "EntityUpdateTerrainDrawFlag" => self.whirlwind_contact = true,
                "EarthlingUpdateBouncingDamageState" => self.cupid_bounced = true,
                "NiceEarthlingCheck" => self.nice_earthling_checked = true,
                "InventoryRedrawSynchronously" => self.map_restored = true,
                "PresentCreateActiveTerrainEffect" => self.rose_abilities |= 1,
                "PresentRetireTerrainEffect" => self.rose_abilities |= 2,
                "PresentCreateTemporaryTerrainEffect" => self.rose_abilities |= 4,
                "HasSpeakerSpeech" => {
                    self.full_inventory_reaction = true;
                    self.goals = [None; 2];
                }
                "ProcessInput" | "MapEnterFinalLevel" => self.scene = "game",
                "PlayerTeleportThroughDoorway" => {
                    let player = event.stack_slots[0] as u16 as usize;
                    if player < 2 {
                        self.doorway_used[player] = true;
                        self.mobility_needed[player] = false;
                        self.goals[player] = None;
                    }
                }
                "MenuWhoHandler" => {
                    self.scene = "game";
                    self.levels = [-1; 2];
                    self.goals = [None; 2];
                    self.blocked.clear();
                    self.arrival_exit = [None; 2];
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
        if self.scene != "game" {
            if frame > self.scene_since + 120 && frame % 48 < 12 {
                pads[self.scene_player] |=
                    if self.scene == "map" && (self.players != 2 || self.map_restored) {
                        C
                    } else {
                        A
                    };
            }
            return;
        }
        if self.prepare_life_transfer(memory, rom, revision, content, frame, pads) {
            return;
        }
        for (player, pad) in pads.iter_mut().enumerate().take(self.players) {
            if *pad == 0 {
                continue;
            }
            if ram::g_player_lives(revision).at(player as i32).read(memory) < 0 {
                if self.players == 2 && frame % 64 < 12 {
                    *pad |= A;
                }
                continue;
            }
            if frame < self.held_until[player] {
                *pad |= self.held[player];
                continue;
            }
            let view = ram::g_player_info(revision).at(player as i32);
            let level = view.level().read(memory);
            let position = (view.x().read(memory), view.y().read(memory));
            if self.levels[player] != level {
                self.levels[player] = level;
                self.goals[player] = None;
                self.bath_since[player] = None;
                self.service_contacts[player] = None;
            }
            let mode = ram::g_player_inventory_open(revision)
                .at(player as i32)
                .read(memory);
            if self.inventory_modes[player] != mode {
                self.inventory_modes[player] = mode;
                self.inventory_actions[player] = 0;
                self.next_action[player] = frame + 24;
                if mode == 0 {
                    self.close_inventory[player] = false;
                    if ram::g_player_powerups(revision)
                        .at(player as i32)
                        .read(memory)
                        == 0
                    {
                        self.mobility_needed[player] = false;
                        self.blocked.retain(|&(old_level, _, _)| old_level != level);
                    }
                    if matches!(self.goals[player], Some(Goal::Present { .. })) {
                        self.goals[player] = None;
                    }
                }
            }
            if mode != 0 {
                self.inventory_input(memory, rom, revision, content, player, frame, pad);
                continue;
            }
            if matches!(view.sprite_state().read(memory), 5 | 6) {
                continue;
            }
            if view.sprite_state().read(memory) == 119 {
                let since = *self.bath_since[player].get_or_insert(frame);
                if frame >= since + 300 {
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
                } else if self
                    .present_goal(memory, revision, content, player)
                    .is_some()
                {
                    let departure = ram::g_elevator_records(revision)
                        .at((i32::from(elevator) - 1) / 2)
                        .at(0);
                    self.arrival_exit[player] = Some(departure.y().read(memory) + 40);
                    self.goals[player] = None;
                    *pad |= DOWN;
                }
                continue;
            }
            if let Some(exit_y) = self.arrival_exit[player] {
                if position.1 < exit_y {
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
                } else if (24..36).contains(&(frame % 48)) {
                    *pad |= A;
                }
                continue;
            }
            let health = ram::g_player_health(revision)
                .at(player as i32)
                .read(memory);
            if health < 12 && self.previous_health[player] >= 12 {
                self.goals[player] = None;
            }
            self.previous_health[player] = health;
            if health >= 12 && self.exercise_roses(memory, rom, revision, player, frame, pad) {
                continue;
            }
            if let Some(goal) = self.goals[player]
                && self.goal_done(memory, revision, content, player, goal)
            {
                self.goals[player] = None;
            }
            if let Some(goal) = self.goals[player]
                && frame.saturating_sub(self.goal_since[player]) >= 3600
            {
                self.defer(player, goal, frame + 12000);
            }
            let partner_flag = ram::g_player_elevator_flags(revision)
                .at((player ^ 1) as i32)
                .read(memory);
            let present = self.present_goal(memory, revision, content, player);
            if matches!(self.goals[player], Some(Goal::Elevator { .. })) && present.is_some() {
                self.goals[player] = present;
                self.goal_since[player] = frame;
            }
            if self.players == 2
                && partner_flag & 1 != 0
                && ram::g_player_info(revision)
                    .at((player ^ 1) as i32)
                    .level()
                    .read(memory)
                    == level
                && present.is_none()
            {
                let goal = Goal::Elevator {
                    bank: (i32::from(partner_flag) - 1) / 2,
                };
                if self.goals[player] != Some(goal) {
                    self.goals[player] = Some(goal);
                    self.goal_since[player] = frame;
                }
            }
            if self.goals[player].is_none() {
                self.goals[player] =
                    self.select_goal(memory, rom, revision, content, player, frame);
                self.goal_since[player] = frame;
            }
            if let Some(goal) = self.goals[player] {
                match goal {
                    Goal::Present { .. } => {
                        if frame >= self.next_action[player] {
                            self.press(player, frame, B, pad);
                        }
                    }
                    Goal::Item {
                        bank,
                        index,
                        dropped,
                    } => {
                        let item = if dropped {
                            ram::g_dropped_present_table(revision).at(index)
                        } else {
                            ram::g_object_table(revision).at(bank).at(index)
                        };
                        let target = (item.x().read(memory), item.y().read(memory));
                        self.travel(memory, rom, revision, player, target, frame, pad);
                    }
                    Goal::Telephone { index } => {
                        let phone = ram::g_telephone_states(revision).at(index);
                        self.travel(
                            memory,
                            rom,
                            revision,
                            player,
                            (phone.x().read(memory), phone.y().read(memory) + 4),
                            frame,
                            pad,
                        );
                    }
                    Goal::Earthling { bank, index } => {
                        let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                        let kind = earthling.kind().read(memory);
                        let x = earthling.x().read(memory);
                        let y = earthling.y().read(memory);
                        if matches!(kind, 10 | 11 | 17)
                            && !self.nice_earthling_checked
                            && service_done(content, kind)
                            && matches!(
                                ram::g_player_powerups(revision)
                                    .at(player as i32)
                                    .read(memory),
                                3 | 4
                            )
                        {
                            if (position.1 - y).abs() < 8 && (position.0 - x).abs() < 56 {
                                *pad |= A | if position.0 < x { RIGHT } else { LEFT };
                            } else {
                                let target = (x + if position.0 < x { -40 } else { 40 }, y);
                                self.travel(memory, rom, revision, player, target, frame, pad);
                            }
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
                            if (position.0 - x).abs() < range_x
                                && position.1 > y
                                && position.1 - y < range_y
                            {
                                *pad |= UP;
                                continue;
                            }
                        }
                        let mut target = (x, y + 10);
                        if kind == 24 {
                            target = (x - 48, y - 4);
                        } else if kind == 23 {
                            target = if position.1 < y + 16 {
                                if position.0 > x - 60 {
                                    (x - 64, position.1)
                                } else {
                                    (x - 64, y + 24)
                                }
                            } else {
                                (x, y + 16)
                            };
                        }
                        if matches!(kind, EARTHLING_CUPID | 8..=11 | 17) {
                            let limits = if kind == EARTHLING_CUPID {
                                (24, 8)
                            } else if kind == 8 {
                                let definition = tje_mem::rom::earthling_definitions(revision)
                                    .at(i32::from(kind));
                                (
                                    i16::from(
                                        rom[definition.contact_distance_x().address() as usize],
                                    ),
                                    i16::from(
                                        rom[definition.contact_distance_y().address() as usize],
                                    ),
                                )
                            } else {
                                (if kind == 9 { 30 } else { 35 }, 15)
                            };
                            if (position.0 - x).abs() < limits.0
                                && (position.1 - y).abs() < limits.1
                            {
                                if kind == EARTHLING_CUPID {
                                    *pad |= A;
                                    if ram::g_player_powerups(revision)
                                        .at(player as i32)
                                        .read(memory)
                                        == 1
                                        && view.height().read(memory) <= 10
                                    {
                                        // A stationary spring-shoe player cannot
                                        // start a jump. Preserve the real windup.
                                        let velocity_x = view.velocity_x().read(memory);
                                        let velocity_y = view.velocity_y().read(memory);
                                        if velocity_x != 0 {
                                            *pad |= if velocity_x > 0 { RIGHT } else { LEFT };
                                        } else if velocity_y != 0 {
                                            *pad |= if velocity_y > 0 { DOWN } else { UP };
                                        } else {
                                            *pad |= if position.0 < x { RIGHT } else { LEFT };
                                        }
                                    }
                                }
                                continue;
                            }
                            let cached = self.service_contacts[player].filter(|(old, point)| {
                                *old == goal
                                    && (point.0 - x).abs() < limits.0
                                    && (point.1 - y).abs() < limits.1
                            });
                            if let Some((_, point)) = cached {
                                target = point;
                            } else {
                                let _ = self.navigation[player]
                                    .steer(memory, rom, revision, player, position, frame);
                                if let Some(point) = self.navigation[player]
                                    .reachable_land_in_range(position, (x, y), limits)
                                {
                                    self.service_contacts[player] = Some((goal, point));
                                    target = point;
                                }
                            }
                        }
                        if kind == 23
                            && (position.0 - x).abs() < 160
                            && (position.1 - y).abs() < 128
                        {
                            if position.1 < y + 16 {
                                *pad |= if position.0 > x - 60 { LEFT } else { DOWN };
                            } else if (position.0 - x).abs() > 4 {
                                *pad |= if position.0 < x { RIGHT } else { LEFT };
                            } else {
                                *pad |= UP;
                            }
                            continue;
                        }
                        if kind == 24 && (position.0 - x).abs() < 160 && (position.1 - y).abs() < 80
                        {
                            if position.1 > y + 12 && position.0 > x + 64 {
                                *pad |= LEFT;
                            } else if (position.1 - (y - 4)).abs() > 3 {
                                *pad |= if position.1 < y - 4 { DOWN } else { UP };
                            } else {
                                *pad |= if position.0 < x { RIGHT } else { LEFT };
                            }
                            continue;
                        }
                        if kind == 20
                            && (position.0 - x).abs() < 160
                            && (position.1 - y).abs() < 100
                        {
                            // Santa hears ordinary walking, powers and burps. A is
                            // the original sneak modifier when no power is active.
                            if ram::g_player_powerups(revision)
                                .at(player as i32)
                                .read(memory)
                                >= 0
                                || ram::g_player_burps_left(revision)
                                    .at(player as i32)
                                    .read(memory)
                                    > 0
                            {
                                continue;
                            }
                            if earthling.state().read(memory) == 1 {
                                continue;
                            }
                            if earthling.state().read(memory) > 1 {
                                self.defer(player, goal, frame + 1500);
                                continue;
                            }
                        }
                        if (position.0 - target.0).abs() < 10 && (position.1 - target.1).abs() < 5 {
                            if kind == 24 {
                                *pad |= RIGHT;
                            } else if kind == 23 {
                                *pad |= UP;
                            }
                        } else {
                            self.travel(memory, rom, revision, player, target, frame, pad);
                        }
                        if kind == EARTHLING_CUPID
                            && matches!(
                                ram::g_player_powerups(revision)
                                    .at(player as i32)
                                    .read(memory),
                                0 | 1 | 7
                            )
                        {
                            *pad |= A;
                        }
                        if kind == 20
                            && *pad & 15 != 0
                            && ram::g_player_powerups(revision)
                                .at(player as i32)
                                .read(memory)
                                < 0
                        {
                            *pad |= A;
                        }
                    }
                    Goal::Doorway => {
                        let handle = ram::g_player_doorway_entities(revision)
                            .at(player as i32)
                            .read(memory);
                        let door = ram::g_player_info(revision).at(i32::from(handle));
                        let target = (door.x().read(memory), door.y().read(memory) + 12);
                        let dx = position.0 - target.0;
                        let dy = position.1 - door.y().read(memory);
                        if dx.abs() < 64 && dy.abs() < 48 {
                            if dx.abs() > 3 {
                                *pad |= if dx < 0 { RIGHT } else { LEFT };
                            } else if !(-4..24).contains(&dy) {
                                *pad |= if dy < 12 { DOWN } else { UP };
                            } else if door.sprite_state().read(memory) != 0 {
                                *pad |= UP;
                            }
                        } else {
                            self.travel(memory, rom, revision, player, target, frame, pad);
                        }
                    }
                    Goal::Survey { point, .. }
                    | Goal::Path { point }
                    | Goal::FinalContact { point } => {
                        self.travel(memory, rom, revision, player, point, frame, pad);
                    }
                    Goal::Elevator { bank } => {
                        let elevator = ram::g_elevator_records(revision).at(bank).at(0);
                        let (x, y) = (elevator.x().read(memory), elevator.y().read(memory));
                        let (target, at_door) = elevator_approach(position, (x, y));
                        let dy = position.1 - y;
                        if at_door {
                            if elevator.state().read(memory) == 3 {
                                *pad |= UP;
                            } else if elevator.state().read(memory) <= 1 {
                                if dy < 12 {
                                    *pad |= DOWN;
                                } else if dy > 18 {
                                    *pad |= UP;
                                }
                            }
                        } else {
                            self.travel(memory, rom, revision, player, target, frame, pad);
                        }
                    }
                }
            }
        }
        if needs_gifts(content) {
            for bank in 0..2 {
                for index in 0..20 {
                    let santa = ram::g_earthling_banks(revision).at(bank).at(index);
                    if santa.kind().read(memory) != 20 || santa.state().read(memory) > 1 {
                        continue;
                    }
                    for (player, pad) in pads.iter_mut().enumerate().take(self.players) {
                        let view = ram::g_player_info(revision).at(player as i32);
                        if view.level().read(memory) != santa.level().read(memory)
                            || (view.x().read(memory) - santa.x().read(memory)).abs() >= 170
                            || (view.y().read(memory) - santa.y().read(memory)).abs() >= 110
                            || ram::g_player_lives(revision).at(player as i32).read(memory) < 0
                            || ram::g_player_inventory_open(revision)
                                .at(player as i32)
                                .read(memory)
                                != 0
                            || ram::g_player_interaction_choice_active(revision)
                                .at(player as i32)
                                .read(memory)
                                != 0
                        {
                            continue;
                        }
                        if santa.state().read(memory) == 1
                            || ram::g_player_powerups(revision)
                                .at(player as i32)
                                .read(memory)
                                >= 0
                            || ram::g_player_burps_left(revision)
                                .at(player as i32)
                                .read(memory)
                                > 0
                        {
                            *pad &= !(15 | A);
                        } else if *pad & 15 != 0 {
                            *pad |= A;
                        }
                    }
                }
            }
        }
        for (player, pad) in pads.iter_mut().enumerate().take(self.players) {
            if *pad & 15 == 0
                || ram::g_player_inventory_open(revision)
                    .at(player as i32)
                    .read(memory)
                    != 0
                || ram::g_player_elevator_flags(revision)
                    .at(player as i32)
                    .read(memory)
                    != 0
            {
                continue;
            }
            match ram::g_player_powerups(revision)
                .at(player as i32)
                .read(memory)
            {
                // Projectiles require a direction together with A; roses need
                // fresh A edges. Opening either gift alone exercises no ability.
                3 | 4 if frame % 96 < 12 => *pad |= A,
                6 if frame % 64 < 12 => *pad |= A,
                _ => {}
            }
        }
        if frame >= self.last_status + 1000 {
            self.last_status = frame;
            let stats: [(i8, i16, i16, i8, i8, i8, i8); 2] = std::array::from_fn(|player| {
                let view = ram::g_player_info(revision).at(player as i32);
                (
                    view.level().read(memory),
                    view.x().read(memory),
                    view.y().read(memory),
                    ram::g_player_health(revision)
                        .at(player as i32)
                        .read(memory),
                    ram::g_player_lives(revision).at(player as i32).read(memory),
                    ram::g_player_rank(revision).at(player as i32).read(memory),
                    view.sprite_state().read(memory),
                )
            });
            let inventories: [[i8; 16]; 2] = std::array::from_fn(|player| {
                std::array::from_fn(|slot| {
                    ram::g_player_inventory(revision)
                        .at(player as i32)
                        .at(slot as i32)
                        .read(memory)
                })
            });
            let resources: [(i8, i8, i8); 2] = std::array::from_fn(|player| {
                (
                    ram::g_player_bucks(revision).at(player as i32).read(memory),
                    ram::g_player_powerups(revision)
                        .at(player as i32)
                        .read(memory),
                    ram::g_player_elevator_flags(revision)
                        .at(player as i32)
                        .read(memory),
                )
            });
            let points: [u16; 2] = std::array::from_fn(|player| {
                ram::g_player_points(revision)
                    .at(player as i32)
                    .read(memory)
            });
            let terrain = [
                ram::g_player1_terrain_underfoot(revision).read(memory),
                ram::g_player2_terrain_underfoot(revision).read(memory),
            ];
            eprintln!(
                "content frame={frame} players={stats:?} points={points:?} terrain={terrain:?} resources={resources:?} inventories={inventories:?} goals={:?} presents={} earthlings={}",
                self.goals,
                content
                    .presents_opened
                    .values()
                    .filter(|v| v.is_some())
                    .count(),
                content
                    .earthlings_encountered
                    .values()
                    .filter(|v| v.is_some())
                    .count()
            );
        }
    }

    fn exercise_roses(
        &mut self,
        memory: &Ram<'_>,
        rom: &[u8],
        revision: Revision,
        player: usize,
        frame: u64,
        pad: &mut u16,
    ) -> bool {
        if self.rose_abilities == 7
            || ram::g_player_powerups(revision)
                .at(player as i32)
                .read(memory)
                != 6
        {
            return false;
        }
        let terrain = (if player == 0 {
            ram::g_player1_terrain_underfoot(revision).read(memory)
        } else {
            ram::g_player2_terrain_underfoot(revision).read(memory)
        }) & 0xf0;
        if matches!(terrain, 0x10 | 0x50) && self.rose_abilities & 3 != 3
            || !matches!(terrain, 0x10 | 0x50) && terrain < 0x80 && self.rose_abilities & 4 == 0
        {
            if frame % 48 < 12 {
                *pad |= A;
            }
            return true;
        }
        if self.rose_abilities & 4 != 0 {
            return false;
        }
        if let Some(Goal::Path { point }) = self.goals[player]
            && matches!(self.navigation[player].terrain_at(point), 0x20 | 0x30)
        {
            self.travel(memory, rom, revision, player, point, frame, pad);
            return true;
        }
        let view = ram::g_player_info(revision).at(player as i32);
        let position = (view.x().read(memory), view.y().read(memory));
        let _ = self.navigation[player].steer(memory, rom, revision, player, position, frame);
        let point = [0x30, 0x20]
            .into_iter()
            .filter_map(|terrain| {
                let desired = self.navigation[player].nearest_terrain(position, terrain)?;
                self.navigation[player]
                    .reachable_land_in_range(position, desired, (16, 16))
                    .filter(|point| {
                        matches!(self.navigation[player].terrain_at(*point), 0x20 | 0x30)
                    })
            })
            .min_by_key(|point| (position.0 - point.0).abs() + (position.1 - point.1).abs());
        if let Some(point) = point {
            self.goals[player] = Some(Goal::Path { point });
            self.goal_since[player] = frame;
            self.travel(memory, rom, revision, player, point, frame, pad);
            return true;
        }
        false
    }

    fn prepare_life_transfer(
        &mut self,
        memory: &Ram<'_>,
        rom: &[u8],
        revision: Revision,
        content: &Content,
        frame: u64,
        pads: &mut [u16; 2],
    ) -> bool {
        // Only a fresh two-player game prepares this service deliberately.
        // Continuing an existing campaign must not discard its real inventory.
        let active = self.initial_life_setup.get_or_insert_with(|| {
            self.players == 2
                && content.milestones["fixed_world"].is_some_and(|start| frame <= start + 120)
        });
        if !*active {
            return false;
        }
        if seen(&content.friendly_interactions, "life_request") {
            *active = false;
            return false;
        }
        if ram::g_player_lives(revision).at(1).read(memory) < 0 {
            if frame % 64 < 12 {
                pads[1] |= A;
            }
            if ram::g_player_interaction_choice_active(revision)
                .at(0)
                .read(memory)
                != 0
            {
                if frame % 48 < 12 {
                    pads[0] |= UP;
                } else if (24..36).contains(&(frame % 48)) {
                    pads[0] |= A;
                }
            } else if (32..44).contains(&(frame % 64)) {
                // Wake a waiting donor before requesting the real transfer.
                pads[0] |= A;
            }
            return true;
        }
        let player = ram::g_player_info(revision).at(1);
        if matches!(player.sprite_state().read(memory), 5 | 6) {
            return true;
        }
        let position = (player.x().read(memory), player.y().read(memory));
        let _ = self.navigation[1].steer(memory, rom, revision, 1, position, frame);
        if self.navigation[1].terrain_at(position) != 0x80
            && let Some(target) = self.navigation[1].nearest_terrain(position, 0x80)
            && let Some(input) =
                self.navigation[1].toward(memory, rom, revision, 1, (target, Travel::Swim), frame)
        {
            pads[1] |= input;
        }
        true
    }

    pub fn complete(
        &self,
        _memory: &Ram<'_>,
        _revision: Revision,
        content: &Content,
        _frame: u64,
    ) -> bool {
        if self.stockpiling {
            return self.full_inventory_reaction;
        }
        content.presents_opened.values().all(Option::is_some)
            && content.earthlings_encountered.values().all(Option::is_some)
            && content.friendly_interactions.values().all(Option::is_some)
            && (0..=8).all(|rank| content.ranks_reached.contains(&rank))
            && [
                "inventory_drop",
                "inventory_mail_order",
                "inventory_identification",
            ]
            .iter()
            .all(|name| seen(&content.milestones, name))
    }

    fn press(&mut self, player: usize, frame: u64, button: u16, pad: &mut u16) {
        *pad |= button;
        self.held[player] = button;
        self.held_until[player] = frame + 12;
        self.next_action[player] = frame + 36;
    }

    #[allow(clippy::too_many_arguments)]
    fn travel(
        &mut self,
        memory: &Ram<'_>,
        rom: &[u8],
        revision: Revision,
        player: usize,
        target: (i16, i16),
        frame: u64,
        pad: &mut u16,
    ) {
        let view = ram::g_player_info(revision).at(player as i32);
        let position = (view.x().read(memory), view.y().read(memory));
        let power = ram::g_player_powerups(revision)
            .at(player as i32)
            .read(memory);
        let travel = if power == 0 {
            Travel::Fly
        } else if power == 2 {
            Travel::Swim
        } else {
            Travel::Land
        };
        let input =
            self.navigation[player].toward(memory, rom, revision, player, (target, travel), frame);
        if travel == Travel::Land
            && self.navigation[player].terrain_at(position) == 0x80
            && let Some(shore) = self.navigation[player].nearest_terrain(position, 0x10)
            && let Some(input) = self.navigation[player].toward(
                memory,
                rom,
                revision,
                player,
                (shore, Travel::Swim),
                frame,
            )
        {
            *pad |= input;
            return;
        }
        if let Some(input) = input {
            self.mobility_needed[player] = false;
            *pad |= input;
            if power == 7 && input & 15 != 0 {
                *pad |= A;
            }
            return;
        }
        if travel == Travel::Land
            && let Some(point) = self.navigation[player]
                .marked_path_approach(memory, revision, player, position, target)
            && ((position.0 - point.0).abs() >= 4 || (position.1 - point.1).abs() >= 4)
            && self.goals[player] != Some(Goal::Path { point })
            && !self.blocked.iter().any(|&(level, goal, until)| {
                level == self.levels[player] && goal == Goal::Path { point } && until > frame
            })
        {
            self.goals[player] = Some(Goal::Path { point });
            self.goal_since[player] = frame;
            self.mobility_needed[player] = false;
            return;
        }
        self.mobility_needed[player] =
            !self.stockpiling || matches!(self.goals[player], Some(Goal::Elevator { .. }));
        if let Some(goal) = self.goals[player] {
            self.defer(player, goal, frame + 12000);
        }
    }

    fn defer(&mut self, player: usize, goal: Goal, until: u64) {
        let level = self.levels[player];
        if let Some(entry) = self
            .blocked
            .iter_mut()
            .find(|(old_level, old_goal, _)| *old_level == level && *old_goal == goal)
        {
            entry.2 = until;
        } else {
            self.blocked.push((level, goal, until));
        }
        self.goals[player] = None;
    }

    fn goal_done(
        &self,
        memory: &Ram<'_>,
        revision: Revision,
        content: &Content,
        player: usize,
        goal: Goal,
    ) -> bool {
        let level = self.levels[player];
        match goal {
            Goal::FinalContact { .. } => {
                ram::g_present_display_panel_drawn(revision).read(memory) != 0
            }
            Goal::Present { kind, drop } => {
                let inventory = ram::g_player_inventory(revision).at(player as i32);
                !(0..16).any(|slot| inventory.at(slot).read(memory) == kind)
                    || (drop && seen(&content.milestones, "inventory_drop"))
            }
            Goal::Item {
                bank,
                index,
                dropped,
            } => {
                let item = if dropped {
                    ram::g_dropped_present_table(revision).at(index)
                } else {
                    ram::g_object_table(revision).at(bank).at(index)
                };
                item.kind().read(memory) < 0 || item.level().read(memory) != level
            }
            Goal::Earthling { bank, index } => {
                let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                earthling.kind().read(memory) < 0
                    || earthling.level().read(memory) != level
                    || (earthling.kind().read(memory) == 20
                        && earthling.object().read(memory) != 0
                        && earthling.state().read(memory) > 1)
                    || (service_done(content, earthling.kind().read(memory))
                        && !(earthling.kind().read(memory) == 8 && !self.whirlwind_contact)
                        && !(earthling.kind().read(memory) == EARTHLING_CUPID
                            && !self.cupid_bounced
                            && matches!(
                                ram::g_player_powerups(revision)
                                    .at(player as i32)
                                    .read(memory),
                                0 | 1 | 7
                            ))
                        && !(matches!(earthling.kind().read(memory), 10 | 11 | 17)
                            && !self.nice_earthling_checked
                            && matches!(
                                ram::g_player_powerups(revision)
                                    .at(player as i32)
                                    .read(memory),
                                3 | 4
                            ))
                        && !(earthling.kind().read(memory) == 20 && needs_gifts(content))
                        && !(earthling.kind().read(memory) == 10
                            && ram::g_player_health(revision)
                                .at(player as i32)
                                .read(memory)
                                < 18
                            && ram::g_player_bucks(revision).at(player as i32).read(memory) > 0))
            }
            Goal::Doorway => {
                self.doorway_used[player]
                    || ram::g_player_doorway_entities(revision)
                        .at(player as i32)
                        .read(memory)
                        <= 0
            }
            Goal::Telephone { index } => {
                ram::g_telephone_states(revision)
                    .at(index)
                    .level()
                    .read(memory)
                    != level
            }
            Goal::Path { point } => {
                let view = ram::g_player_info(revision).at(player as i32);
                (view.x().read(memory) - point.0).abs() < 4
                    && (view.y().read(memory) - point.1).abs() < 4
            }
            Goal::Survey { column, row, .. } => {
                ram::g_map_visited_cover_bits(revision)
                    .at(i32::from(level))
                    .at(i32::from(row))
                    .read(memory)
                    & (0x80 >> column)
                    != 0
            }
            Goal::Elevator { bank } => {
                ram::g_elevator_records(revision)
                    .at(bank)
                    .at(0)
                    .level()
                    .read(memory)
                    != level
            }
        }
    }

    fn present_goal(
        &self,
        memory: &Ram<'_>,
        revision: Revision,
        content: &Content,
        player: usize,
    ) -> Option<Goal> {
        let inventory = ram::g_player_inventory(revision).at(player as i32);
        let terrain = if player == 0 {
            ram::g_player1_terrain_underfoot(revision).read(memory)
        } else {
            ram::g_player2_terrain_underfoot(revision).read(memory)
        };
        if (terrain >= 0x80
            || self.players == 1
                && self.levels[player] < ram::g_highest_level(revision).read(memory))
            && (0..16).any(|slot| inventory.at(slot).read(memory) == 23)
            && (0..16).any(|slot| inventory.at(slot).read(memory) == 12)
        {
            // Water forces a shark; togetherness also recovers a lone carrier
            // who fell below the highest level, without spending the Earthling.
            return Some(Goal::Present {
                kind: 12,
                drop: false,
            });
        }
        let health = ram::g_player_health(revision)
            .at(player as i32)
            .read(memory);
        if self.stockpiling && health >= 12 {
            if !content.presents_opened.get(&7).is_some_and(Option::is_some) {
                for kind in [27, 7] {
                    if (0..16).any(|slot| inventory.at(slot).read(memory) == kind) {
                        return Some(Goal::Present { kind, drop: false });
                    }
                }
            }
            let power = ram::g_player_powerups(revision)
                .at(player as i32)
                .read(memory);
            if self.mobility_needed[player] && power != 0 {
                for kind in [0, 2, 8, 1, 7, 27, 12, 26] {
                    if kind != power
                        && !(kind == 27 && power == 7)
                        && (0..16).any(|slot| inventory.at(slot).read(memory) == kind)
                    {
                        return Some(Goal::Present { kind, drop: false });
                    }
                }
            }
            return None;
        }
        let rank = ram::g_player_rank(revision).at(player as i32).read(memory);
        let count = (0..16)
            .filter(|&slot| inventory.at(slot).read(memory) >= 0)
            .count();
        let farming = promotion_stock_ready(memory, revision, player);
        let gearing = needs_mobility_gift(memory, revision, player);
        let jumping = !farming && needs_cupid_height(memory, revision, player, self.cupid_bounced);
        let hoarding = rank == 8 && !self.full_inventory_reaction && health >= 12;
        let cupid_supply = self.cupid_bounced
            || matches!(
                ram::g_player_powerups(revision)
                    .at(player as i32)
                    .read(memory),
                0 | 1
            )
            || (0..16).any(|slot| matches!(inventory.at(slot).read(memory), 0 | 1 | 18));
        let collecting = count < 12
            && health >= 12
            && ((1..8).contains(&rank)
                || !self.mobility_needed[player]
                    && (0..16).any(|slot| inventory.at(slot).read(memory) == 18))
            && (0..2).any(|bank| {
                (0..28).any(|index| {
                    let item = ram::g_object_table(revision).at(bank).at(index);
                    (0..28).contains(&item.kind().read(memory))
                        && item.level().read(memory) == self.levels[player]
                })
            });
        (0..16)
            .filter_map(|slot| {
                let kind = inventory.at(slot).read(memory);
                if !(0..28).contains(&kind) {
                    return None;
                }
                if kind == 23 && !cupid_supply {
                    // Preserve the sole carrier until a real height ability can
                    // accompany Cupid, or a retained reroller can supply one.
                    return None;
                }
                if hoarding
                    && !(self.mobility_needed[player] && kind == 0
                        || gearing && kind == 18
                        || jumping && matches!(kind, 0 | 1 | 18))
                {
                    return None;
                }
                if farming && !matches!(kind, 11 | 18) {
                    return None;
                }
                if collecting
                    && !(farming && kind == 18)
                    && !(jumping && matches!(kind, 0 | 1 | 18))
                    && !matches!(kind, 0 | 9..=11 | 14 | 17 | 20..=21 | 23)
                {
                    return None;
                }
                let missing = !content
                    .presents_opened
                    .get(&(kind as u8))
                    .is_some_and(Option::is_some);
                let score = if health < 12 && matches!(kind, 9 | 10 | 14 | 17) {
                    0
                } else if kind == 11 && rank < 8 || jumping && matches!(kind, 0 | 1 | 18) {
                    1
                } else if matches!(kind, 20 | 21)
                    || kind == 23
                        && (1..23).any(|kind| {
                            !content
                                .earthlings_encountered
                                .get(&kind)
                                .is_some_and(Option::is_some)
                        })
                {
                    2
                } else if missing && kind != 18 {
                    3
                } else if (kind == 18
                    && (farming || gearing || missing || count >= 5 || health < 12))
                    || matches!(kind, 7 | 27)
                        && !matches!(
                            ram::g_player_powerups(revision)
                                .at(player as i32)
                                .read(memory),
                            0 | 2 | 7
                        )
                {
                    4
                } else if count >= 12
                    && matches!(kind, 1..=4 | 6..=7 | 9..=10 | 14 | 17 | 19 | 25 | 27)
                {
                    5
                } else {
                    return None;
                };
                Some((score, slot, Goal::Present { kind, drop: false }))
            })
            .min_by_key(|&(score, slot, _)| (score, slot))
            .map(|(_, _, goal)| goal)
    }

    #[allow(clippy::too_many_arguments)]
    fn select_goal(
        &mut self,
        memory: &Ram<'_>,
        rom: &[u8],
        revision: Revision,
        content: &Content,
        player: usize,
        frame: u64,
    ) -> Option<Goal> {
        let level = self.levels[player];
        let blocked = &self.blocked;
        let eligible = |goal| {
            !blocked.iter().any(|&(old_level, old_goal, until)| {
                old_level == level && old_goal == goal && until > frame
            })
        };
        if level == 26 {
            if ram::g_present_display_panel_drawn(revision).read(memory) != 0 {
                return None;
            }
            let view = ram::g_player_info(revision).at(player as i32);
            let position = (view.x().read(memory), view.y().read(memory));
            let _ = self.navigation[player].steer(memory, rom, revision, player, position, frame);
            let word = |address: u32| {
                let address = address as usize;
                i16::from_be_bytes([rom[address], rom[address + 1]])
            };
            for index in 0..277 {
                let placement = tje_mem::rom::funkatron_scene_placements(revision).at(index);
                if word(placement.interaction().address()) == if player == 0 { 54 } else { 31 } {
                    let target = (word(placement.x().address()), word(placement.y().address()));
                    if let Some(point) =
                        self.navigation[player].reachable_land_in_range(position, target, (30, 20))
                    {
                        return Some(Goal::FinalContact { point });
                    }
                }
            }
            return None;
        }
        if !self.stockpiling && !self.whirlwind_contact {
            for bank in 0..2 {
                for index in 0..20 {
                    let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                    let goal = Goal::Earthling { bank, index };
                    if earthling.kind().read(memory) == 8
                        && earthling.level().read(memory) == level
                        && eligible(goal)
                    {
                        return Some(goal);
                    }
                }
            }
        }
        if !self.stockpiling
            && !self.cupid_bounced
            && matches!(
                ram::g_player_powerups(revision)
                    .at(player as i32)
                    .read(memory),
                0 | 1 | 7
            )
        {
            for bank in 0..2 {
                for index in 0..20 {
                    let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                    let goal = Goal::Earthling { bank, index };
                    if earthling.kind().read(memory) == EARTHLING_CUPID
                        && earthling.level().read(memory) == level
                        && eligible(goal)
                    {
                        return Some(goal);
                    }
                }
            }
        }
        if !self.stockpiling
            && !self.nice_earthling_checked
            && matches!(
                ram::g_player_powerups(revision)
                    .at(player as i32)
                    .read(memory),
                3 | 4
            )
        {
            for bank in 0..2 {
                for index in 0..20 {
                    let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                    let kind = earthling.kind().read(memory);
                    let goal = Goal::Earthling { bank, index };
                    if matches!(kind, 10 | 11 | 17)
                        && service_done(content, kind)
                        && earthling.level().read(memory) == level
                        && eligible(goal)
                    {
                        return Some(goal);
                    }
                }
            }
        }
        if level == 0 {
            for kind in [23, 24] {
                if service_done(content, kind) {
                    continue;
                }
                for bank in 0..2 {
                    for index in 0..20 {
                        let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                        let goal = Goal::Earthling { bank, index };
                        if earthling.kind().read(memory) == kind
                            && earthling.level().read(memory) == level
                            && eligible(goal)
                        {
                            return Some(goal);
                        }
                    }
                }
            }
        }
        let mobility_needed = self.mobility_needed[player]
            && ram::g_player_powerups(revision)
                .at(player as i32)
                .read(memory)
                != 0;
        if mobility_needed {
            if !self.doorway_used[player]
                && ram::g_player_doorway_entities(revision)
                    .at(player as i32)
                    .read(memory)
                    > 0
                && eligible(Goal::Doorway)
            {
                return Some(Goal::Doorway);
            }
            if (0..16).any(|slot| {
                ram::g_player_inventory(revision)
                    .at(player as i32)
                    .at(slot)
                    .read(memory)
                    == 8
            }) {
                return Some(Goal::Present {
                    kind: 8,
                    drop: false,
                });
            }
        }
        if mobility_needed
            && (0..16).any(|slot| {
                ram::g_player_inventory(revision)
                    .at(player as i32)
                    .at(slot)
                    .read(memory)
                    == 0
            })
        {
            return Some(Goal::Present {
                kind: 0,
                drop: false,
            });
        }
        let view = ram::g_player_info(revision).at(player as i32);
        let position = (view.x().read(memory), view.y().read(memory));
        if needs_gifts(content) {
            for bank in 0..2 {
                for index in 0..20 {
                    let santa = ram::g_earthling_banks(revision).at(bank).at(index);
                    let goal = Goal::Earthling { bank, index };
                    if santa.kind().read(memory) == 20
                        && santa.level().read(memory) == level
                        && (santa.state().read(memory) <= 1 || santa.object().read(memory) == 0)
                        && (santa.x().read(memory) - position.0).abs() < 400
                        && (santa.y().read(memory) - position.1).abs() < 250
                        && eligible(goal)
                    {
                        return Some(goal);
                    }
                }
            }
        }
        if let Some(
            goal @ Goal::Present {
                kind: 1 | 11 | 23, ..
            },
        ) = self.present_goal(memory, revision, content, player)
            && eligible(goal)
        {
            return Some(goal);
        }
        let health = ram::g_player_health(revision)
            .at(player as i32)
            .read(memory);
        let bucks = ram::g_player_bucks(revision).at(player as i32).read(memory);
        let inventory = ram::g_player_inventory(revision).at(player as i32);
        if health >= 12
            && bucks >= 2
            && !service_done(content, 11)
            && (0..16).any(|slot| {
                let kind = inventory.at(slot).read(memory);
                (0..26).contains(&kind)
                    && ram::g_present_info(revision)
                        .at(i32::from(kind))
                        .identified()
                        .read(memory)
                        == 0
            })
        {
            for bank in 0..2 {
                for index in 0..20 {
                    let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                    let goal = Goal::Earthling { bank, index };
                    if earthling.kind().read(memory) == 11
                        && earthling.level().read(memory) == level
                        && eligible(goal)
                    {
                        return Some(goal);
                    }
                }
            }
        }
        if let Some(goal) = self.present_goal(memory, revision, content, player)
            && eligible(goal)
            && (health >= 12
                || matches!(
                    goal,
                    Goal::Present {
                        kind: 0 | 7 | 9..=12 | 14 | 17..=21 | 27,
                        ..
                    }
                ))
        {
            return Some(goal);
        }
        let count = (0..16)
            .filter(|&slot| inventory.at(slot).read(memory) >= 0)
            .count();
        if !seen(&content.milestones, "inventory_drop")
            && let Some(kind) = (0..16)
                .map(|slot| inventory.at(slot).read(memory))
                .find(|&kind| {
                    kind >= 0
                        && content
                            .presents_opened
                            .get(&(kind as u8))
                            .is_some_and(Option::is_some)
                })
        {
            return Some(Goal::Present { kind, drop: true });
        }
        let mut best: Option<(i32, Goal)> = None;
        let mut offer = |priority: i32, target: (i16, i16), goal| {
            let distance = (i32::from(position.0) - i32::from(target.0)).abs()
                + (i32::from(position.1) - i32::from(target.1)).abs();
            let score = priority * 10000 + distance;
            if eligible(goal) && best.is_none_or(|(old, _)| score < old) {
                best = Some((score, goal));
            }
        };
        for bank in 0..2 {
            for index in 0..20 {
                let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                let kind = earthling.kind().read(memory);
                if !(0..25).contains(&kind)
                    || earthling.level().read(memory) != level
                    || self.stockpiling && !matches!(kind, 9..=10 | 20 | 23..=24)
                    || (service_done(content, kind)
                        && !(kind == 20 && needs_gifts(content))
                        && !(kind == 10 && health < 18 && bucks > 0))
                {
                    continue;
                }
                let can_pay = match kind {
                    9 => count < 16 && bucks >= 3,
                    10 => bucks >= 1,
                    11 => count > 0 && bucks >= 2,
                    17 => bucks >= 3,
                    20 => {
                        earthling.state().read(memory) <= 1 || earthling.object().read(memory) == 0
                    }
                    _ => true,
                };
                let encountered = content
                    .earthlings_encountered
                    .get(&(kind as u8))
                    .is_some_and(Option::is_some);
                if !can_pay && encountered {
                    continue;
                }
                offer(
                    if can_pay && (kind == 10 && health < 18 || kind == 17) {
                        0
                    } else if encountered {
                        2
                    } else {
                        3
                    },
                    (earthling.x().read(memory), earthling.y().read(memory)),
                    Goal::Earthling { bank, index },
                );
            }
            for index in 0..28 {
                let item = ram::g_object_table(revision).at(bank).at(index);
                if item.level().read(memory) != level {
                    continue;
                }
                let kind = item.kind().read(memory);
                if self.stockpiling && count >= 15 && kind == 80 && health >= 18 {
                    continue;
                }
                let priority = item_priority(
                    kind,
                    health,
                    bucks,
                    count,
                    rom,
                    revision,
                    content,
                    !self.full_inventory_reaction,
                );
                if let Some(priority) = priority {
                    offer(
                        priority,
                        (item.x().read(memory), item.y().read(memory)),
                        Goal::Item {
                            bank,
                            index,
                            dropped: false,
                        },
                    );
                }
            }
        }
        for index in 0..32 {
            let item = ram::g_dropped_present_table(revision).at(index);
            if item.level().read(memory) == level
                && let Some(priority) = item_priority(
                    item.kind().read(memory),
                    health,
                    bucks,
                    count,
                    rom,
                    revision,
                    content,
                    !self.full_inventory_reaction,
                )
            {
                offer(
                    priority,
                    (item.x().read(memory), item.y().read(memory)),
                    Goal::Item {
                        bank: 0,
                        index,
                        dropped: true,
                    },
                );
            }
        }
        for index in 0..2 {
            let phone = ram::g_telephone_states(revision).at(index);
            if phone.level().read(memory) == level
                && level > 0
                && !seen(&content.friendly_interactions, "telephone_answered")
            {
                offer(
                    2,
                    (phone.x().read(memory), phone.y().read(memory)),
                    Goal::Telephone { index },
                );
            }
        }
        if let Some((_, goal)) = best {
            return Some(goal);
        }
        if needs_gifts(content)
            && (0..2).any(|bank| {
                (0..20).any(|index| {
                    let santa = ram::g_earthling_banks(revision).at(bank).at(index);
                    santa.kind().read(memory) == 20
                        && santa.level().read(memory) == level
                        && santa.object().read(memory) != 0
                        && santa.state().read(memory) > 1
                })
            })
            && let Some(point) = self.navigation[player].nearest_terrain(
                (
                    position.0 + if position.0 < 1344 { 512 } else { -512 },
                    position.1,
                ),
                0x10,
            )
            && (position.0 - point.0).abs() > 320
            && eligible(Goal::Path { point })
        {
            return Some(Goal::Path { point });
        }
        // Discovery points are awarded only for actually entering an uncovered
        // 320x256 region. These goals earn the remaining ranks by walking the
        // generated world, not by setting points or opening the map screen.
        if !self.stockpiling
            && (2..26).contains(&level)
            && ram::g_player_rank(revision).at(player as i32).read(memory) < 8
        {
            let mut survey = None;
            for row in 0..7_i16 {
                let visited = ram::g_map_visited_cover_bits(revision)
                    .at(i32::from(level))
                    .at(i32::from(row))
                    .read(memory);
                for column in 0..8_i16 {
                    if visited & (0x80 >> column) != 0 {
                        continue;
                    }
                    let center = (72 + column * 320 + 160, 8 + row * 256 + 128);
                    if let Some(point) = self.navigation[player].reachable_land_in_range(
                        position,
                        center,
                        (160, 128),
                    ) {
                        let distance = (i32::from(position.0) - i32::from(point.0)).abs()
                            + (i32::from(position.1) - i32::from(point.1)).abs();
                        let goal = Goal::Survey { column, row, point };
                        if eligible(goal) && survey.is_none_or(|(old, _)| distance < old) {
                            survey = Some((distance, goal));
                        }
                    }
                }
            }
            if let Some((_, goal)) = survey {
                return Some(goal);
            }
        }
        (0..2).find_map(|bank| {
            let elevator = ram::g_elevator_records(revision).at(bank).at(0);
            (elevator.level().read(memory) == level
                && elevator.state().read(memory) != 11
                && eligible(Goal::Elevator { bank }))
            .then_some(Goal::Elevator { bank })
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn earthling_present_ready(
        &mut self,
        memory: &Ram<'_>,
        rom: &[u8],
        revision: Revision,
        content: &Content,
        player: usize,
        frame: u64,
    ) -> bool {
        let view = ram::g_player_info(revision).at(player as i32);
        if view.level().read(memory) == 0 || ram::g_split_screen_enabled(revision).read(memory) != 0
        {
            return true;
        }
        let wanted = (!self.cupid_bounced)
            .then_some(EARTHLING_CUPID as u8)
            .or_else(|| {
                [15, 17, 12, 13, 19, 21, 22].into_iter().find(|&kind| {
                    !content
                        .earthlings_encountered
                        .get(&kind)
                        .is_some_and(Option::is_some)
                })
            });
        let Some(wanted) = wanted else {
            return true;
        };
        let position = (view.x().read(memory), view.y().read(memory));
        let _ = self.navigation[player].steer(memory, rom, revision, player, position, frame);
        let underfoot = if player == 0 {
            ram::g_player1_terrain_underfoot(revision).read(memory)
        } else {
            ram::g_player2_terrain_underfoot(revision).read(memory)
        };
        let mut state = ram::g_rng_state(revision).read(memory);
        let mut terrain = underfoot;
        // OpenEarthlingPresent samples up to four real +/-40,+/-25 positions.
        // Forced water/edge definitions cannot be changed just by timing.
        for _ in 0..4 {
            let x = position.0
                + if next_random(&mut state) & 1 != 0 {
                    40
                } else {
                    -40
                };
            let y = position.1
                + if next_random(&mut state) & 1 != 0 {
                    25
                } else {
                    -25
                };
            terrain = self.navigation[player].terrain_at((x, y));
            if (underfoot == 0x80) == (terrain == 0x80) {
                break;
            }
        }
        if matches!(terrain, 0x50 | 0x80 | 0x90) {
            return true;
        }
        next_random(&mut state) % 22 + 1 == u16::from(wanted)
    }

    #[allow(clippy::too_many_arguments)]
    fn inventory_input(
        &mut self,
        memory: &Ram<'_>,
        rom: &[u8],
        revision: Revision,
        content: &Content,
        player: usize,
        frame: u64,
        pad: &mut u16,
    ) {
        if frame < self.next_action[player] {
            return;
        }
        let mode = self.inventory_modes[player];
        if self.close_inventory[player] || self.inventory_actions[player] > 0 {
            self.press(player, frame, B, pad);
            return;
        }
        let inventory = ram::g_player_inventory(revision).at(player as i32);
        let bucks = ram::g_player_bucks(revision).at(player as i32).read(memory);
        let selected = if mode == 2 {
            let full = (0..16).all(|slot| inventory.at(slot).read(memory) >= 0);
            (0..3)
                .filter(|&row| {
                    !full
                        && ram::g_player_mailbox_prices(revision)
                            .at(player as i32)
                            .at(row)
                            .read(memory)
                            <= bucks
                })
                .min_by_key(|&row| {
                    let kind = ram::g_player_mailbox_items(revision)
                        .at(player as i32)
                        .at(row)
                        .read(memory);
                    (
                        content
                            .presents_opened
                            .get(&(kind as u8))
                            .is_some_and(Option::is_some),
                        row,
                    )
                })
                .map(|row| (row * 2, false))
        } else if mode == 3 {
            (0..16)
                .filter(|&slot| {
                    let kind = inventory.at(slot).read(memory);
                    bucks >= 2
                        && (0..28).contains(&kind)
                        && (kind == 26
                            || ram::g_present_info(revision)
                                .at(i32::from(kind))
                                .identified()
                                .read(memory)
                                == 0)
                })
                .min_by_key(|&slot| (inventory.at(slot).read(memory) == 26, slot))
                .map(|slot| (slot, false))
        } else {
            let goal = match self.goals[player] {
                Some(Goal::Present { .. }) => self.goals[player],
                _ => self.present_goal(memory, revision, content, player),
            };
            goal.and_then(|goal| {
                let Goal::Present { kind, drop } = goal else {
                    return None;
                };
                (0..16)
                    .find(|&slot| inventory.at(slot).read(memory) == kind)
                    .map(|slot| (slot, drop))
            })
        };
        let Some((slot, drop)) = selected else {
            self.press(player, frame, B, pad);
            return;
        };
        let row = ram::g_player_inventory_scroll_row(revision)
            .at(player as i32)
            .read(memory)
            + ram::g_player_inventory_selection_row(revision)
                .at(player as i32)
                .read(memory);
        let column = ram::g_player_inventory_selection_column(revision)
            .at(player as i32)
            .read(memory);
        let button = if i32::from(row) > slot / 2 {
            UP
        } else if i32::from(row) < slot / 2 {
            DOWN
        } else if mode != 2 && i32::from(column) != slot % 2 {
            if column == 0 { RIGHT } else { LEFT }
        } else if mode == 1
            && (ram::g_inventory_banner_mode(revision)
                .at(player as i32)
                .read(memory)
                != 0)
                != drop
        {
            C
        } else if mode == 1
            && !drop
            && inventory.at(slot).read(memory) == 18
            && !randomizer_ready(memory, rom, revision, content, player, self.cupid_bounced)
        {
            // Menus preserve the live RNG. Close this real menu to let ordinary
            // gameplay advance it, then try another genuine opening timing.
            self.close_inventory[player] = true;
            B
        } else if mode == 1
            && !drop
            && inventory.at(slot).read(memory) == 23
            && !self.earthling_present_ready(memory, rom, revision, content, player, frame)
        {
            self.close_inventory[player] = true;
            B
        } else {
            self.inventory_actions[player] += 1;
            self.close_inventory[player] = true;
            if mode == 1 && !drop && inventory.at(slot).read(memory) == 8 {
                self.doorway_used[player] = false;
            }
            A
        };
        self.press(player, frame, button, pad);
    }
}

fn elevator_approach(position: (i16, i16), elevator: (i16, i16)) -> ((i16, i16), bool) {
    let (x, y) = elevator;
    let flank = x + if position.0 < x { -32 } else { 32 };
    let dx = (position.0 - x).abs();
    let target = if position.1 <= y && dx < 24 {
        (flank, y - 8)
    } else if position.1 < y + 18 && dx >= 24 {
        (flank, y + 22)
    } else {
        (x, y + 22)
    };
    let at_door = target.0 == x && dx < 8 && (0..40).contains(&(position.1 - y));
    (target, at_door)
}

fn needs_mobility_gift(memory: &Ram<'_>, revision: Revision, player: usize) -> bool {
    ram::g_player_rank(revision).at(player as i32).read(memory) == 8
        && !matches!(
            ram::g_player_powerups(revision)
                .at(player as i32)
                .read(memory),
            0 | 2
        )
        && !(0..16).any(|slot| {
            matches!(
                ram::g_player_inventory(revision)
                    .at(player as i32)
                    .at(slot)
                    .read(memory),
                0 | 2
            )
        })
}

fn promotion_stock_ready(memory: &Ram<'_>, revision: Revision, player: usize) -> bool {
    let rank = ram::g_player_rank(revision).at(player as i32).read(memory);
    if !(0..8).contains(&rank)
        || ram::g_player_health(revision)
            .at(player as i32)
            .read(memory)
            < 12
    {
        return false;
    }
    let inventory = ram::g_player_inventory(revision).at(player as i32);
    let mutable = (0..16)
        .filter(|&slot| matches!(inventory.at(slot).read(memory), 0..=26))
        .count();
    // The last promotion needs no further randomizer; earlier ranks do.
    let required = if rank == 7 { 1 } else { 9 - rank };
    mutable >= required as usize && (0..16).any(|slot| inventory.at(slot).read(memory) == 18)
}

fn needs_cupid_height(memory: &Ram<'_>, revision: Revision, player: usize, bounced: bool) -> bool {
    if bounced
        || matches!(
            ram::g_player_powerups(revision)
                .at(player as i32)
                .read(memory),
            0 | 1
        )
    {
        return false;
    }
    let level = ram::g_player_info(revision)
        .at(player as i32)
        .level()
        .read(memory);
    (0..2).any(|bank| {
        ram::g_earthling_bank_users(revision).at(bank).read(memory) > 0
            && (0..20).any(|index| {
                let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                earthling.kind().read(memory) == EARTHLING_CUPID
                    && earthling.level().read(memory) == level
            })
    })
}

fn randomizer_ready(
    memory: &Ram<'_>,
    rom: &[u8],
    revision: Revision,
    content: &Content,
    player: usize,
    cupid_bounced: bool,
) -> bool {
    if ram::g_split_screen_enabled(revision).read(memory) != 0 {
        return true;
    }
    let bucks = ram::g_player_bucks(revision).at(player as i32).read(memory);
    let level = ram::g_player_info(revision)
        .at(player as i32)
        .level()
        .read(memory);
    let needs_singer_cash = bucks < 3
        && !service_done(content, 17)
        && (0..2).any(|bank| {
            (0..20).any(|index| {
                let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                earthling.kind().read(memory) == 17
                    && earthling.level().read(memory) == level
                    && earthling.object().read(memory) > 0
            })
        });
    let farming = promotion_stock_ready(memory, revision, player);
    let gearing = needs_mobility_gift(memory, revision, player);
    let jumping = !farming && needs_cupid_height(memory, revision, player, cupid_bounced);
    if !farming && !gearing && !jumping && !needs_singer_cash {
        return true;
    }
    let inventory = predict_randomizer(memory, rom, revision);
    if needs_singer_cash {
        let cash: i16 = inventory[player]
            .iter()
            .map(|kind| match kind {
                20 => 1,
                21 => 5,
                _ => 0,
            })
            .sum();
        return i16::from(bucks) + cash >= 3;
    }
    if jumping {
        return inventory[player].contains(&0) || inventory[player].contains(&1);
    }
    if gearing {
        return inventory[player].contains(&0) || inventory[player].contains(&2);
    }
    inventory[player].contains(&11)
        && (ram::g_player_rank(revision).at(player as i32).read(memory) == 7
            || inventory[player].contains(&18))
}

fn next_random(state: &mut i32) -> u16 {
    // GetRandomNumber: Schrage's Park-Miller step, on a private scalar only.
    let next = (*state % 127773)
        .wrapping_mul(16807)
        .wrapping_sub((*state / 127773).wrapping_mul(2836));
    *state = if next > 0 {
        next
    } else {
        next.wrapping_add(2147483647)
    };
    *state as u16
}

fn predict_randomizer(memory: &Ram<'_>, rom: &[u8], revision: Revision) -> [[i8; 16]; 2] {
    let mut inventory = std::array::from_fn(|player| {
        std::array::from_fn(|slot| {
            ram::g_player_inventory(revision)
                .at(player as i32)
                .at(slot as i32)
                .read(memory)
        })
    });
    let mut state = ram::g_rng_state(revision).read(memory);
    let address = tje_mem::rom::present_weight_table(revision).address() as usize;
    let weights = &rom[address..address + 27];
    let total: u32 = weights.iter().map(|&weight| u32::from(weight)).sum();
    // SetupPresentIdentities makes 26 draws. RandomizePresents then runs sixty
    // rounds; redraws and cached UI randoms do not advance gRNGState. All weights
    // remain runtime cartridge data, never embedded in the recording or planner.
    for _ in 0..26 {
        next_random(&mut state);
    }
    for _ in 0..60 {
        for player in &mut inventory {
            for kind in player.iter_mut().filter(|kind| matches!(kind, 0..=26)) {
                let mut selected = 0;
                while selected == 0 {
                    selected = u32::from(next_random(&mut state)) % (total + 1);
                }
                let mut cumulative = 0;
                for (index, &weight) in weights.iter().enumerate() {
                    cumulative += u32::from(weight);
                    if selected <= cumulative {
                        *kind = index as i8;
                        break;
                    }
                }
            }
        }
    }
    inventory
}

fn seen(map: &std::collections::BTreeMap<String, Option<u64>>, name: &str) -> bool {
    map.get(name).is_some_and(Option::is_some)
}

fn needs_gifts(content: &Content) -> bool {
    !service_done(content, 20)
        || !content.ranks_reached.contains(&8)
        || content.presents_opened.values().any(Option::is_none)
        || content
            .earthlings_encountered
            .range(1..23)
            .any(|(_, frame)| frame.is_none())
}

fn service_done(content: &Content, kind: i8) -> bool {
    let key = match kind {
        5 => "hula_dance",
        9 => "mail_order_purchase",
        10 => "wizard_paid",
        11 => "wiseman_identification",
        17 => "opera_paid",
        20 => "santa_gifts",
        23 => "lemonade_reward",
        24 => "hottub_bath",
        _ => {
            return content
                .earthlings_encountered
                .get(&(kind as u8))
                .is_some_and(Option::is_some);
        }
    };
    seen(&content.friendly_interactions, key)
}

#[allow(clippy::too_many_arguments)]
fn item_priority(
    kind: i8,
    health: i8,
    bucks: i8,
    count: usize,
    rom: &[u8],
    revision: Revision,
    content: &Content,
    full_reaction_needed: bool,
) -> Option<i32> {
    if kind < 0 {
        return None;
    }
    if full_reaction_needed && kind < 28 {
        return Some(if count == 16 { 0 } else { 1 });
    }
    if (64..80).contains(&kind) && health < 18 {
        let unit = tje_mem::rom_data_units(revision)
            .iter()
            .find(|unit| unit.name == "PresentAuxiliarySignedLookup")
            .unwrap();
        if rom[unit.address as usize + (kind - 64) as usize] as i8 > 0 {
            return Some(0);
        }
    }
    if kind == 80 && bucks < 12 {
        return Some(1);
    }
    if kind < 28 && count < 16 {
        if !content
            .presents_opened
            .get(&(kind as u8))
            .is_some_and(Option::is_some)
        {
            return Some(1);
        }
        if matches!(kind, 11 | 18 | 20 | 21) {
            return Some(2);
        }
        if kind == 23 {
            return Some(0);
        }
        if count < 14 {
            return Some(4);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn byte(memory: &mut [u8], address: u32, value: i8) {
        memory[(address & 0xFFFF) as usize] = value as u8;
    }

    #[test]
    fn an_elevator_flank_is_not_the_doorway() {
        let elevator = (1392, 768);
        for side in [-1, 1] {
            let flank = elevator.0 + side * 32;
            assert_eq!(
                elevator_approach((elevator.0 + side * 16, 756), elevator),
                ((flank, 760), false)
            );
            for y in [760, 779, 785] {
                assert_eq!(
                    elevator_approach((flank, y), elevator),
                    ((flank, 790), false)
                );
            }
            assert_eq!(
                elevator_approach((flank, 790), elevator),
                ((1392, 790), false)
            );
        }
        assert_eq!(
            elevator_approach((1392, 784), elevator),
            ((1392, 790), true)
        );
    }

    #[test]
    fn storage_identity_and_promotions_drive_inventory_selection() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                byte(&mut memory, inventory.at(slot).address(), -1);
            }
            byte(
                &mut memory,
                ram::g_player_health(revision).at(0).address(),
                23,
            );
            byte(&mut memory, inventory.at(0).address(), 27);
            byte(&mut memory, inventory.at(1).address(), 26);
            let bot = ContentBot::new(1);
            let mut content = Content::new();
            assert_eq!(
                bot.present_goal(&Ram(&memory), revision, &content, 0),
                Some(Goal::Present {
                    kind: 27,
                    drop: false
                })
            );
            content.presents_opened.insert(27, Some(1));
            assert_eq!(
                bot.present_goal(&Ram(&memory), revision, &content, 0),
                Some(Goal::Present {
                    kind: 26,
                    drop: false
                })
            );
            byte(&mut memory, inventory.at(2).address(), 11);
            assert_eq!(
                bot.present_goal(&Ram(&memory), revision, &content, 0),
                Some(Goal::Present {
                    kind: 11,
                    drop: false
                })
            );
        }
    }

    #[test]
    fn ringing_phone_remains_an_unfinished_interaction() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            byte(
                &mut memory,
                ram::g_telephone_states(revision).at(0).level().address(),
                3,
            );
            byte(
                &mut memory,
                ram::g_telephone_activated(revision).at(0).address(),
                1,
            );
            let mut bot = ContentBot::new(1);
            bot.levels[0] = 3;
            assert!(!bot.goal_done(
                &Ram(&memory),
                revision,
                &Content::new(),
                0,
                Goal::Telephone { index: 0 }
            ));
        }
    }

    #[test]
    fn an_encounter_does_not_substitute_for_a_paid_service() {
        let mut content = Content::new();
        content.earthlings_encountered.insert(10, Some(1));
        assert!(!service_done(&content, 10));
        content
            .friendly_interactions
            .insert("wizard_paid".into(), Some(2));
        assert!(service_done(&content, 10));
        assert!(!ContentBot::new(1).complete(&Ram(&[0; 65536]), Revision::Rev02, &content, 3));
    }

    #[test]
    fn hidden_island_service_precedes_inventory_effects() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                byte(&mut memory, inventory.at(slot).address(), -1);
            }
            byte(&mut memory, inventory.at(0).address(), 27);
            for bank in 0..2 {
                for index in 0..20 {
                    byte(
                        &mut memory,
                        ram::g_earthling_banks(revision)
                            .at(bank)
                            .at(index)
                            .kind()
                            .address(),
                        -1,
                    );
                }
            }
            let stand = ram::g_earthling_banks(revision).at(0).at(0);
            byte(&mut memory, stand.kind().address(), 23);
            let mut bot = ContentBot::new(1);
            bot.levels[0] = 0;
            assert_eq!(
                bot.select_goal(&Ram(&memory), &[], revision, &Content::new(), 0, 1),
                Some(Goal::Earthling { bank: 0, index: 0 })
            );
        }
    }

    #[test]
    fn previously_opened_wings_remain_useful_for_unreachable_goals() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                byte(&mut memory, inventory.at(slot).address(), -1);
            }
            byte(&mut memory, inventory.at(0).address(), 0);
            byte(&mut memory, inventory.at(1).address(), 27);
            byte(
                &mut memory,
                ram::g_player_powerups(revision).at(0).address(),
                -1,
            );
            let mut content = Content::new();
            content.presents_opened.insert(0, Some(1));
            let mut bot = ContentBot::new(1);
            bot.levels[0] = 5;
            bot.mobility_needed[0] = true;
            assert_eq!(
                bot.select_goal(&Ram(&memory), &[], revision, &content, 0, 2),
                Some(Goal::Present {
                    kind: 0,
                    drop: false
                })
            );
        }
    }

    #[test]
    fn active_wings_are_not_displaced_by_stale_doorway_mobility() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                byte(&mut memory, inventory.at(slot).address(), -1);
            }
            byte(&mut memory, inventory.at(0).address(), 8);
            byte(
                &mut memory,
                ram::g_player_doorway_entities(revision).at(0).address() + 1,
                2,
            );
            let mut content = Content::new();
            content.presents_opened.insert(8, Some(1));
            let mut bot = ContentBot::new(1);
            bot.levels[0] = 1;
            bot.mobility_needed[0] = true;
            assert_ne!(
                bot.select_goal(&Ram(&memory), &[], revision, &content, 0, 40000),
                Some(Goal::Doorway)
            );
            assert_ne!(
                bot.select_goal(&Ram(&memory), &[], revision, &content, 0, 40000),
                Some(Goal::Present {
                    kind: 8,
                    drop: false
                })
            );
        }
    }

    #[test]
    fn retry_bookkeeping_updates_instead_of_growing_per_frame() {
        let mut bot = ContentBot::new(1);
        bot.levels[0] = 4;
        let goal = Goal::Elevator { bank: 0 };
        for frame in 0..1000 {
            bot.defer(0, goal, frame + 12000);
        }
        assert_eq!(bot.blocked, vec![(4, goal, 12999)]);
        assert_eq!(bot.navigation.len(), 1);
    }

    #[test]
    fn unseen_randomizer_opens_without_an_arbitrary_inventory_quota() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                byte(&mut memory, inventory.at(slot).address(), -1);
            }
            byte(&mut memory, inventory.at(0).address(), 18);
            byte(
                &mut memory,
                ram::g_player_health(revision).at(0).address(),
                23,
            );
            assert_eq!(
                ContentBot::new(1).present_goal(&Ram(&memory), revision, &Content::new(), 0),
                Some(Goal::Present {
                    kind: 18,
                    drop: false
                })
            );
        }
    }

    #[test]
    fn collecting_for_a_randomizer_keeps_promotions_and_earthlings_actionable() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                byte(&mut memory, inventory.at(slot).address(), -1);
            }
            byte(&mut memory, inventory.at(0).address(), 18);
            byte(&mut memory, inventory.at(1).address(), 23);
            byte(&mut memory, inventory.at(2).address(), 11);
            byte(
                &mut memory,
                ram::g_player_health(revision).at(0).address(),
                23,
            );
            let mut bot = ContentBot::new(1);
            bot.levels[0] = 4;
            let item = ram::g_object_table(revision).at(0).at(0);
            byte(&mut memory, item.kind().address(), 9);
            byte(&mut memory, item.level().address(), 4);
            let content = Content::new();
            assert_eq!(
                bot.present_goal(&Ram(&memory), revision, &content, 0),
                Some(Goal::Present {
                    kind: 11,
                    drop: false
                })
            );
            byte(&mut memory, inventory.at(2).address(), -1);
            assert_eq!(
                bot.present_goal(&Ram(&memory), revision, &content, 0),
                Some(Goal::Present {
                    kind: 23,
                    drop: false
                })
            );
        }
    }

    #[test]
    fn a_waiting_rider_leaves_to_open_available_content() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            for player in 0..2 {
                let inventory = ram::g_player_inventory(revision).at(player);
                for slot in 0..16 {
                    byte(&mut memory, inventory.at(slot).address(), -1);
                }
                byte(
                    &mut memory,
                    ram::g_player_info(revision).at(player).level().address(),
                    5 + player as i8,
                );
                byte(
                    &mut memory,
                    ram::g_player_health(revision).at(player).address(),
                    23,
                );
                byte(
                    &mut memory,
                    ram::g_player_lives(revision).at(player).address(),
                    3,
                );
            }
            byte(
                &mut memory,
                ram::g_player_inventory(revision).at(1).at(0).address(),
                23,
            );
            byte(
                &mut memory,
                ram::g_player_elevator_flags(revision).at(1).address(),
                1,
            );
            let mut bot = ContentBot::new(2);
            let mut pads = [tje_session::CONNECTED; 2];
            bot.choose(
                &Ram(&memory),
                &[],
                revision,
                &Content::new(),
                74000,
                &mut pads,
            );
            assert_eq!(pads[1], tje_session::CONNECTED | DOWN);
            assert_eq!(bot.arrival_exit[1], Some(40));
        }
    }

    #[test]
    fn late_prefixes_do_not_restart_the_life_transfer_setup() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut bot = ContentBot::new(2);
            let mut content = Content::new();
            content.milestones.insert("fixed_world".into(), Some(936));
            let mut pads = [tje_session::CONNECTED; 2];
            assert!(!bot.prepare_life_transfer(
                &Ram(&[0; 65536]),
                &[],
                revision,
                &content,
                74000,
                &mut pads,
            ));
            assert_eq!(bot.initial_life_setup, Some(false));
            assert_eq!(pads, [tje_session::CONNECTED; 2]);
        }
    }

    #[test]
    fn a_nearby_quiet_santa_precedes_opening_another_present() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                byte(&mut memory, inventory.at(slot).address(), -1);
            }
            byte(&mut memory, inventory.at(0).address(), 23);
            let santa = ram::g_earthling_banks(revision).at(0).at(0);
            byte(&mut memory, santa.kind().address(), 20);
            byte(&mut memory, santa.level().address(), 4);
            let mut bot = ContentBot::new(1);
            bot.levels[0] = 4;
            assert_eq!(
                bot.select_goal(&Ram(&memory), &[], revision, &Content::new(), 0, 40000),
                Some(Goal::Earthling { bank: 0, index: 0 })
            );
        }
    }

    #[test]
    fn a_previously_seen_randomizer_remains_a_low_health_recovery_option() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                byte(&mut memory, inventory.at(slot).address(), -1);
            }
            byte(&mut memory, inventory.at(0).address(), 18);
            byte(
                &mut memory,
                ram::g_player_health(revision).at(0).address(),
                5,
            );
            let mut content = Content::new();
            content.presents_opened.insert(18, Some(1));
            let mut bot = ContentBot::new(1);
            bot.levels[0] = 1;
            assert_eq!(
                bot.select_goal(&Ram(&memory), &[], revision, &content, 0, 40000),
                Some(Goal::Present {
                    kind: 18,
                    drop: false
                })
            );
        }
    }

    #[test]
    fn a_gift_witness_does_not_end_the_inventory_rank_and_earthling_farm() {
        let mut content = Content::new();
        content
            .friendly_interactions
            .insert("santa_gifts".into(), Some(1));
        for frame in content.presents_opened.values_mut() {
            *frame = Some(2);
        }
        for (_, frame) in content.earthlings_encountered.range_mut(1..23) {
            *frame = Some(3);
        }
        assert!(needs_gifts(&content));
        content.ranks_reached.insert(8);
        assert!(!needs_gifts(&content));
        content.presents_opened.insert(23, None);
        assert!(needs_gifts(&content));
        content.presents_opened.insert(23, Some(4));
        content.earthlings_encountered.insert(17, None);
        assert!(needs_gifts(&content));
    }

    #[test]
    fn togetherness_recovers_a_stranded_earthling_carrier_before_opening_it() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                byte(&mut memory, inventory.at(slot).address(), -1);
            }
            byte(&mut memory, inventory.at(0).address(), 23);
            byte(&mut memory, inventory.at(1).address(), 12);
            byte(&mut memory, ram::g_highest_level(revision).address(), 2);
            let mut bot = ContentBot::new(1);
            bot.levels[0] = 1;
            let content = Content::new();
            assert_eq!(
                bot.present_goal(&Ram(&memory), revision, &content, 0),
                Some(Goal::Present {
                    kind: 12,
                    drop: false,
                })
            );
            bot.levels[0] = 2;
            assert_eq!(
                bot.present_goal(&Ram(&memory), revision, &content, 0),
                Some(Goal::Present {
                    kind: 23,
                    drop: false,
                })
            );
            byte(
                &mut memory,
                ram::g_player1_terrain_underfoot(revision).address(),
                -128,
            );
            assert_eq!(
                bot.present_goal(&Ram(&memory), revision, &content, 0),
                Some(Goal::Present {
                    kind: 12,
                    drop: false,
                })
            );
        }
    }

    #[test]
    fn promotion_farming_collects_reroll_stock_before_consuming_unknown_gifts() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                byte(&mut memory, inventory.at(slot).address(), -1);
            }
            byte(&mut memory, inventory.at(0).address(), 3);
            byte(&mut memory, inventory.at(1).address(), 18);
            byte(
                &mut memory,
                ram::g_player_health(revision).at(0).address(),
                27,
            );
            byte(&mut memory, ram::g_player_rank(revision).at(0).address(), 1);
            let item = ram::g_object_table(revision).at(0).at(0);
            byte(&mut memory, item.kind().address(), 4);
            byte(&mut memory, item.level().address(), 2);
            let mut bot = ContentBot::new(1);
            bot.levels[0] = 2;
            let content = Content::new();
            assert_eq!(bot.present_goal(&Ram(&memory), revision, &content, 0), None);
            byte(&mut memory, inventory.at(2).address(), 11);
            assert_eq!(
                bot.present_goal(&Ram(&memory), revision, &content, 0),
                Some(Goal::Present {
                    kind: 11,
                    drop: false,
                })
            );
        }
    }

    #[test]
    fn promotion_stock_reserves_one_randomizer_and_one_gift_per_remaining_rank() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                byte(&mut memory, inventory.at(slot).address(), -1);
            }
            byte(&mut memory, inventory.at(0).address(), 18);
            for slot in 1..5 {
                byte(&mut memory, inventory.at(slot).address(), 1);
            }
            byte(&mut memory, inventory.at(5).address(), 27);
            byte(&mut memory, ram::g_player_rank(revision).at(0).address(), 3);
            byte(
                &mut memory,
                ram::g_player_health(revision).at(0).address(),
                19,
            );
            assert!(!promotion_stock_ready(&Ram(&memory), revision, 0));
            byte(&mut memory, inventory.at(6).address(), 1);
            assert!(promotion_stock_ready(&Ram(&memory), revision, 0));
            let bot = ContentBot::new(1);
            assert_eq!(
                bot.present_goal(&Ram(&memory), revision, &Content::new(), 0),
                Some(Goal::Present {
                    kind: 18,
                    drop: false,
                })
            );
            byte(&mut memory, inventory.at(6).address(), -1);
            byte(&mut memory, ram::g_player_rank(revision).at(0).address(), 4);
            assert!(promotion_stock_ready(&Ram(&memory), revision, 0));
            byte(
                &mut memory,
                ram::g_player_health(revision).at(0).address(),
                11,
            );
            assert!(!promotion_stock_ready(&Ram(&memory), revision, 0));
        }
    }

    #[test]
    fn randomizer_prediction_preserves_slot_occupancy_and_never_changes_live_state() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            for player in 0..2 {
                let inventory = ram::g_player_inventory(revision).at(player);
                for slot in 0..16 {
                    byte(&mut memory, inventory.at(slot).address(), -1);
                }
                byte(&mut memory, inventory.at(0).address(), 27);
                byte(&mut memory, inventory.at(1).address(), 18);
                byte(&mut memory, inventory.at(2).address(), 1);
            }
            byte(&mut memory, ram::g_rng_state(revision).address() + 3, 1);
            let address = tje_mem::rom::present_weight_table(revision).address() as usize;
            let mut rom = vec![0; address + 27];
            // Synthetic single-outcome weights, not cartridge resource bytes.
            rom[address + 21] = 1;
            let predicted = predict_randomizer(&Ram(&memory), &rom, revision);
            for (player, items) in predicted.iter().enumerate() {
                assert_eq!(&items[..3], &[27, 21, 21]);
                assert!(items[3..].iter().all(|&kind| kind == -1));
                assert_eq!(
                    ram::g_player_inventory(revision)
                        .at(player as i32)
                        .at(1)
                        .read(&Ram(&memory)),
                    18
                );
            }
            assert_eq!(ram::g_rng_state(revision).read(&Ram(&memory)), 1);
        }
        let mut state = 1;
        assert_eq!(next_random(&mut state), 16807);
        assert_eq!(state, 16807);
        assert_eq!(next_random(&mut state), 15089);
        assert_eq!(state, 282475249);
    }

    #[test]
    fn seeing_a_whirlwind_does_not_replace_its_actual_capture_goal() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let earthling = ram::g_earthling_banks(revision).at(0).at(0);
            byte(&mut memory, earthling.kind().address(), 8);
            byte(&mut memory, earthling.level().address(), 7);
            let mut content = Content::new();
            content.earthlings_encountered.insert(8, Some(1));
            let mut bot = ContentBot::new(1);
            bot.levels[0] = 7;
            let goal = Goal::Earthling { bank: 0, index: 0 };
            assert!(!bot.goal_done(&Ram(&memory), revision, &content, 0, goal));
            assert_eq!(
                bot.select_goal(&Ram(&memory), &[], revision, &content, 0, 100),
                Some(goal)
            );
            bot.whirlwind_contact = true;
            assert!(bot.goal_done(&Ram(&memory), revision, &content, 0, goal));
        }
    }

    #[test]
    fn final_contact_completes_only_when_the_original_panel_flag_is_set() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let bot = ContentBot::new(1);
            let goal = Goal::FinalContact { point: (0, 0) };
            assert!(!bot.goal_done(&Ram(&memory), revision, &Content::new(), 0, goal));
            byte(
                &mut memory,
                ram::g_present_display_panel_drawn(revision).address(),
                1,
            );
            assert!(bot.goal_done(&Ram(&memory), revision, &Content::new(), 0, goal));
        }
    }

    #[test]
    fn final_promotion_does_not_require_another_randomizer() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            for player in 0..2 {
                for slot in 0..16 {
                    byte(
                        &mut memory,
                        ram::g_player_inventory(revision)
                            .at(player)
                            .at(slot)
                            .address(),
                        -1,
                    );
                }
            }
            byte(
                &mut memory,
                ram::g_player_inventory(revision).at(0).at(0).address(),
                18,
            );
            byte(&mut memory, ram::g_player_rank(revision).at(0).address(), 7);
            byte(
                &mut memory,
                ram::g_player_health(revision).at(0).address(),
                19,
            );
            byte(&mut memory, ram::g_rng_state(revision).address() + 3, 1);
            let address = tje_mem::rom::present_weight_table(revision).address() as usize;
            let mut rom = vec![0; address + 27];
            rom[address + 11] = 1;
            assert!(promotion_stock_ready(&Ram(&memory), revision, 0));
            assert!(randomizer_ready(
                &Ram(&memory),
                &rom,
                revision,
                &Content::new(),
                0,
                false
            ));
        }
    }

    #[test]
    fn an_unwitnessed_full_inventory_pickup_remains_a_real_goal() {
        let content = Content::new();
        assert_eq!(
            item_priority(3, 31, 4, 16, &[], Revision::Rev02, &content, true),
            Some(0)
        );
        assert_eq!(
            item_priority(3, 31, 4, 16, &[], Revision::Rev02, &content, false),
            None
        );
        assert_eq!(
            item_priority(-1, 31, 4, 16, &[], Revision::Rev02, &content, true),
            None
        );
    }

    #[test]
    fn stockpile_capture_keeps_real_gifts_and_waits_for_the_original_full_pickup() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let inventory = ram::g_player_inventory(revision).at(0);
            for slot in 0..16 {
                byte(&mut memory, inventory.at(slot).address(), -1);
            }
            for slot in 0..4 {
                byte(&mut memory, inventory.at(slot).address(), 27);
            }
            byte(&mut memory, inventory.at(4).address(), 3);
            byte(
                &mut memory,
                ram::g_player_health(revision).at(0).address(),
                23,
            );
            byte(
                &mut memory,
                ram::g_player_powerups(revision).at(0).address(),
                -1,
            );
            let mut bot = ContentBot::stockpile(1);
            let mut content = Content::new();
            assert_eq!(
                bot.present_goal(&Ram(&memory), revision, &content, 0),
                Some(Goal::Present {
                    kind: 27,
                    drop: false
                })
            );
            content.presents_opened.insert(7, Some(1));
            content.presents_opened.insert(27, Some(1));
            assert_eq!(bot.present_goal(&Ram(&memory), revision, &content, 0), None);
            assert!(!bot.complete(&Ram(&memory), revision, &content, 100));
            byte(&mut memory, inventory.at(5).address(), 0);
            bot.mobility_needed[0] = true;
            assert_eq!(
                bot.present_goal(&Ram(&memory), revision, &content, 0),
                Some(Goal::Present {
                    kind: 0,
                    drop: false
                })
            );
            bot.full_inventory_reaction = true;
            assert!(bot.complete(&Ram(&memory), revision, &content, 101));
        }
    }

    #[test]
    fn cupid_knockdown_tracks_definition_seven_not_the_hamster() {
        assert_eq!(EARTHLING_CUPID, 7);
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut memory = vec![0; 65536];
            let earthling = ram::g_earthling_banks(revision).at(0).at(0);
            byte(&mut memory, earthling.kind().address(), EARTHLING_CUPID);
            byte(&mut memory, earthling.level().address(), 3);
            byte(
                &mut memory,
                ram::g_player_powerups(revision).at(0).address(),
                1,
            );
            let mut bot = ContentBot::new(1);
            bot.levels[0] = 3;
            let mut content = Content::new();
            content.earthlings_encountered.insert(7, Some(1));
            content.earthlings_encountered.insert(2, Some(1));
            let goal = Goal::Earthling { bank: 0, index: 0 };
            assert!(!bot.goal_done(&Ram(&memory), revision, &content, 0, goal));
            assert_eq!(
                bot.select_goal(&Ram(&memory), &[], revision, &content, 0, 100),
                Some(goal)
            );
            byte(&mut memory, earthling.kind().address(), 2);
            assert!(bot.goal_done(&Ram(&memory), revision, &content, 0, goal));
        }
    }

    #[test]
    fn blocking_map_acknowledges_its_requesting_controller() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut bot = ContentBot::new(2);
            bot.scene = "map";
            bot.scene_player = 1;
            bot.scene_since = 1;
            bot.map_restored = true;
            let mut pads = [tje_session::CONNECTED; 2];
            bot.choose(
                &Ram(&[0; 65536]),
                &[],
                revision,
                &Content::new(),
                144,
                &mut pads,
            );
            assert_eq!(pads, [tje_session::CONNECTED, tje_session::CONNECTED | C]);
        }
    }
}
