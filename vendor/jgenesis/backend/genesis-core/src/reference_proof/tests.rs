//! Synthetic recorder tests, not retail execution or natural-play evidence.
use super::*;
use genesis_config::GenesisEmulatorConfig;

fn armed(owner: AudioOwner, revision: RetailRevision) -> (ReferenceProof, Memory) {
    let mut memory = Memory::new(&GenesisEmulatorConfig::default());
    assert!(memory.begin_audio_fixture_watch());
    let mut proof = ReferenceProof::default();
    proof.observe_power_on();
    proof.audio = Some(AudioInterval {
        receipt: AudioRetryReceipt {
            recipe: AudioRetryRecipe {
                id: "synthetic-recorder-unit-test".into(),
                session: "synthetic-input-only".into(),
                session_sha1: format!("{:x}", Sha1::digest(b"synthetic-input-only")),
                revision,
                frame: 0,
                original_instruction_ordinal: 1,
                owner,
            },
            natural: false,
            cartridge_identity_verified: false,
            status: AudioIntervalStatus::Armed,
            expected_initial_busy: 0,
            expected_initial_command: 0,
            caller_sp: 0xff8000,
            caller_return_pc: 0x100,
            source_call: None,
            activation: None,
            first_retry_release: None,
            eligible_ticks: Vec::with_capacity(112),
            byte_events: Vec::with_capacity(4),
            owner_return: None,
            failure: None,
        },
        cpu_accesses: 0,
        physical_commits: 0,
        byte_sequence: 0,
    });
    proof.original_step(
        Some(OriginalInstruction {
            frame: 0,
            ordinal: 1,
            pc: owner.entry(revision),
            completed_successfully: true,
        }),
        None,
        0,
    );
    (proof, memory)
}
fn released(owner: AudioOwner, revision: RetailRevision) -> (ReferenceProof, Memory) {
    let (mut proof, mut memory) = armed(owner, revision);
    proof.cpu_access();
    proof.physical_commit();
    proof.busreq_commit(&mut memory, 100, true, 0x100, true, false);
    assert_eq!(memory.read_audio_ram(LOCK), 1);
    proof.original_step(
        Some(OriginalInstruction {
            frame: 0,
            ordinal: 2,
            pc: owner.retry(revision),
            completed_successfully: true,
        }),
        None,
        200,
    );
    proof.cpu_access();
    proof.physical_commit();
    proof.busreq_commit(&mut memory, 200, true, 0, true, false);
    assert_eq!(proof.audio.as_ref().unwrap().receipt.status, AudioIntervalStatus::Released);
    (proof, memory)
}
#[test]
fn exactly_two_one_shot_mutations_at_the_112th_eligible_tick_tail() {
    for revision in [RetailRevision::Rev02, RetailRevision::Rev00] {
        for owner in [AudioOwner::Pcm, AudioOwner::Psg] {
            let (mut proof, mut memory) = released(owner, revision);
            for clock in 0..50 {
                proof.tick_tail(&mut memory, 0x123, clock, false, true, true);
                proof.tick_tail(&mut memory, 0x123, clock, true, true, false);
            }
            assert!(proof.audio.as_ref().unwrap().receipt.eligible_ticks.is_empty());
            for tick in 1..112 {
                proof.tick_tail(&mut memory, 0x123, tick * 15, true, true, true);
                assert_eq!(memory.read_audio_ram(LOCK), 1);
            }
            proof.tick_tail(&mut memory, 0x123, 112 * 15, true, true, true);
            assert_eq!(memory.read_audio_ram(LOCK), 0);
            for tick in 113..140 {
                proof.tick_tail(&mut memory, 0x123, tick * 15, true, true, true);
            }
            let receipt = &proof.audio.as_ref().unwrap().receipt;
            assert_eq!(receipt.status, AudioIntervalStatus::Expired);
            assert!(!receipt.natural);
            assert_eq!(receipt.eligible_ticks.len(), 112);
            assert_eq!(receipt.byte_events.len(), 2);
            assert_eq!((receipt.byte_events[0].before, receipt.byte_events[0].value), (0, 1));
            assert_eq!((receipt.byte_events[1].before, receipt.byte_events[1].value), (1, 0));
            assert_eq!(receipt.byte_events[1].origin, ByteOrigin::AuthoredExpiry);
            assert_eq!(receipt.byte_events[1].eligible_ticks_after_release, 112);
            assert_eq!(
                receipt.byte_events[1].clock,
                receipt.eligible_ticks[111].clock_at_tick_start
            );
            assert!(
                receipt.first_retry_release.unwrap().event_ordinal
                    < receipt.eligible_ticks[0].event_ordinal
            );
            assert!(
                receipt.eligible_ticks[111].event_ordinal < receipt.byte_events[1].event_ordinal
            );
        }
    }
}
#[test]
fn same_value_mirrored_z80_writer_is_preserved_and_failure_is_sticky() {
    let (mut proof, mut memory) = released(AudioOwner::Pcm, RetailRevision::Rev02);
    proof.begin_z80_tick(0x123, 300);
    memory.write_audio_ram(0x2041, 1);
    proof.physical_audio_write(&mut memory, 400);
    proof.end_z80_tick();
    assert_eq!(proof.error().unwrap_err().kind, FailureKind::UnexpectedByteWriter);
    for tick in 1..120 {
        proof.tick_tail(&mut memory, 0x123, tick * 15, true, true, true);
    }
    proof.reset_commit(999);
    let receipt = &proof.audio.as_ref().unwrap().receipt;
    assert_eq!(receipt.failure.unwrap().kind, FailureKind::UnexpectedByteWriter);
    assert_eq!(receipt.status, AudioIntervalStatus::Failed);
    assert_eq!(receipt.byte_events.len(), 2);
    assert_eq!(receipt.byte_events[1].origin, ByteOrigin::OriginalZ80);
    assert_eq!((receipt.byte_events[1].before, receipt.byte_events[1].value), (1, 1));
    assert_eq!(memory.read_audio_ram(LOCK), 1);
}
#[test]
fn external_writers_are_not_hidden_by_reassertion_or_tail_restore() {
    let (mut proof, mut memory) = released(AudioOwner::Psg, RetailRevision::Rev00);
    memory.write_audio_ram(LOCK, 0);
    memory.write_audio_ram(0x2041, 0);
    proof.tick_tail(&mut memory, 0x123, 999, true, true, true);
    assert_eq!(proof.error().unwrap_err().kind, FailureKind::UnexpectedByteWriter);
    let receipt = &proof.audio.as_ref().unwrap().receipt;
    assert_eq!(receipt.byte_events.len(), 3);
    assert!(receipt.byte_events[1..].iter().all(|event| event.origin == ByteOrigin::Unattributed));
    assert!(!receipt.byte_events.iter().any(|event| event.origin == ByteOrigin::AuthoredExpiry));
    assert_eq!(memory.read_audio_ram(LOCK), 0);
}
#[test]
fn opaque_mutable_views_and_removed_writer_leases_are_rejected() {
    let (mut proof, mut memory) = released(AudioOwner::Pcm, RetailRevision::Rev02);
    let _ = memory.debug_ram_view();
    proof.check_unclaimed(&mut memory, 999);
    assert_eq!(proof.error().unwrap_err().kind, FailureKind::UnobservedWriter);
    let (mut proof, mut memory) = released(AudioOwner::Pcm, RetailRevision::Rev02);
    memory.finish_audio_fixture_watch();
    proof.check_unclaimed(&mut memory, 999);
    assert_eq!(proof.error().unwrap_err().kind, FailureKind::UnobservedWriter);
}
#[test]
fn decoding_and_speculative_cloning_do_not_mint_live_origin() {
    let (proof, memory) = released(AudioOwner::Pcm, RetailRevision::Rev02);
    let encoded = bincode::encode_to_vec(&proof, bincode::config::standard()).unwrap();
    let (decoded, consumed): (ReferenceProof, usize) =
        bincode::decode_from_slice(&encoded, bincode::config::standard()).unwrap();
    assert_eq!(consumed, encoded.len());
    for mut untrusted in [proof.clone(), decoded] {
        assert_eq!(untrusted.error().unwrap_err().kind, FailureKind::UntrustedOrigin);
        let mut memory = memory.clone();
        untrusted.tick_tail(&mut memory, 0x123, 999, true, true, true);
        assert_eq!(untrusted.audio.as_ref().unwrap().receipt.status, AudioIntervalStatus::Failed);
        assert_eq!(memory.read_audio_ram(LOCK), 1);
        assert!(
            !untrusted
                .audio
                .as_ref()
                .unwrap()
                .receipt
                .byte_events
                .iter()
                .any(|event| event.origin == ByteOrigin::AuthoredExpiry)
        );
    }
}
#[test]
fn wrong_width_value_issuer_and_release_pc_never_mutate_the_lock() {
    for (word, value, m68k) in [(false, 0x100, true), (true, 0x101, true), (true, 0x100, false)] {
        let (mut proof, mut memory) = armed(AudioOwner::Pcm, RetailRevision::Rev02);
        proof.busreq_commit(&mut memory, 100, word, value, m68k, false);
        assert!(proof.error().is_err());
        assert_eq!(memory.read_audio_ram(LOCK), 0);
        assert!(proof.audio.as_ref().unwrap().receipt.byte_events.is_empty());
    }
    let (mut proof, mut memory) = armed(AudioOwner::Pcm, RetailRevision::Rev02);
    proof.busreq_commit(&mut memory, 100, true, 0x100, true, false);
    proof.original_step(
        Some(OriginalInstruction { frame: 0, ordinal: 2, pc: 0x100, completed_successfully: true }),
        None,
        200,
    );
    proof.busreq_commit(&mut memory, 200, true, 0, true, false);
    assert_eq!(proof.error().unwrap_err().kind, FailureKind::UnexpectedCommit);
    assert_eq!(memory.read_audio_ram(LOCK), 1);
}
#[test]
fn reset_and_instruction_bounds_abort_without_manufacturing_readiness() {
    let (mut proof, mut memory) = released(AudioOwner::Psg, RetailRevision::Rev02);
    proof.reset_commit(999);
    proof.tick_tail(&mut memory, 0x123, 999, true, true, true);
    assert_eq!(proof.error().unwrap_err().kind, FailureKind::ResetDuringInterval);
    assert_eq!(memory.read_audio_ram(LOCK), 1);
    let (mut proof, memory) = released(AudioOwner::Psg, RetailRevision::Rev02);
    proof.original_step(
        Some(OriginalInstruction {
            frame: 0,
            ordinal: 1026,
            pc: 0x100,
            completed_successfully: true,
        }),
        None,
        999,
    );
    assert_eq!(proof.error().unwrap_err().kind, FailureKind::InstructionCeilingExceeded);
    assert_eq!(memory.read_audio_ram(LOCK), 1);
}

