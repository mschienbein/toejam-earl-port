//! Backend-owned readable root state and retained actual-transfer leases.
use super::{Reader, UninitializedLocalRead, Unsupported, UnsupportedReason};
use crate::{
    FunctionId,
    call_frame::{CallBoundaryEvent, CallBoundaryKind, FrameState},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
use tje_mem::written_ram::{WrittenRam, canonical_ram_byte};

/// Reviewed controller resource ceiling, NOT a hardware/deferred-queue bound.
pub const GUARD_CAPACITY: usize = 12;
static NEXT_GUARD: AtomicU64 = AtomicU64::new(1);

/// Backend allocation evidence, minted only from an actual sealed framework
/// entry. It is neither an operand nor permission to read a returned frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterializedFrame {
    guard: u64,
    epoch: u64,
    serial: u64,
    parent_serial: u64,
    owner: FunctionId,
    frame: FrameState,
    supervisor: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DmaMetrics {
    pub opened: u64,
    pub completed: u64,
    pub cancelled: u64,
    pub reset_invalidated: u64,
    pub discarded: u64,
    pub failed: u64,
    /// Cumulative distinct (Guard identity, epoch) scopes that encountered
    /// unknown-origin proof debt; retirement/reset does not erase the count.
    pub unknown_origin_proof_debt: u64,
    pub pending_proof: u64,
    pub capacity_hits: u64,
    pub active: u64,
    pub queued: u64,
    pub queued_consumed: u64,
    pub queued_discarded: u64,
}
#[derive(Default)]
struct MetricCounters {
    opened: AtomicU64,
    completed: AtomicU64,
    cancelled: AtomicU64,
    reset_invalidated: AtomicU64,
    discarded: AtomicU64,
    failed: AtomicU64,
    unknown_origin_proof_debt: AtomicU64,
    pending_proof: AtomicU64,
    capacity_hits: AtomicU64,
    active: AtomicU64,
    queued: AtomicU64,
    queued_consumed: AtomicU64,
    queued_discarded: AtomicU64,
}
#[derive(Default)]
pub struct Namespace {
    first: Mutex<Option<Unsupported>>,
    failed: AtomicBool,
    metrics: MetricCounters,
}
impl Namespace {
    pub fn failure(&self) -> Option<Unsupported> {
        if !self.failed.load(Ordering::Relaxed) {
            return None;
        }
        *self.first.lock().expect("readable namespace lock poisoned")
    }
    fn latch(&self, error: Unsupported) -> Unsupported {
        let first = *self
            .first
            .lock()
            .expect("readable namespace lock poisoned")
            .get_or_insert(error);
        self.failed.store(true, Ordering::Relaxed);
        first
    }
    /// Only a backend starting a genuinely new discarded analysis trajectory
    /// or completing an actual reset may clear its execution-failure latch.
    pub fn reset_failure(&self) {
        *self.first.lock().expect("readable namespace lock poisoned") = None;
        self.failed.store(false, Ordering::Relaxed);
    }
    /// Ordinary original ticks need no unwind boundary without a retained
    /// owned transfer or a previously latched execution failure.
    pub fn needs_boundary(&self) -> bool {
        self.failed.load(Ordering::Relaxed) || self.has_retained_dma()
    }
    pub fn has_retained_dma(&self) -> bool {
        self.metrics.active.load(Ordering::Relaxed) != 0
            || self.metrics.queued.load(Ordering::Relaxed) != 0
    }
    pub fn capacity_failure(&self, root: FunctionId, entry_sp: u32) -> Unsupported {
        self.metrics.capacity_hits.fetch_add(1, Ordering::Relaxed);
        self.latch(Unsupported::capability(
            root,
            entry_sp,
            UnsupportedReason::GuardCapacityExhausted,
        ))
    }
    pub fn metrics(&self) -> DmaMetrics {
        let m = &self.metrics;
        DmaMetrics {
            opened: m.opened.load(Ordering::Relaxed),
            completed: m.completed.load(Ordering::Relaxed),
            cancelled: m.cancelled.load(Ordering::Relaxed),
            reset_invalidated: m.reset_invalidated.load(Ordering::Relaxed),
            discarded: m.discarded.load(Ordering::Relaxed),
            failed: m.failed.load(Ordering::Relaxed),
            unknown_origin_proof_debt: m.unknown_origin_proof_debt.load(Ordering::Relaxed),
            pending_proof: m.pending_proof.load(Ordering::Relaxed),
            capacity_hits: m.capacity_hits.load(Ordering::Relaxed),
            active: m.active.load(Ordering::Relaxed),
            queued: m.queued.load(Ordering::Relaxed),
            queued_consumed: m.queued_consumed.load(Ordering::Relaxed),
            queued_discarded: m.queued_discarded.load(Ordering::Relaxed),
        }
    }
    /// Convert only an exact failed scope from THIS physical backend. A late
    /// transfer may name an older root; no frame is reopened or reconstructed.
    pub fn catch<T>(self: &Arc<Self>, operation: impl FnOnce() -> T) -> Result<T, Unsupported> {
        if let Some(error) = self.failure() {
            return Err(error);
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
            Ok(value) => self.failure().map_or(Ok(value), Err),
            Err(payload) => {
                let sealed = payload.downcast_ref::<Abort>().is_some_and(|abort| {
                    Arc::ptr_eq(&abort.scope.namespace, self)
                        && abort.scope.epoch.load(Ordering::Relaxed) == abort.epoch
                        && abort.scope.failed_epoch.load(Ordering::Relaxed) == abort.epoch
                        && *abort
                            .scope
                            .error
                            .lock()
                            .expect("readable scope lock poisoned")
                            == Some(abort.error)
                });
                if !sealed {
                    std::panic::resume_unwind(payload)
                }
                Err(payload
                    .downcast_ref::<Abort>()
                    .expect("sealed readable abort")
                    .error)
            }
        }
    }
}

struct ScopeSeal {
    namespace: Arc<Namespace>,
    identity: u64,
    epoch: AtomicU64,
    failed_epoch: AtomicU64,
    unknown_debt_epoch: AtomicU64,
    error: Mutex<Option<Unsupported>>,
}
struct Abort {
    scope: Arc<ScopeSeal>,
    epoch: u64,
    error: Unsupported,
}
#[derive(Default)]
struct State {
    root: Option<FunctionId>,
    entry_sp: u32,
    raw_entry_sp: u32,
    root_supervisor: bool,
    root_read_permitted: bool,
    root_materialized: bool,
    allocation: Option<MaterializedFrame>,
    allocation_read_permitted: bool,
    next_allocation: u64,
    expected_restore: Option<u64>,
    elided_depth: usize,
    elided_parent: Option<u64>,
    written: WrittenRam,
    non_own: WrittenRam,
    original_depth: usize,
    queued: usize,
    active_dma: Option<u64>,
    uncertain_dma: bool,
}

pub struct Guard {
    executing_sp: AtomicU32,
    enforce_reads: AtomicBool,
    state: Mutex<State>,
    scope: Arc<ScopeSeal>,
}
impl Default for Guard {
    fn default() -> Self {
        Self::with_namespace(Arc::new(Namespace::default()))
    }
}
impl Guard {
    pub fn with_namespace(namespace: Arc<Namespace>) -> Self {
        Self {
            executing_sp: AtomicU32::new(0),
            enforce_reads: AtomicBool::new(true),
            state: Mutex::new(State::default()),
            scope: Arc::new(ScopeSeal {
                namespace,
                identity: NEXT_GUARD
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                    .unwrap_or(0),
                epoch: AtomicU64::new(0),
                failed_epoch: AtomicU64::new(0),
                unknown_debt_epoch: AtomicU64::new(0),
                error: Mutex::new(None),
            }),
        }
    }
    pub fn active(&self) -> bool {
        self.executing_sp.load(Ordering::Relaxed) != 0
    }
    pub fn epoch(&self) -> u64 {
        self.scope.epoch.load(Ordering::Relaxed)
    }
    pub fn failure(&self) -> Option<Unsupported> {
        self.scope.namespace.failure()
    }
    fn error(&self, raw: u32, width: u8, reader: Reader, reason: UnsupportedReason) -> Unsupported {
        let state = self.state.lock().expect("readable stack lock poisoned");
        Unsupported::new(
            state.root.expect("readable scope owner"),
            state.entry_sp,
            raw,
            width,
            reader,
            reason,
        )
    }
    fn latch(&self, error: Unsupported) -> Unsupported {
        let epoch = self.epoch();
        let first = self.scope.namespace.latch(error);
        *self
            .scope
            .error
            .lock()
            .expect("readable scope lock poisoned") = Some(first);
        self.scope.failed_epoch.store(epoch, Ordering::Relaxed);
        first
    }
    fn record_unknown_origin_debt(&self) {
        let epoch = self.epoch();
        if self.scope.unknown_debt_epoch.swap(epoch, Ordering::Relaxed) != epoch {
            self.scope
                .namespace
                .metrics
                .unknown_origin_proof_debt
                .fetch_add(1, Ordering::Relaxed);
        }
    }
    fn abort(&self, error: Unsupported) -> ! {
        let error = self.latch(error);
        std::panic::resume_unwind(Box::new(Abort {
            scope: Arc::clone(&self.scope),
            epoch: self.epoch(),
            error,
        }))
    }
    pub fn assert_no_failure(&self) {
        if self.active()
            && let Some(error) = self.failure()
        {
            self.abort(error)
        }
    }
    pub fn catch_abort<T>(&self, operation: impl FnOnce() -> T) -> Result<T, Unsupported> {
        self.scope.namespace.catch(operation)
    }
    pub fn run_root<T>(
        &self,
        root: FunctionId,
        entry_sp: u32,
        supervisor: bool,
        operation: impl FnOnce() -> T,
    ) -> Result<T, Unsupported> {
        self.run_policy(root, entry_sp, supervisor, true, operation)
    }
    pub fn track_root<T>(
        &self,
        root: FunctionId,
        entry_sp: u32,
        supervisor: bool,
        operation: impl FnOnce() -> T,
    ) -> Result<T, Unsupported> {
        self.run_policy(root, entry_sp, supervisor, false, operation)
    }
    fn run_policy<T>(
        &self,
        root: FunctionId,
        entry_sp: u32,
        supervisor: bool,
        enforce: bool,
        operation: impl FnOnce() -> T,
    ) -> Result<T, Unsupported> {
        if self.active() {
            return Ok(operation());
        }
        if let Some(error) = self.failure() {
            return Err(error);
        }
        if !cfg!(panic = "unwind") {
            return Err(Unsupported::capability(
                root,
                entry_sp,
                UnsupportedReason::UnwindUnavailable,
            ));
        }
        let Some(high) = canonical_ram_byte(entry_sp) else {
            return Err(Unsupported::capability(
                root,
                entry_sp,
                UnsupportedReason::InvalidRootStack,
            ));
        };
        let Some(epoch) = self
            .epoch()
            .checked_add(1)
            .filter(|_| self.scope.identity != 0)
        else {
            return Err(Unsupported::capability(
                root,
                entry_sp,
                UnsupportedReason::ScopeExhausted,
            ));
        };
        {
            let mut state = self.state.lock().expect("readable stack lock poisoned");
            // A cache may NEVER reset a retained queued/active transfer's data.
            if state.queued != 0 || state.active_dma.is_some() {
                return Err(Unsupported::capability(
                    root,
                    entry_sp,
                    UnsupportedReason::GuardCapacityExhausted,
                ));
            }
            state.written.reset();
            state.non_own.reset();
            state.root = Some(root);
            state.entry_sp = high;
            state.raw_entry_sp = entry_sp;
            state.root_supervisor = supervisor;
            state.root_read_permitted = true;
            state.root_materialized = true;
            state.allocation = None;
            state.allocation_read_permitted = false;
            state.next_allocation = 0;
            state.expected_restore = None;
            state.elided_depth = 0;
            state.elided_parent = None;
            state.original_depth = 0;
            state.uncertain_dma = false;
        }
        self.scope.epoch.store(epoch, Ordering::Relaxed);
        self.scope.failed_epoch.store(0, Ordering::Relaxed);
        *self
            .scope
            .error
            .lock()
            .expect("readable scope lock poisoned") = None;
        self.enforce_reads.store(enforce, Ordering::Relaxed);
        self.executing_sp.store(high, Ordering::Relaxed);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.scope.namespace.catch(operation)
        }));
        self.executing_sp.store(0, Ordering::Relaxed);
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
    fn read_window(&self, raw: u32, width: u8, reader: Reader, high: u32) -> Option<Unsupported> {
        let state = self.state.lock().expect("readable stack lock poisoned");
        (0..u32::from(width))
            .filter_map(|lane| canonical_ram_byte(raw.wrapping_add(lane)))
            .any(|address| address < high && !state.written.get(address))
            .then(|| {
                Unsupported::new(
                    state.root.expect("readable root"),
                    state.entry_sp,
                    raw,
                    width,
                    reader,
                    UnsupportedReason::Unwritten,
                )
            })
    }
    pub fn read(&self, raw: u32, width: u8, reader: Reader) {
        self.read_at(raw, raw, width, reader);
    }
    fn read_at(&self, raw: u32, actual: u32, width: u8, reader: Reader) {
        self.assert_no_failure();
        if !self.enforce_reads.load(Ordering::Relaxed) {
            return;
        }
        let high = self.executing_sp.load(Ordering::Relaxed);
        if high == 0 {
            return;
        }
        if !(0..u32::from(width))
            .filter_map(|lane| canonical_ram_byte(actual.wrapping_add(lane)))
            .any(|address| address < high)
        {
            return;
        }
        if let Some(mut error) = self.read_window(actual, width, reader, high) {
            error.raw_address = raw & tje_mem::ADDRESS_MASK;
            self.abort(error)
        }
    }
    pub fn semantic_ram_read(&self, raw: u32, width: u8) {
        let original = self
            .state
            .lock()
            .expect("readable stack lock poisoned")
            .original_depth
            != 0;
        if !original {
            self.read_at(
                raw,
                if width == 2 { raw & !1 } else { raw },
                width,
                Reader::HardwareRam,
            )
        }
    }
    pub fn commit(&self, raw: u32, width: u8, rust_owned: bool) {
        let high = self.executing_sp.load(Ordering::Relaxed);
        if high == 0 {
            return;
        }
        let mut state = self.state.lock().expect("readable stack lock poisoned");
        let own = rust_owned && state.original_depth == 0;
        for address in (0..u32::from(width))
            .filter_map(|lane| canonical_ram_byte(raw.wrapping_add(lane)))
            .filter(|&address| address < high)
        {
            if own {
                state.written.set(address)
            } else {
                state.non_own.set(address)
            }
        }
    }
    pub fn original_segment(&self) -> OriginalSegment<'_> {
        let mut state = self.state.lock().expect("readable stack lock poisoned");
        state.original_depth += 1;
        state.root_read_permitted = false;
        state.allocation_read_permitted = false;
        drop(state);
        OriginalSegment { guard: self }
    }
    /// True only at an actual issuing Rust command, never at eventual DMA time.
    pub fn owns_dma_command(&self, foreign_actor: bool, unknown_actor: bool) -> Option<bool> {
        if !self.active() || !self.enforce_reads.load(Ordering::Relaxed) {
            return None;
        }
        let original = self
            .state
            .lock()
            .expect("readable stack lock poisoned")
            .original_depth
            != 0;
        if original || foreign_actor {
            None
        } else {
            Some(!unknown_actor)
        }
    }
    fn check_epoch(&self, epoch: u64) {
        if self.epoch() != epoch {
            self.abort(self.error(0, 0, Reader::DmaBoundary, UnsupportedReason::StaleDmaLease))
        }
    }
    pub fn command_queued(&self, epoch: u64) {
        self.check_epoch(epoch);
        let mut state = self.state.lock().expect("readable stack lock poisoned");
        state.queued = state
            .queued
            .checked_add(1)
            .expect("source command count fits addressable memory");
        self.scope
            .namespace
            .metrics
            .queued
            .fetch_add(1, Ordering::Relaxed);
    }
    pub fn command_retired(&self, epoch: u64, discarded: bool) {
        self.check_epoch(epoch);
        let mut state = self.state.lock().expect("readable stack lock poisoned");
        state.queued = state
            .queued
            .checked_sub(1)
            .expect("actual queued token retired once");
        let metrics = &self.scope.namespace.metrics;
        metrics.queued.fetch_sub(1, Ordering::Relaxed);
        let counter = if discarded {
            &metrics.queued_discarded
        } else {
            &metrics.queued_consumed
        };
        counter.fetch_add(1, Ordering::Relaxed);
        drop(state);
        if discarded {
            self.latch(self.error(
                0,
                0,
                Reader::DmaBoundary,
                UnsupportedReason::DiscardedOwnedDma,
            ));
        }
    }
    pub fn dma_started(&self, epoch: u64, generation: u64, uncertain: bool) {
        self.check_epoch(epoch);
        let mut state = self.state.lock().expect("readable stack lock poisoned");
        if state.active_dma.is_some() {
            drop(state);
            self.abort(self.error(0, 0, Reader::DmaBoundary, UnsupportedReason::StaleDmaLease))
        }
        state.active_dma = Some(generation);
        state.uncertain_dma = uncertain || generation == u64::MAX;
        self.scope
            .namespace
            .metrics
            .opened
            .fetch_add(1, Ordering::Relaxed);
        self.scope
            .namespace
            .metrics
            .active
            .fetch_add(1, Ordering::Relaxed);
        let unknown = state.uncertain_dma;
        drop(state);
        if unknown {
            self.scope
                .namespace
                .metrics
                .failed
                .fetch_add(1, Ordering::Relaxed);
            // Let origin.start finish constructing the physical lease. The
            // boundary reports this debt even if fill/copy never reads RAM.
            self.record_unknown_origin_debt();
            self.latch(self.error(
                0,
                0,
                Reader::DmaBoundary,
                UnsupportedReason::UnknownDmaOrigin,
            ));
        }
    }
    pub fn dma_read(&self, epoch: u64, generation: u64, raw: u32, uncertain: bool) {
        self.check_epoch(epoch);
        if let Some(error) = self.failure() {
            self.abort(error)
        }
        let (high, valid, unknown) = {
            let state = self.state.lock().expect("readable stack lock poisoned");
            (
                state.entry_sp,
                state.active_dma == Some(generation),
                state.uncertain_dma || uncertain,
            )
        };
        if !valid || unknown {
            self.scope
                .namespace
                .metrics
                .failed
                .fetch_add(1, Ordering::Relaxed);
            self.record_unknown_origin_debt();
            self.abort(self.error(raw, 2, Reader::VdpDma, UnsupportedReason::UnknownDmaOrigin));
        }
        if let Some(error) = self.read_window(raw & !1, 2, Reader::VdpDma, high) {
            self.scope
                .namespace
                .metrics
                .failed
                .fetch_add(1, Ordering::Relaxed);
            self.abort(error);
        }
    }
    pub fn dma_retired(&self, epoch: u64, generation: u64, end: DmaRetirement) {
        self.check_epoch(epoch);
        let mut state = self.state.lock().expect("readable stack lock poisoned");
        if state.active_dma != Some(generation) {
            drop(state);
            self.abort(self.error(0, 0, Reader::DmaBoundary, UnsupportedReason::StaleDmaLease))
        }
        state.active_dma = None;
        let m = &self.scope.namespace.metrics;
        m.active.fetch_sub(1, Ordering::Relaxed);
        match end {
            DmaRetirement::Completed => &m.completed,
            DmaRetirement::Cancelled => &m.cancelled,
            DmaRetirement::ResetInvalidated => &m.reset_invalidated,
            DmaRetirement::Discarded => &m.discarded,
        }
        .fetch_add(1, Ordering::Relaxed);
        drop(state);
        if end == DmaRetirement::Discarded {
            // Drop cannot unwind. Losing a physical issuer is unresolved
            // proof debt even though the physical lease no longer exists.
            self.latch(self.error(
                0,
                0,
                Reader::DmaBoundary,
                UnsupportedReason::DiscardedOwnedDma,
            ));
        }
    }
    pub fn missing_dma_origin(&self) -> ! {
        self.record_unknown_origin_debt();
        self.abort(self.error(
            0,
            0,
            Reader::DmaBoundary,
            UnsupportedReason::UnknownDmaOrigin,
        ))
    }
    pub fn finish_proof(&self) -> Result<(), Unsupported> {
        if let Some(error) = self.failure() {
            return Err(error);
        }
        let open = {
            let state = self.state.lock().expect("readable stack lock poisoned");
            state.queued != 0 || state.active_dma.is_some()
        };
        if open {
            self.scope
                .namespace
                .metrics
                .pending_proof
                .fetch_add(1, Ordering::Relaxed);
            Err(self.latch(self.error(
                0,
                0,
                Reader::DmaBoundary,
                UnsupportedReason::PendingOwnedDma,
            )))
        } else {
            Ok(())
        }
    }
    pub fn unwritten_mask(&self, raw: u32, width: u8) -> u8 {
        let state = self.state.lock().expect("readable stack lock poisoned");
        (0..u32::from(width)).fold(0, |mask, lane| {
            let missing = canonical_ram_byte(raw.wrapping_add(lane))
                .is_some_and(|a| a < state.entry_sp && !state.written.get(a));
            mask | (u8::from(missing) << lane)
        })
    }
    /// Preserve allocation metadata while irrevocably revoking the current
    /// frame's read permission, including accepted IRQs and stack-bank changes.
    pub fn revoke_read_permissions(&self) {
        let mut state = self.state.lock().expect("readable stack lock poisoned");
        state.root_read_permitted = false;
        state.allocation_read_permitted = false;
    }
    /// Observe real framework materialization without maintaining another call
    /// stack. Parent restoration requires the shared recorder's opaque token.
    pub fn observe_call_boundary(
        &self,
        event: &CallBoundaryEvent,
        current: FrameState,
        actual_sp: u32,
        supervisor: bool,
    ) -> Option<MaterializedFrame> {
        if !self.active() {
            return None;
        }
        let mut state = self.state.lock().expect("readable stack lock poisoned");
        let root_frame = |frame: FrameState| {
            frame.entry_sp == state.raw_entry_sp
                && state.root_supervisor == supervisor
                && state.root_materialized
                && state.allocation.is_none()
                && (frame == FrameState::new(state.raw_entry_sp)
                    || state.root.is_some_and(|root| frame.is_prepared_for(root)))
        };
        let parent_serial = event.parent().and_then(|parent| {
            if root_frame(parent) {
                Some(0)
            } else {
                state
                    .allocation
                    .as_ref()
                    .filter(|allocation| {
                        allocation.frame == parent && allocation.supervisor == supervisor
                    })
                    .map(|allocation| allocation.serial)
            }
        });
        match event.kind() {
            CallBoundaryKind::PreparedEnter => {
                let valid = state.original_depth == 0
                    && state.elided_depth == 0
                    && parent_serial.is_some()
                    && event.entry().is_some_and(|entry| {
                        entry == current
                            && entry.is_prepared_for(event.owner())
                            && entry.entry_sp == actual_sp
                            && entry.entry_sp & 1 == 0
                            && canonical_ram_byte(entry.entry_sp)
                                .is_some_and(|sp| sp < state.entry_sp)
                    });
                state.root_read_permitted = false;
                state.root_materialized = false;
                state.allocation_read_permitted = false;
                state.expected_restore = None;
                if !valid {
                    state.allocation = None;
                    return None;
                }
                let Some(serial) = state.next_allocation.checked_add(1) else {
                    state.allocation = None;
                    drop(state);
                    self.latch(self.error(
                        actual_sp,
                        0,
                        Reader::RootEntry,
                        UnsupportedReason::ScopeExhausted,
                    ));
                    return None;
                };
                let token = MaterializedFrame {
                    guard: self.scope.identity,
                    epoch: self.epoch(),
                    serial,
                    parent_serial: parent_serial.expect("validated parent"),
                    owner: event.owner(),
                    frame: current,
                    supervisor,
                };
                state.next_allocation = serial;
                state.allocation = Some(token);
                state.allocation_read_permitted = true;
                Some(token)
            }
            CallBoundaryKind::ElidedEnter => {
                state.root_materialized = false;
                if state.elided_depth == 0 {
                    state.elided_parent = parent_serial
                }
                let Some(depth) = state.elided_depth.checked_add(1) else {
                    state.allocation = None;
                    state.allocation_read_permitted = false;
                    state.root_read_permitted = false;
                    drop(state);
                    self.latch(self.error(
                        actual_sp,
                        0,
                        Reader::RootEntry,
                        UnsupportedReason::ScopeExhausted,
                    ));
                    return None;
                };
                state.elided_depth = depth;
                state.root_read_permitted = false;
                state.allocation = None;
                state.allocation_read_permitted = false;
                state.expected_restore = None;
                None
            }
            CallBoundaryKind::Returned => {
                state.expected_restore = if event.entry().is_none() {
                    state.elided_depth = state.elided_depth.saturating_sub(1);
                    if state.elided_depth == 0 {
                        state.elided_parent.take()
                    } else {
                        None
                    }
                } else {
                    state
                        .allocation
                        .as_ref()
                        .filter(|allocation| {
                            allocation.owner == event.owner()
                                && Some(allocation.frame) == event.entry()
                                && allocation.frame == current
                                && allocation.supervisor == supervisor
                                && actual_sp == allocation.frame.entry_sp.wrapping_add(4)
                        })
                        .map(|allocation| allocation.parent_serial)
                };
                state.root_materialized = state.expected_restore == Some(0);
                state.allocation = None;
                state.allocation_read_permitted = false;
                None
            }
            CallBoundaryKind::Abandoned => {
                state.allocation = None;
                state.allocation_read_permitted = false;
                state.root_read_permitted = false;
                state.root_materialized = false;
                state.expected_restore = None;
                None
            }
        }
    }
    /// Restore only the recorded parent's materialization, never its revoked
    /// read permission. A token from another guard, epoch or sibling cannot do so.
    pub fn restore_materialization(
        &self,
        token: MaterializedFrame,
        current: FrameState,
        actual_sp: u32,
        supervisor: bool,
    ) -> bool {
        if !self.active() {
            return false;
        }
        let mut state = self.state.lock().expect("readable stack lock poisoned");
        let valid = token.guard == self.scope.identity
            && token.epoch == self.epoch()
            && state.expected_restore == Some(token.serial)
            && state.original_depth == 0
            && state.elided_depth == 0
            && token.frame == current
            && token.supervisor == supervisor
            && canonical_ram_byte(actual_sp)
                .zip(canonical_ram_byte(token.frame.entry_sp))
                .is_some_and(|(sp, entry)| sp <= entry && entry < state.entry_sp);
        if valid {
            state.expected_restore = None;
            state.allocation = Some(token);
            state.allocation_read_permitted = false;
        }
        valid
    }
    pub fn reject_explicit_local(
        &self,
        read: UninitializedLocalRead,
        reason: UnsupportedReason,
    ) -> Unsupported {
        self.latch(self.error(
            read.address(),
            read.width.bytes(),
            Reader::ExplicitLocal,
            reason,
        ))
    }
    pub fn explicit_local(
        &self,
        read: UninitializedLocalRead,
        current: FrameState,
        current_sp: u32,
        supervisor: bool,
    ) -> Result<u32, Unsupported> {
        if let Some(error) = self.failure() {
            return Err(error);
        }
        let state = self.state.lock().expect("readable stack lock poisoned");
        let root = state.root.unwrap_or(read.owner);
        let raw = read.address();
        let width = read.width.bytes();
        let address = canonical_ram_byte(raw);
        let entry = canonical_ram_byte(read.frame.entry_sp);
        let low = canonical_ram_byte(current_sp);
        let owner = (read.owner == root
            && read.frame.entry_sp == state.raw_entry_sp
            && state.root_supervisor == supervisor
            && state.root_read_permitted)
            || (state.allocation_read_permitted
                && state.allocation.as_ref().is_some_and(|allocation| {
                    allocation.owner == read.owner
                        && allocation.frame == current
                        && allocation.supervisor == supervisor
                }));
        let valid = self.active()
            && state.original_depth == 0
            && current == read.frame
            && owner
            && !read.citation.trim().is_empty()
            && read.offset < 0
            && read
                .offset
                .checked_add(i32::from(width))
                .is_some_and(|end| end <= 0)
            && (width == 1 || raw & 1 == 0)
            && address.zip(entry).zip(low).is_some_and(|((a, e), l)| {
                l <= a
                    && a.checked_add(u32::from(width))
                        .is_some_and(|end| end <= e && end <= state.entry_sp)
            });
        let reason = if !valid {
            Some(UnsupportedReason::InvalidExplicitLocal)
        } else if (0..u32::from(width)).all(|lane| state.written.get(raw.wrapping_add(lane))) {
            Some(UnsupportedReason::AlreadyWritten)
        } else if (0..u32::from(width)).any(|lane| {
            !state.written.get(raw.wrapping_add(lane)) && state.non_own.get(raw.wrapping_add(lane))
        }) {
            Some(UnsupportedReason::InterveningNonOwnWrite)
        } else {
            None
        };
        drop(state);
        match reason {
            Some(reason) => Err(self.latch(self.error(raw, width, Reader::ExplicitLocal, reason))),
            None => Ok(raw),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaRetirement {
    Completed,
    Cancelled,
    ResetInvalidated,
    Discarded,
}
pub struct OriginalSegment<'a> {
    guard: &'a Guard,
}
impl Drop for OriginalSegment<'_> {
    fn drop(&mut self) {
        self.guard
            .state
            .lock()
            .expect("readable stack lock poisoned")
            .original_depth -= 1;
    }
}

