use crate::generation::{GeneratedLevel, Generation, GenerationSpec, Phase, Rejection};
use crate::land::LandState;
use crate::loading::{CellSource, FloorSpec, ImportJob, ImportProgress, ImportSpec};
use crate::population::{PopulationBaseline, StableBits};
use crate::{
    Cell, CellPosition, Error, LevelId, MAX_ACTIVE_FLOORS, MAX_DROPPED_ITEMS, MAX_EFFECTS,
    MAX_ENTITIES, MAX_FLOORS, Result, WorldConfig,
};
use bincode::{Decode, Encode};

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Discovery {
    columns: u16,
    rows: u16,
    visited: StableBits,
    awards: u32,
}
impl Discovery {
    pub fn new(dimensions: crate::Dimensions) -> Result<Self> {
        dimensions.validate()?;
        let (width, height) = dimensions.world_envelope();
        let columns = ((width - 128) as u32).div_ceil(320) as u16;
        let rows = (height as u32).div_ceil(256) as u16;
        Ok(Self {
            columns,
            rows,
            visited: StableBits::new(columns * rows)?,
            awards: 0,
        })
    }
    pub fn visit(&mut self, x: i32, y: i32, dimensions: crate::Dimensions) -> Result<bool> {
        let (width, height) = dimensions.world_envelope();
        if x < 72
            || y < 8
            || i64::from(x) - 72 >= i64::from(width - 128)
            || i64::from(y) - 8 >= i64::from(height)
        {
            return Ok(false);
        }
        let column = ((x - 72) / 320) as u16;
        let row = ((y - 8) / 256) as u16;
        if column >= self.columns || row >= self.rows {
            return Ok(false);
        }
        let fresh = self.visited.insert(row * self.columns + column)?;
        if fresh {
            self.awards += 1;
        }
        Ok(fresh)
    }
    pub const fn awards(&self) -> u32 {
        self.awards
    }
    pub fn validate(&self, dimensions: crate::Dimensions) -> Result<()> {
        let expected = Self::new(dimensions)?;
        if self.columns != expected.columns
            || self.rows != expected.rows
            || self.awards != self.visited.count()
        {
            return Err(Error::State("discovery grid/award extent"));
        }
        self.visited.validate(self.columns * self.rows)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct StoredFloor {
    pub spec: FloorSpec,
    pub generated: GeneratedLevel,
    pub discovery: Discovery,
    pub dirty_cells: StableBits,
    pub land: LandState,
}
impl StoredFloor {
    pub fn replace_cell(&mut self, position: CellPosition, cell: Cell, path: u8) -> Result<()> {
        let index = position.index(self.generated.geometry.dimensions())?;
        self.generated.geometry.replace_cell(position, cell, path)?;
        self.land
            .set_flags(position, path != 0, self.land.alternate(position))?;
        self.dirty_cells.insert(index as u16)?;
        Ok(())
    }
    pub fn step_land(
        &mut self,
        resources: &impl crate::land::PathResources,
    ) -> Result<Option<crate::land::LandChange>> {
        self.land.step(
            &mut self.generated.geometry,
            &mut self.dirty_cells,
            &mut self.generated.runtime_rng,
            resources,
        )
    }
    pub fn validate(&self, config: &WorldConfig) -> Result<()> {
        self.spec.validate()?;
        self.generated.validate()?;
        let dimensions = self.generated.geometry.dimensions();
        if self.spec.config() != config
            || self.spec.identity() != self.generated.identity
            || dimensions != self.spec.dimensions()
        {
            return Err(Error::State(
                "stored floor configuration/identity/dimensions",
            ));
        }
        self.land.validate(&self.generated.geometry)?;
        self.discovery.validate(dimensions)?;
        self.dirty_cells.validate(dimensions.area() as u16)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct LoadFailure {
    pub spec: GenerationSpec,
    pub attempts: u8,
    pub reason: Rejection,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct DroppedItem {
    pub id: u32,
    pub level: LevelId,
    pub x: i32,
    pub y: i32,
    pub item_kind: u16,
}

/// Bounded deterministic sidecar. No physical RAM bank is authoritative, and no
/// partial loading result can overwrite a stored floor or a seat's current floor.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct WorldState {
    config: WorldConfig,
    configuration: [u8; 32],
    floors: Vec<StoredFloor>,
    active: [Option<LevelId>; 4],
    pending: Option<Generation>,
    importing: Option<ImportJob>,
    last_failure: Option<LoadFailure>,
    dropped: Vec<DroppedItem>,
    next_drop_id: u32,
    effects_in_use: u16,
    companions_in_use: u16,
}
impl WorldState {
    pub fn new(config: WorldConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            configuration: config.identity(),
            config,
            floors: Vec::with_capacity(MAX_FLOORS),
            active: [None; 4],
            pending: None,
            importing: None,
            last_failure: None,
            dropped: Vec::with_capacity(MAX_DROPPED_ITEMS),
            next_drop_id: 0,
            effects_in_use: 0,
            companions_in_use: 0,
        })
    }
    pub fn config(&self) -> &WorldConfig {
        &self.config
    }
    pub fn configuration_identity(&self) -> &[u8; 32] {
        &self.configuration
    }
    pub fn floors(&self) -> &[StoredFloor] {
        &self.floors
    }
    pub fn floor(&self, level: LevelId) -> Option<&StoredFloor> {
        self.floors
            .binary_search_by_key(&level, |floor| floor.spec.level())
            .ok()
            .map(|index| &self.floors[index])
    }
    pub fn floor_mut(&mut self, level: LevelId) -> Option<&mut StoredFloor> {
        self.floors
            .binary_search_by_key(&level, |floor| floor.spec.level())
            .ok()
            .map(|index| &mut self.floors[index])
    }
    pub fn pending(&self) -> Option<&Generation> {
        self.pending.as_ref()
    }
    pub fn last_failure(&self) -> Option<&LoadFailure> {
        self.last_failure.as_ref()
    }
    pub fn active(&self) -> &[Option<LevelId>; 4] {
        &self.active
    }
    pub fn dropped(&self) -> &[DroppedItem] {
        &self.dropped
    }
    pub fn importing(&self) -> Option<&ImportJob> {
        self.importing.as_ref()
    }
    pub fn request_import(&mut self, spec: ImportSpec) -> Result<()> {
        if self.pending.is_some() || self.importing.is_some() {
            return Err(Error::State("a world loading operation is already pending"));
        }
        if spec.config != self.config || self.floor(spec.level).is_some() {
            return Err(Error::State("import configuration/already stored floor"));
        }
        if self.floors.len() == MAX_FLOORS {
            return Err(Error::Capacity("persistent floors"));
        }
        self.importing = Some(ImportJob::new(spec)?);
        Ok(())
    }
    pub fn advance_import(
        &mut self,
        source: &impl CellSource,
        resources: &impl crate::Resources,
        work: u32,
    ) -> Result<ImportProgress> {
        let job = self
            .importing
            .as_mut()
            .ok_or(Error::State("no pending import"))?;
        let phase = job.advance(source, resources, work)?;
        if phase == ImportProgress::Ready {
            let (spec, generated, land) = self.importing.take().expect("ready import").finish()?;
            self.publish_floor(spec, generated, land)?;
        }
        Ok(phase)
    }
    /// Discard only unpublished work. Stored deltas and committed seat levels
    /// remain untouched, including when a generation/import has failed.
    pub fn cancel_loading(&mut self) {
        self.pending = None;
        self.importing = None;
    }
    fn publish_floor(
        &mut self,
        spec: FloorSpec,
        generated: GeneratedLevel,
        land: LandState,
    ) -> Result<()> {
        let dimensions = generated.geometry.dimensions();
        let floor = StoredFloor {
            spec,
            generated,
            land,
            discovery: Discovery::new(dimensions)?,
            dirty_cells: StableBits::new(dimensions.area() as u16)?,
        };
        floor.validate(&self.config)?;
        let index = self
            .floors
            .binary_search_by_key(&floor.spec.level(), |value| value.spec.level())
            .err()
            .ok_or(Error::State("loading would overwrite a stored floor"))?;
        self.floors.insert(index, floor);
        Ok(())
    }
    pub fn request_load(&mut self, level: LevelId, baseline: PopulationBaseline) -> Result<()> {
        if self.pending.is_some() || self.importing.is_some() {
            return Err(Error::State(
                "a world loading/transition operation is already pending",
            ));
        }
        if self.floor(level).is_some() {
            return Ok(());
        }
        if self.floors.len() >= MAX_FLOORS {
            return Err(Error::Capacity("persistent floors"));
        }
        let pending = Generation::new(GenerationSpec {
            config: self.config.clone(),
            level,
            baseline,
        })?;
        self.pending = Some(pending);
        Ok(())
    }
    /// Reentrant loading driver, also used by the enhanced resumable override.
    pub fn advance_loading(
        &mut self,
        resources: &impl crate::Resources,
        work: u32,
    ) -> Result<Phase> {
        let mut pending = self.take_generation()?;
        let result = pending.advance(resources, work);
        self.return_generation(pending, result)
    }
    pub(crate) fn take_generation(&mut self) -> Result<Generation> {
        self.pending
            .take()
            .ok_or(Error::State("no pending world generation"))
    }
    pub(crate) fn return_generation(
        &mut self,
        pending: Generation,
        result: Result<Phase>,
    ) -> Result<Phase> {
        self.pending = Some(pending);
        let phase = result?;
        let pending = self.pending.as_ref().expect("returned generation");
        if phase == Phase::Rejected {
            self.last_failure = Some(LoadFailure {
                spec: pending.spec().clone(),
                attempts: crate::MAX_ATTEMPTS,
                reason: pending
                    .last_rejection()
                    .ok_or(Error::State("missing generation rejection"))?,
            });
            self.pending = None;
        } else if phase == Phase::Complete {
            let spec = pending.spec().clone();
            // finish validates everything before publication; removing the
            // temporary job cannot alter a live floor or its mutable deltas.
            let generated = self.pending.take().expect("pending generation").finish()?;
            let land = LandState::from_geometry(&generated.geometry)?;
            self.publish_floor(FloorSpec::Expanded(spec), generated, land)?;
        }
        Ok(phase)
    }
    pub fn set_active(&mut self, seat: u8, level: LevelId) -> Result<()> {
        if seat >= 4 || self.floor(level).is_none() {
            return Err(Error::Bounds("active seat/floor"));
        }
        self.active[usize::from(seat)] = Some(level);
        Ok(())
    }
    pub fn drop_item(&mut self, level: LevelId, x: i32, y: i32, item_kind: u16) -> Result<u32> {
        let floor = self
            .floor(level)
            .ok_or(Error::Bounds("dropped item floor"))?;
        if floor
            .generated
            .geometry
            .dimensions()
            .world_cell(x, y)
            .is_none()
        {
            return Err(Error::Bounds("dropped item position"));
        }
        if self.dropped.len() == MAX_DROPPED_ITEMS {
            return Err(Error::Capacity("global dropped items"));
        }
        let next = self
            .next_drop_id
            .checked_add(1)
            .ok_or(Error::Capacity("stable dropped item identity"))?;
        let id = self.next_drop_id;
        self.dropped.push(DroppedItem {
            id,
            level,
            x,
            y,
            item_kind,
        });
        self.next_drop_id = next;
        Ok(id)
    }
    pub fn take_drop(&mut self, id: u32) -> Result<DroppedItem> {
        let index = self
            .dropped
            .binary_search_by_key(&id, |item| item.id)
            .map_err(|_| Error::Bounds("dropped item identity"))?;
        Ok(self.dropped.remove(index))
    }
    /// Identity references replace original map copies/swaps: both heroes borrow
    /// the same record and every edit follows LevelId, not an active slot.
    pub fn copy_active_level(&mut self, source: u8, target: u8) -> Result<()> {
        if source >= 4 || target >= 4 {
            return Err(Error::Bounds("copy active seat"));
        }
        let level =
            self.active[usize::from(source)].ok_or(Error::State("source seat is inactive"))?;
        self.set_active(target, level)
    }
    pub fn swap_active_levels(&mut self, first: u8, second: u8) -> Result<()> {
        if first >= 4 || second >= 4 {
            return Err(Error::Bounds("swap active seat"));
        }
        self.active.swap(usize::from(first), usize::from(second));
        Ok(())
    }
    /// Validate a whole cohort before changing any seat. Player services apply
    /// the returned arrival positions in the same aggregate session transaction.
    pub fn validate_cohort(&self, cohort: u8, destinations: &[Option<LevelId>; 4]) -> Result<()> {
        if cohort == 0 || cohort & !15 != 0 {
            return Err(Error::State("cohort mask"));
        }
        for (seat, destination) in destinations.iter().enumerate() {
            if (cohort & (1 << seat) != 0) != destination.is_some() {
                return Err(Error::State("cohort destination mask"));
            }
            if let Some(level) = destination
                && (self.active[seat].is_none() || self.floor(*level).is_none())
            {
                return Err(Error::State("cohort source/destination is absent"));
            }
        }
        Ok(())
    }
    pub fn commit_cohort(&mut self, cohort: u8, destinations: [Option<LevelId>; 4]) -> Result<()> {
        self.validate_cohort(cohort, &destinations)?;
        for (seat, destination) in destinations.into_iter().enumerate() {
            if destination.is_some() {
                self.active[seat] = destination;
            }
        }
        Ok(())
    }
    pub fn logical_entities_reserved(&self) -> usize {
        let mut total = self.active.iter().flatten().count()
            + self.dropped.len()
            + usize::from(self.effects_in_use)
            + usize::from(self.companions_in_use);
        let mut prior = [None; MAX_ACTIVE_FLOORS];
        let mut count = 0;
        for level in self.active.iter().flatten() {
            if prior[..count].contains(&Some(*level)) {
                continue;
            }
            prior[count] = Some(*level);
            count += 1;
            if let Some(floor) = self.floor(*level) {
                total += floor.generated.population.items.len()
                    + floor.generated.population.earthlings.len();
            }
        }
        total
    }
    pub fn validate(&self) -> Result<()> {
        self.config.validate()?;
        if self.configuration != self.config.identity()
            || self.floors.len() > MAX_FLOORS
            || self.dropped.len() > MAX_DROPPED_ITEMS
            || usize::from(self.effects_in_use) > MAX_EFFECTS
            || self.companions_in_use > 60
            || self.logical_entities_reserved() > MAX_ENTITIES
        {
            return Err(Error::State("world immutable configuration/resource caps"));
        }
        let mut previous = None;
        for floor in &self.floors {
            floor.validate(&self.config)?;
            if previous.is_some_and(|prior| prior >= floor.spec.level()) {
                return Err(Error::State("floor order/duplicates"));
            }
            previous = Some(floor.spec.level());
        }
        for level in self.active.iter().flatten() {
            if self.floor(*level).is_none() {
                return Err(Error::State("active floor is absent"));
            }
        }
        if let Some(pending) = &self.pending {
            pending.validate()?;
            if pending.spec().config != self.config || self.floor(pending.spec().level).is_some() {
                return Err(Error::State("pending generation configuration/floor"));
            }
        }
        if let Some(importing) = &self.importing {
            importing.validate()?;
            if self.pending.is_some()
                || importing.spec().config != self.config
                || self.floor(importing.spec().level).is_some()
            {
                return Err(Error::State("pending import configuration/floor/ownership"));
            }
        }
        if let Some(failure) = &self.last_failure {
            failure.spec.validate()?;
            if failure.spec.config != self.config || failure.attempts != crate::MAX_ATTEMPTS {
                return Err(Error::State("failed request identity/attempt count"));
            }
        }
        let mut previous_id = None;
        for item in &self.dropped {
            let floor = self
                .floor(item.level)
                .ok_or(Error::State("dropped item floor"))?;
            if floor
                .generated
                .geometry
                .dimensions()
                .world_cell(item.x, item.y)
                .is_none()
                || item.id >= self.next_drop_id
                || previous_id.is_some_and(|previous| previous >= item.id)
            {
                return Err(Error::State("dropped item identity/position"));
            }
            previous_id = Some(item.id);
        }
        Ok(())
    }
}

pub fn codec() -> impl bincode::config::Config {
    bincode::config::standard()
        .with_fixed_int_encoding()
        .with_little_endian()
        .with_limit::<{ crate::MAX_STATE_BYTES }>()
}

/// Standalone sidecar round-trips use the same bounded canonical codec as the
/// enhanced envelope. Decode validates before the caller can replace live data.
pub fn decode(bytes: &[u8], expected: &WorldConfig) -> Result<WorldState> {
    let (state, used): (WorldState, usize) = bincode::decode_from_slice(bytes, codec())
        .map_err(|_| Error::State("world snapshot codec"))?;
    if used != bytes.len() || &state.config != expected {
        return Err(Error::State("world snapshot configuration/trailing bytes"));
    }
    state.validate()?;
    Ok(state)
}
