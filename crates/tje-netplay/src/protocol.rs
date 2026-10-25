use crate::{Config, Machine, Rollback, StateHash, Stats};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_PACKET_BYTES: usize = 1400;
const INPUTS_PER_PACKET: usize = 16;

/// Equality is mandatory before either engine advances. ROM identity is a
/// normalized cartridge digest; only the digest, never the cartridge, is sent.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub protocol: u32,
    pub engine: String,
    pub rom_sha1: String,
    pub mode: String,
    pub sample_rate: u32,
    pub config: Config,
    pub initial_state: StateHash,
}
impl Identity {
    pub fn validate(&self) -> Result<()> {
        self.config.validate()?;
        ensure!(
            self.protocol == PROTOCOL_VERSION,
            "different netplay protocol"
        );
        ensure!(
            !self.engine.is_empty() && self.engine.len() <= 128,
            "invalid engine identity"
        );
        ensure!(
            !self.mode.is_empty() && self.mode.len() <= 64,
            "invalid simulation mode identity"
        );
        ensure!(
            self.rom_sha1.len() == 40 && self.rom_sha1.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid cartridge hash"
        );
        ensure!(
            (8000..=192_000).contains(&self.sample_rate),
            "invalid audio configuration"
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Connecting,
    Running,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Packet {
    token: [u8; 16],
    player: u8,
    identity: Identity,
    ready: bool,
    inputs: Vec<(u64, u8)>,
    acknowledged: u64,
    checkpoint: Option<(u64, StateHash)>,
}

#[derive(Serialize)]
struct Outgoing<'a> {
    token: [u8; 16],
    player: u8,
    identity: &'a Identity,
    ready: bool,
    inputs: &'a [(u64, u8)],
    acknowledged: u64,
    checkpoint: Option<(u64, StateHash)>,
}

/// Reliable input delivery on an unreliable datagram channel: every packet
/// repeats the oldest unacknowledged inputs and a cumulative contiguous ACK.
/// A stalled/paused frontend must still send packets (at least every 50 ms).
/// The invitation's random capability pins the session; native UDP is not
/// encrypted, so it is intended for a trusted peer, not adversarial matchmaking.
pub struct Peer<S> {
    identity: Identity,
    token: [u8; 16],
    player: u8,
    saw_peer: bool,
    peer_ready: bool,
    rollback: Rollback<S>,
    last_receive_ms: Option<u64>,
    started_ms: u64,
}
impl<S> Peer<S> {
    pub fn new(identity: Identity, token: [u8; 16], player: u8, now_ms: u64) -> Result<Self> {
        identity.validate()?;
        let rollback = Rollback::new(usize::from(player), identity.config)?;
        Ok(Self {
            identity,
            token,
            player,
            saw_peer: false,
            peer_ready: false,
            rollback,
            last_receive_ms: None,
            started_ms: now_ms,
        })
    }
    pub fn phase(&self) -> Phase {
        if self.saw_peer && self.peer_ready {
            Phase::Running
        } else {
            Phase::Connecting
        }
    }
    pub fn stats(&self) -> Stats {
        self.rollback.stats()
    }
    pub fn packet(&self) -> Result<Vec<u8>> {
        let mut inputs = [(0, 0); INPUTS_PER_PACKET];
        let mut count = 0;
        for input in self.rollback.inputs().take(INPUTS_PER_PACKET) {
            inputs[count] = input;
            count += 1;
        }
        let mut bytes = Vec::with_capacity(MAX_PACKET_BYTES);
        bytes.extend_from_slice(&self.token);
        serde_json::to_writer(
            &mut bytes,
            &Outgoing {
                token: self.token,
                player: self.player,
                identity: &self.identity,
                ready: self.saw_peer,
                inputs: &inputs[..count],
                acknowledged: self.rollback.received_through(),
                checkpoint: self.rollback.checkpoint(),
            },
        )?;
        ensure!(
            bytes.len() <= MAX_PACKET_BYTES,
            "netplay packet exceeds datagram bound"
        );
        Ok(bytes)
    }
    /// Returns false for a foreign capability. Such packets never select or
    /// replace the native peer address. All matching-session protocol errors
    /// stop play instead of silently continuing after possible state corruption.
    pub fn receive(&mut self, bytes: &[u8], now_ms: u64) -> Result<bool> {
        if bytes.get(..16) != Some(self.token.as_slice()) {
            return Ok(false);
        }
        ensure!(
            bytes.len() <= MAX_PACKET_BYTES,
            "oversized netplay datagram"
        );
        let packet: Packet = serde_json::from_slice(&bytes[16..])?;
        if packet.token != self.token {
            return Ok(false);
        }
        ensure!(
            packet.player == 1 - self.player,
            "both peers selected the same player slot"
        );
        ensure!(
            packet.identity == self.identity,
            "netplay requires the same ROM hash, engine build, simulation mode, audio rate, input delay, rollback window and power-on state"
        );
        ensure!(
            packet.inputs.len() <= INPUTS_PER_PACKET,
            "too many input frames in a datagram"
        );
        self.rollback.acknowledge(packet.acknowledged)?;
        for (frame, pad) in packet.inputs {
            self.rollback.receive(frame, pad)?;
        }
        if let Some((frame, hash)) = packet.checkpoint {
            self.rollback.receive_hash(frame, hash)?;
        }
        self.saw_peer = true;
        self.peer_ready |= packet.ready;
        self.last_receive_ms = Some(now_ms);
        Ok(true)
    }
    pub fn check_timeout(&self, now_ms: u64) -> Result<()> {
        let deadline = if self.last_receive_ms.is_some() {
            15_000
        } else {
            120_000
        };
        ensure!(
            now_ms.saturating_sub(self.last_receive_ms.unwrap_or(self.started_ms)) < deadline,
            "netplay peer timed out; disconnect and exchange new connection codes"
        );
        Ok(())
    }
    pub fn reconcile<M: Machine<Snapshot = S>>(&mut self, machine: &mut M) -> Result<()> {
        self.rollback.reconcile(machine)
    }
    pub fn advance<M: Machine<Snapshot = S>>(&mut self, machine: &mut M, pad: u8) -> Result<bool> {
        if self.phase() != Phase::Running {
            return Ok(false);
        }
        self.rollback.advance(machine, pad)
    }
}
