//! Canonical prepared Context bridge. Root ledger and actual chronological
//! collector stay platform-owned; source code never receives an emulator.
use super::*;
use crate::{
    check_projection::{LogicEvent, tight_ym_busy_loop},
    execution::{ClockRun, PhysicalObservers},
};
use std::sync::Arc;
use tje_engine_runtime::{
    portable_timing::{
        ActiveSource, BoundProgram, CallIntent, CallProfile, CertifiedReturn, CompletionOwners,
        Invocation, OriginalController, PausedSource, PortableControl, PortableError,
        PortableInstruction, PreparedSources, ValidatedCalls,
    },
    timing_region::{CallToken, RegionWork, ReturnTarget},
};
use tje_game::{
    FunctionId,
    call_frame::FrameState,
    execution::{ClockUsage, ExecutionClass},
    timing as game,
};
use tje_rom::Rom;

#[cfg(test)]
pub(crate) mod proof;
#[cfg(test)]
mod tests;

fn portable_error(cause: PortableError) -> HostError {
    match cause {
        PortableError::Timing(cause) => timing_error(cause),
        PortableError::Source(cause) => cause.into(),
    }
}

pub(crate) struct TimingServices {
    revision: tje_rom::Revision,
    divider: u32,
    pub(crate) plans: PreparedSources,
    selected_dispatcher: tje_game::Dispatcher,
    check_selection: ValidatedCalls,
    live_selection: std::result::Result<ValidatedCalls, PortableError>,
    pub(crate) authority: SourceAuthority,
}
#[derive(Default)]
pub(crate) struct SourceAuthority {
    pub(crate) completion: CompletionOwners,
    pub(crate) parents: Vec<PausedHostSource>,
}
pub(crate) struct PausedHostSource {
    pub(crate) source: PausedSource,
    pub(crate) proof: SourceProof,
}
#[derive(Default)]
pub(crate) struct SourceProof {
    #[cfg(test)]
    oracle: Option<proof::Oracle>,
}
impl SourceProof {
    pub(crate) fn begin(
        state: &mut GenesisEmulator,
        clock: &WaitClock,
        audio_prefix: usize,
        parents: &[PausedHostSource],
    ) -> Self {
        #[cfg(test)]
        {
            if parents.iter().any(|parent| parent.proof.oracle.is_some()) {
                return Self::default();
            }
            Self {
                oracle: proof::Oracle::begin(state, clock, audio_prefix),
            }
        }
        #[cfg(not(test))]
        {
            let _ = (state, clock, audio_prefix, parents);
            Self {}
        }
    }
    fn step(
        &mut self,
        state: &mut GenesisEmulator,
        owner: ProgressOwner,
        step: DeviceStep,
    ) -> Result<()> {
        #[cfg(test)]
        if let Some(oracle) = &mut self.oracle {
            oracle.step(state, owner, step)?;
        }
        #[cfg(not(test))]
        let _ = (state, owner, step);
        Ok(())
    }
    fn finish(self, state: &mut GenesisEmulator, audio: &[[f64; 2]]) -> Result<()> {
        #[cfg(test)]
        if let Some(oracle) = self.oracle {
            oracle.finish(state, audio)?;
        }
        #[cfg(not(test))]
        let _ = (state, audio);
        Ok(())
    }
}
impl TimingServices {
    pub(crate) fn prepare(
        cartridge: &Rom,
        sources: &[&'static game::Program],
        divider: u32,
        dispatcher: tje_game::Dispatcher,
        mode: crate::Mode,
    ) -> Result<Self> {
        let bindings = sources
            .iter()
            .map(|&source| {
                BoundProgram::compile(cartridge, source.entry.function, source, divider, &[4, 6])
                    .map(Arc::new)
                    .map_err(timing_error)
            })
            .collect::<Result<Vec<_>>>()?;
        let plans = PreparedSources::new(bindings).map_err(timing_error)?;
        let check_selection = plans
            .validate_selection(dispatcher, CallProfile::HostCheckOriginalSubstitution)
            .map_err(portable_error)?;
        let live_selection = plans.validate_selection(dispatcher, CallProfile::HostHybridReference);
        if mode == crate::Mode::Replace
            && let Err(cause) = &live_selection
        {
            return Err(portable_error(cause.clone()));
        }
        let mut completion = CompletionOwners::default();
        completion.prepare_depth(plans.len());
        let parents = Vec::with_capacity(plans.len());
        Ok(Self {
            revision: cartridge.revision(),
            divider,
            plans,
            selected_dispatcher: dispatcher,
            check_selection,
            live_selection,
            authority: SourceAuthority {
                completion,
                parents,
            },
        })
    }
    pub(crate) fn usage(&self, entry: FunctionId) -> Option<RegionUsage> {
        self.plans.usage(entry)
    }
    pub(crate) fn profile(
        &self,
        mode: crate::Mode,
        dispatcher: tje_game::Dispatcher,
    ) -> Result<CallProfile> {
        if !self.selected_dispatcher.same_selection(dispatcher) {
            return Err(HostError::Metadata(
                "source call selection changed without startup revalidation".into(),
            ));
        }
        if mode == crate::Mode::Check {
            Ok(CallProfile::HostCheckOriginalSubstitution)
        } else {
            self.live_selection
                .as_ref()
                .map(|_| CallProfile::HostHybridReference)
                .map_err(|cause| portable_error(cause.clone()))
        }
    }
    pub(crate) fn permits_original(
        &self,
        mode: crate::Mode,
        dispatcher: tje_game::Dispatcher,
        entry: FunctionId,
    ) -> Result<bool> {
        let profile = self.profile(mode, dispatcher)?;
        let selection = if mode == crate::Mode::Check {
            &self.check_selection
        } else {
            self.live_selection
                .as_ref()
                .map_err(|cause| portable_error(cause.clone()))?
        };
        Ok(selection.permits_original(profile, entry))
    }
    pub(crate) fn discard_authority(&mut self, authority: SourceAuthority) -> Result<()> {
        for parent in authority.parents {
            self.plans
                .discard_paused(parent.source)
                .map_err(timing_error)?;
        }
        Ok(())
    }
    pub(crate) fn reset_after_actual_reset(&mut self) -> Result<()> {
        let old = std::mem::take(&mut self.authority);
        self.discard_authority(old)?;
        self.authority.completion.prepare_depth(self.plans.len());
        self.authority.parents.reserve(self.plans.len());
        Ok(())
    }
}

pub(crate) struct EntryScope<'a> {
    pub emulator: &'a GenesisEmulator,
    pub revision: tje_rom::Revision,
    pub frame: FrameState,
    pub class: ExecutionClass,
    pub clocked: bool,
    pub clock: &'a mut WaitClock,
    pub mode: crate::Mode,
    pub dispatcher: tje_game::Dispatcher,
}
pub(crate) struct PreparedInvocation {
    pub initial: CpuState,
    pub target: ReturnTarget,
    pub active: ActiveSource,
    pub profile: CallProfile,
}
pub(crate) fn prepare_entry(
    scope: EntryScope<'_>,
    services: &mut TimingServices,
    entry: FunctionId,
    source: &dyn game::TimingSource,
) -> Result<PreparedInvocation> {
    if !scope.clocked
        || !scope.clock.call_clock.contains_scope(entry)
        || tje_game::execution_class(entry) != Some(scope.class)
        || !scope.frame.is_prepared_for(entry)
        || scope.emulator.hybrid_bus().hybrid_functional_mode()
        || scope.revision != services.revision
        || scope.emulator.hybrid_bus().cycles.m68k_divider_u32.get() != services.divider
    {
        return Err(HostError::Metadata(
            "clocked prepared source capability is unavailable".into(),
        ));
    }
    scope.clock.call_clock.check()?;
    let profile = services.profile(scope.mode, scope.dispatcher)?;
    let initial = state(scope.emulator);
    let entry_sp = scope.frame.entry_sp;
    let physical = entry_sp & 0xffffff;
    let bytes = 4_u32
        .checked_mul(1 + u32::from(entry.metadata().abi.argument_count))
        .ok_or_else(|| HostError::Metadata("source frame extent overflow".into()))?;
    if initial.pc != entry.metadata().address(scope.revision)
        || initial.read_address(7) != entry_sp
        || entry_sp & 1 != 0
        || physical < 0xe00000
        || physical
            .checked_add(bytes)
            .is_none_or(|end| end > 0x1000000)
    {
        return Err(HostError::Metadata(
            "source entry is not its actual prepared CPU frame".into(),
        ));
    }
    let binding = services.plans.binding(entry).ok_or_else(|| {
        HostError::Metadata("source descriptor was not prepared at startup".into())
    })?;
    if !binding.accepts(entry, source.program()) {
        return Err(HostError::Metadata(
            "source program differs from its prepared descriptor".into(),
        ));
    }
    let memory = &scope.emulator.hybrid_bus().memory;
    let return_pc = (u32::from(memory.read_main_ram_word(entry_sp)) << 16)
        | u32::from(memory.read_main_ram_word(entry_sp.wrapping_add(2)));
    let target = binding.invocation_target(return_pc).map_err(timing_error)?;
    let active = services.plans.acquire(entry).map_err(timing_error)?;
    Ok(PreparedInvocation {
        initial,
        target,
        active,
        profile,
    })
}

