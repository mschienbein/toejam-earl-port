//! Opt-in live-original call measurements, never check-fork accounting.
use crate::{Host, HostError, Mode, Result};
use m68000_emu::HybridInstructionCounts;
use serde::Serialize;

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
struct CallBoundary {
    pc: u32,
    sp: u32,
    depth: u32,
    clock: u64,
    frame: u64,
}
struct ActiveCall {
    caller_pc: u32,
    entry_pc: u32,
    entry_sp: u32,
    depth: u32,
    return_pc: u32,
    entry: CallCounters,
}
#[derive(Default)]
pub(crate) struct CallObserver {
    coverage: Vec<CallEntryCoverage>,
    ticks: u64,
    previous_instructions: u64,
    active: Vec<ActiveCall>,
    completed: Vec<CallObservation>,
}
impl CallObserver {
    fn after_tick(
        &mut self,
        counts: &HybridInstructionCounts,
        boundary: CallBoundary,
        return_word: Option<u32>,
    ) -> Result<()> {
        let CallBoundary {
            pc,
            sp,
            depth,
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
        if let Some(return_pc) = return_word.filter(|_| instruction.completed_successfully) {
            self.coverage
                .iter_mut()
                .find(|entry| entry.entry_pc == pc)
                .expect("selected call entry")
                .recognized_calls += 1;
            self.active.push(ActiveCall {
                caller_pc: instruction.pc,
                entry_pc: pc,
                entry_sp: sp,
                depth,
                return_pc,
                entry: CallCounters::capture(counts, self.ticks, clock, frame),
            });
        }
        if instruction.opcode != 0x4e75 || !instruction.completed_successfully {
            return Ok(());
        }
        let Some(active) = self.active.last() else {
            return Ok(());
        };
        if pc != active.return_pc || sp != active.entry_sp.wrapping_add(4) || depth != active.depth
        {
            return Ok(());
        }
        let active = self.active.pop().expect("matched actual RTS");
        self.coverage
            .iter_mut()
            .find(|entry| entry.entry_pc == active.entry_pc)
            .expect("selected return entry")
            .completed_calls += 1;
        let returned = CallCounters::capture(counts, self.ticks, clock, frame);
        self.completed.push(CallObservation {
            caller_pc: active.caller_pc,
            entry_pc: active.entry_pc,
            entry_sp: active.entry_sp,
            entry_exception_depth: active.depth,
            return_pc: pc,
            return_sp: sp,
            rts_pc: instruction.pc,
            entry: active.entry,
            returned,
            usage: returned.since(active.entry),
        });
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
        trace.call_observer = Some(CallObserver {
            coverage: entries
                .iter()
                .map(|&entry_pc| CallEntryCoverage {
                    entry_pc,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
        self.emulator
            .hybrid_parts()
            .0
            .hybrid_enable_instruction_counts();
        self.emulator
            .hybrid_parts()
            .0
            .hybrid_set_instruction_counting(false);
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
                            .filter(|call| call.entry_pc == entry.entry_pc)
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
        let (cpu, bus) = self.emulator.hybrid_parts();
        let counts = cpu
            .hybrid_instruction_counts()
            .expect("enabled with call observer");
        let executed =
            counts.main_thread + counts.exception_handler != observer.previous_instructions;
        let is_call = executed
            && counts.last_instruction.is_some_and(|instruction| {
                instruction.completed_successfully
                    && (instruction.opcode & 0xffc0 == 0x4e80
                        || instruction.opcode & 0xff00 == 0x6100)
            });
        let pc = cpu.pc();
        let sp = cpu.stack_pointer();
        let return_word = if is_call && observer.coverage.iter().any(|entry| entry.entry_pc == pc) {
            if sp & 1 != 0 || (sp & 0xffffff) < 0xe00000 {
                return Err(HostError::Metadata(
                    "observed call has no aligned work-RAM return slot".into(),
                ));
            }
            let (ram, _) = bus.memory.debug_ram_view();
            let high = ram[((sp & 0xffff) / 2) as usize];
            let low = ram[(((sp + 2) & 0xffff) / 2) as usize];
            Some((u32::from(high) << 16) | u32::from(low))
        } else {
            None
        };
        observer.after_tick(
            counts,
            CallBoundary {
                pc,
                sp,
                depth: cpu.hybrid_exception_depth(),
                clock: bus.cycles.m68k_mclk_cycles,
                frame: self.frame + u64::from(rendered),
            },
            return_word,
        )
    }
}

#[cfg(test)]
mod tests;
