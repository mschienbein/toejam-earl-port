//! Separately owned callee boundaries and allocation-free parked continuations.
//! A portable Call is intent; only an actual completed JSR/BSR can mint a grant.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionCall {
    pub from: u32,
    pub target: u32,
    pub resume: u32,
}

pub(super) fn is_call(opcode: u16) -> bool {
    matches!(
        tje_engine_core::decode::decode(opcode),
        tje_engine_core::decode::Instruction::JumpToSubroutine(_)
            | tje_engine_core::decode::Instruction::BranchToSubroutine(_)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CallEntry {
    stack: u32,
    status: u32,
    other_stack: u32,
}
impl CallEntry {
    pub(super) fn capture(cpu: &CpuState) -> Self {
        Self {
            stack: cpu.read_address(7),
            status: cpu.sr,
            other_stack: if cpu.sr & 0x2000 != 0 {
                cpu.usp
            } else {
                cpu.ssp
            },
        }
    }
    fn matches_call(&self, cpu: &CpuState) -> bool {
        cpu.read_address(7) == self.stack.wrapping_sub(4)
            && cpu.sr == self.status
            && self.other_stack
                == if cpu.sr & 0x2000 != 0 {
                    cpu.usp
                } else {
                    cpu.ssp
                }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingCall {
    edge: RegionCall,
    entry: CallEntry,
    serial: u64,
    start_steps: u64,
    return_prefetch: u16,
    issued: bool,
}

/// Move-only permission to enter the callee reached by the real source call.
/// Neither this grant nor a public return expectation is a return receipt.
#[derive(Debug)]
pub struct CallToken {
    owner: CursorIdentity,
    call: PendingCall,
}
impl CallToken {
    pub fn identity(&self) -> (CursorIdentity, u64) {
        (self.owner, self.call.serial)
    }
    pub fn call_pc(&self) -> u32 {
        self.call.edge.from
    }
    pub fn target(&self) -> u32 {
        self.call.edge.target
    }
    pub fn return_pc(&self) -> u32 {
        self.call.edge.resume
    }
    pub fn return_prefetch(&self) -> u16 {
        self.call.return_prefetch
    }
    pub fn entry_stack(&self) -> u32 {
        self.call.entry.stack.wrapping_sub(4)
    }
    pub(crate) fn parent_stack(&self) -> u32 {
        self.call.entry.stack
    }
}

/// Actual callee ownership. A substitution is never readable composition proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalleeKind {
    TimedRust,
    ReferenceOriginalSubstitution,
    CompiledOriginal,
}
impl CalleeKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::TimedRust => "timed Rust callee",
            Self::ReferenceOriginalSubstitution => {
                "original-callee substitution (callee proven separately)"
            }
            Self::CompiledOriginal => {
                "compiled-original callee (transitional, not readable coverage)"
            }
        }
    }
    pub const fn separate_proof_required(self) -> bool {
        !matches!(self, Self::TimedRust)
    }
}

/// Actual work observed while a parent is suspended in its separately owned
/// child. Counters are inclusive observations; the root clock is charged once
/// by the physical step owner, never by this observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionWork {
    SourceInstruction,
    OriginalCalleeInstruction,
    CompiledOriginalCalleeInstruction,
    OriginalIsrInstruction,
    CompiledIsrInstruction,
    ScheduledIsrInstruction,
    Hardware,
}