#[test]
fn a_non_original_prefix_irreversibly_invalidates_prearm_origin() {
    let mut proof = ReferenceProof::default();
    proof.observe_power_on();
    proof.reject_non_original(10);
    // Unarmed analysis can continue, but a genuine later instruction cannot
    // resurrect original-from-power-on authority.
    assert!(proof.error().is_ok());
    proof.original_step(
        Some(OriginalInstruction {
            frame: 0,
            ordinal: 1,
            pc: 0x3ed90,
            completed_successfully: true,
        }),
        Some([0; 25]),
        20,
    );
    assert_eq!(proof.require_audio_origin(20).unwrap_err().kind, FailureKind::NotOriginal);
    assert_eq!(proof.require_audio_origin(30).unwrap_err().clock, 10);
}

#[test]
fn a_wait_or_later_original_step_invalidates_the_fresh_exit_checkpoint() {
    let mut proof = ReferenceProof::default();
    proof.observe_power_on();
    proof.begin_original_attempt();
    proof.original_step(
        Some(OriginalInstruction {
            frame: 0,
            ordinal: 1,
            pc: 0x3f6b2,
            completed_successfully: true,
        }),
        Some([0; 25]),
        20,
    );
    assert!(proof.fresh_exit_matches(&[0; 25], 1, 0x3f6b2));
    proof.begin_original_attempt();
    proof.wait_step();
    assert!(!proof.fresh_exit_matches(&[0; 25], 1, 0x3f6b2));
}