pub(crate) struct ExecutionScope<'a> {
    pub emulator: &'a mut GenesisEmulator,
    #[cfg(test)]
    pub frame: FrameState,
    pub clock: &'a mut WaitClock,
    pub audio: Vec<[f64; 2]>,
    pub audit: bool,
    pub observers: PhysicalObservers<'a>,
}
pub(crate) enum SourceAdvance {
    Call {
        intent: CallIntent,
        token: CallToken,
    },
    Returned(CertifiedReturn),
}
pub(crate) struct SourceRequest<'a, 'program> {
    pub bound: &'a BoundProgram,
    pub region: &'a mut TimingRegion<'program>,
    pub source: &'a mut dyn game::TimingSource,
    pub invocation: &'a Invocation,
    pub parents: &'a mut [PausedHostSource],
    pub proof: SourceProof,
}
pub(crate) struct SourceRun {
    pub outcome: Result<SourceAdvance>,
    pub clock_run: ClockRun,
    pub proof: SourceProof,
}
pub(crate) fn run(scope: ExecutionScope<'_>, request: SourceRequest<'_, '_>) -> SourceRun {
    let mut run = ClockRun::default();
    run.audio = scope.audio;
    let auditing = scope.audit || scope.observers.enabled();
    let installed_trace = auditing && scope.emulator.hybrid_bus().hybrid_cpu_accesses.is_none();
    if installed_trace {
        scope.emulator.hybrid_bus_mut().hybrid_cpu_accesses = Some(Vec::new());
    }
    let (result, mut source_proof) = {
        let mut ledger = SourceLedger {
            run: &mut run,
            observers: scope.observers,
            base_depth: scope.emulator.hybrid_cpu().hybrid_exception_depth(),
            before: None,
            auditing,
            frame: scope.clock.frame,
            parents: request.parents,
            proof: request.proof,
        };
        let audio = std::mem::take(&mut ledger.run.audio);
        let result: Result<_> = (|| {
            let mut host = TimingHost::with_audio(scope.emulator, scope.clock, &mut ledger, audio)?;
            #[cfg(test)]
            host.seed_wrong_cycle_mutant(proof::take_mutation());
            let result = drive(
                &mut host,
                request.bound,
                request.region,
                request.source,
                request.invocation,
            );
            let audio = host.into_audio();
            Ok((result, audio))
        })();
        (result, std::mem::take(&mut ledger.proof))
    };
    let outcome = match result {
        Ok((result, audio)) => {
            run.audio = audio;
            if matches!(result, Ok(SourceAdvance::Returned(_))) {
                std::mem::take(&mut source_proof)
                    .finish(scope.emulator, &run.audio)
                    .and(result)
            } else {
                result
            }
        }
        Err(cause) => Err(cause),
    };
    if installed_trace {
        scope.emulator.hybrid_bus_mut().hybrid_cpu_accesses = None;
    }
    #[cfg(test)]
    if outcome.is_err() {
        proof::record_failure(scope.emulator, scope.frame, scope.clock.frame);
    }
    SourceRun {
        outcome,
        clock_run: run,
        proof: source_proof,
    }
}

