//! Deferred analysis of genuine completed live invocations. No original tick,
//! interrupt, input boundary or live CPU state is replayed by this driver.
use super::{
    Failure, RootAdmission, RootKind, State, Unsupported, UnsupportedKind, Verdict,
    context::{CompletedAnalysis, ReplayContext},
    ledger::{CaseKey, OutputCoverage, Requirement, Tier},
    lifecycle::{CapturedRoot, CompletedRoot},
    record::{self, EntryKind, Invocation},
    report::{
        BodyReport, ComparisonDetail, CompletedProofReport, ContractReport, FailurePacket,
        ObligationStatus, OutputsReport, Report, WitnessReport,
    },
};
use crate::future_frame_watch::ProofDependency;
use crate::{
    CheckFailure, CheckFailurePolicy, Host, HostError, Mode, Result,
    abi::ReturnState,
    check_failure::bounded_detail,
    fixture::FrameProgress,
    sinks::{HeadlessAudio, HeadlessOutput, NoCartridgeSaves},
};
use genesis_core::GenesisInputs;
use std::{
    collections::BTreeSet,
    panic::{AssertUnwindSafe, catch_unwind},
};
use tje_engine_core::{dependencies::StateMask, ir::StateField};
use tje_game::TranslatedEntry;

pub(super) struct StrictObservation {
    candidate: super::CandidateObservation,
    provenance: Vec<ProofDependency>,
}

pub(super) struct ProofRecord {
    inventory_id: &'static str,
    replayed: bool,
    outputs: OutputCoverage,
    evidence: CompletedProofReport,
}
impl State {
    fn record_match(
        &mut self,
        admission: &RootAdmission,
        original: &Invocation,
        outputs: OutputCoverage,
        requirements: Vec<Requirement>,
        obligations: Vec<super::ObligationReport>,
        provenance: Vec<ProofDependency>,
    ) -> std::result::Result<(), Unsupported> {
        let key = CaseKey::root(admission, original)?;
        let replayed = admission.identity().kind == RootKind::Replayed;
        self.ledger.own_match(
            key,
            if replayed {
                Tier::Replayed
            } else {
                Tier::Strict
            },
            outputs,
            requirements.clone(),
            provenance,
        )?;
        self.obligations
            .extend(requirements.into_iter().zip(obligations));
        let inventory_id = admission.hook().symbol.inventory_id;
        self.changed_rows.insert(inventory_id);
        let row = self
            .rows
            .get_mut(inventory_id)
            .expect("registered independent root");
        row.own_matches += 1;
        if self.ledger.complete(key) {
            row.passed += 1;
            if replayed {
                self.completed_replayed += 1;
            }
        } else {
            row.provisional += 1;
            self.pending_proofs.insert(key);
        }
        if replayed {
            self.own_matches += 1;
        }
        self.unpublished_cases.push(key);
        self.proofs.insert(
            key,
            ProofRecord {
                inventory_id,
                replayed,
                outputs,
                evidence: CompletedProofReport {
                    witness: WitnessReport::completed(original),
                    body: BodyReport::admitted(admission),
                    effective_contract: ContractReport::registered(
                        admission.hook(),
                        admission.identity().profile,
                    ),
                    checked_outputs: OutputsReport::covered(&outputs),
                    result: "completed_successful",
                },
            },
        );
        Ok(())
    }
    fn publish(&mut self, report: &mut Report, finalizing: bool) {
        self.pending_proofs.retain(|key| {
            if !self.ledger.complete(*key) {
                return true;
            }
            let proof = self.proofs.get(key).expect("independent pending proof");
            let row = self
                .rows
                .get_mut(proof.inventory_id)
                .expect("registered proof row");
            row.passed += 1;
            row.provisional -= 1;
            self.changed_rows.insert(proof.inventory_id);
            if proof.replayed {
                self.completed_replayed += 1;
            }
            false
        });
        if finalizing {
            self.completed_replayed = 0;
            for row in self.rows.values_mut() {
                row.passed = 0;
                row.provisional = 0;
            }
            for (&key, proof) in &self.proofs {
                let row = self
                    .rows
                    .get_mut(proof.inventory_id)
                    .expect("registered final proof row");
                if self.ledger.complete(key) {
                    row.passed += 1;
                    if proof.replayed {
                        self.completed_replayed += 1;
                    }
                } else {
                    row.provisional += 1;
                }
            }
        }
        report.trajectory_id = self.trajectory.0;
        report.schema_version = super::report::SCHEMA_VERSION;
        report.label = super::report::LABEL;
        report.attempted = self.replay_attempts;
        report.reference_completions = self.reference_completions;
        report.own_matches = self.own_matches;
        report.completed_proofs = self.completed_replayed;
        report.substitutions = self.substitutions;
        report.open_segments = self.rows.values().map(|row| row.open).sum();
        report.unsupported = self.unsupported;
        report.mismatches = self.mismatches;
        report.unresolved_obligations = self.ledger.unresolved_count();
        report.finalized = self.finalized;
        report.complete = false;
        if finalizing {
            report.by_function.clone_from(&self.rows);
            report.obligations.clear();
            self.published_obligations = 0;
        } else {
            for id in &self.changed_rows {
                report.by_function.insert(
                    id,
                    self.rows.get(id).expect("changed registered row").clone(),
                );
            }
        }
        self.changed_rows.clear();
        for (requirement, diagnostic) in self.obligations.iter().skip(self.published_obligations) {
            let mut diagnostic = diagnostic.clone();
            match requirement {
                Requirement::Checked {
                    case,
                    tier,
                    outputs,
                } if self.ledger.satisfies(*case, *tier, *outputs) => {
                    diagnostic.status = ObligationStatus::Discharged;
                    diagnostic.discharged_by =
                        self.proofs.get(case).map(|proof| proof.evidence.clone());
                    debug_assert!(diagnostic.discharged_by.is_some());
                }
                Requirement::Checked { case, .. } => {
                    if let Some(reason) = self.ledger.failed(*case) {
                        diagnostic.status = ObligationStatus::Failed;
                        diagnostic.reason = Some(reason.detail.clone());
                    } else if let Some(proof) = self.proofs.get(case) {
                        diagnostic.reason = Some(format!(
                            "same-invocation child own-check exists but its measured output/callee closure is insufficient (state={:?}, nonframe_ram={}, ordered_device_effects={})",
                            proof.outputs.state,
                            proof.outputs.nonframe_ram,
                            proof.outputs.ordered_device_effects
                        ));
                    }
                }
                Requirement::Uncovered { detail, .. } => {
                    diagnostic.status = ObligationStatus::Unsupported;
                    diagnostic.reason = Some(detail.clone());
                }
            }
            report.obligations.push(diagnostic);
        }
        self.published_obligations = self.obligations.len();
    }

