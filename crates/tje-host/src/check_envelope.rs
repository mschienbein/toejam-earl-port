//! Check-only clock envelopes. These never alter the source execution class.

use crate::{HostError, Result, stack_dependency::CheckRun};
use genesis_core::GenesisEmulator;
use tje_game::execution::{ExecutionClass, ProofEnvelope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckForkRoute {
    FrozenAtomic,
    FrozenFunctional,
    TimeAwareLogic,
    ClockedCommutingInterrupts,
    ClockedStrict,
}

impl CheckForkRoute {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FrozenAtomic => "FrozenAtomic",
            Self::FrozenFunctional => "FrozenFunctional",
            Self::TimeAwareLogic => "TimeAwareLogic",
            Self::ClockedCommutingInterrupts => "ClockedCommutingInterrupts",
            Self::ClockedStrict => "ClockedStrict",
        }
    }

    pub const fn for_contract(class: ExecutionClass, envelope: ProofEnvelope) -> Self {
        match envelope {
            ProofEnvelope::ClockedStrict => Self::ClockedStrict,
            ProofEnvelope::ClockedCommutingInterrupts => Self::ClockedCommutingInterrupts,
            ProofEnvelope::ClassDefined => match class {
                ExecutionClass::Atomic => Self::FrozenAtomic,
                ExecutionClass::Functional => Self::FrozenFunctional,
                ExecutionClass::TimeAware => Self::TimeAwareLogic,
            },
        }
    }

    pub const fn progresses_time(self) -> bool {
        matches!(
            self,
            Self::TimeAwareLogic | Self::ClockedCommutingInterrupts | Self::ClockedStrict
        )
    }

    pub const fn normalizes_functional_latency(self) -> bool {
        matches!(self, Self::FrozenFunctional | Self::TimeAwareLogic)
    }

    pub const fn uses_logic_projection(self) -> bool {
        matches!(self, Self::TimeAwareLogic)
    }

    /// Until paired commutation certificates are integrated, that declaration
    /// admits only a fully exact identity case, never the old logic quotient.
    pub const fn is_strict(self) -> bool {
        matches!(self, Self::ClockedStrict | Self::ClockedCommutingInterrupts)
    }
}

/// Arm actual interval observations before either source body executes. A prior
/// functional normalization or experimental retiming policy is not strict input.
pub(crate) fn install_strict_observers(machine: &mut GenesisEmulator) -> Result<()> {
    let bus = machine.hybrid_bus_mut();
    if bus.hybrid_functional_mode() {
        return Err(HostError::Metadata(
            "ClockedStrict requires an unnormalized real-clock entry state".into(),
        ));
    }
    if bus
        .timing_survey
        .as_ref()
        .is_some_and(|survey| survey.policy.is_some())
    {
        return Err(HostError::Metadata(
            "ClockedStrict rejects an experimental device-retiming policy".into(),
        ));
    }
    bus.native_cpu_accesses.get_or_insert_with(Vec::new).clear();
    bus.native_ram_writes.get_or_insert_with(Vec::new).clear();
    bus.enable_native_write_trace();
    bus.native_writes
        .as_mut()
        .expect("enabled actual dispatch trace")
        .clear();
    bus.timing_survey
        .as_mut()
        .expect("enabled actual timing observer")
        .accesses
        .clear();
    Ok(())
}