struct Saves;
impl jgenesis_common::frontend::SaveWriter for Saves {
    type Err = std::io::Error;
    fn load_bytes(&mut self, _: &str) -> Result<Vec<u8>, Self::Err> {
        Err(std::io::ErrorKind::NotFound.into())
    }
    fn persist_bytes(&mut self, _: &str, _: &[u8]) -> Result<(), Self::Err> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
    fn load_serialized<D: bincode::Decode<()>>(&mut self, _: &str) -> Result<D, Self::Err> {
        Err(std::io::ErrorKind::NotFound.into())
    }
    fn persist_serialized<E: bincode::Encode>(&mut self, _: &str, _: E) -> Result<(), Self::Err> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}
fn toy_machine() -> GenesisEmulator {
    // Authored ISA-only JSR/RTS/spin toy; no retail cartridge data.
    let mut rom = vec![0; 0x400];
    rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
    rom[4..8].copy_from_slice(&0x100u32.to_be_bytes());
    rom[0x100..0x102].copy_from_slice(&0x4eb9u16.to_be_bytes());
    rom[0x102..0x106].copy_from_slice(&0x110u32.to_be_bytes());
    rom[0x106..0x108].copy_from_slice(&0x60feu16.to_be_bytes());
    rom[0x110..0x112].copy_from_slice(&0x4e75u16.to_be_bytes());
    GenesisEmulator::create_with_reference_proof(
        Some(rom),
        GenesisEmulatorConfig::default(),
        &mut Saves,
    )
}
fn dummy_recipe() -> AudioRetryRecipe {
    AudioRetryRecipe {
        id: "negative-only".into(),
        session: "synthetic-only".into(),
        session_sha1: format!("{:x}", Sha1::digest(b"synthetic-input-only")),
        revision: RetailRevision::Rev02,
        frame: 0,
        original_instruction_ordinal: 1,
        owner: AudioOwner::Pcm,
    }
}
#[test]
fn public_mutable_escapes_are_irreversible_before_and_during_arm() {
    for cpu_escape in [false, true] {
        let mut machine = toy_machine();
        if cpu_escape {
            let _ = machine.hybrid_parts();
        } else {
            let _ = machine.hybrid_bus_mut();
        }
        assert_eq!(
            machine.arm_audio_retry_fixture(dummy_recipe()).unwrap_err().kind,
            FailureKind::MutableEscape
        );
    }
    let mut machine = toy_machine();
    let (proof, memory) = released(AudioOwner::Pcm, RetailRevision::Rev02);
    let (_, bus) = machine.reference_parts();
    bus.memory = memory;
    bus.timing_survey.as_mut().unwrap().reference_proof = proof;
    machine.hybrid_bus_mut().memory.write_audio_ram(LOCK, 1);
    assert_eq!(machine.finish_audio_retry_fixture().unwrap_err().kind, FailureKind::MutableEscape);
    let diagnostic = machine.audio_retry_fixture_diagnostic().unwrap();
    assert_eq!(diagnostic.byte_events.last().unwrap().origin, ByteOrigin::Unattributed);
    assert!(!diagnostic.byte_events.iter().any(|event| event.origin == ByteOrigin::AuthoredExpiry));
}

