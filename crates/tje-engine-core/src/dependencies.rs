//! Register/bit dependencies derived from the compiler's existing instruction SSA.
//!
//! This is analysis, not a CPU: no bus access, device advance or state update is
//! performed. All 25 canonical state domains are represented. A7 is the selected
//! USP/SSP bank and its SR.S dependency, exactly as in `CpuState` and the IR.
//! Preserved upper register/status bits are reported separately and also count
//! as reads in the conservative default fold. No opcode dependency table exists.
mod analysis;
mod facts;
#[cfg(test)]
mod output_tests;
#[cfg(test)]
mod tests;

use crate::{
    CompileError, CpuState, InstructionIr, ReadKind, compile_instruction,
    ir::{StateField, Width},
};
pub use analysis::DependencyScratch;
use std::{
    fmt,
    ops::{BitAnd, BitOr, BitOrAssign, Sub},
};

/// Bit masks for every canonical field, not only the architectural registers.
/// `get(Sr) & 0x1f` is CCR. Reserved/nonarchitectural domains are never silently
/// projected away; callers must explicitly choose `registers_only` if appropriate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StateMask([u32; StateField::COUNT]);
impl StateMask {
    pub const EMPTY: Self = Self([0; StateField::COUNT]);
    pub const ALL: Self = Self([u32::MAX; StateField::COUNT]);
    pub fn get(&self, field: StateField) -> u32 {
        self.0[field.index() as usize]
    }
    pub fn insert(&mut self, field: StateField, bits: u32) {
        self.0[field.index() as usize] |= bits;
    }
    pub fn single(field: StateField, bits: u32) -> Self {
        let mut masks = Self::EMPTY;
        masks.insert(field, bits);
        masks
    }
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|&mask| mask == 0)
    }
    pub fn iter(&self) -> impl Iterator<Item = (StateField, u32)> + '_ {
        self.0
            .iter()
            .copied()
            .enumerate()
            .filter(|&(_, bits)| bits != 0)
            .map(|(index, bits)| {
                (
                    StateField::from_index(index).expect("canonical field index"),
                    bits,
                )
            })
    }
    /// Explicit projection; PC/prefetch/pending/halt/fault/timing domains remain
    /// present in the original report and still require a consumer policy.
    pub fn registers_only(self) -> Self {
        let mut result = self;
        for index in 17..StateField::COUNT {
            if index != 18 {
                result.0[index] = 0;
            }
        }
        result
    }
    pub fn active_a7(&self, sr: u32) -> u32 {
        self.get(if sr & 0x2000 != 0 {
            StateField::Ssp
        } else {
            StateField::Usp
        })
    }
}
impl BitOr for StateMask {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(std::array::from_fn(|index| self.0[index] | rhs.0[index]))
    }
}
impl BitOrAssign for StateMask {
    fn bitor_assign(&mut self, rhs: Self) {
        for (lhs, rhs) in self.0.iter_mut().zip(rhs.0) {
            *lhs |= rhs;
        }
    }
}
impl BitAnd for StateMask {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(std::array::from_fn(|index| self.0[index] & rhs.0[index]))
    }
}
impl Sub for StateMask {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(std::array::from_fn(|index| self.0[index] & !rhs.0[index]))
    }
}

/// Observation-address domain is explicit, never inferred from a pointer.
/// Bus24 is 68000 pin truncation only, not Genesis RAM mirror normalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedAddressDomain {
    Raw32,
    M68000Bus24,
}
impl ObservedAddressDomain {
    pub fn matches(self, raw_ir: u32, observed: u32) -> bool {
        match self {
            Self::Raw32 => raw_ir == observed,
            Self::M68000Bus24 => observed <= 0x00ff_ffff && raw_ir & 0x00ff_ffff == observed,
        }
    }
}

