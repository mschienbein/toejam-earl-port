//! The readable stock generation pipeline over owned records. OriginalV1 is
//! deliberately fixed at original dimensions; ExpandedV1 owns larger geometry.
use super::growth::{Grow, PathWalk};
use super::shapes::{ConnectionFrame, TraceFrame};
use super::{
    OriginalRevision, OriginalRng,
    resources::{OriginalResources, ShapeBank},
};
use crate::{
    Anchors, Cell, CellPosition, Dimensions, Error, LevelGeometry, LevelId, Result,
    loading::{CellSource, SourceCell, SpecialFloor},
};
use bincode::{Decode, Encode};

const AREA: usize = 19 * 26;
const MAX_ATTEMPT_WORK: u64 = 8_000_000;
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct OriginalSpec {
    pub revision: OriginalRevision,
    pub rom_sha1: [u8; 20],
    pub level: LevelId,
    pub seed: u16,
    pub ambient_rng: i32,
    pub directional_path_mode: bool,
}
impl OriginalSpec {
    pub fn validate(&self) -> Result<()> {
        self.level.validate()?;
        if self.level.get() > 26 {
            return Err(Error::Bounds("OriginalV1 campaign floor"));
        }
        Ok(())
    }
    pub fn identity(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"tje-world/original/1\0");
        hash.update(self.rom_sha1);
        hash.update([match self.revision {
            OriginalRevision::Rev00 => 0,
            OriginalRevision::Rev02 => 2,
        }]);
        hash.update(self.level.get().to_be_bytes());
        hash.update(self.seed.to_be_bytes());
        hash.update(self.ambient_rng.to_be_bytes());
        hash.update([u8::from(self.directional_path_mode)]);
        hash.finalize().into()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum OriginalPhase {
    Anchors,
    Clear,
    Connect,
    GrowLand,
    Stabilize,
    Connectivity,
    FilledRegions,
    EdgeShapes,
    FilledShapes,
    PathShapes,
    Features,
    Transitions,
    Alternatives,
    Special,
    Finalize,
    Complete,
    Rejected,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct OriginalGeometry {
    pub spec: OriginalSpec,
    pub accepted_seed: u16,
    pub accepted_attempt: u8,
    pub ambient_rng_after: i32,
    pub geometry: LevelGeometry,
    pub anchors: [CellPosition; 8],
    pub anchor_count: u8,
    pub flags: Vec<u8>,
    pub walkable_tiles: u32,
}
impl OriginalGeometry {
    pub fn raw_cell(&self, index: usize) -> Result<u16> {
        let cell = self
            .geometry
            .cells()
            .get(index)
            .ok_or(Error::Bounds("OriginalV1 final cell"))?;
        let flags = *self
            .flags
            .get(index)
            .ok_or(Error::Bounds("OriginalV1 final flags"))?;
        Ok(cell.word() | (u16::from(flags) << 13))
    }
    pub fn identity(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"tje-world/original-geometry/1\0");
        hash.update(self.spec.identity());
        hash.update(self.accepted_seed.to_be_bytes());
        hash.update([self.accepted_attempt, self.anchor_count]);
        hash.update(self.ambient_rng_after.to_be_bytes());
        for cell in self.geometry.cells() {
            hash.update(cell.word().to_be_bytes());
        }
        hash.update(self.geometry.path_state());
        hash.update(&self.flags);
        for anchor in &self.anchors {
            hash.update(anchor.x.to_be_bytes());
            hash.update(anchor.y.to_be_bytes());
        }
        hash.finalize().into()
    }
    pub fn ship_position(&self) -> Option<(i32, i32)> {
        self.geometry.anchors().ship_piece.map(|anchor| {
            (
                (i32::from(anchor.x) + 2) * 128,
                (i32::from(anchor.y) + 2) * 64 + 8,
            )
        })
    }
    pub fn validate(&self) -> Result<()> {
        self.spec.validate()?;
        self.geometry.validate()?;
        if self.geometry.dimensions() != Dimensions::ORIGINAL
            || self.flags.len() != AREA
            || self.flags.iter().any(|flags| *flags > 7)
            || self.accepted_attempt >= crate::MAX_ATTEMPTS
            || self.anchor_count > 8
            || self.walkable_tiles == 0
            || self.walkable_tiles > AREA as u32 * 128
            || (self.spec.level.ordinary() && !(3..=8).contains(&self.anchor_count))
        {
            return Err(Error::State("OriginalV1 final extent/attempt/anchors"));
        }
        for anchor in &self.anchors[..usize::from(self.anchor_count)] {
            anchor.index(Dimensions::ORIGINAL)?;
        }
        Ok(())
    }
    pub fn elevator_position(&self, arrival: bool) -> (i32, i32) {
        let anchor = if arrival {
            self.geometry.anchors().arrival
        } else {
            self.geometry.anchors().exit
        };
        (
            (i32::from(anchor.x) + 2) * 128 - 16,
            (i32::from(anchor.y) + 2) * 64 - 5,
        )
    }
}
impl CellSource for OriginalGeometry {
    fn cell(&self, index: usize) -> Result<SourceCell> {
        let word = self.raw_cell(index)?;
        Ok(SourceCell {
            cell: Cell::from_word(word & 0x1fff)?,
            directions: self.geometry.path_state()[index],
            marked: word & 0x4000 != 0,
            alternate: word & 0x8000 != 0,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct OriginalGeneration {
    pub(super) spec: OriginalSpec,
    pub(super) phase: OriginalPhase,
    pub(super) rng: OriginalRng,
    accepted_seed: u16,
    ambient: i32,
    attempt: u8,
    attempt_work: u64,
    total_work: u64,
    pub(super) words: Vec<u16>,
    pub(super) directions: Vec<u8>,
    pub(super) anchors: [CellPosition; 8],
    pub(super) anchor_count: u8,
    pub(super) cursor: u16,
    pub(super) pass: u8,
    pub(super) region: u8,
    pub(super) probability: u16,
    filled: u16,
    connect: u8,
    extra_anchor: u8,
    path: Option<PathWalk>,
    grow: Option<Grow>,
    changed: bool,
    pub(super) connection: Vec<ConnectionFrame>,
    pub(super) connection_depth: u16,
    pub(super) trace: Vec<TraceFrame>,
    pub(super) trace_depth: u16,
    pub(super) trace_visits: i16,
    pub(super) trace_side: u8,
    pub(super) output: Vec<Cell>,
    pub(super) flags: Vec<u8>,
    pub(super) walkable_tiles: u32,
}
impl OriginalGeneration {
    pub fn new(spec: OriginalSpec) -> Result<Self> {
        spec.validate()?;
        let special = !spec.level.ordinary();
        Ok(Self {
            rng: OriginalRng::new(u32::from(spec.seed)),
            accepted_seed: spec.seed,
            ambient: if spec.level.get() == 26 {
                spec.ambient_rng
            } else {
                OriginalRng::new(spec.ambient_rng as u32).state()
            },
            spec,
            phase: if special {
                OriginalPhase::Special
            } else {
                OriginalPhase::Anchors
            },
            attempt: 0,
            attempt_work: 0,
            total_work: 0,
            words: vec![0; AREA],
            directions: vec![0; AREA],
            anchors: [CellPosition::default(); 8],
            anchor_count: 0,
            cursor: 0,
            pass: 0,
            region: 0,
            probability: 0,
            filled: 0,
            connect: 0,
            extra_anchor: 0,
            path: None,
            grow: None,
            changed: false,
            connection: vec![ConnectionFrame::default(); AREA],
            connection_depth: 0,
            trace: vec![TraceFrame::default(); AREA],
            trace_depth: 0,
            trace_visits: 0,
            trace_side: 0,
            output: Vec::with_capacity(AREA),
            flags: Vec::with_capacity(AREA),
            walkable_tiles: 0,
        })
    }
    pub const fn phase(&self) -> OriginalPhase {
        self.phase
    }
    pub const fn accepted_seed(&self) -> u16 {
        self.accepted_seed
    }
    pub const fn total_work(&self) -> u64 {
        self.total_work
    }
    pub fn spec(&self) -> &OriginalSpec {
        &self.spec
    }
    pub(super) fn index(row: i16, column: i16) -> Result<usize> {
        let index = i32::from(row) * 19 + i32::from(column);
        if !(0..AREA as i32).contains(&index) {
            return Err(Error::Bounds(
                "OriginalV1 source access outside owned scratch",
            ));
        }
        Ok(index as usize)
    }
    pub(super) fn get(&self, row: i16, column: i16) -> Result<u16> {
        Ok(self.words[Self::index(row, column)?])
    }
    pub(super) fn set(&mut self, row: i16, column: i16, value: u16) -> Result<()> {
        let index = Self::index(row, column)?;
        self.words[index] = value;
        Ok(())
    }
    pub(super) fn parameter(
        &mut self,
        resources: &impl OriginalResources,
        index: u8,
    ) -> Result<u16> {
        let (minimum, maximum) = resources.parameter(self.spec.level, index)?;
        if minimum > maximum {
            return Err(Error::Resource("OriginalV1 generation parameter"));
        }
        Ok((self.rng.bounded(u32::from(maximum - minimum) + 1) + u32::from(minimum)) as u16)
    }
    fn switch(&mut self, phase: OriginalPhase) {
        self.phase = phase;
        self.cursor = 0;
        self.pass = 0;
    }
    fn retry(&mut self) {
        if self.attempt + 1 == crate::MAX_ATTEMPTS {
            self.phase = OriginalPhase::Rejected;
            return;
        }
        // ChangeLevel does not restore its scoped RNG on rejection. The outer
        // loader draws the replacement seed from precisely that retained state.
        self.accepted_seed = self.rng.next_u16();
        self.ambient = self.rng.state();
        self.rng = OriginalRng::new(u32::from(self.accepted_seed));
        self.attempt += 1;
        self.attempt_work = 0;
        self.anchors = [CellPosition::default(); 8];
        self.anchor_count = 0;
        self.path = None;
        self.grow = None;
        self.region = 0;
        self.connect = 0;
        self.extra_anchor = 0;
        self.switch(OriginalPhase::Anchors);
    }
    pub fn advance(
        &mut self,
        resources: &impl OriginalResources,
        work: u32,
    ) -> Result<OriginalPhase> {
        if resources.original_revision() != self.spec.revision || !(1..=1024).contains(&work) {
            return Err(Error::Bounds("OriginalV1 resource revision/work"));
        }
        for _ in 0..work {
            if matches!(
                self.phase,
                OriginalPhase::Complete | OriginalPhase::Rejected
            ) {
                break;
            }
            self.attempt_work += 1;
            self.total_work += 1;
            if self.attempt_work > MAX_ATTEMPT_WORK {
                return Err(Error::Capacity("OriginalV1 finite attempt work"));
            }
            self.step(resources)?;
        }
        Ok(self.phase)
    }
    fn step(&mut self, resources: &impl OriginalResources) -> Result<()> {
        match self.phase {
            OriginalPhase::Anchors => {
                if self.anchor_count == 0 {
                    let count = self.parameter(resources, 6)?;
                    if !(3..=8).contains(&count) {
                        return Err(Error::Resource("OriginalV1 anchor count"));
                    }
                    self.anchor_count = count as u8;
                    return Ok(());
                }
                let (min_x, max_x, min_y, max_y) = if self.spec.level.get() < 4 {
                    (5, 13, 8, 17)
                } else {
                    (2, 16, 2, 23)
                };
                let candidate = CellPosition {
                    x: (self.rng.bounded(max_x - min_x + 1) + min_x) as u16,
                    y: (self.rng.bounded(max_y - min_y + 1) + min_y) as u16,
                };
                if self.anchors[..usize::from(self.cursor)]
                    .iter()
                    .all(|prior| {
                        (i32::from(prior.x) - i32::from(candidate.x)).abs() >= 3
                            || (i32::from(prior.y) - i32::from(candidate.y)).abs() >= 5
                    })
                {
                    self.anchors[usize::from(self.cursor)] = candidate;
                    self.cursor += 1;
                }
                if self.cursor == u16::from(self.anchor_count) {
                    self.switch(OriginalPhase::Clear);
                }
            }
            OriginalPhase::Clear => {
                self.words[usize::from(self.cursor)] = 0;
                self.directions[usize::from(self.cursor)] = 0;
                self.cursor += 1;
                if usize::from(self.cursor) == AREA {
                    self.switch(OriginalPhase::Connect);
                }
            }
            OriginalPhase::Connect => {
                if let Some(mut path) = self.path.take() {
                    if path.step(self, resources)? {
                        self.path = None;
                    } else {
                        self.path = Some(path);
                    }
                    return Ok(());
                }
                if self.connect < 2 {
                    let a = self.anchors[usize::from(self.connect)];
                    let b = self.anchors[usize::from(self.connect) + 1];
                    self.path = Some(PathWalk::new(a, b, 20, 60, &mut self.rng, self.spec.level));
                    self.connect += 1;
                } else if self.extra_anchor < self.anchor_count {
                    let anchor = self.anchors[usize::from(self.extra_anchor)];
                    let mut tx = (self.rng.next_u16() & 3) as i16 + 4;
                    if i32::from(anchor.x) + i32::from(tx) < 17 && self.rng.bounded(100) < 50 {
                        tx += anchor.x as i16;
                    } else {
                        if i32::from(anchor.x) - i32::from(tx) <= 2 {
                            return Ok(());
                        }
                        tx = anchor.x as i16 - tx;
                    }
                    let mut ty = (self.rng.next_u16() & 3) as i16 + 5;
                    if i32::from(anchor.y) + i32::from(ty) < 24 && self.rng.bounded(100) < 50 {
                        ty += anchor.y as i16;
                    } else {
                        if i32::from(anchor.y) - i32::from(ty) <= 2 {
                            return Ok(());
                        }
                        ty = anchor.y as i16 - ty;
                    }
                    self.path = Some(PathWalk::new(
                        anchor,
                        CellPosition {
                            x: tx as u16,
                            y: ty as u16,
                        },
                        1,
                        0,
                        &mut self.rng,
                        self.spec.level,
                    ));
                    self.extra_anchor += 1;
                } else {
                    self.filled =
                        ((u32::from(self.parameter(resources, 3)?) * 19 * 26) as u16) / 100;
                    self.region = 0;
                    self.switch(OriginalPhase::GrowLand);
                }
            }
            OriginalPhase::GrowLand => {
                if let Some(mut grow) = self.grow.take() {
                    if !grow.step(self, resources)? {
                        self.grow = Some(grow);
                    } else {
                        self.region += 1;
                    }
                    return Ok(());
                }
                if self.region < self.anchor_count {
                    let anchor = self.anchors[usize::from(self.region)];
                    let minimum = (u32::from(self.filled) / u32::from(self.anchor_count))
                        * (u32::from(self.region) + 1);
                    self.grow = Some(Grow::new(
                        0x1000,
                        self.region,
                        minimum as u16,
                        anchor,
                        self,
                        resources,
                    )?);
                } else {
                    self.switch(OriginalPhase::Stabilize);
                }
            }
            OriginalPhase::Stabilize => {
                if self.pass == 0 {
                    self.widen_horizontal()?;
                    self.pass = 1;
                } else if self.pass == 1 {
                    self.widen_vertical()?;
                    self.pass = 2;
                    self.cursor = 0;
                    self.changed = false;
                } else {
                    let index = usize::from(self.cursor);
                    let row = (index / 19) as i16;
                    let column = (index % 19) as i16;
                    if self.words[index] & 0x3000 == 0x2000
                        && self.expand_beside_path(row, column)?
                    {
                        self.changed = true;
                    }
                    self.cursor += 1;
                    if usize::from(self.cursor) == AREA {
                        if self.changed {
                            self.cursor = 0;
                            self.changed = false;
                        } else {
                            self.start_connection();
                            self.switch(OriginalPhase::Connectivity);
                        }
                    }
                }
            }
            OriginalPhase::Connectivity => {
                if let Some(connected) = self.connection_step()? {
                    if connected {
                        self.switch(OriginalPhase::FilledRegions);
                    } else {
                        self.retry();
                    }
                }
            }
            OriginalPhase::FilledRegions => {
                if self.pass == 0 {
                    for word in &mut self.words {
                        *word &= 0xf7ff;
                    }
                    self.probability = self.parameter(resources, 4)?;
                    self.pass = 1;
                    self.cursor = 0;
                    return Ok(());
                }
                if let Some(mut grow) = self.grow.take() {
                    if !grow.step(self, resources)? {
                        self.grow = Some(grow);
                    }
                    return Ok(());
                }
                if self.cursor >= 23 * 16 {
                    self.region = 0;
                    self.switch(OriginalPhase::EdgeShapes);
                    return Ok(());
                }
                let row = (self.cursor / 16) as i16;
                let column = (self.cursor % 16) as i16;
                self.cursor += 1;
                let value = self.get(row, column)?;
                if value & 0x3000 == 0x1000 {
                    let replacement = (value & 0x700) | 0x3000;
                    let mut eligible = true;
                    for y in row..row + 4 {
                        for x in column..column + 4 {
                            let tile = self.get(y, x)?;
                            if (tile != value && tile != replacement)
                                || self.anchors[..3]
                                    .iter()
                                    .any(|anchor| anchor.x as i16 == x && anchor.y as i16 == y)
                            {
                                eligible = false;
                                break;
                            }
                        }
                        if !eligible {
                            break;
                        }
                    }
                    if eligible && u32::from(self.probability) > self.rng.bounded(100) {
                        self.grow = Some(Grow::new(
                            0x3000,
                            ((value >> 8) & 7) as u8,
                            0,
                            CellPosition {
                                x: (column + 1) as u16,
                                y: (row + 1) as u16,
                            },
                            self,
                            resources,
                        )?);
                    }
                }
            }
            OriginalPhase::EdgeShapes => {
                if self.region >= self.anchor_count {
                    self.switch(OriginalPhase::FilledShapes);
                    return Ok(());
                }
                let upper = (self.parameter(resources, 5)? as i32) > self.rng.bounded(100) as i32;
                for word in &mut self.words {
                    if matches!(*word & 0x3000, 0x1000 | 0x3000)
                        && ((*word >> 8) & 7) == u16::from(self.region)
                    {
                        *word |= 0x800;
                        if upper {
                            *word |= 0x8000;
                        }
                    } else {
                        *word &= 0xf7ff;
                    }
                }
                for row in 0..26 {
                    for column in 0..19 {
                        let value = self.get(row, column)?;
                        if value & 0x800 != 0 && value & 0x3000 == 0x1000 {
                            let mask = self.corners(row, column)?;
                            let tile = resources.original_shape(
                                if upper {
                                    ShapeBank::EdgeAlternate
                                } else {
                                    ShapeBank::EdgePlain
                                },
                                mask,
                            )?;
                            self.set(row, column, value | u16::from(tile))?;
                        }
                    }
                }
                self.region += 1;
            }
            OriginalPhase::FilledShapes => {
                self.build_filled_shapes(resources)?;
                self.switch(OriginalPhase::PathShapes);
            }
            OriginalPhase::PathShapes => {
                self.build_path_shapes(resources)?;
                self.switch(OriginalPhase::Features);
            }
            OriginalPhase::Features => {
                let row = (self.cursor / 19) as i16;
                let column = (self.cursor % 19) as i16;
                self.scatter_feature(row, column, resources)?;
                self.cursor += 1;
                if usize::from(self.cursor) == AREA {
                    self.switch(OriginalPhase::Transitions);
                }
            }
            OriginalPhase::Transitions => {
                self.transition_step(resources)?;
            }
            OriginalPhase::Alternatives => {
                let index = usize::from(self.cursor);
                let old = self.words[index];
                let shape = old as u8;
                let mut new = old & 0xc000;
                let replacement = if old & 0x1f00 == 0 {
                    resources.alternative(shape, u32::from(self.rng.next_u16()))?
                } else {
                    resources
                        .transition(
                            shape,
                            (((old & 0x1f00) >> 8) as u8).wrapping_sub(1),
                            u32::from(self.rng.next_u16()),
                        )?
                        .ok_or(Error::Resource(
                            "OriginalV1 transition shape has no descriptor member",
                        ))?
                };
                new |= replacement.word();
                if new & 0x4000 != 0 {
                    new |= 0x2000;
                }
                self.words[index] = new;
                self.cursor += 1;
                if usize::from(self.cursor) == AREA {
                    self.switch(OriginalPhase::Finalize);
                }
            }
            OriginalPhase::Special => {
                let floor = match self.spec.level.get() {
                    0 => SpecialFloor::Zero,
                    1 => SpecialFloor::One,
                    26 => SpecialFloor::Final,
                    _ => return Err(Error::Bounds("OriginalV1 authored special floor")),
                };
                self.words[usize::from(self.cursor)] =
                    resources.original_special_word(floor, usize::from(self.cursor))?;
                self.cursor += 1;
                if usize::from(self.cursor) == AREA {
                    self.switch(OriginalPhase::Finalize);
                }
            }
            OriginalPhase::Finalize => {
                let word = self.words[usize::from(self.cursor)];
                let cell = Cell::from_word(word & 0x1fff)?;
                let mut walkable = 0;
                for row in 0..8 {
                    for column in 0..16 {
                        let overlay = if cell.overlay() == 0 {
                            0
                        } else {
                            resources.overlay_tile(cell.overlay(), row, column)?
                        };
                        let tile = if overlay == 0 {
                            resources.base_tile(cell.base(), row, column)?
                        } else {
                            overlay
                        };
                        if matches!(resources.property(tile)?, 0x10 | 0x20) {
                            walkable += 1;
                        }
                    }
                }
                self.walkable_tiles += walkable;
                self.output.push(cell);
                self.flags.push((word >> 13) as u8);
                self.cursor += 1;
                if usize::from(self.cursor) == AREA {
                    self.phase = OriginalPhase::Complete;
                }
            }
            OriginalPhase::Complete | OriginalPhase::Rejected => (),
        }
        Ok(())
    }
    pub fn finish(self) -> Result<OriginalGeometry> {
        self.validate()?;
        if self.phase != OriginalPhase::Complete {
            return Err(Error::State("OriginalV1 generation is not complete"));
        }
        let (arrival, exit, ship, regions) = if self.spec.level.ordinary() {
            (
                self.anchors[1],
                self.anchors[0],
                Some(self.anchors[2]),
                self.anchors[3..usize::from(self.anchor_count)].to_vec(),
            )
        } else {
            let anchor = match self.spec.level.get() {
                1 => CellPosition { x: 9, y: 10 },
                26 => CellPosition { x: 2, y: 21 },
                _ => self
                    .output
                    .iter()
                    .position(|cell| matches!(cell.base() & 127, 30 | 33))
                    .and_then(|index| Dimensions::ORIGINAL.position(index))
                    .ok_or(Error::Resource(
                        "OriginalV1 floor0 has no full supported cell",
                    ))?,
            };
            (anchor, anchor, None, Vec::new())
        };
        let geometry = LevelGeometry::new(
            Dimensions::ORIGINAL,
            self.output,
            self.directions,
            Anchors {
                arrival,
                exit,
                ship_piece: ship,
                regions,
            },
        )?;
        let value = OriginalGeometry {
            spec: self.spec,
            accepted_seed: self.accepted_seed,
            accepted_attempt: self.attempt,
            ambient_rng_after: self.ambient,
            geometry,
            anchors: self.anchors,
            anchor_count: self.anchor_count,
            flags: self.flags,
            walkable_tiles: self.walkable_tiles,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        self.spec.validate()?;
        if self.words.len() != AREA
            || self.directions.len() != AREA
            || self.connection.len() != AREA
            || self.trace.len() != AREA
            || self.attempt >= crate::MAX_ATTEMPTS
            || self.anchor_count > 8
            || self.output.len() > AREA
            || self.flags.len() != self.output.len()
            || usize::from(self.connection_depth) > AREA
            || usize::from(self.trace_depth) > AREA
            || self.attempt_work > MAX_ATTEMPT_WORK + 1
            || self.total_work < self.attempt_work
            || self.walkable_tiles > self.output.len() as u32 * 128
            || (self.phase == OriginalPhase::Complete && self.output.len() != AREA)
            || self.directions.iter().any(|direction| *direction > 15)
            || usize::from(self.cursor) > AREA
            || self.flags.iter().any(|flags| *flags > 7)
            || (self.phase == OriginalPhase::Anchors && self.cursor > u16::from(self.anchor_count))
            || (matches!(
                self.phase,
                OriginalPhase::Clear
                    | OriginalPhase::Features
                    | OriginalPhase::Alternatives
                    | OriginalPhase::Special
                    | OriginalPhase::Finalize
            ) && usize::from(self.cursor) >= AREA)
            || (self.phase == OriginalPhase::Finalize
                && self.output.len() != usize::from(self.cursor))
            || (!matches!(
                self.phase,
                OriginalPhase::Finalize | OriginalPhase::Complete
            ) && !self.output.is_empty())
            || (self.spec.level.ordinary()
                && self.phase != OriginalPhase::Anchors
                && self.anchor_count < 3)
        {
            return Err(Error::State(
                "OriginalV1 generation continuation extent/work",
            ));
        }
        for cell in &self.output {
            cell.validate()?;
        }
        for anchor in &self.anchors[..usize::from(self.anchor_count)] {
            anchor.index(Dimensions::ORIGINAL)?;
        }
        for frame in &self.connection[..usize::from(self.connection_depth)] {
            frame.validate()?;
        }
        for frame in &self.trace[..usize::from(self.trace_depth)] {
            frame.validate()?;
        }
        if let Some(path) = &self.path {
            path.validate()?;
        }
        if let Some(grow) = &self.grow {
            grow.validate()?;
        }
        Ok(())
    }
}