/// Opaque, non-Clone continuation. Its scratch remains exclusively leased even
/// though there is no Rust borrow across the callee's canonical entry call.
pub struct ParkedRegion {
    program: ScratchIdentity,
    next: PipelineState,
    current: Option<(u32, CursorIdentity)>,
    suspended: bool,
    serial: u64,
    owner: CursorIdentity,
    usage: RegionUsage,
    ownership: IsrOwnership,
    native_controller: Option<native_interrupt::ControllerState>,
    native_program: Option<std::sync::Arc<NativeIsrProgram>>,
    target: ReturnTarget,
    entry_sp: u32,
    returned: bool,
    last_opcode: Option<u16>,
    call_entry: Option<CallEntry>,
    call_pending: Option<PendingCall>,
    maximum_steps: u64,
}
impl ParkedRegion {
    pub fn identity(&self) -> CursorIdentity {
        self.owner
    }
    pub fn usage(&self) -> RegionUsage {
        self.usage
    }
    pub fn observe_nested(
        &mut self,
        step: DeviceStep,
        work: RegionWork,
    ) -> Result<(), TimingError> {
        if !self.call_pending.is_some_and(|call| call.issued) {
            return Err(TimingError::WrongToken);
        }
        let counter = match work {
            RegionWork::SourceInstruction => Some(&mut self.usage.nested_source_instructions),
            RegionWork::OriginalCalleeInstruction => {
                Some(&mut self.usage.original_callee_instructions)
            }
            RegionWork::CompiledOriginalCalleeInstruction => {
                Some(&mut self.usage.compiled_original_callee_instructions)
            }
            RegionWork::OriginalIsrInstruction => Some(&mut self.usage.original_isr_instructions),
            RegionWork::CompiledIsrInstruction => Some(&mut self.usage.compiled_isr_instructions),
            RegionWork::ScheduledIsrInstruction => Some(&mut self.usage.scheduled_isr_instructions),
            RegionWork::Hardware => None,
        };
        if let Some(counter) = counter {
            *counter = counter.checked_add(1).ok_or(TimingError::Exhausted)?;
        }
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
        if self.usage.scheduler_steps > self.maximum_steps {
            return Err(TimingError::Exhausted);
        }
        Ok(())
    }
    pub fn observe_return(&mut self, kind: CalleeKind) -> Result<(), TimingError> {
        if !self.call_pending.is_some_and(|call| call.issued) {
            return Err(TimingError::WrongToken);
        }
        let count = match kind {
            CalleeKind::TimedRust => &mut self.usage.timed_callee_calls,
            CalleeKind::ReferenceOriginalSubstitution => {
                &mut self.usage.original_substitution_calls
            }
            CalleeKind::CompiledOriginal => &mut self.usage.compiled_original_callee_calls,
        };
        *count = count.checked_add(1).ok_or(TimingError::Exhausted)?;
        if kind.separate_proof_required() {
            self.usage.separate_callee_proof_obligations = self
                .usage
                .separate_callee_proof_obligations
                .checked_add(1)
                .ok_or(TimingError::Exhausted)?;
            match kind {
                CalleeKind::ReferenceOriginalSubstitution => {
                    self.usage.original_substitution_label = Some(kind.label())
                }
                CalleeKind::CompiledOriginal => {
                    self.usage.compiled_original_callee_label = Some(kind.label())
                }
                CalleeKind::TimedRust => {}
            }
        }
        Ok(())
    }
    pub fn resume<'a>(
        self,
        program: &'a TimingProgram,
        storage: &'a mut RegionStorage,
    ) -> Result<TimingRegion<'a>, TimingError> {
        if self.program != program.id
            || storage.program != program.id
            || storage.lease != Some(self.owner)
        {
            return Err(TimingError::WrongToken);
        }
        Ok(TimingRegion {
            program,
            scratch: &mut storage.scratch,
            next: self.next,
            current: self.current,
            suspended: self.suspended,
            serial: self.serial,
            owner: self.owner,
            usage: self.usage,
            ownership: self.ownership,
            native_isr: None,
            native_controller: self.native_controller,
            native_program: self.native_program,
            native_frames: &mut storage.native_frames,
            target: self.target,
            entry_sp: self.entry_sp,
            returned: self.returned,
            last_opcode: self.last_opcode,
            call_entry: self.call_entry,
            call_pending: self.call_pending,
            lease: &mut storage.lease,
            parked: false,
        })
    }
}
impl TimingRegion<'_> {
    pub(super) fn arm_call(&mut self, timing: &InstructionTiming) -> Result<(), TimingError> {
        let Some(entry) = self.call_entry.take() else {
            return Ok(());
        };
        let resume = self
            .program
            .calls
            .get(&(timing.pc(), timing.pipeline().pc))
            .copied()
            .ok_or(TimingError::WrongEntry)?;
        self.serial = self.serial.checked_add(1).ok_or(TimingError::Exhausted)?;
        self.call_pending = Some(PendingCall {
            edge: RegionCall {
                from: timing.pc(),
                target: timing.pipeline().pc,
                resume,
            },
            entry,
            serial: self.serial,
            start_steps: self.usage.scheduler_steps,
            return_prefetch: self
                .program
                .instructions
                .get(&resume)
                .map(TimingInstruction::opcode)
                .or_else(|| {
                    self.program
                        .fixed_target
                        .filter(|target| target.pc == resume)
                        .and_then(|target| target.prefetch)
                })
                .ok_or(TimingError::WrongEntry)?,
            issued: false,
        });
        Ok(())
    }
    pub fn authorize_call(&mut self, cpu: &CpuState) -> Result<CallToken, TimingError> {
        let call = self.call_pending.as_mut().ok_or(TimingError::WrongToken)?;
        if call.issued
            || self.current.is_some()
            || self.suspended
            || self.usage.scheduler_steps <= call.start_steps
            || cpu.pc != call.edge.target
            || cpu.pc != self.next.pc
            || cpu.prefetch != self.next.prefetch
            || !call.entry.matches_call(cpu)
            || cpu.address_error != 0
            || cpu.frozen != 0
        {
            return Err(TimingError::WrongEntry);
        }
        call.issued = true;
        Ok(CallToken {
            owner: self.owner,
            call: *call,
        })
    }
    /// The adapter calls this only after consuming the child's sealed real RTS
    /// receipt. It changes continuation metadata, never physical CPU/device state.
    pub(crate) fn finish_call(
        &mut self,
        token: CallToken,
        cpu: &CpuState,
        signals: BoundarySignals,
    ) -> Result<(), TimingError> {
        if token.owner != self.owner
            || self.call_pending != Some(token.call)
            || !token.call.issued
            || self.current.is_some()
            || self.suspended
            || cpu.pc != token.call.edge.resume
            || cpu.prefetch != u32::from(token.return_prefetch())
            || cpu.read_address(7) != token.parent_stack()
            || classify_boundary(cpu, signals) != BoundaryAction::Ready
        {
            return Err(TimingError::WrongToken);
        }
        self.next = PipelineState {
            pc: cpu.pc,
            prefetch: cpu.prefetch,
            last_muldiv: cpu.last_muldiv,
        };
        if self.program.reviewed.return_pc == Some(self.next.pc) {
            self.returned = true;
        }
        self.call_pending = None;
        Ok(())
    }
    pub fn park(mut self) -> Result<ParkedRegion, TimingError> {
        if self.current.is_some()
            || self.suspended
            || self.native_isr.is_some()
            || !self.call_pending.is_some_and(|call| call.issued)
        {
            return Err(TimingError::WrongToken);
        }
        self.parked = true;
        Ok(ParkedRegion {
            program: self.program.id,
            next: self.next,
            current: self.current,
            suspended: self.suspended,
            serial: self.serial,
            owner: self.owner,
            usage: self.usage,
            ownership: self.ownership,
            native_controller: self.native_controller.take(),
            native_program: self.native_program.take(),
            target: self.target,
            entry_sp: self.entry_sp,
            returned: self.returned,
            last_opcode: self.last_opcode,
            call_entry: self.call_entry,
            call_pending: self.call_pending,
            maximum_steps: self.program.reviewed.maximum_steps,
        })
    }
}
impl Drop for TimingRegion<'_> {
    fn drop(&mut self) {
        if !self.parked {
            *self.lease = None;
        }
    }
}

