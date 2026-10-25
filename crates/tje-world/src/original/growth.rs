use super::{OriginalRng, geometry::OriginalGeneration, resources::OriginalResources};
use crate::{CellPosition, Error, LevelId, Result};
use bincode::{Decode, Encode};

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub(super) struct PathWalk {
    row: i16,
    column: i16,
    target_row: i16,
    target_column: i16,
    random_steps: i16,
    turn_chance: i16,
    direction_steps: i16,
    opposite: i16,
    previous: i16,
    repeated: i16,
    steps: i16,
    direction: u8,
    next: u8,
    copy: bool,
}
impl PathWalk {
    pub fn new(
        start: CellPosition,
        target: CellPosition,
        random_steps: i16,
        turn_chance: i16,
        rng: &mut OriginalRng,
        level: LevelId,
    ) -> Self {
        let copy = (rng.bounded(100) as i32) < 10 + 3 * i32::from(level.get());
        let direction = 1_u8 << (rng.next_u16() & 3);
        Self {
            row: start.y as i16,
            column: start.x as i16,
            target_row: target.y as i16,
            target_column: target.x as i16,
            random_steps,
            turn_chance,
            direction_steps: 0,
            opposite: -1,
            previous: -1,
            repeated: 0,
            steps: 0,
            direction,
            next: 0,
            copy,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if !(0..26).contains(&self.row)
            || !(0..19).contains(&self.column)
            || !(0..26).contains(&self.target_row)
            || !(0..19).contains(&self.target_column)
            || ![1, 2, 4, 8].contains(&self.direction)
            || ![-1, 1, 2, 4, 8].contains(&self.opposite)
            || self.turn_chance < 0
            || self.turn_chance > 100
        {
            return Err(Error::State("OriginalV1 path continuation"));
        }
        Ok(())
    }
    pub fn step(
        &mut self,
        world: &mut OriginalGeneration,
        resources: &impl OriginalResources,
    ) -> Result<bool> {
        if self.row == self.target_row && self.column == self.target_column {
            return Ok(true);
        }
        if self.direction_steps <= 0 || self.turn_chance == 0 {
            self.direction_steps = world.rng.bounded(3) as i16 + 2;
            self.next = if i32::from(self.turn_chance) > world.rng.bounded(100) as i32 {
                resources.original_turn(self.direction, (world.rng.bounded(3) + 1) as u8)?
            } else if world.rng.bounded(100) < 50 {
                if self.target_row > self.row {
                    4
                } else if self.target_row != self.row || world.rng.bounded(100) < 50 {
                    1
                } else {
                    4
                }
            } else if self.target_column > self.column {
                2
            } else if self.target_column != self.column || world.rng.bounded(100) < 50 {
                8
            } else {
                2
            };
        }
        self.direction = if self.turn_chance == 0 || world.rng.bounded(100) < 80 {
            self.next
        } else {
            resources.original_turn(self.next, (world.rng.bounded(3) + 1) as u8)?
        };
        if self.previous == i16::from(self.direction) {
            if self.repeated >= self.direction_steps {
                self.direction =
                    resources.original_turn(self.direction, (world.rng.bounded(3) + 1) as u8)?;
            } else {
                self.repeated = self.repeated.wrapping_add(1);
            }
        } else {
            self.repeated = 0;
        }
        if i16::from(self.direction) != self.opposite
            && ((self.direction == 1 && self.row > 2)
                || (self.direction == 4 && self.row < 24)
                || (self.direction == 8 && self.column > 2)
                || (self.direction == 2 && self.column < 17))
        {
            let index = OriginalGeneration::index(self.row, self.column)?;
            world.words[index] |= u16::from(self.direction) | 0x2000;
            if self.copy {
                world.directions[index] |= self.direction;
            }
            match self.direction {
                1 => {
                    self.row -= 1;
                    self.opposite = 4;
                }
                4 => {
                    self.row += 1;
                    self.opposite = 1;
                }
                8 => {
                    self.column -= 1;
                    self.opposite = 2;
                }
                2 => {
                    self.column += 1;
                    self.opposite = 8;
                }
                _ => return Err(Error::Resource("OriginalV1 path direction")),
            }
            let index = OriginalGeneration::index(self.row, self.column)?;
            world.words[index] |= self.opposite as u16 | 0x2000;
            if self.copy {
                world.directions[index] |= self.opposite as u8;
            }
            self.previous = i16::from(self.direction);
            self.direction_steps = self.direction_steps.wrapping_sub(1);
            self.steps = self.steps.wrapping_add(1);
            if u32::from(world.spec.level.get()) > world.rng.bounded(100) {
                self.copy = true;
            }
        } else {
            self.direction_steps = 0;
        }
        if self.steps >= self.random_steps {
            self.turn_chance = 0;
        }
        Ok(self.row == self.target_row && self.column == self.target_column)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
enum GrowthPhase {
    Fill,
    Search { tried: u8 },
    Turn { tried: u8 },
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub(super) struct Grow {
    kind: u16,
    region: u16,
    minimum: u16,
    row: i16,
    column: i16,
    remaining: i16,
    direction: u8,
    preferred: u8,
    join: u16,
    stop: u16,
    phase: GrowthPhase,
}
impl Grow {
    pub fn new(
        kind: u16,
        number: u8,
        minimum: u16,
        position: CellPosition,
        world: &mut OriginalGeneration,
        resources: &impl OriginalResources,
    ) -> Result<Self> {
        let direction = 1_u8 << (world.rng.next_u16() & 3);
        let (stop, join) = if kind == 0x1000 {
            (
                world.parameter(resources, 0)?,
                world.parameter(resources, 2)?,
            )
        } else if kind == 0x3000 {
            (world.parameter(resources, 1)?, 0)
        } else {
            return Err(Error::Bounds("OriginalV1 growth class"));
        };
        Ok(Self {
            kind,
            region: (u16::from(number) << 8) | kind,
            minimum,
            row: position.y as i16,
            column: position.x as i16,
            remaining: 0,
            direction,
            preferred: 0,
            join,
            stop,
            phase: GrowthPhase::Fill,
        })
    }
    pub fn validate(&self) -> Result<()> {
        if !matches!(self.kind, 0x1000 | 0x3000)
            || !(0..25).contains(&self.row)
            || !(0..18).contains(&self.column)
            || ![1, 2, 4, 8].contains(&self.direction)
            || self.join > 100
            || self.stop > 100
            || self.minimum > 494
        {
            return Err(Error::State("OriginalV1 region continuation"));
        }
        Ok(())
    }
    pub fn step(
        &mut self,
        world: &mut OriginalGeneration,
        resources: &impl OriginalResources,
    ) -> Result<bool> {
        match self.phase {
            GrowthPhase::Fill => {
                if self.kind == 0x1000 {
                    let mut y = 0_i16;
                    while y < 2 {
                        let mut x = 0_i16;
                        while x < 2 {
                            let tile = world.get(self.row + y, self.column + x)?;
                            if tile & 0x3000 == 0x1000 && tile != self.region {
                                for word in &mut world.words {
                                    if *word == tile {
                                        *word = self.region;
                                    }
                                }
                                x = 2;
                                y = 2;
                            }
                            x = x.wrapping_add(1);
                        }
                        y = y.wrapping_add(1);
                    }
                }
                for y in self.row..self.row + 2 {
                    for x in self.column..self.column + 2 {
                        world.set(y, x, self.region)?;
                    }
                }
                if self.remaining <= 0 {
                    self.remaining = (world.rng.next_u16() & 1) as i16;
                    if self.kind == 0x3000 {
                        self.remaining += 1;
                    }
                    self.preferred = resources
                        .original_turn(self.direction, (world.rng.bounded(3) + 1) as u8)?;
                }
                if world.rng.bounded(100) < 80 {
                    self.direction = self.preferred;
                }
                self.phase = GrowthPhase::Search { tried: 0 };
            }
            GrowthPhase::Search { tried } => {
                let found = if self.kind == 0x1000 {
                    world.find_region_origin(
                        self.row,
                        self.column,
                        self.direction,
                        self.region,
                        self.join,
                    )?
                } else {
                    world.find_interior_origin(
                        self.row,
                        self.column,
                        self.direction,
                        self.region,
                    )?
                };
                if let Some((row, column)) = found {
                    self.row = row;
                    self.column = column;
                    self.remaining = self.remaining.wrapping_sub(1);
                    if u32::from(self.stop) > world.rng.bounded(100) {
                        let count = if self.minimum > 0 {
                            world
                                .words
                                .iter()
                                .filter(|word| **word & 0x3000 == self.kind)
                                .count()
                        } else {
                            0
                        };
                        if count >= usize::from(self.minimum) {
                            self.phase = GrowthPhase::Done;
                            return Ok(true);
                        }
                    }
                    self.phase = GrowthPhase::Fill;
                } else {
                    let tried = tried | self.direction;
                    if tried == 15 {
                        self.phase = GrowthPhase::Done;
                        return Ok(true);
                    }
                    self.phase = GrowthPhase::Turn { tried };
                }
            }
            GrowthPhase::Turn { tried } => {
                let next =
                    resources.original_turn(self.direction, (world.rng.bounded(3) + 1) as u8)?;
                if tried & next == 0 {
                    self.direction = next;
                    self.phase = GrowthPhase::Search { tried };
                }
            }
            GrowthPhase::Done => return Ok(true),
        }
        Ok(false)
    }
}
impl OriginalGeneration {
    fn advance_position(row: &mut i16, column: &mut i16, direction: u8) {
        match direction {
            1 => *row -= 1,
            4 => *row += 1,
            8 => *column -= 1,
            2 => *column += 1,
            _ => (),
        }
    }
    pub(super) fn find_region_origin(
        &mut self,
        mut row: i16,
        mut column: i16,
        direction: u8,
        region: u16,
        join: u16,
    ) -> Result<Option<(i16, i16)>> {
        for _ in 0..26 {
            Self::advance_position(&mut row, &mut column, direction);
            if row < 0
                || column < 0
                || row >= 25
                || column >= 18
                || ((row == 0 || row == 24) && direction & 10 != 0)
                || ((column == 0 || column == 17) && direction & 5 != 0)
            {
                return Ok(None);
            }
            let mut overlaps = 0;
            for dy in 0..2 {
                for dx in 0..2 {
                    let tile = self.get(row + dy, column + dx)?;
                    if tile & 0x3000 == 0x1000 && tile != region {
                        overlaps += 1;
                        if overlaps >= 2 && u32::from(join) > self.rng.bounded(100) {
                            return Ok(Some((row, column)));
                        }
                    }
                }
            }
            if overlaps != 0 {
                return Ok(None);
            }
            for dy in 0..2 {
                for dx in 0..2 {
                    if self.get(row + dy, column + dx)? != region {
                        return Ok(Some((row, column)));
                    }
                }
            }
        }
        Err(Error::State("OriginalV1 region search exceeded axis"))
    }
    pub(super) fn find_interior_origin(
        &self,
        mut row: i16,
        mut column: i16,
        direction: u8,
        region: u16,
    ) -> Result<Option<(i16, i16)>> {
        for _ in 0..26 {
            Self::advance_position(&mut row, &mut column, direction);
            if row < 1 || column < 1 || row >= 24 || column >= 17 {
                return Ok(None);
            }
            if self.anchors[..3].iter().any(|anchor| {
                anchor.x as i16 >= column
                    && anchor.x as i16 <= column + 1
                    && anchor.y as i16 >= row
                    && anchor.y as i16 <= row + 1
            }) {
                return Ok(None);
            }
            for dy in -1..3 {
                for dx in -1..3 {
                    let tile = self.get(row + dy, column + dx)?;
                    if tile != region && tile != ((region & 0x700) | 0x1000) {
                        return Ok(None);
                    }
                }
            }
            for dy in 0..2 {
                for dx in 0..2 {
                    if self.get(row + dy, column + dx)? != region {
                        return Ok(Some((row, column)));
                    }
                }
            }
        }
        Err(Error::State("OriginalV1 interior search exceeded axis"))
    }
    fn fill_merge(
        &mut self,
        row: i16,
        column: i16,
        height: i16,
        width: i16,
        replacement: u16,
    ) -> Result<()> {
        for y in row..row + height {
            for x in column..column + width {
                let tile = self.get(y, x)?;
                if tile & 0x3000 == 0x1000 && tile != replacement {
                    for word in &mut self.words {
                        if *word == tile {
                            *word = replacement;
                        }
                    }
                } else {
                    self.set(y, x, replacement)?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn expand_beside_path(&mut self, row: i16, column: i16) -> Result<bool> {
        let mut changed = false;
        for (bit, dy, dx, checks, sy, sx, height, width) in [
            (
                1,
                -1,
                0,
                [(-1, -1), (-1, 1), (-2, -1), (-2, 1)],
                -2,
                -1,
                2,
                3,
            ),
            (4, 1, 0, [(1, -1), (1, 1), (2, -1), (2, 1)], 1, -1, 2, 3),
            (
                8,
                0,
                -1,
                [(-1, -1), (1, -1), (-1, -2), (1, -2)],
                -1,
                -2,
                3,
                2,
            ),
            (2, 0, 1, [(-1, 1), (1, 1), (-1, 2), (1, 2)], -1, 1, 3, 2),
        ] {
            if self.get(row, column)? & bit != 0 {
                let terrain = self.get(row + dy, column + dx)?;
                if terrain & 0x3000 == 0x1000 {
                    let mut differs = false;
                    for (y, x) in checks {
                        if self.get(row + y, column + x)? != terrain {
                            differs = true;
                            break;
                        }
                    }
                    if differs {
                        self.fill_merge(row + sy, column + sx, height, width, terrain)?;
                        changed = true;
                    }
                }
            }
        }
        Ok(changed)
    }
    pub(super) fn widen_horizontal(&mut self) -> Result<()> {
        for pass in 0..2 {
            let rows = if pass == 0 { 0..25 } else { 1..26 };
            for row in rows {
                let mut length = 0_i16;
                let mut prior = 0xffff;
                for column in 0..19 {
                    let terrain = self.get(row, column)?;
                    let qualifies = terrain & 0x3000 == 0x1000
                        && (if pass == 0 {
                            row == 0 || self.get(row - 1, column)? & 0x3000 != 0x1000
                        } else {
                            row == 25 || self.get(row + 1, column)? & 0x3000 != 0x1000
                        });
                    if qualifies {
                        if terrain == prior {
                            length = length.wrapping_add(1);
                        } else {
                            prior = terrain;
                            length = 1;
                        }
                    } else {
                        length = 1;
                    }
                    if length >= 4 {
                        if (pass == 0 && row > 0) || (pass == 1 && row < 25) {
                            let target = if pass == 0 { row - 1 } else { row + 1 };
                            self.set(target, column - 2, terrain)?;
                            self.set(target, column - 1, terrain)?;
                            length = 1;
                        } else if length >= 5 {
                            self.set(row, column - 2, 0)?;
                            length = 2;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn widen_vertical(&mut self) -> Result<()> {
        for pass in 0..2 {
            let columns = if pass == 0 { 0..18 } else { 1..19 };
            for column in columns {
                let mut length = 0_i16;
                let mut prior = 0xffff;
                for row in 0..26 {
                    let terrain = self.get(row, column)?;
                    let qualifies = terrain & 0x3000 == 0x1000
                        && (if pass == 0 {
                            column == 0 || self.get(row, column - 1)? & 0x3000 != 0x1000
                        } else {
                            column == 18 || self.get(row, column + 1)? & 0x3000 != 0x1000
                        });
                    if qualifies {
                        if terrain == prior {
                            length = length.wrapping_add(1);
                        } else {
                            prior = terrain;
                            length = 1;
                        }
                    } else {
                        length = 1;
                    }
                    if length >= 4 {
                        if (pass == 0 && column > 0) || (pass == 1 && column < 18) {
                            let target = if pass == 0 { column - 1 } else { column + 1 };
                            self.set(row - 2, target, terrain)?;
                            self.set(row - 1, target, terrain)?;
                            length = 1;
                        } else if length >= 5 {
                            self.set(row - 2, column, 0)?;
                            length = 2;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
