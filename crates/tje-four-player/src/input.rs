use crate::{Error, Result, SEATS, SeatId, SeatMask};
use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

pub const INPUT_SCHEMA: u32 = 1;
const MAX_SAMPLES: usize = 16;

/// The existing session wire order, NOT the game's processed-input byte order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct PadWord(u16);
impl PadWord {
    pub const DISCONNECTED: Self = Self(0);

    pub fn new(word: u16) -> Result<Self> {
        if word & !0x1ff != 0 || (word & tje_session::CONNECTED == 0 && word != 0) {
            return Err(Error::Invalid(
                "pad must be disconnected or a connected three-button word",
            ));
        }
        Ok(Self(word))
    }
    pub const fn bits(self) -> u16 {
        self.0
    }
    pub const fn connected(self) -> bool {
        self.0 & tje_session::CONNECTED != 0
    }
    pub fn validate(self) -> Result<()> {
        Self::new(self.0).map(|_| ())
    }
    /// RAM uses directions/B/C/A/Start; the wire uses directions/A/B/C/Start.
    pub fn game_byte(self, swap_ac: bool) -> u8 {
        if !self.connected() {
            return 0;
        }
        let mut wire = self.0 as u8;
        if swap_ac {
            wire = (wire & !0x50) | ((wire & 0x10) << 2) | ((wire & 0x40) >> 2);
        }
        (wire & 0x8f) | ((wire & 0x20) >> 1) | ((wire & 0x40) >> 1) | ((wire & 0x10) << 2)
    }
}

