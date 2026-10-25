use crate::{GENERATOR_VERSION, LevelId};
use bincode::{Decode, Encode};
use sha2::{Digest, Sha256};

const MODULUS: u32 = 2_147_483_647;

/// Park–Miller/Schrage, with a full 31-bit internal state and the original's
/// low-sixteen-bit public draw. No faithful RNG/cache is read or written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct StageRng {
    state: u32,
}
impl StageRng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: (seed % u64::from(MODULUS - 1)) as u32 + 1,
        }
    }
    pub fn derive(identity: &[u8; 32], level: LevelId, attempt: u8, stage: &[u8]) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"tje-world/stage/1\0");
        hash.update(GENERATOR_VERSION.to_be_bytes());
        hash.update(identity);
        hash.update(level.get().to_be_bytes());
        hash.update([attempt]);
        hash.update((stage.len() as u16).to_be_bytes());
        hash.update(stage);
        let digest = hash.finalize();
        Self::new(u64::from_be_bytes(
            digest[..8].try_into().expect("eight digest bytes"),
        ))
    }
    pub const fn state(self) -> u32 {
        self.state
    }
    pub fn valid(self) -> bool {
        self.state != 0 && self.state < MODULUS
    }
    pub fn next_u16(&mut self) -> u16 {
        let next =
            16807_i64 * i64::from(self.state % 127773) - 2836_i64 * i64::from(self.state / 127773);
        self.state = if next <= 0 {
            (next + i64::from(MODULUS)) as u32
        } else {
            next as u32
        };
        self.state as u16
    }
    /// Deliberately specified modulo reduction; draws are never rejection loops.
    pub fn bounded(&mut self, bound: u32) -> u32 {
        assert!(bound > 0 && bound <= 65536);
        u32::from(self.next_u16()) % bound
    }
}
