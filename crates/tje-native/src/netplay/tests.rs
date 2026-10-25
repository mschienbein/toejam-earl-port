use super::*;
use std::{fs, path::PathBuf};
use tje_engine_cranelift::CacheConfig;
use tje_netplay::{Peer, udp::Connection};
use tje_rom::Rom;

fn source() -> (Vec<u8>, Vec<[u8; 2]>, CacheConfig) {
    let rom_path = PathBuf::from(
        std::env::var_os("TJE_REV02_ROM")
            .expect("set TJE_REV02_ROM to the user's private verified ROM"),
    );
    let session_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sessions/rev02-two-player.tje.json");
    let session =
        tje_session::Session::from_json(&fs::read_to_string(session_path).unwrap()).unwrap();
    let frames: usize =
        std::env::var("TJE_NETPLAY_FRAMES").map_or(3600, |value| value.parse().unwrap());
    assert!((120..=session.frames as usize).contains(&frames));
    let inputs = session
        .inputs()
        .take(frames)
        .map(|pads| pads.map(|pad| pad as u8))
        .collect();
    let cache = CacheConfig {
        data_dir: std::env::var_os("TJE_NETPLAY_CACHE").map_or_else(
            || std::env::temp_dir().join("tje-netplay-private-cache"),
            PathBuf::from,
        ),
    };
    (fs::read(rom_path).unwrap(), inputs, cache)
}
fn load(bytes: &[u8], cache: &CacheConfig) -> NativeBackend {
    let mut backend = NativeBackend::load(Rom::import(bytes).unwrap(), cache, |_| {}).unwrap();
    backend.set_audio_sample_rate(48000).unwrap();
    backend
}
fn identity(backend: &NativeBackend, config: Config) -> Identity {
    Identity {
        protocol: PROTOCOL_VERSION,
        engine: ENGINE_VERSION.into(),
        rom_sha1: backend.revision().sha1().into(),
        mode: "faithful".into(),
        sample_rate: 48000,
        config,
        initial_state: backend.engine().snapshot().unwrap().state_hash(),
    }
}
fn baseline(backend: &mut NativeBackend, inputs: &[[u8; 2]], delay: usize) -> MachineSnapshot {
    backend.reset();
    for frame in 0..inputs.len() {
        Adapter(backend)
            .advance(if frame < delay {
                [0; 2]
            } else {
                inputs[frame - delay]
            })
            .unwrap();
    }
    backend.engine().snapshot().unwrap()
}

#[test]
#[ignore = "requires the user's private ROM; runs the production compiled engine through a two-peer latency/loss/reordering harness"]
fn real_engine_rollback_matches_no_latency_full_state_video_and_audio() {
    let (bytes, inputs, cache) = source();
    let config = Config::default();
    let mut machines = [load(&bytes, &cache), load(&bytes, &cache)];
    let start = Instant::now();
    let expected = baseline(&mut machines[0], &inputs, usize::from(config.input_delay));
    println!(
        "NETPLAY_BASELINE frames={} elapsed_ms={:.3} snapshot_bytes={} final_state_hash={:02x?}",
        inputs.len(),
        start.elapsed().as_secs_f64() * 1000.0,
        expected.byte_len(),
        expected.state_hash()
    );
    for (latency, jitter, loss_every) in [(0, 0, 0), (6, 3, 7), (18, 6, 5)] {
        for machine in &mut machines {
            machine.reset();
        }
        let mut peers = [
            Peer::new(identity(&machines[0], config), [42; 16], 0, 0).unwrap(),
            Peer::new(identity(&machines[1], config), [42; 16], 1, 0).unwrap(),
        ];
        let mut deliveries: Vec<(u64, usize, Vec<u8>)> = Vec::new();
        let started = Instant::now();
        let mut completed = false;
        for tick in 0..inputs.len() as u64 * 8 + 1000 {
            let mut index = 0;
            while index < deliveries.len() {
                if deliveries[index].0 <= tick {
                    let (_, player, packet) = deliveries.swap_remove(index);
                    peers[player].receive(&packet, tick * 17).unwrap();
                } else {
                    index += 1;
                }
            }
            for player in 0..2 {
                peers[player]
                    .reconcile(&mut Adapter(&mut machines[player]))
                    .unwrap();
                let frame = peers[player].stats().frames as usize;
                if frame < inputs.len() {
                    peers[player]
                        .advance(&mut Adapter(&mut machines[player]), inputs[frame][player])
                        .unwrap();
                }
                if loss_every == 0 || !(tick + player as u64).is_multiple_of(loss_every) {
                    let delay = latency
                        + if jitter == 0 {
                            0
                        } else {
                            (tick * 7 + player as u64 * 3) % (jitter + 1)
                        };
                    deliveries.push((tick + delay, 1 - player, peers[player].packet().unwrap()));
                }
            }
            if peers
                .iter()
                .all(|peer| peer.stats().confirmed == inputs.len() as u64)
            {
                completed = true;
                break;
            }
        }
        assert!(completed, "latency harness did not finish");
        for (player, machine) in machines.iter().enumerate() {
            if latency > u64::from(config.input_delay)
                && inputs.iter().any(|pads| pads[1 - player] != 0)
            {
                assert!(
                    peers[player].stats().rollbacks > 0,
                    "remote input must exercise correction"
                );
            }
            let actual = machine.engine().snapshot().unwrap();
            // Canonical bytes contain the CPU, all hardware, exact last-frame
            // stereo f64 bits, framebuffer and frame count, not only work RAM.
            assert_eq!(
                actual.as_bytes(),
                expected.as_bytes(),
                "latency={latency} player={player}"
            );
            println!(
                "NETPLAY_ENGINE one_way_frames={latency} jitter_frames={jitter} loss_every={loss_every} player={player} elapsed_ms={:.3} stats={:?} final_state_hash={:02x?}",
                started.elapsed().as_secs_f64() * 1000.0,
                peers[player].stats(),
                actual.state_hash()
            );
        }
    }
}