    fn failed_publication(&mut self, detail: String) {
        let reason = Unsupported {
            kind: UnsupportedKind::PlatformCapability,
            detail,
        };
        self.latch_infrastructure_failure(reason.clone());
        // Retain own-match evidence and prior published good cases. Only this
        // still-unpublished approval batch becomes ineligible.
        for &case in &self.unpublished_cases {
            let _ = self.ledger.fail(case, reason.clone());
        }
    }
}

/// The one production publisher, also used by the private fault controls.
pub(super) fn publish_replayed_state(
    state: &mut State,
    checks: &mut crate::CheckReport,
) -> Result<()> {
    let report = checks
        .replayed_environment
        .as_mut()
        .ok_or_else(|| HostError::Metadata("replay output report was removed".into()))?;
    state.publish(report, false);
    Ok(())
}

fn returned_state(values: &[u32; StateField::COUNT], original: &Invocation) -> ReturnState {
    let return_pc = match original.entry.kind {
        EntryKind::Call { return_pc, .. } => return_pc,
        EntryKind::Interrupt { .. } => unreachable!("ordinary admission requires an actual call"),
    };
    ReturnState {
        data: std::array::from_fn(|index| values[index]),
        address: std::array::from_fn(|index| values[index + 8]),
        usp: values[StateField::Usp.index() as usize],
        ssp: values[StateField::Ssp.index() as usize],
        pc: return_pc,
        status: values[StateField::Sr.index() as usize] as u16,
        exception_depth: original.entry.boundary.cpu.exception_depth,
    }
}
fn reference_state(original: &Invocation) -> ReturnState {
    let values = &original.exit.cpu.words;
    ReturnState {
        data: std::array::from_fn(|index| values[index]),
        address: std::array::from_fn(|index| values[index + 8]),
        usp: values[StateField::Usp.index() as usize],
        ssp: values[StateField::Ssp.index() as usize],
        pc: original.exit.cpu.pc(),
        status: original.exit.cpu.sr() as u16,
        exception_depth: original.exit.cpu.exception_depth,
    }
}
fn abi_json(state: &ReturnState, abi: &tje_game::Abi) -> serde_json::Value {
    let mut registers = Vec::new();
    for index in 0..8 {
        if abi.strict_registers
            || abi.preserved_data & (1 << index) != 0
            || index == 0 && abi.returns_value
        {
            registers.push(serde_json::json!({"field":format!("D{index}"),"bits":u32::MAX,"value":state.data[index]}));
        }
    }
    for index in 0..7 {
        if abi.strict_registers || abi.preserved_address & (1 << index) != 0 {
            registers.push(serde_json::json!({"field":format!("A{index}"),"bits":u32::MAX,"value":state.address[index]}));
        }
    }
    let status_mask = if abi.strict_registers { 0xffff } else { 0xffe0 };
    serde_json::json!({"pc":state.pc,"sp":state.sp(),"usp":state.usp,"ssp":state.ssp,
        "exception_depth":state.exception_depth,"status_mask":status_mask,"status":state.status & status_mask,
        "registers":registers})
}
fn measured_outputs(analysis: &CompletedAnalysis, original: &Invocation) -> OutputCoverage {
    let mut state = StateMask::EMPTY;
    for (field, observed) in analysis.observed.iter() {
        state.insert(
            field,
            observed
                & !(analysis.values[field.index() as usize]
                    ^ original.exit.cpu.words[field.index() as usize]),
        );
    }
    OutputCoverage {
        state,
        nonframe_ram: true,
        ordered_device_effects: true,
    }
}
struct AnalysisAttempt {
    executed: bool,
    position: Option<(Option<u32>, Option<u16>, u64)>,
    result: std::result::Result<CompletedAnalysis, Failure>,
    /// Carried only to restore leases/account first cause before resuming this
    /// exact owned payload. It is never turned into an API error or a proof.
    unwind: Option<Box<dyn std::any::Any + Send>>,
}
fn analyze(
    completed: &CompletedRoot,
    registry: &crate::hooks::HookRegistry,
    state: &mut State,
    reader: &dyn Fn(u32) -> Option<u16>,
    poison: u8,
) -> AnalysisAttempt {
    let root = &completed.root;
    let original = &completed.original;
    let admission = &root.admission;
    let mut executed = false;
    let mut position = None;
    let mut unwind = None;
    let result = (|| {
        let derived = RootAdmission {
            hook: admission.hook().clone(),
            selection: admission.selection(),
            identity: admission.derived_identity(),
        };
        let ram = record::raw_ram(
            root.analysis_entry
                .as_ref()
                .expect("retained actual entry fork"),
        );
        let options = super::context::AnalysisOptions {
            divider: state.divider,
            rom_len: state.rom_len,
            poison,
            actual_usage: completed.usage,
            handler_inputs: &mut state.handler_inputs,
        };
        let mut context = ReplayContext::new(
            &derived,
            original,
            ram,
            registry,
            &mut state.compiler,
            reader,
            options,
        )?;
        executed = true;
        let result = catch_unwind(AssertUnwindSafe(|| match admission.selection().entry() {
            TranslatedEntry::Values(_) => admission.selection().invoke_values(&mut context, &[]),
            TranslatedEntry::Framed(_) => admission.selection().invoke_prepared(&mut context),
        }));
        position = Some(context.position());
        #[cfg(test)]
        if poison == 0 && original.entry.parent.is_none() && context.waited() {
            state.real_wait = Some((admission.identity().function, original.entry.identity));
        }
        // A sticky proof failure is not permission to swallow another panic.
        // Only the private marker from this exact derived context is converted.
        let result = match result {
            Ok(result) => result,
            Err(payload) => {
                if !context.owns_abort(payload.as_ref()) {
                    unwind = Some(payload);
                    return Err(context.sticky_failure().unwrap_or_else(|| Failure::Unsupported(
                        Unsupported { kind: UnsupportedKind::PlatformCapability,
                            detail: "unrelated readable-body unwind; exact payload will resume after cleanup, no completed proof".into() })));
                }
                return Err(context.sticky_failure().unwrap_or_else(|| {
                    Failure::Unsupported(Unsupported {
                        kind: UnsupportedKind::PlatformCapability,
                        detail: "owned analysis abort lacked its required sticky failure".into(),
                    })
                }));
            }
        };
        if let Some(failure) = context.sticky_failure() {
            return Err(failure);
        }
        let returned = match result {
            Ok(value) => value,
            Err(error) => {
                if let Some(reason) = error.unsupported_stack_read() {
                    return Err(Failure::StackRead(reason));
                }
                return Err(Failure::Mismatch(format!(
                    "readable body returned an error after the actual original's ordinary return: {error}"
                )));
            }
        };
        let analysis = context.finish(returned, &completed.exit_ram, &derived)?;
        let expected = reference_state(original);
        let actual = returned_state(&analysis.values, original);
        let abi = &admission.hook().symbol.abi;
        expected.compare(&actual, abi).map_err(|detail| {
            Failure::Compared(Box::new(ComparisonDetail {
                point: "abi_return",
                original_pc: Some(original.exit.cpu.pc()),
                original_opcode: None,
                own_access_index: None,
                expected: Some(abi_json(&expected, abi)),
                actual: Some(abi_json(&actual, abi)),
                detail,
            }))
        })?;
        Ok(analysis)
    })();
    AnalysisAttempt {
        executed,
        position,
        result,
        unwind,
    }
}

