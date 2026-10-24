//! Check-mode bridge for dependency-only, Rust-owned timing regions.
//!
//! Original code is used only for explicitly reported interrupt service and
//! hardware noninstruction steps. The main region is never executed by the
//! original CPU, and no register/stack/PC repair is performed after an ISR.
use crate::{
    HostError, Result,
    execution::WaitClock,
    sinks::{HeadlessAudio, HeadlessOutput, Inputs, NoCartridgeSaves},
};
use genesis_core::GenesisEmulator;
use jgenesis_common::frontend::{EmulatorTrait, TickEffect};
use m68000_emu::BusInterface;
use tje_engine_core::{
    CpuState, ReadKind,
    ir::Width,
    timing_region::{
        AccessKind, BusObservation, ExpectedAccess, RustPathChoice, TimingError, TimingEvent,
    },
};
use tje_engine_runtime::{
    DeviceStep,
    timing_region::{
        BoundaryAction, BoundarySignals, RegionUsage, TimingRegion, classify_boundary,
    },
};

pub(crate) mod game_bridge;
pub mod wait_vblank;

fn timing_error(error: TimingError) -> HostError {
    HostError::Metadata(error.to_string())
}

/// Rust supplies every source effect and branch. The scheduler can inspect an
/// effect's expected identity, but cannot perform it or invent its read value.
pub trait RustInstruction {
    fn entry_state(&self) -> &CpuState;
    fn effect(
        &mut self,
        emulator: &mut GenesisEmulator,
        expected: ExpectedAccess,
    ) -> Result<BusObservation>;
    fn finish(&mut self, emulator: &mut GenesisEmulator) -> Result<RustPathChoice>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressOwner {
    RustInstruction,
    OriginalInterrupt { instruction: bool },
    HardwareWait,
}
/// Root call ceilings can plug into this exact per-step hook. No new deadline is
/// created by a region or nested instruction. Returning an error prevents issue.
pub trait TimingLedger {
    fn before_step(&mut self, owner: ProgressOwner, clock: &mut WaitClock) -> Result<()>;
    fn record_step(
        &mut self,
        owner: ProgressOwner,
        step: DeviceStep,
        state: &GenesisEmulator,
        clock: &mut WaitClock,
    ) -> Result<()>;
    /// Optional actual-actor audit, distinct from the shared root budget.
    fn prepare_actual_step(
        &mut self,
        _owner: ProgressOwner,
        _state: &GenesisEmulator,
        _frame: u64,
    ) -> Result<()> {
        Ok(())
    }
    /// Always called after an attempted real step, including Result errors.
    fn capture_actual_step(
        &mut self,
        _owner: ProgressOwner,
        _state: &mut GenesisEmulator,
        _step: Option<DeviceStep>,
    ) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectReceipt {
    pub expected: ExpectedAccess,
    pub observation: BusObservation,
    pub master_clock: u64,
}

pub struct TimingHost<'a, L> {
    emulator: &'a mut GenesisEmulator,
    clock: &'a mut WaitClock,
    ledger: &'a mut L,
    audio: HeadlessAudio,
    effects: Vec<EffectReceipt>,
    collect_effects: bool,
    counting_before: bool,
    last_step: Option<DeviceStep>,
    #[cfg(test)]
    cycle_mutation: u32,
}
impl<'a, L: TimingLedger> TimingHost<'a, L> {
    pub fn new(
        emulator: &'a mut GenesisEmulator,
        clock: &'a mut WaitClock,
        ledger: &'a mut L,
    ) -> Result<Self> {
        Self::build(emulator, clock, ledger, HeadlessAudio::default(), true)
    }
    pub(crate) fn with_audio(
        emulator: &'a mut GenesisEmulator,
        clock: &'a mut WaitClock,
        ledger: &'a mut L,
        audio: Vec<[f64; 2]>,
    ) -> Result<Self> {
        Self::build(
            emulator,
            clock,
            ledger,
            HeadlessAudio { samples: audio },
            false,
        )
    }
    fn build(
        emulator: &'a mut GenesisEmulator,
        clock: &'a mut WaitClock,
        ledger: &'a mut L,
        audio: HeadlessAudio,
        collect_effects: bool,
    ) -> Result<Self> {
        if emulator.hybrid_bus().hybrid_functional_mode() {
            return Err(HostError::Metadata(
                "a timing-only region cannot use a zero-clock Functional bus".into(),
            ));
        }
        let cpu = emulator.hybrid_parts().0;
        let previous = cpu
            .hybrid_instruction_counts()
            .map(|counts| counts.enabled());
        if previous.is_none() {
            cpu.hybrid_enable_instruction_counts();
        } else {
            cpu.hybrid_set_instruction_counting(true);
        }
        Ok(Self {
            emulator,
            clock,
            ledger,
            audio,
            effects: Vec::new(),
            collect_effects,
            counting_before: previous.unwrap_or(false),
            last_step: None,
            #[cfg(test)]
            cycle_mutation: 0,
        })
    }
    pub fn state(&self) -> CpuState {
        state(self.emulator)
    }
    pub fn effects(&self) -> &[EffectReceipt] {
        &self.effects
    }
    pub fn samples(&self) -> &[[f64; 2]] {
        &self.audio.samples
    }
    pub fn usage(&self, region: &TimingRegion<'_>) -> RegionUsage {
        region.usage()
    }
    pub fn emulator(&self) -> &GenesisEmulator {
        self.emulator
    }
    pub(crate) fn into_audio(mut self) -> Vec<[f64; 2]> {
        std::mem::take(&mut self.audio.samples)
    }
    #[cfg(test)]
    pub(crate) fn seed_wrong_cycle_mutant(&mut self, cycles: u32) {
        self.cycle_mutation = cycles;
    }