pub(crate) fn run_original(
    scope: ExecutionScope<'_>,
    parents: &mut [PausedHostSource],
    controller: &mut OriginalController,
) -> (Result<()>, ClockRun) {
    let mut run = ClockRun::default();
    run.audio = scope.audio;
    let auditing = scope.audit || scope.observers.enabled();
    let installed_trace = auditing && scope.emulator.hybrid_bus().hybrid_cpu_accesses.is_none();
    if installed_trace {
        scope.emulator.hybrid_bus_mut().hybrid_cpu_accesses = Some(Vec::new());
    }
    let base_depth = scope.emulator.hybrid_cpu().hybrid_exception_depth();
    let result: Result<_> = {
        let mut ledger = SourceLedger {
            run: &mut run,
            observers: scope.observers,
            base_depth,
            before: None,
            auditing,
            frame: scope.clock.frame,
            parents,
            proof: SourceProof::default(),
        };
        let audio = std::mem::take(&mut ledger.run.audio);
        (|| {
            let mut host = TimingHost::with_audio(scope.emulator, scope.clock, &mut ledger, audio)?;
            let result = (|| {
                loop {
                    let cpu = host.state();
                    let boundary = host.boundary(&cpu);
                    if controller.saw_return() && boundary == BoundaryAction::Ready {
                        break;
                    }
                    if matches!(boundary, BoundaryAction::Faulted | BoundaryAction::Reset) {
                        return Err(HostError::Metadata(
                            "original callee abandoned by actual fault or reset".into(),
                        ));
                    }
                    host.step_original_callee(controller, base_depth)?;
                }
                Ok(())
            })();
            Ok((result, host.into_audio()))
        })()
    };
    let outcome = match result {
        Ok((result, audio)) => {
            run.audio = audio;
            result
        }
        Err(cause) => Err(cause),
    };
    if installed_trace {
        scope.emulator.hybrid_bus_mut().hybrid_cpu_accesses = None;
    }
    #[cfg(test)]
    if outcome.is_err() {
        proof::record_failure(scope.emulator, scope.frame, scope.clock.frame);
    }
    (outcome, run)
}

