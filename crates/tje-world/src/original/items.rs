//! Stock item slots and first-two-record clearance, factored from readable
//! level_item_setup and spawning_and_banks bodies. No bank/entity RAM is used.
use super::{OriginalRng, geometry::OriginalGeometry, resources::OriginalResources};
use crate::{
    Dimensions, Error, LevelId, Result,
    terrain::{Footprint, TerrainMask, position_in_cell},
};
use bincode::{Decode, Encode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct OriginalItem {
    pub slot: u8,
    pub kind: i8,
    pub level: LevelId,
    pub x: i32,
    pub y: i32,
}
impl OriginalItem {
    fn empty(slot: u8, level: LevelId) -> Self {
        Self {
            slot,
            kind: -1,
            level,
            x: 0,
            y: 0,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct StockSpecialPositions {
    pub positions: [Option<(i32, i32)>; 2],
    pub mail_order: bool,
    pub failed: u8,
}
impl StockSpecialPositions {
    pub fn prepare(
        level: LevelId,
        seed: u16,
        geometry: &crate::LevelGeometry,
        ship: Option<(i32, i32)>,
        resources: &impl crate::Resources,
    ) -> Result<Self> {
        let mut job = StockSpecialJob::new(level, seed, geometry, ship)?;
        while !job.advance(geometry, resources, 256)? {}
        job.finish()
    }
    pub fn is_near(&self, x: i32, y: i32) -> bool {
        self.positions.iter().enumerate().any(|(index, position)| {
            position.is_some_and(|(px, py)| {
                (index != 0 || self.mail_order)
                    && (i64::from(x) - i64::from(px)).abs() < 340
                    && (i64::from(y) - i64::from(py)).abs() < 200
            })
        })
    }
    pub fn validate(&self) -> Result<()> {
        if self.failed & !3 != 0 {
            return Err(Error::State("OriginalV1 special failure mask"));
        }
        for (index, position) in self.positions.iter().enumerate() {
            if let Some((x, y)) = position
                && (self.failed & (1 << index) != 0
                    || Dimensions::ORIGINAL.world_cell(*x, *y).is_none())
            {
                return Err(Error::State("OriginalV1 special prefix position/failure"));
            }
        }
        Ok(())
    }
}

/// Candidate-sized continuation for the seeded mailbox/ship prefix. The
/// standalone helper above drives this same owner, not a second search.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct StockSpecialJob {
    level: LevelId,
    seed: u16,
    rng: OriginalRng,
    result: StockSpecialPositions,
    random_trials: u8,
    scan_start: Option<(u16, u16)>,
    scanned: u16,
    complete: bool,
}
impl StockSpecialJob {
    pub fn new(
        level: LevelId,
        seed: u16,
        geometry: &crate::LevelGeometry,
        ship: Option<(i32, i32)>,
    ) -> Result<Self> {
        level.validate()?;
        if level.get() > 26
            || geometry.dimensions() != Dimensions::ORIGINAL
            || ship.is_some_and(|(x, y)| geometry.dimensions().world_cell(x, y).is_none())
        {
            return Err(Error::Bounds("OriginalV1 special prefix extent/ship"));
        }
        let complete = level.get() <= 1 || level.get() == 26;
        let mut rng = OriginalRng::new(u32::from(seed));
        let mail_order =
            complete || level.get() < 4 || level.get() == 12 || rng.next_u16() % 100 >= 50;
        Ok(Self {
            level,
            seed,
            rng,
            result: StockSpecialPositions {
                positions: [None, if complete { None } else { ship }],
                mail_order,
                failed: 0,
            },
            random_trials: 0,
            scan_start: None,
            scanned: 0,
            complete,
        })
    }
    pub const fn level(&self) -> LevelId {
        self.level
    }
    pub const fn seed(&self) -> u16 {
        self.seed
    }
    pub const fn ship_position(&self) -> Option<(i32, i32)> {
        self.result.positions[1]
    }
    pub fn validate(&self) -> Result<()> {
        self.level.validate()?;
        self.result.validate()?;
        let mut initial = OriginalRng::new(u32::from(self.seed));
        let special = self.level.get() <= 1 || self.level.get() == 26;
        let mail_order = special
            || self.level.get() < 4
            || self.level.get() == 12
            || initial.next_u16() % 100 >= 50;
        if self.result.mail_order != mail_order
            || special
                && (self.result.positions != [None; 2] || self.result.failed != 0 || !self.complete)
        {
            return Err(Error::State("OriginalV1 special seed/mode identity"));
        }
        if self.level.get() > 26
            || self.random_trials > 10
            || self.scanned > 494
            || self
                .scan_start
                .is_some_and(|(row, column)| row >= 26 || column >= 19)
            || self.scanned != 0 && self.scan_start.is_none()
            || self.scan_start.is_some() && self.random_trials != 10
            || !self.complete && (self.result.positions[0].is_some() || self.result.failed != 0)
            || self.result.failed & !1 != 0
            || !self.complete && self.scanned == 494
            || !special
                && self.complete
                && (self.result.positions[0].is_none() != (self.result.failed & 1 != 0))
            || special
                && (self.random_trials != 0 || self.scan_start.is_some() || self.scanned != 0)
        {
            return Err(Error::State("OriginalV1 special prefix continuation"));
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        geometry: &crate::LevelGeometry,
        resources: &impl crate::Resources,
        work: u32,
    ) -> Result<bool> {
        self.validate()?;
        if !(1..=1024).contains(&work) || geometry.dimensions() != Dimensions::ORIGINAL {
            return Err(Error::Bounds("OriginalV1 special prefix work"));
        }
        for _ in 0..work {
            if self.complete {
                break;
            }
            let random = self.random_trials < 10;
            let mut rng = self.rng;
            let (row, column, start) = if random {
                (
                    rng.bounded(26) as u16,
                    rng.bounded(19) as u16,
                    self.scan_start,
                )
            } else {
                let (row, column) = self
                    .scan_start
                    .unwrap_or_else(|| (rng.bounded(26) as u16, rng.bounded(19) as u16));
                (
                    (row + self.scanned / 19) % 26,
                    (column + self.scanned % 19) % 19,
                    Some((row, column)),
                )
            };
            let candidate =
                special_candidate(self.level, column, row, geometry, resources, &mut rng)?;
            self.rng = rng;
            self.scan_start = start;
            if random {
                self.random_trials += 1;
            } else {
                self.scanned += 1;
            }
            if let Some(position) = candidate {
                self.result.positions[0] = Some(position);
                self.complete = true;
            } else if self.scanned == 494 {
                self.result.failed |= 1;
                self.complete = true;
            }
        }
        Ok(self.complete)
    }
    pub fn finish(self) -> Result<StockSpecialPositions> {
        self.validate()?;
        if !self.complete {
            return Err(Error::State("OriginalV1 special prefix is incomplete"));
        }
        Ok(self.result)
    }
}
fn special_candidate(
    level: LevelId,
    column: u16,
    row: u16,
    geometry: &crate::LevelGeometry,
    resources: &impl crate::Resources,
    rng: &mut OriginalRng,
) -> Result<Option<(i32, i32)>> {
    let cell = geometry
        .cell(i32::from(column), i32::from(row))
        .ok_or(Error::Bounds("OriginalV1 mailbox cell"))?;
    let Some((x, y)) = position_in_cell(
        cell,
        Footprint::new(8, 2)?,
        TerrainMask::LAND_OR_SAND,
        false,
        rng,
        resources,
    )?
    else {
        return Ok(None);
    };
    let position = (
        (i32::from(column) + 1) * 128 + x,
        (i32::from(row) + 1) * 64 + y,
    );
    Ok(elevator_allows(level, position, geometry).then_some(position))
}
fn elevator_allows(level: LevelId, position: (i32, i32), geometry: &crate::LevelGeometry) -> bool {
    level.get() == 1
        || [geometry.anchors().arrival, geometry.anchors().exit]
            .iter()
            .all(|anchor| {
                let x = (i32::from(anchor.x) + 2) * 128 - 16;
                let y = (i32::from(anchor.y) + 2) * 64;
                (i64::from(position.0) - i64::from(x)).abs() >= 30
                    || (i64::from(position.1) - i64::from(y)).abs() >= 20
            })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum ItemPhase {
    Placement,
    Kind,
    Present,
    LargePlacement,
    LargeValidation,
    LargeKind,
    Complete,
    Rejected,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
struct ItemPlacement {
    large: bool,
    random_failures: u8,
    first_row: u16,
    first_column: u16,
    scanned: u16,
    started_scan: bool,
}
impl ItemPlacement {
    fn new(large: bool) -> Self {
        Self {
            large,
            random_failures: 0,
            first_row: 0,
            first_column: 0,
            scanned: 0,
            started_scan: false,
        }
    }
    fn step(
        &mut self,
        geometry: &OriginalGeometry,
        resources: &impl OriginalResources,
        rng: &mut OriginalRng,
    ) -> Result<Option<(i32, i32)>> {
        let random = self.random_failures < 10;
        let (row, column) = if random {
            (rng.bounded(26) as u16, rng.bounded(19) as u16)
        } else {
            if !self.started_scan {
                self.first_row = rng.bounded(26) as u16;
                self.first_column = rng.bounded(19) as u16;
                self.started_scan = true;
            }
            if self.scanned == 19 * 26 {
                return Err(Error::Resource(
                    "OriginalV1 exhausted item placement without initialized coordinates",
                ));
            }
            (
                ((self.first_row + self.scanned / 19) % 26),
                ((self.first_column + self.scanned % 19) % 19),
            )
        };
        let index = usize::from(row) * 19 + usize::from(column);
        let raw = geometry.raw_cell(index)?;
        let excluded =
            raw & 0x2000 != 0 || (self.large && resources.original_large_excluded(raw)?);
        let position = if excluded {
            None
        } else {
            let footprint = if self.large {
                Footprint::new(4, 2)?
            } else {
                Footprint::new(3, 1)?
            };
            if let Some((x, y)) = position_in_cell(
                geometry.geometry.cells()[index],
                footprint,
                TerrainMask::LAND_OR_SAND,
                false,
                rng,
                resources,
            )? {
                let position = (
                    (i32::from(column) + 1) * 128 + x,
                    (i32::from(row) + 1) * 64 + y,
                );
                elevator_allows(geometry.spec.level, position, &geometry.geometry)
                    .then_some(position)
            } else {
                None
            }
        };
        if random {
            if !excluded && position.is_none() {
                self.random_failures += 1;
            }
        } else {
            self.scanned += 1;
        }
        Ok(position)
    }
    fn validate(&self) -> Result<()> {
        if self.random_failures > 10
            || self.first_row >= 26
            || self.first_column >= 19
            || self.scanned > 494
            || (!self.started_scan && self.scanned != 0)
        {
            return Err(Error::State("OriginalV1 item placement continuation"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct OriginalContent {
    pub level: LevelId,
    pub seed: u16,
    pub geometry_identity: [u8; 32],
    pub two_player_baseline: bool,
    pub specials: StockSpecialPositions,
    pub items: [OriginalItem; 32],
}
impl OriginalContent {
    pub fn validate(&self, geometry: &OriginalGeometry) -> Result<()> {
        geometry.validate()?;
        self.specials.validate()?;
        if self.level != geometry.spec.level
            || self.seed != geometry.accepted_seed
            || self.geometry_identity != geometry.identity()
        {
            return Err(Error::State("OriginalV1 item/geometry identity"));
        }
        for (slot, item) in self.items.iter().enumerate() {
            if usize::from(item.slot) != slot
                || item.level != self.level
                || !(item.kind == -1
                    || (0..=27).contains(&item.kind)
                    || (64..=83).contains(&item.kind))
                || item.kind != -1 && Dimensions::ORIGINAL.world_cell(item.x, item.y).is_none()
            {
                return Err(Error::State("OriginalV1 item identity/kind/position"));
            }
        }
        Ok(())
    }
    pub fn validate_complete(&self, geometry: &OriginalGeometry) -> Result<()> {
        self.validate(geometry)?;
        let count = if self.level.get() == 0 || self.level.get() == 26 {
            0
        } else if self.level.get() == 1 {
            12
        } else {
            ((if self.two_player_baseline { 16 } else { 12 }) + self.level.get().saturating_sub(2))
                .min(28) as usize
        };
        if self.items[..count].iter().any(|item| item.kind == -1)
            || self.items[count..28].iter().any(|item| item.kind != -1)
            || (self.level.get() == 26 && self.items.iter().any(|item| item.kind != -1))
            || (self.level.get() == 1 && self.items[28..].iter().any(|item| item.kind != -1))
        {
            return Err(Error::State("OriginalV1 incomplete stock item slots"));
        }
        Ok(())
    }
    /// S4 supplies its real persistent actor placements; this preserves original
    /// item-slot holes rather than renumbering stable collected identities.
    pub fn population(
        &self,
        geometry: &OriginalGeometry,
        earthlings: Vec<crate::population::Placement>,
    ) -> Result<crate::population::Population> {
        use crate::population::{
            Placement, PlacementRole, Population, PopulationBudget, StableBits,
        };
        self.validate_complete(geometry)?;
        let identity = geometry.identity();
        let items: Vec<_> = self
            .items
            .iter()
            .filter(|item| item.kind != -1)
            .map(|item| Placement {
                id: u16::from(item.slot),
                x: item.x,
                y: item.y,
                role: PlacementRole::Resource,
                dormant_ticks: 0,
                ai_rng: crate::random::StageRng::derive(
                    &identity,
                    self.level,
                    item.slot,
                    b"stock-item",
                ),
            })
            .collect();
        let value = Population {
            budget: PopulationBudget {
                items: items.len() as u16,
                earthlings: earthlings.len() as u16,
                reserved_items: 0,
                promotion_numerator: geometry.walkable_tiles,
                promotion_denominator: geometry.walkable_tiles,
            },
            items,
            earthlings,
            collected: StableBits::new(crate::MAX_ITEMS as u16)?,
            removed: StableBits::new(crate::MAX_EARTHLINGS as u16)?,
        };
        value.validate(Dimensions::ORIGINAL)?;
        Ok(value)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct OriginalItems {
    content: OriginalContent,
    phase: ItemPhase,
    rng: OriginalRng,
    slot: u8,
    ordinary_count: u8,
    large_attempts: u8,
    candidate: Option<(i32, i32)>,
    placement: Option<ItemPlacement>,
    total_work: u64,
}
impl OriginalItems {
    pub fn new(
        geometry: &OriginalGeometry,
        two_player_baseline: bool,
        specials: StockSpecialPositions,
    ) -> Result<Self> {
        geometry.validate()?;
        specials.validate()?;
        let level = geometry.spec.level;
        let ordinary_count = if level.get() == 0 || level.get() == 26 {
            0
        } else if level.get() == 1 {
            12
        } else {
            ((if two_player_baseline { 16 } else { 12 }) + level.get().saturating_sub(2)).min(28)
                as u8
        };
        let mut content = OriginalContent {
            level,
            seed: geometry.accepted_seed,
            geometry_identity: geometry.identity(),
            two_player_baseline,
            specials,
            items: std::array::from_fn(|slot| OriginalItem::empty(slot as u8, level)),
        };
        let phase = if level.get() == 26 {
            ItemPhase::Complete
        } else if level.get() == 0 {
            for (slot, x, y) in [(28, 1464, 854), (29, 1536, 928), (30, 1336, 1110)] {
                content.items[slot] = OriginalItem {
                    slot: slot as u8,
                    kind: 82,
                    level,
                    x,
                    y,
                };
            }
            ItemPhase::Complete
        } else {
            ItemPhase::Placement
        };
        Ok(Self {
            rng: OriginalRng::new(u32::from(geometry.accepted_seed)),
            content,
            phase,
            slot: 0,
            ordinary_count,
            large_attempts: 0,
            candidate: None,
            placement: None,
            total_work: 0,
        })
    }
    pub const fn phase(&self) -> ItemPhase {
        self.phase
    }
    pub const fn total_work(&self) -> u64 {
        self.total_work
    }
    pub fn advance(
        &mut self,
        geometry: &OriginalGeometry,
        resources: &impl OriginalResources,
        work: u32,
    ) -> Result<ItemPhase> {
        if !(1..=1024).contains(&work)
            || geometry.spec.level != self.content.level
            || geometry.accepted_seed != self.content.seed
            || resources.original_revision() != geometry.spec.revision
        {
            return Err(Error::Bounds("OriginalV1 item source/work/revision"));
        }
        for _ in 0..work {
            if matches!(self.phase, ItemPhase::Complete | ItemPhase::Rejected) {
                break;
            }
            self.total_work += 1;
            if self.total_work > 8_000_000 {
                self.phase = ItemPhase::Rejected;
                return Err(Error::Capacity("OriginalV1 item work"));
            }
            self.step(geometry, resources)?;
        }
        Ok(self.phase)
    }
    fn accept(&mut self, kind: i8) {
        let (x, y) = self.candidate.expect("prepared original item position");
        self.content.items[usize::from(self.slot)] = OriginalItem {
            slot: self.slot,
            kind,
            level: self.content.level,
            x,
            y,
        };
        self.slot += 1;
        self.candidate = None;
        self.placement = None;
        if self.slot < self.ordinary_count {
            self.phase = ItemPhase::Placement;
        } else if self.content.level.get() == 1 {
            self.phase = ItemPhase::Complete;
        } else {
            self.slot = 28;
            self.phase = ItemPhase::LargePlacement;
        }
    }
    fn step(
        &mut self,
        geometry: &OriginalGeometry,
        resources: &impl OriginalResources,
    ) -> Result<()> {
        match self.phase {
            ItemPhase::Placement => {
                if self.content.level.get() == 1 {
                    self.candidate = Some(resources.original_level1_item(self.slot)?);
                    self.phase = ItemPhase::Kind;
                } else {
                    if self.placement.is_none() {
                        self.placement = Some(ItemPlacement::new(false));
                    }
                    if let Some(position) = self
                        .placement
                        .as_mut()
                        .expect("original item placement")
                        .step(geometry, resources, &mut self.rng)?
                    {
                        self.candidate = Some(position);
                        self.phase = ItemPhase::Kind;
                        self.placement = None;
                    }
                }
            }
            ItemPhase::Kind => {
                if self.content.level.get() == 1 || self.rng.bounded(100) < 50 {
                    self.phase = ItemPhase::Present;
                } else if self.rng.bounded(100) < 75 {
                    let food = (self.rng.bounded(16) + 64) as i8;
                    self.accept(food);
                } else {
                    self.accept(80);
                }
            }
            ItemPhase::Present => {
                let mut total = 0_i16;
                for index in 0..27 {
                    total =
                        total.wrapping_add(i16::from(resources.original_present_weight(index)?));
                }
                if total <= 0 {
                    return Err(Error::Resource("OriginalV1 present weight total"));
                }
                let selected = (u32::from(self.rng.next_u16()) % (total as u32 + 1)) as i16;
                if selected != 0 {
                    let mut total = 0_i16;
                    let mut kind = 27;
                    for index in 0..27 {
                        total = total
                            .wrapping_add(i16::from(resources.original_present_weight(index)?));
                        if selected <= total {
                            kind = index as i8;
                            break;
                        }
                    }
                    if !(self.content.level.get() == 1
                        && self.slot < 4
                        && matches!(kind, 0 | 2 | 5 | 26 | 18 | 16))
                    {
                        self.accept(kind);
                    }
                }
            }
            ItemPhase::LargePlacement => {
                if self.slot == 32 {
                    self.phase = ItemPhase::Complete;
                    return Ok(());
                }
                if self.placement.is_none() {
                    self.placement = Some(ItemPlacement::new(true));
                }
                if let Some(position) = self
                    .placement
                    .as_mut()
                    .expect("original large placement")
                    .step(geometry, resources, &mut self.rng)?
                {
                    self.candidate = Some(position);
                    self.phase = ItemPhase::LargeValidation;
                    self.placement = None;
                }
            }
            ItemPhase::LargeValidation => {
                let (x, y) = self.candidate.expect("large item candidate");
                let prior_large =
                    self.content.items[28..usize::from(self.slot)]
                        .iter()
                        .any(|item| {
                            (i64::from(x) - i64::from(item.x)).abs() < 340
                                && (i64::from(y) - i64::from(item.y)).abs() < 150
                        });
                let prior_small = self.content.items[..28].iter().any(|item| {
                    item.kind != -1
                        && (i64::from(x) - i64::from(item.x)).abs() < 16
                        && (i64::from(y) - i64::from(item.y)).abs() < 10
                });
                let elevator = [
                    geometry.geometry.anchors().arrival,
                    geometry.geometry.anchors().exit,
                ]
                .iter()
                .any(|anchor| {
                    let ex = (i32::from(anchor.x) + 2) * 128 - 16;
                    let ey = (i32::from(anchor.y) + 2) * 64 - 5;
                    (i64::from(x) - i64::from(ex)).abs() < 340
                        && (i64::from(y) - i64::from(ey)).abs() < 150
                });
                if prior_large || prior_small || elevator || self.content.specials.is_near(x, y) {
                    self.large_attempts += 1;
                    self.candidate = None;
                    if self.large_attempts == 10 {
                        self.slot += 1;
                        self.large_attempts = 0;
                    }
                    self.phase = ItemPhase::LargePlacement;
                } else {
                    self.phase = ItemPhase::LargeKind;
                }
            }
            ItemPhase::LargeKind => {
                let (x, y) = self.candidate.expect("large item candidate");
                let cell = geometry
                    .geometry
                    .dimensions()
                    .world_cell(x, y)
                    .ok_or(Error::Bounds("OriginalV1 large item cell"))?;
                let word = geometry.geometry.cells()[cell.index(Dimensions::ORIGINAL)?];
                let tile = resources.base_tile(
                    word.base(),
                    ((y >> 3) & 7) as u8,
                    ((x >> 3) & 15) as u8,
                )?;
                let terrain = resources.property(tile)? & 0xf0;
                let kind = if terrain == 0x20 {
                    if self.rng.next_u16() & 1 != 0 { 82 } else { 83 }
                } else {
                    81
                };
                self.content.items[usize::from(self.slot)] = OriginalItem {
                    slot: self.slot,
                    kind,
                    level: self.content.level,
                    x,
                    y,
                };
                self.slot += 1;
                self.large_attempts = 0;
                self.candidate = None;
                self.phase = ItemPhase::LargePlacement;
            }
            ItemPhase::Complete | ItemPhase::Rejected => (),
        }
        Ok(())
    }
    pub fn validate(&self, geometry: &OriginalGeometry) -> Result<()> {
        self.content.validate(geometry)?;
        if self.slot > 32
            || self.ordinary_count > 28
            || self.large_attempts > 9
            || self.total_work > 8_000_001
            || matches!(
                self.phase,
                ItemPhase::Kind
                    | ItemPhase::Present
                    | ItemPhase::LargeValidation
                    | ItemPhase::LargeKind
            ) && self.candidate.is_none()
            || matches!(
                self.phase,
                ItemPhase::Placement | ItemPhase::Kind | ItemPhase::Present
            ) && self.slot >= self.ordinary_count
            || matches!(
                self.phase,
                ItemPhase::LargePlacement | ItemPhase::LargeValidation | ItemPhase::LargeKind
            ) && self.slot < 28
            || matches!(
                self.phase,
                ItemPhase::LargeValidation | ItemPhase::LargeKind
            ) && self.slot >= 32
            || (self.phase == ItemPhase::Complete
                && self.content.level.ordinary()
                && self.slot != 32)
        {
            return Err(Error::State("OriginalV1 item continuation phase/slot/work"));
        }
        if let Some(placement) = &self.placement {
            placement.validate()?;
            if placement.large != (self.phase == ItemPhase::LargePlacement)
                || !matches!(self.phase, ItemPhase::Placement | ItemPhase::LargePlacement)
            {
                return Err(Error::State("OriginalV1 placement kind/phase"));
            }
        }
        if let Some((x, y)) = self.candidate
            && Dimensions::ORIGINAL.world_cell(x, y).is_none()
        {
            return Err(Error::State("OriginalV1 candidate item position"));
        }
        Ok(())
    }
    pub fn finish(self, geometry: &OriginalGeometry) -> Result<OriginalContent> {
        self.validate(geometry)?;
        if self.phase != ItemPhase::Complete {
            return Err(Error::State("OriginalV1 item generation is incomplete"));
        }
        self.content.validate_complete(geometry)?;
        Ok(self.content)
    }
}
