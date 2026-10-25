//! Geometry services generalized from tje-game/map/{terrain,placement}. No
//! player number, camera bank, renderer residency or cartridge RAM is consulted.
use crate::{
    Cell, Dimensions, Error, LevelGeometry, LevelId, Resources, Result,
    random::{Random16, StageRng},
};
use bincode::{Decode, Encode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct TerrainMask(u8);
impl TerrainMask {
    pub const LAND: Self = Self(1);
    pub const SAND: Self = Self(2);
    pub const WATER: Self = Self(8);
    pub const LAND_OR_SAND: Self = Self(3);
    pub fn new(bits: u8) -> Result<Self> {
        if bits == 0 || bits & !11 != 0 {
            return Err(Error::Bounds("terrain eligibility mask"));
        }
        Ok(Self(bits))
    }
    pub fn validate(self) -> Result<()> {
        Self::new(self.0).map(|_| ())
    }
    pub const fn accepts(self, terrain: u8) -> bool {
        match terrain {
            0x10 => self.0 & 1 != 0,
            0x20 => self.0 & 2 != 0,
            0x80 => self.0 & 8 != 0,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Footprint {
    width: u8,
    height: u8,
}
impl Footprint {
    pub fn new(width: u8, height: u8) -> Result<Self> {
        if !(1..=16).contains(&width) || !(1..=8).contains(&height) {
            return Err(Error::Bounds("terrain footprint must fit a 16x8 metatile"));
        }
        Ok(Self { width, height })
    }
    pub fn validate(self) -> Result<()> {
        Self::new(self.width, self.height).map(|_| ())
    }
    /// Position is the original foot anchor: horizontal center, two pixels above
    /// the bottom of the support rectangle (TerrainValidation's output rule).
    pub fn supported_at(
        self,
        geometry: &LevelGeometry,
        level: LevelId,
        position: (i32, i32),
        allowed: TerrainMask,
        resources: &impl Resources,
    ) -> Result<bool> {
        self.validate()?;
        allowed.validate()?;
        let left = i64::from(position.0) - i64::from(self.width) * 4;
        let top = i64::from(position.1) - i64::from(self.height) * 8 + 2;
        let right = left + i64::from(self.width) * 8 - 1;
        let bottom = top + i64::from(self.height) * 8 - 1;
        if left < 128
            || top < 64
            || right >= i64::from((geometry.dimensions().width() as i32 + 1) * 128)
            || bottom >= i64::from((geometry.dimensions().height() as i32 + 1) * 64)
        {
            return Ok(false);
        }
        for row in top.div_euclid(8)..=bottom.div_euclid(8) {
            for column in left.div_euclid(8)..=right.div_euclid(8) {
                if !allowed.accepts(geometry.terrain_at(
                    level,
                    (column * 8) as i32,
                    (row * 8) as i32,
                    resources,
                )?) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct DoorwaySupport {
    pub owner: u8,
    pub level: LevelId,
    pub x: i32,
    pub y: i32,
    pub open: bool,
}
impl DoorwaySupport {
    pub fn validate(&self, dimensions: Dimensions) -> Result<()> {
        self.level.validate()?;
        if self.owner >= 4 || dimensions.world_cell(self.x, self.y).is_none() {
            return Err(Error::Bounds("doorway owner/position"));
        }
        Ok(())
    }
    pub fn supports_tile(&self, level: LevelId, player_y: i32, column: i32, row: i32) -> bool {
        let first_row = (i64::from(self.y) - 10).div_euclid(8);
        let first_column = (i64::from(self.x) - 16).div_euclid(8);
        self.level == level
            && (self.open || player_y >= self.y)
            && (first_row..first_row + 3).contains(&i64::from(row))
            && (first_column..first_column + 4).contains(&i64::from(column))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct TerrainCache {
    pub tile_x: i32,
    pub tile_y: i32,
    pub tiles: [u8; 25],
}
impl TerrainCache {
    pub fn sample(
        geometry: &LevelGeometry,
        level: LevelId,
        position: (i32, i32),
        resources: &impl Resources,
        doorways: &[DoorwaySupport],
    ) -> Result<Self> {
        if doorways.len() > 4 {
            return Err(Error::Bounds("terrain cache doorway count"));
        }
        let tile_x = position.0.div_euclid(8);
        let tile_y = position.1.div_euclid(8);
        let mut tiles = [0; 25];
        for row in 0..5 {
            for column in 0..5 {
                let x = i64::from(tile_x) + column as i64 - 2;
                let y = i64::from(tile_y) + row as i64 - 2;
                let x = i32::try_from(x * 8).map_err(|_| Error::Bounds("terrain cache X"))?;
                let y = i32::try_from(y * 8).map_err(|_| Error::Bounds("terrain cache Y"))?;
                tiles[row * 5 + column] = if doorways.iter().any(|doorway| {
                    doorway.supports_tile(level, position.1, x.div_euclid(8), y.div_euclid(8))
                }) {
                    0x10
                } else {
                    geometry.terrain_at(level, x, y, resources)?
                };
            }
        }
        Ok(Self {
            tile_x,
            tile_y,
            tiles,
        })
    }
    pub const fn underfoot(&self) -> u8 {
        self.tiles[12]
    }
}

/// TerrainValidation's cyclic rectangle search, with explicit valid dimensions.
/// The random row is drawn before the column, and a successful mirrored anchor
/// uses 128-x, as in the readable body. Overlay permission is a separate flag.
pub fn position_in_cell(
    cell: Cell,
    footprint: Footprint,
    allowed: TerrainMask,
    allow_overlay: bool,
    rng: &mut impl Random16,
    resources: &impl Resources,
) -> Result<Option<(i32, i32)>> {
    footprint.validate()?;
    allowed.validate()?;
    cell.validate()?;
    if (!allow_overlay && cell.overlay() != 0) || matches!(cell.base() & 127, 34 | 82 | 84) {
        return Ok(None);
    }
    if footprint.width == 16 && footprint.height == 8 {
        let base = cell.base() & 127;
        return Ok(((allowed.0 & 1 != 0 && matches!(base, 30 | 33))
            || (allowed.0 & 2 != 0 && matches!(base, 53 | 56..=58))
            || (allowed.0 & 8 != 0 && matches!(base, 75 | 76 | 80)))
        .then_some((63, 31)));
    }
    let columns = 17 - u32::from(footprint.width);
    let rows = 9 - u32::from(footprint.height);
    let start_row = rng.bounded(rows);
    let start_column = rng.bounded(columns);
    for dy in 0..rows {
        let row = (start_row + dy) % rows;
        for dx in 0..columns {
            let column = (start_column + dx) % columns;
            let mut valid = true;
            'tiles: for y in row..row + u32::from(footprint.height) {
                for x in column..column + u32::from(footprint.width) {
                    let tile = resources.base_tile(cell.base() & 127, y as u8, x as u8)?;
                    if !allowed.accepts(resources.property(tile)?) {
                        valid = false;
                        break 'tiles;
                    }
                }
            }
            if valid {
                let x = (column * 8 + u32::from(footprint.width) * 4) as i32;
                let x = if cell.base() & 128 != 0 { 128 - x } else { x };
                return Ok(Some((
                    x,
                    (row * 8 + u32::from(footprint.height) * 8 - 2) as i32,
                )));
            }
        }
    }
    Ok(None)
}

/// Collision/arrival exclusions are independent of camera and entity residency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Exclusion {
    pub level: LevelId,
    pub x: i32,
    pub y: i32,
    pub half_width: u16,
    pub half_height: u16,
}
impl Exclusion {
    pub fn contains(self, level: LevelId, position: (i32, i32)) -> bool {
        self.level == level
            && (i64::from(position.0) - i64::from(self.x)).abs() < i64::from(self.half_width)
            && (i64::from(position.1) - i64::from(self.y)).abs() < i64::from(self.half_height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum SearchProgress {
    Searching,
    Found((i32, i32)),
    Exhausted,
}

/// Ten random metatiles then a cyclic exhaustive scan. Every rejected cell,
/// including a marked one, consumes work; the original all-marked infinite loop
/// and uninitialized-coordinate fallthrough are intentionally not generalized.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PlacementSearch {
    dimensions: Dimensions,
    footprint: Footprint,
    allowed: TerrainMask,
    rng: StageRng,
    random_trials: u8,
    start: u16,
    scanned: u16,
    progress: SearchProgress,
}
impl PlacementSearch {
    pub fn new(
        dimensions: Dimensions,
        footprint: Footprint,
        allowed: TerrainMask,
        rng: StageRng,
    ) -> Result<Self> {
        dimensions.validate()?;
        footprint.validate()?;
        allowed.validate()?;
        if !rng.valid() {
            return Err(Error::State("placement RNG"));
        }
        Ok(Self {
            dimensions,
            footprint,
            allowed,
            rng,
            random_trials: 0,
            start: 0,
            scanned: 0,
            progress: SearchProgress::Searching,
        })
    }
    pub const fn progress(&self) -> SearchProgress {
        self.progress
    }
    pub const fn dimensions(&self) -> Dimensions {
        self.dimensions
    }
    pub const fn rng(&self) -> StageRng {
        self.rng
    }
    /// A late actor/arrival conflict rejects this candidate without restarting
    /// the RNG/search or granting an unbounded retry loop.
    pub fn skip_found(&mut self) -> Result<()> {
        if !matches!(self.progress, SearchProgress::Found(_)) {
            return Err(Error::State("placement has no found candidate"));
        }
        self.progress = if usize::from(self.scanned) == self.dimensions.area() {
            SearchProgress::Exhausted
        } else {
            SearchProgress::Searching
        };
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.dimensions.validate()?;
        self.footprint.validate()?;
        self.allowed.validate()?;
        if !self.rng.valid()
            || self.random_trials > 10
            || usize::from(self.start) >= self.dimensions.area()
            || usize::from(self.scanned) > self.dimensions.area()
            || matches!(self.progress, SearchProgress::Exhausted)
                && usize::from(self.scanned) != self.dimensions.area()
            || matches!(self.progress, SearchProgress::Found((x,y)) if self.dimensions.world_cell(x,y).is_none())
        {
            return Err(Error::State("placement continuation"));
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        geometry: &LevelGeometry,
        level: LevelId,
        resources: &impl Resources,
        work: u32,
        exclusions: &[Exclusion],
    ) -> Result<SearchProgress> {
        if geometry.dimensions() != self.dimensions
            || !(1..=1024).contains(&work)
            || exclusions.len() > crate::MAX_ENTITIES
        {
            return Err(Error::Bounds("placement geometry/work/exclusions"));
        }
        for _ in 0..work {
            if self.progress != SearchProgress::Searching {
                break;
            }
            let mut rng = self.rng;
            let (index, random) = if self.random_trials < 10 {
                let row = rng.bounded(u32::from(self.dimensions.height()));
                let column = rng.bounded(u32::from(self.dimensions.width()));
                (
                    (row * u32::from(self.dimensions.width()) + column) as u16,
                    true,
                )
            } else {
                if self.scanned == 0 {
                    let row = rng.bounded(u32::from(self.dimensions.height()));
                    let column = rng.bounded(u32::from(self.dimensions.width()));
                    self.start = (row * u32::from(self.dimensions.width()) + column) as u16;
                }
                (
                    (((self.start / self.dimensions.width()
                        + self.scanned / self.dimensions.width())
                        % self.dimensions.height())
                        * self.dimensions.width()
                        + (self.start % self.dimensions.width()
                            + self.scanned % self.dimensions.width())
                            % self.dimensions.width()),
                    false,
                )
            };
            let cell = geometry.cells()[usize::from(index)];
            let placement = if geometry.path_state()[usize::from(index)] != 0 {
                None
            } else {
                position_in_cell(
                    cell,
                    self.footprint,
                    self.allowed,
                    false,
                    &mut rng,
                    resources,
                )?
            };
            self.rng = rng;
            if random {
                self.random_trials += 1;
            } else {
                self.scanned += 1;
            }
            if let Some((x, y)) = placement {
                let cell_position = self
                    .dimensions
                    .position(usize::from(index))
                    .expect("bounded index");
                let position = (
                    (i32::from(cell_position.x) + 1) * 128 + x,
                    (i32::from(cell_position.y) + 1) * 64 + y,
                );
                if !exclusions
                    .iter()
                    .any(|exclusion| exclusion.contains(level, position))
                {
                    self.progress = SearchProgress::Found(position);
                }
            }
            if self.progress == SearchProgress::Searching
                && usize::from(self.scanned) == self.dimensions.area()
            {
                self.progress = SearchProgress::Exhausted;
            }
        }
        Ok(self.progress)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum ElevatorDoor {
    Closed,
    Open,
    Inactive,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct ElevatorBarrier {
    pub level: LevelId,
    pub x: i32,
    pub y: i32,
    pub door: ElevatorDoor,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevatorContact {
    Clear,
    Blocked,
    Enter,
    Exit,
}
impl ElevatorBarrier {
    /// The original placement predicate intentionally includes inactive records.
    pub fn contains(self, level: LevelId, position: (i32, i32)) -> bool {
        self.level == level
            && (i64::from(position.0) - i64::from(self.x)).abs() < 20
            && (i64::from(position.1) - i64::from(self.y) - 5).abs() < 15
    }
    pub fn blocks_entity(self, level: LevelId, previous: (i32, i32), proposed: (i32, i32)) -> bool {
        self.door != ElevatorDoor::Inactive
            && self.level == level
            && (i64::from(proposed.0) - i64::from(self.x)).abs() < 20
            && (i64::from(proposed.1) - i64::from(self.y) - 5).abs() < 7
            && ((proposed.1 < previous.1 && previous.1 > self.y)
                || (proposed.1 > previous.1 && previous.1 < self.y))
    }
    /// Generalized ElevatorBlocksPlayerMovement; both height arguments and the
    /// rider identity are explicit rather than inferred from a two-player flag.
    pub fn player_contact(
        self,
        level: LevelId,
        previous: (i32, i32),
        proposed: (i32, i32),
        previous_height: i32,
        proposed_height: i32,
        riding: bool,
    ) -> ElevatorContact {
        if self.door == ElevatorDoor::Inactive || self.level != level {
            return ElevatorContact::Clear;
        }
        let dx = (i64::from(proposed.0) - i64::from(self.x)).abs();
        if riding {
            if self.door != ElevatorDoor::Open || proposed.1 <= self.y || dx != 0 {
                return ElevatorContact::Blocked;
            }
            if i64::from(proposed.1) >= i64::from(self.y) + 10 {
                return ElevatorContact::Exit;
            }
        } else if dx < 20 {
            if (previous.1 > self.y && proposed.1 < self.y)
                || (previous.1 < self.y && proposed.1 > self.y)
            {
                return ElevatorContact::Blocked;
            }
            let dy = (i64::from(proposed.1) - i64::from(self.y) - 5).abs();
            if dy < 7 {
                if previous_height > 0 {
                    return ElevatorContact::Blocked;
                }
                if self.door == ElevatorDoor::Open && dx < 10 && previous.1 > self.y {
                    if proposed_height == 0 && i64::from(proposed.1) < i64::from(self.y) + 8 {
                        return ElevatorContact::Enter;
                    }
                } else {
                    return ElevatorContact::Blocked;
                }
            }
        }
        ElevatorContact::Clear
    }
    pub fn placement_exclusion(self) -> Result<Exclusion> {
        Ok(Exclusion {
            level: self.level,
            x: self.x,
            y: self
                .y
                .checked_add(5)
                .ok_or(Error::Bounds("elevator exclusion Y"))?,
            half_width: 20,
            half_height: 15,
        })
    }
}
