//! Direct IPv4/IPv6 UDP. No rendezvous, discovery, relay or ROM transfer.
//! Across NAT, the host supplies a reachable address and forwards its UDP port.
use crate::{Config, Identity, MAX_PACKET_BYTES, Peer};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::ErrorKind,
    net::{SocketAddr, UdpSocket},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    pub address: SocketAddr,
    pub token: [u8; 16],
    pub config: Config,
    pub sample_rate: u32,
}
impl Invitation {
    pub fn encode(&self) -> Result<String> {
        Ok(format!("tje-udp1:{}", serde_json::to_string(self)?))
    }
    pub fn decode(code: &str) -> Result<Self> {
        ensure!(code.len() <= 1024, "connection code is too long");
        let invitation: Self =
            serde_json::from_str(code.trim().strip_prefix("tje-udp1:").context(
                "expected a native tje-udp1 connection code (browser codes use WebRTC)",
            )?)?;
        invitation.config.validate()?;
        ensure!(
            invitation.address.port() != 0
                && !invitation.address.ip().is_unspecified()
                && !invitation.address.ip().is_multicast(),
            "connection code needs a reachable unicast address and nonzero port"
        );
        ensure!(
            (8000..=192_000).contains(&invitation.sample_rate),
            "invalid invitation sample rate"
        );
        Ok(invitation)
    }
}

pub struct Connection<S> {
    socket: UdpSocket,
    address: Option<SocketAddr>,
    peer: Peer<S>,
    invitation: Invitation,
    last_send_ms: Option<u64>,
    last_sent_frame: u64,
}
impl<S> Connection<S> {
    pub fn host(
        identity: Identity,
        bind: SocketAddr,
        advertised: SocketAddr,
        now_ms: u64,
    ) -> Result<Self> {
        let socket = UdpSocket::bind(bind).context("bind netplay host UDP socket")?;
        socket.set_nonblocking(true)?;
        let mut token = [0; 16];
        getrandom::fill(&mut token)
            .map_err(|error| anyhow::anyhow!("generate private connection capability: {error}"))?;
        let invitation = Invitation {
            address: SocketAddr::new(
                advertised.ip(),
                if advertised.port() == 0 {
                    socket.local_addr()?.port()
                } else {
                    advertised.port()
                },
            ),
            token,
            config: identity.config,
            sample_rate: identity.sample_rate,
        };
        // Use the same validation as a pasted invitation.
        Invitation::decode(&invitation.encode()?)?;
        Ok(Self {
            socket,
            address: None,
            peer: Peer::new(identity, token, 0, now_ms)?,
            invitation,
            last_send_ms: None,
            last_sent_frame: 0,
        })
    }
    pub fn join(
        identity: Identity,
        bind: SocketAddr,
        invitation: Invitation,
        now_ms: u64,
    ) -> Result<Self> {
        ensure!(
            identity.config == invitation.config && identity.sample_rate == invitation.sample_rate,
            "join configuration differs from host invitation"
        );
        let socket = UdpSocket::bind(bind).context("bind netplay join UDP socket")?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            address: Some(invitation.address),
            peer: Peer::new(identity, invitation.token, 1, now_ms)?,
            invitation,
            last_send_ms: None,
            last_sent_frame: 0,
        })
    }
    pub fn code(&self) -> Result<String> {
        self.invitation.encode()
    }
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }
    pub fn peer(&self) -> &Peer<S> {
        &self.peer
    }
    pub fn peer_mut(&mut self) -> &mut Peer<S> {
        &mut self.peer
    }
    pub fn pump(&mut self, now_ms: u64) -> Result<()> {
        let mut buffer = [0; MAX_PACKET_BYTES + 1];
        for _ in 0..64 {
            match self.socket.recv_from(&mut buffer) {
                Ok((length, address)) => {
                    if self.address.is_some_and(|expected| expected != address) {
                        continue;
                    }
                    if self.peer.receive(&buffer[..length], now_ms)? {
                        self.address = Some(address);
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => return Err(error).context("receive netplay datagram"),
            }
        }
        self.peer.check_timeout(now_ms)?;
        let frames = self.peer.stats().frames;
        if let Some(address) = self.address
            && (self
                .last_send_ms
                .is_none_or(|last| now_ms.saturating_sub(last) >= 50)
                || frames != self.last_sent_frame)
        {
            let packet = self.peer.packet()?;
            match self.socket.send_to(&packet, address) {
                Ok(_) => {
                    self.last_send_ms = Some(now_ms);
                    self.last_sent_frame = frames;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => return Err(error).context("send netplay datagram"),
            }
        }
        Ok(())
    }
}
