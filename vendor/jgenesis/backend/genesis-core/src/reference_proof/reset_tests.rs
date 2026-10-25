//! Authored ROM-free reference-backend tests, not retail root proofs.
use super::*;
use genesis_config::{GenesisEmulatorConfig, GenesisInputs};
use jgenesis_common::frontend::{EmulatorTrait, PartialClone};

struct Saves;
impl jgenesis_common::frontend::SaveWriter for Saves {
    type Err = std::io::Error;
    fn load_bytes(&mut self, _: &str) -> Result<Vec<u8>, Self::Err> {
        Err(std::io::ErrorKind::NotFound.into())
    }
    fn persist_bytes(&mut self, _: &str, _: &[u8]) -> Result<(), Self::Err> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
    fn load_serialized<D: Decode<()>>(&mut self, _: &str) -> Result<D, Self::Err> {
        Err(std::io::ErrorKind::NotFound.into())
    }
    fn persist_serialized<E: Encode>(&mut self, _: &str, _: E) -> Result<(), Self::Err> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}
fn rom() -> Vec<u8> {
    let mut bytes = vec![0; 0x400];
    bytes[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&0x200u32.to_be_bytes());
    bytes[0x200..0x202].copy_from_slice(&0x60feu16.to_be_bytes()); // Authored BRA self.
    bytes[0x202..0x204].copy_from_slice(&0x4e71u16.to_be_bytes()); // Authored NOP prefetch.
    bytes
}
fn observed() -> GenesisEmulator {
    GenesisEmulator::create_with_reference_proof(
        Some(rom()),
        GenesisEmulatorConfig::default(),
        &mut Saves,
    )
}
fn ordinary() -> GenesisEmulator {
    GenesisEmulator::create(Some(rom()), GenesisEmulatorConfig::default(), &mut Saves)
}
fn physical_bytes(machine: &GenesisEmulator) -> Vec<u8> {
    // Compare the complete existing save-state view after removing ONLY the
    // optional observer from an analysis copy. CPU counters are already excluded
    // by M68000's existing save-state encoding. No live state is repaired.
    let mut analysis = machine.partial_clone();
    analysis.reference_parts().1.timing_survey = None;
    bincode::encode_to_vec(analysis, bincode::config::standard()).unwrap()
}
#[derive(Default)]
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
struct Pads(GenesisInputs);
impl jgenesis_common::frontend::InputPoller<GenesisInputs> for Pads {
    fn poll(&mut self) -> &GenesisInputs {
        &self.0
    }
}
fn tick(machine: &mut GenesisEmulator) {
    machine.tick(&mut Sink, &mut Sink, &mut Pads(GenesisInputs::default()), &mut Saves).unwrap();
}
fn restore_rom_to_analysis(analysis: GenesisEmulator) -> GenesisEmulator {
    let mut receiver = observed();
    // The ordinary import moves its real ROM into the analysis state. Import
    // invalidates live lineage; matching CPU/ROM bytes do not reauthenticate it.
    receiver.load_state(analysis);
    receiver
}

