//! Original-execution settling before a labelled live-state fixture is seeded.
//!
//! A rendered frame is not necessarily a call-ready CPU boundary: the reference
//! can stop there with an IRQ pending, a bus stall, or an active exception frame.
//! Settling executes that original work, never patches its caller into readiness.

use crate::{
    FrameSnapshot, HardwareWrite, Host, HostError, LagEvent, Result, catalogue, sinks::Inputs,
};
use genesis_config::GenesisInputs;
use jgenesis_common::frontend::{PartialClone, TickEffect};
use m68000_emu::BusInterface;

const MAX_SETTLE_TICKS: u64 = 1_000_000;

/// Original progress made before fixture inputs may be written.
///
/// `ticks` counts reference ticks, including IRQ recognition and bus/refresh
/// waits, not just executed instructions. `master_clocks` is the actual elapsed
/// hardware clock count. A zero-length advance means the entry was already safe.
#[derive(Debug)]
pub struct BoundaryAdvance {
    pub start_pc: u32,
    pub end_pc: u32,
    pub start_frame: u64,
    pub end_frame: u64,
    pub ticks: u64,
    pub master_clocks: u64,
    /// Successfully returned exception frames, including traps nested in IRQs.
    pub handlers_completed: u64,
    /// Every video frame completed while settling, with the same snapshots and
    /// output retention as `Host::run_frame`. These are not fixture coverage.
    pub completed_frames: Vec<FrameSnapshot>,
}

/// A settle operation may leave the next video frame partially executed. Keep
/// its observations until run_frame or another settle completes that same frame.
#[derive(Default)]
pub(crate) struct FrameProgress {
    pub idle_draws: u64,
    pub lag_events: Vec<LagEvent>,
}

impl Host {
    /// Whether a diagnostic call can be seeded without overtaking original work.
    /// Masked but asserted IRQs are not settled either: their original code must
    /// eventually unmask/acknowledge them, rather than a fixture clearing them.
    pub fn fixture_boundary_is_settled(&self) -> bool {
        let bus = self.emulator.hybrid_bus();
        let cpu = self.emulator.hybrid_cpu();
        bus.cycles.m68k_wait_cpu_cycles == 0
            && bus.interrupt_level() & 7 == 0
            && cpu.hybrid_exception_depth() == 0
            && cpu.hybrid_instruction_ready(bus)
    }

    /// Advance only the original machine to the first settled call boundary.
    ///
    /// Call this **before** writing fixture RAM, registers, arguments or other
    /// inputs, then use the existing `check_fixture` API. No entry hook executes
    /// during settling, even in check/replacement mode. The original PC/prefetch,
    /// SP, SR, pending IRQs and device state are never rewritten to make progress.
    ///
    /// `inputs` samples the current video frame, just as for `run_frame`. Apply
    /// any current-frame session recipe before this call. Crossing further video
    /// frames requires a `WaitInputPoller`, which samples/applies each new frame
    /// once; missing samples are errors. Partial-frame audio/writes/observations
    /// remain available to the next `run_frame`; completed frames are returned.
    /// The fixed settling bound is separate from, and never raises, a function's
    /// call-instruction bound. Failure leaves real progress in the live machine.
    pub fn settle_fixture_boundary(&mut self, inputs: GenesisInputs) -> Result<BoundaryAdvance> {
        self.settle_fixture_boundary_inner(inputs, MAX_SETTLE_TICKS)
    }

