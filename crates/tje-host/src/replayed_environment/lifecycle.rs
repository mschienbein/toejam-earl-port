//! Capture only while the ordinary original scheduler progresses. Candidate
//! work is queued at real completion; this module never runs/fast-forwards it.
use super::{
    Profile, RootAdmission, RootKind, Trajectory, Unsupported, UnsupportedKind, admit_root,
    host::ProofRecord,
    inputs::CompilerCache,
    ledger::{CaseKey, Ledger, Requirement},
    record::{Before, Invocation, InvocationId, Journal, Target, Tracker},
    report::{ObligationReport, Report, RootReport},
};
use crate::{HostError, Result, hooks::HookRegistry};
use genesis_core::{GenesisEmulator, GenesisInputs};
use jgenesis_common::frontend::PartialClone;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};
use tje_game::execution::{CallCeilingScope, CallClock, ClockUsage};

pub(super) struct CapturedRoot {
    pub admission: RootAdmission,
    /// PartialClone intentionally has no ROM and no live/reset authority.
    pub analysis_entry: Option<GenesisEmulator>,
    pub inputs: GenesisInputs,
    pub frame: u64,
    pub frame_write_index: usize,
}
struct ActiveRoot {
    root: CapturedRoot,
    scope: CallCeilingScope,
}
pub(super) struct CompletedRoot {
    pub root: CapturedRoot,
    pub original: Arc<Invocation>,
    pub usage: ClockUsage,
    pub exit_ram: Box<[u8; 65536]>,
}
pub(crate) struct State {
    pub(super) trajectory: Trajectory,
    pub(super) ledger: Ledger,
    pub(super) compiler: CompilerCache,
    pub(super) handler_inputs: super::inputs::InterruptedRegisterReads,
    #[cfg(test)]
    pub(super) real_wait: Option<(tje_game::FunctionId, InvocationId)>,
    pub(super) rom_len: usize,
    pub(super) divider: u32,
    pub(super) rows: BTreeMap<&'static str, RootReport>,
    pub(super) proofs: BTreeMap<CaseKey, ProofRecord>,
    pub(super) obligations: Vec<(Requirement, ObligationReport)>,
    pub(super) pending_proofs: BTreeSet<CaseKey>,
    /// Reused transaction bookkeeping, independent of conditional-only pending.
    /// Cleared only after successful diagnostic publication, never by a DTO.
    pub(super) unpublished_cases: Vec<CaseKey>,
    pub(super) changed_rows: BTreeSet<&'static str>,
    pub(super) completed_replayed: u64,
    pub(super) published_obligations: usize,
    pub(super) reference_completions: u64,
    pub(super) own_matches: u64,
    pub(super) substitutions: u64,
    pub(super) mismatches: u64,
    pub(super) unsupported: u64,
    pub(super) finalized: bool,
    capture_broken: bool,
    journal: Journal,
    tracker: Tracker,
    clock: CallClock,
    active: BTreeMap<InvocationId, ActiveRoot>,
    completed: VecDeque<CompletedRoot>,
    error: Option<HostError>,
    infrastructure_failure: Option<Unsupported>,
    pending_tick: Option<Before>,
    pub(super) failed: bool,
    pub(super) replay_attempts: u64,
    pub(super) checked_requested: BTreeSet<tje_game::FunctionId>,
    pub(super) requested: BTreeSet<tje_game::FunctionId>,
}
impl State {
    pub fn new(registry: &HookRegistry, rom_len: usize, divider: u32) -> Result<(Self, Report)> {
        if cfg!(panic = "abort") || !cfg!(feature = "reference-proof-hooks") {
            return Err(HostError::ReplayUnsupported { function: "replay registry".into(),
                reason: Unsupported { kind: UnsupportedKind::PlatformCapability,
                    detail: "recorded infallible bus mismatches require an unwind-enabled native analysis build with reference-proof-hooks actual-read contexts".into() } });
        }
        let trajectory = Trajectory::default();
        let report = Report::registered(trajectory, registry).map_err(|reason| {
            HostError::ReplayUnsupported {
                function: "replay registry".into(),
                reason,
            }
        })?;
        let rows = report.by_function.clone();
        let requested = registry
            .iter()
            .filter(|hook| {
                hook.proof_envelope == tje_game::execution::ProofEnvelope::ReplayedEnvironment
            })
            .map(|hook| hook.symbol.id)
            .collect();
        Ok((
            Self {
                trajectory,
                ledger: Ledger::new(trajectory),
                compiler: CompilerCache::default(),
                handler_inputs: BTreeMap::new(),
                #[cfg(test)]
                real_wait: None,
                rom_len,
                divider,
                rows,
                proofs: BTreeMap::new(),
                obligations: Vec::new(),
                reference_completions: 0,
                own_matches: 0,
                substitutions: 0,
                mismatches: 0,
                unsupported: 0,
                finalized: false,
                capture_broken: false,
                pending_proofs: BTreeSet::new(),
                unpublished_cases: Vec::new(),
                changed_rows: BTreeSet::new(),
                completed_replayed: 0,
                published_obligations: 0,
                journal: Journal::default(),
                tracker: Tracker::new(trajectory, registry.revision()),
                clock: CallClock::default(),
                active: BTreeMap::new(),
                completed: VecDeque::new(),
                error: None,
                infrastructure_failure: None,
                pending_tick: None,
                failed: false,
                replay_attempts: 0,
                checked_requested: BTreeSet::new(),
                requested,
            },
            report,
        ))
    }
    /// Diagnostic ownership/observation failures are fatal to this trajectory,
    /// independently of the policy for a translated root's proof failure.
    pub(super) fn latch_infrastructure_failure(&mut self, reason: Unsupported) {
        self.infrastructure_failure.get_or_insert(reason);
        self.failed = true;
        self.capture_broken = true;
    }
    pub(super) fn infrastructure_failure(&self) -> Option<&Unsupported> {
        self.infrastructure_failure.as_ref()
    }
    fn failure(&mut self, report: &mut Report, function: impl Into<String>, reason: Unsupported) {
        self.failed = true;
        self.unsupported += 1;
        report.unsupported += 1;
        report.last_verdict = Some(super::Verdict::Unsupported);
        report.last_unsupported = Some(reason.clone());
        self.error.get_or_insert(HostError::ReplayUnsupported {
            function: function.into(),
            reason,
        });
    }
    pub fn before_tick(
        &mut self,
        emulator: &mut GenesisEmulator,
        frame: u64,
        report: &mut Report,
    ) -> Result<()> {
        if let Some(reason) = self.infrastructure_failure() {
            return Err(HostError::ReplayUnsupported {
                function: "actual original replay infrastructure".into(),
                reason: reason.clone(),
            });
        }
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        if self.clock.active()
            && let Err(error) = self.clock.before_tick()
        {
            self.capture_broken = true;
            let reason = Unsupported {
                kind: UnsupportedKind::OperationalCeiling,
                detail: format!("operational ceiling, not a bound: {error}"),
            };
            self.failure(report, "actual original call scope", reason);
            return Err(self.error.take().expect("recorded operational failure"));
        }
        let before = self.journal.begin(emulator, frame).map_err(|reason| {
            self.latch_infrastructure_failure(reason.clone());
            HostError::ReplayUnsupported {
                function: "actual original journal".into(),
                reason,
            }
        })?;
        self.pending_tick = Some(before);
        Ok(())
    }
    /// A partial scheduler tick has no completed instruction/frame witness.
    /// Restore only diagnostic loans; actual CPU/device state stays untouched
    /// and interrupted calls remain open, never converted into completions.
    pub(super) fn abort_tick(&mut self, emulator: &mut GenesisEmulator) {
        if let Some(before) = self.pending_tick.take()
            && let Err(reason) = self.journal.restore(emulator, before)
        {
            self.latch_infrastructure_failure(reason.clone());
            self.error.get_or_insert(HostError::ReplayUnsupported {
                function: "aborted original journal cleanup".into(),
                reason,
            });
        }
        self.capture_broken = true;
        self.failed = true;
    }
    pub fn after_tick(
        &mut self,
        emulator: &mut GenesisEmulator,
        registry: &HookRegistry,
        disabled: &BTreeSet<u32>,
        rendered: bool,
        inputs: GenesisInputs,
        report: &mut Report,
    ) {
        self.after_tick_with_cleanup(
            emulator,
            registry,
            disabled,
            (rendered, inputs),
            report,
            Journal::restore,
        );
    }
    /// The same live phase and owned token, with no alternate evidence path.
    /// Production delegates only to the actual Journal restore operation.
    pub(super) fn after_tick_with_cleanup(
        &mut self,
        emulator: &mut GenesisEmulator,
        registry: &HookRegistry,
        disabled: &BTreeSet<u32>,
        outcome: (bool, GenesisInputs),
        report: &mut Report,
        cleanup: impl FnOnce(
            &mut Journal,
            &mut GenesisEmulator,
            Before,
        ) -> std::result::Result<(), Unsupported>,
    ) {
        let (rendered, inputs) = outcome;
        let before = self
            .pending_tick
            .take()
            .expect("one actual original step per journal lease");
        let frame = before.frame();
        let revision = registry.revision();
        let queued_before = self.completed.len();
        let mut journal_failure = None;
        let observed = (|| {
            let step = self
                .journal
                .observe(emulator, frame + u64::from(rendered), &before)
                .inspect_err(|reason| journal_failure = Some(reason.clone()))?;
            if self.clock.active() {
                self.clock
                    .record(ClockUsage {
                        ticks: 1,
                        instructions: step
                            .after
                            .instructions
                            .checked_sub(step.before.instructions)
                            .ok_or_else(|| Unsupported {
                                kind: UnsupportedKind::Observation,
                                detail: "original opcode counter moved backwards".into(),
                            })?,
                        master_clocks: step
                            .after
                            .master_clock
                            .checked_sub(step.before.master_clock)
                            .ok_or_else(|| Unsupported {
                                kind: UnsupportedKind::Observation,
                                detail: "original master clock moved backwards".into(),
                            })?,
                        frames: u64::from(rendered),
                        dispatch_steps: 0,
                    })
                    .map_err(|error| Unsupported {
                        kind: UnsupportedKind::OperationalCeiling,
                        detail: format!("operational ceiling, not a bound: {error}"),
                    })?;
            }
            let pc = step.after.cpu.pc();
            let hook = registry.at(pc);
            let target = Target {
                function: hook
                    .map(|hook| hook.symbol.id)
                    .or_else(|| tje_game::function_at(revision, pc)),
                wanted: hook.is_some() && !disabled.contains(&pc),
            };
            self.tracker.observe(emulator, &step, target)
        })();
        if let Some(reason) = journal_failure {
            self.latch_infrastructure_failure(reason);
        }
        match observed {
            Err(reason) => {
                self.capture_broken = true;
                self.tracker.reject(reason.clone());
                self.failure(report, "actual original invocation capture", reason);
            }
            Ok(progress) => {
                if let Some(entry) = progress.entered
                    && let Some(hook) = registry
                        .at(entry.boundary.cpu.pc())
                        .filter(|hook| !disabled.contains(&hook.symbol.address(revision)))
                {
                    self.changed_rows.insert(hook.symbol.inventory_id);
                    let row = self
                        .rows
                        .get_mut(hook.symbol.inventory_id)
                        .expect("registered replay evidence row");
                    row.entered += 1;
                    let admitted = (|| {
                        let binding = registry.replay_binding(hook.symbol.id).ok_or_else(|| {
                            super::unsupported(
                                hook,
                                UnsupportedKind::CalleeCoverage,
                                "root has no sealed effective dispatcher route",
                            )
                        })?;
                        if binding.body() != hook.body_identity() {
                            return Err(super::unsupported(
                                hook,
                                UnsupportedKind::CalleeCoverage,
                                "root payload/version changed after the effective route was sealed; rebuild the registry/trajectory",
                            ));
                        }
                        // Only this admission point may consume stronger live
                        // entry input, BEFORE clone/public journal restoration.
                        let admission = admit_root(
                            hook,
                            revision,
                            emulator,
                            self.trajectory,
                            Profile::OrdinaryRts,
                            &entry,
                        )?;
                        let scope = self
                            .clock
                            .enter(hook.symbol.id, hook.clock_ceiling)
                            .map_err(|error| {
                                super::unsupported(
                                    hook,
                                    UnsupportedKind::OperationalCeiling,
                                    format!("operational ceiling admission: {error}"),
                                )
                            })?;
                        let frame_write_index = if rendered {
                            0
                        } else {
                            emulator
                                .hybrid_bus()
                                .hybrid_writes
                                .as_ref()
                                .map_or(0, Vec::len)
                        };
                        let analysis_entry = emulator.partial_clone();
                        Ok(ActiveRoot {
                            root: CapturedRoot {
                                admission,
                                analysis_entry: Some(analysis_entry),
                                inputs,
                                frame: entry.boundary.frame,
                                frame_write_index,
                            },
                            scope,
                        })
                    })();
                    match admitted {
                        Ok(root) => {
                            self.rows
                                .get_mut(hook.symbol.inventory_id)
                                .expect("registered row")
                                .open += 1;
                            if root.root.admission.identity().kind == RootKind::Replayed {
                                self.replay_attempts += 1;
                                report.attempted += 1;
                                report.open_segments += 1;
                            }
                            self.active.insert(entry.identity, root);
                        }
                        Err(error) => {
                            self.rows
                                .get_mut(hook.symbol.inventory_id)
                                .expect("registered row")
                                .unsupported += 1;
                            let reason = match error {
                                HostError::ReplayUnsupported { reason, .. } => reason,
                                error => Unsupported {
                                    kind: UnsupportedKind::Observation,
                                    detail: error.to_string(),
                                },
                            };
                            self.failure(report, hook.symbol.name, reason);
                        }
                    }
                }
                if let Some(original) = progress.completed
                    && let Some(active) = self.active.remove(&original.entry.identity)
                {
                    let root = active.root;
                    self.changed_rows
                        .insert(root.admission.hook().symbol.inventory_id);
                    let row = self
                        .rows
                        .get_mut(root.admission.hook().symbol.inventory_id)
                        .expect("registered row");
                    row.reference_completed += 1;
                    row.open -= 1;
                    if root.admission.identity().kind == RootKind::Replayed {
                        self.reference_completions += 1;
                        report.reference_completions += 1;
                        report.open_segments -= 1;
                    }
                    match self.clock.leave(active.scope) {
                        Ok(usage) => self.completed.push_back(CompletedRoot {
                            root,
                            original,
                            usage,
                            exit_ram: super::record::raw_ram(emulator),
                        }),
                        Err(error) => self.failure(
                            report,
                            root.admission.hook().symbol.name,
                            Unsupported {
                                kind: UnsupportedKind::OperationalCeiling,
                                detail: format!("operational ceiling at actual return: {error}"),
                            },
                        ),
                    }
                }
            }
        }
        // This happens only after every actual entry was admitted/recorded.
        // Restoring diagnostics cannot refresh or copy live/reset authority.
        if let Err(reason) = cleanup(&mut self.journal, emulator, before) {
            self.latch_infrastructure_failure(reason.clone());
            self.tracker.reject(reason.clone());
            // No analysis/completion may be minted from a phase whose
            // diagnostic ownership failed to close. Earlier queued work stays.
            self.completed.truncate(queued_before);
            if self.error.is_none() {
                self.failure(report, "actual original journal cleanup", reason);
            }
        }
    }
    pub(super) fn next_completed(&mut self) -> Option<CompletedRoot> {
        self.completed.pop_front()
    }
    #[cfg(all(test, feature = "reference-proof-hooks"))]
    pub(super) fn peek_completed(&self) -> Option<&CompletedRoot> {
        self.completed.front()
    }
    pub fn take_error(&mut self) -> Option<HostError> {
        self.error.take()
    }
    pub fn has_pending(&self) -> bool {
        !self.completed.is_empty() || self.error.is_some()
    }
    pub fn has_open_requested(&self) -> bool {
        self.active
            .values()
            .any(|active| active.root.admission.identity().kind == RootKind::Replayed)
    }
    pub fn finalized(&self) -> bool {
        self.finalized
    }
    pub fn collecting(&self) -> bool {
        !self.capture_broken && !self.finalized
    }
}

impl State {
    pub(crate) fn after_actual_reset(&mut self) -> Unsupported {
        self.failed = true;
        self.capture_broken = true;
        self.finalized = true;
        self.unsupported += 1;
        self.active.clear();
        self.completed.clear();
        self.clock = CallClock::default();
        Unsupported { kind: UnsupportedKind::TypedControlProfile,
            detail: "actual Host reset ended the ordinary replay trajectory; prior open/conditional proofs remain ineligible, and a fresh Host is required for another campaign".into() }
    }
}
