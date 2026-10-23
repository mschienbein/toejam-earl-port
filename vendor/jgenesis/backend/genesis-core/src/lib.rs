pub mod api;
pub mod audio;
mod bus;
mod input;
mod timing;
// TJE timing survey: disabled unless explicitly installed by a measurement tool.
pub use timing::survey as timing_survey;

pub use api::{GenesisEmulator, GenesisError, GenesisResult};
// TJE hybrid access: expose only the bus type and opt-in observation events.
pub use bus::{GenesisBus, HybridAccess, HybridWrite};
pub use genesis_config::{GenesisButton, GenesisInputs, GenesisJoypadState};
