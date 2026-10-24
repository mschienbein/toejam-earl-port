//! Passive check-fork observations. These records are not commutation approvals.
//! Only actual indexed CPU boundary markers create exception/return records.

use crate::{abi::canonical_ram_address, catalogue, future_frame_watch::Side};
use genesis_core::{GenesisBus, GenesisEmulator, HybridAccess, HybridExceptionBoundary};
use m68000_emu::{BusInterface, ExceptionBoundary, HybridExecutedInstruction, M68000, ReadPurpose};
use serde::Serialize;
use std::cell::RefCell;

mod diagnostic;
mod journal;
mod journal_lease;
mod transfer;
pub use diagnostic::{CapturedSide, CheckCaptureDiagnostic};
pub use journal::{BusJournals, JournalLengths};
pub(crate) use journal_lease::JournalLease;
pub use transfer::TransferCheckpoint;

#[derive(Debug, Clone, Serialize)]
pub struct CaptureBinding {
    pub function: String,
    pub entry_pc: u32,
    pub revision: &'static str,
    pub side: Side,
    pub purpose: String,
    pub origin: String,
    pub declared_class: &'static str,
    pub proof_envelope: &'static str,
    pub actual_route: &'static str,
    pub explicit_proof_only_admission: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CpuCheckpoint {
    pub words: [u32; 25],
    pub exception_depth: u32,
    pub master_clock: u64,
    pub frame: u64,
    pub transfers: TransferCheckpoint,
}
impl CpuCheckpoint {
    pub(crate) fn capture(emulator: &GenesisEmulator, frame: u64) -> Self {
        Self::capture_parts(emulator.hybrid_cpu(), emulator.hybrid_bus(), frame)
    }