struct Sink;
impl jgenesis_common::frontend::Renderer for Sink {
    type Err = std::convert::Infallible;
    fn render_frame(
        &mut self,
        _: &[jgenesis_common::frontend::Color],
        _: jgenesis_common::frontend::FrameSize,
        _: f64,
        _: jgenesis_common::frontend::RenderFrameOptions,
    ) -> Result<(), Self::Err> {
        Ok(())
    }
}
impl jgenesis_common::frontend::AudioOutput for Sink {
    type Err = std::convert::Infallible;
    fn push_sample(&mut self, _: f64, _: f64) -> Result<(), Self::Err> {
        Ok(())
    }
}
struct Pads(genesis_config::GenesisInputs);
impl jgenesis_common::frontend::InputPoller<genesis_config::GenesisInputs> for Pads {
    fn poll(&mut self) -> &genesis_config::GenesisInputs {
        &self.0
    }
}
#[test]
fn post_actual_rts_escape_cannot_reuse_the_exit_witness() {
    use jgenesis_common::frontend::EmulatorTrait;
    let mut machine = toy_machine();
    let mut pads = Pads(genesis_config::GenesisInputs::default());
    machine.tick(&mut Sink, &mut Sink, &mut pads, &mut Saves).unwrap();
    machine.tick(&mut Sink, &mut Sink, &mut pads, &mut Saves).unwrap();
    assert_eq!(machine.hybrid_cpu().pc(), 0x106);
    assert_eq!(
        machine.hybrid_cpu().hybrid_instruction_counts().unwrap().last_instruction.unwrap().opcode,
        0x4e75
    );
    let (cpu, _) = machine.hybrid_parts();
    cpu.set_status_register(cpu.status_register()); // Same-value escape still invalidates.
    assert_eq!(machine.finish_audio_retry_fixture().unwrap_err().kind, FailureKind::MutableEscape);
}
#[test]
fn unarmed_analysis_clone_can_tick_but_cannot_arm() {
    use jgenesis_common::frontend::{EmulatorTrait, PartialClone};
    let original = toy_machine();
    let mut analysis = original.partial_clone();
    let mut pads = Pads(genesis_config::GenesisInputs::default());
    analysis.tick(&mut Sink, &mut Sink, &mut pads, &mut Saves).unwrap();
    assert_eq!(
        analysis.arm_audio_retry_fixture(dummy_recipe()).unwrap_err().kind,
        FailureKind::UntrustedOrigin
    );
}

