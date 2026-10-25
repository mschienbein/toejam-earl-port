//! Opt-in, transport-independent two-player rollback. No cartridge or snapshot
//! bytes ever enter the wire protocol. Offline engines do not construct a peer.
mod protocol;
mod rollback;
#[cfg(not(target_arch = "wasm32"))]
pub mod udp;

pub use protocol::{Identity, MAX_PACKET_BYTES, PROTOCOL_VERSION, Peer, Phase};
pub use rollback::{Config, Machine, Rollback, Stats};

/// Hash of all deterministic machine state, not just work RAM or pixels.
pub type StateHash = [u8; 20];

#[cfg(test)]
mod tests;