    fn settle_fixture_boundary_inner(
        &mut self,
        inputs: GenesisInputs,
        max_ticks: u64,
    ) -> Result<BoundaryAdvance> {
        self.wait_clock.frame = self.frame;
        self.wait_clock.inputs = inputs;
        let start_clock = self.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
        let mut advance = BoundaryAdvance {
            start_pc: self.cpu().pc(),
            end_pc: self.cpu().pc(),
            start_frame: self.frame,
            end_frame: self.frame,
            ticks: 0,
            master_clocks: 0,
            handlers_completed: 0,
            completed_frames: Vec::new(),
        };
        let mut inputs = Inputs(inputs);
        loop {
            advance.end_pc = self.cpu().pc();
            advance.end_frame = self.frame;
            advance.master_clocks =
                self.emulator.hybrid_bus().cycles.m68k_mclk_cycles - start_clock;
            if self.fixture_boundary_is_settled() {
                return Ok(advance);
            }
            if advance.ticks == max_ticks {
                return Err(HostError::Emulator(format!(
                    "fixture boundary did not settle after {} original ticks ({} master clocks, {} video frames, PC={:06X}, exception depth={})",
                    advance.ticks,
                    advance.master_clocks,
                    advance.end_frame - advance.start_frame,
                    advance.end_pc,
                    self.cpu().hybrid_exception_depth(),
                )));
            }
            let bus = self.emulator.hybrid_bus();
            if bus.cycles.m68k_wait_cpu_cycles == 0 && self.cpu().hybrid_instruction_ready(bus) {
                self.observe_live_instruction(self.cpu().pc());
            }
            let depth = self.cpu().hybrid_exception_depth();
            let effect = self.tick_live_original(&mut inputs)?;
            advance.ticks += 1;
            advance.handlers_completed +=
                u64::from(depth.saturating_sub(self.cpu().hybrid_exception_depth()));
            if effect == TickEffect::FrameRendered {
                advance.completed_frames.push(self.finish_live_frame()?);
                self.wait_clock.advance_frame(&mut self.emulator)?;
                inputs.0 = self.wait_clock.inputs;
            }
        }
    }

    pub(crate) fn require_fixture_boundary(&self) -> Result<()> {
        if !self.fixture_boundary_is_settled() {
            return Err(HostError::Metadata(format!(
                "fixture call requires a settled live boundary (PC={:06X}, IRQ={}, exception depth={}, wait cycles={}); call settle_fixture_boundary(inputs) before seeding fixture inputs",
                self.cpu().pc(),
                self.emulator.hybrid_bus().interrupt_level() & 7,
                self.cpu().hybrid_exception_depth(),
                self.emulator.hybrid_bus().cycles.m68k_wait_cpu_cycles,
            )));
        }
        Ok(())
    }

    /// Check a deliberately prepared reset, exception or ordinary entry without
    /// inventing a caller's return address. Set frame/register inputs explicitly
    /// before calling; unlike RTS fixtures, this never settles the prepared frame.
    /// Machine state is restored afterward, while coverage and evidence remain.
    pub fn check_entry_fixture(&mut self, address: u32) -> Result<tje_game::exits::EntryOutcome> {
        let hook = self
            .registry
            .at(address)
            .cloned()
            .ok_or_else(|| HostError::Metadata(format!("no hook at {address:06X}")))?;
        self.prepared_hook_fixture(hook, address)
    }

    /// Check a reviewed persistent-loop continuation on an already prepared
    /// frame. Preserve its original entry `frame` across modal calls; the current
    /// lower SP is not a replacement for it. `body` is the real stage translation.
    pub fn check_continuation_fixture(
        &mut self,
        id: tje_game::FunctionId,
        stage: tje_game::exits::MainLoopStage,
        body: tje_game::TranslationFn,
        frame: tje_game::call_frame::FrameState,
    ) -> Result<tje_game::exits::EntryOutcome> {
        let root = {
            let mut roots = self.registry.iter().filter(|hook| hook.symbol.id == id);
            let root = roots.next().cloned().ok_or_else(|| {
                HostError::Metadata(format!("no continuation hook for {}", id.metadata().name))
            })?;
            if roots.next().is_some() {
                return Err(HostError::Metadata(format!(
                    "ambiguous continuation hooks for {}",
                    id.metadata().name,
                )));
            }
            root
        };
        let (hook, pc) = root.continuation(
            stage,
            body,
            self.revision,
            frame,
            self.emulator.hybrid_cpu().stack_pointer(),
        )?;
        self.prepared_hook_fixture(hook, pc)
    }

    fn prepared_hook_fixture(
        &mut self,
        hook: crate::hooks::Hook,
        pc: u32,
    ) -> Result<tje_game::exits::EntryOutcome> {
        self.isolated_fixture(|host| {
            let (cpu, bus) = host.emulator.hybrid_parts();
            cpu.set_pc(pc, bus);
            host.check_hook(&hook)
        })
    }

