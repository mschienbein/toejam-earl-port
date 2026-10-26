//! TJE opt-in reference proof hooks. Authored audio intervals are never natural
//! play, production eligibility, or a proof that a Rust body is equivalent.
//! Only the original CPU/tick/physical-commit loop can advance this recorder.
use crate::api::GenesisEmulator;
use bincode::{Decode, Encode};
use genesis_components::memory::Memory;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use thiserror::Error;

mod reset;
pub use reset::{
    FreshResetInput, ResetAccess, ResetBoundary, ResetCaptureFailure, ResetCause, ResetClocks,
    ResetInputError, ResetObservation, ResetReadPurpose,
};
pub(crate) use reset::{ResetContinuation, cartridge_identity};

mod read_context;
pub use passive_observer::{PassiveJournalError, PassiveJournalLoan, PassiveJournalView};
pub use read_context::ReadContext;

const BUSREQ: u32 = 0xa11100;
const LOCK: u16 = 0x41;
const RELEASED_TICKS: u16 = 112;
// Diagnostic/operational ceiling only: not a proved source bound or liveness guarantee.
const ORIGINAL_INSTRUCTION_DIAGNOSTIC_CEILING: u64 = 1024;

// Archived or speculative copies cannot mint live-origin authority.
#[derive(Debug, Default, jgenesis_proc_macros::FakeEncode, jgenesis_proc_macros::FakeDecode)]
struct LiveOrigin(bool);
impl Clone for LiveOrigin {
    fn clone(&self) -> Self {
        Self(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetailRevision {
    Rev02,
    Rev00,
}
impl RetailRevision {
    fn sha1(self) -> &'static [u8; 40] {
        match self {
            Self::Rev02 => b"85e8d0a4fac591b25b77c35680ac4175976f251b",
            Self::Rev00 => b"7f82d8b57fff88bdca5d8aff85b01e231dc1239a",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioOwner {
    Pcm,
    Psg,
}
impl AudioOwner {
    #[must_use]
    pub const fn entry(self, revision: RetailRevision) -> u32 {
        match (self, revision) {
            (Self::Pcm, RetailRevision::Rev02) => 0x3f62a,
            (Self::Pcm, RetailRevision::Rev00) => 0x3f5ba,
            (Self::Psg, RetailRevision::Rev02) => 0x3f7aa,
            (Self::Psg, RetailRevision::Rev00) => 0x3f73a,
        }
    }
    #[must_use]
    pub const fn retry(self, revision: RetailRevision) -> u32 {
        self.entry(revision) - 14
    }
}
/// Input identity and the actual original opcode ordinal (one-based from power-on).
/// No field can choose another RAM byte, value, duration, CPU state or clock.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioRetryRecipe {
    pub id: String,
    pub session: String,
    pub session_sha1: String,
    pub revision: RetailRevision,
    pub frame: u64,
    pub original_instruction_ordinal: u64,
    pub owner: AudioOwner,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize)]
pub struct OriginalInstruction {
    pub frame: u64,
    pub ordinal: u64,
    pub pc: u32,
    pub completed_successfully: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize)]
pub struct CommitWitness {
    pub instruction: OriginalInstruction,
    pub event_ordinal: u64,
    pub clock: u64,
    pub scoped_cpu_access_ordinal: u64,
    pub scoped_physical_commit_ordinal: u64,
    pub address: u32,
    pub word: bool,
    pub value: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ByteOrigin {
    AuthoredActivation,
    AuthoredExpiry,
    OriginalM68k,
    OriginalZ80,
    Unattributed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize)]
pub struct PhysicalByteEvent {
    pub sequence: u64,
    pub event_ordinal: u64,
    pub origin: ByteOrigin,
    pub frame: u64,
    pub clock: u64,
    pub pc: Option<u32>,
    pub address: u32,
    pub before: u8,
    pub value: u8,
    pub eligible_ticks_after_release: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize)]
pub struct EligibleTickWitness {
    pub ordinal_after_release: u16,
    pub event_ordinal: u64,
    pub z80_pc: u16,
    pub clock_at_tick_start: u64,
    pub released: bool,
    pub not_reset: bool,
    pub not_vdp_halted: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioIntervalStatus {
    Armed,
    Active,
    Released,
    Expired,
    Completed,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    ObservationNotFromPowerOn,
    BindingMismatch,
    CartridgeMismatch,
    InvalidBoundary,
    BusyNotZero,
    CommandNotZero,
    BusNotReleased,
    ExistingWatch,
    NotOriginal,
    UnexpectedCommit,
    UnexpectedByteWriter,
    UnobservedWriter,
    InstructionCeilingExceeded,
    ResetDuringInterval,
    IncompleteInterval,
    WrongReturn,
    AlreadyArmed,
    UntrustedOrigin,
    MutableEscape,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error, Encode, Decode, Serialize)]
#[error("authored reference audio fixture failed: {kind:?} at frame {frame}, clock {clock}")]
pub struct ProofFailure {
    pub kind: FailureKind,
    pub frame: u64,
    pub clock: u64,
}
#[derive(Debug, Clone, Encode, Decode, Serialize)]
pub struct AudioRetryReceipt {
    pub recipe: AudioRetryRecipe,
    pub natural: bool,
    pub cartridge_identity_verified: bool,
    pub status: AudioIntervalStatus,
    pub expected_initial_busy: u8,
    pub expected_initial_command: u8,
    pub caller_sp: u32,
    pub caller_return_pc: u32,
    pub source_call: Option<OriginalInstruction>,
    pub activation: Option<CommitWitness>,
    pub first_retry_release: Option<CommitWitness>,
    pub eligible_ticks: Vec<EligibleTickWitness>,
    pub byte_events: Vec<PhysicalByteEvent>,
    pub owner_return: Option<OriginalInstruction>,
    pub failure: Option<ProofFailure>,
}

/// Live, borrow-only completion witness. Archival receipt bytes cannot construct
/// this type, and it cannot be cloned, decoded or used as native admission.
#[derive(Debug)]
pub struct CompletedAudioRetryFixture<'a> {
    receipt: &'a AudioRetryReceipt,
    _origin: &'a LiveOrigin,
}
impl CompletedAudioRetryFixture<'_> {
    #[must_use]
    pub fn evidence(&self) -> &AudioRetryReceipt {
        self.receipt
    }
}
#[derive(Debug, Clone, Encode, Decode)]
struct AudioInterval {
    receipt: AudioRetryReceipt,
    cpu_accesses: u64,
    physical_commits: u64,
    byte_sequence: u64,
}
/// Shared proof-only bridge; independent passive observers may be added here.
/// Enabling the Cargo feature neither enables observation nor arms a mutation.
#[derive(Debug, Clone, Default, Encode, Decode)]
pub(crate) struct ReferenceProof {
    frame: u64,
    observed_from_power_on: bool,
    original: Option<OriginalInstruction>,
    last_original: Option<OriginalInstruction>,
    original_exit: Option<[u32; 25]>,
    step_epoch: u64,
    original_exit_epoch: Option<u64>,
    origin_violation: Option<ProofFailure>,
    z80: Option<(u16, u64)>,
    audio: Option<AudioInterval>,
    failure: Option<ProofFailure>,
    event_ordinal: u64,
    live_origin: LiveOrigin,
    reset: reset::ResetLane,
    passive: passive_observer::State,
}
impl ReferenceProof {
    fn fail(&mut self, kind: FailureKind, clock: u64) -> ProofFailure {
        let failure = *self.failure.get_or_insert(ProofFailure { kind, frame: self.frame, clock });
        if let Some(audio) = &mut self.audio {
            audio.receipt.status = AudioIntervalStatus::Failed;
            audio.receipt.failure = Some(failure);
        }
        failure
    }
    pub(crate) fn error(&self) -> Result<(), ProofFailure> {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        if self.audio.is_some() && !self.live_origin.0 {
            return Err(ProofFailure {
                kind: FailureKind::UntrustedOrigin,
                frame: self.frame,
                clock: 0,
            });
        }
        Ok(())
    }
    pub(crate) fn observe_power_on(&mut self) {
        self.observed_from_power_on = true;
        self.live_origin = LiveOrigin(true);
    }
    pub(crate) fn current_frame(&self) -> u64 {
        self.frame
    }
    pub(crate) fn observing_original(&self) -> bool {
        self.observed_from_power_on
    }
    pub(crate) fn has_audio_interval(&self) -> bool {
        self.audio.is_some()
    }
    pub(crate) fn begin_original_attempt(&mut self) {
        self.reset_progress();
        if !self.observed_from_power_on {
            return;
        }
        self.step_epoch += 1;
    }
    fn require_audio_origin(&mut self, clock: u64) -> Result<(), ProofFailure> {
        self.error()?;
        if let Some(violation) = self.origin_violation {
            self.failure.get_or_insert(violation);
            return Err(violation);
        }
        if !self.observed_from_power_on {
            return Err(self.fail(FailureKind::ObservationNotFromPowerOn, clock));
        }
        if !self.live_origin.0 {
            return Err(self.fail(FailureKind::UntrustedOrigin, clock));
        }
        Ok(())
    }
    fn invalidate_audio_origin(&mut self, kind: FailureKind, clock: u64) {
        if self.observed_from_power_on || self.audio.is_some() {
            let violation = *self.origin_violation.get_or_insert(ProofFailure {
                kind,
                frame: self.frame,
                clock,
            });
            self.live_origin.0 = false;
            self.original = None;
            self.z80 = None;
            self.original_exit = None;
            self.original_exit_epoch = None;
            if self.audio.is_some() {
                self.fail(violation.kind, violation.clock);
            }
        }
    }
    pub(crate) fn public_mutable_escape(&mut self, clock: u64) {
        self.passive.invalidate();
        self.invalidate_reset_instance();
        self.invalidate_audio_origin(FailureKind::MutableEscape, clock);
    }
    fn fresh_exit_matches(&self, words: &[u32; 25], ordinal: u64, pc: u32) -> bool {
        self.original_exit_epoch == Some(self.step_epoch)
            && self.original_exit.as_ref() == Some(words)
            && self.last_original.is_some_and(|step| {
                step.completed_successfully && step.ordinal == ordinal && step.pc == pc
            })
    }
    pub(crate) fn wait_step(&mut self) {
        if !self.observed_from_power_on {
            return;
        }
        self.original = None;
        self.z80 = None;
    }
    pub(crate) fn frame_rendered(&mut self) {
        if !self.observed_from_power_on {
            return;
        }
        self.frame += 1;
    }
    pub(crate) fn cpu_access(&mut self) {
        if let Some(audio) = &mut self.audio {
            audio.cpu_accesses += 1;
        }
    }
    pub(crate) fn physical_commit(&mut self) {
        if self.audio.is_some() {
            self.event_ordinal += 1;
        }
        if let Some(audio) = &mut self.audio {
            audio.physical_commits += 1;
        }
    }
    pub(crate) fn original_step(
        &mut self,
        original: Option<OriginalInstruction>,
        exit: Option<[u32; 25]>,
        clock: u64,
    ) {
        if !self.observed_from_power_on {
            return;
        }
        self.original = original;
        self.z80 = None;
        if original.is_some() {
            self.last_original = original;
            self.original_exit = exit;
            self.original_exit_epoch = exit.map(|_| self.step_epoch);
        }
        if let Err(failure) = self.error() {
            self.fail(failure.kind, clock);
            return;
        }
        if let Some(audio) = &self.audio
            && !matches!(
                audio.receipt.status,
                AudioIntervalStatus::Completed | AudioIntervalStatus::Failed
            )
        {
            let Some(step) = original.filter(|step| step.completed_successfully) else {
                self.fail(FailureKind::NotOriginal, clock);
                return;
            };
            if step.ordinal.saturating_sub(audio.receipt.recipe.original_instruction_ordinal)
                > ORIGINAL_INSTRUCTION_DIAGNOSTIC_CEILING
            {
                self.fail(FailureKind::InstructionCeilingExceeded, clock);
            }
        }
    }
    pub(crate) fn reject_non_original(&mut self, clock: u64) {
        self.invalidate_reset_instance();
        if !self.observed_from_power_on {
            return;
        }
        self.invalidate_audio_origin(FailureKind::NotOriginal, clock);
        self.original = None;
    }
    pub(crate) fn begin_z80_tick(&mut self, pc: u16, clock: u64) {
        if !self.observed_from_power_on {
            return;
        }
        self.z80 = Some((pc, clock));
    }
    pub(crate) fn end_z80_tick(&mut self) {
        if !self.observed_from_power_on {
            return;
        }
        self.z80 = None;
    }
    fn record_byte(
        &mut self,
        memory: &mut Memory,
        origin: ByteOrigin,
        clock: u64,
        pc: Option<u32>,
    ) {
        while let Some(write) = memory.pop_audio_fixture_write() {
            let Some(audio) = &mut self.audio else {
                return;
            };
            let lost_write = audio.byte_sequence.checked_add(1) != Some(write.sequence);
            audio.byte_sequence = write.sequence;
            self.event_ordinal += 1;
            audio.receipt.byte_events.push(PhysicalByteEvent {
                sequence: write.sequence,
                event_ordinal: self.event_ordinal,
                origin,
                frame: self.frame,
                clock,
                pc,
                address: 0xa00041,
                before: write.before,
                value: write.value,
                eligible_ticks_after_release: audio.receipt.eligible_ticks.len() as u16,
            });
            let unexpected =
                !matches!(origin, ByteOrigin::AuthoredActivation | ByteOrigin::AuthoredExpiry)
                    && !matches!(
                        audio.receipt.status,
                        AudioIntervalStatus::Expired
                            | AudioIntervalStatus::Completed
                            | AudioIntervalStatus::Failed
                    );
            if lost_write {
                self.fail(FailureKind::UnobservedWriter, clock);
            } else if unexpected {
                self.fail(FailureKind::UnexpectedByteWriter, clock);
            }
            // Preserve/log every actual write even after the first sticky failure.
            // No reassertion or expiry mutation is allowed after it.
        }
    }
    pub(crate) fn check_unclaimed(&mut self, memory: &mut Memory, clock: u64) {
        if self.audio.is_none() {
            return;
        }
        self.record_byte(memory, ByteOrigin::Unattributed, clock, None);
        if let Err(failure) = self.error() {
            self.fail(failure.kind, clock);
            return;
        }
        let Some(audio) = &self.audio else {
            return;
        };
        if memory.audio_fixture_watch_status() != Some((audio.byte_sequence, false)) {
            self.fail(FailureKind::UnobservedWriter, clock);
        }
    }
    pub(crate) fn physical_audio_write(&mut self, memory: &mut Memory, clock: u64) {
        let (origin, clock, pc) = if !self.live_origin.0 {
            (ByteOrigin::Unattributed, clock, None)
        } else if let Some((pc, tick_clock)) = self.z80 {
            (ByteOrigin::OriginalZ80, tick_clock, Some(u32::from(pc)))
        } else if let Some(original) = self.original.filter(|step| step.completed_successfully) {
            (ByteOrigin::OriginalM68k, clock, Some(original.pc))
        } else {
            (ByteOrigin::Unattributed, clock, None)
        };
        self.record_byte(memory, origin, clock, pc);
    }
    pub(crate) fn busreq_commit(
        &mut self,
        memory: &mut Memory,
        clock: u64,
        word: bool,
        value: u16,
        m68k: bool,
        reset: bool,
    ) {
        self.check_unclaimed(memory, clock);
        if self.failure.is_some() {
            return;
        }
        let Some(audio) = &mut self.audio else {
            return;
        };
        let Some(step) = self.original.filter(|step| step.completed_successfully && m68k) else {
            self.fail(FailureKind::NotOriginal, clock);
            return;
        };
        let witness = CommitWitness {
            instruction: step,
            event_ordinal: self.event_ordinal,
            clock,
            scoped_cpu_access_ordinal: audio.cpu_accesses,
            scoped_physical_commit_ordinal: audio.physical_commits,
            address: BUSREQ,
            word,
            value,
        };
        match audio.receipt.status {
            AudioIntervalStatus::Armed => {
                let recipe = &audio.receipt.recipe;
                if !word
                    || value != 0x100
                    || reset
                    || step.pc != recipe.owner.entry(recipe.revision)
                    || step.ordinal != recipe.original_instruction_ordinal
                    || step.frame != recipe.frame
                {
                    self.fail(FailureKind::UnexpectedCommit, clock);
                    return;
                }
                if memory.read_audio_ram(LOCK) != 0 {
                    self.fail(FailureKind::BusyNotZero, clock);
                    return;
                }
                if memory.read_audio_ram(0x40) != 0 {
                    self.fail(FailureKind::CommandNotZero, clock);
                    return;
                }
                audio.receipt.activation = Some(witness);
                audio.receipt.status = AudioIntervalStatus::Active;
                memory.write_audio_ram(LOCK, 1);
                self.record_byte(memory, ByteOrigin::AuthoredActivation, clock, Some(step.pc));
            }
            AudioIntervalStatus::Active => {
                let recipe = &audio.receipt.recipe;
                if !word || value != 0 || step.pc != recipe.owner.retry(recipe.revision) || reset {
                    self.fail(FailureKind::UnexpectedCommit, clock);
                    return;
                }
                audio.receipt.first_retry_release = Some(witness);
                audio.receipt.status = AudioIntervalStatus::Released;
            }
            AudioIntervalStatus::Released => {
                self.fail(FailureKind::UnexpectedCommit, clock);
            }
            AudioIntervalStatus::Expired
            | AudioIntervalStatus::Completed
            | AudioIntervalStatus::Failed => {}
        }
    }
    pub(crate) fn tick_tail(
        &mut self,
        memory: &mut Memory,
        pc: u16,
        clock: u64,
        released: bool,
        not_reset: bool,
        not_vdp_halted: bool,
    ) {
        self.check_unclaimed(memory, clock);
        if self.failure.is_some() {
            return;
        }
        let Some(audio) = &mut self.audio else {
            return;
        };
        if !matches!(
            audio.receipt.status,
            AudioIntervalStatus::Active | AudioIntervalStatus::Released
        ) {
            return;
        }
        if !not_reset {
            self.fail(FailureKind::ResetDuringInterval, clock);
            return;
        }
        if audio.receipt.status != AudioIntervalStatus::Released
            || !(released && not_reset && not_vdp_halted)
        {
            return;
        }
        let ordinal = audio.receipt.eligible_ticks.len() as u16 + 1;
        self.event_ordinal += 1;
        audio.receipt.eligible_ticks.push(EligibleTickWitness {
            ordinal_after_release: ordinal,
            event_ordinal: self.event_ordinal,
            z80_pc: pc,
            clock_at_tick_start: clock,
            released,
            not_reset,
            not_vdp_halted,
        });
        if ordinal == RELEASED_TICKS {
            if memory.read_audio_ram(LOCK) != 1 {
                self.fail(FailureKind::UnobservedWriter, clock);
                return;
            }
            audio.receipt.status = AudioIntervalStatus::Expired;
            memory.write_audio_ram(LOCK, 0);
            // Exact tick tail; no inferred M68k/master-clock duration or extra tick.
            self.record_byte(memory, ByteOrigin::AuthoredExpiry, clock, Some(u32::from(pc)));
        }
    }
    pub(crate) fn reset_commit(&mut self, clock: u64) {
        if let Some(audio) = &self.audio
            && !matches!(
                audio.receipt.status,
                AudioIntervalStatus::Expired
                    | AudioIntervalStatus::Completed
                    | AudioIntervalStatus::Failed
            )
        {
            self.fail(FailureKind::ResetDuringInterval, clock);
        }
    }
}
impl GenesisEmulator {
    /// Authenticate one real original owner invocation, then wait for its actual BUSREQ commit.
    /// # Errors
    /// Any failed binding/guard is sticky; there is no reset/re-arm escape hatch.
    pub fn arm_audio_retry_fixture(
        &mut self,
        recipe: AudioRetryRecipe,
    ) -> Result<(), ProofFailure> {
        let (cpu, bus) = self.reference_parts();
        let ready = cpu.hybrid_instruction_ready(&*bus);
        let proof = &mut bus.timing_survey.get_or_insert_with(Default::default).reference_proof;
        proof.error()?;
        let clock = bus.cycles.m68k_mclk_cycles;
        proof.require_audio_origin(clock)?;
        if proof.audio.is_some() {
            return Err(proof.fail(FailureKind::AlreadyArmed, clock));
        }
        let sp = cpu.stack_pointer();
        let return_pc = (0..4).fold(0, |value, offset| {
            (value << 8) | u32::from(bus.memory.read_main_ram_byte(sp.wrapping_add(offset)))
        });
        proof.audio = Some(AudioInterval {
            receipt: AudioRetryReceipt {
                recipe,
                natural: false,
                cartridge_identity_verified: false,
                status: AudioIntervalStatus::Armed,
                expected_initial_busy: 0,
                expected_initial_command: 0,
                caller_sp: sp,
                caller_return_pc: return_pc,
                source_call: None,
                activation: None,
                first_retry_release: None,
                eligible_ticks: Vec::with_capacity(usize::from(RELEASED_TICKS)),
                byte_events: Vec::with_capacity(4),
                owner_return: None,
                failure: None,
            },
            cpu_accesses: 0,
            physical_commits: 0,
            byte_sequence: 0,
        });
        let recipe = &proof.audio.as_ref().unwrap().receipt.recipe;
        let valid_hash = recipe.session_sha1.len() == 40
            && recipe.session_sha1.bytes().all(|byte| byte.is_ascii_hexdigit());
        let next = cpu
            .hybrid_instruction_counts()
            .filter(|counts| counts.enabled())
            .map(|counts| counts.main_thread + counts.exception_handler + 1);
        let binding_matches = proof.observed_from_power_on
            && valid_hash
            && !recipe.session.is_empty()
            && !recipe.id.is_empty()
            && recipe.frame == proof.frame
            && next == Some(recipe.original_instruction_ordinal)
            && cpu.pc() == recipe.owner.entry(recipe.revision);
        if !binding_matches {
            return Err(proof.fail(FailureKind::BindingMismatch, clock));
        }
        if bus.cycles.m68k_wait_cpu_cycles != 0
            || !ready
            || !bus.pending_writes.reference_is_empty()
            || !(0xff0000..=0xfffffb).contains(&(sp & 0xffffff))
        {
            return Err(proof.fail(FailureKind::InvalidBoundary, clock));
        }
        if bus.z80_signals.bus_requested()
            || bus.z80_signals.reset_asserted()
            || bus.cycles.z80_halt
        {
            return Err(proof.fail(FailureKind::BusNotReleased, clock));
        }
        if bus.memory.read_audio_ram(LOCK) != 0 {
            return Err(proof.fail(FailureKind::BusyNotZero, clock));
        }
        if bus.memory.read_audio_ram(0x40) != 0 {
            return Err(proof.fail(FailureKind::CommandNotZero, clock));
        }
        let Some(cartridge) = bus.cartridge.as_mut() else {
            return Err(proof.fail(FailureKind::CartridgeMismatch, clock));
        };
        let mut digest = Sha1::new();
        for word in cartridge.debug_rom_view().iter() {
            digest.update(word.to_be_bytes());
        }
        let actual = digest.finalize();
        let expected = recipe.revision.sha1();
        let nibble = |byte: u8| if byte.is_ascii_digit() { byte - b'0' } else { byte - b'a' + 10 };
        if !actual
            .iter()
            .zip(expected.chunks_exact(2))
            .all(|(byte, pair)| *byte == (nibble(pair[0]) << 4) | nibble(pair[1]))
        {
            return Err(proof.fail(FailureKind::CartridgeMismatch, clock));
        }
        let call = proof.last_original;
        let counts = cpu.hybrid_instruction_counts();
        let sequencer = if recipe.revision == RetailRevision::Rev02 { 0x3ed6e } else { 0x3ecfe };
        let valid_call = call.is_some_and(|call| {
            call.completed_successfully
                && (sequencer..sequencer + 764).contains(&call.pc)
                && call.ordinal + 1 == recipe.original_instruction_ordinal
                && call.pc + 6 == return_pc
                && counts
                    .and_then(|counts| counts.last_instruction)
                    .is_some_and(|last| last.pc == call.pc && last.opcode == 0x4eb9)
                && proof.original_exit == Some(cpu.hybrid_state_words())
                && {
                    let words = cartridge.debug_rom_view();
                    (u32::from(words[(call.pc as usize + 2) / 2]) << 16)
                        | u32::from(words[(call.pc as usize + 4) / 2])
                        == recipe.owner.entry(recipe.revision)
                }
        });
        if !valid_call {
            return Err(proof.fail(FailureKind::NotOriginal, clock));
        }
        proof.audio.as_mut().unwrap().receipt.source_call = call;
        if !bus.memory.begin_audio_fixture_watch() {
            return Err(proof.fail(FailureKind::ExistingWatch, clock));
        }
        proof.audio.as_mut().unwrap().receipt.cartridge_identity_verified = true;
        Ok(())
    }
    /// Archival diagnostics, including failures; never a live-origin credential.
    #[must_use]
    pub fn audio_retry_fixture_diagnostic(&self) -> Option<&AudioRetryReceipt> {
        self.hybrid_bus()
            .timing_survey
            .as_ref()?
            .reference_proof
            .audio
            .as_ref()
            .map(|audio| &audio.receipt)
    }
    /// Seal only after the original owner performs its real RTS to the existing caller frame.
    /// This never grants native-body admission or labels the authored interval as natural.
    /// # Errors
    /// Refuses incomplete intervals, wrong callers, missing original RTS and every earlier failure.
    pub fn finish_audio_retry_fixture(
        &mut self,
    ) -> Result<CompletedAudioRetryFixture<'_>, ProofFailure> {
        let (cpu, bus) = self.reference_parts();
        let proof = &mut bus.timing_survey.get_or_insert_with(Default::default).reference_proof;
        let clock = bus.cycles.m68k_mclk_cycles;
        proof.check_unclaimed(&mut bus.memory, clock);
        proof.error()?;
        proof.require_audio_origin(clock)?;
        let Some(audio) = &proof.audio else {
            return Err(proof.fail(FailureKind::IncompleteInterval, clock));
        };
        if audio.receipt.status != AudioIntervalStatus::Expired
            || audio.receipt.eligible_ticks.len() != usize::from(RELEASED_TICKS)
            || audio.receipt.activation.is_none()
            || audio.receipt.first_retry_release.is_none()
            || audio.receipt.source_call.is_none()
        {
            return Err(proof.fail(FailureKind::IncompleteInterval, clock));
        }
        let mut authored = audio
            .receipt
            .byte_events
            .iter()
            .filter(|event| {
                matches!(event.origin, ByteOrigin::AuthoredActivation | ByteOrigin::AuthoredExpiry)
            })
            .map(|event| {
                (event.origin, event.before, event.value, event.eligible_ticks_after_release)
            });
        if authored.next() != Some((ByteOrigin::AuthoredActivation, 0, 1, 0))
            || authored.next() != Some((ByteOrigin::AuthoredExpiry, 1, 0, RELEASED_TICKS))
            || authored.next().is_some()
        {
            return Err(proof.fail(FailureKind::IncompleteInterval, clock));
        }
        let counts = cpu.hybrid_instruction_counts();
        let last = counts.and_then(|counts| counts.last_instruction);
        if cpu.pc() != audio.receipt.caller_return_pc
            || cpu.stack_pointer() != audio.receipt.caller_sp.wrapping_add(4)
            || !last.is_some_and(|step| step.completed_successfully && step.opcode == 0x4e75)
            || !counts.is_some_and(|counts| {
                let last = counts.last_instruction.unwrap();
                let recipe = &audio.receipt.recipe;
                let entry = recipe.owner.entry(recipe.revision);
                let size = match recipe.owner {
                    AudioOwner::Pcm => 156,
                    AudioOwner::Psg => 84,
                };
                (entry..entry + size).contains(&last.pc)
                    && proof.fresh_exit_matches(
                        &cpu.hybrid_state_words(),
                        counts.main_thread + counts.exception_handler,
                        last.pc,
                    )
            })
        {
            return Err(proof.fail(FailureKind::WrongReturn, clock));
        }
        let counts = counts.unwrap();
        let last = last.unwrap();
        let audio = proof.audio.as_mut().unwrap();
        audio.receipt.owner_return = Some(OriginalInstruction {
            frame: proof.frame,
            ordinal: counts.main_thread + counts.exception_handler,
            pc: last.pc,
            completed_successfully: true,
        });
        audio.receipt.status = AudioIntervalStatus::Completed;
        bus.memory.finish_audio_fixture_watch();
        Ok(CompletedAudioRetryFixture { receipt: &audio.receipt, _origin: &proof.live_origin })
    }
}

