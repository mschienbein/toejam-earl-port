//! Shared, bounded world data for explicitly selected extras.
//!
//! This crate never resizes the cartridge's RAM arrays. ExpandedV1 is a new
//! ruleset, not the original generator with a changed stride. Native admission
//! is deliberately closed until all world consumers have mode-owned storage.
#![forbid(unsafe_code)]

use bincode::{Decode, Encode};
use std::fmt;

#[cfg(feature = "enhanced-mode")]
pub mod enhanced;
pub mod generation;
pub mod geometry;
pub mod land;
pub mod loading;
pub mod original;
pub mod population;
pub mod random;
pub mod rom;
pub mod terrain;
pub mod transitions;
pub mod world;
pub use geometry::{Anchors, Cell, CellPosition, Dimensions, LevelGeometry, Resources, Shape};

pub const MAX_FLOORS: usize = 32;
pub const MAX_ACTIVE_FLOORS: usize = 4;
pub const MAX_ENTITIES: usize = 1024;
pub const MAX_EARTHLINGS: usize = 64;
pub const MAX_ITEMS: usize = 128;
pub const MAX_DROPPED_ITEMS: usize = 128;
pub const MAX_EFFECTS: usize = 64;
pub const MAX_ATTEMPTS: u8 = 64;
pub const MAX_STATE_BYTES: usize = 4 * 1024 * 1024;
pub const GENERATOR_VERSION: u16 = 1;
pub const GENERATOR_ID: &str = "ExpandedV1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum GeneratorId {
    OriginalV1,
    ExpandedV1,
    DesignedV1,
}
impl GeneratorId {
    pub const fn tag(self) -> u8 {
        match self {
            Self::OriginalV1 => 0,
            Self::ExpandedV1 => 1,
            Self::DesignedV1 => 2,
        }
    }
    pub const fn version(self) -> u16 {
        1
    }
}

/// Shared campaign/Maker identity; unrelated to a physical A/B/C map bank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Encode, Decode)]
pub struct LevelId(u16);
impl LevelId {
    pub const fn new(value: u16) -> Option<Self> {
        if value < MAX_FLOORS as u16 {
            Some(Self(value))
        } else {
            None
        }
    }
    pub const fn get(self) -> u16 {
        self.0
    }
    pub const fn index(self) -> usize {
        self.0 as usize
    }
    pub const fn ordinary(self) -> bool {
        self.0 >= 2 && self.0 <= 25
    }
    pub fn validate(self) -> Result<()> {
        Self::new(self.0)
            .map(|_| ())
            .ok_or(Error::Bounds("level identity"))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub enum LevelSize {
    #[default]
    Original,
    Expanded,
    Large,
    Custom(Dimensions),
}
impl LevelSize {
    pub const fn dimensions(self) -> Dimensions {
        match self {
            Self::Original => Dimensions::ORIGINAL,
            Self::Expanded => Dimensions::EXPANDED,
            Self::Large => Dimensions::LARGE,
            Self::Custom(dimensions) => dimensions,
        }
    }
}

/// Independent of size, presentation width, viewport count and hero count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub enum Density {
    #[default]
    Original,
    Low,
    Normal,
    Busy,
}
impl Density {
    pub const fn percent(self) -> u32 {
        match self {
            Self::Original | Self::Normal => 100,
            Self::Low => 75,
            Self::Busy => 125,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct WorldConfig {
    pub root_seed: u64,
    pub rom_sha1: [u8; 20],
    pub size: LevelSize,
    pub density: Density,
}
impl WorldConfig {
    pub fn validate(&self) -> Result<()> {
        self.size.dimensions().validate()
    }
    pub fn faithful(&self) -> bool {
        self.size == LevelSize::Original && self.density == Density::Original
    }
    pub fn dimensions_for(&self, level: LevelId) -> Dimensions {
        if level.ordinary() {
            self.size.dimensions()
        } else {
            Dimensions::ORIGINAL
        }
    }
    pub fn identity(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"tje-world/config/1\0");
        hash.update(GENERATOR_VERSION.to_be_bytes());
        hash.update(self.root_seed.to_be_bytes());
        hash.update(self.rom_sha1);
        hash.update([match self.size {
            LevelSize::Original => 0,
            LevelSize::Expanded => 1,
            LevelSize::Large => 2,
            LevelSize::Custom(_) => 3,
        }]);
        let dimensions = self.size.dimensions();
        hash.update(dimensions.width().to_be_bytes());
        hash.update(dimensions.height().to_be_bytes());
        hash.update([match self.density {
            Density::Original => 0,
            Density::Low => 1,
            Density::Normal => 2,
            Density::Busy => 3,
        }]);
        hash.finalize().into()
    }
}

/// Call this *before* profile registration, machine reset, RAM writes or sidecar
/// insertion. Even density-only enhanced worlds require the missing closure.
pub fn admit_native(config: &WorldConfig) -> Result<()> {
    config.validate()?;
    if config.faithful() {
        Ok(())
    } else {
        Err(Error::OwnershipClosure)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Bounds(&'static str),
    InvalidCell(u16),
    Resource(&'static str),
    Capacity(&'static str),
    Generation {
        level: LevelId,
        attempts: u8,
        reason: &'static str,
    },
    State(&'static str),
    OwnershipClosure,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bounds(what) => write!(f, "world bounds: {what}"),
            Self::InvalidCell(word) => write!(f, "invalid final world cell {word:#06x}"),
            Self::Resource(what) => write!(f, "invalid imported world resource: {what}"),
            Self::Capacity(what) => write!(f, "world capacity exhausted: {what}"),
            Self::Generation { level, attempts, reason } => write!(f,
                "ExpandedV1 level {} rejected after {attempts} attempts: {reason}", level.get()),
            Self::State(what) => write!(f, "invalid world state: {what}"),
            Self::OwnershipClosure => f.write_str("larger/density worlds are not admitted: terrain/render, collision/player, population, inventory/phone/map and elevator transitions still require mode-owned storage"),
        }
    }
}
impl std::error::Error for Error {}
pub type Result<T, E = Error> = std::result::Result<T, E>;
