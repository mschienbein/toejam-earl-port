//! Ordinary readable source composed with authenticated generated IRQ code.
//! This profile is NOT evidence for readable Rust ISR implementations.
use super::{BoundProgram, CompletionLedger, PortableError, TimingBackend, apply_registers, game};
use crate::timing_region::{
    BoundaryAction, BoundarySignals, InterruptToken, RegionStorage, RegionUsage, TimingRegion,
    advance_and_commit, classify_boundary,
};
use crate::{Bus, CpuState, DeviceStep, Devices, Engine, RuntimeStep, StepKind};
use genesis_config::GenesisInputs;
use std::sync::Arc;
use tje_engine_core::{
    ReadKind, metadata,
    timing_region::{AccessKind, BusObservation, CursorIdentity, TimingEvent},
};
use tje_game::{
    Context, Dispatcher, Error, FunctionId, Platform, Result,
    call_frame::FrameState,
    execution::{CallCeiling, CallCeilingScope, CallClock, ClockUsage, ExecutionClass},
    exits::StackBanks,
};
use tje_mem::Bus as Memory;
use tje_rom::Rom;

fn error(error: impl std::fmt::Display) -> Error {
    Error::Platform(error.to_string())
}

fn protect(ledger: &CompletionLedger, cause: Error) -> Error {
    if matches!(cause, Error::ControlTransfer(_)) {
        return cause;
    }
    match ledger.protected_entry() {
        Some(entry) => {
            ledger.fail();
            Error::timing_execution_failure(entry, cause)
        }
        None => cause,
    }
}

