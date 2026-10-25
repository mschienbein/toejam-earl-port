//! Live path consumption generalized from map/consume.rs and ProcessLand.
//! Marks/alternate/dirty flags are separate from final metatile words. Work is
//! deterministic and bounded; presentation never allocates authoritative paths.
use crate::{
    Cell, CellPosition, Dimensions, Error, LevelGeometry, Resources, Result, Shape,
    population::StableBits, random::StageRng,
};
use bincode::{Decode, Encode};

pub trait PathResources: Resources {
    fn path_shape(&self, metatile: u8) -> Result<Option<u8>>;
    fn edge_boundary(&self, boundary: u8, alternate: bool) -> Result<u8>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Direction {
    Up,
    Right,
    Down,
    Left,
}
impl Direction {
    pub const fn bit(self) -> u8 {
        match self {
            Self::Up => 1,
            Self::Right => 2,
            Self::Down => 4,
            Self::Left => 8,
        }
    }
    pub const fn opposite(self) -> Self {
        match self {
            Self::Up => Self::Down,
            Self::Right => Self::Left,
            Self::Down => Self::Up,
            Self::Left => Self::Right,
        }
    }
    fn offset(self) -> (i32, i32) {
        match self {
            Self::Up => (0, -1),
            Self::Right => (1, 0),
            Self::Down => (0, 1),
            Self::Left => (-1, 0),
        }
    }
    fn first(bits: u8) -> Option<Self> {
        [Self::Up, Self::Right, Self::Down, Self::Left]
            .into_iter()
            .find(|direction| bits & direction.bit() != 0)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct LandChange {
    pub position: CellPosition,
    pub before: Cell,
    pub after: Cell,
    pub direction: Option<Direction>,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
struct Collapse {
    start: CellPosition,
    current: CellPosition,
    direction: Option<Direction>,
    work: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct LandState {
    dimensions: Dimensions,
    marked: StableBits,
    alternate: StableBits,
    collapse: Option<Collapse>,
    last_change: Option<LandChange>,
}
impl LandState {
    pub fn new(dimensions: Dimensions) -> Result<Self> {
        dimensions.validate()?;
        Ok(Self {
            dimensions,
            marked: StableBits::new(dimensions.area() as u16)?,
            alternate: StableBits::new(dimensions.area() as u16)?,
            collapse: None,
            last_change: None,
        })
    }
    pub fn from_geometry(geometry: &LevelGeometry) -> Result<Self> {
        let mut value = Self::new(geometry.dimensions())?;
        for (index, directions) in geometry.path_state().iter().enumerate() {
            if *directions != 0 {
                value.mark(
                    geometry
                        .dimensions()
                        .position(index)
                        .expect("bounded path index"),
                    false,
                )?;
            }
        }
        Ok(value)
    }
    pub fn mark(&mut self, position: CellPosition, alternate: bool) -> Result<()> {
        let index = position.index(self.dimensions)? as u16;
        self.marked.insert(index)?;
        if alternate {
            self.alternate.insert(index)?;
        }
        Ok(())
    }
    pub fn marked(&self, position: CellPosition) -> bool {
        position
            .index(self.dimensions)
            .is_ok_and(|index| self.marked.contains(index as u16))
    }
    pub fn busy(&self) -> bool {
        self.collapse.is_some()
    }
    pub fn last_change(&self) -> Option<LandChange> {
        self.last_change
    }
    /// ProcessLand's cardinal-neighbor edge predicate; per-seat terrain cache is
    /// supplied explicitly, never inferred from an active camera bank.
    pub fn near_edge(terrain: &[u8; 25]) -> bool {
        [7, 11, 13, 17]
            .into_iter()
            .any(|index| terrain[index] >= 0x50)
    }
    pub fn begin(&mut self, position: CellPosition) -> Result<()> {
        position.index(self.dimensions)?;
        if self.busy() || !self.marked(position) {
            return Err(Error::State("path collapse busy/unmarked origin"));
        }
        self.collapse = Some(Collapse {
            start: position,
            current: position,
            direction: None,
            work: 0,
        });
        self.last_change = None;
        Ok(())
    }
    /// One cell per authoritative land step. The original allocator/animation
    /// waits are not simulation gates; a renderer consumes last_change instead.
    pub fn step(
        &mut self,
        geometry: &mut LevelGeometry,
        dirty: &mut StableBits,
        rng: &mut StageRng,
        resources: &impl PathResources,
    ) -> Result<Option<LandChange>> {
        if geometry.dimensions() != self.dimensions {
            return Err(Error::State("land geometry extent"));
        }
        let Some(collapse) = self.collapse.as_ref() else {
            return Ok(None);
        };
        if collapse.work >= self.dimensions.area() as u32 * 4 {
            return Err(Error::State("path traversal exceeds finite edge budget"));
        }
        let position = collapse.current;
        let index = position.index(self.dimensions)?;
        if !self.marked.contains(index as u16) {
            self.collapse = None;
            return Ok(None);
        }
        let before = geometry.cells()[index];
        let mut directions = geometry.path_state()[index];
        let mut direction = collapse.direction;
        let mut finished = false;
        let replacement_shape = if directions == 0 {
            finished = direction.is_some();
            let mut boundary = None;
            for candidate in [
                Direction::Up,
                Direction::Right,
                Direction::Down,
                Direction::Left,
            ] {
                let (dx, dy) = candidate.offset();
                let neighbor = self
                    .dimensions
                    .index(i32::from(position.x) + dx, i32::from(position.y) + dy)
                    .map_or(0, |index| geometry.path_state()[index]);
                if direction == Some(candidate.opposite())
                    || neighbor & candidate.opposite().bit() != 0
                {
                    if direction.is_none() {
                        direction = Some(candidate);
                    }
                    boundary = Some(
                        16 + match candidate {
                            Direction::Up => 0,
                            Direction::Right => 1,
                            Direction::Down => 2,
                            Direction::Left => 3,
                        },
                    );
                    break;
                }
            }
            let boundary = boundary.ok_or(Error::State("marked edge has no path neighbor"))?;
            resources.edge_boundary(boundary, self.alternate.contains(index as u16))?
        } else {
            let mut shape = resources
                .path_shape(before.base())?
                .ok_or(Error::Resource("marked path metatile has no shape"))?;
            if shape > 15 {
                return Err(Error::Resource("path shape mask"));
            }
            if let Some(incoming) = direction.map(Direction::opposite) {
                shape |= incoming.bit();
                directions &= !incoming.bit();
            }
            if direction.is_none_or(|candidate| directions & candidate.bit() == 0) {
                if let Some(next) = Direction::first(directions) {
                    direction = Some(next);
                } else if direction.is_none() {
                    self.collapse = None;
                    return Ok(None);
                } else {
                    finished = true;
                }
            }
            if !finished {
                shape |= direction.expect("outgoing path").bit();
            }
            directions &= !shape;
            resources.shape(Shape::Path, shape, false)?
        };
        let mut next_rng = *rng;
        let replacement =
            resources.alternative(replacement_shape, u32::from(next_rng.next_u16()))?;
        let after = Cell::from_word((before.word() & 0x1f00) | u16::from(replacement.base()))?;
        geometry.replace_cell(position, after, directions)?;
        dirty.insert(index as u16)?;
        if directions == 0 {
            self.marked.remove(index as u16)?;
        }
        *rng = next_rng;
        let change = LandChange {
            position,
            before,
            after,
            direction,
        };
        let collapse = self.collapse.as_mut().expect("active collapse");
        collapse.work += 1;
        collapse.direction = direction;
        if finished {
            collapse.current = collapse.start;
            collapse.direction = None;
            if !self
                .marked
                .contains(collapse.start.index(self.dimensions)? as u16)
            {
                self.collapse = None;
            }
        } else if let Some(direction) = direction {
            let (dx, dy) = direction.offset();
            if let Some(index) = self
                .dimensions
                .index(i32::from(position.x) + dx, i32::from(position.y) + dy)
            {
                collapse.current = self.dimensions.position(index).expect("bounded neighbor");
            } else {
                self.collapse = None;
            }
        } else {
            self.collapse = None;
        }
        self.last_change = Some(change);
        Ok(Some(change))
    }
    pub fn validate(&self, geometry: &LevelGeometry) -> Result<()> {
        self.dimensions.validate()?;
        if self.dimensions != geometry.dimensions() {
            return Err(Error::State("land geometry extent"));
        }
        self.marked.validate(self.dimensions.area() as u16)?;
        self.alternate.validate(self.dimensions.area() as u16)?;
        for (index, directions) in geometry.path_state().iter().enumerate() {
            if *directions != 0 && !self.marked.contains(index as u16) {
                return Err(Error::State("unmarked live path directions"));
            }
        }
        if let Some(collapse) = &self.collapse {
            collapse.start.index(self.dimensions)?;
            collapse.current.index(self.dimensions)?;
            if collapse.work > self.dimensions.area() as u32 * 4 {
                return Err(Error::State("land work budget"));
            }
        }
        if let Some(change) = self.last_change {
            change.position.index(self.dimensions)?;
            change.before.validate()?;
            change.after.validate()?;
        }
        Ok(())
    }
}