#[cfg(test)]
mod tests;

// Diagnostic ownership only. This module never produces original-entry,
// reset, read, call, source-instruction or native-execution authority.
mod passive_observer {
    use super::{GenesisEmulator, ReadContext};
    use crate::bus::{HybridAccess, HybridExceptionBoundary};
    use crate::native::CpuAccess;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

    #[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
    pub enum PassiveJournalError {
        #[error("explicit instruction-counter provider setup is required before a passive loan")]
        MissingCounterProvider,
        #[error("a passive journal loan is already active")]
        AlreadyActive,
        #[error("passive journal loan belongs to another instance or completed phase")]
        ForeignOrStale,
        #[error("passive journal identity space is exhausted")]
        Exhausted,
        #[error("an installed passive journal channel was removed")]
        MissingChannel,
        #[error("a public mutable escape invalidated this passive observation")]
        MutableEscape,
        #[error("foreign exception journal index cannot be represented")]
        IndexOverflow,
    }

    /// Opaque diagnostic loan, not an instruction or original-origin receipt.
    /// No event buffers, numeric counters or caller-supplied marks are accepted.
    #[derive(Debug)]
    #[must_use = "finish the passive loan on both success and failure paths"]
    pub struct PassiveJournalLoan {
        instance: u64,
        serial: u64,
        consumed: bool,
    }

