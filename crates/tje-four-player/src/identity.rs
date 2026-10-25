use crate::{Error, Result, SEATS};
use bincode::{Decode, Encode};

/// A seat, never a Genesis player selector (whose value 2 means BOTH).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Encode, Decode)]
pub struct SeatId(u8);
impl SeatId {
    pub const ALL: [Self; SEATS] = [Self(0), Self(1), Self(2), Self(3)];

    pub const fn new(value: u8) -> Option<Self> {
        if value < SEATS as u8 {
            Some(Self(value))
        } else {
            None
        }
    }
    pub const fn get(self) -> u8 {
        self.0
    }
    pub const fn index(self) -> usize {
        self.0 as usize
    }
    pub fn validate(self) -> Result<()> {
        if Self::new(self.0).is_some() {
            Ok(())
        } else {
            Err(Error::Invalid("seat must be in 0..4"))
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub struct SeatMask(u8);
impl SeatMask {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(15);

    pub const fn new(value: u8) -> Option<Self> {
        if value & !15 == 0 {
            Some(Self(value))
        } else {
            None
        }
    }
    pub const fn bits(self) -> u8 {
        self.0
    }
    pub const fn contains(self, seat: SeatId) -> bool {
        // Also safe for an unvalidated decoded SeatId.
        seat.0 < SEATS as u8 && self.0 & (1 << seat.0) != 0
    }
    pub fn iter(self) -> impl Iterator<Item = SeatId> {
        SeatId::ALL
            .into_iter()
            .filter(move |&seat| self.contains(seat))
    }
    pub fn validate(self) -> Result<()> {
        if Self::new(self.0).is_some() {
            Ok(())
        } else {
            Err(Error::Invalid("seat mask contains an unknown seat"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Archetype {
    ToeJam,
    Earl,
}
impl Archetype {
    pub const fn base_health(self) -> u8 {
        match self {
            Self::ToeJam => 23,
            Self::Earl => 31,
        }
    }
    pub const fn dimensions(self) -> (u8, u8) {
        match self {
            Self::ToeJam => (24, 32),
            Self::Earl => (30, 40),
        }
    }
    /// Rank is a gameplay rank, not the original eliminated-player sentinel 9.
    pub fn maximum_health(self, rank: u8) -> Result<u8> {
        if rank > 8 {
            return Err(Error::Invalid("rank must be in 0..=8"));
        }
        Ok(self.base_health() + 4 * rank)
    }
    /// Resource identity only; neither a host pointer nor a player ABI argument.
    pub fn animation_descriptors(self, revision: tje_rom::Revision) -> u32 {
        match self {
            Self::ToeJam => tje_mem::rom::player_animation_descriptors(revision).address(),
            Self::Earl => tje_mem::rom::player2_animation_descriptors(revision).address(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum PaletteVariant {
    Original,
    RotateForward,
    RotateBackward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Character {
    pub archetype: Archetype,
    pub palette: PaletteVariant,
}
impl Character {
    pub const DEFAULT_ROSTER: [Self; SEATS] = [
        Self {
            archetype: Archetype::ToeJam,
            palette: PaletteVariant::Original,
        },
        Self {
            archetype: Archetype::Earl,
            palette: PaletteVariant::Original,
        },
        Self {
            archetype: Archetype::ToeJam,
            palette: PaletteVariant::RotateForward,
        },
        Self {
            archetype: Archetype::Earl,
            palette: PaletteVariant::RotateBackward,
        },
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seats_and_masks_do_not_alias_the_original_both_selector() {
        for seat in SeatId::ALL {
            let mask = SeatMask::new(1 << seat.get()).unwrap();
            assert_eq!(mask.iter().collect::<Vec<_>>(), [seat]);
            assert_eq!(SeatId::new(seat.get()), Some(seat));
        }
        assert_eq!(SeatMask::NONE.iter().count(), 0);
        assert_eq!(SeatMask::ALL.iter().count(), 4);
        assert!(SeatId::new(4).is_none());
        assert!(SeatId::new(255).is_none());
        assert!(SeatMask::new(16).is_none());
        assert!(SeatMask::new(255).is_none());
        assert_eq!(SeatId::new(2).unwrap().index(), 2);
    }

    #[test]
    fn archetype_not_seat_or_palette_owns_stats_and_resources() {
        for archetype in [Archetype::ToeJam, Archetype::Earl] {
            for seat in SeatId::ALL {
                let mut roster = Character::DEFAULT_ROSTER;
                roster[seat.index()].archetype = archetype;
                assert_eq!(
                    roster[seat.index()].archetype.maximum_health(0).unwrap(),
                    archetype.base_health()
                );
            }
            assert_eq!(
                archetype.maximum_health(8).unwrap(),
                archetype.base_health() + 32
            );
            assert!(archetype.maximum_health(9).is_err());
        }
        for revision in tje_rom::Revision::ALL {
            assert_ne!(
                Archetype::ToeJam.animation_descriptors(revision),
                Archetype::Earl.animation_descriptors(revision)
            );
        }
    }

    #[test]
    fn decoding_is_followed_by_invariant_validation() {
        let bytes = bincode::encode_to_vec(SeatId(9), bincode::config::standard()).unwrap();
        let (seat, _): (SeatId, _) =
            bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert!(seat.validate().is_err());
        assert!(!SeatMask::ALL.contains(seat));
        assert!(SeatMask(128).validate().is_err());
    }
}