/// A separate enhanced input schema; faithful tje-session records stay two-pad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Deserialize, Serialize)]
#[serde(try_from = "FrameFields", into = "FrameFields")]
pub struct InputFrame {
    pub frame: u64,
    pub pads: [PadWord; SEATS],
    /// Profile-defined actions. A disconnected seat always has zero actions.
    pub actions: [u16; SEATS],
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FrameFields {
    schema: u32,
    frame: u64,
    pads: [u16; SEATS],
    actions: [u16; SEATS],
}
impl TryFrom<FrameFields> for InputFrame {
    type Error = Error;
    fn try_from(fields: FrameFields) -> Result<Self> {
        if fields.schema != INPUT_SCHEMA {
            return Err(Error::Invalid("unsupported enhanced input schema"));
        }
        Self::new(fields.frame, fields.pads, fields.actions)
    }
}
impl From<InputFrame> for FrameFields {
    fn from(frame: InputFrame) -> Self {
        Self {
            schema: INPUT_SCHEMA,
            frame: frame.frame,
            pads: frame.pads.map(PadWord::bits),
            actions: frame.actions,
        }
    }
}
impl InputFrame {
    pub fn neutral(frame: u64) -> Self {
        Self {
            frame,
            pads: [PadWord::DISCONNECTED; SEATS],
            actions: [0; SEATS],
        }
    }
    pub fn new(frame: u64, pads: [u16; SEATS], actions: [u16; SEATS]) -> Result<Self> {
        let mut result = Self::neutral(frame);
        for (destination, source) in result.pads.iter_mut().zip(pads) {
            *destination = PadWord::new(source)?;
        }
        result.actions = actions;
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<()> {
        for (&pad, &actions) in self.pads.iter().zip(&self.actions) {
            pad.validate()?;
            if !pad.connected() && actions != 0 {
                return Err(Error::Invalid("disconnected pad has enhanced actions"));
            }
        }
        Ok(())
    }
    pub fn game_bytes(&self, swap_ac: [bool; SEATS]) -> [u8; SEATS] {
        std::array::from_fn(|index| self.pads[index].game_byte(swap_ac[index]))
    }
    #[cfg(feature = "enhanced-mode")]
    pub fn hardware_inputs(&self) -> Result<tje_engine_runtime::enhanced::EnhancedInputs> {
        use genesis_config::{GenesisButton, GenesisController, GenesisControllerType};
        const BUTTONS: [GenesisButton; 8] = [
            GenesisButton::Up,
            GenesisButton::Down,
            GenesisButton::Left,
            GenesisButton::Right,
            GenesisButton::A,
            GenesisButton::B,
            GenesisButton::C,
            GenesisButton::Start,
        ];
        self.validate()?;
        Ok(tje_engine_runtime::enhanced::EnhancedInputs {
            pads: self.pads.map(|pad| {
                if !pad.connected() {
                    return GenesisController::None;
                }
                let mut controller = GenesisController::new(GenesisControllerType::ThreeButton);
                for (bit, button) in BUTTONS.into_iter().enumerate() {
                    controller.set_field(button, pad.bits() & (1 << bit) != 0);
                }
                controller
            }),
        })
    }
}

/// Stable frontend-assigned identity, not the index returned by an OS poll.
/// Keyboard binding profiles can use their own identities in the same table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct SourceId(pub [u8; 16]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSample {
    pub source: SourceId,
    pub pad: PadWord,
    pub actions: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Router {
    enabled: SeatMask,
    sources: [Option<SourceId>; SEATS],
    swap_ac: [bool; SEATS],
    assignment_epoch: u64,
}
impl Router {
    pub fn new(enabled: SeatMask, swap_ac: [bool; SEATS]) -> Result<Self> {
        enabled.validate()?;
        Ok(Self {
            enabled,
            sources: [None; SEATS],
            swap_ac,
            assignment_epoch: 0,
        })
    }
    pub fn enabled(&self) -> SeatMask {
        self.enabled
    }
    pub fn sources(&self) -> &[Option<SourceId>; SEATS] {
        &self.sources
    }
    pub fn swap_ac(&self) -> [bool; SEATS] {
        self.swap_ac
    }
    pub fn assignment_epoch(&self) -> u64 {
        self.assignment_epoch
    }
    pub fn validate(&self) -> Result<()> {
        self.enabled.validate()?;
        Self::validate_sources(self.enabled, &self.sources)
    }
    fn validate_sources(enabled: SeatMask, sources: &[Option<SourceId>; SEATS]) -> Result<()> {
        for (index, source) in sources.iter().enumerate() {
            if source.is_some() && !enabled.contains(SeatId::ALL[index]) {
                return Err(Error::Invalid("input source assigned to a disabled seat"));
            }
            if source.is_some() && sources[..index].contains(source) {
                return Err(Error::Invalid("one input source cannot own two seats"));
            }
        }
        Ok(())
    }
    /// One explicit lobby event, including swaps. Validate the complete candidate
    /// before changing any assignment; unplugging never invokes this operation.
    pub fn reassign(&mut self, changes: &[(SeatId, Option<SourceId>)]) -> Result<()> {
        if changes.is_empty() || changes.len() > SEATS {
            return Err(Error::Invalid(
                "assignment event must name one to four seats",
            ));
        }
        let mut candidate = self.sources;
        let mut seen = 0_u8;
        for &(seat, source) in changes {
            seat.validate()?;
            if seen & (1 << seat.get()) != 0 {
                return Err(Error::Invalid("assignment event repeats a seat"));
            }
            seen |= 1 << seat.get();
            candidate[seat.index()] = source;
        }
        Self::validate_sources(self.enabled, &candidate)?;
        let epoch = self
            .assignment_epoch
            .checked_add(1)
            .ok_or(Error::Invalid("assignment epoch overflow"))?;
        self.sources = candidate;
        self.assignment_epoch = epoch;
        Ok(())
    }
    /// Poll order is irrelevant. Missing/disconnected sources produce neutral
    /// input without transferring ownership or forgetting reconnect identities.
    pub fn route(&self, frame: u64, samples: &[SourceSample]) -> Result<InputFrame> {
        self.validate()?;
        if samples.len() > MAX_SAMPLES {
            return Err(Error::Invalid("too many simultaneous input sources"));
        }
        for (index, sample) in samples.iter().enumerate() {
            sample.pad.validate()?;
            if samples[..index]
                .iter()
                .any(|other| other.source == sample.source)
            {
                return Err(Error::Invalid("poll contains a duplicate source identity"));
            }
            if !sample.pad.connected() && sample.actions != 0 {
                return Err(Error::Invalid("disconnected source has enhanced actions"));
            }
        }
        let mut result = InputFrame::neutral(frame);
        for seat in self.enabled.iter() {
            if let Some(source) = self.sources[seat.index()]
                && let Some(sample) = samples.iter().find(|sample| sample.source == source)
            {
                result.pads[seat.index()] = sample.pad;
                result.actions[seat.index()] = sample.actions;
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tje_session::{A, B, C, CONNECTED, START};

    fn samples() -> [SourceSample; SEATS] {
        std::array::from_fn(|index| SourceSample {
            source: SourceId([index as u8; 16]),
            pad: PadWord::new(CONNECTED | (1 << index)).unwrap(),
            actions: 0x100 << index,
        })
    }
    fn assigned() -> Router {
        let mut router = Router::new(SeatMask::ALL, [false; SEATS]).unwrap();
        for (seat, sample) in SeatId::ALL.into_iter().zip(samples()) {
            router.reassign(&[(seat, Some(sample.source))]).unwrap();
        }
        router
    }

    #[test]
    fn every_wire_bit_and_all_button_combinations_convert_explicitly() {
        let expected = [1, 2, 4, 8, 64, 16, 32, 128];
        let swapped = [1, 2, 4, 8, 32, 16, 64, 128];
        for word in 0..=255_u16 {
            let pad = PadWord::new(CONNECTED | word).unwrap();
            for (swap, map) in [(false, expected), (true, swapped)] {
                let byte = (0..8)
                    .filter(|bit| word & (1 << bit) != 0)
                    .fold(0, |byte, bit| byte | map[bit]);
                assert_eq!(pad.game_byte(swap), byte);
            }
        }
        assert_eq!(PadWord::DISCONNECTED.game_byte(true), 0);
        for invalid in [A, B, C, START, 0x200, 0xffff] {
            assert!(PadWord::new(invalid).is_err());
        }
    }

    #[test]
    fn all_four_sources_route_independently_of_poll_order_and_reconnect() {
        let router = assigned();
        let mut polled = samples();
        let frame = router.route(7, &polled).unwrap();
        polled.reverse();
        assert_eq!(router.route(7, &polled).unwrap(), frame);
        let before = router.clone();
        let disconnected = router.route(8, &polled[..3]).unwrap();
        assert_eq!(disconnected.pads[0], PadWord::DISCONNECTED);
        assert_eq!(disconnected.actions[0], 0);
        assert_eq!(router, before);
        assert_eq!(router.route(9, &polled).unwrap().pads, frame.pads);
        for index in 0..SEATS {
            assert_eq!(frame.pads[index].bits(), CONNECTED | (1 << index));
            assert_eq!(frame.actions[index], 0x100 << index);
        }
    }

    #[test]
    fn explicit_assignment_swaps_are_atomic_and_never_duplicate_ownership() {
        let mut router = assigned();
        let before = router.clone();
        assert!(
            router
                .reassign(&[(SeatId::ALL[0], before.sources[1])])
                .is_err()
        );
        assert_eq!(router, before);
        assert!(
            router
                .reassign(&[(SeatId::ALL[0], None), (SeatId::ALL[0], None)])
                .is_err()
        );
        assert_eq!(router, before);
        router
            .reassign(&[
                (SeatId::ALL[0], before.sources[1]),
                (SeatId::ALL[1], before.sources[0]),
            ])
            .unwrap();
        assert_eq!(router.assignment_epoch(), before.assignment_epoch() + 1);
        let routed = router.route(0, &samples()).unwrap();
        assert_eq!(routed.pads[0], samples()[1].pad);
        let mut one = Router::new(SeatMask::new(1).unwrap(), [false; SEATS]).unwrap();
        assert!(
            one.reassign(&[(SeatId::ALL[2], Some(SourceId([7; 16])))])
                .is_err()
        );
    }

    #[test]
    fn bad_polls_and_disconnected_actions_reject() {
        let router = assigned();
        assert!(router.route(0, &[samples()[0], samples()[0]]).is_err());
        assert!(
            router
                .route(
                    0,
                    &[SourceSample {
                        source: SourceId([9; 16]),
                        pad: PadWord::DISCONNECTED,
                        actions: 1
                    }]
                )
                .is_err()
        );
        assert!(router.route(0, &[samples()[0]; MAX_SAMPLES + 1]).is_err());
        assert!(InputFrame::new(0, [0; SEATS], [1; SEATS]).is_err());
    }

    #[test]
    fn enhanced_wire_schema_is_strict_and_never_reinterprets_faithful_records() {
        let frame = assigned().route(42, &samples()).unwrap();
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(serde_json::from_str::<InputFrame>(&json).unwrap(), frame);
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value["schema"] = 2.into();
        assert!(serde_json::from_value::<InputFrame>(value).is_err());
        assert!(serde_json::from_str::<InputFrame>(r#"{"frames":1,"pads":[256,256]}"#).is_err());
        assert!(
            serde_json::from_str::<InputFrame>(
                r#"{"schema":1,"frame":0,"pads":[256,256,256,256],"actions":[0,0,0,0],"extra":0}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<InputFrame>(
                r#"{"schema":1,"frame":0,"pads":[1,256,256,256],"actions":[0,0,0,0]}"#
            )
            .is_err()
        );
    }

    #[cfg(feature = "enhanced-mode")]
    #[test]
    fn enhanced_driver_gets_four_pads_but_original_hardware_only_two() {
        use genesis_config::{GenesisController, GenesisControllerType};
        for bit in 0..8 {
            let wire = CONNECTED | (1 << bit);
            let input = InputFrame::new(0, [wire; SEATS], [0; SEATS])
                .unwrap()
                .hardware_inputs()
                .unwrap();
            assert_eq!(input.pads[0], input.original().p1);
            assert_eq!(input.pads[1], input.original().p2);
            assert!(input.pads.iter().all(|pad| *pad == input.pads[0]));
            assert_ne!(
                input.pads[3],
                GenesisController::new(GenesisControllerType::ThreeButton)
            );
            let GenesisController::ThreeButton(state) = input.pads[3] else {
                panic!("fourth pad was not routed as a three-button controller");
            };
            let buttons = [
                state.up,
                state.down,
                state.left,
                state.right,
                state.a,
                state.b,
                state.c,
                state.start,
            ];
            assert_eq!(buttons, std::array::from_fn(|index| index == bit));
        }
        assert_eq!(
            InputFrame::neutral(0).hardware_inputs().unwrap().pads,
            [GenesisController::None; SEATS]
        );
        let mut invalid = InputFrame::neutral(0);
        invalid.pads[3] = PadWord(0x300);
        assert!(invalid.hardware_inputs().is_err());
    }
}
