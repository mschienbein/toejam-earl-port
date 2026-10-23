//! Instruction-boundary coordination for Rust-owned, reference-phased effects.
//!
//! Core exposes one-use effect tokens and never executes a game effect. This
//! layer supplies reviewed control-flow admission, finite diagnostic resources,
//! device advance/commit and interrupt suspension/resumption. The host adapter
//! may explicitly use original ISRs as an oracle; native startup must supply its
//! real scheduled ISR closure instead, never an original-code fallback.
use crate::{CpuState, DeviceStep, Devices};
use std::collections::{BTreeMap, BTreeSet};
mod native_interrupt;
mod native_program;
mod native_source;
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
    /// Dynamic RTS destination from the actual source-visible entry stack.
    pub return_pc: u32,
    /// Operational resource ceiling, not a semantic timeout or termination proof.
    pub maximum_steps: u64,
    /// Reviewed native IRQ/callee closure requirements, checked at startup.
    pub required_interrupts: &'static [u8],
}

pub struct TimingProgram {
    reviewed: ReviewedRegion,
    instructions: BTreeMap<u32, TimingInstruction>,
    edges: BTreeSet<(u32, u32)>,
    divider: u32,
}
impl TimingProgram {
    pub fn compile(
        rom: &[u8],
        reviewed: ReviewedRegion,
        divider: u32,
    ) -> Result<Self, TimingError> {
        if reviewed.name.is_empty()
            || reviewed.source.is_empty()
            || reviewed.maximum_steps == 0
            || reviewed.entry & 1 != 0
            || reviewed.return_pc & 1 != 0
        {
            return Err(TimingError::WrongEntry);
        }
        let instructions = compile_schedules(
            rom,
            &reviewed.instruction_pcs,
            &[reviewed.return_pc],
            divider,
        )?;
        if !instructions.contains_key(&reviewed.entry) {
            return Err(TimingError::WrongEntry);
        }
        let edges: BTreeSet<_> = reviewed.edges.iter().map(|e| (e.from, e.to)).collect();
        if edges.len() != reviewed.edges.len()
            || edges.iter().any(|&(from, to)| {
                !instructions.contains_key(&from)
                    || !(instructions.contains_key(&to) || to == reviewed.return_pc)
            })
        {
            return Err(TimingError::InvalidIr("uncovered reviewed edge"));
        }
        Ok(Self {
            reviewed,
            instructions,
            edges,
            divider,
        })
    }
    pub fn reviewed(&self) -> &ReviewedRegion {
        &self.reviewed
    }
}

fn compile_schedules(
    rom: &[u8],
    pcs: &[u32],
    extra_prefetch: &[u32],
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
    let mut decoded = Vec::with_capacity(pcs.len());
    for &pc in pcs {
        let phase = tje_engine_core::timing::cycles_if_move_btst_cmp(read_word(pc)?)
            .map_or(8, |cycles| cycles - 4);
        phase
            .checked_mul(divider)
            .ok_or(TimingError::InvalidIr("read phase overflows master clocks"))?;
        let ir = compile_instruction_with_divider(rom, pc, divider)
            .map_err(|_| TimingError::InvalidIr("reviewed instruction no longer decodes in ROM"))?;
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
    for &pc in extra_prefetch {
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
}
#[derive(Debug)]
pub struct InterruptToken {
    owner: ScratchIdentity,
    serial: u64,
    pipeline: PipelineState,
    ssp: u32,
    usp: u32,
    action: BoundaryAction,
    start_steps: u64,
    start_clocks: u64,
    start_original: u64,
    start_scheduled: u64,
    sr: u32,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RegionUsage {
    pub instructions: u64,
    pub scheduler_steps: u64,
    pub master_clocks: u64,
    pub frames: u64,
    pub original_isr_instructions: u64,
    pub scheduled_isr_instructions: u64,
}

// Native ISR service is implemented by the real exception/clock controller in
// native_interrupt, not a provider-supplied completion flag.

pub struct TimingRegion<'a> {
    program: &'a TimingProgram,
    scratch: TimingScratch,
    next: PipelineState,
    current: Option<(u32, CursorIdentity)>,
    suspended: bool,
    serial: u64,
    owner: ScratchIdentity,
    usage: RegionUsage,
    ownership: IsrOwnership,
    native_isr: Option<&'a mut dyn ScheduledIsrClosure>,
    native_controller: Option<native_interrupt::ControllerState>,
    native_program: Option<std::sync::Arc<NativeIsrProgram>>,
    native_frames: Vec<native_interrupt::Frame>,
}
impl<'a> TimingRegion<'a> {
    pub fn begin_oracle(program: &'a TimingProgram, entry: &CpuState) -> Result<Self, TimingError> {
        Self::begin(program, entry, IsrOwnership::OriginalOracle, None, None)
    }
    pub fn begin_native(
        program: &'a TimingProgram,
        entry: &CpuState,
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
        Self::begin(
            program,
            entry,
            IsrOwnership::ScheduledRust,
            native_isr,
            Some(native_program),
        )
    }
    fn begin(
        program: &'a TimingProgram,
        entry: &CpuState,
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
        let mut scratch = TimingScratch::default();
        scratch.reserve_for(program.instructions.values());
        if let Some(native_program) = &native_program {
            scratch.reserve_for(native_program.instructions.values());
        }
        let owner = scratch.identity()?;
        Ok(Self {
            program,
            scratch,
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
            native_frames: Vec::with_capacity(8),
        })
    }
    pub fn usage(&self) -> RegionUsage {
        self.usage
    }
    pub fn is_complete(&self) -> bool {
        self.current.is_none() && !self.suspended && self.next.pc == self.program.reviewed.return_pc
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
        let cursor = TimingCursor::begin(instruction, entry, &mut self.scratch)?;
        self.current = Some((entry.pc, cursor.identity()));
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
            || !self
                .program
                .edges
                .contains(&(timing.pc(), timing.pipeline().pc))
        {
            return Err(TimingError::InvalidIr(
                "unreviewed Rust control-flow edge or foreign grant",
            ));
        }
        self.current = None;
        self.next = timing.pipeline();
        self.usage.instructions += 1;
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
