//! MCC68K C ABI: caller-owned 32-bit argument slots, D0 result, D2-D7 and
//! A2-A6 preserved. Short/byte arguments occupy the low part of their slot.
//! Default atomic calls freeze clocks; explicitly registered functional checks
//! quotient device latency, and time-aware calls run real clocks/interrupts.
//! Check-mode live execution retains the original instruction timing.

use crate::execution::{
    ClockRun, WaitClock, run_original_timed_with_audio, run_wait, run_wait_with_audio,
};
use crate::{
    HostError, Result,
    catalogue::{STACK_BOTTOM, STACK_TOP},
    check_projection::LogicTrace,
    exits::ReferenceBoundary,
};
use genesis_core::{GenesisBus, GenesisEmulator};
use m68000_emu::{BusInterface, M68000, ReadPurpose, debug::DummyM68000Debugger};
use serde::Serialize;
use std::{cell::RefCell, collections::BTreeSet};
use tje_game::call_frame::FrameState;
use tje_game::execution::{ExecutionClass, WaitRequest};
use tje_game::{Dispatcher, FunctionId};
use tje_rom::Revision;

pub(crate) fn canonical_ram_address(address: u32) -> u32 {
    let physical = address & 0xFF_FFFF;
    if physical >= 0xE00000 {
        0xFF0000 | (physical & 0xFFFF)
    } else {
        physical
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OrderedWrite {
    Ram { address: u32, value: u8 },
    DeviceByte { address: u32, value: u8 },
    DeviceWord { address: u32, value: u16 },
}

pub const CALL_SENTINEL: u32 = 0x003F_FFFE;
const MAX_CALL_INSTRUCTIONS: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct UninitializedStackRead {
    pub pc: u32,
    pub address: u32,
}

#[derive(Debug, Default)]
pub(crate) struct AtomicRun {
    pub instructions: u64,
    pub cycles: u64,
    pub uninitialized_reads: BTreeSet<UninitializedStackRead>,
    pub writes: Vec<OrderedWrite>,
    pub stack_reads: Option<Vec<crate::dependent_equivalence::journal::JournalRead>>,
}

impl AtomicRun {
    pub(crate) fn append(&mut self, other: Self) {
        self.instructions += other.instructions;
        self.cycles += other.cycles;
        self.uninitialized_reads.extend(other.uninitialized_reads);
        self.writes.extend(other.writes);
        if let Some(reads) = other.stack_reads {
            self.stack_reads.get_or_insert_with(Vec::new).extend(reads);
        }
    }
}

/// Atomic originals suppress IRQ delivery, but otherwise use the real bus.
/// Auditing happens from its actual read/commit stream after each instruction,
/// never from the CPU's instruction-local write issue calls.
struct StackAuditBus<'a> {
    inner: &'a mut GenesisBus,
}

impl BusInterface for StackAuditBus<'_> {
    type DebugView<'a>
        = DummyM68000Debugger
    where
        Self: 'a;
    fn read_byte(&mut self, address: u32) -> u8 {
        self.inner.read_byte(address)
    }
    fn read_word(&mut self, address: u32) -> u16 {
        self.inner.read_word(address)
    }
    fn read_byte_tagged(&mut self, address: u32, purpose: ReadPurpose) -> u8 {
        self.inner.read_byte_tagged(address, purpose)
    }
    fn read_word_tagged(&mut self, address: u32, purpose: ReadPurpose) -> u16 {
        self.inner.read_word_tagged(address, purpose)
    }
    fn write_byte(&mut self, address: u32, value: u8) {
        self.inner.write_byte(address, value);
    }
    fn write_word(&mut self, address: u32, value: u16) {
        self.inner.write_word(address, value);
    }
    fn interrupt_level(&self) -> u8 {
        0
    }
    fn acknowledge_interrupt(&mut self, level: u8) {
        self.inner.acknowledge_interrupt(level);
    }
    fn halt(&self) -> bool {
        self.inner.halt()
    }
    fn reset(&self) -> bool {
        <GenesisBus as BusInterface>::reset(self.inner)
    }
}

pub(crate) fn run_original_atomic(
    emulator: &mut GenesisEmulator,
    return_pc: u32,
    return_sp: u32,
    observe: bool,
) -> Result<AtomicRun> {
    run_original_atomic_until(
        emulator,
        ReferenceBoundary::returned(return_pc, return_sp),
        observe,
        false,
    )
}

pub(crate) fn run_original_atomic_until(
    emulator: &mut GenesisEmulator,
    boundary: ReferenceBoundary,
    observe: bool,
    capture_reads: bool,
) -> Result<AtomicRun> {
    let (cpu, bus) = emulator.hybrid_parts();
    if !observe {
        return run_atomic_loop(cpu, bus, boundary, |cpu, bus| {
            let cycles = cpu.execute_instruction(bus);
            bus.apply_pending_writes();
            cycles
        });
    }
    let mut stack = StackAuditState::new(cpu.stack_pointer(), capture_reads);
    run_original_atomic_scoped(emulator, boundary, &mut stack)
}

