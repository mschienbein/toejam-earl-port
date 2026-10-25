use crate::{Error, Result};
use bincode::{Decode, Encode};
use tje_four_player::input::InputFrame;

/// Original loop's relative four-phase order; initial preincrement selects 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
#[repr(u8)]
pub enum Phase {
    Players = 0,
    PresentationA = 1,
    Population = 2,
    PresentationB = 3,
}
impl Phase {
    pub const fn index(self) -> usize {
        self as usize
    }
    pub const fn next(self) -> Self {
        match self {
            Self::Players => Self::PresentationA,
            Self::PresentationA => Self::Population,
            Self::Population => Self::PresentationB,
            Self::PresentationB => Self::Players,
        }
    }
    pub const fn advances_motion(self) -> bool {
        matches!(self, Self::Players | Self::Population)
    }
    pub const fn publishes(self) -> bool {
        matches!(self, Self::PresentationA | Self::PresentationB)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct PublicationStamp {
    pub hardware_frame: u64,
    pub phase: Phase,
    pub epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Repeated,
    Step(PublicationStamp),
}

/// Saved admission state, never driven by redraws, wall time, or audio polls.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct PhaseClock {
    setup_complete: bool,
    next: Phase,
    epoch: u64,
    last: [Option<u64>; 4],
    input: Option<InputFrame>,
}
impl Default for PhaseClock {
    fn default() -> Self {
        Self {
            setup_complete: false,
            next: Phase::PresentationA,
            epoch: 0,
            last: [None; 4],
            input: None,
        }
    }
}
impl PhaseClock {
    pub const fn setup_complete(&self) -> bool {
        self.setup_complete
    }
    pub const fn next(&self) -> Phase {
        self.next
    }
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn input(&self) -> Option<&InputFrame> {
        self.input.as_ref()
    }
    pub fn last_admission(&self, phase: Phase) -> Option<u64> {
        self.last[phase.index()]
    }
    pub(crate) fn complete_setup(&mut self) -> Result<()> {
        if self.setup_complete || self.epoch != 0 {
            return Err(Error::State("setup may commit exactly once"));
        }
        self.setup_complete = true;
        Ok(())
    }
    /// Validates everything before changing admission or the latched input.
    pub fn admit(&mut self, phase: Phase, input: InputFrame) -> Result<Admission> {
        input.validate().map_err(|_| Error::State("invalid phase input"))?;
        if !self.setup_complete {
            return Err(Error::State("world phase entered before setup commit"));
        }
        if self.last[phase.index()] == Some(input.frame) {
            if self.input == Some(input) {
                return Ok(Admission::Repeated);
            }
            return Err(Error::State("repeated phase changed its input latch"));
        }
        if phase != self.next {
            return Err(Error::State("world phase entered out of relative order"));
        }
        if self.input.is_some_and(|previous| input.frame <= previous.frame) {
            return Err(Error::State("world phases require increasing hardware frames"));
        }
        let epoch = self.epoch.checked_add(1).ok_or(Error::Capacity("phase epoch"))?;
        self.last[phase.index()] = Some(input.frame);
        self.input = Some(input);
        self.next = phase.next();
        self.epoch = epoch;
        Ok(Admission::Step(PublicationStamp {
            hardware_frame: input.frame,
            phase,
            epoch,
        }))
    }
    pub fn validate(&self) -> Result<()> {
        let expected = match self.epoch % 4 {
            0 => Phase::PresentationA,
            1 => Phase::Population,
            2 => Phase::PresentationB,
            _ => Phase::Players,
        };
        if self.next != expected || (!self.setup_complete && self.epoch != 0) {
            return Err(Error::State("phase schedule/setup mismatch"));
        }
        if self.epoch == 0 {
            if self.input.is_some() || self.last.iter().any(Option::is_some) {
                return Err(Error::State("unstarted phase has admission/input state"));
            }
            return Ok(());
        }
        let input = self.input.ok_or(Error::State("phase input latch is absent"))?;
        input.validate().map_err(|_| Error::State("invalid saved phase input"))?;
        let count = self.epoch.min(4) as usize;
        let mut latest = None;
        for offset in 0..4 {
            let index = ((self.epoch % 4) as usize + 4 - offset) % 4;
            let frame = self.last[index];
            if offset >= count {
                if frame.is_some() {
                    return Err(Error::State("phase admitted before its first schedule slot"));
                }
                continue;
            }
            let frame = frame.ok_or(Error::State("saved phase admission is absent"))?;
            if offset == 0 && frame != input.frame {
                return Err(Error::State("phase frame and input latch disagree"));
            }
            if latest.is_some_and(|newer| frame >= newer) {
                return Err(Error::State("saved phase admissions are not ordered"));
            }
            latest = Some(frame);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_schedule_and_duplicate_entry_are_saved() {
        let mut clock = PhaseClock::default();
        assert!(clock.admit(Phase::PresentationA, InputFrame::neutral(7)).is_err());
        clock.complete_setup().unwrap();
        for (index, phase) in [Phase::PresentationA, Phase::Population, Phase::PresentationB, Phase::Players]
            .into_iter().cycle().take(12).enumerate()
        {
            let input = InputFrame::neutral(index as u64 + 7);
            assert!(matches!(clock.admit(phase, input).unwrap(), Admission::Step(_)));
            let saved = clock.clone();
            assert_eq!(clock.admit(phase, input).unwrap(), Admission::Repeated);
            assert_eq!(clock, saved);
            clock.validate().unwrap();
            let encoded = bincode::encode_to_vec(&clock, bincode::config::standard()).unwrap();
            let (decoded, consumed): (PhaseClock, _) = bincode::decode_from_slice(&encoded, bincode::config::standard()).unwrap();
            assert_eq!(consumed, encoded.len());
            assert_eq!(clock, decoded);
        }
    }

    #[test]
    fn invalid_admission_does_not_change_the_latch() {
        let mut clock = PhaseClock::default();
        clock.complete_setup().unwrap();
        clock.admit(Phase::PresentationA, InputFrame::neutral(11)).unwrap();
        let before = clock.clone();
        assert!(clock.admit(Phase::Population, InputFrame::neutral(11)).is_err());
        assert!(clock.admit(Phase::Players, InputFrame::neutral(12)).is_err());
        let changed = InputFrame::new(11, [0x100, 0, 0, 0], [0; 4]).unwrap();
        assert!(clock.admit(Phase::PresentationA, changed).is_err());
        assert_eq!(clock, before);
    }
}