/// The caller owns a per-machine input latch. Entry only borrows its current
/// sample; polling, recipes and outputs occur once per actual renderer event.
pub trait FrameTimeline {
    /// Pure getter for the already sampled frame, never a controller poll.
    /// A stale/missing latch must fail closed rather than sampling on entry.
    fn latched_inputs(&self, frame: u64) -> Result<GenesisInputs>;
    /// Poll and update the caller's latch after an actual FrameRendered event.
    fn sample_inputs(&self, frame: u64) -> Result<GenesisInputs>;
    fn before_frame(&self, _frame: u64, _memory: &mut dyn Memory) -> Result<()> {
        Ok(())
    }
    /// Consume the actual completed frame and its borrowed sample chunk before
    /// another instruction. A trailing partial chunk remains in the devices.
    fn frame_completed(&self, frame: u64, devices: &crate::PlatformDevices) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressOwner {
    SourceInstruction,
    CompiledEngineInterrupt { instruction: bool },
    HardwareWait,
}

/// Optional proof observer of actual completed steps, never an effect source or
/// a second audit truth. Implementations receive only borrowed live state.
pub trait StepObserver {
    fn completed_step(
        &mut self,
        owner: ProgressOwner,
        step: DeviceStep,
        cpu: &CpuState,
        devices: &crate::PlatformDevices,
    ) -> Result<()>;
}

/// Immutable startup attestation for the actual compiled backend, not an ISA
/// interpreter label. It retains neither ROM nor the frontend's generated IR.
pub struct CompiledTimingCapability {
    identity: ([u8; 20], [u8; 32]),
    divider: u32,
    revision: tje_mem::Revision,
    rom_identity: tje_mem::RomIdentity,
}
impl CompiledTimingCapability {
    pub fn validate<B: TimingBackend>(machine: &Engine<B>, cartridge: &Rom) -> Result<Self> {
        if machine.devices().memory().revision() != cartridge.revision()
            || machine.devices().memory().rom() != cartridge.bytes()
        {
            return Err(error(
                "compiled timing cartridge does not match the live device cartridge",
            ));
        }
        let divider = machine.devices().cycles().m68k_divider_u32.get();
        let mut digest = [0; 20];
        let text = cartridge.revision().sha1().as_bytes();
        for (index, byte) in digest.iter_mut().enumerate() {
            let hex = std::str::from_utf8(&text[index * 2..index * 2 + 2]).map_err(error)?;
            *byte = u8::from_str_radix(hex, 16).map_err(error)?;
        }
        let program = metadata::compile_program_with_divider(
            cartridge.bytes(),
            &metadata::game_metadata(cartridge.revision()),
            divider,
        )
        .map_err(error)?;
        if !machine
            .backend()
            .authenticates_compilation(digest, &program)
            || program
                .instructions
                .iter()
                .any(|ir| !machine.backend().contains_compiled_instruction(ir.pc))
        {
            return Err(error(
                "backend has no authenticated complete compiled IRQ/callee capability",
            ));
        }
        // These are the Genesis device layer's IRQ sources, not source-supplied
        // permissions. Every instruction in their conservative metadata closure
        // was checked above, including all indirect catalogue continuations.
        for level in [4_u32, 6] {
            let address = ((24 + level) * 4) as usize;
            let bytes = cartridge
                .bytes()
                .get(address..address + 4)
                .ok_or_else(|| error("missing device IRQ vector"))?;
            let pc = u32::from_be_bytes(bytes.try_into().map_err(error)?);
            if !machine.backend().contains_compiled_instruction(pc) {
                return Err(error("compiled device IRQ entry is missing"));
            }
        }
        Ok(Self {
            identity: machine.backend().compiled_identity(),
            divider,
            revision: cartridge.revision(),
            rom_identity: machine.devices().memory().rom_identity(),
        })
    }
    fn check<B: TimingBackend>(&self, machine: &Engine<B>, cartridge: &Rom) -> Result<()> {
        if machine.backend().compiled_identity() != self.identity
            || machine.devices().cycles().m68k_divider_u32.get() != self.divider
            || self.revision != cartridge.revision()
            || machine.devices().memory().revision() != self.revision
            || machine.devices().memory().rom_identity() != self.rom_identity
        {
            return Err(error(
                "compiled timing capability is stale or has the wrong divider",
            ));
        }
        Ok(())
    }
}

/// Startup-prepared immutable binding plus reusable per-machine scratch.
/// Construction may allocate; creating/invoking a warm Context does not.
pub struct NativeSourceState {
    cache: Arc<BoundProgram>,
    storage: RegionStorage,
    irq_frames: Vec<Frame>,
    completion: CompletionLedger,
}
impl NativeSourceState {
    pub fn new(cache: Arc<BoundProgram>) -> Self {
        let storage = cache.prepare_storage();
        Self {
            cache,
            storage,
            irq_frames: Vec::with_capacity(8),
            completion: CompletionLedger::default(),
        }
    }
}

/// A clocked Context at a real prepared CPU entry. Its ordinary MMIO primitives
/// are actual device accesses; a TimingSource never receives this Context and
/// therefore can perform effects only through the authenticated region driver.
pub struct NativeTimingContext<'a, B> {
    machine: &'a mut Engine<B>,
    cartridge: &'a Rom,
    capability: &'a CompiledTimingCapability,
    clock: &'a mut CallClock,
    timeline: &'a dyn FrameTimeline,
    inputs: GenesisInputs,
    dispatcher: Dispatcher,
    frame: FrameState,
    class: ExecutionClass,
    usage: Option<RegionUsage>,
    observer: Option<&'a mut dyn StepObserver>,
    state: &'a mut NativeSourceState,
}
impl<'a, B: TimingBackend> NativeTimingContext<'a, B>
where
    B::Error: std::fmt::Display,
{
    pub fn new(
        machine: &'a mut Engine<B>,
        cartridge: &'a Rom,
        capability: &'a CompiledTimingCapability,
        clock: &'a mut CallClock,
        timeline: &'a dyn FrameTimeline,
        dispatcher: Dispatcher,
        state: &'a mut NativeSourceState,
    ) -> Result<Self> {
        if !clock.contains_scope(state.cache.entry()) {
            return Err(error(
                "native timing requires its actual owner-bound root scope",
            ));
        }
        capability.check(machine, cartridge)?;
        if !state.cache.matches(cartridge, capability.divider, &[4, 6]) {
            return Err(error(
                "native source cache has the wrong cartridge, divider or IRQ policy",
            ));
        }
        let class = tje_game::execution_contract(state.cache.entry())
            .ok_or_else(|| error("native source has no reviewed execution contract"))?
            .class;
        state
            .completion
            .change_scope()
            .map_err(|cause| protect(&state.completion, error(cause)))?;
        let inputs = timeline.latched_inputs(machine.frame_count())?;
        let frame = FrameState::new(machine.cpu().read_address(7));
        Ok(Self {
            machine,
            cartridge,
            capability,
            clock,
            timeline,
            inputs,
            dispatcher,
            frame,
            class,
            usage: None,
            observer: None,
            state,
        })
    }
    pub fn with_observer(mut self, observer: &'a mut dyn StepObserver) -> Self {
        self.observer = Some(observer);
        self
    }
    pub fn usage(&self) -> Option<RegionUsage> {
        self.usage
    }
    pub fn machine(&self) -> &Engine<B> {
        self.machine
    }
    /// The same public framed body used by the host proof, not a private effect
    /// implementation. Only its authenticated, completed RTS is finalized here.
    pub fn invoke_prepared(&mut self, entry: FunctionId) -> Result<Option<u32>> {
        let entry_sp = self.machine.cpu().read_address(7);
        if tje_game::exits::contract(entry) != tje_game::exits::ExitContract::Rts
            || entry != self.state.cache.entry()
        {
            return Err(error(
                "native source context is bound to a different ordinary-return entry",
            ));
        }
        let outcome = tje_game::exits::enter(self, entry, &[])?;
        let tje_game::exits::EntryOutcome::Returned(value) = outcome else {
            return Err(error("ordinary source entry attempted a non-RTS transfer"));
        };
        let completion = self.consume_timing_completion(game::CompletionRequest {
            entry,
            entry_sp,
            kind: game::CompletionKind::Rts,
            expected_pc: None,
            value,
        })?;
        if completion != game::Completion::Consumed {
            return Err(error(
                "native source entry did not complete an authenticated timing program",
            ));
        }
        Ok(value)
    }
}

