use super::*;
use anyhow::Result;

#[derive(Clone)]
struct Toy([u64; 4]);
impl Default for Toy {
    fn default() -> Self {
        Self([0x1234, 0xabcd, 0, 0])
    }
}
impl Machine for Toy {
    type Snapshot = Self;
    fn snapshot_hash(snapshot: &Self) -> StateHash {
        snapshot.state_hash().unwrap()
    }
    fn capture(&self) -> Result<Self> {
        Ok(self.clone())
    }
    fn restore(&mut self, snapshot: &Self) -> Result<()> {
        *self = snapshot.clone();
        Ok(())
    }
    fn advance(&mut self, pads: [u8; 2]) -> Result<()> {
        self.0[0] = self.0[0]
            .wrapping_mul(6364136223846793005)
            .wrapping_add(u64::from(pads[0]) + self.0[2]);
        self.0[1] = self.0[1].rotate_left(13) ^ self.0[0] ^ u64::from(pads[1]);
        self.0[2] += 1;
        self.0[3] = self.0[3].wrapping_add(self.0[1].rotate_left(u32::from(pads[0] & 63)));
        Ok(())
    }
    fn state_hash(&self) -> Result<StateHash> {
        let mut hash = [0; 20];
        for (index, byte) in self.0.into_iter().flat_map(u64::to_le_bytes).enumerate() {
            hash[index % 20] ^= byte;
        }
        Ok(hash)
    }
}
fn identity(config: Config) -> Identity {
    Identity {
        protocol: PROTOCOL_VERSION,
        engine: "test-engine-1".into(),
        rom_sha1: "0".repeat(40),
        mode: "faithful".into(),
        sample_rate: 48000,
        config,
        initial_state: Toy::default().state_hash().unwrap(),
    }
}
fn input(player: usize, frame: u64) -> u8 {
    ((frame.wrapping_mul(13 + player as u64 * 4) ^ frame.rotate_right(3)) & 255) as u8
}

#[test]
fn delayed_lossy_reordered_peers_match_no_latency_exactly() {
    for (one_way, jitter, loss_every, delay, window) in [
        (0, 0, 0, 0, 12),
        (3, 2, 0, 2, 12),
        (6, 3, 7, 2, 12),
        (18, 6, 5, 2, 12),
    ] {
        let config = Config {
            input_delay: delay,
            max_rollback: window,
        };
        let mut peers = [
            Peer::new(identity(config), [19; 16], 0, 0).unwrap(),
            Peer::new(identity(config), [19; 16], 1, 0).unwrap(),
        ];
        let mut machines = [Toy::default(), Toy::default()];
        let mut queue = Vec::new();
        const FRAMES: u64 = 600;
        let mut completed_tick = None;
        for tick in 0..10000_u64 {
            let mut index = 0;
            while index < queue.len() {
                let (due, _, _): &(u64, usize, Vec<u8>) = &queue[index];
                if *due <= tick {
                    let (_, destination, packet) = queue.swap_remove(index);
                    peers[destination].receive(&packet, tick * 17).unwrap();
                } else {
                    index += 1;
                }
            }
            for player in 0..2 {
                peers[player].reconcile(&mut machines[player]).unwrap();
                if peers[player].stats().frames < FRAMES {
                    let pad = input(player, peers[player].stats().frames);
                    peers[player].advance(&mut machines[player], pad).unwrap();
                }
                if loss_every == 0 || !(tick + player as u64).is_multiple_of(loss_every) {
                    let latency = one_way
                        + if jitter == 0 {
                            0
                        } else {
                            (tick * 7 + player as u64 * 3) % (jitter + 1)
                        };
                    queue.push((tick + latency, 1 - player, peers[player].packet().unwrap()));
                }
            }
            if peers.iter().all(|peer| peer.stats().confirmed == FRAMES) {
                completed_tick = Some(tick);
                break;
            }
        }
        assert!(completed_tick.is_some(), "peers did not finish");
        let mut reference = Toy::default();
        for frame in 0..FRAMES {
            reference
                .advance(std::array::from_fn(|player| {
                    if frame < u64::from(delay) {
                        0
                    } else {
                        input(player, frame - u64::from(delay))
                    }
                }))
                .unwrap();
        }
        for machine in &machines {
            assert_eq!(
                machine.state_hash().unwrap(),
                reference.state_hash().unwrap()
            );
        }
        for (player, peer) in peers.iter().enumerate() {
            let stats = peer.stats();
            assert!(stats.largest_rollback <= u64::from(window));
            if one_way > u64::from(delay) {
                assert!(stats.rollbacks > 0);
            }
            println!(
                "one_way_frames={one_way} jitter_frames={jitter} loss_every={loss_every} input_delay={delay} player={player} ticks={} stats={stats:?} final_hash={:02x?}",
                completed_tick.unwrap(),
                reference.state_hash().unwrap()
            );
        }
    }
}