#[cfg(all(test, panic = "unwind"))]
mod frame_tests;

#[cfg(all(test, panic = "unwind"))]
mod tests {
    use super::*;
    use crate::{ids, stack_read::LocalWidth};
    use std::sync::Arc;
    const S: u32 = 0xffff00;
    fn guard() -> Arc<Guard> {
        Arc::new(Guard::default())
    }
    fn declaration(offset: i32, width: LocalWidth) -> UninitializedLocalRead {
        FrameState::new(S).uninitialized_local(
            ids::GET_RANDOM_NUMBER,
            offset,
            width,
            "authored test: own-frame local at this exact entry offset/width",
        )
    }

    #[test]
    fn unwritten_reads_abort_before_an_operand_is_returned_even_above_catalogue_stack_top() {
        let guard = guard();
        let mut returned = false;
        let error = guard
            .run_root(ids::GET_RANDOM_NUMBER, S, false, || {
                guard.read(S - 4, 4, Reader::RustMemory);
                returned = true;
            })
            .unwrap_err();
        assert!(!returned);
        assert_eq!(error.root, ids::GET_RANDOM_NUMBER);
        assert_eq!(error.address, S - 4);
        assert_eq!(error.width, 4);
        assert_eq!(error.reader, Reader::RustMemory);
    }