#[test]
#[ignore = "requires the user's private ROM; two actual nonblocking UDP endpoints drive the compiled production engine"]
fn real_engine_direct_udp_matches_no_latency() {
    let (bytes, inputs, cache) = source();
    let config = Config::default();
    let mut machines = [load(&bytes, &cache), load(&bytes, &cache)];
    let expected = baseline(&mut machines[0], &inputs, usize::from(config.input_delay));
    for machine in &mut machines {
        machine.reset();
    }
    // Override to a reachable local interface to exercise LAN addressing rather
    // than loopback. No external endpoint, server or STUN service is contacted.
    let address = std::env::var("TJE_NETPLAY_TEST_IP")
        .unwrap_or_else(|_| "127.0.0.1".into())
        .parse()
        .unwrap();
    let socket = std::net::SocketAddr::new(address, 0);
    let mut host = Connection::host(identity(&machines[0], config), socket, socket, 0).unwrap();
    let invitation = Invitation::decode(&host.code().unwrap()).unwrap();
    let mut join = Connection::join(identity(&machines[1], config), socket, invitation, 0).unwrap();
    let started = Instant::now();
    for tick in 0..inputs.len() as u64 * 8 + 1000 {
        host.pump(tick * 17).unwrap();
        join.pump(tick * 17).unwrap();
        for (player, connection) in [&mut host, &mut join].into_iter().enumerate() {
            connection
                .peer_mut()
                .reconcile(&mut Adapter(&mut machines[player]))
                .unwrap();
            let frame = connection.peer().stats().frames as usize;
            if frame < inputs.len() {
                connection
                    .peer_mut()
                    .advance(&mut Adapter(&mut machines[player]), inputs[frame][player])
                    .unwrap();
            }
        }
        if host.peer().stats().confirmed == inputs.len() as u64
            && join.peer().stats().confirmed == inputs.len() as u64
        {
            break;
        }
    }
    for machine in &machines {
        assert_eq!(
            machine.engine().snapshot().unwrap().as_bytes(),
            expected.as_bytes()
        );
    }
    assert_eq!(host.peer().stats().confirmed, inputs.len() as u64);
    assert_eq!(join.peer().stats().confirmed, inputs.len() as u64);
    println!(
        "NETPLAY_UDP bind={} frames={} elapsed_ms={:.3} host={:?} join={:?} final_state_hash={:02x?}",
        host.local_addr().unwrap(),
        inputs.len(),
        started.elapsed().as_secs_f64() * 1000.0,
        host.peer().stats(),
        join.peer().stats(),
        expected.state_hash()
    );
}
