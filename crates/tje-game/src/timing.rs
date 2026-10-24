//! Portable source-owned instruction regions. No opcode, engine, clock or host API.
//!
//! Locations are authored source-role ordinals inside a catalogued function.
//! The adapter binds them to instructions decoded from the user's ROM. A source
//! yields one semantic/discarded bus effect at a time and accepts its actual
//! acknowledgement before proceeding. Pipeline prefetch is adapter-owned.
use crate::{FunctionId, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
    pub function: FunctionId,
    pub role: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct FunctionRegion {
    pub function: FunctionId,
    pub roles: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct Edge {
    pub from: Location,
    pub to: Location,
}

/// Source control-flow metadata, never a ROM listing or an elapsed-time table.
pub struct Program {
    pub name: &'static str,
    pub source: &'static str,
    pub entry: Location,
    pub functions: &'static [FunctionRegion],
    pub edges: &'static [Edge],
}

/// Architectural game state only: no opcode, prefetch or private timing state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registers {
    pub data: [u32; 8],
    pub address: [u32; 7],
    pub user_stack: u32,
    pub supervisor_stack: u32,
    pub status: u16,
}
impl Registers {
    pub fn stack_pointer(&self) -> u32 {
        if self.status & 0x2000 != 0 {
            self.supervisor_stack
        } else {
            self.user_stack
        }
    }
    pub fn set_stack_pointer(&mut self, value: u32) {
        if self.status & 0x2000 != 0 {
            self.supervisor_stack = value;
        } else {
            self.user_stack = value;
        }
    }
}

/// Addresses are runtime code identities, not instruction bytes or game values.
#[derive(Debug, Clone, Copy)]
pub struct InstructionSite {
    pub location: Location,
    pub address: u32,
    pub fallthrough: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Byte,
    Word,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadPurpose {
    Semantic,
    Discarded,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    Read(ReadPurpose),
    Write(u16),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effect {
    pub address: u32,
    pub width: Width,
    pub kind: EffectKind,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub effect: Effect,
    pub value: u16,
}

/// Source chooses every branch/callee/return. The engine validates, not chooses,
/// the corresponding runtime instruction boundary. Calls and their stack effects
/// are real source operations; there is no original-callee execution directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    Next(Location),
    /// Complete this program, never an internal RTS. Internal returns use Next
    /// after the source's real return-address reads and stack update.
    Return {
        address: u32,
        value: Option<u32>,
    },
}
#[derive(Debug)]
pub struct Commit {
    pub registers: Registers,
    pub path: Path,
}

/// Object safe; no retained Context/platform borrow, cycle constants or decoded
/// instruction callback. Implementations must reject missing/repeated/wrong acks.
pub trait TimingSource {
    fn program(&self) -> &'static Program;
    fn begin_instruction(&mut self, site: InstructionSite, registers: Registers) -> Result<()>;
    fn next_effect(&mut self) -> Result<Option<Effect>>;
    fn acknowledge(&mut self, actual: Observation) -> Result<()>;
    fn finish_instruction(&mut self) -> Result<Commit>;
}

/// A completed ordinary return. The platform separately retains the sealed
/// completion receipt; constructing a value cannot skip exit finalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedExit {
    pub entry: FunctionId,
    pub program_counter: u32,
    pub stack_pointer: u32,
    pub value: Option<u32>,
}

/// Finalization requests contain expectations only, never completion authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletionRequest {
    pub entry: FunctionId,
    pub entry_sp: u32,
    pub kind: CompletionKind,
    pub expected_pc: Option<u32>,
    pub value: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    /// No timing program ran in this invocation: retain ordinary finalization.
    Absent,
    /// A platform-private, live-validated receipt was consumed exactly once.
    Consumed,
}

/// Expected finalizer kind, not a request to execute an exception return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Rts,
    ExceptionReturn,
    ControlTransfer,
}

#[cfg(test)]
mod tests;
