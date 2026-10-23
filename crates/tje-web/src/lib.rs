//! Browser frontend for the hybrid replacement-mode game.
//!
//! `web/build.sh` builds this cdylib with the version-matched wasm-bindgen CLI.
//! DOM/storage/input and the transferable AudioWorklet queue live in `web/`;
//! emulation and wgpu presentation stay here. No shared memory is required.

#[cfg(any(target_arch = "wasm32", test))]
mod input;

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
mod video;

#[cfg(target_arch = "wasm32")]
pub use browser::WebGame;