/// A nested original uses the suspended root's initialized bytes and boundary.
/// Incoming callee arguments can still be unwritten locals of that root.
pub(crate) fn run_original_atomic_scoped(
    emulator: &mut GenesisEmulator,
    boundary: ReferenceBoundary,
    stack: &mut StackAuditState,
) -> Result<AtomicRun> {
    let (cpu, bus) = emulator.hybrid_parts();
    let owns_trace = bus.hybrid_cpu_accesses.is_none();
    if owns_trace {
        bus.hybrid_cpu_accesses = Some(Vec::new());
    }
    let mut observed = stack.empty_run();
    stack.drain(bus, cpu.pc(), &mut observed);
    let mut audit = StackAuditBus { inner: bus };
    let result = run_atomic_loop(cpu, &mut audit, boundary, |cpu, audit| {
        let pc = cpu.pc();
        let cycles = cpu.execute_instruction(audit);
        audit.inner.apply_pending_writes();
        stack.drain(audit.inner, pc, &mut observed);
        cycles
    });
    if owns_trace {
        audit.inner.hybrid_cpu_accesses = None;
    }
    let mut result = result?;
    result.append(observed);
    Ok(result)
}

fn run_atomic_loop<B: BusInterface>(
    cpu: &mut M68000,
    bus: &mut B,
    boundary: ReferenceBoundary,
    mut step: impl FnMut(&mut M68000, &mut B) -> u32,
) -> Result<AtomicRun> {
    let mut result = AtomicRun::default();
    let mut previous_pc = None;
    while !boundary.reached(cpu, previous_pc, result.instructions) {
        if result.instructions == MAX_CALL_INSTRUCTIONS {
            return Err(HostError::CallDidNotReturn {
                pc: cpu.pc(),
                return_pc: boundary.pc,
            });
        }
        previous_pc = Some(cpu.pc());
        result.cycles += u64::from(step(cpu, bus));
        result.instructions += 1;
    }
    Ok(result)
}

pub(crate) fn set_sp(cpu: &mut M68000, value: u32) {
    let (mut usp, mut ssp) = (cpu.user_stack_pointer(), cpu.supervisor_stack_pointer());
    if cpu.status_register() & 0x2000 != 0 {
        ssp = value;
    } else {
        usp = value;
    }
    cpu.set_address_registers(cpu.address_registers(), usp, ssp);
}

pub(crate) struct StackAuditState {
    entry_sp: u32,
    written: [u64; 512],
    capture_reads: bool,
}

impl StackAuditState {
    pub(crate) fn new(entry_sp: u32, capture_reads: bool) -> Self {
        Self {
            entry_sp: canonical_ram_address(entry_sp),
            written: [0; 512],
            capture_reads,
        }
    }

    pub(crate) fn empty_run(&self) -> AtomicRun {
        AtomicRun {
            stack_reads: self.capture_reads.then(Vec::new),
            ..AtomicRun::default()
        }
    }

    fn stack_offset(&self, address: u32) -> Option<usize> {
        let address = address & 0xffffff;
        if address < 0xe00000 {
            return None;
        }
        let physical = canonical_ram_address(address);
        (STACK_BOTTOM..self.entry_sp.min(STACK_TOP))
            .contains(&physical)
            .then_some(physical.wrapping_sub(STACK_BOTTOM) as usize)
    }

    fn read_mask(&self, address: u32, word: bool) -> u8 {
        let mut mask = 0;
        for offset in 0..if word { 2 } else { 1 } {
            if let Some(index) = self.stack_offset(address.wrapping_add(offset))
                && self.written[index / 64] & (1 << (index % 64)) == 0
            {
                mask |= 1 << offset;
            }
        }
        mask
    }

    fn read(&self, pc: u32, access: genesis_core::HybridAccess, audit: &mut AtomicRun) {
        let (address, word, semantic) = match access {
            genesis_core::HybridAccess::Read {
                address,
                word,
                purpose,
                ..
            } => (address, word, purpose == ReadPurpose::Semantic),
            genesis_core::HybridAccess::DmaRead { address, .. } => (address, true, true),
            genesis_core::HybridAccess::Write { .. } => unreachable!("read observation"),
        };
        let mask = self.read_mask(address, word);
        if semantic {
            for offset in 0..if word { 2 } else { 1 } {
                if mask & (1 << offset) != 0 {
                    audit.uninitialized_reads.insert(UninitializedStackRead {
                        pc,
                        address: canonical_ram_address(address.wrapping_add(offset)),
                    });
                }
            }
        }
        if let Some(reads) = &mut audit.stack_reads
            && let Some(read) =
                crate::dependent_equivalence::journal::JournalRead::new(pc, access, mask)
        {
            reads.push(read);
        }
    }

    pub(crate) fn observe(
        &mut self,
        pc: u32,
        access: genesis_core::HybridAccess,
        audit: &mut AtomicRun,
    ) {
        let genesis_core::HybridAccess::Write {
            address,
            word,
            value,
        } = access
        else {
            self.read(pc, access, audit);
            return;
        };
        let address = address & 0xffffff;
        if address >= 0xe00000 {
            for offset in 0..if word { 2 } else { 1 } {
                let address = address.wrapping_add(offset);
                if let Some(index) = self.stack_offset(address) {
                    self.written[index / 64] |= 1 << (index % 64);
                }
                audit.writes.push(OrderedWrite::Ram {
                    address: canonical_ram_address(address),
                    value: if word && offset == 0 {
                        (value >> 8) as u8
                    } else {
                        value as u8
                    },
                });
            }
        } else if address >= 0x400000 {
            audit.writes.push(if word {
                OrderedWrite::DeviceWord { address, value }
            } else {
                OrderedWrite::DeviceByte {
                    address,
                    value: value as u8,
                }
            });
        }
    }

