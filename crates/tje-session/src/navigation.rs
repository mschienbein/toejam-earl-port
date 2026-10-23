//! Feedback navigation over live RAM maps and runtime cartridge terrain.
use crate::runner::Ram;
use std::{cmp::Reverse, collections::BinaryHeap};
use tje_mem::{Bus, Revision, ram};
use tje_session::{DOWN, LEFT, RIGHT, UP};

const WIDTH: usize = 336;
const HEIGHT: usize = 224;
const CELLS: usize = WIDTH * HEIGHT;

struct Cartridge<'a>(&'a [u8]);
impl Bus for Cartridge<'_> {
    fn read_u8(&self, address: u32) -> u8 {
        self.0[(address & 0xFFFFFF) as usize]
    }
    fn write_u8(&mut self, _address: u32, _value: u8) {
        panic!("read-only cartridge observation");
    }
}

fn terrain_from_cell(
    rom: &Cartridge<'_>,
    revision: Revision,
    cell: u16,
    row: u16,
    column: u16,
) -> u8 {
    let overlay = (cell & 0x1F00) >> 8;
    let mut tile = if overlay == 0 {
        0
    } else {
        tje_game::camera::get_overlay_metatile_tile(rom, revision, overlay, row, column) as u16
    };
    if tile == 0 {
        tile = tje_game::camera::get_level_metatile_tile(
            rom,
            revision,
            u16::from(cell as u8),
            row,
            column,
        );
    }
    tje_mem::rom::player_tile_property_lookup(revision)
        .at(i32::from(tile & 0x7FF))
        .read(rom)
        & 0xF0
}

#[derive(Clone, Copy)]
struct EdgeAlternatives {
    values: tje_mem::Address<u8>,
    count: u8,
}