/// No ABI/dead-stack, undefined-bit, ISR-poll, clock, VDP-phase or audio quotient.
/// This is intentionally stronger than ordinary readable-layer logic checking.
pub(crate) fn compare_strict(
    original: &GenesisEmulator,
    translated: &GenesisEmulator,
    original_run: &CheckRun,
    translated_run: &CheckRun,
) -> std::result::Result<u64, String> {
    let (a, b) = (original.hybrid_bus(), translated.hybrid_bus());
    if original.hybrid_cpu().hybrid_state_words() != translated.hybrid_cpu().hybrid_state_words() {
        return Err("ClockedStrict CPU/register/banked-stack/prefetch/IRQ state differs".into());
    }
    if a.hybrid_functional_mode() || b.hybrid_functional_mode() {
        return Err("ClockedStrict fork was functionally normalized".into());
    }
    match (&a.native_cpu_accesses, &b.native_cpu_accesses) {
        (Some(a), Some(b)) if a == b => {}
        (Some(_), Some(_)) => {
            return Err("ClockedStrict raw CPU-issued access stream differs".into());
        }
        _ => return Err("ClockedStrict lacks its actual CPU-access observer".into()),
    }
    match (&a.native_ram_writes, &b.native_ram_writes) {
        (Some(a), Some(b)) if a == b => {}
        (Some(_), Some(_)) => return Err("ClockedStrict actual RAM-commit stream differs".into()),
        _ => return Err("ClockedStrict lacks its actual RAM-commit observer".into()),
    }
    match (&a.native_writes, &b.native_writes) {
        (Some(a), Some(b)) if a == b => {}
        (Some(_), Some(_)) => return Err("ClockedStrict timed chip-dispatch stream differs".into()),
        _ => return Err("ClockedStrict lacks its actual timed chip-dispatch observer".into()),
    }
    let original_reads = original_run
        .audit
        .stack_reads
        .as_ref()
        .ok_or("ClockedStrict lacks its complete DMA-source journal")?;
    let translated_reads = translated_run
        .audit
        .stack_reads
        .as_ref()
        .ok_or("ClockedStrict lacks its complete DMA-source journal")?;
    if !original_reads
        .iter()
        .filter(|event| event.read.origin == "vdp_dma")
        .map(|event| &event.read)
        .eq(translated_reads
            .iter()
            .filter(|event| event.read.origin == "vdp_dma")
            .map(|event| &event.read))
    {
        return Err("ClockedStrict ordered actual DMA-source reads differ".into());
    }
    if !original_run
        .audio
        .iter()
        .flat_map(|sample| sample.iter())
        .map(|sample| sample.to_bits())
        .eq(translated_run
            .audio
            .iter()
            .flat_map(|sample| sample.iter())
            .map(|sample| sample.to_bits()))
    {
        return Err("ClockedStrict emitted audio sample bits differ".into());
    }
    // Use the same complete encoder as the existing source-timing verifier.
    // The CPU observer's diagnostic instruction counts are deliberately not
    // physical state; device timing, pending writes, FIFOs, DMA, RAM, raster,
    // resampler, input and raw timing observations remain encoded and exact.
    let expected = bincode::encode_to_vec(original, bincode::config::standard())
        .map_err(|error| format!("ClockedStrict original state encoding failed: {error}"))?;
    let actual = bincode::encode_to_vec(translated, bincode::config::standard())
        .map_err(|error| format!("ClockedStrict Rust state encoding failed: {error}"))?;
    if expected != actual {
        let offset = expected
            .iter()
            .zip(&actual)
            .position(|(a, b)| a != b)
            .unwrap_or(expected.len().min(actual.len()));
        return Err(format!(
            "ClockedStrict complete CPU/Z80/device/RAM/clock/timing state differs at encoded byte {offset}"
        ));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        abi::AtomicRun,
        check_projection::LogicTrace,
        sinks::{HeadlessAudio, HeadlessOutput, Inputs, NoCartridgeSaves},
    };
    use jgenesis_common::frontend::{EmulatorTrait, PartialClone};
    use tje_game::exits::EntryOutcome;

    fn observations() -> CheckRun {
        CheckRun {
            audit: AtomicRun {
                stack_reads: Some(Vec::new()),
                ..AtomicRun::default()
            },
            outcome: EntryOutcome::Returned(None),
            logic_trace: LogicTrace::default(),
            nested_calls: 0,
            wait_frames: 0,
            dependent_writes: None,
            exact_device_reads: Vec::new(),
            audio: Vec::new(),
        }
    }

    #[test]
    fn strict_comparator_rejects_dropped_real_step_pending_state_and_audio() {
        let mut host = crate::fixture::tests::synthetic_host(&[0x4e71, 0x60fc], &[0x4e73]);
        install_strict_observers(&mut host.emulator).unwrap();
        let mut omitted = host.emulator.partial_clone();
        let a = observations();
        let mut b = observations();
        assert_eq!(compare_strict(&host.emulator, &omitted, &a, &b), Ok(0));
        host.emulator
            .tick(
                &mut HeadlessOutput,
                &mut HeadlessAudio::default(),
                &mut Inputs(Default::default()),
                &mut NoCartridgeSaves,
            )
            .unwrap();
        assert!(compare_strict(&host.emulator, &omitted, &a, &b).is_err());
        // No fake cycles or state repair: a freshly cloned exact checkpoint is
        // only the identity unit case, never evidence for a candidate body.
        omitted = host.emulator.partial_clone();
        b.audio.push([1.0, 0.0]);
        assert!(
            compare_strict(&host.emulator, &omitted, &a, &b)
                .unwrap_err()
                .contains("audio")
        );
        omitted.hybrid_bus_mut().hybrid_set_functional_mode(true);
        assert!(compare_strict(&host.emulator, &omitted, &a, &a).is_err());
    }

    #[test]
    fn class_and_route_are_independent_and_strict_never_normalizes() {
        for class in [
            ExecutionClass::Atomic,
            ExecutionClass::Functional,
            ExecutionClass::TimeAware,
        ] {
            let strict = CheckForkRoute::for_contract(class, ProofEnvelope::ClockedStrict);
            assert!(strict.progresses_time());
            assert!(!strict.normalizes_functional_latency());
            assert!(!strict.uses_logic_projection());
        }
        assert_eq!(
            CheckForkRoute::for_contract(ExecutionClass::Functional, ProofEnvelope::ClassDefined),
            CheckForkRoute::FrozenFunctional,
        );
    }
}
