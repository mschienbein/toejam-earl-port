//! One normal-original pass. The shared recorder alone seals actual calls and
//! exceptions. This stack groups raw trace ownership; it grants no read witness.
use super::{
    Boundary, Chronology, Entry, EntryEvidence, EntryKind, Event, Invocation, InvocationId,
    ObservedStep, Trace,
};
use crate::replayed_environment::{Trajectory, Unsupported, UnsupportedKind};
use crate::{
    abi::canonical_ram_address,
    call_observation::{
        CallReturnPoint,
        invocation::{
            ActualCallId, ExceptionContextId, Invalidation, InvocationId as ActualInvocationId,
            InvocationPoint, InvocationTracker, InvocationUpdate,
        },
        successful_call_instruction,
    },
};
use genesis_core::{GenesisEmulator, HybridAccess};
use m68000_emu::ExceptionBoundary;
use std::sync::Arc;
use tje_game::FunctionId;
use tje_rom::Revision;

pub(in crate::replayed_environment) struct Target {
    pub function: Option<FunctionId>,
    pub wanted: bool,
}
pub(in crate::replayed_environment) struct Progress {
    pub entered: Option<Entry>,
    pub completed: Option<Arc<Invocation>>,
}
struct Active {
    entry: Entry,
    trace: Option<Trace>,
    unsupported: Option<Unsupported>,
    irq_return: Option<CallReturnPoint>,
    irq_sr: u32,
    own_code: Option<std::ops::Range<u32>>,
}
pub(in crate::replayed_environment) struct Tracker {
    trajectory: Trajectory,
    revision: Revision,
    shared: InvocationTracker,
    active: Vec<Active>,
}
fn unsupported(kind: UnsupportedKind, detail: impl Into<String>) -> Unsupported {
    Unsupported {
        kind,
        detail: detail.into(),
    }
}
fn point(boundary: Boundary) -> InvocationPoint {
    InvocationPoint {
        pc: boundary.cpu.pc(),
        sp: boundary.cpu.sp(),
        status: boundary.cpu.sr() as u16,
        usp: boundary.cpu.words[15],
        ssp: boundary.cpu.words[16],
        exception_depth: boundary.cpu.exception_depth,
    }
}
fn checked_update(update: InvocationUpdate) -> Result<InvocationUpdate, Unsupported> {
    if matches!(
        update.invalidated,
        Some(
            Invalidation::Exhausted
                | Invalidation::MissingReceipt
                | Invalidation::MissingFrame
                | Invalidation::Lifecycle
        )
    ) {
        return Err(unsupported(
            UnsupportedKind::Observation,
            format!(
                "shared actual invocation recorder rejected its source boundary: {:?}",
                update.invalidated
            ),
        ));
    }
    Ok(update)
}
impl Tracker {
    pub fn new(trajectory: Trajectory, revision: Revision) -> Self {
        Self {
            trajectory,
            revision,
            shared: InvocationTracker::capturing(revision, None),
            active: Vec::new(),
        }
    }
    pub fn current(&self) -> Option<InvocationId> {
        self.active.last().map(|scope| scope.entry.identity)
    }
    pub fn reject(&mut self, reason: Unsupported) {
        for scope in &mut self.active {
            if scope.trace.is_some() && scope.unsupported.is_none() {
                scope.unsupported = Some(reason.clone());
            }
        }
    }
    fn reject_current(&mut self, reason: Unsupported) {
        if let Some(scope) = self.active.last_mut()
            && scope.trace.is_some()
            && scope.unsupported.is_none()
        {
            scope.unsupported = Some(reason);
        }
    }
    fn parent_call(&self, skip: usize) -> Option<ActualInvocationId> {
        self.active
            .iter()
            .rev()
            .skip(skip)
            .find_map(|scope| match scope.entry.evidence {
                EntryEvidence::Call { raw, .. } => Some(raw),
                EntryEvidence::Interrupt(_) => None,
            })
    }
    fn parent_exception(&self, skip: usize) -> Option<ExceptionContextId> {
        self.active
            .iter()
            .rev()
            .skip(skip)
            .find_map(|scope| match scope.entry.evidence {
                EntryEvidence::Interrupt(id) => Some(id),
                EntryEvidence::Call { .. } => None,
            })
    }
    fn validate_call(
        &self,
        raw: ActualInvocationId,
        actual: ActualCallId,
        boundary: Boundary,
        kind: EntryKind,
        function: Option<FunctionId>,
        skip: usize,
    ) -> Result<(), Unsupported> {
        let record = self
            .shared
            .resolve_actual(actual)
            .filter(|_| self.shared.actual_call(raw) == Some(actual))
            .ok_or_else(|| {
                unsupported(
                    UnsupportedKind::Observation,
                    "actual call seal belongs to another capture, index or serial",
                )
            })?;
        let EntryKind::Call {
            caller_pc,
            opcode,
            return_pc,
        } = kind
        else {
            return Err(unsupported(
                UnsupportedKind::Observation,
                "actual call seal was assigned to an exception group",
            ));
        };
        let parent = self.parent_call(skip);
        let depth = parent
            .map(|id| {
                self.shared
                    .record(id)
                    .and_then(|record| record.invocation_depth().checked_add(1))
            })
            .unwrap_or(Some(0));
        if record.entry_point() != point(boundary)
            || record.entry_pc() != boundary.cpu.pc()
            || record.entry_sp() != Some(boundary.cpu.sp())
            || record.owner() != function
            || record.caller_pc() != Some(caller_pc)
            || record.opcode() != Some(opcode)
            || record.return_pc() != Some(return_pc)
            || record.return_site() != Some(return_pc)
            || record.supervisor() != (boundary.cpu.sr() & 0x2000 != 0)
            || record.exception_depth() != boundary.cpu.exception_depth
            || record.parent() != parent
            || Some(record.invocation_depth()) != depth
            || record.occurrence() == 0
            || record.exception_context() != self.parent_exception(skip)
        {
            return Err(unsupported(
                UnsupportedKind::Observation,
                "shared actual call seal differs in owner, target, frame, bank, return site, parent, depth, occurrence or IRQ context",
            ));
        }
        Ok(())
    }
    fn validate_exception(
        &self,
        id: ExceptionContextId,
        boundary: Boundary,
        kind: EntryKind,
        skip: usize,
    ) -> Result<(), Unsupported> {
        let record = self.shared.exception_record(id).ok_or_else(|| {
            unsupported(
                UnsupportedKind::Observation,
                "actual exception context belongs to another capture or index",
            )
        })?;
        let EntryKind::Interrupt { vector } = kind else {
            return Err(unsupported(
                UnsupportedKind::Observation,
                "exception context was assigned to a call group",
            ));
        };
        if record.vector != vector
            || record.handler != boundary.cpu.pc()
            || record.depth != boundary.cpu.exception_depth
            || record.occurrence == 0
            || record.parent != self.parent_exception(skip)
            || record.call_parent != self.parent_call(skip)
        {
            return Err(unsupported(
                UnsupportedKind::Observation,
                "shared exception context differs in vector, handler, depth, occurrence or call/IRQ parent",
            ));
        }
        Ok(())
    }
    fn validate(&self, entry: Entry, skip: usize) -> Result<(), Unsupported> {
        let parent = self
            .active
            .iter()
            .rev()
            .nth(skip)
            .map(|scope| scope.entry.identity);
        if entry.identity.trajectory() != self.trajectory || entry.parent != parent {
            return Err(unsupported(
                UnsupportedKind::Observation,
                "raw trace group lost its sealed parent association",
            ));
        }
        match entry.evidence {
            EntryEvidence::Call { raw, actual } => self.validate_call(
                raw,
                actual,
                entry.boundary,
                entry.kind,
                entry.function,
                skip,
            ),
            EntryEvidence::Interrupt(id) => {
                self.validate_exception(id, entry.boundary, entry.kind, skip)
            }
        }
    }
    fn open(
        &mut self,
        boundary: Boundary,
        kind: EntryKind,
        target: Target,
        evidence: EntryEvidence,
        irq_return: Option<CallReturnPoint>,
        irq_sr: u32,
    ) -> Result<Entry, Unsupported> {
        let entry = Entry {
            identity: InvocationId::new(self.trajectory),
            function: target.function,
            boundary,
            kind,
            parent: self.current(),
            evidence,
        };
        self.validate(entry, 0)?;
        let retained = target.wanted
            || self
                .active
                .last()
                .is_some_and(|scope| scope.trace.is_some());
        self.active.push(Active {
            entry,
            trace: retained.then(Trace::default),
            unsupported: None,
            irq_return,
            irq_sr,
            // Semantic roots need own-PC ownership. Canonical production
            // auto-roots retain their independent complete strict checker,
            // including its already-proven shared arithmetic/music bodies.
            own_code: target
                .function
                .filter(|id| {
                    tje_game::execution_contract(*id).is_some_and(|contract| {
                        contract.proof_envelope
                            == tje_game::execution::ProofEnvelope::ReplayedEnvironment
                    })
                })
                .map(|id| {
                    let start = id.metadata().address(self.revision);
                    start
                        ..start
                            .checked_add(id.metadata().size(self.revision))
                            .expect("committed function extent overflows")
                }),
        });
        Ok(entry)
    }
    fn close(
        &mut self,
        exit: Boundary,
        completion_evidence: EntryEvidence,
    ) -> Result<Arc<Invocation>, Unsupported> {
        let entry = self
            .active
            .last()
            .ok_or_else(|| {
                unsupported(
                    UnsupportedKind::Observation,
                    "shared return has no raw trace group",
                )
            })?
            .entry;
        // Historical call identity remains resolvable after an IRQ revoked read
        // permission. No InvocationWitness is requested, recreated or rearmed.
        self.validate(entry, 1)?;
        if completion_evidence != entry.evidence {
            return Err(unsupported(
                UnsupportedKind::Observation,
                "actual shared completion identity differs from the retained entry",
            ));
        }
        let scope = self.active.pop().expect("validated trace group");
        let invocation = Arc::new(Invocation {
            entry: scope.entry,
            exit,
            completion_evidence,
            trace: scope.trace.unwrap_or_default(),
            unsupported: scope.unsupported,
        });
        if let Some(parent) = self.active.last_mut()
            && let Some(trace) = &mut parent.trace
        {
            trace.events.push(match invocation.entry.kind {
                EntryKind::Call { .. } => Event::Callee(Arc::clone(&invocation)),
                EntryKind::Interrupt { .. } => Event::Interrupt(Arc::clone(&invocation)),
            });
        }
        Ok(invocation)
    }
    pub fn observe(
        &mut self,
        emulator: &GenesisEmulator,
        step: &ObservedStep<'_>,
        target: Target,
    ) -> Result<Progress, Unsupported> {
        // All inputs are retained before-step metadata and existing physical
        // journals. No post-entry reconstruction or additional bus read occurs.
        if step.actual.is_some() && !step.instruction_ready {
            return Err(unsupported(
                UnsupportedKind::Observation,
                "actual opcode lacks its original before-ready point; an intermediate source observation is missing",
            ));
        }
        if let Some(actual) = step.actual
            && self
                .active
                .last()
                .and_then(|scope| scope.own_code.as_ref())
                .is_some_and(|range| !range.contains(&actual.pc))
        {
            self.reject_current(unsupported(UnsupportedKind::TypedControlProfile,
                "own instruction/read owner PC is outside its committed function extent; an unsealed tail is unsupported"));
        }
        self.shared.begin_actual_step(
            point(step.before),
            step.before.cpu.opcode,
            step.instruction_ready,
        );
        let mut entered = None;
        let mut returned = None;
        for &event in step.chronology {
            match event {
                Chronology::CpuCommit { access, .. } => {
                    self.shared.observe_actual_access(HybridAccess::Write {
                        address: access.address,
                        word: access.word,
                        value: access.value,
                    })
                }
                Chronology::Exception(payload) => {
                    let update =
                        checked_update(self.shared.observe_exception_boundary(payload.boundary))?;
                    match payload.boundary {
                        ExceptionBoundary::Entered {
                            vector,
                            depth,
                            handler,
                        } => {
                            let id = update.exception_entered.ok_or_else(|| {
                                unsupported(
                                    UnsupportedKind::Observation,
                                    "shared recorder did not seal its actual exception entry",
                                )
                            })?;
                            if entered.is_some() || step.actual.is_some() {
                                return Err(unsupported(
                                    UnsupportedKind::TypedControlProfile,
                                    "coalesced exception/opcode needs an intermediate original observation; no call seal is reconstructed after IRQ return",
                                ));
                            }
                            if !(25..=31).contains(&vector) {
                                self.reject_current(unsupported(
                                    UnsupportedKind::TypedControlProfile,
                                    "synchronous exception is not an ordinary IRQ environment",
                                ));
                            }
                            if step.before.cpu.exception_depth.checked_add(1) != Some(depth)
                                || depth != step.after.cpu.exception_depth
                                || handler != step.after.cpu.pc()
                            {
                                return Err(unsupported(
                                    UnsupportedKind::Observation,
                                    "actual exception boundary differs from its retained before/after points",
                                ));
                            }
                            entered = Some((
                                id,
                                vector,
                                point(step.before).returned(),
                                step.before.cpu.sr(),
                            ));
                        }
                        ExceptionBoundary::Returned { .. } => {
                            let id = update.exception_returned.ok_or_else(|| {
                                unsupported(
                                    UnsupportedKind::Observation,
                                    "shared recorder did not resolve its actual exception return",
                                )
                            })?;
                            if returned.replace(id).is_some() {
                                return Err(unsupported(
                                    UnsupportedKind::Observation,
                                    "multiple exception returns in one actual tick",
                                ));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if entered.is_none()
            && let Some(actual) = step.actual
            && let Some(scope) = self.active.last_mut()
        {
            if actual.exception_depth != scope.entry.boundary.cpu.exception_depth {
                return Err(unsupported(
                    UnsupportedKind::Observation,
                    "opcode is outside its sealed call/IRQ trace ownership",
                ));
            }
            if let Some(trace) = &mut scope.trace {
                trace.instruction(
                    step.before,
                    step.after,
                    actual,
                    step.issues,
                    step.driven_masks,
                    step.chronology,
                );
            }
            if !actual.completed_successfully {
                self.reject_current(unsupported(
                    UnsupportedKind::TypedControlProfile,
                    "faulted original opcode is not an ordinary return proof",
                ));
            }
        }
        // This is the same readonly physical return-slot observation used by
        // the existing observer; the shared recorder validates both real commits.
        let return_pc = if step.actual.is_some_and(successful_call_instruction) {
            let sp = step.after.cpu.sp();
            let physical = canonical_ram_address(sp);
            if sp & 1 != 0
                || !(crate::catalogue::STACK_BOTTOM..=crate::catalogue::STACK_TOP - 4)
                    .contains(&physical)
            {
                return Err(unsupported(
                    UnsupportedKind::Frame,
                    "actual call has no aligned reviewed work-RAM return slot",
                ));
            }
            let ram = emulator.hybrid_bus().memory.hybrid_raw_main_ram();
            let index = ((sp & 65535) / 2) as usize;
            Some((u32::from(ram[index]) << 16) | u32::from(ram[index + 1]))
        } else {
            None
        };
        let update = checked_update(self.shared.finish_actual_step(
            step.actual,
            point(step.after),
            return_pc,
        ))?;
        if let Some((id, vector, irq_return, sr)) = entered {
            if update.entered.is_some() || update.returned.is_some() || returned.is_some() {
                return Err(unsupported(
                    UnsupportedKind::Observation,
                    "exception entry coalesced with another sealed boundary",
                ));
            }
            if (25..=31).contains(&vector) {
                let mut acknowledgements = step.chronology.iter().filter_map(|event| match event {
                    Chronology::Acknowledge { level } => Some(*level),
                    _ => None,
                });
                if acknowledgements
                    .next()
                    .map(u32::from)
                    .map(|level| level + 24)
                    != Some(vector)
                    || acknowledgements.next().is_some()
                {
                    return Err(unsupported(
                        UnsupportedKind::Observation,
                        "actual IRQ ordering class lacks its single matching CPU acknowledgement",
                    ));
                }
            }
            let entry = self.open(
                step.after,
                EntryKind::Interrupt { vector },
                target,
                EntryEvidence::Interrupt(id),
                Some(irq_return),
                sr,
            )?;
            if let Some(trace) = &mut self.active.last_mut().expect("sealed IRQ group").trace {
                trace.idle(
                    step.before,
                    step.after,
                    step.issues,
                    step.driven_masks,
                    step.chronology,
                );
            }
            return Ok(Progress {
                entered: Some(entry),
                completed: None,
            });
        }
        if let Some(raw) = update.entered {
            let actual = self.shared.actual_call(raw).ok_or_else(|| unsupported(UnsupportedKind::Observation,
                "shared call entry lacks both actual return commits; no post-entry call reconstruction is permitted"))?;
            let record = self.shared.resolve_actual(actual).ok_or_else(|| {
                unsupported(
                    UnsupportedKind::Observation,
                    "shared actual call seal is unavailable",
                )
            })?;
            let kind = EntryKind::Call {
                caller_pc: record.caller_pc().ok_or_else(|| {
                    unsupported(
                        UnsupportedKind::Observation,
                        "sealed call lacks its actual source site",
                    )
                })?,
                opcode: record.opcode().ok_or_else(|| {
                    unsupported(
                        UnsupportedKind::Observation,
                        "sealed call lacks its actual opcode",
                    )
                })?,
                return_pc: record.return_site().ok_or_else(|| {
                    unsupported(
                        UnsupportedKind::Observation,
                        "sealed call lacks its committed return site",
                    )
                })?,
            };
            if target.function.is_none() {
                self.reject_current(unsupported(
                    UnsupportedKind::CalleeCoverage,
                    "original call target is uncatalogued",
                ));
            }
            let entry = self.open(
                step.after,
                kind,
                target,
                EntryEvidence::Call { raw, actual },
                None,
                0,
            )?;
            return Ok(Progress {
                entered: Some(entry),
                completed: None,
            });
        }
        if let Some(raw) = update.returned {
            if returned.is_some() || !self.active.last().is_some_and(|scope| matches!(scope.entry.evidence, EntryEvidence::Call { raw: expected, .. } if expected == raw)) {
                return Err(unsupported(UnsupportedKind::Observation, "shared actual RTS identity differs from the current raw trace group"));
            }
            let actual = self.shared.actual_call(raw).ok_or_else(|| {
                unsupported(
                    UnsupportedKind::Observation,
                    "actual shared RTS omitted its historical completion identity",
                )
            })?;
            return Ok(Progress {
                entered: None,
                completed: Some(self.close(step.after, EntryEvidence::Call { raw, actual })?),
            });
        }
        if let Some(id) = returned {
            let scope = self.active.last().ok_or_else(|| {
                unsupported(
                    UnsupportedKind::Observation,
                    "actual exception return has no raw IRQ trace group",
                )
            })?;
            if !matches!(scope.entry.evidence, EntryEvidence::Interrupt(expected) if expected == id)
                || scope.irq_return != Some(point(step.after).returned())
                || step.after.cpu.sr() != scope.irq_sr
                || !step
                    .actual
                    .is_some_and(|actual| actual.completed_successfully && actual.opcode == 0x4e73)
            {
                return Err(unsupported(
                    UnsupportedKind::TypedControlProfile,
                    "actual IRQ return changed its interrupted frame/status or has no matching shared exception identity",
                ));
            }
            return Ok(Progress {
                entered: None,
                completed: Some(self.close(step.after, EntryEvidence::Interrupt(id))?),
            });
        }
        if let Some(actual) = step.actual {
            if successful_call_instruction(actual) {
                return Err(unsupported(
                    UnsupportedKind::Observation,
                    "actual JSR/BSR has no shared source call seal",
                ));
            }
            if actual.opcode & 0xffc0 == 0x4ec0
                && self
                    .active
                    .last()
                    .is_some_and(|scope| scope.own_code.is_some())
                && !matches!(actual.opcode & 63, 0x38..=0x3a)
            {
                self.reject_current(unsupported(
                    UnsupportedKind::IndirectControl,
                    "computed/indirect own JMP is unsupported",
                ));
            }
            if actual.completed_successfully
                // The shared recorder alone seals the ordinary RTS/RTE after
                // retirement; its architectural target is not an own-code tail.
                && !matches!(actual.opcode, 0x4e75 | 0x4e73)
                && self.active.last().is_some_and(|scope| {
                    scope.own_code.as_ref().is_some_and(|range|
                        target.function.is_some() && target.function != scope.entry.function
                        || !range.contains(&step.after.cpu.pc()))
                })
            {
                self.reject_current(unsupported(
                    UnsupportedKind::TypedControlProfile,
                    "unsealed cross-catalogue JMP/BRA/fallthrough requires an explicit reviewed tail profile",
                ));
            }
        } else if let Some(scope) = self.active.last_mut()
            && let Some(trace) = &mut scope.trace
        {
            trace.idle(
                step.before,
                step.after,
                step.issues,
                step.driven_masks,
                step.chronology,
            );
        }
        Ok(Progress {
            entered: None,
            completed: None,
        })
    }
}

#[cfg(all(test, feature = "reference-proof-hooks"))]
mod tests;
