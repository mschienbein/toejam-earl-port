//! Per-entry differential discharge of semantic read-before-write stack bytes.
//! ReadPurpose remains the CPU bus classification; value-flow independence is
//! established separately by replaying the audited side, never by hiding reads.
use crate::{
    Hook, HookContext, Host, HostError, Result, StackReadSite, compare_calls_with_execution,
    compare_io_ram_order,
};
use crate::{
    abi::{AtomicRun, UninitializedStackRead, canonical_ram_address},
    call_clock::{CallClockRecord, ClockRunPurpose},
    check_projection::{self, LogicTrace},
    execution::run_original_timed_until,
    exits::{self, ReferenceBoundary},
};
use genesis_core::GenesisEmulator;
use std::collections::BTreeSet;
use tje_game::{execution::ExecutionClass, exits::EntryOutcome};

#[derive(Clone, Copy)]
pub(super) enum CheckSide {
    Original {
        boundary: ReferenceBoundary,
        outcome: EntryOutcome,
        explicit_inputs: bool,
    },
    Rust,
}

impl CheckSide {
    fn name(self) -> &'static str {
        match self {
            Self::Original { .. } => "original",
            Self::Rust => "Rust",
        }
    }
}

pub(super) struct CheckRun {
    pub audit: AtomicRun,
    pub outcome: EntryOutcome,
    pub logic_trace: LogicTrace,
    pub nested_calls: u64,
    pub wait_frames: u64,
    pub dependent_writes: Option<crate::dependent_equivalence::RamWriteFootprint>,
    pub exact_device_reads: Vec<usize>,
    pub audio: Vec<[f64; 2]>,
}

fn side_error(execution: &std::thread::Result<Result<CheckRun>>) -> Option<&HostError> {
    execution.as_ref().ok()?.as_ref().err()
}

impl Host {
    /// Baselines and dependency probes use precisely the same entry execution.
    /// Probe work is deliberately excluded from coverage and gameplay clocks.
    pub(super) fn run_check_side(
        &mut self,
        fork: &mut GenesisEmulator,
        hook: &Hook,
        side: CheckSide,
        purpose: ClockRunPurpose,
        origin: crate::future_frame_watch::ObservationOrigin,
    ) -> Result<CheckRun> {
        self.run_check_side_observed(fork, hook, side, purpose, origin, None)
    }

