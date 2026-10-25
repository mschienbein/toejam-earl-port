//! Shared typed services for explicitly selected world-changing extras.
//!
//! Data construction is not gameplay admission. Faithful mode never depends on
//! this crate's session owner, and incomplete ownership manifests fail closed.
#![forbid(unsafe_code)]

pub mod ids;
pub mod phase;
pub use ids::{ActorId, AnimationId, BehaviorId, DefinitionId, EffectId, FrameId};
pub use phase::{Admission, Phase, PhaseClock, PublicationStamp};
pub use tje_four_player::{SeatId, SeatMask};
pub use tje_world::LevelId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    State(&'static str),
    Capacity(&'static str),
    Ownership(&'static str),
}
impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::State(reason) => write!(formatter, "invalid mode-owned state: {reason}"),
            Self::Capacity(reason) => write!(formatter, "mode-owned capacity: {reason}"),
            Self::Ownership(reason) => write!(formatter, "mode ownership rejected: {reason}"),
        }
    }
}
impl std::error::Error for Error {}
pub type Result<T, E = Error> = std::result::Result<T, E>;