    #[test]
    fn written_aliases_and_above_entry_arguments_are_allowed_across_nested_entries() {
        let guard = guard();
        guard
            .run_root(ids::GET_RANDOM_NUMBER, S, false, || {
                guard.read(S, 4, Reader::RustMemory);
                guard.commit((S - 8) & 0xefffff, 4, true);
                guard
                    .run_root(ids::NO_OP, S - 16, false, || {
                        guard.read(S - 8, 4, Reader::HardwareRam)
                    })
                    .unwrap();
            })
            .unwrap();
    }

    #[test]
    fn catching_the_private_abort_cannot_clear_the_first_failure() {
        let guard = guard();
        let error = guard
            .run_root(ids::GET_RANDOM_NUMBER, S, false, || {
                let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    guard.read(S - 1, 1, Reader::RustMemory)
                }));
                assert!(caught.is_err());
                123
            })
            .unwrap_err();
        assert_eq!(error.reason, UnsupportedReason::Unwritten);
    }

    #[test]
    fn unrelated_panics_keep_their_original_payload_and_release_the_scope() {
        let guard = guard();
        let error = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = guard.run_root(ids::GET_RANDOM_NUMBER, S, false, || {
                std::panic::resume_unwind(Box::new(37_u32))
            });
        }))
        .unwrap_err();
        assert_eq!(*error.downcast::<u32>().unwrap(), 37);
        assert!(!guard.active());
    }

    #[test]
    fn original_spills_are_not_imported_as_own_writes() {
        let guard = guard();
        let error = guard
            .run_root(ids::GET_RANDOM_NUMBER, S, false, || {
                {
                    let _original = guard.original_segment();
                    guard.commit(S - 8, 4, true);
                }
                guard.read(S - 8, 4, Reader::RustMemory);
            })
            .unwrap_err();
        assert_eq!(error.reason, UnsupportedReason::Unwritten);
    }

    #[test]
    fn cited_mixed_locals_preserve_entry_lanes_without_blessing_generic_reads() {
        let guard = guard();
        let read = declaration(-8, LocalWidth::Long);
        let error = guard
            .run_root(ids::GET_RANDOM_NUMBER, S, false, || {
                guard.commit(S - 8, 1, true);
                assert_eq!(
                    guard
                        .explicit_local(read, read.frame, S - 32, false)
                        .unwrap(),
                    S - 8
                );
                guard.read(S - 8, 4, Reader::RustMemory);
            })
            .unwrap_err();
        assert_eq!(error.reason, UnsupportedReason::Unwritten);
        assert_eq!(error.reader, Reader::RustMemory);
    }

    #[test]
    fn intervening_non_own_writes_cannot_be_served_as_original_entry_residue() {
        let guard = guard();
        let read = declaration(-8, LocalWidth::Word);
        let error = guard
            .run_root(ids::GET_RANDOM_NUMBER, S, false, || {
                guard.commit(S - 8, 1, false);
                let caught = guard.explicit_local(read, read.frame, S - 32, false);
                assert!(caught.is_err());
                0
            })
            .unwrap_err();
        assert_eq!(error.reason, UnsupportedReason::InterveningNonOwnWrite);
    }

    #[test]
    fn citation_owner_alignment_and_current_allocation_are_checked_before_fetch() {
        for case in 0..5 {
            let guard = guard();
            let mut read = declaration(-8, LocalWidth::Long);
            match case {
                0 => read.citation = "",
                1 => read.owner = ids::NO_OP,
                2 => read.offset = -7,
                3 => read.offset = 0,
                _ => read.offset = -36,
            }
            let error = guard
                .run_root(ids::GET_RANDOM_NUMBER, S, false, || {
                    guard.explicit_local(read, read.frame, S - 32, false)
                })
                .unwrap_err();
            assert_eq!(error.reason, UnsupportedReason::InvalidExplicitLocal);
        }
    }

    #[test]
    fn typed_unsupported_preserves_the_actual_source_frame() {
        let error = Unsupported::unavailable(declaration(-4, LocalWidth::Word));
        let error = crate::Error::from(error);
        assert!(error.is_failed_execution());
        assert!(error.preserves_call_frame());
        assert!(!matches!(error, crate::Error::ControlTransfer(_)));
    }
    #[test]
    fn unknown_issuer_never_becomes_a_proof_by_completing_without_ram_reads() {
        for end in [
            DmaRetirement::Completed,
            DmaRetirement::Cancelled,
            DmaRetirement::ResetInvalidated,
            DmaRetirement::Discarded,
        ] {
            for (generation, uncertain) in [(1, true), (u64::MAX, false)] {
                let guard = guard();
                let error = guard
                    .run_root(ids::GET_RANDOM_NUMBER, S, false, || {
                        let epoch = guard.epoch();
                        guard.dma_started(epoch, generation, uncertain);
                        // Models a real no-RAM fill/copy retirement callback. Start
                        // does not unwind or leave the active ownership half-built.
                        assert!(guard.state.lock().unwrap().active_dma.is_some());
                        guard.dma_retired(epoch, generation, end);
                    })
                    .unwrap_err();
                assert_eq!(error.reason, UnsupportedReason::UnknownDmaOrigin);
                assert_eq!(error.reader, Reader::DmaBoundary);
                assert_eq!(guard.finish_proof().unwrap_err(), error);
                assert!(guard.state.lock().unwrap().active_dma.is_none());
                assert_eq!(guard.scope.namespace.metrics().active, 0);
                assert_eq!(guard.scope.namespace.metrics().failed, 1);
                assert_eq!(guard.scope.namespace.metrics().unknown_origin_proof_debt, 1);
            }
        }
    }
    #[test]
    fn unknown_debt_counts_scope_epochs_without_replacing_an_earlier_cause() {
        let namespace = Arc::new(Namespace::default());
        let guard = Guard::with_namespace(Arc::clone(&namespace));
        let first = guard
            .run_root(ids::GET_RANDOM_NUMBER, S, false, || {
                let _caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    guard.read(S - 1, 1, Reader::RustMemory);
                }));
                let epoch = guard.epoch();
                guard.dma_started(epoch, 1, true);
                guard.dma_retired(epoch, 1, DmaRetirement::Completed);
                // Another uncertain start in the same scope is not another debt.
                guard.dma_started(epoch, 2, true);
                guard.dma_retired(epoch, 2, DmaRetirement::Cancelled);
            })
            .unwrap_err();
        assert_eq!(first.reason, UnsupportedReason::Unwritten);
        assert_eq!(namespace.metrics().unknown_origin_proof_debt, 1);
        assert_eq!(namespace.metrics().active, 0);
        assert_eq!(guard.finish_proof().unwrap_err(), first);

        // A genuinely new analysis trajectory has a new epoch; clearing its
        // failure latch must not clear previously recorded accounting.
        namespace.reset_failure();
        guard
            .run_root(ids::GET_RANDOM_NUMBER, S, false, || {
                let epoch = guard.epoch();
                guard.dma_started(epoch, 2, true);
                guard.dma_retired(epoch, 2, DmaRetirement::Cancelled);
            })
            .unwrap_err();
        assert_eq!(guard.epoch(), 2);
        assert_eq!(namespace.metrics().unknown_origin_proof_debt, 2);
    }

    #[test]
    fn different_guard_slots_at_the_same_epoch_count_independently() {
        let namespace = Arc::new(Namespace::default());
        let slots = [
            Guard::with_namespace(Arc::clone(&namespace)),
            Guard::with_namespace(Arc::clone(&namespace)),
        ];
        for (index, guard) in slots.iter().enumerate() {
            namespace.reset_failure();
            let error = guard
                .run_root(ids::GET_RANDOM_NUMBER, S, false, || {
                    let epoch = guard.epoch();
                    guard.dma_started(epoch, 1, true);
                    guard.dma_retired(epoch, 1, DmaRetirement::Discarded);
                })
                .unwrap_err();
            assert_eq!(error.reason, UnsupportedReason::UnknownDmaOrigin);
            assert_eq!(guard.epoch(), 1);
            assert_eq!(
                namespace.metrics().unknown_origin_proof_debt,
                index as u64 + 1
            );
        }
        namespace.reset_failure();
        assert_eq!(namespace.metrics().unknown_origin_proof_debt, 2);
        assert_eq!(namespace.metrics().active, 0);
    }
    #[test]
    fn forwarding_another_slots_first_error_does_not_create_new_origin_debt() {
        let namespace = Arc::new(Namespace::default());
        let origin = Guard::with_namespace(Arc::clone(&namespace));
        let caller = Guard::with_namespace(Arc::clone(&namespace));
        let error = caller
            .run_root(ids::NO_OP, S, false, || {
                origin
                    .run_root(ids::GET_RANDOM_NUMBER, S - 16, false, || {
                        origin.dma_started(origin.epoch(), 1, true);
                        origin.dma_retired(origin.epoch(), 1, DmaRetirement::Completed);
                    })
                    .unwrap_err();
                caller.assert_no_failure();
            })
            .unwrap_err();
        assert_eq!(error.root, ids::GET_RANDOM_NUMBER);
        assert_eq!(error.reason, UnsupportedReason::UnknownDmaOrigin);
        assert_eq!(namespace.metrics().unknown_origin_proof_debt, 1);
    }
}
