//! Opt-in reference-only prepared-entry resource measurements, never approvals.
use crate::{Host, HostError, Mode, Result, call_observation::CallCounters, sinks::Inputs};
use jgenesis_common::frontend::TickEffect;
use serde::Serialize;
use tje_game::{
    Context, FunctionId,
    execution::{CallCeiling, CallClock, ClockUsage},
    ids,
};

/// A completed actual original RTS interval in an authored prepared frame.
/// This is NOT a naturally observed JSR/BSR, equivalence proof or completion receipt.
#[derive(Debug, Serialize)]
pub struct OriginalFixtureMeasurement {
    pub purpose: &'static str,
    pub entry_origin: &'static str,
    pub approves_translation: bool,
    pub function: &'static str,
    pub revision: &'static str,
    pub caller_pc: u32,
    pub entry_pc: u32,
    pub entry_sp: u32,
    pub entry_exception_depth: u32,
    pub arguments: Vec<u32>,
    pub entry_sr: u16,
    pub return_pc: u32,
    pub return_sp: u32,
    pub rts_pc: u32,
    pub ceiling_ticks: u64,
    pub ceiling_frames: u64,
    pub ceiling_reference_opcode_attempts: u64,
    pub entry: CallCounters,
    pub at_rts: CallCounters,
    pub usage_to_rts: CallCounters,
    pub successful_rte_returns: u64,
}

impl Host {
    // Shared verbatim caller/stack validation with check_clocked_fixture. This
    // inspects immutable ROM; it does not pre-read/substitute the callee opcode.
    pub(super) fn real_fixture_caller(&mut self, arguments: &[u32]) -> Result<(u32, u32, u32)> {
        let caller_pc = self.cpu().pc();
        let caller_prefetch = self.cpu().next_opcode();
        if caller_pc & 1 != 0 || caller_pc == crate::abi::CALL_SENTINEL {
            return Err(HostError::Metadata(
                "clocked fixture caller is not a real even immutable-ROM return target".into(),
            ));
        }
        let expected = self
            .emulator
            .hybrid_bus_mut()
            .cartridge
            .as_mut()
            .and_then(|cartridge| {
                cartridge
                    .debug_rom_view()
                    .get(caller_pc as usize / 2)
                    .copied()
            });
        if expected != Some(caller_prefetch) {
            return Err(HostError::Metadata(
                "clocked fixture caller return is outside immutable ROM or has invalid prefetch"
                    .into(),
            ));
        }
        let saved_sp = self.cpu().stack_pointer();
        let frame_bytes = u32::try_from(arguments.len())
            .ok()
            .and_then(|slots| slots.checked_add(1))
            .and_then(|slots| slots.checked_mul(4))
            .ok_or_else(|| {
                HostError::Metadata("clocked fixture frame exceeds its representation".into())
            })?;
        let entry_sp = saved_sp
            .checked_sub(frame_bytes)
            .ok_or_else(|| HostError::Metadata("clocked fixture frame underflows".into()))?;
        if saved_sp & 1 != 0
            || (entry_sp & 0xffffff) < 0xe00000
            || (saved_sp & 0xffffff) < 0xe00000
            || (entry_sp & 0xffff) > (saved_sp & 0xffff)
        {
            return Err(HostError::Metadata(
                "clocked fixture frame is outside its actual stack bank".into(),
            ));
        }
        Ok((caller_pc, saved_sp, entry_sp))
    }