#[test]
fn ordinary_timing_survey_does_not_activate_reference_proof_recording() {
    let mut inactive = ReferenceProof::default();
    let before = bincode::encode_to_vec(&inactive, bincode::config::standard()).unwrap();
    inactive.begin_original_attempt();
    inactive.original_step(
        Some(OriginalInstruction { frame: 0, ordinal: 1, pc: 0x100, completed_successfully: true }),
        Some([0; 25]),
        10,
    );
    inactive.frame_rendered();
    inactive.begin_z80_tick(0x123, 10);
    inactive.end_z80_tick();
    inactive.wait_step();
    inactive.reject_non_original(20);
    inactive.public_mutable_escape(30);
    assert_eq!(before, bincode::encode_to_vec(&inactive, bincode::config::standard()).unwrap());
}

#[test]
fn escaped_bus_writer_never_borrows_a_previous_original_issuer() {
    let mut machine = toy_machine();
    let (proof, memory) = released(AudioOwner::Pcm, RetailRevision::Rev02);
    let (_, bus) = machine.reference_parts();
    bus.memory = memory;
    bus.timing_survey.as_mut().unwrap().reference_proof = proof;
    let escaped = machine.hybrid_bus_mut();
    z80_emu::BusInterface::write_memory(escaped, LOCK, 1);
    let receipt = machine.audio_retry_fixture_diagnostic().unwrap();
    let event = receipt.byte_events.last().unwrap();
    assert_eq!(event.origin, ByteOrigin::Unattributed);
    assert_eq!(event.pc, None);
    assert_eq!(receipt.failure.unwrap().kind, FailureKind::MutableEscape);
}

#[test]
fn decoded_active_recipe_cannot_hide_behind_an_unobserved_archive_flag() {
    for activated in [false, true] {
        let (mut source, mut memory) = if activated {
            released(AudioOwner::Pcm, RetailRevision::Rev02)
        } else {
            armed(AudioOwner::Pcm, RetailRevision::Rev02)
        };
        source.observed_from_power_on = false; // Forge archival metadata, not live authority.
        let bytes = bincode::encode_to_vec(&source, bincode::config::standard()).unwrap();
        let (mut decoded, _): (ReferenceProof, usize) =
            bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        if !activated {
            decoded.busreq_commit(&mut memory, 999, true, 0x100, true, false);
        }
        for tick in 0..112 {
            decoded.tick_tail(&mut memory, 0x123, tick, true, true, true);
        }
        assert_eq!(decoded.error().unwrap_err().kind, FailureKind::UntrustedOrigin);
        assert_eq!(memory.read_audio_ram(LOCK), u8::from(activated));
        assert!(
            !decoded
                .audio
                .as_ref()
                .unwrap()
                .receipt
                .byte_events
                .iter()
                .any(|event| event.origin == ByteOrigin::AuthoredExpiry)
        );
    }
}