    /// Reach a legal original instruction boundary without executing a single
    /// main-region opcode. ISR ownership is explicit; a terminal/nonlocal exit
    /// is rejected rather than restoring the old main state to fit the region.
    pub fn admit(&mut self, region: &mut TimingRegion<'_>) -> Result<()> {
        loop {
            let cpu = self.state();
            let action = self.boundary(&cpu);
            match action {
                BoundaryAction::Ready => return Ok(()),
                BoundaryAction::Faulted => return Err(timing_error(TimingError::PendingFault)),
                BoundaryAction::Wait | BoundaryAction::Halted | BoundaryAction::Stopped => {
                    self.original_noninstruction(region, ProgressOwner::HardwareWait)?;
                }
                BoundaryAction::Reset => {
                    return Err(HostError::Metadata(
                        "timing region abandoned by hardware reset".into(),
                    ));
                }
                BoundaryAction::InterruptAdmission { .. }
                | BoundaryAction::InterruptAcknowledge { .. } => {
                    let token = region
                        .suspend_interrupt(&cpu, action)
                        .map_err(timing_error)?;
                    let base_depth = self.emulator.hybrid_cpu().hybrid_exception_depth();
                    let mut entered = false;
                    let mut instructions = 0_u64;
                    let mut rte_observed = false;
                    loop {
                        let before = self.state();
                        let action = self.boundary(&before);
                        let executing = action == BoundaryAction::Ready;
                        if matches!(action, BoundaryAction::Reset) {
                            return Err(HostError::Metadata(
                                "ISR timing region abandoned by reset".into(),
                            ));
                        }
                        if matches!(action, BoundaryAction::Faulted) {
                            return Err(timing_error(TimingError::PendingFault));
                        }
                        let before_depth = self.emulator.hybrid_cpu().hybrid_exception_depth();
                        self.original_noninstruction(
                            region,
                            ProgressOwner::OriginalInterrupt {
                                instruction: executing,
                            },
                        )?;
                        let depth = self.emulator.hybrid_cpu().hybrid_exception_depth();
                        entered |= depth > base_depth;
                        if depth < base_depth {
                            return Err(HostError::Metadata(
                                "ISR exited below the suspended exception depth".into(),
                            ));
                        }
                        if executing {
                            instructions += 1;
                            rte_observed |= before.prefetch == 0x4e73 && depth < before_depth;
                        }
                        if entered && depth == base_depth && rte_observed {
                            region
                                .resume_oracle_interrupt(token, &self.state(), instructions, true)
                                .map_err(timing_error)?;
                            break;
                        }
                    }
                }
            }
        }
    }
    fn boundary(&self, cpu: &CpuState) -> BoundaryAction {
        classify_boundary(cpu, self.signals())
    }
    pub(crate) fn signals(&self) -> BoundarySignals {
        let bus = self.emulator.hybrid_bus();
        BoundarySignals {
            wait: bus.cycles.m68k_wait_cpu_cycles != 0,
            reset: bus.m68k_reset,
            halted: bus.vdp.should_halt_cpu(),
            interrupt_level: bus.interrupt_level(),
        }
    }
    fn original_noninstruction(
        &mut self,
        region: &mut TimingRegion<'_>,
        owner: ProgressOwner,
    ) -> Result<()> {
        region.check_step_budget().map_err(timing_error)?;
        if owner == ProgressOwner::HardwareWait
            && self.boundary(&self.state()) == BoundaryAction::Ready
        {
            return Err(HostError::Metadata(
                "original main execution forbidden in timing adapter".into(),
            ));
        }
        self.ledger.before_step(owner, self.clock)?;
        let before = self.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
        let before_count = Self::instruction_total(self.emulator)?;
        self.ledger
            .prepare_actual_step(owner, self.emulator, self.clock.frame)?;
        self.last_step = None;
        let result = (|| {
            let effect = self
                .emulator
                .tick(
                    &mut HeadlessOutput,
                    &mut self.audio,
                    &mut Inputs(self.clock.inputs),
                    &mut NoCartridgeSaves,
                )
                .map_err(|e| HostError::Emulator(e.to_string()))?;
            let delta = Self::instruction_total(self.emulator)?
                .checked_sub(before_count)
                .ok_or_else(|| {
                    HostError::Metadata("actual instruction counter decreased".into())
                })?;
            let owner = match owner {
                ProgressOwner::OriginalInterrupt { .. } if delta <= 1 => {
                    ProgressOwner::OriginalInterrupt {
                        instruction: delta == 1,
                    }
                }
                ProgressOwner::HardwareWait if delta == 0 => ProgressOwner::HardwareWait,
                _ => {
                    return Err(HostError::Metadata(
                        "timing adapter executed an undeclared original instruction".into(),
                    ));
                }
            };
            self.record(region, owner, before, effect)
        })();
        let captured = self
            .ledger
            .capture_actual_step(owner, self.emulator, self.last_step);
        result.and(captured)
    }
    fn record(
        &mut self,
        region: &mut TimingRegion<'_>,
        owner: ProgressOwner,
        before: u64,
        effect: TickEffect,
    ) -> Result<()> {
        let bus = self.emulator.hybrid_bus();
        let clocks = bus
            .cycles
            .m68k_mclk_cycles
            .checked_sub(before)
            .ok_or_else(|| HostError::Metadata("device clock regressed".into()))?;
        let cycles = u32::try_from(clocks / bus.cycles.m68k_divider.get())
            .map_err(|_| HostError::Metadata("timing step charge overflow".into()))?;
        let step = DeviceStep {
            cpu_cycles: cycles,
            master_clocks: clocks,
            frame_complete: effect == TickEffect::FrameRendered,
        };
        self.last_step = Some(step);
        let progress = region.record_progress(step).map_err(timing_error);
        let retired = if matches!(
            owner,
            ProgressOwner::OriginalInterrupt { instruction: true }
        ) {
            region.record_original_instruction().map_err(timing_error)
        } else {
            Ok(())
        };
        let input = if step.frame_complete {
            self.clock.advance_frame(self.emulator)
        } else {
            Ok(())
        };
        // Account for a real completed step even when its input/recipe sample
        // fails, but preserve the earlier input error over a ceiling failure.
        let budget = self
            .ledger
            .record_step(owner, step, self.emulator, self.clock);
        input?;
        progress?;
        retired?;
        budget
    }

