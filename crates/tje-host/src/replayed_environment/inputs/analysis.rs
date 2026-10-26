//! Consumed callee/ISR inputs on the actual instruction and physical-bus path.
//! Private spills carry bit origins; they do not eagerly demand a whole read.
//! Every nonprivate semantic RAM read remains a full-width input obligation.
#[cfg(test)]
use super::InputDemandSite;
use super::{
    Access, Chronology, CompilerCache, DependencyInput, DependencyReport, DependencyScratch,
    DependencyUseKind, Direction, Event, Failure, Invocation, ObservedAddressDomain, ObservedRead,
    ReadKind, StateField, StateMask, Unsupported, UnsupportedKind, Width, analyze_instruction,
    canonical_ram_address,
    exception_frame::ExceptionFrame,
    kind,
    origins::{Origin, Origins},
    private, stack, unsupported,
};
use tje_engine_core::dependencies::{DependencyUse, ExecutedReadDemand, OutputDependencies};
use tje_engine_core::ir::{Op, Value};

#[derive(Clone, Copy)]
struct ReadSlot {
    issue: usize,
    occurrence: usize,
    access: Access,
    mandatory: bool,
}
struct Store {
    issue: usize,
    access: Access,
    sources: [Origin; 16],
    committed: bool,
}

pub(super) struct Analyzer {
    pub origins: Origins,
    pub required: Origin,
    scratch: DependencyScratch,
    report: DependencyReport,
    reads: Vec<ObservedRead>,
    slots: Vec<ReadSlot>,
    leaves: Vec<ExecutedReadDemand>,
    pending: Vec<Store>,
    changes: Box<[[Origin; 32]; StateField::COUNT]>,
    initial_stack: StateField,
    root: super::super::record::InvocationId,
    return_sp: u32,
    pub return_slot_as_data: bool,
    exception_frames: Vec<ExceptionFrame>,
    #[cfg(test)]
    pub demands: Option<Vec<InputDemandSite>>,
    #[cfg(test)]
    sites: Vec<(DependencyUseKind, Origin, Origin)>,
}
impl Analyzer {
    pub fn new(invocation: &Invocation) -> Self {
        Self {
            origins: Origins::new(),
            required: Origin::EMPTY,
            scratch: DependencyScratch::default(),
            report: DependencyReport::default(),
            reads: Vec::new(),
            slots: Vec::new(),
            leaves: Vec::new(),
            pending: Vec::new(),
            changes: Box::new([[Origin::EMPTY; 32]; StateField::COUNT]),
            root: invocation.entry.identity,
            return_sp: canonical_ram_address(invocation.entry.boundary.cpu.sp()),
            return_slot_as_data: false,
            exception_frames: Vec::new(),
            initial_stack: if invocation.entry.boundary.cpu.sr() & 0x2000 != 0 {
                StateField::Ssp
            } else {
                StateField::Usp
            },
            #[cfg(test)]
            demands: None,
            #[cfg(test)]
            sites: Vec::new(),
        }
    }
    pub fn require(&mut self, source: Origin) {
        self.required = self.origins.union(self.required, source);
    }
    fn observes_return_slot(&self, access: Access) -> bool {
        access.ram()
            && access
                .bytes()
                .any(|(address, _)| canonical_ram_address(address).wrapping_sub(self.return_sp) < 4)
    }
    fn begin_reads(
        &mut self,
        owner: &Invocation,
        range: std::ops::Range<usize>,
        instruction: Option<&super::super::record::Instruction>,
    ) -> Result<(), Failure> {
        self.reads.clear();
        self.slots.clear();
        for issue in range {
            let access = owner.trace.issues[issue];
            if let Direction::Read(purpose) = access.direction {
                if purpose == m68000_emu::ReadPurpose::Semantic
                    && self.observes_return_slot(access)
                    && !(owner.entry.identity == self.root
                        && instruction.is_some_and(|instruction| {
                            instruction.actual.opcode == 0x4e75
                                && instruction.actual.completed_successfully
                        }))
                {
                    self.return_slot_as_data = true;
                }
                let driven =
                    *owner.trace.driven_masks.get(issue).ok_or_else(|| {
                        unsupported("actual read lost its bound driven-mask record")
                    })?;
                let occurrence = self.origins.new_read(access, driven)?;
                self.slots.push(ReadSlot {
                    issue,
                    occurrence,
                    access,
                    mandatory: access.ram()
                        && purpose == m68000_emu::ReadPurpose::Semantic
                        && instruction.is_none_or(|instruction| {
                            !private(owner.entry.boundary.cpu.sp(), instruction, access)
                        })
                        && !instruction.is_some_and(|instruction| {
                            self.exception_frames
                                .last()
                                .is_some_and(|frame| frame.transports(owner, instruction, access))
                        }),
                });
                self.reads.push(ObservedRead {
                    address: access.address,
                    width: if access.word {
                        Width::Word
                    } else {
                        Width::Byte
                    },
                    kind: kind(purpose),
                    value: u32::from(access.value),
                });
            }
        }
        Ok(())
    }
    fn source(
        &mut self,
        output: OutputDependencies,
        controls_only: bool,
        private_address: bool,
    ) -> Result<Origin, Failure> {
        let controls = self.origins.registers(output.control_reads);
        let mut data = if controls_only {
            Origin::EMPTY
        } else {
            self.origins.registers(output.reads - output.control_reads)
        };
        if private_address {
            // Only initial stack-base data is private transport. A value that
            // the stack acquired from another register/RAM remains an input;
            // control sources are outside this data-only filter.
            data = self.origins.stack_transport(data, self.initial_stack)?;
        }
        let mut source = self.origins.union(data, controls);
        for leaf in &self.leaves {
            let slot = self.slots.get(leaf.ordinal).ok_or_else(|| {
                unsupported("SSA leaf has no exact supplied physical read ordinal")
            })?;
            let expected_width = if slot.access.word {
                Width::Word
            } else {
                Width::Byte
            };
            let Direction::Read(purpose) = slot.access.direction else {
                return Err(unsupported("SSA read leaf points to a store"));
            };
            if (leaf.address, leaf.width, leaf.kind)
                != (slot.access.address, expected_width, kind(purpose))
            {
                return Err(unsupported(
                    "SSA leaf changed its raw read identity/address/width/purpose",
                ));
            }
            let bits = if controls_only {
                leaf.control_bits
            } else {
                leaf.data_bits | leaf.control_bits
            };
            let bits = u16::try_from(bits)
                .map_err(|_| unsupported("SSA leaf exceeds the physical Genesis read width"))?;
            let read = self.origins.read_source(slot.occurrence, bits)?;
            source = self.origins.union(source, read);
        }
        Ok(source)
    }
    fn value_source(
        &mut self,
        value: Value,
        bits: u32,
        control: bool,
        controls_only: bool,
        private_address: bool,
    ) -> Result<Origin, Failure> {
        let output = self
            .scratch
            .value_read_demands(value, bits, control, &mut self.leaves)
            .map_err(|error| unsupported(format!("bound SSA value/read dependency: {error}")))?;
        self.source(output, controls_only, private_address)
    }
    fn output_source(
        &mut self,
        field: StateField,
        bits: u32,
        private_base: bool,
    ) -> Result<Origin, Failure> {
        let output = self
            .scratch
            .output_read_demands(field, bits, &mut self.leaves)
            .map_err(|error| unsupported(format!("bound SSA output/read dependency: {error}")))?;
        self.source(output, false, private_base)
    }
    fn operation_source(
        &mut self,
        op: &Op,
        use_: DependencyUse,
        controls_only: bool,
        private_address: bool,
    ) -> Result<Origin, Failure> {
        let (value, bits, control) = match (op, use_.kind) {
            (
                Op::ReadMemory { address, .. } | Op::ReadMemoryIf { address, .. },
                DependencyUseKind::MemoryReadAddress { .. },
            ) => (*address, u32::MAX, false),
            (Op::ReadMemoryIf { condition, .. }, DependencyUseKind::ConditionalReadGuard) => {
                (*condition, u32::MAX, true)
            }
            (Op::WriteMemory { address, .. }, DependencyUseKind::MemoryWriteAddress { .. }) => {
                (*address, u32::MAX, false)
            }
            (Op::WriteMemory { value, width, .. }, DependencyUseKind::MemoryWriteValue { .. }) => {
                (*value, width_bits(*width), false)
            }
            (
                Op::StoreState { value, .. },
                DependencyUseKind::StateWrite { .. } | DependencyUseKind::PreservedState { .. },
            ) => (*value, u32::MAX, false),
            (Op::ExitIf { condition, .. }, DependencyUseKind::ExceptionGuard { .. }) => {
                (*condition, u32::MAX, true)
            }
            (Op::SetReadOffset { cycles }, DependencyUseKind::ReadOffset)
            | (Op::Finish { cycles }, DependencyUseKind::BoundaryTiming) => {
                (*cycles, u32::MAX, false)
            }
            _ => {
                return Err(unsupported(
                    "dependency root does not match its actual SSA operation",
                ));
            }
        };
        self.value_source(value, bits, control, controls_only, private_address)
    }
    fn instruction(
        &mut self,
        owner: &Invocation,
        index: usize,
        reader: &dyn Fn(u32) -> Option<u16>,
        divider: u32,
        cache: &mut CompilerCache,
        interrupt: bool,
    ) -> Result<(), Failure> {
        let instruction = &owner.trace.instructions[index];
        if !instruction.actual.completed_successfully {
            return Err(unsupported(
                "faulted opcode cannot supply ordinary callee inputs",
            ));
        }
        if instruction.actual.opcode == 0x4e73 {
            self.exception_frames
                .last_mut()
                .ok_or_else(|| {
                    Failure::Unsupported(Unsupported {
                        kind: UnsupportedKind::TypedControlProfile,
                        detail: "RTE has no actual accepted own hardware frame".into(),
                    })
                })?
                .begin_return(owner, index, instruction)?;
        }
        self.begin_reads(owner, instruction.issues.clone(), Some(instruction))?;
        self.pending.clear();
        #[cfg(test)]
        self.sites.clear();
        let issues = &owner.trace.issues[instruction.issues.clone()];
        let private_traffic = issues
            .iter()
            .any(|&access| private(owner.entry.boundary.cpu.sp(), instruction, access));
        let before = instruction.before.cpu.engine();
        let ir = cache.instruction(reader, instruction.actual.pc, divider)?;
        if ir.opcode != instruction.actual.opcode {
            return Err(unsupported(
                "immutable ROM opcode and actual original receipt differ",
            ));
        }
        analyze_instruction(
            ir,
            DependencyInput::Executed {
                before: &before,
                reads: &self.reads,
                addresses: ObservedAddressDomain::M68000Bus24,
            },
            &mut self.scratch,
            &mut self.report,
        )
        .map_err(|error| {
            unsupported(format!(
                "actual read binding at {:06x}: {error:?}",
                instruction.actual.pc
            ))
        })?;
        if self
            .report
            .instruction
            .obligations
            .exception_taken
            .is_some()
            || self.report.instruction.obligations.opaque_memory_values
        {
            return Err(unsupported(
                "callee path has an unresolved memory/exception dependency",
            ));
        }
        if self
            .report
            .instruction
            .obligations
            .halt_or_status_control_write
            && !(interrupt && instruction.actual.opcode == 0x4e73)
            || instruction.actual.opcode & 0xfff0 == 0x4e60
            || matches!(instruction.actual.opcode, 0x4e70 | 0x4e72)
        {
            return Err(Failure::Unsupported(Unsupported { kind: UnsupportedKind::TypedControlProfile, detail: "callee has privileged/status/stop/reset effects needing its explicit control profile".into() }));
        }
        let mut read_ordinal = 0;
        let mut address_issue = 0;
        let mut value_issue = 0;
        for use_index in 0..self.report.uses.len() {
            let use_ = self.report.uses[use_index];
            let op = ir
                .ops
                .get(use_.operation)
                .ok_or_else(|| unsupported("dependency root has no SSA operation"))?;
            #[cfg(test)]
            let previous = self.required;
            let controls = self.operation_source(op, use_, true, false)?;
            self.require(controls);
            match use_.kind {
                DependencyUseKind::MemoryReadAddress {
                    kind: ReadKind::Prefetch,
                    ..
                } => {
                    read_ordinal += 1;
                }
                DependencyUseKind::MemoryReadAddress {
                    width,
                    kind: read_kind,
                } => {
                    let slot = *self
                        .slots
                        .get(read_ordinal)
                        .ok_or_else(|| unsupported("IR read root lacks its physical occurrence"))?;
                    read_ordinal += 1;
                    let Direction::Read(purpose) = slot.access.direction else {
                        return Err(unsupported("read root selected a write"));
                    };
                    if width
                        != if slot.access.word {
                            Width::Word
                        } else {
                            Width::Byte
                        }
                        || kind(purpose) != read_kind
                    {
                        return Err(unsupported(
                            "IR read root differs from its physical width/purpose",
                        ));
                    }
                    let source = self.operation_source(op, use_, false, stack(slot.access))?;
                    self.require(source);
                }
                DependencyUseKind::MemoryWriteAddress { .. } => {
                    let (index, access) = next_write(issues, address_issue)
                        .ok_or_else(|| unsupported("store address has no actual issue"))?;
                    address_issue = index + 1;
                    let source = self.operation_source(op, use_, false, stack(access))?;
                    self.require(source);
                }
                DependencyUseKind::MemoryWriteValue { width } => {
                    let (index, access) = next_write(issues, value_issue)
                        .ok_or_else(|| unsupported("IR store root lacks its actual issue"))?;
                    value_issue = index + 1;
                    if access.word != (width == Width::Word) {
                        return Err(unsupported("IR store root changed physical width"));
                    }
                    let Op::WriteMemory { value, .. } = op else {
                        return Err(unsupported("store value root is not an SSA store"));
                    };
                    let mut sources = [Origin::EMPTY; 16];
                    for (bit, source) in
                        sources
                            .iter_mut()
                            .enumerate()
                            .take(if access.word { 16 } else { 8 })
                    {
                        *source = self.value_source(*value, 1 << bit, false, false, false)?;
                    }
                    if !private(owner.entry.boundary.cpu.sp(), instruction, access) {
                        let source = self.operation_source(op, use_, false, false)?;
                        self.require(source);
                    }
                    self.pending.push(Store {
                        issue: instruction.issues.start + index,
                        access,
                        sources,
                        committed: false,
                    });
                }
                DependencyUseKind::StateWrite { field } => {
                    if !private_traffic
                        && field != StateField::Prefetch
                        && !(interrupt && instruction.actual.opcode == 0x4e73)
                    {
                        let source = self.output_source(
                            field,
                            self.report.instruction.possible_writes.get(field),
                            matches!(field, StateField::Usp | StateField::Ssp),
                        )?;
                        self.require(source);
                    }
                }
                DependencyUseKind::PreservedState { .. } => {}
                _ => {
                    let source = self.operation_source(op, use_, false, false)?;
                    self.require(source);
                }
            }
            #[cfg(test)]
            if self.demands.is_some() {
                self.sites.push((use_.kind, previous, self.required));
            }
        }
        if read_ordinal != self.reads.len()
            || next_write(issues, value_issue).is_some()
            || next_write(issues, address_issue).is_some()
        {
            return Err(unsupported(
                "IR roots do not exhaust the exact physical read/write issues",
            ));
        }
        for index in 0..StateField::COUNT {
            let field = StateField::from_index(index).expect("canonical output field");
            let mut bits = self.report.instruction.possible_writes.get(field);
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                self.changes[field.index() as usize][bit] =
                    self.output_source(field, 1 << bit, false)?;
                bits &= bits - 1;
            }
        }
        self.chronology(&owner.trace.chronology[instruction.chronology.clone()])?;
        for slot in &self.slots {
            self.origins.check_read(slot.occurrence)?;
        }
        for store in &self.pending {
            if !store.committed && store.access.address >= 0x400000 {
                return Err(unsupported(
                    "actual non-ROM store has no exact physical commit",
                ));
            }
        }
        // Input validation occurs only after every read has its actual-time
        // binding; a later commit cannot retroactively supply an earlier read.
        for index in 0..self.slots.len() {
            let slot = self.slots[index];
            if slot.mandatory {
                #[cfg(test)]
                let previous = self.required;
                let source = self.origins.read_source(
                    slot.occurrence,
                    if slot.access.word { u16::MAX } else { 0xff },
                )?;
                self.require(source);
                #[cfg(test)]
                if let Some(demands) = &mut self.demands {
                    let added = self.origins.demand(self.required)?.registers
                        - self.origins.demand(previous)?.registers;
                    if !added.is_empty() {
                        demands.push(InputDemandSite {
                            owner: owner.entry.function,
                            pc: instruction.actual.pc,
                            opcode: instruction.actual.opcode,
                            kind: format!("MandatoryRead({:06x})", slot.access.address),
                            private_traffic: false,
                            registers: added,
                        });
                    }
                }
            }
        }
        #[cfg(test)]
        if let Some(demands) = &mut self.demands {
            for &(kind, previous, current) in &self.sites {
                let added = self.origins.demand(current)?.registers
                    - self.origins.demand(previous)?.registers;
                if !added.is_empty() {
                    demands.push(InputDemandSite {
                        owner: owner.entry.function,
                        pc: instruction.actual.pc,
                        opcode: instruction.actual.opcode,
                        kind: format!("{kind:?}"),
                        private_traffic,
                        registers: added,
                    });
                }
            }
        }
        for (field, mut possible) in self.report.instruction.possible_writes.iter() {
            while possible != 0 {
                let bit = possible.trailing_zeros() as usize;
                self.origins.fields[field.index() as usize][bit] =
                    self.changes[field.index() as usize][bit];
                possible &= possible - 1;
            }
        }
        Ok(())
    }
    fn chronology(&mut self, events: &[Chronology]) -> Result<(), Failure> {
        for event in events {
            match *event {
                Chronology::CpuRead { access, issue } => {
                    let slot = self
                        .slots
                        .iter()
                        .find(|slot| slot.issue == issue)
                        .ok_or_else(|| {
                            unsupported("physical read marker has no exact actual issue occurrence")
                        })?;
                    self.origins.bind_read(slot.occurrence, access)?;
                }
                Chronology::CpuCommit { access, issue } => {
                    let store = self
                        .pending
                        .iter_mut()
                        .find(|store| store.issue == issue)
                        .ok_or_else(|| {
                            unsupported("actual RAM/device commit lost its source-value provenance")
                        })?;
                    if store.access != access || store.committed {
                        return Err(unsupported(
                            "physical store commit is changed or duplicated",
                        ));
                    }
                    for frame in &mut self.exception_frames {
                        frame.observe_write(access);
                    }
                    self.origins.store(access, &store.sources);
                    store.committed = true;
                }
                Chronology::DmaRead { address, value } => {
                    self.return_slot_as_data |= self.observes_return_slot(Access {
                        address,
                        word: true,
                        direction: Direction::Read(m68000_emu::ReadPurpose::Semantic),
                        value,
                    });
                    let source = self.origins.dma(Access {
                        address,
                        word: true,
                        direction: Direction::Read(m68000_emu::ReadPurpose::Semantic),
                        value,
                    })?;
                    self.require(source);
                }
                Chronology::Z80Write(access) | Chronology::HostMutation(access) => {
                    for frame in &mut self.exception_frames {
                        frame.observe_write(access);
                    }
                    self.origins.store(access, &[Origin::EMPTY; 16])
                }
                Chronology::Exception(payload) => {
                    if let m68000_emu::ExceptionBoundary::Returned { depth } = payload.boundary {
                        self.exception_frames.last_mut().ok_or_else(|| unsupported(
                            "actual exception return has no current certified hardware frame"))?
                            .finish_return(depth)?;
                    }
                }
                Chronology::Acknowledge { .. } => {}
            }
        }
        Ok(())
    }
    fn walk_callee(
        &mut self,
        child: &Invocation,
        reader: &dyn Fn(u32) -> Option<u16>,
        divider: u32,
        cache: &mut CompilerCache,
        interrupt: bool,
    ) -> Result<(), Failure> {
        let abi = child.entry.function.map(|id| &id.metadata().abi);
        let capacity = abi.map_or(0, |abi| {
            (abi.preserved_data.count_ones() + abi.preserved_address.count_ones()) as usize
        });
        let mut preserved = Vec::with_capacity(capacity);
        if let Some(abi) = abi {
            for index in 0..8 {
                if abi.preserved_data & (1 << index) != 0 {
                    preserved.push((index, self.origins.fields[index]));
                }
            }
            for index in 0..7 {
                if abi.preserved_address & (1 << index) != 0 {
                    preserved.push((index + 8, self.origins.fields[index + 8]));
                }
            }
        }
        self.walk(child, reader, divider, cache, interrupt)?;
        for &(index, _) in &preserved {
            if child.entry.boundary.cpu.words[index] != child.exit.cpu.words[index] {
                return Err(unsupported(format!(
                    "actual callee violates ABI-declared preservation of {:?}",
                    StateField::from_index(index).expect("canonical preserved D/A field")
                )));
            }
        }
        for (index, provider) in preserved {
            self.origins.fields[index] = provider;
        }
        Ok(())
    }
    pub fn walk(
        &mut self,
        invocation: &Invocation,
        reader: &dyn Fn(u32) -> Option<u16>,
        divider: u32,
        cache: &mut CompilerCache,
        interrupt: bool,
    ) -> Result<(), Failure> {
        if let Some(reason) = &invocation.unsupported {
            return Err(Failure::Unsupported(reason.clone()));
        }
        let frame_depth = self.exception_frames.len();
        for event in &invocation.trace.events {
            match event {
                Event::Instruction(index) => {
                    self.instruction(invocation, *index, reader, divider, cache, interrupt)?
                }
                Event::Callee(child) => {
                    self.walk_callee(child, reader, divider, cache, interrupt)?
                }
                Event::Interrupt(child) => self.walk(child, reader, divider, cache, true)?,
                Event::Idle(index) => {
                    let idle = &invocation.trace.idles[*index];
                    self.begin_reads(invocation, idle.issues.clone(), None)?;
                    let mut entered = None;
                    self.pending.clear();
                    if idle.before.cpu.exception_depth != idle.after.cpu.exception_depth {
                        if !interrupt
                            || idle.after.cpu.exception_depth != idle.before.cpu.exception_depth + 1
                            || idle.before.cpu.sr() & 0x2000 == 0
                        {
                            return Err(unsupported(
                                "hardware frame/bank change needs its explicit exception profile",
                            ));
                        }
                        entered = Some(ExceptionFrame::capture(invocation, *index, idle)?);
                        let frame = canonical_ram_address(idle.after.cpu.sp());
                        for issue in idle.issues.clone() {
                            let access = invocation.trace.issues[issue];
                            if access.direction != Direction::Write {
                                continue;
                            }
                            let address = canonical_ram_address(access.address);
                            let mut sources = [Origin::EMPTY; 16];
                            if address == frame
                                && access.word
                                && access.value == idle.before.cpu.sr() as u16
                            {
                                for (bit, source) in sources.iter_mut().enumerate() {
                                    *source = self
                                        .origins
                                        .registers(StateMask::single(StateField::Sr, 1 << bit));
                                }
                            } else if !(frame + 2..frame + 6).contains(&address) || !access.word {
                                return Err(unsupported(
                                    "unrecognized actual hardware exception-frame store",
                                ));
                            }
                            self.pending.push(Store {
                                issue,
                                access,
                                sources,
                                committed: false,
                            });
                        }
                        self.origins.clear_bits(StateField::Sr, !0x1f);
                    } else if !idle.issues.is_empty() {
                        return Err(unsupported(
                            "noninstruction CPU effects lack an actual hardware frame",
                        ));
                    }
                    self.chronology(&invocation.trace.chronology[idle.chronology.clone()])?;
                    for slot in &self.slots {
                        self.origins.check_read(slot.occurrence)?;
                    }
                    if self.pending.iter().any(|store| !store.committed) {
                        return Err(unsupported(
                            "hardware frame store lacks its physical commit",
                        ));
                    }
                    if let Some(frame) = entered {
                        self.exception_frames.push(frame);
                    }
                }
            }
        }
        if matches!(
            invocation.entry.kind,
            super::super::record::EntryKind::Interrupt { .. }
        ) && (self.exception_frames.len() != frame_depth + 1
            || !self
                .exception_frames
                .pop()
                .is_some_and(|frame| frame.finishes(invocation)))
        {
            return Err(Failure::Unsupported(Unsupported {
                kind: UnsupportedKind::TypedControlProfile,
                detail: "IRQ trace lacks its own complete untouched hardware-save/RTE identity"
                    .into(),
            }));
        }
        Ok(())
    }
}
fn width_bits(width: Width) -> u32 {
    match width {
        Width::Byte => 0xff,
        Width::Word => 0xffff,
    }
}
fn next_write(issues: &[Access], start: usize) -> Option<(usize, Access)> {
    issues
        .iter()
        .copied()
        .enumerate()
        .skip(start)
        .find(|(_, access)| access.direction == Direction::Write)
}

#[cfg(all(test, feature = "reference-proof-hooks"))]
mod tests;
