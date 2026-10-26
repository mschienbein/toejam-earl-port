//! Saved, bounded world transitions. No blocking scene, synthetic player RAM or
//! implicit camera-bank swap occurs here. The aggregate owner commits returned
//! arrivals and player state at one phase boundary before permitting new queries.
use crate::{
    Error, LevelId, Resources, Result,
    loading::SpecialFloor,
    population::PopulationBaseline,
    random::StageRng,
    terrain::{DoorwaySupport, Exclusion, Footprint, PlacementSearch, SearchProgress, TerrainMask},
    world::WorldState,
};
use bincode::{Decode, Encode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Position {
    pub level: LevelId,
    pub x: i32,
    pub y: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Landing {
    Arrival,
    Exit,
    Exact(i32, i32),
    Search,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Destination {
    pub level: LevelId,
    pub landing: Landing,
}
impl Destination {
    pub fn campaign_fall(source: LevelId, highest: LevelId) -> Result<Self> {
        Ok(Self {
            level: SpecialFloor::fall_destination(source, highest)?,
            landing: Landing::Search,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum TravelKind {
    Elevator,
    Fall,
    Doorway,
    Reenter,
    Ship { required_parts: u16 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum TravelPhase {
    Joining,
    Departing,
    Loading,
    Placing,
    Ready,
    Arriving,
    Complete,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum TravelFailure {
    Generation,
    NoLanding,
}
/// These are explicit enhanced simulation timers, not a cartridge cycle proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct TravelTiming {
    pub join_ticks: u16,
    pub departure_ticks: u16,
    pub arrival_ticks: u16,
}
impl Default for TravelTiming {
    fn default() -> Self {
        Self {
            join_ticks: 30,
            departure_ticks: 12,
            arrival_ticks: 30,
        }
    }
}
impl TravelTiming {
    pub fn validate(self) -> Result<()> {
        if self.join_ticks > 600
            || self.departure_ticks == 0
            || self.departure_ticks > 600
            || self.arrival_ticks == 0
            || self.arrival_ticks > 600
        {
            return Err(Error::Bounds("transition timers"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Journey {
    kind: TravelKind,
    timing: TravelTiming,
    cohort: u8,
    sources: [Option<LevelId>; 4],
    destinations: [Option<Destination>; 4],
    arrivals: [Option<Position>; 4],
    searches: [Option<PlacementSearch>; 4],
    phase: TravelPhase,
    remaining: u16,
    last_tick: Option<u64>,
    failure: Option<TravelFailure>,
    committed: bool,
}
impl Journey {
    pub fn new(
        world: &WorldState,
        kind: TravelKind,
        timing: TravelTiming,
        destinations: [Option<Destination>; 4],
        collected_parts: u16,
    ) -> Result<Self> {
        timing.validate()?;
        let mut cohort = 0;
        let mut sources = [None; 4];
        for (seat, destination) in destinations.iter().enumerate() {
            if let Some(destination) = destination {
                let source = world.active()[seat]
                    .ok_or(Error::State("transition source seat is inactive"))?;
                Self::validate_link(kind, source, *destination, collected_parts)?;
                sources[seat] = Some(source);
                cohort |= 1 << seat;
            }
        }
        if cohort == 0 {
            return Err(Error::State("transition empty cohort"));
        }
        let joining = kind == TravelKind::Elevator && timing.join_ticks != 0;
        let journey = Self {
            kind,
            timing,
            cohort,
            sources,
            destinations,
            arrivals: [None; 4],
            searches: std::array::from_fn(|_| None),
            phase: if joining {
                TravelPhase::Joining
            } else {
                TravelPhase::Departing
            },
            remaining: if joining {
                timing.join_ticks
            } else {
                timing.departure_ticks
            },
            last_tick: None,
            failure: None,
            committed: false,
        };
        journey.validate(world)?;
        Ok(journey)
    }
    fn validate_link(
        kind: TravelKind,
        source: LevelId,
        destination: Destination,
        collected_parts: u16,
    ) -> Result<()> {
        source.validate()?;
        destination.level.validate()?;
        match kind {
            TravelKind::Elevator
                if source.get() >= 25 || destination.level.get() != source.get() + 1 =>
            {
                return Err(Error::Bounds(
                    "campaign elevator link; final floor requires ship transition",
                ));
            }
            TravelKind::Doorway if destination.level != source => {
                return Err(Error::Bounds("doorway must remain on its floor"));
            }
            TravelKind::Ship { required_parts }
                if required_parts == 0
                    || required_parts & !0x3ff != 0
                    || collected_parts & required_parts != required_parts
                    || source.get() != 25
                    || destination.level.get() != 26 =>
            {
                return Err(Error::State("ship objectives/source/final destination"));
            }
            _ => (),
        }
        Ok(())
    }
    pub const fn phase(&self) -> TravelPhase {
        self.phase
    }
    pub const fn kind(&self) -> TravelKind {
        self.kind
    }
    pub const fn committed(&self) -> bool {
        self.committed
    }
    pub const fn cohort(&self) -> u8 {
        self.cohort
    }
    pub const fn sources(&self) -> &[Option<LevelId>; 4] {
        &self.sources
    }
    pub const fn destinations(&self) -> &[Option<Destination>; 4] {
        &self.destinations
    }
    pub const fn arrivals(&self) -> &[Option<Position>; 4] {
        &self.arrivals
    }
    pub const fn failure(&self) -> Option<TravelFailure> {
        self.failure
    }
    pub fn needed_import(&self, world: &WorldState) -> Option<LevelId> {
        self.destinations
            .iter()
            .flatten()
            .map(|destination| destination.level)
            .find(|level| !level.ordinary() && world.floor(*level).is_none())
    }
    /// An elevator's visible join window is the only time its cohort may grow.
    pub fn join(&mut self, world: &WorldState, seat: u8) -> Result<()> {
        if seat >= 4 || self.phase != TravelPhase::Joining || self.cohort & (1 << seat) != 0 {
            return Err(Error::State("elevator join seat/phase"));
        }
        let leader = self.cohort.trailing_zeros() as usize;
        let source =
            world.active()[usize::from(seat)].ok_or(Error::State("joining seat is inactive"))?;
        if self.sources[leader] != Some(source) {
            return Err(Error::State("joining seat is on another floor"));
        }
        self.sources[usize::from(seat)] = Some(source);
        self.destinations[usize::from(seat)] = self.destinations[leader];
        self.cohort |= 1 << seat;
        Ok(())
    }
    pub fn cancel(&mut self, world: &mut WorldState) -> Result<()> {
        if self.committed {
            return Err(Error::State("cannot cancel committed travel"));
        }
        self.validate(world)?;
        let loading = world
            .pending()
            .map(|pending| pending.spec().level)
            .or_else(|| world.importing().map(|importing| importing.spec().level));
        if loading.is_some_and(|level| {
            self.destinations
                .iter()
                .flatten()
                .any(|destination| destination.level == level)
        }) {
            world.cancel_loading();
        }
        self.phase = TravelPhase::Complete;
        self.remaining = 0;
        self.failure = None;
        self.searches = std::array::from_fn(|_| None);
        Ok(())
    }
    /// One authoritative tick and at most `work` bounded generation/placement
    /// units. Reentry with the same tick cannot advance timers/RNG twice.
    pub fn advance(
        &mut self,
        world: &mut WorldState,
        resources: &impl Resources,
        baseline: PopulationBaseline,
        tick: u64,
        work: u32,
        exclusions: &[Exclusion],
    ) -> Result<TravelPhase> {
        if !(1..=1024).contains(&work) || exclusions.len() > crate::MAX_ENTITIES {
            return Err(Error::Bounds("travel work/exclusions"));
        }
        if self.last_tick.is_some_and(|last| tick < last) {
            return Err(Error::State("transition clock moved backwards"));
        }
        if self.last_tick == Some(tick) {
            return Ok(self.phase);
        }
        self.validate(world)?;
        self.last_tick = Some(tick);
        match self.phase {
            TravelPhase::Joining | TravelPhase::Departing | TravelPhase::Arriving => {
                self.remaining = self.remaining.saturating_sub(1);
                if self.remaining == 0 {
                    self.phase = match self.phase {
                        TravelPhase::Joining => {
                            self.remaining = self.timing.departure_ticks;
                            TravelPhase::Departing
                        }
                        TravelPhase::Departing => TravelPhase::Loading,
                        _ => TravelPhase::Complete,
                    };
                }
            }
            TravelPhase::Loading => {
                let next = self
                    .destinations
                    .iter()
                    .flatten()
                    .map(|destination| destination.level)
                    .find(|level| world.floor(*level).is_none());
                if let Some(level) = next {
                    if world.importing().is_some() || !level.ordinary() {
                        return Ok(self.phase);
                    }
                    if let Some(pending) = world.pending() {
                        if pending.spec().level != level {
                            return Ok(self.phase);
                        }
                    } else {
                        world.request_load(level, baseline)?;
                    }
                    if world.advance_loading(resources, work)? == crate::generation::Phase::Rejected
                    {
                        self.phase = TravelPhase::Failed;
                        self.failure = Some(TravelFailure::Generation);
                    }
                } else {
                    self.phase = TravelPhase::Placing;
                }
            }
            TravelPhase::Placing => {
                let footprint = Footprint::new(4, 2)?;
                for seat in 0..4 {
                    let Some(destination) = self.destinations[seat] else {
                        continue;
                    };
                    if self.arrivals[seat].is_some() {
                        continue;
                    }
                    let floor = world
                        .floor(destination.level)
                        .ok_or(Error::State("placing floor is absent"))?;
                    let geometry = &floor.generated.geometry;
                    let rank = (self.cohort & ((1 << seat) - 1)).count_ones() as i32;
                    let anchor = match destination.landing {
                        Landing::Arrival => Some(geometry.anchors().arrival.center()),
                        Landing::Exit => Some(geometry.anchors().exit.center()),
                        Landing::Exact(x, y) => Some((x, y)),
                        Landing::Search => None,
                    };
                    let position = if let Some((x, y)) = anchor {
                        let offset =
                            if matches!(destination.landing, Landing::Arrival | Landing::Exit) {
                                rank * 64
                            } else {
                                0
                            };
                        let position = (
                            x.checked_add(offset)
                                .ok_or(Error::Bounds("cohort anchor X"))?,
                            y,
                        );
                        if footprint.supported_at(
                            geometry,
                            destination.level,
                            position,
                            TerrainMask::LAND_OR_SAND,
                            resources,
                        )? {
                            Some(position)
                        } else {
                            None
                        }
                    } else {
                        if self.searches[seat].is_none() {
                            self.searches[seat] = Some(PlacementSearch::new(
                                geometry.dimensions(),
                                footprint,
                                TerrainMask::LAND_OR_SAND,
                                StageRng::derive(
                                    &floor.generated.identity,
                                    destination.level,
                                    seat as u8,
                                    b"transition-landing",
                                ),
                            )?);
                        }
                        match self.searches[seat]
                            .as_mut()
                            .expect("placement job")
                            .advance(geometry, destination.level, resources, work, exclusions)?
                        {
                            SearchProgress::Searching => return Ok(self.phase),
                            SearchProgress::Exhausted => None,
                            SearchProgress::Found(position) => Some(position),
                        }
                    };
                    if let Some(position) = position {
                        let marked = matches!(destination.landing, Landing::Search)
                            && geometry
                                .dimensions()
                                .world_cell(position.0, position.1)
                                .is_some_and(|position| floor.land.marked(position));
                        let overlapping = marked
                            || exclusions
                                .iter()
                                .any(|exclusion| exclusion.contains(destination.level, position))
                            || self.arrivals.iter().flatten().any(|prior| {
                                prior.level == destination.level
                                    && (i64::from(prior.x) - i64::from(position.0)).abs() < 32
                                    && (i64::from(prior.y) - i64::from(position.1)).abs() < 16
                            });
                        if overlapping {
                            if let Some(search) = self.searches[seat].as_mut() {
                                search.skip_found()?;
                                return Ok(self.phase);
                            }
                        } else {
                            self.arrivals[seat] = Some(Position {
                                level: destination.level,
                                x: position.0,
                                y: position.1,
                            });
                            return Ok(self.phase);
                        }
                    }
                    self.phase = TravelPhase::Failed;
                    self.failure = Some(TravelFailure::NoLanding);
                    return Ok(self.phase);
                }
                self.phase = TravelPhase::Ready;
            }
            _ => (),
        }
        Ok(self.phase)
    }
    /// Every landing is rechecked against live geometry at the commit boundary.
    /// A failure leaves every hero on the old level and preserves this request.
    pub fn commit(
        &mut self,
        world: &mut WorldState,
        resources: &impl Resources,
        exclusions: &[Exclusion],
    ) -> Result<[Option<Position>; 4]> {
        if self.phase != TravelPhase::Ready || self.committed {
            return Err(Error::State("travel is not ready for commit"));
        }
        self.validate(world)?;
        let footprint = Footprint::new(4, 2)?;
        let mut destinations = [None; 4];
        for (seat, arrival) in self.arrivals.iter().enumerate() {
            if let Some(arrival) = arrival {
                let floor = world
                    .floor(arrival.level)
                    .ok_or(Error::State("arrival floor disappeared"))?;
                if !footprint.supported_at(
                    &floor.generated.geometry,
                    arrival.level,
                    (arrival.x, arrival.y),
                    TerrainMask::LAND_OR_SAND,
                    resources,
                )? || exclusions
                    .iter()
                    .any(|exclusion| exclusion.contains(arrival.level, (arrival.x, arrival.y)))
                {
                    return Err(Error::State("arrival became unsafe before commit"));
                }
                destinations[seat] = Some(arrival.level);
            }
        }
        world.commit_cohort(self.cohort, destinations)?;
        self.commit_after_world(world)
    }
    /// Aggregate arrival tokens already checked geometry and every owner. This
    /// completes only the journey ledger after that one atomic world commit.
    pub fn commit_after_world(&mut self, world: &WorldState) -> Result<[Option<Position>; 4]> {
        if self.phase != TravelPhase::Ready || self.committed {
            return Err(Error::State("journey is not awaiting atomic arrival"));
        }
        for (seat, arrival) in self.arrivals.iter().enumerate() {
            if let Some(arrival) = arrival
                && world.active()[seat] != Some(arrival.level)
            {
                return Err(Error::State(
                    "journey destination was not atomically committed",
                ));
            }
        }
        self.phase = TravelPhase::Arriving;
        self.remaining = self.timing.arrival_ticks;
        self.committed = true;
        Ok(self.arrivals)
    }
    pub fn retarget_landings(&mut self, landings: [Option<Landing>; 4]) -> Result<()> {
        if self.committed
            || !matches!(
                self.phase,
                TravelPhase::Joining | TravelPhase::Departing | TravelPhase::Loading
            )
        {
            return Err(Error::State("journey landing was already prepared"));
        }
        for (seat, landing) in landings.into_iter().enumerate() {
            if self.destinations[seat].is_some() != landing.is_some() {
                return Err(Error::State("retargeted journey cohort mask"));
            }
        }
        for (seat, landing) in landings.into_iter().enumerate() {
            if let Some(landing) = landing {
                self.destinations[seat]
                    .as_mut()
                    .expect("cohort destination")
                    .landing = landing;
            }
        }
        Ok(())
    }
    /// Removing an eliminated hero must not strand surviving fall/rider cohorts.
    /// Returns true only when no participant remains in this journey.
    pub fn remove_seat(&mut self, seat: u8) -> Result<bool> {
        if seat >= 4 {
            return Err(Error::Bounds("journey released seat"));
        }
        let index = usize::from(seat);
        self.cohort &= !(1 << seat);
        self.sources[index] = None;
        self.destinations[index] = None;
        self.arrivals[index] = None;
        self.searches[index] = None;
        Ok(self.cohort == 0)
    }
    pub fn validate(&self, world: &WorldState) -> Result<()> {
        self.timing.validate()?;
        let timed = matches!(
            self.phase,
            TravelPhase::Joining | TravelPhase::Departing | TravelPhase::Arriving
        );
        let maximum_timer = match self.phase {
            TravelPhase::Joining => self.timing.join_ticks,
            TravelPhase::Departing => self.timing.departure_ticks,
            TravelPhase::Arriving => self.timing.arrival_ticks,
            _ => 0,
        };
        if self.cohort == 0
            || self.cohort & !15 != 0
            || (self.phase == TravelPhase::Failed) != self.failure.is_some()
            || self.remaining > maximum_timer
            || (timed && self.remaining == 0)
            || (self.phase == TravelPhase::Joining && self.kind != TravelKind::Elevator)
            || (self.committed
                && !matches!(self.phase, TravelPhase::Arriving | TravelPhase::Complete))
            || (!self.committed && self.phase == TravelPhase::Arriving)
        {
            return Err(Error::State("transition cohort/phase/timer/failure"));
        }
        let leader = self.cohort.trailing_zeros() as usize;
        for seat in 0..4 {
            let included = self.cohort & (1 << seat) != 0;
            if included != self.sources[seat].is_some()
                || included != self.destinations[seat].is_some()
                || (!included && (self.arrivals[seat].is_some() || self.searches[seat].is_some()))
            {
                return Err(Error::State("transition cohort reference mask"));
            }
            if let (Some(source), Some(destination)) = (self.sources[seat], self.destinations[seat])
            {
                Self::validate_link(self.kind, source, destination, 0x3ff)?;
                if matches!(self.kind, TravelKind::Elevator | TravelKind::Ship { .. })
                    && (self.sources[seat] != self.sources[leader]
                        || self.destinations[seat].map(|target| target.level)
                            != self.destinations[leader].map(|target| target.level))
                {
                    return Err(Error::State("elevator/ship cohort spans unrelated routes"));
                }
                if world.active()[seat]
                    != Some(if self.committed {
                        destination.level
                    } else {
                        source
                    })
                {
                    return Err(Error::State("transition source is no longer current"));
                }
                let needs_floor = matches!(
                    self.phase,
                    TravelPhase::Placing | TravelPhase::Ready | TravelPhase::Arriving
                ) || self.committed;
                if needs_floor && world.floor(destination.level).is_none() {
                    return Err(Error::State("prepared transition destination is absent"));
                }
                if matches!(
                    self.phase,
                    TravelPhase::Joining | TravelPhase::Departing | TravelPhase::Loading
                ) && (self.arrivals[seat].is_some() || self.searches[seat].is_some())
                {
                    return Err(Error::State("transition prepared landing before loading"));
                }
                if let Some(search) = &self.searches[seat] {
                    search.validate()?;
                    if destination.landing != Landing::Search
                        || world.floor(destination.level).is_none_or(|floor| {
                            floor.generated.geometry.dimensions() != search.dimensions()
                        })
                    {
                        return Err(Error::State("transition search destination/extent"));
                    }
                }
                if let Some(arrival) = self.arrivals[seat] {
                    if arrival.level != destination.level
                        || world.floor(arrival.level).is_none_or(|floor| {
                            floor
                                .generated
                                .geometry
                                .dimensions()
                                .world_cell(arrival.x, arrival.y)
                                .is_none()
                        })
                        || self.arrivals[..seat].iter().flatten().any(|prior| {
                            prior.level == arrival.level
                                && (i64::from(prior.x) - i64::from(arrival.x)).abs() < 32
                                && (i64::from(prior.y) - i64::from(arrival.y)).abs() < 16
                        })
                    {
                        return Err(Error::State(
                            "transition arrival reference/position/overlap",
                        ));
                    }
                } else if matches!(self.phase, TravelPhase::Ready | TravelPhase::Arriving)
                    || self.committed
                {
                    return Err(Error::State("transition missing prepared arrival"));
                }
            }
        }
        Ok(())
    }
}

/// Doorway terrain support/contact has four independent seat latches. The
/// crossing mask names actual seats rather than original packed owner+1 bits.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Doorway {
    pub support: DoorwaySupport,
    centered: u8,
    previous_y: [Option<i32>; 4],
    last_tick: Option<u64>,
}
impl Doorway {
    pub fn new(support: DoorwaySupport) -> Self {
        Self {
            support,
            centered: 0,
            previous_y: [None; 4],
            last_tick: None,
        }
    }
    pub fn advance(&mut self, heroes: &[Option<Position>; 4], tick: u64) -> Result<u8> {
        if self.last_tick.is_some_and(|last| tick < last) {
            return Err(Error::State("doorway clock moved backwards"));
        }
        if self.last_tick == Some(tick) {
            return Ok(0);
        }
        self.last_tick = Some(tick);
        let mut centered = 0;
        let mut crossed = 0;
        for (seat, hero) in heroes.iter().enumerate() {
            let Some(hero) = hero else {
                continue;
            };
            if hero.level == self.support.level
                && self.centered & (1 << seat) != 0
                && hero.y < self.support.y
                && self.previous_y[seat].is_some_and(|y| y >= self.support.y)
            {
                crossed |= 1 << seat;
            }
            if self.support.supports_tile(
                hero.level,
                hero.y,
                hero.x.div_euclid(8),
                hero.y.div_euclid(8),
            ) && (i64::from(hero.x) - i64::from(self.support.x)).abs() < 6
            {
                centered |= 1 << seat;
            }
        }
        self.previous_y = heroes.map(|hero| {
            hero.filter(|hero| hero.level == self.support.level)
                .map(|hero| hero.y)
        });
        self.centered = centered & !crossed;
        self.support.open = centered != 0;
        Ok(crossed)
    }
    pub fn validate(&self, world: &WorldState) -> Result<()> {
        let floor = world
            .floor(self.support.level)
            .ok_or(Error::State("doorway floor"))?;
        self.support
            .validate(floor.generated.geometry.dimensions())?;
        if self.centered & !15 != 0 {
            return Err(Error::State("doorway contact mask"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum PhonePhase {
    Dormant,
    Ringing { rings_left: u8, delay: u16 },
    Revealing { seat: u8 },
    Spent,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhoneEvent {
    None,
    Ring,
    Reveal { seat: u8, level: LevelId },
    Expired,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Telephone {
    pub position: Position,
    phase: PhonePhase,
    last_tick: Option<u64>,
}
impl Telephone {
    pub fn new(position: Position) -> Self {
        Self {
            position,
            phase: PhonePhase::Dormant,
            last_tick: None,
        }
    }
    pub const fn phase(&self) -> PhonePhase {
        self.phase
    }
    pub fn advance(
        &mut self,
        heroes: &[Option<Position>; 4],
        sound_busy: bool,
        tick: u64,
    ) -> Result<PhoneEvent> {
        if self.last_tick.is_some_and(|last| tick < last) {
            return Err(Error::State("telephone clock moved backwards"));
        }
        if self.last_tick == Some(tick) {
            return Ok(PhoneEvent::None);
        }
        self.last_tick = Some(tick);
        let near = |hero: &Position, dx: i64, dy: i64| {
            hero.level == self.position.level
                && (i64::from(hero.x) - i64::from(self.position.x)).abs() < dx
                && (i64::from(hero.y) - i64::from(self.position.y)).abs() < dy
        };
        let mut event = PhoneEvent::None;
        if self.phase == PhonePhase::Dormant
            && !sound_busy
            && heroes.iter().flatten().any(|hero| near(hero, 320, 224))
        {
            self.phase = PhonePhase::Ringing {
                rings_left: 6,
                delay: 60,
            };
            event = PhoneEvent::Ring;
        }
        if let PhonePhase::Ringing { rings_left, delay } = self.phase {
            if delay > 1 {
                self.phase = PhonePhase::Ringing {
                    rings_left,
                    delay: delay - 1,
                };
            } else if rings_left <= 1
                || !heroes
                    .iter()
                    .flatten()
                    .any(|hero| hero.level == self.position.level)
            {
                self.phase = PhonePhase::Spent;
                return Ok(PhoneEvent::Expired);
            } else if sound_busy {
                self.phase = PhonePhase::Ringing {
                    rings_left,
                    delay: 1,
                };
            } else {
                self.phase = PhonePhase::Ringing {
                    rings_left: rings_left - 1,
                    delay: 60,
                };
                event = PhoneEvent::Ring;
            }
            for (seat, hero) in heroes.iter().enumerate() {
                if hero.as_ref().is_some_and(|hero| near(hero, 16, 8)) {
                    self.phase = PhonePhase::Revealing { seat: seat as u8 };
                    return Ok(PhoneEvent::Reveal {
                        seat: seat as u8,
                        level: self.position.level,
                    });
                }
            }
        }
        Ok(event)
    }
    /// The seat UI owner calls this when its nonblocking map overlay closes.
    /// Telephone reveal never changes the floor or awards exploration points.
    pub fn finish_reveal(&mut self, seat: u8) -> Result<()> {
        if self.phase != (PhonePhase::Revealing { seat }) {
            return Err(Error::State("telephone reveal owner/phase"));
        }
        self.phase = PhonePhase::Spent;
        Ok(())
    }
    pub fn validate(&self, world: &WorldState) -> Result<()> {
        let floor = world
            .floor(self.position.level)
            .ok_or(Error::State("telephone floor"))?;
        if floor
            .generated
            .geometry
            .dimensions()
            .world_cell(self.position.x, self.position.y)
            .is_none()
            || matches!(self.phase,PhonePhase::Ringing{rings_left,delay} if !(1..=6).contains(&rings_left) || delay==0 || delay>60)
            || matches!(self.phase,PhonePhase::Revealing{seat} if seat>=4)
        {
            return Err(Error::State("telephone position/timer/seat"));
        }
        Ok(())
    }
}
