use super::{
    facts::{Fact, VALUE_BASE},
    *,
};
use crate::ir::{BinaryOp, Op, Value};

#[derive(Clone, Copy, Debug, Default)]
enum Node {
    #[default]
    Empty,
    Constant,
    Entry(StateField),
    Alias(Value),
    Binary(BinaryOp, Value, Value),
    Compare(Value, Value),
    Select(Value, Value, Value),
    Memory,
}
#[derive(Clone, Copy, Debug)]
struct Demand {
    value: Value,
    bits: u32,
    control: bool,
}
#[derive(Clone, Copy, Debug)]
struct ReadBinding {
    value: Value,
    ordinal: usize,
    address: u32,
    width: Width,
    kind: ReadKind,
}
#[derive(Clone, Copy, Debug)]
struct OutputBinding {
    state: [Option<Value>; StateField::COUNT],
    pc: u32,
    opcode: u16,
    path: PathBinding,
    controls: StateMask,
    memory_controls: bool,
}
/// Reusable storage, independent of any game, ROM revision or host context.
#[derive(Debug, Default)]
pub struct DependencyScratch {
    nodes: Vec<Node>,
    definition_order: Vec<Value>,
    facts: Vec<Fact>,
    data_demands: Vec<u32>,
    control_demands: Vec<u32>,
    touched: Vec<usize>,
    work: Vec<Demand>,
    taints: Vec<u32>,
    outputs: Option<OutputBinding>,
    reads_memory: bool,
    memory_controls: bool,
    path_memory_controls: bool,
    executed_values: Vec<bool>,
    current_reachable: bool,
    read_bindings: Vec<ReadBinding>,
    path_memory_demands: Vec<u32>,
}
impl DependencyScratch {
    /// Query the last successful `Executed` analysis using its retained SSA
    /// bindings and observed path. Every actual path control is retained.
    /// Conservative/Entry analyses and non-unique exits fail closed. Reusing
    /// scratch for any analysis invalidates the old binding, including errors.
    pub fn output_reads(
        &mut self,
        field: StateField,
        bits: u32,
    ) -> Result<OutputDependencies, DependencyError> {
        let index = field_index(field)?;
        let value = self
            .outputs
            .as_ref()
            .ok_or(DependencyError::OutputQueryUnavailable)?
            .state[index];
        let (reads, controls, memory) = if let Some(value) = value {
            let (reads, controls) = self.reads(value, bits, false);
            (reads, controls, self.reads_memory)
        } else {
            (StateMask::single(field, bits), StateMask::EMPTY, false)
        };
        let output = self
            .outputs
            .as_ref()
            .expect("read demand retains its successful binding");
        Ok(OutputDependencies {
            pc: output.pc,
            opcode: output.opcode,
            binding: output.path,
            reads: reads | output.controls,
            control_reads: controls | output.controls,
            memory_values: memory || output.memory_controls,
        })
    }

    /// Exact physical memory leaves for the same selected output query as
    /// `output_reads`. All path controls and the old summary are preserved.
    /// The caller's reusable vector is cleared even when the query fails.
    pub fn output_read_demands(
        &mut self,
        field: StateField,
        bits: u32,
        demands: &mut Vec<ExecutedReadDemand>,
    ) -> Result<OutputDependencies, DependencyError> {
        demands.clear();
        let summary = self.output_reads(field, bits)?;
        if self
            .outputs
            .as_ref()
            .expect("successful output binding")
            .state[field.index() as usize]
            .is_none()
        {
            self.clear_demands();
        }
        self.collect_read_demands(demands);
        Ok(summary)
    }