    /// Immutable newly observed suffixes only; foreign prefixes never appear as
    /// this loan's events. The live CPU/device state is not exposed mutably.
    #[derive(Debug)]
    pub struct PassiveJournalView<'a> {
        native: &'a [CpuAccess],
        chronology: &'a [HybridAccess],
        exceptions: &'a [HybridExceptionBoundary],
        reads: &'a [ReadContext],
    }
    impl PassiveJournalView<'_> {
        #[must_use]
        pub fn native(&self) -> &[CpuAccess] {
            self.native
        }
        #[must_use]
        pub fn chronology(&self) -> &[HybridAccess] {
            self.chronology
        }
        #[must_use]
        pub fn exceptions(&self) -> &[HybridExceptionBoundary] {
            self.exceptions
        }
        #[must_use]
        pub fn reads(&self) -> &[ReadContext] {
            self.reads
        }
    }

    #[derive(Debug)]
    struct Active {
        serial: u64,
        counting: bool,
        native: Option<Vec<CpuAccess>>,
        chronology: Option<Vec<HybridAccess>>,
        exceptions: Option<Vec<HybridExceptionBoundary>>,
        reads: Option<Vec<ReadContext>>,
        all_dma: bool,
        invalidated: bool,
    }

    #[derive(
        Debug, Default, jgenesis_proc_macros::FakeEncode, jgenesis_proc_macros::FakeDecode,
    )]
    pub(super) struct State {
        instance: u64,
        serial: u64,
        active: Option<Active>,
        native: Vec<CpuAccess>,
        chronology: Vec<HybridAccess>,
        exceptions: Vec<HybridExceptionBoundary>,
        reads: Vec<ReadContext>,
    }
    impl Clone for State {
        fn clone(&self) -> Self {
            Self::default()
        }
    }
    impl State {
        pub(super) fn invalidate(&mut self) {
            if let Some(active) = &mut self.active {
                active.invalidated = true;
            }
        }
        fn validate(&self, loan: &PassiveJournalLoan) -> Result<(), PassiveJournalError> {
            if loan.consumed
                || self.instance == 0
                || self.instance != loan.instance
                || self.active.as_ref().map(|active| active.serial) != Some(loan.serial)
            {
                return Err(PassiveJournalError::ForeignOrStale);
            }
            Ok(())
        }
        fn next(&mut self) -> Result<u64, PassiveJournalError> {
            if self.active.is_some() {
                return Err(PassiveJournalError::AlreadyActive);
            }
            let serial = self.serial.checked_add(1).ok_or(PassiveJournalError::Exhausted)?;
            if self.instance == 0 {
                self.instance = NEXT_INSTANCE
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| value.checked_add(1))
                    .map_err(|_| PassiveJournalError::Exhausted)?;
            }
            self.serial = serial;
            Ok(serial)
        }
    }

    impl GenesisEmulator {
        /// Explicit persistent DIAGNOSTIC setup, not temporary loan restoration.
        /// None becomes Some(enabled). An existing provider, values and enabled
        /// setting are untouched, including a disabled provider's real tallies.
        pub fn ensure_reference_instruction_counter(&mut self) {
            let (cpu, _) = self.reference_parts();
            if cpu.hybrid_instruction_counts().is_none() {
                cpu.hybrid_enable_instruction_counts();
            }
        }

        /// Configure only the existing diagnostic enabled flag, never numeric
        /// values. A live loan owns that flag until it is restored.
        pub fn set_reference_instruction_counting(
            &mut self,
            enabled: bool,
        ) -> Result<(), PassiveJournalError> {
            let (cpu, bus) = self.reference_parts();
            if bus
                .timing_survey
                .as_ref()
                .is_some_and(|survey| survey.reference_proof.passive.active.is_some())
            {
                return Err(PassiveJournalError::AlreadyActive);
            }
            if cpu.hybrid_instruction_counts().is_none() {
                return Err(PassiveJournalError::MissingCounterProvider);
            }
            cpu.hybrid_set_instruction_counting(enabled);
            Ok(())
        }

        /// Install empty vendor-owned diagnostic suffixes. Counter setup must
        /// already be explicit; absence rejects BEFORE changing any option.
        pub fn begin_reference_journal(
            &mut self,
        ) -> Result<PassiveJournalLoan, PassiveJournalError> {
            let (cpu, bus) = self.reference_parts();
            let counting = cpu
                .hybrid_instruction_counts()
                .ok_or(PassiveJournalError::MissingCounterProvider)?
                .enabled();
            let state =
                &mut bus.timing_survey.get_or_insert_with(Default::default).reference_proof.passive;
            let serial = state.next()?;
            state.native.clear();
            state.chronology.clear();
            state.exceptions.clear();
            state.reads.clear();
            let active = Active {
                serial,
                counting,
                native: bus.native_cpu_accesses.replace(std::mem::take(&mut state.native)),
                chronology: bus.hybrid_cpu_accesses.replace(std::mem::take(&mut state.chronology)),
                exceptions: bus
                    .hybrid_exception_boundaries
                    .replace(std::mem::take(&mut state.exceptions)),
                reads: bus.reference_read_contexts.replace(std::mem::take(&mut state.reads)),
                all_dma: std::mem::replace(&mut bus.hybrid_dma_all_sources, true),
                invalidated: false,
            };
            state.active = Some(active);
            cpu.hybrid_set_instruction_counting(true);
            Ok(PassiveJournalLoan { instance: state.instance, serial, consumed: false })
        }

        /// Borrow the exact suffix while this same diagnostic loan is active.
        /// A real public mutation still invalidates both origin lanes and makes
        /// this view unavailable; no recorded data can repair those origins.
        pub fn reference_journal(
            &self,
            loan: &PassiveJournalLoan,
        ) -> Result<PassiveJournalView<'_>, PassiveJournalError> {
            let bus = self.hybrid_bus();
            let state = &bus
                .timing_survey
                .as_ref()
                .ok_or(PassiveJournalError::ForeignOrStale)?
                .reference_proof
                .passive;
            state.validate(loan)?;
            if state.active.as_ref().is_some_and(|active| active.invalidated) {
                return Err(PassiveJournalError::MutableEscape);
            }
            Ok(PassiveJournalView {
                native: bus
                    .native_cpu_accesses
                    .as_deref()
                    .ok_or(PassiveJournalError::MissingChannel)?,
                chronology: bus
                    .hybrid_cpu_accesses
                    .as_deref()
                    .ok_or(PassiveJournalError::MissingChannel)?,
                exceptions: bus
                    .hybrid_exception_boundaries
                    .as_deref()
                    .ok_or(PassiveJournalError::MissingChannel)?,
                reads: bus
                    .reference_read_contexts
                    .as_deref()
                    .ok_or(PassiveJournalError::MissingChannel)?,
            })
        }

        /// Restore foreign diagnostic options/prefixes and append their actual
        /// suffixes, rebasing marker indices and retaining the prior DMA filter.
        /// Cleanup runs even after a removed channel or mutable escape, returns
        /// a typed failure and NEVER restores CPU/device/original-origin state.
        pub fn finish_reference_journal(
            &mut self,
            loan: &mut PassiveJournalLoan,
        ) -> Result<(), PassiveJournalError> {
            let (cpu, bus) = self.reference_parts();
            let state = &mut bus
                .timing_survey
                .as_mut()
                .ok_or(PassiveJournalError::ForeignOrStale)?
                .reference_proof
                .passive;
            state.validate(loan)?;
            let mut active = state.active.take().expect("validated active passive loan");
            loan.consumed = true;
            let mut error = active.invalidated.then_some(PassiveJournalError::MutableEscape);
            let native = bus.native_cpu_accesses.take();
            let chronology = bus.hybrid_cpu_accesses.take();
            let exceptions = bus.hybrid_exception_boundaries.take();
            let reads = bus.reference_read_contexts.take();
            if native.is_none() || chronology.is_none() || exceptions.is_none() || reads.is_none() {
                error.get_or_insert(PassiveJournalError::MissingChannel);
            }
            let mut native = native.unwrap_or_default();
            let mut chronology = chronology.unwrap_or_default();
            let mut exceptions = exceptions.unwrap_or_default();
            let mut reads = reads.unwrap_or_default();
            if let Some(previous) = &mut active.native {
                previous.extend_from_slice(&native);
            }
            if let Some(previous) = &mut active.reads {
                previous.extend_from_slice(&reads);
            }
            let base = active.exceptions.as_ref().map_or(0, Vec::len);
            if let Some(previous) = &mut active.exceptions {
                previous.extend_from_slice(&exceptions);
            }
            if let Some(previous) = &mut active.chronology {
                for &event in &chronology {
                    let event = match event {
                        HybridAccess::ExceptionBoundary { index } => {
                            if active.exceptions.is_none() {
                                continue;
                            }
                            let Some(index) =
                                u32::try_from(base).ok().and_then(|base| index.checked_add(base))
                            else {
                                error.get_or_insert(PassiveJournalError::IndexOverflow);
                                continue;
                            };
                            HybridAccess::ExceptionBoundary { index }
                        }
                        HybridAccess::DmaRead { address, .. }
                            if !active.all_dma && address < 0xe00000 =>
                        {
                            continue;
                        }
                        event => event,
                    };
                    previous.push(event);
                }
            }
            bus.native_cpu_accesses = active.native;
            bus.hybrid_cpu_accesses = active.chronology;
            bus.hybrid_exception_boundaries = active.exceptions;
            bus.reference_read_contexts = active.reads;
            bus.hybrid_dma_all_sources = active.all_dma;
            cpu.hybrid_set_instruction_counting(active.counting);
            native.clear();
            chronology.clear();
            exceptions.clear();
            reads.clear();
            state.native = native;
            state.chronology = chronology;
            state.exceptions = exceptions;
            state.reads = reads;
            error.map_or(Ok(()), Err)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::reference_proof::{ResetInputError, ResetObservation};
        use genesis_config::{GenesisEmulatorConfig, GenesisInputs};
        use jgenesis_common::frontend::{EmulatorTrait, PartialClone};
        use m68000_emu::ExceptionBoundary;

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
            fn persist_serialized<E: bincode::Encode>(
                &mut self,
                _: &str,
                _: E,
            ) -> Result<(), Self::Err> {
                Err(std::io::ErrorKind::Unsupported.into())
            }
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
        struct Pads(GenesisInputs);
        impl jgenesis_common::frontend::InputPoller<GenesisInputs> for Pads {
            fn poll(&mut self) -> &GenesisInputs {
                &self.0
            }
        }
        fn observed(trap: bool) -> GenesisEmulator {
            let mut rom = vec![0; 0x400];
            rom[..4].copy_from_slice(&0xff8000u32.to_be_bytes());
            rom[4..8].copy_from_slice(&0x200u32.to_be_bytes());
            rom[20..24].copy_from_slice(&0x300u32.to_be_bytes());
            rom[0x200..0x202]
                .copy_from_slice(&if trap { 0x82c2u16 } else { 0x60feu16 }.to_be_bytes());
            rom[0x202..0x204].copy_from_slice(&0x4e71u16.to_be_bytes());
            rom[0x300..0x302].copy_from_slice(&0x60feu16.to_be_bytes());
            GenesisEmulator::create_with_reference_proof(
                Some(rom),
                GenesisEmulatorConfig::default(),
                &mut Saves,
            )
        }
        fn tick(machine: &mut GenesisEmulator) {
            machine
                .tick(&mut Sink, &mut Sink, &mut Pads(GenesisInputs::default()), &mut Saves)
                .unwrap();
        }
        fn physical(machine: &GenesisEmulator) -> Vec<u8> {
            // Normalize ONLY diagnostic fields in an untrusted analysis copy,
            // never repair a live CPU/device/frame or original-origin state.
            let mut copy = machine.partial_clone();
            let (_, bus) = copy.reference_parts();
            bus.timing_survey = None;
            bus.native_cpu_accesses = None;
            bus.hybrid_cpu_accesses = None;
            bus.hybrid_exception_boundaries = None;
            bus.reference_read_contexts = None;
            bus.hybrid_writes = None;
            bus.hybrid_dma_all_sources = false;
            bincode::encode_to_vec(copy, bincode::config::standard()).unwrap()
        }
        fn reset(machine: &GenesisEmulator) -> &ResetObservation {
            machine.reference_reset_observation().unwrap()
        }

        #[test]
        fn absent_provider_rejects_before_any_change_and_setup_is_explicit() {
            let mut machine =
                GenesisEmulator::create(None, GenesisEmulatorConfig::default(), &mut Saves);
            let before = physical(&machine);
            let words = machine.hybrid_cpu().hybrid_state_words();
            assert!(machine.hybrid_cpu().hybrid_instruction_counts().is_none());
            assert!(matches!(
                machine.begin_reference_journal(),
                Err(PassiveJournalError::MissingCounterProvider)
            ));
            assert!(machine.hybrid_cpu().hybrid_instruction_counts().is_none());
            assert_eq!(physical(&machine), before);
            assert_eq!(machine.hybrid_cpu().hybrid_state_words(), words);
            machine.ensure_reference_instruction_counter();
            assert!(machine.hybrid_cpu().hybrid_instruction_counts().unwrap().enabled());
            machine.set_reference_instruction_counting(false).unwrap();
            let existing = format!("{:?}", machine.hybrid_cpu().hybrid_instruction_counts());
            machine.ensure_reference_instruction_counter();
            assert_eq!(format!("{:?}", machine.hybrid_cpu().hybrid_instruction_counts()), existing);
            let mut loan = machine.begin_reference_journal().unwrap();
            assert!(machine.hybrid_cpu().hybrid_instruction_counts().unwrap().enabled());
            assert_eq!(
                machine.set_reference_instruction_counting(false),
                Err(PassiveJournalError::AlreadyActive)
            );
            assert!(machine.reference_journal(&loan).unwrap().native().is_empty());
            machine.finish_reference_journal(&mut loan).unwrap();
            assert!(!machine.hybrid_cpu().hybrid_instruction_counts().unwrap().enabled());
            assert_eq!(physical(&machine), before);
            assert_eq!(machine.hybrid_cpu().hybrid_state_words(), words);
            assert_eq!(machine.take_fresh_reset_input().unwrap_err(), ResetInputError::NotObserved);
        }

        #[test]
        fn passive_setup_frame_trace_and_loans_preserve_reset_and_audio_origins() {
            let mut machine = observed(false);
            let cpu = machine.hybrid_cpu().hybrid_state_words();
            let boundary = reset(&machine).after;
            machine.ensure_reference_instruction_counter();
            machine.enable_hybrid_write_trace();
            machine.enable_hybrid_write_trace();
            assert_eq!(machine.drain_hybrid_writes().unwrap().count(), 0);
            let mut loan = machine.begin_reference_journal().unwrap();
            assert!(machine.reference_journal(&loan).unwrap().chronology().is_empty());
            machine.finish_reference_journal(&mut loan).unwrap();
            assert_eq!(machine.hybrid_cpu().hybrid_state_words(), cpu);
            assert_eq!(reset(&machine).after, boundary);
            let proof = &machine.hybrid_bus().timing_survey.as_ref().unwrap().reference_proof;
            assert!(proof.live_origin.0);
            assert!(proof.origin_violation.is_none());
            machine.take_fresh_reset_input().unwrap();
        }

        #[test]
        fn actual_ticks_change_no_physical_result_and_retain_real_counter_delta() {
            let mut watched = observed(false);
            let mut plain = observed(false);
            watched.ensure_reference_instruction_counter();
            watched.set_reference_instruction_counting(false).unwrap();
            let mut loan = watched.begin_reference_journal().unwrap();
            for _ in 0..4 {
                tick(&mut watched);
                tick(&mut plain);
            }
            let suffix = watched.reference_journal(&loan).unwrap();
            assert!(!suffix.native().is_empty());
            assert!(!suffix.reads().is_empty());
            assert_eq!(physical(&watched), physical(&plain));
            watched.finish_reference_journal(&mut loan).unwrap();
            let counts = watched.hybrid_cpu().hybrid_instruction_counts().unwrap();
            assert!(!counts.enabled());
            assert!(counts.main_thread + counts.exception_handler > 0);
            let proof = &watched.hybrid_bus().timing_survey.as_ref().unwrap().reference_proof;
            assert!(proof.live_origin.0);
            assert!(proof.origin_violation.is_none());
            assert_eq!(watched.take_fresh_reset_input().unwrap_err(), ResetInputError::NotFresh);
        }

        #[test]
        fn foreign_prefixes_actual_suffixes_and_exception_indices_are_preserved() {
            let mut machine = observed(true);
            machine.ensure_reference_instruction_counter();
            let prefix_native =
                CpuAccess::Read { address: 0xff1234, word: true, value: 0x1234, purpose: 0 };
            let prefix_boundary = HybridExceptionBoundary {
                master_clock: 0,
                boundary: ExceptionBoundary::Entered { vector: 30, handler: 0x340, depth: 1 },
            };
            // Authored diagnostic prefixes: no claim that they are original
            // instruction evidence or that this public escape retains origin.
            {
                let bus = machine.hybrid_bus_mut();
                bus.native_cpu_accesses = Some(vec![prefix_native]);
                bus.hybrid_cpu_accesses = Some(vec![HybridAccess::ExceptionBoundary { index: 0 }]);
                bus.hybrid_exception_boundaries = Some(vec![prefix_boundary]);
                bus.reference_read_contexts = Some(Vec::new());
                bus.hybrid_dma_all_sources = false;
            }
            let mut loan = machine.begin_reference_journal().unwrap();
            for _ in 0..4 {
                tick(&mut machine);
            }
            let suffix = machine.reference_journal(&loan).unwrap();
            let native = suffix.native().to_vec();
            let boundaries = suffix.exceptions().to_vec();
            assert!(!boundaries.is_empty());
            assert!(boundaries.iter().any(|event| matches!(
                event.boundary,
                ExceptionBoundary::Entered { vector: 5, .. }
            )));
            assert!(!native.contains(&prefix_native));
            machine.finish_reference_journal(&mut loan).unwrap();
            let bus = machine.hybrid_bus();
            let actual_native = bus.native_cpu_accesses.as_ref().unwrap();
            assert_eq!(actual_native[0], prefix_native);
            assert_eq!(&actual_native[1..], native);
            let actual_boundaries = bus.hybrid_exception_boundaries.as_ref().unwrap();
            assert_eq!(actual_boundaries[0], prefix_boundary);
            assert_eq!(&actual_boundaries[1..], boundaries);
            assert!(
                bus.hybrid_cpu_accesses
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|event| matches!(event, HybridAccess::ExceptionBoundary { index: 1 }))
            );
            assert!(!bus.hybrid_dma_all_sources);
        }

        #[test]
        fn foreign_stale_overlapping_and_clone_tokens_never_rebind() {
            let mut a = observed(false);
            let mut b = observed(false);
            a.ensure_reference_instruction_counter();
            b.ensure_reference_instruction_counter();
            let mut first = a.begin_reference_journal().unwrap();
            let mut other = b.begin_reference_journal().unwrap();
            assert!(matches!(a.begin_reference_journal(), Err(PassiveJournalError::AlreadyActive)));
            assert_eq!(
                a.finish_reference_journal(&mut other),
                Err(PassiveJournalError::ForeignOrStale)
            );
            let mut copy = a.partial_clone();
            assert_eq!(
                copy.finish_reference_journal(&mut first),
                Err(PassiveJournalError::ForeignOrStale)
            );
            assert_eq!(
                copy.take_fresh_reset_input().unwrap_err(),
                ResetInputError::UntrustedInstance
            );
            let archived = bincode::encode_to_vec(&a, bincode::config::standard()).unwrap();
            let (mut decoded, used): (GenesisEmulator, usize) =
                bincode::decode_from_slice(&archived, bincode::config::standard()).unwrap();
            assert_eq!(used, archived.len());
            assert_eq!(
                decoded.finish_reference_journal(&mut first),
                Err(PassiveJournalError::ForeignOrStale)
            );
            assert_eq!(
                decoded.take_fresh_reset_input().unwrap_err(),
                ResetInputError::UntrustedInstance
            );
            a.finish_reference_journal(&mut first).unwrap();
            b.finish_reference_journal(&mut other).unwrap();
            assert_eq!(
                a.finish_reference_journal(&mut first),
                Err(PassiveJournalError::ForeignOrStale)
            );
            assert!(matches!(
                a.reference_journal(&first),
                Err(PassiveJournalError::ForeignOrStale)
            ));
            let mut next = a.begin_reference_journal().unwrap();
            assert!(matches!(
                a.reference_journal(&first),
                Err(PassiveJournalError::ForeignOrStale)
            ));
            a.finish_reference_journal(&mut next).unwrap();
        }

        #[test]
        fn real_mutable_escape_remains_fatal_and_cleanup_does_not_renew_origin() {
            let mut machine = observed(false);
            machine.ensure_reference_instruction_counter();
            machine.set_reference_instruction_counting(false).unwrap();
            let mut loan = machine.begin_reference_journal().unwrap();
            machine.hybrid_bus_mut().native_cpu_accesses = None;
            assert!(matches!(
                machine.reference_journal(&loan),
                Err(PassiveJournalError::MutableEscape)
            ));
            assert_eq!(
                machine.finish_reference_journal(&mut loan),
                Err(PassiveJournalError::MutableEscape)
            );
            assert!(!machine.hybrid_cpu().hybrid_instruction_counts().unwrap().enabled());
            assert!(machine.hybrid_bus().native_cpu_accesses.is_none());
            assert_eq!(
                machine.take_fresh_reset_input().unwrap_err(),
                ResetInputError::UntrustedInstance
            );
            let proof = &machine.hybrid_bus().timing_survey.as_ref().unwrap().reference_proof;
            assert!(!proof.live_origin.0);
            assert!(proof.origin_violation.is_some());
        }

        #[test]
        fn missing_channel_cleanup_restores_options_without_fabricating_events() {
            let mut machine = observed(false);
            machine.ensure_reference_instruction_counter();
            let mut loan = machine.begin_reference_journal().unwrap();
            // Internal negative, not a publicly available mutation API.
            machine.reference_parts().1.reference_read_contexts = None;
            assert!(matches!(
                machine.reference_journal(&loan),
                Err(PassiveJournalError::MissingChannel)
            ));
            assert_eq!(
                machine.finish_reference_journal(&mut loan),
                Err(PassiveJournalError::MissingChannel)
            );
            let bus = machine.hybrid_bus();
            assert!(bus.native_cpu_accesses.is_none());
            assert!(bus.hybrid_cpu_accesses.is_none());
            assert!(bus.hybrid_exception_boundaries.is_none());
            assert!(bus.reference_read_contexts.is_none());
            assert!(!bus.hybrid_dma_all_sources);
            assert_eq!(
                machine.finish_reference_journal(&mut loan),
                Err(PassiveJournalError::ForeignOrStale)
            );
        }

        #[test]
        fn explicit_unwind_cleanup_restores_diagnostics_without_replacing_first_cause() {
            let mut machine = observed(false);
            machine.ensure_reference_instruction_counter();
            machine.set_reference_instruction_counting(false).unwrap();
            let mut loan = machine.begin_reference_journal().unwrap();
            let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tick(&mut machine);
                std::panic::panic_any(0x5152u32);
            }))
            .unwrap_err();
            machine.finish_reference_journal(&mut loan).unwrap();
            assert_eq!(failure.downcast_ref::<u32>(), Some(&0x5152));
            assert!(!machine.hybrid_cpu().hybrid_instruction_counts().unwrap().enabled());
            let bus = machine.hybrid_bus();
            assert!(bus.native_cpu_accesses.is_none());
            assert!(bus.hybrid_cpu_accesses.is_none());
            assert!(bus.hybrid_exception_boundaries.is_none());
            assert!(bus.reference_read_contexts.is_none());
            assert!(!bus.hybrid_dma_all_sources);
        }
    }
}
