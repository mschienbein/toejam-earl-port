//! Default-off Overlord rules. These services do not install a profile or expose
//! a session choice. The shared S6 owner must integrate and admit the whole mode.
#![forbid(unsafe_code)]

pub mod bridge;
pub mod command;
pub mod config;
pub mod placement;
#[cfg(any(test, feature = "proof"))]
pub mod proof;
pub mod state;
#[cfg(test)]
mod tests;
pub mod traps;
pub mod view;

pub use config::{Config, Preset};
pub use state::OverlordState;
pub use tje_simulation::population::{Kind, PopulationId};
pub use tje_simulation::{ActorId, LevelId, SeatId, SeatMask};

pub const HEROES: SeatMask = SeatMask::new(7).expect("three hero seats");
pub const DIRECTOR: SeatId = SeatId::ALL[3];
pub const MAX_SPAWNS: usize = 6;
pub const MAX_TRAPS: usize = 8;
pub const MAX_COMMANDS: usize = 32;
pub const MAX_STATE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid(&'static str),
    Role,
    Order,
    Full,
    Energy,
    Cooldown,
    StaleActor,
    UnsupportedKind,
    Protected,
    Placement(placement::Rejection),
    Shared,
    AdmissionClosed,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Overlord rejected: {self:?}")
    }
}
impl std::error::Error for Error {}
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An explicit capability list, including the existing Dentist variant.
pub const fn supported(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Devil | Kind::Dentist | Kind::DentistVariant | Kind::Lawnmower | Kind::NerdHerd
    )
}

/// There is deliberately no partial profile, fallback to co-op, or hidden menu.
pub fn request_gameplay() -> Result<()> {
    Err(Error::AdmissionClosed)
}
