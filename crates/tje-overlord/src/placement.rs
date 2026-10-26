//! Exact shared terrain footprints, fixed hero interest, and a bounded supported
//! ground flood. No renderer camera, map mutation, RNG, or original RAM writes.
use crate::{Error, HEROES, Kind, LevelId, Result};
use bincode::{Decode, Encode};
use tje_simulation::population::{DefinitionCatalog, HeroView, PopulationState};
use tje_world::{
    LevelGeometry, Resources,
    terrain::{Footprint, TerrainMask},
    world::WorldState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}
impl Point {
    /// Commands are recorded at the eight-pixel terrain tile grid.
    pub fn new(x: i32, y: i32) -> Result<Self> {
        let point = Self { x, y };
        point.validate()?;
        Ok(point)
    }
    pub fn quantize(x: i32, y: i32) -> Result<Self> {
        Self::new(x.div_euclid(8) * 8, y.div_euclid(8) * 8)
    }
    pub fn validate(self) -> Result<()> {
        if self.x < 128
            || self.y < 64
            || self.x > 8320
            || self.y > 4160
            || self.x % 8 != 0
            || self.y % 8 != 0
        {
            return Err(Error::Placement(Rejection::Coordinates));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    Coordinates,
    DormantFloor,
    Terrain,
    SafeZone,
    Visible,
    TooFar,
    Occupied,
    Unreachable,
}
const AXIS: usize = 161;
const CELLS: usize = AXIS * AXIS;

/// Allocated once, not serialized: about 76 KiB regardless of world size.
pub struct Scratch {
    seen: Vec<bool>,
    queue: Vec<u16>,
}
impl Default for Scratch {
    fn default() -> Self {
        Self {
            seen: vec![false; CELLS],
            queue: Vec::with_capacity(CELLS),
        }
    }
}
impl Scratch {
    fn reachable(
        &mut self,
        geometry: &LevelGeometry,
        level: LevelId,
        point: Point,
        heroes: &[Option<HeroView>; 4],
        footprint: Footprint,
        resources: &impl Resources,
    ) -> Result<bool> {
        self.seen.fill(false);
        self.queue.clear();
        let left = point.x / 8 - 80;
        let top = point.y / 8 - 80;
        let target = 80 * AXIS + 80;
        let walkable = |x: i32, y: i32| -> Result<bool> {
            footprint
                .supported_at(
                    geometry,
                    level,
                    (x * 8, y * 8),
                    TerrainMask::LAND_OR_SAND,
                    resources,
                )
                .map_err(|_| Error::Shared)
        };
        if !walkable(point.x / 8, point.y / 8)? {
            return Ok(false);
        }
        self.queue.push(target as u16);
        self.seen[target] = true;
        let mut next = 0;
        while next < self.queue.len() {
            let index = usize::from(self.queue[next]);
            next += 1;
            let x = left + (index % AXIS) as i32;
            let y = top + (index / AXIS) as i32;
            if heroes.iter().flatten().any(|hero| {
                HEROES.contains(hero.seat)
                    && hero.level == level
                    && (hero.position.x >> 11) == x
                    && (hero.position.y >> 11) == y
            }) {
                return Ok(true);
            }
            let column = index % AXIS;
            let row = index / AXIS;
            for neighbor in [
                (column > 0).then(|| index - 1),
                (column + 1 < AXIS).then(|| index + 1),
                (row > 0).then(|| index - AXIS),
                (row + 1 < AXIS).then(|| index + AXIS),
            ]
            .into_iter()
            .flatten()
            {
                if self.seen[neighbor] {
                    continue;
                }
                self.seen[neighbor] = true;
                let nx = left + (neighbor % AXIS) as i32;
                let ny = top + (neighbor / AXIS) as i32;
                if walkable(nx, ny)? {
                    self.queue.push(neighbor as u16);
                }
            }
        }
        Ok(false)
    }
}

pub struct Context<'a, R> {
    pub world: &'a WorldState,
    pub resources: &'a R,
    pub heroes: &'a [Option<HeroView>; 4],
    pub population: &'a PopulationState,
    pub catalog: &'a DefinitionCatalog,
}
impl<R: Resources> Context<'_, R> {
    pub fn geometry(&self, level: LevelId) -> Result<&LevelGeometry> {
        self.world
            .floor(level)
            .map(|floor| &floor.generated.geometry)
            .ok_or(Error::Placement(Rejection::DormantFloor))
    }
    pub fn active(&self, level: LevelId) -> bool {
        self.heroes
            .iter()
            .flatten()
            .any(|hero| HEROES.contains(hero.seat) && hero.level == level)
    }
    pub fn safe_zone(&self, level: LevelId, point: Point) -> Result<bool> {
        let geometry = self.geometry(level)?;
        let anchors = geometry.anchors();
        let near = |x: i32, y: i32| {
            (i64::from(point.x) - i64::from(x)).abs() <= 128
                && (i64::from(point.y) - i64::from(y)).abs() <= 128
        };
        if [
            Some(anchors.arrival),
            Some(anchors.exit),
            anchors.ship_piece,
        ]
        .into_iter()
        .flatten()
        .any(|anchor| {
            near(
                (i32::from(anchor.x) + 1) * 128 + 64,
                (i32::from(anchor.y) + 1) * 64 + 32,
            )
        }) {
            return Ok(true);
        }
        Ok(self
            .population
            .floors()
            .iter()
            .filter(|floor| floor.level == level)
            .flat_map(|floor| &floor.actors)
            .any(|actor| {
                !actor.retired
                    && (actor.kind.required() || actor.kind.service().is_some())
                    && near(actor.pixels().0, actor.pixels().1)
            }))
    }
    pub fn validate(
        &self,
        level: LevelId,
        point: Point,
        kind: Option<Kind>,
        scratch: &mut Scratch,
    ) -> Result<()> {
        self.validate_ignoring(level, point, kind, None, scratch)
    }
    pub fn validate_ignoring(
        &self,
        level: LevelId,
        point: Point,
        kind: Option<Kind>,
        ignore: Option<tje_simulation::population::PopulationId>,
        scratch: &mut Scratch,
    ) -> Result<()> {
        point.validate()?;
        if !self.active(level) {
            return Err(Error::Placement(Rejection::DormantFloor));
        }
        let geometry = self.geometry(level)?;
        if geometry.dimensions().world_cell(point.x, point.y).is_none() {
            return Err(Error::Placement(Rejection::Coordinates));
        }
        let (width, height, mask) = kind.map_or((6, 4, 3), Kind::spawn_footprint);
        if !Footprint::new(width, height)
            .map_err(|_| Error::Shared)?
            .supported_at(
                geometry,
                level,
                (point.x, point.y),
                TerrainMask::new(mask).map_err(|_| Error::Shared)?,
                self.resources,
            )
            .map_err(|_| Error::Shared)?
            || kind == Some(Kind::Lawnmower)
                && geometry
                    .terrain_at(level, point.x, point.y, self.resources)
                    .map_err(|_| Error::Shared)?
                    != 0x10
        {
            return Err(Error::Placement(Rejection::Terrain));
        }
        if self.safe_zone(level, point)? {
            return Err(Error::Placement(Rejection::SafeZone));
        }
        let mut near = false;
        for hero in self
            .heroes
            .iter()
            .flatten()
            .filter(|hero| HEROES.contains(hero.seat) && hero.level == level)
        {
            let dx = (i64::from(point.x) - i64::from(hero.position.x >> 8)).abs();
            let dy = (i64::from(point.y) - i64::from(hero.position.y >> 8)).abs();
            near |= dx * dx + dy * dy <= 640 * 640;
            if kind.is_some() && dx <= 232 && dy <= 144 {
                return Err(Error::Placement(Rejection::Visible));
            }
        }
        if !near {
            return Err(Error::Placement(Rejection::TooFar));
        }
        let radius = kind.map_or(24, |kind| i64::from(self.catalog.get(kind).radius) * 4);
        if self
            .population
            .floors()
            .iter()
            .filter(|floor| floor.level == level)
            .flat_map(|floor| &floor.actors)
            .any(|actor| {
                Some(actor.id) != ignore
                    && !actor.retired
                    && (i64::from(point.x) - i64::from(actor.pixels().0)).abs()
                        < radius + i64::from(self.catalog.get(actor.kind).radius) * 4
                    && (i64::from(point.y) - i64::from(actor.pixels().1)).abs()
                        < radius + i64::from(self.catalog.get(actor.kind).radius) * 4
            })
        {
            return Err(Error::Placement(Rejection::Occupied));
        }
        if !scratch.reachable(
            geometry,
            level,
            point,
            self.heroes,
            Footprint::new(width, height).map_err(|_| Error::Shared)?,
            self.resources,
        )? {
            return Err(Error::Placement(Rejection::Unreachable));
        }
        Ok(())
    }
}