    pub(crate) fn isolated_fixture<T>(
        &mut self,
        execute: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let mut saved = self.emulator.partial_clone();
        let terminal_entry = self.terminal_entry;
        let provenance = self.begin_isolated_provenance();
        let result = execute(self);
        if let Some(cartridge) = saved.hybrid_bus_mut().cartridge.as_mut() {
            cartridge.take_rom_from(
                self.emulator
                    .hybrid_bus_mut()
                    .cartridge
                    .as_mut()
                    .expect("cartridge"),
            );
        }
        self.emulator = saved;
        self.terminal_entry = terminal_entry;
        let watched = self.finish_isolated_provenance(provenance);
        let value = result?;
        watched?;
        Ok(value)
    }

    pub(crate) fn observe_live_instruction(&mut self, pc: u32) {
        self.trace_instruction(pc);
        if pc == self.symbols.intro {
            self.measurements.intro_frame.get_or_insert(self.frame);
        } else if pc == self.symbols.title {
            self.measurements.title_frame.get_or_insert(self.frame);
        } else if pc == self.symbols.new_game_menu {
            self.measurements.menu_frame.get_or_insert(self.frame);
        } else if pc == self.symbols.main_loop {
            self.measurements.game_frame.get_or_insert(self.frame);
            self.measurements.logic_ticks += 1;
            self.main_idle = false;
        } else if pc == self.symbols.main_wait {
            self.main_idle = true;
        } else if pc == self.symbols.random {
            let sp = self.cpu().stack_pointer();
            if self.context().read_u32(sp) == self.symbols.idle_random_return {
                self.frame_progress.idle_draws += 1;
            }
        }
    }

    pub(crate) fn tick_live_original(&mut self, inputs: &mut Inputs) -> Result<TickEffect> {
        let pc = self.cpu().pc();
        let previous_line = self.emulator.hybrid_bus().vdp.scanline();
        let observing_calls = self
            .session_trace
            .as_ref()
            .is_some_and(|trace| trace.call_observer.is_some());
        let watch_before = self.begin_live_frame_observation()?;
        if observing_calls {
            self.emulator
                .hybrid_parts()
                .0
                .hybrid_set_instruction_counting(true);
        }
        let effect = self.tick(inputs);
        if observing_calls {
            self.emulator
                .hybrid_parts()
                .0
                .hybrid_set_instruction_counting(false);
        }
        let provenance = self.finish_live_frame_observation(watch_before);
        let effect = effect?;
        self.observe_call_return(effect == TickEffect::FrameRendered)?;
        provenance?;
        let vdp = &self.emulator.hybrid_bus().vdp;
        let blank_line = vdp
            .hybrid_registers()
            .vertical_display_size
            .active_scanlines();
        if previous_line != vdp.scanline()
            && vdp.scanline() == blank_line
            && self.measurements.game_frame.is_some()
            && !self.main_idle
        {
            let event = LagEvent {
                frame: self.frame,
                pc_at_vblank: pc,
                frame_counter: self.context().read_u16(catalogue::FRAME_COUNTER),
            };
            self.trace_lag();
            self.measurements.lag_events.push(event.clone());
            self.frame_progress.lag_events.push(event);
        }
        Ok(effect)
    }

    pub(crate) fn finish_live_frame(&mut self) -> Result<FrameSnapshot> {
        let progress = std::mem::take(&mut self.frame_progress);
        let snapshot = self.snapshot(progress.idle_draws, progress.lag_events)?;
        self.frame += 1;
        self.measurements
            .idle_draws_per_frame
            .push(progress.idle_draws);
        for write in &snapshot.writes {
            match write {
                HardwareWrite::Vdp { .. } => self.measurements.vdp_writes += 1,
                HardwareWrite::Ym2612 { .. } => self.measurements.ym2612_writes += 1,
                HardwareWrite::Psg { .. } => self.measurements.psg_writes += 1,
            }
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
pub(crate) mod tests;