    pub(crate) fn drain(&mut self, bus: &mut GenesisBus, pc: u32, audit: &mut AtomicRun) {
        if let Some(accesses) = &mut bus.hybrid_cpu_accesses {
            for access in accesses.drain(..) {
                self.observe(pc, access, audit);
            }
        }
    }
}

/// A Rust translation sees the actual emulated address space and stack arguments.
/// It keeps its own locals in Rust; it never recreates the compiler's save area.
pub struct HookContext<'a> {
    emulator: &'a mut GenesisEmulator,
    pub nested_calls: u64,
    revision: Revision,
    dispatcher: Dispatcher,
    pub(crate) audit: RefCell<AtomicRun>,
    pub(crate) logic_trace: RefCell<LogicTrace>,
    pub(crate) exact_device_reads: Vec<usize>,
    mmio_contract_error: Option<tje_game::Error>,
    pub(crate) stack_audit: Option<StackAuditState>,
    frame_state: FrameState,
    pub(crate) execution_class: ExecutionClass,
    execution_ceiling: ExecutionClass,
    timed_execution: bool,
    pub(crate) wait_clock: WaitClock,
    pub(crate) time_audio: Vec<[f64; 2]>,
    pub time_frames: u64,
}

impl<'a> HookContext<'a> {
    pub(crate) fn new(
        emulator: &'a mut GenesisEmulator,
        revision: Revision,
        dispatcher: Dispatcher,
    ) -> Self {
        let frame_state = FrameState::new(emulator.hybrid_cpu().stack_pointer());
        Self {
            emulator,
            revision,
            dispatcher,
            audit: RefCell::new(AtomicRun::default()),
            logic_trace: RefCell::new(LogicTrace::default()),
            nested_calls: 0,
            exact_device_reads: Vec::new(),
            mmio_contract_error: None,
            stack_audit: None,
            frame_state,
            execution_class: ExecutionClass::Atomic,
            execution_ceiling: ExecutionClass::Atomic,
            timed_execution: false,
            wait_clock: WaitClock::default(),
            time_audio: Vec::new(),
            time_frames: 0,
        }
    }

    pub(crate) fn with_execution(mut self, class: ExecutionClass, clock: WaitClock) -> Self {
        self.execution_class = class;
        self.execution_ceiling = class;
        self.timed_execution = class == ExecutionClass::TimeAware
            || (class == ExecutionClass::Functional
                && !self.emulator.hybrid_bus().hybrid_functional_mode());
        self.wait_clock = clock;
        self
    }

    fn latch_mmio_error(&mut self, error: tje_game::Error) -> tje_game::Error {
        if self.mmio_contract_error.is_none() {
            self.mmio_contract_error = Some(error.clone());
        }
        error
    }

    fn game_platform_error(error: HostError) -> tje_game::Error {
        match error {
            HostError::Translation(error) => error,
            other => tje_game::Error::Platform(other.to_string()),
        }
    }

    /// Merge real IRQ/recipe accesses into the existing active-frame audit.
    /// A wait must never reset initialized bytes or replace the frame identity.
    fn merge_clock_run(&mut self, run: ClockRun) {
        if self.stack_audit.is_none() {
            self.time_audio = run.audio;
            self.time_frames += run.frames;
            return;
        }
        self.logic_trace.get_mut().append(run.logic_trace);
        let stack = self.stack_audit.as_mut().expect("enabled audit");
        for (pc, access) in run.accesses {
            stack.observe(pc, access, self.audit.get_mut());
        }
        self.time_audio = run.audio;
        self.time_frames += run.frames;
    }

    pub(crate) fn enable_stack_audit(&mut self, stack: StackAuditState, audit: AtomicRun) {
        self.stack_audit = Some(stack);
        *self.audit.get_mut() = audit;
    }

    /// Every operation that can tick/flush the bus consumes its observations
    /// before returning. Consequently a following `&self` RAM read sees the
    /// actual preceding commits without another queue or initialized bitmap.
    fn flush_audit_accesses(&mut self) {
        if let Some(stack) = &mut self.stack_audit {
            let pc = self.emulator.hybrid_cpu().pc();
            stack.drain(self.emulator.hybrid_bus_mut(), pc, self.audit.get_mut());
        }
    }

    fn record_memory_read(&self, address: u32, word: bool, purpose: ReadPurpose, value: u16) {
        if let Some(stack) = &self.stack_audit {
            stack.read(
                self.emulator.hybrid_cpu().pc(),
                genesis_core::HybridAccess::Read {
                    address: address & 0xffffff,
                    word,
                    value,
                    purpose,
                },
                &mut self.audit.borrow_mut(),
            );
        }
        if address & 0xffffff >= 0xe00000 && self.stack_audit.is_some() && self.timed_execution {
            self.logic_trace.borrow_mut().access(
                self.emulator.hybrid_cpu().pc(),
                self.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
                genesis_core::HybridAccess::Read {
                    address: address & 0xffffff,
                    word,
                    value,
                    purpose,
                },
                false,
            );
        }
    }