    pub(super) fn run_check_side_observed(
        &mut self,
        fork: &mut GenesisEmulator,
        hook: &Hook,
        side: CheckSide,
        purpose: ClockRunPurpose,
        origin: crate::future_frame_watch::ObservationOrigin,
        observation: Option<crate::replayed_environment::CandidateRequest>,
    ) -> Result<CheckRun> {
        let revision = self.revision;
        let dispatcher = self.registry.dispatcher_for_hook(hook);
        let mode = self.mode;
        let mut clock = self.wait_clock.clone();
        let entry_frame = clock.frame;
        let dependent = hook.dependent_proof.is_some();
        let route = hook.check_route();
        let readable = if matches!(side, CheckSide::Rust) {
            Some(self.readable_stack_for_root(hook.symbol.id, fork.hybrid_cpu().stack_pointer())?)
        } else {
            None
        };
        if let Some(setting) = hook.dependent_proof {
            setting.validate_route(hook)?;
            setting.validate_actual_strict(fork)?;
        }
        let provenance_side = match side {
            CheckSide::Original { .. } => crate::future_frame_watch::Side::Original,
            CheckSide::Rust => crate::future_frame_watch::Side::Rust,
        };
        // Even the reference fork is speculative; purpose is not authority.
        let provenance = self.future_frame_branch(provenance_side, origin);
        let capture_enabled = self.checks.actual_capture_enabled;
        let mut completed_capture = None;
        let mut receipt = None;
        let mut timing_services = self.timing_services.take();
        if !self.registry.timing_sources().is_empty() && timing_services.is_none() {
            return Err(HostError::Metadata("reentrant timing service lease".into()));
        }
        // Each fork/probe is a separate discarded CPU/device trajectory.
        // Reuse only plans/scratch; never clone, consume or poison live authority.
        let live_authority = timing_services
            .as_mut()
            .map(|services| std::mem::take(&mut services.authority));
        let executed = self.execute_fork(fork, |emulator| {
            let entry_usage = clock.call_clock.usage();
            let ceiling = if route.progresses_time() {
                Some(clock.enter_call_ceiling(hook.symbol.id, hook.clock_ceiling)?)
            } else {
                None
            };
            let entry_sp = emulator.hybrid_cpu().stack_pointer();
            let entry_pc = emulator.hybrid_cpu().pc();
            let entry_supervisor = emulator.hybrid_cpu().status_register() & 0x2000 != 0;
            let mut stack =
                crate::abi::StackAuditState::new(entry_sp, dependent || route.is_strict());
            stack.provenance = provenance;
            stack.readable_stack = readable.clone();
            if matches!(
                side,
                CheckSide::Original {
                    explicit_inputs: true,
                    ..
                }
            ) {
                stack.capture_explicit_inputs();
            }
            if matches!(
                side,
                CheckSide::Rust
                    | CheckSide::Original {
                        explicit_inputs: true,
                        ..
                    }
            ) {
                stack.original_root = Some(std::sync::Arc::new(
                    crate::readable_stack::OriginalRootScope::new(
                        emulator.hybrid_cpu(),
                        emulator.hybrid_bus(),
                        revision,
                        hook.symbol.id,
                        hook.symbol.address(revision),
                        entry_sp,
                        matches!(side, CheckSide::Rust),
                    ),
                ));
            }
            if let Some(branch) = &stack.provenance {
                branch.begin_native(entry_frame, emulator.hybrid_bus().cycles.m68k_mclk_cycles);
            }
            let journal_lease = (capture_enabled && !route.is_strict())
                .then(|| crate::check_capture::JournalLease::install(emulator.hybrid_bus_mut()));
            if capture_enabled {
                let declared_class = match hook.execution_class {
                    ExecutionClass::Atomic => "Atomic",
                    ExecutionClass::Functional => "Functional",
                    ExecutionClass::TimeAware => "TimeAware",
                };
                let binding = crate::check_capture::CaptureBinding {
                    function: hook.symbol.name.to_owned(),
                    entry_pc: hook.symbol.address(revision),
                    revision: match revision {
                        tje_rom::Revision::Rev02 => "rev02",
                        tje_rom::Revision::Rev00 => "rev00",
                    },
                    side: provenance_side,
                    purpose: format!("{purpose:?}"),
                    origin: format!("{origin:?}"),
                    declared_class,
                    proof_envelope: hook.proof_envelope.as_str(),
                    actual_route: route.as_str(),
                    explicit_proof_only_admission: hook.proof_only_admission(),
                };
                stack.capture = Some(Box::new(crate::check_capture::RawCheckCapture::new(
                    binding,
                    emulator,
                    entry_frame,
                )));
            }
            let mut audit = stack.empty_run();
            if hook
                .dependent_proof
                .is_some_and(|setting| setting.is_clocked_strict())
            {
                audit.strict_control = Some(Default::default());
            }
            let saved_trace = emulator
                .hybrid_bus_mut()
                .hybrid_cpu_accesses
                .replace(Vec::new());
            let saved_boundaries = std::mem::replace(
                &mut emulator.hybrid_bus_mut().hybrid_exception_boundaries,
                (stack.provenance.is_some()
                    || stack.capture.is_some()
                    || stack.original_root.is_some())
                .then(Vec::new),
            );
            let saved_all_dma = emulator.hybrid_bus().hybrid_dma_all_sources;
            emulator.hybrid_bus_mut().hybrid_dma_all_sources = route.is_strict()
                || capture_enabled
                || hook
                    .dependent_proof
                    .is_some_and(|setting| setting.captures_all_dma());
            #[cfg(feature = "ram-dma-observer")]
            let saved_readable = {
                let observer = readable.clone().map(|observer| {
                    observer as std::sync::Arc<dyn genesis_core::ram_dma_observer::Observer>
                });
                emulator.hybrid_bus_mut().replace_ram_dma_observer(observer)
            };
            // Retain the branch on every execution error, including an error
            // after a watched read. No early `?` may discard its rejection.
            let mut stack_slot = Some(stack);
            let attempted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let execute = || -> Result<CheckRun> {
                    if route == crate::CheckForkRoute::FrozenFunctional {
                        emulator.hybrid_bus_mut().hybrid_set_functional_mode(true);
                    }
                    if hook
                        .dependent_proof
                        .is_some_and(|setting| setting.requires_frozen())
                        && !emulator.hybrid_bus().hybrid_functional_mode()
                    {
                        return Err(HostError::Metadata(
                            "zero-count DMA proof requires the actual frozen Functional fork"
                                .into(),
                        ));
                    }
                    if route.is_strict() {
                        crate::check_envelope::install_strict_observers(emulator)?;
                    }
                    if let Some(capture) = stack_slot
                        .as_ref()
                        .and_then(|stack| stack.capture.as_deref())
                    {
                        capture.mark_native_journal_scope();
                        capture.observe_native_batch_parts(
                            emulator.hybrid_cpu(),
                            emulator.hybrid_bus(),
                            clock.frame,
                        );
                    }
                    if hook
                        .dependent_proof
                        .is_some_and(|setting| setting.is_clocked_strict())
                    {
                        let actual_entry_depth = emulator.hybrid_cpu().hybrid_exception_depth();
                        emulator
                            .hybrid_bus_mut()
                            .hybrid_arm_dma_capture(actual_entry_depth);
                    }
                    stack_slot.as_mut().expect("root audit").drain(
                        emulator.hybrid_bus_mut(),
                        entry_pc,
                        &mut audit,
                    );
                    let mut run = match side {
                        CheckSide::Original {
                            boundary, outcome, ..
                        } => {
                            let stack = stack_slot.as_mut().expect("root audit");
                            let (logic_trace, audio) = if route.progresses_time() {
                                let clock_run = run_original_timed_until(
                                    emulator,
                                    &mut clock,
                                    boundary,
                                    stack.physical_observers(),
                                )?;
                                stack.merge_timed_accesses(
                                    clock_run.accesses,
                                    clock_run.original_root_reads,
                                    &mut audit,
                                );
                                (clock_run.logic_trace, clock_run.audio)
                            } else {
                                audit.append(crate::abi::run_original_atomic_scoped(
                                    emulator, boundary, stack,
                                )?);
                                (LogicTrace::default(), Vec::new())
                            };
                            CheckRun {
                                audit,
                                outcome,
                                logic_trace,
                                nested_calls: 0,
                                wait_frames: clock.frame - entry_frame,
                                dependent_writes: None,
                                exact_device_reads: Vec::new(),
                                audio,
                            }
                        }
                        CheckSide::Rust => {
                            let mut context =
                                HookContext::new(emulator, revision, dispatcher, mode)
                                    .with_timing_services(timing_services.as_mut())
                                    .with_execution(
                                        hook.execution_class,
                                        std::mem::take(&mut clock),
                                    )
                                    .with_check_route(route);
                            context
                                .enable_stack_audit(stack_slot.take().expect("root audit"), audit);
                            if let Some(request) = observation {
                                context.observe_frozen_stack_alignment(request);
                            }
                            let observer = readable.as_ref().expect("Rust side readable guard");
                            let attempted =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    observer.guard.run_root(
                                        hook.symbol.id,
                                        entry_sp,
                                        entry_supervisor,
                                        || {
                                            hook.execute(&mut context).and_then(|outcome| {
                                                exits::finish(&mut context, hook, outcome)?;
                                                Ok(outcome)
                                            })
                                        },
                                    )
                                }));
                            clock = context.wait_clock;
                            stack_slot = context.stack_audit.take();
                            let outcome = match attempted {
                                Ok(Ok(outcome)) => outcome?,
                                Ok(Err(error)) => return Err(HostError::Translation(error.into())),
                                Err(payload) => std::panic::resume_unwind(payload),
                            };
                            CheckRun {
                                audit: context.audit.into_inner(),
                                outcome,
                                logic_trace: context.logic_trace.into_inner(),
                                nested_calls: context.nested_calls,
                                wait_frames: context.time_frames,
                                dependent_writes: None,
                                exact_device_reads: context.exact_device_reads,
                                audio: context.time_audio,
                            }
                        }
                    };
                    if let Some(capture) = stack_slot
                        .as_ref()
                        .and_then(|stack| stack.capture.as_deref())
                    {
                        capture.mark_main_end(emulator, clock.frame);
                    }
                    if route.uses_logic_projection() {
                        emulator.hybrid_bus_mut().hybrid_set_functional_mode(true);
                    }
                    let checkpoint = emulator.hybrid_cpu().pc();
                    let stack = stack_slot.as_mut().expect("retained root audit");
                    if let Some(branch) = &stack.provenance {
                        branch.begin_native(
                            entry_frame + run.wait_frames,
                            emulator.hybrid_bus().cycles.m68k_mclk_cycles,
                        );
                    }
                    if let Some(capture) = &stack.capture {
                        capture.observe_native_batch_parts(
                            emulator.hybrid_cpu(),
                            emulator.hybrid_bus(),
                            clock.frame,
                        );
                    }
                    stack.drain(emulator.hybrid_bus_mut(), checkpoint, &mut run.audit);
                    if dependent {
                        run.dependent_writes = Some(
                            crate::dependent_equivalence::ram_write_footprint(&run.audit.writes),
                        );
                    }
                    if let Some(observer) = &readable {
                        observer
                            .guard
                            .finish_proof()
                            .map_err(|error| HostError::Translation(error.into()))?;
                    }
                    Ok(run)
                };
                if let Some(observer) = &readable {
                    observer
                        .guard
                        .catch_abort(execute)
                        .map_err(|error| HostError::Translation(error.into()))
                        .and_then(|result| result)
                } else {
                    execute()
                }
            }));
            if let Some(capture) = stack_slot
                .as_ref()
                .and_then(|stack| stack.capture.as_deref())
            {
                capture.observe_native_batch_parts(
                    emulator.hybrid_cpu(),
                    emulator.hybrid_bus(),
                    clock.frame,
                );
            }
            emulator.hybrid_bus_mut().hybrid_cpu_accesses = saved_trace;
            emulator.hybrid_bus_mut().hybrid_exception_boundaries = saved_boundaries;
            emulator.hybrid_bus_mut().hybrid_dma_all_sources = saved_all_dma;
            #[cfg(feature = "ram-dma-observer")]
            emulator
                .hybrid_bus_mut()
                .restore_ram_dma_observer(saved_readable);
            let cleanup =
                ceiling.map_or(Ok(()), |scope| clock.leave_call_ceiling(scope).map(|_| ()));
            let mut execution = match attempted {
                Ok(Ok(run)) => Ok(cleanup.map(|()| run)),
                Ok(Err(error)) => Ok(Err(error)),
                Err(payload) => Err(payload),
            };
            if route.progresses_time() {
                match clock.call_clock.usage().since(entry_usage) {
                    Ok(usage) => {
                        receipt = Some(CallClockRecord::new(
                            hook,
                            side.name(),
                            purpose,
                            usage,
                            (
                                emulator.hybrid_cpu().pc(),
                                emulator.hybrid_cpu().stack_pointer(),
                            ),
                            side_error(&execution).map(ToString::to_string),
                        ));
                    }
                    Err(error) if matches!(&execution, Ok(Ok(_))) => {
                        execution = Ok(Err(HostError::Clock(error)))
                    }
                    Err(_) => {}
                }
            }
            let provenance = stack_slot.and_then(|mut stack| {
                if let Some(capture) = stack.capture.take() {
                    completed_capture = Some(Box::new((*capture).finish(
                        emulator,
                        clock.frame,
                        side_error(&execution).map(ToString::to_string),
                    )));
                }
                stack.provenance.take()
            });
            if let Some(lease) = journal_lease {
                lease.restore(emulator.hybrid_bus_mut());
            }
            Ok((execution, provenance))
        });
        // Restore before any Result/provenance propagation. A failed/pending
        // speculative receipt belongs only to the now-discarded fork.
        let discarded = if let Some(authority) = live_authority {
            let services = timing_services
                .as_mut()
                .expect("proof retains timing-service lease");
            let speculative = std::mem::replace(&mut services.authority, authority);
            services.discard_authority(speculative)
        } else {
            Ok(())
        };
        self.timing_services = timing_services;
        let result = match executed {
            Ok((Ok(result), provenance)) => {
                let observed = self.merge_future_frame_branch(provenance);
                Ok(result.and_then(|run| observed.map(|()| run)))
            }
            Ok((Err(payload), provenance)) => {
                let _recorded_rejection = self.merge_future_frame_branch(provenance);
                Err(payload)
            }
            Err(error) => Ok(Err(error)),
        };
        if let Some(receipt) = receipt {
            self.checks.call_clocks.push(receipt);
        }
        if let Some(evidence) = completed_capture {
            self.record_actual_check_capture(
                evidence,
                matches!(purpose, ClockRunPurpose::Baseline),
            );
        }
        match result {
            Ok(result) => result.and_then(|run| discarded.map(|()| run)),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub(super) fn check_stack_dependencies(
        &mut self,
        baseline: &mut GenesisEmulator,
        run: &CheckRun,
        hook: &Hook,
        side: CheckSide,
    ) -> Result<()> {
        if run.audit.uninitialized_reads.is_empty() {
            return Ok(());
        }
        let mut reads = run.audit.uninitialized_reads.clone();
        let result = self.probe_stack_reads(baseline, run, hook, side, &mut reads);
        let sites = if result.is_ok() {
            &mut self.checks.nondependent_stack_reads
        } else {
            &mut self.checks.uninitialized_stack_reads
        };
        for UninitializedStackRead { pc, address } in reads {
            sites.insert(StackReadSite {
                function: hook.symbol.name.to_owned(),
                pc,
                address,
            });
        }
        result
    }

    fn probe_stack_reads(
        &mut self,
        baseline: &mut GenesisEmulator,
        run: &CheckRun,
        hook: &Hook,
        side: CheckSide,
        reads: &mut BTreeSet<UninitializedStackRead>,
    ) -> Result<()> {
        let sp = self.emulator.hybrid_cpu().stack_pointer();
        let mut addresses: BTreeSet<_> = reads.iter().map(|read| read.address).collect();
        loop {
            let previous_count = addresses.len();
            // Collective probes catch interactions; bytewise probes prevent
            // correlated changes from hiding dependencies such as old_a-old_b.
            let scopes = std::iter::once(None).chain(
                addresses
                    .iter()
                    .copied()
                    .filter(|_| addresses.len() > 1)
                    .map(Some),
            );
            for selected in scopes {
                let mut failure = None;
                for mask in [0x55, 0xaa] {
                    let mut perturbed = self.fork();
                    perturb_stack_bytes(&mut perturbed, &addresses, selected, mask);
                    self.checks.stack_dependency_reruns += 1;
                    let comparison = match self.run_check_side(
                        &mut perturbed,
                        hook,
                        side,
                        ClockRunPurpose::IndependenceProbe {
                            xor_mask: mask,
                            byte: selected,
                        },
                        crate::future_frame_watch::ObservationOrigin::DependencyProbe,
                    ) {
                        Ok(mut probe) => {
                            crate::readable_stack::revalidate(
                                hook,
                                sp,
                                &run.audit,
                                &mut probe.audit,
                            )
                            .map_err(|error| HostError::Translation(error.into()))?;
                            reads.extend(probe.audit.uninitialized_reads.iter().copied());
                            let (entry_ram, _) =
                                self.emulator.hybrid_bus_mut().memory.debug_ram_view();
                            compare_check_runs(
                                baseline,
                                &mut perturbed,
                                run,
                                &probe,
                                hook,
                                entry_ram,
                                sp,
                            )
                            .map(|_| ())
                            .map_err(|detail| HostError::Mismatch {
                                frame: self.frame,
                                function: hook.symbol.name.to_owned(),
                                detail,
                            })
                        }
                        Err(error) => Err(HostError::DependencyProbe {
                            cause: Box::new(error),
                        }),
                    };
                    if let Err(error) = comparison {
                        if error.unsupported_stack_read().is_some() {
                            return Err(error);
                        }
                        let mut detail = match error {
                            HostError::Mismatch { detail, .. } => detail,
                            error => error.to_string(),
                        };
                        self.record_vdp_mismatch(
                            &mut detail,
                            baseline,
                            &perturbed,
                            hook,
                            crate::diagnostics::VdpMismatchOrigin::StackProbe {
                                side: side.name(),
                                xor_mask: mask,
                                selected_address: selected,
                            },
                        );
                        failure.get_or_insert_with(|| HostError::Mismatch {
                            frame: self.frame,
                            function: hook.symbol.name.to_owned(),
                            detail: format!(
                                "uninitialized stack read: {} dependency with XOR {mask:02X} {}: {detail}",
                                side.name(),
                                selected.map_or_else(
                                    || "over audited bytes".to_owned(),
                                    |address| format!("at {address:06X}")
                                ),
                            ),
                        });
                    }
                }
                if let Some(detail) = failure {
                    return Err(detail);
                }
            }
            addresses.extend(reads.iter().map(|read| read.address));
            if addresses.len() == previous_count {
                return Ok(());
            }
            // A changed local path can expose more unwritten bytes without
            // changing outputs yet. Discharge their union before accepting.
        }
    }
}