#[test]
fn feature_enabled_but_ordinary_constructor_is_unarmed() {
    let mut machine = ordinary();
    assert!(machine.reference_reset_observation().is_none());
    assert_eq!(machine.take_fresh_reset_input().unwrap_err(), ResetInputError::NotObserved);
}
#[test]
fn actual_constructor_records_vectors_prefetch_raw_line_and_ignored_charge() {
    let mut machine = observed();
    let expected_hash: [u8; 20] = Sha1::digest(rom()).into();
    let o = machine.reference_reset_observation().unwrap();
    assert_eq!(o.cartridge_sha1, Some(expected_hash));
    assert_eq!(o.cause, ResetCause::Constructor);
    assert_eq!(o.reset_ordinal, 1);
    assert_ne!(o.instance, 0);
    assert_eq!(o.previous_instance, None);
    assert!(o.before.raw_reset_line); // GenesisBus::new already asserts the real reset line.
    assert_eq!(o.raw_line_at_execute, Some(true));
    let after = o.after.unwrap();
    assert!(!after.raw_reset_line);
    assert_eq!(after.cpu[16], 0xff8000);
    assert_eq!(after.cpu[17], 0x200);
    assert_eq!(o.returned_cpu_cycles, Some(132)); // Actual baseline reset return, not a device tick.
    assert_eq!(after.clocks, o.before.clocks);
    assert_eq!(after.scanline, o.before.scanline);
    assert_eq!(after.scanline_mclk, o.before.scanline_mclk);
    assert_eq!(o.failure, None);
    for (index, address, value) in [(0, 0, 0x00ff), (1, 2, 0x8000), (2, 4, 0), (3, 6, 0x200)] {
        assert!(matches!(o.accesses[index], Some(ResetAccess::Read { address: a, word: true,
            value: v, purpose: ResetReadPurpose::Semantic, raw_reset_line: true, .. })
            if a == address && v == value));
    }
    assert!(o.accesses[..o.access_count].iter().any(|access| matches!(
        access,
        Some(ResetAccess::Read { purpose: ResetReadPurpose::Prefetch, .. })
    )));
    let input = machine.take_fresh_reset_input().unwrap();
    assert_eq!(input.evidence().after, Some(after));
    assert_eq!(machine.take_fresh_reset_input().unwrap_err(), ResetInputError::NotFresh);
}
#[test]
fn observation_is_physically_inert_at_constructor_and_later_original_steps() {
    let mut baseline = ordinary();
    let mut proof = observed();
    assert_eq!(physical_bytes(&baseline), physical_bytes(&proof));
    for _ in 0..8 {
        tick(&mut baseline);
        tick(&mut proof);
        assert_eq!(physical_bytes(&baseline), physical_bytes(&proof));
    }
    assert_eq!(proof.take_fresh_reset_input().unwrap_err(), ResetInputError::NotFresh);
}
#[test]
fn distinct_live_instances_have_distinct_opaque_lineage() {
    let mut a = observed();
    let mut b = observed();
    let a = a.take_fresh_reset_input().unwrap();
    let b = b.take_fresh_reset_input().unwrap();
    assert_ne!(a.evidence().instance, b.evidence().instance);
    assert_eq!(a.evidence().cartridge_sha1, b.evidence().cartridge_sha1);
}
#[test]
fn stale_reentry_cannot_resurrect_constructor_input() {
    let mut machine = observed();
    tick(&mut machine);
    let (cpu, bus) = machine.hybrid_parts();
    cpu.set_pc(0x200, bus);
    assert_eq!(machine.hybrid_cpu().pc(), 0x200);
    assert_eq!(machine.take_fresh_reset_input().unwrap_err(), ResetInputError::UntrustedInstance);
}
#[test]
fn same_value_cpu_and_bus_mutable_escapes_invalidate_irreversibly() {
    let mut cpu_escape = observed();
    let sr = cpu_escape.hybrid_cpu().status_register();
    cpu_escape.hybrid_parts().0.set_status_register(sr);
    assert_eq!(
        cpu_escape.take_fresh_reset_input().unwrap_err(),
        ResetInputError::UntrustedInstance
    );
    let mut bus_escape = observed();
    let value = bus_escape.hybrid_bus().memory.read_main_ram_byte(0xff1000);
    bus_escape.hybrid_bus_mut().memory.write_main_ram_byte(0xff1000, value);
    assert_eq!(
        bus_escape.take_fresh_reset_input().unwrap_err(),
        ResetInputError::UntrustedInstance
    );
}
#[test]
fn clone_and_decode_archive_facts_but_never_fresh_or_instance_authority() {
    let original = observed();
    let mut clone = original.partial_clone();
    assert!(clone.reference_reset_observation().is_some());
    assert_eq!(clone.take_fresh_reset_input().unwrap_err(), ResetInputError::UntrustedInstance);
    let bytes = bincode::encode_to_vec(&original, bincode::config::standard()).unwrap();
    let (mut decoded, used): (GenesisEmulator, _) =
        bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
    assert_eq!(used, bytes.len());
    assert!(decoded.reference_reset_observation().is_some());
    assert_eq!(decoded.take_fresh_reset_input().unwrap_err(), ResetInputError::UntrustedInstance);
}
#[test]
fn actual_soft_and_hard_reset_on_analysis_copies_never_revive_authority() {
    for hard in [false, true] {
        let original = observed();
        let mut analysis = restore_rom_to_analysis(original.partial_clone());
        if hard {
            analysis.hard_reset(&mut Saves);
        } else {
            analysis.soft_reset();
        }
        let observation = analysis.reference_reset_observation().unwrap();
        assert_eq!(
            observation.cause,
            if hard { ResetCause::HardReset } else { ResetCause::SoftReset }
        );
        assert_eq!(observation.failure, None); // A genuine valid reset still isn't live origin.
        assert_eq!(observation.after.unwrap().cpu[17], 0x200);
        assert_eq!(
            analysis.take_fresh_reset_input().unwrap_err(),
            ResetInputError::UntrustedInstance
        );
    }
}
#[test]
fn genuine_soft_reset_refreshes_only_independent_reset_input_not_audio_origin() {
    let mut machine = observed();
    let first = machine.take_fresh_reset_input().unwrap();
    tick(&mut machine);
    machine.soft_reset();
    let second = machine.take_fresh_reset_input().unwrap();
    assert_eq!(second.evidence().cause, ResetCause::SoftReset);
    assert_eq!(second.evidence().instance, first.evidence().instance);
    assert_eq!(second.evidence().reset_ordinal, 2);
    let (_, bus) = machine.reference_parts();
    assert!(!bus.timing_survey.as_ref().unwrap().reference_proof.live_origin.0);
}
#[test]
fn genuine_hard_reset_moves_only_reset_lineage_and_is_physically_inert() {
    let mut baseline = ordinary();
    let mut proof = observed();
    let first = proof.take_fresh_reset_input().unwrap();
    tick(&mut baseline);
    tick(&mut proof);
    baseline.hard_reset(&mut Saves);
    proof.hard_reset(&mut Saves);
    assert_eq!(physical_bytes(&baseline), physical_bytes(&proof));
    let input = proof.take_fresh_reset_input().unwrap();
    assert_eq!(input.evidence().cause, ResetCause::HardReset);
    assert_eq!(input.evidence().previous_instance, Some(first.evidence().instance));
    assert_ne!(input.evidence().instance, first.evidence().instance);
    let (_, bus) = proof.reference_parts();
    let reference = &bus.timing_survey.as_ref().unwrap().reference_proof;
    assert!(!reference.live_origin.0);
    assert!(!reference.observed_from_power_on);
}
#[test]
fn nonoriginal_execution_cannot_be_erased_by_a_later_actual_reset() {
    let mut machine = observed();
    machine.hybrid_begin_timing_instruction(GenesisInputs::default());
    machine.soft_reset();
    assert_eq!(machine.reference_reset_observation().unwrap().failure, None);
    assert_eq!(machine.take_fresh_reset_input().unwrap_err(), ResetInputError::UntrustedInstance);
}
#[test]
fn changing_entry_after_a_private_boundary_is_detected_and_consumes_freshness() {
    let mut machine = observed();
    // Private unit fault injection bypasses the public escape guard to exercise
    // the independent boundary comparison, not to manufacture positive evidence.
    machine.reference_parts().0.set_data_registers([1; 8]);
    assert_eq!(machine.take_fresh_reset_input().unwrap_err(), ResetInputError::EntryChanged);
    assert_eq!(machine.take_fresh_reset_input().unwrap_err(), ResetInputError::NotFresh);
}
#[test]
fn raw_capture_overflow_is_diagnostic_and_never_fresh_input() {
    let mut machine = observed();
    let before = machine.reference_reset_observation().unwrap().before;
    let (cpu, bus) = machine.reference_parts();
    let now = ResetBoundary::capture(cpu, bus);
    let proof = &mut bus.timing_survey.as_mut().unwrap().reference_proof;
    proof.begin_reset(before);
    proof.reset_line_at_execute(true);
    for address in 0..=MAX_RESET_ACCESSES {
        proof.reset_access(ResetAccess::Read {
            address: address as u32 * 2,
            word: true,
            value: 0,
            purpose: ResetReadPurpose::Semantic,
            raw_reset_line: true,
            open_bus_before: 0,
            open_bus_after: 0,
        });
    }
    proof.finish_reset(now, 132);
    assert_eq!(
        proof.reset.observation.as_ref().unwrap().failure,
        Some(ResetCaptureFailure::AccessOverflow)
    );
    assert!(machine.take_fresh_reset_input().is_err());
}
#[test]
fn raw_observer_never_credits_a_write_issue_as_a_completed_reset_read() {
    let mut machine = observed();
    let before = machine.reference_reset_observation().unwrap().before;
    let (cpu, bus) = machine.reference_parts();
    let now = ResetBoundary::capture(cpu, bus);
    let proof = &mut bus.timing_survey.as_mut().unwrap().reference_proof;
    proof.begin_reset(before);
    proof.reset_line_at_execute(true);
    proof.reset_access(ResetAccess::WriteIssued { address: 0xff8000, word: true, value: 0 });
    proof.finish_reset(now, 132);
    assert!(proof.reset.observation.as_ref().unwrap().failure.is_some());
    assert!(machine.take_fresh_reset_input().is_err());
}
#[test]
fn observer_metadata_drain_is_not_a_mutable_machine_escape() {
    let mut machine = observed();
    drop(machine.drain_reference_timing_accesses());
    machine.take_fresh_reset_input().unwrap();
}
