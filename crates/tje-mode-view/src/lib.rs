//! Presentation is a consumer of a committed world, never its clock or allocator.
//! This crate supplies no gameplay profile and cannot admit an incomplete mode.
#![forbid(unsafe_code)]

pub mod animation;
pub mod compositor;
pub mod overlay;
pub mod packet;
pub mod source;

pub use compositor::Compositor;
pub use jgenesis_common::frontend::{Color, FrameSize};
pub use packet::{Actor, Packet, Publisher, View};
use std::fmt;

pub const MAX_ACTORS: usize = 1024;
pub const MAX_PIECES: usize = 12;
pub const MAX_PATCHES: usize = 4;
pub const MAX_CACHE_BYTES: usize = 32 * 1024 * 1024;
pub const WIDTH: usize = 400;
pub const HEIGHT: usize = 224;
pub const CANVAS_WIDTH: usize = 800;
pub const CANVAS_HEIGHT: usize = 448;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Invalid(&'static str),
    Resource(tje_extended_view::RenderError),
}
impl From<tje_extended_view::RenderError> for Error {
    fn from(value: tje_extended_view::RenderError) -> Self {
        Self::Resource(value)
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => f.write_str(reason),
            Self::Resource(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for Error {}
pub type Result<T, E = Error> = std::result::Result<T, E>;
