//! One registered owner validates identities and references across its services.
use crate::capacity::{CapacityPlan, FLOOR_RESERVATION, GENERATION_RESERVATION, Reservation};
use crate::ownership::{self, TypedCallback};
use crate::{ActorId, Error, LevelId, PhaseClock, PublicationStamp, Result, SeatId, SeatMask};
use bincode::{Decode, Encode};
use sha2::{Digest, Sha256};
use tje_four_player::{
    CoopState,
    state::{Config as CoopConfig, LifeState},
};
use tje_world::{WorldConfig, random::StageRng, world::WorldState};

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
pub struct SessionConfig {
    pub version: u16,
    pub body_manifest: [u8; 32],
    pub world: WorldConfig,
    pub coop: CoopConfig,
    pub roles: Roles,
    pub capacities: CapacityPlan,
    pub course_identity: Option<[u8; 32]>,
    pub resources_identity: [u8; 32],
}
impl SessionConfig {
    pub fn stock(
        world: WorldConfig,
        coop: CoopConfig,
        callbacks: &[TypedCallback],
    ) -> Result<Self> {
        let heroes = coop.participants.iter().count() as u16;
        let config = Self {
            version: 1,
            body_manifest: ownership::IDENTITY,
            resources_identity: ownership::resource_identity(world.rom_sha1, callbacks)?,
            roles: Roles::coop(coop.participants),
            capacities: CapacityPlan {
                active_floors: heroes,
                ..CapacityPlan::STOCK_COOP
            },
            world,
            coop,
            course_identity: None,
        };
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<Reservation> {
        self.world
            .validate()
            .map_err(|_| Error::State("invalid world configuration"))?;
        self.coop
            .validate()
            .map_err(|_| Error::State("invalid hero configuration"))?;
        self.roles.validate()?;
        if self.version != 1
            || self.body_manifest != ownership::IDENTITY
            || self.world.root_seed != self.coop.world_seed
            || self.coop.participants != self.roles.heroes
        {
            return Err(Error::State(
                "session version/manifest/seed/role identity mismatch",
            ));
        }
        self.capacities
            .reserve(self.roles.heroes.iter().count() as u16)
    }
    pub fn identity(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"tje-simulation/config/1\0");
        hash.update(self.version.to_be_bytes());
        hash.update(self.body_manifest);
        hash.update(self.world.identity());
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
        hash.update([u8::from(self.course_identity.is_some())]);
        if let Some(course) = self.course_identity {
            hash.update(course);
        }
        hash.update(self.resources_identity);
        hash.finalize().into()
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

/// Construct off the live machine, then consume through finish_setup. No partial
/// transaction can enter the registered sidecar or a running world phase.
#[derive(Debug)]
pub struct SetupTransaction {
    session: Session,
}
impl SetupTransaction {
    pub fn prepare(
        config: SessionConfig,
        world: WorldState,
        players: CoopState,
        callbacks: Vec<TypedCallback>,
    ) -> Result<Self> {
        ownership::validate_manifest()?;
        let reservation = config.validate()?;
        let configuration = config.identity();
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
            hero_actors,
            callbacks,
            clock: PhaseClock::default(),
            rngs,
            setup_seen: 0,
            publication: None,
            failed: false,
        };
        session.validate_references()?;
        Ok(Self { session })
    }
    pub fn world_mut(&mut self) -> &mut WorldState {
        &mut self.session.world
    }
    pub fn players_mut(&mut self) -> &mut CoopState {
        &mut self.session.players
    }
    pub fn finish_setup(mut self) -> Result<Session> {
        self.session.validate_references()?;
        for seat in self.session.config.roles.heroes.iter() {
            if self.session.players.players[seat.index()].life == LifeState::Unplaced {
                return Err(Error::State("setup has an unplaced hero"));
            }
        }
        self.session.clock.complete_setup()?;
        self.session.validate()?;
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
            if floor.generated.population.earthlings.len()
                > usize::from(self.config.capacities.earthlings_per_floor)
                || floor.generated.population.items.len()
                    > usize::from(self.config.capacities.items_per_floor)
            {
                return Err(Error::Capacity(
                    "persistent population exceeds its admitted joint caps",
                ));
            }
            if encoded_size(floor)? > FLOOR_RESERVATION {
                return Err(Error::Capacity(
                    "floor exceeds its worst-case codec reservation",
                ));
            }
        }
        if self.world.dropped().len() > usize::from(self.config.capacities.drops) {
            return Err(Error::Capacity("persistent global drop reservation"));
        }
        if let Some(pending) = self.world.pending()
            && encoded_size(pending)? > GENERATION_RESERVATION
        {
            return Err(Error::Capacity(
                "pending generation exceeds its codec reservation",
            ));
        }
        if let Some(stamp) = self.publication
            && (!stamp.phase.publishes()
                || stamp.epoch == 0
                || stamp.epoch > self.clock.epoch()
                || self
                    .clock
                    .input()
                    .is_none_or(|input| stamp.hardware_frame > input.frame))
        {
            return Err(Error::State("invalid saved publication boundary"));
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
        if !self.clock.setup_complete() {
            return Err(Error::State(
                "uncommitted setup cannot be a registered session",
            ));
        }
        Ok(())
    }
    /// Pure pre-mutation gate; constructing valid data never selects gameplay.
    pub fn admit(&self) -> Result<()> {
        self.validate()?;
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
    const VERSION: u32 = 1;
    fn validate(&self) -> tje_game::Result<()> {
        Session::validate(self).map_err(|error| tje_game::Error::Platform(error.to_string()))
    }
    fn configuration_identity(&self) -> &[u8] {
        &self.configuration
    }
}
