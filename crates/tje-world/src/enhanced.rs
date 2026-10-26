//! Loading primitive for a future *mode-owned* loading root. No profile here
//! binds it to MapLoadPlayerLevel: returning into that original would expose
//! expanded dimensions to literal-bound consumers. Native admission stays closed.
use crate::{WorldConfig, generation::Phase, rom::RomResources, world::WorldState};
use tje_engine_runtime::enhanced::{
    Context, Continuation, EnhancedMode, OverrideStep, Sidecar, Wait,
};

impl Sidecar for WorldState {
    const KEY: &'static str = "tje/world/expanded-v1";
    const VERSION: u32 = 3;
    fn validate(&self) -> tje_game::Result<()> {
        WorldState::validate(self).map_err(game_error)
    }
    fn configuration_identity(&self) -> &[u8] {
        WorldState::configuration_identity(self)
    }
}
fn game_error(error: impl std::fmt::Display) -> tje_game::Error {
    tje_game::Error::Platform(error.to_string())
}

/// Registration remains a no-op for faithful defaults and rejects all requested
/// larger/density worlds *before* changing the existing driver or its sidecars.
pub fn configure_native(_mode: &mut EnhancedMode, config: &WorldConfig) -> crate::Result<()> {
    crate::admit_native(config)
}

/// Bounded resumable generator override body. The owner must request a load and
/// register WorldState first, and must not select this body at an original world
/// root until the shared ownership closure is implemented. Every pending slice
/// yields a real hardware frame; no Atomic callback drains loading or VBlank.
pub fn generate(
    context: &mut dyn Context,
    continuation: &mut Continuation,
) -> tje_game::Result<OverrideStep> {
    if continuation.stage > 1 {
        return Err(game_error("invalid world loading continuation"));
    }
    let revision = context.revision();
    let mut generation = context
        .sidecars_mut()
        .get_mut::<WorldState>()?
        .take_generation()
        .map_err(game_error)?;
    let result = RomResources::from_verified_bus(&*context, revision)
        .and_then(|resources| generation.advance(&resources, 256));
    let phase = context
        .sidecars_mut()
        .get_mut::<WorldState>()?
        .return_generation(generation, result)
        .map_err(game_error)?;
    continuation.stage = 1;
    match phase {
        Phase::Complete => Ok(OverrideStep::Return(Some(0))),
        Phase::Rejected => Err(game_error(
            "ExpandedV1 exhausted its 64 attempts; requested world was not published",
        )),
        _ => Ok(OverrideStep::Wait(Wait::Frames(1))),
    }
}
