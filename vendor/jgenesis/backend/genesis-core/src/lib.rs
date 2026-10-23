pub mod api;
pub mod audio;
mod bus;
mod input;
mod timing;

pub use api::{GenesisEmulator, GenesisError, GenesisResult};
// TJE hybrid access: expose only the bus type and opt-in observation events.
pub use bus::{GenesisBus, HybridAccess, HybridWrite};
pub use genesis_config::{GenesisButton, GenesisInputs, GenesisJoypadState};
