//! Private full-session proof that shipping native playback remains the original
//! engine when Maker/extras are absent, including a feature-enabled binary.
use genesis_config::GenesisEmulatorConfig;
use sha2::{Digest, Sha256};
use tje_engine_cranelift::{CacheConfig, load_engine};
use tje_native::{
    backend::{GameBackend, NativeBackend, genesis_inputs},
    replay::controllers,
};
use tje_rom::{Revision, Rom};
use tje_session::Session;

#[test]
#[ignore = "requires both private ROM paths; two engines, no window or ROM-derived public fixture"]
fn native_maker_absent_sessions_match_original_frame_and_audio_hashes() {
    for (revision, key, session_name) in [
        (
            Revision::Rev02,
            "TJE_REV02_ROM",
            "rev02-one-player.tje.json",
        ),
        (
            Revision::Rev00,
            "TJE_REV00_ROM",
            "rev00-one-player.tje.json",
        ),
    ] {
        let bytes = std::fs::read(std::env::var(key).expect("private ROM path")).unwrap();
        let session = Session::from_json(
            &std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../sessions")
                    .join(session_name),
            )
            .unwrap(),
        )
        .unwrap();
        let cache = CacheConfig {
            data_dir: std::env::temp_dir().join("tje-maker-private-cache"),
        };
        let mut baseline = load_engine(
            Rom::import(&bytes).unwrap(),
            &GenesisEmulatorConfig::default(),
            &cache,
            |_| {},
        )
        .unwrap();
        let mut native = NativeBackend::load(Rom::import(&bytes).unwrap(), &cache, |_| {}).unwrap();
        assert_eq!(native.profile_id(), "faithful");
        let (mut video, mut audio) = (Sha256::new(), Sha256::new());
        let (mut baseline_video, mut baseline_audio) = (Sha256::new(), Sha256::new());
        let mut pcm = Vec::new();
        for (frame, pads) in session.inputs().enumerate() {
            let pads = controllers(pads);
            let original_pcm = baseline.play_frame(genesis_inputs(pads)).unwrap();
            pcm.clear();
            for sample in original_pcm {
                for value in sample {
                    pcm.extend_from_slice(&value.to_bits().to_le_bytes());
                }
            }
            baseline_audio.update(&pcm);
            let actual_pcm = native.advance(pads).unwrap();
            pcm.clear();
            for sample in actual_pcm {
                for value in sample {
                    pcm.extend_from_slice(&value.to_bits().to_le_bytes());
                }
            }
            audio.update(&pcm);
            let (original, size) = baseline.frame_buffer();
            let actual = native.video();
            assert_eq!((actual.width, actual.height), (size.width, size.height));
            let expected_rgba: &[u8] = bytemuck::cast_slice(&original[..size.len() as usize]);
            assert_eq!(
                actual.rgba, expected_rgba,
                "{revision:?} video frame {frame}"
            );
            for hash in [&mut video, &mut baseline_video] {
                hash.update(size.width.to_le_bytes());
                hash.update(size.height.to_le_bytes());
            }
            video.update(actual.rgba);
            baseline_video.update(expected_rgba);
            assert_eq!(
                native.engine().cpu(),
                baseline.cpu(),
                "{revision:?} CPU frame {frame}"
            );
            assert_eq!(
                native.engine().devices().memory().ram(),
                baseline.devices().memory().ram(),
                "{revision:?} RAM frame {frame}"
            );
            assert_eq!(
                native.engine().devices().clock(),
                baseline.devices().clock()
            );
        }
        let actual = (
            format!("{:x}", video.finalize()),
            format!("{:x}", audio.finalize()),
        );
        let expected = (
            format!("{:x}", baseline_video.finalize()),
            format!("{:x}", baseline_audio.finalize()),
        );
        assert_eq!(actual, expected);
        eprintln!(
            "{revision:?} native_maker=absent frames={} video_sha256={} audio_sha256={} identical=true",
            session.frames, actual.0, actual.1
        );
    }
}
