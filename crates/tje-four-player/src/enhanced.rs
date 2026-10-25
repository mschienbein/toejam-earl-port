//! Sidecar codec integration, NOT an admitted gameplay profile.
use crate::{Config, CoopState, Result, check_admission};
use tje_engine_runtime::enhanced::{EnhancedMode, Sidecar};

impl Sidecar for CoopState {
    const KEY: &'static str = "four-player/seats";
    const VERSION: u32 = 1;

    fn validate(&self) -> tje_game::Result<()> {
        CoopState::validate(self).map_err(|error| tje_game::Error::Platform(error.to_string()))
    }
    fn configuration_identity(&self) -> &[u8] {
        CoopState::configuration_identity(self)
    }
}

/// The only selection boundary in this crate. Until the shared simulation
/// closure is complete, enabling rejects BEFORE reset, sidecar registration,
/// profile replacement, RAM writes, input changes or presentation publication.
/// Disabling is an exact no-op, even on an already-running faithful session.
/// There is deliberately no partially selected FOUR_PLAYER_PROFILE constant.
pub fn request(_current: &mut EnhancedMode, enabled: bool, config: &Config) -> Result<()> {
    if enabled {
        config.validate()?;
    }
    check_admission(enabled)
}

#[cfg(test)]
#[path = "enhanced_tests.rs"]
mod tests;