/// Captured before, and consumed only by, an actual authenticated compiled call
/// instruction. This is internal kernel evidence, never a game-source request.
#[cfg(feature = "devices")]
pub(crate) struct CompiledCallStart {
    owner: CursorIdentity,
    serial: u64,
    from: u32,
    resume: u32,
    return_prefetch: u16,
    entry: CallEntry,
    start_steps: u64,
}
#[cfg(feature = "devices")]
impl CompiledCallStart {
    pub(crate) fn capture(
        owner: CursorIdentity,
        serial: u64,
        start_steps: u64,
        cpu: &CpuState,
        resume: u32,
        return_prefetch: u16,
    ) -> Result<Self, TimingError> {
        if !is_call(cpu.prefetch as u16)
            || cpu.prefetch > u32::from(u16::MAX)
            || cpu.pc & 1 != 0
            || resume & 1 != 0
            || resume <= cpu.pc
            || cpu.address_error != 0
            || cpu.frozen != 0
            || cpu.pending_interrupt != 0
            || cpu.stopped != 0
        {
            return Err(TimingError::WrongEntry);
        }
        Ok(Self {
            owner,
            serial,
            from: cpu.pc,
            resume,
            return_prefetch,
            entry: CallEntry::capture(cpu),
            start_steps,
        })
    }
    pub(crate) fn complete(
        self,
        step: crate::RuntimeStep,
        observed_steps: u64,
        cpu: &CpuState,
        return_word: u32,
    ) -> Result<CallToken, TimingError> {
        if step.kind != crate::StepKind::Instruction
            || step.instruction_pc != Some(self.from)
            || step.devices.master_clocks == 0
            || step.devices.cpu_cycles == 0
            || self.start_steps.checked_add(1) != Some(observed_steps)
            || !self.entry.matches_call(cpu)
            || return_word != self.resume
            || cpu.pc & 1 != 0
            || cpu.address_error != 0
            || cpu.frozen != 0
        {
            return Err(TimingError::WrongToken);
        }
        Ok(CallToken {
            owner: self.owner,
            call: PendingCall {
                edge: RegionCall {
                    from: self.from,
                    target: cpu.pc,
                    resume: self.resume,
                },
                entry: self.entry,
                serial: self.serial,
                start_steps: self.start_steps,
                return_prefetch: self.return_prefetch,
                issued: true,
            },
        })
    }
}

impl RegionStorage {
    /// Trusted trajectory owner only: recycle scratch after the associated
    /// speculative world was discarded or its actual hardware reset completed.
    /// This cancels metadata, never produces a return or repairs a live machine.
    pub(crate) fn discard_parked(
        &mut self,
        parked: ParkedRegion,
    ) -> Result<RegionUsage, TimingError> {
        if self.program != parked.program || self.lease != Some(parked.owner) {
            return Err(TimingError::WrongToken);
        }
        self.lease = None;
        Ok(parked.usage)
    }
}
