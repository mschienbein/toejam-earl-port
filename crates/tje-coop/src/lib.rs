//! Bounded animation admissions and frozen logical publication adapters for
//! mode-owned co-op. These services do not admit or expose a gameplay profile.
#![forbid(unsafe_code)]
pub mod animation;
#[cfg(feature = "enhanced-mode")]
pub mod mode;
pub mod publication;
