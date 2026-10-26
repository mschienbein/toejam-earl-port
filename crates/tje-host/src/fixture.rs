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

mod original_measurement;
pub use original_measurement::OriginalFixtureMeasurement;

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

    /// Public differential entrypoints preflight before touching caller state.
    /// Snapshot guard metrics on Result completion, never at every game root.
    pub(crate) fn check_fixture_api<T>(
        &mut self,
        api: &'static str,
        execute: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        if self.mode != crate::Mode::Check {
            return Err(HostError::Metadata(format!("{api} requires Check mode")));
        }
        let result = execute(self);
        self.record_readable_stack_metrics();
        result
    }

    /// Inject an RTS fixture from a recorded live, settled boundary using that
    /// caller's real immutable-ROM PC, not a sentinel. This is diagnostic call
    /// injection, never evidence of a natural JSR. Both actual check forks run;
    /// no third original replay or invented scratch D0 result is returned.
    /// Requires Check mode; this does not finalize a campaign's proof obligations.
    pub fn check_clocked_fixture(
        &mut self,
        address: u32,
        arguments: &[u32],
    ) -> Result<tje_game::exits::EntryOutcome> {
        self.check_fixture_api("check_clocked_fixture", |host| {
            host.check_clocked_fixture_inner(address, arguments)
        })
    }

    fn check_clocked_fixture_inner(
        &mut self,
        address: u32,
        arguments: &[u32],
    ) -> Result<tje_game::exits::EntryOutcome> {
        self.observe_retained_native_prefix()?;
        self.require_fixture_boundary()?;
        let hook = self
            .registry
            .at(address)
            .cloned()
            .ok_or_else(|| HostError::Metadata(format!("no hook at {address:06X}")))?;
        if hook.check_route() != crate::CheckForkRoute::ClockedStrict
            || hook.exit_contract != tje_game::exits::ExitContract::Rts
        {
            return Err(HostError::Metadata(
                "clocked fixtures require exactly ClockedStrict and an RTS contract".into(),
            ));
        }
        let (caller_pc, saved_sp, entry_sp) = self.real_fixture_caller(arguments)?;
        self.isolated_fixture(|host| {
            let mut sp = saved_sp;
            for &argument in arguments.iter().rev() {
                sp -= 4;
                host.context().write_u32(sp, argument);
            }
            host.context().write_u32(entry_sp, caller_pc);
            let (cpu, bus) = host.emulator.hybrid_parts();
            crate::abi::set_sp(cpu, entry_sp);
            cpu.set_pc(address, bus);
            host.check_hook(&hook)
        })
    }

    /// Check a deliberately prepared reset, exception or ordinary entry without
    /// inventing a caller's return address. Set frame/register inputs explicitly
    /// before calling; unlike RTS fixtures, this never settles the prepared frame.
    /// Machine state is restored afterward, while coverage and evidence remain.
    /// Requires Check mode before looking up or entering the prepared frame.
    pub fn check_entry_fixture(&mut self, address: u32) -> Result<tje_game::exits::EntryOutcome> {
        self.check_fixture_api("check_entry_fixture", |host| {
            let hook = host
                .registry
                .at(address)
                .cloned()
                .ok_or_else(|| HostError::Metadata(format!("no hook at {address:06X}")))?;
            host.prepared_hook_fixture(hook, address)
        })
    }

    /// Check a reviewed persistent-loop continuation on an already prepared
    /// frame. Preserve its original entry `frame` across modal calls; the current
    /// lower SP is not a replacement for it. `body` is the real stage translation.
    /// Requires Check mode; prepared frame inspection happens after preflight.
    pub fn check_continuation_fixture(
        &mut self,
        id: tje_game::FunctionId,
        stage: tje_game::exits::MainLoopStage,
        body: tje_game::TranslationFn,
        frame: tje_game::call_frame::FrameState,
    ) -> Result<tje_game::exits::EntryOutcome> {
        self.check_fixture_api("check_continuation_fixture", |host| {
            host.check_continuation_fixture_inner(id, stage, body, frame)
        })
    }

    fn check_continuation_fixture_inner(
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
        if self.mode == crate::Mode::Replace
            && let Some(pool) = self
                .readable_stack
                .as_ref()
                .filter(|pool| pool.namespace.has_retained_dma())
        {
            // Physical snapshots intentionally cannot revive command authority.
            // Reject this capability before cloning or borrowing any live lease.
            let pending = pool.finish_proof();
            self.record_readable_stack_metrics();
            pending?;
        }
        let mut saved = self.emulator.partial_clone();
        let terminal_entry = self.terminal_entry;
        let live_authority = self
            .timing_services
            .as_mut()
            .map(|services| std::mem::take(&mut services.authority));
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
        let discarded = if let Some(authority) = live_authority {
            let services = self
                .timing_services
                .as_mut()
                .expect("fixture restores timing-service lease");
            let speculative = std::mem::replace(&mut services.authority, authority);
            services.discard_authority(speculative)
        } else {
            Ok(())
        };
        let watched = self.finish_isolated_provenance(provenance);
        let value = result?;
        watched?;
        discarded?;
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
            let return_pc = u32::from_be_bytes(std::array::from_fn(|offset| {
                crate::abi::peek_u8(&self.emulator, sp.wrapping_add(offset as u32))
            }));
            if return_pc == self.symbols.idle_random_return {
                self.frame_progress.idle_draws += 1;
            }
        }
    }

    pub(crate) fn tick_live_original(&mut self, inputs: &mut Inputs) -> Result<TickEffect> {
        self.tick_live_original_counted(inputs, false)
    }

    fn tick_live_original_counted(
        &mut self,
        inputs: &mut Inputs,
        force_counting: bool,
    ) -> Result<TickEffect> {
        self.with_live_tick_observation(inputs, force_counting, Self::tick)
    }

    fn with_live_tick_observation(
        &mut self,
        inputs: &mut Inputs,
        force_counting: bool,
        tick: impl FnOnce(&mut Self, &mut Inputs) -> Result<TickEffect>,
    ) -> Result<TickEffect> {
        self.ensure_replayed_lifecycle()?;
        let pc = self.cpu().pc();
        let previous_line = self.emulator.hybrid_bus().vdp.scanline();
        let observing_calls = force_counting
            || self
                .session_trace
                .as_ref()
                .is_some_and(|trace| trace.call_observer.is_some());
        let watch_before = self.begin_live_frame_observation()?;
        let previous_counting = self
            .cpu()
            .hybrid_instruction_counts()
            .is_some_and(|counts| counts.enabled());
        let set_counting = |emulator: &mut genesis_core::GenesisEmulator, enabled| -> Result<()> {
            #[cfg(feature = "reference-proof-hooks")]
            {
                emulator
                    .set_reference_instruction_counting(enabled)
                    .map_err(|error| {
                        HostError::Metadata(format!(
                            "passive live-tick counter configuration: {error}"
                        ))
                    })
            }
            #[cfg(not(feature = "reference-proof-hooks"))]
            {
                emulator
                    .hybrid_parts()
                    .0
                    .hybrid_set_instruction_counting(enabled);
                Ok(())
            }
        };
        if observing_calls {
            // Explicit persistent provider setup; never rewind an existing
            // tally. The temporary enabled flag is restored below on every path.
            #[cfg(feature = "reference-proof-hooks")]
            self.emulator.ensure_reference_instruction_counter();
            #[cfg(not(feature = "reference-proof-hooks"))]
            if self.cpu().hybrid_instruction_counts().is_none() {
                self.emulator
                    .hybrid_parts()
                    .0
                    .hybrid_enable_instruction_counts();
            }
            if let Err(error) = set_counting(&mut self.emulator, true) {
                self.abort_live_frame_observation(watch_before);
                return Err(error);
            }
        }
        let replayed = match self.begin_replayed_tick() {
            Ok(observing) => observing,
            Err(error) => {
                if observing_calls {
                    // The original begin failure wins; cleanup is still tried.
                    let _cleanup = set_counting(&mut self.emulator, previous_counting);
                }
                self.abort_live_frame_observation(watch_before);
                return Err(error);
            }
        };
        // Only active diagnostic loans require a finally boundary. This does
        // not decode private aborts: the owning backend authenticates those.
        let effect = if replayed || watch_before || observing_calls {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick(self, inputs)))
        } else {
            Ok(tick(self, inputs))
        };
        let completed = matches!(effect.as_ref(), Ok(Ok(_)));
        let replay_cleanup = if completed {
            self.finish_replayed_tick(
                replayed,
                matches!(effect.as_ref(), Ok(Ok(TickEffect::FrameRendered))),
                inputs.0,
            )
        } else {
            self.abort_replayed_tick(replayed);
            Ok(())
        };
        let counter_cleanup = if observing_calls {
            set_counting(&mut self.emulator, previous_counting)
        } else {
            Ok(())
        };
        let provenance = if completed {
            self.finish_live_frame_observation(watch_before)
        } else {
            self.abort_live_frame_observation(watch_before);
            Ok(())
        };
        let effect = match effect {
            Ok(effect) => effect?,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        // A tick error or unrelated panic above remains primary. All loans
        // have been restored before propagating infrastructure failure, which
        // remains primary over any later counter-cleanup failure.
        replay_cleanup?;
        counter_cleanup?;
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
                frame_counter: self
                    .emulator
                    .hybrid_bus()
                    .memory
                    .read_main_ram_word(catalogue::FRAME_COUNTER),
            };
            self.trace_lag();
            self.measurements.lag_events.push(event.clone());
            self.frame_progress.lag_events.push(event);
        }
        self.flush_completed_replays()?;
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

#[cfg(test)]
mod clocked_tests;

#[cfg(test)]
mod check_api_tests;
