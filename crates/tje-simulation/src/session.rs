//! One registered owner validates identities and references across its services.
use crate::capacity::{CapacityPlan, FLOOR_RESERVATION, GENERATION_RESERVATION, Reservation};
use crate::ownership::{self, TypedCallback};
use crate::visual::AnimationContracts;
use crate::{ActorId, Error, LevelId, PhaseClock, PublicationStamp, Result, SeatId, SeatMask};
use crate::{
    loading::{LoadingConfig, LoadingState},
    players::{PlayerError, PlayerServices},
    population::{PopulationLimits, PopulationState},
    publication::PublishedScene,
    resources::ResourceLimits,
    visual::VisualState,
};
use bincode::{Decode, Encode};
use sha2::{Digest, Sha256};
use tje_four_player::{
    CoopState,
    state::{Config as CoopConfig, LifeState},
};
use tje_world::{
    GeneratorId, LevelSize, WorldConfig,
    loading::{FloorSpec, ImportKind, ImportedFloorSpec},
    original::OriginalRevision,
    random::StageRng,
    world::WorldState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Roles {
    pub inputs: SeatMask,
    pub heroes: SeatMask,
    pub local_owners: SeatMask,
    pub director: Option<SeatId>,
}
impl Roles {
    pub const fn coop(seats: SeatMask) -> Self {
        Self {
            inputs: seats,
            heroes: seats,
            local_owners: seats,
            director: None,
        }
    }
    pub fn validate(self) -> Result<()> {
        for mask in [self.inputs, self.heroes, self.local_owners] {
            mask.validate()
                .map_err(|_| Error::State("invalid role seat mask"))?;
        }
        if self.heroes == SeatMask::NONE
            || self.heroes.bits() & !self.inputs.bits() != 0
            || self.local_owners.bits() & !self.inputs.bits() != 0
        {
            return Err(Error::State("input, hero and ownership masks disagree"));
        }
        let director = if let Some(seat) = self.director {
            seat.validate()
                .map_err(|_| Error::State("invalid director seat"))?;
            if self.heroes.contains(seat) || !self.inputs.contains(seat) {
                return Err(Error::State("director cannot also own a hero body"));
            }
            1 << seat.get()
        } else {
            0
        };
        if self.inputs.bits() != self.heroes.bits() | director {
            return Err(Error::State("input seat has no declared role"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct SourceBinding {
    pub level: LevelId,
    pub source_digest: [u8; 32],
    pub definitions_digest: [u8; 32],
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct CourseManifest {
    pub identity: [u8; 32],
    pub sources: Vec<SourceBinding>,
}
impl CourseManifest {
    fn validate(&self) -> Result<()> {
        if self.sources.is_empty()
            || self.sources.len() > tje_world::MAX_FLOORS
            || self
                .sources
                .windows(2)
                .any(|pair| pair[0].level >= pair[1].level)
        {
            return Err(Error::State(
                "course source manifest is not bounded/sorted/unique",
            ));
        }
        for source in &self.sources {
            source
                .level
                .validate()
                .map_err(|_| Error::State("course source LevelId"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct SessionConfig {
    pub version: u16,
    pub body_manifest: [u8; 32],
    pub world: WorldConfig,
    pub revision: OriginalRevision,
    pub generator: GeneratorId,
    pub coop: CoopConfig,
    pub roles: Roles,
    pub capacities: CapacityPlan,
    pub course: Option<CourseManifest>,
    pub resources_identity: [u8; 32],
    pub catalogue_identity: [u8; 32],
    pub frame_count: u32,
    pub animations: AnimationContracts,
}
impl SessionConfig {
    /// Stock gameplay requires OriginalV1; a 19x26 ExpandedV1 fixture is not stock.
    pub fn stock(
        world: WorldConfig,
        coop: CoopConfig,
        revision: OriginalRevision,
        callbacks: &[TypedCallback],
    ) -> Result<Self> {
        Self::new(
            world,
            coop,
            revision,
            GeneratorId::OriginalV1,
            None,
            callbacks,
        )
    }
    pub fn new(
        world: WorldConfig,
        coop: CoopConfig,
        revision: OriginalRevision,
        generator: GeneratorId,
        course: Option<CourseManifest>,
        callbacks: &[TypedCallback],
    ) -> Result<Self> {
        let heroes = coop.participants.iter().count() as u16;
        let config = Self {
            version: 3,
            body_manifest: ownership::IDENTITY,
            resources_identity: ownership::resource_identity(world.rom_sha1, callbacks)?,
            catalogue_identity: [0; 32],
            frame_count: 0,
            animations: AnimationContracts::default(),
            roles: Roles::coop(coop.participants),
            capacities: CapacityPlan {
                active_floors: heroes,
                ..CapacityPlan::STOCK_COOP
            },
            world,
            revision,
            generator,
            coop,
            course,
        };
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<Reservation> {
        let reservation = self.validate_settings()?;
        self.animations.validate()?;
        if self.frame_count > 65536
            || (self.animations.programs.is_empty() != (self.frame_count == 0))
        {
            return Err(Error::State("imported catalogue frame/animation bounds"));
        }
        Ok(reservation)
    }
    /// Normalized user/configuration settings, without imported ROM programs.
    pub(crate) fn validate_settings(&self) -> Result<Reservation> {
        self.world
            .validate()
            .map_err(|_| Error::State("invalid world configuration"))?;
        self.coop
            .validate()
            .map_err(|_| Error::State("invalid hero configuration"))?;
        self.roles.validate()?;
        if self.version != 3
            || self.frame_count > 65536
            || self.body_manifest != ownership::IDENTITY
            || self.world.root_seed != self.coop.world_seed
            || self.coop.participants != self.roles.heroes
            || (self.generator == GeneratorId::OriginalV1 && self.world.size != LevelSize::Original)
            || (self.generator == GeneratorId::DesignedV1) != self.course.is_some()
        {
            return Err(Error::State(
                "session version/manifest/generator/seed/role identity mismatch",
            ));
        }
        if let Some(course) = &self.course {
            course.validate()?;
        }
        self.capacities
            .reserve(self.roles.heroes.iter().count() as u16)
    }
    pub fn settings_identity(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"tje-simulation/settings/3\0");
        hash.update(self.version.to_be_bytes());
        hash.update(self.body_manifest);
        hash.update(self.world.identity());
        hash.update([match self.revision {
            OriginalRevision::Rev00 => 0,
            OriginalRevision::Rev02 => 2,
        }]);
        hash.update([self.generator.tag()]);
        hash.update(self.generator.version().to_be_bytes());
        hash.update(self.coop.identity());
        hash.update([
            self.roles.inputs.bits(),
            self.roles.heroes.bits(),
            self.roles.local_owners.bits(),
        ]);
        hash.update([self.roles.director.map_or(255, SeatId::get)]);
        let caps = self.capacities;
        for value in [
            caps.floors,
            caps.active_floors,
            caps.actors,
            caps.earthlings_per_floor,
            caps.items_per_floor,
            caps.progression,
            caps.companions,
            caps.effects,
            caps.drops,
        ] {
            hash.update(value.to_be_bytes());
        }
        hash.update(caps.presentation_bytes.to_be_bytes());
        hash.update([u8::from(self.course.is_some())]);
        if let Some(course) = &self.course {
            hash.update(course.identity);
            hash.update((course.sources.len() as u16).to_be_bytes());
            for source in &course.sources {
                hash.update(source.level.get().to_be_bytes());
                hash.update(source.source_digest);
                hash.update(source.definitions_digest);
            }
        }
        hash.update(self.resources_identity);
        hash.update(self.catalogue_identity);
        hash.update(self.frame_count.to_be_bytes());
        hash.finalize().into()
    }
    pub fn identity(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"tje-simulation/config/3\0");
        hash.update(self.settings_identity());
        hash.update(self.animations.identity());
        hash.finalize().into()
    }
    fn validate_import(&self, spec: &ImportedFloorSpec) -> Result<()> {
        match &spec.kind {
            ImportKind::Designed { course_identity } => {
                let course = self.course.as_ref().ok_or(Error::State(
                    "DesignedV1 floor has no immutable course manifest",
                ))?;
                let binding = course
                    .sources
                    .binary_search_by_key(&spec.level, |source| source.level)
                    .ok()
                    .map(|index| &course.sources[index])
                    .ok_or(Error::State(
                        "DesignedV1 floor is absent from the immutable source manifest",
                    ))?;
                if self.generator != GeneratorId::DesignedV1
                    || course_identity != &course.identity
                    || binding.source_digest != spec.source_digest
                    || binding.definitions_digest != spec.definitions_digest
                {
                    return Err(Error::State(
                        "DesignedV1 floor/course/definition provenance mismatch",
                    ));
                }
            }
            ImportKind::Stock if self.generator != GeneratorId::OriginalV1 => {
                return Err(Error::State(
                    "stock floor cannot substitute for the selected generator",
                ));
            }
            ImportKind::Special(_) if self.generator == GeneratorId::DesignedV1 => {
                return Err(Error::State(
                    "campaign special floor cannot substitute for DesignedV1",
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum SetupRoot {
    Player,
    MainGame,
    Inventory,
    Earthlings,
    Elevator,
    Seeds,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulationState {
    pub world: WorldState,
    pub players: CoopState,
    pub services: PlayerServices,
    pub population: PopulationState,
    pub loading: LoadingState,
}
impl SimulationState {
    pub fn new(config: &SessionConfig) -> Result<Self> {
        config.validate()?;
        let limits = ResourceLimits {
            effects: config.capacities.effects,
            effects_per_seat: config.capacities.effects.min(16),
            drops: config.capacities.drops,
            drops_per_seat: config.capacities.drops.min(32),
            placed_items: config.capacities.floors * config.capacities.items_per_floor,
            ..ResourceLimits::default()
        };
        Ok(Self {
            world: WorldState::new(config.world.clone())
                .map_err(|_| Error::State("initial persistent world"))?,
            players: CoopState::new(config.coop.clone())
                .map_err(|_| Error::State("initial hero roster"))?,
            services: PlayerServices::new(config.world.root_seed, limits).map_err(player_error)?,
            population: PopulationState::new(
                PopulationLimits {
                    per_floor: config.capacities.earthlings_per_floor,
                    active_bodies: config.capacities.active_floors
                        * config.capacities.earthlings_per_floor
                        + config.capacities.companions,
                },
                config.identity(),
            )
            .map_err(|_| Error::Capacity("initial population arena"))?,
            loading: LoadingState::with_config(LoadingConfig {
                world: config.world.clone(),
                heroes: config.roles.heroes,
                revision: config.revision,
                generator: config.generator,
            })?,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Scene {
    Boot,
    Ready,
    World,
    ShipEnding,
    GameOver,
    Finished,
}

/// Construct off the live machine, then consume through finish_setup. No partial
/// transaction can enter the registered sidecar or a running world phase.
#[derive(Debug)]
pub struct SetupTransaction {
    session: Session,
}
impl SetupTransaction {
    pub fn prepare(
        config: SessionConfig,
        state: SimulationState,
        callbacks: Vec<TypedCallback>,
    ) -> Result<Self> {
        ownership::validate_manifest()?;
        let reservation = config.validate()?;
        let configuration = config.identity();
        let SimulationState {
            world,
            players,
            services,
            population,
            loading,
        } = state;
        let level = LevelId::new(0).expect("bounded derivation level");
        let rngs = [b"population".as_slice(), b"loot", b"ai", b"director"]
            .map(|purpose| StageRng::derive(&configuration, level, 0, purpose));
        let hero_actors = std::array::from_fn(|index| {
            config
                .roles
                .heroes
                .contains(SeatId::ALL[index])
                .then(|| ActorId::new(index as u16, 1).expect("four hero lifetime identities"))
        });
        let session = Session {
            config,
            configuration,
            reservation,
            world,
            players,
            services,
            population,
            loading,
            visuals: VisualState::default(),
            published: PublishedScene::default(),
            scene: Scene::Boot,
            hero_actors,
            callbacks,
            clock: PhaseClock::default(),
            rngs,
            setup_seen: 0,
            publication: None,
            failed: false,
        };
        session.validate_references()?;
        session.validate_encoded_reservation()?;
        Ok(Self { session })
    }
    pub fn into_loading(self) -> Result<Session> {
        if self
            .session
            .config
            .roles
            .heroes
            .iter()
            .any(|seat| self.session.players.players[seat.index()].life != LifeState::Unplaced)
        {
            return Err(Error::State(
                "new-session source preparation already placed a hero",
            ));
        }
        self.session.validate()?;
        Ok(self.session)
    }
    pub fn world_mut(&mut self) -> &mut WorldState {
        &mut self.session.world
    }
    pub fn players_mut(&mut self) -> &mut CoopState {
        &mut self.session.players
    }
    pub fn finish_setup(mut self) -> Result<Session> {
        self.session.finish_setup()?;
        Ok(self.session)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Session {
    config: SessionConfig,
    configuration: [u8; 32],
    reservation: Reservation,
    pub world: WorldState,
    pub players: CoopState,
    pub services: PlayerServices,
    pub population: PopulationState,
    pub loading: LoadingState,
    pub visuals: VisualState,
    pub published: PublishedScene,
    pub scene: Scene,
    hero_actors: [Option<ActorId>; 4],
    callbacks: Vec<TypedCallback>,
    pub clock: PhaseClock,
    pub rngs: [StageRng; 4],
    setup_seen: u8,
    publication: Option<PublicationStamp>,
    failed: bool,
}
impl Session {
    pub fn config(&self) -> &SessionConfig {
        &self.config
    }
    pub fn finish_setup(&mut self) -> Result<()> {
        self.validate_references()?;
        if !self.loading.initial_setup
            || self
                .config
                .roles
                .heroes
                .iter()
                .any(|seat| self.players.players[seat.index()].life != LifeState::Alive)
        {
            return Err(Error::State(
                "new-session setup is not one complete hero transaction",
            ));
        }
        self.clock.complete_setup()?;
        self.scene = Scene::Ready;
        self.validate()?;
        self.validate_encoded_reservation()
    }
    pub fn publication_access(
        &mut self,
    ) -> (crate::publication::PublicationRefs<'_>, &mut PublishedScene) {
        (
            crate::publication::PublicationRefs {
                config: &self.config,
                world: &self.world,
                players: &self.players,
                services: &self.services,
                population: &self.population,
                loading: &self.loading,
                visuals: &self.visuals,
                clock: &self.clock,
                scene: self.scene,
                hero_actors: &self.hero_actors,
            },
            &mut self.published,
        )
    }
    pub fn configuration_identity(&self) -> &[u8; 32] {
        &self.configuration
    }
    pub fn reservation(&self) -> Reservation {
        self.reservation
    }
    pub fn hero_actor(&self, seat: SeatId) -> Option<ActorId> {
        SeatId::new(seat.get()).and_then(|seat| self.hero_actors[seat.index()])
    }
    pub fn callbacks(&self) -> &[TypedCallback] {
        &self.callbacks
    }
    pub const fn publication(&self) -> Option<PublicationStamp> {
        self.publication
    }
    pub const fn failed(&self) -> bool {
        self.failed
    }
    pub fn fail(&mut self) {
        self.failed = true;
    }
    pub fn validate_inputs(&self, input: &tje_four_player::input::InputFrame) -> Result<()> {
        input
            .validate()
            .map_err(|_| Error::State("invalid phase input words"))?;
        for seat in SeatId::ALL {
            if !self.config.roles.inputs.contains(seat) && input.pads[seat.index()].connected() {
                return Err(Error::State("phase input supplied for a disabled role"));
            }
        }
        Ok(())
    }
    /// Input processing runs only in the Players phase. Its first hardware
    /// frame may follow boot/intro, and subsequent latches are four phases apart;
    /// do not reuse the primitive codec's consecutive-from-zero bookkeeping API.
    pub fn latch_phase_inputs(&mut self) -> Result<bool> {
        if self.config.roles.inputs != self.config.roles.heroes {
            return Err(Error::Ownership(
                "non-hero role input requires its separate role-aware phase adapter",
            ));
        }
        let input = *self
            .clock
            .input()
            .ok_or(Error::State("no phase input latch"))?;
        self.validate_inputs(&input)?;
        if self.clock.last_admission(crate::Phase::Players) != Some(input.frame) {
            return Err(Error::State(
                "hero input processing entered outside the Players phase",
            ));
        }
        if let Some(previous) = self.players.inputs {
            if previous == input {
                return Ok(false);
            }
            if previous.frame >= input.frame {
                return Err(Error::State(
                    "hero input processing moved backwards or changed an admitted latch",
                ));
            }
        }
        self.players.inputs = Some(input);
        Ok(true)
    }
    pub fn publish(&mut self, stamp: PublicationStamp) -> Result<()> {
        if !stamp.phase.publishes()
            || stamp.epoch != self.clock.epoch()
            || self.clock.last_admission(stamp.phase) != Some(stamp.hardware_frame)
            || self
                .publication
                .is_some_and(|previous| previous.epoch >= stamp.epoch)
        {
            return Err(Error::State(
                "publication does not name the current committed phase",
            ));
        }
        self.publication = Some(stamp);
        Ok(())
    }
    /// Original setup reentries acknowledge the transaction; never clear a live
    /// hero, population, map, seed stream or inventory after setup has committed.
    pub fn acknowledge_setup(&mut self, root: SetupRoot) -> Result<bool> {
        if !self.clock.setup_complete() || self.failed {
            return Err(Error::State("setup bridge has no committed session"));
        }
        let bit = 1 << root as u8;
        let first = self.setup_seen & bit == 0;
        self.setup_seen |= bit;
        Ok(first)
    }
    fn validate_animation_references(&self) -> Result<()> {
        if self.config.frame_count == 0 {
            return Ok(());
        }
        let contracts = &self.config.animations;
        for seat in self.config.roles.heroes.iter() {
            let player = &self.players.players[seat.index()];
            let runtime = &self.services.runtime[seat.index()].animation;
            if runtime.descriptor != player.animation.descriptor {
                return Err(Error::State("hero playback names another pose"));
            }
            let archetype = match player.character.archetype {
                tje_four_player::Archetype::ToeJam => 0,
                tje_four_player::Archetype::Earl => 1,
            };
            let animation = *contracts.heroes[archetype]
                .get(usize::from(player.animation.descriptor))
                .ok_or(Error::State("hero pose has no admitted animation binding"))?;
            let period = contracts
                .program(animation)?
                .period
                .ok_or(Error::State("hero animation lacks descriptor timing"))?;
            contracts.validate_cursor(&crate::visual::Playback {
                animation,
                period,
                sequence_index: player.animation.sequence_index,
                ticks_left: u16::from(player.animation.ticks_left),
                held: runtime.held,
                flip_horizontal: player.animation.flip_horizontal,
                started: runtime.started,
            })?;
        }
        for floor in self.population.floors() {
            for actor in &floor.actors {
                let key = (actor.kind.index() as u8, actor.state);
                let index = contracts
                    .earthlings
                    .binary_search_by_key(&key, |entry| (entry.kind, entry.state))
                    .map_err(|_| Error::State("actor pose has no admitted animation binding"))?;
                let animation = contracts.earthlings[index].animation;
                let contract = contracts.program(animation)?;
                let period = contract
                    .period
                    .ok_or(Error::State("actor animation lacks descriptor timing"))?;
                let cursor = actor.animation;
                contracts.validate_cursor(&crate::visual::Playback {
                    animation,
                    period,
                    sequence_index: cursor.sequence_index,
                    ticks_left: cursor.ticks_left,
                    held: cursor.held,
                    flip_horizontal: cursor.flip_horizontal,
                    started: cursor.started,
                })?;
                if cursor.started
                    && u16::from(contract.frame_indices[usize::from(cursor.sequence_index)])
                        != actor.animation_frame
                {
                    return Err(Error::State(
                        "actor frame disagrees with its admitted saved selection",
                    ));
                }
            }
        }
        Ok(())
    }
    fn validate_resource_actor_references(&self) -> Result<()> {
        let known_source = |source: crate::population::PopulationId, actor: ActorId| {
            self.population.actor(source).is_some()
                && self.population.known_body(actor)
                && self
                    .population
                    .resolve(actor)
                    .is_none_or(|live| live.id == source)
        };
        for runtime in &self.services.runtime {
            if runtime
                .service
                .is_some_and(|service| !self.population.known_body(service.actor))
            {
                return Err(Error::State("hero service names an unissued actor"));
            }
            if runtime
                .contacts
                .offer
                .is_some_and(|offer| !known_source(offer.source, offer.actor))
            {
                return Err(Error::State(
                    "hero service offer has no persistent actor source",
                ));
            }
            if runtime
                .contacts
                .mole
                .is_some_and(|mole| !known_source(mole.source, mole.actor))
            {
                return Err(Error::State("mole hold has no persistent actor source"));
            }
        }
        for service in self.services.resources.services() {
            if !self.population.known_body(service.actor)
                || service
                    .source
                    .is_some_and(|source| !known_source(source, service.actor))
            {
                return Err(Error::State(
                    "trade service has no issued persistent actor source",
                ));
            }
        }
        for offer in self.services.resources.offers() {
            if !self.population.known_body(offer.service) {
                return Err(Error::State("mail offer names an unissued service actor"));
            }
        }
        Ok(())
    }
    fn validate_references(&self) -> Result<()> {
        let reservation = self.config.validate()?;
        if self.configuration != self.config.identity()
            || self.reservation != reservation
            || self.world.config() != &self.config.world
            || self.players.config() != &self.config.coop
            || ownership::resource_identity(self.config.world.rom_sha1, &self.callbacks)?
                != self.config.resources_identity
        {
            return Err(Error::State(
                "aggregate configuration/reservation/resource identity mismatch",
            ));
        }
        self.world
            .validate()
            .map_err(|_| Error::State("invalid aggregate persistent world"))?;
        self.players
            .validate()
            .map_err(|_| Error::State("invalid aggregate hero state"))?;
        self.services
            .validate(&self.players)
            .map_err(player_error)?;
        self.population
            .validate_world(&self.world)
            .map_err(|_| Error::State("population/world references"))?;
        crate::world_bridge::validate_population_deltas(&self.world, &self.population)
            .map_err(player_error)?;
        if self.population.identity() != &self.configuration {
            return Err(Error::State(
                "population differs from immutable session identity",
            ));
        }
        self.loading.validate_config(
            &self.config.world,
            self.config.roles.heroes,
            self.config.revision,
            self.config.generator,
        )?;
        self.loading.validate_world(&self.world, &self.players)?;
        self.loading.validate_population(&self.population)?;
        self.loading
            .validate_players(&self.services, &self.players)?;
        self.loading.validate_resources(&self.services.resources)?;
        self.visuals.validate(&self.config.animations)?;
        self.validate_animation_references()?;
        self.validate_resource_actor_references()?;
        self.published.validate(self.config.frame_count)?;
        self.published.validate_geometry(&self.world)?;
        for actor in &self.published.actors {
            let known = match actor.id {
                crate::DrawId::Actor(id) => match id.slot() {
                    0..4 => self.hero_actors[usize::from(id.slot())] == Some(id),
                    ActorId::POPULATION_START..ActorId::EFFECT_START => {
                        self.population.known_body(id)
                    }
                    ActorId::EFFECT_START..ActorId::DROP_START => {
                        crate::EffectId::new(id.slot() - ActorId::EFFECT_START, id.generation())
                            .is_some_and(|id| self.services.resources.known_effect(id))
                    }
                    ActorId::DROP_START..544 => crate::resources::DropId::new(
                        id.slot() - ActorId::DROP_START,
                        id.generation(),
                    )
                    .is_some_and(|id| self.services.resources.known_drop(id)),
                    _ => false,
                },
                crate::DrawId::Item { level, id } => self
                    .services
                    .resources
                    .items()
                    .iter()
                    .any(|item| item.level == level && item.id == id),
                crate::DrawId::Prop { level, id } => id < 2 && self.world.floor(level).is_some(),
            };
            if !known {
                return Err(Error::State(
                    "published actor has no issued source identity",
                ));
            }
        }
        if self.config.frame_count > 0 && self.published.stamp != self.publication {
            return Err(Error::State(
                "logical publication rows disagree with the committed stamp",
            ));
        }
        if self.scene == Scene::World
            && self.published.kind != crate::publication::PublishedKind::World
        {
            return Err(Error::State(
                "world scene has no explicit frozen world publication",
            ));
        }
        let limits = self.services.resources.limits();
        let population = self.population.limits();
        if limits.effects != self.config.capacities.effects
            || limits.drops != self.config.capacities.drops
            || limits.placed_items
                != self.config.capacities.floors * self.config.capacities.items_per_floor
            || population.per_floor != self.config.capacities.earthlings_per_floor
            || population.active_bodies
                != self.config.capacities.active_floors
                    * self.config.capacities.earthlings_per_floor
                    + self.config.capacities.companions
        {
            return Err(Error::State(
                "service pools differ from admitted simultaneous reservations",
            ));
        }
        for (_, effect) in self.services.resources.effects() {
            if self.world.floor(effect.level).is_none() {
                return Err(Error::State("effect references absent floor"));
            }
            if let crate::resources::EffectOwner::Actor(actor) = effect.owner
                && !self.population.known_body(actor)
            {
                return Err(Error::State("effect has unissued source actor generation"));
            }
            if let crate::resources::EffectSource::Actor(definition) = effect.source
                && definition.get() >= 25
            {
                return Err(Error::State("effect source definition is not imported"));
            }
        }
        for (_, drop) in self.services.resources.drops() {
            if self.world.floor(drop.level).is_none() {
                return Err(Error::State("drop references absent floor"));
            }
            if let crate::resources::EffectOwner::Actor(actor) = drop.owner
                && !self.population.known_body(actor)
            {
                return Err(Error::State("drop has unissued source actor provenance"));
            }
        }
        for item in self.services.resources.items() {
            if self.world.floor(item.level).is_none() {
                return Err(Error::State("item references absent floor"));
            }
        }
        for entry in &self.visuals.companions {
            if !self
                .population
                .active_bodies()
                .any(|(actor, _, companion)| actor == entry.actor && companion)
            {
                return Err(Error::State("companion playback has stale generation"));
            }
        }
        for entry in &self.visuals.effects {
            if self.services.resources.effect(entry.effect).is_none() {
                return Err(Error::State("effect playback has stale generation"));
            }
        }
        self.clock.validate()?;
        if self.setup_seen & !63 != 0 || self.rngs.iter().any(|rng| !rng.valid()) {
            return Err(Error::State(
                "invalid setup ledger or independent RNG stream",
            ));
        }
        if self.world.floors().len() > usize::from(self.config.capacities.floors) {
            return Err(Error::Capacity("persistent floor reservation"));
        }
        let mut active = [None; 4];
        let mut active_count = 0;
        for seat in SeatId::ALL {
            let player = &self.players.players[seat.index()];
            let expected = self
                .config
                .roles
                .heroes
                .contains(seat)
                .then(|| ActorId::new(u16::from(seat.get()), 1).expect("hero identity"));
            if self.hero_actors[seat.index()] != expected
                || player.level != self.world.active()[seat.index()]
            {
                return Err(Error::State(
                    "hero identity/current level differs across aggregate services",
                ));
            }
            if let Some(level) = player.level {
                level
                    .validate()
                    .map_err(|_| Error::State("invalid hero LevelId"))?;
                if self.world.floor(level).is_none() {
                    return Err(Error::State("hero references an absent persistent floor"));
                }
                if !active.contains(&Some(level)) {
                    active[active_count] = Some(level);
                    active_count += 1;
                }
            }
        }
        if active_count > usize::from(self.config.capacities.active_floors) {
            return Err(Error::Capacity("hero-active floor reservation"));
        }
        for floor in self.world.floors() {
            match &floor.spec {
                FloorSpec::Imported(spec) => self.config.validate_import(spec)?,
                spec if spec.generator() != self.config.generator => {
                    return Err(Error::State(
                        "floor cannot satisfy the selected generator capability",
                    ));
                }
                _ => {}
            }
            if floor.generated.population.earthlings.len()
                > usize::from(self.config.capacities.earthlings_per_floor)
                || floor.generated.population.items.len()
                    > usize::from(self.config.capacities.items_per_floor)
            {
                return Err(Error::Capacity(
                    "persistent population exceeds its admitted joint caps",
                ));
            }
        }
        if self.world.pending().is_some() && self.config.generator != GeneratorId::ExpandedV1 {
            return Err(Error::State(
                "pending ExpandedV1 generation cannot satisfy the selected generator",
            ));
        }
        if let Some(importing) = self.world.importing() {
            self.config
                .validate_import(&importing.spec().provenance())?;
        }
        if self.world.dropped().len() > usize::from(self.config.capacities.drops) {
            return Err(Error::Capacity("persistent global drop reservation"));
        }
        if let Some(phase_input) = self.clock.input() {
            self.validate_inputs(phase_input)?;
        }
        if let Some(player_input) = self.players.inputs {
            self.validate_inputs(&player_input)?;
            let phase_input = self.clock.input().ok_or(Error::State(
                "hero input exists before any admitted hardware phase",
            ))?;
            if player_input.frame > phase_input.frame
                || (player_input.frame == phase_input.frame && &player_input != phase_input)
            {
                return Err(Error::State("hero and phase input latches disagree"));
            }
        }
        if let Some(stamp) = self.publication
            && (!stamp.phase.publishes()
                || stamp.epoch == 0
                || stamp.epoch > self.clock.epoch()
                || self.clock.last_admission(stamp.phase) != Some(stamp.hardware_frame)
                || self
                    .clock
                    .input()
                    .is_none_or(|input| stamp.hardware_frame > input.frame))
        {
            return Err(Error::State("invalid saved publication boundary"));
        }
        Ok(())
    }
    /// Exact wire-size checks are admission/setup work. Capture/restore already
    /// serialize through the registry's global bound; their invariant pass uses
    /// the admitted per-field cardinalities, not additional full-world encodes.
    fn validate_encoded_reservation(&self) -> Result<()> {
        for floor in self.world.floors() {
            if encoded_size(floor)? > FLOOR_RESERVATION {
                return Err(Error::Capacity(
                    "floor exceeds its worst-case codec reservation",
                ));
            }
        }
        if let Some(pending) = self.world.pending()
            && encoded_size(pending)? > GENERATION_RESERVATION
        {
            return Err(Error::Capacity(
                "pending generation exceeds its codec reservation",
            ));
        }
        if encoded_size(self)? > self.reservation.authoritative_bytes {
            return Err(Error::Capacity(
                "aggregate exceeds its admitted snapshot reservation",
            ));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.validate_references()?;
        if !self.clock.setup_complete()
            && (self.scene != Scene::Boot
                || self
                    .config
                    .roles
                    .heroes
                    .iter()
                    .any(|seat| self.players.players[seat.index()].life != LifeState::Unplaced))
        {
            return Err(Error::State(
                "uncommitted setup contains a partial hero transaction",
            ));
        }
        if self.clock.setup_complete() && self.scene == Scene::Boot {
            return Err(Error::State("committed setup has not left boot"));
        }
        Ok(())
    }
    /// Pure pre-mutation gate; constructing valid data never selects gameplay.
    pub fn admit(&self) -> Result<()> {
        self.validate()?;
        self.validate_encoded_reservation()?;
        if self.config.roles.director.is_some() {
            return Err(Error::Ownership(
                "Overlord role/effect closure is not admitted",
            ));
        }
        ownership::admit(self.config.body_manifest, &self.callbacks)
    }
}

/// Reusable no-allocation size calculation with the exact sidecar wire widths.
fn encoded_size(value: &impl Encode) -> Result<u32> {
    struct Count(u32);
    impl bincode::enc::write::Writer for Count {
        fn write(&mut self, bytes: &[u8]) -> std::result::Result<(), bincode::error::EncodeError> {
            self.0 = self
                .0
                .checked_add(bytes.len() as u32)
                .ok_or(bincode::error::EncodeError::Other("snapshot size overflow"))?;
            Ok(())
        }
    }
    let mut count = Count(0);
    bincode::encode_into_writer(
        value,
        &mut count,
        bincode::config::standard()
            .with_fixed_int_encoding()
            .with_little_endian(),
    )
    .map_err(|_| Error::Capacity("snapshot encoded size"))?;
    Ok(count.0)
}

#[cfg(feature = "enhanced-mode")]
impl tje_engine_runtime::enhanced::Sidecar for Session {
    const KEY: &'static str = "tje/simulation/session";
    const VERSION: u32 = 3;
    fn validate(&self) -> tje_game::Result<()> {
        Session::validate(self).map_err(|error| tje_game::Error::Platform(error.to_string()))
    }
    fn configuration_identity(&self) -> &[u8] {
        &self.configuration
    }
}

fn player_error(error: PlayerError) -> Error {
    match error {
        PlayerError::Capacity(reason) => Error::Capacity(reason),
        PlayerError::Invalid(reason)
        | PlayerError::Geometry(reason)
        | PlayerError::Rejected(reason) => Error::State(reason),
    }
}
