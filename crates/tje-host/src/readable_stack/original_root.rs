//! Positive per-access invocation evidence, never parent read-permission restoration.
use crate::call_observation::{
    invocation::{
        Invalidation, InvocationPoint, InvocationRecord, InvocationTracker, InvocationWitness,
    },
    successful_call_instruction,
};
use genesis_core::{GenesisBus, HybridAccess};
use m68000_emu::{HybridExecutedInstruction, M68000};
use std::sync::Mutex;
use tje_game::{
    FunctionId,
    call_frame::{CallBoundaryEvent, FrameState},
};
use tje_rom::Revision;

pub(crate) type OriginalRootWitness = InvocationWitness;
#[derive(Debug)]
struct State {
    tracker: InvocationTracker,
    before_count: Option<u64>,
    instruction: Option<HybridExecutedInstruction>,
    in_step: bool,
    rejected: bool,
    first_exception: Option<crate::call_observation::invocation::ExceptionContextId>,
}
#[derive(Debug)]
pub(crate) struct OriginalRootScope {
    state: Mutex<State>,
    prepared: bool,
}
fn total(cpu: &M68000) -> Option<u64> {
    let counts = cpu.hybrid_instruction_counts()?;
    counts.main_thread.checked_add(counts.exception_handler)
}
pub(crate) fn return_word(bus: &GenesisBus, sp: u32) -> Option<u32> {
    if sp & 1 != 0 || sp & 0xffffff < 0xe00000 {
        return None;
    }
    Some(
        (u32::from(bus.memory.read_main_ram_word(sp)) << 16)
            | u32::from(bus.memory.read_main_ram_word(sp.wrapping_add(2))),
    )
}
impl OriginalRootScope {
    pub(crate) fn new(
        cpu: &M68000,
        bus: &GenesisBus,
        revision: Revision,
        function: FunctionId,
        entry_pc: u32,
        entry_sp: u32,
        prepared: bool,
    ) -> Self {
        Self {
            state: Mutex::new(State {
                tracker: InvocationTracker::rooted(
                    revision,
                    function,
                    entry_pc,
                    entry_sp,
                    InvocationPoint::capture(cpu),
                    return_word(bus, entry_sp),
                    prepared,
                ),
                before_count: None,
                instruction: None,
                in_step: false,
                rejected: false,
                first_exception: None,
            }),
            prepared,
        }
    }
    pub(crate) fn before_step(&self, cpu: &M68000, ready: bool) {
        let mut state = self
            .state
            .lock()
            .expect("invocation recorder lock poisoned");
        state.before_count = total(cpu);
        state.instruction = None;
        state.in_step = true;
        state
            .tracker
            .begin_actual_step(InvocationPoint::capture(cpu), cpu.next_opcode(), ready);
    }
    /// Revoke an IRQ/bank transition before attributing any of this tick's reads.
    pub(crate) fn after_step(&self, cpu: &M68000, bus: &GenesisBus) {
        let mut state = self
            .state
            .lock()
            .expect("invocation recorder lock poisoned");
        state.tracker.observe_point(InvocationPoint::capture(cpu));
        if bus
            .hybrid_exception_boundaries
            .as_ref()
            .is_some_and(|boundaries| !boundaries.is_empty())
        {
            state.tracker.invalidate(Invalidation::ExceptionOrBank);
        }
        state.instruction = total(cpu)
            .zip(state.before_count)
            .filter(|(after, before)| after > before)
            .and_then(|_| cpu.hybrid_instruction_counts()?.last_instruction);
    }
    pub(crate) fn observe_stream(
        &self,
        access: HybridAccess,
        boundaries: &[genesis_core::HybridExceptionBoundary],
    ) {
        let mut state = self
            .state
            .lock()
            .expect("invocation recorder lock poisoned");
        state.tracker.observe_actual_access(access);
        if let HybridAccess::ExceptionBoundary { index } = access {
            if let Some(boundary) = boundaries.get(index as usize) {
                let update = state.tracker.observe_exception_boundary(boundary.boundary);
                if state.first_exception.is_none() {
                    state.first_exception = update.exception_entered.or(update.exception_returned);
                }
            } else {
                state.rejected = true;
                state.tracker.invalidate(Invalidation::MissingReceipt);
            }
        }
    }
    /// Complete only after the existing chronological access drain has consumed
    /// this step's real CPU writes. No instruction-issue/final-RAM inference.
    pub(crate) fn finish_step(&self, cpu: &M68000, bus: &GenesisBus) {
        let mut state = self
            .state
            .lock()
            .expect("invocation recorder lock poisoned");
        if !state.in_step {
            return;
        }
        let instruction = state.instruction.take();
        let frame = instruction
            .filter(|instruction| successful_call_instruction(*instruction))
            .and_then(|_| return_word(bus, cpu.stack_pointer()));
        state
            .tracker
            .finish_actual_step(instruction, InvocationPoint::capture(cpu), frame);
        state.in_step = false;
    }
    pub(crate) fn witness(&self) -> Option<OriginalRootWitness> {
        let state = self
            .state
            .lock()
            .expect("invocation recorder lock poisoned");
        if state.rejected {
            return None;
        }
        if state.in_step {
            state.tracker.step_witness()
        } else {
            state.tracker.current_witness()
        }
    }
    pub(crate) fn native_witness(
        &self,
        cpu: &M68000,
        frame: FrameState,
        owner: FunctionId,
    ) -> Option<OriginalRootWitness> {
        let mut state = self
            .state
            .lock()
            .expect("invocation recorder lock poisoned");
        if state.rejected {
            return None;
        }
        let actual = InvocationPoint::capture(cpu);
        state.tracker.observe_point(actual);
        let witness = state.tracker.current_witness()?;
        let record = state.tracker.resolve(witness)?;
        (record.owner() == Some(owner)
            && record.entry_sp() == Some(frame.entry_sp)
            && record.supervisor() == actual.supervisor()
            && record.exception_depth() == actual.exception_depth
            && record.exception_context().is_none()
            && (record.parent().is_none() || frame.is_prepared_for(owner)))
        .then_some(witness)
    }
    pub(crate) fn observe_framework(
        &self,
        event: CallBoundaryEvent,
        cpu: &M68000,
        bus: &GenesisBus,
        frame: FrameState,
        materialization: Option<tje_game::stack_read::MaterializedFrame>,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("invocation recorder lock poisoned");
        if state.in_step {
            state.tracker.invalidate(Invalidation::Lifecycle);
            return;
        }
        let actual = InvocationPoint::capture(cpu);
        state.tracker.observe_framework(
            event,
            actual,
            frame,
            return_word(bus, actual.sp),
            materialization,
        );
    }
    pub(crate) fn record(&self, witness: OriginalRootWitness) -> Option<InvocationRecord> {
        let state = self
            .state
            .lock()
            .expect("invocation recorder lock poisoned");
        if state.rejected {
            return None;
        }
        let record = *state.tracker.resolve(witness)?;
        if !self.prepared && record.parent().is_some() {
            let actual = state.tracker.actual_call(witness.id())?;
            state.tracker.resolve_actual(actual)?;
        }
        Some(record)
    }
    pub(crate) fn parent(&self, witness: OriginalRootWitness) -> Option<InvocationRecord> {
        let state = self
            .state
            .lock()
            .expect("invocation recorder lock poisoned");
        if state.rejected {
            return None;
        }
        let record = state.tracker.resolve(witness)?;
        state.tracker.record(record.parent()?).copied()
    }
    pub(crate) fn first_later_exception(&self) -> Option<super::admission::ExceptionFrameEvidence> {
        let state = self
            .state
            .lock()
            .expect("invocation recorder lock poisoned");
        let record = state.tracker.exception_record(state.first_exception?)?;
        let parent = record
            .parent
            .and_then(|parent| state.tracker.exception_record(parent));
        let call = record
            .call_parent
            .and_then(|parent| state.tracker.record(parent));
        Some(super::admission::ExceptionFrameEvidence {
            vector: record.vector,
            handler: record.handler,
            depth: record.depth,
            occurrence: record.occurrence,
            parent_occurrence: parent.map(|parent| parent.occurrence),
            call_parent_owner: call
                .and_then(|call| call.owner())
                .map(|owner| owner.metadata().name),
            call_parent_entry_sp: call.and_then(|call| call.entry_sp()),
        })
    }
    pub(crate) fn current_materialization(
        &self,
    ) -> Option<tje_game::stack_read::MaterializedFrame> {
        let state = self
            .state
            .lock()
            .expect("invocation recorder lock poisoned");
        if state.rejected {
            return None;
        }
        state.tracker.current()?.materialization()
    }
    pub(crate) fn reject_capture(&self) {
        let mut state = self
            .state
            .lock()
            .expect("invocation recorder lock poisoned");
        state.rejected = true;
        state.tracker.invalidate(Invalidation::Lifecycle);
    }
    pub(crate) fn matches(
        &self,
        witness: OriginalRootWitness,
        other: &Self,
        other_witness: OriginalRootWitness,
    ) -> bool {
        if std::ptr::eq(self, other) {
            return false;
        }
        if (self as *const Self) < (other as *const Self) {
            let a = self
                .state
                .lock()
                .expect("invocation recorder lock poisoned");
            let b = other
                .state
                .lock()
                .expect("invocation recorder lock poisoned");
            if a.rejected || b.rejected {
                return false;
            }
            a.tracker
                .witnesses_match(witness, &b.tracker, other_witness)
        } else {
            other.matches(other_witness, self, witness)
        }
    }
}

#[cfg(test)]
mod tests;