    /// Query any SSA value actually defined in the selected executed prefix.
    /// Allocated/defined values in an unreachable suffix are not query roots.
    /// `control=false` does not remove actual path or value-selection controls.
    /// Read ordinals and address policy belong to the returned Executed binding.
    pub fn value_read_demands(
        &mut self,
        value: Value,
        bits: u32,
        control: bool,
        demands: &mut Vec<ExecutedReadDemand>,
    ) -> Result<OutputDependencies, DependencyError> {
        demands.clear();
        self.outputs
            .as_ref()
            .ok_or(DependencyError::OutputQueryUnavailable)?;
        let index = self.checked(value)?;
        if !self.executed_values[index] {
            return Err(DependencyError::OutputQueryUnavailable);
        }
        let (reads, controls) = self.reads(value, bits, control);
        let output = self
            .outputs
            .as_ref()
            .expect("read demand retains its successful binding");
        let summary = OutputDependencies {
            pc: output.pc,
            opcode: output.opcode,
            binding: output.path,
            reads: reads | output.controls,
            control_reads: controls | output.controls,
            memory_values: self.reads_memory || output.memory_controls,
        };
        self.collect_read_demands(demands);
        Ok(summary)
    }

    fn collect_read_demands(&self, demands: &mut Vec<ExecutedReadDemand>) {
        for binding in &self.read_bindings {
            let index = binding.value.0 as usize;
            let mask = width_mask(binding.width);
            let data_bits = self.data_demands[index] & mask;
            let control_bits =
                (self.control_demands[index] | self.path_memory_demands[index]) & mask;
            if data_bits | control_bits != 0 {
                demands.push(ExecutedReadDemand {
                    ordinal: binding.ordinal,
                    address: binding.address,
                    width: binding.width,
                    kind: binding.kind,
                    data_bits,
                    control_bits,
                });
            }
        }
    }

