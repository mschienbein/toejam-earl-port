//! Positive assertions exercise the vendor's real live-origin gates, not a
//! matching snapshot or a caller-supplied boolean. All cartridge bytes authored.
use super::*;
use genesis_core::reference_proof::{
    AudioOwner, AudioRetryRecipe, FailureKind, ResetInputError, RetailRevision,
};

fn observed_host() -> Host {
    let bytes = rom();
    let config = GenesisEmulatorConfig::default();
    let mut host = self::host();
    host.emulator =
        GenesisEmulator::create_with_reference_proof(Some(bytes), config, &mut NoCartridgeSaves);
    host.emulator.enable_hybrid_write_trace();
    host
}
fn recipe() -> AudioRetryRecipe {
    AudioRetryRecipe {
        id: "authored-origin-negative-binding".into(),
        session: "authored-no-retail-session".into(),
        session_sha1: "0".repeat(40),
        revision: RetailRevision::Rev02,
        frame: 0,
        original_instruction_ordinal: 1,
        owner: AudioOwner::Pcm,
    }
}
fn assert_live_origin(host: &mut Host) {
    host.emulator
        .take_fresh_reset_input()
        .expect("same authentic fresh reset input");
    // An authored ROM cannot satisfy this retail binding. BindingMismatch is
    // AFTER require_audio_origin; MutableEscape/UntrustedOrigin is not accepted.
    assert_eq!(
        host.emulator
            .arm_audio_retry_fixture(recipe())
            .unwrap_err()
            .kind,
        FailureKind::BindingMismatch,
        "audio power-on authority survives; no actual audio interval is claimed"
    );
}
fn assert_mutated_origin(host: &mut Host) {
    assert_eq!(
        host.emulator.take_fresh_reset_input().unwrap_err(),
        ResetInputError::UntrustedInstance
    );
    assert_eq!(
        host.emulator
            .arm_audio_retry_fixture(recipe())
            .unwrap_err()
            .kind,
        FailureKind::MutableEscape
    );
}

#[test]
fn original_reset_and_audio_authority_survive_fork_success_error_and_unwind() {
    for outcome in 0..3 {
        let mut host = observed_host();
        let pointer = asset(&mut host).0;
        let mut fork = host.fork();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            host.execute_fork(&mut fork, |fork| {
                assert_eq!(rom_pointer(fork), pointer);
                assert_eq!(
                    fork.take_fresh_reset_input().unwrap_err(),
                    ResetInputError::UntrustedInstance
                );
                match outcome {
                    0 => Ok(()),
                    1 => Err(HostError::Metadata("origin closure failure".into())),
                    _ => std::panic::resume_unwind(Box::new(OriginalPayload { marker: 19 })),
                }
            })
        }));
        match outcome {
            0 => result.unwrap().unwrap(),
            1 => assert!(matches!(result.unwrap(), Err(HostError::Metadata(_)))),
            _ => assert_eq!(
                result
                    .unwrap_err()
                    .downcast::<OriginalPayload>()
                    .unwrap()
                    .marker,
                19
            ),
        }
        assert_eq!(asset(&mut host).0, pointer);
        assert_live_origin(&mut host);
    }
}

#[test]
fn whole_analysis_host_and_nested_forks_never_acquire_live_origin() {
    let mut host = observed_host();
    let mut analysis = self::host();
    analysis.emulator = host.fork();
    analysis.analysis_rom = None;
    host.with_fork_rom(&mut analysis, |analysis| {
        let mut child = analysis.fork();
        analysis.execute_fork(&mut child, |child| {
            assert_eq!(
                child.take_fresh_reset_input().unwrap_err(),
                ResetInputError::UntrustedInstance
            );
            assert_eq!(
                child.arm_audio_retry_fixture(recipe()).unwrap_err().kind,
                FailureKind::MutableEscape
            );
            Ok(())
        })?;
        assert_eq!(
            analysis.emulator.take_fresh_reset_input().unwrap_err(),
            ResetInputError::UntrustedInstance
        );
        assert_eq!(
            analysis
                .emulator
                .arm_audio_retry_fixture(recipe())
                .unwrap_err()
                .kind,
            FailureKind::MutableEscape
        );
        Ok(())
    })
    .unwrap();
    assert!(analysis.analysis_rom.is_none());
    assert_live_origin(&mut host);
}

