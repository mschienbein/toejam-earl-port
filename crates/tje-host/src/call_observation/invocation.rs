//! One actual/prepared invocation metadata recorder, shared by observation and
//! readable-check witnesses. Metadata depth restoration never restores permission.
use super::{CallReturnPoint, matches_successful_rts, successful_call_instruction};
use m68000_emu::HybridExecutedInstruction;
use std::sync::{
    LazyLock,
    atomic::{AtomicU64, Ordering},
};
use tje_game::{
    FunctionId,
    call_frame::{CallBoundaryEvent, CallBoundaryKind, FrameState},
};
use tje_rom::Revision;

static NEXT_CAPTURE: AtomicU64 = AtomicU64::new(1);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InvocationId {
    capture: u64,
    index: usize,
    serial: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InvocationWitness(InvocationId);
/// Historical actual-call evidence, independent of revocable read permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActualCallId(InvocationId);
impl InvocationWitness {
    pub(crate) fn id(&self) -> InvocationId {
        self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExceptionContextId {
    capture: u64,
    index: usize,
}
/// Actual exception context is distinct from the call-parent tree. It records
/// only the observed payload, never a reconstructed exception-frame SP.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ExceptionContextRecord {
    pub vector: u32,
    pub handler: u32,
    pub depth: u32,
    pub occurrence: u64,
    pub parent: Option<ExceptionContextId>,
    pub call_parent: Option<InvocationId>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InvocationPoint {
    pub pc: u32,
    pub sp: u32,
    pub status: u16,
    pub usp: u32,
    pub ssp: u32,
    pub exception_depth: u32,
}
impl InvocationPoint {
    pub(crate) fn capture(cpu: &m68000_emu::M68000) -> Self {
        Self {
            pc: cpu.pc(),
            sp: cpu.stack_pointer(),
            status: cpu.status_register(),
            usp: cpu.user_stack_pointer(),
            ssp: cpu.supervisor_stack_pointer(),
            exception_depth: cpu.hybrid_exception_depth(),
        }
    }
    pub(crate) fn supervisor(&self) -> bool {
        self.status & 0x2000 != 0
    }
    pub(crate) fn returned(&self) -> CallReturnPoint {
        CallReturnPoint {
            pc: self.pc,
            sp: self.sp,
            depth: self.exception_depth,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Invalidation {
    Call,
    Return,
    Nonlocal,
    ExceptionOrBank,
    MissingReceipt,
    MissingFrame,
    MissingOwner,
    Lifecycle,
    Exhausted,
}
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct InvocationUpdate {
    pub entered: Option<InvocationId>,
    pub returned: Option<InvocationId>,
    pub invalidated: Option<Invalidation>,
    pub exception_entered: Option<ExceptionContextId>,
    pub exception_returned: Option<ExceptionContextId>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameKind {
    Root,
    Actual,
    Prepared,
    Elided,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct InvocationRecord {
    serial: u64,
    owner: Option<FunctionId>,
    entry_pc: u32,
    entry_point: InvocationPoint,
    entry_sp: Option<u32>,
    return_pc: Option<u32>,
    return_site: Option<u32>,
    caller_pc: Option<u32>,
    opcode: Option<u16>,
    supervisor: bool,
    exception_depth: u32,
    invocation_depth: u32,
    parent: Option<InvocationId>,
    occurrence: u64,
    kind: FrameKind,
    lineage_known: bool,
    materialization: Option<tje_game::stack_read::MaterializedFrame>,
    exception_context: Option<ExceptionContextId>,
}
impl InvocationRecord {
    pub(crate) fn owner(&self) -> Option<FunctionId> {
        self.owner
    }
    pub(crate) fn entry_pc(&self) -> u32 {
        self.entry_pc
    }
    pub(crate) fn entry_point(&self) -> InvocationPoint {
        self.entry_point
    }
    pub(crate) fn entry_sp(&self) -> Option<u32> {
        self.entry_sp
    }
    pub(crate) fn return_pc(&self) -> Option<u32> {
        self.return_pc
    }
    pub(crate) fn caller_pc(&self) -> Option<u32> {
        self.caller_pc
    }
    pub(crate) fn return_site(&self) -> Option<u32> {
        self.return_site
    }
    pub(crate) fn opcode(&self) -> Option<u16> {
        self.opcode
    }
    pub(crate) fn supervisor(&self) -> bool {
        self.supervisor
    }
    pub(crate) fn exception_depth(&self) -> u32 {
        self.exception_depth
    }
    pub(crate) fn invocation_depth(&self) -> u32 {
        self.invocation_depth
    }
    pub(crate) fn parent(&self) -> Option<InvocationId> {
        self.parent
    }
    pub(crate) fn occurrence(&self) -> u64 {
        self.occurrence
    }
    pub(crate) fn materialization(&self) -> Option<tje_game::stack_read::MaterializedFrame> {
        self.materialization
    }
    pub(crate) fn exception_context(&self) -> Option<ExceptionContextId> {
        self.exception_context
    }
    fn return_point(&self) -> Option<CallReturnPoint> {
        Some(CallReturnPoint {
            pc: self.return_pc?,
            sp: self.entry_sp?.wrapping_add(4),
            depth: self.exception_depth,
        })
    }
}
#[derive(Debug)]
struct ActiveInvocation {
    id: InvocationId,
    child_occurrences: u64,
    permitted: bool,
    physical_returned: bool,
    elided_sp: Option<u32>,
}
#[derive(Debug, Clone, Copy)]
struct PendingStep {
    before: InvocationPoint,
    opcode: u16,
    ready: bool,
    last_writes: [Option<(u32, u16)>; 2],
}
#[derive(Debug)]
pub(crate) struct InvocationTracker {
    capture: u64,
    revision: Revision,
    root: Option<(FunctionId, u32)>,
    records: Vec<InvocationRecord>,
    next_record_serial: u64,
    retain_history: bool,
    active: Vec<ActiveInvocation>,
    orphan_occurrences: u64,
    lineage_intact: bool,
    root_permitted: bool,
    prepared: bool,
    pending: Option<PendingStep>,
    step_witness: Option<InvocationWitness>,
    exception_records: Vec<ExceptionContextRecord>,
    exception_active: Vec<ExceptionContextId>,
    exception_base: Option<u32>,
}
impl InvocationTracker {
    pub(crate) fn observing(revision: Revision) -> Self {
        let mut tracker = Self::capturing(revision, None);
        tracker.retain_history = false;
        tracker
    }
    pub(crate) fn capturing(revision: Revision, root: Option<(FunctionId, u32)>) -> Self {
        let mut tracker = Self::new(revision, root, false);
        tracker.root_permitted = false;
        tracker
    }
    fn new(revision: Revision, root: Option<(FunctionId, u32)>, prepared: bool) -> Self {
        let capture = NEXT_CAPTURE
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .unwrap_or(0);
        Self {
            capture,
            revision,
            root,
            records: Vec::new(),
            next_record_serial: 0,
            retain_history: true,
            active: Vec::new(),
            orphan_occurrences: 0,
            lineage_intact: capture != 0,
            root_permitted: root.is_some(),
            prepared,
            pending: None,
            step_witness: None,
            exception_records: Vec::new(),
            exception_active: Vec::new(),
            exception_base: None,
        }
    }
    pub(crate) fn rooted(
        revision: Revision,
        owner: FunctionId,
        expected_pc: u32,
        expected_sp: u32,
        actual: InvocationPoint,
        return_pc: Option<u32>,
        prepared: bool,
    ) -> Self {
        let mut tracker = Self::new(revision, Some((owner, expected_pc)), prepared);
        tracker.exception_base = Some(actual.exception_depth);
        let known = actual.pc == expected_pc
            && actual.sp == expected_sp
            && return_pc.is_some()
            && expected_sp & 1 == 0
            && expected_sp & 0xffffff >= 0xe00000;
        let record = InvocationRecord {
            serial: 0,
            owner: Some(owner),
            entry_pc: expected_pc,
            entry_point: actual,
            entry_sp: Some(expected_sp),
            return_pc,
            return_site: return_pc,
            caller_pc: None,
            opcode: None,
            supervisor: actual.supervisor(),
            exception_depth: actual.exception_depth,
            invocation_depth: 0,
            parent: None,
            occurrence: 0,
            kind: FrameKind::Root,
            lineage_known: known,
            materialization: None,
            exception_context: None,
        };
        tracker.next_record_serial = 1;
        tracker.push(record, known);
        tracker
    }
    fn push(&mut self, mut record: InvocationRecord, permitted: bool) -> InvocationId {
        let id = InvocationId {
            capture: self.capture,
            index: self.records.len(),
            serial: self.next_record_serial,
        };
        record.serial = id.serial;
        self.records.push(record);
        self.active.push(ActiveInvocation {
            id,
            child_occurrences: 0,
            permitted: permitted && self.root_permitted && self.lineage_intact,
            physical_returned: false,
            elided_sp: None,
        });
        id
    }
    pub(crate) fn record(&self, id: InvocationId) -> Option<&InvocationRecord> {
        (self.capture != 0 && id.capture == self.capture)
            .then(|| self.records.get(id.index))
            .flatten()
            .filter(|record| record.serial == id.serial)
    }
    pub(crate) fn resolve(&self, witness: InvocationWitness) -> Option<&InvocationRecord> {
        self.record(witness.0).filter(|record| record.lineage_known)
    }
    pub(crate) fn actual_call(&self, id: InvocationId) -> Option<ActualCallId> {
        if !self.retain_history {
            return None;
        }
        self.record(id)
            .filter(|record| record.kind == FrameKind::Actual && record.return_site.is_some())
            .map(|_| ActualCallId(id))
    }
    pub(crate) fn resolve_actual(&self, id: ActualCallId) -> Option<&InvocationRecord> {
        if !self.retain_history {
            return None;
        }
        self.record(id.0)
            .filter(|record| record.kind == FrameKind::Actual && record.return_site.is_some())
    }
    /// Measurement consumers must retire only after reading the returned DTO.
    /// Proof/capture records and their historical IDs are never retired here.
    pub(crate) fn retire_measurement_return(&mut self, id: InvocationId) -> bool {
        if self.retain_history
            || self.prepared
            || self.root.is_some()
            || id.index.checked_add(1) != Some(self.records.len())
            || self.record(id).is_none()
            || self
                .active
                .last()
                .is_some_and(|active| active.id.index >= id.index)
        {
            return false;
        }
        self.records.pop();
        true
    }
    #[cfg(test)]
    pub(super) fn measurement_storage(&self) -> (usize, usize) {
        (self.records.len(), self.records.capacity())
    }
    pub(crate) fn current_witness(&self) -> Option<InvocationWitness> {
        let active = self.active.last()?;
        (active.permitted && self.lineage_intact).then_some(InvocationWitness(active.id))
    }
    pub(crate) fn step_witness(&self) -> Option<InvocationWitness> {
        self.step_witness
    }
    pub(crate) fn current(&self) -> Option<&InvocationRecord> {
        self.record(self.active.last()?.id)
    }
    fn revoke_current(&mut self, reason: Invalidation) -> InvocationUpdate {
        if let Some(active) = self.active.last_mut() {
            active.permitted = false;
        }
        InvocationUpdate {
            invalidated: Some(reason),
            ..Default::default()
        }
    }
    pub(crate) fn invalidate(&mut self, reason: Invalidation) -> InvocationUpdate {
        self.lineage_intact = false;
        self.step_witness = None;
        for active in &mut self.active {
            active.permitted = false;
        }
        InvocationUpdate {
            invalidated: Some(reason),
            ..Default::default()
        }
    }
    fn point_matches(&self, point: InvocationPoint) -> bool {
        self.current().is_none_or(|record| {
            record.supervisor == point.supervisor()
                && record.exception_depth == point.exception_depth
        })
    }
    pub(crate) fn observe_point(&mut self, point: InvocationPoint) -> InvocationUpdate {
        if self.point_matches(point) {
            InvocationUpdate::default()
        } else {
            self.invalidate(Invalidation::ExceptionOrBank)
        }
    }
    pub(crate) fn begin_actual_step(&mut self, point: InvocationPoint, opcode: u16, ready: bool) {
        self.exception_base.get_or_insert(point.exception_depth);
        self.observe_point(point);
        self.step_witness = if ready { self.current_witness() } else { None };
        if ready && attempted_call(opcode) {
            self.revoke_current(Invalidation::Call);
            self.step_witness = None;
        } else if ready && opcode == 0x4e75 {
            self.revoke_current(Invalidation::Return);
            self.step_witness = None;
        } else if ready && nonlocal_opcode(opcode) {
            self.invalidate(Invalidation::Nonlocal);
        }
        self.pending = Some(PendingStep {
            before: point,
            opcode,
            ready,
            last_writes: [None; 2],
        });
    }
    /// Consume existing chronological CPU commits; never fetch a bus value here.
    pub(crate) fn observe_actual_access(&mut self, access: genesis_core::HybridAccess) {
        if let Some(step) = &mut self.pending
            && let genesis_core::HybridAccess::Write {
                address,
                word,
                value,
            } = access
        {
            step.last_writes = [
                step.last_writes[1],
                word.then_some((address & 0xffffff, value)),
            ];
        }
    }
    pub(crate) fn exception_record(
        &self,
        id: ExceptionContextId,
    ) -> Option<&ExceptionContextRecord> {
        (self.capture != 0 && id.capture == self.capture)
            .then(|| self.exception_records.get(id.index))
            .flatten()
    }
    pub(crate) fn observe_exception_boundary(
        &mut self,
        boundary: m68000_emu::ExceptionBoundary,
    ) -> InvocationUpdate {
        if self.capture == 0 {
            return self.invalidate(Invalidation::Exhausted);
        }
        self.invalidate(Invalidation::ExceptionOrBank);
        match boundary {
            m68000_emu::ExceptionBoundary::Entered {
                vector,
                handler,
                depth,
            } => {
                let parent = self.exception_active.last().copied();
                let prior_depth = parent
                    .and_then(|parent| self.exception_record(parent))
                    .map(|parent| parent.depth)
                    .or(self.exception_base);
                if prior_depth.and_then(|depth| depth.checked_add(1)) != Some(depth) {
                    return self.invalidate(Invalidation::Lifecycle);
                }
                let Some(occurrence) = u64::try_from(self.exception_records.len())
                    .ok()
                    .and_then(|n| n.checked_add(1))
                else {
                    return self.invalidate(Invalidation::Exhausted);
                };
                let id = ExceptionContextId {
                    capture: self.capture,
                    index: self.exception_records.len(),
                };
                self.exception_records.push(ExceptionContextRecord {
                    vector,
                    handler,
                    depth,
                    occurrence,
                    parent,
                    call_parent: self.active.last().map(|active| active.id),
                });
                self.exception_active.push(id);
                InvocationUpdate {
                    exception_entered: Some(id),
                    invalidated: Some(Invalidation::ExceptionOrBank),
                    ..Default::default()
                }
            }
            m68000_emu::ExceptionBoundary::Returned { depth } => {
                let Some(id) = self.exception_active.last().copied().filter(|id| {
                    self.exception_record(*id)
                        .is_some_and(|record| record.depth == depth)
                }) else {
                    return self.invalidate(Invalidation::Lifecycle);
                };
                self.exception_active.pop();
                InvocationUpdate {
                    exception_returned: Some(id),
                    invalidated: Some(Invalidation::ExceptionOrBank),
                    ..Default::default()
                }
            }
        }
    }
    fn next_child(&mut self) -> Option<(Option<InvocationId>, u32, u64, bool)> {
        self.next_record_serial = self.next_record_serial.checked_add(1)?;
        if let Some(parent) = self.active.last_mut() {
            parent.permitted = false;
            parent.child_occurrences = parent.child_occurrences.checked_add(1)?;
            let parent_id = parent.id;
            let occurrence = parent.child_occurrences;
            let record = self.record(parent_id)?;
            Some((
                Some(parent_id),
                record.invocation_depth.checked_add(1)?,
                occurrence,
                record.lineage_known && self.lineage_intact,
            ))
        } else {
            self.orphan_occurrences = self.orphan_occurrences.checked_add(1)?;
            Some((None, 0, self.orphan_occurrences, false))
        }
    }
    fn owner_at(&self, pc: u32) -> Option<FunctionId> {
        if let Some((owner, _)) = self.root.filter(|(_, root_pc)| *root_pc == pc) {
            return Some(owner);
        }
        catalogued_owner(self.revision, pc)
    }
    /// Measurement-only post-instruction path. It cannot mint stack-read witnesses
    /// because it has no pre-instruction scope or source-store evidence.
    pub(crate) fn observe_unrooted_boundary(
        &mut self,
        instruction: HybridExecutedInstruction,
        after: InvocationPoint,
        return_pc: Option<u32>,
    ) -> InvocationUpdate {
        if self.capture == 0 {
            return self.invalidate(Invalidation::Exhausted);
        }
        if self.root.is_some() || self.retain_history {
            return self.invalidate(Invalidation::Lifecycle);
        }
        if successful_call_instruction(instruction) {
            let Some((parent, depth, occurrence, _)) = self.next_child() else {
                return self.invalidate(Invalidation::Exhausted);
            };
            let owner = self.owner_at(after.pc);
            let id = self.push(
                InvocationRecord {
                    serial: 0,
                    owner,
                    entry_pc: after.pc,
                    entry_point: after,
                    entry_sp: Some(after.sp),
                    return_pc,
                    return_site: None,
                    caller_pc: Some(instruction.pc),
                    opcode: Some(instruction.opcode),
                    supervisor: after.supervisor(),
                    exception_depth: after.exception_depth,
                    invocation_depth: depth,
                    parent,
                    occurrence,
                    kind: FrameKind::Actual,
                    lineage_known: false,
                    materialization: None,
                    exception_context: self.exception_active.last().copied(),
                },
                false,
            );
            return InvocationUpdate {
                entered: Some(id),
                ..Default::default()
            };
        }
        let records = &self.records;
        let capture = self.capture;
        if let Some(active) = self.active.pop_if(|active| {
            active.id.capture == capture
                && records
                    .get(active.id.index)
                    .is_some_and(|record| record.serial == active.id.serial)
                && records
                    .get(active.id.index)
                    .and_then(|record| record.return_point())
                    .is_some_and(|expected| {
                        matches_successful_rts(instruction, after.returned(), expected)
                    })
        }) {
            let id = active.id;
            return InvocationUpdate {
                returned: Some(id),
                ..Default::default()
            };
        }
        InvocationUpdate::default()
    }
    pub(crate) fn finish_actual_step(
        &mut self,
        instruction: Option<HybridExecutedInstruction>,
        after: InvocationPoint,
        return_pc: Option<u32>,
    ) -> InvocationUpdate {
        if self.capture == 0 {
            return self.invalidate(Invalidation::Exhausted);
        }
        let Some(step) = self.pending.take() else {
            return self.invalidate(Invalidation::MissingReceipt);
        };
        let context_changed = after.supervisor() != step.before.supervisor()
            || after.exception_depth != step.before.exception_depth;
        if context_changed {
            self.invalidate(Invalidation::ExceptionOrBank);
        }
        if !step.ready && instruction.is_none() {
            return InvocationUpdate::default();
        }
        let Some(instruction) = instruction else {
            return self.invalidate(Invalidation::MissingReceipt);
        };
        let same_instruction = instruction.pc == step.before.pc
            && instruction.opcode == step.opcode
            && instruction.exception_depth == step.before.exception_depth;
        if !same_instruction && !context_changed {
            return self.invalidate(Invalidation::MissingReceipt);
        }
        if !instruction.completed_successfully {
            return self.invalidate(Invalidation::MissingReceipt);
        }
        if successful_call_instruction(instruction) {
            let same_frame = after.sp.wrapping_add(4) == step.before.sp;
            if after.sp & 1 != 0
                || (!same_frame && !context_changed)
                || return_pc.is_none()
                || instruction.exception_depth != after.exception_depth
            {
                return self.invalidate(Invalidation::MissingFrame);
            }
            let Some((parent, depth, occurrence, parent_known)) = self.next_child() else {
                return self.invalidate(Invalidation::Exhausted);
            };
            let physical_sp = after.sp & 0xffffff;
            let issued_return_site = match step.last_writes {
                [Some((high_address, high)), Some((low_address, low))]
                    if high_address == physical_sp
                        && low_address == (physical_sp.wrapping_add(2) & 0xffffff) =>
                {
                    Some((u32::from(high) << 16) | u32::from(low))
                }
                _ => None,
            }
            .filter(|site| Some(*site) == return_pc);
            let owner = self.owner_at(after.pc);
            let known = parent_known
                && same_instruction
                && same_frame
                && !context_changed
                && owner.is_some()
                && issued_return_site.is_some()
                && issued_return_site == return_pc;
            let id = self.push(
                InvocationRecord {
                    serial: 0,
                    owner,
                    entry_pc: after.pc,
                    entry_point: after,
                    entry_sp: Some(after.sp),
                    return_pc,
                    return_site: issued_return_site,
                    caller_pc: Some(instruction.pc),
                    opcode: Some(instruction.opcode),
                    supervisor: after.supervisor(),
                    exception_depth: after.exception_depth,
                    invocation_depth: depth,
                    parent,
                    occurrence,
                    kind: FrameKind::Actual,
                    lineage_known: known,
                    materialization: None,
                    exception_context: self.exception_active.last().copied(),
                },
                known,
            );
            return InvocationUpdate {
                entered: Some(id),
                invalidated: if owner.is_none() {
                    Some(Invalidation::MissingOwner)
                } else {
                    None
                },
                ..Default::default()
            };
        }
        if instruction.opcode == 0x4e75 {
            let Some(active) = self.active.last() else {
                return InvocationUpdate::default();
            };
            let record = self.record(active.id).expect("active invocation");
            if record.kind == FrameKind::Elided && self.prepared {
                // The existing original-fallback runner owns its opaque return;
                // only the sealed framework end may close this unpermitted frame.
                return self.revoke_current(Invalidation::Return);
            }
            if !record.return_point().is_some_and(|expected| {
                matches_successful_rts(instruction, after.returned(), expected)
            }) {
                return self.invalidate(Invalidation::MissingFrame);
            }
            let id = active.id;
            if record.kind == FrameKind::Prepared && self.prepared {
                self.active
                    .last_mut()
                    .expect("active prepared invocation")
                    .physical_returned = true;
            } else {
                self.active.pop();
            }
            return InvocationUpdate {
                returned: Some(id),
                ..Default::default()
            };
        }
        InvocationUpdate::default()
    }
    pub(crate) fn observe_framework(
        &mut self,
        event: CallBoundaryEvent,
        actual: InvocationPoint,
        current_frame: FrameState,
        return_pc: Option<u32>,
        materialization: Option<tje_game::stack_read::MaterializedFrame>,
    ) -> InvocationUpdate {
        if self.capture == 0 {
            return self.invalidate(Invalidation::Exhausted);
        }
        if !self.prepared {
            return self.invalidate(Invalidation::Lifecycle);
        }
        self.observe_point(actual);
        match event.kind() {
            CallBoundaryKind::PreparedEnter => {
                let (Some(parent_frame), Some(entry), Some(site)) =
                    (event.parent(), event.entry(), event.site())
                else {
                    return self.invalidate(Invalidation::MissingFrame);
                };
                let parent_matches = self
                    .current()
                    .is_some_and(|parent| frame_matches(parent, parent_frame));
                let valid = parent_matches
                    && current_frame == entry
                    && entry.is_prepared_for(event.owner())
                    && actual.sp == entry.entry_sp
                    && actual.sp & 1 == 0
                    && return_pc == Some(site.return_pc);
                let Some((parent, depth, occurrence, parent_known)) = self.next_child() else {
                    return self.invalidate(Invalidation::Exhausted);
                };
                let id = self.push(
                    InvocationRecord {
                        serial: 0,
                        owner: Some(event.owner()),
                        entry_pc: event.owner().metadata().address(self.revision),
                        entry_point: actual,
                        entry_sp: Some(entry.entry_sp),
                        return_pc: Some(site.return_pc),
                        return_site: Some(site.return_pc),
                        caller_pc: None,
                        opcode: None,
                        supervisor: actual.supervisor(),
                        exception_depth: actual.exception_depth,
                        invocation_depth: depth,
                        parent,
                        occurrence,
                        kind: FrameKind::Prepared,
                        lineage_known: valid && parent_known,
                        materialization,
                        exception_context: self.exception_active.last().copied(),
                    },
                    valid && parent_known,
                );
                InvocationUpdate {
                    entered: Some(id),
                    invalidated: if valid {
                        None
                    } else {
                        Some(Invalidation::MissingFrame)
                    },
                    ..Default::default()
                }
            }
            CallBoundaryKind::ElidedEnter => {
                let Some((parent, depth, occurrence, _)) = self.next_child() else {
                    return self.invalidate(Invalidation::Exhausted);
                };
                let id = self.push(
                    InvocationRecord {
                        serial: 0,
                        owner: Some(event.owner()),
                        entry_pc: event.owner().metadata().address(self.revision),
                        entry_point: actual,
                        entry_sp: None,
                        return_pc: None,
                        return_site: None,
                        caller_pc: None,
                        opcode: None,
                        supervisor: actual.supervisor(),
                        exception_depth: actual.exception_depth,
                        invocation_depth: depth,
                        parent,
                        occurrence,
                        kind: FrameKind::Elided,
                        lineage_known: false,
                        materialization: None,
                        exception_context: self.exception_active.last().copied(),
                    },
                    false,
                );
                self.active.last_mut().expect("elided invocation").elided_sp = Some(actual.sp);
                InvocationUpdate {
                    entered: Some(id),
                    ..Default::default()
                }
            }
            CallBoundaryKind::Returned | CallBoundaryKind::Abandoned => {
                let Some(active) = self.active.last() else {
                    return self.invalidate(Invalidation::Lifecycle);
                };
                let record = self.record(active.id).expect("active invocation");
                if event.kind() == CallBoundaryKind::Abandoned {
                    return self.invalidate(Invalidation::Lifecycle);
                }
                let valid = record.owner == Some(event.owner())
                    && match record.kind {
                        FrameKind::Prepared => {
                            event
                                .entry()
                                .is_some_and(|entry| record.entry_sp == Some(entry.entry_sp))
                                && event
                                    .site()
                                    .is_some_and(|site| record.return_pc == Some(site.return_pc))
                                && event.entry() == Some(current_frame)
                                && record
                                    .parent
                                    .and_then(|parent| self.record(parent))
                                    .zip(event.parent())
                                    .is_some_and(|(parent, frame)| frame_matches(parent, frame))
                                && record
                                    .entry_sp
                                    .is_some_and(|sp| actual.sp == sp.wrapping_add(4))
                        }
                        FrameKind::Elided => {
                            event.entry().is_none() && active.elided_sp == Some(actual.sp)
                        }
                        FrameKind::Root | FrameKind::Actual => false,
                    };
                if !valid {
                    return self.invalidate(Invalidation::Lifecycle);
                }
                let completed = self.active.pop().expect("matching framework return");
                InvocationUpdate {
                    returned: (!completed.physical_returned).then_some(completed.id),
                    ..Default::default()
                }
            }
        }
    }
    pub(crate) fn witnesses_match(
        &self,
        left: InvocationWitness,
        other: &Self,
        right: InvocationWitness,
    ) -> bool {
        let (Some(mut a), Some(mut b)) = (self.resolve(left), other.resolve(right)) else {
            return false;
        };
        loop {
            if a.exception_context.is_some() || b.exception_context.is_some() {
                return false;
            }
            if a.owner.is_none()
                || a.owner != b.owner
                || a.entry_pc != b.entry_pc
                || a.entry_sp != b.entry_sp
                || a.return_pc != b.return_pc
                || a.return_site != b.return_site
                || a.supervisor != b.supervisor
                || a.exception_depth != b.exception_depth
                || a.invocation_depth != b.invocation_depth
                || a.occurrence != b.occurrence
                || !a.lineage_known
                || !b.lineage_known
            {
                return false;
            }
            match (a.parent, b.parent) {
                (None, None) => return true,
                (Some(parent_a), Some(parent_b)) => {
                    let (Some(next_a), Some(next_b)) =
                        (self.record(parent_a), other.record(parent_b))
                    else {
                        return false;
                    };
                    a = next_a;
                    b = next_b;
                }
                _ => return false,
            }
        }
    }
}
fn attempted_call(opcode: u16) -> bool {
    opcode & 0xffc0 == 0x4e80 || opcode & 0xff00 == 0x6100
}
fn frame_matches(record: &InvocationRecord, frame: FrameState) -> bool {
    record.entry_sp == Some(frame.entry_sp)
        && record.owner.is_some_and(|owner| {
            frame.is_prepared_for(owner)
                || (record.parent.is_none() && frame == FrameState::new(frame.entry_sp))
        })
}
fn nonlocal_opcode(opcode: u16) -> bool {
    opcode & 0xffc0 == 0x4ec0
        || opcode & 0xfff0 == 0x4e40
        || matches!(opcode, 0x4e70 | 0x4e72..=0x4e74 | 0x4e76 | 0x4e77 | 0x4afc)
        || matches!(opcode & 0xf000, 0xa000 | 0xf000)
}
fn catalogued_owner(revision: Revision, pc: u32) -> Option<FunctionId> {
    static ENTRIES: LazyLock<[Vec<(u32, FunctionId)>; 2]> = LazyLock::new(|| {
        [Revision::Rev02, Revision::Rev00].map(|revision| {
            let mut entries: Vec<_> = tje_game::functions()
                .map(|function| (function.address(revision), function.id))
                .collect();
            entries.sort_unstable_by_key(|(pc, _)| *pc);
            entries
        })
    });
    let entries = &ENTRIES[match revision {
        Revision::Rev02 => 0,
        Revision::Rev00 => 1,
    }];
    let first = entries.partition_point(|(entry, _)| *entry < pc);
    let end = entries.partition_point(|(entry, _)| *entry <= pc);
    (end == first + 1).then(|| entries[first].1)
}

#[cfg(test)]
mod tests;