    pub(crate) fn capture_parts(cpu: &M68000, bus: &GenesisBus, frame: u64) -> Self {
        Self {
            words: cpu.hybrid_state_words(),
            exception_depth: cpu.hybrid_exception_depth(),
            master_clock: bus.cycles.m68k_mclk_cycles,
            frame,
            transfers: TransferCheckpoint::capture(bus),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AccessPoint {
    pub pc: u32,
    pub exception_depth: u32,
    pub master_clock: u64,
    pub frame: u64,
}

impl AccessPoint {
    pub(crate) fn capture(emulator: &GenesisEmulator, frame: u64) -> Self {
        Self::capture_parts(emulator.hybrid_cpu(), emulator.hybrid_bus(), frame)
    }

    pub(crate) fn capture_parts(cpu: &M68000, bus: &GenesisBus, frame: u64) -> Self {
        Self {
            pc: cpu.pc(),
            exception_depth: cpu.hybrid_exception_depth(),
            master_clock: bus.cycles.m68k_mclk_cycles,
            frame,
        }
    }
}

impl From<&CpuCheckpoint> for AccessPoint {
    fn from(checkpoint: &CpuCheckpoint) -> Self {
        Self {
            pc: checkpoint.words[17],
            exception_depth: checkpoint.exception_depth,
            master_clock: checkpoint.master_clock,
            frame: checkpoint.frame,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessActor {
    M68k,
    Native,
    Z80,
    VdpDma,
    ExternalMutation,
    Unattributed,
}
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedPurpose {
    Semantic,
    Discarded,
    Prefetch,
}
impl From<ReadPurpose> for ObservedPurpose {
    fn from(purpose: ReadPurpose) -> Self {
        match purpose {
            ReadPurpose::Semantic => Self::Semantic,
            ReadPurpose::Discarded => Self::Discarded,
            ReadPurpose::Prefetch => Self::Prefetch,
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ObservedAccess {
    pub step: Option<u64>,
    /// Exact only for an immediately observed native access; physical batches
    /// retain their actual enclosing checkpoints, never a guessed bus phase.
    pub precise_access_master_clock: Option<u64>,
    pub step_begin_master_clock: Option<u64>,
    pub observation_master_clock: u64,
    pub frame: u64,
    pub pc: Option<u32>,
    pub exception_depth: Option<u32>,
    pub actor: AccessActor,
    pub address: u32,
    pub word: bool,
    pub value: u16,
    pub writing: bool,
    pub purpose: Option<ObservedPurpose>,
    pub exact_operand: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemorySnapshot {
    pub start: u32,
    pub bytes: Vec<u8>,
    pub checkpoint: CpuCheckpoint,
}
fn physical_ram_byte(words: &[u16], address: u32) -> u8 {
    let offset = (address & 0xffff) as usize;
    let word = words[offset >> 1];
    if offset & 1 == 0 {
        (word >> 8) as u8
    } else {
        word as u8
    }
}

impl MemorySnapshot {
    fn known_stack(bus: &GenesisBus, checkpoint: CpuCheckpoint, mut bytes: Vec<u8>) -> Self {
        bytes.clear();
        let raw = bus.memory.hybrid_raw_main_ram();
        bytes.extend(
            (catalogue::STACK_BOTTOM..catalogue::STACK_TOP)
                .map(|address| physical_ram_byte(raw, address)),
        );
        Self {
            start: catalogue::STACK_BOTTOM,
            bytes,
            checkpoint,
        }
    }

    fn slice(&self, start: u32, end: u32) -> Option<Self> {
        let offset = usize::try_from(start.checked_sub(self.start)?).ok()?;
        let count = usize::try_from(end.checked_sub(start)?).ok()?;
        let bytes = self.bytes.get(offset..offset.checked_add(count)?)?.to_vec();
        Some(Self {
            start,
            bytes,
            checkpoint: self.checkpoint,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ExecutedOpcode {
    pub pc: u32,
    pub opcode: u16,
    pub exception_depth: u32,
    pub completed_successfully: bool,
}
impl From<HybridExecutedInstruction> for ExecutedOpcode {
    fn from(instruction: HybridExecutedInstruction) -> Self {
        Self {
            pc: instruction.pc,
            opcode: instruction.opcode,
            exception_depth: instruction.exception_depth,
            completed_successfully: instruction.completed_successfully,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct InstructionObservation {
    pub step: u64,
    pub instruction: ExecutedOpcode,
    pub before: CpuCheckpoint,
    pub after: CpuCheckpoint,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservedBoundary {
    pub step: Option<u64>,
    pub master_clock: u64,
    pub vector: Option<u32>,
    pub handler: Option<u32>,
    pub depth: u32,
    pub returned: bool,
    pub exception_index: Option<usize>,
    pub before: Option<CpuCheckpoint>,
    pub after: CpuCheckpoint,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "observation", rename_all = "snake_case")]
pub enum Observation {
    Access(ObservedAccess),
    Boundary(Box<ObservedBoundary>),
    Instruction(Box<InstructionObservation>),
}

#[derive(Debug, Clone, Serialize)]
pub struct ExceptionCapture {
    pub vector: u32,
    pub handler: u32,
    pub depth: u32,
    pub parent: Option<usize>,
    pub entry_master_clock: u64,
    pub interrupted: Option<CpuCheckpoint>,
    pub entered: CpuCheckpoint,
    pub returned: Option<CpuCheckpoint>,
    pub return_master_clock: Option<u64>,
    pub first_event: usize,
    pub last_event: Option<usize>,
    pub minimum_observed_ssp: u32,
    /// Actual memory images, not GPR/difference reconstruction or ownership.
    pub before_frame: Option<MemorySnapshot>,
    pub after_frame: Option<MemorySnapshot>,
    /// Actual bytes on the other completed side at these same physical lanes.
    /// This records old/current memory, not an inferred paired-frame mapping.
    pub counterpart_final_frame: Option<MemorySnapshot>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckCaptureEvidence {
    pub schema: &'static str,
    pub binding: CaptureBinding,
    pub entry: CpuCheckpoint,
    pub exit: Option<CpuCheckpoint>,
    pub final_observer_checkpoint: Option<CpuCheckpoint>,
    /// Set only at the observed main boundary, before endpoint normalization.
    pub main_end_event: Option<usize>,
    pub main_journal_lengths: Option<JournalLengths>,
    pub bus_journals: BusJournals,
    pub native_journal_scope_established: bool,
    pub execution_complete: bool,
    pub execution_error: Option<String>,
    pub observations: Vec<Observation>,
    pub exceptions: Vec<ExceptionCapture>,
    pub issues: Vec<String>,
    /// Always capture-only: no equality/admission or provenance approval is issued.
    pub admission: &'static str,
}

#[derive(Debug, Clone)]
struct Step {
    index: u64,
    before: CpuCheckpoint,
    after: Option<CpuCheckpoint>,
    instruction: Option<ExecutedOpcode>,
    before_stack: Option<MemorySnapshot>,
    before_instruction_count: Option<u64>,
    boundaries: u32,
}
#[derive(Debug, Clone)]
struct ActiveException {
    index: usize,
    before_stack: Option<MemorySnapshot>,
}
#[derive(Debug, Clone)]
struct CaptureState {
    evidence: CheckCaptureEvidence,
    next_step: u64,
    step: Option<Step>,
    active: Vec<ActiveException>,
    returned: Vec<ActiveException>,
    spare_stacks: Vec<Vec<u8>>,
}

/// A root owns this box. Ordinary borrows share its observations; Clone is a deep
/// speculative copy and never shares an event buffer or creates CPU authority.
#[derive(Debug, Clone)]
pub(crate) struct RawCheckCapture(RefCell<CaptureState>);

impl RawCheckCapture {
    pub(crate) fn new(binding: CaptureBinding, emulator: &GenesisEmulator, frame: u64) -> Self {
        Self::new_checkpoint(binding, CpuCheckpoint::capture(emulator, frame))
    }

    pub(crate) fn new_checkpoint(binding: CaptureBinding, entry: CpuCheckpoint) -> Self {
        Self(RefCell::new(CaptureState {
            evidence: CheckCaptureEvidence {
                schema: "tje-passive-check-capture-v1",
                binding,
                entry,
                exit: None,
                final_observer_checkpoint: None,
                main_end_event: None,
                main_journal_lengths: None,
                bus_journals: BusJournals::default(),
                native_journal_scope_established: false,
                execution_complete: false,
                execution_error: None,
                observations: Vec::new(),
                exceptions: Vec::new(),
                issues: Vec::new(),
                admission: "capture_only_no_projection_approval",
            },
            next_step: 0,
            step: None,
            active: Vec::new(),
            returned: Vec::new(),
            spare_stacks: Vec::new(),
        }))
    }

    pub(crate) fn begin_cpu_step(&self, emulator: &GenesisEmulator, frame: u64) {
        self.begin_cpu_step_parts(emulator.hybrid_cpu(), emulator.hybrid_bus(), frame);
    }

    pub(crate) fn begin_cpu_step_parts(&self, cpu: &M68000, bus: &GenesisBus, frame: u64) {
        let before = CpuCheckpoint::capture_parts(cpu, bus, frame);
        let count = cpu
            .hybrid_instruction_counts()
            .and_then(|counts| counts.main_thread.checked_add(counts.exception_handler));
        self.begin_cpu_checkpoint(before, bus, count);
    }

    /// Native providers pass their actual canonical CPU checkpoint, never a
    /// reconstructed wrapper or readiness prediction. The RAM view is still the
    /// borrowed actual device bus.
    pub(crate) fn begin_cpu_checkpoint(
        &self,
        before: CpuCheckpoint,
        bus: &GenesisBus,
        before_instruction_count: Option<u64>,
    ) {
        // Published hybrid_state_words order: 20=latched IRQ, 21=STOP, 22=frozen.
        // This overinclusive trigger is NOT proof of acceptance. STOP is allowed
        // when an IRQ is eligible. A disappearing line never reuses an old image.
        let mask = ((before.words[18] >> 8) & 7) as u8;
        let level = bus.interrupt_level();
        let physical_ssp = canonical_ram_address(before.words[16]);
        let may_accept = (before.words[20] != 0 || level > mask || level == 7)
            && before.words[22] == 0
            && !bus.m68k_reset
            && !bus.halt()
            && bus.cycles.m68k_wait_cpu_cycles == 0
            && before.words[16] & 0xffffff >= 0xe00000
            && (catalogue::STACK_BOTTOM + 6..=catalogue::STACK_TOP).contains(&physical_ssp);
        let mut state = self.0.borrow_mut();
        if let Some(old) = state.step.take().and_then(|step| step.before_stack) {
            state.spare_stacks.push(old.bytes);
        }
        let before_stack = if may_accept {
            let bytes = state.spare_stacks.pop().unwrap_or_default();
            Some(MemorySnapshot::known_stack(bus, before, bytes))
        } else {
            None
        };
        let index = state.next_step;
        state.next_step += 1;
        state.step = Some(Step {
            index,
            before,
            after: None,
            instruction: None,
            before_stack,
            before_instruction_count,
            boundaries: 0,
        });
    }

    pub(crate) fn finish_cpu_checkpoint(
        &self,
        after: CpuCheckpoint,
        instruction: Option<ExecutedOpcode>,
    ) {
        let mut state = self.0.borrow_mut();
        if let Some(step) = &mut state.step {
            step.after = Some(after);
            step.instruction = instruction;
        } else {
            state
                .evidence
                .issues
                .push("CPU completion has no observed pre-step checkpoint".into());
        }
    }

    pub(crate) fn finish_reference_cpu_step(&self, emulator: &GenesisEmulator, frame: u64) {
        self.finish_reference_cpu_step_parts(emulator.hybrid_cpu(), emulator.hybrid_bus(), frame);
    }

    pub(crate) fn finish_reference_cpu_step_parts(
        &self,
        cpu: &M68000,
        bus: &GenesisBus,
        frame: u64,
    ) {
        let mut issue = None;
        let instruction = {
            let state = self.0.borrow();
            let before = state
                .step
                .as_ref()
                .and_then(|step| step.before_instruction_count);
            match (before, cpu.hybrid_instruction_counts()) {
                (Some(before), Some(counts)) => {
                    match counts
                        .main_thread
                        .checked_add(counts.exception_handler)
                        .and_then(|after| after.checked_sub(before))
                    {
                        Some(0) => None,
                        Some(1) => {
                            if counts.last_instruction.is_none() {
                                issue = Some(
                                    "actual opcode counter advanced without its opcode receipt",
                                );
                            }
                            counts.last_instruction
                        }
                        _ => {
                            issue = Some(
                                "actual opcode counter is non-monotonic or spans multiple opcodes",
                            );
                            None
                        }
                    }
                }
                _ => {
                    issue = Some("actual opcode counter was unavailable for this observed step");
                    None
                }
            }
        };
        if let Some(issue) = issue {
            self.0.borrow_mut().evidence.issues.push(issue.into());
        }
        self.finish_cpu_checkpoint(
            CpuCheckpoint::capture_parts(cpu, bus, frame),
            instruction.map(ExecutedOpcode::from),
        );
    }

    pub(crate) fn observe_stream(
        &self,
        access: HybridAccess,
        boundaries: &[HybridExceptionBoundary],
        emulator_checkpoint: CpuCheckpoint,
    ) {
        let mut state = self.0.borrow_mut();
        if let HybridAccess::ExceptionBoundary { index } = access {
            let Some(payload) = boundaries.get(index as usize) else {
                state
                    .evidence
                    .issues
                    .push("indexed actual boundary has no payload".into());
                return;
            };
            Self::boundary(&mut state, payload, emulator_checkpoint);
            return;
        }
        let (step, before) = state.step.as_ref().map_or((None, None), |step| {
            (Some(step.index), Some(AccessPoint::from(&step.before)))
        });
        let actor = if step.is_some() {
            AccessActor::M68k
        } else {
            AccessActor::Unattributed
        };
        Self::access(
            &mut state.evidence,
            access,
            actor,
            step,
            before,
            AccessPoint::from(&emulator_checkpoint),
            false,
        );
    }

    pub(crate) fn observe_native(
        &self,
        access: HybridAccess,
        checkpoint: AccessPoint,
        exact_operand: bool,
    ) {
        let mut state = self.0.borrow_mut();
        if matches!(access, HybridAccess::ExceptionBoundary { .. }) {
            state
                .evidence
                .issues
                .push("native effect cannot supply an architectural boundary marker".into());
            return;
        }
        Self::access(
            &mut state.evidence,
            access,
            AccessActor::Native,
            None,
            None,
            checkpoint,
            exact_operand,
        );
    }

    pub(crate) fn mark_last_native_exact_read(&self, address: u32, word: bool, value: u16) {
        let mut state = self.0.borrow_mut();
        match state.evidence.observations.last_mut() {
            Some(Observation::Access(access))
                if matches!(access.actor, AccessActor::Native)
                    && !access.writing
                    && access.address & 0xffffff == address & 0xffffff
                    && access.word == word
                    && access.value == value =>
            {
                access.exact_operand = true;
            }
            _ => state
                .evidence
                .issues
                .push("exact native read annotation has no matching last actual access".into()),
        }
    }

    pub(crate) fn entry_frame(&self) -> u64 {
        self.0.borrow().evidence.entry.frame
    }

    pub(crate) fn observe_cpu_batch_parts(&self, cpu: &M68000, bus: &GenesisBus, frame: u64) {
        let checkpoint = CpuCheckpoint::capture_parts(cpu, bus, frame);
        let boundaries = bus
            .hybrid_exception_boundaries
            .as_deref()
            .unwrap_or_default();
        if let Some(accesses) = &bus.hybrid_cpu_accesses {
            for &access in accesses {
                self.observe_stream(access, boundaries, checkpoint);
            }
        }
        self.finish_access_batch_parts(cpu, bus, frame);
    }

    pub(crate) fn observe_native_batch_parts(&self, cpu: &M68000, bus: &GenesisBus, frame: u64) {
        let checkpoint = AccessPoint::capture_parts(cpu, bus, frame);
        let boundaries = bus
            .hybrid_exception_boundaries
            .as_deref()
            .unwrap_or_default();
        if let Some(accesses) = &bus.hybrid_cpu_accesses {
            for &access in accesses {
                if matches!(access, HybridAccess::ExceptionBoundary { .. }) {
                    // Preserve an unexpected real marker but never invent its
                    // missing before-step state or pretend it came from native Rust.
                    self.observe_stream(
                        access,
                        boundaries,
                        CpuCheckpoint::capture_parts(cpu, bus, frame),
                    );
                } else {
                    self.observe_native(access, checkpoint, false);
                }
            }
        }
        self.finish_access_batch_parts(cpu, bus, frame);
    }

    fn access(
        evidence: &mut CheckCaptureEvidence,
        access: HybridAccess,
        cpu_actor: AccessActor,
        step: Option<u64>,
        before: Option<AccessPoint>,
        checkpoint: AccessPoint,
        exact_operand: bool,
    ) {
        let (actor, address, word, value, writing, purpose) = match access {
            HybridAccess::Read {
                address,
                word,
                value,
                purpose,
            } => (cpu_actor, address, word, value, false, Some(purpose.into())),
            HybridAccess::Write {
                address,
                word,
                value,
            } => (cpu_actor, address, word, value, true, None),
            HybridAccess::Z80Write {
                address,
                word,
                value,
            } => (AccessActor::Z80, address, word, value, true, None),
            HybridAccess::HostMutation {
                address,
                word,
                value,
            } => (
                AccessActor::ExternalMutation,
                address,
                word,
                value,
                true,
                None,
            ),
            HybridAccess::DmaRead { address, value } => {
                (AccessActor::VdpDma, address, true, value, false, None)
            }
            HybridAccess::ExceptionBoundary { .. } => unreachable!("resolved separately"),
        };
        evidence
            .observations
            .push(Observation::Access(ObservedAccess {
                step,
                precise_access_master_clock: matches!(actor, AccessActor::Native)
                    .then_some(checkpoint.master_clock),
                step_begin_master_clock: before.map(|point| point.master_clock),
                observation_master_clock: checkpoint.master_clock,
                frame: checkpoint.frame,
                pc: Some(before.map_or(checkpoint.pc, |point| point.pc)),
                exception_depth: Some(
                    before.map_or(checkpoint.exception_depth, |point| point.exception_depth),
                ),
                actor,
                address,
                word,
                value,
                writing,
                purpose,
                exact_operand,
            }));
    }

    fn boundary(state: &mut CaptureState, payload: &HybridExceptionBoundary, after: CpuCheckpoint) {
        if let Some(step) = &mut state.step {
            step.boundaries += 1;
        }
        if !state.step.as_ref().is_some_and(|step| step.after.is_some()) {
            state
                .evidence
                .issues
                .push("architectural marker lacks a completed observed CPU step".into());
        }
        let (step_index, before) = state
            .step
            .as_ref()
            .map_or((None, None), |step| (Some(step.index), Some(step.before)));
        let event_index = state.evidence.observations.len();
        let (vector, handler, depth, returned, exception_index) = match payload.boundary {
            ExceptionBoundary::Entered {
                vector,
                handler,
                depth,
            } => {
                let index = state.evidence.exceptions.len();
                let before_stack = state
                    .step
                    .as_mut()
                    .and_then(|step| step.before_stack.take());
                let mut issues = Vec::new();
                if before.is_none() {
                    issues.push("actual entry lacks a pre-step CPU checkpoint".into());
                }
                if before_stack.is_none() {
                    issues
                        .push("actual entry lacks a fresh actual pre-entry stack snapshot".into());
                }
                let parent = state.active.last().map(|active| active.index);
                let expected_depth = state
                    .active
                    .last()
                    .map_or(state.evidence.entry.exception_depth + 1, |active| {
                        state.evidence.exceptions[active.index].depth + 1
                    });
                if depth != expected_depth {
                    issues.push(
                        "actual exception nesting is not contiguous with this capture".into(),
                    );
                }
                state.evidence.exceptions.push(ExceptionCapture {
                    vector,
                    handler,
                    depth,
                    parent,
                    entry_master_clock: payload.master_clock,
                    interrupted: before,
                    entered: after,
                    returned: None,
                    return_master_clock: None,
                    first_event: event_index,
                    last_event: None,
                    minimum_observed_ssp: after.words[16],
                    before_frame: None,
                    after_frame: None,
                    counterpart_final_frame: None,
                    issues,
                });
                state.active.push(ActiveException {
                    index,
                    before_stack,
                });
                (Some(vector), Some(handler), depth, false, Some(index))
            }
            ExceptionBoundary::Returned { depth } => {
                let owner = if state.active.last().is_some_and(|active| {
                    state.evidence.exceptions[active.index].depth.checked_sub(1) == Some(depth)
                }) {
                    state.active.pop()
                } else {
                    None
                };
                let index = owner.as_ref().map(|owner| owner.index);
                if let Some(owner) = owner {
                    let record = &mut state.evidence.exceptions[owner.index];
                    record.returned = Some(after);
                    record.return_master_clock = Some(payload.master_clock);
                    record.last_event = Some(event_index);
                    state.returned.push(owner);
                } else {
                    state
                        .evidence
                        .issues
                        .push("actual RTE has no captured owned entry (possibly inherited)".into());
                }
                (None, None, depth, true, index)
            }
        };
        state
            .evidence
            .observations
            .push(Observation::Boundary(Box::new(ObservedBoundary {
                step: step_index,
                master_clock: payload.master_clock,
                vector,
                handler,
                depth,
                returned,
                exception_index,
                before,
                after,
            })));
    }

    pub(crate) fn finish_access_batch(&self, emulator: &GenesisEmulator, frame: u64) {
        self.finish_access_batch_parts(emulator.hybrid_cpu(), emulator.hybrid_bus(), frame);
    }

    pub(crate) fn finish_access_batch_parts(&self, cpu: &M68000, bus: &GenesisBus, frame: u64) {
        let after = CpuCheckpoint::capture_parts(cpu, bus, frame);
        self.finish_access_checkpoint(after, bus);
    }

    pub(crate) fn finish_access_checkpoint(&self, after: CpuCheckpoint, bus: &GenesisBus) {
        let mut state = self.0.borrow_mut();
        if state.step.as_ref().is_some_and(|step| {
            step.after
                .is_some_and(|after| after.exception_depth != step.before.exception_depth)
                && step.boundaries == 0
        }) {
            state.evidence.issues.push(
                "actual exception depth changed without a retained architectural marker".into(),
            );
        }
        if state.step.as_ref().is_some_and(|step| step.boundaries > 1) {
            state
                .evidence
                .issues
                .push("multiple architectural markers share one observed CPU step".into());
        }
        let inside_exception = !state.active.is_empty() || !state.returned.is_empty();
        if let Some(step) = state.step.as_ref()
            && let (Some(instruction), Some(completed)) = (step.instruction, step.after)
        {
            let movem =
                instruction.opcode & 0xfb80 == 0x4880 && instruction.opcode & 0x0038 >= 0x0010;
            let stack_relevant = inside_exception
                || step.before.words[16] != completed.words[16]
                || step.before.words[15] != completed.words[15]
                || movem
                || instruction.opcode == 0x4e73;
            if stack_relevant {
                let observation = InstructionObservation {
                    step: step.index,
                    instruction,
                    before: step.before,
                    after: completed,
                };
                state
                    .evidence
                    .observations
                    .push(Observation::Instruction(Box::new(observation)));
            }
        }
        if let Some(active) = state.active.last() {
            let index = active.index;
            let record = &mut state.evidence.exceptions[index];
            if after.exception_depth == record.depth {
                record.minimum_observed_ssp = record.minimum_observed_ssp.min(after.words[16]);
            }
        }
        while let Some(owner) = state.returned.pop() {
            Self::retain_frame_images(&mut state, owner, after, bus);
        }
        if let Some(snapshot) = state
            .step
            .as_mut()
            .and_then(|step| step.before_stack.take())
        {
            state.spare_stacks.push(snapshot.bytes);
        }
        state.step = None;
    }

    fn retain_frame_images(
        state: &mut CaptureState,
        mut owner: ActiveException,
        after: CpuCheckpoint,
        bus: &GenesisBus,
    ) {
        let record = &mut state.evidence.exceptions[owner.index];
        let raw_start = record.minimum_observed_ssp;
        let raw_end = record.interrupted.map(|point| point.words[16]);
        if let Some(raw_end) = raw_end
            && raw_start & 0xffffff >= 0xe00000
            && raw_end & 0xffffff >= 0xe00000
        {
            let start = canonical_ram_address(raw_start);
            let end = canonical_ram_address(raw_end);
            if start >= catalogue::STACK_BOTTOM && end <= catalogue::STACK_TOP && start < end {
                record.before_frame = owner
                    .before_stack
                    .as_ref()
                    .and_then(|snapshot| snapshot.slice(start, end));
                let raw = bus.memory.hybrid_raw_main_ram();
                let bytes = (start..end)
                    .map(|address| physical_ram_byte(raw, address))
                    .collect();
                record.after_frame = Some(MemorySnapshot {
                    start,
                    bytes,
                    checkpoint: after,
                });
            } else {
                record.issues.push(
                    "observed SSP footprint leaves the known stack or wraps physical RAM".into(),
                );
            }
        } else {
            record
                .issues
                .push("observed SSP is not a known physical work-RAM stack".into());
        }
        if let Some(snapshot) = owner.before_stack.take() {
            state.spare_stacks.push(snapshot.bytes);
        }
    }

    pub(crate) fn mark_native_journal_scope(&self) {
        self.0
            .borrow_mut()
            .evidence
            .native_journal_scope_established = true;
    }

    pub(crate) fn mark_main_end(&self, emulator: &GenesisEmulator, frame: u64) {
        self.mark_main_checkpoint(
            CpuCheckpoint::capture(emulator, frame),
            emulator.hybrid_bus(),
        );
    }

    pub(crate) fn mark_main_checkpoint(&self, checkpoint: CpuCheckpoint, bus: &GenesisBus) {
        let mut state = self.0.borrow_mut();
        if state.evidence.main_end_event.is_some() {
            state
                .evidence
                .issues
                .push("main comparison boundary was sealed more than once".into());
            return;
        }
        state.evidence.exit = Some(checkpoint);
        state.evidence.main_end_event = Some(state.evidence.observations.len());
        state.evidence.main_journal_lengths = Some(JournalLengths::capture(bus));
    }

    pub(crate) fn finish(
        self,
        emulator: &GenesisEmulator,
        frame: u64,
        failure: Option<String>,
    ) -> CheckCaptureEvidence {
        self.finish_checkpoint(
            CpuCheckpoint::capture(emulator, frame),
            emulator.hybrid_bus(),
            failure,
        )
    }

    pub(crate) fn finish_checkpoint(
        mut self,
        checkpoint: CpuCheckpoint,
        bus: &GenesisBus,
        failure: Option<String>,
    ) -> CheckCaptureEvidence {
        self.finish_access_checkpoint(checkpoint, bus);
        let state = self.0.get_mut();
        if !state.active.is_empty() {
            state
                .evidence
                .issues
                .push("execution ended with actual exception frames still open".into());
        }
        while let Some(owner) = state.active.pop() {
            state.evidence.exceptions[owner.index]
                .issues
                .push("actual frame remains open at execution end".into());
            Self::retain_frame_images(state, owner, checkpoint, bus);
        }
        if state.evidence.exit.is_none() {
            state.evidence.exit = Some(checkpoint);
        }
        state.evidence.final_observer_checkpoint = Some(checkpoint);
        state.evidence.bus_journals = BusJournals::capture(bus);
        state.evidence.execution_complete = failure.is_none();
        state.evidence.execution_error = failure;
        self.0.into_inner().evidence
    }
}

impl crate::Host {
    /// Explicit private diagnostics only. This records neither a new comparison
    /// contract nor a hook registration, and cannot enable a production body.
    pub fn set_check_capture(&mut self, enabled: bool) -> crate::Result<()> {
        if self.mode != crate::Mode::Check {
            return Err(crate::HostError::Metadata(
                "actual check capture is available only in check mode".into(),
            ));
        }
        self.checks.actual_capture_enabled = enabled;
        Ok(())
    }

    /// Latest opted-in evidence. Disabling recording does not relabel it as
    /// evidence for later checks, and archived root failures retain their own data.
    pub fn actual_check_capture(&self) -> Option<&CheckCaptureDiagnostic> {
        self.checks.actual_capture.as_deref().or_else(|| {
            self.checks
                .failures
                .iter()
                .rev()
                .find_map(|failure| failure.actual_capture.as_deref())
        })
    }

    pub(crate) fn record_actual_check_capture(
        &mut self,
        evidence: Box<CheckCaptureEvidence>,
        baseline: bool,
    ) {
        let capture = self.checks.actual_capture.get_or_insert_with(|| {
            Box::new(CheckCaptureDiagnostic::new(&evidence.binding.function))
        });
        capture.record(evidence, baseline);
    }
}

#[cfg(test)]
mod tests;
