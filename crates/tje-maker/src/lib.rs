//! Asset-free, explicitly authored Maker courses. Nothing in this crate exports
//! original maps, ROM resources, machine snapshots, or hosted lookup tokens.
#![forbid(unsafe_code)]
mod admission;
mod codec;
#[cfg(feature = "enhanced-mode")]
pub mod enhanced;
mod model;
mod validate;
pub use admission::admit_native;
pub use codec::{Error, ValidatedCourse};
pub use model::*;
pub use validate::{Issue, ValidationErrors, valid_cell, validate};
