//! Ordinary readable source composed with authenticated generated IRQ code.
//! This profile is NOT evidence for readable Rust ISR implementations.
use super::{
    ActiveSource, BoundProgram, CallIntent, CallProfile, CompletionOwners, PausedSource,
    PortableControl, PortableError, PreparedSources, TimingBackend, apply_registers, game,
};
use crate::timing_region::{
    BoundaryAction, BoundarySignals, CallToken, InterruptToken, RegionUsage, RegionWork,
    TimingRegion, advance_and_commit, classify_boundary,
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
    CompiledOriginalCallee { instruction: bool },
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
/// interpreter label. It retains no ROM or full IR, only authenticated call boundaries.
#[derive(Clone, Copy)]
struct OriginalCallSite {
    opcode: u16,
    resume: u32,
    return_prefetch: Option<u16>,
}
pub struct CompiledTimingCapability {
    identity: ([u8; 20], [u8; 32]),
    divider: u32,
    revision: tje_mem::Revision,
    rom_identity: tje_mem::RomIdentity,
    calls: std::collections::BTreeMap<u32, OriginalCallSite>,
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
        {
            return Err(error(
                "backend has no authenticated complete compiled IRQ/callee capability",
            ));
        }
        let mut calls = std::collections::BTreeMap::new();
        for instruction in &program.instructions {
            if !machine
                .backend()
                .contains_compiled_instruction(instruction.pc)
            {
                return Err(error("backend omits an authenticated continuation"));
            }
            if matches!(
                tje_engine_core::decode::decode(instruction.opcode),
                tje_engine_core::decode::Instruction::JumpToSubroutine(_)
                    | tje_engine_core::decode::Instruction::BranchToSubroutine(_)
            ) {
                calls.insert(
                    instruction.pc,
                    OriginalCallSite {
                        opcode: instruction.opcode,
                        resume: instruction.next_pc,
                        return_prefetch: program
                            .instruction(instruction.next_pc)
                            .map(|next| next.opcode),
                    },
                );
            }
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
            calls,
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
    root_cache: Option<Arc<BoundProgram>>,
    plans: PreparedSources,
    parents: Vec<PausedSource>,
    irq_frames: Vec<Frame>,
    completion: CompletionOwners,
    profile: CallProfile,
    validated_dispatcher: Option<(Dispatcher, CallProfile, bool)>,
    selected_calls: Option<super::ValidatedCalls>,
}
impl NativeSourceState {
    fn root_binding(&self) -> Result<&BoundProgram> {
        self.root_cache
            .as_deref()
            .ok_or_else(|| error("proof call-window authority cannot enter a whole source root"))
    }
    pub fn new(
        cache: Arc<BoundProgram>,
        callees: &[Arc<BoundProgram>],
        profile: CallProfile,
    ) -> Result<Self> {
        if matches!(
            profile,
            CallProfile::HostHybridReference | CallProfile::HostCheckOriginalSubstitution
        ) || callees.iter().any(|callee| {
            callee.revision != cache.revision
                || callee.rom_digest != cache.rom_digest
                || callee.divider != cache.divider
                || callee.irq_policy != cache.irq_policy
        }) {
            return Err(error(
                "native source set has an invalid profile or mixed binding authority",
            ));
        }
        let plans = PreparedSources::new(
            std::iter::once(Arc::clone(&cache)).chain(callees.iter().cloned()),
        )
        .map_err(error)?;
        let mut completion = CompletionOwners::default();
        completion.prepare_depth(plans.len());
        let parents = Vec::with_capacity(plans.len());
        Ok(Self {
            root_cache: Some(cache),
            plans,
            parents,
            irq_frames: Vec::with_capacity(8),
            completion,
            profile,
            validated_dispatcher: None,
            selected_calls: None,
        })
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
        let root = state.root_binding()?;
        let root_entry = root.entry();
        dispatcher
            .check_native_rust(root_entry, dispatcher.resolve_entry(root_entry)?.is_some())?;
        if !clock.contains_scope(root_entry) {
            return Err(error(
                "native timing requires its actual owner-bound root scope",
            ));
        }
        capability.check(machine, cartridge)?;
        if !root.matches(cartridge, capability.divider, &[4, 6]) {
            return Err(error(
                "native source cache has the wrong cartridge, divider or IRQ policy",
            ));
        }
        match state.validated_dispatcher {
            Some((selected, profile, diagnostic))
                if selected.same_selection(dispatcher)
                    && profile == state.profile
                    && state
                        .selected_calls
                        .as_ref()
                        .is_some_and(|calls| calls.has_diagnostic_candidates() == diagnostic) => {}
            Some(_) => {
                return Err(error(
                    "native call closure selection/profile changed after startup validation",
                ));
            }
            None => {
                let calls = state
                    .plans
                    .validate_selection(dispatcher, state.profile)
                    .map_err(PortableError::into_game_error)?;
                let diagnostic = calls.has_diagnostic_candidates();
                state.selected_calls = Some(calls);
                state.validated_dispatcher = Some((dispatcher, state.profile, diagnostic));
            }
        }
        let class = tje_game::execution_contract(root_entry)
            .ok_or_else(|| error("native source has no reviewed execution contract"))?
            .class;
        state
            .completion
            .begin_root_scope()
            .map_err(|cause| state.completion.protect_failure(error(cause)))?;
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
            || entry != self.state.root_binding()?.entry()
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
    fn invoke_callee(
        &mut self,
        parent: &super::Invocation,
        target: FunctionId,
        token: &CallToken,
    ) -> Result<()> {
        let result = (|| {
            let class = tje_game::execution_class(target).ok_or(Error::Unclassified(target))?;
            if !self.class.permits(class)
                || tje_game::exits::contract(target) != tje_game::exits::ExitContract::Rts
            {
                return Err(Error::ExecutionUnavailable {
                    caller: self.class,
                    callee: class,
                });
            }
            let selected = self.dispatcher.resolve_entry(target)?;
            self.check_callee_dispatch(self.dispatcher, target, selected.is_some())?;
            match selected.map(|entry| entry.entry()) {
                Some(tje_game::TranslatedEntry::Framed(_))
                    if self.state.plans.binding(target).is_some() => {}
                Some(_) => {
                    return Err(error(
                        "selected Rust callee lacks reviewed framed timing roles",
                    ));
                }
                None if self.state.profile == CallProfile::NativeCompiledOriginalCallees => {}
                None => return Err(Error::Untranslated(target)),
            }
            let kind = if selected.is_some() {
                crate::timing_region::CalleeKind::TimedRust
            } else {
                crate::timing_region::CalleeKind::CompiledOriginal
            };
            self.state
                .completion
                .enter_child(parent, token, target, self.state.profile, kind)
                .map_err(error)?;
            self.frame = FrameState::new(token.entry_stack());
            self.class = class;
            let scope = self.enter_call_ceiling(target)?;
            let body = match selected {
                Some(body) => body.invoke_prepared(self),
                None => self.execute_original_callee(target, token),
            };
            let body = body.map_err(|cause| self.state.completion.protect_failure(cause));
            let result = tje_game::exits::validate_call(target, body, self.revision());
            let value = Error::after_contract_check(self, result)?;
            if self.consume_timing_completion(game::CompletionRequest {
                entry: target,
                entry_sp: token.entry_stack(),
                kind: game::CompletionKind::Rts,
                expected_pc: Some(token.return_pc()),
                value,
            })? != game::Completion::Consumed
            {
                return Err(error("callee did not produce a sealed actual RTS receipt"));
            }
            // Never restore a suspended parent before fallible child cleanup.
            self.leave_call_ceiling(scope)?;
            Ok(())
        })();
        result.map_err(|cause| self.state.completion.protect_failure(cause))
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
    fn discard_u8(&mut self, address: u32) -> tje_game::Result<()> {
        let _ = self
            .machine
            .devices_mut()
            .read8(address, ReadKind::Discarded);
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> tje_game::Result<()> {
        let _ = self
            .machine
            .devices_mut()
            .read16(address, ReadKind::Discarded);
        Ok(())
    }
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
        self.state.completion.bind_frame().map_err(error)?;
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
    fn check_callee_dispatch(
        &self,
        dispatcher: Dispatcher,
        entry: FunctionId,
        rust_selected: bool,
    ) -> Result<()> {
        dispatcher.check_native_rust(entry, rust_selected)
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
        self.check_callee_dispatch(
            self.dispatcher,
            entry,
            self.dispatcher.resolve_entry(entry)?.is_some(),
        )?;
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
        let preview = self
            .state
            .plans
            .binding(entry)
            .ok_or_else(|| error("source has no prepared timing roles"))?;
        if !preview.accepts(entry, source.program()) {
            return Err(error("source cache descriptor/entry mismatch"));
        }
        let return_pc = self.machine.devices().memory().read_u32(entry_sp);
        let target = preview
            .return_target(self.cartridge, return_pc)
            .map_err(error)?;
        if !self
            .machine
            .backend()
            .contains_compiled_instruction(target.pc)
        {
            return Err(error("source caller target is not a compiled continuation"));
        }
        let ActiveSource {
            mut bound,
            mut storage,
        } = self.state.plans.acquire(entry).map_err(error)?;
        let begun =
            TimingRegion::begin_compiled(bound.timing(), self.machine.cpu(), target, &mut storage);
        if let Err(cause) = &begun {
            let primary = error(cause);
            drop(begun);
            let _ = self
                .state
                .plans
                .release(ActiveSource { bound, storage }, RegionUsage::default());
            return Err(primary);
        }
        let mut region = begun.expect("region admission error handled");
        region
            .set_call_profile(self.state.profile.label())
            .map_err(error)?;
        let invocation = match self.state.completion.begin(entry, entry_sp, &region) {
            Ok(invocation) => invocation,
            Err(cause) => {
                let usage = region.usage();
                drop(region);
                let _ = self
                    .state
                    .plans
                    .release(ActiveSource { bound, storage }, usage);
                return Err(self.state.completion.protect_failure(error(cause)));
            }
        };
        let mut initial_dispatch = self.clock.dispatch_step().map_err(Error::from);
        let result = loop {
            if let Err(cause) = initial_dispatch {
                break Err(cause);
            }
            initial_dispatch = Ok(());
            let advanced = NativeDrive {
                machine: self.machine,
                clock: self.clock,
                timeline: self.timeline,
                inputs: &mut self.inputs,
                observer: &mut self.observer,
                irq_frames: &mut self.state.irq_frames,
                parents: &mut self.state.parents,
            }
            .run(&bound, source, &mut region, &invocation);
            match advanced {
                Err(cause) => break Err(cause),
                Ok(SourceAdvance::Returned(completed)) => {
                    break self
                        .state
                        .completion
                        .complete(invocation, completed)
                        .map_err(error);
                }
                Ok(SourceAdvance::Call { intent, token }) => {
                    let parent_frame = self.frame;
                    let parent_class = self.class;
                    let continuation = region
                        .park()
                        .map_err(|cause| self.state.completion.protect_failure(error(cause)))?;
                    self.state.parents.push(PausedSource {
                        active: ActiveSource { bound, storage },
                        continuation,
                    });
                    if let Err(cause) = self.invoke_callee(&invocation, intent.target, &token) {
                        self.state.completion.fail();
                        self.usage = self
                            .state
                            .parents
                            .last()
                            .map(|parent| parent.continuation.usage());
                        return Err(Error::timing_execution_failure(entry, cause));
                    }
                    let returned = self
                        .state
                        .completion
                        .finish_child(token, self.machine.cpu())
                        .map_err(|cause| self.state.completion.protect_failure(error(cause)))?;
                    for parent in &mut self.state.parents {
                        parent
                            .continuation
                            .observe_return(returned.kind())
                            .map_err(|cause| self.state.completion.protect_failure(error(cause)))?;
                    }
                    let paused = self.state.parents.pop().ok_or_else(|| {
                        self.state
                            .completion
                            .protect_failure(error("missing suspended parent source"))
                    })?;
                    bound = paused.active.bound;
                    storage = paused.active.storage;
                    region = paused
                        .continuation
                        .resume(bound.timing(), &mut storage)
                        .map_err(|cause| self.state.completion.protect_failure(error(cause)))?;
                    returned
                        .resume(&mut region, self.machine.cpu(), signals(self.machine))
                        .map_err(|cause| self.state.completion.protect_failure(error(cause)))?;
                    self.frame = parent_frame;
                    self.class = parent_class;
                }
            }
        };
        let usage = region.usage();
        self.usage = Some(usage);
        drop(region);
        let release = self
            .state
            .plans
            .release(ActiveSource { bound, storage }, usage)
            .map_err(error);
        let result = match result {
            Err(cause) => Err(cause),
            Ok(exit) => release.map(|()| exit),
        };
        result.map_err(|cause| {
            self.state.completion.fail();
            Error::timing_execution_failure(entry, cause)
        })
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
        self.state.completion.protect_failure(cause)
    }
}

enum SourceAdvance {
    Call {
        intent: CallIntent,
        token: CallToken,
    },
    Returned(super::CertifiedReturn),
}

struct NativeDrive<'a, 'observer, B> {
    machine: &'a mut Engine<B>,
    clock: &'a mut CallClock,
    timeline: &'a dyn FrameTimeline,
    inputs: &'a mut GenesisInputs,
    observer: &'a mut Option<&'observer mut dyn StepObserver>,
    irq_frames: &'a mut Vec<Frame>,
    parents: &'a mut [PausedSource],
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
    ) -> Result<SourceAdvance> {
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
            let (timing, registers, control) = loop {
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
                        let finished = portable.finish().map_err(PortableError::into_game_error)?;
                        break (
                            cursor
                                .complete_instruction(token, finished.path)
                                .map_err(error)?,
                            finished.registers,
                            finished.control,
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
            self.record(Some(&mut *region), step, ProgressOwner::SourceInstruction)?;
            match control {
                PortableControl::Continue => {}
                PortableControl::Return(exit) => {
                    completed = Some(exit);
                    region.complete_return(self.machine.cpu()).map_err(error)?;
                }
                PortableControl::Call(intent) => {
                    let token = region.authorize_call(self.machine.cpu()).map_err(error)?;
                    let return_word = self
                        .machine
                        .devices()
                        .memory()
                        .read_u32(token.entry_stack());
                    bound
                        .authenticate_call(intent, &token, self.machine.cpu(), return_word)
                        .map_err(error)?;
                    return Ok(SourceAdvance::Call { intent, token });
                }
            }
        }
        let exit =
            completed.ok_or_else(|| error("source region completed without a source return"))?;
        bound
            .certify_return(region, invocation, self.machine.cpu(), exit, self.signals())
            .map(SourceAdvance::Returned)
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
        self.record(Some(&mut *region), step.devices, owner)?;
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
        mut region: Option<&mut TimingRegion<'_>>,
        step: DeviceStep,
        owner: ProgressOwner,
    ) -> Result<()> {
        let compiled_isr = owner == (ProgressOwner::CompiledEngineInterrupt { instruction: true });
        let compiled =
            compiled_isr || owner == (ProgressOwner::CompiledOriginalCallee { instruction: true });
        let progress = match region.as_mut() {
            Some(region) => region.record_progress(step).map_err(error),
            None => Ok(()),
        };
        let retired = match region.as_mut() {
            Some(region) if compiled_isr => region.record_compiled_instruction().map_err(error),
            _ => Ok(()),
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
        let work = match owner {
            ProgressOwner::SourceInstruction => RegionWork::SourceInstruction,
            ProgressOwner::CompiledEngineInterrupt { instruction: true } => {
                RegionWork::CompiledIsrInstruction
            }
            ProgressOwner::CompiledOriginalCallee { instruction: true } => {
                RegionWork::CompiledOriginalCalleeInstruction
            }
            _ => RegionWork::Hardware,
        };
        let mut ancestors = Ok(());
        for parent in self.parents.iter_mut() {
            if let Err(cause) = parent.observe(step, work)
                && ancestors.is_ok()
            {
                ancestors = Err(error(cause));
            }
        }
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
        ancestors?;
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
    fn for_original(identity: (CursorIdentity, u64), mut frames: Vec<Frame>) -> Self {
        frames.clear();
        Self {
            identity,
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

fn signals<B: TimingBackend>(machine: &Engine<B>) -> BoundarySignals {
    BoundarySignals {
        wait: machine.devices().cycles().m68k_wait_cpu_cycles != 0,
        reset: machine.devices().reset_asserted(),
        halted: machine.devices().cpu_halted(),
        interrupt_level: machine.devices().pending_interrupt(),
    }
}

impl<B: TimingBackend> NativeTimingContext<'_, B>
where
    B::Error: std::fmt::Display,
{
    fn execute_original_callee(
        &mut self,
        entry: FunctionId,
        token: &CallToken,
    ) -> Result<Option<u32>> {
        let result = (|| {
            if self.state.profile != CallProfile::NativeCompiledOriginalCallees
                || !self
                    .state
                    .selected_calls
                    .as_ref()
                    .is_some_and(|selection| selection.permits_original(self.state.profile, entry))
                || self.dispatcher.resolve_entry(entry)?.is_some()
            {
                return Err(error(
                    "compiled original callee is not in the opted-in unselected closure",
                ));
            }
            let mut controller = self
                .state
                .completion
                .begin_original(token, self.machine.cpu())
                .map_err(error)?;
            let mut irq: Option<CompiledIrqGuard> = None;
            loop {
                let boundary = classify_boundary(self.machine.cpu(), signals(self.machine));
                if controller.saw_return() && irq.is_none() && boundary == BoundaryAction::Ready {
                    break;
                }
                if matches!(boundary, BoundaryAction::Faulted | BoundaryAction::Reset) {
                    return Err(error("original callee abandoned by actual fault or reset"));
                }
                if irq.is_none()
                    && matches!(
                        boundary,
                        BoundaryAction::InterruptAdmission { .. }
                            | BoundaryAction::InterruptAcknowledge { .. }
                    )
                {
                    let identity = controller.interrupt_identity(boundary).map_err(error)?;
                    irq = Some(CompiledIrqGuard::for_original(
                        identity,
                        std::mem::take(&mut self.state.irq_frames),
                    ));
                }
                let interrupt = irq.is_some();
                if !interrupt && boundary == BoundaryAction::Ready && !controller.saw_return() {
                    let pc = self.machine.cpu().pc;
                    let start = entry.metadata().address(self.revision());
                    let end = start
                        .checked_add(entry.metadata().size(self.revision()))
                        .ok_or_else(|| error("original callee extent overflow"))?;
                    if !(start..end).contains(&pc) {
                        return Err(error(
                            "original callee took an unsupported nonlocal or tail exit",
                        ));
                    }
                }
                self.clock.before_tick()?;
                if let Some(guard) = &irq {
                    guard.before(self.machine.cpu())?;
                }
                let cpu = self.machine.cpu();
                let before_pc = cpu.pc;
                let before_opcode = cpu.prefetch as u16;
                let before_stack = cpu.read_address(7);
                let before_irq = interrupt.then(|| Frame::capture(cpu));
                let prepared_call = if !interrupt
                    && boundary == BoundaryAction::Ready
                    && !controller.saw_return()
                {
                    if let Some(call) = self.capability.calls.get(&cpu.pc).copied() {
                        if u32::from(call.opcode) != cpu.prefetch {
                            return Err(error(
                                "compiled original call has a stale prefetched opcode",
                            ));
                        }
                        let prefetch = call.return_prefetch.ok_or_else(|| {
                            error("original call has no published return continuation")
                        })?;
                        Some(
                            controller
                                .prepare_compiled_call(cpu, call.resume, prefetch)
                                .map_err(error)?,
                        )
                    } else {
                        None
                    }
                } else {
                    None
                };
                let step = self.machine.step(self.inputs).map_err(error)?;
                let instruction = step.instruction_pc.map(|pc| super::OriginalInstruction {
                    pc,
                    opcode: before_opcode,
                    stack: before_stack,
                    interrupt,
                });
                let reported = if step.instruction_pc.is_some_and(|pc| pc != before_pc) {
                    Err(error(
                        "actual compiled instruction receipt disagrees with its entry",
                    ))
                } else {
                    Ok(())
                };
                let observed = controller
                    .observe(step.devices, instruction, self.machine.cpu())
                    .map_err(error);
                let monitored = if let Some(guard) = &mut irq {
                    guard.observe(
                        before_irq.expect("active IRQ checkpoint"),
                        &step,
                        self.machine.cpu(),
                    )
                } else {
                    Ok(())
                };
                let owner = if interrupt {
                    ProgressOwner::CompiledEngineInterrupt {
                        instruction: step.kind == StepKind::Instruction,
                    }
                } else {
                    ProgressOwner::CompiledOriginalCallee {
                        instruction: step.kind == StepKind::Instruction,
                    }
                };
                let recorded = NativeDrive {
                    machine: self.machine,
                    clock: self.clock,
                    timeline: self.timeline,
                    inputs: &mut self.inputs,
                    observer: &mut self.observer,
                    irq_frames: &mut self.state.irq_frames,
                    parents: &mut self.state.parents,
                }
                .record(None, step.devices, owner);
                reported?;
                observed?;
                monitored?;
                recorded?;
                if irq.as_ref().is_some_and(CompiledIrqGuard::finished)
                    && classify_boundary(self.machine.cpu(), signals(self.machine))
                        == BoundaryAction::Ready
                {
                    let (_, frames) = irq.take().expect("completed actual IRQ").receipt()?;
                    self.state.irq_frames = frames;
                }
                if let Some(start) = prepared_call
                    && step.kind == StepKind::Instruction
                {
                    let word = self
                        .machine
                        .devices()
                        .memory()
                        .read_u32(self.machine.cpu().read_address(7));
                    let call = controller
                        .suspend_compiled_call(start, step, self.machine.cpu(), word)
                        .map_err(error)?;
                    let target = tje_game::function_at(self.revision(), call.target())
                        .ok_or_else(|| error("original call resolved an unclassified target"))?;
                    if !entry.metadata().callees.contains(&target) {
                        return Err(error(
                            "original call escaped its committed callee whitelist",
                        ));
                    }
                    let parent_frame = self.frame;
                    let parent_class = self.class;
                    self.invoke_callee(controller.invocation(), target, &call)?;
                    let returned = self
                        .state
                        .completion
                        .finish_child(call, self.machine.cpu())
                        .map_err(error)?;
                    for parent in &mut self.state.parents {
                        parent
                            .continuation
                            .observe_return(returned.kind())
                            .map_err(error)?;
                    }
                    returned
                        .resume_original(&mut controller, self.machine.cpu(), signals(self.machine))
                        .map_err(error)?;
                    self.frame = parent_frame;
                    self.class = parent_class;
                }
            }
            let exit = self
                .state
                .completion
                .complete_original(controller, self.machine.cpu(), signals(self.machine))
                .map_err(error)?;
            Ok(exit.value)
        })();
        result.map_err(|cause| self.state.completion.protect_failure(cause))
    }
}

#[cfg(feature = "call-window-proofs")]
#[path = "call_window.rs"]
mod call_window;
#[cfg(feature = "call-window-proofs")]
pub use call_window::{CallWindow, CallWindowReceipt, WindowFailureObservation};