/// The same contract comparison is used for cross-language checks and same-side
/// perturbations. A transfer preserves every register and its still-live frame.
pub(super) fn compare_check_runs(
    original: &mut GenesisEmulator,
    translated: &mut GenesisEmulator,
    original_run: &CheckRun,
    translated_run: &CheckRun,
    hook: &Hook,
    entry_ram: &[u16],
    entry_sp: u32,
) -> std::result::Result<u64, String> {
    match (original_run.outcome, translated_run.outcome) {
        // D0 is compared only when required by the ABI, not merely because a
        // void implementation happens to supply a scratch return value.
        (EntryOutcome::Returned(_), EntryOutcome::Returned(_)) => {}
        (a, b) if a == b => {}
        _ => return Err("control-transfer outcome differs".into()),
    }
    let route = hook.check_route();
    if route.is_strict() {
        return crate::check_envelope::compare_strict(
            original,
            translated,
            original_run,
            translated_run,
        );
    }
    let mut abi = hook.symbol.abi.clone();
    let mut live_start = entry_sp;
    if let EntryOutcome::Transfer(transfer) = original_run.outcome {
        abi.strict_registers = true;
        live_start = canonical_ram_address(entry_sp)
            .min(canonical_ram_address(original.hybrid_cpu().stack_pointer()));
        if matches!(
            transfer.exit,
            tje_game::exits::Exit::ExceptionReturn | tje_game::exits::Exit::Exception { .. }
        ) {
            // A transferred user frame remains live across RTE or synchronous
            // entry to a supervisor handler. Include its actual lower USP;
            // neither a saved D2 nor a suspended caller becomes dead stack.
            let usp = original.hybrid_cpu().user_stack_pointer();
            if usp & 0xffffff >= 0xe00000 {
                live_start = live_start.min(canonical_ram_address(usp));
            }
        }
    }
    let comparison_class = if route.uses_logic_projection() {
        ExecutionClass::TimeAware
    } else {
        hook.execution_class
    };
    let dead_bytes =
        compare_calls_with_execution(original, translated, live_start, &abi, comparison_class)?;
    if route.uses_logic_projection() {
        check_projection::compare(
            &original_run.logic_trace,
            &translated_run.logic_trace,
            live_start,
        )?;
    } else {
        let original_accesses = original
            .hybrid_bus()
            .hybrid_accesses
            .as_deref()
            .unwrap_or(&[]);
        let translated_accesses = translated
            .hybrid_bus()
            .hybrid_accesses
            .as_deref()
            .unwrap_or(&[]);
        crate::open_bus::compare_exact_reads(
            original_accesses,
            translated_accesses,
            &original_run.exact_device_reads,
        )?;
        crate::open_bus::compare_exact_reads(
            original_accesses,
            translated_accesses,
            &translated_run.exact_device_reads,
        )?;
        compare_io_ram_order(
            &original_run.audit.writes,
            &translated_run.audit.writes,
            entry_ram,
            live_start,
        )?;
    }
    Ok(dead_bytes)
}

