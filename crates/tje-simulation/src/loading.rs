//! Aggregate-owned loading and transition continuations. Source candidates stay
//! private until geometry, stock items and real persistent population are ready.
use crate::population::PopulationFloor;
use crate::{EffectId, Error, Result};
use bincode::{Decode, Encode};
use sha2::{Digest, Sha256};
use tje_four_player::{
    CoopState, SeatId, SeatMask,
    state::{LifeState, Position},
};
use tje_world::{
    GeneratorId, LevelId, WorldConfig,
    loading::{ImportKind, ImportSpec},
    original::{
        CampaignSeeds, OriginalRevision, OriginalRng,
        geometry::{OriginalGeneration, OriginalGeometry, OriginalSpec},
        items::{OriginalContent, OriginalItems},
    },
    terrain::{DoorwaySupport, ElevatorBarrier, ElevatorDoor},
    transitions::{Doorway, Journey, Telephone},
    world::WorldState,
};

const CAMPAIGN_FLOORS: usize = 27;
const MAX_JOBS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct LoadingConfig {
    pub world: WorldConfig,
    pub heroes: SeatMask,
    pub revision: OriginalRevision,
    pub generator: GeneratorId,
}
impl LoadingConfig {
    pub fn validate(&self) -> Result<()> {
        self.world.validate().map_err(world_error)?;
        self.heroes
            .validate()
            .map_err(|_| Error::State("loading hero mask"))?;
        if self.heroes == SeatMask::NONE
            || (self.generator == GeneratorId::OriginalV1 && !self.world.faithful())
        {
            return Err(Error::State("loading configuration/stock dimensions"));
        }
        Ok(())
    }
    pub fn identity(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"tje-simulation/loading-config/1\0");
        hash.update(self.world.identity());
        hash.update([
            self.heroes.bits(),
            match self.revision {
                OriginalRevision::Rev00 => 0,
                OriginalRevision::Rev02 => 2,
            },
            self.generator.tag(),
        ]);
        hash.finalize().into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct CampaignManifest {
    pub initial_seeds: CampaignSeeds,
    pub accepted_seeds: CampaignSeeds,
    pub ship_levels: [LevelId; 10],
    pub rng_after: i32,
}
impl CampaignManifest {
    /// This mode's recorded u64 root selects the original 32-bit session stream.
    /// Geometry/item scoped seeds and all subsequent stock draw order are exact.
    fn prepare(root_seed: u64) -> Result<Self> {
        let mut rng = OriginalRng::new(root_seed as u32);
        let seeds = CampaignSeeds::random(&mut rng);
        let mut levels = [0_u16; 10];
        let mut draws = 0_u16;
        let mut draw = |rng: &mut OriginalRng, bound: u32| -> Result<u16> {
            draws = draws
                .checked_add(1)
                .ok_or(Error::Capacity("ship manifest draws"))?;
            if draws > 4096 {
                return Err(Error::Capacity("bounded ship assignment"));
            }
            Ok(rng.bounded(bound) as u16)
        };
        for group in 0..5_u16 {
            let piece = loop {
                let piece = usize::from(draw(&mut rng, 10)?);
                if levels[piece] == 0 {
                    break piece;
                }
            };
            loop {
                let level = draw(&mut rng, 5)? + group * 5 + 1;
                levels[piece] = level;
                if level > 1 && level < 25 {
                    break;
                }
            }
        }
        for _ in 0..5 {
            let piece = loop {
                let piece = usize::from(draw(&mut rng, 10)?);
                if levels[piece] == 0 {
                    break piece;
                }
            };
            let level = loop {
                let level = draw(&mut rng, 23)? + 2;
                if !levels.contains(&level) {
                    break level;
                }
            };
            levels[piece] = level;
        }
        let final_piece = usize::from(draw(&mut rng, 10)?);
        levels[final_piece] = 25;
        Ok(Self {
            initial_seeds: seeds.clone(),
            accepted_seeds: seeds,
            ship_levels: levels.map(|level| LevelId::new(level).expect("bounded ship assignment")),
            rng_after: rng.state(),
        })
    }
    fn validate(&self, root_seed: u64) -> Result<()> {
        let expected = Self::prepare(root_seed)?;
        if self.initial_seeds != expected.initial_seeds
            || self.ship_levels != expected.ship_levels
            || self.rng_after != expected.rng_after
        {
            return Err(Error::State(
                "campaign manifest differs from immutable root",
            ));
        }
        Ok(())
    }
    pub fn ship_assigned(&self, level: LevelId) -> bool {
        self.ship_levels.contains(&level)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum SourceFailure {
    Generation,
    Population,
    Items,
    Import,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub enum SourceStage {
    Geometry(OriginalGeneration),
    Specials {
        geometry: OriginalGeometry,
        job: tje_world::original::items::StockSpecialJob,
    },
    Items {
        geometry: OriginalGeometry,
        items: OriginalItems,
    },
    Population {
        geometry: OriginalGeometry,
        content: OriginalContent,
        job: crate::population::StockPopulationJob,
    },
    Import {
        geometry: OriginalGeometry,
        content: OriginalContent,
        population: PopulationFloor,
        started: bool,
    },
    Rejected {
        spec: OriginalSpec,
        reason: SourceFailure,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct SourceJob {
    pub level: LevelId,
    pub stage: SourceStage,
}
impl SourceJob {
    fn validate(&self, config: &LoadingConfig, manifest: &CampaignManifest) -> Result<()> {
        self.level.validate().map_err(world_error)?;
        if !matches!(self.level.get(), 0..=26) || config.generator != GeneratorId::OriginalV1 {
            return Err(Error::State("stock source job campaign/generator"));
        }
        let spec = match &self.stage {
            SourceStage::Specials { geometry, job } => {
                geometry.validate().map_err(world_error)?;
                job.validate().map_err(world_error)?;
                if job.level() != self.level || job.seed() != geometry.accepted_seed {
                    return Err(Error::State("stock special source level/seed"));
                }
                let ship = if manifest.ship_assigned(self.level) {
                    geometry.ship_position()
                } else {
                    None
                };
                if job.ship_position() != ship {
                    return Err(Error::State("pending stock ship prefix identity"));
                }
                &geometry.spec
            }
            SourceStage::Geometry(job) => {
                job.validate().map_err(world_error)?;
                job.spec()
            }
            SourceStage::Items { geometry, items } => {
                geometry.validate().map_err(world_error)?;
                items.validate(geometry).map_err(world_error)?;
                &geometry.spec
            }
            SourceStage::Population {
                geometry,
                content,
                job,
            } => {
                geometry.validate().map_err(world_error)?;
                content.validate_complete(geometry).map_err(world_error)?;
                job.validate().map_err(world_error)?;
                if job.level() != self.level || job.dimensions() != geometry.geometry.dimensions() {
                    return Err(Error::State("population source geometry/level"));
                }
                &geometry.spec
            }
            SourceStage::Import {
                geometry,
                content,
                population,
                ..
            } => {
                geometry.validate().map_err(world_error)?;
                content.validate_complete(geometry).map_err(world_error)?;
                if population.level != self.level {
                    return Err(Error::State("prepared population source floor"));
                }
                &geometry.spec
            }
            SourceStage::Rejected { spec, .. } => spec,
        };
        let seed = if self.level.get() == 26 {
            0
        } else {
            manifest
                .accepted_seeds
                .get(self.level)
                .map_err(world_error)?
        };
        if spec.level != self.level
            || spec.seed != seed
            || spec.rom_sha1 != config.world.rom_sha1
            || spec.revision != config.revision
            || spec.ambient_rng != manifest.rng_after
            || spec.directional_path_mode
        {
            return Err(Error::State(
                "pending stock source configuration/seed/revision",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct AcceptedSource {
    pub level: LevelId,
    pub requested_seed: u16,
    pub accepted_seed: u16,
    pub attempt: u8,
    pub geometry_identity: [u8; 32],
    pub import_identity: [u8; 32],
    pub content: OriginalContent,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct DoorwayBinding {
    pub owner: SeatId,
    pub effect: Option<EffectId>,
    pub machine: Doorway,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct TelephoneBinding {
    pub owner: SeatId,
    pub effect: Option<EffectId>,
    pub machine: Telephone,
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct LoadingState {
    config: LoadingConfig,
    configuration: [u8; 32],
    campaign: Option<CampaignManifest>,
    jobs: Vec<SourceJob>,
    accepted: Vec<AcceptedSource>,
    pub journeys: [Option<Journey>; 4],
    pub doors: [Option<DoorwayBinding>; 4],
    pub telephones: [Option<TelephoneBinding>; 4],
    pub initial_setup: bool,
    pub highest_level: LevelId,
    pub next_fall_token: u32,
    last_source_tick: Option<u64>,
    last_transition_tick: Option<u64>,
    initial_searches: [Option<tje_world::terrain::PlacementSearch>; 4],
    initial_subcell: [u8; 4],
    initial_arrivals: [Option<crate::players::Landing>; 4],
    initial_tick: Option<u64>,
    prop_tick: Option<u64>,
    door_crossings: [u8; 4],
    door_searches: [[Option<tje_world::terrain::PlacementSearch>; 4]; 4],
    door_ticks: [[Option<u64>; 4]; 4],
    transported_tick: [Option<u64>; 4],
    reveals: [Option<Reveal>; 4],
    fall_tokens: [Option<u32>; 4],
    source_cursor: u8,
    door_rng: [tje_world::random::StageRng; 4],
    elevators: Vec<ElevatorState>,
    elevator_sensor_tick: Option<u64>,
    elevator_door_tick: Option<u64>,
    previous_heroes: [Option<tje_world::transitions::Position>; 4],
}
impl LoadingState {
    pub fn new(world: &WorldConfig, heroes: SeatMask, revision: OriginalRevision) -> Result<Self> {
        Self::with_config(LoadingConfig {
            world: world.clone(),
            heroes,
            revision,
            generator: if world.faithful() {
                GeneratorId::OriginalV1
            } else {
                GeneratorId::ExpandedV1
            },
        })
    }
    pub fn with_config(config: LoadingConfig) -> Result<Self> {
        config.validate()?;
        let campaign = (config.generator == GeneratorId::OriginalV1)
            .then(|| CampaignManifest::prepare(config.world.root_seed))
            .transpose()?;
        let configuration = config.identity();
        let door_rng = std::array::from_fn(|owner| {
            tje_world::random::StageRng::derive(
                &configuration,
                LevelId::new(0).expect("derivation level"),
                owner as u8,
                b"doorway-source",
            )
        });
        Ok(Self {
            configuration,
            config,
            campaign,
            jobs: Vec::with_capacity(MAX_JOBS),
            accepted: Vec::with_capacity(CAMPAIGN_FLOORS),
            journeys: std::array::from_fn(|_| None),
            doors: std::array::from_fn(|_| None),
            telephones: std::array::from_fn(|_| None),
            initial_setup: false,
            highest_level: LevelId::new(1).expect("first floor"),
            next_fall_token: 1,
            last_source_tick: None,
            last_transition_tick: None,
            initial_searches: std::array::from_fn(|_| None),
            initial_subcell: [0; 4],
            initial_arrivals: [None; 4],
            initial_tick: None,
            prop_tick: None,
            door_crossings: [0; 4],
            door_searches: std::array::from_fn(|_| std::array::from_fn(|_| None)),
            door_ticks: [[None; 4]; 4],
            transported_tick: [None; 4],
            reveals: [None; 4],
            fall_tokens: [None; 4],
            source_cursor: 0,
            door_rng,
            elevators: Vec::with_capacity(CAMPAIGN_FLOORS),
            elevator_sensor_tick: None,
            elevator_door_tick: None,
            previous_heroes: [None; 4],
        })
    }
    pub fn config(&self) -> &LoadingConfig {
        &self.config
    }
    pub fn campaign(&self) -> Option<&CampaignManifest> {
        self.campaign.as_ref()
    }
    pub fn source_jobs(&self) -> &[SourceJob] {
        &self.jobs
    }
    pub fn accepted_sources(&self) -> &[AcceptedSource] {
        &self.accepted
    }
    pub fn validate_config(
        &self,
        world: &WorldConfig,
        heroes: SeatMask,
        revision: OriginalRevision,
        generator: GeneratorId,
    ) -> Result<()> {
        self.config.validate()?;
        if &self.config.world != world
            || self.config.heroes != heroes
            || self.config.revision != revision
            || self.config.generator != generator
            || self.configuration != self.config.identity()
        {
            return Err(Error::State("loading immutable session configuration"));
        }
        match (&self.campaign, generator) {
            (Some(campaign), GeneratorId::OriginalV1) => campaign.validate(world.root_seed)?,
            (None, GeneratorId::ExpandedV1 | GeneratorId::DesignedV1) => (),
            _ => return Err(Error::State("loading campaign/generator mismatch")),
        }
        Ok(())
    }
    pub fn prepare_floor(&mut self, world: &WorldState, level: LevelId) -> Result<bool> {
        level.validate().map_err(world_error)?;
        if world.floor(level).is_some() {
            return Ok(false);
        }
        if self.config.generator != GeneratorId::OriginalV1 || level.get() > 26 {
            return Err(Error::Ownership("stock source is not selected"));
        }
        match self.jobs.binary_search_by_key(&level, |job| job.level) {
            Ok(_) => Ok(false),
            Err(index) => {
                if self.jobs.len() == MAX_JOBS {
                    return Err(Error::Capacity("four disjoint source jobs"));
                }
                let manifest = self
                    .campaign
                    .as_ref()
                    .ok_or(Error::State("missing campaign seed manifest"))?;
                let seed = if level.get() == 26 {
                    0
                } else {
                    manifest.accepted_seeds.get(level).map_err(world_error)?
                };
                let job = OriginalGeneration::new(OriginalSpec {
                    revision: self.config.revision,
                    rom_sha1: self.config.world.rom_sha1,
                    level,
                    seed,
                    ambient_rng: manifest.rng_after,
                    directional_path_mode: false,
                })
                .map_err(world_error)?;
                self.jobs.insert(
                    index,
                    SourceJob {
                        level,
                        stage: SourceStage::Geometry(job),
                    },
                );
                Ok(true)
            }
        }
    }
    pub fn validate_world(&self, world: &WorldState, coop: &CoopState) -> Result<()> {
        if world.config() != &self.config.world
            || coop.config().participants != self.config.heroes
            || self.jobs.len() > MAX_JOBS
            || self.accepted.len() > CAMPAIGN_FLOORS
            || self.next_fall_token == 0
            || self.highest_level.get() > 26
        {
            return Err(Error::State("loading/world/roster extent"));
        }
        if self
            .jobs
            .windows(2)
            .any(|pair| pair[0].level >= pair[1].level)
            || self
                .accepted
                .windows(2)
                .any(|pair| pair[0].level >= pair[1].level)
        {
            return Err(Error::State("source records not sorted/unique"));
        }
        if let Some(manifest) = &self.campaign {
            for job in &self.jobs {
                job.validate(&self.config, manifest)?;
                if world.floor(job.level).is_some() {
                    return Err(Error::State("source job would overwrite stored floor"));
                }
            }
            if world.pending().is_some() {
                return Err(Error::State("ExpandedV1 generation in stock source state"));
            }
            if let Some(import) = world.importing()
                && !self.jobs.iter().any(|job| {
                    job.level == import.spec().level
                        && matches!(job.stage, SourceStage::Import { started: true, .. })
                })
            {
                return Err(Error::State("stock import has no aggregate source owner"));
            }
            for record in &self.accepted {
                let floor = world
                    .floor(record.level)
                    .ok_or(Error::State("accepted source has no persistent floor"))?;
                let requested = if record.level.get() == 26 {
                    0
                } else {
                    manifest
                        .initial_seeds
                        .get(record.level)
                        .map_err(world_error)?
                };
                if record.requested_seed != requested {
                    return Err(Error::State("accepted source changed requested seed"));
                }
                if record.level != record.content.level
                    || record.accepted_seed != record.content.seed
                    || record.attempt >= tje_world::MAX_ATTEMPTS
                    || record.geometry_identity != record.content.geometry_identity
                    || floor.spec.identity() != record.import_identity
                    || (record.level.get() < 26
                        && manifest
                            .accepted_seeds
                            .get(record.level)
                            .map_err(world_error)?
                            != record.accepted_seed)
                {
                    return Err(Error::State("accepted stock source identity/seed"));
                }
            }
            for index in 0..26 {
                let level = LevelId::new(index).expect("seed bank index");
                if !self.accepted.iter().any(|record| record.level == level)
                    && manifest.accepted_seeds.get(level).map_err(world_error)?
                        != manifest.initial_seeds.get(level).map_err(world_error)?
                {
                    return Err(Error::State(
                        "unvisited floor changed its immutable requested seed",
                    ));
                }
            }
        } else if !self.jobs.is_empty() || !self.accepted.is_empty() {
            return Err(Error::State("unselected stock source state"));
        }
        if self.elevators.len() > CAMPAIGN_FLOORS
            || self
                .elevators
                .windows(2)
                .any(|pair| pair[0].level >= pair[1].level)
        {
            return Err(Error::State("elevator floor ledger extent/order"));
        }
        for elevator in &self.elevators {
            if world.floor(elevator.level).is_none() {
                return Err(Error::State("elevator ledger has no floor"));
            }
            for door in elevator.doors {
                if matches!(door,DoorMotion::Opening{remaining}|DoorMotion::Closing{remaining} if remaining==0||remaining>12)
                    || matches!(door,DoorMotion::Open{remaining} if remaining==0||remaining>30)
                {
                    return Err(Error::State("elevator door timer continuation"));
                }
            }
        }
        let mut travelling = 0;
        for journey in self.journeys.iter().flatten() {
            journey.validate(world).map_err(world_error)?;
            if travelling & journey.cohort() != 0 {
                return Err(Error::State("overlapping journey cohorts"));
            }
            travelling |= journey.cohort();
        }
        if travelling & !self.config.heroes.bits() != 0 {
            return Err(Error::State("journey includes nonhero seat"));
        }
        for (index, door) in self.doors.iter().enumerate() {
            if let Some(door) = door {
                if door.owner.index() != index
                    || !self.config.heroes.contains(door.owner)
                    || door.effect.is_some_and(|id| !id.valid())
                {
                    return Err(Error::State("doorway effect owner/generation"));
                }
                door.machine.validate(world).map_err(world_error)?;
            }
        }
        for (index, phone) in self.telephones.iter().enumerate() {
            if let Some(phone) = phone {
                if phone.owner.index() != index
                    || !self.config.heroes.contains(phone.owner)
                    || phone.effect.is_some_and(|id| !id.valid())
                {
                    return Err(Error::State("telephone effect owner/generation"));
                }
                phone.machine.validate(world).map_err(world_error)?;
            }
        }
        if self.source_cursor >= 4 || self.door_rng.iter().any(|rng| !rng.valid()) {
            return Err(Error::State("loading work cursor/prop random state"));
        }
        for seat in SeatId::ALL {
            if let Some(search) = &self.initial_searches[seat.index()] {
                search.validate().map_err(world_error)?;
                if self.initial_setup || !self.config.heroes.contains(seat) {
                    return Err(Error::State("orphan initial placement search"));
                }
            }
            if self.initial_subcell[seat.index()] > 16
                || self.initial_subcell[seat.index()] != 0
                    && self.initial_searches[seat.index()]
                        .as_ref()
                        .is_none_or(|search| {
                            !matches!(
                                search.progress(),
                                tje_world::terrain::SearchProgress::Found(_)
                            )
                        })
            {
                return Err(Error::State("initial subcell placement cursor"));
            }
            if let Some(landing) = self.initial_arrivals[seat.index()] {
                if self.initial_setup
                    || !self.config.heroes.contains(seat)
                    || world.floor(landing.level).is_none()
                {
                    return Err(Error::State("initial arrival floor/hero"));
                }
                landing
                    .position
                    .validate()
                    .map_err(|_| Error::State("initial arrival position"))?;
            }
            if let Some(reveal) = self.reveals[seat.index()]
                && (!self.config.heroes.contains(seat)
                    || !self.config.heroes.contains(reveal.owner)
                    || world.floor(reveal.level).is_none()
                    || coop.players[seat.index()].level != Some(reveal.level))
            {
                return Err(Error::State("temporary phone reveal reference"));
            }
            for owner in SeatId::ALL {
                if let Some(search) = &self.door_searches[owner.index()][seat.index()] {
                    search.validate().map_err(world_error)?;
                    if self.doors[owner.index()].is_none() || !self.config.heroes.contains(seat) {
                        return Err(Error::State("orphan doorway placement search"));
                    }
                }
            }
            if let Some(token) = self.fall_tokens[seat.index()]
                && (token == 0
                    || self.journeys[seat.index()].as_ref().is_none_or(|journey| {
                        journey.kind() != tje_world::transitions::TravelKind::Fall
                    }))
            {
                return Err(Error::State("fall token has no owned journey"));
            }
            if self.door_crossings[seat.index()] & !self.config.heroes.bits() != 0 {
                return Err(Error::State("doorway crossing nonhero mask"));
            }
        }
        Ok(())
    }
    pub fn travelling_mask(&self) -> SeatMask {
        SeatMask::new(
            self.journeys
                .iter()
                .flatten()
                .fold(0, |mask, journey| mask | journey.cohort()),
        )
        .expect("four seats")
    }
    pub fn doorway_supports(&self, level: LevelId) -> [Option<DoorwaySupport>; 4] {
        self.doors.each_ref().map(|door| {
            door.as_ref()
                .filter(|door| door.machine.support.level == level)
                .map(|door| door.machine.support)
        })
    }
    pub fn elevator_barriers(
        &self,
        world: &WorldState,
        level: LevelId,
    ) -> tje_world::Result<[ElevatorBarrier; 2]> {
        let floor = world
            .floor(level)
            .ok_or(tje_world::Error::State("elevator floor is absent"))?;
        let anchors = floor.generated.geometry.anchors();
        let mut result = [ElevatorBarrier {
            level,
            x: 0,
            y: 0,
            door: ElevatorDoor::Inactive,
        }; 2];
        for (index, anchor) in [anchors.exit, anchors.arrival].into_iter().enumerate() {
            let state = self
                .elevators
                .binary_search_by_key(&level, |state| state.level)
                .ok()
                .map(|slot| self.elevators[slot].doors[index]);
            result[index] = ElevatorBarrier {
                level,
                x: (i32::from(anchor.x) + 2) * 128 - 16,
                y: (i32::from(anchor.y) + 2) * 64 - 5,
                door: if level.get() == 0
                    || level.get() == 26 && index == 0
                    || index == 1 && state.is_none_or(|state| state == DoorMotion::Closed)
                {
                    ElevatorDoor::Inactive
                } else if state.is_some_and(|state| matches!(state, DoorMotion::Open { .. })) {
                    ElevatorDoor::Open
                } else {
                    ElevatorDoor::Closed
                },
            };
        }
        Ok(result)
    }
}
fn world_error(error: tje_world::Error) -> Error {
    match error {
        tje_world::Error::Capacity(reason) => Error::Capacity(reason),
        tje_world::Error::Resource(reason) => Error::Ownership(reason),
        tje_world::Error::State(reason) | tje_world::Error::Bounds(reason) => Error::State(reason),
        _ => Error::State("invalid loading world source"),
    }
}

pub struct LoadingSources<'a, R, I, K> {
    pub terrain: &'a R,
    pub items: &'a I,
    pub catalog: &'a crate::population::DefinitionCatalog,
    pub stock_kind: K,
}

impl LoadingState {
    /// One deterministic source slice. No whole-world/seat/resource clone is
    /// used; the three owners preflight before the import's publication boundary.
    pub fn advance_sources<R, I, K>(
        &mut self,
        world: &mut WorldState,
        players: &mut crate::players::PlayerServices,
        population: &mut crate::population::PopulationState,
        sources: &LoadingSources<'_, R, I, K>,
        tick: u64,
        work: u32,
    ) -> Result<usize>
    where
        R: tje_world::original::resources::OriginalResources,
        I: crate::resources::ItemDefinitions,
        K: Fn(LevelId, u16) -> tje_world::Result<Option<crate::population::Kind>>,
    {
        if !(1..=1024).contains(&work)
            || sources.terrain.original_revision() != self.config.revision
        {
            return Err(Error::State("source work/revision"));
        }
        if self.last_source_tick.is_some_and(|last| tick < last) {
            return Err(Error::State("source clock moved backwards"));
        }
        if self.last_source_tick == Some(tick) {
            return Ok(0);
        }
        self.last_source_tick = Some(tick);
        let mut published = 0;
        for _ in 0..work {
            if self.jobs.is_empty() {
                break;
            }
            let index = usize::from(self.source_cursor) % self.jobs.len();
            if let Some(record) = advance_source_job(
                &mut self.jobs[index],
                &self.config,
                self.campaign
                    .as_ref()
                    .ok_or(Error::State("stock campaign missing"))?,
                world,
                players,
                population,
                sources,
            )? {
                if record.level.get() < 26 {
                    self.campaign
                        .as_mut()
                        .expect("stock manifest")
                        .accepted_seeds
                        .replace_accepted(record.level, record.accepted_seed)
                        .map_err(world_error)?;
                }
                let slot = self
                    .accepted
                    .binary_search_by_key(&record.level, |source| source.level)
                    .expect_err("new source level");
                self.accepted.insert(slot, record);
                self.jobs.remove(index);
                published += 1;
                self.source_cursor = (index % self.jobs.len().max(1)) as u8;
            } else {
                self.source_cursor = ((index + 1) % self.jobs.len()) as u8;
            }
        }
        Ok(published)
    }
    pub fn cancel_source(&mut self, world: &mut WorldState, level: LevelId) -> Result<()> {
        let index = self
            .jobs
            .binary_search_by_key(&level, |job| job.level)
            .map_err(|_| Error::State("source request absent"))?;
        if world
            .importing()
            .is_some_and(|job| job.spec().level == level)
        {
            world.cancel_loading();
        }
        self.jobs.remove(index);
        Ok(())
    }
}

fn advance_source_job<R, I, K>(
    job: &mut SourceJob,
    config: &LoadingConfig,
    manifest: &CampaignManifest,
    world: &mut WorldState,
    players: &mut crate::players::PlayerServices,
    population: &mut crate::population::PopulationState,
    sources: &LoadingSources<'_, R, I, K>,
) -> Result<Option<AcceptedSource>>
where
    R: tje_world::original::resources::OriginalResources,
    I: crate::resources::ItemDefinitions,
    K: Fn(LevelId, u16) -> tje_world::Result<Option<crate::population::Kind>>,
{
    use tje_world::original::geometry::OriginalPhase;
    use tje_world::original::items::{ItemPhase, StockSpecialJob};
    let request = match &job.stage {
        SourceStage::Geometry(value) => value.spec(),
        SourceStage::Specials { geometry, .. }
        | SourceStage::Items { geometry, .. }
        | SourceStage::Population { geometry, .. }
        | SourceStage::Import { geometry, .. } => &geometry.spec,
        SourceStage::Rejected { spec, .. } => spec,
    }
    .clone();
    let previous = std::mem::replace(
        &mut job.stage,
        SourceStage::Rejected {
            spec: request,
            reason: SourceFailure::Generation,
        },
    );
    match previous {
        SourceStage::Geometry(mut generation) => {
            let phase = match generation.advance(sources.terrain, 1) {
                Ok(phase) => phase,
                Err(error) => {
                    job.stage = SourceStage::Geometry(generation);
                    return Err(world_error(error));
                }
            };
            if phase == OriginalPhase::Rejected {
                return Err(Error::State("OriginalV1 exhausted generation attempts"));
            }
            if phase == OriginalPhase::Complete {
                let geometry = generation.finish().map_err(world_error)?;
                let specials = StockSpecialJob::new(
                    job.level,
                    geometry.accepted_seed,
                    &geometry.geometry,
                    if manifest.ship_assigned(job.level) {
                        geometry.ship_position()
                    } else {
                        None
                    },
                )
                .map_err(world_error)?;
                job.stage = SourceStage::Specials {
                    geometry,
                    job: specials,
                };
            } else {
                job.stage = SourceStage::Geometry(generation);
            }
        }
        SourceStage::Specials {
            geometry,
            job: mut specials,
        } => {
            let complete = match specials.advance(&geometry.geometry, sources.terrain, 1) {
                Ok(complete) => complete,
                Err(error) => {
                    job.stage = SourceStage::Specials {
                        geometry,
                        job: specials,
                    };
                    return Err(world_error(error));
                }
            };
            if complete {
                let specials = specials.finish().map_err(world_error)?;
                let items =
                    OriginalItems::new(&geometry, config.heroes.bits().count_ones() > 1, specials)
                        .map_err(world_error)?;
                job.stage = SourceStage::Items { geometry, items };
            } else {
                job.stage = SourceStage::Specials {
                    geometry,
                    job: specials,
                };
            }
        }
        SourceStage::Items {
            geometry,
            mut items,
        } => {
            let phase = match items.advance(&geometry, sources.terrain, 1) {
                Ok(phase) => phase,
                Err(error) => {
                    job.stage = SourceStage::Items { geometry, items };
                    return Err(world_error(error));
                }
            };
            if phase == ItemPhase::Rejected {
                return Err(Error::State("OriginalV1 item generation rejected"));
            }
            if phase == ItemPhase::Complete {
                let content = items.finish(&geometry).map_err(world_error)?;
                let prepared = prepare_population(&geometry, &content, population, sources)?;
                job.stage = SourceStage::Population {
                    geometry,
                    content,
                    job: prepared,
                };
            } else {
                job.stage = SourceStage::Items { geometry, items };
            }
        }
        SourceStage::Population {
            geometry,
            content,
            job: mut prepared,
        } => {
            let environment = CandidateEnvironment {
                level: job.level,
                geometry: &geometry.geometry,
                resources: sources.terrain,
            };
            let progress = match prepared.advance(&environment, sources.catalog, 1) {
                Ok(progress) => progress,
                Err(error) => {
                    job.stage = SourceStage::Population {
                        geometry,
                        content,
                        job: prepared,
                    };
                    return Err(world_error(error));
                }
            };
            if progress == crate::population::StockPopulationProgress::Complete {
                let mut prepared = prepared.finish().map_err(world_error)?;
                for id in 0..2 {
                    if content.specials.failed & (1 << id) != 0 {
                        prepared.failed.insert(id).map_err(world_error)?;
                    }
                }
                job.stage = SourceStage::Import {
                    geometry,
                    content,
                    population: prepared,
                    started: false,
                };
            } else {
                job.stage = SourceStage::Population {
                    geometry,
                    content,
                    job: prepared,
                };
            }
        }
        SourceStage::Import {
            geometry,
            content,
            population: prepared,
            mut started,
        } => {
            let result = (|| {
                population
                    .preflight_prepared_floor(&prepared)
                    .map_err(world_error)?;
                let stock = content.items.map(|item| crate::resources::StockItem {
                    slot: item.slot,
                    kind: item.kind,
                    level: item.level,
                    x: item.x,
                    y: item.y,
                });
                let item_binding = players
                    .resources
                    .prepare_stock_items(job.level, &stock, sources.items)
                    .map_err(player_error)?;
                if !started {
                    if world.importing().is_some() || world.pending().is_some() {
                        return Ok(None);
                    }
                    let mut actors = Vec::with_capacity(prepared.actors.len());
                    for actor in &prepared.actors {
                        if prepared.failed.contains(actor.id.local) {
                            continue;
                        }
                        actors.push(tje_world::population::Placement {
                            id: actor.id.local,
                            x: actor.motion.position.x >> 8,
                            y: actor.motion.position.y >> 8,
                            role: if actor.id.local == 0
                                && actor.kind == crate::population::Kind::Mailbox
                            {
                                tje_world::population::PlacementRole::Mailbox
                            } else {
                                tje_world::population::PlacementRole::Earthling
                            },
                            dormant_ticks: 0,
                            ai_rng: actor.rng,
                        });
                    }
                    let placements = content.population(&geometry, actors).map_err(world_error)?;
                    let kind = match job.level.get() {
                        0 => ImportKind::Special(tje_world::loading::SpecialFloor::Zero),
                        1 => ImportKind::Special(tje_world::loading::SpecialFloor::One),
                        26 => ImportKind::Special(tje_world::loading::SpecialFloor::Final),
                        _ => ImportKind::Stock,
                    };
                    world
                        .request_import(ImportSpec {
                            config: config.world.clone(),
                            level: job.level,
                            kind,
                            dimensions: geometry.geometry.dimensions(),
                            anchors: geometry.geometry.anchors().clone(),
                            population: placements,
                            source_digest: ImportSpec::content_digest(
                                geometry.geometry.dimensions(),
                                &geometry,
                            )
                            .map_err(world_error)?,
                        })
                        .map_err(world_error)?;
                    started = true;
                }
                if world
                    .importing()
                    .is_none_or(|pending| pending.spec().level != job.level)
                {
                    return Err(Error::State("source import ownership changed"));
                }
                let phase = world
                    .advance_import(&geometry, sources.terrain, 1)
                    .map_err(world_error)?;
                if phase == tje_world::loading::ImportProgress::Rejected {
                    return Err(Error::State("stock import source digest rejected"));
                }
                Ok((phase == tje_world::loading::ImportProgress::Ready).then_some(item_binding))
            })();
            match result {
                Ok(Some(item_binding)) => {
                    let record = AcceptedSource {
                        level: job.level,
                        requested_seed: geometry.spec.seed,
                        accepted_seed: geometry.accepted_seed,
                        attempt: geometry.accepted_attempt,
                        geometry_identity: geometry.identity(),
                        import_identity: world
                            .floor(job.level)
                            .expect("published stock floor")
                            .spec
                            .identity(),
                        content,
                    };
                    // Neither owner changed during the import slice, so these
                    // preflighted publications cannot fail or invoke geometry.
                    population
                        .install_prepared_floor(prepared)
                        .expect("preflighted stock population");
                    players.resources.commit_stock_items(item_binding);
                    return Ok(Some(record));
                }
                Ok(None) => {
                    job.stage = SourceStage::Import {
                        geometry,
                        content,
                        population: prepared,
                        started,
                    }
                }
                Err(error) => {
                    job.stage = SourceStage::Import {
                        geometry,
                        content,
                        population: prepared,
                        started,
                    };
                    return Err(error);
                }
            }
        }
        SourceStage::Rejected { spec, reason } => {
            job.stage = SourceStage::Rejected { spec, reason };
            return Err(Error::State("source request is rejected"));
        }
    }
    Ok(None)
}

fn prepare_population<R, I, K>(
    geometry: &OriginalGeometry,
    content: &OriginalContent,
    state: &crate::population::PopulationState,
    sources: &LoadingSources<'_, R, I, K>,
) -> Result<crate::population::StockPopulationJob>
where
    R: tje_world::original::resources::OriginalResources,
    I: crate::resources::ItemDefinitions,
    K: Fn(LevelId, u16) -> tje_world::Result<Option<crate::population::Kind>>,
{
    use crate::population::{AuthoredActor, Kind};
    let level = geometry.spec.level;
    let mut slots = [None; 20];
    let mut required = Vec::with_capacity(20);
    if !matches!(level.get(), 1 | 26) {
        for (index, slot) in slots.iter_mut().enumerate() {
            *slot = (sources.stock_kind)(level, index as u16).map_err(world_error)?;
        }
    }
    if level.get() == 0 {
        for (index, kind) in slots.iter().enumerate() {
            if let Some(kind) = kind {
                let (x, y) = match kind {
                    Kind::LemonadeStand => (1536, 1024),
                    Kind::HotTub => (1398, 980),
                    Kind::HulaDancer => (1536, 944),
                    _ => {
                        return Err(Error::Ownership(
                            "unsupported original level-zero actor position",
                        ));
                    }
                };
                required.push(AuthoredActor {
                    local: index as u16,
                    kind: *kind,
                    position: Position {
                        x: x * 256,
                        y: y * 256,
                        elevation: 0,
                    },
                    mail_order: false,
                });
            }
        }
    } else if level.ordinary() {
        for (index, position) in content.specials.positions.iter().enumerate() {
            if index == 0 || position.is_some() {
                slots[index] = None;
            }
            if let Some((x, y)) = position {
                required.push(AuthoredActor {
                    local: index as u16,
                    kind: if index == 0 {
                        Kind::Mailbox
                    } else {
                        Kind::ShipPiece
                    },
                    position: Position {
                        x: x * 256,
                        y: y * 256,
                        elevation: 0,
                    },
                    mail_order: index == 0 && content.specials.mail_order,
                });
            }
        }
    }
    state
        .prepare_stock_job(level, &slots, &required, geometry.geometry.dimensions())
        .map_err(world_error)
}

struct CandidateEnvironment<'a, R> {
    level: LevelId,
    geometry: &'a tje_world::LevelGeometry,
    resources: &'a R,
}
impl<R: tje_world::Resources> crate::population::Environment for CandidateEnvironment<'_, R> {
    fn dimensions(&self, level: LevelId) -> tje_world::Result<tje_world::Dimensions> {
        if level != self.level {
            return Err(tje_world::Error::State("candidate geometry level"));
        }
        Ok(self.geometry.dimensions())
    }
    fn terrain(&self, level: LevelId, x: i32, y: i32) -> tje_world::Result<u8> {
        if level != self.level {
            return Err(tje_world::Error::State("candidate terrain level"));
        }
        self.geometry.terrain_at(level, x, y, self.resources)
    }
    fn base_metatile(&self, level: LevelId, x: i32, y: i32) -> tje_world::Result<Option<u8>> {
        if level != self.level {
            return Err(tje_world::Error::State("candidate base level"));
        }
        Ok(self
            .geometry
            .dimensions()
            .world_cell(x, y)
            .and_then(|position| {
                self.geometry
                    .cell(i32::from(position.x), i32::from(position.y))
            })
            .map(|cell| cell.base() & 127))
    }
    fn placement(
        &self,
        level: LevelId,
        kind: crate::population::Kind,
        column: u16,
        row: u16,
        rng: &mut tje_world::random::StageRng,
    ) -> tje_world::Result<Option<(i32, i32)>> {
        if level != self.level {
            return Err(tje_world::Error::State("candidate placement level"));
        }
        let cell = self
            .geometry
            .cell(i32::from(column), i32::from(row))
            .ok_or(tje_world::Error::Bounds("candidate spawn cell"))?;
        let (width, height, mask) = kind.spawn_footprint();
        Ok(tje_world::terrain::position_in_cell(
            cell,
            tje_world::terrain::Footprint::new(width, height)?,
            tje_world::terrain::TerrainMask::new(mask)?,
            false,
            rng,
            self.resources,
        )?
        .map(|(x, y)| {
            (
                (i32::from(column) + 1) * 128 + x,
                (i32::from(row) + 1) * 64 + y,
            )
        }))
    }
    fn allows_placement(&self, level: LevelId, x: i32, y: i32) -> tje_world::Result<bool> {
        if level != self.level {
            return Err(tje_world::Error::State("candidate clearance level"));
        }
        if level.get() <= 1 {
            return Ok(true);
        }
        Ok([
            self.geometry.anchors().arrival,
            self.geometry.anchors().exit,
        ]
        .iter()
        .all(|anchor| {
            let ex = (i32::from(anchor.x) + 2) * 128 - 16;
            let ey = (i32::from(anchor.y) + 2) * 64;
            (i64::from(x) - i64::from(ex)).abs() >= 30 || (i64::from(y) - i64::from(ey)).abs() >= 20
        }))
    }
    fn animation_step(
        &self,
        _: crate::population::Kind,
        _: u8,
        _: &mut crate::population::AnimationCursor,
    ) -> tje_world::Result<crate::population::AnimationSample> {
        Err(tje_world::Error::State(
            "loading preparation cannot advance animation",
        ))
    }
}
fn player_error(error: crate::players::PlayerError) -> Error {
    match error {
        crate::players::PlayerError::Capacity(reason) => Error::Capacity(reason),
        crate::players::PlayerError::Invalid(reason)
        | crate::players::PlayerError::Geometry(reason)
        | crate::players::PlayerError::Rejected(reason) => Error::State(reason),
    }
}

impl LoadingState {
    /// S3 owns the already-preflighted hero mutation for Togetherness. Only the
    /// two world-retain ledgers move here, in one synchronous borrowed operation.
    pub fn relocate_world(
        &mut self,
        owner: SeatId,
        destination: crate::players::Landing,
        world: &mut WorldState,
        population: &mut crate::population::PopulationState,
    ) -> crate::players::PlayerResult<()> {
        use crate::players::PlayerError;
        if !self.config.heroes.contains(owner) || self.travelling_mask().contains(owner) {
            return Err(PlayerError::Rejected(
                "relocation hero is travelling or disabled",
            ));
        }
        destination
            .position
            .validate()
            .map_err(|_| PlayerError::Invalid("relocation position"))?;
        if !population
            .floors()
            .iter()
            .any(|floor| floor.level == destination.level)
        {
            return Err(PlayerError::Geometry("destination population absent"));
        }
        let mut levels = [None; 4];
        levels[owner.index()] = Some(destination.level);
        world
            .validate_cohort(1 << owner.get(), &levels)
            .map_err(|_| PlayerError::Geometry("relocation world destination"))?;
        let mut retained = *population.retained();
        retained[owner.index()] = Some(destination.level);
        let prepared = population
            .prepare_retains(retained)
            .map_err(|_| PlayerError::Capacity("relocation population reservation"))?;
        population
            .release_seat_contacts(owner)
            .expect("validated relocation seat");
        world
            .commit_cohort(1 << owner.get(), levels)
            .expect("preflighted relocation levels");
        population.commit_retains(prepared);
        self.highest_level = self.highest_level.max(destination.level);
        Ok(())
    }
    pub fn open_doorway(
        &mut self,
        owner: SeatId,
        level: LevelId,
        position: Position,
    ) -> crate::players::PlayerResult<()> {
        validate_prop_owner(&self.config, owner, level, position)?;
        self.doors[owner.index()] = Some(DoorwayBinding {
            owner,
            effect: None,
            machine: Doorway::new(DoorwaySupport {
                owner: owner.get(),
                level,
                x: position.x >> 8,
                y: position.y >> 8,
                open: false,
            }),
        });
        self.door_crossings[owner.index()] = 0;
        self.door_searches[owner.index()] = std::array::from_fn(|_| None);
        self.door_ticks[owner.index()] = [None; 4];
        Ok(())
    }
    pub fn open_telephone(
        &mut self,
        owner: SeatId,
        level: LevelId,
        position: Position,
    ) -> crate::players::PlayerResult<()> {
        validate_prop_owner(&self.config, owner, level, position)?;
        self.telephones[owner.index()] = Some(TelephoneBinding {
            owner,
            effect: None,
            machine: Telephone::new(tje_world::transitions::Position {
                level,
                x: position.x >> 8,
                y: position.y >> 8,
            }),
        });
        Ok(())
    }
    pub fn bind_doorway(
        &mut self,
        effect: EffectId,
        owner: SeatId,
        level: LevelId,
        position: Position,
    ) -> crate::players::PlayerResult<()> {
        use crate::players::PlayerError;
        let door = self
            .doors
            .get_mut(owner.index())
            .and_then(Option::as_mut)
            .ok_or(PlayerError::Rejected("doorway owner has no prepared prop"))?;
        if !effect.valid()
            || door.owner != owner
            || door.machine.support.level != level
            || door.machine.support.x != position.x >> 8
            || door.machine.support.y != position.y >> 8
            || door.effect.is_some_and(|bound| bound != effect)
        {
            return Err(PlayerError::Rejected("doorway effect generation/position"));
        }
        door.effect = Some(effect);
        Ok(())
    }
    pub fn bind_telephone(
        &mut self,
        effect: EffectId,
        owner: SeatId,
        level: LevelId,
        position: Position,
    ) -> crate::players::PlayerResult<()> {
        use crate::players::PlayerError;
        let phone = self
            .telephones
            .get_mut(owner.index())
            .and_then(Option::as_mut)
            .ok_or(PlayerError::Rejected(
                "telephone owner has no prepared prop",
            ))?;
        if !effect.valid()
            || phone.owner != owner
            || phone.machine.position.level != level
            || phone.machine.position.x != position.x >> 8
            || phone.machine.position.y != position.y >> 8
            || phone.effect.is_some_and(|bound| bound != effect)
        {
            return Err(PlayerError::Rejected(
                "telephone effect generation/position",
            ));
        }
        phone.effect = Some(effect);
        Ok(())
    }
    pub fn retire_effect(
        &mut self,
        effect: EffectId,
        level: LevelId,
    ) -> crate::players::PlayerResult<bool> {
        use crate::players::PlayerError;
        if !effect.valid() {
            return Err(PlayerError::Invalid("retired effect generation"));
        }
        let mut retired = false;
        for (index, door) in self.doors.iter_mut().enumerate() {
            if door.as_ref().is_some_and(|door| {
                door.effect == Some(effect) && door.machine.support.level == level
            }) {
                *door = None;
                self.door_crossings[index] = 0;
                self.door_searches[index] = std::array::from_fn(|_| None);
                self.door_ticks[index] = [None; 4];
                retired = true;
            }
        }
        for phone in &mut self.telephones {
            if phone.as_ref().is_some_and(|phone| {
                phone.effect == Some(effect) && phone.machine.position.level == level
            }) {
                *phone = None;
                retired = true;
            }
        }
        Ok(retired)
    }
}
fn validate_prop_owner(
    config: &LoadingConfig,
    owner: SeatId,
    level: LevelId,
    position: Position,
) -> crate::players::PlayerResult<()> {
    use crate::players::PlayerError;
    owner
        .validate()
        .map_err(|_| PlayerError::Invalid("prop owner"))?;
    level
        .validate()
        .map_err(|_| PlayerError::Invalid("prop level"))?;
    position
        .validate()
        .map_err(|_| PlayerError::Invalid("prop position"))?;
    if !config.heroes.contains(owner)
        || !tje_world::Dimensions::ORIGINAL
            .world_cell(position.x >> 8, position.y >> 8)
            .is_some()
    {
        return Err(PlayerError::Rejected("prop owner/stock world bounds"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Reveal {
    pub owner: SeatId,
    pub effect: Option<EffectId>,
    pub level: LevelId,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropEvent {
    Ring {
        owner: SeatId,
        level: LevelId,
    },
    Expired {
        owner: SeatId,
        effect: Option<EffectId>,
        level: LevelId,
    },
}
pub struct PreparedLoadingArrivals {
    players: crate::players::PreparedArrivals,
    population: crate::population::PreparedRetains,
    destinations: [Option<crate::players::Landing>; 4],
    initial: bool,
    journey: Option<usize>,
}

impl LoadingState {
    pub fn release_hero_world(
        &mut self,
        seat: SeatId,
        world: &mut WorldState,
        population: &mut crate::population::PopulationState,
    ) -> crate::players::PlayerResult<()> {
        use crate::players::PlayerError;
        seat.validate()
            .map_err(|_| PlayerError::Invalid("released hero seat"))?;
        if !self.config.heroes.contains(seat)
            || population.retained()[seat.index()] != world.active()[seat.index()]
        {
            return Err(PlayerError::Invalid(
                "released hero world/population references",
            ));
        }
        let mut retained = *population.retained();
        retained[seat.index()] = None;
        let prepared = population
            .prepare_retains(retained)
            .map_err(|_| PlayerError::Capacity("released population reservation"))?;
        // Keep surviving fall/rider members progressing with the same token.
        for (index, journey) in self.journeys.iter_mut().enumerate() {
            if let Some(active) = journey
                && active.cohort() & (1 << seat.get()) != 0
                && active
                    .remove_seat(seat.get())
                    .expect("validated released seat")
            {
                *journey = None;
                self.fall_tokens[index] = None;
            }
        }
        world
            .release_seat(seat.get())
            .expect("validated released seat");
        population
            .release_seat_contacts(seat)
            .expect("validated released contact seat");
        population.commit_retains(prepared);
        self.initial_arrivals[seat.index()] = None;
        self.initial_searches[seat.index()] = None;
        self.initial_subcell[seat.index()] = 0;
        self.reveals[seat.index()] = None;
        Ok(())
    }
    pub fn reveal(&self, seat: SeatId) -> Option<Reveal> {
        self.reveals[seat.index()]
    }
    pub fn tick_props(
        &mut self,
        coop: &mut CoopState,
        sound_busy: bool,
        tick: u64,
    ) -> Result<[Option<PropEvent>; 8]> {
        if self.prop_tick.is_some_and(|last| tick < last) {
            return Err(Error::State("prop clock moved backwards"));
        }
        let mut events = [None; 8];
        if self.prop_tick == Some(tick) {
            return Ok(events);
        }
        self.prop_tick = Some(tick);
        let heroes = coop.players.each_ref().map(|hero| {
            if hero.life == LifeState::Alive {
                hero.level.map(|level| tje_world::transitions::Position {
                    level,
                    x: hero.motion.position.x >> 8,
                    y: hero.motion.position.y >> 8,
                })
            } else {
                None
            }
        });
        for index in 0..4 {
            if let Some(door) = &mut self.doors[index] {
                self.door_crossings[index] |=
                    door.machine.advance(&heroes, tick).map_err(world_error)?;
                for (seat, hero) in heroes.iter().enumerate() {
                    if hero.is_none_or(|hero| {
                        hero.level != door.machine.support.level
                            || (i64::from(hero.x) - i64::from(door.machine.support.x)).abs() >= 6
                            || (i64::from(hero.y) - i64::from(door.machine.support.y)).abs() >= 8
                    }) {
                        self.door_crossings[index] &= !(1 << seat);
                        self.door_searches[index][seat] = None;
                    }
                }
            }
            if let Some(phone) = &mut self.telephones[index] {
                use tje_world::transitions::PhoneEvent;
                match phone
                    .machine
                    .advance(&heroes, sound_busy, tick)
                    .map_err(world_error)?
                {
                    PhoneEvent::Ring => {
                        events[index * 2] = Some(PropEvent::Ring {
                            owner: phone.owner,
                            level: phone.machine.position.level,
                        })
                    }
                    PhoneEvent::Expired => {
                        events[index * 2] = Some(PropEvent::Expired {
                            owner: phone.owner,
                            effect: phone.effect,
                            level: phone.machine.position.level,
                        })
                    }
                    PhoneEvent::Reveal { seat, level } => {
                        let hero =
                            SeatId::new(seat).ok_or(Error::State("telephone reveal seat"))?;
                        self.reveals[hero.index()] = Some(Reveal {
                            owner: phone.owner,
                            effect: phone.effect,
                            level,
                        });
                        coop.players[hero.index()].ui.overlay =
                            tje_four_player::state::Overlay::Map;
                    }
                    PhoneEvent::None => (),
                }
            }
        }
        for seat in SeatId::ALL {
            if let Some(reveal) = self.reveals[seat.index()]
                && (coop.players[seat.index()].ui.overlay != tje_four_player::state::Overlay::Map
                    || coop.players[seat.index()].life != LifeState::Alive
                    || coop.players[seat.index()].level != Some(reveal.level))
            {
                if let Some(phone) = &mut self.telephones[reveal.owner.index()]
                    && phone.effect == reveal.effect
                    && phone.machine.phase()
                        == (tje_world::transitions::PhonePhase::Revealing { seat: seat.get() })
                {
                    phone
                        .machine
                        .finish_reveal(seat.get())
                        .map_err(world_error)?;
                }
                self.reveals[seat.index()] = None;
            }
        }
        Ok(events)
    }
    pub fn answer_telephone(
        &mut self,
        effect: EffectId,
        owner: SeatId,
        hero: SeatId,
        level: LevelId,
    ) -> crate::players::PlayerResult<()> {
        use crate::players::PlayerError;
        let phone = self
            .telephones
            .get(owner.index())
            .and_then(Option::as_ref)
            .ok_or(PlayerError::Rejected("telephone prop absent"))?;
        if phone.effect != Some(effect)
            || phone.machine.position.level != level
            || !self.config.heroes.contains(hero)
        {
            return Err(PlayerError::Rejected("telephone effect/hero/floor"));
        }
        if self.reveals[hero.index()].is_none() {
            self.reveals[hero.index()] = Some(Reveal {
                owner,
                effect: Some(effect),
                level,
            });
        }
        Ok(())
    }
    pub fn doorway_crossing<R, F>(
        &mut self,
        request: DoorwayCrossing,
        world: &WorldState,
        resources: &R,
        blocked: F,
    ) -> crate::players::PlayerResult<Option<crate::players::Landing>>
    where
        R: tje_world::Resources,
        F: Fn(crate::players::Landing) -> crate::players::PlayerResult<bool>,
    {
        use crate::players::PlayerError;
        let DoorwayCrossing {
            effect,
            owner,
            hero,
            level,
            position,
            tick,
        } = request;
        let door = self
            .doors
            .get(owner.index())
            .and_then(Option::as_ref)
            .ok_or(PlayerError::Rejected("doorway prop absent"))?;
        if door.effect != Some(effect)
            || door.machine.support.level != level
            || !self.config.heroes.contains(hero)
        {
            return Err(PlayerError::Rejected("doorway effect/hero/floor"));
        }
        if self.transported_tick[hero.index()] == Some(tick)
            || self.door_ticks[owner.index()][hero.index()] == Some(tick)
        {
            return Ok(None);
        }
        if self.prop_tick != Some(tick) {
            return Err(PlayerError::Invalid("doorway contacts were not admitted"));
        }
        if self.door_crossings[owner.index()] & (1 << hero.get()) == 0 {
            return Ok(None);
        }
        if (i64::from(position.x >> 8) - i64::from(door.machine.support.x)).abs() >= 6
            || position.y >> 8 >= door.machine.support.y
        {
            return Ok(None);
        }
        let floor = world
            .floor(level)
            .ok_or(PlayerError::Geometry("doorway floor absent"))?;
        let search = &mut self.door_searches[owner.index()][hero.index()];
        if search.is_none() {
            *search = Some(
                tje_world::terrain::PlacementSearch::new(
                    floor.generated.geometry.dimensions(),
                    tje_world::terrain::Footprint::new(4, 2).expect("hero footprint"),
                    tje_world::terrain::TerrainMask::LAND_OR_SAND,
                    self.door_rng[owner.index()],
                )
                .map_err(|_| PlayerError::Geometry("doorway placement state"))?,
            );
        }
        self.door_ticks[owner.index()][hero.index()] = Some(tick);
        let state = search.as_mut().expect("prepared doorway search");
        let progress = state
            .advance(&floor.generated.geometry, level, resources, 32, &[])
            .map_err(|_| PlayerError::Geometry("doorway placement resource"))?;
        self.door_rng[owner.index()] = state.rng();
        match progress {
            tje_world::terrain::SearchProgress::Searching => Ok(None),
            tje_world::terrain::SearchProgress::Exhausted => {
                Err(PlayerError::Rejected("doorway has no safe destination"))
            }
            tje_world::terrain::SearchProgress::Found((x, y)) => {
                let landing = crate::players::Landing {
                    level,
                    position: Position {
                        x: x * 256,
                        y: y * 256,
                        elevation: 0,
                    },
                };
                if blocked(landing)? {
                    state.skip_found().expect("found doorway candidate");
                    return Ok(None);
                }
                self.door_crossings[owner.index()] &= !(1 << hero.get());
                *search = None;
                self.transported_tick[hero.index()] = Some(tick);
                Ok(Some(landing))
            }
        }
    }
}

impl LoadingState {
    pub fn advance_initial_positions<R: tje_world::Resources>(
        &mut self,
        world: &WorldState,
        coop: &CoopState,
        resources: &R,
        tick: u64,
        work: u32,
    ) -> Result<bool> {
        if self.initial_setup {
            return Ok(true);
        }
        if !(1..=1024).contains(&work) || self.initial_tick.is_some_and(|last| tick < last) {
            return Err(Error::State("initial placement clock/work"));
        }
        if self.initial_tick == Some(tick) {
            return Ok(self
                .config
                .heroes
                .iter()
                .all(|seat| self.initial_arrivals[seat.index()].is_some()));
        }
        self.initial_tick = Some(tick);
        let level = LevelId::new(1).expect("first floor");
        let Some(floor) = world.floor(level) else {
            self.prepare_floor(world, level)?;
            return Ok(false);
        };
        let barriers = self.elevator_barriers(world, level).map_err(world_error)?;
        let exclusions = [
            barriers[0].placement_exclusion().map_err(world_error)?,
            barriers[1].placement_exclusion().map_err(world_error)?,
        ];
        for _ in 0..work {
            let Some(seat) = self
                .config
                .heroes
                .iter()
                .find(|seat| self.initial_arrivals[seat.index()].is_none())
            else {
                break;
            };
            if coop.players[seat.index()].life != LifeState::Unplaced {
                return Err(Error::State("initial placement requires unplaced hero"));
            }
            let search = &mut self.initial_searches[seat.index()];
            if search.is_none() {
                *search = Some(
                    tje_world::terrain::PlacementSearch::new(
                        floor.generated.geometry.dimensions(),
                        tje_world::terrain::Footprint::new(16, 8).expect("special floor footprint"),
                        tje_world::terrain::TerrainMask::LAND_OR_SAND,
                        tje_world::random::StageRng::derive(
                            &self.configuration,
                            level,
                            seat.get(),
                            b"initial-arrival",
                        ),
                    )
                    .map_err(world_error)?,
                );
            }
            let search = search.as_mut().expect("initial placement search");
            // G/map/terrain_placement.c qualifies a full 16×8 support cell on
            // floor one, then places a cohort in it. One centre per hero would
            // exhaust a valid one-cell island after its first participant.
            // Keep that full-cell rule and examine one real 4×2 footprint per
            // work unit; never relax the terrain, elevator or overlap tests.
            match search
                .advance(&floor.generated.geometry, level, resources, 1, &[])
                .map_err(world_error)?
            {
                tje_world::terrain::SearchProgress::Searching => (),
                tje_world::terrain::SearchProgress::Exhausted => {
                    return Err(Error::Capacity("initial four-hero safe placement"));
                }
                tje_world::terrain::SearchProgress::Found((center_x, center_y)) => {
                    let cursor = self.initial_subcell[seat.index()];
                    let (x, y) = if cursor == 0 {
                        (center_x, center_y)
                    } else {
                        let index = i32::from(cursor - 1);
                        (
                            center_x - 63 + 16 + (index % 4) * 32,
                            center_y - 31 + 14 + (index / 4) * 16,
                        )
                    };
                    let blocked = exclusions
                        .iter()
                        .any(|barrier| barrier.contains(level, (x, y)))
                        || self.initial_arrivals.iter().flatten().any(|other| {
                            (i64::from(other.position.x >> 8) - i64::from(x)).abs() < 32
                                && (i64::from(other.position.y >> 8) - i64::from(y)).abs() < 16
                        });
                    let mut supported = !blocked;
                    if supported {
                        'footprint: for row in (y - 14).div_euclid(8)..=(y + 1).div_euclid(8) {
                            for column in (x - 16).div_euclid(8)..=(x + 15).div_euclid(8) {
                                let terrain = floor
                                    .generated
                                    .geometry
                                    .terrain_at(level, column * 8, row * 8, resources)
                                    .map_err(world_error)?;
                                if !tje_world::terrain::TerrainMask::LAND_OR_SAND.accepts(terrain) {
                                    supported = false;
                                    break 'footprint;
                                }
                            }
                        }
                    }
                    if supported {
                        self.initial_arrivals[seat.index()] = Some(crate::players::Landing {
                            level,
                            position: Position {
                                x: x * 256,
                                y: y * 256,
                                elevation: 0,
                            },
                        });
                        self.initial_searches[seat.index()] = None;
                        self.initial_subcell[seat.index()] = 0;
                    } else if cursor == 16 {
                        search.skip_found().map_err(world_error)?;
                        self.initial_subcell[seat.index()] = 0;
                    } else {
                        self.initial_subcell[seat.index()] = cursor + 1;
                    }
                }
            }
        }
        Ok(self
            .config
            .heroes
            .iter()
            .all(|seat| self.initial_arrivals[seat.index()].is_some()))
    }
    pub fn prepare_initial_arrivals(
        &self,
        world: &WorldState,
        coop: &CoopState,
        players: &crate::players::PlayerServices,
        population: &crate::population::PopulationState,
        geometry: &impl crate::players::PlayerGeometry,
    ) -> Result<PreparedLoadingArrivals> {
        if self.initial_setup
            || !self
                .config
                .heroes
                .iter()
                .all(|seat| self.initial_arrivals[seat.index()].is_some())
        {
            return Err(Error::State(
                "initial source/placement transaction is not ready",
            ));
        }
        for seat in self.config.heroes.iter() {
            let hero = &coop.players[seat.index()];
            if hero.life != LifeState::Unplaced
                || hero.inventory.iter().any(Option::is_some)
                || world.active()[seat.index()].is_some()
            {
                return Err(Error::State(
                    "initial setup must precede every arrival commit",
                ));
            }
        }
        self.prepare_arrivals(
            ArrivalContext {
                world,
                coop,
                players,
                population,
                geometry,
            },
            ArrivalRequest {
                destinations: self.initial_arrivals,
                policy: crate::players::ArrivalPolicy::Setup { lives: 3 },
                initial: true,
                journey: None,
            },
        )
    }
    fn prepare_arrivals<G: crate::players::PlayerGeometry>(
        &self,
        context: ArrivalContext<'_, G>,
        request: ArrivalRequest,
    ) -> Result<PreparedLoadingArrivals> {
        let ArrivalContext {
            world,
            coop,
            players,
            population,
            geometry,
        } = context;
        let ArrivalRequest {
            destinations,
            policy,
            initial,
            journey,
        } = request;
        for seat in SeatId::ALL {
            if let Some(landing) = destinations[seat.index()]
                && (!self.config.heroes.contains(seat)
                    || world.floor(landing.level).is_none()
                    || !population
                        .floors()
                        .iter()
                        .any(|floor| floor.level == landing.level))
            {
                return Err(Error::State("arrival floor/population/hero reference"));
            }
        }
        let prepared = players
            .prepare_arrivals(coop, destinations, policy, geometry)
            .map_err(player_error)?;
        let mut retained = *population.retained();
        for seat in SeatId::ALL {
            if let Some(landing) = destinations[seat.index()] {
                retained[seat.index()] = Some(landing.level);
            }
        }
        let retained = population.prepare_retains(retained).map_err(world_error)?;
        Ok(PreparedLoadingArrivals {
            players: prepared,
            population: retained,
            destinations,
            initial,
            journey,
        })
    }
    /// Linear token, consumed immediately after preparation; no geometry query,
    /// allocation, actor spawn or other fallible work is performed here.
    pub fn commit_arrivals(
        &mut self,
        prepared: PreparedLoadingArrivals,
        world: &mut WorldState,
        coop: &mut CoopState,
        players: &mut crate::players::PlayerServices,
        population: &mut crate::population::PopulationState,
    ) {
        if prepared.initial {
            players
                .initialize_stock_inventory(coop)
                .expect("preflighted initial inventory");
        }
        for seat in SeatId::ALL {
            if prepared.destinations[seat.index()].is_some() {
                population
                    .release_seat_contacts(seat)
                    .expect("preflighted arrival contact seat");
            }
        }
        players.commit_arrivals(coop, prepared.players);
        for seat in SeatId::ALL {
            if let Some(landing) = prepared.destinations[seat.index()] {
                world
                    .set_active(seat.get(), landing.level)
                    .expect("preflighted arrival floor");
                self.highest_level = self.highest_level.max(landing.level);
                if prepared.journey.is_some_and(|index| {
                    self.journeys[index].as_ref().is_some_and(|journey| {
                        matches!(
                            journey.kind(),
                            tje_world::transitions::TravelKind::Elevator
                                | tje_world::transitions::TravelKind::Ship { .. }
                        )
                    })
                }) {
                    let slot = self.ensure_elevator(landing.level);
                    self.elevators[slot].doors[1] = DoorMotion::Open { remaining: 30 };
                }
            }
        }
        population.commit_retains(prepared.population);
        if let Some(index) = prepared.journey {
            self.journeys[index]
                .as_mut()
                .expect("prepared journey")
                .commit_after_world(world)
                .expect("preflighted journey world commit");
        }
        if prepared.initial {
            self.initial_setup = true;
            self.initial_arrivals = [None; 4];
            self.initial_searches = std::array::from_fn(|_| None);
            self.initial_subcell = [0; 4];
        }
    }
    pub fn begin_journey(
        &mut self,
        world: &WorldState,
        kind: tje_world::transitions::TravelKind,
        destinations: [Option<tje_world::transitions::Destination>; 4],
        collected_parts: u16,
        fall_token: Option<u32>,
    ) -> Result<usize> {
        let journey = Journey::new(
            world,
            kind,
            tje_world::transitions::TravelTiming::default(),
            destinations,
            collected_parts,
        )
        .map_err(world_error)?;
        if journey.cohort() & !self.config.heroes.bits() != 0
            || self
                .journeys
                .iter()
                .flatten()
                .any(|other| other.cohort() & journey.cohort() != 0)
        {
            return Err(Error::State("journey overlaps another cohort/nonhero"));
        }
        if (kind == tje_world::transitions::TravelKind::Fall) != fall_token.is_some() {
            return Err(Error::State("fall journey token policy"));
        }
        let slot = self
            .journeys
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity("four disjoint journeys"))?;
        self.journeys[slot] = Some(journey);
        self.fall_tokens[slot] = fall_token;
        Ok(slot)
    }
    pub fn begin_elevator(
        &mut self,
        world: &WorldState,
        cohort: SeatMask,
        collected_parts: u16,
    ) -> Result<usize> {
        let first = cohort
            .iter()
            .next()
            .ok_or(Error::State("empty elevator cohort"))?;
        let source = world.active()[first.index()].ok_or(Error::State("elevator hero unplaced"))?;
        if source.get() == 0 || source.get() >= 26 {
            return Err(Error::State("special floor has no ascending elevator"));
        }
        if cohort
            .iter()
            .any(|seat| world.active()[seat.index()] != Some(source))
        {
            return Err(Error::State("elevator cohort floors differ"));
        }
        let level = LevelId::new(source.get() + 1).expect("campaign elevator destination");
        let landing = if let Some(floor) = world.floor(level) {
            let anchor = floor.generated.geometry.anchors().arrival;
            let x = (i32::from(anchor.x) + 2) * 128 - 16;
            let y = (i32::from(anchor.y) + 2) * 64 + 16;
            tje_world::transitions::Landing::Exact(x, y)
        } else {
            tje_world::transitions::Landing::Arrival
        };
        let destinations = std::array::from_fn(|index| {
            cohort
                .contains(SeatId::ALL[index])
                .then_some(tje_world::transitions::Destination { level, landing })
        });
        let kind = if source.get() == 25 {
            tje_world::transitions::TravelKind::Ship {
                required_parts: 0x3ff,
            }
        } else {
            tje_world::transitions::TravelKind::Elevator
        };
        self.begin_journey(world, kind, destinations, collected_parts, None)
    }
    pub fn begin_pending_falls(
        &mut self,
        world: &WorldState,
        coop: &mut CoopState,
        players: &mut crate::players::PlayerServices,
    ) -> Result<Option<u32>> {
        let requested = players.pending_falls();
        let seats = SeatMask::new(requested.bits() & !self.travelling_mask().bits())
            .ok_or(Error::State("fall cohort mask"))?;
        if seats == SeatMask::NONE {
            return Ok(None);
        }
        let token = self.next_fall_token;
        let next = token
            .checked_add(1)
            .ok_or(Error::Capacity("fall token identity"))?;
        let mut destinations = [None; 4];
        for seat in seats.iter() {
            let source = world.active()[seat.index()].ok_or(Error::State("fall hero floor"))?;
            destinations[seat.index()] = Some(
                tje_world::transitions::Destination::campaign_fall(source, self.highest_level)
                    .map_err(world_error)?,
            );
        }
        let slot = self.begin_journey(
            world,
            tje_world::transitions::TravelKind::Fall,
            destinations,
            0,
            Some(token),
        )?;
        if let Err(error) = players.begin_fall(coop, crate::players::FallCohort { token, seats }) {
            self.journeys[slot] = None;
            self.fall_tokens[slot] = None;
            return Err(player_error(error));
        }
        self.next_fall_token = next;
        Ok(Some(token))
    }
    pub fn advance_journeys<R: tje_world::Resources>(
        &mut self,
        world: &mut WorldState,
        players: &crate::players::PlayerServices,
        resources: &R,
        tick: u64,
        work: u32,
    ) -> Result<()> {
        use tje_world::transitions::TravelPhase;
        if !(1..=1024).contains(&work) || self.last_transition_tick.is_some_and(|last| tick < last)
        {
            return Err(Error::State("journey clock/work"));
        }
        if self.last_transition_tick == Some(tick) {
            return Ok(());
        }
        self.last_transition_tick = Some(tick);
        for index in 0..4 {
            let Some(journey) = self.journeys[index].as_ref() else {
                continue;
            };
            if self.fall_tokens[index].is_some_and(|token| !players.fall_ready(token)) {
                continue;
            }
            let mut missing = [None; 4];
            for (seat, destination) in journey.destinations().iter().enumerate() {
                if let Some(destination) = destination
                    && world.floor(destination.level).is_none()
                {
                    missing[seat] = Some(destination.level);
                }
            }
            for level in missing.into_iter().flatten() {
                self.prepare_floor(world, level)?;
            }
            let journey = self.journeys[index].as_mut().expect("journey");
            if journey.phase() == TravelPhase::Loading
                && journey
                    .destinations()
                    .iter()
                    .flatten()
                    .any(|destination| world.floor(destination.level).is_none())
            {
                continue;
            }
            if journey.phase() == TravelPhase::Loading
                && matches!(
                    journey.kind(),
                    tje_world::transitions::TravelKind::Elevator
                        | tje_world::transitions::TravelKind::Ship { .. }
                )
            {
                let members = journey.cohort().count_ones();
                let mut rank = 0;
                let mut landings = [None; 4];
                for (seat, destination) in journey.destinations().iter().enumerate() {
                    if let Some(destination) = destination {
                        let anchor = world
                            .floor(destination.level)
                            .expect("loaded elevator floor")
                            .generated
                            .geometry
                            .anchors()
                            .arrival;
                        let (dx, dy) = if members == 1 {
                            (0, 20)
                        } else {
                            (if rank % 2 == 0 { -20 } else { 20 }, 20 + (rank / 2) * 20)
                        };
                        landings[seat] = Some(tje_world::transitions::Landing::Exact(
                            (i32::from(anchor.x) + 2) * 128 - 16 + dx,
                            (i32::from(anchor.y) + 2) * 64 - 5 + dy,
                        ));
                        rank += 1;
                    }
                }
                journey.retarget_landings(landings).map_err(world_error)?;
            }
            let baseline = tje_world::population::PopulationBaseline {
                walkable_tiles: 1,
                ordinary_items: 0,
                earthlings: 0,
                reserved_items: 0,
                ship_piece: false,
                mailbox: false,
            };
            // Every destination is supplied by OriginalV1, never by the
            // standalone model's ExpandedV1 missing-floor branch.
            let phase = journey
                .advance(world, resources, baseline, tick, work, &[])
                .map_err(world_error)?;
            if phase == TravelPhase::Complete {
                self.journeys[index] = None;
                self.fall_tokens[index] = None;
            }
            if phase == TravelPhase::Failed {
                return Err(Error::State("journey safe placement rejected"));
            }
        }
        Ok(())
    }
    pub fn prepare_journey_arrival(
        &self,
        index: usize,
        world: &WorldState,
        coop: &CoopState,
        players: &crate::players::PlayerServices,
        population: &crate::population::PopulationState,
        geometry: &impl crate::players::PlayerGeometry,
    ) -> Result<PreparedLoadingArrivals> {
        let journey = self
            .journeys
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(Error::State("journey slot absent"))?;
        if journey.phase() != tje_world::transitions::TravelPhase::Ready {
            return Err(Error::State("journey is not ready"));
        }
        journey.validate(world).map_err(world_error)?;
        let destinations = journey.arrivals().map(|point| {
            point.map(|point| crate::players::Landing {
                level: point.level,
                position: Position {
                    x: point.x * 256,
                    y: point.y * 256,
                    elevation: 0,
                },
            })
        });
        let policy = if let Some(token) = self.fall_tokens[index] {
            crate::players::ArrivalPolicy::Fall { token }
        } else {
            crate::players::ArrivalPolicy::Relocate
        };
        self.prepare_arrivals(
            ArrivalContext {
                world,
                coop,
                players,
                population,
                geometry,
            },
            ArrivalRequest {
                destinations,
                policy,
                initial: false,
                journey: Some(index),
            },
        )
    }
    pub fn process_land<R: tje_world::land::PathResources>(
        &mut self,
        world: &mut WorldState,
        coop: &CoopState,
        players: &crate::players::PlayerServices,
        resources: &R,
    ) -> Result<usize> {
        let mut levels = [None; 4];
        let mut count = 0;
        for seat in self.config.heroes.iter() {
            let hero = &coop.players[seat.index()];
            if hero.life != LifeState::Alive
                || self.travelling_mask().contains(seat)
                || hero.motion.position.elevation > 4 * 256
                || players.runtime[seat.index()].mode == crate::players::PlayerMode::Whirlwind
            {
                continue;
            }
            let Some(level) = hero.level else {
                continue;
            };
            if level.get() <= 1 {
                continue;
            }
            if !levels[..count].contains(&Some(level)) {
                levels[count] = Some(level);
                count += 1;
            }
            let floor = world
                .floor_mut(level)
                .ok_or(Error::State("land hero geometry"))?;
            let position = floor
                .generated
                .geometry
                .dimensions()
                .world_cell(hero.motion.position.x >> 8, hero.motion.position.y >> 8);
            let mut terrain = [0; 25];
            for (row, values) in hero.terrain.iter().enumerate() {
                terrain[row * 5..row * 5 + 5].copy_from_slice(values);
            }
            if let Some(position) = position
                && !floor.land.busy()
                && floor.land.marked(position)
                && tje_world::land::LandState::near_edge(&terrain)
            {
                floor.land.begin(position).map_err(world_error)?;
            }
        }
        let mut changed = 0;
        for level in levels.into_iter().flatten() {
            if world
                .floor_mut(level)
                .expect("active land floor")
                .step_land(resources)
                .map_err(world_error)?
                .is_some()
            {
                changed += 1;
            }
        }
        Ok(changed)
    }
}

impl LoadingState {
    pub fn validate_resources(&self, resources: &crate::resources::ResourceState) -> Result<()> {
        use crate::resources::{EffectKind, EffectOwner};
        for door in self.doors.iter().flatten() {
            let id = door
                .effect
                .ok_or(Error::State("unbound doorway outside present transaction"))?;
            let effect = resources
                .effects()
                .find(|(effect, _)| *effect == id)
                .map(|(_, effect)| effect)
                .ok_or(Error::State("doorway retains a retired effect generation"))?;
            if effect.kind != EffectKind::Doorway
                || effect.owner != EffectOwner::Seat(door.owner)
                || effect.level != door.machine.support.level
                || effect.position.x >> 8 != door.machine.support.x
                || effect.position.y >> 8 != door.machine.support.y
            {
                return Err(Error::State("doorway resource/position binding"));
            }
        }
        for phone in self.telephones.iter().flatten() {
            let id = phone.effect.ok_or(Error::State(
                "unbound telephone outside present transaction",
            ))?;
            let effect = resources
                .effects()
                .find(|(effect, _)| *effect == id)
                .map(|(_, effect)| effect)
                .ok_or(Error::State(
                    "telephone retains a retired effect generation",
                ))?;
            if effect.kind != EffectKind::Telephone
                || effect.owner != EffectOwner::Seat(phone.owner)
                || effect.level != phone.machine.position.level
                || effect.position.x >> 8 != phone.machine.position.x
                || effect.position.y >> 8 != phone.machine.position.y
            {
                return Err(Error::State("telephone resource/position binding"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "loading_tests.rs"]
mod tests;

impl LoadingState {
    pub fn arrive_hero_world(
        &mut self,
        owner: SeatId,
        destination: crate::players::Landing,
        world: &mut WorldState,
        population: &mut crate::population::PopulationState,
    ) -> crate::players::PlayerResult<()> {
        use crate::players::PlayerError;
        owner
            .validate()
            .map_err(|_| PlayerError::Invalid("arriving hero seat"))?;
        destination
            .level
            .validate()
            .map_err(|_| PlayerError::Invalid("arriving hero level"))?;
        destination
            .position
            .validate()
            .map_err(|_| PlayerError::Invalid("arriving hero position"))?;
        if !self.config.heroes.contains(owner)
            || population.retained()[owner.index()] != world.active()[owner.index()]
            || world.floor(destination.level).is_none()
            || !population
                .floors()
                .iter()
                .any(|floor| floor.level == destination.level)
        {
            return Err(PlayerError::Geometry(
                "arrival world/population/hero reference",
            ));
        }
        let mut retained = *population.retained();
        retained[owner.index()] = Some(destination.level);
        let prepared = population
            .prepare_retains(retained)
            .map_err(|_| PlayerError::Capacity("arriving population reservation"))?;
        population
            .release_seat_contacts(owner)
            .expect("validated arriving contact seat");
        world
            .set_active(owner.get(), destination.level)
            .expect("preflighted arrival level");
        population.commit_retains(prepared);
        self.highest_level = self.highest_level.max(destination.level);
        Ok(())
    }
}

impl LoadingState {
    pub fn validate_population(
        &self,
        population: &crate::population::PopulationState,
    ) -> Result<()> {
        for job in &self.jobs {
            match &job.stage {
                SourceStage::Population { job, .. } => population
                    .validate_preparation_job(job)
                    .map_err(world_error)?,
                SourceStage::Import {
                    population: prepared,
                    ..
                } => population
                    .preflight_prepared_floor(prepared)
                    .map_err(world_error)?,
                _ => (),
            }
        }
        Ok(())
    }
    pub fn validate_players(
        &self,
        players: &crate::players::PlayerServices,
        coop: &CoopState,
    ) -> Result<()> {
        players.validate(coop).map_err(player_error)?;
        for (index, token) in self.fall_tokens.iter().enumerate() {
            if let Some(token) = token {
                let journey = self.journeys[index]
                    .as_ref()
                    .ok_or(Error::State("fall source journey absent"))?;
                if !players
                    .fall_cohorts
                    .iter()
                    .flatten()
                    .any(|cohort| cohort.token == *token && cohort.seats.bits() == journey.cohort())
                {
                    return Err(Error::State(
                        "fall player/loading cohort references disagree",
                    ));
                }
            }
        }
        Ok(())
    }
}

pub struct DoorwayCrossing {
    pub effect: EffectId,
    pub owner: SeatId,
    pub hero: SeatId,
    pub level: LevelId,
    pub position: Position,
    pub tick: u64,
}

struct ArrivalContext<'a, G> {
    world: &'a WorldState,
    coop: &'a CoopState,
    players: &'a crate::players::PlayerServices,
    population: &'a crate::population::PopulationState,
    geometry: &'a G,
}
struct ArrivalRequest {
    destinations: [Option<crate::players::Landing>; 4],
    policy: crate::players::ArrivalPolicy,
    initial: bool,
    journey: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
enum DoorMotion {
    Closed,
    Opening { remaining: u8 },
    Open { remaining: u8 },
    Closing { remaining: u8 },
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
struct ElevatorState {
    level: LevelId,
    doors: [DoorMotion; 2],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElevatorProp {
    pub level: LevelId,
    pub x: i32,
    pub y: i32,
    pub arrival: bool,
    pub door: ElevatorDoor,
    /// `ElevatorSetState` descriptor index for both primary and secondary
    /// programmes; the secondary programme retains its decoded Y offset.
    /// This visual state is distinct from the coarser collision door state.
    pub animation_state: u8,
    pub moving: bool,
    pub riders: SeatMask,
    pub final_ship: bool,
}
impl LoadingState {
    fn ensure_elevator(&mut self, level: LevelId) -> usize {
        match self
            .elevators
            .binary_search_by_key(&level, |state| state.level)
        {
            Ok(index) => index,
            Err(index) => {
                self.elevators.insert(
                    index,
                    ElevatorState {
                        level,
                        doors: [DoorMotion::Closed; 2],
                    },
                );
                index
            }
        }
    }
    pub fn elevator_props(
        &self,
        world: &WorldState,
        level: LevelId,
    ) -> tje_world::Result<[Option<ElevatorProp>; 2]> {
        let barriers = self.elevator_barriers(world, level)?;
        let doors = self
            .elevators
            .binary_search_by_key(&level, |state| state.level)
            .ok()
            .map_or([DoorMotion::Closed; 2], |slot| self.elevators[slot].doors);
        let mut riders = 0;
        let mut moving = false;
        for journey in self.journeys.iter().flatten() {
            if journey
                .sources()
                .iter()
                .flatten()
                .any(|source| *source == level)
                && matches!(
                    journey.kind(),
                    tje_world::transitions::TravelKind::Elevator
                        | tje_world::transitions::TravelKind::Ship { .. }
                )
                && !journey.committed()
            {
                riders |= journey.cohort();
                moving |= journey.phase() != tje_world::transitions::TravelPhase::Joining;
            }
        }
        Ok(std::array::from_fn(|index| {
            let barrier = barriers[index];
            (barrier.door != ElevatorDoor::Inactive).then_some(ElevatorProp {
                level,
                x: barrier.x,
                y: barrier.y,
                arrival: index == 1,
                door: barrier.door,
                animation_state: if index == 0 && moving {
                    5
                } else {
                    match doors[index] {
                        DoorMotion::Closed => 1,
                        DoorMotion::Opening { .. } => 2,
                        DoorMotion::Open { .. } => 3,
                        DoorMotion::Closing { .. } => 4,
                    }
                },
                moving: index == 0 && moving,
                riders: SeatMask::new(if index == 0 { riders } else { 0 }).expect("four riders"),
                final_ship: level.get() == 25 && index == 0,
            })
        }))
    }
    fn elevator_levels(&self, coop: &CoopState) -> [Option<LevelId>; 4] {
        let mut visited = [None; 4];
        let mut levels = 0;
        for seat in self.config.heroes.iter() {
            let hero = &coop.players[seat.index()];
            if hero.life != LifeState::Alive {
                continue;
            }
            let Some(level) = hero.level else {
                continue;
            };
            if !visited[..levels].contains(&Some(level)) {
                visited[levels] = Some(level);
                levels += 1;
            }
        }
        visited
    }

    fn elevator_cohorts(&self, level: LevelId) -> (Option<usize>, bool) {
        let mut joining = None;
        let mut departing = false;
        for (index, journey) in self.journeys.iter().enumerate() {
            let Some(journey) = journey else {
                continue;
            };
            if journey.committed()
                || !matches!(
                    journey.kind(),
                    tje_world::transitions::TravelKind::Elevator
                        | tje_world::transitions::TravelKind::Ship { .. }
                )
                || !journey
                    .sources()
                    .iter()
                    .flatten()
                    .any(|source| *source == level)
            {
                continue;
            }
            if journey.phase() == tje_world::transitions::TravelPhase::Joining {
                joining.get_or_insert(index);
            } else {
                departing = true;
            }
        }
        (joining, departing)
    }

    /// Advance finite door motion only at the population/elevator operation.
    /// The readonly hero poses supply the original approach predicates without
    /// coupling this cadence to the more frequent post-movement entry sensor.
    /// Enhanced timings (12 opening/closing, 30 open) are not a claim of
    /// cartridge animation/cycle equivalence.
    pub fn advance_elevator_doors(
        &mut self,
        world: &WorldState,
        coop: &CoopState,
        tick: u64,
    ) -> Result<()> {
        if self
            .elevator_door_tick
            .is_some_and(|previous| tick < previous)
        {
            return Err(Error::State("elevator door clock moved backwards"));
        }
        if self.elevator_door_tick == Some(tick) {
            return Ok(());
        }
        self.elevator_door_tick = Some(tick);
        for level in self.elevator_levels(coop).into_iter().flatten() {
            let slot = self.ensure_elevator(level);
            let barriers = self.elevator_barriers(world, level).map_err(world_error)?;
            let (joining, departing) = self.elevator_cohorts(level);
            for (kind, barrier) in barriers.into_iter().enumerate() {
                if barrier.door == ElevatorDoor::Inactive {
                    continue;
                }
                let near = self.config.heroes.iter().any(|seat| {
                    let hero = &coop.players[seat.index()];
                    if hero.life != LifeState::Alive || hero.level != Some(level) {
                        return false;
                    }
                    let x = hero.motion.position.x >> 8;
                    let dy = (hero.motion.position.y >> 8) - barrier.y;
                    (i64::from(x) - i64::from(barrier.x)).abs() < 10
                        && dy < 20
                        && dy
                            >= if barrier.door == ElevatorDoor::Open {
                                1
                            } else {
                                10
                            }
                });
                self.elevators[slot].doors[kind] = if kind == 0 && departing {
                    DoorMotion::Closed
                } else if kind == 0 && joining.is_some() {
                    DoorMotion::Open { remaining: 30 }
                } else {
                    match self.elevators[slot].doors[kind] {
                        DoorMotion::Closed if near => DoorMotion::Opening { remaining: 12 },
                        DoorMotion::Opening { remaining } if remaining > 1 => DoorMotion::Opening {
                            remaining: remaining - 1,
                        },
                        DoorMotion::Opening { .. } => DoorMotion::Open { remaining: 30 },
                        DoorMotion::Open { .. } if near => DoorMotion::Open { remaining: 30 },
                        DoorMotion::Open { remaining } if remaining > 1 => DoorMotion::Open {
                            remaining: remaining - 1,
                        },
                        DoorMotion::Open { .. } => DoorMotion::Closing { remaining: 12 },
                        DoorMotion::Closing { remaining } if remaining > 1 => DoorMotion::Closing {
                            remaining: remaining - 1,
                        },
                        DoorMotion::Closing { .. } => DoorMotion::Closed,
                        other => other,
                    }
                };
            }
        }
        Ok(())
    }

    /// Poll after XY movement on both movement phases, not only when the
    /// population/elevator timers run: entry's original dy1..7 interval can be
    /// crossed between those operations. This owns boarding, joining-phase Down
    /// exits and previous poses, but never consumes a door or journey timer.
    pub fn poll_elevator_sensors(
        &mut self,
        world: &WorldState,
        coop: &mut CoopState,
        input: &[crate::players::PlayerInput; 4],
        tick: u64,
        ship_parts: u16,
    ) -> Result<SeatMask> {
        if self
            .elevator_sensor_tick
            .is_some_and(|previous| tick < previous)
        {
            return Err(Error::State("elevator sensor clock moved backwards"));
        }
        if self.elevator_sensor_tick == Some(tick) {
            return Ok(SeatMask::NONE);
        }
        self.elevator_sensor_tick = Some(tick);
        for index in 0..4 {
            let Some(journey) = self.journeys[index].as_ref() else {
                continue;
            };
            if journey.phase() != tje_world::transitions::TravelPhase::Joining
                || journey.kind() != tje_world::transitions::TravelKind::Elevator
            {
                continue;
            }
            let cohort = journey.cohort();
            let sources = *journey.sources();
            for seat in self.config.heroes.iter() {
                if cohort & (1 << seat.get()) == 0 || input[seat.index()].directions & 2 == 0 {
                    continue;
                }
                let level = sources[seat.index()].expect("rider source");
                let barrier = self.elevator_barriers(world, level).map_err(world_error)?[0];
                if barrier.door != ElevatorDoor::Open {
                    continue;
                }
                let hero = &mut coop.players[seat.index()];
                hero.motion.position = Position {
                    x: barrier.x * 256,
                    y: (barrier.y + 12) * 256,
                    elevation: 0,
                };
                hero.motion.velocity = [0; 3];
                hero.motion.acceleration = [0; 3];
                if self.journeys[index]
                    .as_mut()
                    .expect("joining rider cohort")
                    .remove_seat(seat.get())
                    .map_err(world_error)?
                {
                    self.journeys[index] = None;
                    self.fall_tokens[index] = None;
                    break;
                }
            }
        }
        let mut boarded = 0;
        let travelling = self.travelling_mask();
        for level in self.elevator_levels(coop).into_iter().flatten() {
            let barrier = self.elevator_barriers(world, level).map_err(world_error)?[0];
            let (_, departing) = self.elevator_cohorts(level);
            if barrier.door != ElevatorDoor::Open
                || departing
                || level.get() == 25 && ship_parts & 0x3ff != 0x3ff
            {
                continue;
            }
            for seat in self.config.heroes.iter() {
                let hero = &coop.players[seat.index()];
                if hero.life != LifeState::Alive
                    || hero.level != Some(level)
                    || travelling.contains(seat)
                {
                    continue;
                }
                let (x, y, z) = (
                    hero.motion.position.x >> 8,
                    hero.motion.position.y >> 8,
                    hero.motion.position.elevation >> 8,
                );
                let dx = (i64::from(x) - i64::from(barrier.x)).abs();
                let dy = y - barrier.y;
                let entered = dx < 10
                    && (1..8).contains(&dy)
                    && z == 0
                    && input[seat.index()].directions & 1 != 0
                    && self.previous_heroes[seat.index()].is_some_and(|previous| {
                        previous.level == level && previous.y > barrier.y && previous.y >= y
                    });
                if entered {
                    // Re-read the shared cohort after each new rider, so four
                    // simultaneous entrants join one journey, not four copies.
                    let (active, _) = self.elevator_cohorts(level);
                    if let Some(index) = active {
                        self.journeys[index]
                            .as_mut()
                            .expect("joining elevator")
                            .join(world, seat.get())
                            .map_err(world_error)?;
                    } else {
                        let cohort = SeatMask::new(1 << seat.get()).expect("single rider");
                        self.begin_elevator(world, cohort, ship_parts)?;
                    }
                    let hero = &mut coop.players[seat.index()];
                    hero.motion.position.x = barrier.x * 256;
                    hero.motion.position.y = (barrier.y + 5) * 256;
                    hero.motion.velocity = [0; 3];
                    hero.motion.acceleration = [0; 3];
                    boarded |= 1 << seat.get();
                }
            }
        }
        self.previous_heroes = coop.players.each_ref().map(|hero| {
            if hero.life == LifeState::Alive {
                hero.level.map(|level| tje_world::transitions::Position {
                    level,
                    x: hero.motion.position.x >> 8,
                    y: hero.motion.position.y >> 8,
                })
            } else {
                None
            }
        });
        Ok(SeatMask::new(boarded).expect("four riders"))
    }
}
