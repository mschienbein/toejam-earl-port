//! Browser frontend for the cycle-exact, ROM-compiled WebAssembly engine.
//!
//! `web/build.sh` builds this cdylib with the version-matched wasm-bindgen CLI.
//! DOM/storage/input and the transferable AudioWorklet queue live in `web/`;
//! generated continuations, original devices and wgpu presentation stay here.
//! No 68000 interpreter, browser oracle or shared memory is required.

#[cfg(any(target_arch = "wasm32", test))]
pub mod input;

#[cfg(any(target_arch = "wasm32", test))]
mod viewport;

#[cfg(any(feature = "replay-observer", feature = "reference-export"))]
pub mod observation;

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
mod video;

#[cfg(target_arch = "wasm32")]
pub use browser::{CompiledCartridge, VideoFilter, WebGame};
