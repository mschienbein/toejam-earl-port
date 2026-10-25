use crate::{Error, LevelId, Result};
use bincode::{Decode, Encode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Dimensions {
    width: u16,
    height: u16,
}
impl Dimensions {
    pub const ORIGINAL: Self = Self {
        width: 19,
        height: 26,
    };
    pub const EXPANDED: Self = Self {
        width: 28,
        height: 39,
    };
    pub const LARGE: Self = Self {
        width: 38,
        height: 52,
    };
    pub fn new(width: u16, height: u16) -> Result<Self> {
        let value = Self { width, height };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(self) -> Result<()> {
        if (8..=64).contains(&self.width) && (8..=64).contains(&self.height) {
            Ok(())
        } else {
            Err(Error::Bounds("each map axis must be 8..=64 cells"))
        }
    }
    pub const fn width(self) -> u16 {
        self.width
    }
    pub const fn height(self) -> u16 {
        self.height
    }
    pub const fn area(self) -> usize {
        self.width as usize * self.height as usize
    }
    pub const fn world_envelope(self) -> (i32, i32) {
        ((self.width as i32 + 2) * 128, (self.height as i32 + 2) * 64)
    }
    pub fn index(self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= i32::from(self.width) || y >= i32::from(self.height) {
            return None;
        }
        (y as usize)
            .checked_mul(usize::from(self.width))?
            .checked_add(x as usize)
    }
    pub fn position(self, index: usize) -> Option<CellPosition> {
        (index < self.area()).then(|| CellPosition {
            x: (index % usize::from(self.width)) as u16,
            y: (index / usize::from(self.width)) as u16,
        })
    }
    pub fn world_cell(self, x: i32, y: i32) -> Option<CellPosition> {
        if x < 128 || y < 64 {
            return None;
        }
        let x = x / 128 - 1;
        let y = y / 64 - 1;
        self.index(x, y)?;
        Some(CellPosition {
            x: x as u16,
            y: y as u16,
        })
    }
    pub fn camera_bounds(
        self,
        viewport_width: u16,
        viewport_height: u16,
    ) -> Result<((i32, i32), (i32, i32))> {
        let (width, height) = self.world_envelope();
        if viewport_width == 0
            || viewport_height == 0
            || i32::from(viewport_width) + 16 > width
            || i32::from(viewport_height) + 16 > height
        {
            return Err(Error::Bounds("viewport exceeds world envelope"));
        }
        Ok((
            (8, width - i32::from(viewport_width) - 8),
            (8, height - i32::from(viewport_height) - 8),
        ))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct CellPosition {
    pub x: u16,
    pub y: u16,
}
impl CellPosition {
    pub const fn center(self) -> (i32, i32) {
        (
            (self.x as i32 + 1) * 128 + 64,
            (self.y as i32 + 1) * 64 + 32,
        )
    }
    pub fn index(self, dimensions: Dimensions) -> Result<usize> {
        dimensions
            .index(i32::from(self.x), i32::from(self.y))
            .ok_or(Error::Bounds("cell position"))
    }
}

/// Final imported metatile identity only. Generation labels never inhabit this
/// type. Original pending-path/item-inhibited flags live in separate sidecars.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct Cell(u16);
impl Cell {
    pub fn from_word(word: u16) -> Result<Self> {
        if word & 0xe000 != 0 || word as u8 & 0x7f >= 122 || (word >> 8) & 31 == 16 {
            return Err(Error::InvalidCell(word));
        }
        Ok(Self(word))
    }
    pub fn new(base: u8, overlay: u8) -> Result<Self> {
        Self::from_word(u16::from(base) | (u16::from(overlay) << 8))
    }
    pub const fn word(self) -> u16 {
        self.0
    }
    pub const fn base(self) -> u8 {
        self.0 as u8
    }
    pub const fn overlay(self) -> u8 {
        ((self.0 >> 8) & 31) as u8
    }
    pub fn validate(self) -> Result<()> {
        Self::from_word(self.0).map(|_| ())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Anchors {
    pub arrival: CellPosition,
    pub exit: CellPosition,
    pub ship_piece: Option<CellPosition>,
    pub regions: Vec<CellPosition>,
}
impl Anchors {
    pub fn validate(&self, dimensions: Dimensions) -> Result<()> {
        self.arrival.index(dimensions)?;
        self.exit.index(dimensions)?;
        if self.regions.len() > 8 {
            return Err(Error::State("anchor count"));
        }
        if let Some(position) = self.ship_piece {
            position.index(dimensions)?;
        }
        for position in &self.regions {
            position.index(dimensions)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Shape {
    Path,
    Edge,
    Filled,
}

/// Import-only resource interface, also shared by terrain queries and generation.
/// No art, terrain tables or alternative lists are embedded in public fixtures.
pub trait Resources {
    fn parameter(&self, level: LevelId, parameter: u8) -> Result<(u8, u8)>;
    fn shape(&self, kind: Shape, corners: u8, alternate: bool) -> Result<u8>;
    fn alternative(&self, shape: u8, draw: u32) -> Result<Cell>;
    /// One descriptor lookup, with bounded shape-list traversal. None means the
    /// shape is not a member; it is never an unspecified replacement cell.
    fn transition(&self, shape: u8, descriptor: u8, draw: u32) -> Result<Option<Cell>>;
    fn base_tile(&self, base: u8, row: u8, column: u8) -> Result<u16>;
    fn overlay_tile(&self, overlay: u8, row: u8, column: u8) -> Result<u16>;
    fn property(&self, tile: u16) -> Result<u8>;
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct LevelGeometry {
    dimensions: Dimensions,
    cells: Vec<Cell>,
    path_state: Vec<u8>,
    anchors: Anchors,
}
impl LevelGeometry {
    pub fn new(
        dimensions: Dimensions,
        cells: Vec<Cell>,
        path_state: Vec<u8>,
        anchors: Anchors,
    ) -> Result<Self> {
        let value = Self {
            dimensions,
            cells,
            path_state,
            anchors,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        self.dimensions.validate()?;
        if self.cells.len() != self.dimensions.area() || self.path_state.len() != self.cells.len() {
            return Err(Error::State("geometry/path extent"));
        }
        for cell in &self.cells {
            cell.validate()?;
        }
        if self.path_state.iter().any(|value| *value > 15) {
            return Err(Error::State("path direction bits"));
        }
        self.anchors.validate(self.dimensions)
    }
    pub const fn dimensions(&self) -> Dimensions {
        self.dimensions
    }
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }
    pub fn path_state(&self) -> &[u8] {
        &self.path_state
    }
    pub fn anchors(&self) -> &Anchors {
        &self.anchors
    }
    pub fn cell(&self, x: i32, y: i32) -> Option<Cell> {
        self.cells.get(self.dimensions.index(x, y)?).copied()
    }
    pub fn replace_cell(&mut self, position: CellPosition, cell: Cell, path: u8) -> Result<()> {
        cell.validate()?;
        if path > 15 {
            return Err(Error::State("path direction bits"));
        }
        let index = position.index(self.dimensions)?;
        self.cells[index] = cell;
        self.path_state[index] = path;
        Ok(())
    }
    pub fn terrain_at(
        &self,
        level: LevelId,
        x: i32,
        y: i32,
        resources: &impl Resources,
    ) -> Result<u8> {
        let Some(position) = self.dimensions.world_cell(x, y) else {
            return Ok(if matches!(level.get(), 1 | 26) {
                0x80
            } else {
                0x90
            });
        };
        let cell = self.cells[position.index(self.dimensions)?];
        let row = ((y % 64) / 8) as u8;
        let column = ((x % 128) / 8) as u8;
        let mut tile = 0;
        if cell.overlay() != 0 {
            tile = resources.overlay_tile(cell.overlay(), row, column)?;
        }
        if tile == 0 {
            tile = resources.base_tile(cell.base(), row, column)?;
        }
        resources.property(tile)
    }
}

/// Exact 16x8-tile terrain reachability within one metatile. Kept separate from
/// coarse generation classes; worklists elsewhere contain at most one entry per
/// cell, while each local expansion uses only this fixed 128-tile mask.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub(crate) struct TileMask(pub [u64; 2]);
impl TileMask {
    pub fn contains(self, bit: usize) -> bool {
        self.0[bit / 64] & (1 << (bit % 64)) != 0
    }
    pub fn insert(&mut self, bit: usize) -> bool {
        let old = self.contains(bit);
        self.0[bit / 64] |= 1 << (bit % 64);
        !old
    }
    pub fn count(self) -> u32 {
        self.0.iter().map(|word| word.count_ones()).sum()
    }
    pub fn flood(self, mut reached: Self) -> Self {
        reached.0[0] &= self.0[0];
        reached.0[1] &= self.0[1];
        let mut queue = [0_u8; 128];
        let mut length = 0;
        for bit in 0..128 {
            if reached.contains(bit) {
                queue[length] = bit as u8;
                length += 1;
            }
        }
        let mut cursor = 0;
        while cursor < length {
            let bit = usize::from(queue[cursor]);
            cursor += 1;
            let neighbors = [
                (bit % 16 > 0).then(|| bit - 1),
                (bit % 16 < 15).then_some(bit + 1),
                (bit >= 16).then(|| bit - 16),
                (bit < 112).then_some(bit + 16),
            ];
            for next in neighbors.into_iter().flatten() {
                if self.contains(next) && reached.insert(next) {
                    queue[length] = next as u8;
                    length += 1;
                }
            }
        }
        reached
    }
}

pub(crate) fn walkable_mask(cell: Cell, resources: &impl Resources) -> Result<TileMask> {
    let mut mask = TileMask::default();
    for row in 0..8 {
        for column in 0..16 {
            let mut tile = 0;
            if cell.overlay() != 0 {
                tile = resources.overlay_tile(cell.overlay(), row, column)?;
            }
            if tile == 0 {
                tile = resources.base_tile(cell.base(), row, column)?;
            }
            if resources.property(tile)? & 0xf0 == 0x10 {
                mask.insert(usize::from(row) * 16 + usize::from(column));
            }
        }
    }
    Ok(mask)
}
