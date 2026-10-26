use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum StockPopulationProgress {
    Working,
    Complete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
struct RequiredActor {
    local: u16,
    kind: Kind,
    position: Position,
    mail_order: bool,
}

/// One work unit visits one source hole/fixed actor or one bounded candidate
/// metatile. Footprint scanning is bounded to that metatile's 128 tiles. No
/// elapsed time, rendering visit or callback-local Rust stack is a continuation.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct StockPopulationJob {
    identity: [u8; 32],
    capacity: u16,
    level: LevelId,
    dimensions: Dimensions,
    slots: Vec<Option<Kind>>,
    required: Vec<RequiredActor>,
    next_slot: u16,
    floor: PopulationFloor,
    random_trials: u8,
    scan_start: Option<(u16, u16)>,
    scanned: u16,
    failed: bool,
}
impl PopulationState {
    pub fn validate_preparation_job(&self, job: &StockPopulationJob) -> Result<()> {
        job.validate()?;
        if job.identity != self.identity || job.capacity != self.limits.per_floor {
            return Err(Error::State(
                "population preparation immutable identity/capacity",
            ));
        }
        Ok(())
    }
    pub fn prepare_stock_job(
        &self,
        level: LevelId,
        slots: &[Option<Kind>; ORIGINAL_POPULATION as usize],
        required: &[AuthoredActor],
        dimensions: Dimensions,
    ) -> Result<StockPopulationJob> {
        self.new_preparation_job(level, slots, required, dimensions)
    }
    pub(super) fn new_preparation_job(
        &self,
        level: LevelId,
        slots: &[Option<Kind>],
        required: &[AuthoredActor],
        dimensions: Dimensions,
    ) -> Result<StockPopulationJob> {
        self.limits.validate()?;
        StockPopulationJob::new(
            self.identity,
            self.limits.per_floor,
            level,
            slots,
            required,
            dimensions,
        )
    }
}
impl StockPopulationJob {
    fn new(
        identity: [u8; 32],
        capacity: u16,
        level: LevelId,
        slots: &[Option<Kind>],
        required: &[AuthoredActor],
        dimensions: Dimensions,
    ) -> Result<Self> {
        level.validate()?;
        dimensions.validate()?;
        if capacity == 0
            || capacity > MAX_POPULATION
            || slots.len() > usize::from(capacity)
            || required.len() > slots.len()
        {
            return Err(Error::Capacity("population preparation sources"));
        }
        let mut previous = None;
        for actor in required {
            actor
                .position
                .validate()
                .map_err(|_| Error::Bounds("required population pose"))?;
            if usize::from(actor.local) >= slots.len()
                || previous.is_some_and(|id| id >= actor.local)
                || actor.mail_order && actor.kind != Kind::Mailbox
                || dimensions
                    .world_cell(actor.position.x >> 8, actor.position.y >> 8)
                    .is_none()
            {
                return Err(Error::State(
                    "population preparation required identity/position",
                ));
            }
            previous = Some(actor.local);
        }
        let floor = PopulationFloor {
            level,
            actors: Vec::with_capacity(usize::from(capacity)),
            declared_slots: if slots.len() == 64 {
                u64::MAX
            } else {
                (1_u64 << slots.len()) - 1
            },
            removed: StableBits::new(MAX_POPULATION)?,
            failed: StableBits::new(MAX_POPULATION)?,
            collected: StableBits::new(MAX_POPULATION)?,
            spawn_rng: StageRng::derive(&identity, level, 0, b"population"),
        };
        let job = Self {
            identity,
            capacity,
            level,
            dimensions,
            slots: slots.to_vec(),
            required: required
                .iter()
                .map(|actor| RequiredActor {
                    local: actor.local,
                    kind: actor.kind,
                    position: actor.position,
                    mail_order: actor.mail_order,
                })
                .collect(),
            next_slot: 0,
            floor,
            random_trials: 0,
            scan_start: None,
            scanned: 0,
            failed: false,
        };
        job.validate()?;
        Ok(job)
    }
    pub fn progress(&self) -> StockPopulationProgress {
        if usize::from(self.next_slot) == self.slots.len() && !self.failed {
            StockPopulationProgress::Complete
        } else {
            StockPopulationProgress::Working
        }
    }
    pub fn level(&self) -> LevelId {
        self.level
    }
    pub fn dimensions(&self) -> Dimensions {
        self.dimensions
    }
    pub fn validate(&self) -> Result<()> {
        self.level.validate()?;
        self.dimensions.validate()?;
        if self.capacity == 0
            || self.capacity > MAX_POPULATION
            || self.slots.len() > usize::from(self.capacity)
            || usize::from(self.next_slot) > self.slots.len()
            || self.required.len() > self.slots.len()
            || self.random_trials > 10
            || usize::from(self.scanned) > self.dimensions.area()
            || self.scan_start.is_some_and(|(column, row)| {
                column >= self.dimensions.width() || row >= self.dimensions.height()
            })
            || self.scanned != 0 && self.scan_start.is_none()
            || self.scan_start.is_some() && self.random_trials != 10
        {
            return Err(Error::State("population preparation continuation extent"));
        }
        self.floor.validate_records(self.capacity)?;
        let declared = if self.slots.len() == 64 {
            u64::MAX
        } else {
            (1_u64 << self.slots.len()) - 1
        };
        if self.floor.level != self.level
            || self.floor.declared_slots != declared
            || self.floor.actors.iter().any(|actor| {
                actor.id.local >= self.next_slot
                    || actor.body.is_some()
                    || actor.companion_body.is_some()
                    || actor.interest != SeatMask::NONE
            })
        {
            return Err(Error::State(
                "population preparation floor/cursor references",
            ));
        }
        let mut previous = None;
        for actor in &self.required {
            if usize::from(actor.local) >= self.slots.len()
                || previous.is_some_and(|id| id >= actor.local)
                || actor.mail_order && actor.kind != Kind::Mailbox
                || self
                    .dimensions
                    .world_cell(actor.position.x >> 8, actor.position.y >> 8)
                    .is_none()
            {
                return Err(Error::State("population preparation required source"));
            }
            actor
                .position
                .validate()
                .map_err(|_| Error::Bounds("population required pose"))?;
            previous = Some(actor.local);
        }
        for index in 0..usize::from(self.next_slot) {
            let expected = self
                .required
                .iter()
                .find(|actor| usize::from(actor.local) == index)
                .map(|actor| actor.kind)
                .or(self.slots[index]);
            let actual = self
                .floor
                .actors
                .iter()
                .find(|actor| usize::from(actor.id.local) == index);
            if expected != actual.map(|actor| actor.kind) {
                return Err(Error::State("population preparation source completeness"));
            }
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
        work: u32,
    ) -> Result<StockPopulationProgress> {
        self.validate()?;
        if self.failed {
            return Err(Error::State("population preparation previously failed"));
        }
        if !(1..=1024).contains(&work) || environment.dimensions(self.level)? != self.dimensions {
            return Err(Error::Bounds("population preparation work/geometry"));
        }
        for _ in 0..work {
            if self.progress() == StockPopulationProgress::Complete {
                break;
            }
            let fixed = self
                .required
                .iter()
                .find(|actor| actor.local == self.next_slot)
                .copied();
            let Some(kind) = fixed
                .map(|actor| actor.kind)
                .or(self.slots[usize::from(self.next_slot)])
            else {
                self.finish_slot();
                continue;
            };
            if let Some(actor) = fixed {
                if actor.kind != Kind::ShipPiece
                    && !environment.allows_placement(
                        self.level,
                        actor.position.x >> 8,
                        actor.position.y >> 8,
                    )?
                {
                    self.failed = true;
                    return Err(Error::Bounds("required population anchor"));
                }
                self.append(kind, actor.position, actor.mail_order, catalog);
                continue;
            }
            let (column, row) = if self.random_trials < 10 {
                self.random_trials += 1;
                let row = self.floor.spawn_rng.next_u16() % self.dimensions.height();
                let column = self.floor.spawn_rng.next_u16() % self.dimensions.width();
                (column, row)
            } else {
                if self.scan_start.is_none() {
                    let row = self.floor.spawn_rng.next_u16() % self.dimensions.height();
                    let column = self.floor.spawn_rng.next_u16() % self.dimensions.width();
                    self.scan_start = Some((column, row));
                }
                let (column, row) = self.scan_start.expect("initialized cyclic spawn cursor");
                let offset = self.scanned;
                self.scanned += 1;
                (
                    (column + offset % self.dimensions.width()) % self.dimensions.width(),
                    (row + offset / self.dimensions.width()) % self.dimensions.height(),
                )
            };
            let candidate =
                environment.placement(self.level, kind, column, row, &mut self.floor.spawn_rng)?;
            if let Some((x, y)) = candidate {
                let valid = self.dimensions.world_cell(x, y).is_some()
                    && environment.allows_placement(self.level, x, y)?
                    && !self.floor.actors.iter().any(|other| {
                        !self.floor.failed.contains(other.id.local)
                            && preparation::overlaps(
                                catalog,
                                kind,
                                x,
                                y,
                                other.kind,
                                other.motion.position,
                            )
                    })
                    && !self.required.iter().any(|other| {
                        preparation::overlaps(catalog, kind, x, y, other.kind, other.position)
                    });
                if valid {
                    self.append(
                        kind,
                        Position {
                            x: x * 256,
                            y: y * 256,
                            elevation: 0,
                        },
                        false,
                        catalog,
                    );
                    continue;
                }
            }
            if usize::from(self.scanned) == self.dimensions.area() {
                if kind.required() {
                    self.failed = true;
                    return Err(Error::Capacity("required Earthling placement"));
                }
                self.floor.failed.insert(self.next_slot)?;
                self.append(
                    kind,
                    Position {
                        x: 128 * 256,
                        y: 64 * 256,
                        elevation: 0,
                    },
                    false,
                    catalog,
                );
            }
        }
        Ok(self.progress())
    }
    fn append(
        &mut self,
        kind: Kind,
        position: Position,
        mail_order: bool,
        catalog: &DefinitionCatalog,
    ) {
        let id = PopulationId {
            level: self.level,
            local: self.next_slot,
        };
        self.floor.actors.push(PersistentActor::new(
            id,
            kind,
            position,
            catalog.get(kind),
            initial_actor_rng(&self.identity, id),
            mail_order,
        ));
        self.finish_slot();
    }
    fn finish_slot(&mut self) {
        self.next_slot += 1;
        self.random_trials = 0;
        self.scan_start = None;
        self.scanned = 0;
    }
    pub fn finish(self) -> Result<PopulationFloor> {
        self.validate()?;
        if self.progress() != StockPopulationProgress::Complete {
            return Err(Error::State("population preparation is not complete"));
        }
        Ok(self.floor)
    }
}
