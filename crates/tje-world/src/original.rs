//! OriginalV1 stock-world algorithms. Random state follows the readable
//! StackBasedRandom/GetRandomNumber bodies, not ExpandedV1's domain derivation.
use crate::{Error, LevelId, Result, random::Random16};
use bincode::{Decode, Encode};

pub mod geometry;
mod growth;
pub mod items;
pub mod resources;
mod shapes;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum OriginalRevision {
    Rev00,
    Rev02,
}
impl From<tje_mem::Revision> for OriginalRevision {
    fn from(revision: tje_mem::Revision) -> Self {
        match revision {
            tje_mem::Revision::Rev00 => Self::Rev00,
            tje_mem::Revision::Rev02 => Self::Rev02,
        }
    }
}
impl OriginalRevision {
    pub const fn revision(self) -> tje_mem::Revision {
        match self {
            Self::Rev00 => tje_mem::Revision::Rev00,
            Self::Rev02 => tje_mem::Revision::Rev02,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct OriginalRng {
    state: i32,
}
impl OriginalRng {
    pub fn new(seed: u32) -> Self {
        let mut state = seed as i32;
        if state < 0 {
            state = state.wrapping_neg();
        }
        if state == i32::MAX {
            state = state.wrapping_sub(i32::MAX);
        }
        if state == 0 {
            state = 1;
        }
        Self { state }
    }
    pub const fn from_state(state: i32) -> Self {
        Self { state }
    }
    pub const fn state(self) -> i32 {
        self.state
    }
    pub fn reseed(&mut self, seed: u32) -> i32 {
        let prior = self.state;
        *self = Self::new(seed);
        prior
    }
    pub fn next_u16(&mut self) -> u16 {
        let high = self.state / 127_773;
        let low = self.state % 127_773;
        let next = 16_807_i32
            .wrapping_mul(low)
            .wrapping_sub(2_836_i32.wrapping_mul(high));
        self.state = if next > 0 {
            next
        } else {
            next.wrapping_add(i32::MAX)
        };
        self.state as u16
    }
    pub fn bounded(&mut self, bound: u32) -> u32 {
        assert!(bound > 0 && bound <= 65_536);
        u32::from(self.next_u16()) % bound
    }
}
impl Random16 for OriginalRng {
    fn next_u16(&mut self) -> u16 {
        Self::next_u16(self)
    }
}

/// The original 26 seed words; final floor 26 is authored and has no seed slot.
/// The bank does not assign camera/player records as GenerateLevelSeeds did.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct CampaignSeeds {
    words: [u16; 26],
}
impl CampaignSeeds {
    pub fn random(rng: &mut OriginalRng) -> Self {
        Self {
            words: std::array::from_fn(|_| rng.next_u16()),
        }
    }
    pub fn fixed(resources: &crate::rom::RomResources<'_>) -> Result<Self> {
        let mut words = [0; 26];
        for (level, word) in words.iter_mut().enumerate() {
            *word =
                resources.fixed_seed(LevelId::new(level as u16).expect("campaign seed index"))?;
        }
        Ok(Self { words })
    }
    pub fn from_words(words: [u16; 26]) -> Self {
        Self { words }
    }
    pub fn get(&self, level: LevelId) -> Result<u16> {
        self.words
            .get(level.index())
            .copied()
            .ok_or(Error::Bounds("authored final floor has no campaign seed"))
    }
    pub fn replace_accepted(&mut self, level: LevelId, seed: u16) -> Result<()> {
        *self
            .words
            .get_mut(level.index())
            .ok_or(Error::Bounds("campaign accepted seed index"))? = seed;
        Ok(())
    }
    pub const fn words(&self) -> &[u16; 26] {
        &self.words
    }
}
