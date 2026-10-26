//! Mode-owned player services. All callbacks have finite typed dispatch; nothing
//! below this boundary accepts an original player selector or RAM object pointer.
use crate::resources::{
    Effect, EffectKind, EffectOwner, PresentKind, ResourceLimits, ResourceState, compact_slot,
};
use crate::{ActorId, EffectId};
use bincode::{Decode, Encode};
use tje_four_player::{
    Archetype, CoopState, SEATS, SeatId, SeatMask,
    state::{FallState, LifeState, Motion, Overlay, Position},
};
use tje_world::{Dimensions, LevelId};

mod audio;
pub use audio::{AudioCommand, AudioCue, AudioIntent, AudioQueue, PcmSound};
mod audio_catalog;
mod effect_timing;
pub use audio_catalog::CheckedAudioCatalog;
pub use effect_timing::{
    AnimationPeriod, EffectAnimation, EffectAnimationTiming, effect_animation_timing,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub enum VoluntaryScale {
    #[default]
    Normal,
    Snared,
}
impl VoluntaryScale {
    fn delta(self, value: i32) -> i32 {
        match self {
            Self::Normal => value,
            Self::Snared => ((i64::from(value) * 3) / 4) as i32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerError {
    Invalid(&'static str),
    Capacity(&'static str),
    Geometry(&'static str),
    Rejected(&'static str),
}
impl std::fmt::Display for PlayerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (category, reason) = match self {
            Self::Invalid(reason) => ("invalid player state", reason),
            Self::Capacity(reason) => ("player resource capacity", reason),
            Self::Geometry(reason) => ("player geometry", reason),
            Self::Rejected(reason) => ("player action rejected", reason),
        };
        write!(formatter, "{category}: {reason}")
    }
}
impl std::error::Error for PlayerError {}
pub type PlayerResult<T> = Result<T, PlayerError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Landing {
    pub level: LevelId,
    pub position: Position,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct PlayerInput {
    /// Game order: up, down, left, right. Wire A/C conversion is upstream.
    pub directions: u8,
    pub action: bool,
    pub jump: bool,
    pub sneak: bool,
    pub wake: bool,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct AuthoredMotion {
    pub axes: u8,
    pub multiplier: [i8; 2],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct AnimationPlayback {
    pub descriptor: u8,
    pub started: bool,
    pub held: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerAnimationStep {
    pub descriptor: u8,
    pub sequence_index: u16,
    pub ticks_left: u8,
    pub flip_horizontal: bool,
    pub held: bool,
    pub motion: Option<[i8; 2]>,
}

/// Geometry is borrowed by LevelId. Collision includes solid world actors and
/// elevators but excludes the named hero; it must not depend on visible views.
pub trait PlayerGeometry {
    fn dimensions(&self, level: LevelId) -> PlayerResult<Dimensions>;
    fn terrain(&self, level: LevelId, x: i32, y: i32) -> PlayerResult<u8>;
    fn blocked(
        &self,
        seat: SeatId,
        archetype: Archetype,
        level: LevelId,
        position: Position,
    ) -> PlayerResult<bool>;
    fn respawn(&self, seat: SeatId, checkpoint: Landing) -> PlayerResult<Landing>;
    fn movement_blocked(
        &self,
        seat: SeatId,
        archetype: Archetype,
        level: LevelId,
        _previous: Position,
        proposed: Position,
    ) -> PlayerResult<bool> {
        self.blocked(seat, archetype, level, proposed)
    }
    fn terrain_cache(&self, landing: Landing) -> PlayerResult<[[u8; 5]; 5]> {
        let mut tiles = [[0; 5]; 5];
        for (row, values) in tiles.iter_mut().enumerate() {
            for (column, tile) in values.iter_mut().enumerate() {
                *tile = self.terrain(
                    landing.level,
                    ((landing.position.x >> 11) + column as i32 - 2) * 8,
                    ((landing.position.y >> 11) + row as i32 - 2) * 8,
                )?;
            }
        }
        Ok(tiles)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldPresent {
    SpawnEarthling {
        owner: SeatId,
        level: LevelId,
        position: Position,
        choice: u16,
    },
    Togetherness {
        owner: SeatId,
        target: SeatId,
        destination: Landing,
    },
    OpenDoorway {
        owner: SeatId,
        level: LevelId,
        position: Position,
        choice: u16,
    },
    OpenTelephone {
        owner: SeatId,
        level: LevelId,
        position: Position,
    },
    OpenDecoy {
        owner: SeatId,
        level: LevelId,
        position: Position,
    },
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorldCommit {
    pub effect_position: Option<Position>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorEffect {
    Projectile {
        effect: EffectId,
        owner: EffectOwner,
        level: LevelId,
        position: Position,
        damage: u8,
    },
    Roses {
        effect: EffectId,
        owner: SeatId,
        level: LevelId,
        position: Position,
    },
    Boombox {
        effect: EffectId,
        level: LevelId,
        position: Position,
    },
    ServiceCollision {
        actor: ActorId,
        collidable: bool,
    },
    WakeBoombox {
        effect: EffectId,
        level: LevelId,
    },
    Burp {
        owner: SeatId,
        level: LevelId,
        position: Position,
    },
    Retire {
        effect: EffectId,
        level: LevelId,
    },
    Opera {
        seats: SeatMask,
    },
}
/// External mutations are implemented by the aggregate's geometry/population
/// adapter, not discarded events. A rejected present MUST leave its world
/// unchanged. A successful present commits before infallible inventory removal.
/// Projectile returns true only for a real authoritative hit; sleep/music tokens
/// name their effect generation so one boombox cannot wake another's sleepers.
pub trait PlayerWorld: PlayerGeometry {
    fn commit_present(&mut self, present: WorldPresent) -> PlayerResult<WorldCommit>;
    fn commit_arrival(&mut self, seat: SeatId, landing: Landing) -> PlayerResult<()>;
    fn release_hero(&mut self, seat: SeatId) -> PlayerResult<()>;
    fn effect_created(&mut self, id: EffectId, effect: Effect) -> PlayerResult<()>;
    fn choose_present(&self, rng: &mut tje_world::random::StageRng) -> PlayerResult<PresentKind>;
    fn contact_damage(&self, kind: crate::population::Kind) -> PlayerResult<u8>;
    fn apply_actor_effect(&mut self, effect: ActorEffect) -> PlayerResult<bool>;
    fn doorway_crossing(
        &mut self,
        effect: EffectId,
        owner: SeatId,
        hero: SeatId,
        level: LevelId,
        position: Position,
    ) -> PlayerResult<Option<Landing>>;
    fn answer_telephone(
        &mut self,
        effect: EffectId,
        hero: SeatId,
        level: LevelId,
    ) -> PlayerResult<()>;
    /// Exact indexed direction-mask row, including neutral at zero. Inputs are
    /// normalized before indexing; diagonals are not reconstructed by OR.
    fn control_remap(&self, selection: u8) -> PlayerResult<[u8; 16]>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub enum PlayerMode {
    #[default]
    Grounded,
    Jumping,
    Flying,
    Skating,
    Swimming,
    Hurt,
    Squished,
    Sleeping,
    Whirlwind,
    Falling,
    AwaitingFloor,
    Dying,
    MoleCaptured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum HeroServiceKind {
    Lemonade,
    HotTub,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum ServiceStage {
    Approach,
    Windup,
    Takeoff,
    Descend,
    Splash,
    Bathing,
    Exit,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct HeroService {
    pub actor: ActorId,
    pub kind: HeroServiceKind,
    pub position: Position,
    pub stage: ServiceStage,
    pub ticks: u16,
    /// Physical approach side, not a two-player seat/archetype branch.
    pub approach_left: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Randomization {
    pub owner: SeatId,
    pub steps_left: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PlayerRuntime {
    pub mode: PlayerMode,
    pub equipped: Option<PresentKind>,
    pub active_map_epoch: u32,
    pub authored_motion: AuthoredMotion,
    pub animation: AnimationPlayback,
    pub facing: u8,
    pub mode_ticks: u16,
    pub invulnerable_ticks: u16,
    pub sleep_ticks: u16,
    pub burps_left: u16,
    pub burp_delay: u16,
    pub rain_left: u8,
    pub rain_delay: u8,
    pub shot_delay: u8,
    pub remap: [u8; 16],
    pub checkpoint: Option<Landing>,
    pub previous_input: PlayerInput,
    pub effective_input: PlayerInput,
    pub last_animation_tick: Option<u64>,
    pub locomotion_locked: bool,
    pub fall_requested: bool,
    pub fall_token: Option<u32>,
    pub ui_service: Option<ActorId>,
    pub ui_offer: Option<u32>,
    pub ui_intent: Option<UiCommand>,
    pub blocked_axes: u8,
    pub service: Option<HeroService>,
    pub contacts: crate::contact::SeatContacts,
    pub lemonade_rewarded: bool,
}
impl Default for PlayerRuntime {
    fn default() -> Self {
        Self {
            mode: PlayerMode::Grounded,
            equipped: None,
            active_map_epoch: 0,
            authored_motion: AuthoredMotion::default(),
            animation: AnimationPlayback::default(),
            facing: 1,
            mode_ticks: 0,
            invulnerable_ticks: 0,
            sleep_ticks: 0,
            burps_left: 0,
            burp_delay: 0,
            rain_left: 0,
            rain_delay: 0,
            shot_delay: 0,
            remap: IDENTITY_REMAP,
            checkpoint: None,
            previous_input: PlayerInput::default(),
            effective_input: PlayerInput::default(),
            last_animation_tick: None,
            locomotion_locked: false,
            fall_requested: false,
            fall_token: None,
            ui_service: None,
            ui_offer: None,
            ui_intent: None,
            blocked_axes: 0,
            service: None,
            lemonade_rewarded: false,
            contacts: crate::contact::SeatContacts::default(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum UiCommand {
    OpenPresent(u8),
    Drop(u8),
    Purchase { offer: u32, service: ActorId },
    Identify { slot: u8, service: ActorId },
    DonateLife(SeatId),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiIntent {
    OpenInventory,
    OpenMail(ActorId),
    OpenIdentify(ActorId),
    OpenMap,
    Close,
    Select(u8),
    SelectOffer(u32),
    Activate,
    Drop,
    Pause(bool),
    DonateLifeTo(SeatId),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageStyle {
    Hurt,
    Squish,
    Direct,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageSource {
    Actor(ActorId),
    Effect(EffectId),
    Terrain,
    Present(PresentKind),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageRequest {
    pub source: DamageSource,
    pub amount: u8,
    pub style: DamageStyle,
    pub origin: Position,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusRequest {
    Hula(u8),
    Remap { ticks: u16, directions: [u8; 16] },
    Sleep(u16),
    Whirlwind { ticks: u16, position: Position },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct FallCohort {
    pub token: u32,
    pub seats: SeatMask,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentReceipt {
    pub requested: PresentKind,
    pub applied: PresentKind,
    pub effect: Option<EffectId>,
}

/// A prepared arrival is a linear, local transaction token. It contains no
/// borrowed geometry and cannot be serialized across an intervening world step.
#[derive(Debug)]
pub(crate) struct PreparedArrivals {
    entries: [Option<PreparedArrival>; SEATS],
    policy: ArrivalPolicy,
    fall_slot: Option<usize>,
}
#[derive(Debug, Clone, Copy)]
struct PreparedArrival {
    seat: SeatId,
    landing: Landing,
    terrain: [[u8; 5]; 5],
    count: u32,
    health: Option<u8>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArrivalPolicy {
    Setup { lives: u8 },
    Relocate,
    Fall { token: u32 },
    Revive { lives: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PlayerServices {
    pub runtime: [PlayerRuntime; SEATS],
    pub resources: ResourceState,
    pub fall_cohorts: [Option<FallCohort>; SEATS],
    pub randomization: Option<Randomization>,
    pub audio: AudioQueue,
    last_player_tick: Option<u64>,
    last_movement_tick: Option<u64>,
    pub(crate) last_input_tick: Option<u64>,
    last_effect_tick: Option<u64>,
    donation_tick: Option<u64>,
    donated_mask: u8,
}
impl PlayerServices {
    pub fn new(seed: u64, limits: ResourceLimits) -> PlayerResult<Self> {
        Ok(Self {
            runtime: std::array::from_fn(|_| PlayerRuntime::default()),
            resources: ResourceState::new(seed, limits)?,
            fall_cohorts: [None; SEATS],
            randomization: None,
            audio: AudioQueue::default(),
            last_player_tick: None,
            last_movement_tick: None,
            last_input_tick: None,
            last_effect_tick: None,
            donation_tick: None,
            donated_mask: 0,
        })
    }
    pub fn validate(&self, coop: &CoopState) -> PlayerResult<()> {
        coop.validate()
            .map_err(|_| PlayerError::Invalid("canonical seat state"))?;
        self.resources.validate(coop)?;
        self.audio.validate()?;
        if self.donated_mask & !15 != 0 {
            return Err(PlayerError::Invalid("donation mask"));
        }
        if self.randomization.is_some_and(|state| {
            state.steps_left == 0
                || state.steps_left > 60
                || !coop.config().participants.contains(state.owner)
        }) {
            return Err(PlayerError::Invalid("randomizer continuation"));
        }
        let mut falling = 0;
        for (index, runtime) in self.runtime.iter().enumerate() {
            let player = &coop.players[index];
            runtime.contacts.validate(player.seat, self)?;
            if runtime.facing > 3
                || runtime.authored_motion.axes & !3 != 0
                || runtime.blocked_axes & !3 != 0
                || runtime.equipped.is_some_and(|kind| !kind.equipped())
                || !valid_remap(runtime.remap)
                || runtime.animation.descriptor > 120
                || (!runtime.animation.started && runtime.animation.held)
            {
                return Err(PlayerError::Invalid("typed player motion/status"));
            }
            if let Some(service) = runtime.service {
                service
                    .position
                    .validate()
                    .map_err(|_| PlayerError::Invalid("hero service position"))?;
                if !service.actor.valid() || player.life != LifeState::Alive {
                    return Err(PlayerError::Invalid("hero service actor/life"));
                }
            }
            if runtime.equipped.is_some() != player.active_present.is_some() {
                return Err(PlayerError::Invalid("equipped present ownership mismatch"));
            }
            if let Some(checkpoint) = runtime.checkpoint {
                checkpoint
                    .level
                    .validate()
                    .map_err(|_| PlayerError::Invalid("checkpoint level"))?;
                checkpoint
                    .position
                    .validate()
                    .map_err(|_| PlayerError::Invalid("checkpoint position"))?;
            }
            if let Some(token) = runtime.fall_token
                && !self
                    .fall_cohorts
                    .iter()
                    .flatten()
                    .any(|cohort| cohort.token == token && cohort.seats.contains(player.seat))
            {
                return Err(PlayerError::Invalid("orphan fall token"));
            }
            if matches!(
                runtime.mode,
                PlayerMode::Falling | PlayerMode::AwaitingFloor
            ) != runtime.fall_token.is_some()
                || (runtime.fall_token.is_some() && player.counters.fall != FallState::Descending)
            {
                return Err(PlayerError::Invalid("fall state ownership"));
            }
            if player.life == LifeState::Disabled && (runtime != &PlayerRuntime::default()) {
                return Err(PlayerError::Invalid("disabled seat has simulation state"));
            }
            if player.life == LifeState::Alive && runtime.checkpoint.is_none() {
                return Err(PlayerError::Invalid("placed hero lacks a checkpoint"));
            }
            if let Some(command) = runtime.ui_intent {
                validate_ui_command(command)?;
            }
        }
        for (index, cohort) in self.fall_cohorts.iter().enumerate() {
            if let Some(cohort) = cohort {
                cohort
                    .seats
                    .validate()
                    .map_err(|_| PlayerError::Invalid("fall cohort mask"))?;
                if cohort.token == 0
                    || cohort.seats == SeatMask::NONE
                    || falling & cohort.seats.bits() != 0
                    || self.fall_cohorts[..index]
                        .iter()
                        .flatten()
                        .any(|other| other.token == cohort.token)
                {
                    return Err(PlayerError::Invalid("duplicate fall cohort"));
                }
                for seat in cohort.seats.iter() {
                    if self.runtime[seat.index()].fall_token != Some(cohort.token)
                        || coop.players[seat.index()].life != LifeState::Alive
                    {
                        return Err(PlayerError::Invalid("fall cohort member"));
                    }
                }
                falling |= cohort.seats.bits();
            }
        }
        Ok(())
    }
    /// Original new-session supply: four bonus HiTops for a solo hero, otherwise
    /// two each. Course-authored inventories take a separate setup path.
    pub fn initialize_stock_inventory(&self, coop: &mut CoopState) -> PlayerResult<()> {
        let participants = coop.config().participants;
        for seat in participants.iter() {
            let player = &coop.players[seat.index()];
            if player.life != LifeState::Unplaced || player.inventory.iter().any(Option::is_some) {
                return Err(PlayerError::Rejected(
                    "initial inventory already supplied or hero placed",
                ));
            }
        }
        let count = if participants.bits().count_ones() == 1 {
            4
        } else {
            2
        };
        let present = self
            .resources
            .presents
            .wrapper_for(PresentKind::BonusHiTops);
        for seat in participants.iter() {
            coop.players[seat.index()].inventory[..count].fill(Some(present));
        }
        Ok(())
    }
    /// Prepare one unplaced hero against frozen geometry, then publish only the
    /// already-checked state. Aggregate loaders use the cohort API below.
    pub fn setup_seat(
        &mut self,
        coop: &mut CoopState,
        seat: SeatId,
        landing: Landing,
        lives: u8,
        world: &impl PlayerGeometry,
    ) -> PlayerResult<()> {
        seat.validate()
            .map_err(|_| PlayerError::Invalid("setup seat"))?;
        let mut destinations = [None; SEATS];
        destinations[seat.index()] = Some(landing);
        let prepared =
            self.prepare_arrivals(coop, destinations, ArrivalPolicy::Setup { lives }, world)?;
        self.commit_arrivals(coop, prepared);
        Ok(())
    }
    pub(crate) fn prepare_arrivals(
        &self,
        coop: &CoopState,
        destinations: [Option<Landing>; SEATS],
        policy: ArrivalPolicy,
        world: &impl PlayerGeometry,
    ) -> PlayerResult<PreparedArrivals> {
        let bits = destinations
            .iter()
            .enumerate()
            .fold(0_u8, |bits, (index, landing)| {
                bits | (u8::from(landing.is_some()) << index)
            });
        if bits == 0 {
            return Err(PlayerError::Rejected("empty arrival cohort"));
        }
        if matches!(policy, ArrivalPolicy::Setup { lives } | ArrivalPolicy::Revive { lives } if lives > 127)
        {
            return Err(PlayerError::Invalid("arrival life supply"));
        }
        let fall_slot = if let ArrivalPolicy::Fall { token } = policy {
            let slot = self
                .fall_cohorts
                .iter()
                .position(|cohort| {
                    cohort
                        .is_some_and(|cohort| cohort.token == token && cohort.seats.bits() == bits)
                })
                .ok_or(PlayerError::Rejected("partial or unknown fall cohort"))?;
            if !self.fall_ready(token) {
                return Err(PlayerError::Rejected("fall cohort not ready"));
            }
            Some(slot)
        } else {
            None
        };
        let mut entries = [None; SEATS];
        for seat in SeatId::ALL {
            let Some(landing) = destinations[seat.index()] else {
                continue;
            };
            let player = &coop.players[seat.index()];
            if !coop.config().participants.contains(seat) {
                return Err(PlayerError::Rejected("arrival names disabled hero"));
            }
            let eligible = match policy {
                ArrivalPolicy::Setup { .. } => player.life == LifeState::Unplaced,
                ArrivalPolicy::Relocate => {
                    player.life == LifeState::Alive
                        && self.runtime[seat.index()].fall_token.is_none()
                }
                ArrivalPolicy::Fall { .. } => player.life == LifeState::Alive,
                ArrivalPolicy::Revive { .. } => {
                    player.life == LifeState::Eliminated
                        || player.life == LifeState::Alive
                            && self.runtime[seat.index()].mode == PlayerMode::Dying
                }
            };
            if !eligible {
                return Err(PlayerError::Rejected("arrival hero state"));
            }
            validate_landing(world, seat, player.character.archetype, landing)?;
            let count = coop.levels[landing.level.index()].arrivals[seat.index()]
                .checked_add(1)
                .ok_or(PlayerError::Invalid("arrival counter"))?;
            let terrain = sample_terrain(world, landing)?;
            let health = if matches!(
                policy,
                ArrivalPolicy::Setup { .. } | ArrivalPolicy::Revive { .. }
            ) {
                Some(maximum_health(player)?)
            } else {
                None
            };
            entries[seat.index()] = Some(PreparedArrival {
                seat,
                landing,
                terrain,
                count,
                health,
            });
        }
        Ok(PreparedArrivals {
            entries,
            policy,
            fall_slot,
        })
    }
    /// Must be called immediately after preflight, with no intervening mutation
    /// of the aggregate. No geometry/resource call can fail in this commit.
    pub(crate) fn commit_arrivals(&mut self, coop: &mut CoopState, prepared: PreparedArrivals) {
        for entry in prepared.entries.into_iter().flatten() {
            let player = &mut coop.players[entry.seat.index()];
            let runtime = &mut self.runtime[entry.seat.index()];
            let landing = entry.landing;
            let falling = matches!(prepared.policy, ArrivalPolicy::Fall { .. });
            player.life = LifeState::Alive;
            player.level = Some(landing.level);
            player.motion = Motion {
                position: if falling {
                    Position {
                        elevation: 70 * 256,
                        ..landing.position
                    }
                } else {
                    landing.position
                },
                acceleration: if falling || landing.position.elevation > 0 {
                    [0, 0, -32]
                } else {
                    [0; 3]
                },
                ..Motion::default()
            };
            player.terrain = entry.terrain;
            player.spectating = None;
            player.counters.fall = if falling {
                FallState::Landing
            } else {
                FallState::None
            };
            player.ui.overlay = Overlay::Closed;
            player.ui.choice = None;
            runtime.mode = if falling || landing.position.elevation > 0 {
                PlayerMode::Jumping
            } else {
                PlayerMode::Grounded
            };
            runtime.fall_token = None;
            runtime.locomotion_locked = false;
            runtime.fall_requested = false;
            runtime.checkpoint = Some(Landing {
                position: Position {
                    elevation: 0,
                    ..landing.position
                },
                ..landing
            });
            runtime.ui_service = None;
            runtime.ui_offer = None;
            runtime.ui_intent = None;
            runtime.service = None;
            runtime.contacts = Default::default();
            if let Some(health) = entry.health {
                player.health = health;
            }
            match prepared.policy {
                ArrivalPolicy::Setup { lives } => player.lives = lives,
                ArrivalPolicy::Revive { lives } => {
                    player.lives = lives;
                    player.active_present = None;
                    player.counters = Default::default();
                    let lemonade_rewarded = runtime.lemonade_rewarded;
                    *runtime = PlayerRuntime {
                        checkpoint: Some(landing),
                        invulnerable_ticks: 60,
                        lemonade_rewarded,
                        ..PlayerRuntime::default()
                    };
                }
                ArrivalPolicy::Relocate | ArrivalPolicy::Fall { .. } => {
                    runtime.invulnerable_ticks = 30
                }
            }
            coop.levels[landing.level.index()].arrivals[entry.seat.index()] = entry.count;
        }
        if let Some(slot) = prepared.fall_slot {
            self.fall_cohorts[slot] = None;
        }
    }
    pub fn refresh_terrain(
        &self,
        coop: &mut CoopState,
        seat: SeatId,
        world: &impl PlayerGeometry,
    ) -> PlayerResult<()> {
        let player = &mut coop.players[seat.index()];
        let level = player
            .level
            .ok_or(PlayerError::Rejected("unplaced terrain cache"))?;
        player.terrain = sample_terrain(
            world,
            Landing {
                level,
                position: player.motion.position,
            },
        )?;
        Ok(())
    }
    pub fn animation_axes(&self, seat: SeatId) -> PlayerResult<u8> {
        seat.validate()
            .map_err(|_| PlayerError::Invalid("animation axes seat"))?;
        Ok(self.runtime[seat.index()].authored_motion.axes)
    }
    /// Admit a simulation-animation result once. Only a newly authored frame
    /// callback moves authored axes; held/redraw-only samples never replay it.
    pub fn apply_animation(
        &mut self,
        coop: &mut CoopState,
        seat: SeatId,
        tick: u64,
        step: PlayerAnimationStep,
        scale: VoluntaryScale,
        world: &impl PlayerGeometry,
    ) -> PlayerResult<bool> {
        seat.validate()
            .map_err(|_| PlayerError::Invalid("animation seat"))?;
        let runtime = &self.runtime[seat.index()];
        if runtime.last_animation_tick == Some(tick) {
            return Ok(false);
        }
        if runtime.last_animation_tick.is_some_and(|last| tick < last) {
            return Err(PlayerError::Invalid("animation tick moved backwards"));
        }
        if step.descriptor > 120
            || coop.players[seat.index()].animation.descriptor != step.descriptor
        {
            return Err(PlayerError::Invalid(
                "animation admission descriptor/motion",
            ));
        }
        let player = &mut coop.players[seat.index()];
        let runtime = &mut self.runtime[seat.index()];
        if let Some(motion) = step.motion {
            if player.life == LifeState::Alive
                && !runtime.locomotion_locked
                && !matches!(
                    runtime.mode,
                    PlayerMode::Falling
                        | PlayerMode::AwaitingFloor
                        | PlayerMode::Dying
                        | PlayerMode::Whirlwind
                )
            {
                let mut delta = [0; 2];
                for (axis, value) in delta.iter_mut().enumerate() {
                    if runtime.authored_motion.axes & (1 << axis) != 0 {
                        *value = player.motion.velocity[axis]
                            .checked_mul(i32::from(motion[axis]))
                            .ok_or(PlayerError::Invalid("authored motion overflow"))?;
                        if ordinary_voluntary(player, runtime) {
                            *value = scale.delta(*value);
                        }
                    }
                }
                apply_displacement(world, seat, player, runtime, delta)?;
            }
            runtime.authored_motion.multiplier = motion;
        }
        player.animation.sequence_index = step.sequence_index;
        player.animation.ticks_left = step.ticks_left;
        player.animation.flip_horizontal = step.flip_horizontal;
        runtime.animation = AnimationPlayback {
            descriptor: step.descriptor,
            started: true,
            held: step.held,
        };
        runtime.last_animation_tick = Some(tick);
        Ok(true)
    }
    /// Continuous velocity/acceleration is independent of animation callback
    /// cadence and advances on both original even phases, never on redraw.
    pub fn advance_movement(
        &mut self,
        coop: &mut CoopState,
        tick: u64,
        scales: [VoluntaryScale; SEATS],
        world: &impl PlayerGeometry,
    ) -> PlayerResult<bool> {
        if self.last_movement_tick == Some(tick) {
            return Ok(false);
        }
        if self.last_movement_tick.is_some_and(|last| tick < last) {
            return Err(PlayerError::Invalid("movement tick moved backwards"));
        }
        for seat in SeatId::ALL {
            let player = &mut coop.players[seat.index()];
            let runtime = &mut self.runtime[seat.index()];
            if player.life != LifeState::Alive || runtime.mode == PlayerMode::AwaitingFloor {
                continue;
            }
            runtime.blocked_axes = 0;
            if runtime.mode == PlayerMode::Falling {
                integrate_vertical(&mut player.motion)?;
                if player.motion.position.elevation <= -224 * 256 {
                    runtime.mode = PlayerMode::AwaitingFloor;
                    player.motion.velocity = [0; 3];
                    player.motion.acceleration = [0; 3];
                }
                continue;
            }
            if runtime.locomotion_locked || runtime.mode == PlayerMode::Whirlwind {
                continue;
            }
            let mut delta = [0; 2];
            for (axis, value) in delta.iter_mut().enumerate() {
                if runtime.authored_motion.axes & (1 << axis) == 0 {
                    player.motion.velocity[axis] = checked_add(
                        player.motion.velocity[axis],
                        player.motion.acceleration[axis],
                    )?
                    .clamp(-64 * 256, 64 * 256);
                    *value = player.motion.velocity[axis];
                    if ordinary_voluntary(player, runtime) {
                        *value = scales[seat.index()].delta(*value);
                    }
                }
            }
            apply_displacement(world, seat, player, runtime, delta)?;
            if player.motion.position.elevation > 0
                || player.motion.velocity[2] != 0
                || player.motion.acceleration[2] != 0
            {
                integrate_vertical(&mut player.motion)?;
            }
            let level = player.level.expect("alive movement level");
            if runtime.mode != PlayerMode::Dying
                && player.motion.position.elevation <= 0
                && world.terrain(
                    level,
                    player.motion.position.x >> 8,
                    player.motion.position.y >> 8,
                )? & 0xf0
                    <= 0x50
            {
                player.motion.position.elevation = 0;
                player.motion.velocity[2] = 0;
                player.motion.acceleration[2] = 0;
                player.counters.fall = FallState::None;
                if matches!(
                    runtime.mode,
                    PlayerMode::Jumping | PlayerMode::Flying | PlayerMode::Swimming
                ) {
                    runtime.mode = PlayerMode::Grounded;
                }
            }
            player
                .motion
                .position
                .validate()
                .map_err(|_| PlayerError::Invalid("hero motion bound"))?;
        }
        for effect in self.resources.effects_mut() {
            if matches!(
                effect.kind,
                EffectKind::Tomato
                    | EffectKind::Slingshot
                    | EffectKind::RainTomato
                    | EffectKind::ControlRemap
                    | EffectKind::StolenPresent
            ) {
                for axis in 0..3 {
                    effect.velocity[axis] =
                        checked_add(effect.velocity[axis], effect.acceleration[axis])?;
                }
                effect.position.x = checked_add(effect.position.x, effect.velocity[0])?;
                effect.position.y = checked_add(effect.position.y, effect.velocity[1])?;
                effect.position.elevation =
                    checked_add(effect.position.elevation, effect.velocity[2])?;
                effect
                    .position
                    .validate()
                    .map_err(|_| PlayerError::Invalid("effect position overflow"))?;
            }
        }
        self.last_movement_tick = Some(tick);
        Ok(true)
    }
    pub fn enter_transition(&mut self, coop: &mut CoopState, seats: SeatMask) -> PlayerResult<()> {
        seats
            .validate()
            .map_err(|_| PlayerError::Invalid("transition mask"))?;
        for seat in seats.iter() {
            if coop.players[seat.index()].life != LifeState::Alive {
                return Err(PlayerError::Rejected("transition hero is not alive"));
            }
        }
        for seat in seats.iter() {
            self.runtime[seat.index()].locomotion_locked = true;
            if !matches!(
                self.runtime[seat.index()].mode,
                PlayerMode::Falling | PlayerMode::AwaitingFloor
            ) {
                coop.players[seat.index()].motion.velocity = [0; 3];
                coop.players[seat.index()].motion.acceleration = [0; 3];
            }
        }
        Ok(())
    }
    pub fn leave_transition(&mut self, seats: SeatMask) -> PlayerResult<()> {
        seats
            .validate()
            .map_err(|_| PlayerError::Invalid("transition mask"))?;
        for seat in seats.iter() {
            self.runtime[seat.index()].locomotion_locked = false;
        }
        Ok(())
    }
    pub fn try_ground_bump(
        &mut self,
        coop: &mut CoopState,
        seat: SeatId,
        destination: Landing,
        world: &impl PlayerGeometry,
    ) -> PlayerResult<bool> {
        seat.validate()
            .map_err(|_| PlayerError::Invalid("bump seat"))?;
        let player = &coop.players[seat.index()];
        let runtime = &self.runtime[seat.index()];
        if !self.can_receive_effect(coop, seat)
            || runtime.mode != PlayerMode::Grounded
            || runtime.locomotion_locked
            || runtime.service.is_some()
            || player.ui.overlay != Overlay::Closed
            || player.counters.respawn_suppression_ticks != 0
            || player.counters.hula_dance_ticks != 0
            || player.level != Some(destination.level)
            || destination.position.elevation != 0
        {
            return Ok(false);
        }
        let dx = i64::from(destination.position.x) - i64::from(player.motion.position.x);
        let dy = i64::from(destination.position.y) - i64::from(player.motion.position.y);
        if dx * dx + dy * dy > i64::from(12 * 256).pow(2) {
            return Ok(false);
        }
        match validate_landing(world, seat, player.character.archetype, destination) {
            Ok(()) => {}
            Err(PlayerError::Geometry("unsafe landing")) => return Ok(false),
            Err(error) => return Err(error),
        }
        let terrain = sample_terrain(world, destination)?;
        coop.players[seat.index()].motion.position = destination.position;
        coop.players[seat.index()].terrain = terrain;
        Ok(true)
    }
    pub fn ui(&mut self, coop: &mut CoopState, seat: SeatId, intent: UiIntent) -> PlayerResult<()> {
        seat.validate()
            .map_err(|_| PlayerError::Invalid("UI seat"))?;
        let participants = coop.config().participants;
        let player = &mut coop.players[seat.index()];
        let runtime = &mut self.runtime[seat.index()];
        if !participants.contains(seat) {
            return Err(PlayerError::Rejected("disabled UI seat"));
        }
        if let UiIntent::Pause(request) = intent {
            player.ui.pause_requested = request;
            return Ok(());
        }
        if player.life != LifeState::Alive && !matches!(intent, UiIntent::DonateLifeTo(_)) {
            return Err(PlayerError::Rejected("UI hero is not alive"));
        }
        match intent {
            UiIntent::OpenInventory => {
                player.ui.overlay = Overlay::Inventory;
                runtime.ui_service = None;
            }
            UiIntent::OpenMail(service) => {
                player.ui.overlay = Overlay::MailOrder;
                runtime.ui_service = Some(service);
            }
            UiIntent::OpenIdentify(service) => {
                player.ui.overlay = Overlay::Identify;
                runtime.ui_service = Some(service);
            }
            UiIntent::OpenMap => {
                player.ui.overlay = Overlay::Map;
                runtime.ui_service = None;
            }
            UiIntent::Close => {
                player.ui.overlay = Overlay::Closed;
                runtime.ui_service = None;
                runtime.ui_intent = None;
            }
            UiIntent::Select(slot) => {
                if slot >= 16 {
                    return Err(PlayerError::Rejected("UI inventory selection"));
                }
                player.ui.selected_slot = slot;
            }
            UiIntent::SelectOffer(offer) => {
                runtime.ui_offer = Some(offer);
            }
            UiIntent::Activate => {
                if player.ui.action_release_latch || runtime.ui_intent.is_some() {
                    return Err(PlayerError::Rejected("release UI action before repeating"));
                }
                runtime.ui_intent = Some(match player.ui.overlay {
                    Overlay::Inventory => UiCommand::OpenPresent(player.ui.selected_slot),
                    Overlay::MailOrder => UiCommand::Purchase {
                        offer: runtime
                            .ui_offer
                            .ok_or(PlayerError::Rejected("mail offer not selected"))?,
                        service: runtime
                            .ui_service
                            .ok_or(PlayerError::Rejected("mail service not selected"))?,
                    },
                    Overlay::Identify => UiCommand::Identify {
                        slot: player.ui.selected_slot,
                        service: runtime
                            .ui_service
                            .ok_or(PlayerError::Rejected("identification service not selected"))?,
                    },
                    _ => return Err(PlayerError::Rejected("overlay has no activate action")),
                });
                player.ui.action_release_latch = true;
            }
            UiIntent::Drop => {
                if player.ui.overlay != Overlay::Inventory || runtime.ui_intent.is_some() {
                    return Err(PlayerError::Rejected("drop requires inventory"));
                }
                runtime.ui_intent = Some(UiCommand::Drop(player.ui.selected_slot));
            }
            UiIntent::DonateLifeTo(recipient) => {
                recipient
                    .validate()
                    .map_err(|_| PlayerError::Invalid("life recipient"))?;
                if recipient == seat || !participants.contains(recipient) {
                    return Err(PlayerError::Rejected("invalid life recipient"));
                }
                player.ui.life_recipient = Some(recipient);
                runtime.ui_intent = Some(UiCommand::DonateLife(recipient));
            }
            UiIntent::Pause(_) => unreachable!("handled above"),
        }
        Ok(())
    }
    pub fn release_ui_action(&mut self, coop: &mut CoopState, seat: SeatId) {
        coop.players[seat.index()].ui.action_release_latch = false;
    }
    pub fn take_ui_command(&mut self, seat: SeatId) -> Option<UiCommand> {
        self.runtime[seat.index()].ui_intent.take()
    }
    pub fn pause_agreed(&self, coop: &CoopState) -> bool {
        coop.config()
            .participants
            .iter()
            .all(|seat| coop.players[seat.index()].ui.pause_requested)
    }
    /// Automatic friendly contacts start hero-owned finite pose machines. Entry
    /// side is geometric; seat two/three never fall through an Earl-only branch.
    pub fn service_contact(
        &mut self,
        coop: &mut CoopState,
        seat: SeatId,
        actor: ActorId,
        kind: HeroServiceKind,
        position: Position,
    ) -> PlayerResult<bool> {
        seat.validate()
            .map_err(|_| PlayerError::Invalid("service hero"))?;
        position
            .validate()
            .map_err(|_| PlayerError::Invalid("service position"))?;
        if !actor.valid() {
            return Err(PlayerError::Invalid("service actor"));
        }
        if !self.can_receive_effect(coop, seat) {
            return Ok(false);
        }
        let player = &mut coop.players[seat.index()];
        let runtime = &mut self.runtime[seat.index()];
        if let Some(active) = runtime.service {
            return Ok(active.actor == actor && active.kind == kind);
        }
        let dx = i64::from(position.x) - i64::from(player.motion.position.x);
        let dy = i64::from(player.motion.position.y) - i64::from(position.y);
        let entering = match kind {
            HeroServiceKind::Lemonade => {
                runtime.previous_input.directions & 1 != 0
                    && dy > 0
                    && dy < 32 * 256
                    && dx.abs() < 40 * 256
            }
            HeroServiceKind::HotTub => {
                runtime.equipped != Some(PresentKind::SpringShoes)
                    && (dy + 4 * 256).abs() < 5 * 256
                    && ((dx > 34 * 256 && runtime.previous_input.directions & 8 != 0)
                        || (dx < -34 * 256 && runtime.previous_input.directions & 4 != 0))
            }
        };
        if !entering {
            return Ok(false);
        }
        runtime.service = Some(HeroService {
            actor,
            kind,
            position,
            stage: if kind == HeroServiceKind::HotTub {
                ServiceStage::Windup
            } else {
                ServiceStage::Approach
            },
            ticks: 0,
            approach_left: dx > 0,
        });
        player.motion.velocity = [0; 3];
        player.motion.acceleration = [0; 3];
        player.animation.descriptor = if kind == HeroServiceKind::Lemonade {
            118
        } else if dx > 0 {
            31
        } else {
            30
        };
        Ok(true)
    }
    pub(crate) fn advance_automatic_service(
        &mut self,
        coop: &mut CoopState,
        seat: SeatId,
        actor: ActorId,
        kind: HeroServiceKind,
        position: Position,
        world: &impl PlayerGeometry,
    ) -> PlayerResult<(bool, Option<ActorEffect>)> {
        let existing = self.runtime[seat.index()].service;
        if !self.service_contact(coop, seat, actor, kind, position)? {
            return Ok((false, None));
        }
        if existing.is_none() {
            return Ok((true, None));
        }
        self.audio.reserve(2)?;
        let was_rewarded = self.runtime[seat.index()].lemonade_rewarded;
        let previous_stage = existing.expect("existing service").stage;
        let input = self.runtime[seat.index()].previous_input;
        let mut effect = None;
        let accepted = tick_service(
            &mut coop.players[seat.index()],
            &mut self.runtime[seat.index()],
            input,
            world,
            &mut effect,
        )?;
        let runtime = &self.runtime[seat.index()];
        if kind == HeroServiceKind::Lemonade && runtime.service.is_none() {
            self.audio.enqueue(
                seat,
                AudioCue::Pcm {
                    sound: PcmSound::Burp,
                    priority: 50,
                    mode: 0,
                },
            )?;
            if !was_rewarded && runtime.lemonade_rewarded {
                self.audio.enqueue(
                    seat,
                    AudioCue::Pcm {
                        sound: PcmSound::ExtraLife,
                        priority: 87,
                        mode: 0,
                    },
                )?;
            }
        }
        if kind == HeroServiceKind::HotTub
            && previous_stage == ServiceStage::Descend
            && runtime
                .service
                .is_some_and(|service| service.stage == ServiceStage::Splash)
        {
            self.audio.enqueue(
                seat,
                AudioCue::Pcm {
                    sound: PcmSound::Splash,
                    priority: 50,
                    mode: 0,
                },
            )?;
        }
        Ok((accepted, effect))
    }
    pub(crate) fn release_forced_pose(&mut self, coop: &mut CoopState, seat: SeatId) {
        let player = &mut coop.players[seat.index()];
        let runtime = &mut self.runtime[seat.index()];
        if matches!(
            runtime.mode,
            PlayerMode::Grounded | PlayerMode::Swimming | PlayerMode::MoleCaptured
        ) {
            if runtime.mode == PlayerMode::MoleCaptured {
                runtime.mode = PlayerMode::Grounded;
            }
            select_motion_pose(
                player,
                runtime,
                PlayerInput::default(),
                player.terrain[2][2] & 0xf0,
            );
        }
    }
    pub fn can_receive_effect(&self, coop: &CoopState, seat: SeatId) -> bool {
        let player = &coop.players[seat.index()];
        let runtime = &self.runtime[seat.index()];
        player.life == LifeState::Alive
            && runtime.invulnerable_ticks == 0
            && !matches!(
                runtime.mode,
                PlayerMode::Dying
                    | PlayerMode::Falling
                    | PlayerMode::AwaitingFloor
                    | PlayerMode::Hurt
                    | PlayerMode::Squished
                    | PlayerMode::Whirlwind
            )
    }
    pub fn damage(
        &mut self,
        coop: &mut CoopState,
        seat: SeatId,
        request: DamageRequest,
        world: &impl PlayerGeometry,
    ) -> PlayerResult<bool> {
        seat.validate()
            .map_err(|_| PlayerError::Invalid("damage seat"))?;
        request
            .origin
            .validate()
            .map_err(|_| PlayerError::Invalid("damage origin"))?;
        if matches!(request.source, DamageSource::Actor(actor) if !actor.valid())
            || matches!(request.source, DamageSource::Effect(effect) if !effect.valid())
        {
            return Err(PlayerError::Invalid("damage source identity"));
        }
        if request.amount == 0 || !self.can_receive_effect(coop, seat) {
            return Ok(false);
        }
        let cue = match request.style {
            DamageStyle::Direct => AudioCue::Psg {
                command: 18,
                priority: 40,
            },
            DamageStyle::Squish => AudioCue::Pcm {
                sound: PcmSound::Squish,
                priority: 50,
                mode: 0,
            },
            DamageStyle::Hurt => AudioCue::Pcm {
                sound: PcmSound::youch(coop.players[seat.index()].character.archetype),
                priority: 50,
                mode: 0,
            },
        };
        self.audio.reserve(1)?;
        if request.style == DamageStyle::Direct {
            let player = &mut coop.players[seat.index()];
            player.health = player.health.saturating_sub(request.amount);
            self.audio.enqueue(seat, cue)?;
            return Ok(true);
        }
        if request.style == DamageStyle::Squish {
            let player = &mut coop.players[seat.index()];
            let runtime = &mut self.runtime[seat.index()];
            player.health = player.health.saturating_sub(request.amount);
            player.motion.velocity = [0; 3];
            player.motion.acceleration = [0; 3];
            player.animation.descriptor = 2;
            runtime.mode = PlayerMode::Squished;
            runtime.mode_ticks = 30;
            runtime.invulnerable_ticks = 30;
            runtime.authored_motion.axes = 3;
            reset_playback(player, runtime);
            self.audio.enqueue(seat, cue)?;
            return Ok(true);
        }
        let player = &coop.players[seat.index()];
        let mut destination = player.motion.position;
        let dx = i64::from(destination.x) - i64::from(request.origin.x);
        let dy = i64::from(destination.y) - i64::from(request.origin.y);
        let facing = if dx.abs() > 10 * 256 {
            destination.x = checked_add(destination.x, if dx > 0 { 16 * 256 } else { -16 * 256 })?;
            if dx > 0 { 3 } else { 2 }
        } else {
            destination.y = checked_add(destination.y, if dy > 0 { 16 * 256 } else { -16 * 256 })?;
            if dy > 0 { 1 } else { 0 }
        };
        let blocked = world.blocked(
            seat,
            player.character.archetype,
            player.level.expect("alive level"),
            destination,
        )?;
        let player = &mut coop.players[seat.index()];
        let runtime = &mut self.runtime[seat.index()];
        if !blocked {
            player.motion.position = destination;
        }
        player.motion.velocity = [0; 3];
        player.motion.acceleration = [0; 3];
        runtime.facing = facing;
        player.counters.pending_hurt_damage = request.amount;
        runtime.mode = PlayerMode::Hurt;
        runtime.mode_ticks = 12;
        player.animation.descriptor = 52 + facing;
        runtime.authored_motion.axes = 3;
        reset_playback(player, runtime);
        self.audio.enqueue(seat, cue)?;
        Ok(true)
    }
    pub fn status(
        &mut self,
        coop: &mut CoopState,
        seat: SeatId,
        status: StatusRequest,
    ) -> PlayerResult<bool> {
        seat.validate()
            .map_err(|_| PlayerError::Invalid("status seat"))?;
        if matches!(
            status,
            StatusRequest::Hula(0)
                | StatusRequest::Sleep(0)
                | StatusRequest::Remap { ticks: 0, .. }
                | StatusRequest::Whirlwind { ticks: 0, .. }
        ) {
            return Err(PlayerError::Rejected("zero-duration status"));
        }
        if let StatusRequest::Remap { directions, .. } = status
            && !valid_remap(directions)
        {
            return Err(PlayerError::Invalid(
                "direction remap must be a permutation",
            ));
        }
        if let StatusRequest::Whirlwind { position, .. } = status {
            position
                .validate()
                .map_err(|_| PlayerError::Invalid("whirlwind origin"))?;
        }
        if !self.can_receive_effect(coop, seat) {
            return Ok(false);
        }
        let player = &mut coop.players[seat.index()];
        let runtime = &mut self.runtime[seat.index()];
        match status {
            StatusRequest::Hula(ticks) => {
                player.counters.hula_dance_ticks = ticks;
                player.animation.descriptor = 89;
                runtime.authored_motion.axes = 3;
            }
            StatusRequest::Remap { ticks, directions } => {
                runtime.remap = directions;
                player.counters.controls_remap_ticks = ticks;
            }
            StatusRequest::Sleep(ticks) => {
                runtime.sleep_ticks = ticks;
                runtime.mode = PlayerMode::Sleeping;
                player.animation.descriptor = 117;
                runtime.authored_motion.axes = 0;
            }
            StatusRequest::Whirlwind { ticks, position } => {
                runtime.mode = PlayerMode::Whirlwind;
                runtime.mode_ticks = ticks;
                player.motion.position = position;
                player.animation.descriptor = 79;
                runtime.authored_motion.axes = 0;
            }
        }
        if !matches!(status, StatusRequest::Remap { .. }) {
            player.motion.velocity = [0; 3];
            player.motion.acceleration = [0; 3];
            reset_playback(player, runtime);
        }
        Ok(true)
    }
    /// Explicit donor/recipient, never XOR-one. Both ends participate at most once
    /// per tick, preventing a just-donated life from being forwarded or duplicated.
    pub fn donate_life(
        &mut self,
        coop: &mut CoopState,
        tick: u64,
        donor: SeatId,
        recipient: SeatId,
        world: &mut impl PlayerWorld,
    ) -> PlayerResult<()> {
        self.check_donation(tick, donor, recipient)?;
        let from = &coop.players[donor.index()];
        let to = &coop.players[recipient.index()];
        if from.life != LifeState::Alive || from.lives == 0 || to.life != LifeState::Eliminated {
            return Err(PlayerError::Rejected("life donor/recipient eligibility"));
        }
        let checkpoint = self.runtime[recipient.index()]
            .checkpoint
            .ok_or(PlayerError::Invalid("life recipient checkpoint"))?;
        let landing = world.respawn(recipient, checkpoint)?;
        let mut destinations = [None; SEATS];
        destinations[recipient.index()] = Some(landing);
        let prepared = self.prepare_arrivals(
            coop,
            destinations,
            ArrivalPolicy::Revive { lives: 0 },
            world,
        )?;
        self.audio.reserve(1)?;
        world.commit_arrival(recipient, landing)?;
        coop.players[donor.index()].lives -= 1;
        self.commit_arrivals(coop, prepared);
        self.audio.enqueue(
            recipient,
            AudioCue::Pcm {
                sound: PcmSound::ExtraLife,
                priority: 87,
                mode: 0,
            },
        )?;
        self.commit_donation(tick, donor, recipient);
        Ok(())
    }
    pub fn donate_health(
        &mut self,
        coop: &mut CoopState,
        tick: u64,
        donor: SeatId,
        recipient: SeatId,
        amount: u8,
    ) -> PlayerResult<()> {
        self.check_donation(tick, donor, recipient)?;
        let from = &coop.players[donor.index()];
        let to = &coop.players[recipient.index()];
        let capacity = maximum_health(to)?.saturating_sub(to.health);
        if amount == 0
            || from.life != LifeState::Alive
            || to.life != LifeState::Alive
            || from.level != to.level
            || from.health <= amount
            || capacity < amount
            || (i64::from(from.motion.position.x) - i64::from(to.motion.position.x)).abs()
                >= 32 * 256
            || (i64::from(from.motion.position.y) - i64::from(to.motion.position.y)).abs()
                >= 8 * 256
        {
            return Err(PlayerError::Rejected("health donation range/resources"));
        }
        coop.players[donor.index()].health -= amount;
        coop.players[recipient.index()].health += amount;
        self.commit_donation(tick, donor, recipient);
        Ok(())
    }
    fn check_donation(&self, tick: u64, donor: SeatId, recipient: SeatId) -> PlayerResult<()> {
        donor
            .validate()
            .map_err(|_| PlayerError::Invalid("donor seat"))?;
        recipient
            .validate()
            .map_err(|_| PlayerError::Invalid("recipient seat"))?;
        if donor == recipient
            || self.donation_tick.is_some_and(|previous| previous > tick)
            || (self.donation_tick == Some(tick)
                && self.donated_mask & ((1 << donor.get()) | (1 << recipient.get())) != 0)
        {
            return Err(PlayerError::Rejected("duplicate/chained donation"));
        }
        Ok(())
    }
    fn commit_donation(&mut self, tick: u64, donor: SeatId, recipient: SeatId) {
        if self.donation_tick != Some(tick) {
            self.donated_mask = 0;
        }
        self.donation_tick = Some(tick);
        self.donated_mask |= (1 << donor.get()) | (1 << recipient.get());
    }
    pub fn pending_falls(&self) -> SeatMask {
        SeatMask::new(
            self.runtime
                .iter()
                .enumerate()
                .fold(0, |mask, (index, runtime)| {
                    mask | (u8::from(runtime.fall_requested) << index)
                }),
        )
        .expect("four seats")
    }
    pub fn begin_fall(&mut self, coop: &mut CoopState, cohort: FallCohort) -> PlayerResult<()> {
        cohort
            .seats
            .validate()
            .map_err(|_| PlayerError::Invalid("fall seats"))?;
        if cohort.token == 0
            || cohort.seats == SeatMask::NONE
            || self
                .fall_cohorts
                .iter()
                .flatten()
                .any(|existing| existing.token == cohort.token)
        {
            return Err(PlayerError::Rejected("fall token/mask"));
        }
        let slot = self
            .fall_cohorts
            .iter()
            .position(Option::is_none)
            .ok_or(PlayerError::Capacity("fall cohorts"))?;
        for seat in cohort.seats.iter() {
            if coop.players[seat.index()].life != LifeState::Alive
                || self.runtime[seat.index()].fall_token.is_some()
            {
                return Err(PlayerError::Rejected("fall cohort hero"));
            }
        }
        self.fall_cohorts[slot] = Some(cohort);
        for seat in cohort.seats.iter() {
            let runtime = &mut self.runtime[seat.index()];
            let player = &mut coop.players[seat.index()];
            runtime.mode = PlayerMode::Falling;
            runtime.fall_token = Some(cohort.token);
            runtime.fall_requested = false;
            player.counters.fall = FallState::Descending;
            player.ui.overlay = Overlay::Closed;
            player.motion.velocity = [0; 3];
            player.motion.acceleration = [0, 0, -128];
            runtime.authored_motion.axes = 0;
            player.animation.descriptor = 20 + runtime.facing;
        }
        Ok(())
    }
    pub fn fall_ready(&self, token: u32) -> bool {
        self.fall_cohorts
            .iter()
            .flatten()
            .find(|cohort| cohort.token == token)
            .is_some_and(|cohort| {
                cohort
                    .seats
                    .iter()
                    .all(|seat| self.runtime[seat.index()].mode == PlayerMode::AwaitingFloor)
            })
    }
    /// S2 supplies prepared destinations. Validate the complete cohort before any
    /// level pointer, arrival history, motion, or cached terrain changes.
    pub fn finish_fall(
        &mut self,
        coop: &mut CoopState,
        token: u32,
        destinations: [Option<Landing>; SEATS],
        world: &impl PlayerGeometry,
    ) -> PlayerResult<()> {
        let prepared =
            self.prepare_arrivals(coop, destinations, ArrivalPolicy::Fall { token }, world)?;
        self.commit_arrivals(coop, prepared);
        Ok(())
    }
    pub fn open_present(
        &mut self,
        coop: &mut CoopState,
        seat: SeatId,
        slot: u8,
        world: &mut impl PlayerWorld,
    ) -> PlayerResult<PresentReceipt> {
        seat.validate()
            .map_err(|_| PlayerError::Invalid("present seat"))?;
        let player = &coop.players[seat.index()];
        let wrapper = player
            .inventory
            .get(usize::from(slot))
            .copied()
            .flatten()
            .ok_or(PlayerError::Rejected("empty present slot"))?;
        if player.life != LifeState::Alive
            || matches!(
                self.runtime[seat.index()].mode,
                PlayerMode::Dying
                    | PlayerMode::Falling
                    | PlayerMode::AwaitingFloor
                    | PlayerMode::Whirlwind
            )
        {
            return Err(PlayerError::Rejected(
                "present hero is in a protected transition",
            ));
        }
        let level = player.level.expect("alive level");
        let position = player.motion.position;
        let terrain = world.terrain(level, position.x >> 8, position.y >> 8)? & 0xf0;
        let requested = self.resources.presents.kind(wrapper);
        // Roll locally first: rejection does not consume randomness or inventory.
        let mut rng = self.resources.effect_rng;
        let kind = if requested == PresentKind::Mystery {
            let mut resolved = None;
            for _ in 0..1024 {
                let candidate = world.choose_present(&mut rng)?;
                if candidate != PresentKind::Mystery {
                    resolved = Some(candidate);
                    break;
                }
            }
            resolved.ok_or(PlayerError::Rejected("mystery resolution exhausted"))?
        } else {
            requested
        };
        if matches!(
            kind,
            PresentKind::Decoy
                | PresentKind::Telephone
                | PresentKind::SchoolBook
                | PresentKind::Boombox
                | PresentKind::Doorway
        ) && (terrain >= 0x80 || (kind == PresentKind::SchoolBook && position.elevation > 0))
        {
            return Err(PlayerError::Rejected("present requires supported ground"));
        }
        if kind == PresentKind::SpringShoes && level.get() == 0 {
            return Err(PlayerError::Rejected(
                "spring shoes unavailable on level zero",
            ));
        }
        let effect_kind = match kind {
            PresentKind::Decoy => Some((EffectKind::Decoy, 3000)),
            PresentKind::RainCloud => Some((EffectKind::RainCloud, rng.bounded(101) as u16 + 400)),
            PresentKind::Doorway => Some((EffectKind::Doorway, 3000)),
            PresentKind::Telephone => Some((EffectKind::Telephone, 3000)),
            PresentKind::Boombox => Some((EffectKind::Boombox, 300)),
            _ => None,
        };
        let existing_effect = effect_kind.and_then(|(kind, _)| {
            self.resources
                .effects()
                .find(|(_, effect)| effect.owner == EffectOwner::Seat(seat) && effect.kind == kind)
                .map(|(id, effect)| (id, *effect))
        });
        if effect_kind.is_some()
            && existing_effect.is_none()
            && !self
                .resources
                .effect_capacity_available(EffectOwner::Seat(seat))
        {
            return Err(PlayerError::Capacity("present effect quota"));
        }
        let food_delta = if kind == PresentKind::Food {
            if rng.bounded(100) < 70 { 8_i16 } else { -8 }
        } else {
            0
        };
        if kind == PresentKind::Randomizer && self.randomization.is_some() {
            return Err(PlayerError::Rejected(
                "a shared randomization is already active",
            ));
        }
        let burps = if kind == PresentKind::RootBeer {
            rng.bounded(7) as u16 + 12
        } else {
            0
        };
        let rain = if kind == PresentKind::TomatoRain {
            rng.bounded(31) as u8 + 50
        } else {
            0
        };
        let target = if kind == PresentKind::Togetherness {
            Some(nearest_partner(coop, seat).ok_or(PlayerError::Rejected(
                "togetherness has no surviving recipient",
            ))?)
        } else {
            None
        };
        let present_audio = present_audio(kind, food_delta);
        self.audio.reserve(present_audio.iter().flatten().count())?;
        if let Some(target) = target {
            let other = &coop.players[target.index()];
            let destination = Landing {
                level: other.level.expect("alive target"),
                position: Position {
                    elevation: 70 * 256,
                    ..other.motion.position
                },
            };
            validate_landing(world, seat, player.character.archetype, destination)?;
            coop.levels[destination.level.index()].arrivals[seat.index()]
                .checked_add(1)
                .ok_or(PlayerError::Invalid("arrival counter"))?;
            world.commit_present(WorldPresent::Togetherness {
                owner: seat,
                target,
                destination,
            })?;
        }
        let mut effect_position = position;
        match kind {
            PresentKind::Earthling => {
                world.commit_present(WorldPresent::SpawnEarthling {
                    owner: seat,
                    level,
                    position,
                    choice: rng.next_u16(),
                })?;
            }
            PresentKind::Decoy | PresentKind::Doorway | PresentKind::Telephone => {
                let request = match kind {
                    PresentKind::Decoy => WorldPresent::OpenDecoy {
                        owner: seat,
                        level,
                        position,
                    },
                    PresentKind::Doorway => WorldPresent::OpenDoorway {
                        owner: seat,
                        level,
                        position,
                        choice: rng.next_u16(),
                    },
                    PresentKind::Telephone => WorldPresent::OpenTelephone {
                        owner: seat,
                        level,
                        position,
                    },
                    _ => unreachable!("checked placement kind"),
                };
                effect_position =
                    world
                        .commit_present(request)?
                        .effect_position
                        .ok_or(PlayerError::Invalid(
                            "present world omitted accepted placement",
                        ))?;
                effect_position
                    .validate()
                    .map_err(|_| PlayerError::Invalid("accepted effect placement"))?;
            }
            PresentKind::Boombox => {
                effect_position.y = checked_add(effect_position.y, 4 * 256)?;
            }
            _ => {}
        }
        let effect = if let Some((effect_kind, ticks)) = effect_kind {
            let fresh = Effect::stationary(seat, kind, effect_kind, level, effect_position, ticks);
            let id = if let Some((id, previous)) = existing_effect {
                world.apply_actor_effect(ActorEffect::Retire {
                    effect: id,
                    level: previous.level,
                })?;
                *self
                    .resources
                    .effect_mut(id)
                    .expect("live preflight effect") = fresh;
                id
            } else {
                self.resources.spawn_effect(fresh)?
            };
            world.effect_created(id, fresh)?;
            Some(id)
        } else {
            None
        };
        self.resources.effect_rng = rng;
        for cue in present_audio.into_iter().flatten() {
            self.audio.enqueue(seat, cue)?;
        }
        if requested != PresentKind::Mystery {
            self.resources.presents.identify(wrapper);
        }
        compact_slot(&mut coop.players[seat.index()].inventory, usize::from(slot));
        let player = &mut coop.players[seat.index()];
        let runtime = &mut self.runtime[seat.index()];
        player.points = player.points.wrapping_add(2);
        if kind.equipped() {
            runtime.equipped = Some(if kind == PresentKind::BonusHiTops {
                PresentKind::SuperHiTops
            } else {
                kind
            });
            runtime.active_map_epoch = self.resources.presents.epoch;
            player.active_present = Some(wrapper);
            player.counters.active_present_ticks = match kind {
                PresentKind::IcarusWings => 500,
                PresentKind::RocketSkates => 250,
                PresentKind::SuperHiTops | PresentKind::BonusHiTops => 750,
                _ => 1000,
            };
            if kind == PresentKind::RocketSkates {
                runtime.mode = PlayerMode::Skating;
                player.motion.velocity = [768, 0, 0];
                runtime.facing = 3;
            }
            if kind == PresentKind::IcarusWings {
                player.counters.phased_jump_phase = 0;
                player.counters.phased_jump_armed = true;
            }
        }
        match kind {
            PresentKind::Food => {
                player.health = (i16::from(player.health) + food_delta)
                    .clamp(0, i16::from(maximum_health(player)?))
                    as u8;
            }
            PresentKind::FudgeSundae => {
                player.health = player
                    .health
                    .saturating_add(12)
                    .min(maximum_health(player)?);
            }
            PresentKind::RootBeer => {
                runtime.burps_left = runtime.burps_left.saturating_add(burps);
                runtime.burp_delay = 50;
                player.health = player.health.saturating_add(4).min(maximum_health(player)?);
            }
            PresentKind::Promotion => {
                if player.rank < 8 {
                    player.rank += 1;
                    player.health = player.health.saturating_add(4).min(maximum_health(player)?);
                    if player.rank % 3 == 2 && player.lives < 8 {
                        player.lives += 1;
                    }
                }
            }
            PresentKind::TotalBummer => {
                player.health = 0;
            }
            PresentKind::ExtraLife => {
                if player.lives < 8 {
                    player.lives += 1;
                }
            }
            PresentKind::ExtraBuck => {
                player.bucks = player.bucks.saturating_add(1).min(127);
            }
            PresentKind::Jackpot => {
                player.bucks = player.bucks.saturating_add(5).min(127);
            }
            PresentKind::TomatoRain => {
                runtime.rain_left = rain;
                runtime.rain_delay = 0;
            }
            PresentKind::SchoolBook => {
                player.counters.book_sleep_delay = 60;
                player.animation.descriptor = 116;
                player.motion.velocity = [0; 3];
            }
            PresentKind::Randomizer => {
                self.resources.reroll_wrappers();
                self.randomization = Some(Randomization {
                    owner: seat,
                    steps_left: 60,
                });
            }
            PresentKind::Togetherness => {
                let target = target.expect("validated recipient");
                let other = &coop.players[target.index()];
                let destination = Landing {
                    level: other.level.expect("alive target"),
                    position: Position {
                        elevation: 70 * 256,
                        ..other.motion.position
                    },
                };
                let player = &mut coop.players[seat.index()];
                player.level = Some(destination.level);
                player.motion = Motion {
                    position: destination.position,
                    acceleration: [0, 0, -32],
                    ..Motion::default()
                };
                coop.levels[destination.level.index()].arrivals[seat.index()] += 1;
                runtime.mode = PlayerMode::Jumping;
                runtime.checkpoint = Some(Landing {
                    position: Position {
                        elevation: 0,
                        ..destination.position
                    },
                    ..destination
                });
            }
            _ => {}
        }
        Ok(PresentReceipt {
            requested,
            applied: kind,
            effect,
        })
    }
    /// Player phases advance once per authoritative tick, even with an overlay.
    /// Overlays suppress only voluntary locomotion, never damage/status timers.
    pub fn tick_players(
        &mut self,
        coop: &mut CoopState,
        tick: u64,
        inputs: [PlayerInput; SEATS],
        world: &mut impl PlayerWorld,
    ) -> PlayerResult<()> {
        if self
            .last_player_tick
            .is_some_and(|previous| tick <= previous)
            || inputs.iter().any(|input| input.directions & !15 != 0)
        {
            return Err(PlayerError::Invalid("repeated player phase/input bits"));
        }
        for seat in SeatId::ALL {
            if coop.players[seat.index()].life != LifeState::Alive {
                continue;
            }
            self.refresh_terrain(coop, seat, world)?;
            self.tick_seat(coop, seat, inputs[seat.index()], world)?;
        }
        refresh_spectators(coop);
        self.last_player_tick = Some(tick);
        Ok(())
    }
    fn tick_seat(
        &mut self,
        coop: &mut CoopState,
        seat: SeatId,
        mut input: PlayerInput,
        world: &mut impl PlayerWorld,
    ) -> PlayerResult<()> {
        let index = seat.index();
        let runtime = &mut self.runtime[index];
        let player = &mut coop.players[index];
        if runtime.animation.descriptor != player.animation.descriptor {
            runtime.animation = AnimationPlayback {
                descriptor: player.animation.descriptor,
                ..AnimationPlayback::default()
            };
        }
        runtime.invulnerable_ticks = runtime.invulnerable_ticks.saturating_sub(1);
        runtime.shot_delay = runtime.shot_delay.saturating_sub(1);
        input.directions = normalize_directions(input.directions);
        runtime.contacts.shark_alert_ticks = runtime.contacts.shark_alert_ticks.saturating_sub(1);
        if player.counters.controls_remap_ticks != 0 {
            player.counters.controls_remap_ticks -= 1;
            input.directions = runtime.remap[usize::from(input.directions)];
            if player.counters.controls_remap_ticks == 0 {
                runtime.remap = IDENTITY_REMAP;
            }
        }
        let blocked_input = player.ui.overlay != Overlay::Closed;
        let new_jump = (input.jump || input.action)
            && !(runtime.previous_input.jump || runtime.previous_input.action);
        let changed_input = input != runtime.previous_input;
        runtime.previous_input = input;
        if blocked_input {
            input.directions = 0;
            input.action = false;
            input.jump = false;
        }
        if player.counters.hula_dance_ticks != 0 {
            player.counters.hula_dance_ticks -= 1;
            input.directions = 0;
            input.action = false;
            if player.counters.hula_dance_ticks == 0 {
                player.animation.descriptor = 0;
            }
        }
        if player.counters.active_present_ticks != 0 {
            player.counters.active_present_ticks -= 1;
        }
        if player.counters.active_present_ticks == 0 && runtime.equipped.is_some() {
            runtime.equipped = None;
            player.active_present = None;
            if matches!(runtime.mode, PlayerMode::Flying | PlayerMode::Skating) {
                runtime.mode = PlayerMode::Jumping;
                player.motion.acceleration[2] = -32;
            }
        }
        if player.counters.book_sleep_delay != 0 {
            player.counters.book_sleep_delay -= 1;
            input.directions = 0;
            if player.counters.book_sleep_delay == 0 {
                runtime.mode = PlayerMode::Sleeping;
                runtime.sleep_ticks = 0;
                player.counters.wake_effects_left = self.resources.effect_rng.bounded(7) as u8 + 4;
                player.animation.descriptor = 117;
            }
        }
        if runtime.mode == PlayerMode::Sleeping {
            if runtime.sleep_ticks != 0 {
                runtime.sleep_ticks -= 1;
            }
            if player.counters.wake_effects_left != 0 {
                if runtime.sleep_ticks == 0
                    && changed_input
                    && (input.wake || input.directions != 0 || input.action || input.jump)
                    && self.resources.effect_rng.bounded(100) < 20
                {
                    player.counters.wake_effects_left -= 1;
                    runtime.sleep_ticks = if player.counters.wake_effects_left == 0 {
                        30
                    } else {
                        15
                    };
                }
            } else if runtime.sleep_ticks == 0 {
                runtime.mode = PlayerMode::Grounded;
                player.animation.descriptor = 0;
            }
            if runtime.mode == PlayerMode::Sleeping {
                input.directions = 0;
                input.action = false;
            }
        }
        if matches!(runtime.mode, PlayerMode::Hurt | PlayerMode::Squished) {
            runtime.mode_ticks = runtime.mode_ticks.saturating_sub(1);
            if runtime.mode_ticks == 0 {
                player.health = player
                    .health
                    .saturating_sub(player.counters.pending_hurt_damage);
                player.counters.pending_hurt_damage = 0;
                runtime.invulnerable_ticks = 30;
                runtime.mode = PlayerMode::Grounded;
                player.animation.descriptor = 0;
            } else {
                input.directions = 0;
                input.action = false;
            }
        }
        if player.health == 0 && runtime.mode != PlayerMode::Dying {
            runtime.mode = PlayerMode::Dying;
            runtime.mode_ticks = 60;
            runtime.equipped = None;
            player.active_present = None;
            player.counters.active_present_ticks = 0;
            player.ui.overlay = Overlay::Closed;
            player.motion.velocity = [0, 0, 256];
            player.motion.acceleration = [0; 3];
            player.animation.descriptor = 5;
            if let Some(service) = runtime.service.take() {
                world.apply_actor_effect(ActorEffect::ServiceCollision {
                    actor: service.actor,
                    collidable: true,
                })?;
            }
        }
        if runtime.mode == PlayerMode::Dying {
            runtime.mode_ticks = runtime.mode_ticks.saturating_sub(1);
            if runtime.mode_ticks != 0 {
                return Ok(());
            }
            if player.lives == 0 {
                world.release_hero(seat)?;
                player.life = LifeState::Eliminated;
                player.level = None;
                player.motion = Motion::default();
                runtime.contacts = Default::default();
                runtime.service = None;
                runtime.fall_token = None;
                for entry in &mut self.fall_cohorts {
                    if let Some(cohort) = entry
                        && cohort.seats.contains(seat)
                    {
                        let seats = cohort.seats.bits() & !(1 << seat.get());
                        if seats == 0 {
                            *entry = None;
                        } else {
                            cohort.seats = SeatMask::new(seats).expect("four seats");
                        }
                    }
                }
            } else {
                let checkpoint = runtime
                    .checkpoint
                    .ok_or(PlayerError::Invalid("death checkpoint"))?;
                let landing = world.respawn(seat, checkpoint)?;
                let lives = player.lives - 1;
                let mut destinations = [None; SEATS];
                destinations[seat.index()] = Some(landing);
                let prepared = self.prepare_arrivals(
                    coop,
                    destinations,
                    ArrivalPolicy::Revive { lives },
                    world,
                )?;
                world.commit_arrival(seat, landing)?;
                self.commit_arrivals(coop, prepared);
            }
            return Ok(());
        }
        if runtime.mode == PlayerMode::Whirlwind {
            runtime.mode_ticks = runtime.mode_ticks.saturating_sub(1);
            if runtime.mode_ticks == 0 {
                runtime.mode = PlayerMode::Jumping;
                runtime.invulnerable_ticks = 30;
                player.motion.acceleration[2] = -32;
            } else {
                return Ok(());
            }
        }
        if runtime.mode == PlayerMode::AwaitingFloor {
            return Ok(());
        }
        if runtime.mode == PlayerMode::Falling {
            runtime.effective_input = PlayerInput::default();
            return Ok(());
        }
        let terrain = player.terrain[2][2] & 0xf0;
        if runtime.service.is_some()
            || runtime.contacts.offer.is_some_and(|offer| {
                offer.active
                    && matches!(
                        offer.service,
                        crate::population::Service::Heal | crate::population::Service::Opera
                    )
            })
            || runtime.mode == PlayerMode::MoleCaptured
        {
            runtime.effective_input = PlayerInput::default();
            return Ok(());
        }
        if runtime.locomotion_locked {
            runtime.effective_input = PlayerInput::default();
            return Ok(());
        }
        if terrain == 0x90 && player.motion.position.elevation <= 0 {
            runtime.fall_requested = true;
        }
        if terrain == 0x80 && player.motion.position.elevation <= 0 {
            if runtime.equipped == Some(PresentKind::Innertube) {
                runtime.mode = PlayerMode::Swimming;
                player.motion.position.elevation = 0;
            } else {
                player.counters.terrain_sinking = player.counters.terrain_sinking.saturating_add(1);
                player.motion.position.elevation = -3 * 256;
                if player.counters.terrain_sinking == 120 {
                    player.health = 0;
                }
            }
        } else if terrain <= 0x50 {
            player.counters.terrain_sinking = 0;
        }
        if input.directions != 0 {
            runtime.facing = direction_facing(input.directions);
            player.counters.idle_ticks = 0;
        } else {
            player.counters.idle_ticks = player.counters.idle_ticks.saturating_add(1);
        }
        if runtime.equipped == Some(PresentKind::IcarusWings) && (input.jump || input.action) {
            runtime.mode = PlayerMode::Flying;
            if player.motion.position.elevation < 50 * 256 {
                player.motion.velocity[2] = 256;
                player.motion.acceleration[2] = -10;
            } else {
                player.motion.velocity[2] = 0;
                player.motion.acceleration[2] = 0;
            }
        } else if new_jump
            && !blocked_input
            && runtime.equipped == Some(PresentKind::SpringShoes)
            && player.motion.position.elevation <= 0
        {
            runtime.mode = PlayerMode::Jumping;
            player.motion.velocity[2] = 1536;
            player.motion.acceleration[2] = -64;
            player.animation.descriptor = 20 + runtime.facing;
        }
        if runtime.equipped == Some(PresentKind::RocketSkates) {
            runtime.mode = PlayerMode::Skating;
            if input.directions != 0 {
                runtime.facing = direction_facing(input.directions);
            }
            let direction = runtime.facing;
            player.motion.velocity[0] = if direction == 2 {
                (player.motion.velocity[0] - 128).clamp(-4096, -768)
            } else if direction == 3 {
                (player.motion.velocity[0] + 128).clamp(768, 4096)
            } else {
                0
            };
            player.motion.velocity[1] = if direction == 0 {
                (player.motion.velocity[1] - 128).clamp(-3072, -576)
            } else if direction == 1 {
                (player.motion.velocity[1] + 128).clamp(576, 3072)
            } else {
                0
            };
        } else if !matches!(
            runtime.mode,
            PlayerMode::Hurt | PlayerMode::Squished | PlayerMode::Sleeping
        ) {
            let (mut sx, mut sy) = if terrain == 0x30 {
                (400, 300)
            } else {
                (320, 240)
            };
            if runtime.equipped == Some(PresentKind::SuperHiTops) && (input.jump || input.action) {
                sx *= 3;
                sy *= 3;
            }
            if input.sneak {
                sx /= 2;
                sy /= 2;
            }
            if player.counters.terrain_sinking != 0 {
                sx = (sx - 160).max(80);
                sy = (sy - 128).max(64);
            }
            let horizontal =
                i32::from(input.directions & 8 != 0) - i32::from(input.directions & 4 != 0);
            let vertical =
                i32::from(input.directions & 2 != 0) - i32::from(input.directions & 1 != 0);
            if runtime.mode == PlayerMode::Flying {
                player.motion.velocity[0] =
                    (player.motion.velocity[0] + horizontal * 10).clamp(-512, 512);
                player.motion.velocity[1] =
                    (player.motion.velocity[1] + vertical * 10).clamp(-448, 448);
                if !input.jump {
                    player.motion.acceleration[2] = -10;
                }
            } else {
                player.motion.velocity[0] = horizontal * if vertical != 0 { sy } else { sx };
                player.motion.velocity[1] = vertical * if horizontal != 0 { sy } else { sx };
            }
        }
        runtime.authored_motion.axes = match runtime.mode {
            PlayerMode::Grounded | PlayerMode::Swimming if input.directions != 0 => {
                if runtime.equipped == Some(PresentKind::SuperHiTops)
                    && (input.jump || input.action)
                {
                    0
                } else if input.directions & 12 != 0 {
                    3
                } else {
                    2
                }
            }
            PlayerMode::Hurt | PlayerMode::Squished => 3,
            _ => 0,
        };
        runtime.effective_input = input;
        select_motion_pose(player, runtime, input, terrain);
        if player.counters.idle_ticks > 300
            && runtime.mode == PlayerMode::Grounded
            && !blocked_input
        {
            runtime.mode = PlayerMode::Sleeping;
            runtime.sleep_ticks = 0;
            player.counters.wake_effects_left = self.resources.effect_rng.bounded(7) as u8 + 4;
            player.animation.descriptor = 117;
        }
        if input.action
            && runtime.shot_delay == 0
            && matches!(
                runtime.equipped,
                Some(PresentKind::Tomatoes | PresentKind::Slingshot | PresentKind::RoseBushes)
            )
        {
            self.launch_present(coop, seat, world)?;
        }
        Ok(())
    }
    fn launch_present(
        &mut self,
        coop: &CoopState,
        seat: SeatId,
        world: &mut impl PlayerWorld,
    ) -> PlayerResult<()> {
        let player = &coop.players[seat.index()];
        let runtime = &mut self.runtime[seat.index()];
        let source = runtime
            .equipped
            .ok_or(PlayerError::Rejected("no projectile present"))?;
        let kind = match source {
            PresentKind::Tomatoes => EffectKind::Tomato,
            PresentKind::Slingshot => EffectKind::Slingshot,
            PresentKind::RoseBushes => EffectKind::RoseBush,
            _ => return Err(PlayerError::Rejected("not a projectile present")),
        };
        if !self
            .resources
            .effect_capacity_available(EffectOwner::Seat(seat))
        {
            return Ok(());
        }
        let mut effect = Effect::stationary(
            seat,
            source,
            kind,
            player.level.expect("alive level"),
            player.motion.position,
            if kind == EffectKind::RoseBush {
                1000
            } else {
                150
            },
        );
        if kind != EffectKind::RoseBush {
            let (sx, sy, vz, az) = if source == PresentKind::Tomatoes {
                (896, 512, 160, -32)
            } else {
                (1152, 768, 64, -8)
            };
            let facing = runtime.facing;
            effect.position.elevation = checked_add(
                effect.position.elevation,
                i32::from(player.character.archetype.dimensions().1 / 2) * 256,
            )?;
            effect.velocity = [
                if facing == 2 {
                    -sx
                } else if facing == 3 {
                    sx
                } else {
                    0
                },
                if facing == 0 {
                    -sy
                } else if facing == 1 {
                    sy
                } else {
                    0
                },
                vz,
            ];
            effect.acceleration[2] = az;
        }
        let id = self.resources.spawn_effect(effect)?;
        world.effect_created(id, effect)?;
        runtime.shot_delay = if kind == EffectKind::RoseBush { 20 } else { 10 };
        Ok(())
    }
    pub fn tick_effects(
        &mut self,
        coop: &mut CoopState,
        tick: u64,
        world: &mut impl PlayerWorld,
    ) -> PlayerResult<()> {
        if self
            .last_effect_tick
            .is_some_and(|previous| tick <= previous)
        {
            return Err(PlayerError::Invalid("repeated effect phase"));
        }
        if let Some(randomization) = &mut self.randomization {
            let mut rng = self.resources.effect_rng;
            let mut inventory =
                std::array::from_fn::<_, SEATS, _>(|index| coop.players[index].inventory);
            for hero in &mut inventory {
                for wrapper in hero.iter_mut().flatten() {
                    if self.resources.presents.kind(*wrapper) != PresentKind::BonusHiTops {
                        *wrapper = self
                            .resources
                            .presents
                            .wrapper_for(world.choose_present(&mut rng)?);
                    }
                }
            }
            for (player, inventory) in coop.players.iter_mut().zip(inventory) {
                player.inventory = inventory;
            }
            self.resources.effect_rng = rng;
            randomization.steps_left -= 1;
            if randomization.steps_left == 0 {
                self.randomization = None;
            }
        }
        // IDs fit fixed scratch; no per-frame pool copy/allocation or renderer reads.
        let mut ids = [None; 64];
        for (index, (id, _)) in self.resources.effects().enumerate() {
            ids[index] = Some(id);
        }
        for id in ids.into_iter().flatten() {
            self.tick_effect(coop, id, world)?;
        }
        for seat in SeatId::ALL {
            if coop.players[seat.index()].life != LifeState::Alive {
                continue;
            }
            let runtime = &mut self.runtime[seat.index()];
            let player = &coop.players[seat.index()];
            if runtime.burps_left != 0 {
                runtime.burp_delay = runtime.burp_delay.saturating_sub(1);
                if runtime.burp_delay == 0 {
                    self.audio.reserve(1)?;
                    world.apply_actor_effect(ActorEffect::Burp {
                        owner: seat,
                        level: player.level.expect("alive level"),
                        position: player.motion.position,
                    })?;
                    self.audio.enqueue(
                        seat,
                        AudioCue::Pcm {
                            sound: PcmSound::Burp,
                            priority: 50,
                            mode: 0,
                        },
                    )?;
                    runtime.burps_left -= 1;
                    runtime.burp_delay = 20 + self.resources.effect_rng.bounded(81) as u16;
                }
            }
            if runtime.rain_left != 0 {
                runtime.rain_delay = runtime.rain_delay.saturating_sub(1);
                if runtime.rain_delay == 0
                    && self
                        .resources
                        .effect_capacity_available(EffectOwner::Seat(seat))
                {
                    let mut effect = Effect::stationary(
                        seat,
                        PresentKind::TomatoRain,
                        EffectKind::RainTomato,
                        player.level.expect("alive level"),
                        player.motion.position,
                        180,
                    );
                    effect.position.x = checked_add(
                        effect.position.x,
                        (self.resources.effect_rng.bounded(201) as i32 - 100) * 256,
                    )?;
                    effect.position.y = checked_add(
                        effect.position.y,
                        (self.resources.effect_rng.bounded(101) as i32 - 50) * 256,
                    )?;
                    effect.position.elevation = 224 * 256;
                    effect.velocity[2] = -384;
                    effect.acceleration[2] = -18;
                    let id = self.resources.spawn_effect(effect)?;
                    world.effect_created(id, effect)?;
                    runtime.rain_left -= 1;
                    runtime.rain_delay = self.resources.effect_rng.bounded(5) as u8 + 4;
                }
            }
        }
        self.last_effect_tick = Some(tick);
        Ok(())
    }
    pub fn retire_effect(
        &mut self,
        id: EffectId,
        world: &mut impl PlayerWorld,
    ) -> PlayerResult<bool> {
        let Some(effect) = self.resources.effect(id).copied() else {
            return Ok(false);
        };
        world.apply_actor_effect(ActorEffect::Retire {
            effect: id,
            level: effect.level,
        })?;
        Ok(self.resources.release_effect(id))
    }
    pub fn set_effect_animation(&mut self, id: EffectId, held: bool) -> PlayerResult<()> {
        let effect = self
            .resources
            .effect_mut(id)
            .ok_or(PlayerError::Invalid("stale effect animation"))?;
        effect.animation_complete = held;
        Ok(())
    }
    pub fn hit_decoy(&mut self, id: EffectId, world: &mut impl PlayerWorld) -> PlayerResult<bool> {
        if !self.resources.hit_decoy(id) {
            return Ok(false);
        }
        if self
            .resources
            .effect(id)
            .is_some_and(|effect| effect.hits >= 3)
        {
            self.retire_effect(id, world)?;
        }
        Ok(true)
    }
    fn tick_effect(
        &mut self,
        coop: &mut CoopState,
        id: EffectId,
        world: &mut impl PlayerWorld,
    ) -> PlayerResult<()> {
        let Some(mut effect) = self.resources.effect(id).copied() else {
            return Ok(());
        };
        effect.ticks_left -= 1;
        if effect.ticks_left == 0 {
            if effect.kind == EffectKind::Boombox {
                world.apply_actor_effect(ActorEffect::WakeBoombox {
                    effect: id,
                    level: effect.level,
                })?;
            }
            self.retire_effect(id, world)?;
            return Ok(());
        }
        let owner = match effect.owner {
            EffectOwner::Seat(seat) => Some(seat),
            _ => None,
        };
        match effect.kind {
            EffectKind::Decoy => {
                if owner.is_none_or(|seat| coop.players[seat.index()].level != Some(effect.level))
                    || effect.hits >= 3
                {
                    self.retire_effect(id, world)?;
                    return Ok(());
                }
            }
            EffectKind::RainCloud => {
                let seat = owner.ok_or(PlayerError::Invalid("rain cloud seat owner"))?;
                let hero = &coop.players[seat.index()];
                if hero.life != LifeState::Alive {
                    self.retire_effect(id, world)?;
                    return Ok(());
                }
                effect.level = hero.level.expect("alive level");
                effect.position = hero.motion.position;
                if effect.delay != 0 {
                    effect.delay -= 1;
                    if effect.delay == 0 {
                        self.damage(
                            coop,
                            seat,
                            DamageRequest {
                                source: DamageSource::Effect(id),
                                amount: 2,
                                style: DamageStyle::Hurt,
                                origin: effect.position,
                            },
                            world,
                        )?;
                    }
                } else if self.resources.effect_rng.next_u16() & 63 == 1 {
                    effect.delay = 8;
                }
            }
            EffectKind::Boombox => {
                world.apply_actor_effect(ActorEffect::Boombox {
                    effect: id,
                    level: effect.level,
                    position: effect.position,
                })?;
            }
            EffectKind::RoseBush => {
                if effect.animation_complete {
                    world.apply_actor_effect(ActorEffect::Roses {
                        effect: id,
                        owner: owner.ok_or(PlayerError::Invalid("rose owner"))?,
                        level: effect.level,
                        position: effect.position,
                    })?;
                    let damage = world.contact_damage(crate::population::Kind::Devil)?;
                    for seat in SeatId::ALL {
                        let hero = &coop.players[seat.index()];
                        let near = hero.life == LifeState::Alive
                            && hero.level == Some(effect.level)
                            && (i64::from(hero.motion.position.x) - i64::from(effect.position.x))
                                .abs()
                                < 25 * 256
                            && (i64::from(hero.motion.position.y) - i64::from(effect.position.y))
                                .abs()
                                < 10 * 256;
                        if near && effect.contact_mask & (1 << seat.get()) == 0 {
                            if self.damage(
                                coop,
                                seat,
                                DamageRequest {
                                    source: DamageSource::Effect(id),
                                    amount: damage,
                                    style: DamageStyle::Hurt,
                                    origin: effect.position,
                                },
                                world,
                            )? {
                                effect.contact_mask |= 1 << seat.get();
                            }
                        } else if !near {
                            effect.contact_mask &= !(1 << seat.get());
                        }
                    }
                }
            }
            EffectKind::Tomato
            | EffectKind::Slingshot
            | EffectKind::RainTomato
            | EffectKind::ControlRemap => {
                if effect.position.elevation >= 0 && effect.position.elevation <= 25 * 256 {
                    for seat in SeatId::ALL {
                        let hero = &coop.players[seat.index()];
                        let reach =
                            i64::from(hero.character.archetype.dimensions().0 / 2 - 1) * 256;
                        let eligible = hero.life == LifeState::Alive
                            && hero.level == Some(effect.level)
                            && (effect.owner != EffectOwner::Seat(seat)
                                || effect.kind == EffectKind::RainTomato);
                        if eligible
                            && (i64::from(hero.motion.position.x) - i64::from(effect.position.x))
                                .abs()
                                < reach
                            && (i64::from(hero.motion.position.y) - i64::from(effect.position.y))
                                .abs()
                                < reach * 3 / 4
                            && i64::from(hero.motion.position.elevation)
                                + i64::from(hero.character.archetype.dimensions().1 - 10) * 256
                                > i64::from(effect.position.elevation)
                        {
                            if effect.kind == EffectKind::ControlRemap {
                                let directions = world
                                    .control_remap(self.resources.effect_rng.bounded(4) as u8)?;
                                self.status(
                                    coop,
                                    seat,
                                    StatusRequest::Remap {
                                        ticks: 240,
                                        directions,
                                    },
                                )?;
                            } else {
                                let amount = if matches!(effect.owner, EffectOwner::Actor(_)) {
                                    6
                                } else {
                                    4
                                };
                                self.damage(
                                    coop,
                                    seat,
                                    DamageRequest {
                                        source: DamageSource::Effect(id),
                                        amount,
                                        style: DamageStyle::Direct,
                                        origin: effect.position,
                                    },
                                    world,
                                )?;
                            }
                            self.retire_effect(id, world)?;
                            return Ok(());
                        }
                    }
                    let decoy = self.resources.effects().find_map(|(target, candidate)| {
                        (target != id
                            && candidate.kind == EffectKind::Decoy
                            && candidate.level == effect.level
                            && (i64::from(candidate.position.x) - i64::from(effect.position.x))
                                .abs()
                                < 16 * 256
                            && (i64::from(candidate.position.y) - i64::from(effect.position.y))
                                .abs()
                                < 12 * 256)
                            .then_some(target)
                    });
                    if let Some(decoy) = decoy {
                        self.hit_decoy(decoy, world)?;
                        self.retire_effect(id, world)?;
                        return Ok(());
                    }
                }
                if effect.position.elevation < 0
                    || world.apply_actor_effect(ActorEffect::Projectile {
                        effect: id,
                        owner: effect.owner,
                        level: effect.level,
                        position: effect.position,
                        damage: 1,
                    })?
                {
                    self.retire_effect(id, world)?;
                    return Ok(());
                }
            }
            EffectKind::Doorway => {
                let owner = owner.ok_or(PlayerError::Invalid("doorway owner"))?;
                for seat in SeatId::ALL {
                    let hero = &coop.players[seat.index()];
                    let in_range = hero.life == LifeState::Alive
                        && hero.level == Some(effect.level)
                        && (i64::from(hero.motion.position.x) - i64::from(effect.position.x)).abs()
                            < 6 * 256
                        && (i64::from(hero.motion.position.y) - i64::from(effect.position.y)).abs()
                            < 8 * 256;
                    if in_range && effect.contact_mask & (1 << seat.get()) == 0 {
                        if let Some(landing) = world.doorway_crossing(
                            id,
                            owner,
                            seat,
                            effect.level,
                            hero.motion.position,
                        )? {
                            if landing.level != effect.level {
                                return Err(PlayerError::Invalid("doorway changed floor"));
                            }
                            validate_landing(world, seat, hero.character.archetype, landing)?;
                            coop.players[seat.index()].motion.position = landing.position;
                            effect.contact_mask |= 1 << seat.get();
                        }
                    } else if !in_range {
                        effect.contact_mask &= !(1 << seat.get());
                    }
                }
            }
            EffectKind::Telephone => {
                for seat in SeatId::ALL {
                    let hero = &coop.players[seat.index()];
                    let near = hero.life == LifeState::Alive
                        && hero.level == Some(effect.level)
                        && (i64::from(hero.motion.position.x) - i64::from(effect.position.x)).abs()
                            < 16 * 256
                        && (i64::from(hero.motion.position.y) - i64::from(effect.position.y)).abs()
                            < 8 * 256;
                    if near
                        && hero.ui.overlay == Overlay::Closed
                        && effect.contact_mask & (1 << seat.get()) == 0
                    {
                        world.answer_telephone(id, seat, effect.level)?;
                        coop.players[seat.index()].ui.overlay = Overlay::Map;
                        effect.contact_mask |= 1 << seat.get();
                    }
                }
            }
            EffectKind::StolenPresent => {
                if effect.position.elevation <= 0 {
                    self.retire_effect(id, world)?;
                    return Ok(());
                }
            }
        }
        if let Some(stored) = self.resources.effect_mut(id) {
            *stored = effect;
        }
        Ok(())
    }
}

pub(crate) fn maximum_health(player: &tje_four_player::state::PlayerState) -> PlayerResult<u8> {
    player
        .character
        .archetype
        .maximum_health(player.rank)
        .map_err(|_| PlayerError::Invalid("hero rank"))
}
pub(crate) fn validate_landing(
    world: &impl PlayerGeometry,
    seat: SeatId,
    archetype: Archetype,
    landing: Landing,
) -> PlayerResult<()> {
    landing
        .level
        .validate()
        .map_err(|_| PlayerError::Invalid("landing level"))?;
    landing
        .position
        .validate()
        .map_err(|_| PlayerError::Invalid("landing position"))?;
    let (width, height) = world.dimensions(landing.level)?.world_envelope();
    let (x, y) = (landing.position.x >> 8, landing.position.y >> 8);
    if x < 8
        || y < 8
        || x >= width - 8
        || y >= height - 8
        || world.terrain(landing.level, x, y)? & 0xf0 > 0x50
        || world.blocked(seat, archetype, landing.level, landing.position)?
    {
        return Err(PlayerError::Geometry("unsafe landing"));
    }
    Ok(())
}
fn checked_add(left: i32, right: i32) -> PlayerResult<i32> {
    left.checked_add(right)
        .ok_or(PlayerError::Invalid("24.8 arithmetic overflow"))
}
fn integrate_vertical(motion: &mut Motion) -> PlayerResult<()> {
    motion.velocity[2] =
        checked_add(motion.velocity[2], motion.acceleration[2])?.clamp(-64 * 256, 64 * 256);
    motion.position.elevation = checked_add(motion.position.elevation, motion.velocity[2])?;
    Ok(())
}
fn ordinary_voluntary(
    player: &tje_four_player::state::PlayerState,
    runtime: &PlayerRuntime,
) -> bool {
    runtime.mode == PlayerMode::Grounded
        && !runtime.locomotion_locked
        && runtime.service.is_none()
        && player.ui.overlay == Overlay::Closed
        && player.counters.hula_dance_ticks == 0
        && runtime.effective_input.directions != 0
}
fn apply_displacement(
    world: &impl PlayerGeometry,
    seat: SeatId,
    player: &mut tje_four_player::state::PlayerState,
    runtime: &mut PlayerRuntime,
    delta: [i32; 2],
) -> PlayerResult<()> {
    if delta == [0; 2] {
        return Ok(());
    }
    let level = player.level.expect("alive movement level");
    let old = player.motion.position;
    let mut candidate = old;
    let (width, height) = world.dimensions(level)?.world_envelope();
    let (hero_width, hero_height) = player.character.archetype.dimensions();
    // Preserve the source's general overlap-escape policy; this is not an
    // exception for an owner's decoy or any particular collision kind.
    for (axis, value) in delta.into_iter().enumerate() {
        if value == 0 {
            continue;
        }
        if axis == 0 {
            candidate.x = checked_add(old.x, value)?;
        } else {
            candidate.y = checked_add(old.y, value)?;
        }
        let x = candidate.x >> 8;
        let y = candidate.y >> 8;
        let outside = x - i32::from(hero_width) <= 8
            || x + i32::from(hero_width) >= width - 36
            || y - i32::from(hero_height) - (old.elevation >> 8) <= 8
            || y >= height - 56;
        let terrain = world.terrain(level, x, y)? & 0xf0;
        let unsupported = old.elevation <= 0
            && terrain > 0x50
            && !(terrain == 0x80 && runtime.equipped == Some(PresentKind::Innertube))
            && !matches!(
                runtime.mode,
                PlayerMode::Skating | PlayerMode::Flying | PlayerMode::Jumping
            );
        if outside
            || unsupported
            || world.movement_blocked(seat, player.character.archetype, level, old, candidate)?
        {
            if axis == 0 {
                candidate.x = old.x;
            } else {
                candidate.y = old.y;
            }
            runtime.blocked_axes |= 1 << axis;
        }
    }
    candidate
        .validate()
        .map_err(|_| PlayerError::Invalid("hero motion exceeded 24.8 bounds"))?;
    player.motion.position = candidate;
    Ok(())
}
fn direction_facing(input: u8) -> u8 {
    if input & 4 != 0 {
        2
    } else if input & 8 != 0 {
        3
    } else if input & 1 != 0 {
        0
    } else {
        1
    }
}
pub const IDENTITY_REMAP: [u8; 16] = [0, 1, 2, 0, 4, 5, 6, 0, 8, 9, 10, 0, 0, 0, 0, 0];
pub const fn normalize_directions(mut input: u8) -> u8 {
    input &= 15;
    if input & 3 == 3 {
        input &= !3;
    }
    if input & 12 == 12 {
        input &= !12;
    }
    input
}
fn valid_remap(remap: [u8; 16]) -> bool {
    remap[0] == 0 && remap.into_iter().all(|direction| direction <= 15)
}
fn validate_ui_command(command: UiCommand) -> PlayerResult<()> {
    match command {
        UiCommand::OpenPresent(slot) | UiCommand::Drop(slot) | UiCommand::Identify { slot, .. }
            if slot >= 16 =>
        {
            Err(PlayerError::Invalid("UI command slot"))
        }
        UiCommand::DonateLife(seat) => seat
            .validate()
            .map_err(|_| PlayerError::Invalid("UI command recipient")),
        _ => Ok(()),
    }
}
fn nearest_partner(coop: &CoopState, seat: SeatId) -> Option<SeatId> {
    let hero = &coop.players[seat.index()];
    coop.players
        .iter()
        .filter(|other| other.seat != seat && other.life == LifeState::Alive)
        .min_by_key(|other| {
            let dx = i64::from(other.motion.position.x) - i64::from(hero.motion.position.x);
            let dy = i64::from(other.motion.position.y) - i64::from(hero.motion.position.y);
            (
                u8::from(other.level != hero.level),
                dx * dx + dy * dy,
                3 - other.seat.get(),
            )
        })
        .map(|other| other.seat)
}
fn refresh_spectators(coop: &mut CoopState) {
    for seat in SeatId::ALL {
        if coop.players[seat.index()].life == LifeState::Eliminated {
            coop.players[seat.index()].spectating = nearest_partner(coop, seat);
        }
    }
}

pub(crate) fn sample_terrain(
    world: &impl PlayerGeometry,
    landing: Landing,
) -> PlayerResult<[[u8; 5]; 5]> {
    world.terrain_cache(landing)
}

fn tick_service(
    player: &mut tje_four_player::state::PlayerState,
    runtime: &mut PlayerRuntime,
    input: PlayerInput,
    world: &impl PlayerGeometry,
    effect: &mut Option<ActorEffect>,
) -> PlayerResult<bool> {
    let Some(mut service) = runtime.service else {
        return Ok(false);
    };
    service.ticks = service.ticks.saturating_add(1);
    if service.kind == HeroServiceKind::Lemonade {
        if animation_complete(player, runtime) {
            if !runtime.lemonade_rewarded {
                if player.lives < 8 {
                    player.lives += 1;
                }
                runtime.lemonade_rewarded = true;
            }
            runtime.service = None;
            player.animation.descriptor = 0;
        } else {
            runtime.service = Some(service);
        }
        return Ok(true);
    }
    match service.stage {
        ServiceStage::Windup if service.ticks > 4 => {
            *effect = Some(ActorEffect::ServiceCollision {
                actor: service.actor,
                collidable: false,
            });
            service.stage = ServiceStage::Takeoff;
            service.ticks = 0;
            player.motion.position.elevation = 256;
            player.animation.descriptor = if service.approach_left { 35 } else { 34 };
        }
        ServiceStage::Takeoff if animation_complete(player, runtime) => {
            *effect = Some(ActorEffect::ServiceCollision {
                actor: service.actor,
                collidable: true,
            });
            player.motion.position = Position {
                elevation: 35 * 256,
                ..service.position
            };
            player.motion.acceleration[2] = -64;
            service.stage = ServiceStage::Descend;
            player.animation.descriptor = if service.approach_left { 39 } else { 38 };
        }
        ServiceStage::Descend => {
            if player.motion.position.elevation < 22 * 256 {
                player.motion.position.elevation = 22 * 256;
                player.motion.velocity[2] = 0;
                player.motion.acceleration[2] = 0;
                service.stage = ServiceStage::Splash;
                player.animation.descriptor = if service.approach_left { 43 } else { 42 };
            }
        }
        ServiceStage::Splash if animation_complete(player, runtime) => {
            service.stage = ServiceStage::Bathing;
            player.motion.position.elevation = 0;
            player.counters.jump = 0;
            player.animation.descriptor = 119;
        }
        ServiceStage::Bathing => {
            player.counters.jump = player.counters.jump.wrapping_add(1);
            if player.counters.jump > 5 {
                player.health = player.health.saturating_add(1).min(maximum_health(player)?);
            }
            if input.directions & if service.approach_left { 4 } else { 8 } != 0 {
                service.stage = ServiceStage::Exit;
                player.animation.descriptor = 120;
            }
        }
        ServiceStage::Exit if animation_complete(player, runtime) => {
            let mut destination = player.motion.position;
            destination.x = checked_add(
                destination.x,
                if service.approach_left {
                    -55 * 256
                } else {
                    67 * 256
                },
            )?;
            destination.elevation = if service.approach_left {
                42 * 256
            } else {
                39 * 256
            };
            if !world.blocked(
                player.seat,
                player.character.archetype,
                player.level.expect("alive service"),
                destination,
            )? {
                player.motion.position = destination;
                player.motion.acceleration[2] = -32;
                runtime.mode = PlayerMode::Jumping;
                runtime.service = None;
                player.animation.descriptor = if service.approach_left { 23 } else { 22 };
                return Ok(true);
            }
        }
        _ => {}
    }
    runtime.service = Some(service);
    Ok(true)
}

fn animation_complete(
    player: &tje_four_player::state::PlayerState,
    runtime: &PlayerRuntime,
) -> bool {
    runtime.animation.descriptor == player.animation.descriptor
        && runtime.animation.started
        && runtime.animation.held
}

fn select_motion_pose(
    player: &mut tje_four_player::state::PlayerState,
    runtime: &mut PlayerRuntime,
    input: PlayerInput,
    terrain: u8,
) {
    if player.counters.hula_dance_ticks != 0
        || player.counters.book_sleep_delay != 0
        || runtime.service.is_some()
    {
        return;
    }
    let moving = input.directions != 0;
    let descriptor = match runtime.mode {
        PlayerMode::Grounded => {
            if moving {
                let family = match runtime.equipped {
                    Some(PresentKind::SpringShoes) => 56,
                    Some(PresentKind::Innertube) => 72,
                    Some(PresentKind::SuperHiTops) if input.jump || input.action => 108,
                    None if input.sneak => 12,
                    _ => 8,
                };
                family + runtime.facing
            } else {
                match runtime.equipped {
                    Some(PresentKind::IcarusWings) => 4,
                    Some(PresentKind::SpringShoes) => 57,
                    Some(PresentKind::Innertube) => {
                        if terrain == 0x80 {
                            77
                        } else {
                            76
                        }
                    }
                    _ => {
                        if terrain == 0x80 {
                            3
                        } else {
                            0
                        }
                    }
                }
            }
        }
        PlayerMode::Flying => 60 + runtime.facing,
        PlayerMode::Jumping => 20 + runtime.facing,
        PlayerMode::Swimming => {
            if moving {
                72 + runtime.facing
            } else {
                77
            }
        }
        PlayerMode::Skating => {
            let family = if player.motion.position.elevation > 0 || terrain == 0x90 {
                104
            } else if terrain == 0x80 {
                100
            } else if input.jump || input.action {
                96
            } else {
                92
            };
            family + runtime.facing
        }
        _ => return,
    };
    if player.animation.descriptor != descriptor {
        player.animation.descriptor = descriptor;
        player.animation.sequence_index = 0;
        player.animation.ticks_left = 0;
        runtime.animation = AnimationPlayback {
            descriptor,
            ..AnimationPlayback::default()
        };
    }
}

pub(crate) fn reset_playback(
    player: &mut tje_four_player::state::PlayerState,
    runtime: &mut PlayerRuntime,
) {
    runtime.animation = AnimationPlayback {
        descriptor: player.animation.descriptor,
        ..AnimationPlayback::default()
    };
    player.animation.sequence_index = 0;
    player.animation.ticks_left = 0;
}
fn present_audio(kind: PresentKind, food_delta: i16) -> [Option<AudioCue>; 2] {
    let pcm = |sound, priority, mode| {
        Some(AudioCue::Pcm {
            sound,
            priority,
            mode,
        })
    };
    let cue = match kind {
        PresentKind::Decoy | PresentKind::Doorway | PresentKind::Togetherness => {
            Some(AudioCue::Psg {
                command: 16,
                priority: 50,
            })
        }
        PresentKind::RocketSkates => Some(AudioCue::Psg {
            command: 5,
            priority: 94,
        }),
        PresentKind::Randomizer => pcm(PcmSound::Randomize, 96, 1),
        PresentKind::ExtraLife => pcm(PcmSound::ExtraLife, 87, 0),
        PresentKind::ExtraBuck | PresentKind::Jackpot => pcm(PcmSound::Money, 50, 0),
        PresentKind::RootBeer => Some(AudioCue::Template { effect: 14 }),
        PresentKind::Boombox => Some(AudioCue::BoomboxStart),
        PresentKind::Food if food_delta < 0 => Some(AudioCue::Psg {
            command: 18,
            priority: 40,
        }),
        PresentKind::Food | PresentKind::FudgeSundae => pcm(PcmSound::ChowDown, 50, 0),
        PresentKind::TotalBummer => Some(AudioCue::Psg {
            command: 18,
            priority: 40,
        }),
        _ => None,
    };
    [cue, None]
}
