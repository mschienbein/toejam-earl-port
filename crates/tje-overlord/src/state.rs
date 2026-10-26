use crate::{
    ActorId, Config, DIRECTOR, Error, HEROES, Kind, LevelId, MAX_COMMANDS, MAX_SPAWNS, MAX_TRAPS,
    PopulationId, Result, SeatId, SeatMask,
    command::{Command, Direction, Envelope, PadLatch},
    config::*,
    placement::Point,
    supported,
    traps::{self, Trap, TrapKind},
};
use bincode::{Decode, Encode};
use tje_four_player::{CoopState, state::LifeState};
use tje_world::random::StageRng;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum MatchResult {
    Running,
    Heroes,
    DirectorExhaustion,
    DirectorTime,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Tool {
    Select,
    Devil,
    Dentist,
    DentistVariant,
    Lawnmower,
    NerdHerd,
    Snare,
    Bump,
}
impl Tool {
    fn next(self) -> Self {
        match self {
            Self::Select => Self::Devil,
            Self::Devil => Self::Dentist,
            Self::Dentist => Self::DentistVariant,
            Self::DentistVariant => Self::Lawnmower,
            Self::Lawnmower => Self::NerdHerd,
            Self::NerdHerd => Self::Snare,
            Self::Snare => Self::Bump,
            Self::Bump => Self::Select,
        }
    }
    pub fn placement(self, level: LevelId, point: Point) -> Option<Command> {
        let kind = match self {
            Self::Devil => Kind::Devil,
            Self::Dentist => Kind::Dentist,
            Self::DentistVariant => Kind::DentistVariant,
            Self::Lawnmower => Kind::Lawnmower,
            Self::NerdHerd => Kind::NerdHerd,
            Self::Snare => {
                return Some(Command::PlaceTrap {
                    kind: TrapKind::Snare,
                    level,
                    point,
                });
            }
            Self::Bump => {
                return Some(Command::PlaceTrap {
                    kind: TrapKind::Bump,
                    level,
                    point,
                });
            }
            Self::Select => return None,
        };
        Some(Command::PlaceSpawn { kind, level, point })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum SpawnStage {
    Telegraph,
    Active,
    Expired,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Spawn {
    pub source: u32,
    pub population: PopulationId,
    pub kind: Kind,
    pub point: Point,
    pub owner: SeatId,
    pub stage: SpawnStage,
    pub arm_left: u16,
    pub refunded: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PossessionInfo {
    pub kind: Kind,
    pub level: LevelId,
    pub enterable: bool,
    pub retainable: bool,
}
/// Implemented by the shared-world adapter: every operation is bounded and a
/// rejected reservation/activation/cancellation leaves that world unchanged.
/// A reservation is a real noncontacting, nonthinking population record, not a
/// render instance or a promise to allocate later. Stock/present records are not
/// director-owned and cannot be canceled by a forged source number.
pub trait World {
    fn possession(&self, actor: ActorId) -> Result<PossessionInfo>;
    fn validate_placement(
        &mut self,
        level: LevelId,
        point: Point,
        kind: Option<Kind>,
        ignore: Option<PopulationId>,
    ) -> Result<()>;
    fn reserve_spawn(
        &mut self,
        source: u32,
        kind: Kind,
        level: LevelId,
        point: Point,
    ) -> Result<PopulationId>;
    fn activate_spawn(&mut self, spawn: &Spawn) -> Result<()>;
    fn cancel_spawn(&mut self, spawn: &Spawn) -> Result<()>;
    fn spawn_alive(&self, spawn: &Spawn) -> bool;
    fn validate_spawn(&self, spawn: &Spawn) -> Result<()>;
    fn next_source(&self) -> u32;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Control {
    pub actor: ActorId,
    pub level: LevelId,
    pub direction: Direction,
    pub ability: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Receipt {
    pub sequence: u32,
    pub result: Result<()>,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct OverlordState {
    config: Config,
    identity: [u8; 32],
    pub energy: u16,
    pub energy_remainder: u8,
    pub possession_remainder: u8,
    pub spawn_cooldown: u16,
    pub possession_cooldown: u16,
    pub trap_cooldown: u16,
    pub selected: Option<ActorId>,
    pub control: Option<Control>,
    pub cursor_level: LevelId,
    pub cursor: Point,
    pub tool: Tool,
    pub body_view: bool,
    pub pause_requested: bool,
    pub pad: PadLatch,
    pub director_router: tje_four_player::input::Router,
    director_input: Option<tje_four_player::input::InputFrame>,
    pub elapsed: u64,
    pub result: MatchResult,
    pub collected: u32,
    pub spawns: [Option<Spawn>; MAX_SPAWNS],
    pub traps: [Option<Trap>; MAX_TRAPS],
    pub snares: [u16; 3],
    pending: [Option<Envelope>; MAX_COMMANDS],
    pending_len: u8,
    next_sequence: u32,
    last_input: Option<u64>,
    last_tick: Option<u64>,
    last_paused: bool,
    last_contacts: Option<u64>,
    last_commands: Option<u64>,
    next_source: u32,
    next_trap: u32,
    pub rng: StageRng,
}
impl OverlordState {
    pub fn new(config: Config) -> Result<Self> {
        config.validate()?;
        let identity = config.identity();
        let level = config
            .shared
            .course
            .as_ref()
            .map_or(LevelId::new(1).expect("first floor"), |course| {
                course.sources[0].level
            });
        let director_router = tje_four_player::input::Router::new(
            SeatMask::new(8).expect("director seat"),
            config.shared.coop.swap_ac,
        )
        .map_err(|_| Error::Role)?;
        let state = Self {
            rng: StageRng::derive(&identity, level, 0, b"director/1"),
            config,
            identity,
            energy: START_ENERGY,
            energy_remainder: 0,
            possession_remainder: 0,
            spawn_cooldown: 0,
            possession_cooldown: 0,
            trap_cooldown: 0,
            selected: None,
            control: None,
            cursor_level: level,
            cursor: Point { x: 128, y: 64 },
            tool: Tool::Select,
            body_view: false,
            pause_requested: false,
            pad: PadLatch::default(),
            director_input: None,
            director_router,
            elapsed: 0,
            result: MatchResult::Running,
            collected: 0,
            spawns: [None; MAX_SPAWNS],
            traps: [None; MAX_TRAPS],
            snares: [0; 3],
            pending: [None; MAX_COMMANDS],
            pending_len: 0,
            next_sequence: 0,
            last_input: None,
            last_tick: None,
            last_paused: false,
            last_contacts: None,
            last_commands: None,
            next_source: 1,
            next_trap: 1,
        };
        state.validate()?;
        Ok(state)
    }
    pub fn config(&self) -> &Config {
        &self.config
    }
    pub fn identity(&self) -> &[u8; 32] {
        &self.identity
    }
    pub fn next_sequence(&self) -> u32 {
        self.next_sequence
    }
    pub fn pending(&self) -> &[Option<Envelope>] {
        &self.pending[..usize::from(self.pending_len)]
    }
    pub fn last_tick(&self) -> Option<u64> {
        self.last_tick
    }
    pub fn validate(&self) -> Result<()> {
        self.config.validate()?;
        self.cursor.validate()?;
        self.cursor_level
            .validate()
            .map_err(|_| Error::Invalid("cursor level"))?;
        self.director_router.validate().map_err(|_| Error::Role)?;
        if self.director_router.enabled().bits() != 8
            || self.director_router.swap_ac() != self.config.shared.coop.swap_ac
        {
            return Err(Error::Invalid("director source router configuration"));
        }
        if self.identity != self.config.identity()
            || self.energy > ENERGY_CAP
            || self.energy_remainder >= ENERGY_PERIOD
            || self.possession_remainder >= ENERGY_PERIOD
            || self.spawn_cooldown > SPAWN_COOLDOWN
            || self.possession_cooldown > POSSESSION_COOLDOWN
            || self.trap_cooldown > TRAP_COOLDOWN
            || usize::from(self.pending_len) > MAX_COMMANDS
            || !self.rng.valid()
            || self.next_source == 0
            || self.next_trap == 0
            || self.snares.iter().any(|ticks| *ticks > SNARE_TICKS)
            || self.selected.is_some_and(|id| !id.valid())
            || self.control.is_some_and(|control| !control.actor.valid())
            || self.control.is_none() && self.possession_remainder != 0
            || self
                .last_contacts
                .is_some_and(|tick| self.last_tick.is_none_or(|last| tick > last))
            || self
                .last_commands
                .is_some_and(|tick| self.last_tick.is_none_or(|last| tick > last))
            || self
                .control
                .is_some_and(|control| control.level.validate().is_err())
            || self.result == MatchResult::Heroes
                && self.collected & self.config.preset.required() != self.config.preset.required()
        {
            return Err(Error::Invalid("director counters/configuration"));
        }
        if let Some(input) = self.director_input {
            input
                .validate()
                .map_err(|_| Error::Invalid("saved director input"))?;
            if input.actions[3] != 0 {
                return Err(Error::Invalid("unknown director action bits"));
            }
        }
        let mut previous_frame = self.last_input;
        for (index, item) in self.pending.iter().enumerate() {
            if index >= usize::from(self.pending_len) {
                if item.is_some() {
                    return Err(Error::Invalid("command queue tail"));
                }
                continue;
            }
            let envelope = item.ok_or(Error::Invalid("command queue hole"))?;
            envelope.validate()?;
            if self.next_sequence.checked_add(index as u32) != Some(envelope.sequence)
                || previous_frame.is_some_and(|frame| envelope.frame < frame)
            {
                return Err(Error::Order);
            }
            previous_frame = Some(envelope.frame);
        }
        for (index, spawn) in self
            .spawns
            .iter()
            .enumerate()
            .filter_map(|(index, spawn)| spawn.map(|spawn| (index, spawn)))
        {
            spawn.point.validate()?;
            spawn
                .population
                .level
                .validate()
                .map_err(|_| Error::Invalid("spawn level"))?;
            if spawn.owner != DIRECTOR
                || !supported(spawn.kind)
                || spawn.source == 0
                || spawn.source >= self.next_source
                || spawn.arm_left > TELEGRAPH_TICKS
                || spawn.stage == SpawnStage::Active && spawn.arm_left != 0
                || spawn.refunded && spawn.stage != SpawnStage::Expired
                || self.spawns[..index].iter().flatten().any(|other| {
                    other.source == spawn.source || other.population == spawn.population
                })
            {
                return Err(Error::Invalid("director spawn ledger"));
            }
        }
        for (index, trap) in self
            .traps
            .iter()
            .enumerate()
            .filter_map(|(index, trap)| trap.map(|trap| (index, trap)))
        {
            trap.validate()?;
            if trap.id >= self.next_trap
                || self.traps[..index]
                    .iter()
                    .flatten()
                    .any(|other| other.id == trap.id)
                || self
                    .traps
                    .iter()
                    .flatten()
                    .filter(|other| other.active() && other.level == trap.level)
                    .count()
                    > 3
            {
                return Err(Error::Invalid("trap IDs/per-level cap"));
            }
        }
        Ok(())
    }
    /// Validate cross-service references before aggregate capture or restore.
    pub fn validate_world(&self, coop: &CoopState, world: &impl World) -> Result<()> {
        self.validate()?;
        coop.validate().map_err(|_| Error::Shared)?;
        if coop.config() != &self.config.shared.coop
            || coop.players[3].life != LifeState::Disabled
            || coop.players[3].level.is_some()
            || coop.players[3].inventory.iter().any(Option::is_some)
        {
            return Err(Error::Role);
        }
        self.validate_sources(coop)?;
        if let Some(control) = self.control {
            let info = world.possession(control.actor)?;
            if !supported(info.kind) || !info.retainable || info.level != control.level {
                return Err(Error::StaleActor);
            }
        }
        for spawn in self
            .spawns
            .iter()
            .flatten()
            .filter(|spawn| spawn.stage != SpawnStage::Expired)
        {
            world.validate_spawn(spawn)?;
        }
        if self.next_source != world.next_source() {
            return Err(Error::Invalid("director/population source sequence"));
        }
        Ok(())
    }
    pub fn validate_session(
        &self,
        session: &tje_simulation::session::Session,
        world: &impl World,
    ) -> Result<()> {
        session.validate().map_err(|_| Error::Shared)?;
        if session.config() != &self.config.shared {
            return Err(Error::Invalid("aggregate/director configuration identity"));
        }
        self.validate_world(&session.players, world)
    }
    /// A duplicate is accepted only if byte-identical to the queued command. An
    /// already committed sequence cannot be reapplied or charged a second time.
    pub fn enqueue(&mut self, envelope: Envelope, owners: SeatMask) -> Result<bool> {
        envelope.validate()?;
        owners.validate().map_err(|_| Error::Role)?;
        if !owners.contains(DIRECTOR) {
            return Err(Error::Role);
        }
        if envelope.sequence < self.next_sequence {
            return Ok(false);
        }
        if self
            .last_commands
            .is_some_and(|frame| envelope.frame <= frame)
        {
            return Err(Error::Order);
        }
        let offset = envelope
            .sequence
            .checked_sub(self.next_sequence)
            .ok_or(Error::Order)? as usize;
        if offset < usize::from(self.pending_len) {
            return if self.pending[offset] == Some(envelope) {
                Ok(false)
            } else {
                Err(Error::Order)
            };
        }
        if offset != usize::from(self.pending_len) {
            return Err(Error::Order);
        }
        if offset == MAX_COMMANDS {
            return Err(Error::Full);
        }
        let previous_frame = if offset == 0 {
            self.last_input
        } else {
            self.pending[offset - 1].map(|entry| entry.frame)
        };
        if previous_frame.is_some_and(|frame| envelope.frame < frame) {
            return Err(Error::Order);
        }
        self.pending[offset] = Some(envelope);
        self.pending_len += 1;
        Ok(true)
    }
    pub fn enqueue_local(&mut self, envelope: Envelope) -> Result<bool> {
        self.enqueue(envelope, self.config.shared.roles.local_owners)
    }
    fn validate_sources(&self, coop: &CoopState) -> Result<()> {
        if coop.config() != &self.config.shared.coop || coop.router.enabled() != HEROES {
            return Err(Error::Role);
        }
        self.director_router.validate().map_err(|_| Error::Role)?;
        coop.router.validate().map_err(|_| Error::Role)?;
        if self
            .director_router
            .sources()
            .iter()
            .flatten()
            .any(|source| coop.router.sources().contains(&Some(*source)))
        {
            return Err(Error::Invalid(
                "one source cannot own both a hero and the director",
            ));
        }
        Ok(())
    }
    pub fn route_local_samples(
        &mut self,
        coop: &CoopState,
        frame: u64,
        samples: &[tje_four_player::input::SourceSample],
        hovered: Option<ActorId>,
    ) -> Result<Option<tje_four_player::input::InputFrame>> {
        self.validate_sources(coop)?;
        let mut input = coop.router.route(frame, samples).map_err(|_| Error::Role)?;
        let director = self
            .director_router
            .route(frame, samples)
            .map_err(|_| Error::Role)?;
        input.pads[3] = director.pads[3];
        input.actions[3] = director.actions[3];
        self.route_local_input(input, hovered)
    }
    /// Role-aware local latch. Returns only the three hero seats to S3, while
    /// preserving the complete four-pad frame and director edges here. This
    /// never constructs a hero, inventory or donation recipient for seat three.
    pub fn route_local_input(
        &mut self,
        input: tje_four_player::input::InputFrame,
        hovered: Option<ActorId>,
    ) -> Result<Option<tje_four_player::input::InputFrame>> {
        input
            .validate()
            .map_err(|_| Error::Invalid("director input frame"))?;
        if input.actions[3] != 0 {
            return Err(Error::Invalid("unknown director action bits"));
        }
        if !self.config.shared.roles.local_owners.contains(DIRECTOR) {
            return Err(Error::Role);
        }
        if SeatId::ALL.into_iter().any(|seat| {
            !self.config.shared.roles.local_owners.contains(seat)
                && input.pads[seat.index()].connected()
        }) {
            return Err(Error::Role);
        }
        if let Some(previous) = self.director_input {
            if previous == input {
                return Ok(None);
            }
            if previous.frame >= input.frame {
                return Err(Error::Order);
            }
        }
        if self.last_tick != Some(input.frame) || self.last_commands == Some(input.frame) {
            return Err(Error::Order);
        }
        let mut pad = self.pad;
        pad.next_sequence = self
            .next_sequence
            .checked_add(u32::from(self.pending_len))
            .ok_or(Error::Order)?;
        let mut generated = [None; 5];
        let count = pad.commands(
            crate::command::Controls {
                frame: input.frame,
                buttons: input.pads[3].game_byte(self.config.shared.coop.swap_ac[3]),
                connected: input.pads[3].connected(),
                selected: self.selected.or(hovered),
                placement: self.tool.placement(self.cursor_level, self.cursor),
                possessed: self.control.is_some(),
                body_view: self.body_view,
                pause_requested: self.pause_requested,
            },
            &mut generated,
        )?;
        if usize::from(self.pending_len) + count > MAX_COMMANDS {
            return Err(Error::Full);
        }
        if self.last_input.is_some_and(|last| input.frame < last)
            || self
                .pending()
                .last()
                .and_then(|entry| *entry)
                .is_some_and(|last| input.frame < last.frame)
        {
            return Err(Error::Order);
        }
        for envelope in generated.iter().flatten() {
            envelope.validate()?;
        }
        for envelope in generated.into_iter().flatten() {
            self.enqueue_local(envelope)?;
        }
        self.pad = pad;
        self.director_input = Some(input);
        let mut heroes = input;
        heroes.pads[3] = tje_four_player::input::PadWord::DISCONNECTED;
        heroes.actions[3] = 0;
        Ok(Some(heroes))
    }
    /// Only a shared decision receipt may consume an ability request. Merely
    /// drawing, colliding, or visiting a protected animation cannot consume it.
    pub fn acknowledge_ability(&mut self, actor: ActorId) -> Result<()> {
        let control = self.control.as_mut().ok_or(Error::StaleActor)?;
        if control.actor != actor {
            return Err(Error::StaleActor);
        }
        control.ability = false;
        Ok(())
    }
    /// Call once per admitted video frame. A paused frame records admission but
    /// advances neither team, cooldowns, effects, clock nor fractional budgets.
    pub fn video_tick(&mut self, frame: u64, paused: bool) -> Result<bool> {
        if self.last_tick == Some(frame) {
            return if self.last_paused == paused {
                Ok(false)
            } else {
                Err(Error::Order)
            };
        }
        if self
            .last_tick
            .is_some_and(|last| last.checked_add(1) != Some(frame))
        {
            return Err(Error::Order);
        }
        if !paused && self.result == MatchResult::Running {
            let elapsed = self.elapsed.checked_add(1).ok_or(Error::Order)?;
            self.elapsed = elapsed;
            self.energy_remainder += 1;
            if self.energy_remainder == ENERGY_PERIOD {
                self.energy_remainder = 0;
                self.energy = (self.energy + 1).min(ENERGY_CAP);
            }
            for timer in [
                &mut self.spawn_cooldown,
                &mut self.possession_cooldown,
                &mut self.trap_cooldown,
            ] {
                *timer = timer.saturating_sub(1);
            }
            for timer in &mut self.snares {
                *timer = timer.saturating_sub(1);
            }
            if self.control.is_some() {
                self.possession_remainder += 1;
                if self.possession_remainder == ENERGY_PERIOD {
                    self.possession_remainder = 0;
                    if self.energy == 0 {
                        self.release();
                    } else {
                        self.energy -= 1;
                    }
                }
            }
            for spawn in self
                .spawns
                .iter_mut()
                .flatten()
                .filter(|spawn| spawn.stage == SpawnStage::Telegraph)
            {
                spawn.arm_left = spawn.arm_left.saturating_sub(1);
            }
            for trap in self.traps.iter_mut().flatten().filter(|trap| trap.active()) {
                trap.life_left = trap.life_left.saturating_sub(1);
                trap.arm_left = trap.arm_left.saturating_sub(1);
                if trap.life_left == 0 {
                    trap.stage = traps::Stage::Expired;
                }
            }
        }
        self.last_tick = Some(frame);
        self.last_paused = paused;
        Ok(true)
    }
    /// Ordered command boundary immediately before shared population decisions.
    /// Rejected commands consume their sequence, but do not change gameplay IDs,
    /// cost, RNG, selected/control state or shared world. The receipt is local UI.
    pub fn commands(
        &mut self,
        frame: u64,
        world: &mut impl World,
        receipts: &mut [Option<Receipt>; MAX_COMMANDS],
    ) -> Result<usize> {
        if self.last_tick != Some(frame) {
            return Err(Error::Order);
        }
        receipts.fill(None);
        if self.last_commands == Some(frame) {
            return Ok(0);
        }
        let mut count = 0;
        while let Some(envelope) = self.pending.get(count).copied().flatten() {
            if envelope.frame > frame {
                break;
            }
            let result = if self.last_paused && !matches!(envelope.command, Command::Pause(_)) {
                Err(Error::Protected)
            } else {
                self.apply(envelope.command, world)
            };
            receipts[count] = Some(Receipt {
                sequence: envelope.sequence,
                result,
            });
            count += 1;
            self.last_input = Some(envelope.frame);
            self.next_sequence += 1;
        }
        let length = usize::from(self.pending_len);
        self.pending.copy_within(count..length, 0);
        self.pending[length - count..length].fill(None);
        self.pending_len -= count as u8;
        self.last_commands = Some(frame);
        Ok(count)
    }
    fn apply(&mut self, command: Command, world: &mut impl World) -> Result<()> {
        if self.result != MatchResult::Running {
            return Err(Error::Protected);
        }
        match command {
            Command::SelectActor(actor) => {
                let info = world.possession(actor)?;
                if !supported(info.kind) {
                    return Err(Error::UnsupportedKind);
                }
                self.selected = Some(actor);
            }
            Command::Possess(actor) => {
                if self.control.is_some() {
                    return Err(Error::Protected);
                }
                let info = world.possession(actor)?;
                if !supported(info.kind) {
                    return Err(Error::UnsupportedKind);
                }
                if !info.enterable {
                    return Err(Error::Protected);
                }
                self.afford(POSSESSION_COST, self.possession_cooldown)?;
                self.energy -= POSSESSION_COST;
                self.possession_cooldown = POSSESSION_COOLDOWN;
                self.control = Some(Control {
                    actor,
                    level: info.level,
                    direction: Direction::Neutral,
                    ability: false,
                });
                self.possession_remainder = 0;
                self.body_view = true;
            }
            Command::Release => self.release(),
            Command::Move(direction) => {
                if let Some(control) = self.control.as_mut().filter(|_| self.body_view) {
                    control.direction = direction;
                } else {
                    let (dx, dy) = direction.delta();
                    let point = Point::new(self.cursor.x + dx * 8, self.cursor.y + dy * 8)?;
                    self.cursor = point;
                }
            }
            Command::UseAbility => {
                let control = self.control.ok_or(Error::Protected)?;
                if !matches!(
                    world.possession(control.actor)?.kind,
                    Kind::Dentist | Kind::DentistVariant
                ) {
                    return Err(Error::UnsupportedKind);
                }
                self.control.as_mut().expect("checked possession").ability = true;
            }
            Command::PlaceSpawn { kind, level, point } => {
                let cost = spawn_cost(kind).ok_or(Error::UnsupportedKind)?;
                self.afford(cost, self.spawn_cooldown)?;
                let index = self
                    .spawns
                    .iter()
                    .position(|spawn| spawn.is_none_or(|spawn| spawn.stage == SpawnStage::Expired))
                    .ok_or(Error::Full)?;
                let next = self.next_source.checked_add(1).ok_or(Error::Full)?;
                self.excludes_pending(level, point)?;
                world.validate_placement(level, point, Some(kind), None)?;
                let population = world.reserve_spawn(self.next_source, kind, level, point)?;
                self.spawns[index] = Some(Spawn {
                    source: self.next_source,
                    population,
                    kind,
                    point,
                    owner: DIRECTOR,
                    stage: SpawnStage::Telegraph,
                    arm_left: TELEGRAPH_TICKS,
                    refunded: false,
                });
                self.next_source = next;
                self.energy -= cost;
                self.spawn_cooldown = SPAWN_COOLDOWN;
            }
            Command::PlaceTrap { kind, level, point } => {
                self.afford(kind.cost(), self.trap_cooldown)?;
                let index = self
                    .traps
                    .iter()
                    .position(|trap| trap.is_none_or(|trap| !trap.active()))
                    .ok_or(Error::Full)?;
                if self
                    .traps
                    .iter()
                    .flatten()
                    .filter(|trap| trap.active() && trap.level == level)
                    .count()
                    >= 3
                {
                    return Err(Error::Full);
                }
                let next = self.next_trap.checked_add(1).ok_or(Error::Full)?;
                self.excludes_pending(level, point)?;
                world.validate_placement(level, point, None, None)?;
                self.traps[index] = Some(Trap::new(self.next_trap, kind, level, point));
                self.next_trap = next;
                self.energy -= kind.cost();
                self.trap_cooldown = TRAP_COOLDOWN;
            }
            Command::CancelPlacement => self.tool = Tool::Select,
            Command::Cursor { level, point } => {
                point.validate()?;
                self.cursor = point;
                self.cursor_level = level;
            }
            Command::CycleTool => {
                self.tool = self.tool.next();
                if self.tool == Tool::Select && self.control.is_some() {
                    self.body_view = true;
                }
            }
            Command::ToggleView => {
                let control = self.control.as_mut().ok_or(Error::Protected)?;
                control.direction = Direction::Neutral;
                self.body_view = !self.body_view;
            }
            Command::Pause(value) => self.pause_requested = value,
        }
        Ok(())
    }
    fn afford(&self, cost: u16, cooldown: u16) -> Result<()> {
        if cooldown != 0 {
            Err(Error::Cooldown)
        } else if self.energy < cost {
            Err(Error::Energy)
        } else {
            Ok(())
        }
    }
    fn excludes_pending(&self, level: LevelId, point: Point) -> Result<()> {
        let near = |other: Point| {
            (i64::from(point.x) - i64::from(other.x)).abs() <= 128
                && (i64::from(point.y) - i64::from(other.y)).abs() <= 64
        };
        if self.spawns.iter().flatten().any(|spawn| {
            spawn.stage != SpawnStage::Expired
                && spawn.population.level == level
                && near(spawn.point)
        }) || self
            .traps
            .iter()
            .flatten()
            .any(|trap| trap.active() && trap.level == level && near(trap.point))
        {
            return Err(Error::Placement(crate::placement::Rejection::Occupied));
        }
        Ok(())
    }
    fn release(&mut self) {
        self.control = None;
        self.possession_remainder = 0;
        self.body_view = false;
    }
    /// Clears stale/out-of-interest possession without restoring old health,
    /// position, animation or timers. The shared thinker resumes at its next
    /// safe decision slot, not an immediate second decision on release.
    pub fn reconcile(&mut self, world: &impl World) {
        if self.control.is_some_and(|control| {
            world.possession(control.actor).map_or(true, |info| {
                !supported(info.kind) || !info.retainable || info.level != control.level
            })
        }) {
            self.release();
        }
        if self
            .selected
            .is_some_and(|actor| world.possession(actor).is_err())
        {
            self.selected = None;
        }
    }
    /// Revalidates all reservations immediately before activation. Invalid
    /// telegraphs cancel/refund once; live death/release never refunds energy.
    pub fn resolve_spawns(&mut self, world: &mut impl World) -> Result<()> {
        if self.last_paused || self.result != MatchResult::Running {
            return Ok(());
        }
        for spawn in self.spawns.iter_mut().flatten() {
            if spawn.stage == SpawnStage::Active && !world.spawn_alive(spawn) {
                spawn.stage = SpawnStage::Expired;
            }
            if spawn.stage != SpawnStage::Telegraph || spawn.arm_left != 0 {
                continue;
            }
            match world.validate_placement(
                spawn.population.level,
                spawn.point,
                Some(spawn.kind),
                Some(spawn.population),
            ) {
                Err(Error::Placement(_)) => {
                    world.cancel_spawn(spawn)?;
                    spawn.stage = SpawnStage::Expired;
                    spawn.refunded = true;
                    self.energy = (self.energy
                        + spawn_cost(spawn.kind).expect("validated capability"))
                    .min(ENERGY_CAP);
                }
                Err(error) => return Err(error),
                Ok(()) => {
                    world.activate_spawn(spawn)?;
                    spawn.stage = SpawnStage::Active;
                }
            }
        }
        Ok(())
    }
    pub fn resolve_traps(&mut self, frame: u64, sink: &mut impl traps::Sink) -> Result<bool> {
        if self.last_tick != Some(frame) || self.last_contacts.is_some_and(|last| frame < last) {
            return Err(Error::Order);
        }
        if self.last_contacts == Some(frame) {
            return Ok(false);
        }
        if !self.last_paused && self.result == MatchResult::Running {
            traps::resolve(&mut self.traps, &mut self.snares, sink)?;
        }
        self.last_contacts = Some(frame);
        Ok(true)
    }
    /// The aggregate calls this after objective/exit and death transactions, in
    /// that order. `exit` is a shared validated commit, not a director command.
    /// An exit beats terminal death and exact clock expiry in the same phase.
    pub fn arbitrate(
        &mut self,
        coop: &CoopState,
        objectives: u32,
        exit: Option<(SeatId, LevelId)>,
    ) -> Result<MatchResult> {
        if coop.config() != &self.config.shared.coop {
            return Err(Error::Role);
        }
        if self.result != MatchResult::Running {
            return Ok(self.result);
        }
        if objectives & self.collected != self.collected {
            return Err(Error::Invalid("objective progress moved backwards"));
        }
        if let Some((seat, level)) = exit
            && (!HEROES.contains(seat) || level != self.config.preset.exit())
        {
            return Err(Error::Invalid("exit receipt"));
        }
        self.collected = objectives;
        self.result = if exit.is_some()
            && objectives & self.config.preset.required() == self.config.preset.required()
        {
            MatchResult::Heroes
        } else if HEROES
            .iter()
            .all(|seat| coop.players[seat.index()].life == LifeState::Eliminated)
        {
            MatchResult::DirectorExhaustion
        } else if self
            .config
            .preset
            .time_limit()
            .is_some_and(|limit| self.elapsed >= limit)
        {
            MatchResult::DirectorTime
        } else {
            MatchResult::Running
        };
        if self.result != MatchResult::Running {
            self.release();
        }
        Ok(self.result)
    }
}

#[cfg(test)]
mod finite_counter_tests {
    use super::*;
    #[test]
    fn sequence_exhaustion_cannot_wrap_or_reapply_the_last_cost() {
        let mut fixture = crate::proof::Fixture::new(0);
        fixture.state.next_sequence = u32::MAX - 1;
        fixture.state.video_tick(0, false).unwrap();
        fixture.command(0, Command::CancelPlacement).unwrap();
        assert_eq!(fixture.state.next_sequence, u32::MAX);
        fixture.state.validate().unwrap();
        assert_eq!(
            fixture.state.enqueue_local(Envelope {
                seat: DIRECTOR,
                frame: 0,
                sequence: u32::MAX - 1,
                command: Command::CancelPlacement
            }),
            Ok(false)
        );
        let before = fixture.state.clone();
        assert_eq!(
            fixture.state.enqueue_local(Envelope {
                seat: DIRECTOR,
                frame: 1,
                sequence: u32::MAX,
                command: Command::CycleTool
            }),
            Err(Error::Order)
        );
        assert_eq!(fixture.state, before);
    }
    #[test]
    fn elapsed_overflow_rejects_before_any_timer_or_admission_mutates() {
        let mut state = OverlordState::new(crate::proof::config(0)).unwrap();
        state.elapsed = u64::MAX;
        let before = state.clone();
        assert_eq!(state.video_tick(0, false), Err(Error::Order));
        assert_eq!(state, before);
    }
}
