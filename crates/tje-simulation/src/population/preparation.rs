use super::*;

impl PopulationState {
    /// Prepare a stock population off the live world. Every original slot is a
    /// declared stable identity, including unused source slots. Required records
    /// override those slots without compacting later ordinary identities.
    pub fn prepare_stock_floor(
        &self,
        level: LevelId,
        slots: &[Option<Kind>; ORIGINAL_POPULATION as usize],
        required: &[AuthoredActor],
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
    ) -> Result<PopulationFloor> {
        self.prepare_slots(level, slots, required, environment, catalog)
    }

    pub(super) fn prepare_slots(
        &self,
        level: LevelId,
        slots: &[Option<Kind>],
        required: &[AuthoredActor],
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
    ) -> Result<PopulationFloor> {
        let mut job =
            self.new_preparation_job(level, slots, required, environment.dimensions(level)?)?;
        while job.advance(environment, catalog, 256)? != StockPopulationProgress::Complete {}
        job.finish()
    }

    /// Loading can validate every owner's publication before committing any one
    /// of them. No geometry or live seat is changed by this preflight.
    pub fn preflight_prepared_floor(&self, floor: &PopulationFloor) -> Result<()> {
        self.limits.validate()?;
        floor.validate_records(self.limits.per_floor)?;
        if self.floors.len() >= 32 || self.floor_index(floor.level).is_ok() {
            return Err(Error::Capacity("already loaded/full persistent population"));
        }
        if floor.actors.iter().any(|actor| {
            actor.body.is_some()
                || actor.companion_body.is_some()
                || actor.interest != SeatMask::NONE
                || actor.state == SLEEP_STATE
        }) {
            return Err(Error::State("prepared population contains active bodies"));
        }
        Ok(())
    }
    pub fn install_prepared_floor(&mut self, mut floor: PopulationFloor) -> Result<()> {
        self.preflight_prepared_floor(&floor)?;
        for actor in &mut floor.actors {
            actor.retired = floor.removed.contains(actor.id.local);
        }
        let index = self
            .floors
            .binary_search_by_key(&floor.level, |record| record.level)
            .expect_err("preflighted new population floor");
        self.floors.insert(index, floor);
        Ok(())
    }
}

impl PopulationFloor {
    pub(super) fn validate_records(&self, capacity: u16) -> Result<()> {
        self.level.validate()?;
        if capacity == 0 || capacity > MAX_POPULATION {
            return Err(Error::Capacity("population record capacity"));
        }
        let allowed = if capacity == 64 {
            u64::MAX
        } else {
            (1_u64 << capacity) - 1
        };
        if self.actors.len() > usize::from(capacity)
            || self.declared_slots & !allowed != 0
            || !self.spawn_rng.valid()
        {
            return Err(Error::State("population declarations/capacity/RNG"));
        }
        self.removed.validate(MAX_POPULATION)?;
        self.failed.validate(MAX_POPULATION)?;
        self.collected.validate(MAX_POPULATION)?;
        let mut previous = None;
        for actor in &self.actors {
            actor.validate()?;
            if actor.id.level != self.level
                || actor.id.local >= capacity
                || self.declared_slots & (1_u64 << actor.id.local) == 0
                || previous.is_some_and(|id| id >= actor.id.local)
            {
                return Err(Error::State("population stable record declarations"));
            }
            previous = Some(actor.id.local);
            if (self.removed.contains(actor.id.local) || self.failed.contains(actor.id.local))
                && (actor.body.is_some() || actor.companion_body.is_some())
            {
                return Err(Error::State("removed/failed actor has active body"));
            }
        }
        for id in 0..MAX_POPULATION {
            if (self.removed.contains(id)
                || self.failed.contains(id)
                || self.collected.contains(id))
                && self.declared_slots & (1_u64 << id) == 0
            {
                return Err(Error::State("population flag names undeclared source slot"));
            }
            if self.collected.contains(id)
                && (!self.removed.contains(id)
                    || !self.actors.iter().any(|actor| actor.id.local == id))
            {
                return Err(Error::State("collected actor identity/removal"));
            }
        }
        Ok(())
    }
}

pub(super) fn overlaps(
    catalog: &DefinitionCatalog,
    kind: Kind,
    x: i32,
    y: i32,
    other_kind: Kind,
    other: Position,
) -> bool {
    let clearance =
        (i64::from(catalog.get(kind).radius) + i64::from(catalog.get(other_kind).radius)) * 4;
    (i64::from(x) - i64::from(other.x >> 8)).abs() < clearance
        && (i64::from(y) - i64::from(other.y >> 8)).abs() < clearance
}