    fn read_bus_tagged(&mut self, address: u32, word: bool, purpose: ReadPurpose) -> u16 {
        self.emulator.hybrid_bus_mut().apply_pending_writes();
        self.read_prepared_bus_tagged(address, word, purpose, false)
    }

    fn read_prepared_bus_tagged(
        &mut self,
        address: u32,
        word: bool,
        purpose: ReadPurpose,
        exact_operand: bool,
    ) -> u16 {
        let bus = self.emulator.hybrid_bus_mut();
        let full = if word { 0xffff } else { 0xff };
        let driven = if purpose == ReadPurpose::Semantic && !exact_operand {
            crate::open_bus::driven_mask(bus, address, word)
        } else {
            full
        };
        let value = if word {
            bus.read_word_tagged(address, purpose)
        } else {
            u16::from(bus.read_byte_tagged(address, purpose))
        };
        self.flush_audit_accesses();
        self.record_logic_operand(
            genesis_core::HybridAccess::Read {
                address: address & 0xffffff,
                word,
                value,
                purpose,
            },
            exact_operand,
        );
        if driven != full {
            self.latch_mmio_error(
                tje_game::open_bus::OpenBusError::SourceRequired {
                    address: address & tje_mem::ADDRESS_MASK,
                    word,
                }
                .into(),
            );
        }
        value
    }

    fn read_bus_with_open_bus(
        &mut self,
        address: u32,
        word: bool,
        source: tje_game::open_bus::OpenBusSource,
    ) -> tje_game::Result<u16> {
        tje_game::Platform::check_mmio_contract(self)?;
        let prepared = crate::open_bus::prepare(self.emulator, address, word, source);
        let exact_trace_index = match prepared {
            Ok(bus) => bus.hybrid_accesses.as_ref().map(Vec::len),
            Err(error) => return Err(self.latch_mmio_error(error.into())),
        };
        let value = self.read_prepared_bus_tagged(address, word, ReadPurpose::Semantic, true);
        if self.stack_audit.is_some()
            && let Some(index) = exact_trace_index
        {
            self.exact_device_reads.push(index);
        }
        tje_game::Platform::check_mmio_contract(self)?;
        Ok(value)
    }
    fn record_logic_access(&mut self, access: genesis_core::HybridAccess) {
        self.record_logic_operand(access, false);
    }

    fn record_logic_operand(&mut self, access: genesis_core::HybridAccess, exact_operand: bool) {
        if self.stack_audit.is_some() && self.timed_execution {
            self.logic_trace.get_mut().access_with_exact(
                self.emulator.hybrid_cpu().pc(),
                self.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
                access,
                false,
                exact_operand,
            );
        }
    }

    /// Side-effect-free ROM/RAM read. For hardware reads use `bus()` so that
    /// read side effects and open-bus behavior remain explicit.
    fn peek_u8(&self, address: u32) -> u8 {
        let address = address & 0xFF_FFFF;
        let bus = self.emulator.hybrid_bus();
        match address {
            0..=0x3F_FFFF => bus
                .cartridge
                .as_ref()
                .expect("cartridge")
                .peek_word(address)
                .to_be_bytes()[(address & 1) as usize],
            0xE00000..=0xFFFFFF => bus.memory.read_main_ram_byte(address),
            _ => panic!("read_u8 is ROM/RAM-only; use bus() for hardware address {address:06X}"),
        }
    }
    pub fn read_u8(&self, address: u32) -> u8 {
        let value = self.peek_u8(address);
        self.record_memory_read(address, false, ReadPurpose::Semantic, u16::from(value));
        value
    }
    pub fn read_u16(&self, address: u32) -> u16 {
        let value =
            u16::from_be_bytes([self.peek_u8(address), self.peek_u8(address.wrapping_add(1))]);
        self.record_memory_read(address, true, ReadPurpose::Semantic, value);
        value
    }
    pub fn read_u32(&self, address: u32) -> u32 {
        (u32::from(self.read_u16(address)) << 16)
            | u32::from(self.read_u16(address.wrapping_add(2)))
    }
    pub fn write_u8(&mut self, address: u32, value: u8) {
        self.flush_audit_accesses();
        let address = address & 0xFF_FFFF;
        if address >= 0xe00000 {
            self.record_logic_access(genesis_core::HybridAccess::Write {
                address,
                word: false,
                value: u16::from(value),
            });
        }
        let bus = self.emulator.hybrid_bus_mut();
        match address {
            0..=0x3F_FFFF => {} // Cartridge ROM is immutable.
            0xE00000..=0xFFFFFF => {
                bus.memory.write_main_ram_byte(address, value);
                if let Some(stack) = &mut self.stack_audit {
                    stack.observe(
                        self.emulator.hybrid_cpu().pc(),
                        genesis_core::HybridAccess::Write {
                            address,
                            word: false,
                            value: u16::from(value),
                        },
                        self.audit.get_mut(),
                    );
                }
            }
            _ => panic!("write_u8 is ROM/RAM-only; use bus() for hardware address {address:06X}"),
        }
    }
    pub fn write_u16(&mut self, address: u32, value: u16) {
        let [hi, lo] = value.to_be_bytes();
        self.write_u8(address, hi);
        self.write_u8(address.wrapping_add(1), lo);
    }
    pub fn write_u32(&mut self, address: u32, value: u32) {
        self.write_u16(address, (value >> 16) as u16);
        self.write_u16(address.wrapping_add(2), value as u16);
    }
    pub fn argument_u32(&self, index: u32) -> u32 {
        self.frame_state.argument_slot(index as i32).read(self)
    }
    pub fn argument_i16(&self, index: u32) -> i16 {
        self.argument_u32(index) as i16
    }