/// First-cell edge replacement from MapConsumePathCellDirections with its
/// original initial traversal=-1. This observes every possible random choice,
/// without drawing randomness or modifying the game's map.
fn edge_alternatives(
    rom: &Cartridge<'_>,
    memory: &Ram<'_>,
    revision: Revision,
    bank: usize,
    cell: u16,
    position: (i32, i32),
) -> Option<EdgeAlternatives> {
    let (row, column) = position;
    let directions = |row, column| {
        (ram::g_map_path_direction_banks(revision)
            .at(row)
            .at(column)
            .read(memory)
            >> (bank * 4))
            & 15
    };
    if cell & 0x4000 == 0 || directions(row, column) != 0 {
        return None;
    }
    let shape = if directions(row - 1, column) & 4 != 0 {
        16
    } else if directions(row, column + 1) & 8 != 0 {
        17
    } else if directions(row + 1, column) & 1 != 0 {
        18
    } else if directions(row, column - 1) & 2 != 0 {
        19
    } else {
        return None;
    };
    let tables = tje_mem::rom::map_generation_lookup_tables(revision);
    let kind = if cell & 0x8000 != 0 {
        tables.edge_alternate()
    } else {
        tables.edge_plain()
    }
    .at(shape)
    .read(rom);
    let values = tje_mem::rom::tile_alternative_pointers(revision)
        .at(i32::from(kind))
        .read(rom)
        .target();
    Some(EdgeAlternatives {
        values,
        count: values.read(rom),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Travel {
    Land,
    Swim,
    Fall,
    Fly,
}

pub struct Navigator {
    level: i8,
    map: Vec<u16>,
    terrain: Vec<u8>,
    safe_footing: Vec<bool>,
    cost: Vec<u16>,
    dynamic_cost: Vec<u16>,
    parents: Vec<usize>,
    distance: Vec<u32>,
    frontier: BinaryHeap<Reverse<(u32, u32, usize)>>,
    path: Vec<usize>,
    path_index: usize,
    target: Option<((i16, i16), Travel)>,
    last_plan: u64,
    previous: (i16, i16),
    last_motion: u64,
    reachable: bool,
}
impl Navigator {
    pub fn new() -> Self {
        Self {
            level: -1,
            map: vec![0; 26 * 19],
            terrain: vec![0x90; CELLS],
            safe_footing: vec![true; CELLS],
            cost: vec![0; CELLS],
            dynamic_cost: vec![0; CELLS],
            parents: vec![usize::MAX; CELLS],
            distance: vec![u32::MAX; CELLS],
            frontier: BinaryHeap::new(),
            path: Vec::new(),
            path_index: 0,
            target: None,
            last_plan: 0,
            previous: (0, 0),
            last_motion: 0,
            reachable: false,
        }
    }

    pub fn steer(
        &mut self,
        memory: &Ram<'_>,
        rom: &[u8],
        revision: Revision,
        player: usize,
        target: (i16, i16),
        frame: u64,
    ) -> Option<u16> {
        let travel = match ram::g_player_powerups(revision)
            .at(player as i32)
            .read(memory)
        {
            0 => Travel::Fly,
            2 => Travel::Swim,
            _ => Travel::Land,
        };
        self.toward(memory, rom, revision, player, (target, travel), frame)
    }

    pub fn toward(
        &mut self,
        memory: &Ram<'_>,
        rom: &[u8],
        revision: Revision,
        player: usize,
        goal: ((i16, i16), Travel),
        frame: u64,
    ) -> Option<u16> {
        let view = ram::g_player_info(revision).at(player as i32);
        let position = (view.x().read(memory), view.y().read(memory));
        let level = view.level().read(memory);
        let bank = if player == 1
            && ram::g_player_lives(revision).at(0).read(memory) >= 0
            && ram::g_split_screen_enabled(revision).read(memory) != 0
        {
            ram::g_player2_level_map(revision)
        } else {
            ram::g_player1_level_map(revision)
        };
        let changed = self.level != level
            || (0..26).any(|row| {
                (0..19).any(|col| {
                    self.map[(row * 19 + col) as usize]
                        != bank.at(row).at(col).read(memory) & 0xDFFF
                })
            });
        if changed {
            self.level = level;
            for row in 0..26 {
                for col in 0..19 {
                    self.map[(row * 19 + col) as usize] =
                        bank.at(row).at(col).read(memory) & 0xDFFF;
                }
            }
            self.decode(
                rom,
                revision,
                memory,
                usize::from(bank.address() == ram::g_player2_level_map(revision).address()),
            );
            self.target = None;
        }
        if manhattan(position, self.previous) > 64 {
            self.target = None;
        }
        if position != self.previous {
            self.last_motion = frame;
            self.previous = position;
        }
        if self.target != Some(goal)
            || frame.saturating_sub(self.last_plan) > 120
            || frame.saturating_sub(self.last_motion) > 60
            || (self.path_index >= self.path.len() && self.reachable)
        {
            self.plan(memory, revision, player, position, goal);
            self.target = Some(goal);
            self.last_plan = frame;
            if frame.saturating_sub(self.last_motion) > 60 {
                self.last_motion = frame;
            }
        }
        if !self.reachable {
            return None;
        }
        while let Some(&cell) = self.path.get(self.path_index) {
            let point = cell_position(cell);
            if (position.0 - point.0).abs() <= 3 && (position.1 - point.1).abs() <= 3 {
                self.path_index += 1;
                continue;
            }
            let dx = point.0 - position.0;
            let dy = point.1 - position.1;
            return Some(heading(dx, dy));
        }
        let dx = goal.0.0 - position.0;
        let dy = goal.0.1 - position.1;
        if dx.abs() <= 3 && dy.abs() <= 3 {
            return Some(0);
        }
        Some(heading(dx, dy))
    }

    pub fn terrain_at(&self, position: (i16, i16)) -> u8 {
        position_cell(position).map_or(0x90, |cell| self.terrain[cell])
    }

    pub fn nearest_terrain(&self, position: (i16, i16), terrain: u8) -> Option<(i16, i16)> {
        self.terrain
            .iter()
            .enumerate()
            .filter(|(_, value)| **value == terrain)
            .min_by_key(|(cell, _)| manhattan(cell_position(*cell), position))
            .map(|(cell, _)| cell_position(cell))
    }

    pub fn reachable_land_near(
        &mut self,
        start: (i16, i16),
        desired: (i16, i16),
    ) -> Option<(i16, i16)> {
        self.reachable_land_choose(start, desired, |_, _| true)
    }

    pub fn reachable_land_in_range(
        &mut self,
        start: (i16, i16),
        desired: (i16, i16),
        exclusive_range: (i16, i16),
    ) -> Option<(i16, i16)> {
        // Enter the interaction rectangle from its nearest reachable boundary;
        // the sprite center can itself be inside a solid collision body.
        self.reachable_land_choose(start, start, |cell, _| {
            let point = cell_position(cell);
            (i32::from(point.0) - i32::from(desired.0)).abs() < i32::from(exclusive_range.0)
                && (i32::from(point.1) - i32::from(desired.1)).abs() < i32::from(exclusive_range.1)
        })
    }

    /// ProcessLand reveals marked roads only when a grounded player reaches
    /// a marked metatile's edge. Select a real reachable trigger, never a
    /// speculative road over the current water/void.
    pub fn marked_path_approach(
        &mut self,
        memory: &Ram<'_>,
        revision: Revision,
        player: usize,
        start: (i16, i16),
        desired: (i16, i16),
    ) -> Option<(i16, i16)> {
        if self.level <= 1 {
            return None;
        }
        let bank = if player == 1
            && ram::g_player_lives(revision).at(0).read(memory) >= 0
            && ram::g_split_screen_enabled(revision).read(memory) != 0
        {
            ram::g_player2_level_map(revision)
        } else {
            ram::g_player1_level_map(revision)
        };
        self.reachable_land_choose(start, desired, |cell, terrain| {
            let x = cell % WIDTH;
            let y = cell / WIDTH;
            (16..320).contains(&x)
                && (8..216).contains(&y)
                && bank
                    .at((y / 8 - 1) as i32)
                    .at((x / 16 - 1) as i32)
                    .read(memory)
                    & 0x4000
                    != 0
                && [cell - WIDTH, cell - 1, cell + 1, cell + WIDTH]
                    .into_iter()
                    .any(|neighbor| terrain[neighbor] >= 0x50)
        })
    }

    fn reachable_land_choose(
        &mut self,
        start: (i16, i16),
        desired: (i16, i16),
        accept: impl Fn(usize, &[u8]) -> bool,
    ) -> Option<(i16, i16)> {
        let start = position_cell(start)?;
        self.parents.fill(usize::MAX);
        self.frontier.clear();
        self.parents[start] = start;
        self.frontier.push(Reverse((0, 0, start)));
        let mut best = None;
        while let Some(Reverse((_, _, cell))) = self.frontier.pop() {
            if self.terrain[cell] <= 0x50
                && self.safe_footing[cell]
                && accept(cell, &self.terrain)
                && best.is_none_or(|old| {
                    manhattan(cell_position(cell), desired) < manhattan(cell_position(old), desired)
                })
            {
                best = Some(cell);
            }
            for next in neighbors(cell).into_iter().flatten() {
                if self.parents[next] == usize::MAX
                    && self.terrain[next] <= 0x50
                    && self.safe_footing[next]
                {
                    self.parents[next] = cell;
                    self.frontier.push(Reverse((0, 0, next)));
                }
            }
        }
        best.map(cell_position)
    }

    fn decode(&mut self, rom: &[u8], revision: Revision, memory: &Ram<'_>, bank: usize) {
        let image = Cartridge(rom);
        let future_edges: [Option<EdgeAlternatives>; 26 * 19] = std::array::from_fn(|index| {
            edge_alternatives(
                &image,
                memory,
                revision,
                bank,
                self.map[index],
                ((index / 19) as i32, (index % 19) as i32),
            )
        });
        self.safe_footing.fill(true);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let index = y * WIDTH + x;
                if !(16..320).contains(&x) || !(8..216).contains(&y) {
                    self.terrain[index] = if self.level == 1 || self.level == 26 {
                        0x80
                    } else {
                        0x90
                    };
                    continue;
                }
                let map = self.map[(y / 8 - 1) * 19 + x / 16 - 1];
                self.terrain[index] =
                    terrain_from_cell(&image, revision, map, (y % 8) as u16, (x % 16) as u16);
                if let Some(alternatives) = future_edges[(y / 8 - 1) * 19 + x / 16 - 1] {
                    self.safe_footing[index] = (1..=alternatives.count).all(|choice| {
                        let future = (map & 0x9F00)
                            | u16::from(alternatives.values.offset(i32::from(choice)).read(&image));
                        terrain_from_cell(&image, revision, future, (y % 8) as u16, (x % 16) as u16)
                            <= 0x50
                    });
                }
            }
        }
        for cell in 0..CELLS {
            let x = cell % WIDTH;
            let y = cell / WIDTH;
            let mut cost = 10;
            for dy in -2_i32..=2 {
                for dx in -2_i32..=2 {
                    let xx = x as i32 + dx;
                    let yy = y as i32 + dy;
                    if xx < 0
                        || yy < 0
                        || xx >= WIDTH as i32
                        || yy >= HEIGHT as i32
                        || self.terrain[yy as usize * WIDTH + xx as usize] > 0x50
                    {
                        cost += if dx.abs() <= 1 && dy.abs() <= 1 {
                            24
                        } else {
                            4
                        };
                    }
                }
            }
            self.cost[cell] = cost;
        }
    }

    fn plan(
        &mut self,
        memory: &Ram<'_>,
        revision: Revision,
        player: usize,
        start: (i16, i16),
        goal: ((i16, i16), Travel),
    ) {
        self.path.clear();
        self.path_index = 0;
        self.reachable = false;
        let Some(start_cell) = position_cell(start) else {
            return;
        };
        let Some(goal_cell) = position_cell(goal.0) else {
            return;
        };
        let heuristic = |cell: usize| {
            let point = cell_position(cell);
            (i32::from(point.0) - i32::from(goal.0.0))
                .unsigned_abs()
                .max((i32::from(point.1) - i32::from(goal.0.1)).unsigned_abs())
        };
        self.parents.fill(usize::MAX);
        self.distance.fill(u32::MAX);
        self.frontier.clear();
        self.parents[start_cell] = start_cell;
        self.distance[start_cell] = 0;
        self.frontier
            .push(Reverse((heuristic(start_cell), 0, start_cell)));
        let partner = ram::g_player_info(revision).at((player ^ 1) as i32);
        let partner_position = (partner.x().read(memory), partner.y().read(memory));
        let avoid_partner = ram::g_player_lives(revision)
            .at((player ^ 1) as i32)
            .read(memory)
            >= 0
            && partner.level().read(memory) == self.level
            && ram::g_player_elevator_flags(revision)
                .at(player as i32)
                .read(memory)
                == 0
            && ram::g_player_elevator_flags(revision)
                .at((player ^ 1) as i32)
                .read(memory)
                == 0;
        self.dynamic_cost.fill(0);
        if avoid_partner {
            self.obstacle(partner_position, (20, 10), 500);
        }
        for bank in 0..2 {
            for index in 0..20 {
                let earthling = ram::g_earthling_banks(revision).at(bank).at(index);
                let kind = earthling.kind().read(memory);
                if earthling.level().read(memory) == self.level
                    && matches!(kind, 1..=8 | 12..=16 | 18..=19 | 21..=22)
                {
                    self.obstacle(
                        (earthling.x().read(memory), earthling.y().read(memory)),
                        (48, 24),
                        150,
                    );
                }
            }
        }
        let query = ram::g_player_info(revision).at(player as i32);
        let doorway_handles: [i32; 2] = std::array::from_fn(|owner| {
            i32::from(
                ram::g_player_doorway_entities(revision)
                    .at(owner as i32)
                    .read(memory),
            )
        });
        // PresentFindDoorwayOwnerAtTile makes this exact 4-by-3 footprint
        // ground, even when the underlying level metatile is water or space.
        let doorway_ground: [Option<(i32, i32)>; 2] = doorway_handles.map(|handle| {
            if handle == 0 {
                return None;
            }
            let door = ram::g_player_info(revision).at(handle);
            (door.level().read(memory) == self.level
                && (door.sprite_state().read(memory) != 0
                    || query.y().read(memory) >= door.y().read(memory)))
            .then(|| {
                (
                    (i32::from(door.x().read(memory)) - 16) >> 3,
                    (i32::from(door.y().read(memory)) - 10) >> 3,
                )
            })
        });
        for index in 6..26 {
            let object = ram::g_player_info(revision).at(index);
            // A doorway is an interactive entrance, not an impassable wall:
            // centered approach input opens it before the player can cross.
            if doorway_handles.contains(&index) {
                continue;
            }
            if object.draw_flags().read(memory) & 1 != 0
                && object.behavior_flags().read(memory) & 1 != 0
                && object.level().read(memory) == self.level
                && (object.height().read(memory) - query.height().read(memory)).abs() <= 10
            {
                let radius =
                    i16::from(object.width().read(memory) / 2 + query.width().read(memory) / 2)
                        .saturating_sub(6);
                // The original only rejects a *new* narrow collision. An
                // already-overlapping entity (a decoy or arriving elevator)
                // must not imprison the player inside an artificial route wall.
                if (query.x().read(memory) - object.x().read(memory)).abs() < radius
                    && (query.y().read(memory) - object.y().read(memory)).abs() < 12
                {
                    continue;
                }
                self.obstacle(
                    (object.x().read(memory), object.y().read(memory)),
                    (radius, 10),
                    u16::MAX,
                );
            }
        }
        for bank in 0..2 {
            for kind in 0..2 {
                let elevator = ram::g_elevator_records(revision).at(bank).at(kind);
                if elevator.level().read(memory) == self.level
                    && !matches!(elevator.state().read(memory), 11 | 13)
                {
                    let position = (elevator.x().read(memory), elevator.y().read(memory) + 5);
                    if (query.x().read(memory) - position.0).abs() >= 24
                        || (query.y().read(memory) - position.1).abs() >= 10
                    {
                        self.obstacle(position, (24, 10), u16::MAX);
                    }
                }
            }
        }
        let mut end = None;
        while let Some(Reverse((_, cost, cell))) = self.frontier.pop() {
            if cost != self.distance[cell] {
                continue;
            }
            if cell == goal_cell {
                end = Some(cell);
                break;
            }
            for next in neighbors(cell).into_iter().flatten() {
                let column = (next % WIDTH) as i32;
                let row = (next / WIDTH) as i32;
                let doorway = doorway_ground.iter().flatten().any(|&(left, top)| {
                    column >= left && column < left + 4 && row >= top && row < top + 3
                });
                let terrain = if doorway { 0x10 } else { self.terrain[next] };
                let permitted = goal.1 == Travel::Fly
                    || (terrain <= 0x50
                        && (doorway || self.safe_footing[next] || goal.1 == Travel::Fall))
                    || (goal.1 == Travel::Swim && terrain == 0x80)
                    || (goal.1 == Travel::Fall
                        && (next == goal_cell
                            || cell == goal_cell
                            || (self.level == 1 && terrain == 0x80)));
                let point = cell_position(next);
                if !permitted || point.0 < 32 || point.0 > 2648 || point.1 < 52 || point.1 > 1732 {
                    continue;
                }
                if goal.1 != Travel::Fly && self.dynamic_cost[next] == u16::MAX && next != goal_cell
                {
                    continue;
                }
                let mut penalty = if goal.1 == Travel::Fly {
                    10
                } else {
                    u32::from(self.cost[next]) + u32::from(self.dynamic_cost[next])
                };
                if terrain == 0x80 {
                    penalty += 16;
                }
                let next_cost = cost + penalty;
                if next_cost < self.distance[next] {
                    self.distance[next] = next_cost;
                    self.parents[next] = cell;
                    self.frontier
                        .push(Reverse((next_cost + heuristic(next), next_cost, next)));
                }
            }
        }
        let Some(mut cell) = end else {
            return;
        };
        while cell != start_cell {
            self.path.push(cell);
            cell = self.parents[cell];
        }
        self.path.reverse();
        self.reachable = true;
    }

    fn obstacle(&mut self, position: (i16, i16), radius: (i16, i16), cost: u16) {
        let left = (i32::from(position.0 - radius.0) / 8).clamp(0, WIDTH as i32 - 1) as usize;
        let right = (i32::from(position.0 + radius.0) / 8).clamp(0, WIDTH as i32 - 1) as usize;
        let top = (i32::from(position.1 - radius.1) / 8).clamp(0, HEIGHT as i32 - 1) as usize;
        let bottom = (i32::from(position.1 + radius.1) / 8).clamp(0, HEIGHT as i32 - 1) as usize;
        for y in top..=bottom {
            for x in left..=right {
                self.dynamic_cost[y * WIDTH + x] =
                    self.dynamic_cost[y * WIDTH + x].saturating_add(cost);
            }
        }
    }
}
fn neighbors(cell: usize) -> [Option<usize>; 8] {
    let x = cell % WIDTH;
    let y = cell / WIDTH;
    [
        (y > 0).then(|| cell - WIDTH),
        (y + 1 < HEIGHT).then_some(cell + WIDTH),
        (x > 0).then(|| cell - 1),
        (x + 1 < WIDTH).then_some(cell + 1),
        (y > 0 && x > 0).then(|| cell - WIDTH - 1),
        (y > 0 && x + 1 < WIDTH).then(|| cell - WIDTH + 1),
        (y + 1 < HEIGHT && x > 0).then(|| cell + WIDTH - 1),
        (y + 1 < HEIGHT && x + 1 < WIDTH).then_some(cell + WIDTH + 1),
    ]
}

fn heading(dx: i16, dy: i16) -> u16 {
    let horizontal = if dx.abs() <= 3 {
        0
    } else if dx < 0 {
        LEFT
    } else {
        RIGHT
    };
    let vertical = if dy.abs() <= 3 {
        0
    } else if dy < 0 {
        UP
    } else {
        DOWN
    };
    horizontal | vertical
}
fn position_cell(position: (i16, i16)) -> Option<usize> {
    if position.0 < 0 || position.1 < 0 {
        return None;
    }
    let x = position.0 as usize / 8;
    let y = position.1 as usize / 8;
    (x < WIDTH && y < HEIGHT).then_some(y * WIDTH + x)
}
fn cell_position(cell: usize) -> (i16, i16) {
    (
        ((cell % WIDTH) * 8 + 4) as i16,
        ((cell / WIDTH) * 8 + 4) as i16,
    )
}
fn manhattan(a: (i16, i16), b: (i16, i16)) -> i32 {
    (i32::from(a.0) - i32::from(b.0)).abs() + (i32::from(a.1) - i32::from(b.1)).abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagonal_land_contacts_are_valid_original_pad_routes() {
        let revision = Revision::Rev02;
        let memory = tje_mem::AddressSpace::new(Vec::new(), revision);
        let mut navigator = Navigator::new();
        navigator.level = 1;
        let start = 10 * WIDTH + 20;
        let goal = 11 * WIDTH + 21;
        navigator.terrain[start] = 0x10;
        navigator.terrain[goal] = 0x10;
        navigator.plan(
            &Ram(memory.ram()),
            revision,
            0,
            cell_position(start),
            (cell_position(goal), Travel::Land),
        );
        assert!(navigator.reachable);
        assert_eq!(heading(8, 8), RIGHT | DOWN);
        assert_eq!(neighbors(0).into_iter().flatten().count(), 3);
        assert_eq!(neighbors(CELLS - 1).into_iter().flatten().count(), 3);
    }

    #[test]
    fn current_overlap_does_not_create_a_fake_collision_prison() {
        let revision = Revision::Rev02;
        let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
        let start = (164, 84);
        let player = ram::g_player_info(revision).at(0);
        player.level().write(&mut memory, 1);
        player.x().write(&mut memory, start.0);
        player.y().write(&mut memory, start.1);
        player.width().write(&mut memory, 24);
        let obstacle = ram::g_player_info(revision).at(6);
        obstacle.level().write(&mut memory, 1);
        obstacle.x().write(&mut memory, start.0);
        obstacle.y().write(&mut memory, start.1);
        obstacle.width().write(&mut memory, 40);
        obstacle.draw_flags().write(&mut memory, 1);
        obstacle.behavior_flags().write(&mut memory, 1);
        let mut navigator = Navigator::new();
        navigator.level = 1;
        for x in 20..=26 {
            navigator.terrain[10 * WIDTH + x] = 0x10;
        }
        navigator.plan(
            &Ram(memory.ram()),
            revision,
            0,
            start,
            ((212, 84), Travel::Land),
        );
        assert!(navigator.reachable);
    }

    #[test]
    fn land_failure_does_not_silently_become_a_drowning_route() {
        let revision = Revision::Rev02;
        let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
        let player = ram::g_player_info(revision).at(0);
        player.level().write(&mut memory, 1);
        player.x().write(&mut memory, 164);
        player.y().write(&mut memory, 84);
        let mut navigator = Navigator::new();
        navigator.level = 1;
        navigator.terrain[10 * WIDTH + 20] = 0x10;
        navigator.terrain[10 * WIDTH + 21] = 0x80;
        navigator.terrain[10 * WIDTH + 22] = 0x10;
        navigator.terrain[10 * WIDTH + 23] = 0x10;
        let memory = Ram(memory.ram());
        assert!(
            navigator
                .toward(&memory, &[], revision, 0, ((188, 84), Travel::Land), 10)
                .is_none()
        );
        assert!(
            navigator
                .toward(&memory, &[], revision, 0, ((188, 84), Travel::Swim), 11)
                .is_some()
        );
    }

    #[test]
    fn selector_sixteen_uses_the_original_preceding_overlay_record() {
        for revision in Revision::ALL {
            let mut bytes = vec![0; revision.rom_size()];
            let base = tje_mem::rom::level_metatile_maps(revision).address() as usize;
            bytes[base..base + 2].copy_from_slice(&3_u16.to_be_bytes());
            let preceding =
                (tje_mem::rom::overlay_metatile_maps(revision).address() - 256 + 30) as usize;
            bytes[preceding..preceding + 2].copy_from_slice(&5_u16.to_be_bytes());
            let properties = tje_mem::rom::player_tile_property_lookup(revision).address() as usize;
            bytes[properties + 3] = 0x10;
            bytes[properties + 5] = 0x30;
            assert_eq!(
                terrain_from_cell(&Cartridge(&bytes), revision, 0, 0, 0),
                0x10
            );
            assert_eq!(
                terrain_from_cell(&Cartridge(&bytes), revision, 0x1000, 0, 0),
                0x30
            );
        }
    }

    #[test]
    fn a_nearby_water_goal_is_not_reached_by_walking_blindly_from_land() {
        let revision = Revision::Rev02;
        let memory = tje_mem::AddressSpace::new(Vec::new(), revision);
        let mut navigator = Navigator::new();
        navigator.level = 1;
        let start = 10 * WIDTH + 20;
        navigator.terrain[start] = 0x10;
        navigator.terrain[start + 1] = 0x80;
        navigator.plan(
            &Ram(memory.ram()),
            revision,
            0,
            cell_position(start),
            (cell_position(start + 1), Travel::Land),
        );
        assert!(!navigator.reachable);
    }

    #[test]
    fn a_real_doorway_adds_its_conditional_ground_footprint() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let player = ram::g_player_info(revision).at(0);
            player.level().write(&mut memory, 1);
            player.x().write(&mut memory, 164);
            player.y().write(&mut memory, 92);
            player.width().write(&mut memory, 24);
            let door = ram::g_player_info(revision).at(6);
            door.level().write(&mut memory, 1);
            door.x().write(&mut memory, 188);
            door.y().write(&mut memory, 82);
            door.width().write(&mut memory, 24);
            door.draw_flags().write(&mut memory, 1);
            door.behavior_flags().write(&mut memory, 1);
            ram::g_player_doorway_entities(revision)
                .at(0)
                .write(&mut memory, 6);
            let mut navigator = Navigator::new();
            navigator.level = 1;
            navigator.terrain[11 * WIDTH + 20] = 0x10;
            navigator.plan(
                &Ram(memory.ram()),
                revision,
                0,
                (164, 92),
                ((188, 92), Travel::Land),
            );
            assert!(navigator.reachable);
            player.y().write(&mut memory, 60);
            navigator.plan(
                &Ram(memory.ram()),
                revision,
                0,
                (164, 92),
                ((188, 92), Travel::Land),
            );
            assert!(!navigator.reachable);
            door.sprite_state().write(&mut memory, 1);
            navigator.plan(
                &Ram(memory.ram()),
                revision,
                0,
                (164, 92),
                ((188, 92), Travel::Land),
            );
            assert!(navigator.reachable);
        }
    }

    #[test]
    fn ship_contact_uses_real_reachable_land_instead_of_the_object_center() {
        let mut navigator = Navigator::new();
        for column in 20..=23 {
            navigator.terrain[10 * WIDTH + column] = 0x10;
        }
        assert_eq!(
            navigator.reachable_land_in_range((164, 84), (196, 84), (12, 8)),
            Some((188, 84))
        );
        assert_eq!(
            navigator.reachable_land_in_range((164, 84), (212, 84), (12, 8)),
            None
        );
        assert_eq!(
            navigator.reachable_land_in_range((164, 84), (196, 84), (40, 8)),
            Some((164, 84))
        );
        assert_eq!(
            navigator.reachable_land_near((164, 84), (196, 84)),
            Some((188, 84))
        );
    }

    #[test]
    fn only_a_real_active_innertube_allows_automatic_water_routes() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let player = ram::g_player_info(revision).at(0);
            player.level().write(&mut memory, 1);
            player.x().write(&mut memory, 164);
            player.y().write(&mut memory, 84);
            ram::g_player_powerups(revision)
                .at(0)
                .write(&mut memory, -1);
            let mut navigator = Navigator::new();
            navigator.level = 1;
            navigator.terrain[10 * WIDTH + 20] = 0x10;
            navigator.terrain[10 * WIDTH + 21] = 0x80;
            navigator.terrain[10 * WIDTH + 22] = 0x10;
            assert_eq!(
                navigator.steer(&Ram(memory.ram()), &[], revision, 0, (180, 84), 100),
                None
            );
            ram::g_player_powerups(revision).at(0).write(&mut memory, 2);
            assert!(
                navigator
                    .steer(&Ram(memory.ram()), &[], revision, 0, (180, 84), 101)
                    .is_some()
            );
        }
    }

    #[test]
    fn hidden_paths_require_a_real_reachable_marked_cell_and_adjacent_edge() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            let mut navigator = Navigator::new();
            navigator.level = 5;
            for row in 8..16 {
                for column in 16..32 {
                    navigator.terrain[row * WIDTH + column] = 0x10;
                }
            }
            assert_eq!(
                navigator.marked_path_approach(
                    &Ram(memory.ram()),
                    revision,
                    0,
                    (164, 84),
                    (164, 300)
                ),
                None
            );
            ram::g_player1_level_map(revision)
                .at(0)
                .at(0)
                .write(&mut memory, 0x4000);
            let target = navigator
                .marked_path_approach(&Ram(memory.ram()), revision, 0, (164, 84), (164, 300))
                .unwrap();
            assert_eq!(target.1, 124);
            assert_eq!(navigator.terrain[position_cell(target).unwrap()], 0x10);
            assert!(target.0 >= 128 && target.0 < 256);
            navigator.level = 1;
            assert_eq!(
                navigator.marked_path_approach(
                    &Ram(memory.ram()),
                    revision,
                    0,
                    (164, 84),
                    (164, 300)
                ),
                None
            );
        }
    }

    #[test]
    fn pending_edges_keep_real_terrain_but_reject_footing_lost_by_any_replacement() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            // A below-cell UP link supplies source edge shape 18.
            ram::g_map_path_direction_banks(revision)
                .at(1)
                .at(0)
                .write(&mut memory, 1);
            let mut bytes = vec![0; revision.rom_size()];
            let tables = tje_mem::rom::map_generation_lookup_tables(revision);
            bytes[tables.edge_plain().at(18).address() as usize] = 42;
            let pointer = tje_mem::rom::tile_alternative_pointers(revision)
                .at(42)
                .address() as usize;
            bytes[pointer..pointer + 4].copy_from_slice(&0x400_u32.to_be_bytes());
            bytes[0x400] = 2;
            bytes[0x401] = 5;
            bytes[0x402] = 6;
            let base = tje_mem::rom::level_metatile_maps(revision).address() as usize;
            for word in 0..128 {
                for (metatile, tile) in [(0, 1_u16), (5, 1), (6, 2)] {
                    let offset = base + metatile * 256 + word * 2;
                    bytes[offset..offset + 2].copy_from_slice(&tile.to_be_bytes());
                }
            }
            let properties = tje_mem::rom::player_tile_property_lookup(revision).address() as usize;
            bytes[properties + 1] = 0x10;
            bytes[properties + 2] = 0x90;
            let mut navigator = Navigator::new();
            navigator.level = 5;
            navigator.map[0] = 0x4000;
            navigator.decode(&bytes, revision, &Ram(memory.ram()), 0);
            let cell = position_cell((132, 68)).unwrap();
            assert_eq!(navigator.terrain_at((132, 68)), 0x10);
            assert!(!navigator.safe_footing[cell]);
            // The prediction consumes no game RNG or direction bits.
            assert_eq!(
                ram::g_map_path_direction_banks(revision)
                    .at(1)
                    .at(0)
                    .read(&memory),
                1
            );
            bytes[properties + 2] = 0x10;
            navigator.decode(&bytes, revision, &Ram(memory.ram()), 0);
            assert!(navigator.safe_footing[cell]);
        }
    }

    #[test]
    fn pending_edge_prediction_uses_the_active_direction_nibble() {
        for revision in Revision::ALL {
            let mut memory = tje_mem::AddressSpace::new(Vec::new(), revision);
            ram::g_map_path_direction_banks(revision)
                .at(1)
                .at(0)
                .write(&mut memory, 0x10);
            let image = vec![0; revision.rom_size()];
            assert!(
                edge_alternatives(
                    &Cartridge(&image),
                    &Ram(memory.ram()),
                    revision,
                    0,
                    0x4000,
                    (0, 0)
                )
                .is_none()
            );
            assert!(
                edge_alternatives(
                    &Cartridge(&image),
                    &Ram(memory.ram()),
                    revision,
                    1,
                    0x4000,
                    (0, 0)
                )
                .is_some()
            );
            ram::g_map_path_direction_banks(revision)
                .at(0)
                .at(0)
                .write(&mut memory, 0x10);
            assert!(
                edge_alternatives(
                    &Cartridge(&image),
                    &Ram(memory.ram()),
                    revision,
                    1,
                    0x4000,
                    (0, 0)
                )
                .is_none()
            );
        }
    }
}
