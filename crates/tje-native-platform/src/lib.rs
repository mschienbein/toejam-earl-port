//! Genesis devices driven by native Rust, without a 68000 execution engine.
//!
//! `DeviceRuntime` is the cycle-driven boundary for generated instruction
//! continuations. `NativePlatform` supplies the readable `tje_game::Context`
//! adapter for closed-subsystem, native-versus-reference differential proofs.
//! Neither production path can fall back to original 68000 execution.

#[cfg(feature = "game-adapter")]
mod context;
mod devices;

#[cfg(feature = "game-adapter")]
pub use context::{InterruptHandlers, NativePlatform, StartError};
pub use devices::{
    ClockedDevices, DeviceAccess, DeviceRuntime, DeviceStep, DeviceTrace, DeviceWrite, Origin,
    RamWrite, Registers, TimedWrite,
};
