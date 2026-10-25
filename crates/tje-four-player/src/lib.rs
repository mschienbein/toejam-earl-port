//! Deterministic primitives for a future mode-owned four-seat campaign.
//!
//! This crate does not install gameplay overrides. Admission deliberately fails
//! until the complete player/present/AI/transition/UI/render closure exists.
//! Nothing here changes faithful gameplay or invents additional Genesis ports.
#![forbid(unsafe_code)]

mod identity;
pub mod input;
pub mod palette;
pub mod state;
pub mod view;

#[cfg(feature = "enhanced-mode")]
pub mod enhanced;

pub use identity::{Archetype, Character, PaletteVariant, SeatId, SeatMask};
pub use state::{Config, CoopState};
pub use tje_world::LevelId;

pub const SEATS: usize = 4;
pub const INVENTORY_SLOTS: usize = 16;
pub const RULESET_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid(&'static str),
    SimulationUnavailable,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "invalid four-seat state: {reason}"),
            Self::SimulationUnavailable => f.write_str(
                "four-player is unavailable: the mode-owned players, presents, AI, level transitions, UI and host sprite pipeline are not implemented; no state was changed",
            ),
        }
    }
}
impl std::error::Error for Error {}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Disabled is always a no-op. A request cannot silently become a two-player
/// game, a debug override or a partially owned simulation.
pub fn check_admission(enabled: bool) -> Result<()> {
    if enabled {
        Err(Error::SimulationUnavailable)
    } else {
        Ok(())
    }
}