    fn bind_read(
        &mut self,
        value: Value,
        reads: Option<&[ObservedRead]>,
        ordinal: usize,
        consumed: usize,
    ) {
        if consumed == ordinal {
            return;
        }
        let read = reads.expect("consumed executed read")[ordinal];
        self.read_bindings.push(ReadBinding {
            value,
            ordinal,
            address: read.address,
            width: read.width,
            kind: read.kind,
        });
    }
    pub(super) fn invalidate_outputs(&mut self) {
        self.outputs = None;
    }
    fn prepare(&mut self, count: usize) -> Result<(), DependencyError> {
        if u32::try_from(count)
            .ok()
            .and_then(|n| n.checked_mul(32))
            .and_then(|n| n.checked_add(VALUE_BASE))
            .is_none()
        {
            return Err(DependencyError::InvalidIr(
                "dependency value space overflow",
            ));
        }
        self.nodes.clear();
        self.nodes.resize(count, Node::Empty);
        self.definition_order.clear();
        self.definition_order.reserve(count);
        self.facts.resize(count, Fact::default());
        self.data_demands.clear();
        self.data_demands.resize(count, 0);
        self.control_demands.clear();
        self.control_demands.resize(count, 0);
        self.taints.resize(count, 0);
        self.touched.clear();
        self.work.clear();
        self.path_memory_controls = false;
        self.executed_values.clear();
        self.executed_values.resize(count, false);
        self.current_reachable = false;
        self.read_bindings.clear();
        self.path_memory_demands.clear();
        self.path_memory_demands.resize(count, 0);
        self.touched.reserve(count);
        self.work.reserve(count);
        Ok(())
    }
    fn checked(&self, value: Value) -> Result<usize, DependencyError> {
        let index = value.0 as usize;
        if !self
            .nodes
            .get(index)
            .is_some_and(|node| !matches!(node, Node::Empty))
        {
            return Err(DependencyError::InvalidIr("value used before definition"));
        }
        Ok(index)
    }
    fn fact(&self, value: Value) -> Result<&Fact, DependencyError> {
        Ok(&self.facts[self.checked(value)?])
    }
    fn define(&mut self, value: Value, node: Node, fact: Fact) -> Result<(), DependencyError> {
        let index = value.0 as usize;
        if !self
            .nodes
            .get(index)
            .is_some_and(|node| matches!(node, Node::Empty))
        {
            return Err(DependencyError::InvalidIr(
                "duplicate or out-of-range SSA definition",
            ));
        }
        self.nodes[index] = node;
        self.facts[index] = fact;
        self.executed_values[index] = self.current_reachable;
        self.definition_order.push(value);
        Ok(())
    }
    fn mark(&mut self, value: Value, bits: u32, control: bool) {
        if bits == 0 {
            return;
        }
        let index = value.0 as usize;
        if self.data_demands[index] == 0 && self.control_demands[index] == 0 {
            self.touched.push(index);
        }
        let prior = if control {
            &mut self.control_demands[index]
        } else {
            &mut self.data_demands[index]
        };
        let delta = bits & !*prior;
        if delta != 0 {
            *prior |= bits;
            self.work.push(Demand {
                value,
                bits: delta,
                control,
            });
        }
    }
    fn clear_demands(&mut self) {
        for index in self.touched.drain(..) {
            self.data_demands[index] = 0;
            self.control_demands[index] = 0;
        }
        self.work.clear();
        self.reads_memory = false;
        self.memory_controls = false;
    }
    fn reads(&mut self, value: Value, bits: u32, control: bool) -> (StateMask, StateMask) {
        self.clear_demands();
        self.mark(value, bits, control);
        let mut data = StateMask::EMPTY;
        let mut controls = StateMask::EMPTY;
        while let Some(Demand {
            value,
            bits,
            control,
        }) = self.work.pop()
        {
            match self.nodes[value.0 as usize] {
                Node::Empty => unreachable!("validated SSA demand"),
                Node::Constant => {}
                Node::Memory => {
                    self.reads_memory = true;
                    self.memory_controls |= control;
                }
                Node::Entry(field) => {
                    if control {
                        controls.insert(field, bits)
                    } else {
                        data.insert(field, bits)
                    }
                }
                Node::Alias(value) => self.mark(value, bits, control),
                Node::Compare(lhs, rhs) => {
                    if bits & 1 != 0 {
                        self.mark(lhs, u32::MAX, control);
                        self.mark(rhs, u32::MAX, control);
                    }
                }
                Node::Select(condition, yes, no) => {
                    if yes == no {
                        self.mark(yes, bits, control);
                        continue;
                    }
                    let fact = &self.facts[condition.0 as usize];
                    let constant = fact.constant_value();
                    let observed = fact.observed.or(constant);
                    if constant.is_none() {
                        self.mark(condition, u32::MAX, true);
                    }
                    match observed {
                        Some(0) => self.mark(no, bits, control),
                        Some(_) => self.mark(yes, bits, control),
                        None => {
                            self.mark(yes, bits, control);
                            self.mark(no, bits, control);
                        }
                    }
                }
                Node::Binary(op, lhs, rhs) => {
                    let (az, ao, bz, bo, observed) = {
                        let a = &self.facts[lhs.0 as usize];
                        let b = &self.facts[rhs.0 as usize];
                        (
                            a.zeros,
                            a.ones,
                            b.zeros,
                            b.ones,
                            b.observed.or(b.constant_value()),
                        )
                    };
                    match op {
                        BinaryOp::And => {
                            self.mark(lhs, bits & !bz, control);
                            self.mark(rhs, bits & !az, control);
                        }
                        BinaryOp::Or => {
                            self.mark(lhs, bits & !bo, control);
                            self.mark(rhs, bits & !ao, control);
                        }
                        BinaryOp::Xor if lhs == rhs => {}
                        BinaryOp::Xor => {
                            self.mark(lhs, bits, control);
                            self.mark(rhs, bits, control);
                        }
                        BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply => {
                            let low = low_through_highest(bits);
                            self.mark(lhs, low, control);
                            self.mark(rhs, low, control);
                        }
                        BinaryOp::ShiftLeft
                        | BinaryOp::ShiftRight
                        | BinaryOp::ArithmeticShiftRight => {
                            self.mark(rhs, 31, true);
                            let wanted = match observed.map(|count| count & 31) {
                                Some(count) => match op {
                                    BinaryOp::ShiftLeft => bits.wrapping_shr(count),
                                    BinaryOp::ShiftRight => bits.wrapping_shl(count),
                                    BinaryOp::ArithmeticShiftRight => {
                                        bits.wrapping_shl(count)
                                            | if bits & !(u32::MAX.wrapping_shr(count)) != 0 {
                                                1 << 31
                                            } else {
                                                0
                                            }
                                    }
                                    _ => unreachable!(),
                                },
                                None => u32::MAX,
                            };
                            self.mark(lhs, wanted, control);
                        }
                        BinaryOp::DivideUnsigned
                        | BinaryOp::DivideSigned
                        | BinaryOp::RemainderUnsigned
                        | BinaryOp::RemainderSigned => {
                            self.mark(lhs, u32::MAX, control);
                            self.mark(rhs, u32::MAX, control);
                        }
                    }
                }
            }
        }
        (data | controls, controls)
    }
    fn use_(
        &mut self,
        report: &mut DependencyReport,
        operation: usize,
        kind: DependencyUseKind,
        value: Value,
        bits: u32,
        control: bool,
    ) {
        if bits == 0 {
            return;
        }
        let (reads, control_reads) = self.reads(value, bits, control);
        self.path_memory_controls |= self.memory_controls;
        for binding in &self.read_bindings {
            let index = binding.value.0 as usize;
            self.path_memory_demands[index] |= self.control_demands[index];
        }
        report.instruction.reads |= reads;
        report.instruction.effect_reads |= control_reads;
        if !matches!(kind, DependencyUseKind::PreservedState { .. }) {
            report.instruction.effect_reads |= reads;
        }
        report.uses.push(DependencyUse {
            operation,
            kind,
            reads,
            control_reads,
        });
    }
    /// Sufficient new-definition proof: bits not tainted by this field's entry
    /// value. Guards/counts remain dependencies on an observed path as well.
    fn independent_bits(&mut self, value: Value, field: StateField) -> u32 {
        for &value in &self.definition_order {
            let index = value.0 as usize;
            let get = |value: Value| self.taints[value.0 as usize];
            self.taints[index] = match self.nodes[index] {
                Node::Empty | Node::Constant | Node::Memory => 0,
                Node::Entry(source) => {
                    if source == field {
                        u32::MAX
                    } else {
                        0
                    }
                }
                Node::Alias(value) => get(value),
                Node::Compare(lhs, rhs) => u32::from(get(lhs) | get(rhs) != 0),
                Node::Select(condition, yes, no) => {
                    if yes == no {
                        get(yes)
                    } else {
                        let branches = match self.facts[condition.0 as usize].observed {
                            Some(0) => get(no),
                            Some(_) => get(yes),
                            None => get(yes) | get(no),
                        };
                        branches | if get(condition) != 0 { u32::MAX } else { 0 }
                    }
                }
                Node::Binary(op, lhs, rhs) => {
                    let a = get(lhs);
                    let b = get(rhs);
                    let af = &self.facts[lhs.0 as usize];
                    let bf = &self.facts[rhs.0 as usize];
                    match op {
                        BinaryOp::And => (a & !bf.zeros) | (b & !af.zeros),
                        BinaryOp::Or => (a & !bf.ones) | (b & !af.ones),
                        BinaryOp::Xor if lhs == rhs => 0,
                        BinaryOp::Xor => a | b,
                        BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply => {
                            if a | b == 0 {
                                0
                            } else {
                                u32::MAX.wrapping_shl((a | b).trailing_zeros())
                            }
                        }
                        BinaryOp::ShiftLeft
                        | BinaryOp::ShiftRight
                        | BinaryOp::ArithmeticShiftRight => {
                            if b & 31 != 0 {
                                u32::MAX
                            } else {
                                match bf.observed {
                                    Some(count) => match op {
                                        BinaryOp::ShiftLeft => a.wrapping_shl(count),
                                        BinaryOp::ShiftRight => a.wrapping_shr(count),
                                        BinaryOp::ArithmeticShiftRight => {
                                            (a as i32).wrapping_shr(count) as u32
                                        }
                                        _ => unreachable!(),
                                    },
                                    None => {
                                        if a == 0 {
                                            0
                                        } else {
                                            u32::MAX
                                        }
                                    }
                                }
                            }
                        }
                        BinaryOp::DivideUnsigned
                        | BinaryOp::DivideSigned
                        | BinaryOp::RemainderUnsigned
                        | BinaryOp::RemainderSigned => {
                            if a | b == 0 {
                                0
                            } else {
                                u32::MAX
                            }
                        }
                    }
                }
            };
        }
        !self.taints[value.0 as usize]
    }
}
fn low_through_highest(mask: u32) -> u32 {
    if mask == 0 {
        0
    } else {
        u32::MAX.wrapping_shr(mask.leading_zeros())
    }
}
fn field_index(field: StateField) -> Result<usize, DependencyError> {
    if matches!(field, StateField::Data(n) if n >= 8)
        || matches!(field, StateField::Address(n) if n >= 7)
    {
        return Err(DependencyError::InvalidIr("invalid canonical state field"));
    }
    Ok(field.index() as usize)
}
fn initial(field: StateField, before: Option<&CpuState>) -> Fact {
    Fact::entry(field, before.map(|state| field.read(state)))
}
fn current(
    field: StateField,
    state: &[Option<Value>; StateField::COUNT],
    scratch: &DependencyScratch,
    before: Option<&CpuState>,
) -> Fact {
    state[field.index() as usize].map_or_else(
        || initial(field, before),
        |value| scratch.facts[value.0 as usize],
    )
}
fn exit_masks(
    state: &[Option<Value>; StateField::COUNT],
    scratch: &mut DependencyScratch,
    before: Option<&CpuState>,
    physical: StateMask,
) -> (StateMask, StateMask) {
    let mut defined = StateMask::EMPTY;
    let mut preserved = StateMask::EMPTY;
    for index in 0..StateField::COUNT {
        let field = StateField::from_index(index).expect("canonical state index");
        let unchanged = current(field, state, scratch, before).initial_bits(field);
        if let Some(value) = state[index]
            && physical.get(field) != 0
        {
            defined.insert(field, scratch.independent_bits(value, field) & !unchanged);
        }
        preserved.insert(field, unchanged);
    }
    (defined, preserved)
}