    pub fn step_source(
        &mut self,
        region: &mut TimingRegion<'_>,
        source: &mut impl RustInstruction,
    ) -> Result<()> {
        self.admit(region)?;
        self.ledger
            .before_step(ProgressOwner::RustInstruction, self.clock)?;
        self.ledger.prepare_actual_step(
            ProgressOwner::RustInstruction,
            self.emulator,
            self.clock.frame,
        )?;
        self.last_step = None;
        let before_count = Self::instruction_total(self.emulator)?;
        let result = self.step_source_inner(region, source);
        let captured = self.ledger.capture_actual_step(
            ProgressOwner::RustInstruction,
            self.emulator,
            self.last_step,
        );
        let actual = Self::instruction_total(self.emulator);
        result?;
        captured?;
        let actual = actual?;
        if actual != before_count {
            return Err(HostError::Metadata(
                "Rust source executed a forbidden original CPU opcode".into(),
            ));
        }
        Ok(())
    }
    fn step_source_inner(
        &mut self,
        region: &mut TimingRegion<'_>,
        source: &mut impl RustInstruction,
    ) -> Result<()> {
        let entry = self.state();
        if source.entry_state() != &entry {
            return Err(HostError::Metadata(
                "Rust source state predates timing-boundary admission".into(),
            ));
        }
        let mut cursor = region
            .cursor(&entry, BoundaryAction::Ready)
            .map_err(timing_error)?;
        self.emulator
            .hybrid_begin_timing_instruction(self.clock.inputs);
        let timing = loop {
            match cursor.next_event().map_err(timing_error)? {
                TimingEvent::Bus { token, expected } => {
                    let clock = self
                        .collect_effects
                        .then(|| self.emulator.hybrid_bus().cycles.m68k_mclk_cycles);
                    let observation = source.effect(self.emulator, expected)?;
                    cursor
                        .acknowledge_bus(token, observation)
                        .map_err(timing_error)?;
                    if let Some(clock) = clock {
                        self.effects.push(EffectReceipt {
                            expected,
                            observation,
                            master_clock: clock,
                        });
                    }
                }
                TimingEvent::SetReadOffset {
                    token,
                    master_clocks,
                } => {
                    let bus = self.emulator.hybrid_bus();
                    let expected =
                        tje_engine_core::timing::cycles_if_move_btst_cmp(entry.prefetch as u16)
                            .map_or(8, |c| c - 4) as u64
                            * bus.cycles.m68k_divider.get();
                    if u64::from(master_clocks) != expected {
                        return Err(HostError::Metadata(
                            "timing plan divider/read phase changed".into(),
                        ));
                    }
                    cursor.acknowledge_offset(token).map_err(timing_error)?;
                }
                TimingEvent::InstructionComplete { token, .. } => {
                    let path = source.finish(self.emulator)?;
                    break cursor
                        .complete_instruction(token, path)
                        .map_err(timing_error)?;
                }
                TimingEvent::Exception { .. } => return Err(HostError::Metadata(
                    "timing region exits through a typed exception; no implicit original fallback"
                        .into(),
                )),
            }
        };
        let timing = region.accept_instruction(timing).map_err(timing_error)?;
        let pipeline = timing.pipeline();
        let (cpu, bus) = self.emulator.hybrid_parts();
        cpu.hybrid_finish_timing_instruction(
            pipeline.pc,
            pipeline.prefetch as u16,
            pipeline.last_muldiv != 0,
            bus,
        );
        let before = self.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
        let cycles = timing.cycles();
        #[cfg(test)]
        let cycles = cycles
            .checked_add(std::mem::take(&mut self.cycle_mutation))
            .ok_or_else(|| HostError::Metadata("test charge mutation overflow".into()))?;
        let effect = self
            .emulator
            .hybrid_advance_timing_cycles(
                timing.pc(),
                cycles,
                false,
                &mut HeadlessOutput,
                &mut self.audio,
                &mut NoCartridgeSaves,
            )
            .map_err(|e| HostError::Emulator(e.to_string()))?;
        self.record(region, ProgressOwner::RustInstruction, before, effect)
    }
    fn instruction_total(state: &GenesisEmulator) -> Result<u64> {
        let counts = state
            .hybrid_cpu()
            .hybrid_instruction_counts()
            .ok_or_else(|| {
                HostError::Metadata("actual instruction observer is unavailable".into())
            })?;
        counts
            .main_thread
            .checked_add(counts.exception_handler)
            .ok_or_else(|| HostError::Metadata("actual instruction counter overflow".into()))
    }
}

impl<L> Drop for TimingHost<'_, L> {
    fn drop(&mut self) {
        self.emulator
            .hybrid_parts()
            .0
            .hybrid_set_instruction_counting(self.counting_before);
    }
}

