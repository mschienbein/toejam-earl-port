#![cfg(feature = "native-tests")]
//! Opt-in private-ROM evidence. No cartridge bytes, state dumps or pictures are
//! written to the public repository. At most two engines run in one process.
use genesis_config::{
    GenesisButton, GenesisController, GenesisControllerType, GenesisEmulatorConfig, GenesisInputs,
};
use sha2::{Digest, Sha256};
use tje_engine_cranelift::{CacheConfig, Engine, load_engine};
use tje_engine_runtime::enhanced::{EnhancedInputs, EnhancedMode};
use tje_jam_out::{PROFILE, Settings, State, configure, presentation};
use tje_mem::Revision;
use tje_rom::Rom;
use tje_session::{CONNECTED, Session};

fn inputs(pads: [u16; 2], modifier: bool) -> EnhancedInputs {
    let controllers = pads.map(|bits| {
        let mut pad = if bits & CONNECTED != 0 {
            GenesisController::new(GenesisControllerType::ThreeButton)
        } else {
            GenesisController::None
        };
        for (bit, button) in [
            GenesisButton::Up,
            GenesisButton::Down,
            GenesisButton::Left,
            GenesisButton::Right,
            GenesisButton::A,
            GenesisButton::B,
            GenesisButton::C,
            GenesisButton::Start,
        ]
        .into_iter()
        .enumerate()
        {
            pad.set_field(button, bits & (1 << bit) != 0);
        }
        pad.set_field(GenesisButton::Mode, modifier);
        pad
    });
    GenesisInputs {
        p1: controllers[0],
        p2: controllers[1],
    }
    .into()
}

#[derive(Default)]
struct Hashes {
    frames: Sha256,
    audio: Sha256,
    presentation: Sha256,
}
impl Hashes {
    fn observe(&mut self, engine: &Engine, mode: &EnhancedMode, revision: Revision) -> [bool; 2] {
        let (pixels, size) = engine.frame_buffer();
        self.frames.update(size.width.to_le_bytes());
        self.frames.update(size.height.to_le_bytes());
        for pixel in &pixels[..size.len() as usize] {
            self.frames.update([pixel.r, pixel.g, pixel.b, pixel.a]);
        }
        for sample in engine.devices().audio_samples() {
            for channel in sample {
                self.audio.update(channel.to_bits().to_le_bytes());
            }
        }
        let poses = presentation(mode, engine.devices().memory(), revision);
        for pose in poses {
            if let Some(pose) = pose {
                self.presentation
                    .update([pose.entity, u8::from(pose.horizontal_flip)]);
                self.presentation.update(pose.frame_address.to_le_bytes());
            } else {
                self.presentation.update([0xff]);
            }
        }
        poses.map(|pose| pose.is_some())
    }
    fn finish(self) -> [String; 3] {
        [
            format!("{:x}", self.frames.finalize()),
            format!("{:x}", self.audio.finalize()),
            format!("{:x}", self.presentation.finalize()),
        ]
    }
}

#[test]
#[ignore = "requires both private retail ROMs; deterministic native Jam demo and same-profile restore"]
fn private_jam_demo_both_revisions_and_snapshot_replay() {
    for (revision, variable, session_name) in [
        (
            Revision::Rev02,
            "TJE_REV02_ROM",
            "rev02-two-player.tje.json",
        ),
        (
            Revision::Rev00,
            "TJE_REV00_ROM",
            "rev00-two-player.tje.json",
        ),
    ] {
        let bytes =
            std::fs::read(std::env::var(variable).expect("private ROM environment variable"))
                .unwrap();
        let cache_root = std::path::PathBuf::from(
            std::env::var("TJE_JAM_PRIVATE_CACHE").expect("private compiled cache directory"),
        );
        let cache = CacheConfig {
            data_dir: cache_root,
        };
        let mut a = load_engine(
            Rom::import(&bytes).unwrap(),
            &GenesisEmulatorConfig::default(),
            &cache,
            |_| {},
        )
        .unwrap();
        let mut am = EnhancedMode::new(PROFILE, revision).unwrap();
        let settings = Settings {
            enabled: true,
            backing: 1,
            ..Settings::default()
        };
        configure(&mut am, settings).unwrap();
        let session = Session::from_json(
            &std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../sessions")
                    .join(session_name),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(session.writes().is_empty());
        for pads in session.inputs().take(1800) {
            am.play_frame(&mut a, inputs(pads, false)).unwrap();
        }
        for _ in 0..120 {
            am.play_frame(&mut a, inputs([CONNECTED; 2], false))
                .unwrap();
        }
        let checkpoint = am.snapshot(&a).unwrap();
        let mut b = load_engine(
            Rom::import(&bytes).unwrap(),
            &GenesisEmulatorConfig::default(),
            &cache,
            |_| {},
        )
        .unwrap();
        let mut bm = EnhancedMode::new(PROFILE, revision).unwrap();
        configure(&mut bm, settings).unwrap();
        bm.restore(&mut b, &checkpoint).unwrap();
        let (mut ah, mut bh) = (Hashes::default(), Hashes::default());
        let mut visible = [0; 2];
        for frame in 0..480 {
            let modifier = frame % 48 < 36;
            let buttons = match (frame / 48) % 6 {
                0 => 1 << 4,
                1 => 1 << 5,
                2 => 1 << 6,
                3 => (1 << 4) | 1,
                4 => (1 << 4) | (1 << 5),
                _ => (1 << 4) | 8,
            };
            let pads = [CONNECTED | if modifier { buttons } else { 0 }; 2];
            am.play_frame(&mut a, inputs(pads, modifier)).unwrap();
            bm.play_frame(&mut b, inputs(pads, modifier)).unwrap();
            let poses = ah.observe(&a, &am, revision);
            bh.observe(&b, &bm, revision);
            for seat in 0..2 {
                visible[seat] += usize::from(poses[seat]);
            }
            assert_eq!(a.cpu(), b.cpu());
            assert_eq!(a.devices().memory().ram(), b.devices().memory().ram());
        }
        let state = am.sidecars().get::<State>().unwrap();
        let left = ah.finish();
        assert_eq!(left, bh.finish());
        assert!(
            visible.into_iter().all(|count| count > 0),
            "{revision:?}: no presentation for both characters: {visible:?}"
        );
        assert!(
            state.accepted_pcm > 0,
            "{revision:?}: no PCM performance request accepted"
        );
        assert!(
            state.accepted_fm > 0,
            "{revision:?}: no FM performance request accepted"
        );
        println!(
            "JAM_DEMO {revision:?} frames=480 poses={visible:?} pcm={} fm={} dropped={} video={} audio={} poses_hash={}",
            state.accepted_pcm,
            state.accepted_fm,
            state.dropped_requests,
            left[0],
            left[1],
            left[2]
        );
    }
}