    /// Ordered immediate CPU-bus facade. Unlike the emulator's instruction-local
    /// queues, consecutive Rust byte/word accesses are never regrouped by width.
    pub fn bus(&mut self) -> HardwareBus<'_, 'a> {
        HardwareBus { context: self }
    }
    pub fn cpu(&mut self) -> &mut M68000 {
        self.emulator.hybrid_parts().0
    }

    /// Execute an untranslated MCC68K function, bypassing the Rust registry.
    /// Push arguments right-to-left, run to an unexecuted sentinel with matching
    /// SP (so nested RTS cannot end the call), then perform caller cleanup.
    /// This atomic call is for functions that do not wait for peripheral time.
    pub fn call_original(&mut self, address: u32, arguments: &[u32]) -> Result<u32> {
        let id = tje_game::function_at(self.revision, address).ok_or_else(|| {
            HostError::Metadata(format!("no reviewed call contract at {address:06X}"))
        })?;
        let class = tje_game::execution_class(id).ok_or_else(|| {
            HostError::Metadata(format!(
                "{} has no reviewed execution class",
                id.metadata().name
            ))
        })?;
        self.call_original_with_execution_contract(id.metadata(), class, arguments)
    }

    /// Explicit contracts also support authored, ROM-free machine-code fixtures.
    pub fn call_original_with_contract(
        &mut self,
        contract: &tje_game::FunctionMetadata,
        atomic_capable: bool,
        arguments: &[u32],
    ) -> Result<u32> {
        if !atomic_capable {
            return Err(HostError::NotAtomicCapable {
                function: contract.name.to_owned(),
            });
        }
        self.call_original_with_execution_contract(contract, ExecutionClass::Atomic, arguments)
    }

    pub fn call_original_with_execution_contract(
        &mut self,
        contract: &tje_game::FunctionMetadata,
        execution_class: ExecutionClass,
        arguments: &[u32],
    ) -> Result<u32> {
        tje_game::Platform::check_mmio_contract(self)?;
        if tje_game::exits::contract(contract.id) != tje_game::exits::ExitContract::Rts {
            return Err(HostError::Metadata(format!(
                "{} has a non-RTS exit; ordinary original fallback cannot unwind it",
                contract.name,
            )));
        }
        if !self.execution_class.permits(execution_class) {
            return Err(HostError::Metadata(format!(
                "{} requires {execution_class:?}, unavailable inside {:?}",
                contract.name, self.execution_class
            )));
        }
        let address = contract.address(self.revision);
        self.emulator.hybrid_bus_mut().apply_pending_writes();
        self.flush_audit_accesses();
        let saved_sp = self.emulator.hybrid_cpu().stack_pointer();
        let saved_pc = self.emulator.hybrid_cpu().pc();
        let mut sp = saved_sp;
        for &argument in arguments.iter().rev() {
            sp = sp.wrapping_sub(4);
            self.write_u32(sp, argument);
        }
        let cleanup_sp = sp;
        sp = sp.wrapping_sub(4);
        self.write_u32(sp, CALL_SENTINEL);
        let (cpu, bus) = self.emulator.hybrid_parts();
        set_sp(cpu, sp);
        cpu.set_pc(address, bus);
        self.flush_audit_accesses();
        let audit = if self.timed_execution {
            let clock_run = run_original_timed_with_audio(
                self.emulator,
                &mut self.wait_clock,
                CALL_SENTINEL,
                cleanup_sp,
                std::mem::take(&mut self.time_audio),
                self.stack_audit.is_some(),
            )?;
            self.merge_clock_run(clock_run);
            AtomicRun::default()
        } else if let Some(stack) = &mut self.stack_audit {
            run_original_atomic_scoped(
                self.emulator,
                ReferenceBoundary::returned(CALL_SENTINEL, cleanup_sp),
                stack,
            )?
        } else {
            run_original_atomic(self.emulator, CALL_SENTINEL, cleanup_sp, false)?
        };
        if self.stack_audit.is_some() {
            self.audit.get_mut().append(audit);
        }
        self.nested_calls += 1;
        let (cpu, bus) = self.emulator.hybrid_parts();
        let result = cpu.data_registers()[0];
        set_sp(cpu, saved_sp);
        cpu.set_pc(saved_pc, bus);
        self.flush_audit_accesses();
        tje_game::Platform::check_mmio_contract(self)?;
        Ok(result)
    }

