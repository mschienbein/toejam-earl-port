use crate::{
    Character, Error, INVENTORY_SLOTS, LevelId, RULESET_VERSION, Result, SEATS, SeatId, SeatMask,
    input::{InputFrame, Router},
};
use bincode::{Decode, Encode};
use sha1::{Digest, Sha1};
use tje_world::MAX_FLOORS;

/// Immutable new-session identity. Constructing data is not admission to play.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Config {
    pub ruleset: u32,
    pub participants: SeatMask,
    pub roster: [Character; SEATS],
    pub world_seed: u64,
    pub swap_ac: [bool; SEATS],
}
impl Config {
    pub fn four_seats(world_seed: u64) -> Self {
        Self {
            ruleset: RULESET_VERSION,
            participants: SeatMask::ALL,
            roster: Character::DEFAULT_ROSTER,
            world_seed,
            swap_ac: [false; SEATS],
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.participants.validate()?;
        if self.ruleset != RULESET_VERSION {
            return Err(Error::Invalid("unsupported co-op ruleset"));
        }
        if self.participants == SeatMask::NONE {
            return Err(Error::Invalid(
                "a co-op session needs at least one participating seat",
            ));
        }
        Ok(())
    }
    pub fn identity(&self) -> [u8; 20] {
        let mut hash = Sha1::new();
        hash.update(b"tje-four-player/config/1\0");
        hash.update(self.ruleset.to_be_bytes());
        hash.update([self.participants.bits()]);
        hash.update(self.world_seed.to_be_bytes());
        for character in self.roster {
            hash.update([match character.archetype {
                crate::Archetype::ToeJam => 0,
                crate::Archetype::Earl => 1,
            }]);
            hash.update([match character.palette {
                crate::PaletteVariant::Original => 0,
                crate::PaletteVariant::RotateForward => 1,
                crate::PaletteVariant::RotateBackward => 2,
            }]);
        }
        hash.update(self.swap_ac.map(u8::from));
        hash.finalize().into()
    }
}

/// Signed 24.8 positions. The bound leaves headroom for checked i64 distances
/// and covers the shared world's maximum 64x64 metatile envelope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct Position {
    pub x: i32,
    pub y: i32,
    pub elevation: i32,
}
impl Position {
    pub const LIMIT: u32 = 32767 * 256;
    pub fn validate(self) -> Result<()> {
        if [self.x, self.y, self.elevation]
            .into_iter()
            .any(|coordinate| coordinate.unsigned_abs() > Self::LIMIT)
        {
            return Err(Error::Invalid(
                "24.8 position exceeds the supported world envelope",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct Motion {
    pub position: Position,
    pub velocity: [i32; 3],
    pub acceleration: [i32; 3],
}
impl Motion {
    pub fn validate(&self) -> Result<()> {
        self.position.validate()?;
        if self
            .velocity
            .iter()
            .chain(&self.acceleration)
            .any(|value| value.unsigned_abs() > 64 * 256)
        {
            return Err(Error::Invalid(
                "motion exceeds the bounded actor speed/acceleration envelope",
            ));
        }
        Ok(())
    }
}

/// Wrapping identity is separate from effect identity in the shared present map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct PresentWrapper(u8);
impl PresentWrapper {
    pub const fn new(value: u8) -> Option<Self> {
        if value < 28 { Some(Self(value)) } else { None }
    }
    pub const fn get(self) -> u8 {
        self.0
    }
    pub fn validate(self) -> Result<()> {
        Self::new(self.0)
            .map(|_| ())
            .ok_or(Error::Invalid("present wrapper must be in 0..28"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum LifeState {
    Disabled,
    Unplaced,
    Alive,
    Eliminated,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub enum FallState {
    #[default]
    None,
    Descending,
    Landing,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub enum Overlay {
    #[default]
    Closed,
    Inventory,
    MailOrder,
    Identify,
    Map,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Choice {
    Accept,
    Decline,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Encode, Decode)]
pub struct SeatUi {
    pub overlay: Overlay,
    pub selected_slot: u8,
    pub choice: Option<Choice>,
    pub action_release_latch: bool,
    pub pause_requested: bool,
    pub life_recipient: Option<SeatId>,
}

/// The original byte-counter storage is explicit, not accidentally Earl's state.
/// Future generalized algorithms must state when a byte intentionally wraps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Encode, Decode)]
pub struct PlayerCounters {
    pub jump: u8,
    pub fall: FallState,
    pub terrain_sinking: u8,
    pub phased_jump_phase: u8,
    pub phased_jump_ticks: u8,
    pub phased_jump_armed: bool,
    pub phased_jump_speech_used: bool,
    pub pending_hurt_damage: u8,
    pub landing_seed: u8,
    pub rocket_turn_delay: u8,
    pub book_sleep_delay: u8,
    pub wake_effects_left: u8,
    pub idle_ticks: u16,
    pub hula_dance_ticks: u8,
    pub hula_contact_delay: u8,
    pub respawn_suppression_ticks: u8,
    pub active_present_ticks: u16,
    pub controls_remap_ticks: u16,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Encode, Decode)]
pub struct AnimationState {
    pub descriptor: u8,
    pub sequence_index: u16,
    pub ticks_left: u8,
    pub flip_horizontal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PlayerState {
    pub seat: SeatId,
    pub character: Character,
    pub life: LifeState,
    pub level: Option<LevelId>,
    pub motion: Motion,
    pub health: u8,
    pub lives: u8,
    pub rank: u8,
    pub points: u16,
    pub bucks: u8,
    pub inventory: [Option<PresentWrapper>; INVENTORY_SLOTS],
    pub active_present: Option<PresentWrapper>,
    pub counters: PlayerCounters,
    pub animation: AnimationState,
    pub terrain: [[u8; 5]; 5],
    pub ui: SeatUi,
    pub spectating: Option<SeatId>,
}
impl PlayerState {
    fn new(seat: SeatId, character: Character, participating: bool) -> Self {
        Self {
            seat,
            character,
            life: if participating {
                LifeState::Unplaced
            } else {
                LifeState::Disabled
            },
            level: None,
            motion: Motion::default(),
            health: character.archetype.base_health(),
            lives: 0,
            rank: 0,
            points: 0,
            bucks: 0,
            inventory: [None; INVENTORY_SLOTS],
            active_present: None,
            counters: PlayerCounters::default(),
            animation: AnimationState::default(),
            terrain: [[0; 5]; 5],
            ui: SeatUi::default(),
            spectating: None,
        }
    }
    fn validate(&self) -> Result<()> {
        self.seat.validate()?;
        self.motion.validate()?;
        if self.health > self.character.archetype.maximum_health(self.rank)? {
            return Err(Error::Invalid(
                "health exceeds this archetype/rank's maximum",
            ));
        }
        if self.lives > 127 || self.bucks > 127 {
            return Err(Error::Invalid(
                "original signed-byte resource domain exceeded",
            ));
        }
        if self.animation.descriptor > 120 {
            return Err(Error::Invalid("unknown archetype animation descriptor"));
        }
        for present in self
            .inventory
            .iter()
            .copied()
            .chain([self.active_present])
            .flatten()
        {
            present.validate()?;
        }
        if self.ui.selected_slot as usize >= INVENTORY_SLOTS {
            return Err(Error::Invalid("inventory selection is out of bounds"));
        }
        if let Some(level) = self.level {
            level
                .validate()
                .map_err(|_| Error::Invalid("invalid level identity"))?;
        }
        if (self.life == LifeState::Alive) != self.level.is_some() {
            return Err(Error::Invalid(
                "only an alive placed actor may retain a level reference",
            ));
        }
        if let Some(target) = self.spectating {
            target.validate()?;
            if self.life != LifeState::Eliminated || target == self.seat {
                return Err(Error::Invalid(
                    "spectating requires an eliminated owner and a different target",
                ));
            }
        }
        if let Some(recipient) = self.ui.life_recipient {
            recipient.validate()?;
            if recipient == self.seat {
                return Err(Error::Invalid("life donation cannot target its own seat"));
            }
        }
        if self.life == LifeState::Disabled
            && (self.points != 0
                || self.bucks != 0
                || self.lives != 0
                || self.rank != 0
                || self.health != self.character.archetype.base_health()
                || self.inventory.iter().any(Option::is_some)
                || self.active_present.is_some()
                || self.ui != SeatUi::default()
                || self.counters != PlayerCounters::default()
                || self.motion != Motion::default()
                || self.animation != AnimationState::default()
                || self.terrain != [[0; 5]; 5])
        {
            return Err(Error::Invalid(
                "disabled seat retains gameplay resources or activity",
            ));
        }
        Ok(())
    }
}

/// Seat-local history only. Geometry, fog, land edits, item claims, removals and
/// authoritative populations belong to the ONE shared tje-world LevelId record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct LevelSeatProgress {
    pub arrivals: [u32; SEATS],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arrival {
    pub seat: SeatId,
    pub level: LevelId,
    pub position: Position,
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct CoopState {
    config: Config,
    configuration_identity: [u8; 20],
    pub players: [PlayerState; SEATS],
    pub levels: [LevelSeatProgress; MAX_FLOORS],
    pub router: Router,
    pub inputs: Option<InputFrame>,
    /// Last claimant rotates at encounter ticks, never with viewport iteration.
    pub claim_priority: SeatId,
}
impl CoopState {
    pub fn new(config: Config) -> Result<Self> {
        config.validate()?;
        let configuration_identity = config.identity();
        let players = std::array::from_fn(|index| {
            PlayerState::new(
                SeatId::ALL[index],
                config.roster[index],
                config.participants.contains(SeatId::ALL[index]),
            )
        });
        let router = Router::new(config.participants, config.swap_ac)?;
        let claim_priority = config
            .participants
            .iter()
            .next()
            .ok_or(Error::Invalid("missing participant"))?;
        let state = Self {
            config,
            configuration_identity,
            players,
            levels: [LevelSeatProgress::default(); MAX_FLOORS],
            router,
            inputs: None,
            claim_priority,
        };
        state.validate()?;
        Ok(state)
    }
    pub fn config(&self) -> &Config {
        &self.config
    }
    pub fn configuration_identity(&self) -> &[u8; 20] {
        &self.configuration_identity
    }
    pub fn validate(&self) -> Result<()> {
        self.config.validate()?;
        if self.configuration_identity != self.config.identity() {
            return Err(Error::Invalid(
                "configuration digest does not match immutable data",
            ));
        }
        self.router.validate()?;
        if self.router.enabled() != self.config.participants
            || self.router.swap_ac() != self.config.swap_ac
        {
            return Err(Error::Invalid(
                "input routing differs from immutable session settings",
            ));
        }
        self.claim_priority.validate()?;
        if !self.config.participants.contains(self.claim_priority) {
            return Err(Error::Invalid(
                "claim priority names a nonparticipating seat",
            ));
        }
        for (index, player) in self.players.iter().enumerate() {
            player.validate()?;
            if player.seat != SeatId::ALL[index] || player.character != self.config.roster[index] {
                return Err(Error::Invalid(
                    "actor identity/archetype/palette differs from its roster seat",
                ));
            }
            if (player.life != LifeState::Disabled)
                != self.config.participants.contains(player.seat)
            {
                return Err(Error::Invalid(
                    "actor activity differs from participating seats",
                ));
            }
            if let Some(level) = player.level
                && self.levels[level.index()].arrivals[index] == 0
            {
                return Err(Error::Invalid(
                    "placed actor has no level-ID arrival history",
                ));
            }
            if let Some(target) = player.spectating
                && self.players[target.index()].life != LifeState::Alive
            {
                return Err(Error::Invalid("spectator target is not a surviving actor"));
            }
            if let Some(recipient) = player.ui.life_recipient
                && !self.config.participants.contains(recipient)
            {
                return Err(Error::Invalid("life recipient is not a participating seat"));
            }
            if !self.config.participants.contains(player.seat)
                && self.levels.iter().any(|level| level.arrivals[index] != 0)
            {
                return Err(Error::Invalid("disabled seat has level history"));
            }
        }
        if let Some(inputs) = self.inputs {
            inputs.validate()?;
            for seat in SeatId::ALL {
                if !self.config.participants.contains(seat)
                    && (inputs.pads[seat.index()].connected() || inputs.actions[seat.index()] != 0)
                {
                    return Err(Error::Invalid("disabled seat has latched input"));
                }
            }
        }
        Ok(())
    }
    /// Bookkeeping primitive, NOT an elevator/fall implementation. The future
    /// world coordinator must first prepare the destination's shared LevelId
    /// record, validate geometry/spawns, then commit both sidecars atomically.
    /// No generation, collision, damage or progress effects are fabricated here.
    pub fn record_arrivals(&mut self, arrivals: &[Arrival]) -> Result<()> {
        self.validate()?;
        if arrivals.is_empty() || arrivals.len() > SEATS {
            return Err(Error::Invalid(
                "arrival transaction needs one to four seats",
            ));
        }
        let mut seen = 0_u8;
        for arrival in arrivals {
            arrival.seat.validate()?;
            arrival
                .level
                .validate()
                .map_err(|_| Error::Invalid("invalid arrival level"))?;
            arrival.position.validate()?;
            if seen & (1 << arrival.seat.get()) != 0 {
                return Err(Error::Invalid("arrival transaction repeats a seat"));
            }
            seen |= 1 << arrival.seat.get();
            if !matches!(
                self.players[arrival.seat.index()].life,
                LifeState::Unplaced | LifeState::Alive
            ) {
                return Err(Error::Invalid(
                    "arrival cannot silently enable or revive an actor",
                ));
            }
            self.levels[arrival.level.index()].arrivals[arrival.seat.index()]
                .checked_add(1)
                .ok_or(Error::Invalid("arrival counter overflow"))?;
        }
        for arrival in arrivals {
            let player = &mut self.players[arrival.seat.index()];
            player.level = Some(arrival.level);
            player.life = LifeState::Alive;
            player.motion = Motion {
                position: arrival.position,
                ..Motion::default()
            };
            self.levels[arrival.level.index()].arrivals[arrival.seat.index()] += 1;
        }
        Ok(())
    }
    pub fn latch_inputs(&mut self, inputs: InputFrame) -> Result<()> {
        inputs.validate()?;
        let expected = match self.inputs {
            None => 0,
            Some(previous) => previous
                .frame
                .checked_add(1)
                .ok_or(Error::Invalid("input frame overflow"))?,
        };
        if inputs.frame != expected {
            return Err(Error::Invalid(
                "input frames must be latched once, consecutively, from frame zero",
            ));
        }
        for seat in SeatId::ALL {
            if !self.config.participants.contains(seat) && inputs.pads[seat.index()].connected() {
                return Err(Error::Invalid("input supplied for a disabled seat"));
            }
        }
        self.inputs = Some(inputs);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> CoopState {
        CoopState::new(Config::four_seats(0x1234)).unwrap()
    }
    fn arrival(seat: usize, level: u16) -> Arrival {
        Arrival {
            seat: SeatId::ALL[seat],
            level: LevelId::new(level).unwrap(),
            position: Position {
                x: 100 * 256,
                y: 200 * 256,
                elevation: 0,
            },
        }
    }
    #[test]
    fn four_distinct_levels_and_revisits_follow_identity_not_camera_slot() {
        let mut state = state();
        state
            .record_arrivals(&[arrival(0, 1), arrival(1, 2), arrival(2, 3), arrival(3, 4)])
            .unwrap();
        state.players[2].inventory[15] = PresentWrapper::new(27);
        state.players[3].points = 500;
        for _ in 0..5 {
            state
                .record_arrivals(&[arrival(3, 1), arrival(2, 2), arrival(1, 3), arrival(0, 4)])
                .unwrap();
            state
                .record_arrivals(&[arrival(0, 1), arrival(1, 2), arrival(2, 3), arrival(3, 4)])
                .unwrap();
        }
        assert_eq!(state.levels[1].arrivals, [6, 0, 0, 5]);
        assert_eq!(state.levels[4].arrivals, [5, 0, 0, 6]);
        assert_eq!(state.players[2].inventory[15].unwrap().get(), 27);
        assert_eq!(state.players[3].points, 500);
        state.validate().unwrap();
        let bytes = bincode::encode_to_vec(&state, bincode::config::standard()).unwrap();
        let (restored, used): (CoopState, _) =
            bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        restored.validate().unwrap();
        assert_eq!(used, bytes.len());
        assert_eq!(restored, state);
        assert!(bytes.len() < 16 * 1024);
    }
    #[test]
    fn arrival_transactions_reject_without_any_partial_mutation() {
        let mut state = state();
        let before = state.clone();
        let mut bad = arrival(1, 2);
        bad.position.x = i32::MIN;
        assert!(state.record_arrivals(&[arrival(0, 1), bad]).is_err());
        assert_eq!(state, before);
        assert!(
            state
                .record_arrivals(&[arrival(0, 1), arrival(0, 2)])
                .is_err()
        );
        assert_eq!(state, before);
        state.levels[2].arrivals[1] = u32::MAX;
        let before = state.clone();
        assert!(
            state
                .record_arrivals(&[arrival(0, 1), arrival(1, 2)])
                .is_err()
        );
        assert_eq!(state, before);
        state.players[1].life = LifeState::Eliminated;
        assert!(state.record_arrivals(&[arrival(1, 1)]).is_err());
    }
    #[test]
    fn roster_identity_is_immutable_and_stats_are_archetype_owned_in_every_seat() {
        for seat in SeatId::ALL {
            for archetype in [crate::Archetype::ToeJam, crate::Archetype::Earl] {
                let mut config = Config::four_seats(5);
                config.roster[seat.index()].archetype = archetype;
                let state = CoopState::new(config).unwrap();
                assert_eq!(state.players[seat.index()].health, archetype.base_health());
                state.validate().unwrap();
            }
        }
        let mut state = state();
        state.config.world_seed ^= 1;
        assert!(state.validate().is_err());
        let mut state = self::state();
        state.configuration_identity[0] ^= 1;
        assert!(state.validate().is_err());
    }
    #[test]
    fn seat_state_invariants_cover_resources_inventory_latches_and_level_references() {
        let original = state();
        let corruptions: [fn(&mut CoopState); 11] = [
            |state| state.players[0].seat = SeatId::ALL[3],
            |state| state.players[2].health = 255,
            |state| state.players[3].rank = 9,
            |state| state.players[0].lives = 128,
            |state| state.players[1].inventory[0] = Some(PresentWrapper(28)),
            |state| state.players[2].ui.selected_slot = 16,
            |state| state.players[3].animation.descriptor = 121,
            |state| state.players[0].level = LevelId::new(1),
            |state| state.players[1].ui.life_recipient = Some(SeatId::ALL[1]),
            |state| state.players[2].spectating = Some(SeatId::ALL[0]),
            |state| state.players[3].motion.velocity[0] = i32::MAX,
        ];
        for corrupt in corruptions {
            let mut state = original.clone();
            corrupt(&mut state);
            assert!(state.validate().is_err());
        }
    }
    #[test]
    fn disabled_seats_never_gain_resources_through_routing_or_arrival() {
        let mut config = Config::four_seats(6);
        config.participants = SeatMask::new(3).unwrap();
        let mut state = CoopState::new(config).unwrap();
        assert_eq!(state.players[3].life, LifeState::Disabled);
        let before = state.clone();
        assert!(state.record_arrivals(&[arrival(2, 1)]).is_err());
        assert_eq!(state, before);
        assert!(
            state
                .latch_inputs(InputFrame::new(0, [256; SEATS], [0; SEATS]).unwrap())
                .is_err()
        );
        assert_eq!(state, before);
        state.players[2].inventory[0] = PresentWrapper::new(1);
        assert!(state.validate().is_err());
    }
    #[test]
    fn frame_latching_is_once_only_and_snapshottable() {
        let mut state = state();
        let frame = InputFrame::neutral(0);
        state.latch_inputs(frame).unwrap();
        let before = state.clone();
        assert!(state.latch_inputs(frame).is_err());
        assert!(state.latch_inputs(InputFrame::neutral(2)).is_err());
        assert_eq!(state, before);
        state.latch_inputs(InputFrame::neutral(1)).unwrap();
        state.validate().unwrap();
    }
}
