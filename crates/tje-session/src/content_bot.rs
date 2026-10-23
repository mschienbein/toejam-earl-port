//! Content-directed controller inputs over live game state, never RAM recipes.
use crate::{
    navigation::{Navigator, Travel},
    runner::{Content, Ram, entry_name},
};
use tje_host::session_trace::FrameObservations;
use tje_mem::{Revision, ram};
use tje_session::{A, B, C, DOWN, LEFT, RIGHT, UP};

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
    previous_health: [i8; 2],
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
            previous_health: [i8::MAX; 2],
            scene: "game",
            scene_player: 0,
            scene_since: 0,
            last_status: 0,
        }
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
                pads[self.scene_player] |= if self.scene == "map" { C } else { A };
            }
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
                self.inventory_input(memory, revision, content, player, frame, pad);
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
            if self.players == 2
                && partner_flag & 1 != 0
                && ram::g_player_info(revision)
                    .at((player ^ 1) as i32)
                    .level()
                    .read(memory)
                    == level
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
                        if matches!(kind, 9..=11 | 17) {
                            target = (x + if position.0 < x { -24 } else { 24 }, y + 12);
                            if (position.0 - x).abs() < 96 && (position.1 - y).abs() < 64 {
                                if (position.1 - target.1).abs() > 2 {
                                    *pad |= if position.1 < target.1 { DOWN } else { UP };
                                } else if (position.0 - x).abs() > 28 {
                                    *pad |= if position.0 < x { RIGHT } else { LEFT };
                                }
                                continue;
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
                    Goal::Survey { point, .. } => {
                        self.travel(memory, rom, revision, player, point, frame, pad);
                    }
                    Goal::Elevator { bank } => {
                        let elevator = ram::g_elevator_records(revision).at(bank).at(0);
                        let target = (elevator.x().read(memory), elevator.y().read(memory) + 22);
                        let dy = position.1 - elevator.y().read(memory);
                        if (position.0 - target.0).abs() < 8 && (0..40).contains(&dy) {
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
            eprintln!(
                "content frame={frame} players={stats:?} goals={:?} presents={} earthlings={}",
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

    pub fn complete(
        &self,
        _memory: &Ram<'_>,
        _revision: Revision,
        content: &Content,
        _frame: u64,
    ) -> bool {
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
            *pad |= input;
            if power == 7 && input & 15 != 0 {
                *pad |= A;
            }
            return;
        }
        self.mobility_needed[player] = true;
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
                    || (service_done(content, earthling.kind().read(memory))
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
        let health = ram::g_player_health(revision)
            .at(player as i32)
            .read(memory);
        let rank = ram::g_player_rank(revision).at(player as i32).read(memory);
        let count = (0..16)
            .filter(|&slot| inventory.at(slot).read(memory) >= 0)
            .count();
        let collecting = count < 12
            && !self.mobility_needed[player]
            && (0..16).any(|slot| inventory.at(slot).read(memory) == 18)
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
                if collecting && !matches!(kind, 0 | 9 | 10 | 14 | 17 | 20 | 21) {
                    return None;
                }
                let missing = !content
                    .presents_opened
                    .get(&(kind as u8))
                    .is_some_and(Option::is_some);
                let score = if health < 12 && matches!(kind, 9 | 10 | 14 | 17) {
                    0
                } else if kind == 11 && rank < 8 {
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
                } else if (kind == 18 && (missing || count >= 5))
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
        &self,
        memory: &Ram<'_>,
        rom: &[u8],
        revision: Revision,
        content: &Content,
        player: usize,
        frame: u64,
    ) -> Option<Goal> {
        let level = self.levels[player];
        let eligible = |goal| {
            !self.blocked.iter().any(|&(old_level, old_goal, until)| {
                old_level == level && old_goal == goal && until > frame
            })
        };
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
        if self.mobility_needed[player] {
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
        if self.mobility_needed[player]
            && ram::g_player_powerups(revision)
                .at(player as i32)
                .read(memory)
                != 0
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
                        kind: 0 | 9 | 10 | 14 | 17 | 20 | 21,
                        ..
                    }
                ))
        {
            return Some(goal);
        }
        let view = ram::g_player_info(revision).at(player as i32);
        let position = (view.x().read(memory), view.y().read(memory));
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
                    || (service_done(content, kind) && !(kind == 10 && health < 18 && bucks > 0))
                {
                    continue;
                }
                let can_pay = match kind {
                    9 => count < 16 && bucks >= 3,
                    10 => bucks >= 1,
                    11 => count > 0 && bucks >= 2,
                    17 => bucks >= 3,
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
                let priority = item_priority(kind, health, bucks, count, rom, revision, content);
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
        // Discovery points are awarded only for actually entering an uncovered
        // 320x256 region. These goals earn the remaining ranks by walking the
        // generated world, not by setting points or opening the map screen.
        if (12..26).contains(&level)
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
                    let mut point = None;
                    for dy in (-112..=112_i16).step_by(16) {
                        for dx in (-144..=144_i16).step_by(16) {
                            let candidate = (center.0 + dx, center.1 + dy);
                            let distance = (i32::from(position.0) - i32::from(candidate.0)).abs()
                                + (i32::from(position.1) - i32::from(candidate.1)).abs();
                            if self.navigation[player].terrain_at(candidate) <= 0x50
                                && point.is_none_or(|(old, _)| distance < old)
                            {
                                point = Some((distance, candidate));
                            }
                        }
                    }
                    if let Some((distance, point)) = point {
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

    fn inventory_input(
        &mut self,
        memory: &Ram<'_>,
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

fn seen(map: &std::collections::BTreeMap<String, Option<u64>>, name: &str) -> bool {
    map.get(name).is_some_and(Option::is_some)
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
) -> Option<i32> {
    if kind < 0 {
        return None;
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
    fn blocking_map_acknowledges_its_requesting_controller() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let mut bot = ContentBot::new(2);
            bot.scene = "map";
            bot.scene_player = 1;
            bot.scene_since = 1;
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
