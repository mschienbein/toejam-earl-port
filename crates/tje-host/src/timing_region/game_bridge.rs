//! Canonical prepared Context bridge. Root ledger and actual chronological
//! collector stay platform-owned; source code never receives an emulator.
use super::*;
use crate::{
    check_projection::{LogicEvent, tight_ym_busy_loop},
    execution::{ClockRun, PhysicalObservers},
};
use std::collections::BTreeMap;
use tje_engine_runtime::{
    portable_timing::{
        BoundProgram, CertifiedReturn, CompletionLedger, Invocation, PortableError,
        PortableInstruction,
    },
    timing_region::RegionStorage,
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

pub(crate) struct SourceState {
    cache: BoundProgram,
    storage: RegionStorage,
    usage: Option<RegionUsage>,
}
/// Immutable plans and reusable caller-owned buffers, prepared before playback.
/// The descriptors are not a registry activation or execution-class override.
pub(crate) struct TimingServices {
    revision: tje_rom::Revision,
    divider: u32,
    entries: BTreeMap<FunctionId, SourceState>,
    pub(crate) completion: CompletionLedger,
}
impl TimingServices {
    pub(crate) fn prepare(
        cartridge: &Rom,
        sources: &[&'static game::Program],
        divider: u32,
    ) -> Result<Self> {
        let mut entries = BTreeMap::new();
        for &source in sources {
            let entry = source.entry.function;
            let cache = BoundProgram::compile(cartridge, entry, source, divider, &[4, 6])
                .map_err(timing_error)?;
            let storage = cache.prepare_storage();
            if entries
                .insert(
                    entry,
                    SourceState {
                        cache,
                        storage,
                        usage: None,
                    },
                )
                .is_some()
            {
                return Err(HostError::Metadata(
                    "duplicate prepared timing source".into(),
                ));
            }
        }
        Ok(Self {
            revision: cartridge.revision(),
            divider,
            entries,
            completion: CompletionLedger::default(),
        })
    }
    pub(crate) fn usage(&self, entry: FunctionId) -> Option<RegionUsage> {
        self.entries.get(&entry).and_then(|state| state.usage)
    }
}

pub(crate) struct ExecutionScope<'a> {
    pub emulator: &'a mut GenesisEmulator,
    pub revision: tje_rom::Revision,
    pub frame: FrameState,
    pub class: ExecutionClass,
    pub clocked: bool,
    pub clock: &'a mut WaitClock,
    pub audio: Vec<[f64; 2]>,
    pub audit: bool,
    pub observers: PhysicalObservers<'a>,
}
pub(crate) struct SourceRun {
    pub outcome: Result<game::TimedExit>,
    pub clock_run: ClockRun,
    pub admitted: bool,
}

pub(crate) fn execute(
    scope: ExecutionScope<'_>,
    services: &mut TimingServices,
    entry: FunctionId,
    source: &mut dyn game::TimingSource,
) -> SourceRun {
    let mut run = ClockRun::default();
    run.audio = scope.audio;
    let mut admitted = false;
    let outcome = (|| {
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
        let prepared = services.entries.get_mut(&entry).ok_or_else(|| {
            HostError::Metadata("source descriptor was not prepared at startup".into())
        })?;
        let completion = &mut services.completion;
        if !prepared.cache.accepts(entry, source.program()) {
            return Err(HostError::Metadata(
                "source program differs from its prepared descriptor".into(),
            ));
        }
        // Metadata-only entry-frame peek; the real source RTS reads remain real
        // tagged bus effects, and no caller opcode is read or substituted here.
        let memory = &scope.emulator.hybrid_bus().memory;
        let return_pc = (u32::from(memory.read_main_ram_word(entry_sp)) << 16)
            | u32::from(memory.read_main_ram_word(entry_sp.wrapping_add(2)));
        let target = prepared
            .cache
            .invocation_target(return_pc)
            .map_err(timing_error)?;
        let mut region = TimingRegion::begin_oracle(
            prepared.cache.timing(),
            &initial,
            target,
            &mut prepared.storage,
        )
        .map_err(timing_error)?;
        let invocation = completion
            .begin(entry, entry_sp, &region)
            .map_err(timing_error)?;
        admitted = true;
        #[cfg(test)]
        proof::begin_invocation();
        scope.clock.call_clock.dispatch_step()?;
        let auditing = scope.audit || scope.observers.enabled();
        let installed_trace = auditing && scope.emulator.hybrid_bus().hybrid_cpu_accesses.is_none();
        if installed_trace {
            scope.emulator.hybrid_bus_mut().hybrid_cpu_accesses = Some(Vec::new());
        }
        let audio = std::mem::take(&mut run.audio);
        #[cfg(test)]
        let proof = proof::Oracle::begin(scope.emulator, scope.clock, audio.len());
        let (result, audio) = {
            let mut ledger = SourceLedger {
                run: &mut run,
                observers: scope.observers,
                base_depth: scope.emulator.hybrid_cpu().hybrid_exception_depth(),
                before: None,
                auditing,
                frame: scope.clock.frame,
                #[cfg(test)]
                proof,
            };
            // The functional-bus guard was checked before transferring audio.
            let mut host = TimingHost::with_audio(scope.emulator, scope.clock, &mut ledger, audio)?;
            #[cfg(test)]
            host.seed_wrong_cycle_mutant(proof::take_mutation());
            let result = drive(&mut host, &prepared.cache, &mut region, source, &invocation);
            let audio = host.into_audio();
            #[cfg(test)]
            let result = result.and_then(|certificate| {
                if let Some(proof) = ledger.proof.take() {
                    proof.finish(scope.emulator, &audio)?;
                }
                Ok(certificate)
            });
            (result, audio)
        };
        run.audio = audio;
        prepared.usage = Some(region.usage());
        if installed_trace {
            // Every actual step (including errors) has already consumed its
            // indexed payload before this optional buffer is detached.
            scope.emulator.hybrid_bus_mut().hybrid_cpu_accesses = None;
        }
        result.and_then(|certificate| {
            completion
                .complete(invocation, certificate)
                .map_err(timing_error)
        })
    })();
    if outcome.is_err() && admitted {
        services.completion.fail();
        #[cfg(test)]
        proof::record_failure(scope.emulator, scope.frame, scope.clock.frame);
    }
    SourceRun {
        outcome,
        clock_run: run,
        admitted,
    }
}

fn drive<L: TimingLedger>(
    host: &mut TimingHost<'_, L>,
    bound: &BoundProgram,
    region: &mut TimingRegion<'_>,
    source: &mut dyn game::TimingSource,
    invocation: &Invocation,
) -> Result<CertifiedReturn> {
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
            returned: None,
        };
        host.step_source(region, &mut instruction)?;
        if let Some(exit) = instruction.returned {
            returned = Some(exit);
            region
                .complete_return(&host.state())
                .map_err(timing_error)?;
        }
    }
    let exit = returned
        .ok_or_else(|| HostError::Metadata("source completed without an owned return".into()))?;
    bound
        .certify_return(region, invocation, &host.state(), exit, host.signals())
        .map_err(timing_error)
}

struct SourceInstruction<'a> {
    portable: Option<PortableInstruction<'a>>,
    returned: Option<game::TimedExit>,
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
        let (registers, path, returned) = portable.finish().map_err(portable_error)?;
        let cpu = emulator.hybrid_parts().0;
        cpu.set_data_registers(registers.data);
        cpu.set_address_registers(
            registers.address,
            registers.user_stack,
            registers.supervisor_stack,
        );
        cpu.set_status_register(registers.status);
        self.returned = returned;
        Ok(path)
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
    #[cfg(test)]
    proof: Option<proof::Oracle>,
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
                ProgressOwner::OriginalInterrupt { .. } => capture.begin_cpu_step(state, frame),
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
                ProgressOwner::OriginalInterrupt { .. } => {
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
        if !matches!(_owner, ProgressOwner::OriginalInterrupt { .. })
            && let Some(capture) = observers.capture
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
            #[cfg(test)]
            if let Some(proof) = self.proof.as_mut() {
                proof.step(state, _owner, step)?;
            }
            #[cfg(test)]
            proof::maybe_fail(_owner, before.depth, depth, step)?;
        }
        Ok(())
    }
}
