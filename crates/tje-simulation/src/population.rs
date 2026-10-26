//! Mode-owned Earthlings. Persistent identities/motion are independent of logical
//! body allocation and of all presentation instances. No cartridge object ABI is used.
mod ai;
mod contacts;
mod definitions;
pub mod director;
mod effects;
mod preparation;
mod preparation_job;
pub mod rom;
mod spawning;
pub mod stock;
mod terrain;

use crate::{ActorId, LevelId, SeatId, SeatMask};
pub use ai::nearest_target;
use bincode::{Decode, Encode};
pub use definitions::{
    Definition, DefinitionCatalog, Kind, Movement, ProjectileVectors, Service, Thinker, TickRange,
    Touch,
};
pub use preparation_job::{StockPopulationJob, StockPopulationProgress};
pub use terrain::WorldEnvironment;
use tje_four_player::state::{Motion, Position};
use tje_world::{Dimensions, Error, Result, population::StableBits, random::StageRng};

pub const ORIGINAL_POPULATION: u16 = 20;
pub const MAX_POPULATION: u16 = 64;
pub const MAX_ACTIVE_BODIES: u16 = 320;
pub const INTEREST_X: i64 = 480;
pub const INTEREST_Y: i64 = 336;
pub const SLEEP_STATE: u8 = 99;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Encode, Decode)]
pub struct PopulationId {
    pub level: LevelId,
    pub local: u16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct AnimationCursor {
    pub sequence_index: u16,
    pub ticks_left: u16,
    pub held: bool,
    pub flip_horizontal: bool,
    pub started: bool,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnimationSample {
    pub frame: u16,
    /// Some only at the authored frame-motion callback boundary.
    pub motion: Option<[i8; 2]>,
    pub finished: bool,
}

/// Borrowed, explicitly hero-only eligibility. UI overlays deliberately do not
/// suppress damage, targeting, AI or activity in the generalized ruleset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeroView {
    pub seat: SeatId,
    pub level: LevelId,
    pub position: Position,
    pub width: u8,
    pub velocity: [i32; 3],
    pub targetable: bool,
    pub active_motion: bool,
    pub receives_effects: bool,
    pub swimming: bool,
    pub elevator: bool,
    pub suppressed: bool,
    pub has_inventory: bool,
    pub remapped: bool,
    pub sneaking: bool,
    pub burping: bool,
    pub service_eligible: bool,
    pub response: Option<(PopulationId, bool)>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecoyView {
    pub actor: ActorId,
    pub owner: SeatId,
    pub level: LevelId,
    pub position: Position,
    pub width: u8,
    pub velocity: [i32; 3],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionBody {
    pub actor: ActorId,
    pub level: LevelId,
    pub position: Position,
    pub width: u8,
    pub collidable: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Hero(SeatId),
    Decoy(ActorId),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetSelection {
    pub target: Target,
    pub delta_x: i32,
    pub delta_y: i32,
    pub distance_squared: i64,
    pub position: Position,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStage {
    Offer,
    Begin,
    Tick,
    End,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactEffect {
    Hurt {
        damage: u8,
    },
    Squish {
        damage: u8,
    },
    Hula {
        distance: u16,
    },
    Whirlwind,
    Boogie {
        damage: u8,
    },
    MoleCapture,
    MoleTheft {
        random: u16,
    },
    MoleRelease,
    SantaDrops {
        count: u8,
    },
    ShipPiece,
    Service {
        service: Service,
        stage: ServiceStage,
        cost: u8,
    },
    DecoyHit {
        decoy: ActorId,
    },
    Projectile {
        remap: bool,
        position: Position,
        velocity: [i32; 3],
        acceleration: [i32; 3],
    },
    SharkAlert,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactRequest {
    pub source: PopulationId,
    pub actor: ActorId,
    pub seat: SeatId,
    pub animation_finished: bool,
    pub interest: SeatMask,
    pub kind: Kind,
    pub position: Position,
    pub effect: ContactEffect,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactOutcome {
    Rejected,
    Applied,
    Complete,
}

/// The aggregate adapter performs real player/economy/drop transactions here.
/// Hero eligibility is re-read after every contact, not cached for an entire pass.
/// Rejected operations MUST NOT partially charge, drop, steal or change a hero.
pub trait ContactSink {
    fn hero(&self, seat: SeatId) -> Option<HeroView>;
    fn apply(&mut self, request: ContactRequest) -> Result<ContactOutcome>;
}

/// A checked read-only resource/geometry adapter, never a render camera or GPU.
/// Animation stepping belongs to simulation cadence even when nothing is drawn.
pub trait Environment {
    fn dimensions(&self, level: LevelId) -> Result<Dimensions>;
    fn terrain(&self, level: LevelId, x: i32, y: i32) -> Result<u8>;
    fn base_metatile(&self, level: LevelId, x: i32, y: i32) -> Result<Option<u8>>;
    fn placement(
        &self,
        level: LevelId,
        kind: Kind,
        column: u16,
        row: u16,
        rng: &mut StageRng,
    ) -> Result<Option<(i32, i32)>>;
    fn allows_placement(&self, level: LevelId, x: i32, y: i32) -> Result<bool>;
    fn animation_step(
        &self,
        kind: Kind,
        state: u8,
        cursor: &mut AnimationCursor,
    ) -> Result<AnimationSample>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct PopulationLimits {
    pub per_floor: u16,
    pub active_bodies: u16,
}
impl Default for PopulationLimits {
    fn default() -> Self {
        Self {
            per_floor: MAX_POPULATION,
            active_bodies: 140,
        }
    }
}
impl PopulationLimits {
    pub fn validate(self) -> Result<()> {
        if self.per_floor == 0
            || self.per_floor > MAX_POPULATION
            || self.active_bodies == 0
            || self.active_bodies > MAX_ACTIVE_BODIES
        {
            return Err(Error::Capacity("population limits"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub enum Interaction {
    #[default]
    Idle,
    Offered,
    Active,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PersistentActor {
    pub id: PopulationId,
    pub kind: Kind,
    pub owner: Option<SeatId>,
    pub pending_director: bool,
    pub motion: Motion,
    pub saved_position: Position,
    pub state: u8,
    pub timer: i16,
    pub state_delay: u8,
    pub health: u8,
    pub retired: bool,
    pub phase_count: i8,
    pub terrain_status: u8,
    pub animation: AnimationCursor,
    pub frame: u16,
    pub animation_frame: u16,
    pub animation_finished: bool,
    pub blocked: bool,
    pub hidden: bool,
    pub collidable: bool,
    pub draw_late: bool,
    pub shadow: bool,
    pub masked: bool,
    pub mail_order: bool,
    pub companion: bool,
    pub body: Option<ActorId>,
    pub companion_body: Option<ActorId>,
    pub interest: SeatMask,
    pub mole_contacts: SeatMask,
    pub target_alerts: SeatMask,
    pub interactions: [Interaction; 4],
    pub service_owner: Option<SeatId>,
    pub service_timer: u16,
    pub rng: StageRng,
}
impl PersistentActor {
    fn new(
        id: PopulationId,
        kind: Kind,
        position: Position,
        definition: &Definition,
        mut rng: StageRng,
        mail_order: bool,
    ) -> Self {
        let companion = definition.companion && rng.next_u16() & 1 != 0;
        let state = if companion {
            2
        } else if kind == Kind::Whirlwind {
            3
        } else {
            0
        };
        let mut motion = Motion {
            position,
            ..Motion::default()
        };
        if kind == Kind::Cupid {
            motion.position.elevation = 40 * 256;
        }
        Self {
            id,
            kind,
            owner: None,
            pending_director: false,
            motion,
            saved_position: position,
            state,
            timer: 0,
            state_delay: definition.state_delay,
            health: definition.health,
            retired: false,
            phase_count: -1,
            terrain_status: 0,
            animation: AnimationCursor::default(),
            frame: 0,
            animation_frame: 0,
            animation_finished: false,
            blocked: false,
            hidden: kind == Kind::Whirlwind,
            collidable: kind != Kind::Shark,
            draw_late: false,
            shadow: false,
            masked: false,
            mail_order,
            companion,
            body: None,
            companion_body: None,
            interest: SeatMask::NONE,
            mole_contacts: SeatMask::NONE,
            target_alerts: SeatMask::NONE,
            interactions: [Interaction::Idle; 4],
            service_owner: None,
            service_timer: 0,
            rng,
        }
    }
    pub fn pixels(&self) -> (i32, i32) {
        (self.motion.position.x >> 8, self.motion.position.y >> 8)
    }
    pub fn set_state(&mut self, state: u8) {
        if self.state != state {
            self.state = state;
            self.animation = AnimationCursor::default();
            self.animation_finished = false;
            self.frame = 0;
            self.animation_frame = 0;
        }
        if state != 2 {
            self.companion = false;
        }
    }
    fn validate(&self) -> Result<()> {
        self.id.level.validate()?;
        if let Some(owner) = self.owner {
            owner
                .validate()
                .map_err(|_| Error::State("Earthling owner seat"))?;
        }
        self.motion
            .validate()
            .map_err(|_| Error::State("Earthling motion"))?;
        self.saved_position
            .validate()
            .map_err(|_| Error::State("Earthling saved position"))?;
        if self.id.local >= MAX_POPULATION
            || (self.state > 15 && self.state != SLEEP_STATE)
            || self.health > 127
            || !self.rng.valid()
            || self.animation.sequence_index > 256
            || self.animation.ticks_left > 32767
            || self.frame > 256
            || self.animation_frame > 127
            || self
                .service_owner
                .is_some_and(|seat| SeatId::new(seat.get()).is_none())
            || (self.mail_order && self.kind != Kind::Mailbox)
            || (self.companion && self.state != 2)
        {
            return Err(Error::State("Earthling persistent state"));
        }
        for mask in [self.interest, self.mole_contacts, self.target_alerts] {
            mask.validate()
                .map_err(|_| Error::State("Earthling seat mask"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthoredActor {
    pub local: u16,
    pub kind: Kind,
    pub position: Position,
    pub mail_order: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PopulationFloor {
    pub level: LevelId,
    pub actors: Vec<PersistentActor>,
    pub removed: StableBits,
    pub failed: StableBits,
    pub collected: StableBits,
    pub spawn_rng: StageRng,
    /// Includes unused original source slots; IDs are never compacted or reused.
    pub declared_slots: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
struct BodySlot {
    generation: u32,
    owner: Option<PopulationId>,
    companion: bool,
}

/// Linear, non-serialized receipt for a synchronous aggregate arrival commit.
#[derive(Debug)]
pub struct PreparedRetains {
    previous: [Option<LevelId>; 4],
    next: [Option<LevelId>; 4],
    identity: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PopulationState {
    limits: PopulationLimits,
    identity: [u8; 32],
    floors: Vec<PopulationFloor>,
    retained: [Option<LevelId>; 4],
    bodies: Vec<BodySlot>,
    sleep_contacts: Vec<effects::EffectContact>,
    thorn_contacts: Vec<effects::EffectContact>,
    consumed_projectiles: [u32; 64],
    directors: [Option<director::DirectorSlot>; director::MAX_DIRECTOR_SPAWNS],
    next_director_source: u32,
    pub parity: u8,
    pub contact_priority: u8,
    last_tick: Option<u64>,
    last_motion_tick: Option<u64>,
    last_animation_tick: Option<u64>,
}
impl PopulationState {
    pub fn new(limits: PopulationLimits, identity: [u8; 32]) -> Result<Self> {
        limits.validate()?;
        Ok(Self {
            limits,
            identity,
            floors: Vec::with_capacity(32),
            retained: [None; 4],
            bodies: (0..limits.active_bodies)
                .map(|_| BodySlot {
                    generation: 1,
                    owner: None,
                    companion: false,
                })
                .collect(),
            sleep_contacts: Vec::with_capacity(effects::MAX_CONTACTS),
            thorn_contacts: Vec::with_capacity(effects::MAX_CONTACTS),
            consumed_projectiles: [0; 64],
            directors: [None; director::MAX_DIRECTOR_SPAWNS],
            next_director_source: 1,
            parity: 0,
            contact_priority: 0,
            last_tick: None,
            last_motion_tick: None,
            last_animation_tick: None,
        })
    }
    pub fn limits(&self) -> PopulationLimits {
        self.limits
    }
    pub fn identity(&self) -> &[u8; 32] {
        &self.identity
    }
    pub fn floors(&self) -> &[PopulationFloor] {
        &self.floors
    }
    pub fn retained(&self) -> &[Option<LevelId>; 4] {
        &self.retained
    }
    pub fn active_bodies(&self) -> impl Iterator<Item = (ActorId, PopulationId, bool)> + '_ {
        self.bodies.iter().enumerate().filter_map(|(index, slot)| {
            slot.owner.map(|owner| {
                (
                    ActorId::new(32 + index as u16, slot.generation).expect("validated body slot"),
                    owner,
                    slot.companion,
                )
            })
        })
    }
    pub fn collision_bodies<'a>(
        &'a self,
        catalog: &'a DefinitionCatalog,
    ) -> impl Iterator<Item = CollisionBody> + 'a {
        self.floors.iter().flat_map(move |floor| {
            floor.actors.iter().filter_map(move |actor| {
                actor.body.map(|body| CollisionBody {
                    actor: body,
                    level: actor.id.level,
                    position: actor.motion.position,
                    width: catalog.get(actor.kind).width,
                    collidable: actor.collidable && !actor.hidden && !actor.retired,
                })
            })
        })
    }
    fn floor_index(&self, level: LevelId) -> Result<usize> {
        self.floors
            .binary_search_by_key(&level, |floor| floor.level)
            .map_err(|_| Error::State("population floor not loaded"))
    }
    pub fn actor(&self, id: PopulationId) -> Option<&PersistentActor> {
        let floor = self.floors.get(self.floor_index(id.level).ok()?)?;
        floor
            .actors
            .binary_search_by_key(&id.local, |actor| actor.id.local)
            .ok()
            .map(|index| &floor.actors[index])
    }
    pub fn actor_mut(&mut self, id: PopulationId) -> Option<&mut PersistentActor> {
        let floor_index = self.floor_index(id.level).ok()?;
        let floor = &mut self.floors[floor_index];
        let index = floor
            .actors
            .binary_search_by_key(&id.local, |actor| actor.id.local)
            .ok()?;
        Some(&mut floor.actors[index])
    }
    pub fn resolve(&self, body: ActorId) -> Option<&PersistentActor> {
        let index = body.slot().checked_sub(32)? as usize;
        let slot = self.bodies.get(index)?;
        if slot.generation != body.generation() {
            return None;
        }
        self.actor(slot.owner?)
    }
    /// An effect may retain its original owner after that body leaves interest.
    /// Vacant generation N has issued only 1..N; active N includes N itself.
    pub fn known_body(&self, body: ActorId) -> bool {
        let Some(index) = body.slot().checked_sub(32) else {
            return false;
        };
        let Some(slot) = self.bodies.get(usize::from(index)) else {
            return false;
        };
        slot.generation
            .checked_sub(u32::from(slot.owner.is_none()))
            .is_some_and(|issued| body.valid() && body.generation() <= issued)
    }
    pub fn service(&self, body: ActorId) -> Option<(&PersistentActor, Service)> {
        let actor = self.resolve(body)?;
        if actor.retired || actor.kind == Kind::Mailbox && !actor.mail_order {
            return None;
        }
        Some((actor, actor.kind.service()?))
    }
    /// Prepare once at loading time. Reentry is an explicit no-op, never regeneration.
    pub fn stage_floor(
        &mut self,
        level: LevelId,
        actors: &[AuthoredActor],
        catalog: &DefinitionCatalog,
    ) -> Result<bool> {
        level.validate()?;
        if self.floor_index(level).is_ok() {
            return Ok(false);
        }
        if self.floors.len() == 32 || actors.len() > usize::from(self.limits.per_floor) {
            return Err(Error::Capacity("persistent population"));
        }
        let mut previous = None;
        for actor in actors {
            actor
                .position
                .validate()
                .map_err(|_| Error::Bounds("population position"))?;
            if actor.local >= self.limits.per_floor
                || previous.is_some_and(|id| id >= actor.local)
                || (actor.mail_order && actor.kind != Kind::Mailbox)
            {
                return Err(Error::State("ordered population IDs/mailbox mode"));
            }
            previous = Some(actor.local);
        }
        let mut records = Vec::with_capacity(usize::from(self.limits.per_floor));
        for actor in actors {
            let rng = initial_actor_rng(
                &self.identity,
                PopulationId {
                    level,
                    local: actor.local,
                },
            );
            records.push(PersistentActor::new(
                PopulationId {
                    level,
                    local: actor.local,
                },
                actor.kind,
                actor.position,
                catalog.get(actor.kind),
                rng,
                actor.mail_order,
            ));
        }
        let floor = PopulationFloor {
            level,
            actors: records,
            removed: StableBits::new(MAX_POPULATION)?,
            failed: StableBits::new(MAX_POPULATION)?,
            collected: StableBits::new(MAX_POPULATION)?,
            spawn_rng: StageRng::derive(&self.identity, level, 0, b"population-present"),
            declared_slots: actors
                .iter()
                .fold(0, |bits, actor| bits | (1_u64 << actor.local)),
        };
        let index = self
            .floors
            .binary_search_by_key(&level, |floor| floor.level)
            .expect_err("not yet staged");
        self.floors.insert(index, floor);
        Ok(true)
    }
    pub fn retain(&mut self, seat: SeatId, level: LevelId) -> Result<()> {
        seat.validate()
            .map_err(|_| Error::State("population retain seat"))?;
        let mut next = self.retained;
        next[seat.index()] = Some(level);
        let prepared = self.prepare_retains(next)?;
        self.commit_retains(prepared);
        Ok(())
    }
    pub fn preflight_retain(&self, seat: SeatId, level: LevelId) -> Result<()> {
        seat.validate()
            .map_err(|_| Error::State("population retain seat"))?;
        let mut next = self.retained;
        next[seat.index()] = Some(level);
        self.prepare_retains(next).map(|_| ())
    }
    pub fn prepare_retains(&self, next: [Option<LevelId>; 4]) -> Result<PreparedRetains> {
        self.limits.validate()?;
        if self.potential_body_demand(&next)? > self.bodies.len() {
            return Err(Error::Capacity("retained-floor logical body reservations"));
        }
        Ok(PreparedRetains {
            previous: self.retained,
            next,
            identity: self.identity,
        })
    }
    /// Preflight all aggregate owners before changing any, then consume their
    /// linear receipts without yielding or running unrelated mutations.
    pub fn commit_retains(&mut self, prepared: PreparedRetains) {
        debug_assert_eq!(prepared.identity, self.identity);
        debug_assert_eq!(prepared.previous, self.retained);
        self.retained = prepared.next;
    }
    fn potential_body_demand(&self, retained: &[Option<LevelId>; 4]) -> Result<usize> {
        let mut required = self
            .floors
            .iter()
            .flat_map(|floor| &floor.actors)
            .filter(|actor| actor.pending_director)
            .count();
        for (index, level) in retained.iter().enumerate() {
            let Some(level) = level else {
                continue;
            };
            if retained[..index].contains(&Some(*level)) {
                continue;
            }
            let floor = &self.floors[self.floor_index(*level)?];
            for actor in &floor.actors {
                if !actor.pending_director
                    && !actor.retired
                    && !floor.removed.contains(actor.id.local)
                    && !floor.failed.contains(actor.id.local)
                {
                    required += 1 + usize::from(actor.companion);
                }
            }
        }
        Ok(required)
    }
    fn preflight_growth(&self, additional: usize) -> Result<()> {
        if self
            .potential_body_demand(&self.retained)?
            .checked_add(additional)
            .is_none_or(|required| required > self.bodies.len())
        {
            return Err(Error::Capacity("future-interest logical body reservations"));
        }
        Ok(())
    }
    pub fn release(&mut self, seat: SeatId) -> Result<()> {
        seat.validate()
            .map_err(|_| Error::State("population release seat"))?;
        self.retained[seat.index()] = None;
        Ok(())
    }
    pub fn release_seat_contacts(&mut self, seat: SeatId) -> Result<()> {
        seat.validate()
            .map_err(|_| Error::State("released population contact seat"))?;
        let bit = !(1_u8 << seat.get());
        for floor in &mut self.floors {
            for actor in &mut floor.actors {
                actor.mole_contacts =
                    SeatMask::new(actor.mole_contacts.bits() & bit).expect("four seats");
                actor.target_alerts =
                    SeatMask::new(actor.target_alerts.bits() & bit).expect("four seats");
                actor.interactions[seat.index()] = Interaction::Idle;
                if actor.service_owner == Some(seat) {
                    actor.service_owner = None;
                    actor.service_timer = 0;
                    if actor.kind.service().is_some() {
                        actor.set_state(0);
                        actor.timer = 0;
                        actor.state_delay = 0;
                    }
                }
            }
        }
        Ok(())
    }
    /// Preflight the entire activity change before allocating or releasing anything.
    /// Resource pressure cannot relocate, erase, reroll or partially activate actors.
    pub fn refresh_activity(&mut self, heroes: &[Option<HeroView>; 4]) -> Result<()> {
        validate_heroes(heroes, &self.retained)?;
        let desired = |floor: &PopulationFloor, actor: &PersistentActor| {
            if actor.pending_director
                || floor.removed.contains(actor.id.local)
                || floor.failed.contains(actor.id.local)
            {
                return SeatMask::NONE;
            }
            let (x, y) = actor.pixels();
            let mut mask = 0;
            for hero in heroes.iter().flatten() {
                if hero.level == floor.level
                    && (actor.kind.required()
                        || (i64::from(x) - i64::from(hero.position.x >> 8)).abs() <= INTEREST_X
                            && (i64::from(y) - i64::from(hero.position.y >> 8)).abs() <= INTEREST_Y)
                {
                    mask |= 1 << hero.seat.get();
                }
            }
            SeatMask::new(mask).expect("four seats")
        };
        let mut needed = self
            .floors
            .iter()
            .flat_map(|floor| &floor.actors)
            .filter(|actor| actor.pending_director)
            .count();
        for floor in &self.floors {
            for actor in &floor.actors {
                if desired(floor, actor) != SeatMask::NONE {
                    needed += 1 + usize::from(actor.companion);
                }
            }
        }
        if needed > self.bodies.len() {
            return Err(Error::Capacity("active Earthlings and companions"));
        }
        // Generation exhaustion is rejected before any release can alter state.
        for slot in &self.bodies {
            if let Some(owner) = slot.owner {
                let floor = &self.floors[self.floor_index(owner.level)?];
                let actor = self
                    .actor(owner)
                    .ok_or(Error::State("body owner missing"))?;
                if actor.pending_director {
                    continue;
                }
                if (desired(floor, actor) == SeatMask::NONE || slot.companion && !actor.companion)
                    && slot.generation == u32::MAX
                {
                    return Err(Error::Capacity("Earthling generation exhausted"));
                }
            }
        }
        for floor in &mut self.floors {
            for actor in &mut floor.actors {
                if actor.pending_director {
                    continue;
                }
                // Compute against disjoint state without cloning any floor.
                let mut mask = 0;
                if !floor.removed.contains(actor.id.local) && !floor.failed.contains(actor.id.local)
                {
                    let (x, y) = actor.pixels();
                    for hero in heroes.iter().flatten() {
                        if hero.level == floor.level
                            && (actor.kind.required()
                                || (i64::from(x) - i64::from(hero.position.x >> 8)).abs()
                                    <= INTEREST_X
                                    && (i64::from(y) - i64::from(hero.position.y >> 8)).abs()
                                        <= INTEREST_Y)
                        {
                            mask |= 1 << hero.seat.get();
                        }
                    }
                }
                actor.interest = SeatMask::new(mask).expect("four seats");
                if mask == 0 {
                    release_body(&mut self.bodies, &mut actor.body)?;
                }
                if mask == 0 || !actor.companion {
                    release_body(&mut self.bodies, &mut actor.companion_body)?;
                }
            }
        }
        for floor in &mut self.floors {
            for actor in &mut floor.actors {
                if actor.interest == SeatMask::NONE {
                    continue;
                }
                if actor.body.is_none() {
                    actor.body = Some(allocate_body(&mut self.bodies, actor.id, false)?);
                }
                if actor.companion && actor.companion_body.is_none() {
                    actor.companion_body = Some(allocate_body(&mut self.bodies, actor.id, true)?);
                }
            }
        }
        Ok(())
    }
    /// Exactly one population phase per admitted tick, including across restore.
    pub fn step(
        &mut self,
        tick: u64,
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
        decoys: &[DecoyView],
        sink: &mut impl ContactSink,
    ) -> Result<bool> {
        self.step_controlled(tick, environment, catalog, decoys, sink, None)
            .map(|receipt| receipt.advanced)
    }
    pub fn step_controlled(
        &mut self,
        tick: u64,
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
        decoys: &[DecoyView],
        sink: &mut impl ContactSink,
        intent: Option<director::DecisionIntent>,
    ) -> Result<director::StepReceipt> {
        if self.last_tick == Some(tick) {
            return Ok(director::StepReceipt::default());
        }
        if self.last_tick.is_some_and(|last| tick < last) {
            return Err(Error::State("population tick moved backwards"));
        }
        validate_decoys(decoys)?;
        let heroes = SeatId::ALL.map(|seat| sink.hero(seat));
        validate_heroes(&heroes, &self.retained)?;
        if let Some(intent) = intent {
            self.validate_control(intent, catalog)?;
        }
        let mut decision = director::DecisionSlot {
            intent,
            receipt: director::DecisionReceipt::default(),
        };
        self.retreat_santas(&heroes, environment, catalog)?;
        self.refresh_activity(&heroes)?;
        for floor in &mut self.floors {
            for actor in &mut floor.actors {
                if actor.pending_director
                    || actor.body.is_none()
                    || actor.id.local % 2 != u16::from(self.parity)
                {
                    continue;
                }
                if actor.state != SLEEP_STATE {
                    ai::think_controlled(actor, catalog, &heroes, decoys, sink, &mut decision)?;
                    contacts::dispatch(
                        actor,
                        catalog.get(actor.kind),
                        self.contact_priority,
                        sink,
                    )?;
                    contacts::decoy_contacts(actor, catalog.get(actor.kind), decoys);
                    if !actor.companion {
                        release_body(&mut self.bodies, &mut actor.companion_body)?;
                    }
                }
            }
        }
        self.last_tick = Some(tick);
        self.parity ^= 1;
        self.contact_priority = (self.contact_priority + 1) & 3;
        self.remove_collected()?;
        Ok(director::StepReceipt {
            advanced: true,
            decision: decision.receipt,
        })
    }
    /// Continuous 24.8 motion for every active actor, on both even phases.
    pub fn advance_motion(
        &mut self,
        tick: u64,
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
        decoys: &[DecoyView],
        sink: &mut impl ContactSink,
    ) -> Result<bool> {
        if self.last_motion_tick == Some(tick) {
            return Ok(false);
        }
        if self.last_motion_tick.is_some_and(|last| tick < last) {
            return Err(Error::State("population motion tick moved backwards"));
        }
        self.advance_bodies(environment, catalog, decoys, sink, true)?;
        self.last_motion_tick = Some(tick);
        Ok(true)
    }
    /// Logical animation admission after movement, before speech, regardless of
    /// whether any view, tile cache or graphics backend visits this actor.
    pub fn advance_animation(
        &mut self,
        tick: u64,
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
        decoys: &[DecoyView],
        sink: &mut impl ContactSink,
    ) -> Result<bool> {
        if self.last_animation_tick == Some(tick) {
            return Ok(false);
        }
        if self.last_animation_tick.is_some_and(|last| tick < last) {
            return Err(Error::State("population animation tick moved backwards"));
        }
        self.advance_bodies(environment, catalog, decoys, sink, false)?;
        self.last_animation_tick = Some(tick);
        Ok(true)
    }
    fn advance_bodies(
        &mut self,
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
        decoys: &[DecoyView],
        sink: &mut impl ContactSink,
        physics: bool,
    ) -> Result<()> {
        let heroes = SeatId::ALL.map(|seat| sink.hero(seat));
        validate_heroes(&heroes, &self.retained)?;
        validate_decoys(decoys)?;
        self.retreat_santas(&heroes, environment, catalog)?;
        self.refresh_activity(&heroes)?;
        let mut collisions = [None; MAX_ACTIVE_BODIES as usize];
        for body in self.collision_bodies(catalog) {
            collisions[usize::from(body.actor.slot() - 32)] = Some(body);
        }
        for floor in &mut self.floors {
            for actor in &mut floor.actors {
                if actor.pending_director || actor.body.is_none() {
                    continue;
                }
                let sample = if physics {
                    AnimationSample::default()
                } else {
                    let sample = environment.animation_step(
                        actor.kind,
                        actor.state,
                        &mut actor.animation,
                    )?;
                    actor.frame = actor.animation.sequence_index;
                    actor.animation_frame = sample.frame;
                    actor.animation_finished = sample.finished;
                    sample
                };
                terrain::move_actor(
                    actor,
                    catalog.get(actor.kind),
                    sample,
                    physics,
                    environment,
                    &terrain::CollisionScene {
                        heroes: &heroes,
                        decoys,
                        actors: &collisions,
                    },
                    sink,
                )?;
                if let Some(body) = actor.body
                    && let Some(collision) = &mut collisions[usize::from(body.slot() - 32)]
                {
                    collision.position = actor.motion.position;
                    collision.collidable = actor.collidable && !actor.hidden && !actor.retired;
                }
            }
        }
        self.retreat_santas(&heroes, environment, catalog)?;
        self.remove_collected()
    }
    pub fn remove(&mut self, id: PopulationId, collected: bool) -> Result<bool> {
        let floor_index = self.floor_index(id.level)?;
        let floor = &mut self.floors[floor_index];
        let actor = floor
            .actors
            .iter_mut()
            .find(|actor| actor.id == id)
            .ok_or(Error::State("remove unknown Earthling"))?;
        if floor.removed.contains(id.local) {
            return Ok(false);
        }
        for body in [actor.body, actor.companion_body].into_iter().flatten() {
            if self.bodies[usize::from(body.slot() - 32)].generation == u32::MAX {
                return Err(Error::Capacity("Earthling generation exhausted"));
            }
        }
        release_body(&mut self.bodies, &mut actor.body)?;
        release_body(&mut self.bodies, &mut actor.companion_body)?;
        actor.interest = SeatMask::NONE;
        actor.retired = true;
        floor.removed.insert(id.local)?;
        if collected {
            floor.collected.insert(id.local)?;
        }
        self.discard_actor_contacts(id);
        Ok(true)
    }
    fn remove_collected(&mut self) -> Result<()> {
        for floor in &mut self.floors {
            for actor in &mut floor.actors {
                if !actor.retired {
                    continue;
                }
                if floor.removed.contains(actor.id.local) {
                    continue;
                }
                release_body(&mut self.bodies, &mut actor.body)?;
                release_body(&mut self.bodies, &mut actor.companion_body)?;
                actor.interest = SeatMask::NONE;
                floor.removed.insert(actor.id.local)?;
                if actor.kind == Kind::ShipPiece {
                    floor.collected.insert(actor.id.local)?;
                }
            }
        }
        self.discard_removed_contacts();
        Ok(())
    }
    fn prepared_damage(&self, id: PopulationId, points: u8) -> Result<Option<u8>> {
        if points == 0 {
            return Ok(None);
        }
        let actor = self
            .actor(id)
            .ok_or(Error::State("damage unknown Earthling"))?;
        if actor.pending_director
            || actor.retired
            || actor.mail_order
            || matches!(
                actor.kind,
                Kind::ShipPiece | Kind::Santa | Kind::LemonadeStand | Kind::HotTub
            )
        {
            return Ok(None);
        }
        let protected = actor.kind == Kind::OperaSinger && actor.state == 3
            || actor.kind == Kind::Wizard && actor.state == 2
            || actor.kind == Kind::Wiseman
                && actor.state <= 1
                && actor.interactions.contains(&Interaction::Active);
        let health = actor.health.saturating_sub(points).max(u8::from(protected));
        if health == 0 {
            preflight_body_release(&self.bodies, actor.body)?;
        }
        if health == 0 || actor.companion {
            preflight_body_release(&self.bodies, actor.companion_body)?;
        }
        Ok(Some(health))
    }
    pub fn damage(&mut self, id: PopulationId, points: u8) -> Result<bool> {
        let Some(health) = self.prepared_damage(id, points)? else {
            return Ok(false);
        };
        let floor_index = self.floor_index(id.level)?;
        let actor = self.floors[floor_index]
            .actors
            .iter_mut()
            .find(|actor| actor.id == id)
            .ok_or(Error::State("damage unknown Earthling"))?;
        actor.health = health;
        if actor.companion {
            actor.set_state(0);
            actor.timer = 0;
            release_body(&mut self.bodies, &mut actor.companion_body)?;
        }
        if actor.health == 0 {
            self.remove(id, false)?;
        }
        Ok(true)
    }
    pub fn damage_body(&mut self, body: ActorId, points: u8) -> Result<bool> {
        let id = self
            .resolve(body)
            .ok_or(Error::State("stale Earthling damage body"))?
            .id;
        self.damage(id, points)
    }
    /// Viking/opera uses canonical hero interest, never a viewport draw mask.
    pub fn damage_interest(&mut self, seats: SeatMask) -> Result<usize> {
        seats
            .validate()
            .map_err(|_| Error::State("Earthling damage seat mask"))?;
        let mut count = 0;
        for floor_index in 0..self.floors.len() {
            for actor_index in 0..self.floors[floor_index].actors.len() {
                let actor = &self.floors[floor_index].actors[actor_index];
                if actor.body.is_none()
                    || actor.kind == Kind::OperaSinger
                    || actor.interest.bits() & seats.bits() == 0
                {
                    continue;
                }
                let id = actor.id;
                if self.damage(id, 1)? {
                    count += 1;
                }
            }
        }
        Ok(count)
    }
    pub fn clear_mole_contacts(&mut self, seat: SeatId, level: LevelId) -> Result<()> {
        seat.validate()
            .map_err(|_| Error::State("mole contact seat"))?;
        let index = self.floor_index(level)?;
        for actor in &mut self.floors[index].actors {
            if actor.kind == Kind::Mole {
                actor.mole_contacts =
                    SeatMask::new(actor.mole_contacts.bits() & !(1 << seat.get()))
                        .expect("four seats");
            }
        }
        Ok(())
    }
    pub fn set_service_collision(&mut self, body: ActorId, collidable: bool) -> Result<()> {
        let id = self
            .resolve(body)
            .ok_or(Error::State("stale service collision body"))?
            .id;
        let actor = self.actor_mut(id).expect("resolved stable actor");
        if actor.kind != Kind::HotTub {
            return Err(Error::State("non-hot-tub service collision"));
        }
        actor.collidable = collidable;
        Ok(())
    }
    /// Cross-sidecar references are checked by the aggregate before capture or
    /// restore commit. A population floor cannot silently outlive its geometry.
    pub fn validate_world(&self, world: &tje_world::world::WorldState) -> Result<()> {
        self.validate()?;
        if self.retained != *world.active() {
            return Err(Error::State("population/world active floors disagree"));
        }
        for floor in &self.floors {
            let geometry = &world
                .floor(floor.level)
                .ok_or(Error::State("population geometry absent"))?
                .generated
                .geometry;
            let (width, height) = geometry.dimensions().world_envelope();
            for actor in &floor.actors {
                let (x, y) = actor.pixels();
                if x < 0 || y < 0 || x >= width || y >= height {
                    return Err(Error::State("persistent Earthling outside level"));
                }
            }
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.limits.validate()?;
        self.preflight_growth(0)?;
        if self.floors.len() > 32
            || self.bodies.len() != usize::from(self.limits.active_bodies)
            || self.parity > 1
            || self.contact_priority > 3
        {
            return Err(Error::State("population extent/phase"));
        }
        self.validate_directors()?;
        for level in self.retained.iter().flatten() {
            self.floor_index(*level)?;
        }
        let mut previous = None;
        for floor in &self.floors {
            floor.level.validate()?;
            if previous.is_some_and(|level| level >= floor.level)
                || floor.actors.len() > usize::from(self.limits.per_floor)
            {
                return Err(Error::State("ordered population floors"));
            }
            previous = Some(floor.level);
            floor.validate_records(self.limits.per_floor)?;
            for actor in &floor.actors {
                for (handle, companion) in [(actor.body, false), (actor.companion_body, true)] {
                    if let Some(handle) = handle {
                        let slot = self
                            .bodies
                            .get(usize::from(
                                handle
                                    .slot()
                                    .checked_sub(32)
                                    .ok_or(Error::State("body domain"))?,
                            ))
                            .ok_or(Error::State("body extent"))?;
                        if slot.owner != Some(actor.id)
                            || slot.generation != handle.generation()
                            || slot.companion != companion
                        {
                            return Err(Error::State("body generation/owner reference"));
                        }
                    }
                }
            }
        }
        for (index, slot) in self.bodies.iter().enumerate() {
            if slot.generation == 0 {
                return Err(Error::State("zero Earthling generation"));
            }
            if let Some(owner) = slot.owner {
                let actor = self
                    .actor(owner)
                    .ok_or(Error::State("body owner not persistent"))?;
                let handle = if slot.companion {
                    actor.companion_body
                } else {
                    actor.body
                };
                if handle != ActorId::new(32 + index as u16, slot.generation) {
                    return Err(Error::State("unreferenced Earthling body"));
                }
            }
        }
        self.validate_effect_contacts()?;
        Ok(())
    }
}

fn initial_actor_rng(identity: &[u8; 32], id: PopulationId) -> StageRng {
    let mut label = *b"runtime-ai\0\0";
    label[10..].copy_from_slice(&id.local.to_be_bytes());
    StageRng::derive(identity, id.level, 0, &label)
}

fn validate_heroes(heroes: &[Option<HeroView>; 4], retained: &[Option<LevelId>; 4]) -> Result<()> {
    for (index, hero) in heroes.iter().enumerate() {
        if let Some(hero) = hero {
            if hero.width == 0
                || hero
                    .velocity
                    .iter()
                    .any(|value| value.unsigned_abs() > 64 * 256)
            {
                return Err(Error::Bounds("hero collision/target motion"));
            }
            if hero.seat.index() != index || retained[index] != Some(hero.level) {
                return Err(Error::State("hero/population retain mismatch"));
            }
            hero.position
                .validate()
                .map_err(|_| Error::Bounds("hero interest position"))?;
        } else if retained[index].is_some() {
            return Err(Error::State("population retained by absent hero"));
        }
    }
    Ok(())
}
fn validate_decoys(decoys: &[DecoyView]) -> Result<()> {
    if decoys.len() > 4 {
        return Err(Error::Capacity("four hero decoys"));
    }
    let mut owners = 0;
    for decoy in decoys {
        decoy
            .owner
            .validate()
            .map_err(|_| Error::State("decoy owner"))?;
        decoy.level.validate()?;
        decoy
            .position
            .validate()
            .map_err(|_| Error::Bounds("decoy target position"))?;
        if decoy.width == 0
            || decoy
                .velocity
                .iter()
                .any(|value| value.unsigned_abs() > 64 * 256)
        {
            return Err(Error::Bounds("decoy collision/target motion"));
        }
        if !decoy.actor.valid() || owners & (1 << decoy.owner.get()) != 0 {
            return Err(Error::State("decoy handle/duplicate owner"));
        }
        owners |= 1 << decoy.owner.get();
    }
    Ok(())
}
fn allocate_body(slots: &mut [BodySlot], owner: PopulationId, companion: bool) -> Result<ActorId> {
    let (index, slot) = slots
        .iter_mut()
        .enumerate()
        .find(|(_, slot)| slot.owner.is_none())
        .ok_or(Error::Capacity("Earthling body arena"))?;
    slot.owner = Some(owner);
    slot.companion = companion;
    Ok(ActorId::new(32 + index as u16, slot.generation).expect("bounded body arena"))
}
fn preflight_body_release(slots: &[BodySlot], handle: Option<ActorId>) -> Result<()> {
    if let Some(body) = handle {
        let index = body
            .slot()
            .checked_sub(32)
            .ok_or(Error::State("Earthling body domain"))?;
        let slot = slots
            .get(usize::from(index))
            .ok_or(Error::State("Earthling body extent"))?;
        if slot.generation != body.generation() || slot.owner.is_none() {
            return Err(Error::State("stale Earthling body"));
        }
        if slot.generation == u32::MAX {
            return Err(Error::Capacity("Earthling generation exhausted"));
        }
    }
    Ok(())
}
fn release_body(slots: &mut [BodySlot], handle: &mut Option<ActorId>) -> Result<()> {
    if let Some(body) = *handle {
        let index = body
            .slot()
            .checked_sub(32)
            .ok_or(Error::State("Earthling body domain"))?;
        let slot = slots
            .get_mut(usize::from(index))
            .ok_or(Error::State("Earthling body extent"))?;
        if slot.generation != body.generation() || slot.owner.is_none() {
            return Err(Error::State("stale Earthling body"));
        }
        slot.generation = slot
            .generation
            .checked_add(1)
            .ok_or(Error::Capacity("Earthling generation exhausted"))?;
        slot.owner = None;
        slot.companion = false;
        *handle = None;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
