//! Analysis-only Context. Infallible access failure unwinds to the private root
//! boundary; it never manufactures an operand, readiness, result or live state.
use super::{
    Failure, RootAdmission, Unsupported, UnsupportedKind,
    cursor::{Cursor, Token},
    inputs::{self, CompilerCache},
    ledger::{CaseKey, OutputCoverage, Requirement, Tier},
    record::{Access, Direction, Invocation},
    report::{
        BodyReport, ContractReport, ObligationReport, ObligationStatus, OutputsReport,
        WitnessReport,
    },
};
use crate::hooks::HookRegistry;
use m68000_emu::ReadPurpose;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
use tje_engine_core::{dependencies::StateMask, ir::StateField};
use tje_game::{
    Context, Dispatcher, Error, FunctionId, Result,
    call_frame::FrameState,
    execution::{
        CallCeiling, CallCeilingScope, CallClock, ClockUsage, ExecutionClass, WaitRequest,
    },
};
use tje_mem::{Bus, Revision};

pub(super) struct Abort {
    attempt: super::Attempt,
}
fn game_error(failure: &Failure) -> Error {
    match failure {
        Failure::Mismatch(detail) => Error::Platform(format!("replay mismatch: {detail}")),
        Failure::Compared(detail) => Error::Platform(format!("replay mismatch: {}", detail.detail)),
        Failure::Unsupported(reason) => Error::Platform(format!(
            "replay unsupported ({:?}): {}",
            reason.kind, reason.detail
        )),
        Failure::StackRead(reason) => Error::UnsupportedStackRead(*reason),
    }
}
fn unsupported(kind: UnsupportedKind, detail: impl Into<String>) -> Failure {
    Failure::Unsupported(Unsupported {
        kind,
        detail: detail.into(),
    })
}
fn return_slot_byte(address: u32, entry_sp: u32) -> bool {
    crate::abi::canonical_ram_address(address)
        .wrapping_sub(crate::abi::canonical_ram_address(entry_sp))
        < 4
}
#[derive(Clone)]
enum RegisterValue<'a> {
    Rust(u32),
    Output {
        call: Rc<CallOutput<'a>>,
        field: StateField,
    },
    Masked {
        base: Rc<RegisterValue<'a>>,
        call: Rc<CallOutput<'a>>,
        field: StateField,
        bits: u32,
    },
}
impl RegisterValue<'_> {
    /// Returns only the requested bit projection. Public operand getters request
    /// their full width; no unrequested value is exposed or memoized as Rust data.
    fn resolve(&self, requested: u32) -> std::result::Result<u32, Failure> {
        match self {
            Self::Rust(value) => Ok(value & requested),
            Self::Output { call, field } => call.resolve(*field, requested),
            Self::Masked {
                base,
                call,
                field,
                bits,
            } => {
                let own = requested & !bits;
                let imported = requested & bits;
                Ok((if own != 0 { base.resolve(own)? } else { 0 })
                    | if imported != 0 {
                        call.resolve(*field, imported)?
                    } else {
                        0
                    })
            }
        }
    }
    fn observed(&self) -> (u32, u32) {
        match self {
            Self::Rust(value) => (*value, u32::MAX),
            Self::Output { call, field } => {
                let bits = call.required.borrow().state.get(*field);
                (
                    call.original.exit.cpu.words[field.index() as usize] & bits,
                    bits,
                )
            }
            Self::Masked {
                base,
                call,
                field,
                bits,
            } => {
                let (base_value, base_known) = base.observed();
                let imported = call.required.borrow().state.get(*field) & bits;
                (
                    (base_value & !bits)
                        | (call.original.exit.cpu.words[field.index() as usize] & imported),
                    (base_known & !bits) | imported,
                )
            }
        }
    }
}
struct CallOutput<'a> {
    original: &'a Invocation,
    entry_values: [RegisterValue<'a>; StateField::COUNT],
    output_inputs: inputs::OutputInputs,
    memory_inputs: BTreeMap<u32, std::result::Result<u8, Failure>>,
    required: RefCell<OutputCoverage>,
    case: Option<(CaseKey, Tier)>,
    reason: Option<String>,
    body: Option<BodyReport>,
    contract: Option<ContractReport>,
    obligation_id: u64,
}
impl CallOutput<'_> {
    fn resolve(&self, field: StateField, requested: u32) -> std::result::Result<u32, Failure> {
        // Endpoint-specific input checks occur before THIS output is imported.
        // Entry providers preserve earlier call lineage instead of eagerly
        // observing unrelated scratch values at every subsequent call.
        let inputs = self.output_inputs.demand(field, requested)?;
        validate_registers(inputs.registers, &self.entry_values, self.original)?;
        for (&address, input) in &inputs.memory {
            let actual = self
                .memory_inputs
                .get(&address)
                .ok_or_else(|| {
                    unsupported(
                        UnsupportedKind::RegisterDependency,
                        "lazy output lacks its immutable call-time RAM input provider",
                    )
                })?
                .as_ref()
                .map_err(Clone::clone)?;
            if (actual ^ input.value) & input.bits != 0 {
                return Err(Failure::Compared(Box::new(
                    super::report::ComparisonDetail {
                        point: "callee_output_memory_dependency",
                        original_pc: Some(self.original.entry.boundary.cpu.pc()),
                        original_opcode: None,
                        own_access_index: None,
                        expected: Some(
                            serde_json::json!({"address":address,"bits":input.bits,"value":input.value & input.bits}),
                        ),
                        actual: Some(
                            serde_json::json!({"address":address,"bits":input.bits,"value":actual & input.bits}),
                        ),
                        detail: format!(
                            "callee output consumes RAM {address:06x}/{:02x}: original={:02x}, call-time Rust={:02x}",
                            input.bits,
                            input.value & input.bits,
                            actual & input.bits
                        ),
                    },
                )));
            }
        }
        self.required.borrow_mut().state.insert(field, requested);
        Ok(self.original.exit.cpu.words[field.index() as usize] & requested)
    }
}
fn validate_registers(
    mask: StateMask,
    values: &[RegisterValue<'_>; StateField::COUNT],
    original: &Invocation,
) -> std::result::Result<(), Failure> {
    for (field, bits) in mask.iter() {
        let actual = values[field.index() as usize].resolve(bits)?;
        let expected = original.entry.boundary.cpu.words[field.index() as usize] & bits;
        if actual != expected {
            return Err(Failure::Compared(Box::new(
                super::report::ComparisonDetail {
                    point: "callee_entry_input_dependency",
                    original_pc: Some(original.entry.boundary.cpu.pc()),
                    original_opcode: None,
                    own_access_index: None,
                    expected: Some(
                        serde_json::json!({"field":format!("{field:?}"),"bits":bits,"value":expected}),
                    ),
                    actual: Some(
                        serde_json::json!({"field":format!("{field:?}"),"bits":bits,"value":actual}),
                    ),
                    detail: format!(
                        "callee {} consumes {field:?} bits={bits:08x}: original={expected:08x}, Rust={actual:08x}",
                        original
                            .entry
                            .function
                            .map_or("<uncatalogued>", |id| id.metadata().name)
                    ),
                },
            )));
        }
    }
    Ok(())
}
fn effects(original: &Invocation) -> OutputCoverage {
    let mut result = OutputCoverage::default();
    for event in &original.trace.events {
        match event {
            super::record::Event::Instruction(index) => {
                let instruction = &original.trace.instructions[*index];
                let high = crate::abi::canonical_ram_address(original.entry.boundary.cpu.sp());
                let low = crate::abi::canonical_ram_address(instruction.before.cpu.sp())
                    .min(crate::abi::canonical_ram_address(instruction.after_sp))
                    .min(high);
                for access in &original.trace.issues[instruction.issues.clone()] {
                    if (0x400000..0xe00000).contains(&access.address) {
                        result.ordered_device_effects = true;
                    }
                    if access.direction == Direction::Write
                        && access.ram()
                        && access.bytes().any(|(a, _)| {
                            !(low..high).contains(&crate::abi::canonical_ram_address(a))
                        })
                    {
                        result.nonframe_ram = true;
                    }
                }
            }
            super::record::Event::Callee(child) => {
                let child = effects(child);
                result.nonframe_ram |= child.nonframe_ram;
                result.ordered_device_effects |= child.ordered_device_effects;
            }
            // These are authenticated original IRQ environment, not claims
            // that the callee's readable body produced original ISR traffic.
            super::record::Event::Interrupt(_) | super::record::Event::Idle(_) => {}
        }
    }
    result
}

pub(super) struct Inner<'a> {
    pub(super) cursor: Cursor<'a>,
    values: [RegisterValue<'a>; StateField::COUNT],
    frame: FrameState,
    class: ExecutionClass,
    ceiling: ExecutionClass,
    dispatcher: Dispatcher,
    clock: CallClock,
    root_scope: Option<CallCeilingScope>,
    calls: Vec<Rc<CallOutput<'a>>>,
    failure: Option<Failure>,
    compiler: &'a mut CompilerCache,
    call_target: inputs::CallTarget,
    handler_inputs: &'a mut inputs::InterruptedRegisterReads,
    pub(super) reader: &'a dyn Fn(u32) -> Option<u16>,
    registry: &'a HookRegistry,
    divider: u32,
    #[cfg(test)]
    pub(super) waited: bool,
}
impl<'a> Inner<'a> {
    pub(super) fn peek(&mut self, optional: bool) -> std::result::Result<Token<'a>, Failure> {
        self.peek_mode(optional, false)
    }
    fn peek_mode(
        &mut self,
        optional: bool,
        frame_reads: bool,
    ) -> std::result::Result<Token<'a>, Failure> {
        loop {
            match self.cursor.peek_with_frame_reads(optional, frame_reads)? {
                Token::Interrupt(interrupt) => {
                    if let Some(reason) = &interrupt.unsupported {
                        return Err(Failure::Unsupported(reason.clone()));
                    }
                    let vector = match interrupt.entry.kind {
                        super::record::EntryKind::Interrupt { vector }
                            if (25..=31).contains(&vector) =>
                        {
                            vector
                        }
                        _ => {
                            return Err(unsupported(
                                UnsupportedKind::TypedControlProfile,
                                "the ordinary environment admits only recorded autovector IRQ ordering classes",
                            ));
                        }
                    };
                    let Some(super::record::Event::Idle(index)) = interrupt.trace.events.first()
                    else {
                        return Err(unsupported(
                            UnsupportedKind::Observation,
                            "ISR lacks its actual hardware-entry context",
                        ));
                    };
                    let before = &interrupt.trace.idles[*index].before;
                    if before.cpu.words[..17] != interrupt.exit.cpu.words[..17] {
                        return Err(unsupported(
                            UnsupportedKind::TypedControlProfile,
                            "ISR changes interrupted D/A/banked-stack state; explicit boundary-aware control evidence is required",
                        ));
                    }
                    let inputs = inputs::interrupt_inputs(
                        interrupt,
                        self.reader,
                        self.divider,
                        self.compiler,
                    )?;
                    let root = self
                        .cursor
                        .original
                        .entry
                        .function
                        .expect("admitted catalogue root");
                    self.handler_inputs
                        .entry((root, vector, interrupt.entry.boundary.cpu.pc()))
                        .and_modify(|mask| *mask |= inputs.registers)
                        .or_insert(inputs.registers);
                    #[cfg(test)]
                    if let Some(demands) = &inputs.demands {
                        let sites: Vec<_> = demands.iter().map(|site| {
                            let registers: Vec<_> = site.registers.iter().map(|(field, bits)|
                                serde_json::json!({"field":format!("{field:?}"),"bits":bits})).collect();
                            serde_json::json!({"owner":site.owner.map(|id|id.metadata().inventory_id),
                                "pc":site.pc,"opcode":site.opcode,"kind":site.kind,
                                "private_traffic":site.private_traffic,"registers":registers})
                        }).collect();
                        println!(
                            "REPLAY_ISR_DEMANDS {}",
                            serde_json::json!({
                                "diagnostics_only":true,"revision":self.registry.revision().name(),
                                "root":root.metadata().inventory_id,"vector":vector,
                                "handler_pc":interrupt.entry.boundary.cpu.pc(),
                                "actual_interrupted_pc":before.cpu.pc(),
                                "actual_interrupted_d2_d3_d4":&before.cpu.words[2..5],
                                "own_original_pc":self.cursor.original_pc,
                                "own_access_index":self.cursor.own_accesses,"sites":sites,
                                "scope":"finite new instruction demands; repeated probes are not invocation counts or proof authority"
                            })
                        );
                    }
                    for (field, bits) in inputs.registers.iter() {
                        // The ordinary six-byte hardware frame preserves SP's
                        // alignment bit. Compare that control input normally;
                        // do not mistake it for an algorithmic stack value.
                        if matches!(field, StateField::Usp | StateField::Ssp) && bits & !1 != 0 {
                            return Err(unsupported(
                                UnsupportedKind::TypedControlProfile,
                                format!(
                                    "ISR algorithm consumes its hardware-adjusted stack value ({field:?}/{bits:08x}); an explicit control profile is required"
                                ),
                            ));
                        }
                        let actual = self.values[field.index() as usize].resolve(bits)?;
                        let expected = before.cpu.words[field.index() as usize] & bits;
                        if actual != expected {
                            return Err(Failure::Compared(Box::new(
                                super::report::ComparisonDetail {
                                    point: "isr_entry_input_dependency",
                                    original_pc: Some(interrupt.entry.boundary.cpu.pc()),
                                    original_opcode: None,
                                    own_access_index: None,
                                    expected: Some(
                                        serde_json::json!({"field":format!("{field:?}"),"bits":bits,"value":expected}),
                                    ),
                                    actual: Some(
                                        serde_json::json!({"field":format!("{field:?}"),"bits":bits,"value":actual}),
                                    ),
                                    detail: format!(
                                        "ISR {:06x} consumes {field:?} bits={bits:08x}: original={expected:08x}, Rust={actual:08x}",
                                        interrupt.entry.boundary.cpu.pc()
                                    ),
                                },
                            )));
                        }
                    }
                    for (&address, &value) in &inputs.memory {
                        let access = Access {
                            address,
                            word: false,
                            direction: Direction::Read(ReadPurpose::Semantic),
                            value: u16::from(value.value),
                        };
                        self.cursor.memory.check_environment_read(
                            access,
                            tje_game::stack_read::Reader::HardwareRam,
                        )?;
                        self.cursor
                            .memory
                            .check_call_bits(address, value.value, value.bits)?;
                    }
                    self.cursor.take_interrupt(interrupt)?;
                }
                token => return Ok(token),
            }
        }
    }
    fn peek_frontier(
        &mut self,
        private: Option<(u32, bool)>,
    ) -> std::result::Result<Token<'a>, Failure> {
        loop {
            let token = self.peek(false)?;
            let shadow = private.is_some_and(|(address, word)| {
                self.cursor.memory.contains(Access {
                    address: address & 0xffffff,
                    word,
                    direction: Direction::Read(ReadPurpose::Semantic),
                    value: 0,
                })
            });
            if matches!(token, Token::OptionalAbi(_)) && !shadow {
                self.cursor.elide_retained_abi();
                continue;
            }
            return Ok(token);
        }
    }
    pub(super) fn read(&mut self, address: u32, word: bool) -> std::result::Result<u16, Failure> {
        self.peek_frontier(Some((address, word)))?;
        self.cursor.read(address, word)
    }
    fn read_uninitialized(
        &mut self,
        address: u32,
        word: bool,
    ) -> std::result::Result<u16, Failure> {
        self.peek_mode(false, true)?;
        self.cursor.read_uninitialized(address, word)
    }
    pub(super) fn poll_read(
        &mut self,
        address: u32,
        word: bool,
        required: u16,
    ) -> std::result::Result<u16, Failure> {
        self.peek(false)?;
        self.cursor.read_for_predicate(address, word, required)
    }
    fn write(&mut self, address: u32, word: bool, value: u16) -> std::result::Result<(), Failure> {
        self.peek_frontier(Some((address, word)))?;
        self.cursor.write(address, word, value)
    }
    fn discard(&mut self, address: u32, word: bool) -> std::result::Result<(), Failure> {
        self.peek(true)?;
        self.cursor.discard(address, word)
    }
    fn call(
        &mut self,
        id: FunctionId,
        arguments: Option<&[u32]>,
        wants_value: bool,
    ) -> std::result::Result<Option<u32>, Failure> {
        if wants_value && !id.metadata().abi.returns_value {
            return Err(unsupported(
                UnsupportedKind::CalleeInput,
                format!(
                    "{}: use {} for its actual ABI",
                    id.metadata().name,
                    if wants_value {
                        "call_void/call_at_void"
                    } else {
                        "call/call_at"
                    }
                ),
            ));
        }
        let class = tje_game::execution_class(id).ok_or_else(|| {
            unsupported(
                UnsupportedKind::CalleeInput,
                "callee has no unique reviewed class",
            )
        })?;
        if !self.class.permits(class) {
            return Err(unsupported(
                UnsupportedKind::CalleeInput,
                "callee class is unavailable inside this readable root",
            ));
        }
        let Token::Callee(original) = self.peek_frontier(None)? else {
            return Err(Failure::Mismatch(
                "extra/reordered readable call has no actual original JSR segment".into(),
            ));
        };
        if let Some(reason) = &original.unsupported {
            return Err(Failure::Unsupported(reason.clone()));
        }
        if original.entry.function != Some(id) {
            return Err(Failure::Mismatch(format!(
                "changed callee: Rust requested {}",
                id.metadata().name
            )));
        }
        let call_instruction = self.cursor.call_instruction(original)?;
        let computed = call_instruction.actual.opcode & 0xffc0 == 0x4e80
            && !matches!(call_instruction.actual.opcode & 63, 0x38..=0x3a);
        if computed && (arguments.is_some() || !self.frame.is_prepared_for(id)) {
            return Err(unsupported(
                UnsupportedKind::EntryBoundary,
                "computed call requires its Rust-owned prepared source CallSite",
            ));
        }
        let mut entry_values = self.values.clone();
        let entry_sp = original.entry.boundary.cpu.sp();
        let return_pc = match original.entry.kind {
            super::record::EntryKind::Call { return_pc, .. } => return_pc,
            _ => {
                return Err(unsupported(
                    UnsupportedKind::EntryBoundary,
                    "callee is not a real original JSR-to-RTS invocation",
                ));
            }
        };
        let temporary = if let Some(arguments) = arguments {
            if arguments.len() != usize::from(id.metadata().abi.argument_count) {
                return Err(Failure::Mismatch("scalar outgoing call omitted/invented formal slots; use the explicit frame form".into()));
            }
            let high = entry_sp.wrapping_add(4 + 4 * arguments.len() as u32);
            if crate::abi::canonical_ram_address(high) > self.cursor.memory.rust_sp() {
                return Err(unsupported(
                    UnsupportedKind::Frame,
                    "scalar materialization overlaps live Rust locals/reused outgoing area; use call_at",
                ));
            }
            for (index, &value) in arguments.iter().enumerate() {
                let address = entry_sp.wrapping_add(4 + 4 * index as u32);
                for (address, value) in [
                    (address, (value >> 16) as u16),
                    (address.wrapping_add(2), value as u16),
                ] {
                    let access = Access {
                        address: address & 0xffffff,
                        word: true,
                        direction: Direction::Write,
                        value,
                    };
                    if !self.cursor.memory.contains(access) {
                        return Err(unsupported(
                            UnsupportedKind::Frame,
                            "provided scalar slot is outside the actual caller's own frame",
                        ));
                    }
                    self.cursor.memory.write_private(access);
                }
            }
            // Derive the virtual ABI entry from Rust's OWN pre-call stack.
            // Original geometry only locates transport slots; it never repairs
            // an algorithmic SP demand (including deferred scratch/CCR uses).
            let bank = if self.values[StateField::Sr.index() as usize].resolve(0x2000)? != 0 {
                StateField::Ssp
            } else {
                StateField::Usp
            };
            entry_values[bank.index() as usize] =
                RegisterValue::Rust(self.sp()?.wrapping_sub(4 + 4 * arguments.len() as u32));
            Some((entry_sp, high))
        } else {
            let actual =
                self.values[if self.values[StateField::Sr.index() as usize].resolve(0x2000)? != 0 {
                    StateField::Ssp.index() as usize
                } else {
                    StateField::Usp.index() as usize
                }]
                .resolve(u32::MAX)?;
            if actual != entry_sp || self.frame.entry_sp != actual {
                return Err(Failure::Mismatch(
                    "prepared Rust frame does not match the actual original JSR entry SP".into(),
                ));
            }
            None
        };
        if computed {
            if original.entry.boundary.cpu.pc() != id.metadata().address(self.registry.revision()) {
                return Err(unsupported(
                    UnsupportedKind::EntryBoundary,
                    "actual computed call target is not the requested catalogue entry",
                ));
            }
            let bank = if entry_values[StateField::Sr.index() as usize].resolve(0x2000)? != 0 {
                StateField::Ssp
            } else {
                StateField::Usp
            };
            let rust_entry_sp = entry_values[bank.index() as usize].resolve(u32::MAX)?;
            for (offset, byte) in return_pc.to_be_bytes().into_iter().enumerate() {
                self.cursor
                    .memory
                    .check_call_byte(rust_entry_sp.wrapping_add(offset as u32), byte)?;
            }
            let required = self.call_target.registers(
                self.cursor.original,
                call_instruction,
                self.reader,
                self.divider,
                self.compiler,
            )?;
            for (field, bits) in required.iter() {
                // call_at already pushed the four-byte return address. Derive
                // pre-call SP from Rust's own prepared frame, never copy the
                // original instruction's SP/register snapshot into providers.
                let actual = if field == bank {
                    rust_entry_sp.wrapping_add(4) & bits
                } else {
                    entry_values[field.index() as usize].resolve(bits)?
                };
                let expected = call_instruction.before.cpu.words[field.index() as usize] & bits;
                if actual != expected {
                    return Err(Failure::Compared(Box::new(
                        super::report::ComparisonDetail {
                            point: "call_target_input_dependency",
                            original_pc: Some(call_instruction.actual.pc),
                            original_opcode: Some(call_instruction.actual.opcode),
                            own_access_index: Some(self.cursor.own_accesses),
                            expected: Some(
                                serde_json::json!({"field":format!("{field:?}"),"bits":bits,"value":expected}),
                            ),
                            actual: Some(
                                serde_json::json!({"field":format!("{field:?}"),"bits":bits,"value":actual}),
                            ),
                            detail: format!(
                                "computed call target consumes {field:?}/{bits:08x}: original={expected:08x}, Rust={actual:08x}"
                            ),
                        },
                    )));
                }
            }
        }
        let mut declared = StateMask::EMPTY;
        if wants_value {
            declared.insert(StateField::Data(0), u32::MAX);
        }
        let inputs = inputs::analyze(original, self.reader, self.divider, self.compiler, declared)?;
        if temporary.is_some() && inputs.return_slot_as_data {
            return Err(unsupported(
                UnsupportedKind::CalleeInput,
                "scalar call cannot authenticate return-site bytes as semantic data; use its Rust-owned call_at boundary",
            ));
        }
        validate_registers(inputs.registers, &entry_values, original)?;
        for (&address, &expected) in &inputs.memory {
            if temporary.is_some() && return_slot_byte(address, entry_sp) {
                // The actual shared CALL/RTS seal certifies this transport only.
                // No bytes are copied into the caller or any lazy provider.
                continue;
            }
            self.cursor
                .memory
                .check_call_bits(address, expected.value, expected.bits)
                .map_err(|failure| match failure {
                    Failure::Unsupported(mut reason) => {
                        reason.detail = format!(
                            "callee {} at source {:06x}: {}",
                            id.metadata().name,
                            call_instruction.actual.pc,
                            reason.detail
                        );
                        Failure::Unsupported(reason)
                    }
                    other => other,
                })?;
        }
        let binding = self.registry.replay_binding(id);
        let hook = self
            .registry
            .at(id.metadata().address(self.registry.revision()));
        let (case, reason, body, contract) = match (binding, hook) {
            (Some(binding), Some(hook)) => match CaseKey::callee(binding, hook, original) {
                Ok(case) => (Some((case, if hook.proof_envelope == tje_game::execution::ProofEnvelope::ReplayedEnvironment { Tier::Replayed } else { Tier::Strict })),
                    None, Some(BodyReport::bound(binding)), Some(ContractReport::registered(hook, super::Profile::OrdinaryRts))),
                Err(reason) => (None, Some(reason.detail), Some(BodyReport::bound(binding)), Some(ContractReport::registered(hook, super::Profile::OrdinaryRts))),
            },
            (binding, _) => (None, Some("actual callee has no independently hooked effective body/contract; this invocation remains uncovered".into()),
                binding.map(BodyReport::bound), None),
        };
        let mut memory_inputs = BTreeMap::new();
        for address in inputs.output_inputs.memory_addresses() {
            memory_inputs.entry(address).or_insert_with(|| {
                if temporary.is_some() && return_slot_byte(address, entry_sp) {
                    Err(unsupported(
                        UnsupportedKind::CalleeInput,
                        "deferred scalar output consumes an unauthenticated return-site datum",
                    ))
                } else {
                    self.cursor.memory.call_input_byte(address)
                }
            });
        }
        let call = Rc::new(CallOutput {
            original,
            entry_values,
            output_inputs: inputs.output_inputs,
            memory_inputs,
            required: RefCell::new(effects(original)),
            case,
            reason,
            body,
            contract,
            obligation_id: super::fresh(),
        });
        // No callee RAM/device effects are imported until every consumed input
        // above has authenticated. No Rust callee executes here.
        self.cursor.take_callee(original)?;
        let abi = &id.metadata().abi;
        for index in 0..8 {
            if abi.preserved_data & (1 << index) == 0 || index == 0 && wants_value {
                self.values[index] = RegisterValue::Output {
                    call: Rc::clone(&call),
                    field: StateField::Data(index as u8),
                };
            }
        }
        for index in 0..7 {
            if abi.preserved_address & (1 << index) == 0 {
                self.values[index + 8] = RegisterValue::Output {
                    call: Rc::clone(&call),
                    field: StateField::Address(index as u8),
                };
            }
        }
        let status = StateField::Sr.index() as usize;
        self.values[status] = RegisterValue::Masked {
            base: Rc::new(self.values[status].clone()),
            call: Rc::clone(&call),
            field: StateField::Sr,
            bits: 0x1f,
        };
        self.calls.push(Rc::clone(&call));
        if let Some((low, high)) = temporary {
            self.cursor.memory.clear_temporary(low, high);
        }
        if wants_value {
            Ok(Some(self.values[0].resolve(u32::MAX)?))
        } else {
            Ok(None)
        }
    }
    fn status(&self) -> std::result::Result<u16, Failure> {
        Ok(self.values[StateField::Sr.index() as usize].resolve(0xffff)? as u16)
    }
    fn sp(&self) -> std::result::Result<u32, Failure> {
        let field = if self.values[StateField::Sr.index() as usize].resolve(0x2000)? != 0 {
            StateField::Ssp
        } else {
            StateField::Usp
        };
        self.values[field.index() as usize].resolve(u32::MAX)
    }
    fn set_sp(&mut self, value: u32) -> std::result::Result<(), Failure> {
        self.cursor.memory.rust_stack(value)?;
        let field = if self.values[StateField::Sr.index() as usize].resolve(0x2000)? != 0 {
            StateField::Ssp
        } else {
            StateField::Usp
        };
        self.values[field.index() as usize] = RegisterValue::Rust(value);
        Ok(())
    }
}

