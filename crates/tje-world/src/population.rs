use crate::{Anchors, Dimensions};
use crate::{
    Density, Error, MAX_EARTHLINGS, MAX_ITEMS, Result, geometry::TileMask, random::StageRng,
};
use bincode::{Decode, Encode};

/// Measured from an accepted stock/control floor, not from the enlarged
/// envelope or camera area. It participates in the generation identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct PopulationBaseline {
    pub walkable_tiles: u32,
    pub ordinary_items: u16,
    pub earthlings: u16,
    pub reserved_items: u8,
    pub ship_piece: bool,
    pub mailbox: bool,
}
impl PopulationBaseline {
    pub fn validate(self) -> Result<()> {
        if self.walkable_tiles == 0
            || self.walkable_tiles > 19 * 26 * 128
            || self.ordinary_items > 28
            || self.earthlings > 20
            || self.reserved_items > 4
            || (self.ship_piece && self.reserved_items == 0)
            || (self.mailbox && self.earthlings == 0)
        {
            return Err(Error::Bounds("measured stock population baseline"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct PopulationBudget {
    pub items: u16,
    pub earthlings: u16,
    pub reserved_items: u8,
    pub promotion_numerator: u32,
    pub promotion_denominator: u32,
}
impl PopulationBudget {
    pub fn calculate(
        baseline: PopulationBaseline,
        walkable_tiles: u32,
        density: Density,
    ) -> Result<Self> {
        baseline.validate()?;
        if walkable_tiles == 0 || walkable_tiles > 64 * 64 * 128 {
            return Err(Error::Bounds("walkable tile area"));
        }
        let scale = |count: u16, cap: usize| -> u16 {
            let numerator =
                u64::from(count) * u64::from(walkable_tiles) * u64::from(density.percent());
            let denominator = u64::from(baseline.walkable_tiles) * 100;
            ((numerator + denominator / 2) / denominator).min(cap as u64) as u16
        };
        let reserved = u16::from(baseline.reserved_items);
        let (items, earthlings) = if density == Density::Original {
            (baseline.ordinary_items + reserved, baseline.earthlings)
        } else {
            let ordinary = scale(baseline.ordinary_items, MAX_ITEMS - usize::from(reserved));
            let mailbox = u16::from(baseline.mailbox);
            let actors = scale(
                baseline.earthlings - mailbox,
                MAX_EARTHLINGS - usize::from(mailbox),
            ) + mailbox;
            (ordinary + reserved, actors)
        };
        Ok(Self {
            items,
            earthlings,
            reserved_items: baseline.reserved_items,
            promotion_numerator: if density == Density::Original {
                baseline.walkable_tiles
            } else {
                walkable_tiles
            },
            promotion_denominator: baseline.walkable_tiles,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct StableBits {
    limit: u16,
    words: Vec<u64>,
}
impl StableBits {
    pub fn new(limit: u16) -> Result<Self> {
        if limit > 64 * 64 {
            return Err(Error::Bounds("stable bitset extent"));
        }
        Ok(Self {
            limit,
            words: vec![0; usize::from(limit).div_ceil(64)],
        })
    }
    pub fn contains(&self, id: u16) -> bool {
        id < self.limit && self.words[usize::from(id / 64)] & (1 << (id % 64)) != 0
    }
    pub fn insert(&mut self, id: u16) -> Result<bool> {
        if id >= self.limit {
            return Err(Error::Bounds("stable entity ID"));
        }
        let old = self.contains(id);
        self.words[usize::from(id / 64)] |= 1 << (id % 64);
        Ok(!old)
    }
    pub fn remove(&mut self, id: u16) -> Result<bool> {
        if id >= self.limit {
            return Err(Error::Bounds("stable entity ID"));
        }
        let old = self.contains(id);
        self.words[usize::from(id / 64)] &= !(1 << (id % 64));
        Ok(old)
    }
    pub fn count(&self) -> u32 {
        self.words.iter().map(|word| word.count_ones()).sum()
    }
    pub fn validate(&self, limit: u16) -> Result<()> {
        if limit > 64 * 64 {
            return Err(Error::Bounds("stable bitset extent"));
        }
        if self.limit != limit
            || self.words.len() != usize::from(limit).div_ceil(64)
            || (!limit.is_multiple_of(64)
                && self
                    .words
                    .last()
                    .is_some_and(|word| *word >> (limit % 64) != 0))
        {
            return Err(Error::State("stable-ID bitset extent/padding"));
        }
        Ok(())
    }
}

/// Placement roles deliberately do not invent cartridge actor/item kinds. The
/// future mode-owned loader resolves these requests against imported definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum PlacementRole {
    Resource,
    Service,
    ShipPiece,
    Earthling,
    Mailbox,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Placement {
    pub id: u16,
    pub x: i32,
    pub y: i32,
    pub role: PlacementRole,
    pub dormant_ticks: u32,
    pub ai_rng: StageRng,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Population {
    pub budget: PopulationBudget,
    pub items: Vec<Placement>,
    pub earthlings: Vec<Placement>,
    pub collected: StableBits,
    pub removed: StableBits,
}
impl Population {
    pub fn validate(&self, dimensions: Dimensions) -> Result<()> {
        if self.items.len() != usize::from(self.budget.items)
            || self.earthlings.len() != usize::from(self.budget.earthlings)
        {
            return Err(Error::State("population completeness"));
        }
        self.validate_partial(dimensions)
    }
    fn validate_partial(&self, dimensions: Dimensions) -> Result<()> {
        if usize::from(self.budget.items) > MAX_ITEMS
            || usize::from(self.budget.earthlings) > MAX_EARTHLINGS
            || self.items.len() > usize::from(self.budget.items)
            || self.earthlings.len() > usize::from(self.budget.earthlings)
            || usize::from(self.budget.reserved_items) > self.items.len()
            || self.budget.reserved_items > 4
            || self.budget.promotion_numerator == 0
            || self.budget.promotion_numerator > 64 * 64 * 128
            || self.budget.promotion_denominator == 0
            || self.budget.promotion_denominator > 19 * 26 * 128
        {
            return Err(Error::State("population budget/extent"));
        }
        self.collected.validate(MAX_ITEMS as u16)?;
        self.removed.validate(MAX_EARTHLINGS as u16)?;
        if (self.items.len() as u16..MAX_ITEMS as u16).any(|id| self.collected.contains(id))
            || (self.earthlings.len() as u16..MAX_EARTHLINGS as u16)
                .any(|id| self.removed.contains(id))
        {
            return Err(Error::State(
                "collection/removal ID has no persistent record",
            ));
        }
        for (actors, list) in [(false, &self.items), (true, &self.earthlings)] {
            for (index, placement) in list.iter().enumerate() {
                if usize::from(placement.id) != index
                    || dimensions.world_cell(placement.x, placement.y).is_none()
                    || !placement.ai_rng.valid()
                    || actors
                        != matches!(
                            placement.role,
                            PlacementRole::Earthling | PlacementRole::Mailbox
                        )
                    || (!actors
                        && (index < usize::from(self.budget.reserved_items))
                            != matches!(
                                placement.role,
                                PlacementRole::Service | PlacementRole::ShipPiece
                            ))
                    || (actors && placement.role == PlacementRole::Mailbox && index != 0)
                {
                    return Err(Error::State("stable placement identity/position/role"));
                }
            }
        }
        Ok(())
    }
    /// Simulation interest has no presentation/camera input. Dormant timers pause
    /// in ExpandedV1; the caller advances only IDs returned in this stable order.
    pub fn active_earthlings(&self, players: &[(i32, i32)], output: &mut Vec<u16>) {
        output.clear();
        for actor in &self.earthlings {
            if !self.removed.contains(actor.id)
                && (actor.role == PlacementRole::Mailbox
                    || players.iter().any(|&(x, y)| {
                        (i64::from(actor.x) - i64::from(x)).abs() <= 480
                            && (i64::from(actor.y) - i64::from(y)).abs() <= 336
                    }))
            {
                output.push(actor.id);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub(crate) struct PlacementBuilder {
    pub population: Population,
    rng: StageRng,
    ai: StageRng,
    attempts: u32,
    first: u32,
    cursor: u32,
}
impl PlacementBuilder {
    pub fn new(
        budget: PopulationBudget,
        baseline: PopulationBaseline,
        anchors: &Anchors,
        rng: StageRng,
        ai: StageRng,
    ) -> Result<Self> {
        let mut value = Self {
            population: Population {
                budget,
                items: Vec::with_capacity(usize::from(budget.items)),
                earthlings: Vec::with_capacity(usize::from(budget.earthlings)),
                collected: StableBits::new(MAX_ITEMS as u16)?,
                removed: StableBits::new(MAX_EARTHLINGS as u16)?,
            },
            rng,
            ai,
            attempts: 0,
            first: 0,
            cursor: 0,
        };
        for index in 0..budget.reserved_items {
            let position = if index == 0 && baseline.ship_piece {
                anchors
                    .ship_piece
                    .ok_or(Error::State("ship placement anchor"))?
            } else {
                anchors.regions[usize::from(index) % anchors.regions.len()]
            };
            let (x, y) = position.center();
            value.push(
                false,
                x,
                y,
                if index == 0 && baseline.ship_piece {
                    PlacementRole::ShipPiece
                } else {
                    PlacementRole::Service
                },
            );
        }
        if baseline.mailbox {
            let (x, y) = anchors.arrival.center();
            value.push(true, x, y, PlacementRole::Mailbox);
        }
        Ok(value)
    }
    fn push(&mut self, actor: bool, x: i32, y: i32, role: PlacementRole) {
        let list = if actor {
            &mut self.population.earthlings
        } else {
            &mut self.population.items
        };
        let rng =
            StageRng::new((u64::from(self.ai.next_u16()) << 16) | u64::from(self.ai.next_u16()));
        list.push(Placement {
            id: list.len() as u16,
            x,
            y,
            role,
            dormant_ticks: 0,
            ai_rng: rng,
        });
        self.attempts = 0;
    }
    /// One deterministic candidate, at most one metatile's footprint checks.
    /// Returns Some(success) only once the population is complete or impossible.
    pub fn advance(
        &mut self,
        dimensions: Dimensions,
        masks: &[TileMask],
        arrival: crate::CellPosition,
    ) -> Option<bool> {
        let actor = self.population.items.len() == usize::from(self.population.budget.items);
        if actor
            && self.population.earthlings.len() == usize::from(self.population.budget.earthlings)
        {
            return Some(true);
        }
        let candidates = dimensions.area() as u32 * 4;
        if self.attempts == 0 {
            self.first = self.rng.bounded(dimensions.area() as u32) * 4 + self.rng.bounded(4);
            self.cursor = self.first;
        }
        if self.attempts >= candidates {
            return Some(false);
        }
        let candidate = self.cursor;
        self.cursor = (self.cursor + 1) % candidates;
        self.attempts += 1;
        let index = (candidate / 4) as usize;
        let cell = dimensions.position(index).expect("bounded candidate");
        let tx = if candidate & 1 == 0 { 4 } else { 12 };
        let ty = if candidate & 2 == 0 { 2 } else { 6 };
        let x = (i32::from(cell.x) + 1) * 128 + tx * 8;
        let y = (i32::from(cell.y) + 1) * 64 + ty * 8;
        let mask = masks[index];
        let mut legal = true;
        for dy in -1..=0 {
            for dx in -2..=1 {
                legal &= mask.contains(((ty + dy) * 16 + tx + dx) as usize);
            }
        }
        let (arrival_x, arrival_y) = arrival.center();
        if actor && (x - arrival_x).abs() < 320 && (y - arrival_y).abs() < 192 {
            legal = false;
        }
        let spacing = if actor { (96, 48) } else { (32, 16) };
        let list = if actor {
            &self.population.earthlings
        } else {
            &self.population.items
        };
        if list
            .iter()
            .any(|other| (x - other.x).abs() < spacing.0 && (y - other.y).abs() < spacing.1)
        {
            legal = false;
        }
        if actor
            && list
                .iter()
                .filter(|other| {
                    other.role == PlacementRole::Earthling
                        && (x - other.x).abs() <= 256
                        && (y - other.y).abs() <= 128
                })
                .count()
                >= 4
        {
            legal = false;
        }
        if legal {
            self.push(
                actor,
                x,
                y,
                if actor {
                    PlacementRole::Earthling
                } else {
                    PlacementRole::Resource
                },
            );
        }
        None
    }
    pub fn validate(&self, dimensions: Dimensions) -> Result<()> {
        if self.attempts > dimensions.area() as u32 * 4
            || self.cursor >= dimensions.area() as u32 * 4
            || self.first >= dimensions.area() as u32 * 4
            || !self.rng.valid()
            || !self.ai.valid()
            || self.population.items.len() > usize::from(self.population.budget.items)
            || self.population.earthlings.len() > usize::from(self.population.budget.earthlings)
        {
            return Err(Error::State("pending placement cursor/budget"));
        }
        self.population.validate_partial(dimensions)?;
        Ok(())
    }
}