impl Host {
    fn ensure_replayed_infrastructure(&self) -> Result<()> {
        if let Some(reason) = self
            .replayed_state
            .as_ref()
            .and_then(State::infrastructure_failure)
        {
            return Err(HostError::ReplayUnsupported {
                function: "actual original replay infrastructure".into(),
                reason: reason.clone(),
            });
        }
        Ok(())
    }
    pub(crate) fn ensure_replayed_lifecycle(&self) -> Result<()> {
        self.ensure_replayed_infrastructure()?;
        if self.replayed_state.as_ref().is_some_and(State::finalized) {
            return Err(HostError::Metadata("replay trajectory ended/finalized; a fresh Host is required before another original tick".into()));
        }
        Ok(())
    }
    pub(crate) fn begin_replayed_tick(&mut self) -> Result<bool> {
        if self.mode != Mode::Check {
            return Ok(false);
        }
        self.ensure_replayed_infrastructure()?;
        let Some(state) = &mut self.replayed_state else {
            return Ok(false);
        };
        if !state.collecting() {
            return Ok(false);
        }
        let report = self
            .checks
            .replayed_environment
            .as_mut()
            .ok_or_else(|| HostError::Metadata("replay output report was removed".into()))?;
        state.before_tick(&mut self.emulator, self.frame, report)?;
        Ok(true)
    }
    pub(crate) fn finish_replayed_tick(
        &mut self,
        observing: bool,
        rendered: bool,
        inputs: GenesisInputs,
    ) -> Result<()> {
        if !observing {
            return self.ensure_replayed_infrastructure();
        }
        let state = self
            .replayed_state
            .as_mut()
            .expect("active replay trajectory");
        let report = self
            .checks
            .replayed_environment
            .as_mut()
            .expect("active replay output");
        state.after_tick(
            &mut self.emulator,
            &self.registry,
            &self.disabled_check_roots,
            rendered,
            inputs,
            report,
        );
        self.ensure_replayed_infrastructure()
    }
    pub(crate) fn abort_replayed_tick(&mut self, observing: bool) {
        if observing {
            self.replayed_state
                .as_mut()
                .expect("active replay trajectory")
                .abort_tick(&mut self.emulator);
        }
    }

