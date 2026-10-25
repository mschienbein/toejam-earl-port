//! ExpandedV1: separated anchors -> bounded paths -> iterative region growth ->
//! ROM shapes/alternatives/transitions -> exact tile connectivity -> placement.
//! A tick is one candidate/cell, never an elapsed-time budget or an original call.
use crate::geometry::{TileMask, walkable_mask};
use crate::population::{PlacementBuilder, Population, PopulationBaseline, PopulationBudget};
use crate::random::StageRng;
use crate::{
    Anchors, Cell, CellPosition, Dimensions, Error, LevelGeometry, LevelId, MAX_ATTEMPTS,
    Resources, Result, Shape, WorldConfig,
};
use bincode::{Decode, Encode};
use sha2::{Digest, Sha256};

pub const MAX_WORK_PER_ADVANCE: u32 = 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Phase {
    Anchors,
    Paths,
    Regions,
    Features,
    Shapes,
    Alternatives,
    Terrain,
    Flood,
    Validate,
    Population,
    Complete,
    Rejected,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Rejection {
    AnchorCapacity,
    RegionGrowth,
    RequiredTerrainRoute,
    PopulationCapacity,
}
impl Rejection {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::AnchorCapacity => "separated anchor capacity",
            Self::RegionGrowth => "region growth work limit",
            Self::RequiredTerrainRoute => "required footprint/tile route is not walkable",
            Self::PopulationCapacity => "terrain/spacing population capacity",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct GenerationSpec {
    pub config: WorldConfig,
    pub level: LevelId,
    pub baseline: PopulationBaseline,
}
impl GenerationSpec {
    pub fn validate(&self) -> Result<()> {
        self.config.validate()?;
        self.level.validate()?;
        self.baseline.validate()?;
        if !self.level.ordinary() {
            return Err(Error::Bounds(
                "ExpandedV1 only owns ordinary campaign floors 2..=25",
            ));
        }
        Ok(())
    }
    pub fn identity(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"tje-world/generation/1\0");
        hash.update(self.config.identity());
        hash.update(self.level.get().to_be_bytes());
        hash.update(self.baseline.walkable_tiles.to_be_bytes());
        hash.update(self.baseline.ordinary_items.to_be_bytes());
        hash.update(self.baseline.earthlings.to_be_bytes());
        hash.update([
            self.baseline.reserved_items,
            u8::from(self.baseline.ship_piece),
            u8::from(self.baseline.mailbox),
        ]);
        hash.finalize().into()
    }
    /// Candidate geometry does not change merely because item/actor density or
    /// the measured population baseline changes. Accepted attempts may differ if
    /// a requested population genuinely cannot fit a candidate.
    pub fn geometry_identity(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"tje-world/geometry/1\0");
        hash.update(crate::GENERATOR_VERSION.to_be_bytes());
        hash.update(self.config.root_seed.to_be_bytes());
        hash.update(self.config.rom_sha1);
        let dimensions = self.config.dimensions_for(self.level);
        hash.update(dimensions.width().to_be_bytes());
        hash.update(dimensions.height().to_be_bytes());
        hash.update(self.level.get().to_be_bytes());
        hash.finalize().into()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct GeneratedLevel {
    pub identity: [u8; 32],
    pub accepted_attempt: u8,
    pub geometry: LevelGeometry,
    pub population: Population,
    pub runtime_rng: StageRng,
    pub walkable_tiles: u32,
}
impl GeneratedLevel {
    pub fn validate(&self) -> Result<()> {
        if self.accepted_attempt >= MAX_ATTEMPTS
            || !self.runtime_rng.valid()
            || self.walkable_tiles == 0
            || self.walkable_tiles > self.geometry.dimensions().area() as u32 * 128
        {
            return Err(Error::State("generated identity/attempt/terrain count"));
        }
        self.geometry.validate()?;
        self.population.validate(self.geometry.dimensions())
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
enum RegionKind {
    #[default]
    Empty,
    Path,
    Land,
    Filled,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
struct RegionCell {
    kind: RegionKind,
    label: u8,
    directions: u8,
    alternate: bool,
}

/// Complete serializable loading continuation. Working geometry is not a live
/// level. Failed/exhausted work can be discarded without changing playable data.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Generation {
    spec: GenerationSpec,
    identity: [u8; 32],
    dimensions: Dimensions,
    attempt: u8,
    phase: Phase,
    last_rejection: Option<Rejection>,
    total_work: u64,
    rng: StageRng,
    anchors: Vec<CellPosition>,
    anchor_count: u8,
    anchor_tries: u32,
    route: u8,
    route_position: CellPosition,
    regions: Vec<RegionCell>,
    frontier: Vec<u16>,
    grown: u32,
    target_land: u32,
    growth_work: u32,
    feature_chance: u8,
    cursor: u32,
    shapes: Vec<u8>,
    cells: Vec<Cell>,
    paths: Vec<u8>,
    walkable: Vec<TileMask>,
    reached: Vec<TileMask>,
    queue: Vec<u16>,
    queued: Vec<bool>,
    queue_head: u32,
    queue_len: u32,
    flood_work: u32,
    walkable_tiles: u32,
    placements: Option<PlacementBuilder>,
}
impl Generation {
    pub fn new(spec: GenerationSpec) -> Result<Self> {
        spec.validate()?;
        let identity = spec.identity();
        let dimensions = spec.config.dimensions_for(spec.level);
        let area = dimensions.area();
        Ok(Self {
            rng: StageRng::derive(&spec.geometry_identity(), spec.level, 0, b"anchors"),
            spec,
            identity,
            dimensions,
            attempt: 0,
            phase: Phase::Anchors,
            last_rejection: None,
            total_work: 0,
            anchors: Vec::with_capacity(8),
            anchor_count: 0,
            anchor_tries: 0,
            route: 0,
            route_position: CellPosition::default(),
            regions: vec![RegionCell::default(); area],
            frontier: Vec::with_capacity(area),
            grown: 0,
            target_land: 0,
            growth_work: 0,
            feature_chance: 0,
            cursor: 0,
            shapes: vec![0; area],
            cells: vec![Cell::default(); area],
            paths: vec![0; area],
            walkable: vec![TileMask::default(); area],
            reached: vec![TileMask::default(); area],
            queue: vec![0; area],
            queued: vec![false; area],
            queue_head: 0,
            queue_len: 0,
            flood_work: 0,
            walkable_tiles: 0,
            placements: None,
        })
    }
    pub fn spec(&self) -> &GenerationSpec {
        &self.spec
    }
    pub const fn phase(&self) -> Phase {
        self.phase
    }
    /// Zero-based attempt identity. Exactly 64 attempts (0..63) are permitted.
    pub const fn attempt(&self) -> u8 {
        self.attempt
    }
    pub const fn total_work(&self) -> u64 {
        self.total_work
    }
    pub const fn last_rejection(&self) -> Option<Rejection> {
        self.last_rejection
    }
    fn switch(&mut self, phase: Phase, stage: &[u8]) {
        self.phase = phase;
        self.cursor = 0;
        let identity = if phase == Phase::Population {
            self.identity
        } else {
            self.spec.geometry_identity()
        };
        self.rng = StageRng::derive(&identity, self.spec.level, self.attempt, stage);
    }
    fn reject(&mut self, reason: Rejection) {
        self.last_rejection = Some(reason);
        if self.attempt + 1 == MAX_ATTEMPTS {
            self.phase = Phase::Rejected;
            return;
        }
        self.attempt += 1;
        self.anchors.clear();
        self.anchor_count = 0;
        self.anchor_tries = 0;
        self.route = 0;
        self.route_position = CellPosition::default();
        self.regions.fill(RegionCell::default());
        self.frontier.clear();
        self.grown = 0;
        self.target_land = 0;
        self.growth_work = 0;
        self.feature_chance = 0;
        self.shapes.fill(0);
        self.cells.fill(Cell::default());
        self.paths.fill(0);
        self.walkable.fill(TileMask::default());
        self.reached.fill(TileMask::default());
        self.queue.fill(0);
        self.queued.fill(false);
        self.queue_head = 0;
        self.queue_len = 0;
        self.flood_work = 0;
        self.walkable_tiles = 0;
        self.placements = None;
        self.switch(Phase::Anchors, b"anchors");
    }
    pub fn advance(&mut self, resources: &impl Resources, work: u32) -> Result<Phase> {
        self.validate()?;
        if work == 0 || work > MAX_WORK_PER_ADVANCE {
            return Err(Error::Bounds("generation work budget 1..=1024"));
        }
        for _ in 0..work {
            if matches!(self.phase, Phase::Complete | Phase::Rejected) {
                break;
            }
            self.tick(resources)?;
            self.total_work = self
                .total_work
                .checked_add(1)
                .ok_or(Error::State("generation work overflow"))?;
        }
        Ok(self.phase)
    }
    fn tick(&mut self, resources: &impl Resources) -> Result<()> {
        match self.phase {
            Phase::Anchors => self.anchor_tick(resources)?,
            Phase::Paths => self.path_tick(resources)?,
            Phase::Features => self.feature_tick(resources)?,
            Phase::Regions => self.region_tick(),
            Phase::Shapes => self.shape_tick(resources)?,
            Phase::Alternatives => self.alternative_tick(resources)?,
            Phase::Terrain => self.terrain_tick(resources)?,
            Phase::Flood => self.flood_tick(),
            Phase::Validate => self.route_validation()?,
            Phase::Population => {
                match self
                    .placements
                    .as_mut()
                    .ok_or(Error::State("missing placement continuation"))?
                    .advance(self.dimensions, &self.reached, self.anchors[0])
                {
                    Some(true) => self.phase = Phase::Complete,
                    Some(false) => self.reject(Rejection::PopulationCapacity),
                    None => (),
                }
            }
            Phase::Complete | Phase::Rejected => (),
        }
        Ok(())
    }
    fn parameter(&mut self, resources: &impl Resources, parameter: u8) -> Result<u32> {
        let (minimum, maximum) = resources.parameter(self.spec.level, parameter)?;
        if minimum > maximum {
            return Err(Error::Resource("reversed generation parameter"));
        }
        Ok(u32::from(minimum) + self.rng.bounded(u32::from(maximum - minimum) + 1))
    }
    fn anchor_tick(&mut self, resources: &impl Resources) -> Result<()> {
        if self.anchor_count == 0 {
            self.anchor_count = self
                .parameter(resources, 6)?
                .clamp(3, (self.dimensions.area() / 64).clamp(3, 8) as u32)
                as u8;
        }
        if self.anchor_tries >= self.dimensions.area() as u32 * 8 {
            self.reject(Rejection::AnchorCapacity);
            return Ok(());
        }
        self.anchor_tries += 1;
        let position = CellPosition {
            x: self.rng.bounded(u32::from(self.dimensions.width() - 4)) as u16 + 2,
            y: self.rng.bounded(u32::from(self.dimensions.height() - 4)) as u16 + 2,
        };
        let separation_x = (u32::from(self.dimensions.width()) * 3).div_ceil(19) as i32;
        let separation_y = (u32::from(self.dimensions.height()) * 5).div_ceil(26) as i32;
        if self.anchors.iter().any(|prior| {
            (i32::from(prior.x) - i32::from(position.x)).abs() < separation_x
                && (i32::from(prior.y) - i32::from(position.y)).abs() < separation_y
        }) {
            return Ok(());
        }
        let label = self.anchors.len() as u8;
        self.anchors.push(position);
        let alternate = self.rng.bounded(100) < 40;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let index = self
                    .dimensions
                    .index(i32::from(position.x) + dx, i32::from(position.y) + dy)
                    .expect("interior anchor footprint");
                self.add_land(index, label, alternate);
            }
        }
        if self.anchors.len() == usize::from(self.anchor_count) {
            self.route_position = self.anchors[0];
            self.switch(Phase::Paths, b"paths");
        }
        Ok(())
    }
    fn add_land(&mut self, index: usize, label: u8, alternate: bool) {
        if self.regions[index].kind == RegionKind::Empty {
            self.frontier.push(index as u16);
            self.grown += 1;
        }
        self.regions[index] = RegionCell {
            kind: RegionKind::Land,
            label,
            alternate,
            directions: 0,
        };
    }
    fn path_tick(&mut self, resources: &impl Resources) -> Result<()> {
        if self.route >= self.anchor_count {
            let percentage = self.parameter(resources, 3)?.clamp(20, 85);
            self.target_land = (self.dimensions.area() as u32 * percentage / 100).max(self.grown);
            self.switch(Phase::Regions, b"regions");
            return Ok(());
        }
        let target = self.anchors[(usize::from(self.route) + 1) % self.anchors.len()];
        if self.route_position == target {
            self.route += 1;
            self.route_position = self.anchors[usize::from(self.route) % self.anchors.len()];
            return Ok(());
        }
        let source = self.route_position;
        let horizontal = source.x != target.x && (source.y == target.y || self.rng.bounded(2) == 0);
        let (next, direction, opposite) = if horizontal {
            if source.x < target.x {
                (
                    CellPosition {
                        x: source.x + 1,
                        y: source.y,
                    },
                    2,
                    8,
                )
            } else {
                (
                    CellPosition {
                        x: source.x - 1,
                        y: source.y,
                    },
                    8,
                    2,
                )
            }
        } else if source.y < target.y {
            (
                CellPosition {
                    x: source.x,
                    y: source.y + 1,
                },
                4,
                1,
            )
        } else {
            (
                CellPosition {
                    x: source.x,
                    y: source.y - 1,
                },
                1,
                4,
            )
        };
        for (position, bits) in [(source, direction), (next, opposite)] {
            let index = position.index(self.dimensions)?;
            if self.regions[index].kind == RegionKind::Empty {
                self.frontier.push(index as u16);
                self.grown += 1;
                self.regions[index] = RegionCell {
                    kind: RegionKind::Path,
                    label: self.route,
                    directions: bits,
                    alternate: false,
                };
            } else if self.regions[index].kind == RegionKind::Path {
                self.regions[index].directions |= bits;
            }
        }
        self.route_position = next;
        Ok(())
    }
    fn neighbors(&self, index: usize) -> [Option<usize>; 4] {
        let width = usize::from(self.dimensions.width());
        [
            (index >= width).then(|| index - width),
            (index % width + 1 < width).then_some(index + 1),
            (index + width < self.dimensions.area()).then_some(index + width),
            (!index.is_multiple_of(width)).then(|| index - 1),
        ]
    }
    fn region_tick(&mut self) {
        if self.grown >= self.target_land {
            self.switch(Phase::Features, b"features");
            return;
        }
        if self.growth_work >= self.dimensions.area() as u32 * 32 {
            self.reject(Rejection::RegionGrowth);
            return;
        }
        self.growth_work += 1;
        let index =
            usize::from(self.frontier[self.rng.bounded(self.frontier.len() as u32) as usize]);
        let direction = self.rng.bounded(4) as usize;
        if let Some(next) = self.neighbors(index)[direction] {
            let position = self.dimensions.position(next).expect("neighbor index");
            if position.x != 0
                && position.y != 0
                && position.x + 1 < self.dimensions.width()
                && position.y + 1 < self.dimensions.height()
                && self.regions[next].kind == RegionKind::Empty
            {
                self.add_land(
                    next,
                    self.regions[index].label,
                    self.regions[index].alternate,
                );
            }
        }
    }
    fn feature_tick(&mut self, resources: &impl Resources) -> Result<()> {
        if self.cursor == 0 {
            self.feature_chance = self.parameter(resources, 4)?.min(35) as u8;
        }
        let index = self.cursor as usize;
        let position = self.dimensions.position(index).expect("feature cursor");
        let away_from_anchors = self.anchors.iter().all(|anchor| {
            (i32::from(anchor.x) - i32::from(position.x)).abs() > 2
                || (i32::from(anchor.y) - i32::from(position.y)).abs() > 2
        });
        let mut patch = [0_usize; 9];
        let mut count = 0;
        if away_from_anchors {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if let Some(next) = self
                        .dimensions
                        .index(i32::from(position.x) + dx, i32::from(position.y) + dy)
                        && self.regions[next].kind == RegionKind::Land
                    {
                        patch[count] = next;
                        count += 1;
                    }
                }
            }
        }
        if count == 9 && self.rng.bounded(100) < u32::from(self.feature_chance) {
            for next in patch {
                self.regions[next].kind = RegionKind::Filled;
            }
        }
        self.cursor += 1;
        if self.cursor as usize == self.dimensions.area() {
            self.switch(Phase::Shapes, b"shapes");
        }
        Ok(())
    }
    fn corner_mask(&self, position: CellPosition, filled: bool) -> u8 {
        let occupied = |dx, dy| {
            self.dimensions
                .index(i32::from(position.x) + dx, i32::from(position.y) + dy)
                .is_some_and(|index| {
                    if filled {
                        self.regions[index].kind == RegionKind::Filled
                    } else {
                        self.regions[index].kind != RegionKind::Empty
                    }
                })
        };
        let mut mask = 0;
        for (bit, dx, dy) in [(1, -1, -1), (2, 1, -1), (4, 1, 1), (8, -1, 1)] {
            if occupied(dx, 0) && occupied(0, dy) && occupied(dx, dy) {
                mask |= bit;
            }
        }
        mask
    }
    fn shape_tick(&mut self, resources: &impl Resources) -> Result<()> {
        let index = self.cursor as usize;
        let region = self.regions[index];
        self.shapes[index] = match region.kind {
            RegionKind::Empty => 0,
            RegionKind::Path => {
                self.paths[index] = region.directions;
                resources.shape(Shape::Path, region.directions, false)?
            }
            RegionKind::Land | RegionKind::Filled => {
                let filled = region.kind == RegionKind::Filled;
                let mask = self.corner_mask(
                    self.dimensions.position(index).expect("shape cursor"),
                    filled,
                );
                resources.shape(
                    if filled { Shape::Filled } else { Shape::Edge },
                    mask,
                    region.alternate,
                )?
            }
        };
        self.cursor += 1;
        if self.cursor as usize == self.dimensions.area() {
            self.switch(Phase::Alternatives, b"alternatives");
        }
        Ok(())
    }
    fn alternative_tick(&mut self, resources: &impl Resources) -> Result<()> {
        let index = self.cursor as usize;
        let shape = self.shapes[index];
        let draw = u32::from(self.rng.next_u16());
        let mut cell = resources.alternative(shape, draw)?;
        let region = self.regions[index];
        let boundary = region.kind == RegionKind::Land
            && self.neighbors(index).into_iter().flatten().any(|next| {
                self.regions[next].kind == RegionKind::Land
                    && self.regions[next].alternate != region.alternate
            });
        if boundary && self.rng.bounded(5) == 0 {
            let first = self.rng.bounded(16) as u8;
            for offset in 0..16 {
                if let Some(replacement) =
                    resources.transition(shape, (first + offset) % 16, draw)?
                {
                    cell = replacement;
                    break;
                }
            }
        }
        self.cells[index] = cell;
        self.cursor += 1;
        if self.cursor as usize == self.dimensions.area() {
            self.switch(Phase::Terrain, b"terrain");
        }
        Ok(())
    }
    fn terrain_tick(&mut self, resources: &impl Resources) -> Result<()> {
        let index = self.cursor as usize;
        self.walkable[index] = walkable_mask(self.cells[index], resources)?;
        self.cursor += 1;
        if self.cursor as usize == self.dimensions.area() {
            let arrival = self.anchors[0].index(self.dimensions)?;
            if !self.walkable[arrival].contains(4 * 16 + 8) {
                self.reject(Rejection::RequiredTerrainRoute);
                return Ok(());
            }
            self.reached[arrival].insert(4 * 16 + 8);
            self.enqueue(arrival);
            self.switch(Phase::Flood, b"connectivity");
        }
        Ok(())
    }
    fn enqueue(&mut self, index: usize) {
        if !self.queued[index] {
            let slot = (self.queue_head as usize + self.queue_len as usize) % self.queue.len();
            self.queue[slot] = index as u16;
            self.queue_len += 1;
            self.queued[index] = true;
        }
    }
    fn flood_tick(&mut self) {
        if self.queue_len == 0 {
            self.switch(Phase::Validate, b"validation");
            return;
        }
        if self.flood_work >= self.dimensions.area() as u32 * 128 {
            self.reject(Rejection::RequiredTerrainRoute);
            return;
        }
        self.flood_work += 1;
        let index = usize::from(self.queue[self.queue_head as usize]);
        self.queue_head = (self.queue_head + 1) % self.queue.len() as u32;
        self.queue_len -= 1;
        self.queued[index] = false;
        self.reached[index] = self.walkable[index].flood(self.reached[index]);
        let mask = self.reached[index];
        for (direction, neighbor) in self.neighbors(index).into_iter().enumerate() {
            let Some(next) = neighbor else {
                continue;
            };
            let mut changed = false;
            for offset in 0..if direction % 2 == 0 { 16 } else { 8 } {
                let (source, destination) = match direction {
                    0 => (offset, 112 + offset),
                    1 => (offset * 16 + 15, offset * 16),
                    2 => (112 + offset, offset),
                    _ => (offset * 16, offset * 16 + 15),
                };
                if mask.contains(source) && self.walkable[next].contains(destination) {
                    changed |= self.reached[next].insert(destination);
                }
            }
            if changed {
                self.enqueue(next);
            }
        }
    }
    fn output_anchors(&self) -> Anchors {
        Anchors {
            arrival: self.anchors[0],
            exit: self.anchors[1],
            ship_piece: self.spec.baseline.ship_piece.then_some(self.anchors[2]),
            regions: self.anchors.clone(),
        }
    }
    fn route_validation(&mut self) -> Result<()> {
        for position in &self.anchors {
            let mask = self.reached[position.index(self.dimensions)?];
            for row in 3..=4 {
                for column in 6..=9 {
                    if !mask.contains(row * 16 + column) {
                        self.reject(Rejection::RequiredTerrainRoute);
                        return Ok(());
                    }
                }
            }
        }
        self.walkable_tiles = self.reached.iter().map(|mask| mask.count()).sum();
        let budget = PopulationBudget::calculate(
            self.spec.baseline,
            self.walkable_tiles,
            self.spec.config.density,
        )?;
        self.placements = Some(PlacementBuilder::new(
            budget,
            self.spec.baseline,
            &self.output_anchors(),
            StageRng::derive(&self.identity, self.spec.level, self.attempt, b"population"),
            StageRng::derive(&self.identity, self.spec.level, self.attempt, b"runtime-ai"),
        )?);
        self.switch(Phase::Population, b"loot");
        Ok(())
    }
    pub fn finish(self) -> Result<GeneratedLevel> {
        self.validate()?;
        if self.phase == Phase::Rejected {
            return Err(Error::Generation {
                level: self.spec.level,
                attempts: MAX_ATTEMPTS,
                reason: self
                    .last_rejection
                    .ok_or(Error::State("missing rejection reason"))?
                    .reason(),
            });
        }
        if self.phase != Phase::Complete {
            return Err(Error::State("generation is not complete"));
        }
        let anchors = self.output_anchors();
        let result = GeneratedLevel {
            identity: self.identity,
            accepted_attempt: self.attempt,
            geometry: LevelGeometry::new(self.dimensions, self.cells, self.paths, anchors)?,
            population: self
                .placements
                .ok_or(Error::State("missing final population"))?
                .population,
            runtime_rng: StageRng::derive(
                &self.identity,
                self.spec.level,
                self.attempt,
                b"runtime-ai",
            ),
            walkable_tiles: self.walkable_tiles,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<()> {
        self.spec.validate()?;
        self.dimensions.validate()?;
        let area = self.dimensions.area();
        if self.identity != self.spec.identity()
            || self.dimensions != self.spec.config.dimensions_for(self.spec.level)
            || self.attempt >= MAX_ATTEMPTS
            || !self.rng.valid()
            || self.anchors.len() > 8
            || self.anchor_count > 8
            || self.cursor as usize > area
            || self.route > self.anchor_count
            || self.frontier.len() > area
            || self.queue.len() != area
            || self.queue_head as usize >= area
            || self.queue_len as usize > area
            || self.regions.len() != area
            || self.shapes.len() != area
            || self.cells.len() != area
            || self.paths.len() != area
            || self.walkable.len() != area
            || self.reached.len() != area
            || self.queued.len() != area
            || self.anchor_tries > area as u32 * 8
            || self.growth_work > area as u32 * 32
            || self.flood_work > area as u32 * 128
            || self.grown as usize > area
            || self.target_land as usize > area
            || self.walkable_tiles > area as u32 * 128
            || self.feature_chance > 35
            || self.total_work > u64::from(MAX_ATTEMPTS) * (area as u64 * 940 + 2048)
            || (matches!(
                self.phase,
                Phase::Features | Phase::Shapes | Phase::Alternatives | Phase::Terrain
            ) && self.cursor as usize >= area)
            || (self.anchor_count != 0 && self.anchor_count < 3)
            || self.anchors.len() > usize::from(self.anchor_count)
            || self.frontier.len() != self.grown as usize
        {
            return Err(Error::State("generation identity/phase/extent/work bounds"));
        }
        for position in &self.anchors {
            position.index(self.dimensions)?;
            if position.x < 2
                || position.y < 2
                || position.x + 2 >= self.dimensions.width()
                || position.y + 2 >= self.dimensions.height()
            {
                return Err(Error::State("generation anchor footprint bounds"));
            }
        }
        if self
            .frontier
            .iter()
            .chain(&self.queue)
            .any(|index| usize::from(*index) >= area)
            || self
                .regions
                .iter()
                .any(|cell| cell.directions > 15 || cell.label > 7)
            || self.paths.iter().any(|path| *path > 15)
            || self.shapes.iter().any(|shape| *shape >= 98)
        {
            return Err(Error::State("generation cell/worklist values"));
        }
        for cell in &self.cells {
            cell.validate()?;
        }
        if !matches!(self.phase, Phase::Anchors | Phase::Rejected)
            && (self.anchors.len() < 3 || self.anchors.len() != usize::from(self.anchor_count))
        {
            return Err(Error::State("generation anchor phase"));
        }
        if self.phase == Phase::Paths {
            self.route_position.index(self.dimensions)?;
        }
        if self.phase == Phase::Regions && self.frontier.is_empty() {
            return Err(Error::State("generation growth frontier is empty"));
        }
        let mut frontier_seen = [false; 4096];
        for &index in &self.frontier {
            if std::mem::replace(&mut frontier_seen[usize::from(index)], true) {
                return Err(Error::State("duplicate generation frontier entry"));
            }
        }
        if self.queued.iter().filter(|queued| **queued).count() != self.queue_len as usize {
            return Err(Error::State("generation queue flags/count"));
        }
        let mut queue_seen = [false; 4096];
        for offset in 0..self.queue_len as usize {
            let index = usize::from(self.queue[(self.queue_head as usize + offset) % area]);
            if !self.queued[index] || std::mem::replace(&mut queue_seen[index], true) {
                return Err(Error::State("generation queue membership/duplicates"));
            }
        }
        if self
            .reached
            .iter()
            .zip(&self.walkable)
            .any(|(reached, walkable)| {
                reached.0[0] & !walkable.0[0] != 0 || reached.0[1] & !walkable.0[1] != 0
            })
        {
            return Err(Error::State(
                "generation connectivity is outside walkable terrain",
            ));
        }
        if let Some(placements) = &self.placements {
            placements.validate(self.dimensions)?;
        }
        if matches!(self.phase, Phase::Population | Phase::Complete) && self.placements.is_none() {
            return Err(Error::State("generation placement phase"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Density, LevelSize};

    fn initial() -> Generation {
        Generation::new(GenerationSpec {
            config: WorldConfig {
                root_seed: 1,
                rom_sha1: [0; 20],
                size: LevelSize::Expanded,
                density: Density::Normal,
            },
            level: LevelId::new(2).unwrap(),
            baseline: PopulationBaseline {
                walkable_tiles: 10000,
                ordinary_items: 12,
                earthlings: 10,
                reserved_items: 2,
                ship_piece: true,
                mailbox: true,
            },
        })
        .unwrap()
    }
    #[test]
    fn malformed_decoded_worklists_and_phase_cursors_fail_closed() {
        let mut state = initial();
        state.queue_head = state.dimensions.area() as u32;
        assert!(state.validate().is_err());
        let mut state = initial();
        state.queued[0] = true;
        assert!(state.validate().is_err());
        let mut state = initial();
        state.frontier = vec![0, 0];
        state.grown = 2;
        assert!(state.validate().is_err());
        let mut state = initial();
        state.reached[0].insert(0);
        assert!(state.validate().is_err());
        for phase in [
            Phase::Features,
            Phase::Shapes,
            Phase::Alternatives,
            Phase::Terrain,
        ] {
            let mut state = initial();
            state.phase = phase;
            state.cursor = state.dimensions.area() as u32;
            assert!(state.validate().is_err());
        }
    }
    #[test]
    fn finish_validates_a_decoded_complete_state_before_indexing_anchors() {
        let mut state = initial();
        state.phase = Phase::Complete;
        assert!(state.finish().is_err());
    }
}