pub(super) fn analyze(
    instruction: &InstructionIr,
    input: DependencyInput<'_>,
    scratch: &mut DependencyScratch,
    report: &mut DependencyReport,
) -> Result<(), DependencyError> {
    scratch.invalidate_outputs();
    report.instruction = InstructionDependencies::default();
    report.uses.clear();
    let result = analyze_inner(instruction, input, scratch, report);
    if result.is_err() {
        report.instruction = InstructionDependencies::default();
        report.uses.clear();
        scratch.invalidate_outputs();
    }
    result
}
fn analyze_inner(
    instruction: &InstructionIr,
    input: DependencyInput<'_>,
    scratch: &mut DependencyScratch,
    report: &mut DependencyReport,
) -> Result<(), DependencyError> {
    let (before, observations, addresses) = match input {
        DependencyInput::Conservative => (None, None, ObservedAddressDomain::Raw32),
        DependencyInput::Entry(before) => (Some(before), None, ObservedAddressDomain::Raw32),
        DependencyInput::Executed {
            before,
            reads,
            addresses,
        } => (Some(before), Some(reads), addresses),
    };
    if before.is_some_and(|state| {
        state.pc != instruction.pc || state.prefetch != u32::from(instruction.opcode)
    }) {
        return Err(DependencyError::EntryMismatch);
    }
    if !matches!(instruction.ops.last(), Some(Op::Finish { .. })) {
        return Err(DependencyError::InvalidIr(
            "instruction lacks final boundary",
        ));
    }
    scratch.prepare(instruction.value_count as usize)?;
    let mut state = [None; StateField::COUNT];
    let mut output_state = None;
    let mut reads_consumed = 0;
    let mut reachable = true;
    let mut exits = 0;
    let mut definite = StateMask::ALL;
    let mut preserved = StateMask::ALL;
    for (operation, op) in instruction.ops.iter().enumerate() {
        scratch.current_reachable = reachable;
        // Validate the entire SSA, even an unreachable suffix after a constant
        // exception. Unreachable arithmetic is not evaluated or granted effects.
        let check = |value| scratch.checked(value).map(|_| ());
        match *op {
            Op::Constant { dst, value } => {
                scratch.define(dst, Node::Constant, Fact::constant(value))?
            }
            Op::LoadState { dst, field } => {
                let index = field_index(field)?;
                let fact = current(field, &state, scratch, before);
                let node = state[index].map_or(Node::Entry(field), Node::Alias);
                scratch.define(dst, node, fact)?;
            }
            Op::Binary { dst, op, lhs, rhs } => {
                check(lhs)?;
                check(rhs)?;
                if reachable && rhs_value_zero_divisor(op, scratch.fact(rhs)?) {
                    return Err(DependencyError::ArithmeticDomain);
                }
                let fact = if reachable {
                    Fact::binary(dst, op, scratch.fact(lhs)?, scratch.fact(rhs)?)
                } else {
                    Fact::value(dst)
                };
                scratch.define(dst, Node::Binary(op, lhs, rhs), fact)?;
            }
            Op::Compare { dst, op, lhs, rhs } => {
                check(lhs)?;
                check(rhs)?;
                let fact = Fact::compare(dst, op, scratch.fact(lhs)?, scratch.fact(rhs)?);
                scratch.define(dst, Node::Compare(lhs, rhs), fact)?;
            }
            Op::Select {
                dst,
                condition,
                if_true,
                if_false,
            } => {
                check(condition)?;
                check(if_true)?;
                check(if_false)?;
                let fact = Fact::select(
                    dst,
                    scratch.fact(condition)?,
                    scratch.fact(if_true)?,
                    scratch.fact(if_false)?,
                );
                scratch.define(dst, Node::Select(condition, if_true, if_false), fact)?;
            }
            Op::ReadMemory {
                dst,
                address,
                width,
                kind,
            } => {
                check(address)?;
                let ordinal = reads_consumed;
                let observed = if reachable {
                    scratch.use_(
                        report,
                        operation,
                        DependencyUseKind::MemoryReadAddress { width, kind },
                        address,
                        u32::MAX,
                        false,
                    );
                    read_observation(
                        observations,
                        addresses,
                        &mut reads_consumed,
                        scratch.fact(address)?.observed,
                        width,
                        kind,
                        report,
                    )?
                } else {
                    None
                };
                scratch.bind_read(dst, observations, ordinal, reads_consumed);
                scratch.define(
                    dst,
                    Node::Memory,
                    Fact::memory(dst, width_mask(width), observed),
                )?;
            }
            Op::ReadMemoryIf {
                dst,
                condition,
                address,
                width,
                kind,
            } => {
                check(condition)?;
                check(address)?;
                let fact = scratch.fact(condition)?;
                let condition_value = fact.observed.or(fact.constant_value());
                let ordinal = reads_consumed;
                let observed = if reachable {
                    scratch.use_(
                        report,
                        operation,
                        DependencyUseKind::ConditionalReadGuard,
                        condition,
                        u32::MAX,
                        true,
                    );
                    if condition_value == Some(0) {
                        Some(0)
                    } else {
                        if observations.is_some() && condition_value.is_none() {
                            return Err(DependencyError::UnresolvedExecutedRead(reads_consumed));
                        }
                        scratch.use_(
                            report,
                            operation,
                            DependencyUseKind::MemoryReadAddress { width, kind },
                            address,
                            u32::MAX,
                            false,
                        );
                        read_observation(
                            observations,
                            addresses,
                            &mut reads_consumed,
                            scratch.fact(address)?.observed,
                            width,
                            kind,
                            report,
                        )?
                    }
                } else {
                    None
                };
                let fact = if reachable && condition_value == Some(0) {
                    Fact::constant(0)
                } else {
                    Fact::memory(dst, width_mask(width), observed)
                };
                scratch.bind_read(dst, observations, ordinal, reads_consumed);
                scratch.define(dst, Node::Memory, fact)?;
            }
            Op::StoreState { field, value } => {
                let index = field_index(field)?;
                check(value)?;
                if reachable {
                    let old = current(field, &state, scratch, before);
                    let unchanged = scratch.fact(value)?.same_bits(&old);
                    report.instruction.physical_writes.insert(field, u32::MAX);
                    report.instruction.possible_writes.insert(field, !unchanged);
                    if matches!(field, StateField::Stopped | StateField::Frozen)
                        || field == StateField::Sr && !unchanged & !31 != 0
                    {
                        report.instruction.obligations.halt_or_status_control_write = true;
                    }
                    scratch.use_(
                        report,
                        operation,
                        DependencyUseKind::StateWrite { field },
                        value,
                        !unchanged,
                        false,
                    );
                    scratch.use_(
                        report,
                        operation,
                        DependencyUseKind::PreservedState { field },
                        value,
                        unchanged,
                        false,
                    );
                    state[index] = Some(value);
                }
            }
            Op::WriteMemory {
                address,
                value,
                width,
            } => {
                check(address)?;
                check(value)?;
                if reachable {
                    scratch.use_(
                        report,
                        operation,
                        DependencyUseKind::MemoryWriteAddress { width },
                        address,
                        u32::MAX,
                        false,
                    );
                    scratch.use_(
                        report,
                        operation,
                        DependencyUseKind::MemoryWriteValue { width },
                        value,
                        width_mask(width),
                        false,
                    );
                }
            }
            Op::SetReadOffset { cycles } => {
                check(cycles)?;
                if reachable {
                    scratch.use_(
                        report,
                        operation,
                        DependencyUseKind::ReadOffset,
                        cycles,
                        u32::MAX,
                        false,
                    );
                }
            }
            Op::ExitIf { condition, exit } => {
                for value in [
                    condition,
                    exit.cycles,
                    exit.saved_pc,
                    exit.fault_address,
                    exit.fault_status,
                ] {
                    check(value)?;
                }
                if reachable {
                    scratch.use_(
                        report,
                        operation,
                        DependencyUseKind::ExceptionGuard {
                            vector: exit.vector,
                        },
                        condition,
                        u32::MAX,
                        true,
                    );
                    let fact = scratch.fact(condition)?;
                    let take = fact.observed.or(fact.constant_value());
                    if take != Some(0) {
                        report.instruction.obligations.exception_entry_outside_ir = true;
                        for value in [
                            exit.cycles,
                            exit.saved_pc,
                            exit.fault_address,
                            exit.fault_status,
                        ] {
                            scratch.use_(
                                report,
                                operation,
                                DependencyUseKind::ExceptionPayload {
                                    vector: exit.vector,
                                },
                                value,
                                u32::MAX,
                                false,
                            );
                        }
                        let (here_defined, here_preserved) =
                            exit_masks(&state, scratch, before, report.instruction.physical_writes);
                        definite = definite & here_defined;
                        preserved = preserved & here_preserved;
                        if observations.is_some() {
                            output_state = if exits == 0 { Some(state) } else { None };
                        }
                        exits += 1;
                    }
                    if take.is_some_and(|value| value != 0) {
                        report.instruction.obligations.exception_taken = Some(exit.vector);
                        reachable = false;
                    }
                }
            }
            Op::Finish { cycles } => {
                check(cycles)?;
                if operation + 1 != instruction.ops.len() {
                    return Err(DependencyError::InvalidIr("nonfinal normal boundary"));
                }
                if reachable {
                    scratch.use_(
                        report,
                        operation,
                        DependencyUseKind::BoundaryTiming,
                        cycles,
                        u32::MAX,
                        false,
                    );
                    report.instruction.obligations.interrupt_or_trace_boundary = true;
                    let (here_defined, here_preserved) =
                        exit_masks(&state, scratch, before, report.instruction.physical_writes);
                    definite = definite & here_defined;
                    preserved = preserved & here_preserved;
                    if observations.is_some() {
                        output_state = if exits == 0 { Some(state) } else { None };
                    }
                    exits += 1;
                }
            }
        }
    }
    if scratch.nodes.iter().any(|node| matches!(node, Node::Empty)) || exits == 0 {
        return Err(DependencyError::InvalidIr(
            "incomplete SSA or no possible exit",
        ));
    }
    if let Some(observations) = observations
        && observations.len() != reads_consumed
    {
        return Err(DependencyError::ExtraReads {
            consumed: reads_consumed,
            supplied: observations.len(),
        });
    }
    report.instruction.definite_writes = definite;
    report.instruction.preserved = preserved & report.instruction.physical_writes;
    report.instruction.binding = match input {
        DependencyInput::Conservative => PathBinding::Conservative,
        DependencyInput::Entry(_) => PathBinding::EntryState,
        DependencyInput::Executed { .. } => PathBinding::ExecutedReads {
            consumed: reads_consumed,
            addresses,
        },
    };
    if matches!(input, DependencyInput::Executed { .. })
        && exits == 1
        && let Some(state) = output_state
    {
        scratch.outputs = Some(OutputBinding {
            state,
            pc: instruction.pc,
            opcode: instruction.opcode,
            path: report.instruction.binding,
            controls: report
                .uses
                .iter()
                .fold(StateMask::EMPTY, |mask, use_| mask | use_.control_reads),
            memory_controls: scratch.path_memory_controls,
        });
    }
    Ok(())
}
fn rhs_value_zero_divisor(op: BinaryOp, rhs: &Fact) -> bool {
    matches!(
        op,
        BinaryOp::DivideUnsigned
            | BinaryOp::DivideSigned
            | BinaryOp::RemainderUnsigned
            | BinaryOp::RemainderSigned
    ) && rhs.observed == Some(0)
}
fn width_mask(width: Width) -> u32 {
    match width {
        Width::Byte => 255,
        Width::Word => 65535,
    }
}
fn read_observation(
    observations: Option<&[ObservedRead]>,
    addresses: ObservedAddressDomain,
    consumed: &mut usize,
    address: Option<u32>,
    width: Width,
    kind: ReadKind,
    report: &mut DependencyReport,
) -> Result<Option<u32>, DependencyError> {
    let Some(observations) = observations else {
        report.instruction.obligations.opaque_memory_values = true;
        return Ok(None);
    };
    let index = *consumed;
    let observed = observations
        .get(index)
        .ok_or(DependencyError::MissingRead(index))?;
    let address = address.ok_or(DependencyError::UnresolvedExecutedRead(index))?;
    if !addresses.matches(address, observed.address)
        || observed.width != width
        || observed.kind != kind
        || observed.value & !width_mask(width) != 0
    {
        return Err(DependencyError::ReadMismatch(index));
    }
    *consumed += 1;
    Ok(Some(observed.value))
}