fn drive<L: TimingLedger>(
    host: &mut TimingHost<'_, L>,
    bound: &BoundProgram,
    region: &mut TimingRegion<'_>,
    source: &mut dyn game::TimingSource,
    invocation: &Invocation,
) -> Result<SourceAdvance> {
    let mut returned = None;
    loop {
        // Includes post-RTS debt/IRQ admission. Never execute a caller opcode.
        host.admit(region)?;
        if region.is_complete() {
            break;
        }
        let entry = host.state();
        let portable = bound
            .begin(source, &entry, region.return_target())
            .map_err(portable_error)?;
        let mut instruction = SourceInstruction {
            portable: Some(portable),
            control: None,
        };
        host.step_source(region, &mut instruction)?;
        match instruction.control.take().ok_or_else(|| {
            HostError::Metadata("source instruction produced no owned path".into())
        })? {
            PortableControl::Continue => {}
            PortableControl::Return(exit) => {
                returned = Some(exit);
                region
                    .complete_return(&host.state())
                    .map_err(timing_error)?;
            }
            PortableControl::Call(intent) => {
                let token = region.authorize_call(&host.state()).map_err(timing_error)?;
                let memory = &host.emulator().hybrid_bus().memory;
                let sp = token.entry_stack();
                let word = (u32::from(memory.read_main_ram_word(sp)) << 16)
                    | u32::from(memory.read_main_ram_word(sp.wrapping_add(2)));
                bound
                    .authenticate_call(intent, &token, &host.state(), word)
                    .map_err(timing_error)?;
                return Ok(SourceAdvance::Call { intent, token });
            }
        }
    }
    let exit = returned
        .ok_or_else(|| HostError::Metadata("source completed without an owned return".into()))?;
    bound
        .certify_return(region, invocation, &host.state(), exit, host.signals())
        .map(SourceAdvance::Returned)
        .map_err(timing_error)
}

