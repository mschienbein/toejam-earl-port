//! The shipped native game boundary. No hybrid host, verifier or 68000 interpreter.
//!
//! Windowed play and the separate development proof executable use these exact
//! backend and replay paths. Compilation consumes only the user's verified ROM.
pub mod backend;
pub mod config;
pub mod input;
pub mod replay;