/// Actual accepted IR read, in execution order (including discarded/prefetch).
/// Its address/width/purpose is checked before its value can refine a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedRead {
    pub address: u32,
    pub width: Width,
    pub kind: ReadKind,
    pub value: u32,
}
#[derive(Debug, Clone, Copy, Default)]
pub enum DependencyInput<'a> {
    /// Union possible inputs and intersect definitions across all IR exits.
    #[default]
    Conservative,
    /// Refine only choices resolved by actual entry state and ROM constants.
    /// Memory-derived choices remain conservative and explicitly unresolved.
    Entry(&'a CpuState),
    /// Refine the accepted executed path. Every actual read must be supplied,
    /// matched and consumed exactly once; missing/extra/mismatched reads fail.
    Executed {
        before: &'a CpuState,
        reads: &'a [ObservedRead],
        addresses: ObservedAddressDomain,
    },
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PathBinding {
    #[default]
    Conservative,
    EntryState,
    ExecutedReads {
        consumed: usize,
        addresses: ObservedAddressDomain,
    },
}

/// Individual observable roots allow the host to classify private frame
/// bookkeeping without subtracting a register that also feeds algorithm uses.
/// Omitting roots is a separate consumer proof obligation, never API approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyUseKind {
    MemoryReadAddress { width: Width, kind: ReadKind },
    ConditionalReadGuard,
    MemoryWriteAddress { width: Width },
    MemoryWriteValue { width: Width },
    StateWrite { field: StateField },
    PreservedState { field: StateField },
    ExceptionGuard { vector: u32 },
    ExceptionPayload { vector: u32 },
    ReadOffset,
    BoundaryTiming,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DependencyUse {
    pub operation: usize,
    pub kind: DependencyUseKind,
    pub reads: StateMask,
    /// Selection/bank/variable-shift controls remain required even when the
    /// consumer proves this root's data to be private or identity bookkeeping.
    pub control_reads: StateMask,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DependencyObligations {
    /// Scheduler IRQ/trace admission and their frame effects are not these IR
    /// instructions. The consumer must separately reject or account for them.
    pub interrupt_or_trace_boundary: bool,
    /// Exception frame/vector writes (including SR/SSP) occur in the scheduler.
    pub exception_entry_outside_ir: bool,
    pub exception_taken: Option<u32>,
    /// At least one input read value was unavailable for path refinement.
    pub opaque_memory_values: bool,
    /// A control-field update is visible in the all-state write masks.
    pub halt_or_status_control_write: bool,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstructionDependencies {
    /// All live initial-state uses, including explicit identity passthrough.
    pub reads: StateMask,
    /// Excludes only proven bit-identical passthrough output roots. This does
    /// not classify private storage or license ignoring an observed output.
    pub effect_reads: StateMask,
    /// Physical IR stores, including merged partial fields and inactive banks.
    pub physical_writes: StateMask,
    /// Bits potentially assigned a non-passthrough value on a permitted path.
    pub possible_writes: StateMask,
    /// New definitions present at EVERY permitted exit, never a normal-path
    /// guess after an early ExitIf. Only these may kill sequence input needs.
    pub definite_writes: StateMask,
    /// Physically stored bits provably identical to their instruction-entry
    /// bits at every permitted exit (e.g. byte/word upper halves and kept CCR).
    pub preserved: StateMask,
    pub binding: PathBinding,
    pub obligations: DependencyObligations,
}
#[derive(Debug, Default)]
pub struct DependencyReport {
    pub instruction: InstructionDependencies,
    pub uses: Vec<DependencyUse>,
}
impl DependencyReport {
    pub fn selected_reads(&self, mut retain: impl FnMut(&DependencyUse) -> bool) -> StateMask {
        self.uses.iter().fold(StateMask::EMPTY, |reads, use_| {
            reads
                | use_.control_reads
                | if retain(use_) {
                    use_.reads
                } else {
                    StateMask::EMPTY
                }
        })
    }
}

/// Entry-state bits needed by selected output bits on the last successfully
/// bound executed path. This is analysis data, not permission to import values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputDependencies {
    pub pc: u32,
    pub opcode: u16,
    pub binding: PathBinding,
    /// Includes `control_reads`; path controls are never projected away.
    pub reads: StateMask,
    pub control_reads: StateMask,
    /// A demanded value or path control depends on memory. The caller must
    /// retain its own memory-causality proof; observed values are not constants.
    pub memory_values: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyError {
    Compile(CompileError),
    InvalidIr(&'static str),
    EntryMismatch,
    MissingRead(usize),
    ExtraReads { consumed: usize, supplied: usize },
    ReadMismatch(usize),
    UnresolvedExecutedRead(usize),
    ArithmeticDomain,
    OutputQueryUnavailable,
}
impl fmt::Display for DependencyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl std::error::Error for DependencyError {}
impl From<CompileError> for DependencyError {
    fn from(error: CompileError) -> Self {
        Self::Compile(error)
    }
}

/// Analyze existing compiled semantics; buffers are reused after their largest
/// instruction/use count. Results are cleared on error, never stale/empty success.
pub fn analyze_instruction(
    instruction: &InstructionIr,
    input: DependencyInput<'_>,
    scratch: &mut DependencyScratch,
    report: &mut DependencyReport,
) -> Result<(), DependencyError> {
    analysis::analyze(instruction, input, scratch, report)
}
/// Decode the actual opcode AND extension words through the existing compiler.
/// Prefer `analyze_instruction` with a cached IR on an executed instruction stream.
pub fn analyze_rom_instruction(
    rom: &[u8],
    pc: u32,
    input: DependencyInput<'_>,
    scratch: &mut DependencyScratch,
    report: &mut DependencyReport,
) -> Result<(), DependencyError> {
    scratch.invalidate_outputs();
    report.instruction = InstructionDependencies::default();
    report.uses.clear();
    let instruction = compile_instruction(rom, pc)?;
    analyze_instruction(&instruction, input, scratch, report)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DependencyFold {
    pub read_before_write: StateMask,
    pub written: StateMask,
    pub definite_written: StateMask,
    pub physical_writes: StateMask,
    pub requires_scheduler_accounting: bool,
    pub observed_path_input_equality_required: bool,
}
impl DependencyFold {
    pub fn push(&mut self, instruction: &InstructionDependencies) {
        self.push_reads(instruction, instruction.reads);
    }
    /// The selected root set requires a separate host proof (e.g. exact private
    /// frame bookkeeping), including memory causality. This never authorizes it.
    pub fn push_selected(
        &mut self,
        report: &DependencyReport,
        retain: impl FnMut(&DependencyUse) -> bool,
    ) {
        self.push_reads(&report.instruction, report.selected_reads(retain));
    }
    fn push_reads(&mut self, instruction: &InstructionDependencies, reads: StateMask) {
        self.read_before_write |= reads - self.definite_written;
        self.written |= instruction.possible_writes;
        self.physical_writes |= instruction.physical_writes;
        self.definite_written |= instruction.definite_writes;
        self.requires_scheduler_accounting |= instruction.obligations.exception_entry_outside_ir
            || instruction.obligations.interrupt_or_trace_boundary;
        self.observed_path_input_equality_required |=
            instruction.binding != PathBinding::Conservative;
    }
    /// Declared outputs that pass through unchanged are inputs too if a caller
    /// intends to import them from the oracle rather than keep its own values.
    pub fn observe_outputs(&mut self, outputs: StateMask) {
        self.read_before_write |= outputs - self.definite_written;
    }
}