impl<B: TimingBackend> Memory for NativeTimingContext<'_, B> {
    fn read_u8(&self, address: u32) -> u8 {
        self.machine.devices().memory().read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.machine
            .devices_mut()
            .memory_mut()
            .write_u8(address, value);
    }
}
impl<B: TimingBackend> tje_mem::Platform for NativeTimingContext<'_, B> {
    fn write_vdp_control(&mut self, value: u16) {
        Platform::write_device_u16(self, 0xc00004, value);
    }
    fn write_vdp_data(&mut self, value: u16) {
        Platform::write_device_u16(self, 0xc00000, value);
    }
    fn write_vdp_command(&mut self, value: u32) {
        Platform::write_device_u16(self, 0xc00004, (value >> 16) as u16);
        Platform::write_device_u16(self, 0xc00004, value as u16);
    }
}
impl<B: TimingBackend> Platform for NativeTimingContext<'_, B> {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        self.machine
            .devices_mut()
            .read8(address, ReadKind::Semantic)
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        self.machine
            .devices_mut()
            .read16(address, ReadKind::Semantic)
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.machine.devices_mut().write8(address, value);
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.machine.devices_mut().write16(address, value);
    }
}
impl<B: TimingBackend> Context for NativeTimingContext<'_, B>
where
    B::Error: std::fmt::Display,
{
    fn revision(&self) -> tje_mem::Revision {
        self.cartridge.revision()
    }
    fn data_register(&self, index: usize) -> u32 {
        self.machine.cpu().d[index]
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.machine.cpu_mut().d[index] = value;
    }
    fn address_register(&self, index: usize) -> u32 {
        self.machine.cpu().read_address(index as u8)
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.machine.cpu_mut().write_address(index as u8, value);
    }
    fn status_register(&self) -> u16 {
        self.machine.cpu().sr as u16
    }
    fn set_status_register(&mut self, value: u16) {
        self.machine.cpu_mut().set_sr(u32::from(value));
    }
    fn stack_pointer(&self) -> Result<u32> {
        Ok(self.machine.cpu().read_address(7))
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.machine.cpu_mut().write_address(7, value);
        Ok(())
    }
    fn frame_state(&self) -> Option<FrameState> {
        Some(self.frame)
    }
    fn set_frame_state(&mut self, frame: FrameState) -> Result<()> {
        self.state.completion.change_scope().map_err(error)?;
        self.frame = frame;
        Ok(())
    }
    fn stack_banks(&self) -> Result<StackBanks> {
        Ok(StackBanks {
            user: self.machine.cpu().usp,
            supervisor: self.machine.cpu().ssp,
        })
    }
    fn set_stack_banks(&mut self, banks: StackBanks) -> Result<()> {
        self.machine.cpu_mut().usp = banks.user;
        self.machine.cpu_mut().ssp = banks.supervisor;
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        self.dispatcher
    }
    fn execution_class(&self) -> ExecutionClass {
        self.class
    }
    fn set_execution_class(&mut self, class: ExecutionClass) -> Result<()> {
        self.class = class;
        Ok(())
    }
    fn clocked_execution(&self) -> bool {
        true
    }
    fn enter_call_ceiling(&mut self, entry: FunctionId) -> Result<CallCeilingScope> {
        let contract = tje_game::execution_contract(entry)
            .ok_or_else(|| error("native call has no reviewed execution contract"))?;
        Ok(self.clock.enter(
            entry,
            contract.operational_ceiling.unwrap_or(CallCeiling::DEFAULT),
        )?)
    }
    fn leave_call_ceiling(&mut self, scope: CallCeilingScope) -> Result<()> {
        self.clock.leave(scope)?;
        Ok(())
    }
    fn call_original(&mut self, entry: FunctionId, _arguments: &[u32]) -> Result<u32> {
        Err(Error::Untranslated(entry))
    }
    fn execute_timing_program(
        &mut self,
        entry: FunctionId,
        source: &mut dyn game::TimingSource,
    ) -> Result<game::TimedExit> {
        if !self.clock.contains_scope(entry)
            || tje_game::execution_class(entry) != Some(self.class)
            || !self.frame.is_prepared_for(entry)
            || self.frame.entry_sp != self.machine.cpu().read_address(7)
            || self.machine.cpu().pc != entry.metadata().address(self.revision())
        {
            return Err(error(
                "native timing needs the exact reviewed clocked prepared entry",
            ));
        }
        self.capability.check(self.machine, self.cartridge)?;
        let entry_sp = self.frame.entry_sp;
        let physical = entry_sp & 0xffffff;
        let bytes = 4_u32
            .checked_mul(1 + u32::from(entry.metadata().abi.argument_count))
            .ok_or_else(|| error("prepared source frame extent overflow"))?;
        if physical < 0xe00000
            || physical
                .checked_add(bytes)
                .is_none_or(|end| end > 0x1000000)
        {
            return Err(error("prepared source frame is not entirely live work RAM"));
        }
        let return_pc = self.machine.devices().memory().read_u32(entry_sp);
        let bound = &self.state.cache;
        if !bound.accepts(entry, source.program()) {
            return Err(error("source cache descriptor/entry mismatch"));
        }
        let target = bound
            .return_target(self.cartridge, return_pc)
            .map_err(error)?;
        if !self
            .machine
            .backend()
            .contains_compiled_instruction(target.pc)
        {
            return Err(error("source caller target is not a compiled continuation"));
        }
        let mut region = TimingRegion::begin_compiled(
            bound.timing(),
            self.machine.cpu(),
            target,
            &mut self.state.storage,
        )
        .map_err(error)?;
        let invocation = self
            .state
            .completion
            .begin(entry, entry_sp, &region)
            .map_err(error)?;
        let result = self
            .clock
            .dispatch_step()
            .map_err(Error::from)
            .and_then(|()| {
                NativeDrive {
                    machine: self.machine,
                    clock: self.clock,
                    timeline: self.timeline,
                    inputs: &mut self.inputs,
                    observer: &mut self.observer,
                    irq_frames: &mut self.state.irq_frames,
                }
                .run(bound, source, &mut region, &invocation)
            });
        self.usage = Some(region.usage());
        let result = match result {
            Ok(completed) => self
                .state
                .completion
                .complete(invocation, completed)
                .map_err(error),
            Err(error) => {
                self.state.completion.fail();
                Err(error)
            }
        };
        result.map_err(|cause| Error::timing_execution_failure(entry, cause))
    }
    fn consume_timing_completion(
        &mut self,
        request: game::CompletionRequest,
    ) -> Result<game::Completion> {
        self.state
            .completion
            .consume(request, self.machine.cpu())
            .map_err(|cause| Error::timing_execution_failure(request.entry, error(cause)))
    }
    fn protect_timing_failure(&self, cause: Error) -> Error {
        protect(&self.state.completion, cause)
    }
}

