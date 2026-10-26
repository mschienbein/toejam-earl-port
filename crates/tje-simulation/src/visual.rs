//! Logical animation continuations, independent of renderer/cache residency.
//! Frame IDs are derived from the saved cursor and immutable imported program.
use crate::{ActorId, AnimationId, EffectId, Error, LevelId, Result};
use bincode::{Decode, Encode};

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ProgramContract {
    /// Number of selectable frame entries, excluding the terminal hold/loop.
    pub selections: u16,
    pub frame_indices: Vec<u8>,
    pub flips: [u64; 4],
    pub period: Option<u16>,
}
impl ProgramContract {
    pub fn validate(&self) -> Result<()> {
        if self.selections == 0
            || self.selections > 256
            || self.period.is_some_and(|period| period > 255)
            || self.frame_indices.len() != usize::from(self.selections)
            || self.frame_indices.iter().any(|&frame| frame >= 128)
        {
            return Err(Error::State("invalid imported animation contract"));
        }
        for index in usize::from(self.selections)..256 {
            if self.flips[index / 64] & (1 << (index % 64)) != 0 {
                return Err(Error::State(
                    "animation flip contract exceeds selectable entries",
                ));
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct EarthlingBinding {
    pub kind: u8,
    pub state: u8,
    pub animation: AnimationId,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Encode, Decode)]
pub struct AnimationContracts {
    pub programs: Vec<ProgramContract>,
    pub heroes: [Vec<AnimationId>; 2],
    pub earthlings: Vec<EarthlingBinding>,
}
impl AnimationContracts {
    pub fn validate(&self) -> Result<()> {
        if self.programs.len() > 4096
            || self.heroes.iter().any(|hero| hero.len() > 121)
            || self.earthlings.len() > 25 * 17
            || self
                .earthlings
                .windows(2)
                .any(|pair| (pair[0].kind, pair[0].state) >= (pair[1].kind, pair[1].state))
        {
            return Err(Error::State("animation catalogue extent/order"));
        }
        for program in &self.programs {
            program.validate()?;
        }
        for &id in self.heroes.iter().flatten() {
            self.program(id)?;
        }
        for entry in &self.earthlings {
            if entry.kind >= 25 || (entry.state >= 16 && entry.state != 99) {
                return Err(Error::State("animation actor binding"));
            }
            self.program(entry.animation)?;
        }
        Ok(())
    }
    pub fn identity(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"tje-simulation/animation-contracts/1\0");
        hash.update((self.programs.len() as u16).to_be_bytes());
        for program in &self.programs {
            hash.update(program.selections.to_be_bytes());
            hash.update(&program.frame_indices);
            hash.update([u8::from(program.period.is_some())]);
            hash.update(program.period.unwrap_or(0).to_be_bytes());
            for flips in program.flips {
                hash.update(flips.to_be_bytes());
            }
        }
        for heroes in &self.heroes {
            hash.update((heroes.len() as u16).to_be_bytes());
            for animation in heroes {
                hash.update(animation.get().to_be_bytes());
            }
        }
        hash.update((self.earthlings.len() as u16).to_be_bytes());
        for entry in &self.earthlings {
            hash.update([entry.kind, entry.state]);
            hash.update(entry.animation.get().to_be_bytes());
        }
        hash.finalize().into()
    }
    pub fn program(&self, id: AnimationId) -> Result<&ProgramContract> {
        if !id.valid() {
            return Err(Error::State("animation identity"));
        }
        self.programs
            .get(usize::from(id.get()))
            .ok_or(Error::State("animation program is absent"))
    }
    pub fn validate_cursor(&self, cursor: &Playback) -> Result<()> {
        let contract = self.program(cursor.animation)?;
        if cursor.period > 255
            || contract
                .period
                .is_some_and(|period| period != cursor.period)
            || cursor.sequence_index >= contract.selections
            || cursor.ticks_left > cursor.period
        {
            return Err(Error::State("saved animation cursor extent/period"));
        }
        if !cursor.started {
            if cursor.sequence_index != 0
                || cursor.ticks_left != 0
                || cursor.held
                || cursor.flip_horizontal
            {
                return Err(Error::State("unstarted animation cursor has advanced"));
            }
        } else {
            let index = usize::from(cursor.sequence_index);
            let flip = contract.flips[index / 64] & (1 << (index % 64)) != 0;
            if cursor.flip_horizontal != flip || cursor.held != (cursor.ticks_left == 0) {
                return Err(Error::State("saved animation selection/hold mismatch"));
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Playback {
    pub animation: AnimationId,
    pub period: u16,
    pub sequence_index: u16,
    pub ticks_left: u16,
    pub held: bool,
    pub flip_horizontal: bool,
    pub started: bool,
}
impl Playback {
    pub fn new(animation: AnimationId, period: u16) -> Self {
        Self {
            animation,
            period,
            sequence_index: 0,
            ticks_left: 0,
            held: false,
            flip_horizontal: false,
            started: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ActorPlayback {
    pub actor: ActorId,
    pub playback: Playback,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct EffectPlayback {
    pub effect: EffectId,
    pub playback: Playback,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PropPlayback {
    pub level: LevelId,
    pub id: u16,
    pub playback: Playback,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct VisualState {
    pub companions: Vec<ActorPlayback>,
    pub effects: Vec<EffectPlayback>,
    pub props: Vec<PropPlayback>,
    pub last_tick: Option<u64>,
}
impl Default for VisualState {
    fn default() -> Self {
        Self {
            companions: Vec::with_capacity(64),
            effects: Vec::with_capacity(64),
            props: Vec::with_capacity(28),
            last_tick: None,
        }
    }
}
impl VisualState {
    pub fn validate(&self, contracts: &AnimationContracts) -> Result<()> {
        if self.companions.len() > 64
            || self.effects.len() > 64
            || self.props.len() > 28
            || self
                .companions
                .windows(2)
                .any(|pair| pair[0].actor >= pair[1].actor)
            || self
                .effects
                .windows(2)
                .any(|pair| pair[0].effect >= pair[1].effect)
            || self
                .props
                .windows(2)
                .any(|pair| (pair[0].level, pair[0].id) >= (pair[1].level, pair[1].id))
        {
            return Err(Error::State("visual continuation extent/order"));
        }
        for entry in &self.companions {
            if !entry.actor.valid() {
                return Err(Error::State("companion animation identity"));
            }
            contracts.validate_cursor(&entry.playback)?;
        }
        for entry in &self.effects {
            if !entry.effect.valid() {
                return Err(Error::State("effect animation identity"));
            }
            contracts.validate_cursor(&entry.playback)?;
        }
        for entry in &self.props {
            entry
                .level
                .validate()
                .map_err(|_| Error::State("prop animation level"))?;
            contracts.validate_cursor(&entry.playback)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contracts() -> AnimationContracts {
        AnimationContracts {
            programs: vec![ProgramContract {
                selections: 2,
                frame_indices: vec![0, 1],
                flips: [2, 0, 0, 0],
                period: Some(2),
            }],
            heroes: [vec![AnimationId::new(0).unwrap()], vec![]],
            earthlings: vec![],
        }
    }
    #[test]
    fn saved_cursor_validates_actual_program_selection_and_timer() {
        let contracts = contracts();
        contracts.validate().unwrap();
        let mut cursor = Playback::new(AnimationId::new(0).unwrap(), 2);
        contracts.validate_cursor(&cursor).unwrap();
        cursor.started = true;
        cursor.ticks_left = 2;
        contracts.validate_cursor(&cursor).unwrap();
        cursor.flip_horizontal = true;
        assert!(contracts.validate_cursor(&cursor).is_err());
        cursor.sequence_index = 1;
        contracts.validate_cursor(&cursor).unwrap();
        cursor.ticks_left = 0;
        cursor.held = true;
        contracts.validate_cursor(&cursor).unwrap();
        cursor.sequence_index = 2;
        assert!(contracts.validate_cursor(&cursor).is_err());
        cursor.sequence_index = 1;
        cursor.animation = AnimationId::new(1).unwrap();
        assert!(contracts.validate_cursor(&cursor).is_err());
    }
    #[test]
    fn visual_continuations_are_bounded_unique_and_roundtrip_without_frames() {
        let mut visual = VisualState::default();
        let effect = EffectPlayback {
            effect: EffectId::new(0, 1).unwrap(),
            playback: Playback::new(AnimationId::new(0).unwrap(), 2),
        };
        visual.effects.push(effect.clone());
        visual.validate(&contracts()).unwrap();
        let bytes = bincode::encode_to_vec(&visual, bincode::config::standard()).unwrap();
        let (decoded, _): (VisualState, _) =
            bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert_eq!(visual, decoded);
        visual.effects.push(effect);
        assert!(visual.validate(&contracts()).is_err());
        let mut bad = contracts();
        bad.programs[0].flips[1] = 1;
        assert!(bad.validate().is_err());
    }
}