/// Rust effect helper. It is never called by Core or by a timing expression;
/// source bodies call it once at their declared effect point. Writes retain the
/// reference's deferred queues and are NOT immediately committed by this helper.
pub fn rust_bus_effect(
    emulator: &mut GenesisEmulator,
    expected: ExpectedAccess,
    write_value: Option<u32>,
) -> Result<BusObservation> {
    let purpose = match expected.kind {
        AccessKind::Read(ReadKind::Semantic) => m68000_emu::ReadPurpose::Semantic,
        AccessKind::Read(ReadKind::Prefetch) => m68000_emu::ReadPurpose::Prefetch,
        AccessKind::Read(ReadKind::Discarded) => m68000_emu::ReadPurpose::Discarded,
        AccessKind::Write => m68000_emu::ReadPurpose::Semantic,
    };
    let bus = emulator.hybrid_bus_mut();
    let value = match (expected.kind, expected.width, write_value) {
        (AccessKind::Read(_), Width::Byte, None) => {
            u32::from(bus.read_byte_tagged(expected.address, purpose))
        }
        (AccessKind::Read(_), Width::Word, None) => {
            u32::from(bus.read_word_tagged(expected.address, purpose))
        }
        (AccessKind::Write, Width::Byte, Some(value)) if value <= 255 => {
            bus.write_byte(expected.address, value as u8);
            value
        }
        (AccessKind::Write, Width::Word, Some(value)) if value <= 65535 => {
            bus.write_word(expected.address, value as u16);
            value
        }
        _ => {
            return Err(HostError::Metadata(
                "Rust effect read/write value contract mismatch".into(),
            ));
        }
    };
    Ok(BusObservation {
        address: expected.address,
        width: expected.width,
        kind: expected.kind,
        value,
    })
}

pub fn state(emulator: &GenesisEmulator) -> CpuState {
    let words = emulator.hybrid_cpu().hybrid_state_words();
    let mut cpu = CpuState::default();
    cpu.d.copy_from_slice(&words[..8]);
    cpu.a.copy_from_slice(&words[8..15]);
    cpu.usp = words[15];
    cpu.ssp = words[16];
    cpu.pc = words[17];
    cpu.sr = words[18];
    cpu.prefetch = words[19];
    cpu.pending_interrupt = words[20];
    cpu.stopped = words[21];
    cpu.frozen = words[22];
    cpu.address_error = words[23];
    cpu.last_muldiv = words[24];
    cpu
}

#[cfg(test)]
mod tests;
