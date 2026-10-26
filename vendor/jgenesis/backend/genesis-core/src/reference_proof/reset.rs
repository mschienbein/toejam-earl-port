//! Passive observations of the reference backend's actual reset call. This is
//! not reset execution, a console-timing claim, or root/native admission.
use super::ReferenceProof;
use crate::{api::GenesisEmulator, bus::GenesisBus, timing::CycleCounters};
use bincode::{Decode, Encode};
use m68000_emu::{M68000, ReadPurpose};
use serde::Serialize;
use sha1::{Digest, Sha1};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

const MAX_RESET_ACCESSES: usize = 32;
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

// Neither an archive nor an analysis copy can create a trusted live instance.
#[derive(Debug, Default, jgenesis_proc_macros::FakeEncode, jgenesis_proc_macros::FakeDecode)]
struct LiveResetInstance(bool);
impl Clone for LiveResetInstance {
    fn clone(&self) -> Self {
        Self(false)
    }
}
#[derive(Debug, Default, jgenesis_proc_macros::FakeEncode, jgenesis_proc_macros::FakeDecode)]
struct FreshReset(bool);
impl Clone for FreshReset {
    fn clone(&self) -> Self {
        Self(false)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode, Serialize)]
pub enum ResetCause {
    #[default]
    Constructor,
    SoftReset,
    HardReset,
}