struct NativeDrive<'a, 'observer, B> {
    machine: &'a mut Engine<B>,
    clock: &'a mut CallClock,
    timeline: &'a dyn FrameTimeline,
    inputs: &'a mut GenesisInputs,
    observer: &'a mut Option<&'observer mut dyn StepObserver>,
    irq_frames: &'a mut Vec<Frame>,
}
impl<B: TimingBackend> NativeDrive<'_, '_, B>
where
    B::Error: std::fmt::Display,
{
    fn run(
        &mut self,
        bound: &BoundProgram,
        source: &mut dyn game::TimingSource,
        region: &mut TimingRegion<'_>,
        invocation: &super::Invocation,
    ) -> Result<super::CertifiedReturn> {
        let mut completed = None;
        loop {
            self.admit(region)?;
            if region.is_complete() {
                break;
            }
            self.clock.before_tick()?;
            let entry = &self.machine.cpu;
            let mut portable = bound
                .begin(source, entry, region.return_target())
                .map_err(PortableError::into_game_error)?;
            let mut cursor = region.cursor(entry, BoundaryAction::Ready).map_err(error)?;
            self.machine.devices.set_inputs(*self.inputs);
            self.machine
                .devices
                .begin_instruction(entry.pc, entry.prefetch as u16);
            let (timing, registers) = loop {
                match cursor.next_event().map_err(error)? {
                    TimingEvent::Bus { token, expected } => {
                        let request = portable
                            .request(expected)
                            .map_err(PortableError::into_game_error)?;
                        let bus = &mut self.machine.devices;
                        let value = match expected.kind {
                            AccessKind::Read(kind) => match expected.width {
                                tje_engine_core::ir::Width::Byte => {
                                    u32::from(bus.read8(expected.address, kind))
                                }
                                tje_engine_core::ir::Width::Word => {
                                    u32::from(bus.read16(expected.address, kind))
                                }
                            },
                            AccessKind::Write => {
                                let Some(game::Effect {
                                    kind: game::EffectKind::Write(value),
                                    ..
                                }) = request
                                else {
                                    return Err(error("source did not own its write payload"));
                                };
                                match expected.width {
                                    tje_engine_core::ir::Width::Byte => {
                                        bus.write8(expected.address, value as u8)
                                    }
                                    tje_engine_core::ir::Width::Word => {
                                        bus.write16(expected.address, value)
                                    }
                                }
                                u32::from(value)
                            }
                        };
                        let observed = BusObservation {
                            address: expected.address,
                            width: expected.width,
                            kind: expected.kind,
                            value,
                        };
                        cursor.acknowledge_bus(token, observed).map_err(error)?;
                        portable
                            .acknowledge(observed)
                            .map_err(PortableError::into_game_error)?;
                    }
                    TimingEvent::SetReadOffset {
                        token,
                        master_clocks,
                    } => {
                        self.machine.devices.set_read_offset_mclk(master_clocks);
                        cursor.acknowledge_offset(token).map_err(error)?;
                    }
                    TimingEvent::InstructionComplete { token, .. } => {
                        let (registers, path, exit) =
                            portable.finish().map_err(PortableError::into_game_error)?;
                        if exit.is_some() {
                            completed = exit;
                        }
                        break (
                            cursor.complete_instruction(token, path).map_err(error)?,
                            registers,
                        );
                    }
                    TimingEvent::Exception { .. } => {
                        return Err(error("source timing region exited through an exception"));
                    }
                }
            };
            let timing = region.accept_instruction(timing).map_err(error)?;
            apply_registers(&mut self.machine.cpu, &registers);
            let step = advance_and_commit(&mut self.machine.devices, &mut self.machine.cpu, timing)
                .map_err(error)?;
            self.machine.frames = self
                .machine
                .frames
                .checked_add(u64::from(step.frame_complete))
                .ok_or_else(|| error("native frame counter overflow"))?;
            self.record(region, step, ProgressOwner::SourceInstruction)?;
            if completed.is_some() {
                region.complete_return(self.machine.cpu()).map_err(error)?;
            }
        }
        let exit =
            completed.ok_or_else(|| error("source region completed without a source return"))?;
        bound
            .certify_return(region, invocation, self.machine.cpu(), exit, self.signals())
            .map_err(error)
    }
    fn boundary(&self) -> BoundaryAction {
        classify_boundary(self.machine.cpu(), self.signals())
    }
    fn signals(&self) -> BoundarySignals {
        BoundarySignals {
            wait: self.machine.devices().cycles().m68k_wait_cpu_cycles != 0,
            reset: self.machine.devices().reset_asserted(),
            halted: self.machine.devices().cpu_halted(),
            interrupt_level: self.machine.devices().pending_interrupt(),
        }
    }
    fn step(
        &mut self,
        region: &mut TimingRegion<'_>,
        compiled: bool,
    ) -> Result<(RuntimeStep, Frame)> {
        self.clock.before_tick()?;
        let before = Frame::capture(self.machine.cpu());
        let step = self.machine.step(*self.inputs).map_err(error)?;
        let owner = if compiled {
            ProgressOwner::CompiledEngineInterrupt {
                instruction: step.kind == StepKind::Instruction,
            }
        } else {
            ProgressOwner::HardwareWait
        };
        self.record(region, step.devices, owner)?;
        Ok((step, before))
    }
    fn admit(&mut self, region: &mut TimingRegion<'_>) -> Result<()> {
        loop {
            match self.boundary() {
                BoundaryAction::Ready => return Ok(()),
                BoundaryAction::Faulted | BoundaryAction::Reset => {
                    return Err(error("native region abandoned by fault or reset"));
                }
                BoundaryAction::Wait | BoundaryAction::Halted | BoundaryAction::Stopped => {
                    let (step, _) = self.step(region, false)?;
                    if step.kind == StepKind::Instruction {
                        return Err(error("original main execution forbidden"));
                    }
                }
                action @ (BoundaryAction::InterruptAdmission { .. }
                | BoundaryAction::InterruptAcknowledge { .. }) => {
                    let token = region
                        .suspend_interrupt(self.machine.cpu(), action)
                        .map_err(error)?;
                    let mut guard = CompiledIrqGuard::new(&token, std::mem::take(self.irq_frames));
                    loop {
                        let ready = self.boundary() == BoundaryAction::Ready;
                        if matches!(
                            self.boundary(),
                            BoundaryAction::Reset | BoundaryAction::Faulted
                        ) {
                            return Err(error("compiled ISR abandoned the source region"));
                        }
                        if ready
                            && !self
                                .machine
                                .backend()
                                .contains_compiled_instruction(self.machine.cpu().pc)
                        {
                            return Err(error("compiled ISR reached an unsupported continuation"));
                        }
                        if ready {
                            guard.before(self.machine.cpu())?;
                        }
                        let (step, before) = self.step(region, true)?;
                        guard.observe(before, &step, self.machine.cpu())?;
                        if guard.finished() {
                            let (receipt, frames) = guard.receipt()?;
                            *self.irq_frames = frames;
                            region
                                .resume_compiled_interrupt(token, self.machine.cpu(), receipt)
                                .map_err(error)?;
                            break;
                        }
                    }
                }
            }
        }
    }
    fn record(
        &mut self,
        region: &mut TimingRegion<'_>,
        step: DeviceStep,
        owner: ProgressOwner,
    ) -> Result<()> {
        let compiled = owner == (ProgressOwner::CompiledEngineInterrupt { instruction: true });
        let progress = region.record_progress(step).map_err(error);
        let retired = if compiled {
            region.record_compiled_instruction().map_err(error)
        } else {
            Ok(())
        };
        let clock = self.clock.record(ClockUsage {
            ticks: 1,
            instructions: u64::from(compiled),
            master_clocks: step.master_clocks,
            frames: u64::from(step.frame_complete),
            dispatch_steps: 0,
        });
        let input: Result<()> = if step.frame_complete {
            (|| {
                let next = self.machine.frame_count();
                let inputs = self.timeline.sample_inputs(next)?;
                if self.timeline.latched_inputs(next)? != inputs {
                    return Err(error(
                        "sampled input did not update the actual current-frame latch",
                    ));
                }
                self.timeline
                    .before_frame(next, self.machine.devices_mut().memory_mut())?;
                *self.inputs = inputs;
                Ok(())
            })()
        } else {
            Ok(())
        };
        let observed = if let Some(observer) = self.observer.as_mut() {
            observer.completed_step(owner, step, self.machine.cpu(), self.machine.devices())
        } else {
            Ok(())
        };
        let output = if step.frame_complete {
            let result = self
                .timeline
                .frame_completed(self.machine.frame_count(), self.machine.devices());
            if result.is_ok() {
                self.machine.devices_mut().clear_audio();
            }
            result
        } else {
            Ok(())
        };
        let budget = self.clock.check();
        input?;
        progress?;
        retired?;
        clock?;
        observed?;
        output?;
        budget?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Frame {
    pc: u32,
    prefetch: u32,
    sr: u32,
    usp: u32,
    ssp: u32,
}
impl Frame {
    fn capture(cpu: &CpuState) -> Self {
        Self {
            pc: cpu.pc,
            prefetch: cpu.prefetch,
            sr: cpu.sr,
            usp: cpu.usp,
            ssp: cpu.ssp,
        }
    }
    fn matches(self, cpu: &CpuState) -> bool {
        self.pc == cpu.pc
            && self.prefetch == cpu.prefetch
            && self.sr == cpu.sr
            && self.usp == cpu.usp
            && self.ssp == cpu.ssp
    }
}
struct CompiledIrqGuard {
    identity: (CursorIdentity, u64),
    frames: Vec<Frame>,
    entered: bool,
    instructions: u64,
}
impl CompiledIrqGuard {
    fn new(token: &InterruptToken, mut frames: Vec<Frame>) -> Self {
        frames.clear();
        Self {
            identity: token.identity(),
            frames,
            entered: false,
            instructions: 0,
        }
    }
    fn before(&self, cpu: &CpuState) -> Result<()> {
        if cpu.prefetch == 0x4e73 {
            let frame = self
                .frames
                .last()
                .ok_or_else(|| error("RTE has no actual accepted IRQ entry"))?;
            if cpu.sr & 0x2000 == 0 || cpu.usp != frame.usp || cpu.ssp != frame.ssp.wrapping_sub(6)
            {
                return Err(error("compiled RTE has the wrong owned exception frame"));
            }
        }
        Ok(())
    }
    fn observe(&mut self, before: Frame, step: &RuntimeStep, cpu: &CpuState) -> Result<()> {
        match step.kind {
            StepKind::InterruptAcknowledge { .. } => {
                self.frames.push(before);
                self.entered = true;
            }
            StepKind::Instruction => {
                self.instructions = self
                    .instructions
                    .checked_add(1)
                    .ok_or_else(|| error("compiled ISR count overflow"))?;
                if before.prefetch == 0x4e73 {
                    if before.sr & 0x2000 == 0
                        || step.devices.cpu_cycles != 20
                        || cpu.address_error != 0
                        || cpu.frozen != 0
                    {
                        return Err(error("compiled RTE did not actually return successfully"));
                    }
                    let frame = self
                        .frames
                        .pop()
                        .ok_or_else(|| error("compiled RTE has no owned entry"))?;
                    if !frame.matches(cpu) {
                        return Err(error(
                            "compiled ISR returned a different stack/SR/pipeline; no repair permitted",
                        ));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn finished(&self) -> bool {
        self.entered && self.frames.is_empty() && self.instructions != 0
    }
    fn receipt(self) -> Result<(CompiledInterruptReceipt, Vec<Frame>)> {
        if !self.finished() {
            return Err(error("compiled ISR has not produced a real RTE"));
        }
        Ok((
            CompiledInterruptReceipt {
                identity: self.identity,
                instructions: self.instructions,
            },
            self.frames,
        ))
    }
}
/// Not constructible by a source/provider: minted only by the actual compiled
/// step monitor after accepted entry and successful, validated real RTE.
pub(crate) struct CompiledInterruptReceipt {
    identity: (CursorIdentity, u64),
    instructions: u64,
}
impl CompiledInterruptReceipt {
    pub(crate) fn identity(&self) -> (CursorIdentity, u64) {
        self.identity
    }
    pub(crate) fn instructions(&self) -> u64 {
        self.instructions
    }
}