pub(super) struct ReplayContext<'a> {
    inner: RefCell<Inner<'a>>,
    revision: Revision,
    abort_attempt: super::Attempt,
}
pub(super) struct AnalysisOptions<'a> {
    pub divider: u32,
    pub rom_len: usize,
    pub poison: u8,
    pub actual_usage: ClockUsage,
    pub handler_inputs: &'a mut inputs::InterruptedRegisterReads,
}
pub(super) struct CompletedAnalysis {
    pub values: [u32; StateField::COUNT],
    /// Only these bits are observations; other stored bits are not outputs.
    pub observed: StateMask,
    pub requirements: Vec<Requirement>,
    pub obligations: Vec<ObligationReport>,
    pub substitutions: u64,
}
impl<'a> ReplayContext<'a> {
    pub fn new(
        admission: &RootAdmission,
        original: &'a Invocation,
        ram: Box<[u8; 65536]>,
        registry: &'a HookRegistry,
        compiler: &'a mut CompilerCache,
        reader: &'a dyn Fn(u32) -> Option<u16>,
        options: AnalysisOptions<'a>,
    ) -> std::result::Result<Self, Failure> {
        if let Some(reason) = &original.unsupported {
            return Err(Failure::Unsupported(reason.clone()));
        }
        let mut clock = CallClock::default();
        let root_scope = clock
            .enter(
                admission.identity().function,
                admission.hook().clock_ceiling,
            )
            .map_err(|error| unsupported(UnsupportedKind::OperationalCeiling, error.to_string()))?;
        clock
            .record(options.actual_usage)
            .map_err(|error| unsupported(UnsupportedKind::OperationalCeiling, error.to_string()))?;
        Ok(Self {
            inner: RefCell::new(Inner {
                cursor: Cursor::new(
                    original,
                    ram,
                    options.rom_len,
                    options.poison,
                    registry.revision(),
                )?,
                values: std::array::from_fn(|index| {
                    RegisterValue::Rust(original.entry.boundary.cpu.words[index])
                }),
                frame: FrameState::new(original.entry.boundary.cpu.sp()),
                class: admission.identity().class,
                ceiling: admission.identity().class,
                dispatcher: registry.dispatcher(),
                clock,
                root_scope: Some(root_scope),
                calls: Vec::new(),
                failure: None,
                compiler,
                call_target: inputs::CallTarget::default(),
                reader,
                registry,
                divider: options.divider,
                handler_inputs: options.handler_inputs,
                #[cfg(test)]
                waited: false,
            }),
            revision: registry.revision(),
            abort_attempt: admission.derived_identity().attempt,
        })
    }
    fn operate<T>(
        &self,
        operation: impl FnOnce(&mut Inner<'a>) -> std::result::Result<T, Failure>,
    ) -> T {
        let mut inner = self.inner.borrow_mut();
        if inner.failure.is_some() {
            drop(inner);
            std::panic::resume_unwind(Box::new(Abort {
                attempt: self.abort_attempt,
            }));
        }
        let result = inner
            .clock
            .dispatch_step()
            .map_err(|error| unsupported(UnsupportedKind::OperationalCeiling, error.to_string()))
            .and_then(|()| operation(&mut inner));
        match result {
            Ok(value) => value,
            Err(failure) => {
                inner.failure.get_or_insert(failure);
                drop(inner);
                std::panic::resume_unwind(Box::new(Abort {
                    attempt: self.abort_attempt,
                }));
            }
        }
    }
    fn try_operate<T>(
        &self,
        operation: impl FnOnce(&mut Inner<'a>) -> std::result::Result<T, Failure>,
    ) -> Result<T> {
        let mut inner = self.inner.borrow_mut();
        if let Some(failure) = &inner.failure {
            return Err(game_error(failure));
        }
        let result = inner
            .clock
            .dispatch_step()
            .map_err(|error| unsupported(UnsupportedKind::OperationalCeiling, error.to_string()))
            .and_then(|()| operation(&mut inner));
        result.map_err(|failure| {
            let error = game_error(&failure);
            inner.failure.get_or_insert(failure);
            error
        })
    }
    pub fn owns_abort(&self, payload: &(dyn std::any::Any + Send)) -> bool {
        payload
            .downcast_ref::<Abort>()
            .is_some_and(|abort| abort.attempt == self.abort_attempt)
    }
    pub fn sticky_failure(&self) -> Option<Failure> {
        self.inner.borrow().failure.clone()
    }
    #[cfg(test)]
    pub fn waited(&self) -> bool {
        self.inner.borrow().waited
    }
    pub fn position(&self) -> (Option<u32>, Option<u16>, u64) {
        let inner = self.inner.borrow();
        (
            inner.cursor.original_pc,
            inner.cursor.original_opcode,
            inner.cursor.own_accesses,
        )
    }
    pub fn finish(
        self,
        returned: Option<u32>,
        exit_ram: &[u8; 65536],
        admission: &RootAdmission,
    ) -> std::result::Result<CompletedAnalysis, Failure> {
        let mut inner = self.inner.into_inner();
        if let Some(failure) = inner.failure.take() {
            return Err(failure);
        }
        if returned.is_some() != admission.hook().symbol.abi.returns_value {
            return Err(Failure::Compared(Box::new(super::report::ComparisonDetail {
                point: "abi_return_shape", original_pc: Some(inner.cursor.original.exit.cpu.pc()),
                original_opcode: None, own_access_index: Some(inner.cursor.own_accesses),
                expected: Some(serde_json::json!({"returns_value":admission.hook().symbol.abi.returns_value})),
                actual: Some(serde_json::json!({"returned_value":returned})),
                detail: "readable root's optional scalar result disagrees with its declared ABI; scratch D0 must be an explicit Context effect".into(),
            })));
        }
        if let Some(value) = returned {
            inner.values[0] = RegisterValue::Rust(value);
        }
        if inner.sp()? != inner.cursor.original.entry.boundary.cpu.sp() {
            return Err(Failure::Mismatch(
                "readable root left its own SP unbalanced before RTS finalization".into(),
            ));
        }
        inner.peek_frontier(None)?;
        inner.cursor.finish(exit_ram)?;
        let abi = &admission.hook().symbol.abi;
        for index in 0..8 {
            if abi.strict_registers
                || abi.preserved_data & (1 << index) != 0
                || index == 0 && abi.returns_value
            {
                inner.values[index].resolve(u32::MAX)?;
            }
        }
        for index in 0..7 {
            if abi.strict_registers || abi.preserved_address & (1 << index) != 0 {
                inner.values[index + 8].resolve(u32::MAX)?;
            }
        }
        let sr = StateField::Sr.index() as usize;
        inner.values[sr].resolve(if abi.strict_registers { 0xffff } else { 0xffe0 })?;
        // Perform only the caller's own logical ABI pop, derived from its
        // already-balanced SP; never copy an original exit CPU/SP snapshot.
        let sp = inner.sp()?;
        let bank = if inner.values[sr].resolve(0x2000)? != 0 {
            StateField::Ssp
        } else {
            StateField::Usp
        };
        inner.values[bank.index() as usize] = RegisterValue::Rust(sp.wrapping_add(4));
        let mut values = [0; StateField::COUNT];
        let mut observed = StateMask::EMPTY;
        for (index, value) in inner.values.iter().enumerate() {
            let (value, known) = value.observed();
            values[index] = value;
            observed.insert(
                StateField::from_index(index).expect("canonical observed field"),
                known,
            );
        }
        observed = observed.registers_only();
        let mut requirements = Vec::with_capacity(inner.calls.len());
        let mut obligations = Vec::with_capacity(inner.calls.len());
        for call in &inner.calls {
            let outputs = *call.required.borrow();
            requirements.push(match call.case {
                Some((case, tier)) => Requirement::Checked {
                    case,
                    tier,
                    outputs,
                },
                None => Requirement::Uncovered {
                    function: call.original.entry.function.expect("catalogued callback"),
                    detail: call.reason.clone().expect("uncovered reason"),
                },
            });
            obligations.push(ObligationReport {
                id: call.obligation_id,
                caller_invocation_id: inner.cursor.original.entry.identity.serial(),
                callee_inventory_id: call
                    .original
                    .entry
                    .function
                    .expect("catalogued callback")
                    .metadata()
                    .inventory_id,
                callee_witness: WitnessReport::completed(call.original),
                body: call.body,
                effective_contract: call.contract,
                required_outputs: OutputsReport::covered(&outputs),
                status: ObligationStatus::Pending,
                discharged_by: None,
                reason: call.reason.clone(),
            });
        }
        let scope = inner.root_scope.take().expect("root analysis scope");
        inner
            .clock
            .leave(scope)
            .map_err(|error| unsupported(UnsupportedKind::OperationalCeiling, error.to_string()))?;
        Ok(CompletedAnalysis {
            values,
            observed,
            substitutions: inner.calls.len() as u64,
            requirements,
            obligations,
        })
    }
}
impl Bus for ReplayContext<'_> {
    fn read_u8(&self, address: u32) -> u8 {
        self.operate(|inner| inner.read(address, false).map(|value| value as u8))
    }
    fn read_u16(&self, address: u32) -> u16 {
        self.operate(|inner| inner.read(address, true))
    }
    fn read_u32(&self, address: u32) -> u32 {
        (u32::from(self.read_u16(address)) << 16)
            | u32::from(self.read_u16(address.wrapping_add(2)))
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.operate(|inner| inner.write(address, false, u16::from(value)));
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.operate(|inner| inner.write(address, true, value));
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.write_u16(address, (value >> 16) as u16);
        self.write_u16(address.wrapping_add(2), value as u16);
    }
}
impl tje_mem::Platform for ReplayContext<'_> {
    fn write_vdp_control(&mut self, value: u16) {
        tje_game::Platform::write_device_u16(self, 0xc00004, value);
    }
    fn write_vdp_data(&mut self, value: u16) {
        tje_game::Platform::write_device_u16(self, 0xc00000, value);
    }
}
impl tje_game::Platform for ReplayContext<'_> {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        self.read_u8(address)
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        self.read_u16(address)
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.write_u8(address, value);
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.write_u16(address, value);
    }
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.try_operate(|inner| inner.discard(address, false))
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.try_operate(|inner| inner.discard(address, true))
    }
    fn check_mmio_contract(&self) -> Result<()> {
        self.inner
            .borrow()
            .failure
            .as_ref()
            .map_or(Ok(()), |failure| Err(game_error(failure)))
    }
    fn wait_until(&mut self, request: WaitRequest) -> Result<()> {
        self.try_operate(|inner| {
            if inner.class != ExecutionClass::TimeAware {
                return Err(unsupported(
                    UnsupportedKind::PlatformCapability,
                    "dedicated recorded wait requires its reviewed TimeAware capability",
                ));
            }
            super::wait::run(inner, request).map(|_| ())
        })
    }
    fn poll_yield(&mut self) -> Result<()> {
        self.try_operate(|inner| {
            if inner.class != ExecutionClass::TimeAware {
                return Err(unsupported(
                    UnsupportedKind::PlatformCapability,
                    "loop yield requires reviewed TimeAware capability",
                ));
            }
            Ok(()) // Marker only: exact surrounding accesses own every iteration.
        })
    }
}
impl Context for ReplayContext<'_> {
    fn revision(&self) -> Revision {
        self.revision
    }
    fn data_register(&self, index: usize) -> u32 {
        self.operate(|inner| inner.values[index].resolve(u32::MAX))
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.operate(|inner| {
            inner.values[index] = RegisterValue::Rust(value);
            Ok(())
        });
    }
    fn address_register(&self, index: usize) -> u32 {
        self.operate(|inner| {
            if index == 7 {
                inner.sp()
            } else {
                inner.values[index + 8].resolve(u32::MAX)
            }
        })
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.operate(|inner| {
            if index == 7 {
                inner.set_sp(value)
            } else {
                inner.values[index + 8] = RegisterValue::Rust(value);
                Ok(())
            }
        });
    }
    fn status_register(&self) -> u16 {
        self.operate(|inner| inner.status())
    }
    fn set_status_register(&mut self, value: u16) {
        self.operate(|inner| {
            let system = inner.values[StateField::Sr.index() as usize].resolve(0xffe0)?;
            if u32::from(value) & 0xffe0 != system {
                return Err(unsupported(
                    UnsupportedKind::TypedControlProfile,
                    "full SR/bank/mask write needs its explicit control profile",
                ));
            }
            inner.values[StateField::Sr.index() as usize] = RegisterValue::Rust(u32::from(value));
            Ok(())
        });
    }
    fn stack_pointer(&self) -> Result<u32> {
        self.try_operate(|inner| inner.sp())
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.try_operate(|inner| inner.set_sp(value))
    }
    fn frame_state(&self) -> Option<FrameState> {
        Some(self.operate(|inner| Ok(inner.frame)))
    }
    fn set_frame_state(&mut self, frame: FrameState) -> Result<()> {
        self.try_operate(|inner| {
            inner.frame = frame;
            Ok(())
        })
    }
    fn read_uninitialized_local(
        &mut self,
        read: tje_game::stack_read::UninitializedLocalRead,
    ) -> Result<u32> {
        use tje_game::stack_read::{
            LocalWidth, Unsupported as StackUnsupported, UnsupportedReason,
        };
        self.try_operate(|inner| {
            let root = inner
                .cursor
                .original
                .entry
                .function
                .expect("admitted catalogue root");
            let entry_sp = inner.cursor.original.entry.boundary.cpu.sp();
            let reject =
                |reason| Failure::StackRead(StackUnsupported::paired(root, entry_sp, read, reason));
            let address = read.address();
            let bytes = read.width.bytes();
            if read.owner != root
                || read.frame != inner.frame
                || read.frame.entry_sp != entry_sp
                || read.offset >= 0
                || read.citation.trim().is_empty()
                || read.width != LocalWidth::Byte && address & 1 != 0
            {
                return Err(reject(UnsupportedReason::InvalidExplicitLocal));
            }
            let low = inner.cursor.memory.rust_sp();
            let (_, high) = inner.cursor.memory.private_bounds();
            for lane in 0..bytes {
                let Some(physical) =
                    tje_mem::written_ram::canonical_ram_byte(address.wrapping_add(u32::from(lane)))
                else {
                    return Err(reject(UnsupportedReason::InvalidExplicitLocal));
                };
                if !(low..high).contains(&physical) {
                    return Err(reject(UnsupportedReason::InvalidExplicitLocal));
                }
            }
            // Only after owner/current-frame/Rust-lifetime admission may the
            // actual original cursor expose this precise semantic occurrence.
            inner.peek_mode(false, true)?;
            inner
                .cursor
                .memory
                .explicit_unwritten_mask(address, bytes)
                .map_err(reject)?;
            let value = match read.width {
                LocalWidth::Byte => u32::from(inner.read_uninitialized(address, false)?),
                LocalWidth::Word => u32::from(inner.read_uninitialized(address, true)?),
                LocalWidth::Long => inner.cursor.read_uninitialized_long(address)?,
            };
            // Explicit intent admits this occurrence only, not later generic
            // reads. No initialized/history/proof-status bit changes here.
            Ok(value)
        })
    }
    fn dispatcher(&self) -> Dispatcher {
        self.inner.borrow().dispatcher
    }
    fn check_callee_dispatch(
        &self,
        dispatcher: Dispatcher,
        entry: FunctionId,
        rust_selected: bool,
    ) -> Result<()> {
        self.try_operate(|inner| {
            if rust_selected && inner.cursor.original.entry.function != Some(entry) {
                return Err(unsupported(UnsupportedKind::CalleeCoverage,
                    "replay may execute only its admitted own Rust root; callees are recorded boundaries"));
            }
            if dispatcher.same_selection(inner.dispatcher) {
                Ok(())
            } else {
                Err(unsupported(
                    UnsupportedKind::CalleeCoverage,
                    "caller selected a different unbound dispatcher configuration",
                ))
            }
        })
    }
    fn execution_class(&self) -> ExecutionClass {
        self.inner.borrow().class
    }
    fn set_execution_class(&mut self, class: ExecutionClass) -> Result<()> {
        self.try_operate(|inner| {
            if inner.ceiling.permits(class) {
                inner.class = class;
                Ok(())
            } else {
                Err(unsupported(
                    UnsupportedKind::PlatformCapability,
                    "class exceeds root execution capability",
                ))
            }
        })
    }
    fn clocked_execution(&self) -> bool {
        true
    } // Diagnostic scope support, not an emulated-time grant.
    fn enter_call_ceiling(&mut self, function: FunctionId) -> Result<CallCeilingScope> {
        self.try_operate(|inner| {
            let ceiling = tje_game::execution_contract(function)
                .and_then(|contract| contract.operational_ceiling)
                .unwrap_or(CallCeiling::DEFAULT);
            inner.clock.enter(function, ceiling).map_err(|error| {
                unsupported(UnsupportedKind::OperationalCeiling, error.to_string())
            })
        })
    }
    fn leave_call_ceiling(&mut self, scope: CallCeilingScope) -> Result<()> {
        let mut inner = self.inner.borrow_mut();
        inner.clock.leave(scope).map(|_| ()).map_err(|error| {
            let failure = unsupported(UnsupportedKind::OperationalCeiling, error.to_string());
            let result = game_error(&failure);
            inner.failure.get_or_insert(failure);
            result
        })
    }
    fn call_original(&mut self, function: FunctionId, _: &[u32]) -> Result<u32> {
        Err(Error::Untranslated(function))
    }
    fn call_replayed(&mut self, function: FunctionId, arguments: Option<&[u32]>) -> Result<u32> {
        self.try_operate(|inner| {
            inner
                .call(function, arguments, true)
                .map(|value| value.expect("value request imports declared D0"))
        })
    }
    fn call_replayed_void(
        &mut self,
        function: FunctionId,
        arguments: Option<&[u32]>,
    ) -> Result<()> {
        self.try_operate(|inner| inner.call(function, arguments, false).map(|_| ()))
    }
}

#[cfg(all(test, feature = "reference-proof-hooks"))]
mod tests;