    /// Measure only the original MoveDataToVRAM from the first callee opcode
    /// through its actual matching successful RTS, before subsequent IRQ admission.
    /// The supplied finite per-invocation resources are diagnostic, not game
    /// policy or a termination bound. No Rust hook executes or gains approval.
    pub fn measure_original_clocked_fixture(
        &mut self,
        function: FunctionId,
        arguments: &[u32],
        ceiling: CallCeiling,
    ) -> Result<OriginalFixtureMeasurement> {
        ceiling.validate().map_err(HostError::Clock)?;
        if self.mode != Mode::Reference
            || function != ids::MOVE_DATA_TO_VRAM
            || arguments.len() != 2
            || ceiling.reference_instructions.is_none()
            || self.wait_clock.call_clock.active()
        {
            return Err(HostError::Metadata(
                "prepared-entry measurement requires reference-only MoveDataToVRAM, two actual slots, explicit finite I/T/F and no enclosing clock scope".into(),
            ));
        }
        self.observe_retained_native_prefix()?;
        self.require_fixture_boundary()?;
        let (caller_pc, saved_sp, entry_sp) = self.real_fixture_caller(arguments)?;
        let address = function.metadata().address(self.revision);
        let entry_sr = self.context().status_register();
        let entry_depth = self.cpu().hybrid_exception_depth();
        // Preserve the entire live trajectory, not just its CPU/device snapshot.
        // No live completion authority is cloned or used by this reference run.
        let saved_clock = std::mem::take(&mut self.wait_clock);
        self.wait_clock = saved_clock.clone();
        let saved_frame = self.frame;
        let saved_idle = self.main_idle;
        let saved_progress = std::mem::take(&mut self.frame_progress);
        let saved_audio = std::mem::take(&mut self.audio.samples);
        let saved_measurements = std::mem::take(&mut self.measurements);
        let saved_trace = self.session_trace.take();
        let result = self.isolated_fixture(|host| {
            let mut sp = saved_sp;
            for &argument in arguments.iter().rev() {
                sp -= 4;
                host.context().write_u32(sp, argument);
            }
            host.context().write_u32(entry_sp, caller_pc);
            let (cpu, bus) = host.emulator.hybrid_parts();
            crate::abi::set_sp(cpu, entry_sp);
            cpu.set_pc(address, bus);
            if cpu.hybrid_instruction_counts().is_none() {
                cpu.hybrid_enable_instruction_counts();
            }
            let mut clock = CallClock::default();
            let scope = clock.enter(function, ceiling).map_err(HostError::Clock)?;
            let entry = host.original_measurement_counters(0)?;
            let mut inputs = Inputs(host.wait_clock.inputs);
            let mut successful_rte_returns = 0;
            loop {
                clock.before_tick().map_err(HostError::Clock)?;
                let before = host.original_measurement_counters(clock.usage().ticks)?;
                let effect = host.tick_live_original_counted(&mut inputs, true)?;
                let counts = host.cpu().hybrid_instruction_counts().ok_or_else(|| {
                    HostError::Metadata("original measurement observer was reset".into())
                })?;
                let total_before =
                    before.main_thread_instructions + before.exception_handler_instructions;
                let total_after = counts.main_thread + counts.exception_handler;
                let retired = total_after.checked_sub(total_before).ok_or_else(|| {
                    HostError::Metadata("measurement instruction counters regressed".into())
                })?;
                let instruction = counts.last_instruction;
                if counts.faulted_opcode_attempts != entry.faulted_opcode_attempts
                    || counts.other_exceptions != entry.other_exception_entries
                {
                    return Err(HostError::Metadata(
                        "measurement encountered a fault/non-IRQ exception; not completed".into(),
                    ));
                }
                if retired > 1 {
                    return Err(HostError::Metadata(
                        "one original tick retired multiple CPU opcodes".into(),
                    ));
                }
                if retired == 1
                    && instruction.is_some_and(|instruction| {
                        instruction.opcode == 0x4e73
                            && instruction.completed_successfully
                            && instruction.exception_depth
                                == host.cpu().hybrid_exception_depth() + 1
                    })
                {
                    successful_rte_returns += 1;
                }
                let returned = retired == 1
                    && instruction.is_some_and(|instruction| {
                        instruction.opcode == 0x4e75
                            && instruction.completed_successfully
                            && host.cpu().pc() == caller_pc
                            && host.cpu().stack_pointer() == entry_sp.wrapping_add(4)
                            && host.cpu().hybrid_exception_depth() == entry_depth
                    });
                let actual_clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
                let completed_frame = effect == TickEffect::FrameRendered;
                clock
                    .record(ClockUsage {
                        ticks: 1,
                        instructions: retired,
                        master_clocks: actual_clock.checked_sub(before.master_clocks).ok_or_else(
                            || HostError::Metadata("measurement master clock regressed".into()),
                        )?,
                        frames: u64::from(completed_frame),
                        dispatch_steps: 0,
                    })
                    .map_err(HostError::Clock)?;
                clock.check().map_err(HostError::Clock)?;
                // AT-RTS observation precedes later recognition, IRQs or bus drain.
                let at_rts = if returned {
                    Some((
                        CallCounters::capture(
                            counts,
                            clock.usage().ticks,
                            actual_clock,
                            host.frame + u64::from(completed_frame),
                        ),
                        host.cpu().pc(),
                        host.cpu().stack_pointer(),
                        instruction.expect("observed actual RTS").pc,
                    ))
                } else {
                    None
                };
                if let Some((at_rts, return_pc, return_sp, rts_pc)) = at_rts {
                    clock.leave(scope).map_err(HostError::Clock)?;
                    return Ok(OriginalFixtureMeasurement {
                        purpose: "measurement",
                        entry_origin: "authored prepared entry; caller JSR not observed or counted",
                        approves_translation: false,
                        function: function.metadata().name,
                        revision: host.revision.name(),
                        caller_pc,
                        entry_pc: address,
                        entry_sp,
                        entry_exception_depth: entry_depth,
                        arguments: arguments.to_vec(),
                        entry_sr,
                        return_pc,
                        return_sp,
                        rts_pc,
                        ceiling_ticks: ceiling.ticks,
                        ceiling_frames: ceiling.frames,
                        ceiling_reference_opcode_attempts: ceiling
                            .reference_instructions
                            .expect("validated finite I"),
                        usage_to_rts: at_rts.since(entry),
                        entry,
                        at_rts,
                        successful_rte_returns,
                    });
                }
                if completed_frame {
                    host.finish_live_frame()?;
                    host.wait_clock.advance_frame(&mut host.emulator)?;
                    inputs.0 = host.wait_clock.inputs;
                }
            }
        });
        self.wait_clock = saved_clock;
        self.frame = saved_frame;
        self.main_idle = saved_idle;
        self.frame_progress = saved_progress;
        self.audio.samples = saved_audio;
        self.measurements = saved_measurements;
        self.session_trace = saved_trace;
        result
    }

    fn original_measurement_counters(&self, ticks: u64) -> Result<CallCounters> {
        self.cpu()
            .hybrid_instruction_counts()
            .map(|counts| {
                CallCounters::capture(
                    counts,
                    ticks,
                    self.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
                    self.frame,
                )
            })
            .ok_or_else(|| HostError::Metadata("measurement CPU observer is not enabled".into()))
    }
}

#[cfg(test)]
mod tests;
