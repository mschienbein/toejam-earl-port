//! MCC68K C ABI: caller-owned 32-bit argument slots, D0 result, D2-D7 and
//! A2-A6 preserved. Short/byte arguments occupy the low part of their slot.
//! Default atomic calls freeze clocks; explicitly registered functional checks
//! quotient device latency, and time-aware calls run real clocks/interrupts.
//! Check-mode live execution retains the original instruction timing.

use crate::execution::{
    ClockRun, WaitClock, run_original_timed_with_audio, run_poll_yield_with_audio, run_wait,
    run_wait_with_audio,
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
use tje_game::Context as _;
use tje_game::call_frame::FrameState;
use tje_game::execution::{ExecutionClass, WaitRequest};
use tje_game::{Dispatcher, FunctionId};
use tje_rom::Revision;

mod return_state;
pub(crate) use return_state::ReturnState;

pub(crate) fn canonical_ram_address(address: u32) -> u32 {
    let physical = address & 0xFF_FFFF;
    if physical >= 0xE00000 {
        0xFF0000 | (physical & 0xFFFF)
    } else {
        physical
    }
}

/// Observation only: no semantic read, stack permission or admission evidence.
/// Keep hardware reads on the explicit bus path, with their real side effects.
pub(crate) fn peek_u8(emulator: &GenesisEmulator, address: u32) -> u8 {
    let address = address & 0xFF_FFFF;
    let bus = emulator.hybrid_bus();
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
    pub strict_control: Option<crate::dependent_equivalence::ControlLog>,
    pub explicit_stack_reads: Vec<crate::readable_stack::ExplicitRead>,
    pub read_ordinal: usize,
    pub unwritten_stack_reads: Vec<crate::dependent_equivalence::journal::JournalRead>,
    pub admitted_stack_inputs: Vec<crate::readable_stack::AdmittedRead>,
    pub invocation_scope: Option<std::sync::Arc<crate::readable_stack::OriginalRootScope>>,
    pub(crate) frozen_stack_alignment: Option<crate::replayed_environment::FrozenStackAlignment>,
}

impl AtomicRun {
    pub(crate) fn append(&mut self, other: Self) {
        // A merged execution is not this Rust Context's observed assignment
        // stream. Never adopt another fork's witness or infer its intermediate SP.
        if let Some(alignment) = &mut self.frozen_stack_alignment {
            alignment.poison();
        }
        if let Some(scope) = other.invocation_scope {
            if let Some(current) = &self.invocation_scope {
                if !std::sync::Arc::ptr_eq(current, &scope) {
                    current.reject_capture();
                    scope.reject_capture();
                }
            } else {
                self.invocation_scope = Some(scope);
            }
        }
        let prior_reads = self.stack_reads.as_ref().map_or(0, Vec::len);
        let prior_ordinal = self.read_ordinal;
        self.read_ordinal += other.read_ordinal;
        self.unwritten_stack_reads
            .extend(other.unwritten_stack_reads.into_iter().map(|mut read| {
                read.ordinal += prior_ordinal;
                read
            }));
        self.admitted_stack_inputs
            .extend(other.admitted_stack_inputs.into_iter().map(|mut read| {
                read.token.ordinal += prior_ordinal;
                read
            }));
        self.explicit_stack_reads
            .extend(other.explicit_stack_reads.into_iter().map(|mut read| {
                read.first_read += prior_reads;
                for ordinal in &mut read.ordinals {
                    *ordinal += prior_ordinal;
                }
                read
            }));
        self.instructions += other.instructions;
        self.cycles += other.cycles;
        self.uninitialized_reads.extend(other.uninitialized_reads);
        self.writes.extend(other.writes);
        if let Some(reads) = other.stack_reads {
            self.stack_reads
                .get_or_insert_with(Vec::new)
                .extend(reads.into_iter().map(|mut read| {
                    read.ordinal += prior_ordinal;
                    read
                }));
        }
        if let Some(control) = other.strict_control {
            self.strict_control
                .get_or_insert_with(Default::default)
                .append(control);
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
    fn observe_exception_boundary(&mut self, boundary: m68000_emu::ExceptionBoundary) {
        self.inner.observe_exception_boundary(boundary);
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
    // The owner remains local until every Result cleanup has restored it.
    let capture = stack.capture.take();
    let capture_observer = capture
        .as_deref()
        .map(|capture| (capture, capture.entry_frame()));
    let readable_observer = stack.readable_stack.clone();
    let original_root = stack.original_root.clone();
    let (cpu, bus) = emulator.hybrid_parts();
    let owns_trace = bus.hybrid_cpu_accesses.is_none();
    if owns_trace {
        bus.hybrid_cpu_accesses = Some(Vec::new());
    }
    let owns_boundaries =
        (stack.provenance.is_some() || capture.is_some() || original_root.is_some())
            && bus.hybrid_exception_boundaries.is_none();
    if owns_boundaries {
        bus.hybrid_exception_boundaries = Some(Vec::new());
    }
    let previous_counting = (capture.is_some() || original_root.is_some()).then(|| {
        let previous = cpu
            .hybrid_instruction_counts()
            .map(|counts| counts.enabled());
        if previous.is_none() {
            cpu.hybrid_enable_instruction_counts();
        } else {
            cpu.hybrid_set_instruction_counting(true);
        }
        previous
    });
    let mut observed = stack.empty_run();
    if let Some((capture, frame)) = capture_observer {
        capture.observe_native_batch_parts(cpu, bus, frame);
    }
    stack.drain(bus, cpu.pc(), &mut observed);
    let mut audit = StackAuditBus { inner: bus };
    let attempted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut execute = || {
            run_atomic_loop(cpu, &mut audit, boundary, |cpu, audit| {
                let pc = cpu.pc();
                if let Some(root) = &original_root {
                    root.before_step(cpu, cpu.hybrid_instruction_ready(audit));
                }
                if let Some(branch) = &stack.provenance {
                    branch.begin_cpu_step(cpu, audit.inner.cycles.m68k_mclk_cycles);
                }
                if let Some((capture, frame)) = capture_observer {
                    capture.begin_cpu_step_parts(cpu, audit.inner, frame);
                }
                let _original_segment = readable_observer
                    .as_ref()
                    .map(|observer| observer.guard.original_segment());
                let cycles = cpu.execute_instruction(audit);
                audit.inner.apply_pending_writes();
                if let Some(root) = &original_root {
                    root.after_step(cpu, audit.inner);
                }
                if let Some((capture, frame)) = capture_observer {
                    capture.finish_reference_cpu_step_parts(cpu, audit.inner, frame);
                    capture.observe_cpu_batch_parts(cpu, audit.inner, frame);
                }
                stack.drain(audit.inner, pc, &mut observed);
                if let Some(root) = &original_root {
                    root.finish_step(cpu, audit.inner);
                }
                cycles
            })
        };
        if let Some(observer) = &readable_observer {
            observer
                .guard
                .catch_abort(execute)
                .map_err(|error| HostError::Translation(error.into()))
                .and_then(|result| result)
        } else {
            execute()
        }
    }));
    stack.drain(audit.inner, cpu.pc(), &mut observed);
    if owns_trace {
        audit.inner.hybrid_cpu_accesses = None;
    }
    if owns_boundaries {
        audit.inner.hybrid_exception_boundaries = None;
    }
    if let Some(previous) = previous_counting {
        cpu.hybrid_set_instruction_counting(previous.unwrap_or(false));
    }
    stack.capture = capture;
    let mut result = match attempted {
        Ok(result) => result?,
        Err(payload) => std::panic::resume_unwind(payload),
    };
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
    pub(crate) provenance: Option<crate::future_frame_watch::BranchProvenance>,
    pub(crate) capture: Option<Box<crate::check_capture::RawCheckCapture>>,
    pub(crate) readable_stack: Option<std::sync::Arc<crate::readable_stack::Observer>>,
    pub(crate) original_root: Option<std::sync::Arc<crate::readable_stack::OriginalRootScope>>,
    capture_full_ram: bool,
    full_written: Option<Box<tje_mem::written_ram::WrittenRam>>,
}

impl StackAuditState {
    pub(crate) fn new(entry_sp: u32, capture_reads: bool) -> Self {
        Self {
            entry_sp: canonical_ram_address(entry_sp),
            written: [0; 512],
            capture_reads,
            provenance: None,
            capture: None,
            readable_stack: None,
            original_root: None,
            capture_full_ram: false,
            full_written: None,
        }
    }

    pub(crate) fn capture_explicit_inputs(&mut self) {
        self.capture_reads = true;
        self.capture_full_ram = true;
        self.full_written = Some(Box::default());
    }

    pub(crate) fn physical_observers(&self) -> crate::execution::PhysicalObservers<'_> {
        crate::execution::PhysicalObservers {
            provenance: self.provenance.as_ref(),
            capture: self.capture.as_deref(),
            readable: self.readable_stack.as_deref(),
            original_root: self.original_root.as_deref(),
        }
    }

    pub(crate) fn empty_run(&self) -> AtomicRun {
        AtomicRun {
            stack_reads: self.capture_reads.then(Vec::new),
            invocation_scope: self.original_root.clone(),
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
        if let Some(written) = &self.full_written {
            return (0..if word { 2 } else { 1 }).fold(0, |mask, lane| {
                let raw = address.wrapping_add(lane);
                let unwritten = tje_mem::written_ram::canonical_ram_byte(raw)
                    .is_some_and(|physical| physical < self.entry_sp && !written.get(physical));
                mask | (u8::from(unwritten) << lane)
            });
        }
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

    fn read(
        &self,
        pc: u32,
        access: genesis_core::HybridAccess,
        audit: &mut AtomicRun,
        original_root: Option<crate::readable_stack::OriginalRootWitness>,
    ) {
        let (address, word, semantic) = match access {
            genesis_core::HybridAccess::Read {
                address,
                word,
                purpose,
                ..
            } => (address, word, purpose == ReadPurpose::Semantic),
            genesis_core::HybridAccess::DmaRead { address, .. } => (address, true, true),
            genesis_core::HybridAccess::Write { .. }
            | genesis_core::HybridAccess::Z80Write { .. }
            | genesis_core::HybridAccess::HostMutation { .. }
            | genesis_core::HybridAccess::ExceptionBoundary { .. } => {
                unreachable!("read observation")
            }
        };
        let mask = self.read_mask(address, word);
        audit.read_ordinal += 1;
        if semantic
            && mask != 0
            && let Some(mut read) =
                crate::dependent_equivalence::journal::JournalRead::new_work_ram(pc, access, mask)
        {
            read.ordinal = audit.read_ordinal;
            read.original_root = original_root;
            audit.unwritten_stack_reads.push(read);
        }
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
        if let Some(reads) = &mut audit.stack_reads {
            let read = if self.capture_full_ram {
                crate::dependent_equivalence::journal::JournalRead::new_work_ram(pc, access, mask)
            } else {
                crate::dependent_equivalence::journal::JournalRead::new(pc, access, mask)
            };
            if let Some(mut read) = read {
                read.ordinal = audit.read_ordinal;
                read.original_root = original_root;
                reads.push(read);
            }
        }
    }

    pub(crate) fn observe(
        &mut self,
        pc: u32,
        access: genesis_core::HybridAccess,
        audit: &mut AtomicRun,
    ) {
        let root = self.original_root.as_ref().and_then(|root| root.witness());
        self.observe_with_root(pc, access, audit, root);
    }
    fn observe_with_root(
        &mut self,
        pc: u32,
        access: genesis_core::HybridAccess,
        audit: &mut AtomicRun,
        original_root: Option<crate::readable_stack::OriginalRootWitness>,
    ) {
        if let Some(control) = &mut audit.strict_control {
            control.observe(access);
        }
        let (address, word, value) = match access {
            genesis_core::HybridAccess::Write {
                address,
                word,
                value,
            }
            | genesis_core::HybridAccess::Z80Write {
                address,
                word,
                value,
            }
            | genesis_core::HybridAccess::HostMutation {
                address,
                word,
                value,
            } => (address, word, value),
            genesis_core::HybridAccess::Read { .. }
            | genesis_core::HybridAccess::DmaRead { .. } => {
                self.read(pc, access, audit, original_root);
                return;
            }
            genesis_core::HybridAccess::ExceptionBoundary { .. } => return,
        };
        let address = address & 0xffffff;
        if address >= 0xe00000 {
            for offset in 0..if word { 2 } else { 1 } {
                let address = address.wrapping_add(offset);
                if let Some(written) = &mut self.full_written
                    && canonical_ram_address(address) < self.entry_sp
                {
                    written.set(address);
                }
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

    pub(crate) fn merge_timed_accesses(
        &mut self,
        accesses: Vec<(u32, genesis_core::HybridAccess)>,
        roots: Vec<(usize, crate::readable_stack::OriginalRootWitness)>,
        audit: &mut AtomicRun,
    ) {
        let mut roots = roots.into_iter().peekable();
        for (index, (pc, access)) in accesses.into_iter().enumerate() {
            let root = if roots.peek().is_some_and(|(at, _)| *at == index) {
                roots.next().map(|(_, root)| root)
            } else {
                None
            };
            self.observe_with_root(pc, access, audit, root);
        }
    }

    pub(crate) fn drain(&mut self, bus: &mut GenesisBus, pc: u32, audit: &mut AtomicRun) {
        let boundaries = bus
            .hybrid_exception_boundaries
            .as_deref()
            .unwrap_or_default();
        if let Some(accesses) = &mut bus.hybrid_cpu_accesses {
            for access in accesses.drain(..) {
                if let Some(root) = &self.original_root {
                    root.observe_stream(access, boundaries);
                }
                if let Some(branch) = &self.provenance {
                    branch.observe_stream(access, boundaries);
                }
                self.observe(pc, access, audit);
            }
        }
        if let Some(boundaries) = &mut bus.hybrid_exception_boundaries {
            boundaries.clear();
        }
    }
}

/// A Rust translation sees the actual emulated address space and stack arguments.
/// Explicit native frames reproduce observable emulated stack bytes when needed.
pub struct HookContext<'a> {
    emulator: &'a mut GenesisEmulator,
    pub nested_calls: u64,
    revision: Revision,
    mode: crate::Mode,
    dispatcher: Dispatcher,
    pub(crate) audit: RefCell<AtomicRun>,
    pub(crate) logic_trace: RefCell<LogicTrace>,
    pub(crate) exact_device_reads: Vec<usize>,
    mmio_contract_error: Option<tje_game::Error>,
    pub(crate) stack_audit: Option<StackAuditState>,
    observe_native_provenance: bool,
    frame_state: FrameState,
    pub(crate) execution_class: ExecutionClass,
    execution_ceiling: ExecutionClass,
    timed_execution: bool,
    pub(crate) wait_clock: WaitClock,
    pub(crate) time_audio: Vec<[f64; 2]>,
    pub time_frames: u64,
    timing_services: Option<&'a mut crate::timing_region::game_bridge::TimingServices>,
    timing_root_started: bool,
    readable_stack: Option<std::sync::Arc<crate::readable_stack::Observer>>,
}

impl<'a> HookContext<'a> {
    pub(crate) fn new(
        emulator: &'a mut GenesisEmulator,
        revision: Revision,
        dispatcher: Dispatcher,
        mode: crate::Mode,
    ) -> Self {
        let frame_state = FrameState::new(emulator.hybrid_cpu().stack_pointer());
        Self {
            emulator,
            revision,
            mode,
            dispatcher,
            audit: RefCell::new(AtomicRun::default()),
            logic_trace: RefCell::new(LogicTrace::default()),
            nested_calls: 0,
            exact_device_reads: Vec::new(),
            mmio_contract_error: None,
            stack_audit: None,
            observe_native_provenance: true,
            frame_state,
            execution_class: ExecutionClass::Atomic,
            execution_ceiling: ExecutionClass::Atomic,
            timed_execution: false,
            wait_clock: WaitClock::default(),
            time_audio: Vec::new(),
            time_frames: 0,
            timing_services: None,
            timing_root_started: false,
            readable_stack: None,
        }
    }
    pub(crate) fn with_timing_services(
        mut self,
        services: Option<&'a mut crate::timing_region::game_bridge::TimingServices>,
    ) -> Self {
        self.timing_services = services;
        self
    }

    pub(crate) fn with_inspection_provenance(
        mut self,
        provenance: Option<crate::future_frame_watch::BranchProvenance>,
    ) -> Self {
        if let Some(provenance) = provenance {
            let mut stack = StackAuditState::new(self.emulator.hybrid_cpu().stack_pointer(), false);
            stack.provenance = Some(provenance);
            self.enable_stack_audit(stack, AtomicRun::default());
        }
        self.observe_native_provenance = false;
        self
    }

    pub(crate) fn with_execution(mut self, class: ExecutionClass, clock: WaitClock) -> Self {
        self.execution_class = class;
        self.execution_ceiling = class;
        self.timed_execution = class == ExecutionClass::TimeAware
            || (class == ExecutionClass::Functional
                && !self.emulator.hybrid_bus().hybrid_functional_mode());
        self.wait_clock = clock;
        self.wait_clock.revision = Some(self.revision);
        self
    }

    pub(crate) fn with_check_route(mut self, route: crate::CheckForkRoute) -> Self {
        self.timed_execution = route.progresses_time();
        self
    }
    pub(crate) fn with_readable_stack(
        mut self,
        observer: std::sync::Arc<crate::readable_stack::Observer>,
    ) -> Self {
        self.readable_stack = Some(observer);
        self
    }

    pub fn functional_device_mode(&self) -> bool {
        self.emulator.hybrid_bus().hybrid_functional_mode()
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
            HostError::Clock(error) => tje_game::Error::Clock(error),
            other => tje_game::Error::Platform(other.to_string()),
        }
    }
    pub(crate) fn protect_host_timing_failure(&self, cause: HostError) -> HostError {
        let Some(services) = self.timing_services.as_ref() else {
            return cause;
        };
        match services.authority.completion.protected_entry() {
            Some(entry) => {
                services.authority.completion.fail();
                #[cfg(test)]
                crate::timing_region::game_bridge::proof::record_failure(
                    self.emulator,
                    self.frame_state,
                    self.wait_clock.frame,
                );
                tje_game::Error::timing_execution_failure(entry, Self::game_platform_error(cause))
                    .into()
            }
            None => cause,
        }
    }

    pub(crate) fn enter_call_ceiling_contract(
        &mut self,
        function: FunctionId,
        class: ExecutionClass,
        ceiling: tje_game::execution::CallCeiling,
    ) -> Result<tje_game::execution::CallCeilingScope> {
        if class == ExecutionClass::TimeAware && !self.timed_execution {
            return Err(HostError::Clock(
                tje_game::execution::CallClockError::Unavailable,
            ));
        }
        if !self.timing_root_started && self.timing_services.is_some() {
            if let Some(services) = self.timing_services.as_deref_mut() {
                let changed = services
                    .authority
                    .completion
                    .begin_root_scope()
                    .map_err(|error| {
                        HostError::Metadata(format!("timing root authority: {error}"))
                    });
                changed.map_err(|error| self.protect_host_timing_failure(error))?;
            }
            self.timing_root_started = true;
        }
        if class == ExecutionClass::Atomic
            && !self.timed_execution
            && !self.wait_clock.call_clock.active()
        {
            return Ok(tje_game::execution::CallCeilingScope::not_required());
        }
        self.wait_clock.enter_call_ceiling(function, ceiling)
    }

    pub(crate) fn leave_call_ceiling_contract(
        &mut self,
        scope: tje_game::execution::CallCeilingScope,
    ) -> Result<()> {
        self.wait_clock.leave_call_ceiling(scope).map(|_| ())
    }

    /// Merge real IRQ/recipe accesses into the existing active-frame audit.
    /// A wait must never reset initialized bytes or replace the frame identity.
    pub(crate) fn merge_clock_run(&mut self, run: ClockRun) {
        self.poison_frozen_stack_alignment();
        if self.stack_audit.is_none() {
            self.time_audio = run.audio;
            self.time_frames += run.frames;
            return;
        }
        self.logic_trace.get_mut().append(run.logic_trace);
        let stack = self.stack_audit.as_mut().expect("enabled audit");
        stack.merge_timed_accesses(run.accesses, run.original_root_reads, self.audit.get_mut());
        self.time_audio = run.audio;
        self.time_frames += run.frames;
    }

    pub(crate) fn enable_stack_audit(&mut self, stack: StackAuditState, audit: AtomicRun) {
        self.observe_native_provenance = true;
        self.readable_stack = stack.readable_stack.clone();
        self.stack_audit = Some(stack);
        *self.audit.get_mut() = audit;
    }

    /// Enabled only for a requested, admitted canonical Check analysis.
    pub(crate) fn observe_frozen_stack_alignment(
        &mut self,
        request: crate::replayed_environment::CandidateRequest,
    ) {
        if self.mode != crate::Mode::Check || self.timed_execution {
            return;
        }
        let alignment = &mut self.audit.get_mut().frozen_stack_alignment;
        if let Some(existing) = alignment {
            existing.poison();
        } else {
            *alignment = Some(crate::replayed_environment::FrozenStackAlignment::begin(
                request,
                self.emulator.hybrid_cpu(),
            ));
        }
    }
    fn poison_frozen_stack_alignment(&mut self) {
        if let Some(alignment) = &mut self.audit.get_mut().frozen_stack_alignment {
            alignment.poison();
        }
    }
    fn observe_stack_assignment(&mut self, value: u32) {
        if let Some(alignment) = &mut self.audit.get_mut().frozen_stack_alignment {
            alignment.attempt(value);
        }
    }
    fn observe_stack_banks(&mut self) {
        if let Some(alignment) = &mut self.audit.get_mut().frozen_stack_alignment {
            alignment.observe(self.emulator.hybrid_cpu());
        }
    }

    /// Every operation that can tick/flush the bus consumes its observations
    /// before returning. Consequently a following `&self` RAM read sees the
    /// actual preceding commits without another queue or initialized bitmap.
    fn flush_audit_accesses(&mut self) {
        if let Some(stack) = &mut self.stack_audit {
            let pc = self.emulator.hybrid_cpu().pc();
            if let Some(branch) = &stack.provenance {
                branch.begin_native(
                    self.wait_clock.frame,
                    self.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
                );
            }
            if let Some(capture) = &stack.capture {
                capture.observe_native_batch_parts(
                    self.emulator.hybrid_cpu(),
                    self.emulator.hybrid_bus(),
                    self.wait_clock.frame,
                );
            }
            stack.drain(self.emulator.hybrid_bus_mut(), pc, self.audit.get_mut());
        }
    }

    fn record_memory_read(&self, address: u32, word: bool, purpose: ReadPurpose, value: u16) {
        if !self.observe_native_provenance {
            return;
        }
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
                None,
            );
            if let Some(branch) = &stack.provenance {
                branch.observe_native(
                    genesis_core::HybridAccess::Read {
                        address: address & 0xffffff,
                        word,
                        value,
                        purpose,
                    },
                    self.wait_clock.frame,
                    self.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
                );
            }
            if let Some(capture) = &stack.capture {
                capture.observe_native(
                    genesis_core::HybridAccess::Read {
                        address: address & 0xffffff,
                        word,
                        value,
                        purpose,
                    },
                    crate::check_capture::AccessPoint::capture(
                        self.emulator,
                        self.wait_clock.frame,
                    ),
                    false,
                );
            }
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
        if exact_operand
            && let Some(capture) = self
                .stack_audit
                .as_ref()
                .and_then(|stack| stack.capture.as_deref())
        {
            capture.mark_last_native_exact_read(address, word, value);
        }
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
        peek_u8(self.emulator, address)
    }
    pub fn read_u8(&self, address: u32) -> u8 {
        if let Some(observer) = self.readable_stack.as_deref() {
            observer
                .guard
                .read(address, 1, tje_game::stack_read::Reader::RustMemory);
        }
        let value = self.peek_u8(address);
        self.record_memory_read(address, false, ReadPurpose::Semantic, u16::from(value));
        value
    }
    pub fn read_u16(&self, address: u32) -> u16 {
        if let Some(observer) = self.readable_stack.as_deref() {
            observer
                .guard
                .read(address, 2, tje_game::stack_read::Reader::RustMemory);
        }
        let value =
            u16::from_be_bytes([self.peek_u8(address), self.peek_u8(address.wrapping_add(1))]);
        self.record_memory_read(address, true, ReadPurpose::Semantic, value);
        value
    }
    pub fn read_u32(&self, address: u32) -> u32 {
        if let Some(observer) = self.readable_stack.as_deref() {
            observer
                .guard
                .read(address, 4, tje_game::stack_read::Reader::RustMemory);
        }
        (u32::from(self.read_u16(address)) << 16)
            | u32::from(self.read_u16(address.wrapping_add(2)))
    }
    pub fn write_u8(&mut self, address: u32, value: u8) {
        if let Some(observer) = self.readable_stack.as_deref() {
            observer.guard.assert_no_failure();
        }
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
                if let Some(observer) = self.readable_stack.as_deref() {
                    observer
                        .guard
                        .commit(address, 1, self.observe_native_provenance);
                }
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
                    if let Some(branch) = &stack.provenance {
                        if self.observe_native_provenance {
                            branch.observe_native(
                                genesis_core::HybridAccess::Write {
                                    address,
                                    word: false,
                                    value: u16::from(value),
                                },
                                self.wait_clock.frame,
                                self.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
                            );
                        } else {
                            branch.observe_setup_write(address, value);
                        }
                    }
                    if let Some(capture) = &stack.capture {
                        let access = if self.observe_native_provenance {
                            genesis_core::HybridAccess::Write {
                                address,
                                word: false,
                                value: u16::from(value),
                            }
                        } else {
                            genesis_core::HybridAccess::HostMutation {
                                address,
                                word: false,
                                value: u16::from(value),
                            }
                        };
                        capture.observe_native(
                            access,
                            crate::check_capture::AccessPoint::capture(
                                self.emulator,
                                self.wait_clock.frame,
                            ),
                            false,
                        );
                    }
                }
            }
            _ => panic!("write_u8 is ROM/RAM-only; use bus() for hardware address {address:06X}"),
        }
    }
    pub fn write_u16(&mut self, address: u32, value: u16) {
        if !self.observe_native_provenance {
            // Inspection/recipe stores are external byte mutations, not
            // fabricated CPU word accesses (and may patch unaligned storage).
            let [hi, lo] = value.to_be_bytes();
            self.write_u8(address, hi);
            self.write_u8(address.wrapping_add(1), lo);
            return;
        }
        let physical = address & 0xffffff;
        assert!(
            !(0x400000..0xe00000).contains(&physical),
            "write_u16 is ROM/RAM-only; use Platform for hardware"
        );
        self.flush_audit_accesses();
        // This issues one actual word. No observed bytes are regrouped later.
        tje_game::Platform::write_device_u16(self, address, value);
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
        self.poison_frozen_stack_alignment();
        self.emulator.hybrid_parts().0
    }
    pub(crate) fn exception_depth(&self) -> u32 {
        self.emulator.hybrid_cpu().hybrid_exception_depth()
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
        self.call_original_with_execution_contract_result(
            contract,
            execution_class,
            arguments,
            true,
        )
        .map(|value| value.expect("scalar original call requested D0"))
    }

    fn call_original_with_execution_contract_result(
        &mut self,
        contract: &tje_game::FunctionMetadata,
        execution_class: ExecutionClass,
        arguments: &[u32],
        want_value: bool,
    ) -> Result<Option<u32>> {
        self.poison_frozen_stack_alignment();
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
        let ceiling = self.enter_call_ceiling_contract(
            contract.id,
            execution_class,
            tje_game::execution_contract(contract.id)
                .and_then(|reviewed| reviewed.operational_ceiling)
                .unwrap_or(tje_game::execution::CallCeiling::DEFAULT),
        )?;
        let result =
            self.call_original_with_execution_contract_inner(contract, arguments, want_value);
        let cleanup = self.leave_call_ceiling_contract(ceiling);
        match result {
            Err(error) => Err(error),
            Ok(value) => cleanup.map(|()| value),
        }
    }

    fn call_original_with_execution_contract_inner(
        &mut self,
        contract: &tje_game::FunctionMetadata,
        arguments: &[u32],
        want_value: bool,
    ) -> Result<Option<u32>> {
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
        let readable_observer = self.readable_stack.clone();
        let _original_segment = readable_observer
            .as_ref()
            .map(|observer| observer.guard.original_segment());
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
                self.stack_audit.as_ref().map_or_else(
                    || crate::execution::PhysicalObservers {
                        readable: self.readable_stack.as_deref(),
                        ..Default::default()
                    },
                    StackAuditState::physical_observers,
                ),
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
        let result = want_value.then(|| cpu.data_registers()[0]);
        set_sp(cpu, saved_sp);
        cpu.set_pc(saved_pc, bus);
        self.flush_audit_accesses();
        tje_game::Platform::check_mmio_contract(self)?;
        if let Some(branch) = self
            .stack_audit
            .as_ref()
            .and_then(|stack| stack.provenance.as_ref())
        {
            branch.check_result()?;
        }
        Ok(result)
    }

    /// Run an original with the caller's exact outgoing area/return PC already
    /// present. Unlike call_original_with_execution_contract, push no new slots.
    fn call_original_prepared(&mut self, id: FunctionId, class: ExecutionClass) -> Result<u32> {
        self.call_original_prepared_result(id, class, true)
            .map(|value| value.expect("scalar prepared original call requested D0"))
    }

    fn call_original_prepared_result(
        &mut self,
        id: FunctionId,
        class: ExecutionClass,
        want_value: bool,
    ) -> Result<Option<u32>> {
        self.poison_frozen_stack_alignment();
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
        let ceiling = self.enter_call_ceiling_contract(
            id,
            class,
            tje_game::execution_contract(id)
                .and_then(|reviewed| reviewed.operational_ceiling)
                .unwrap_or(tje_game::execution::CallCeiling::DEFAULT),
        )?;
        let result = self.call_original_prepared_inner(id, want_value);
        let cleanup = self.leave_call_ceiling_contract(ceiling);
        match result {
            Err(error) => Err(error),
            Ok(value) => cleanup.map(|()| value),
        }
    }

    fn call_original_prepared_inner(
        &mut self,
        id: FunctionId,
        want_value: bool,
    ) -> Result<Option<u32>> {
        self.emulator.hybrid_bus_mut().apply_pending_writes();
        self.flush_audit_accesses();
        let entry_sp = self.emulator.hybrid_cpu().stack_pointer();
        let return_pc = self.read_u32(entry_sp);
        let return_sp = entry_sp.wrapping_add(4);
        let readable_observer = self.readable_stack.clone();
        let _original_segment = readable_observer
            .as_ref()
            .map(|observer| observer.guard.original_segment());
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
                self.stack_audit.as_ref().map_or_else(
                    || crate::execution::PhysicalObservers {
                        readable: self.readable_stack.as_deref(),
                        ..Default::default()
                    },
                    StackAuditState::physical_observers,
                ),
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
        if let Some(branch) = self
            .stack_audit
            .as_ref()
            .and_then(|stack| stack.provenance.as_ref())
        {
            branch.check_result()?;
        }
        Ok(want_value.then(|| self.emulator.hybrid_cpu().data_registers()[0]))
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
        self.observe_stack_banks();
        Ok(())
    }

    pub(crate) fn transfer_to(&mut self, pc: u32) -> Result<()> {
        self.poison_frozen_stack_alignment();
        tje_game::Platform::check_mmio_contract(self)?;
        let (cpu, bus) = self.emulator.hybrid_parts();
        cpu.set_pc(pc, bus);
        bus.apply_pending_writes();
        self.flush_audit_accesses();
        Ok(())
    }

    /// Realize only a source-reviewed synchronous transfer. Request data is not
    /// authority: bind its canonical site/opcode and the Rust-owned prepared
    /// state BEFORE the real entry effects. No original opcode is executed.
    pub(crate) fn enter_source_exception(
        &mut self,
        root: FunctionId,
        entry: FunctionId,
        request: tje_game::control::ExceptionRequest,
    ) -> Result<()> {
        self.poison_frozen_stack_alignment();
        let transfer = tje_game::exits::ControlTransfer {
            entry,
            exit: tje_game::exits::Exit::Exception { request },
        };
        let result: tje_game::Result<()> = (|| {
            tje_game::Platform::check_mmio_contract(self)?;
            request.validate_shape(self.revision).map_err(|error| {
                tje_game::Error::Platform(format!("invalid source exception data: {error:?}"))
            })?;
            if request.site.entry != entry {
                return Err(tje_game::Error::Platform(
                    "exception source identity mismatch".into(),
                ));
            }
            let frame = self
                .frame_state()
                .ok_or(tje_game::call_frame::FrameError::Unavailable)?;
            let saved_bytes = if request.prepared.saved_d2.is_some() {
                4
            } else {
                0
            };
            if (root != entry && !frame.is_prepared_for(entry))
                || request.prepared.stack_pointer != frame.entry_sp.wrapping_sub(saved_bytes)
            {
                return Err(tje_game::Error::Platform(
                    "synchronous trap requires its actual prepared entry frame".into(),
                ));
            }
            let p = &request.prepared;
            if [
                self.data_register(0),
                self.data_register(1),
                self.data_register(2),
            ] != p.data
                || [self.address_register(0), self.address_register(1)] != p.address
                || self.stack_pointer()? != p.stack_pointer
                || self.stack_banks()? != p.stack_banks
                || self.status_register() != p.status
            {
                return Err(tje_game::Error::Platform(
                    "source exception prepared registers/stack/status changed".into(),
                ));
            }
            let physical_sp = p.stack_pointer & 0xffffff;
            if p.saved_d2.is_some()
                && (!(0xe00000..=0xfffffc).contains(&physical_sp) || physical_sp & 1 != 0)
            {
                return Err(tje_game::Error::Platform(
                    "runtime D2 save is not an aligned RAM long".into(),
                ));
            }
            let fault_pc = request.site.address(self.revision);
            let opcode = match request.trap {
                tje_game::control::SynchronousTrap::DivideByZero => 0x82c2,
                tje_game::control::SynchronousTrap::PrivilegeViolation => {
                    tje_game::control::privileged_status_opcode(entry)
                        .ok_or(tje_game::Error::Untranslated(entry))?
                }
                _ => return Err(tje_game::Error::Untranslated(entry)),
            };
            let (cpu, bus) = self.emulator.hybrid_parts();
            if let Some(expected) = p.saved_d2 {
                // Inspection, not a fabricated semantic CPU operand/stack read.
                let saved = (u32::from(bus.memory.read_main_ram_word(physical_sp)) << 16)
                    | u32::from(bus.memory.read_main_ram_word(physical_sp.wrapping_add(2)));
                if saved != expected {
                    return Err(tje_game::Error::Platform(
                        "runtime D2 save differs from source store".into(),
                    ));
                }
            }
            let cartridge = bus.cartridge.as_mut().ok_or_else(|| {
                tje_game::Error::Platform("source exception has no cartridge".into())
            })?;
            let words = cartridge.debug_rom_view();
            let index = (fault_pc / 2) as usize;
            if index >= words.len() || words[index] != opcode {
                return Err(tje_game::Error::Platform(
                    "source exception is not the reviewed fault opcode".into(),
                ));
            }
            let vector = request.trap.vector().ok_or_else(|| {
                tje_game::Error::Platform("invalid standard exception vector".into())
            })?;
            // This is the faulting instruction's actual next-word prefetch,
            // not execution of its opcode or an imported reference snapshot.
            cpu.set_pc(fault_pc.wrapping_add(2), bus);
            let entered = cpu.hybrid_enter_trap(vector, fault_pc, request.saved_pc, opcode, bus);
            // Commit the REAL partial/complete writes even on entry failure.
            // The actor remains Rust-owned, never an original-callee exemption.
            bus.apply_pending_writes();
            self.flush_audit_accesses();
            tje_game::Platform::check_mmio_contract(self)?;
            entered.map_err(|error| {
                tje_game::Error::Platform(format!(
                    "real synchronous exception entry failed: {error:?}"
                ))
            })
        })();
        result
            .map_err(|error| tje_game::Error::cleanup_after_transfer(transfer, error))
            .map_err(HostError::Translation)
    }

    pub(crate) fn exception_returned(&mut self) {
        self.poison_frozen_stack_alignment();
        let (cpu, bus) = self.emulator.hybrid_parts();
        cpu.hybrid_exception_returned();
        bus.observe_exception_boundary(m68000_emu::ExceptionBoundary::Returned {
            depth: cpu.hybrid_exception_depth(),
        });
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
    fn write_u16(&mut self, address: u32, value: u16) {
        HookContext::write_u16(self, address, value);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        HookContext::write_u32(self, address, value);
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
        if let Some(error) = self
            .readable_stack
            .as_deref()
            .and_then(|observer| observer.guard.failure())
        {
            return Err(error.into());
        }
        if let Some(address) = self.emulator.hybrid_bus().hybrid_functional_violation {
            return Err(tje_game::Error::Platform(format!(
                "functional contract observed VDP status/HV at {address:06X}"
            )));
        }
        Ok(())
    }

    fn discard_u8(&mut self, address: u32) -> tje_game::Result<()> {
        self.check_mmio_contract()?;
        self.read_bus_tagged(address, false, ReadPurpose::Discarded);
        self.check_mmio_contract()
    }

    fn discard_u16(&mut self, address: u32) -> tje_game::Result<()> {
        self.check_mmio_contract()?;
        self.read_bus_tagged(address, true, ReadPurpose::Discarded);
        self.check_mmio_contract()
    }

    fn poll_yield(&mut self) -> tje_game::Result<()> {
        self.poison_frozen_stack_alignment();
        self.check_mmio_contract()?;
        self.flush_audit_accesses();
        let result = (|| {
            if self.execution_class != ExecutionClass::TimeAware {
                return Err(tje_game::Error::Platform(
                    "scheduler yield requires TimeAware execution".into(),
                ));
            }
            self.wait_clock.call_clock.dispatch_step()?;
            let observe = self.stack_audit.is_some();
            let observers = self.stack_audit.as_ref().map_or_else(
                || crate::execution::PhysicalObservers {
                    readable: self.readable_stack.as_deref(),
                    ..Default::default()
                },
                StackAuditState::physical_observers,
            );
            let run = run_poll_yield_with_audio(
                self.emulator,
                &mut self.wait_clock,
                std::mem::take(&mut self.time_audio),
                observe,
                observers,
            )
            .map_err(Self::game_platform_error)?;
            self.merge_clock_run(run);
            Ok(())
        })();
        self.flush_audit_accesses();
        result.map_err(|error| self.latch_mmio_error(error))
    }

    fn wait_until(&mut self, request: WaitRequest) -> tje_game::Result<()> {
        self.poison_frozen_stack_alignment();
        self.check_mmio_contract()?;
        self.flush_audit_accesses();
        let result = (|| {
            self.wait_clock
                .call_clock
                .dispatch_step()
                .map_err(tje_game::Error::Clock)?;
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
            let observers = self.stack_audit.as_ref().map_or_else(
                || crate::execution::PhysicalObservers {
                    readable: self.readable_stack.as_deref(),
                    ..Default::default()
                },
                StackAuditState::physical_observers,
            );
            let result = if observe && self.time_audio.is_empty() && !observers.enabled() {
                run_wait(self.emulator, &mut self.wait_clock, request)
            } else {
                run_wait_with_audio(
                    self.emulator,
                    &mut self.wait_clock,
                    request,
                    std::mem::take(&mut self.time_audio),
                    observe,
                    observers,
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
        if let Some(observer) = self.readable_stack.as_deref() {
            observer.guard.assert_no_failure();
        }
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
        if let Some(observer) = self.readable_stack.as_deref() {
            observer.guard.assert_no_failure();
        }
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
        // A7 is deliberately not aliased by this seven-register API. Preserve
        // its existing panic, but poison before a caller can catch the attempt.
        if index >= 7 {
            self.poison_frozen_stack_alignment();
        }
        let cpu = self.emulator.hybrid_parts().0;
        let mut addresses = cpu.address_registers();
        addresses[index] = value;
        cpu.set_address_registers(
            addresses,
            cpu.user_stack_pointer(),
            cpu.supervisor_stack_pointer(),
        );
        self.observe_stack_banks();
    }
    fn status_register(&self) -> u16 {
        self.emulator.hybrid_cpu().status_register()
    }
    fn set_status_register(&mut self, value: u16) {
        if (self.emulator.hybrid_cpu().status_register() ^ value) & 0x2000 != 0 {
            self.poison_frozen_stack_alignment();
            if let Some(observer) = &self.readable_stack {
                observer.guard.revoke_read_permissions();
            }
        }
        self.emulator.hybrid_parts().0.set_status_register(value);
        self.observe_stack_banks();
    }
    fn stack_pointer(&self) -> tje_game::Result<u32> {
        Ok(self.emulator.hybrid_cpu().stack_pointer())
    }
    fn set_stack_pointer(&mut self, value: u32) -> tje_game::Result<()> {
        self.observe_stack_assignment(value);
        set_sp(self.emulator.hybrid_parts().0, value);
        self.observe_stack_banks();
        Ok(())
    }
    fn frame_state(&self) -> Option<FrameState> {
        Some(self.frame_state)
    }
    fn set_frame_state(&mut self, state: FrameState) -> tje_game::Result<()> {
        if let Some(services) = self.timing_services.as_deref_mut() {
            services
                .authority
                .completion
                .bind_frame()
                .map_err(|error| tje_game::Error::Platform(error.to_string()))?;
        }
        self.frame_state = state;
        if let Some(observer) = &self.readable_stack
            && let Some(token) = self
                .stack_audit
                .as_ref()
                .and_then(|stack| stack.original_root.as_ref())
                .and_then(|scope| scope.current_materialization())
        {
            observer.guard.restore_materialization(
                token,
                state,
                self.emulator.hybrid_cpu().stack_pointer(),
                self.emulator.hybrid_cpu().status_register() & 0x2000 != 0,
            );
        }
        Ok(())
    }
    fn observe_call_boundary(&mut self, event: tje_game::call_frame::CallBoundaryEvent) {
        let (cpu, bus) = self.emulator.hybrid_parts();
        let materialization = self.readable_stack.as_ref().and_then(|observer| {
            observer.guard.observe_call_boundary(
                &event,
                self.frame_state,
                cpu.stack_pointer(),
                cpu.status_register() & 0x2000 != 0,
            )
        });
        if let Some(scope) = self
            .stack_audit
            .as_ref()
            .and_then(|stack| stack.original_root.as_ref())
        {
            scope.observe_framework(event, cpu, bus, self.frame_state, materialization);
            if let Some(observer) = &self.readable_stack
                && let Some(token) = scope.current_materialization()
            {
                observer.guard.restore_materialization(
                    token,
                    self.frame_state,
                    cpu.stack_pointer(),
                    cpu.status_register() & 0x2000 != 0,
                );
            }
        }
    }
    fn read_uninitialized_local(
        &mut self,
        read: tje_game::stack_read::UninitializedLocalRead,
    ) -> tje_game::Result<u32> {
        tje_game::Platform::check_mmio_contract(self)?;
        self.emulator.hybrid_bus_mut().apply_pending_writes();
        self.flush_audit_accesses();
        let observer = self.readable_stack.clone().ok_or_else(|| {
            tje_game::Error::from(tje_game::stack_read::Unsupported::unavailable(read))
        })?;
        let invocation = if self.mode == crate::Mode::Check {
            Some(
                self.stack_audit
                    .as_ref()
                    .and_then(|stack| stack.original_root.as_ref())
                    .and_then(|scope| {
                        scope.native_witness(
                            self.emulator.hybrid_cpu(),
                            self.frame_state,
                            read.owner,
                        )
                    })
                    .ok_or_else(|| {
                        tje_game::Error::from(observer.guard.reject_explicit_local(
                            read,
                            tje_game::stack_read::UnsupportedReason::InvalidExplicitLocal,
                        ))
                    })?,
            )
        } else {
            None
        };
        let address = observer.guard.explicit_local(
            read,
            self.frame_state,
            self.emulator.hybrid_cpu().stack_pointer(),
            self.emulator.hybrid_cpu().status_register() & 0x2000 != 0,
        )?;
        let mask = observer.guard.unwritten_mask(address, read.width.bytes());
        let first_read = if self.mode == crate::Mode::Check {
            let stack = self.stack_audit.as_mut().expect("explicit checked root");
            stack.capture_reads = true;
            stack.capture_full_ram = true;
            Some(
                self.audit
                    .get_mut()
                    .stack_reads
                    .get_or_insert_with(Vec::new)
                    .len(),
            )
        } else {
            None
        };
        let mut beats = [0; 2];
        let mut ordinals = [0; 2];
        let count = if read.width == tje_game::stack_read::LocalWidth::Long {
            2
        } else {
            1
        };
        for beat in 0..count {
            let raw = address.wrapping_add(2 * beat as u32);
            let word = read.width != tje_game::stack_read::LocalWidth::Byte;
            let value = if word {
                u16::from_be_bytes([self.peek_u8(raw), self.peek_u8(raw.wrapping_add(1))])
            } else {
                u16::from(self.peek_u8(raw))
            };
            self.record_memory_read(raw, word, ReadPurpose::Semantic, value);
            ordinals[beat] = self.audit.borrow().read_ordinal;
            if first_read.is_some() {
                // Seal this native occurrence from its OWN commit bitmap at
                // the actual read, not from original-callee initialization.
                let beat_mask = (mask >> (2 * beat)) & if word { 3 } else { 1 };
                let audit = self.audit.get_mut();
                let token = audit
                    .stack_reads
                    .as_mut()
                    .expect("explicit journal")
                    .last_mut()
                    .expect("actual RAM read");
                token.uninitialized_mask = beat_mask;
                let token = token.clone();
                if audit
                    .unwritten_stack_reads
                    .last()
                    .is_some_and(|read| read.ordinal == token.ordinal)
                {
                    audit.unwritten_stack_reads.pop();
                }
                if beat_mask != 0 {
                    audit.unwritten_stack_reads.push(token);
                }
            }
            beats[beat] = value;
        }
        if let Some(first_read) = first_read {
            self.audit
                .get_mut()
                .explicit_stack_reads
                .push(crate::readable_stack::ExplicitRead {
                    declaration: read,
                    mask,
                    beats,
                    first_read,
                    ordinals,
                    beat_count: count,
                    invocation,
                });
        }
        Ok(if count == 2 {
            (u32::from(beats[0]) << 16) | u32::from(beats[1])
        } else {
            u32::from(beats[0])
        })
    }
    fn call_original_entry(
        &mut self,
        id: FunctionId,
        class: ExecutionClass,
    ) -> tje_game::Result<u32> {
        self.call_original_prepared(id, class)
            .map_err(Self::game_platform_error)
    }
    fn call_original_entry_void(
        &mut self,
        id: FunctionId,
        class: ExecutionClass,
    ) -> tje_game::Result<()> {
        self.call_original_prepared_result(id, class, false)
            .map(|_| ())
            .map_err(Self::game_platform_error)
    }
    fn stack_banks(&self) -> tje_game::Result<tje_game::exits::StackBanks> {
        let cpu = self.emulator.hybrid_cpu();
        Ok(tje_game::exits::StackBanks {
            user: cpu.user_stack_pointer(),
            supervisor: cpu.supervisor_stack_pointer(),
        })
    }
    fn set_stack_banks(&mut self, banks: tje_game::exits::StackBanks) -> tje_game::Result<()> {
        self.observe_stack_assignment(banks.user);
        self.observe_stack_assignment(banks.supervisor);
        let cpu = self.emulator.hybrid_parts().0;
        cpu.set_address_registers(cpu.address_registers(), banks.user, banks.supervisor);
        self.observe_stack_banks();
        Ok(())
    }
    fn execute_timing_program(
        &mut self,
        entry: tje_game::FunctionId,
        source: &mut dyn tje_game::timing::TimingSource,
    ) -> tje_game::Result<tje_game::timing::TimedExit> {
        self.poison_frozen_stack_alignment();
        use crate::timing_region::game_bridge::{
            self, EntryScope, ExecutionScope, PausedHostSource, PreparedInvocation, SourceAdvance,
            SourceProof, SourceRequest,
        };
        use tje_engine_runtime::{
            portable_timing::{ActiveSource, PausedSource},
            timing_region::TimingRegion,
        };
        let timing_error = |cause: tje_engine_core::timing_region::TimingError| {
            tje_game::Error::Platform(cause.to_string())
        };
        tje_game::Platform::check_mmio_contract(self)?;
        let PreparedInvocation {
            initial,
            target,
            active,
            profile,
        } = {
            let services = self.timing_services.as_deref_mut().ok_or_else(|| {
                tje_game::Error::Platform(
                    "source timing descriptors were not prepared at startup".into(),
                )
            })?;
            game_bridge::prepare_entry(
                EntryScope {
                    emulator: self.emulator,
                    revision: self.revision,
                    frame: self.frame_state,
                    class: self.execution_class,
                    clocked: self.timed_execution,
                    clock: &mut self.wait_clock,
                    mode: self.mode,
                    dispatcher: self.dispatcher,
                },
                services,
                entry,
                source,
            )
            .map_err(Self::game_platform_error)?
        };
        let ActiveSource {
            mut bound,
            mut storage,
        } = active;
        let begun = TimingRegion::begin_oracle(bound.timing(), &initial, target, &mut storage);
        if let Err(cause) = &begun {
            let primary = tje_game::Error::Platform(cause.to_string());
            drop(begun);
            let _ = self
                .timing_services
                .as_deref_mut()
                .expect("active source service")
                .plans
                .release(ActiveSource { bound, storage }, Default::default());
            return Err(primary);
        }
        let mut region = begun.expect("region admission error handled");
        region
            .set_call_profile(profile.label())
            .map_err(timing_error)?;
        let invocation = match self
            .timing_services
            .as_deref_mut()
            .expect("active source service")
            .authority
            .completion
            .begin(entry, self.frame_state.entry_sp, &region)
        {
            Ok(invocation) => invocation,
            Err(cause) => {
                let usage = region.usage();
                drop(region);
                let _ = self
                    .timing_services
                    .as_deref_mut()
                    .expect("active source service")
                    .plans
                    .release(ActiveSource { bound, storage }, usage);
                return Err(self.protect_timing_failure(timing_error(cause)));
            }
        };
        #[cfg(test)]
        game_bridge::proof::begin_invocation();
        let mut proof = SourceProof::begin(
            self.emulator,
            &self.wait_clock,
            self.time_audio.len(),
            &self
                .timing_services
                .as_deref()
                .expect("active source service")
                .authority
                .parents,
        );
        let mut dispatch = self
            .wait_clock
            .call_clock
            .dispatch_step()
            .map_err(tje_game::Error::from);
        let result = loop {
            if let Err(cause) = dispatch {
                break Err(cause);
            }
            dispatch = Ok(());
            let run = {
                let services = self
                    .timing_services
                    .as_deref_mut()
                    .expect("active source service");
                game_bridge::run(
                    ExecutionScope {
                        emulator: self.emulator,
                        #[cfg(test)]
                        frame: self.frame_state,
                        clock: &mut self.wait_clock,
                        audio: std::mem::take(&mut self.time_audio),
                        audit: self.stack_audit.is_some(),
                        observers: self.stack_audit.as_ref().map_or_else(
                            || crate::execution::PhysicalObservers {
                                readable: self.readable_stack.as_deref(),
                                ..Default::default()
                            },
                            StackAuditState::physical_observers,
                        ),
                    },
                    SourceRequest {
                        bound: &bound,
                        region: &mut region,
                        source,
                        invocation: &invocation,
                        parents: &mut services.authority.parents,
                        proof,
                    },
                )
            };
            self.merge_clock_run(run.clock_run);
            proof = run.proof;
            match run.outcome.map_err(Self::game_platform_error) {
                Err(cause) => break Err(cause),
                Ok(SourceAdvance::Returned(completed)) => {
                    break self
                        .timing_services
                        .as_deref_mut()
                        .expect("active source service")
                        .authority
                        .completion
                        .complete(invocation, completed)
                        .map_err(timing_error);
                }
                Ok(SourceAdvance::Call { intent, token }) => {
                    let parent_frame = self.frame_state;
                    let parent_class = self.execution_class;
                    let continuation = region
                        .park()
                        .map_err(|cause| self.protect_timing_failure(timing_error(cause)))?;
                    self.timing_services
                        .as_deref_mut()
                        .expect("active source service")
                        .authority
                        .parents
                        .push(PausedHostSource {
                            source: PausedSource {
                                active: ActiveSource { bound, storage },
                                continuation,
                            },
                            proof,
                        });
                    self.invoke_timing_callee(&invocation, intent.target, &token)?;
                    let actual = crate::timing_region::state(self.emulator);
                    let returned = self
                        .timing_services
                        .as_deref_mut()
                        .expect("active source service")
                        .authority
                        .completion
                        .finish_child(token, &actual)
                        .map_err(timing_error)
                        .map_err(|cause| self.protect_timing_failure(cause))?;
                    let counted = {
                        let services = self
                            .timing_services
                            .as_deref_mut()
                            .expect("active source service");
                        services
                            .authority
                            .parents
                            .iter_mut()
                            .try_for_each(|parent| {
                                parent.source.continuation.observe_return(returned.kind())
                            })
                    };
                    counted.map_err(|cause| self.protect_timing_failure(timing_error(cause)))?;
                    let paused = self
                        .timing_services
                        .as_deref_mut()
                        .expect("active source service")
                        .authority
                        .parents
                        .pop()
                        .ok_or_else(|| {
                            self.protect_timing_failure(tje_game::Error::Platform(
                                "missing suspended parent source".into(),
                            ))
                        })?;
                    bound = paused.source.active.bound;
                    storage = paused.source.active.storage;
                    proof = paused.proof;
                    region = paused
                        .source
                        .continuation
                        .resume(bound.timing(), &mut storage)
                        .map_err(|cause| self.protect_timing_failure(timing_error(cause)))?;
                    returned
                        .resume(
                            &mut region,
                            &actual,
                            crate::timing_region::boundary_signals(self.emulator),
                        )
                        .map_err(|cause| self.protect_timing_failure(timing_error(cause)))?;
                    self.frame_state = parent_frame;
                    self.execution_class = parent_class;
                }
            }
        };
        let usage = region.usage();
        drop(region);
        let release = self
            .timing_services
            .as_deref_mut()
            .expect("active source service")
            .plans
            .release(ActiveSource { bound, storage }, usage)
            .map_err(timing_error);
        match result {
            Err(cause) => Err(self.protect_timing_failure(cause)),
            Ok(exit) => release
                .map(|()| exit)
                .map_err(|cause| self.protect_timing_failure(cause)),
        }
    }
    fn consume_timing_completion(
        &mut self,
        request: tje_game::timing::CompletionRequest,
    ) -> tje_game::Result<tje_game::timing::Completion> {
        let Some(services) = self.timing_services.as_deref_mut() else {
            return Ok(tje_game::timing::Completion::Absent);
        };
        let state = crate::timing_region::state(self.emulator);
        #[cfg(test)]
        let result = crate::timing_region::game_bridge::proof::consume_receipt(
            &mut services.authority.completion,
            request,
            &state,
        );
        #[cfg(not(test))]
        let result = services.authority.completion.consume(request, &state);
        #[cfg(test)]
        if result.is_err() {
            crate::timing_region::game_bridge::proof::record_failure(
                self.emulator,
                self.frame_state,
                self.wait_clock.frame,
            );
        }
        result.map_err(|error| {
            tje_game::Error::timing_execution_failure(
                request.entry,
                tje_game::Error::Platform(error.to_string()),
            )
        })
    }
    fn protect_timing_failure(&self, cause: tje_game::Error) -> tje_game::Error {
        let Some(services) = self.timing_services.as_ref() else {
            return cause;
        };
        match services.authority.completion.protected_entry() {
            Some(entry) => {
                services.authority.completion.fail();
                #[cfg(test)]
                crate::timing_region::game_bridge::proof::record_failure(
                    self.emulator,
                    self.frame_state,
                    self.wait_clock.frame,
                );
                tje_game::Error::timing_execution_failure(entry, cause)
            }
            None => cause,
        }
    }
    fn dispatcher(&self) -> Dispatcher {
        self.dispatcher
    }
    fn check_callee_dispatch(
        &self,
        dispatcher: Dispatcher,
        entry: tje_game::FunctionId,
        rust_selected: bool,
    ) -> tje_game::Result<()> {
        if self.mode == crate::Mode::Replace {
            dispatcher.check_native_rust(entry, rust_selected)
        } else {
            Ok(())
        }
    }
    fn execution_class(&self) -> ExecutionClass {
        self.execution_class
    }
    fn clocked_execution(&self) -> bool {
        self.timed_execution || self.wait_clock.call_clock.active()
    }
    fn enter_call_ceiling(
        &mut self,
        function: FunctionId,
    ) -> tje_game::Result<tje_game::execution::CallCeilingScope> {
        let reviewed = tje_game::execution_contract(function)
            .ok_or(tje_game::Error::Unclassified(function))?;
        self.enter_call_ceiling_contract(
            function,
            reviewed.class,
            reviewed
                .operational_ceiling
                .unwrap_or(tje_game::execution::CallCeiling::DEFAULT),
        )
        .map_err(|error| match error {
            HostError::Clock(error) => tje_game::Error::Clock(error),
            other => tje_game::Error::Platform(other.to_string()),
        })
    }
    fn leave_call_ceiling(
        &mut self,
        scope: tje_game::execution::CallCeilingScope,
    ) -> tje_game::Result<()> {
        self.leave_call_ceiling_contract(scope)
            .map_err(|error| match error {
                HostError::Clock(error) => tje_game::Error::Clock(error),
                other => tje_game::Error::Platform(other.to_string()),
            })
    }
    fn set_execution_class(&mut self, class: ExecutionClass) -> tje_game::Result<()> {
        if class != ExecutionClass::Atomic {
            self.poison_frozen_stack_alignment();
        }
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
            .map_err(Self::game_platform_error)
    }
    fn call_original_with_execution(
        &mut self,
        id: FunctionId,
        arguments: &[u32],
        class: ExecutionClass,
    ) -> tje_game::Result<u32> {
        self.call_original_with_execution_contract(id.metadata(), class, arguments)
            .map_err(Self::game_platform_error)
    }
    fn call_original_void_with_execution(
        &mut self,
        id: FunctionId,
        arguments: &[u32],
        class: ExecutionClass,
    ) -> tje_game::Result<()> {
        self.call_original_with_execution_contract_result(id.metadata(), class, arguments, false)
            .map(|_| ())
            .map_err(Self::game_platform_error)
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

impl HookContext<'_> {
    fn invoke_timing_callee(
        &mut self,
        parent: &tje_engine_runtime::portable_timing::Invocation,
        target: FunctionId,
        token: &tje_engine_runtime::timing_region::CallToken,
    ) -> tje_game::Result<()> {
        use tje_engine_runtime::timing_region::CalleeKind;
        let result = (|| {
            let class =
                tje_game::execution_class(target).ok_or(tje_game::Error::Unclassified(target))?;
            if !self.execution_class.permits(class)
                || tje_game::exits::contract(target) != tje_game::exits::ExitContract::Rts
            {
                return Err(tje_game::Error::ExecutionUnavailable {
                    caller: self.execution_class,
                    callee: class,
                });
            }
            let selected = self.dispatcher.resolve_entry(target)?;
            let services = self.timing_services.as_deref_mut().ok_or_else(|| {
                tje_game::Error::Platform("source service lease is unavailable".into())
            })?;
            let profile = services
                .profile(self.mode, self.dispatcher)
                .map_err(Self::game_platform_error)?;
            let timed = services.plans.binding(target).is_some();
            let kind = if timed {
                if !matches!(
                    selected.map(|entry| entry.entry()),
                    Some(tje_game::TranslatedEntry::Framed(_))
                ) {
                    return Err(tje_game::Error::Platform(
                        "timed callee has no effective framed Rust entry".into(),
                    ));
                }
                CalleeKind::TimedRust
            } else {
                if !services
                    .permits_original(self.mode, self.dispatcher, target)
                    .map_err(Self::game_platform_error)?
                {
                    return Err(tje_game::Error::Platform(
                        "callee substitution is unavailable in this actual host mode".into(),
                    ));
                }
                CalleeKind::ReferenceOriginalSubstitution
            };
            services
                .authority
                .completion
                .enter_child(parent, token, target, profile, kind)
                .map_err(|cause| tje_game::Error::Platform(cause.to_string()))?;
            self.frame_state = FrameState::new(token.entry_stack());
            self.execution_class = class;
            let scope = self.enter_call_ceiling(target)?;
            self.nested_calls = self
                .nested_calls
                .checked_add(1)
                .ok_or_else(|| tje_game::Error::Platform("source call counter exhausted".into()))?;
            let body = if timed {
                selected
                    .expect("validated timed callee")
                    .invoke_prepared(self)
            } else {
                self.execute_original_timing_callee(token)
            };
            let body = body.map_err(|cause| self.protect_timing_failure(cause));
            let result = tje_game::exits::validate_call(target, body, self.revision);
            let value = tje_game::Error::after_contract_check(self, result)?;
            if self.consume_timing_completion(tje_game::timing::CompletionRequest {
                entry: target,
                entry_sp: token.entry_stack(),
                kind: tje_game::timing::CompletionKind::Rts,
                expected_pc: Some(token.return_pc()),
                value,
            })? != tje_game::timing::Completion::Consumed
            {
                return Err(tje_game::Error::Platform(
                    "callee did not produce a sealed actual RTS receipt".into(),
                ));
            }
            // Child finalization remains fallible after its real RTS. Close
            // the scope while the child owner/frame is still current.
            self.leave_call_ceiling(scope)?;
            Ok(())
        })();
        result.map_err(|cause| self.protect_timing_failure(cause))
    }
    fn execute_original_timing_callee(
        &mut self,
        token: &tje_engine_runtime::timing_region::CallToken,
    ) -> tje_game::Result<Option<u32>> {
        use crate::timing_region::game_bridge::{self, ExecutionScope};
        let result = (|| {
            let actual = crate::timing_region::state(self.emulator);
            let mut controller = self
                .timing_services
                .as_deref_mut()
                .ok_or_else(|| {
                    tje_game::Error::Platform("original child lost its source owner".into())
                })?
                .authority
                .completion
                .begin_original(token, &actual)
                .map_err(|cause| tje_game::Error::Platform(cause.to_string()))?;
            let (outcome, run) = {
                let services = self
                    .timing_services
                    .as_deref_mut()
                    .expect("active original child");
                game_bridge::run_original(
                    ExecutionScope {
                        emulator: self.emulator,
                        #[cfg(test)]
                        frame: self.frame_state,
                        clock: &mut self.wait_clock,
                        audio: std::mem::take(&mut self.time_audio),
                        audit: self.stack_audit.is_some(),
                        observers: self.stack_audit.as_ref().map_or_else(
                            || crate::execution::PhysicalObservers {
                                readable: self.readable_stack.as_deref(),
                                ..Default::default()
                            },
                            StackAuditState::physical_observers,
                        ),
                    },
                    &mut services.authority.parents,
                    &mut controller,
                )
            };
            self.merge_clock_run(run);
            outcome.map_err(Self::game_platform_error)?;
            let actual = crate::timing_region::state(self.emulator);
            let signals = crate::timing_region::boundary_signals(self.emulator);
            let exit = self
                .timing_services
                .as_deref_mut()
                .expect("active original child")
                .authority
                .completion
                .complete_original(controller, &actual, signals)
                .map_err(|cause| tje_game::Error::Platform(cause.to_string()))?;
            Ok(exit.value)
        })();
        result.map_err(|cause| self.protect_timing_failure(cause))
    }
}

#[cfg(test)]
#[path = "abi/timing_call_tests.rs"]
mod timing_call_tests;

#[cfg(test)]
#[path = "abi/discard_tests.rs"]
mod discard_tests;

#[cfg(test)]
#[path = "abi/width_yield_tests.rs"]
mod width_yield_tests;

#[cfg(test)]
#[path = "abi/void_calls_tests.rs"]
mod void_calls_tests;
