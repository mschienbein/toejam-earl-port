//! Dependency-only schedules for readable Rust effects, not a CPU interpreter.
//!
//! The pinned reference issues every CPU read/prefetch and queues writes before
//! advancing devices by the complete instruction charge. The device layer then
//! publishes its existing deferred-width queues. We preserve those phases, not
//! unmodelled physical 68000 microcycles. No game register/RAM/MMIO effect is
//! executed here; Rust supplies live state, actual reads and its control choice.

use crate::{
    CpuState, ReadKind, StepResult,
    ir::{BinaryOp, CompareOp, InstructionIr, Op, StateField, Width},
};
use std::{fmt, sync::Mutex};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod binding_tests;

#[cfg(test)]
mod lifetime_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceBusPhase {
    Issue,
    AdvanceAndCommit,
    InstructionBoundary,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Read(ReadKind),
    Write,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedAccess {
    pub instruction_pc: u32,
    pub ordinal: u32,
    pub address: u32,
    pub width: Width,
    pub kind: AccessKind,
    pub phase: ReferenceBusPhase,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusObservation {
    pub address: u32,
    pub width: Width,
    pub kind: AccessKind,
    pub value: u32,
}
/// Rust owns this decision. It is checked against the dependency expression,
/// but the scheduler never chooses or follows a game branch on Rust's behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RustPathChoice {
    pub next_pc: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineState {
    pub pc: u32,
    pub prefetch: u32,
    pub last_muldiv: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScratchIdentity(u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorIdentity {
    scratch: ScratchIdentity,
    generation: u64,
}

/// A non-cloneable, non-forgeable advance/commit grant, emitted only after every
/// bus token was acknowledged and Rust's explicit path was validated.
#[derive(Debug)]
pub struct InstructionTiming {
    pc: u32,
    opcode: u16,
    cycles: u32,
    pipeline: PipelineState,
    owner: CursorIdentity,
    bus_effects: u32,
    phase_updates: u32,
}
impl InstructionTiming {
    pub fn pc(&self) -> u32 {
        self.pc
    }
    pub fn opcode(&self) -> u16 {
        self.opcode
    }
    pub fn cycles(&self) -> u32 {
        self.cycles
    }
    pub fn pipeline(&self) -> PipelineState {
        self.pipeline
    }
    pub fn identity(&self) -> CursorIdentity {
        self.owner
    }
    pub fn bus_effects(&self) -> u32 {
        self.bus_effects
    }
    pub fn phase_updates(&self) -> u32 {
        self.phase_updates
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimingError {
    InvalidIr(&'static str),
    MissingIsrCapability,
    MissingRead(u32),
    PendingFault,
    PendingAcknowledgement,
    WrongToken,
    WrongObservation,
    WrongPath { expected: u32, actual: u32 },
    WrongEntry,
    Exhausted,
    ArithmeticDomain,
}
impl fmt::Display for TimingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "timing region: {self:?}")
    }
}
impl std::error::Error for TimingError {}

#[derive(Debug)]
pub struct EventToken {
    owner: CursorIdentity,
    sequence: u32,
}
#[derive(Debug)]
pub enum TimingEvent {
    Bus {
        token: EventToken,
        expected: ExpectedAccess,
    },
    SetReadOffset {
        token: EventToken,
        master_clocks: u32,
    },
    InstructionComplete {
        token: EventToken,
        cycles: u32,
        pipeline: PipelineState,
    },
    Exception {
        token: EventToken,
        result: StepResult,
    },
}

#[derive(Debug, Clone, Copy)]
struct Expr(u32);
#[derive(Debug, Clone)]
enum Node {
    Constant(u32),
    Entry(StateField),
    Read(u32),
    Binary(BinaryOp, Expr, Expr),
    Compare(CompareOp, Expr, Expr),
    Select(Expr, Expr, Expr),
}
#[derive(Debug, Clone)]
enum Effect {
    Bus {
        condition: Option<Expr>,
        address: Expr,
        width: Width,
        kind: AccessKind,
        read: Option<u32>,
    },
    Offset(Expr),
    Exception {
        condition: Expr,
        vector: u32,
        cycles: Expr,
        saved_pc: Expr,
        fault_address: Expr,
        fault_status: Expr,
    },
    Finish {
        cycles: Expr,
        pc: Expr,
        prefetch: Expr,
        last_muldiv: Expr,
    },
}

/// Compiled timing dependencies only. There is no game StoreState operation,
/// no write payload expression and no callback that executes an effect.
#[derive(Debug, Clone)]
pub struct TimingInstruction {
    pc: u32,
    opcode: u16,
    next_pc: u32,
    nodes: Vec<Node>,
    effects: Vec<Effect>,
    reads: usize,
    prefetch_words: Option<std::sync::Arc<std::collections::BTreeMap<u32, u16>>>,
}
impl TimingInstruction {
    pub fn pc(&self) -> u32 {
        self.pc
    }
    pub fn opcode(&self) -> u16 {
        self.opcode
    }
    pub fn next_pc(&self) -> u32 {
        self.next_pc
    }
    pub fn bind_prefetch_words(
        &mut self,
        words: std::sync::Arc<std::collections::BTreeMap<u32, u16>>,
    ) -> Result<(), TimingError> {
        if words.get(&self.pc) != Some(&self.opcode) {
            return Err(TimingError::WrongEntry);
        }
        self.prefetch_words = Some(words);
        Ok(())
    }
    pub fn compile(ir: &InstructionIr) -> Result<Self, TimingError> {
        let mut nodes = Vec::new();
        let mut values = vec![None; ir.value_count as usize];
        let mut fields: [Option<Expr>; 25] = [None; 25];
        let mut effects = Vec::new();
        let mut reads = 0_u32;
        let value = |v: crate::ir::Value, values: &[Option<Expr>]| {
            values
                .get(v.0 as usize)
                .copied()
                .flatten()
                .ok_or(TimingError::InvalidIr("undefined SSA value"))
        };
        let add = |node: Node, nodes: &mut Vec<Node>| {
            let expr = Expr(nodes.len() as u32);
            nodes.push(node);
            expr
        };
        for op in &ir.ops {
            let definition = match *op {
                Op::Constant { dst, value } => Some((dst, add(Node::Constant(value), &mut nodes))),
                Op::LoadState { dst, field } => {
                    let expr = *fields[field.index() as usize]
                        .get_or_insert_with(|| add(Node::Entry(field), &mut nodes));
                    Some((dst, expr))
                }
                Op::StoreState { field, value: v } => {
                    // Substitution is instruction-local timing derivation only.
                    // It never updates a CPU, returns a game result, or survives
                    // as a hidden architectural register at the next boundary.
                    fields[field.index() as usize] = Some(value(v, &values)?);
                    None
                }
                Op::Binary { dst, op, lhs, rhs } => Some((
                    dst,
                    add(
                        Node::Binary(op, value(lhs, &values)?, value(rhs, &values)?),
                        &mut nodes,
                    ),
                )),
                Op::Compare { dst, op, lhs, rhs } => Some((
                    dst,
                    add(
                        Node::Compare(op, value(lhs, &values)?, value(rhs, &values)?),
                        &mut nodes,
                    ),
                )),
                Op::Select {
                    dst,
                    condition,
                    if_true,
                    if_false,
                } => Some((
                    dst,
                    add(
                        Node::Select(
                            value(condition, &values)?,
                            value(if_true, &values)?,
                            value(if_false, &values)?,
                        ),
                        &mut nodes,
                    ),
                )),
                Op::ReadMemory {
                    dst,
                    address,
                    width,
                    kind,
                } => {
                    effects.push(Effect::Bus {
                        condition: None,
                        address: value(address, &values)?,
                        width,
                        kind: AccessKind::Read(kind),
                        read: Some(reads),
                    });
                    let expr = add(Node::Read(reads), &mut nodes);
                    reads += 1;
                    Some((dst, expr))
                }
                Op::ReadMemoryIf {
                    dst,
                    condition,
                    address,
                    width,
                    kind,
                } => {
                    effects.push(Effect::Bus {
                        condition: Some(value(condition, &values)?),
                        address: value(address, &values)?,
                        width,
                        kind: AccessKind::Read(kind),
                        read: Some(reads),
                    });
                    let expr = add(Node::Read(reads), &mut nodes);
                    reads += 1;
                    Some((dst, expr))
                }
                Op::WriteMemory { address, width, .. } => {
                    effects.push(Effect::Bus {
                        condition: None,
                        address: value(address, &values)?,
                        width,
                        kind: AccessKind::Write,
                        read: None,
                    });
                    None
                }
                Op::SetReadOffset { cycles } => {
                    effects.push(Effect::Offset(value(cycles, &values)?));
                    None
                }
                Op::ExitIf { condition, exit } => {
                    effects.push(Effect::Exception {
                        condition: value(condition, &values)?,
                        vector: exit.vector,
                        cycles: value(exit.cycles, &values)?,
                        saved_pc: value(exit.saved_pc, &values)?,
                        fault_address: value(exit.fault_address, &values)?,
                        fault_status: value(exit.fault_status, &values)?,
                    });
                    None
                }
                Op::Finish { cycles } => {
                    let mut field = |f: StateField| {
                        fields[f.index() as usize]
                            .unwrap_or_else(|| add(Node::Entry(f), &mut nodes))
                    };
                    effects.push(Effect::Finish {
                        cycles: value(cycles, &values)?,
                        pc: field(StateField::Pc),
                        prefetch: field(StateField::Prefetch),
                        last_muldiv: field(StateField::LastMulDiv),
                    });
                    None
                }
            };
            if let Some((dst, expr)) = definition {
                let slot = values
                    .get_mut(dst.0 as usize)
                    .ok_or(TimingError::InvalidIr("SSA destination out of range"))?;
                if slot.replace(expr).is_some() {
                    return Err(TimingError::InvalidIr("SSA value defined twice"));
                }
            }
        }
        if !matches!(effects.last(), Some(Effect::Finish { .. })) {
            return Err(TimingError::InvalidIr("missing instruction boundary"));
        }
        // Remove every arithmetic result which cannot affect timing, an EA,
        // pipeline or exception. This is not the full semantic IR evaluator.
        let mut live = vec![false; nodes.len()];
        fn mark(expr: Expr, nodes: &[Node], live: &mut [bool]) {
            if std::mem::replace(&mut live[expr.0 as usize], true) {
                return;
            }
            match nodes[expr.0 as usize] {
                Node::Binary(_, a, b) | Node::Compare(_, a, b) => {
                    mark(a, nodes, live);
                    mark(b, nodes, live);
                }
                Node::Select(c, a, b) => {
                    mark(c, nodes, live);
                    mark(a, nodes, live);
                    mark(b, nodes, live);
                }
                _ => {}
            }
        }
        for effect in &effects {
            match *effect {
                Effect::Bus {
                    condition, address, ..
                } => {
                    mark(address, &nodes, &mut live);
                    if let Some(c) = condition {
                        mark(c, &nodes, &mut live);
                    }
                }
                Effect::Offset(a) => mark(a, &nodes, &mut live),
                Effect::Exception {
                    condition,
                    cycles,
                    saved_pc,
                    fault_address,
                    fault_status,
                    ..
                } => {
                    for a in [condition, cycles, saved_pc, fault_address, fault_status] {
                        mark(a, &nodes, &mut live);
                    }
                }
                Effect::Finish {
                    cycles,
                    pc,
                    prefetch,
                    last_muldiv,
                } => {
                    for a in [cycles, pc, prefetch, last_muldiv] {
                        mark(a, &nodes, &mut live);
                    }
                }
            }
        }
        for (node, live) in nodes.iter_mut().zip(live) {
            if !live {
                *node = Node::Constant(0);
            }
        }
        Ok(Self {
            pc: ir.pc,
            opcode: ir.opcode,
            next_pc: ir.next_pc,
            nodes,
            effects,
            reads: reads as usize,
            prefetch_words: None,
        })
    }
}

/// Reused storage: construction allocates once for the largest admitted
/// instruction; restarting another instruction does not allocate.
#[derive(Debug)]
pub struct TimingScratch {
    cache: Vec<Option<u32>>,
    reads: Vec<Option<u32>>,
    owner: ScratchIdentity,
    generation: u64,
}
impl Default for TimingScratch {
    fn default() -> Self {
        // One portable identity allocation per scratch/region, never per
        // instruction. The hot cursor generation is ordinary u64 under &mut.
        static NEXT_OWNER: Mutex<u64> = Mutex::new(1);
        let owner = NEXT_OWNER
            .lock()
            .ok()
            .and_then(|mut next| {
                let owner = *next;
                *next = next.checked_add(1)?;
                Some(owner)
            })
            .unwrap_or(0);
        Self {
            cache: Vec::new(),
            reads: Vec::new(),
            owner: ScratchIdentity(owner),
            generation: 0,
        }
    }
}
impl TimingScratch {
    pub fn identity(&self) -> Result<ScratchIdentity, TimingError> {
        if self.owner.0 == 0 {
            Err(TimingError::Exhausted)
        } else {
            Ok(self.owner)
        }
    }
    /// A monotonic brand for a region or cursor using this reusable storage.
    /// It is scalar, never wraps, and performs no allocation or global locking.
    pub fn claim_identity(&mut self) -> Result<CursorIdentity, TimingError> {
        let scratch = self.identity()?;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(TimingError::Exhausted)?;
        Ok(CursorIdentity {
            scratch,
            generation: self.generation,
        })
    }
    pub fn reserve_for<'a>(
        &mut self,
        instructions: impl IntoIterator<Item = &'a TimingInstruction>,
    ) {
        for instruction in instructions {
            self.cache
                .resize(self.cache.len().max(instruction.nodes.len()), None);
            self.reads
                .resize(self.reads.len().max(instruction.reads), None);
        }
    }
}

#[derive(Debug)]
enum Pending {
    Bus {
        expected: ExpectedAccess,
        read: Option<u32>,
    },
    Offset(u32),
    Finish {
        pipeline: PipelineState,
        cycles: u32,
    },
    Exception,
}

pub struct TimingCursor<'a> {
    instruction: &'a TimingInstruction,
    entry: &'a CpuState,
    scratch: &'a mut TimingScratch,
    effect: usize,
    ordinal: u32,
    owner: CursorIdentity,
    sequence: u32,
    phase_updates: u32,
    pending: Option<Pending>,
    finished: bool,
    return_prefetch: Option<(u32, Option<u16>)>,
}
impl<'a> TimingCursor<'a> {
    pub fn begin(
        instruction: &'a TimingInstruction,
        entry: &'a CpuState,
        scratch: &'a mut TimingScratch,
    ) -> Result<Self, TimingError> {
        if entry.address_error != 0 {
            return Err(TimingError::PendingFault);
        }
        if entry.pc != instruction.pc || entry.prefetch != u32::from(instruction.opcode) {
            return Err(TimingError::WrongEntry);
        }
        let owner = scratch.claim_identity()?;
        scratch.cache.resize(instruction.nodes.len(), None);
        scratch.cache.fill(None);
        scratch.reads.resize(instruction.reads, None);
        scratch.reads.fill(None);
        Ok(Self {
            instruction,
            entry,
            scratch,
            effect: 0,
            ordinal: 0,
            owner,
            sequence: 0,
            phase_updates: 0,
            pending: None,
            finished: false,
            return_prefetch: None,
        })
    }
    /// Additional immutable ROM expectation for this invocation's dynamic RTS
    /// destination. It never reads the bus or substitutes for a real prefetch.
    pub fn bind_return_prefetch(
        &mut self,
        address: u32,
        word: Option<u16>,
    ) -> Result<(), TimingError> {
        if self.effect != 0
            || self.pending.is_some()
            || self.finished
            || self.return_prefetch.is_some()
            || address & 1 != 0
        {
            return Err(TimingError::WrongEntry);
        }
        self.return_prefetch = Some((address & 0xffffff, word));
        Ok(())
    }
    fn expected_prefetch(&self, address: u32) -> Option<u16> {
        let address = address & 0xffffff;
        self.instruction
            .prefetch_words
            .as_ref()
            .and_then(|words| words.get(&address).copied())
            .or_else(|| {
                self.return_prefetch
                    .filter(|&(pc, _)| pc == address)
                    .and_then(|(_, word)| word)
            })
    }
    fn permits_prefetch(&self, address: u32) -> bool {
        let address = address & 0xffffff;
        self.instruction
            .prefetch_words
            .as_ref()
            .is_some_and(|words| words.contains_key(&address))
            || self.return_prefetch.is_some_and(|(pc, _)| pc == address)
    }
    pub fn pending_access(&self, token: &EventToken) -> Result<ExpectedAccess, TimingError> {
        self.check_token(token)?;
        match self.pending {
            Some(Pending::Bus { expected, .. }) => Ok(expected),
            _ => Err(TimingError::WrongToken),
        }
    }
    pub fn pending_read_offset(&self, token: &EventToken) -> Result<u32, TimingError> {
        self.check_token(token)?;
        match self.pending {
            Some(Pending::Offset(clocks)) => Ok(clocks),
            _ => Err(TimingError::WrongToken),
        }
    }
    pub fn identity(&self) -> CursorIdentity {
        self.owner
    }
    fn eval(&mut self, expr: Expr) -> Result<u32, TimingError> {
        let index = expr.0 as usize;
        if let Some(v) = self.scratch.cache[index] {
            return Ok(v);
        }
        let value = match self.instruction.nodes[index] {
            Node::Constant(v) => v,
            Node::Entry(field) => state_field(self.entry, field),
            Node::Read(n) => self.scratch.reads[n as usize].ok_or(TimingError::MissingRead(n))?,
            Node::Binary(op, a, b) => {
                let a = self.eval(a)?;
                let b = self.eval(b)?;
                binary(op, a, b)?
            }
            Node::Compare(op, a, b) => {
                let a = self.eval(a)?;
                let b = self.eval(b)?;
                u32::from(match op {
                    CompareOp::Equal => a == b,
                    CompareOp::NotEqual => a != b,
                    CompareOp::UnsignedLess => a < b,
                    CompareOp::UnsignedLessEqual => a <= b,
                    CompareOp::SignedLess => (a as i32) < b as i32,
                    CompareOp::SignedLessEqual => (a as i32) <= b as i32,
                })
            }
            Node::Select(c, a, b) => {
                let c = self.eval(c)?;
                self.eval(if c != 0 { a } else { b })?
            }
        };
        self.scratch.cache[index] = Some(value);
        Ok(value)
    }
    fn token(&mut self) -> Result<EventToken, TimingError> {
        self.sequence = self.sequence.checked_add(1).ok_or(TimingError::Exhausted)?;
        Ok(EventToken {
            owner: self.owner,
            sequence: self.sequence,
        })
    }
    fn check_token(&self, token: &EventToken) -> Result<(), TimingError> {
        if self.finished
            || token.owner != self.owner
            || token.sequence != self.sequence
            || self.pending.is_none()
        {
            Err(TimingError::WrongToken)
        } else {
            Ok(())
        }
    }
    pub fn next_event(&mut self) -> Result<TimingEvent, TimingError> {
        let result = self.next_event_inner();
        if result.is_err() {
            self.finished = true;
        }
        result
    }
    fn next_event_inner(&mut self) -> Result<TimingEvent, TimingError> {
        if self.pending.is_some() {
            return Err(TimingError::PendingAcknowledgement);
        }
        if self.finished {
            return Err(TimingError::Exhausted);
        }
        loop {
            let effect = self
                .instruction
                .effects
                .get(self.effect)
                .ok_or(TimingError::InvalidIr("exhausted without completion"))?
                .clone();
            self.effect += 1;
            match effect {
                Effect::Bus {
                    condition,
                    address,
                    width,
                    kind,
                    read,
                } => {
                    if let Some(condition) = condition
                        && self.eval(condition)? == 0
                    {
                        if let Some(read) = read {
                            self.scratch.reads[read as usize] = Some(0);
                        }
                        continue;
                    }
                    let expected = ExpectedAccess {
                        instruction_pc: self.instruction.pc,
                        ordinal: self.ordinal,
                        address: self.eval(address)?,
                        width,
                        kind,
                        phase: ReferenceBusPhase::Issue,
                    };
                    if kind == AccessKind::Read(ReadKind::Prefetch)
                        && (self.instruction.prefetch_words.is_some()
                            || self.return_prefetch.is_some())
                        && !self.permits_prefetch(expected.address)
                    {
                        return Err(TimingError::InvalidIr(
                            "prefetch target escapes reviewed immutable code",
                        ));
                    }
                    self.ordinal += 1;
                    let token = self.token()?;
                    self.pending = Some(Pending::Bus { expected, read });
                    return Ok(TimingEvent::Bus { token, expected });
                }
                Effect::Offset(expr) => {
                    let master_clocks = self.eval(expr)?;
                    let token = self.token()?;
                    self.pending = Some(Pending::Offset(master_clocks));
                    return Ok(TimingEvent::SetReadOffset {
                        token,
                        master_clocks,
                    });
                }
                Effect::Exception {
                    condition,
                    vector,
                    cycles,
                    saved_pc,
                    fault_address,
                    fault_status,
                } => {
                    if self.eval(condition)? == 0 {
                        continue;
                    }
                    let result = StepResult {
                        cycles: self.eval(cycles)?,
                        vector,
                        saved_pc: self.eval(saved_pc)?,
                        fault_address: self.eval(fault_address)?,
                        fault_status: self.eval(fault_status)?,
                    };
                    let token = self.token()?;
                    self.pending = Some(Pending::Exception);
                    return Ok(TimingEvent::Exception { token, result });
                }
                Effect::Finish {
                    cycles,
                    pc,
                    prefetch,
                    last_muldiv,
                } => {
                    let cycles = self.eval(cycles)?;
                    let pipeline = PipelineState {
                        pc: self.eval(pc)?,
                        prefetch: self.eval(prefetch)?,
                        last_muldiv: self.eval(last_muldiv)?,
                    };
                    let token = self.token()?;
                    self.pending = Some(Pending::Finish { pipeline, cycles });
                    return Ok(TimingEvent::InstructionComplete {
                        token,
                        cycles,
                        pipeline,
                    });
                }
            }
        }
    }
    pub fn acknowledge_bus(
        &mut self,
        token: EventToken,
        observation: BusObservation,
    ) -> Result<(), TimingError> {
        let result = self.acknowledge_bus_inner(token, observation);
        if result.is_err() {
            self.finished = true;
        }
        result
    }
    fn acknowledge_bus_inner(
        &mut self,
        token: EventToken,
        observation: BusObservation,
    ) -> Result<(), TimingError> {
        self.check_token(&token)?;
        let Some(Pending::Bus { expected, read }) = &self.pending else {
            return Err(TimingError::WrongToken);
        };
        if observation.address != expected.address
            || observation.width != expected.width
            || observation.kind != expected.kind
            || observation.value
                > match observation.width {
                    Width::Byte => 255,
                    Width::Word => 65535,
                }
        {
            return Err(TimingError::WrongObservation);
        }
        if expected.kind == AccessKind::Read(ReadKind::Prefetch)
            && (self.instruction.prefetch_words.is_some() || self.return_prefetch.is_some())
            && self
                .expected_prefetch(expected.address)
                .is_some_and(|word| u32::from(word) != observation.value)
        {
            return Err(TimingError::WrongObservation);
        }
        if let Some(read) = read {
            self.scratch.reads[*read as usize] = Some(observation.value);
        }
        self.pending = None;
        Ok(())
    }
    pub fn acknowledge_offset(&mut self, token: EventToken) -> Result<(), TimingError> {
        let result = self.acknowledge_offset_inner(token);
        if result.is_err() {
            self.finished = true;
        }
        result
    }
    fn acknowledge_offset_inner(&mut self, token: EventToken) -> Result<(), TimingError> {
        self.check_token(&token)?;
        if !matches!(self.pending, Some(Pending::Offset(_))) {
            return Err(TimingError::WrongToken);
        }
        self.pending = None;
        self.phase_updates = self
            .phase_updates
            .checked_add(1)
            .ok_or(TimingError::Exhausted)?;
        Ok(())
    }
    pub fn complete_instruction(
        &mut self,
        token: EventToken,
        path: RustPathChoice,
    ) -> Result<InstructionTiming, TimingError> {
        let result = self.complete_instruction_inner(token, path);
        if result.is_err() {
            self.finished = true;
        }
        result
    }
    fn complete_instruction_inner(
        &mut self,
        token: EventToken,
        path: RustPathChoice,
    ) -> Result<InstructionTiming, TimingError> {
        self.check_token(&token)?;
        let Some(Pending::Finish { pipeline, cycles }) = self.pending else {
            return Err(TimingError::WrongToken);
        };
        if path.next_pc != pipeline.pc {
            return Err(TimingError::WrongPath {
                expected: pipeline.pc,
                actual: path.next_pc,
            });
        }
        self.pending = None;
        self.finished = true;
        Ok(InstructionTiming {
            pc: self.instruction.pc,
            opcode: self.instruction.opcode,
            cycles,
            pipeline,
            owner: self.owner,
            bus_effects: self.ordinal,
            phase_updates: self.phase_updates,
        })
    }
    pub fn acknowledge_exception(&mut self, token: EventToken) -> Result<(), TimingError> {
        let result = self.acknowledge_exception_inner(token);
        if result.is_err() {
            self.finished = true;
        }
        result
    }
    fn acknowledge_exception_inner(&mut self, token: EventToken) -> Result<(), TimingError> {
        self.check_token(&token)?;
        if !matches!(self.pending, Some(Pending::Exception)) {
            return Err(TimingError::WrongToken);
        }
        self.pending = None;
        self.finished = true;
        Ok(())
    }
}
fn state_field(cpu: &CpuState, field: StateField) -> u32 {
    match field {
        StateField::Data(n) => cpu.d[usize::from(n)],
        StateField::Address(n) => cpu.a[usize::from(n)],
        StateField::Usp => cpu.usp,
        StateField::Ssp => cpu.ssp,
        StateField::Pc => cpu.pc,
        StateField::Sr => cpu.sr,
        StateField::Prefetch => cpu.prefetch,
        StateField::PendingInterrupt => cpu.pending_interrupt,
        StateField::Stopped => cpu.stopped,
        StateField::Frozen => cpu.frozen,
        StateField::AddressError => cpu.address_error,
        StateField::LastMulDiv => cpu.last_muldiv,
    }
}
fn binary(op: BinaryOp, a: u32, b: u32) -> Result<u32, TimingError> {
    Ok(match op {
        BinaryOp::Add => a.wrapping_add(b),
        BinaryOp::Subtract => a.wrapping_sub(b),
        BinaryOp::Multiply => a.wrapping_mul(b),
        BinaryOp::And => a & b,
        BinaryOp::Or => a | b,
        BinaryOp::Xor => a ^ b,
        BinaryOp::ShiftLeft => a.wrapping_shl(b),
        BinaryOp::ShiftRight => a.wrapping_shr(b),
        BinaryOp::ArithmeticShiftRight => (a as i32).wrapping_shr(b) as u32,
        BinaryOp::DivideUnsigned
        | BinaryOp::DivideSigned
        | BinaryOp::RemainderUnsigned
        | BinaryOp::RemainderSigned
            if b == 0 =>
        {
            return Err(TimingError::ArithmeticDomain);
        }
        BinaryOp::DivideUnsigned => a / b,
        BinaryOp::DivideSigned => (a as i32).wrapping_div(b as i32) as u32,
        BinaryOp::RemainderUnsigned => a % b,
        BinaryOp::RemainderSigned => (a as i32).wrapping_rem(b as i32) as u32,
    })
}
