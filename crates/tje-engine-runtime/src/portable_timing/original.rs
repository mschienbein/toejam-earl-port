//! Original-callee controller. Adapters supply observations of actual completed
//! kernel steps; only the controller seals the ordinary return. No original-root
//! directive, CPU snapshot import, cycle grant or provider-owned returned boolean.
use super::{CallProfile, CertifiedReturn, CpuState, Invocation, TimingError};
use crate::{
    DeviceStep,
    timing_region::{BoundarySignals, CallToken},
};
#[cfg(feature = "devices")]
use crate::{RuntimeStep, timing_region::CompiledCallStart};
use tje_engine_core::timing_region::CursorIdentity;

/// Captured at an actual executed opcode, not at a ready-PC guess. The host
/// obtains it from its original instruction counter; native uses RuntimeStep's
/// actual instruction_pc and the live boundary observed immediately before it.
#[derive(Debug, Clone, Copy)]
pub struct OriginalInstruction {
    pub pc: u32,
    pub opcode: u16,
    pub stack: u32,
    pub interrupt: bool,
}

pub struct OriginalController {
    invocation: Invocation,
    owner: CursorIdentity,
    entry_sp: u32,
    return_pc: u32,
    profile: CallProfile,
    steps: u64,
    clocks: u64,
    instructions: u64,
    #[cfg(feature = "devices")]
    serial: u64,
    returned: bool,
    #[cfg(feature = "devices")]
    last: Option<OriginalInstruction>,
    suspended_call: Option<(CursorIdentity, u64)>,
}
impl OriginalController {
    pub(super) fn new(
        invocation: Invocation,
        owner: CursorIdentity,
        entry_sp: u32,
        return_pc: u32,
        profile: CallProfile,
    ) -> Self {
        Self {
            invocation,
            owner,
            entry_sp,
            return_pc,
            profile,
            steps: 0,
            clocks: 0,
            instructions: 0,
            #[cfg(feature = "devices")]
            serial: 0,
            returned: false,
            #[cfg(feature = "devices")]
            last: None,
            suspended_call: None,
        }
    }
    pub fn invocation(&self) -> &Invocation {
        &self.invocation
    }
    pub fn profile(&self) -> CallProfile {
        self.profile
    }
    pub fn return_pc(&self) -> u32 {
        self.return_pc
    }
    pub fn entry_stack(&self) -> u32 {
        self.entry_sp
    }
    pub fn saw_return(&self) -> bool {
        self.returned
    }
    pub fn observe(
        &mut self,
        step: DeviceStep,
        instruction: Option<OriginalInstruction>,
        cpu: &CpuState,
    ) -> Result<(), TimingError> {
        if self.suspended_call.is_some() || step.master_clocks == 0 {
            return Err(TimingError::WrongToken);
        }
        self.steps = self.steps.checked_add(1).ok_or(TimingError::Exhausted)?;
        self.clocks = self
            .clocks
            .checked_add(step.master_clocks)
            .ok_or(TimingError::Exhausted)?;
        #[cfg(feature = "devices")]
        {
            self.last = instruction;
        }
        if let Some(instruction) = instruction
            && !instruction.interrupt
        {
            if self.returned {
                return Err(TimingError::WrongEntry);
            }
            self.instructions = self
                .instructions
                .checked_add(1)
                .ok_or(TimingError::Exhausted)?;
            if instruction.opcode == 0x4e75
                && instruction.stack == self.entry_sp
                && cpu.pc == self.return_pc
                && cpu.read_address(7) == self.entry_sp.wrapping_add(4)
                && cpu.address_error == 0
                && cpu.frozen == 0
            {
                self.returned = true;
            }
        }
        if cpu.address_error != 0 || cpu.frozen != 0 {
            return Err(TimingError::PendingFault);
        }
        Ok(())
    }
    #[cfg(feature = "devices")]
    pub(crate) fn prepare_compiled_call(
        &mut self,
        cpu: &CpuState,
        resume: u32,
        return_prefetch: u16,
    ) -> Result<CompiledCallStart, TimingError> {
        if self.profile != CallProfile::NativeCompiledOriginalCallees
            || self.returned
            || self.suspended_call.is_some()
        {
            return Err(TimingError::WrongToken);
        }
        self.serial = self.serial.checked_add(1).ok_or(TimingError::Exhausted)?;
        CompiledCallStart::capture(
            self.owner,
            self.serial,
            self.steps,
            cpu,
            resume,
            return_prefetch,
        )
    }
    #[cfg(feature = "devices")]
    pub(crate) fn suspend_compiled_call(
        &mut self,
        start: CompiledCallStart,
        step: RuntimeStep,
        cpu: &CpuState,
        return_word: u32,
    ) -> Result<CallToken, TimingError> {
        let token = start.complete(step, self.steps, cpu, return_word)?;
        if self.suspended_call.is_some()
            || !self.last.is_some_and(|instruction| {
                instruction.pc == token.call_pc()
                    && !instruction.interrupt
                    && matches!(
                        tje_engine_core::decode::decode(instruction.opcode),
                        tje_engine_core::decode::Instruction::JumpToSubroutine(_)
                            | tje_engine_core::decode::Instruction::BranchToSubroutine(_)
                    )
            })
        {
            return Err(TimingError::WrongToken);
        }
        self.suspended_call = Some(token.identity());
        Ok(token)
    }
    pub(super) fn resume_call(
        &mut self,
        token: CallToken,
        cpu: &CpuState,
        signals: BoundarySignals,
    ) -> Result<(), TimingError> {
        if self.suspended_call != Some(token.identity())
            || token.identity().0 != self.owner
            || cpu.pc != token.return_pc()
            || cpu.read_address(7) != token.parent_stack()
            || cpu.prefetch != u32::from(token.return_prefetch())
            || crate::timing_region::classify_boundary(cpu, signals)
                != crate::timing_region::BoundaryAction::Ready
        {
            return Err(TimingError::WrongToken);
        }
        self.suspended_call = None;
        #[cfg(feature = "devices")]
        {
            self.last = None;
        }
        Ok(())
    }
    pub(super) fn finish(
        self,
        cpu: &CpuState,
        signals: BoundarySignals,
    ) -> Result<(Invocation, CertifiedReturn), TimingError> {
        if !self.returned
            || self.suspended_call.is_some()
            || self.steps == 0
            || self.clocks == 0
            || self.instructions == 0
        {
            return Err(TimingError::WrongToken);
        }
        let receipt = super::completion::certify_original_return(
            &self.invocation,
            self.owner,
            self.return_pc,
            cpu,
            signals,
        )?;
        Ok((self.invocation, receipt))
    }
}

#[cfg(feature = "devices")]
impl OriginalController {
    pub(crate) fn interrupt_identity(
        &mut self,
        action: crate::timing_region::BoundaryAction,
    ) -> Result<(CursorIdentity, u64), TimingError> {
        if self.profile != CallProfile::NativeCompiledOriginalCallees
            || self.suspended_call.is_some()
            || !matches!(
                action,
                crate::timing_region::BoundaryAction::InterruptAdmission { level: 1..=7 }
                    | crate::timing_region::BoundaryAction::InterruptAcknowledge { level: 1..=7 }
            )
        {
            return Err(TimingError::WrongToken);
        }
        self.serial = self.serial.checked_add(1).ok_or(TimingError::Exhausted)?;
        Ok((self.owner, self.serial))
    }
}
