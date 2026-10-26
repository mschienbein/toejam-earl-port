use super::*;
use crate::{CheckFailure, Host, HostError, Mode, Result, hooks::Hook};
use genesis_core::{GenesisBus, HybridExceptionBoundary};
use serde::ser::SerializeStruct;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceStatus {
    #[default]
    Independent,
    Provisional,
    Discharged,
    Invalidated,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ProvenanceSummary {
    pub status: ProvenanceStatus,
    pub approvals: u64,
    pub provisional: u64,
    pub discharged: u64,
    pub invalidated: u64,
}

impl ProvenanceSummary {
    /// Allocation-free eligibility query for status/report consumers.
    pub fn is_proof_eligible(&self) -> bool {
        self.provisional == 0
            && self.invalidated == 0
            && matches!(
                self.status,
                ProvenanceStatus::Independent | ProvenanceStatus::Discharged
            )
    }

    /// A zero-mismatch run is not approvable while any provenance obligation
    /// remains pending or was invalidated. This guards only provenance; callers
    /// must also enforce their ordinary differential/coverage requirements.
    pub fn ensure_proof_eligible(&self) -> Result<()> {
        if self.invalidated != 0 || self.status == ProvenanceStatus::Invalidated {
            return Err(HostError::Metadata(format!(
                "future-frame provenance invalidated: {} approval(s); proof is ineligible",
                self.invalidated,
            )));
        }
        if self.provisional != 0 || self.status == ProvenanceStatus::Provisional {
            return Err(HostError::Metadata(format!(
                "future-frame provenance pending: {} unresolved approval(s); proof is not approved",
                self.provisional,
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
struct ProducerBinding {
    approval: ApprovalId,
    function: String,
    entry_pc: u32,
    subsystem: &'static str,
    execution_class: &'static str,
    proof_envelope: &'static str,
    actual_check_route: &'static str,
    explicit_proof_only_admission: bool,
    candidate_label: String,
    candidate_hash: u64,
    registry_epoch: u64,
    #[serde(skip)]
    coverage_index: usize,
    rejection_counted: bool,
}

type WatchCell = Arc<Mutex<FutureFrameWatches>>;

/// Read-only debt bound to the exact private watch graph that created it.
/// Public report swaps, numeric reuse and serialized diagnostics cannot mint it.
#[derive(Debug, Clone)]
pub(crate) struct ProofDependency {
    watches: WatchCell,
    approval: ApprovalId,
}
impl ProofDependency {
    pub(crate) fn outcome(&self) -> DependencyOutcome {
        self.watches
            .lock()
            .expect("future-frame observer lock poisoned")
            .dependency_outcome(self.approval)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProofCaptureId(u64);
#[derive(Debug)]
struct ProofCaptureJournal {
    id: ProofCaptureId,
    dependencies: Vec<ProofDependency>,
}
pub(crate) struct ProofCaptureScope {
    id: ProofCaptureId,
    previous: Option<ProofCaptureJournal>,
}

fn serialize_watches<S: serde::Serializer>(
    watches: &Option<WatchCell>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    match watches {
        Some(watches) => {
            let snapshot = watches
                .lock()
                .expect("future-frame observer lock poisoned")
                .evidence_snapshot();
            serializer.serialize_some(&snapshot)
        }
        None => serializer.serialize_none(),
    }
}

#[derive(Debug, Default, Serialize)]
struct Campaign {
    epoch: u64,
    #[serde(serialize_with = "serialize_watches")]
    watches: Option<WatchCell>,
    bindings: BTreeMap<ApprovalId, ProducerBinding>,
}

#[derive(Debug, Default)]
pub struct ProvenanceReport {
    current: Campaign,
    previous: Vec<Campaign>,
    next_approval: u64,
}

impl ProvenanceReport {
    pub fn summary(&self) -> ProvenanceSummary {
        let mut summary = ProvenanceSummary {
            status: ProvenanceStatus::Independent,
            approvals: 0,
            provisional: 0,
            discharged: 0,
            invalidated: 0,
        };
        for campaign in self.previous.iter().chain(std::iter::once(&self.current)) {
            let Some(watches) = &campaign.watches else {
                continue;
            };
            let watches = watches.lock().expect("future-frame observer lock poisoned");
            for id in campaign.bindings.keys() {
                summary.approvals += 1;
                match watches.outcome(*id) {
                    ApprovalOutcome::Invalidated { .. } => summary.invalidated += 1,
                    ApprovalOutcome::Discharged => summary.discharged += 1,
                    ApprovalOutcome::NotTracked | ApprovalOutcome::Provisional { .. } => {
                        summary.provisional += 1
                    }
                }
            }
        }
        summary.status = if summary.invalidated != 0 {
            ProvenanceStatus::Invalidated
        } else if summary.provisional != 0 {
            ProvenanceStatus::Provisional
        } else if summary.approvals != 0 {
            ProvenanceStatus::Discharged
        } else {
            ProvenanceStatus::Independent
        };
        summary
    }

    fn reset_campaign(&mut self) {
        if let Some(watches) = &self.current.watches {
            watches
                .lock()
                .expect("future-frame observer lock poisoned")
                .finish(CampaignEnd::Reset);
        }
        let epoch = self.current.epoch + 1;
        let previous = std::mem::replace(
            &mut self.current,
            Campaign {
                epoch,
                ..Campaign::default()
            },
        );
        if !previous.bindings.is_empty() {
            self.previous.push(previous);
        }
    }
}

impl Serialize for ProvenanceReport {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut report = serializer.serialize_struct("FutureFrameProvenance", 3)?;
        report.serialize_field("summary", &self.summary())?;
        report.serialize_field("current", &self.current)?;
        report.serialize_field("previous", &self.previous)?;
        report.end()
    }
}

#[derive(Debug)]
struct ForwardedObserver {
    accesses: Option<Vec<HybridAccess>>,
    boundaries: Option<Vec<HybridExceptionBoundary>>,
}

impl ForwardedObserver {
    fn forward(&mut self, access: HybridAccess, boundaries: &[HybridExceptionBoundary]) {
        let Some(saved) = &mut self.accesses else {
            return;
        };
        if let HybridAccess::ExceptionBoundary { index } = access {
            if let (Some(target), Some(payload)) =
                (&mut self.boundaries, boundaries.get(index as usize))
            {
                let index = u32::try_from(target.len()).expect("forwarded boundary index");
                target.push(*payload);
                saved.push(HybridAccess::ExceptionBoundary { index });
            }
        } else {
            saved.push(access);
        }
    }
}

/// Installed only while real live receipts need observation. The shared cell is
/// analysis ownership, not game RAM or a second initialized-byte bitmap.
#[derive(Debug, Default)]
pub(crate) struct LiveObserverLease {
    branch: Option<BranchProvenance>,
    pub(crate) natural_checks: usize,
    isolated_depth: usize,
    proof_capture: Option<ProofCaptureJournal>,
}

impl LiveObserverLease {
    fn install(&mut self, bus: &mut GenesisBus, watches: WatchCell, frame: u64) {
        if self.branch.is_none() {
            let forward = ForwardedObserver {
                accesses: bus.hybrid_cpu_accesses.replace(Vec::new()),
                boundaries: bus.hybrid_exception_boundaries.replace(Vec::new()),
            };
            self.branch = Some(BranchProvenance::actual_original(
                watches,
                frame,
                Arc::new(Mutex::new(forward)),
            ));
        }
    }

    fn restore(&mut self, bus: &mut GenesisBus) {
        if let Some(branch) = self.branch.take() {
            let mut forward = branch
                .forward
                .as_ref()
                .expect("live observer owner")
                .lock()
                .expect("future-frame observer lock poisoned");
            bus.hybrid_cpu_accesses = forward.accesses.take();
            bus.hybrid_exception_boundaries = forward.boundaries.take();
        }
    }
}

impl Host {
    /// Enables only dependency observation; this grants no actor/live authority.
    /// The off path remains None and allocates no capture storage.
    pub(crate) fn begin_function_proof_capture(&mut self) -> Result<ProofCaptureScope> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map(ProofCaptureId)
            .map_err(|_| {
                HostError::Metadata("private proof-capture scope identity exhausted".into())
            })?;
        let previous = self
            .future_observer
            .proof_capture
            .replace(ProofCaptureJournal {
                id,
                dependencies: Vec::new(),
            });
        Ok(ProofCaptureScope { id, previous })
    }
    pub(crate) fn finish_function_proof_capture(
        &mut self,
        mut scope: ProofCaptureScope,
    ) -> Result<Vec<ProofDependency>> {
        let current = self.future_observer.proof_capture.take();
        let result = match current {
            Some(current) if current.id == scope.id => {
                if let Some(previous) = &mut scope.previous {
                    previous
                        .dependencies
                        .extend(current.dependencies.iter().cloned());
                }
                Ok(current.dependencies)
            }
            _ => Err(HostError::Metadata(
                "private proof-capture scope is missing or not the exact innermost journal".into(),
            )),
        };
        self.future_observer.proof_capture = scope.previous;
        result
    }
    pub fn future_frame_provenance_outcome(&self) -> ProvenanceSummary {
        self.checks.future_frame_provenance.summary()
    }

    /// Provisional is pending, never a successful zero-mismatch proof.
    pub fn finish_future_frame_provenance(&mut self) -> ProvenanceSummary {
        self.record_readable_stack_metrics();
        // This snapshot API reports the recorded rejection through its status;
        // it does not turn an ignored earlier context error into a clean run.
        let _recorded_rejection = self.record_future_frame_rejections();
        if let Some(watches) = &self.checks.future_frame_provenance.current.watches {
            watches
                .lock()
                .expect("future-frame observer lock poisoned")
                .finish(CampaignEnd::Completed);
        }
        self.future_observer.restore(self.emulator.hybrid_bus_mut());
        self.future_frame_provenance_outcome()
    }

    pub(crate) fn reset_future_frame_provenance(&mut self) -> ProvenanceReport {
        let _recorded_rejection = self.record_future_frame_rejections();
        self.future_observer.restore(self.emulator.hybrid_bus_mut());
        let mut report = std::mem::take(&mut self.checks.future_frame_provenance);
        report.reset_campaign();
        report
    }

    /// The certifier calls this after its main/replay proof. This only binds
    /// rejection obligations; it never registers or promotes a translation.
    pub fn arm_future_frame_watch(
        &mut self,
        hook: &Hook,
        candidate_label: &str,
        candidate_hash: u64,
        receipts: Vec<WatchReceipt>,
    ) -> Result<Option<ApprovalId>> {
        if receipts.is_empty() {
            return Ok(None);
        }
        if self.mode != Mode::Check {
            return Err(HostError::Metadata(
                "future-frame receipts require check mode".into(),
            ));
        }
        let active = self
            .registry
            .at(hook.symbol.address(self.revision))
            .filter(|active| {
                active.symbol.id == hook.symbol.id
                    && active.symbol.name == hook.symbol.name
                    && active.coverage_index == hook.coverage_index
                    && active.execution_class == hook.execution_class
                    && active.proof_envelope == hook.proof_envelope
                    && active.proof_only_admission() == hook.proof_only_admission()
                    && active.clock_ceiling == hook.clock_ceiling
                    && active.clock_evidence == hook.clock_evidence
            })
            .ok_or_else(|| {
                HostError::Metadata(
                    "future-frame producer must match the active hook policy and actual check route"
                        .into(),
                )
            })?;
        let hook = active;
        let natural =
            self.future_observer.natural_checks != 0 && self.future_observer.isolated_depth == 0;
        let report = &mut self.checks.future_frame_provenance;
        let mut isolated = Campaign {
            epoch: report.current.epoch,
            ..Campaign::default()
        };
        let campaign = if natural {
            &mut report.current
        } else {
            &mut isolated
        };
        let cell = campaign
            .watches
            .get_or_insert_with(|| Arc::new(Mutex::new(FutureFrameWatches::default())));
        let mut watches = cell.lock().expect("future-frame observer lock poisoned");
        watches.next_approval = report.next_approval;
        let approval = watches.new_approval().map_err(watch_error)?;
        report.next_approval = watches.next_approval;
        let mut ids = Vec::with_capacity(receipts.len());
        for receipt in receipts {
            ids.push(watches.arm(approval, receipt).map_err(watch_error)?);
        }
        if !natural {
            watches.hold_receipts(&ids, CampaignEnd::NotLiveExecution);
        }
        drop(watches);
        let execution_class = match hook.execution_class {
            tje_game::execution::ExecutionClass::Atomic => "atomic",
            tje_game::execution::ExecutionClass::Functional => "functional",
            tje_game::execution::ExecutionClass::TimeAware => "time_aware",
        };
        campaign.bindings.insert(
            approval,
            ProducerBinding {
                approval,
                function: hook.symbol.name.to_owned(),
                entry_pc: hook.symbol.address(self.revision),
                subsystem: hook.symbol.subsystem,
                execution_class,
                proof_envelope: hook.proof_envelope.as_str(),
                actual_check_route: hook.check_route().as_str(),
                explicit_proof_only_admission: hook.proof_only_admission(),
                candidate_label: candidate_label.to_owned(),
                candidate_hash,
                registry_epoch: campaign.epoch,
                coverage_index: hook.coverage_index,
                rejection_counted: false,
            },
        );
        if let Some(scope) = &mut self.future_observer.proof_capture {
            scope.dependencies.push(ProofDependency {
                watches: Arc::clone(cell),
                approval,
            });
        }
        if natural {
            self.future_observer.install(
                self.emulator.hybrid_bus_mut(),
                Arc::clone(cell),
                self.frame,
            );
        }
        if !natural {
            report.previous.push(isolated);
        }
        Ok(Some(approval))
    }

    pub fn future_frame_approval_outcome(&self, approval: ApprovalId) -> ApprovalOutcome {
        let report = &self.checks.future_frame_provenance;
        for campaign in report
            .previous
            .iter()
            .chain(std::iter::once(&report.current))
        {
            if campaign.bindings.contains_key(&approval)
                && let Some(cell) = &campaign.watches
            {
                return cell
                    .lock()
                    .expect("future-frame observer lock poisoned")
                    .outcome(approval);
            }
        }
        ApprovalOutcome::NotTracked
    }

    pub(crate) fn begin_live_frame_observation(&mut self) -> Result<bool> {
        self.observe_retained_native_prefix()?;
        let Some(branch) = &self.future_observer.branch else {
            return Ok(false);
        };
        branch.begin_step(
            self.emulator.hybrid_cpu(),
            self.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
            self.frame,
        );
        Ok(true)
    }

    pub(crate) fn finish_live_frame_observation(&mut self, observing: bool) -> Result<()> {
        if !observing {
            return Ok(());
        }
        let branch = self
            .future_observer
            .branch
            .as_ref()
            .expect("live provenance lease");
        drain_branch_bus(self.emulator.hybrid_bus_mut(), branch);
        let failure = branch.take_failure();
        let needed = branch
            .watches
            .lock()
            .expect("future-frame observer lock poisoned")
            .needs_observation();
        if !needed {
            self.future_observer.restore(self.emulator.hybrid_bus_mut());
        }
        let rejected = self.record_future_frame_rejections();
        if let Some(error) = failure {
            return Err(watch_error(error));
        }
        rejected
    }

    /// Return a partial tick's actual journal to any foreign observer without
    /// applying watches, completing a step, or discharging a receipt.
    pub(crate) fn abort_live_frame_observation(&mut self, observing: bool) {
        if !observing {
            return;
        }
        let branch = self
            .future_observer
            .branch
            .as_ref()
            .expect("live provenance lease");
        {
            let mut forward = branch
                .forward
                .as_ref()
                .expect("live observer owner")
                .lock()
                .expect("future-frame observer lock poisoned");
            let bus = self.emulator.hybrid_bus_mut();
            let boundaries = bus
                .hybrid_exception_boundaries
                .as_deref()
                .unwrap_or_default();
            if let Some(accesses) = &mut bus.hybrid_cpu_accesses {
                for access in accesses.drain(..) {
                    forward.forward(access, boundaries);
                }
            }
        }
        self.future_observer.restore(self.emulator.hybrid_bus_mut());
    }

    pub(crate) fn future_frame_branch(
        &self,
        side: Side,
        origin: ObservationOrigin,
    ) -> Option<BranchProvenance> {
        let cell = self
            .checks
            .future_frame_provenance
            .current
            .watches
            .as_ref()?;
        let watches = cell.lock().expect("future-frame observer lock poisoned");
        watches
            .needs_observation()
            .then(|| BranchProvenance::speculative(watches.clone(), side, origin, self.frame))
    }

    fn actual_original_provenance(&self) -> Option<BranchProvenance> {
        self.future_observer
            .branch
            .as_ref()
            .map(BranchProvenance::execution_scope)
    }

    pub(crate) fn merge_future_frame_branch(
        &mut self,
        branch: Option<BranchProvenance>,
    ) -> Result<()> {
        let Some(branch) = branch else {
            return Ok(());
        };
        let live = self
            .checks
            .future_frame_provenance
            .current
            .watches
            .as_ref()
            .expect("provenance campaign");
        if !Arc::ptr_eq(live, &branch.watches) {
            live.lock()
                .expect("future-frame observer lock poisoned")
                .merge_branch_rejections(
                    &branch
                        .watches
                        .lock()
                        .expect("future-frame observer lock poisoned"),
                )
                .map_err(watch_error)?;
        }
        let rejected = self.record_future_frame_rejections();
        if let Some(error) = branch.take_failure() {
            return Err(watch_error(error));
        }
        rejected
    }

    fn record_future_frame_rejections(&mut self) -> Result<()> {
        let report = &mut self.checks;
        let campaign = &mut report.future_frame_provenance.current;
        let Some(cell) = &campaign.watches else {
            return Ok(());
        };
        let mut watches = cell.lock().expect("future-frame observer lock poisoned");
        if !watches.take_rejections_pending() {
            return Ok(());
        }
        let mut first = None;
        for binding in campaign.bindings.values_mut() {
            if binding.rejection_counted {
                continue;
            }
            let ApprovalOutcome::Invalidated { receipts } = watches.outcome(binding.approval)
            else {
                continue;
            };
            binding.rejection_counted = true;
            report.mismatches += 1;
            report.by_function.at_mut(binding.coverage_index).mismatches += 1;
            report
                .by_subsystem
                .get_mut(binding.subsystem)
                .expect("bound producer subsystem")
                .mismatches += 1;
            let violation = receipts
                .iter()
                .find_map(|id| {
                    if let ReceiptOutcome::Invalidated { violation } =
                        &watches.evidence()[id.value() as usize].outcome
                    {
                        Some(violation)
                    } else {
                        None
                    }
                })
                .expect("invalidated receipt evidence");
            let detail = format!(
                "future private-frame provenance rejected approval {} (candidate {}): {:?} {:?} read {:06X}/{} at frame {}, checkpoint clock {}; byte {:06X}, current Original={:02X} Rust={:02X}, authoritative-current=({}, {})",
                binding.approval.value(),
                binding.candidate_label,
                violation.side,
                violation.origin,
                violation.raw_address,
                violation.width,
                violation.checkpoint.frame,
                violation.checkpoint.master_clock,
                violation.byte.address,
                violation.current_values.original,
                violation.current_values.translated,
                violation.current_values.original_current_attested,
                violation.current_values.translated_current_attested,
            );
            let (detail, detail_truncated) = crate::check_failure::bounded_detail(detail);
            report.failures.push(CheckFailure {
                frame: self.frame,
                function: binding.function.clone(),
                entry_pc: binding.entry_pc,
                master_clock: self.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
                frame_write_index: self
                    .emulator
                    .hybrid_bus()
                    .hybrid_writes
                    .as_ref()
                    .map_or(0, Vec::len),
                trace_index: None,
                detail: detail.clone(),
                detail_truncated,
                vdp_mismatch: None,
                // This invalidates an earlier producer, not the current reader.
                actual_capture: None,
                unsupported_stack_read: None,
            });
            first.get_or_insert_with(|| HostError::Mismatch {
                frame: self.frame,
                function: binding.function.clone(),
                detail,
            });
        }
        first.map_or(Ok(()), Err)
    }
}

fn watch_error(error: WatchError) -> HostError {
    HostError::Metadata(format!("future private-frame provenance: {error}"))
}

#[derive(Debug)]
struct StepCheckpoint {
    cpu: [u32; 25],
    frame: u64,
    clock: u64,
    cpu_actor: bool,
}

/// Speculative construction deep-copies the model. Only the private Host live
/// factory shares the actual Original trajectory. No Rust-current factory exists.
#[derive(Debug)]
pub(crate) struct BranchProvenance {
    watches: WatchCell,
    side: Side,
    origin: ObservationOrigin,
    step: Mutex<StepCheckpoint>,
    failure: Mutex<Option<WatchError>>,
    actual_original: bool,
    forward: Option<Arc<Mutex<ForwardedObserver>>>,
}

impl BranchProvenance {
    fn speculative(
        watches: FutureFrameWatches,
        side: Side,
        origin: ObservationOrigin,
        frame: u64,
    ) -> Self {
        Self {
            watches: Arc::new(Mutex::new(watches)),
            side,
            origin,
            step: Mutex::new(StepCheckpoint {
                cpu: [0; 25],
                frame,
                clock: 0,
                cpu_actor: false,
            }),
            failure: Mutex::new(None),
            actual_original: false,
            forward: None,
        }
    }

    fn actual_original(
        watches: WatchCell,
        frame: u64,
        forward: Arc<Mutex<ForwardedObserver>>,
    ) -> Self {
        Self {
            watches,
            side: Side::Original,
            origin: ObservationOrigin::Live,
            step: Mutex::new(StepCheckpoint {
                cpu: [0; 25],
                frame,
                clock: 0,
                cpu_actor: false,
            }),
            failure: Mutex::new(None),
            actual_original: true,
            forward: Some(forward),
        }
    }

    fn execution_scope(&self) -> Self {
        let frame = self
            .step
            .lock()
            .expect("future-frame observer lock poisoned")
            .frame;
        Self {
            watches: Arc::clone(&self.watches),
            side: self.side,
            origin: self.origin,
            step: Mutex::new(StepCheckpoint {
                cpu: [0; 25],
                frame,
                clock: 0,
                cpu_actor: false,
            }),
            failure: Mutex::new(None),
            actual_original: self.actual_original,
            forward: self.forward.as_ref().map(Arc::clone),
        }
    }

    pub(crate) fn begin_step(&self, cpu: &m68000_emu::M68000, clock: u64, frame: u64) {
        *self
            .step
            .lock()
            .expect("future-frame observer lock poisoned") = StepCheckpoint {
            cpu: cpu.hybrid_state_words(),
            frame,
            clock,
            cpu_actor: true,
        };
        self.watches
            .lock()
            .expect("future-frame observer lock poisoned")
            .begin_access_batch(self.side);
    }

    pub(crate) fn begin_cpu_step(&self, cpu: &m68000_emu::M68000, clock: u64) {
        let frame = self
            .step
            .lock()
            .expect("future-frame observer lock poisoned")
            .frame;
        self.begin_step(cpu, clock, frame);
    }

    pub(crate) fn begin_native(&self, frame: u64, clock: u64) {
        let mut step = self
            .step
            .lock()
            .expect("future-frame observer lock poisoned");
        step.frame = frame;
        step.clock = clock;
        step.cpu_actor = false;
        drop(step);
        self.watches
            .lock()
            .expect("future-frame observer lock poisoned")
            .begin_access_batch(self.side);
    }

    pub(crate) fn observe_stream(
        &self,
        access: HybridAccess,
        boundaries: &[HybridExceptionBoundary],
    ) {
        let step = self
            .step
            .lock()
            .expect("future-frame observer lock poisoned");
        let checkpoint = AccessCheckpoint {
            frame: step.frame,
            master_clock: step.clock,
            cpu_pc: step.cpu_actor.then_some(step.cpu[17]),
            origin: self.origin,
        };
        if !step.cpu_actor
            && matches!(access, HybridAccess::ExceptionBoundary { index }
            if boundaries.get(index as usize).is_some_and(|payload| {
                matches!(payload.boundary, m68000_emu::ExceptionBoundary::Entered { .. })
            }))
        {
            self.watches
                .lock()
                .expect("future-frame observer lock poisoned")
                .finish(CampaignEnd::ObserverLost);
            self.failure
                .lock()
                .expect("future-frame observer lock poisoned")
                .get_or_insert(WatchError::InterruptChronology);
            if let Some(forward) = &self.forward {
                forward
                    .lock()
                    .expect("future-frame observer lock poisoned")
                    .forward(access, boundaries);
            }
            return;
        }
        let result = if self.actual_original {
            self.watches
                .lock()
                .expect("future-frame observer lock poisoned")
                .observe_live_stream(checkpoint, &step.cpu, access, boundaries)
        } else {
            self.watches
                .lock()
                .expect("future-frame observer lock poisoned")
                .observe_stream(self.side, checkpoint, &step.cpu, access, boundaries)
        };
        if let Err(error) = result {
            self.failure
                .lock()
                .expect("future-frame observer lock poisoned")
                .get_or_insert(error);
        }
        if let Some(forward) = &self.forward {
            forward
                .lock()
                .expect("future-frame observer lock poisoned")
                .forward(access, boundaries);
        }
    }

    pub(crate) fn observe_native(&self, access: HybridAccess, frame: u64, clock: u64) {
        self.watches
            .lock()
            .expect("future-frame observer lock poisoned")
            .observe(
                self.side,
                AccessCheckpoint {
                    frame,
                    master_clock: clock,
                    cpu_pc: None,
                    origin: self.origin,
                },
                access,
            );
    }

    pub(crate) fn observe_setup_write(&self, address: u32, value: u8) {
        self.watches
            .lock()
            .expect("future-frame observer lock poisoned")
            .observe_setup_write(self.side, address, value);
    }

    pub(crate) fn check_result(&self) -> Result<()> {
        if let Some(error) = self.take_failure() {
            return Err(watch_error(error));
        }
        let watches = self
            .watches
            .lock()
            .expect("future-frame observer lock poisoned");
        if let Some(violation) = watches.evidence().iter().find_map(|entry| {
            if let ReceiptOutcome::Invalidated { violation } = &entry.outcome {
                Some(violation)
            } else {
                None
            }
        }) {
            return Err(HostError::Metadata(format!(
                "future private-frame provenance rejected {:?} {:?} read {:06X}/{}",
                violation.side, violation.origin, violation.raw_address, violation.width,
            )));
        }
        Ok(())
    }

    fn take_failure(&self) -> Option<WatchError> {
        self.failure
            .lock()
            .expect("future-frame observer lock poisoned")
            .take()
    }
}

pub(crate) fn drain_branch_bus(bus: &mut GenesisBus, branch: &BranchProvenance) {
    let boundaries = bus
        .hybrid_exception_boundaries
        .as_deref()
        .unwrap_or_default();
    if let Some(accesses) = &mut bus.hybrid_cpu_accesses {
        for access in accesses.drain(..) {
            branch.observe_stream(access, boundaries);
        }
    }
    if let Some(boundaries) = &mut bus.hybrid_exception_boundaries {
        boundaries.clear();
    }
}

pub(crate) struct IsolatedProvenance {
    saved: Option<(WatchCell, Option<BranchProvenance>)>,
}

impl Host {
    pub(crate) fn begin_isolated_provenance(&mut self) -> IsolatedProvenance {
        self.future_observer.isolated_depth += 1;
        let Some(live) = self.checks.future_frame_provenance.current.watches.as_ref() else {
            return IsolatedProvenance { saved: None };
        };
        if !live
            .lock()
            .expect("future-frame observer lock poisoned")
            .needs_observation()
        {
            return IsolatedProvenance { saved: None };
        }
        let live = Arc::clone(live);
        let model = live
            .lock()
            .expect("future-frame observer lock poisoned")
            .clone();
        let cell = Arc::new(Mutex::new(model));
        self.checks.future_frame_provenance.current.watches = Some(Arc::clone(&cell));
        let old = self.future_observer.branch.take();
        self.emulator.hybrid_bus_mut().hybrid_cpu_accesses = Some(Vec::new());
        self.emulator.hybrid_bus_mut().hybrid_exception_boundaries = Some(Vec::new());
        self.future_observer.branch = Some(BranchProvenance {
            watches: cell,
            side: Side::Original,
            origin: ObservationOrigin::FixtureReplay,
            step: Mutex::new(StepCheckpoint {
                cpu: [0; 25],
                frame: self.frame,
                clock: 0,
                cpu_actor: false,
            }),
            failure: Mutex::new(None),
            actual_original: false,
            forward: Some(Arc::new(Mutex::new(ForwardedObserver {
                accesses: None,
                boundaries: None,
            }))),
        });
        IsolatedProvenance {
            saved: Some((live, old)),
        }
    }

    pub(crate) fn finish_isolated_provenance(&mut self, scope: IsolatedProvenance) -> Result<()> {
        self.future_observer.isolated_depth -= 1;
        let Some((live, observer)) = scope.saved else {
            return Ok(());
        };
        let isolated = self
            .checks
            .future_frame_provenance
            .current
            .watches
            .replace(Arc::clone(&live))
            .expect("isolated provenance model");
        self.future_observer.branch = observer;
        live.lock()
            .expect("future-frame observer lock poisoned")
            .merge_branch_rejections(
                &isolated
                    .lock()
                    .expect("future-frame observer lock poisoned"),
            )
            .map_err(watch_error)?;
        self.record_future_frame_rejections()
    }

    pub(crate) fn inspection_context(&mut self) -> crate::abi::HookContext<'_> {
        let provenance = self.actual_original_provenance();
        crate::abi::HookContext::new(
            &mut self.emulator,
            self.revision,
            self.registry.dispatcher(),
            self.mode,
        )
        .with_inspection_provenance(provenance)
        .with_timing_services(self.timing_services.as_mut())
    }

    /// Existing diagnostic original execution, on the retained machine (or the
    /// explicitly isolated fixture machine), never on a counterfeit live fork.
    pub(crate) fn execute_fixture_original(&mut self, hook: &Hook, cleanup_sp: u32) -> Result<()> {
        let Some(provenance) = self.actual_original_provenance() else {
            return match hook.check_route() {
                crate::CheckForkRoute::TimeAwareLogic
                | crate::CheckForkRoute::ClockedCommutingInterrupts
                | crate::CheckForkRoute::ClockedStrict => {
                    self.execute_timed_fixture_original(hook, cleanup_sp, None)
                }
                crate::CheckForkRoute::FrozenAtomic | crate::CheckForkRoute::FrozenFunctional => {
                    if hook.check_route() == crate::CheckForkRoute::FrozenFunctional {
                        self.emulator
                            .hybrid_bus_mut()
                            .hybrid_set_functional_mode(true);
                    }
                    crate::abi::run_original_atomic(
                        &mut self.emulator,
                        crate::abi::CALL_SENTINEL,
                        cleanup_sp,
                        false,
                    )
                    .map(|_| ())
                }
                crate::CheckForkRoute::ReplayedEnvironment => {
                    Err(crate::replayed_environment::unsupported(
                        hook,
                        crate::replayed_environment::UnsupportedKind::EntryBoundary,
                        "replay fixture requires an actual live JSR/BSR witness; a prepared sentinel frame is not admission",
                    ))
                }
            };
        };
        let mut stack = crate::abi::StackAuditState::new(self.cpu().stack_pointer(), false);
        stack.provenance = Some(provenance);
        if let Some(branch) = &stack.provenance {
            branch.begin_native(
                self.frame,
                self.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
            );
        }
        let mut audit = stack.empty_run();
        if hook.check_route() == crate::CheckForkRoute::FrozenFunctional {
            self.emulator
                .hybrid_bus_mut()
                .hybrid_set_functional_mode(true);
            if stack.provenance.is_some() {
                stack.drain(
                    self.emulator.hybrid_bus_mut(),
                    hook.symbol.address(self.revision),
                    &mut audit,
                );
            }
        }
        let result = match hook.check_route() {
            crate::CheckForkRoute::TimeAwareLogic
            | crate::CheckForkRoute::ClockedCommutingInterrupts
            | crate::CheckForkRoute::ClockedStrict => {
                self.execute_timed_fixture_original(hook, cleanup_sp, stack.provenance.as_ref())
            }
            crate::CheckForkRoute::FrozenAtomic | crate::CheckForkRoute::FrozenFunctional => {
                if stack.provenance.is_some() {
                    crate::abi::run_original_atomic_scoped(
                        &mut self.emulator,
                        crate::exits::ReferenceBoundary::returned(
                            crate::abi::CALL_SENTINEL,
                            cleanup_sp,
                        ),
                        &mut stack,
                    )
                    .map(|_| ())
                } else {
                    crate::abi::run_original_atomic(
                        &mut self.emulator,
                        crate::abi::CALL_SENTINEL,
                        cleanup_sp,
                        false,
                    )
                    .map(|_| ())
                }
            }
            crate::CheckForkRoute::ReplayedEnvironment => {
                Err(crate::replayed_environment::unsupported(
                    hook,
                    crate::replayed_environment::UnsupportedKind::EntryBoundary,
                    "replay fixture requires an actual live JSR/BSR witness; a prepared sentinel frame is not admission",
                ))
            }
        };
        let observed = self.merge_future_frame_branch(stack.provenance.take());
        result?;
        observed
    }
}

impl Host {
    pub(crate) fn observe_retained_native_prefix(&mut self) -> Result<()> {
        let Some(branch) = &self.future_observer.branch else {
            return Ok(());
        };
        branch.begin_native(
            self.frame,
            self.emulator.hybrid_bus().cycles.m68k_mclk_cycles,
        );
        drain_branch_bus(self.emulator.hybrid_bus_mut(), branch);
        let failure = branch.take_failure();
        let needed = branch
            .watches
            .lock()
            .expect("future-frame observer lock poisoned")
            .needs_observation();
        if !needed {
            self.future_observer.restore(self.emulator.hybrid_bus_mut());
        }
        let rejected = self.record_future_frame_rejections();
        if let Some(error) = failure {
            return Err(watch_error(error));
        }
        rejected
    }
}

impl Host {
    fn execute_timed_fixture_original(
        &mut self,
        hook: &Hook,
        cleanup_sp: u32,
        provenance: Option<&BranchProvenance>,
    ) -> Result<()> {
        let mut clock = self.wait_clock.clone();
        let entry_usage = clock.call_clock.usage();
        let scope = clock.enter_call_ceiling(hook.symbol.id, hook.clock_ceiling)?;
        let run = if let Some(provenance) = provenance {
            crate::execution::run_original_timed_with_audio(
                &mut self.emulator,
                &mut clock,
                crate::abi::CALL_SENTINEL,
                cleanup_sp,
                Vec::new(),
                true,
                crate::execution::PhysicalObservers {
                    provenance: Some(provenance),
                    capture: None,
                    readable: None,
                    original_root: None,
                },
            )
        } else {
            crate::execution::run_original_timed(
                &mut self.emulator,
                &mut clock,
                crate::abi::CALL_SENTINEL,
                cleanup_sp,
            )
        };
        let cleanup = clock.leave_call_ceiling(scope);
        let result = run.and_then(|_| cleanup.map(|_| ()));
        let usage = clock
            .call_clock
            .usage()
            .since(entry_usage)
            .map_err(HostError::Clock)?;
        self.checks
            .call_clocks
            .push(crate::call_clock::CallClockRecord::new(
                hook,
                "original",
                crate::call_clock::ClockRunPurpose::FixtureReplay,
                usage,
                (
                    self.emulator.hybrid_cpu().pc(),
                    self.emulator.hybrid_cpu().stack_pointer(),
                ),
                result.as_ref().err().map(ToString::to_string),
            ));
        result
    }
}

#[cfg(test)]
mod proof_capture_tests {
    //! Authored watch-model data, not live/Rust authority or a game proof.
    use super::*;
    use tje_game::ids;

    fn machine() -> (Host, Hook) {
        let mut host = crate::fixture::tests::synthetic_host(&[0x60fe], &[0x4e73]);
        host.registry = crate::hooks::HookRegistry::filtered(
            crate::Revision::Rev02,
            &[],
            &[ids::GET_RANDOM_NUMBER],
        )
        .unwrap();
        host.mode = Mode::Check;
        host.checks = crate::CheckReport::registered(&host.registry);
        let hook = host
            .registry
            .at(ids::GET_RANDOM_NUMBER
                .metadata()
                .address(crate::Revision::Rev02))
            .unwrap()
            .clone();
        (host, hook)
    }
    fn receipt() -> WatchReceipt {
        let mut state = [0; 25];
        state[15] = 0xff7000;
        state[16] = 0xff7ff0;
        state[17] = 0x100;
        state[18] = 0x2300;
        WatchReceipt::from_observed_irq(
            Side::Original,
            IrqIdentity {
                vector: 30,
                handler: 0x200,
                exception_depth: 1,
                entry_master_clock: 100,
                interrupted_state: state,
            },
            vec![DifferingByte {
                address: 0xff7fe8,
                original: 0x12,
                translated: 0x34,
            }],
            StackRegion {
                start: 0xff0000,
                end: 0xff8000,
            },
            &[FrameCommit {
                kind: FrameCommitKind::RegisterSave,
                access: HybridAccess::Write {
                    address: 0xff7fe8,
                    word: false,
                    value: 0x12,
                },
            }],
        )
        .unwrap()
    }
    #[test]
    fn exact_private_graph_debts_survive_nested_scopes_errors_and_public_report_swaps() {
        let (mut host, hook) = machine();
        let outer = host.begin_function_proof_capture().unwrap();
        let first = host
            .arm_future_frame_watch(&hook, "authored model", 1, vec![receipt()])
            .unwrap()
            .unwrap();
        let inner = host.begin_function_proof_capture().unwrap();
        host.arm_future_frame_watch(&hook, "authored nested model", 2, vec![receipt()])
            .unwrap();
        let failed_result: Result<()> = Err(HostError::Metadata("authored analysis error".into()));
        assert!(failed_result.is_err());
        let nested = host.finish_function_proof_capture(inner).unwrap();
        assert_eq!(nested.len(), 1);
        let dependencies = host.finish_function_proof_capture(outer).unwrap();
        assert_eq!(
            dependencies.len(),
            2,
            "outer scope retains its complete prefix and nested dependencies"
        );
        assert!(
            dependencies
                .iter()
                .all(|dependency| dependency.outcome() == DependencyOutcome::Provisional)
        );
        let mut foreign = machine().0;
        let foreign_hook = foreign
            .registry
            .at(hook.symbol.address(crate::Revision::Rev02))
            .unwrap()
            .clone();
        let reused = foreign
            .arm_future_frame_watch(&foreign_hook, "foreign authored model", 3, vec![receipt()])
            .unwrap()
            .unwrap();
        assert_eq!(first, reused);
        let foreign_cell = foreign
            .checks
            .future_frame_provenance
            .previous
            .last()
            .unwrap()
            .watches
            .as_ref()
            .unwrap();
        for entry in &mut foreign_cell.lock().unwrap().receipts {
            entry.outcome = ReceiptOutcome::Discharged;
        }
        host.checks.future_frame_provenance = foreign.checks.future_frame_provenance;
        assert_eq!(
            host.future_frame_approval_outcome(first),
            ApprovalOutcome::Discharged,
            "public numeric lookup now concerns the foreign graph"
        );
        assert!(
            dependencies
                .iter()
                .all(|dependency| dependency.outcome() == DependencyOutcome::Provisional),
            "a reused number/report must not substitute the retained exact graph"
        );
        dependencies[0].watches.lock().unwrap().receipts.clear();
        assert_ne!(
            dependencies[0].outcome(),
            DependencyOutcome::Discharged,
            "missing evidence cannot close"
        );
    }
    #[test]
    fn off_path_does_not_capture_and_only_the_exact_graph_can_discharge_a_dependency() {
        let (mut host, hook) = machine();
        assert!(host.future_observer.proof_capture.is_none());
        host.arm_future_frame_watch(&hook, "uncaptured authored model", 1, vec![receipt()])
            .unwrap();
        assert!(host.future_observer.proof_capture.is_none());
        let scope = host.begin_function_proof_capture().unwrap();
        host.arm_future_frame_watch(&hook, "captured authored model", 2, vec![receipt()])
            .unwrap();
        let dependencies = host.finish_function_proof_capture(scope).unwrap();
        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0].outcome(), DependencyOutcome::Provisional);
        for entry in &mut dependencies[0].watches.lock().unwrap().receipts {
            entry.outcome = ReceiptOutcome::Discharged;
        }
        assert_eq!(dependencies[0].outcome(), DependencyOutcome::Discharged);
        host.reset();
        assert_eq!(
            dependencies[0].outcome(),
            DependencyOutcome::Discharged,
            "reset archives do not replace or reconstruct the retained graph"
        );
    }

    #[test]
    fn partial_tick_returns_foreign_journal_without_applying_or_discharging_watches() {
        use m68000_emu::ReadPurpose;
        let (mut host, hook) = machine();
        let prefix = HybridAccess::Read {
            address: 0xff9000,
            word: true,
            value: 0x1234,
            purpose: ReadPurpose::Semantic,
        };
        let overwrite = HybridAccess::Write {
            address: 0xff7fe8,
            word: false,
            value: 0x34,
        };
        host.emulator.hybrid_bus_mut().hybrid_cpu_accesses = Some(vec![prefix]);
        host.emulator.hybrid_bus_mut().hybrid_exception_boundaries = Some(Vec::new());
        host.future_observer.natural_checks = 1;
        let approval = host
            .arm_future_frame_watch(&hook, "authored partial-tick model", 1, vec![receipt()])
            .unwrap()
            .unwrap();
        let before = host.cpu().hybrid_state_words();
        let observing = host.begin_live_frame_observation().unwrap();
        assert!(observing);
        host.emulator
            .hybrid_bus_mut()
            .hybrid_cpu_accesses
            .as_mut()
            .unwrap()
            .push(overwrite);
        host.abort_live_frame_observation(observing);
        assert!(host.future_observer.branch.is_none());
        assert_eq!(host.cpu().hybrid_state_words(), before);
        assert_eq!(
            host.emulator.hybrid_bus().hybrid_cpu_accesses.as_deref(),
            Some([prefix, overwrite].as_slice())
        );
        assert!(
            host.emulator
                .hybrid_bus()
                .hybrid_exception_boundaries
                .as_ref()
                .unwrap()
                .is_empty()
        );
        assert!(
            matches!(
                host.future_frame_approval_outcome(approval),
                ApprovalOutcome::Provisional { .. }
            ),
            "a partial tick cannot discharge the actual pending graph"
        );
        host.abort_live_frame_observation(false);
    }
}