    /// Run an original with the caller's exact outgoing area/return PC already
    /// present. Unlike call_original_with_execution_contract, push no new slots.
    fn call_original_prepared(&mut self, id: FunctionId, class: ExecutionClass) -> Result<u32> {
        tje_game::Platform::check_mmio_contract(self)?;
        if tje_game::exits::contract(id) != tje_game::exits::ExitContract::Rts {
            return Err(HostError::Metadata(format!(
                "{} has a non-RTS exit; prepared original fallback cannot unwind it",
                id.metadata().name,
            )));
        }
        if !self.execution_class.permits(class) {
            return Err(HostError::Metadata(format!(
                "{class:?} original unavailable in {:?}",
                self.execution_class
            )));
        }
        let entry_sp = self.emulator.hybrid_cpu().stack_pointer();
        let return_pc = self.read_u32(entry_sp);
        let return_sp = entry_sp.wrapping_add(4);
        let (cpu, bus) = self.emulator.hybrid_parts();
        bus.apply_pending_writes();
        cpu.set_pc(id.metadata().address(self.revision), bus);
        self.flush_audit_accesses();
        let audit = if self.timed_execution {
            let run = run_original_timed_with_audio(
                self.emulator,
                &mut self.wait_clock,
                return_pc,
                return_sp,
                std::mem::take(&mut self.time_audio),
                self.stack_audit.is_some(),
            )?;
            self.merge_clock_run(run);
            AtomicRun::default()
        } else if let Some(stack) = &mut self.stack_audit {
            run_original_atomic_scoped(
                self.emulator,
                ReferenceBoundary::returned(return_pc, return_sp),
                stack,
            )?
        } else {
            run_original_atomic(self.emulator, return_pc, return_sp, false)?
        };
        if self.stack_audit.is_some() {
            self.audit.get_mut().append(audit);
        }
        self.nested_calls += 1;
        tje_game::Platform::check_mmio_contract(self)?;
        Ok(self.emulator.hybrid_cpu().data_registers()[0])
    }

    pub(crate) fn return_to_caller(&mut self, value: Option<u32>) -> Result<()> {
        tje_game::Platform::check_mmio_contract(self)?;
        let sp = self.emulator.hybrid_cpu().stack_pointer();
        let return_pc = self.read_u32(sp);
        let (cpu, bus) = self.emulator.hybrid_parts();
        if let Some(value) = value {
            let mut registers = cpu.data_registers();
            registers[0] = value;
            cpu.set_data_registers(registers);
        }
        set_sp(cpu, sp.wrapping_add(4));
        cpu.set_pc(return_pc, bus);
        bus.apply_pending_writes();
        self.flush_audit_accesses();
        Ok(())
    }

    pub(crate) fn transfer_to(&mut self, pc: u32) -> Result<()> {
        tje_game::Platform::check_mmio_contract(self)?;
        let (cpu, bus) = self.emulator.hybrid_parts();
        cpu.set_pc(pc, bus);
        bus.apply_pending_writes();
        self.flush_audit_accesses();
        Ok(())
    }

    pub(crate) fn exception_returned(&mut self) {
        self.emulator.hybrid_parts().0.hybrid_exception_returned();
    }
}

pub struct HardwareBus<'context, 'emulator> {
    context: &'context mut HookContext<'emulator>,
}
impl BusInterface for HardwareBus<'_, '_> {
    type DebugView<'a>
        = DummyM68000Debugger
    where
        Self: 'a;
    fn read_byte(&mut self, address: u32) -> u8 {
        self.read_byte_tagged(address, ReadPurpose::Semantic)
    }
    fn read_word(&mut self, address: u32) -> u16 {
        self.read_word_tagged(address, ReadPurpose::Semantic)
    }
    fn read_byte_tagged(&mut self, address: u32, purpose: ReadPurpose) -> u8 {
        self.context.read_bus_tagged(address, false, purpose) as u8
    }
    fn read_word_tagged(&mut self, address: u32, purpose: ReadPurpose) -> u16 {
        self.context.read_bus_tagged(address, true, purpose)
    }
    fn write_byte(&mut self, address: u32, value: u8) {
        let physical = address & 0xFFFFFF;
        if physical >= 0xE00000 || physical <= 0x3FFFFF {
            self.context.write_u8(address, value);
        } else {
            tje_game::Platform::write_device_u8(self.context, address, value);
        }
    }
    fn write_word(&mut self, address: u32, value: u16) {
        let physical = address & 0xFFFFFF;
        if physical >= 0xE00000 || physical <= 0x3FFFFF {
            self.context.write_u16(address, value);
        } else {
            tje_game::Platform::write_device_u16(self.context, address, value);
        }
    }
    fn interrupt_level(&self) -> u8 {
        self.context.emulator.hybrid_bus().interrupt_level()
    }
    fn acknowledge_interrupt(&mut self, level: u8) {
        self.context
            .emulator
            .hybrid_bus_mut()
            .acknowledge_interrupt(level);
    }
    fn halt(&self) -> bool {
        self.context.emulator.hybrid_bus().halt()
    }
    fn reset(&self) -> bool {
        <GenesisBus as BusInterface>::reset(self.context.emulator.hybrid_bus())
    }
}

