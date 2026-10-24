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
        let revision = self.revision;
        let dispatcher = self.registry.dispatcher();
        let mut clock = self.wait_clock.clone();
        let entry_frame = clock.frame;
        let dependent = hook.dependent_proof.is_some();
        let provenance_side = match side {
            CheckSide::Original { .. } => crate::future_frame_watch::Side::Original,
            CheckSide::Rust => crate::future_frame_watch::Side::Rust,
        };
        // Even the reference fork is speculative; purpose is not authority.
        let provenance = self.future_frame_branch(provenance_side, origin);
        let mut receipt = None;
        let executed = self.execute_fork(fork, |emulator| {
            let entry_usage = clock.call_clock.usage();
            let ceiling = if hook.execution_class == ExecutionClass::TimeAware {
                Some(clock.enter_call_ceiling(hook.symbol.id, hook.clock_ceiling)?)
            } else {
                None
            };
            let entry_sp = emulator.hybrid_cpu().stack_pointer();
            let entry_pc = emulator.hybrid_cpu().pc();
            let mut stack = crate::abi::StackAuditState::new(entry_sp, dependent);
            stack.provenance = provenance;
            if let Some(branch) = &stack.provenance {
                branch.begin_native(entry_frame, emulator.hybrid_bus().cycles.m68k_mclk_cycles);
            }
            let mut audit = stack.empty_run();
            let saved_trace = emulator
                .hybrid_bus_mut()
                .hybrid_cpu_accesses
                .replace(Vec::new());
            let saved_boundaries = std::mem::replace(
                &mut emulator.hybrid_bus_mut().hybrid_exception_boundaries,
                stack.provenance.is_some().then(Vec::new),
            );
            let saved_all_dma = emulator.hybrid_bus().hybrid_dma_all_sources;
            emulator.hybrid_bus_mut().hybrid_dma_all_sources = hook
                .dependent_proof
                .is_some_and(|setting| setting.captures_all_dma());
            // Retain the branch on every execution error, including an error
            // after a watched read. No early `?` may discard its rejection.
            let mut stack_slot = Some(stack);
            let result: Result<CheckRun> = (|| {
                if hook.execution_class == ExecutionClass::Functional {
                    emulator.hybrid_bus_mut().hybrid_set_functional_mode(true);
                }
                stack_slot.as_mut().expect("root audit").drain(
                    emulator.hybrid_bus_mut(),
                    entry_pc,
                    &mut audit,
                );
                let mut run = match side {
                    CheckSide::Original { boundary, outcome } => {
                        let stack = stack_slot.as_mut().expect("root audit");
                        let logic_trace = if hook.execution_class == ExecutionClass::TimeAware {
                            let clock_run = run_original_timed_until(
                                emulator,
                                &mut clock,
                                boundary,
                                stack.provenance.as_ref(),
                            )?;
                            for (pc, access) in clock_run.accesses {
                                stack.observe(pc, access, &mut audit);
                            }
                            clock_run.logic_trace
                        } else {
                            audit.append(crate::abi::run_original_atomic_scoped(
                                emulator, boundary, stack,
                            )?);
                            LogicTrace::default()
                        };
                        CheckRun {
                            audit,
                            outcome,
                            logic_trace,
                            nested_calls: 0,
                            wait_frames: clock.frame - entry_frame,
                            dependent_writes: None,
                            exact_device_reads: Vec::new(),
                        }
                    }
                    CheckSide::Rust => {
                        let mut context = HookContext::new(emulator, revision, dispatcher)
                            .with_execution(hook.execution_class, std::mem::take(&mut clock));
                        context.enable_stack_audit(stack_slot.take().expect("root audit"), audit);
                        let outcome: Result<EntryOutcome> =
                            hook.execute(&mut context).and_then(|outcome| {
                                exits::finish(&mut context, hook, outcome)?;
                                Ok(outcome)
                            });
                        clock = context.wait_clock;
                        stack_slot = context.stack_audit.take();
                        let outcome = outcome?;
                        CheckRun {
                            audit: context.audit.into_inner(),
                            outcome,
                            logic_trace: context.logic_trace.into_inner(),
                            nested_calls: context.nested_calls,
                            wait_frames: context.time_frames,
                            dependent_writes: None,
                            exact_device_reads: context.exact_device_reads,
                        }
                    }
                };
                if hook.execution_class == ExecutionClass::TimeAware {
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
                stack.drain(emulator.hybrid_bus_mut(), checkpoint, &mut run.audit);
                if dependent {
                    run.dependent_writes = Some(crate::dependent_equivalence::ram_write_footprint(
                        &run.audit.writes,
                    ));
                }
                Ok(run)
            })();
            emulator.hybrid_bus_mut().hybrid_cpu_accesses = saved_trace;
            emulator.hybrid_bus_mut().hybrid_exception_boundaries = saved_boundaries;
            emulator.hybrid_bus_mut().hybrid_dma_all_sources = saved_all_dma;
            let cleanup =
                ceiling.map_or(Ok(()), |scope| clock.leave_call_ceiling(scope).map(|_| ()));
            let mut result = match result {
                Err(error) => Err(error),
                Ok(run) => cleanup.map(|()| run),
            };
            if hook.execution_class == ExecutionClass::TimeAware {
                match clock.call_clock.usage().since(entry_usage) {
                    Ok(usage) => {
                        receipt = Some(CallClockRecord::new(
                            hook.symbol.name,
                            side.name(),
                            purpose,
                            hook.clock_ceiling,
                            usage,
                            (
                                emulator.hybrid_cpu().pc(),
                                emulator.hybrid_cpu().stack_pointer(),
                            ),
                            result.as_ref().err().map(ToString::to_string),
                        ));
                    }
                    Err(error) if result.is_ok() => result = Err(HostError::Clock(error)),
                    Err(_) => {}
                }
            }
            let provenance = stack_slot.and_then(|mut stack| stack.provenance.take());
            Ok((result, provenance))
        });
        let result = match executed {
            Ok((result, provenance)) => {
                // Merge before propagating an execution error.
                let observed = self.merge_future_frame_branch(provenance);
                result.and_then(|run| observed.map(|()| run))
            }
            Err(error) => Err(error),
        };
        if let Some(receipt) = receipt {
            self.checks.call_clocks.push(receipt);
        }
        result
    }

    pub(super) fn check_stack_dependencies(
        &mut self,
        baseline: &mut GenesisEmulator,
        run: &CheckRun,
        hook: &Hook,
        side: CheckSide,
    ) -> std::result::Result<(), String> {
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
    ) -> std::result::Result<(), String> {
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
                        Ok(probe) => {
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
                        }
                        Err(error) => Err(format!("dependency probe did not complete: {error}")),
                    };
                    if let Err(mut detail) = comparison {
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
                        failure.get_or_insert_with(|| {
                            format!(
                                "uninitialized stack read: {} dependency with XOR {mask:02X} {}: {detail}",
                                side.name(),
                                selected.map_or_else(
                                    || "over audited bytes".to_owned(),
                                    |address| format!("at {address:06X}")
                                ),
                            )
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
    let mut abi = hook.symbol.abi.clone();
    let mut live_start = entry_sp;
    if let EntryOutcome::Transfer(transfer) = original_run.outcome {
        abi.strict_registers = true;
        live_start = canonical_ram_address(entry_sp)
            .min(canonical_ram_address(original.hybrid_cpu().stack_pointer()));
        if transfer.exit == tje_game::exits::Exit::ExceptionReturn {
            // A suspended user caller remains live even when this particular
            // RTE resumes a nested supervisor handler. No disjoint stack bounds
            // are declared, so a valid lower USP conservatively expands checks.
            let usp = original.hybrid_cpu().user_stack_pointer();
            if usp & 0xffffff >= 0xe00000 {
                live_start = live_start.min(canonical_ram_address(usp));
            }
        }
    }
    let dead_bytes =
        compare_calls_with_execution(original, translated, live_start, &abi, hook.execution_class)?;
    if hook.execution_class == ExecutionClass::TimeAware {
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