/// Actual reference scheduler fields, not a charge derived from reset's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize)]
pub struct ResetClocks {
    pub m68k_divider: u64,
    pub m68k_divider_u32: u32,
    pub max_wait_cpu_cycles: u32,
    pub m68k_wait_cpu_cycles: u32,
    pub m68k_wait_counter: u8,
    pub m68k_mclk_cycles: u64,
    pub z80_mclk_cycles: u64,
    pub ym2612_mclk_cycles: u64,
    pub last_ym2612_drain_mclk: u64,
    pub psg_mclk_cycles: u64,
    pub m68k_refresh_counter: u32,
    pub vdp_owns_bus: bool,
    pub z80_halt: bool,
}
impl From<CycleCounters> for ResetClocks {
    fn from(c: CycleCounters) -> Self {
        let CycleCounters {
            m68k_divider,
            m68k_divider_u32,
            max_wait_cpu_cycles,
            m68k_wait_cpu_cycles,
            m68k_wait_counter,
            m68k_mclk_cycles,
            z80_mclk_cycles,
            ym2612_mclk_cycles,
            last_ym2612_drain_mclk,
            psg_mclk_cycles,
            m68k_refresh_counter,
            vdp_owns_bus,
            z80_halt,
        } = c;
        Self {
            m68k_divider: m68k_divider.get(),
            m68k_divider_u32: m68k_divider_u32.get(),
            max_wait_cpu_cycles,
            m68k_wait_cpu_cycles,
            m68k_wait_counter,
            m68k_mclk_cycles,
            z80_mclk_cycles,
            ym2612_mclk_cycles,
            last_ym2612_drain_mclk,
            psg_mclk_cycles,
            m68k_refresh_counter,
            vdp_owns_bus,
            z80_halt,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize)]
pub struct ResetBoundary {
    /// The existing immutable CPU snapshot's stable 25-word layout.
    pub cpu: [u32; 25],
    pub exception_depth: u32,
    pub last_exception_vector: Option<u32>,
    pub clocks: ResetClocks,
    pub raw_reset_line: bool,
    pub open_bus: u16,
    pub z80_bus_requested: bool,
    pub z80_reset: bool,
    pub pending_byte_writes: usize,
    pub pending_word_writes: usize,
    pub interrupt_level: u8,
    pub cpu_halt: bool,
    pub scanline: u16,
    pub scanline_mclk: u64,
}
impl ResetBoundary {
    pub(crate) fn capture(cpu: &M68000, bus: &GenesisBus) -> Self {
        let checkpoint = bus.hybrid_checkpoint();
        Self {
            cpu: cpu.hybrid_state_words(),
            exception_depth: cpu.hybrid_exception_depth(),
            last_exception_vector: cpu.hybrid_last_exception_vector(),
            clocks: bus.cycles.into(),
            raw_reset_line: checkpoint.m68k_reset,
            open_bus: bus.open_bus,
            z80_bus_requested: checkpoint.z80_bus_requested,
            z80_reset: checkpoint.z80_reset,
            pending_byte_writes: checkpoint.pending_byte_writes,
            pending_word_writes: checkpoint.pending_word_writes,
            interrupt_level: checkpoint.interrupt_level,
            cpu_halt: checkpoint.cpu_halt,
            scanline: bus.vdp.scanline(),
            scanline_mclk: bus.vdp.scanline_mclk(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize)]
pub enum ResetReadPurpose {
    Semantic,
    Discarded,
    Prefetch,
}
impl From<ReadPurpose> for ResetReadPurpose {
    fn from(purpose: ReadPurpose) -> Self {
        match purpose {
            ReadPurpose::Semantic => Self::Semantic,
            ReadPurpose::Discarded => Self::Discarded,
            ReadPurpose::Prefetch => Self::Prefetch,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize)]
pub enum ResetAccess {
    Read {
        address: u32,
        word: bool,
        value: u16,
        purpose: ResetReadPurpose,
        raw_reset_line: bool,
        open_bus_before: u16,
        open_bus_after: u16,
    },
    /// The original bus enqueues CPU stores; this is issue, not invented commit.
    WriteIssued {
        address: u32,
        word: bool,
        value: u16,
    },
    AcknowledgeInterrupt {
        level: u8,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize)]
pub enum ResetCaptureFailure {
    AccessOverflow,
    EpochOverflow,
    MissingVectorReads,
    UnexpectedResetAccess,
    ResetNotAsserted,
    IncompleteReset,
    CpuException,
}
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, Serialize)]
pub struct ResetObservation {
    pub instance: u64,
    pub previous_instance: Option<u64>,
    pub cartridge_sha1: Option<[u8; 20]>,
    pub cause: ResetCause,
    /// Actual reset invocations observed by this instance, not retired opcodes.
    pub reset_ordinal: u64,
    /// Reference-loop attempt epoch; waits are progress, not retired instructions.
    pub progress_epoch: u64,
    pub before: ResetBoundary,
    pub raw_line_at_execute: Option<bool>,
    pub after: Option<ResetBoundary>,
    /// Actual execute_instruction return. The caller still ignores this charge.
    pub returned_cpu_cycles: Option<u32>,
    pub accesses: [Option<ResetAccess>; MAX_RESET_ACCESSES],
    pub access_count: usize,
    pub failure: Option<ResetCaptureFailure>,
}

/// A genuine, fresh, one-shot provider input, NOT a root admission or capability
/// to execute a reset/RTE. Only the host's pre-fork admission may consume it.
/// Deliberately no Clone, Encode, Decode, Deserialize, or public constructor.
#[derive(Debug)]
pub struct FreshResetInput {
    observation: ResetObservation,
}
impl FreshResetInput {
    #[must_use]
    pub fn evidence(&self) -> &ResetObservation {
        &self.observation
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ResetInputError {
    #[error("passive reset observation is not enabled")]
    NotObserved,
    #[error("reset observation has no trusted live-instance lineage")]
    UntrustedInstance,
    #[error("no unconsumed fresh reset entry")]
    NotFresh,
    #[error("reset observation is invalid: {0:?}")]
    Invalid(ResetCaptureFailure),
    #[error("reset entry changed after the actual reset boundary")]
    EntryChanged,
}

#[derive(Debug, Default, Clone, Encode, Decode)]
pub(super) struct ResetLane {
    enabled: bool,
    instance: u64,
    previous_instance: Option<u64>,
    cartridge_sha1: Option<[u8; 20]>,
    next_cause: ResetCause,
    reset_ordinal: u64,
    progress_epoch: u64,
    overflowed: bool,
    active: bool,
    observation: Option<ResetObservation>,
    live_instance: LiveResetInstance,
    fresh: FreshReset,
}
/// Moved, never cloned/serialized, across a genuine hard-reset reconstruction.
#[derive(Debug)]
pub(crate) struct ResetContinuation {
    previous_instance: u64,
    live_instance: LiveResetInstance,
}

pub(crate) fn cartridge_identity(bytes: Option<&[u8]>) -> Option<[u8; 20]> {
    bytes.filter(|bytes| !bytes.is_empty()).map(|bytes| Sha1::digest(bytes).into())
}
impl ResetLane {
    fn install(
        &mut self,
        cartridge_sha1: Option<[u8; 20]>,
        continuation: Option<ResetContinuation>,
    ) {
        let instance = NEXT_INSTANCE
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .ok();
        let (previous_instance, live_instance, next_cause) = match continuation {
            Some(c) => (Some(c.previous_instance), c.live_instance, ResetCause::HardReset),
            None => (None, LiveResetInstance(true), ResetCause::Constructor),
        };
        *self = Self {
            enabled: true,
            instance: instance.unwrap_or(0),
            previous_instance,
            cartridge_sha1,
            next_cause,
            live_instance,
            overflowed: instance.is_none(),
            ..Self::default()
        };
    }
    fn progress(&mut self) {
        self.fresh.0 = false;
        if !self.enabled {
            return;
        }
        if let Some(epoch) = self.progress_epoch.checked_add(1) {
            self.progress_epoch = epoch;
        } else {
            self.overflowed = true;
        }
    }
    fn invalidate(&mut self) {
        self.fresh.0 = false;
        self.live_instance.0 = false;
    }
    fn begin(&mut self, before: ResetBoundary) {
        if !self.enabled {
            return;
        }
        self.progress();
        if let Some(ordinal) = self.reset_ordinal.checked_add(1) {
            self.reset_ordinal = ordinal;
        } else {
            self.overflowed = true;
        }
        self.observation = Some(ResetObservation {
            instance: self.instance,
            previous_instance: self.previous_instance,
            cartridge_sha1: self.cartridge_sha1,
            cause: self.next_cause,
            reset_ordinal: self.reset_ordinal,
            progress_epoch: self.progress_epoch,
            before,
            raw_line_at_execute: None,
            after: None,
            returned_cpu_cycles: None,
            accesses: [None; MAX_RESET_ACCESSES],
            access_count: 0,
            failure: self.overflowed.then_some(ResetCaptureFailure::EpochOverflow),
        });
        self.active = true;
    }
    fn access(&mut self, access: ResetAccess) {
        if !self.active {
            return;
        }
        let Some(observation) = &mut self.observation else {
            return;
        };
        if observation.access_count == MAX_RESET_ACCESSES {
            observation.failure.get_or_insert(ResetCaptureFailure::AccessOverflow);
            return;
        }
        observation.accesses[observation.access_count] = Some(access);
        observation.access_count += 1;
    }
    fn finish(&mut self, after: ResetBoundary, charge: u32) {
        if !self.active {
            return;
        }
        self.active = false;
        let Some(o) = &mut self.observation else {
            return;
        };
        o.after = Some(after);
        o.returned_cpu_cycles = Some(charge);
        if o.raw_line_at_execute != Some(true) || after.raw_reset_line {
            o.failure.get_or_insert(ResetCaptureFailure::ResetNotAsserted);
        }
        for (index, address) in [0, 2, 4, 6].into_iter().enumerate() {
            if !matches!(o.accesses[index], Some(ResetAccess::Read { address: a,
                word: true, purpose: ResetReadPurpose::Semantic, raw_reset_line: true, .. })
                if a == address)
            {
                o.failure.get_or_insert(ResetCaptureFailure::MissingVectorReads);
            }
        }
        if o.accesses[..o.access_count].iter().any(|access| {
            !matches!(access, Some(ResetAccess::Read { word: true, raw_reset_line: true, .. }))
        }) {
            o.failure.get_or_insert(ResetCaptureFailure::UnexpectedResetAccess);
        }
        if after.exception_depth != 0
            || after.last_exception_vector.is_some()
            || after.cpu[22] != 0
            || after.cpu[23] != 0
        {
            o.failure.get_or_insert(ResetCaptureFailure::CpuException);
        }
        self.fresh.0 = self.live_instance.0 && o.failure.is_none();
    }
    fn take(&mut self, now: ResetBoundary) -> Result<FreshResetInput, ResetInputError> {
        if !self.enabled {
            return Err(ResetInputError::NotObserved);
        }
        if !self.live_instance.0 {
            return Err(ResetInputError::UntrustedInstance);
        }
        let was_fresh = std::mem::take(&mut self.fresh.0);
        let o = self.observation.as_ref().ok_or(ResetInputError::NotFresh)?;
        if let Some(failure) = o.failure {
            return Err(ResetInputError::Invalid(failure));
        }
        if self.active || o.after.is_none() || o.returned_cpu_cycles.is_none() {
            return Err(ResetInputError::Invalid(ResetCaptureFailure::IncompleteReset));
        }
        if !was_fresh {
            return Err(ResetInputError::NotFresh);
        }
        if o.after != Some(now)
            || o.instance != self.instance
            || o.cartridge_sha1 != self.cartridge_sha1
            || o.progress_epoch != self.progress_epoch
        {
            return Err(ResetInputError::EntryChanged);
        }
        Ok(FreshResetInput { observation: self.observation.take().unwrap() })
    }
    fn validate_input_lineage(&self, input: &FreshResetInput) -> Result<(), ResetInputError> {
        if !self.enabled {
            return Err(ResetInputError::NotObserved);
        }
        let observation = &input.observation;
        if !self.live_instance.0
            || self.instance == 0
            || self.instance != observation.instance
            || self.cartridge_sha1 != observation.cartridge_sha1
        {
            return Err(ResetInputError::UntrustedInstance);
        }
        if self.overflowed {
            return Err(ResetInputError::Invalid(ResetCaptureFailure::EpochOverflow));
        }
        if self.active {
            return Err(ResetInputError::Invalid(ResetCaptureFailure::IncompleteReset));
        }
        if self.reset_ordinal != observation.reset_ordinal
            || self.progress_epoch < observation.progress_epoch
        {
            return Err(ResetInputError::EntryChanged);
        }
        if let Some(failure) = observation.failure {
            return Err(ResetInputError::Invalid(failure));
        }
        if observation.after.is_none() || observation.returned_cpu_cycles.is_none() {
            return Err(ResetInputError::Invalid(ResetCaptureFailure::IncompleteReset));
        }
        Ok(())
    }
}

impl ReferenceProof {
    pub(crate) fn initialize_reset_observer(&mut self, identity: Option<[u8; 20]>) {
        self.reset.install(identity, None);
    }
    pub(crate) fn continue_reset_observer(
        &mut self,
        identity: Option<[u8; 20]>,
        c: ResetContinuation,
    ) {
        self.reset.install(identity, Some(c));
    }
    pub(crate) fn reset_continuation(&mut self) -> Option<ResetContinuation> {
        if !self.reset.enabled {
            return None;
        }
        self.reset.fresh.0 = false;
        Some(ResetContinuation {
            previous_instance: self.reset.instance,
            live_instance: std::mem::take(&mut self.reset.live_instance),
        })
    }
    pub(crate) fn prepare_soft_reset(&mut self, clock: u64) {
        self.invalidate_audio_origin(super::FailureKind::MutableEscape, clock);
        self.reset.fresh.0 = false;
        self.reset.next_cause = ResetCause::SoftReset;
    }
    pub(crate) fn begin_reset(&mut self, before: ResetBoundary) {
        self.reset.begin(before);
    }
    pub(crate) fn reset_line_at_execute(&mut self, line: bool) {
        if self.reset.active
            && let Some(o) = &mut self.reset.observation
        {
            o.raw_line_at_execute = Some(line);
        }
    }
    pub(crate) fn reset_access(&mut self, access: ResetAccess) {
        self.reset.access(access);
    }
    pub(crate) fn finish_reset(&mut self, after: ResetBoundary, charge: u32) {
        self.reset.finish(after, charge);
    }
    pub(crate) fn observing_resets(&self) -> bool {
        self.reset.enabled
    }
    pub(crate) fn reset_capture_active(&self) -> bool {
        self.reset.active
    }
    pub(crate) fn reset_progress(&mut self) {
        self.reset.progress();
    }
    pub(crate) fn invalidate_reset_instance(&mut self) {
        self.reset.invalidate();
    }
}

impl GenesisEmulator {
    /// Raw archival observation only; no input/admission authority is returned.
    #[must_use]
    pub fn reference_reset_observation(&self) -> Option<&ResetObservation> {
        self.hybrid_bus().timing_survey.as_ref()?.reference_proof.reset.observation.as_ref()
    }
    /// Consume actual fresh input immediately before the host's single root
    /// admission. No fork, CPU/device effect, reset or clock advance occurs here.
    pub fn take_fresh_reset_input(&mut self) -> Result<FreshResetInput, ResetInputError> {
        let (cpu, bus) = self.reference_parts();
        let now = ResetBoundary::capture(cpu, bus);
        let proof =
            &mut bus.timing_survey.as_mut().ok_or(ResetInputError::NotObserved)?.reference_proof;
        proof.reset.take(now)
    }
    /// Check an already consumed opaque reset input against THIS live receiver
    /// at its exact fresh entry. This read-only validation does not renew the
    /// input, allocate, advance clocks, or grant root/source-step/native authority.
    ///
    /// # Errors
    /// Reject foreign, stale, mutated, non-original or copied receivers, invalid
    /// reset capture, and any progress/boundary change since the actual take.
    pub fn validate_fresh_reset_input(
        &self,
        input: &FreshResetInput,
    ) -> Result<(), ResetInputError> {
        let bus = self.hybrid_bus();
        let reset =
            &bus.timing_survey.as_ref().ok_or(ResetInputError::NotObserved)?.reference_proof.reset;
        reset.validate_input_lineage(input)?;
        if reset.progress_epoch != input.observation.progress_epoch
            || input.observation.after != Some(ResetBoundary::capture(self.hybrid_cpu(), bus))
        {
            return Err(ResetInputError::EntryChanged);
        }
        Ok(())
    }

    /// Check only the existing input's SAME trusted receiver/reset/ROM lineage.
    /// Genuine original progress is allowed; success never means fresh entry or
    /// source-step/root admission. No observation, state or clock is changed.
    ///
    /// # Errors
    /// Reject a foreign receiver, later reset, untrusted clone/archive/import,
    /// non-original execution, mutable escape, or invalid/reset-overflow state.
    pub fn validate_reset_input_lineage(
        &self,
        input: &FreshResetInput,
    ) -> Result<(), ResetInputError> {
        self.hybrid_bus()
            .timing_survey
            .as_ref()
            .ok_or(ResetInputError::NotObserved)?
            .reference_proof
            .reset
            .validate_input_lineage(input)
    }
}

#[cfg(test)]
#[path = "reset_tests.rs"]
mod tests;