struct SourceInstruction<'a> {
    portable: Option<PortableInstruction<'a>>,
    control: Option<PortableControl>,
}
impl RustInstruction for SourceInstruction<'_> {
    fn entry_state(&self) -> &CpuState {
        self.portable
            .as_ref()
            .expect("active source instruction")
            .entry_state()
    }
    fn effect(
        &mut self,
        emulator: &mut GenesisEmulator,
        expected: ExpectedAccess,
    ) -> Result<BusObservation> {
        let portable = self
            .portable
            .as_mut()
            .ok_or_else(|| HostError::Metadata("source instruction already finished".into()))?;
        let request = portable.request(expected).map_err(portable_error)?;
        let write = match request {
            Some(game::Effect {
                kind: game::EffectKind::Write(value),
                ..
            }) => Some(u32::from(value)),
            _ => None,
        };
        let actual = rust_bus_effect(emulator, expected, write)?;
        portable.acknowledge(actual).map_err(portable_error)?;
        Ok(actual)
    }
    fn finish(&mut self, emulator: &mut GenesisEmulator) -> Result<RustPathChoice> {
        let portable = self
            .portable
            .take()
            .ok_or_else(|| HostError::Metadata("source finish replayed".into()))?;
        let finished = portable.finish().map_err(portable_error)?;
        let registers = finished.registers;
        let cpu = emulator.hybrid_parts().0;
        cpu.set_data_registers(registers.data);
        cpu.set_address_registers(
            registers.address,
            registers.user_stack,
            registers.supervisor_stack,
        );
        cpu.set_status_register(registers.status);
        self.control = Some(finished.control);
        Ok(finished.path)
    }
}