#[test]
fn confirmed_desync_stops_play() {
    let mut a = Rollback::new(0, Config::default()).unwrap();
    let mut machine = Toy::default();
    for frame in 0..30 {
        a.receive(frame + 2, input(1, frame)).unwrap();
        assert!(a.advance(&mut machine, input(0, frame)).unwrap());
    }
    a.receive_hash(30, [99; 20]).unwrap();
    assert!(
        a.reconcile(&mut machine)
            .unwrap_err()
            .to_string()
            .contains("desync")
    );
}

#[test]
fn handshake_rejects_rom_build_configuration_and_slot_mismatch() {
    let base = identity(Config::default());
    for field in 0..6 {
        let mut different = base.clone();
        match field {
            0 => different.rom_sha1 = "1".repeat(40),
            1 => different.engine = "other-build".into(),
            2 => different.mode = "enhanced".into(),
            3 => different.sample_rate = 44100,
            4 => different.config.input_delay = 3,
            _ => different.initial_state[0] ^= 1,
        }
        let mut host = Peer::<Toy>::new(base.clone(), [4; 16], 0, 0).unwrap();
        let join = Peer::<Toy>::new(different, [4; 16], 1, 0).unwrap();
        assert!(host.receive(&join.packet().unwrap(), 0).is_err());
        assert_eq!(host.phase(), Phase::Connecting);
    }
    let mut host = Peer::<Toy>::new(base.clone(), [4; 16], 0, 0).unwrap();
    let foreign = Peer::<Toy>::new(base.clone(), [5; 16], 1, 0).unwrap();
    assert!(!host.receive(&foreign.packet().unwrap(), 0).unwrap());
    assert!(!host.receive(&[0xff; MAX_PACKET_BYTES + 1], 0).unwrap());
    assert!(!host.receive(b"unrelated UDP traffic", 0).unwrap());
    let wrong_slot = Peer::<Toy>::new(base, [4; 16], 0, 0).unwrap();
    assert!(host.receive(&wrong_slot.packet().unwrap(), 0).is_err());
    assert!(host.check_timeout(120_000).is_err());
}

#[test]
fn inputs_are_immutable_and_memory_horizon_is_bounded() {
    let mut rollback: Rollback<Toy> = Rollback::new(0, Config::default()).unwrap();
    rollback.receive(3, 4).unwrap();
    rollback.receive(3, 4).unwrap();
    assert!(rollback.receive(3, 5).is_err());
    assert!(rollback.receive(65, 0).is_err());
    assert!(rollback.acknowledge(999).is_err());
    assert!(
        Config {
            input_delay: 9,
            max_rollback: 12
        }
        .validate()
        .is_err()
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn two_local_udp_instances_exchange_inputs_and_hashes() {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let config = Config::default();
    let mut host = udp::Connection::host(identity(config), address, address, 0).unwrap();
    let invitation = udp::Invitation::decode(&host.code().unwrap()).unwrap();
    let mut join = udp::Connection::join(identity(config), address, invitation, 0).unwrap();
    let mut machines = [Toy::default(), Toy::default()];
    let started = Instant::now();
    let deadline = started + Duration::from_secs(10);
    while Instant::now() < deadline {
        let now_ms = started.elapsed().as_millis() as u64;
        host.pump(now_ms).unwrap();
        join.pump(now_ms).unwrap();
        host.peer_mut().reconcile(&mut machines[0]).unwrap();
        join.peer_mut().reconcile(&mut machines[1]).unwrap();
        for (player, connection) in [&mut host, &mut join].into_iter().enumerate() {
            if connection.peer().stats().frames < 120 {
                let pad = input(player, connection.peer().stats().frames);
                connection
                    .peer_mut()
                    .advance(&mut machines[player], pad)
                    .unwrap();
            }
        }
        if host.peer().stats().confirmed == 120 && join.peer().stats().confirmed == 120 {
            break;
        }
        // These are real OS sockets, not the virtual delivery queue above.
        // Yield to kernel delivery and keep pumping after the last local frame
        // until both peers have received/reconciled all 120 frames.
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(host.peer().stats().confirmed, 120);
    assert_eq!(join.peer().stats().confirmed, 120);
    assert_eq!(
        machines[0].state_hash().unwrap(),
        machines[1].state_hash().unwrap()
    );
    let mut reference = Toy::default();
    for frame in 0..120 {
        let pads = if frame < u64::from(config.input_delay) {
            [0; 2]
        } else {
            std::array::from_fn(|player| input(player, frame - u64::from(config.input_delay)))
        };
        reference.advance(pads).unwrap();
    }
    assert_eq!(
        machines[0].state_hash().unwrap(),
        reference.state_hash().unwrap()
    );
}
