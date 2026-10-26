//! Opt-in live-original call measurements, never check-fork accounting.
use crate::{Host, HostError, Mode, Result};
use m68000_emu::{HybridExecutedInstruction, HybridInstructionCounts};
use serde::Serialize;
pub(crate) mod invocation;
use invocation::{InvocationId, InvocationPoint, InvocationTracker};

#[derive(Clone, Copy, Debug, Serialize)]
pub struct CallCounters {
    /// Actual executed opcode attempts. Exceptional outcomes are explicit below;
    /// zero exceptional attempts permits describing these as successful instructions.
    pub main_thread_instructions: u64,
    /// Includes nested exception handlers. RTE belongs to its pre-execution depth.
    pub exception_handler_instructions: u64,
    pub irq_entries_by_level: [u64; 8],
    pub other_exception_entries: u64,
    pub faulted_opcode_attempts: u64,
    /// Original ticks, including noninstruction hardware and interrupt progress.
    pub ticks: u64,
    pub master_clocks: u64,
    pub completed_video_frames: u64,
}
impl CallCounters {
    pub(crate) fn capture(
        counts: &HybridInstructionCounts,
        ticks: u64,
        clock: u64,
        frame: u64,
    ) -> Self {
        Self {
            main_thread_instructions: counts.main_thread,
            exception_handler_instructions: counts.exception_handler,
            irq_entries_by_level: counts.irq_entries,
            other_exception_entries: counts.other_exceptions,
            faulted_opcode_attempts: counts.faulted_opcode_attempts,
            ticks,
            master_clocks: clock,
            completed_video_frames: frame,
        }
    }
    pub(crate) fn since(self, start: Self) -> Self {
        Self {
            main_thread_instructions: self.main_thread_instructions
                - start.main_thread_instructions,
            exception_handler_instructions: self.exception_handler_instructions
                - start.exception_handler_instructions,
            irq_entries_by_level: std::array::from_fn(|i| {
                self.irq_entries_by_level[i] - start.irq_entries_by_level[i]
            }),
            other_exception_entries: self.other_exception_entries - start.other_exception_entries,
            faulted_opcode_attempts: self.faulted_opcode_attempts - start.faulted_opcode_attempts,
            ticks: self.ticks - start.ticks,
            master_clocks: self.master_clocks - start.master_clocks,
            completed_video_frames: self.completed_video_frames - start.completed_video_frames,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CallObservation {
    pub caller_pc: u32,
    pub entry_pc: u32,
    pub entry_sp: u32,
    pub entry_exception_depth: u32,
    pub return_pc: u32,
    pub return_sp: u32,
    pub rts_pc: u32,
    /// After the actual JSR/BSR; the caller's call opcode is excluded.
    pub entry: CallCounters,
    /// After the matching actual RTS tick, before later IRQ recognition/entry.
    pub returned: CallCounters,
    pub usage: CallCounters,
}
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct CallEntryCoverage {
    pub entry_pc: u32,
    pub executed_entry_opcodes: u64,
    pub faulted_entry_opcodes: u64,
    pub recognized_calls: u64,
    pub completed_calls: u64,
    pub unfinished_calls: usize,
}
/// Raw post-instruction control point; not entry or proof authority by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CallReturnPoint {
    pub pc: u32,
    pub sp: u32,
    pub depth: u32,
}
pub(crate) fn successful_call_instruction(instruction: HybridExecutedInstruction) -> bool {
    instruction.completed_successfully
        && (instruction.opcode & 0xffc0 == 0x4e80 || instruction.opcode & 0xff00 == 0x6100)
}
pub(crate) fn matches_successful_rts(
    instruction: HybridExecutedInstruction,
    actual: CallReturnPoint,
    expected: CallReturnPoint,
) -> bool {
    instruction.opcode == 0x4e75 && instruction.completed_successfully && actual == expected
}

struct CallBoundary {
    point: InvocationPoint,
    clock: u64,
    frame: u64,
}
struct ActiveCall {
    invocation: InvocationId,
    entry: CallCounters,
}
pub(crate) struct CallObserver {
    coverage: Vec<CallEntryCoverage>,
    invocations: InvocationTracker,
    ticks: u64,
    previous_instructions: u64,
    active: Vec<ActiveCall>,
    completed: Vec<CallObservation>,
}
impl CallObserver {
    fn new(revision: tje_rom::Revision, entries: &[u32]) -> Self {
        Self {
            coverage: entries
                .iter()
                .map(|&entry_pc| CallEntryCoverage {
                    entry_pc,
                    ..Default::default()
                })
                .collect(),
            invocations: InvocationTracker::observing(revision),
            ticks: 0,
            previous_instructions: 0,
            active: Vec::new(),
            completed: Vec::new(),
        }
    }
    fn after_tick(
        &mut self,
        counts: &HybridInstructionCounts,
        boundary: CallBoundary,
        return_word: Option<u32>,
    ) -> Result<()> {
        let CallBoundary {
            point,
            clock,
            frame,
        } = boundary;
        self.ticks += 1;
        let instructions = counts.main_thread + counts.exception_handler;
        if instructions < self.previous_instructions {
            return Err(HostError::Metadata(
                "original instruction observer counters reset during playback".into(),
            ));
        }
        let executed = instructions != self.previous_instructions;
        self.previous_instructions = instructions;
        if !executed {
            return Ok(());
        }
        let instruction = counts
            .last_instruction
            .expect("actual instruction has its opcode");
        if let Some(entry) = self
            .coverage
            .iter_mut()
            .find(|entry| entry.entry_pc == instruction.pc)
        {
            entry.executed_entry_opcodes += 1;
            entry.faulted_entry_opcodes += u64::from(!instruction.completed_successfully);
        }
        let update = self
            .invocations
            .observe_unrooted_boundary(instruction, point, return_word);
        if update.invalidated == Some(invocation::Invalidation::Exhausted) {
            return Err(HostError::Metadata(
                "original invocation observer identity exhausted".into(),
            ));
        }
        if let Some(invocation) = update.entered
            && return_word.is_some()
            && let Some(entry) = self
                .coverage
                .iter_mut()
                .find(|entry| entry.entry_pc == point.pc)
        {
            entry.recognized_calls += 1;
            self.active.push(ActiveCall {
                invocation,
                entry: CallCounters::capture(counts, self.ticks, clock, frame),
            });
        }
        let Some(returned_id) = update.returned else {
            return Ok(());
        };
        if let Some(active) = self
            .active
            .pop_if(|active| active.invocation == returned_id)
        {
            let invocation = self
                .invocations
                .record(active.invocation)
                .expect("recorded actual invocation");
            self.coverage
                .iter_mut()
                .find(|entry| entry.entry_pc == invocation.entry_pc())
                .expect("selected return entry")
                .completed_calls += 1;
            let returned = CallCounters::capture(counts, self.ticks, clock, frame);
            self.completed.push(CallObservation {
                caller_pc: invocation.caller_pc().expect("actual call instruction"),
                entry_pc: invocation.entry_pc(),
                entry_sp: invocation.entry_sp().expect("actual entry stack"),
                entry_exception_depth: invocation.exception_depth(),
                return_pc: point.pc,
                return_sp: point.sp,
                rts_pc: instruction.pc,
                entry: active.entry,
                returned,
                usage: returned.since(active.entry),
            });
        }
        if !self.invocations.retire_measurement_return(returned_id) {
            return Err(HostError::Metadata(
                "original measurement record did not retire after its return".into(),
            ));
        }
        Ok(())
    }
}
impl Host {
    /// Trace actual JSR/BSR to tracked entries and their matching RTS PC+SP+depth.
    /// Reference-only, before playback: never a check hook, wait or larger bound.
    pub fn enable_call_observations(&mut self, entries: &[u32]) -> Result<()> {
        if self.mode != Mode::Reference || self.frame_count() != 0 {
            return Err(HostError::Metadata(
                "call observations require reference mode before playback".into(),
            ));
        }
        let trace = self.session_trace.as_mut().ok_or_else(|| {
            HostError::Metadata("enable session tracing before call observations".into())
        })?;
        if trace.call_observer.is_some()
            || entries.is_empty()
            || entries.iter().any(|&pc| {
                pc & 1 != 0 || trace.slots.get(pc as usize / 2).copied().unwrap_or(0) == 0
            })
        {
            return Err(HostError::Metadata(
                "call entries must be tracked, nonempty, and enabled once".into(),
            ));
        }
        #[cfg(feature = "reference-proof-hooks")]
        {
            self.emulator.ensure_reference_instruction_counter();
            self.emulator
                .set_reference_instruction_counting(false)
                .map_err(|error| {
                    HostError::Metadata(format!("passive call-counter setup: {error}"))
                })?;
        }
        #[cfg(not(feature = "reference-proof-hooks"))]
        {
            let (cpu, _) = self.emulator.hybrid_parts();
            if cpu.hybrid_instruction_counts().is_none() {
                cpu.hybrid_enable_instruction_counts();
            }
            cpu.hybrid_set_instruction_counting(false);
        }
        let baseline = self
            .emulator
            .hybrid_cpu()
            .hybrid_instruction_counts()
            .and_then(|counts| counts.main_thread.checked_add(counts.exception_handler))
            .ok_or_else(|| {
                HostError::Metadata("call-counter baseline is absent or overflowed".into())
            })?;
        let mut observer = CallObserver::new(self.revision, entries);
        observer.previous_instructions = baseline;
        trace.call_observer = Some(observer);
        Ok(())
    }
    pub fn take_call_observations(&mut self) -> Vec<CallObservation> {
        self.session_trace
            .as_mut()
            .and_then(|trace| trace.call_observer.as_mut())
            .map_or_else(Vec::new, |observer| std::mem::take(&mut observer.completed))
    }
    pub fn unfinished_observed_calls(&self) -> usize {
        self.session_trace
            .as_ref()
            .and_then(|trace| trace.call_observer.as_ref())
            .map_or(0, |observer| observer.active.len())
    }
    /// Reconcile actual entry opcodes with JSR/BSR starts and completed RTS calls.
    /// A tail JMP or corpus-ending open call must not be called complete coverage.
    pub fn observed_call_entry_counts(&self) -> Vec<CallEntryCoverage> {
        self.session_trace
            .as_ref()
            .and_then(|trace| trace.call_observer.as_ref())
            .map_or_else(Vec::new, |observer| {
                observer
                    .coverage
                    .iter()
                    .map(|entry| {
                        let mut entry = *entry;
                        entry.unfinished_calls = observer
                            .active
                            .iter()
                            .filter(|call| {
                                observer
                                    .invocations
                                    .record(call.invocation)
                                    .is_some_and(|record| record.entry_pc() == entry.entry_pc)
                            })
                            .count();
                        entry
                    })
                    .collect()
            })
    }
    pub(crate) fn observe_call_return(&mut self, rendered: bool) -> Result<()> {
        let Some(observer) = self
            .session_trace
            .as_mut()
            .and_then(|trace| trace.call_observer.as_mut())
        else {
            return Ok(());
        };
        let cpu = self.emulator.hybrid_cpu();
        let bus = self.emulator.hybrid_bus();
        let counts = cpu
            .hybrid_instruction_counts()
            .expect("enabled with call observer");
        let executed =
            counts.main_thread + counts.exception_handler != observer.previous_instructions;
        let is_call = executed
            && counts
                .last_instruction
                .is_some_and(successful_call_instruction);
        let pc = cpu.pc();
        let sp = cpu.stack_pointer();
        let return_word = if is_call {
            if sp & 1 != 0 || (sp & 0xffffff) < 0xe00000 {
                if observer.coverage.iter().any(|entry| entry.entry_pc == pc) {
                    return Err(HostError::Metadata(
                        "observed call has no aligned work-RAM return slot".into(),
                    ));
                }
                None
            } else {
                let ram = bus.memory.hybrid_raw_main_ram();
                let high = ram[((sp & 0xffff) / 2) as usize];
                let low = ram[((sp.wrapping_add(2) & 0xffff) / 2) as usize];
                Some((u32::from(high) << 16) | u32::from(low))
            }
        } else {
            None
        };
        observer.after_tick(
            counts,
            CallBoundary {
                point: InvocationPoint::capture(cpu),
                clock: bus.cycles.m68k_mclk_cycles,
                frame: self.frame + u64::from(rendered),
            },
            return_word,
        )
    }
}

#[cfg(test)]
mod tests;
