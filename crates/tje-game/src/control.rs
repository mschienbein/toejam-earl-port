//! Constructible control-request/observation DATA, independent of any emulator.
//!
//! Shape validation returns only `()`: it does not authenticate an instruction,
//! entry, invocation, origin, frame, device event, root, or trajectory. In
//! particular `epoch_tag`, spans and raw flags are diagnostic data, NOT receipts.
//! Only the host's private original collector and single pre-fork RootAdmission
//! may bind these facts. This module adds no Context operations or activation.
use crate::{
    FunctionId,
    exits::{ControlTransfer, Exit, ExitContract, StackBanks},
};
use tje_mem::Revision;

mod exception;
pub use exception::{
    ExceptionPreparedState, ExceptionRequest, SynchronousTrap, runtime_divide_fault_offset,
};

const SR_MASK: u16 = 0xa71f;
const SUPERVISOR: u16 = 0x2000;
const BUSREQ: u32 = 0xa11100;
const Z80_RESET: u32 = 0xa11200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlSite {
    pub entry: FunctionId,
    /// Signed offset from this invocation's root entry. Preceding retry
    /// fragments must retain their negative offsets, not an unsigned mask.
    pub byte_offset: i32,
}
impl ControlSite {
    pub fn address(self, revision: Revision) -> u32 {
        self.entry
            .metadata()
            .address(revision)
            .wrapping_add_signed(self.byte_offset)
    }
    pub fn validate_shape(self) -> Result<(), ControlDataError> {
        if self.byte_offset & 1 != 0 {
            return Err(ControlDataError::Site);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlSpan {
    /// Half-open successful OWN source-retirement lane of one original
    /// invocation. Nested callees and ISRs are excluded, never subtracted later.
    pub retirement_start: u64,
    pub retirement_end: u64,
    /// Half-open owner-local physical CPU ISSUE lane, including prefetch and
    /// discarded reads. Physical commit indices are a separate diagnostic axis.
    pub access_start: u64,
    pub access_end: u64,
    /// Not trusted invocation/trajectory identity; equality mints no authority.
    pub epoch_tag: u64,
}
impl ControlSpan {
    pub fn validate_shape(&self) -> Result<(), ControlDataError> {
        if self.retirement_end <= self.retirement_start || self.access_end < self.access_start {
            return Err(ControlDataError::Span);
        }
        Ok(())
    }
    pub const fn contains_access(&self, index: u64) -> bool {
        self.access_start <= index && index < self.access_end
    }
    fn require_retirements(&self, count: u64) -> Result<(), ControlDataError> {
        self.validate_shape()?;
        if self.retirement_end - self.retirement_start != count {
            return Err(ControlDataError::Span);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDelayKind {
    Pcm24,
    Psg124,
}
impl RetryDelayKind {
    pub const fn attempts(self) -> u16 {
        match self {
            Self::Pcm24 => 24,
            Self::Psg124 => 124,
        }
    }
    pub const fn moveq_value(self) -> u32 {
        self.attempts() as u32 - 1
    }
    pub const fn retirements(self) -> u64 {
        1 + self.attempts() as u64
    }
}

/// Requests, never proof tokens. Device variants specify exactly one WORD at
/// their fixed port; they cannot request a byte/long substitute or another port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlRequest {
    WriteStatusRegister { value: u16 },
    WriteUserStackPointer { value: u32 },
    Z80BusRequest { value: u16 },
    Z80Reset { value: u16 },
    RetryDelay(RetryDelayKind),
    JoinVdpFill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuControlState {
    pub pc: u32,
    pub sp: u32,
    pub status: u16,
    pub banks: StackBanks,
    pub exception_depth: u32,
}
impl CpuControlState {
    pub const fn selected_stack(&self) -> u32 {
        if self.status & SUPERVISOR != 0 {
            self.banks.supervisor
        } else {
            self.banks.user
        }
    }
    pub fn validate_shape(&self) -> Result<(), ControlDataError> {
        if self.sp != self.selected_stack() || self.status & !SR_MASK != 0 {
            return Err(ControlDataError::CpuState);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Z80ControlState {
    pub bus_requested: bool,
    pub reset_asserted: bool,
    /// Raw observed signal from the retained reference model, not an invented
    /// ready bit. Its current pure formula is checked without reading MMIO.
    pub bus_ack: bool,
}
impl Z80ControlState {
    pub fn validate_shape(self) -> Result<(), ControlDataError> {
        if self.bus_ack != (self.bus_requested && !self.reset_asserted) {
            return Err(ControlDataError::DeviceSignals);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceWriteData {
    pub address: u32,
    pub word: bool,
    pub value: u16,
    /// At issue: owner-local physical CPU ISSUE lane. At commit: the root
    /// invocation's chronological physical-COMMIT lane, including nested and
    /// environment commits. Equality across these diagnostic axes proves nothing.
    pub index: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceControlData {
    pub issue: DeviceWriteData,
    pub commit: DeviceWriteData,
    pub before: Z80ControlState,
    pub after: Z80ControlState,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryDelayData {
    pub kind: RetryDelayKind,
    pub moveq_value: u32,
    pub dbra_attempts: u16,
    pub taken_edges: u16,
    pub terminal_fallthrough: bool,
    pub final_d0: u32,
    pub status_before: u16,
    pub status_after: u16,
    /// Raw summary only. The private collector must authenticate the ACTUAL
    /// released window and every MOVEQ/DBRA, not infer it from endpoint writes.
    pub released_for_all_iterations: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillBoundaryData {
    pub generation: u64,
    pub site: ControlSite,
    pub span: ControlSpan,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillJoinData {
    /// None is UNKNOWN, not generation zero. The private original collector
    /// must bind activation/completion to the same actual owned fill.
    pub activation: Option<FillBoundaryData>,
    pub completion: Option<FillBoundaryData>,
    pub fill_mode: bool,
    pub active_at_join: bool,
    pub queued_data_writes: u32,
    pub queued_control_writes: u32,
    pub intervening_starts: u32,
    pub producer_span: ControlSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlActual {
    WriteStatusRegister {
        value: u16,
        before: CpuControlState,
        after: CpuControlState,
    },
    WriteUserStackPointer {
        value: u32,
        before: CpuControlState,
        after: CpuControlState,
    },
    Z80BusRequest(DeviceControlData),
    Z80Reset(DeviceControlData),
    RetryDelay(RetryDelayData),
    JoinVdpFill(FillJoinData),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlObservation {
    pub site: ControlSite,
    pub span: ControlSpan,
    pub requested: ControlRequest,
    pub actual: ControlActual,
}
impl ControlObservation {
    /// Structural/semantic consistency ONLY, never source authentication.
    pub fn validate_shape(&self, revision: Revision) -> Result<(), ControlDataError> {
        self.site.validate_shape()?;
        match (self.requested, &self.actual) {
            (
                ControlRequest::WriteStatusRegister { value },
                ControlActual::WriteStatusRegister {
                    value: actual,
                    before,
                    after,
                },
            ) => {
                self.cpu_instruction(revision, before, after)?;
                if value != *actual {
                    return Err(ControlDataError::Operand);
                }
                if after.status != *actual & SR_MASK
                    || after.banks != before.banks
                    || after.exception_depth != before.exception_depth
                {
                    return Err(ControlDataError::Transition);
                }
            }
            (
                ControlRequest::WriteUserStackPointer { value },
                ControlActual::WriteUserStackPointer {
                    value: actual,
                    before,
                    after,
                },
            ) => {
                self.cpu_instruction(revision, before, after)?;
                if value != *actual {
                    return Err(ControlDataError::Operand);
                }
                if after.banks.user != *actual
                    || after.banks.supervisor != before.banks.supervisor
                    || after.status != before.status
                    || after.exception_depth != before.exception_depth
                {
                    return Err(ControlDataError::Transition);
                }
            }
            (ControlRequest::Z80BusRequest { value }, ControlActual::Z80BusRequest(data)) => {
                self.device_word(data, BUSREQ, value)?;
                let requested = value == 0x100;
                if data.after.bus_requested != requested
                    || data.after.reset_asserted != data.before.reset_asserted
                {
                    return Err(ControlDataError::DeviceSignals);
                }
            }
            (ControlRequest::Z80Reset { value }, ControlActual::Z80Reset(data)) => {
                self.device_word(data, Z80_RESET, value)?;
                if data.after.reset_asserted != (value == 0)
                    || data.after.bus_requested != data.before.bus_requested
                {
                    return Err(ControlDataError::DeviceSignals);
                }
            }
            (ControlRequest::RetryDelay(kind), ControlActual::RetryDelay(data)) => {
                self.span.require_retirements(kind.retirements())?;
                if data.kind != kind
                    || data.moveq_value != kind.moveq_value()
                    || data.dbra_attempts != kind.attempts()
                    || data.taken_edges != kind.attempts() - 1
                    || !data.terminal_fallthrough
                    || data.final_d0 != 0xffff
                    || data.status_after != data.status_before & !0xf
                {
                    return Err(ControlDataError::Delay);
                }
                if !data.released_for_all_iterations {
                    return Err(ControlDataError::DeviceSignals);
                }
            }
            (ControlRequest::JoinVdpFill, ControlActual::JoinVdpFill(data)) => {
                self.span.validate_shape()?;
                let (Some(activation), Some(completion)) = (&data.activation, &data.completion)
                else {
                    return Err(ControlDataError::FillIdentity);
                };
                activation.site.validate_shape()?;
                completion.site.validate_shape()?;
                activation.span.validate_shape()?;
                completion.span.validate_shape()?;
                data.producer_span.validate_shape()?;
                if activation.generation != completion.generation {
                    return Err(ControlDataError::FillIdentity);
                }
                if activation.site.entry != self.site.entry
                    || completion.site.entry != self.site.entry
                    || !data.fill_mode
                    || data.active_at_join
                    || data.queued_data_writes != 0
                    || data.queued_control_writes != 0
                    || data.intervening_starts != 0
                {
                    return Err(ControlDataError::FillCausality);
                }
                // These raw ranges locate proposed causal work, but equal tags
                // or numeric order never authenticate an invocation/producer.
                if completion.span.retirement_start < activation.span.retirement_start
                    || completion.span.retirement_end < activation.span.retirement_end
                    || activation.span.retirement_end > data.producer_span.retirement_end
                    || completion.span.retirement_end > self.span.retirement_end
                    || data.producer_span.retirement_end > self.span.retirement_end
                {
                    return Err(ControlDataError::FillCausality);
                }
            }
            _ => return Err(ControlDataError::Kind),
        }
        Ok(())
    }
    fn cpu_instruction(
        &self,
        revision: Revision,
        before: &CpuControlState,
        after: &CpuControlState,
    ) -> Result<(), ControlDataError> {
        self.span.require_retirements(1)?;
        before.validate_shape()?;
        after.validate_shape()?;
        if before.pc != self.site.address(revision) {
            return Err(ControlDataError::Site);
        }
        if before.status & SUPERVISOR == 0 {
            return Err(ControlDataError::Privilege);
        }
        Ok(())
    }
    fn device_word(
        &self,
        data: &DeviceControlData,
        address: u32,
        value: u16,
    ) -> Result<(), ControlDataError> {
        self.span.require_retirements(1)?;
        if !matches!(value, 0 | 0x100) {
            return Err(ControlDataError::Operand);
        }
        if !data.issue.word
            || !data.commit.word
            || data.issue.address != address
            || data.commit.address != address
            || data.issue.value != value
            || data.commit.value != value
            || !self.span.contains_access(data.issue.index)
        {
            return Err(ControlDataError::DeviceWrite);
        }
        data.before.validate_shape()?;
        data.after.validate_shape()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlReadPurpose {
    Semantic,
    Discarded,
    Prefetch,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlWordRead {
    /// Physical bus address, unlike the full architectural stack pointer.
    pub address: u32,
    pub word: bool,
    pub value: u16,
    pub purpose: ControlReadPurpose,
    pub issue_index: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtePopData {
    pub words: [ControlWordRead; 3],
}

/// Actual exit DATA, not a second public Exit enum or a successful-exit token.
/// None transfer represents RTS; only the original private collector may attest
/// an actually successful RTS/RTE/tail. STOP/reset/terminal/main-loop are absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitObservation {
    pub site: ControlSite,
    pub span: ControlSpan,
    pub contract: ExitContract,
    pub transfer: Option<ControlTransfer>,
    pub before: CpuControlState,
    pub after: CpuControlState,
    pub rte_pop: Option<RtePopData>,
}
impl ExitObservation {
    pub fn validate_shape(&self, revision: Revision) -> Result<(), ControlDataError> {
        self.site.validate_shape()?;
        self.span.require_retirements(1)?;
        self.before.validate_shape()?;
        self.after.validate_shape()?;
        if self.before.pc != self.site.address(revision) || self.after.pc & 1 != 0 {
            return Err(ControlDataError::Site);
        }
        if self
            .transfer
            .is_some_and(|transfer| transfer.entry != self.site.entry)
        {
            return Err(ControlDataError::Exit);
        }
        match (self.contract, self.transfer.map(|transfer| transfer.exit)) {
            (ExitContract::Rts, None) => {
                if self.before.sp & 1 != 0 {
                    return Err(ControlDataError::Pop);
                }
                if self.rte_pop.is_some()
                    || self.after.status != self.before.status
                    || self.after.exception_depth != self.before.exception_depth
                {
                    return Err(ControlDataError::Exit);
                }
                let mut expected = self.before.banks;
                if self.before.status & SUPERVISOR != 0 {
                    expected.supervisor = expected.supervisor.wrapping_add(4);
                } else {
                    expected.user = expected.user.wrapping_add(4);
                }
                if self.after.banks != expected {
                    return Err(ControlDataError::Exit);
                }
            }
            (ExitContract::ExceptionReturn, Some(Exit::ExceptionReturn)) => self.rte_shape()?,
            (ExitContract::Tail { target }, Some(Exit::Tail { target: actual })) => {
                if target != actual
                    || self.rte_pop.is_some()
                    || self.after.pc != target.metadata().address(revision)
                    || self.after.banks != self.before.banks
                    || self.after.sp != self.before.sp
                    || self.after.status != self.before.status
                    || self.after.exception_depth != self.before.exception_depth
                {
                    return Err(ControlDataError::Exit);
                }
            }
            (
                ExitContract::Call { .. } | ExitContract::Terminal | ExitContract::MainLoop { .. },
                _,
            ) => return Err(ControlDataError::UnsupportedExit),
            _ => return Err(ControlDataError::Exit),
        }
        Ok(())
    }
    fn rte_shape(&self) -> Result<(), ControlDataError> {
        let Some(pop) = self.rte_pop.as_ref() else {
            return Err(ControlDataError::Pop);
        };
        if self.before.status & SUPERVISOR == 0
            || self.before.exception_depth == 0
            || self.after.exception_depth != self.before.exception_depth - 1
        {
            return Err(ControlDataError::Pop);
        }
        let sp = self.before.banks.supervisor;
        for (index, word) in pop.words.iter().enumerate() {
            if !word.word
                || word.purpose != ControlReadPurpose::Semantic
                || word.address != sp.wrapping_add(index as u32 * 2) & 0xffffff
                || !self.span.contains_access(word.issue_index)
                || (index > 0 && pop.words[index - 1].issue_index >= word.issue_index)
            {
                return Err(ControlDataError::Pop);
            }
        }
        if sp & 1 != 0
            || self.after.status != pop.words[0].value & SR_MASK
            || self.after.pc
                != (u32::from(pop.words[1].value) << 16) | u32::from(pop.words[2].value)
            || self.after.banks.supervisor != sp.wrapping_add(6)
            || self.after.banks.user != self.before.banks.user
        {
            return Err(ControlDataError::Pop);
        }
        Ok(())
    }
}

/// A raw consistency error, deliberately not a check/admission/proof verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlDataError {
    Site,
    Span,
    Kind,
    Operand,
    CpuState,
    Privilege,
    Transition,
    DeviceWrite,
    DeviceSignals,
    Delay,
    FillIdentity,
    FillCausality,
    UnsupportedExit,
    Exit,
    Pop,
    Exception,
}

#[cfg(test)]
#[path = "control/tests.rs"]
mod tests;