    fn strict_replayed_child(
        &mut self,
        completed: &mut CompletedRoot,
    ) -> Result<StrictObservation> {
        let request = super::CandidateRequest::new(&completed.root.admission, &completed.original)
            .map_err(|reason| HostError::ReplayUnsupported {
                function: completed.root.admission.hook().symbol.name.into(),
                reason,
            })?;
        let expected = request.clone();
        self.with_strict_child_analysis(&mut completed.root, |analysis, hook| {
            let (_, candidate) = analysis.check_hook_observed(hook, Some(request))?;
            let candidate = candidate.ok_or_else(|| {
                HostError::Metadata(
                    "strict child omitted its required measured candidate observation".into(),
                )
            })?;
            if !candidate.matches_request(&expected) {
                return Err(HostError::Metadata(
                    "strict candidate observation came from another analysis fork/body/entry"
                        .into(),
                ));
            }
            Ok(candidate)
        })
    }

    /// Private monomorphized execution seam: the production callback above is
    /// unchanged. Fault controls exercise real admitted-child loan/capture
    /// cleanup here without fabricating a canonical selection or proof.
    pub(super) fn with_strict_child_analysis(
        &mut self,
        root: &mut CapturedRoot,
        execute: impl FnOnce(&mut Host, &crate::hooks::Hook) -> Result<super::CandidateObservation>,
    ) -> Result<StrictObservation> {
        // This is a private analysis Host, not a state installed into live play.
        // Only the cached analysis asset is leased; the live cartridge and its
        // source origin are never mutably borrowed or replaced.
        let mut clock = self.wait_clock.clone();
        clock.frame = root.frame;
        clock.inputs = root.inputs;
        let mut analysis = Host {
            emulator: root
                .analysis_entry
                .take()
                .expect("one owned analysis entry"),
            analysis_rom: None,
            registry: self.registry.clone(),
            mode: Mode::Check,
            revision: self.revision,
            symbols: crate::catalogue::RevisionSymbols::for_revision(self.revision),
            frame: root.frame,
            main_idle: false,
            frame_progress: FrameProgress::default(),
            terminal_entry: None,
            check_failure_policy: CheckFailurePolicy::FailFast,
            disabled_check_roots: BTreeSet::new(),
            future_observer: Default::default(),
            readable_stack: (cfg!(feature = "ram-dma-observer") && cfg!(panic = "unwind"))
                .then(crate::readable_stack::Pool::new),
            output: HeadlessOutput,
            audio: HeadlessAudio::default(),
            saves: NoCartridgeSaves,
            audio_sample_rate: self.audio_sample_rate,
            session_trace: None,
            wait_clock: clock,
            timing_services: self.timing_services.take(),
            replayed_state: None,
            checks: std::mem::take(&mut self.checks),
            measurements: std::mem::take(&mut self.measurements),
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.with_fork_rom(&mut analysis, |analysis| {
                let scope = analysis.begin_function_proof_capture()?;
                let checked = catch_unwind(AssertUnwindSafe(|| {
                    execute(analysis, root.admission.hook())
                }));
                let provenance = analysis.finish_function_proof_capture(scope);
                match checked {
                    Ok(Ok(candidate)) => provenance.map(|provenance| StrictObservation {
                        candidate,
                        provenance,
                    }),
                    Ok(Err(error)) => Err(error),
                    Err(payload) => std::panic::resume_unwind(payload),
                }
            })
        }));
        self.checks = analysis.checks;
        self.measurements = analysis.measurements;
        self.timing_services = analysis.timing_services;
        root.analysis_entry = Some(analysis.emulator);
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn check_completed_replay(
        &mut self,
        completed: &mut CompletedRoot,
        state: &mut State,
    ) -> Result<()> {
        let hook = completed.root.admission.hook().clone();
        let calls_before = self.checks.calls;
        state.changed_rows.insert(hook.symbol.inventory_id);
        let mismatches_before = self.checks.mismatches;
        let mut position = None;
        let mut comparison = None;
        let mut body_unwind = None;
        let checked = catch_unwind(AssertUnwindSafe(|| {
            if let Some(reason) = &completed.original.unsupported {
                Err(HostError::ReplayUnsupported {
                    function: hook.symbol.name.into(),
                    reason: reason.clone(),
                })
            } else if completed.root.admission.identity().kind == RootKind::Replayed
                && completed
                    .original
                    .trace
                    .instructions
                    .iter()
                    .any(|instruction| {
                        super::ordinary_control_instruction(instruction.actual.opcode)
                    })
            {
                Err(super::unsupported(
                    &hook,
                    UnsupportedKind::TypedControlProfile,
                    "own privileged instruction class requires its explicit control profile, even with an unchanged endpoint",
                ))
            } else if completed.root.admission.identity().kind == RootKind::Replayed
                && completed.original.exit.dma.unresolved()
            {
                Err(super::unsupported(
                    &hook,
                    UnsupportedKind::PlatformCapability,
                    format!(
                        "ordinary replay has unresolved DMA/command debt at actual return; no causal ownership/completion witness: {:?}",
                        completed.original.exit.dma
                    ),
                ))
            } else {
                match completed.root.admission.identity().kind {
                    RootKind::StrictCanonical => {
                        let guard = if hook.check_route().progresses_time() {
                            Ok(super::inputs::FrozenInterruptRequirements::default())
                        } else {
                            let rom_len = state.rom_len;
                            let reader = |pc: u32| {
                                if pc & 1 != 0
                                    || (pc as usize)
                                        .checked_add(1)
                                        .is_none_or(|last| last >= rom_len)
                                {
                                    return None;
                                }
                                self.emulator
                                    .hybrid_bus()
                                    .cartridge
                                    .as_ref()
                                    .map(|cartridge| cartridge.peek_word(pc))
                            };
                            super::inputs::frozen_interrupt_guard(
                                &completed.original,
                                &reader,
                                state.divider,
                                &mut state.compiler,
                            )
                        };
                        guard
                            .map_err(|failure| {
                                comparison = failure.comparison().cloned();
                                failure.into_host_error(
                                    hook.symbol.name,
                                    completed.original.entry.boundary.frame,
                                )
                            })
                            .and_then(|requirements| {
                                let observed = self.strict_replayed_child(completed)?;
                                requirements
                                    .verify(
                                        &observed.candidate,
                                        &completed.root.admission,
                                        &completed.original,
                                    )
                                    .map_err(|failure| {
                                        comparison = failure.comparison().cloned();
                                        failure.into_host_error(
                                            hook.symbol.name,
                                            completed.original.entry.boundary.frame,
                                        )
                                    })?;
                                Ok(observed)
                            })
                            .and_then(|observed| {
                                state
                                    .rows
                                    .get_mut(hook.symbol.inventory_id)
                                    .expect("strict root row")
                                    .checked += 1;
                                let outputs = super::strict::coverage(
                                    &completed.original,
                                    &observed.candidate,
                                );
                                state
                                    .record_match(
                                        &completed.root.admission,
                                        &completed.original,
                                        outputs,
                                        Vec::new(),
                                        Vec::new(),
                                        observed.provenance,
                                    )
                                    .map_err(|reason| HostError::ReplayUnsupported {
                                        function: hook.symbol.name.into(),
                                        reason,
                                    })
                            })
                    }
                    RootKind::Replayed => {
                        // Immutable cartridge word reads borrow the one live ROM; no
                        // second ROM image, interpreter or decoded cartridge is made.
                        let rom_len = state.rom_len;
                        let reader = |pc: u32| {
                            if pc & 1 != 0
                                || (pc as usize)
                                    .checked_add(1)
                                    .is_none_or(|last| last >= rom_len)
                            {
                                return None;
                            }
                            self.emulator
                                .hybrid_bus()
                                .cartridge
                                .as_ref()
                                .map(|cartridge| cartridge.peek_word(pc))
                        };
                        let attempt = analyze(completed, &self.registry, state, &reader, 0);
                        position = attempt.position;
                        body_unwind = attempt.unwind;
                        if attempt.executed {
                            state
                                .rows
                                .get_mut(hook.symbol.inventory_id)
                                .expect("executed replayed root row")
                                .checked += 1;
                            state.checked_requested.insert(hook.symbol.id);
                        }
                        attempt
                            .result
                            .and_then(|baseline| {
                                for poison in [0x55, 0xaa] {
                                    let attempt =
                                        analyze(completed, &self.registry, state, &reader, poison);
                                    body_unwind = attempt.unwind;
                                    if attempt.result.is_err() {
                                        position = attempt.position;
                                    }
                                    let probe = attempt.result?;
                                    if probe.requirements != baseline.requirements {
                                        return Err(Failure::Mismatch(
                                    "private-frame probe changed required callee/output lineage"
                                        .into(),
                                ));
                                    }
                                }
                                state.substitutions += baseline.substitutions;
                                state
                                    .record_match(
                                        &completed.root.admission,
                                        &completed.original,
                                        measured_outputs(&baseline, &completed.original),
                                        baseline.requirements,
                                        baseline.obligations,
                                        Vec::new(),
                                    )
                                    .map_err(Failure::Unsupported)
                            })
                            .map_err(|failure| {
                                comparison = failure.comparison().cloned();
                                failure.into_host_error(
                                    hook.symbol.name,
                                    completed.original.entry.boundary.frame,
                                )
                            })
                    }
                }
            }
        }));
        let (result, unwind) = match checked {
            Ok(result) => (result, body_unwind),
            Err(payload) => (
                Err(super::unsupported(
                    &hook,
                    UnsupportedKind::PlatformCapability,
                    "analysis interrupted by an unrelated unwind; payload is resumed after restoring all leases; no completed proof",
                )),
                Some(payload),
            ),
        };
        if self.checks.calls == calls_before {
            self.checks.calls += 1;
            self.checks.record_function(&hook, true);
        }
        if let Err(error) = &result {
            state.failed = true;
            let reason = match error {
                HostError::ReplayUnsupported { reason, .. } => reason.clone(),
                HostError::Translation(error) if error.unsupported_stack_read().is_some() => {
                    Unsupported {
                        kind: UnsupportedKind::Frame,
                        detail: error.to_string(),
                    }
                }
                _ => Unsupported {
                    kind: UnsupportedKind::Observation,
                    detail: error.to_string(),
                },
            };
            let row = state
                .rows
                .get_mut(hook.symbol.inventory_id)
                .expect("registered failed root");
            let verdict = if matches!(error, HostError::Mismatch { .. }) {
                state.mismatches += 1;
                row.failed += 1;
                Verdict::Mismatch
            } else {
                state.unsupported += 1;
                row.unsupported += 1;
                Verdict::Unsupported
            };
            if let Ok(key) = CaseKey::root(&completed.root.admission, &completed.original) {
                let _ = state.ledger.fail(key, reason.clone());
            }
            if self.checks.mismatches == mismatches_before {
                self.checks.record_mismatch(&hook);
            }
            self.disabled_check_roots
                .insert(hook.symbol.address(self.revision));
            let (detail, detail_truncated) = bounded_detail(error.to_string());
            let check = CheckFailure {
                frame: completed.original.entry.boundary.frame,
                function: hook.symbol.name.into(),
                entry_pc: completed.original.entry.boundary.cpu.pc(),
                master_clock: completed.original.entry.boundary.master_clock,
                frame_write_index: completed.root.frame_write_index,
                trace_index: position.map(|(_, _, access)| access as usize),
                detail: detail.clone(),
                detail_truncated,
                vdp_mismatch: self.checks.last_vdp_mismatch.take(),
                actual_capture: self.checks.actual_capture.take(),
                unsupported_stack_read: error.unsupported_stack_read(),
            };
            let report = self
                .checks
                .replayed_environment
                .as_mut()
                .expect("replay report after diagnostic lease");
            let index = report.failures.len();
            row.first_failure_index.get_or_insert(index);
            report.last_verdict = Some(verdict);
            report.last_unsupported = if verdict == Verdict::Unsupported {
                Some(reason.clone())
            } else {
                None
            };
            report.failures.push(FailurePacket {
                inventory_id: hook.symbol.inventory_id,
                verdict,
                witness: WitnessReport::completed(&completed.original),
                body: BodyReport::admitted(&completed.root.admission),
                effective_contract: ContractReport::registered(
                    &hook,
                    completed.root.admission.identity().profile,
                ),
                unsupported: (verdict == Verdict::Unsupported).then_some(reason),
                check: Some(check),
                comparison: (verdict == Verdict::Mismatch).then(|| {
                    comparison.unwrap_or_else(|| ComparisonDetail {
                        point: "last_original_own_observation",
                        original_pc: position.and_then(|(pc, _, _)| pc),
                        original_opcode: position.and_then(|(_, opcode, _)| opcode),
                        own_access_index: position.map(|(_, _, access)| access),
                        expected: None,
                        actual: None,
                        detail,
                    })
                }),
            });
        }
        if let Some(payload) = unwind {
            std::panic::resume_unwind(payload);
        }
        result
    }

    pub(crate) fn flush_completed_replays(&mut self) -> Result<()> {
        self.flush_replays_with_publisher(publish_replayed_state)
    }

    /// Private monomorphized publication fault seam; never a body/admission or
    /// runtime policy hook. Production uses only the publisher above.
    pub(super) fn flush_replays_with_publisher(
        &mut self,
        publish: impl FnOnce(&mut State, &mut crate::CheckReport) -> Result<()>,
    ) -> Result<()> {
        if let Some(reason) = self
            .replayed_state
            .as_ref()
            .and_then(State::infrastructure_failure)
        {
            return Err(HostError::ReplayUnsupported {
                function: "replay infrastructure".into(),
                reason: reason.clone(),
            });
        }
        if self
            .replayed_state
            .as_ref()
            .is_none_or(|state| !state.has_pending())
        {
            return Ok(());
        }
        let Some(mut state) = self.replayed_state.take() else {
            return Ok(());
        };
        let flushed = catch_unwind(AssertUnwindSafe(|| {
            let mut first_error = state.take_error();
            while let Some(mut completed) = state.next_completed() {
                if let Err(error) = self.check_completed_replay(&mut completed, &mut state) {
                    first_error.get_or_insert(error);
                    if self.check_failure_policy == CheckFailurePolicy::FailFast {
                        break;
                    }
                }
            }
            first_error
        }));
        let published = catch_unwind(AssertUnwindSafe(|| publish(&mut state, &mut self.checks)));
        match &published {
            Ok(Err(error)) => state.failed_publication(error.to_string()),
            Err(_) => state.failed_publication(
                "replay report publisher unwound; no new completed proof or campaign".into(),
            ),
            Ok(Ok(())) => state.unpublished_cases.clear(),
        }
        self.replayed_state = Some(state);
        let mut first_error = match flushed {
            Ok(first_error) => first_error,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        // Original execution panic wins; otherwise an unrelated publisher panic
        // is never swallowed by a prior Result or by ContinueOriginal.
        let published = match published {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        if let Err(error) = published {
            first_error.get_or_insert(error);
        }
        let infrastructure = self
            .replayed_state
            .as_ref()
            .and_then(State::infrastructure_failure)
            .cloned();
        if let Some(reason) = infrastructure {
            return Err(first_error.unwrap_or_else(|| HostError::ReplayUnsupported {
                function: "replay infrastructure".into(),
                reason,
            }));
        }
        if self.check_failure_policy == CheckFailurePolicy::ContinueOriginal {
            Ok(())
        } else {
            first_error.map_or(Ok(()), Err)
        }
    }

    /// Closes diagnostics only. It never runs a frame, waits for an ISR/RTS,
    /// imports proof from public report data or advances a requested boundary.
    pub fn finalize_replayed_campaign(&mut self) -> Result<()> {
        let _retained_failure = self.flush_completed_replays().err();
        let state = self
            .replayed_state
            .as_mut()
            .ok_or_else(|| HostError::Metadata("host has no explicit replay trajectory".into()))?;
        state.finalized = true;
        if state.ledger.refresh_provenance().is_err() {
            state.failed = true;
        }
        let report = self
            .checks
            .replayed_environment
            .as_mut()
            .ok_or_else(|| HostError::Metadata("replay output report was removed".into()))?;
        state.publish(report, true);
        let reason = if let Some(reason) = state.infrastructure_failure() {
            Some(reason.clone())
        } else if state.failed {
            Some(Unsupported { kind: UnsupportedKind::CalleeCoverage,
            detail: "campaign retains a failed/unsupported actual root or analysis; public diagnostic counts cannot turn it into success".into() })
        } else if state.has_open_requested() || state.rows.values().any(|row| row.open != 0) {
            Some(Unsupported { kind: UnsupportedKind::ReferenceCompletion,
            detail: "requested frame/audio/input boundary left an actual original invocation open; no fast-forward was performed".into() })
        } else if let Err(reason) = state.ledger.finish() {
            Some(reason)
        } else if state.requested.is_empty()
            || !state
                .requested
                .iter()
                .all(|id| state.checked_requested.contains(id))
        {
            Some(Unsupported {
                kind: UnsupportedKind::CalleeCoverage,
                detail: "zero-checked requested roots remain unexercised, not successful".into(),
            })
        } else {
            None
        };
        if let Some(reason) = reason {
            report.last_verdict = Some(Verdict::Unsupported);
            report.last_unsupported = Some(reason.clone());
            return Err(HostError::ReplayUnsupported {
                function: "replayed campaign".into(),
                reason,
            });
        }
        report.complete = true;
        report.last_verdict = Some(Verdict::Matched);
        Ok(())
    }
}