struct BeforeStep {
    pc: u32,
    clock: u64,
    depth: u32,
    instructions: u64,
    ym_loop: bool,
}
struct SourceLedger<'a> {
    run: &'a mut ClockRun,
    observers: PhysicalObservers<'a>,
    base_depth: u32,
    before: Option<BeforeStep>,
    auditing: bool,
    frame: u64,
    parents: &'a mut [PausedHostSource],
    proof: SourceProof,
}
fn attempts(state: &GenesisEmulator) -> Result<u64> {
    let counts = state
        .hybrid_cpu()
        .hybrid_instruction_counts()
        .ok_or_else(|| HostError::Metadata("source reference counter is unavailable".into()))?;
    counts
        .main_thread
        .checked_add(counts.exception_handler)
        .ok_or_else(|| HostError::Metadata("source reference counter overflow".into()))
}
impl TimingLedger for SourceLedger<'_> {
    fn before_step(&mut self, _owner: ProgressOwner, clock: &mut WaitClock) -> Result<()> {
        if !clock.call_clock.active() {
            return Err(HostError::Metadata(
                "timing source lost its root clock".into(),
            ));
        }
        clock.call_clock.before_tick()?;
        Ok(())
    }
    fn prepare_actual_step(
        &mut self,
        owner: ProgressOwner,
        state: &GenesisEmulator,
        frame: u64,
    ) -> Result<()> {
        let pc = state.hybrid_cpu().pc();
        let clock = state.hybrid_bus().cycles.m68k_mclk_cycles;
        self.frame = frame;
        if let Some(provenance) = self.observers.provenance {
            match owner {
                ProgressOwner::RustInstruction => provenance.begin_native(frame, clock),
                _ => provenance.begin_step(state.hybrid_cpu(), clock, frame),
            }
        }
        if let Some(capture) = self.observers.capture {
            match owner {
                ProgressOwner::OriginalInterrupt { .. } | ProgressOwner::OriginalCallee { .. } => {
                    capture.begin_cpu_step(state, frame)
                }
                _ => capture.begin_cpu_checkpoint(
                    crate::check_capture::CpuCheckpoint::capture(state, frame),
                    state.hybrid_bus(),
                    None,
                ),
            }
        }
        self.before = Some(BeforeStep {
            pc,
            clock,
            depth: state.hybrid_cpu().hybrid_exception_depth(),
            instructions: attempts(state)?,
            ym_loop: self.auditing && tight_ym_busy_loop(state, pc),
        });
        Ok(())
    }
    fn record_step(
        &mut self,
        _owner: ProgressOwner,
        step: DeviceStep,
        state: &GenesisEmulator,
        clock: &mut WaitClock,
    ) -> Result<()> {
        self.frame = clock.frame;
        let before = self
            .before
            .as_ref()
            .ok_or_else(|| HostError::Metadata("actual step was not prepared".into()))?;
        let instructions = attempts(state)?
            .checked_sub(before.instructions)
            .ok_or_else(|| HostError::Metadata("actual source counter decreased".into()))?;
        clock.call_clock.record(ClockUsage {
            ticks: 1,
            instructions,
            master_clocks: step.master_clocks,
            frames: u64::from(step.frame_complete),
            dispatch_steps: 0,
        })?;
        clock.call_clock.check()?;
        Ok(())
    }
    fn capture_actual_step(
        &mut self,
        _owner: ProgressOwner,
        state: &mut GenesisEmulator,
        step: Option<DeviceStep>,
    ) -> Result<()> {
        let before = self
            .before
            .take()
            .ok_or_else(|| HostError::Metadata("actual capture was not prepared".into()))?;
        let depth = state.hybrid_cpu().hybrid_exception_depth();
        if let Some(capture) = self.observers.capture {
            match _owner {
                ProgressOwner::OriginalInterrupt { .. } | ProgressOwner::OriginalCallee { .. } => {
                    capture.finish_reference_cpu_step(state, self.frame)
                }
                _ => capture.finish_cpu_checkpoint(
                    crate::check_capture::CpuCheckpoint::capture(state, self.frame),
                    None,
                ),
            }
        }
        if self.auditing && depth > before.depth {
            let vector = state
                .hybrid_cpu()
                .hybrid_last_exception_vector()
                .ok_or_else(|| {
                    HostError::Metadata("actual exception entry has no vector".into())
                })?;
            self.run
                .logic_trace
                .events
                .push(LogicEvent::ExceptionEntered {
                    vector,
                    handler: state.hybrid_cpu().pc(),
                    depth: depth - self.base_depth,
                    master_clock: before.clock,
                });
        }
        let mut observers = self.observers;
        if !matches!(
            _owner,
            ProgressOwner::OriginalInterrupt { .. } | ProgressOwner::OriginalCallee { .. }
        ) && let Some(capture) = observers.capture
        {
            capture.observe_native_batch_parts(state.hybrid_cpu(), state.hybrid_bus(), self.frame);
            observers.capture = None;
        }
        crate::execution::drain_accesses(
            state,
            before.pc,
            before.clock,
            self.frame,
            before.ym_loop,
            self.run,
            observers,
        );
        if self.auditing && depth < before.depth {
            let depth = depth.checked_sub(self.base_depth).ok_or_else(|| {
                HostError::Metadata("source ISR escaped below its actual entry depth".into())
            })?;
            self.run
                .logic_trace
                .events
                .push(LogicEvent::ExceptionReturned {
                    depth,
                    master_clock: state.hybrid_bus().cycles.m68k_mclk_cycles,
                });
        }
        if let Some(step) = step {
            self.run.ticks = self
                .run
                .ticks
                .checked_add(1)
                .ok_or_else(|| HostError::Metadata("source tick count overflow".into()))?;
            self.run.frames = self
                .run
                .frames
                .checked_add(u64::from(step.frame_complete))
                .ok_or_else(|| HostError::Metadata("source frame count overflow".into()))?;
            let delta = attempts(state)?
                .checked_sub(before.instructions)
                .ok_or_else(|| {
                    HostError::Metadata("actual reference opcode count regressed".into())
                })?;
            let owner = if matches!(
                _owner,
                ProgressOwner::OriginalInterrupt { .. } | ProgressOwner::OriginalCallee { .. }
            ) && delta == 1
            {
                let instruction = state
                    .hybrid_cpu()
                    .hybrid_instruction_counts()
                    .and_then(|counts| counts.last_instruction)
                    .ok_or_else(|| {
                        HostError::Metadata("actual reference opcode has no receipt".into())
                    })?;
                if instruction.exception_depth > self.base_depth {
                    ProgressOwner::OriginalInterrupt { instruction: true }
                } else {
                    ProgressOwner::OriginalCallee { instruction: true }
                }
            } else {
                _owner
            };
            let work = match owner {
                ProgressOwner::RustInstruction => RegionWork::SourceInstruction,
                ProgressOwner::OriginalInterrupt { instruction: true } => {
                    RegionWork::OriginalIsrInstruction
                }
                ProgressOwner::OriginalCallee { instruction: true } => {
                    RegionWork::OriginalCalleeInstruction
                }
                _ => RegionWork::Hardware,
            };
            for parent in self.parents.iter_mut() {
                parent.source.observe(step, work).map_err(timing_error)?;
                parent.proof.step(state, owner, step)?;
            }
            self.proof.step(state, owner, step)?;
            #[cfg(test)]
            proof::maybe_fail(owner, before.depth, depth, step)?;
        }
        Ok(())
    }
}