fn perturb_stack_bytes(
    emulator: &mut GenesisEmulator,
    addresses: &BTreeSet<u32>,
    selected: Option<u32>,
    mask: u8,
) {
    let memory = &mut emulator.hybrid_bus_mut().memory;
    let mut perturb = |address| {
        let value = memory.read_main_ram_byte(address) ^ mask;
        memory.write_main_ram_byte(address, value);
    };
    if let Some(address) = selected {
        perturb(address);
    } else {
        for &address in addresses {
            perturb(address);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sinks::NoCartridgeSaves;
    use genesis_config::GenesisEmulatorConfig;
    use jgenesis_common::frontend::PartialClone;

    #[test]
    fn probes_change_only_selected_audited_bytes_with_two_distinct_patterns() {
        let mut baseline = GenesisEmulator::create(
            None,
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        let addresses = BTreeSet::from([0xff1000, 0xff1003]);
        baseline
            .hybrid_bus_mut()
            .memory
            .write_main_ram_byte(0xff1000, 0x12);
        baseline
            .hybrid_bus_mut()
            .memory
            .write_main_ram_byte(0xff1003, 0x34);
        for selected in [None, Some(0xff1000), Some(0xff1003)] {
            for mask in [0x55, 0xaa] {
                let mut changed = baseline.partial_clone();
                perturb_stack_bytes(&mut changed, &addresses, selected, mask);
                let (before, _) = baseline.hybrid_bus_mut().memory.debug_ram_view();
                let (after, _) = changed.hybrid_bus_mut().memory.debug_ram_view();
                for (index, (a, b)) in before
                    .iter()
                    .flat_map(|word| word.to_be_bytes())
                    .zip(after.iter().flat_map(|word| word.to_be_bytes()))
                    .enumerate()
                {
                    let address = 0xff0000 + index as u32;
                    let perturb = selected.map_or_else(
                        || addresses.contains(&address),
                        |selected| address == selected,
                    );
                    assert_eq!(b, if perturb { a ^ mask } else { a });
                }
            }
        }
    }
}

#[cfg(test)]
mod dma_tests;