#[test]
fn foreign_live_destination_is_rejected_before_any_mutable_escape() {
    let mut host = observed_host();
    let mut foreign = observed_host();
    assert!(
        host.execute_fork::<()>(&mut foreign.emulator, |_| panic!("foreign live ROM"))
            .is_err()
    );
    assert!(
        host.with_fork_rom::<(), _>(&mut foreign, |_| panic!("foreign live Host"))
            .is_err()
    );
    assert_live_origin(&mut foreign);
    assert_live_origin(&mut host);
}

#[test]
fn real_same_value_live_mutation_remains_irreversibly_untrusted() {
    let mut host = observed_host();
    let address = 0xff9000;
    let value = host
        .emulator
        .hybrid_bus()
        .memory
        .read_main_ram_word(address);
    host.context().write_u16(address, value);
    let mut fork = host.fork();
    host.execute_fork(&mut fork, |_| Ok(())).unwrap();
    assert_mutated_origin(&mut host);
}

#[test]
fn snapshots_and_hardware_trace_drains_are_observation_not_mutable_escape() {
    let mut host = observed_host();
    let cpu = host.emulator.hybrid_cpu().hybrid_state_words();
    let snapshot = host.snapshot(0, Vec::new()).unwrap();
    assert_eq!(snapshot.frame_counter, 0);
    assert_eq!(snapshot.ram.len(), 65_536);
    assert!(snapshot.writes.is_empty());
    assert_eq!(host.emulator.hybrid_cpu().hybrid_state_words(), cpu);
    assert_live_origin(&mut host);
}

#[test]
fn passive_trace_retains_exact_original_hardware_writes_without_touching_cpu_or_ram() {
    let mut bytes = rom();
    // Authored MOVE.W #$8f1d,$c00004; BRA.S *. This is actual original CPU issue
    // and physical device dispatch, not a metadata event injected by the test.
    for (index, word) in [0x33fc_u16, 0x8f1d, 0x00c0, 0x0004, 0x60fe]
        .into_iter()
        .enumerate()
    {
        bytes[0x100 + index * 2..0x102 + index * 2].copy_from_slice(&word.to_be_bytes());
    }
    let mut host = observed_host();
    host.emulator = GenesisEmulator::create_with_reference_proof(
        Some(bytes),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    host.emulator.enable_hybrid_write_trace();
    for _ in 0..100 {
        host.emulator
            .tick(
                &mut HeadlessOutput,
                &mut host.audio,
                &mut Inputs(GenesisInputs::default()),
                &mut NoCartridgeSaves,
            )
            .unwrap();
        if host
            .emulator
            .hybrid_bus()
            .hybrid_writes
            .as_ref()
            .is_some_and(|writes| !writes.is_empty())
        {
            break;
        }
    }
    let cpu = host.emulator.hybrid_cpu().hybrid_state_words();
    let ram = host
        .emulator
        .hybrid_bus()
        .memory
        .hybrid_raw_main_ram()
        .to_vec();
    let snapshot = host.snapshot(0, Vec::new()).unwrap();
    assert_eq!(
        snapshot.writes,
        [HardwareWrite::Vdp {
            port: 4,
            value: 0x8f1d
        }]
    );
    assert!(host.snapshot(0, Vec::new()).unwrap().writes.is_empty());
    assert_eq!(host.emulator.hybrid_cpu().hybrid_state_words(), cpu);
    assert_eq!(host.emulator.hybrid_bus().memory.hybrid_raw_main_ram(), ram);
    assert_eq!(
        host.emulator
            .arm_audio_retry_fixture(recipe())
            .unwrap_err()
            .kind,
        FailureKind::BindingMismatch
    );
    // A genuine later reset may mint its own fresh input only if the live
    // lineage survived observation; this does not renew audio power-on origin.
    host.emulator.soft_reset();
    host.emulator.take_fresh_reset_input().unwrap();
}

#[test]
fn live_idle_rng_observation_retains_original_origin() {
    let mut host = observed_host();
    // Authored observation setup, not a naturally reached retail random call:
    // the actual reset SP names zeroed RAM and the diagnostic target is zero.
    host.symbols.random = host.cpu().pc();
    host.symbols.idle_random_return = 0;
    host.observe_live_instruction(host.cpu().pc());
    assert_eq!(host.frame_progress.idle_draws, 1);
    assert_live_origin(&mut host);
}
