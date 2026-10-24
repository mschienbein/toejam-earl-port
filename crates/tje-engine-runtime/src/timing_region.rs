//! Instruction-boundary coordination for Rust-owned, reference-phased effects.
//!
//! Core exposes one-use effect tokens and never executes a game effect. This
//! layer supplies reviewed control-flow admission, finite diagnostic resources,
//! device advance/commit and interrupt suspension/resumption. The host adapter
//! may explicitly use original ISRs as an oracle; native startup must supply its
//! real scheduled ISR closure instead, never an original-code fallback.
use crate::{CpuState, DeviceStep, Devices};
use std::collections::{BTreeMap, BTreeSet};
mod calls;
mod native_interrupt;
mod native_program;
mod native_source;
#[cfg(feature = "devices")]
pub(crate) use calls::CompiledCallStart;
pub use calls::{CallToken, CalleeKind, ParkedRegion, RegionCall, RegionWork};
pub use native_interrupt::{IsrAction, NativeInterruptStep, RteReceipt, ScheduledIsrClosure};
pub use native_program::{InterruptEntry, NativeIsrContract, NativeIsrProgram};
pub use native_source::{IsrEffects, IsrSourceContext};
use tje_engine_core::{
    compile_instruction_with_divider,
    timing_region::{
        CursorIdentity, InstructionTiming, PipelineState, ScratchIdentity, TimingCursor,
        TimingError, TimingInstruction, TimingScratch,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionEdge {
    pub from: u32,
    pub to: u32,
}
#[derive(Debug, Clone)]
pub struct ReviewedRegion {
    pub name: &'static str,
    pub source: &'static str,
    pub entry: u32,
    pub instruction_pcs: Vec<u32>,
    pub edges: Vec<RegionEdge>,
    pub calls: Vec<RegionCall>,
    /// Some fixes a low-level region boundary; None is a source-owned RTS exit
    /// whose actual target is supplied by the prepared invocation, not compiled.
    pub return_pc: Option<u32>,
    /// Operational resource ceiling, not a semantic timeout or termination proof.
    pub maximum_steps: u64,
    /// Reviewed native IRQ/callee closure requirements, checked at startup.
    pub required_interrupts: &'static [u8],
}

/// Invocation-only immutable-ROM permission. None carries no ahead-of-time
/// opcode value: the real bus prefetch supplies it once at the original phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnTarget {
    pub pc: u32,
    pub prefetch: Option<u16>,
}
impl ReturnTarget {
    pub fn from_rom(rom: &[u8], pc: u32) -> Result<Self, TimingError> {
        if pc & 1 != 0 {
            return Err(TimingError::WrongEntry);
        }
        let address = (pc & 0xffffff) as usize;
        let word = rom.get(address..address + 2).ok_or(TimingError::InvalidIr(
            "return target is outside the immutable-ROM profile",
        ))?;
        Ok(Self {
            pc,
            prefetch: Some(u16::from_be_bytes([word[0], word[1]])),
        })
    }
    pub fn in_rom(rom_length: usize, pc: u32) -> Result<Self, TimingError> {
        let address = (pc & 0xffffff) as usize;
        if pc & 1 != 0 || address.checked_add(2).is_none_or(|end| end > rom_length) {
            return Err(TimingError::InvalidIr(
                "return target is outside the immutable-ROM profile",
            ));
        }
        Ok(Self { pc, prefetch: None })
    }
}

pub struct TimingProgram {
    reviewed: ReviewedRegion,
    instructions: BTreeMap<u32, TimingInstruction>,
    edges: BTreeSet<(u32, u32)>,
    calls: BTreeMap<(u32, u32), u32>,
    divider: u32,
    id: ScratchIdentity,
    fixed_target: Option<ReturnTarget>,
}
impl TimingProgram {
    pub fn compile(
        rom: &[u8],
        reviewed: ReviewedRegion,
        divider: u32,
    ) -> Result<Self, TimingError> {
        let instructions = compile_schedules(
            rom,
            &reviewed.instruction_pcs,
            reviewed
                .calls
                .iter()
                .map(|call| call.target)
                .chain(reviewed.return_pc),
            divider,
        )?;
        let fixed_target = reviewed
            .return_pc
            .map(|pc| ReturnTarget::from_rom(rom, pc))
            .transpose()?;
        Self::from_schedules(reviewed, instructions, divider, fixed_target)
    }
    pub(crate) fn from_schedules(
        reviewed: ReviewedRegion,
        instructions: BTreeMap<u32, TimingInstruction>,
        divider: u32,
        fixed_target: Option<ReturnTarget>,
    ) -> Result<Self, TimingError> {
        if reviewed.name.is_empty()
            || reviewed.source.is_empty()
            || reviewed.maximum_steps == 0
            || reviewed.entry & 1 != 0
            || reviewed.return_pc.is_some_and(|pc| pc & 1 != 0)
            || reviewed.return_pc != fixed_target.map(|target| target.pc)
        {
            return Err(TimingError::WrongEntry);
        }
        if !instructions.contains_key(&reviewed.entry) {
            return Err(TimingError::WrongEntry);
        }
        let edges: BTreeSet<_> = reviewed.edges.iter().map(|e| (e.from, e.to)).collect();
        if edges.len() != reviewed.edges.len()
            || edges.iter().any(|&(from, to)| {
                !instructions.contains_key(&from)
                    || !(instructions.contains_key(&to) || Some(to) == reviewed.return_pc)
            })
        {
            return Err(TimingError::InvalidIr("uncovered reviewed edge"));
        }
        let mut calls = BTreeMap::new();
        for call in &reviewed.calls {
            let instruction = instructions
                .get(&call.from)
                .ok_or(TimingError::WrongEntry)?;
            if !calls::is_call(instruction.opcode())
                || call.target & 1 != 0
                || call.resume != instruction.next_pc()
                || !(instructions.contains_key(&call.resume)
                    || Some(call.resume) == reviewed.return_pc)
                || edges.iter().any(|&(from, _)| from == call.from)
                || calls
                    .insert((call.from, call.target), call.resume)
                    .is_some()
            {
                return Err(TimingError::InvalidIr("invalid or duplicate scoped call"));
            }
        }
        if instructions.iter().any(|(&pc, instruction)| {
            calls::is_call(instruction.opcode()) && !calls.keys().any(|&(from, _)| from == pc)
        }) {
            return Err(TimingError::InvalidIr(
                "call instruction has no scoped target",
            ));
        }
        Ok(Self {
            reviewed,
            instructions,
            edges,
            calls,
            divider,
            id: TimingScratch::default().identity()?,
            fixed_target,
        })
    }
    pub fn reviewed(&self) -> &ReviewedRegion {
        &self.reviewed
    }
    pub fn fixed_target(&self) -> Option<ReturnTarget> {
        self.fixed_target
    }
    pub(crate) fn permits_edge(&self, from: u32, to: u32) -> bool {
        self.edges.contains(&(from, to))
    }
}

/// Caller-owned prepared storage. Preparation occurs at startup, never on the
/// clocked source path. A missing/mismatched preparation fails, not reallocates.
pub struct RegionStorage {
    program: ScratchIdentity,
    scratch: TimingScratch,
    native_frames: Vec<native_interrupt::Frame>,
    native_program: Option<std::sync::Arc<NativeIsrProgram>>,
    lease: Option<CursorIdentity>,
}
impl RegionStorage {
    pub fn new(program: &TimingProgram) -> Self {
        let mut scratch = TimingScratch::default();
        scratch.reserve_for(program.instructions.values());
        Self {
            program: program.id,
            scratch,
            native_frames: Vec::new(),
            native_program: None,
            lease: None,
        }
    }
    pub(crate) fn available_for(&self, program: &TimingProgram) -> bool {
        self.program == program.id && self.lease.is_none()
    }
    pub fn prepare_native(&mut self, program: &std::sync::Arc<NativeIsrProgram>) {
        self.scratch.reserve_for(program.instructions.values());
        self.native_frames.reserve(8);
        self.native_program = Some(std::sync::Arc::clone(program));
    }
}

fn compile_schedules(
    rom: &[u8],
    pcs: &[u32],
    extra_prefetch: impl IntoIterator<Item = u32>,
    divider: u32,
) -> Result<BTreeMap<u32, TimingInstruction>, TimingError> {
    compile_schedule_ir(
        rom,
        pcs.iter().map(|&pc| {
            compile_instruction_with_divider(rom, pc, divider).map_err(|_| {
                TimingError::InvalidIr("reviewed instruction no longer decodes in ROM")
            })
        }),
        extra_prefetch,
        divider,
    )
}

pub(crate) fn compile_schedule_ir(
    rom: &[u8],
    input: impl IntoIterator<Item = Result<tje_engine_core::ir::InstructionIr, TimingError>>,
    extra_prefetch: impl IntoIterator<Item = u32>,
    divider: u32,
) -> Result<BTreeMap<u32, TimingInstruction>, TimingError> {
    if divider == 0 {
        return Err(TimingError::WrongEntry);
    }
    let read_word = |pc: u32| -> Result<u16, TimingError> {
        let bytes = rom
            .get(pc as usize..pc as usize + 2)
            .ok_or(TimingError::InvalidIr("prefetch leaves immutable ROM"))?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    };
    let mut words = BTreeMap::new();
    let mut occupied = BTreeMap::new();
    let input = input.into_iter();
    let mut decoded = Vec::with_capacity(input.size_hint().0);
    for ir in input {
        let ir = ir?;
        let pc = ir.pc;
        let phase = tje_engine_core::timing::cycles_if_move_btst_cmp(read_word(pc)?)
            .map_or(8, |cycles| cycles - 4);
        phase
            .checked_mul(divider)
            .ok_or(TimingError::InvalidIr("read phase overflows master clocks"))?;
        for address in (pc..ir.next_pc).step_by(2) {
            if occupied.insert(address, pc).is_some() {
                return Err(TimingError::InvalidIr(
                    "overlapping or duplicate source instruction",
                ));
            }
        }
        for address in (pc..=ir.next_pc).step_by(2) {
            words.insert(address, read_word(address)?);
        }
        decoded.push(ir);
    }
    for pc in extra_prefetch {
        words.insert(pc, read_word(pc)?);
    }
    let words = std::sync::Arc::new(words);
    let mut instructions = BTreeMap::new();
    for ir in &decoded {
        let mut timing = TimingInstruction::compile(ir)?;
        timing.bind_prefetch_words(std::sync::Arc::clone(&words))?;
        instructions.insert(ir.pc, timing);
    }
    Ok(instructions)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryAction {
    Ready,
    Wait,
    Reset,
    Halted,
    Stopped,
    Faulted,
    InterruptAdmission { level: u8 },
    InterruptAcknowledge { level: u8 },
}
#[derive(Debug, Clone, Copy)]
pub struct BoundarySignals {
    pub wait: bool,
    pub reset: bool,
    pub halted: bool,
    pub interrupt_level: u8,
}

/// Same priority/order as the shipped runtime, without performing any CPU or
/// game effect. The platform must service the returned event before admission.
pub fn classify_boundary(cpu: &CpuState, signals: BoundarySignals) -> BoundaryAction {
    if signals.wait {
        BoundaryAction::Wait
    } else if signals.reset {
        BoundaryAction::Reset
    } else if signals.halted || cpu.frozen != 0 {
        BoundaryAction::Halted
    } else if cpu.address_error != 0 {
        BoundaryAction::Faulted
    } else if cpu.pending_interrupt != 0 {
        BoundaryAction::InterruptAcknowledge {
            level: cpu.pending_interrupt as u8,
        }
    } else if signals.interrupt_level & 7 > ((cpu.sr >> 8) & 7) as u8 {
        BoundaryAction::InterruptAdmission {
            level: signals.interrupt_level & 7,
        }
    } else if cpu.stopped != 0 {
        BoundaryAction::Stopped
    } else {
        BoundaryAction::Ready
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsrOwnership {
    OriginalOracle,
    ScheduledRust,
    /// Generated native continuations, not readable ISR-body coverage.
    CompiledEngine,
}
#[derive(Debug)]
pub struct InterruptToken {
    owner: CursorIdentity,
    serial: u64,
    pipeline: PipelineState,
    ssp: u32,
    usp: u32,
    action: BoundaryAction,
    start_steps: u64,
    start_clocks: u64,
    start_original: u64,
    start_scheduled: u64,
    start_compiled: u64,
    sr: u32,
}
#[cfg(feature = "devices")]
impl InterruptToken {
    pub(crate) fn identity(&self) -> (CursorIdentity, u64) {
        (self.owner, self.serial)
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RegionUsage {
    pub instructions: u64,
    pub scheduler_steps: u64,
    pub master_clocks: u64,
    pub frames: u64,
    pub original_isr_instructions: u64,
    pub scheduled_isr_instructions: u64,
    pub compiled_isr_instructions: u64,
    pub nested_source_instructions: u64,
    pub original_callee_instructions: u64,
    pub compiled_original_callee_instructions: u64,
    pub call_profile: Option<&'static str>,
    pub timed_callee_calls: u64,
    pub original_substitution_calls: u64,
    pub compiled_original_callee_calls: u64,
    pub original_substitution_label: Option<&'static str>,
    pub compiled_original_callee_label: Option<&'static str>,
    pub separate_callee_proof_obligations: u64,
}

// Native ISR service is implemented by the real exception/clock controller in
// native_interrupt, not a provider-supplied completion flag.

pub struct TimingRegion<'a> {
    program: &'a TimingProgram,
    scratch: &'a mut TimingScratch,
    next: PipelineState,
    current: Option<(u32, CursorIdentity)>,
    suspended: bool,
    serial: u64,
    owner: CursorIdentity,
    usage: RegionUsage,
    ownership: IsrOwnership,
    native_isr: Option<&'a mut dyn ScheduledIsrClosure>,
    native_controller: Option<native_interrupt::ControllerState>,
    native_program: Option<std::sync::Arc<NativeIsrProgram>>,
    native_frames: &'a mut Vec<native_interrupt::Frame>,
    target: ReturnTarget,
    entry_sp: u32,
    returned: bool,
    last_opcode: Option<u16>,
    call_entry: Option<calls::CallEntry>,
    call_pending: Option<calls::PendingCall>,
    lease: &'a mut Option<CursorIdentity>,
    parked: bool,
}
impl<'a> TimingRegion<'a> {
    pub fn begin_oracle(
        program: &'a TimingProgram,
        entry: &CpuState,
        target: ReturnTarget,
        storage: &'a mut RegionStorage,
    ) -> Result<Self, TimingError> {
        Self::begin(
            program,
            entry,
            target,
            storage,
            IsrOwnership::OriginalOracle,
            None,
            None,
        )
    }
    #[cfg(feature = "devices")]
    pub(crate) fn begin_compiled(
        program: &'a TimingProgram,
        entry: &CpuState,
        target: ReturnTarget,
        storage: &'a mut RegionStorage,
    ) -> Result<Self, TimingError> {
        Self::begin(
            program,
            entry,
            target,
            storage,
            IsrOwnership::CompiledEngine,
            None,
            None,
        )
    }
    pub fn begin_native(
        program: &'a TimingProgram,
        entry: &CpuState,
        target: ReturnTarget,
        storage: &'a mut RegionStorage,
        native_isr: Option<&'a mut dyn ScheduledIsrClosure>,
    ) -> Result<Self, TimingError> {
        let native_program = std::sync::Arc::clone(
            native_isr
                .as_ref()
                .ok_or(TimingError::MissingIsrCapability)?
                .timing_program(),
        );
        native_program.validate_requirements(program.reviewed())?;
        if native_program.divider != program.divider {
            return Err(TimingError::WrongEntry);
        }
        if native_program.instructions.keys().any(|&pc| {
            !native_isr
                .as_ref()
                .is_some_and(|provider| provider.supports_pc(pc))
        }) {
            return Err(TimingError::MissingIsrCapability);
        }
        if !storage
            .native_program
            .as_ref()
            .is_some_and(|prepared| std::sync::Arc::ptr_eq(prepared, &native_program))
            || storage.native_frames.capacity() < 8
        {
            return Err(TimingError::MissingIsrCapability);
        }
        Self::begin(
            program,
            entry,
            target,
            storage,
            IsrOwnership::ScheduledRust,
            native_isr,
            Some(native_program),
        )
    }
    fn begin(
        program: &'a TimingProgram,
        entry: &CpuState,
        target: ReturnTarget,
        storage: &'a mut RegionStorage,
        ownership: IsrOwnership,
        native_isr: Option<&'a mut dyn ScheduledIsrClosure>,
        native_program: Option<std::sync::Arc<NativeIsrProgram>>,
    ) -> Result<Self, TimingError> {
        if entry.address_error != 0 {
            return Err(TimingError::PendingFault);
        }
        if entry.pc != program.reviewed.entry || entry.pc & 1 != 0 {
            return Err(TimingError::WrongEntry);
        }
        let instruction = program
            .instructions
            .get(&entry.pc)
            .ok_or(TimingError::WrongEntry)?;
        if entry.prefetch != u32::from(instruction.opcode()) {
            return Err(TimingError::WrongEntry);
        }
        if storage.program != program.id
            || storage.lease.is_some()
            || target.pc & 1 != 0
            || program.fixed_target.is_some_and(|fixed| fixed != target)
        {
            return Err(TimingError::WrongEntry);
        }
        let owner = storage.scratch.claim_identity()?;
        storage.lease = Some(owner);
        Ok(Self {
            program,
            scratch: &mut storage.scratch,
            next: PipelineState {
                pc: entry.pc,
                prefetch: entry.prefetch,
                last_muldiv: entry.last_muldiv,
            },
            current: None,
            suspended: false,
            serial: 0,
            owner,
            usage: RegionUsage::default(),
            ownership,
            native_isr,
            native_controller: None,
            native_program,
            native_frames: &mut storage.native_frames,
            target,
            entry_sp: entry.read_address(7),
            returned: false,
            last_opcode: None,
            call_entry: None,
            call_pending: None,
            lease: &mut storage.lease,
            parked: false,
        })
    }
    pub fn usage(&self) -> RegionUsage {
        self.usage
    }
    pub fn set_call_profile(&mut self, profile: &'static str) -> Result<(), TimingError> {
        if profile.is_empty()
            || self.usage.scheduler_steps != 0
            || self.usage.instructions != 0
            || self.usage.call_profile.is_some()
        {
            return Err(TimingError::WrongToken);
        }
        self.usage.call_profile = Some(profile);
        Ok(())
    }
    pub fn is_complete(&self) -> bool {
        self.current.is_none() && !self.suspended && self.call_pending.is_none() && self.returned
    }
    pub fn identity(&self) -> CursorIdentity {
        self.owner
    }
    pub fn return_target(&self) -> ReturnTarget {
        self.target
    }
    pub(crate) fn entry_stack(&self) -> u32 {
        self.entry_sp
    }
    /// Mark only an actual completed source-program RTS, not an internal RTS.
    pub fn complete_return(&mut self, cpu: &CpuState) -> Result<(), TimingError> {
        if self.program.reviewed.return_pc.is_some()
            || self.current.is_some()
            || self.suspended
            || self.call_pending.is_some()
            || self.last_opcode != Some(0x4e75)
            || cpu.pc != self.target.pc
            || cpu.pc != self.next.pc
            || cpu.prefetch != self.next.prefetch
            || cpu.read_address(7) != self.entry_sp.wrapping_add(4)
        {
            return Err(TimingError::WrongEntry);
        }
        self.returned = true;
        Ok(())
    }
    pub(crate) fn belongs_to(&self, program: &TimingProgram) -> bool {
        std::ptr::eq(self.program, program)
    }
    pub fn check_step_budget(&self) -> Result<(), TimingError> {
        if self.usage.scheduler_steps >= self.program.reviewed.maximum_steps {
            Err(TimingError::Exhausted)
        } else {
            Ok(())
        }
    }
    pub fn cursor<'b>(
        &'b mut self,
        entry: &'b CpuState,
        admission: BoundaryAction,
    ) -> Result<TimingCursor<'b>, TimingError> {
        if self.suspended
            || self.current.is_some()
            || self.call_pending.is_some()
            || admission != BoundaryAction::Ready
            || entry.pc != self.next.pc
            || entry.prefetch != self.next.prefetch
            || self.is_complete()
        {
            return Err(TimingError::WrongEntry);
        }
        if self.usage.scheduler_steps >= self.program.reviewed.maximum_steps {
            return Err(TimingError::Exhausted);
        }
        let instruction = self
            .program
            .instructions
            .get(&entry.pc)
            .ok_or(TimingError::InvalidIr("escaped reviewed region"))?;
        let mut cursor = TimingCursor::begin(instruction, entry, self.scratch)?;
        cursor.bind_return_prefetch(self.target.pc, self.target.prefetch)?;
        self.current = Some((entry.pc, cursor.identity()));
        self.call_entry =
            calls::is_call(instruction.opcode()).then(|| calls::CallEntry::capture(entry));
        Ok(cursor)
    }
    /// Validate and consume the grant before advancing clocks. This does not
    /// publish a game register, stack or memory value; only the real pipeline is
    /// advanced. Rust must have completed its own effects before calling.
    pub fn accept_instruction(
        &mut self,
        timing: InstructionTiming,
    ) -> Result<InstructionTiming, TimingError> {
        if self.suspended
            || self.current != Some((timing.pc(), timing.identity()))
            || self.call_pending.is_some()
            || !(self.program.permits_edge(timing.pc(), timing.pipeline().pc)
                || self
                    .program
                    .calls
                    .contains_key(&(timing.pc(), timing.pipeline().pc))
                || (self.program.reviewed.return_pc.is_none()
                    && timing.opcode() == 0x4e75
                    && timing.pipeline().pc == self.target.pc))
        {
            return Err(TimingError::InvalidIr(
                "unreviewed Rust control-flow edge or foreign grant",
            ));
        }
        self.arm_call(&timing)?;
        self.current = None;
        self.next = timing.pipeline();
        self.last_opcode = Some(timing.opcode());
        if self.program.reviewed.return_pc.is_some() && self.next.pc == self.target.pc {
            self.returned = true;
        }
        self.usage.instructions = self
            .usage
            .instructions
            .checked_add(1)
            .ok_or(TimingError::Exhausted)?;
        Ok(timing)
    }
    pub fn record_original_instruction(&mut self) -> Result<(), TimingError> {
        self.usage.original_isr_instructions = self
            .usage
            .original_isr_instructions
            .checked_add(1)
            .ok_or(TimingError::Exhausted)?;
        Ok(())
    }
    #[cfg(feature = "devices")]
    pub(crate) fn record_compiled_instruction(&mut self) -> Result<(), TimingError> {
        self.usage.compiled_isr_instructions = self
            .usage
            .compiled_isr_instructions
            .checked_add(1)
            .ok_or(TimingError::Exhausted)?;
        Ok(())
    }
    pub fn record_progress(&mut self, step: DeviceStep) -> Result<(), TimingError> {
        self.usage.scheduler_steps = self
            .usage
            .scheduler_steps
            .checked_add(1)
            .ok_or(TimingError::Exhausted)?;
        self.usage.master_clocks = self
            .usage
            .master_clocks
            .checked_add(step.master_clocks)
            .ok_or(TimingError::Exhausted)?;
        self.usage.frames = self
            .usage
            .frames
            .checked_add(u64::from(step.frame_complete))
            .ok_or(TimingError::Exhausted)?;
        if self.usage.scheduler_steps > self.program.reviewed.maximum_steps {
            return Err(TimingError::Exhausted);
        }
        Ok(())
    }
    pub fn suspend_interrupt(
        &mut self,
        cpu: &CpuState,
        action: BoundaryAction,
    ) -> Result<InterruptToken, TimingError> {
        if self.current.is_some()
            || self.suspended
            || !matches!(
                action,
                BoundaryAction::InterruptAdmission { .. }
                    | BoundaryAction::InterruptAcknowledge { .. }
            )
            || cpu.pc != self.next.pc
            || cpu.prefetch != self.next.prefetch
        {
            return Err(TimingError::WrongEntry);
        }
        self.serial = self.serial.checked_add(1).ok_or(TimingError::Exhausted)?;
        self.suspended = true;
        Ok(InterruptToken {
            owner: self.owner,
            serial: self.serial,
            pipeline: self.next,
            ssp: cpu.ssp,
            usp: cpu.usp,
            action,
            start_steps: self.usage.scheduler_steps,
            start_clocks: self.usage.master_clocks,
            start_original: self.usage.original_isr_instructions,
            start_scheduled: self.usage.scheduled_isr_instructions,
            start_compiled: self.usage.compiled_isr_instructions,
            sr: cpu.sr,
        })
    }
    pub fn resume_oracle_interrupt(
        &mut self,
        token: InterruptToken,
        returned: &CpuState,
        instructions: u64,
        rte_observed: bool,
    ) -> Result<(), TimingError> {
        if self.ownership != IsrOwnership::OriginalOracle {
            return Err(TimingError::MissingIsrCapability);
        }
        self.resume_interrupt(token, returned, instructions, rte_observed)
    }
    #[cfg(feature = "devices")]
    pub(crate) fn resume_compiled_interrupt(
        &mut self,
        token: InterruptToken,
        returned: &CpuState,
        receipt: crate::portable_timing::engine::CompiledInterruptReceipt,
    ) -> Result<(), TimingError> {
        if self.ownership != IsrOwnership::CompiledEngine || receipt.identity() != token.identity()
        {
            return Err(TimingError::WrongToken);
        }
        self.resume_interrupt(token, returned, receipt.instructions(), true)
    }
    fn resume_interrupt(
        &mut self,
        token: InterruptToken,
        returned: &CpuState,
        instructions: u64,
        rte_observed: bool,
    ) -> Result<(), TimingError> {
        if !self.suspended
            || token.owner != self.owner
            || token.serial != self.serial
            || !rte_observed
            || returned.pc != token.pipeline.pc
            || returned.prefetch != token.pipeline.prefetch
            || returned.ssp != token.ssp
            || returned.usp != token.usp
            || returned.address_error != 0
            || self.usage.scheduler_steps <= token.start_steps
            || self.usage.master_clocks <= token.start_clocks
            || instructions == 0
            || returned.sr != token.sr
            || returned.pending_interrupt != 0
            || returned.frozen != 0
        {
            return Err(TimingError::WrongEntry);
        }
        let retired = match self.ownership {
            IsrOwnership::OriginalOracle => self
                .usage
                .original_isr_instructions
                .checked_sub(token.start_original),
            IsrOwnership::ScheduledRust => self
                .usage
                .scheduled_isr_instructions
                .checked_sub(token.start_scheduled),
            IsrOwnership::CompiledEngine => self
                .usage
                .compiled_isr_instructions
                .checked_sub(token.start_compiled),
        };
        if retired != Some(instructions) {
            return Err(TimingError::WrongEntry);
        }
        self.next.last_muldiv = returned.last_muldiv;
        self.suspended = false;
        Ok(())
    }
}

/// Complete one instruction on the real CPU-free device layer. Only pipeline
/// bookkeeping is runtime-owned; all game effects were Rust's acknowledged bus
/// operations. Existing device queues retain their exact width/commit ordering.
pub fn advance_and_commit<D: Devices>(
    devices: &mut D,
    cpu: &mut CpuState,
    timing: InstructionTiming,
) -> Result<DeviceStep, TimingError> {
    if cpu.address_error != 0 {
        return Err(TimingError::PendingFault);
    }
    let pipeline = timing.pipeline();
    cpu.pc = pipeline.pc;
    cpu.prefetch = pipeline.prefetch;
    cpu.last_muldiv = pipeline.last_muldiv;
    cpu.address_error = 0;
    Ok(devices.finish_instruction(timing.pc(), timing.cycles()))
}

#[cfg(test)]
mod tests;