impl tje_mem::Bus for HookContext<'_> {
    fn read_u8(&self, address: u32) -> u8 {
        HookContext::read_u8(self, address)
    }
    fn read_u16(&self, address: u32) -> u16 {
        HookContext::read_u16(self, address)
    }
    fn read_u32(&self, address: u32) -> u32 {
        HookContext::read_u32(self, address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        HookContext::write_u8(self, address, value);
    }
}
impl tje_mem::Platform for HookContext<'_> {
    fn write_vdp_control(&mut self, value: u16) {
        tje_game::Platform::write_device_u16(self, 0xC00004, value);
    }
    fn write_vdp_data(&mut self, value: u16) {
        tje_game::Platform::write_device_u16(self, 0xC00000, value);
    }
    fn write_vdp_command(&mut self, value: u32) {
        // MOVE.L to the command port uses its two word-address aliases.
        tje_game::Platform::write_device_u16(self, 0xC00004, (value >> 16) as u16);
        tje_game::Platform::write_device_u16(self, 0xC00006, value as u16);
    }
}
impl tje_game::Platform for HookContext<'_> {
    fn check_mmio_contract(&self) -> tje_game::Result<()> {
        if let Some(error) = &self.mmio_contract_error {
            return Err(error.clone());
        }
        if let Some(address) = self.emulator.hybrid_bus().hybrid_functional_violation {
            return Err(tje_game::Error::Platform(format!(
                "functional contract observed VDP status/HV at {address:06X}"
            )));
        }
        Ok(())
    }

    fn discard_z80_bus_request_read(&mut self) -> tje_game::Result<()> {
        self.check_mmio_contract()?;
        self.read_bus_tagged(0xa11100, true, ReadPurpose::Discarded);
        self.check_mmio_contract()
    }

    fn wait_until(&mut self, request: WaitRequest) -> tje_game::Result<()> {
        self.check_mmio_contract()?;
        self.flush_audit_accesses();
        let result = (|| {
            if self.execution_class == ExecutionClass::Atomic {
                return Err(tje_game::Error::Platform(
                    "an atomic function cannot wait".into(),
                ));
            }
            if self.execution_class == ExecutionClass::Functional {
                if !crate::execution::functional_wait_supported(request) {
                    return Err(tje_game::Error::Platform(
                        "Functional admits only reviewed YM/BUSACK readiness".into(),
                    ));
                }
                if self.emulator.hybrid_bus().hybrid_functional_mode() {
                    return crate::execution::discharge_functional_wait(self.emulator, request)
                        .map_err(Self::game_platform_error);
                }
            }
            let observe = self.stack_audit.is_some();
            let result = if observe && self.time_audio.is_empty() {
                run_wait(self.emulator, &mut self.wait_clock, request)
            } else {
                run_wait_with_audio(
                    self.emulator,
                    &mut self.wait_clock,
                    request,
                    std::mem::take(&mut self.time_audio),
                    observe,
                )
            };
            let run = result.map_err(Self::game_platform_error)?;
            self.merge_clock_run(run);
            Ok(())
        })();
        self.flush_audit_accesses();
        result.map_err(|error| self.latch_mmio_error(error))
    }

    fn read_device_u8(&mut self, address: u32) -> u8 {
        self.read_bus_tagged(address, false, ReadPurpose::Semantic) as u8
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        self.read_bus_tagged(address, true, ReadPurpose::Semantic)
    }
    fn read_device_u8_with_open_bus(
        &mut self,
        address: u32,
        source: tje_game::open_bus::OpenBusSource,
    ) -> tje_game::Result<u8> {
        self.read_bus_with_open_bus(address, false, source)
            .map(|value| value as u8)
    }

    fn read_device_u16_with_open_bus(
        &mut self,
        address: u32,
        source: tje_game::open_bus::OpenBusSource,
    ) -> tje_game::Result<u16> {
        self.read_bus_with_open_bus(address, true, source)
    }

    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.record_logic_access(genesis_core::HybridAccess::Write {
            address: address & 0xffffff,
            word: false,
            value: u16::from(value),
        });
        let bus = self.emulator.hybrid_bus_mut();
        bus.write_byte(address, value);
        bus.apply_pending_writes();
        self.flush_audit_accesses();
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.record_logic_access(genesis_core::HybridAccess::Write {
            address: address & 0xffffff,
            word: true,
            value,
        });
        let bus = self.emulator.hybrid_bus_mut();
        bus.write_word(address, value);
        bus.apply_pending_writes();
        self.flush_audit_accesses();
    }
}
impl tje_game::Context for HookContext<'_> {
    fn revision(&self) -> Revision {
        self.revision
    }
    fn data_register(&self, index: usize) -> u32 {
        self.emulator.hybrid_cpu().data_registers()[index]
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        let mut registers = self.emulator.hybrid_cpu().data_registers();
        registers[index] = value;
        self.emulator.hybrid_parts().0.set_data_registers(registers);
    }
    fn address_register(&self, index: usize) -> u32 {
        self.emulator.hybrid_cpu().address_registers()[index]
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        let cpu = self.emulator.hybrid_parts().0;
        let mut addresses = cpu.address_registers();
        addresses[index] = value;
        cpu.set_address_registers(
            addresses,
            cpu.user_stack_pointer(),
            cpu.supervisor_stack_pointer(),
        );
    }
    fn status_register(&self) -> u16 {
        self.emulator.hybrid_cpu().status_register()
    }
    fn set_status_register(&mut self, value: u16) {
        self.emulator.hybrid_parts().0.set_status_register(value);
    }
    fn stack_pointer(&self) -> tje_game::Result<u32> {
        Ok(self.emulator.hybrid_cpu().stack_pointer())
    }
    fn set_stack_pointer(&mut self, value: u32) -> tje_game::Result<()> {
        set_sp(self.emulator.hybrid_parts().0, value);
        Ok(())
    }
    fn frame_state(&self) -> Option<FrameState> {
        Some(self.frame_state)
    }
    fn set_frame_state(&mut self, state: FrameState) -> tje_game::Result<()> {
        self.frame_state = state;
        Ok(())
    }
    fn call_original_entry(
        &mut self,
        id: FunctionId,
        class: ExecutionClass,
    ) -> tje_game::Result<u32> {
        self.call_original_prepared(id, class)
            .map_err(|error| tje_game::Error::Platform(error.to_string()))
    }
    fn stack_banks(&self) -> tje_game::Result<tje_game::exits::StackBanks> {
        let cpu = self.emulator.hybrid_cpu();
        Ok(tje_game::exits::StackBanks {
            user: cpu.user_stack_pointer(),
            supervisor: cpu.supervisor_stack_pointer(),
        })
    }
    fn set_stack_banks(&mut self, banks: tje_game::exits::StackBanks) -> tje_game::Result<()> {
        let cpu = self.emulator.hybrid_parts().0;
        cpu.set_address_registers(cpu.address_registers(), banks.user, banks.supervisor);
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        self.dispatcher
    }
    fn execution_class(&self) -> ExecutionClass {
        self.execution_class
    }
    fn set_execution_class(&mut self, class: ExecutionClass) -> tje_game::Result<()> {
        if !self.execution_ceiling.permits(class) {
            return Err(tje_game::Error::ExecutionUnavailable {
                caller: self.execution_ceiling,
                callee: class,
            });
        }
        self.execution_class = class;
        Ok(())
    }
    fn call_original(&mut self, id: FunctionId, arguments: &[u32]) -> tje_game::Result<u32> {
        HookContext::call_original(self, id.metadata().address(self.revision), arguments)
            .map_err(|error| tje_game::Error::Platform(error.to_string()))
    }
    fn call_original_with_execution(
        &mut self,
        id: FunctionId,
        arguments: &[u32],
        class: ExecutionClass,
    ) -> tje_game::Result<u32> {
        self.call_original_with_execution_contract(id.metadata(), class, arguments)
            .map_err(|error| tje_game::Error::Platform(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genesis_config::GenesisEmulatorConfig;
    use jgenesis_common::frontend::TimingMode;

    #[test]
    fn stack_audit_records_reads_before_writes_not_caller_arguments() {
        let mut bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        bus.hybrid_cpu_accesses = Some(Vec::new());
        let mut state = StackAuditState::new(0xEF7FF0, false);
        let mut audit = state.empty_run();
        bus.read_word(0xFF7FEC);
        state.drain(&mut bus, 0x1234, &mut audit);
        assert_eq!(audit.uninitialized_reads.len(), 2);
        audit.uninitialized_reads.clear();
        bus.write_word(0xFF7FEC, 0x4321);
        // A queued write has not initialized RAM for another real read.
        bus.read_word(0xFF7FEC);
        state.drain(&mut bus, 0x1234, &mut audit);
        assert_eq!(audit.uninitialized_reads.len(), 2);
        audit.uninitialized_reads.clear();
        bus.apply_pending_writes();
        bus.read_word(0xFF7FEC);
        bus.read_word(0xFF7FF0); // caller return address: not a callee local
        bus.read_word(0xFF8000); // allocated globals: not stack
        state.drain(&mut bus, 0x1234, &mut audit);
        assert!(audit.uninitialized_reads.is_empty());
        bus.read_word(0xE07FEC); // mirrors share the root's physical bitmap
        state.drain(&mut bus, 0x1234, &mut audit);
        assert!(audit.uninitialized_reads.is_empty());
        bus.read_byte(0xE07FEB);
        state.drain(&mut bus, 0x1234, &mut audit);
        assert_eq!(
            audit.uninitialized_reads.iter().next().unwrap().address,
            0xFF7FEB
        );
        audit.uninitialized_reads.clear();
        bus.read_byte(0xFFFF7FEE); // 24-bit wrapping is still stack
        state.drain(&mut bus, 0x1234, &mut audit);
        assert_eq!(
            audit.uninitialized_reads.into_iter().collect::<Vec<_>>(),
            [UninitializedStackRead {
                pc: 0x1234,
                address: 0xFF7FEE
            }]
        );
    }
}
