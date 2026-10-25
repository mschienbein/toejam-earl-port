//! Headless REV 02/REV 00 reference and hybrid host built on jgenesis.
//!
//! Checks compare independent state forks while live Check retains original ROM
//! timing. Atomic freezes clocks and compares full device state; explicitly
//! registered Functional and TimeAware contracts use the documented transaction
//! and logical-state projections. Replace is not cycle-equivalent. Both original
//! and Rust semantic reads of dead stack are audited; discarded CPU bus reads
//! retain their device effects but are not software data dependencies.

pub mod abi;
#[cfg(test)]
#[path = "../tests/support/allocation_counter.rs"]
mod allocation_counter;
mod call_clock;
pub mod call_observation;
pub use call_clock::CallClockRecord;
pub mod catalogue;
pub mod check_capture;
pub mod check_envelope;
mod check_failure;
pub use check_envelope::CheckForkRoute;
pub use check_failure::{CheckFailure, CheckFailurePolicy};
pub mod check_projection;
mod coverage;
pub mod diagnostics;
pub use coverage::FunctionCoverageTable;
pub mod dependent_equivalence;
pub mod execution;
mod exits;
pub mod fixture;
pub mod future_frame_watch;
pub mod hooks;
pub mod input;
mod open_bus;
mod play;
pub mod session_trace;
mod sinks;
mod stack_dependency;
pub mod timing_region;

use abi::{HookContext, OrderedWrite, set_sp};
use execution::{WaitClock, WaitInputPoller};
use genesis_config::{GenesisEmulatorConfig, GenesisInputs};
use genesis_core::{GenesisEmulator, HybridWrite};
use hooks::{Hook, HookRegistry};
use jgenesis_common::frontend::{Color, EmulatorTrait, FrameSize, PartialClone, TickEffect};
use serde::Serialize;
use sinks::{HeadlessAudio, HeadlessOutput, Inputs, NoCartridgeSaves};
use stack_dependency::{CheckSide, compare_check_runs};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tje_game::execution::ExecutionClass;
use tje_rom::{Revision, Rom};

pub type Result<T, E = HostError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("ROM import failed: {0}")]
    Rom(#[from] tje_rom::ImportError),
    #[error("hook registry targets {registry:?}, ROM is {rom:?}")]
    HookRevisionMismatch { registry: Revision, rom: Revision },
    #[error(transparent)]
    Translation(#[from] tje_game::Error),
    #[error(transparent)]
    Clock(#[from] tje_game::execution::CallClockError),
    #[error("invalid metadata: {0}")]
    Metadata(String),
    #[error(
        "{function} has no atomic-capable contract; clock/interrupt progress is unsupported by atomic hooks"
    )]
    NotAtomicCapable { function: String },
    #[error("invalid input script: {0}")]
    Input(String),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("emulator: {0}")]
    Emulator(String),
    #[error(
        "original call did not return to {return_pc:06X} (PC={pc:06X}); atomic calls must not wait for peripheral time"
    )]
    CallDidNotReturn { pc: u32, return_pc: u32 },
    #[error("check mismatch at frame {frame}, {function}: {detail}")]
    Mismatch {
        frame: u64,
        function: String,
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    Reference,
    Check,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum HardwareWrite {
    Vdp { port: u8, value: u16 },
    Ym2612 { port: u8, value: u8 },
    Psg { value: u8 },
}
impl From<HybridWrite> for HardwareWrite {
    fn from(value: HybridWrite) -> Self {
        match value {
            HybridWrite::Vdp { port, value } => Self::Vdp { port, value },
            HybridWrite::Ym2612 { port, value } => Self::Ym2612 { port, value },
            HybridWrite::Psg { value } => Self::Psg { value },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VdpSnapshot {
    pub vram: Box<[u8]>,
    pub cram: Box<[u16]>,
    pub vsram: Box<[u16]>,
    /// bincode encoding of jgenesis's typed `vdp::registers::Registers`.
    pub registers: Vec<u8>,
}

#[derive(Debug)]
pub struct FrameSnapshot {
    /// Zero-based completed video frame, independent of the resettable game counter.
    pub frame: u64,
    pub frame_counter: u16,
    pub pc: u32,
    pub ram: Box<[u8]>,
    pub vdp: VdpSnapshot,
    pub writes: Vec<HardwareWrite>,
    /// Stereo 48 kHz resampled output, preserving the emulator's f64 bits.
    pub audio_samples: Vec<[f64; 2]>,
    pub idle_random_draws: u64,
    pub lag_events: Vec<LagEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LagEvent {
    pub frame: u64,
    pub pc_at_vblank: u32,
    pub frame_counter: u16,
}

#[derive(Debug, Default, Serialize)]
pub struct Measurements {
    pub intro_frame: Option<u64>,
    pub title_frame: Option<u64>,
    pub menu_frame: Option<u64>,
    pub game_frame: Option<u64>,
    pub logic_ticks: u64,
    pub idle_draws_per_frame: Vec<u64>,
    pub lag_events: Vec<LagEvent>,
    pub vdp_writes: u64,
    pub ym2612_writes: u64,
    pub psg_writes: u64,
    pub wait_video_frames: u64,
    pub check_original_wait_frames: u64,
    pub check_rust_wait_frames: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct StackReadSite {
    pub function: String,
    pub pc: u32,
    pub address: u32,
}

#[derive(Debug, Default, Serialize)]
pub struct FunctionCoverage {
    pub calls_checked: u64,
    pub calls_replaced: u64,
    pub mismatches: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct SubsystemCoverage {
    pub registered_functions: usize,
    pub calls_checked: u64,
    pub calls_replaced: u64,
    pub mismatches: u64,
    pub unexercised: BTreeSet<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct CheckReport {
    pub calls: u64,
    pub by_function: FunctionCoverageTable,
    pub by_subsystem: BTreeMap<String, SubsystemCoverage>,
    /// Registered entries with no checked or replacement call in this run.
    pub unexercised: BTreeSet<String>,
    pub mismatches: u64,
    pub nested_original_calls: u64,
    pub dead_stack_differing_calls: u64,
    pub dead_stack_differing_bytes: u64,
    /// Read sites whose dependency probes failed or could not complete.
    pub uninitialized_stack_reads: BTreeSet<StackReadSite>,
    /// Informational semantic reads with unchanged outputs on additional forks.
    pub nondependent_stack_reads: BTreeSet<StackReadSite>,
    /// Additional side executions, not checked calls or gameplay wait frames.
    pub stack_dependency_reruns: u64,
    /// Genuine frame-read sites, never informational. DMA-only proofs retain
    /// bounded source/cardinality summaries in dependent_equivalence instead.
    pub dependent_stack_reads: BTreeSet<StackReadSite>,
    pub dependent_equivalence: BTreeMap<String, dependent_equivalence::DependentEvidence>,
    pub time_aware: BTreeMap<String, check_projection::TimeAwareCheckSummary>,
    /// Separate baseline/probe completions and failures under the same deadline.
    pub call_clocks: Vec<CallClockRecord>,
    /// Private raw observations; never export cartridge-derived runtime traces.
    #[serde(skip)]
    pub last_time_aware: Option<check_projection::TimeAwareObservation>,
    #[serde(skip)]
    pub(crate) actual_capture_enabled: bool,
    #[serde(skip)]
    pub actual_capture: Option<Box<check_capture::CheckCaptureDiagnostic>>,
    /// Bounded field-level VDP evidence captured before the failing forks drop.
    pub last_vdp_mismatch: Option<diagnostics::VdpMismatchReport>,
    pub failure_policy: CheckFailurePolicy,
    /// One bounded event per failed root; retained even if later live work fails.
    pub failures: Vec<CheckFailure>,
    /// Pending frame-version obligations never constitute approved checks.
    pub future_frame_provenance: future_frame_watch::ProvenanceReport,
}

impl CheckReport {
    pub(crate) fn registered(registry: &HookRegistry) -> Self {
        let mut report = Self {
            by_function: FunctionCoverageTable::registered(registry),
            ..Self::default()
        };
        for subsystem in tje_game::SUBSYSTEMS {
            report
                .by_subsystem
                .insert(subsystem.name.to_owned(), SubsystemCoverage::default());
        }
        for hook in registry.iter() {
            report.unexercised.insert(hook.symbol.name.to_owned());
            let coverage = report
                .by_subsystem
                .entry(hook.symbol.subsystem.to_owned())
                .or_default();
            coverage.registered_functions += 1;
            coverage.unexercised.insert(hook.symbol.name.to_owned());
        }
        report
    }

    fn record_function(&mut self, hook: &Hook, checked: bool) {
        let name = hook.symbol.name;
        let subsystem = hook.symbol.subsystem;
        let coverage = self.by_function.at_mut(hook.coverage_index);
        let first = coverage.calls_checked == 0 && coverage.calls_replaced == 0;
        if checked {
            coverage.calls_checked += 1;
        } else {
            coverage.calls_replaced += 1;
        }
        if first {
            self.unexercised.remove(name);
        }
        let coverage = self
            .by_subsystem
            .get_mut(subsystem)
            .expect("registered subsystem");
        if checked {
            coverage.calls_checked += 1;
        } else {
            coverage.calls_replaced += 1;
        }
        if first {
            coverage.unexercised.remove(name);
        }
    }

    fn record_mismatch(&mut self, hook: &Hook) {
        self.mismatches += 1;
        self.by_function.at_mut(hook.coverage_index).mismatches += 1;
        self.by_subsystem
            .get_mut(hook.symbol.subsystem)
            .expect("registered subsystem")
            .mismatches += 1;
    }
}

pub struct Host {
    emulator: GenesisEmulator,
    registry: HookRegistry,
    mode: Mode,
    revision: Revision,
    symbols: catalogue::RevisionSymbols,
    frame: u64,
    main_idle: bool,
    frame_progress: fixture::FrameProgress,
    terminal_entry: Option<tje_game::FunctionId>,
    check_failure_policy: CheckFailurePolicy,
    disabled_check_roots: BTreeSet<u32>,
    future_observer: future_frame_watch::LiveObserverLease,
    output: HeadlessOutput,
    audio: HeadlessAudio,
    saves: NoCartridgeSaves,
    audio_sample_rate: u32,
    pub(crate) session_trace: Option<session_trace::SessionTrace>,
    wait_clock: WaitClock,
    timing_services: Option<timing_region::game_bridge::TimingServices>,
    pub checks: CheckReport,
    pub measurements: Measurements,
}

impl Host {
    pub fn new(bytes: &[u8], mode: Mode, registry: HookRegistry) -> Result<Self> {
        if mode == Mode::Replace {
            registry.ensure_replacement_admission()?;
        }
        Self::from_admitted_rom(Rom::import(bytes)?, mode, registry)
    }

    pub fn from_rom(rom: Rom, mode: Mode, registry: HookRegistry) -> Result<Self> {
        if mode == Mode::Replace {
            registry.ensure_replacement_admission()?;
        }
        Self::from_admitted_rom(rom, mode, registry)
    }

    fn from_admitted_rom(rom: Rom, mode: Mode, registry: HookRegistry) -> Result<Self> {
        let revision = rom.revision();
        if registry.revision() != revision {
            return Err(HostError::HookRevisionMismatch {
                registry: registry.revision(),
                rom: revision,
            });
        }
        if registry
            .iter()
            .any(|hook| hook.check_route().progresses_time())
        {
            execution::prepare_operational_callee_indexes()?;
        }
        let initial_sp = u32::from_be_bytes(
            rom.bytes()[..4]
                .try_into()
                .expect("identified ROM has vectors"),
        );
        if initial_sp & 0xFF_FFFF != catalogue::STACK_TOP {
            return Err(HostError::Metadata(format!(
                "reset SP {initial_sp:08X} does not meet RAM map's stack boundary"
            )));
        }
        let config = GenesisEmulatorConfig::default();
        let timing_services = if registry.timing_sources().is_empty() {
            None
        } else {
            let divider = u32::try_from(config.clamped_m68k_divider().get()).map_err(|_| {
                HostError::Metadata("configured clock divider exceeds the timing ABI".into())
            })?;
            Some(timing_region::game_bridge::TimingServices::prepare(
                &rom,
                registry.timing_sources(),
                divider,
                registry.dispatcher(),
                mode,
            )?)
        };
        let mut saves = NoCartridgeSaves;
        let mut emulator = GenesisEmulator::create(Some(rom.into_bytes()), config, &mut saves);
        emulator.hybrid_bus_mut().hybrid_writes = Some(Vec::new());
        let checks = CheckReport::registered(&registry);
        Ok(Self {
            emulator,
            registry,
            mode,
            revision,
            symbols: catalogue::RevisionSymbols::for_revision(revision),
            frame: 0,
            main_idle: false,
            frame_progress: fixture::FrameProgress::default(),
            terminal_entry: None,
            check_failure_policy: CheckFailurePolicy::FailFast,
            disabled_check_roots: BTreeSet::new(),
            future_observer: Default::default(),
            output: HeadlessOutput,
            audio: HeadlessAudio::default(),
            saves,
            audio_sample_rate: 48_000,
            session_trace: None,
            wait_clock: WaitClock {
                revision: Some(revision),
                ..WaitClock::default()
            },
            timing_services,
            checks,
            measurements: Measurements::default(),
        })
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn revision(&self) -> Revision {
        self.revision
    }
    pub fn frame_count(&self) -> u64 {
        self.frame
    }

    pub fn set_wait_input_poller(&mut self, poller: Arc<dyn WaitInputPoller>) {
        self.wait_clock.poller = Some(poller);
    }
    pub fn frame_buffer(&self) -> (&[Color], FrameSize) {
        let vdp = &self.emulator.hybrid_bus().vdp;
        (vdp.frame_buffer(), vdp.frame_size())
    }
    pub fn cpu(&self) -> &m68000_emu::M68000 {
        self.emulator.hybrid_cpu()
    }
    /// Last source-region attempt; actual original ISR execution is separate
    /// from modeled Rust instruction grants.
    pub fn last_timing_source_usage(
        &self,
        entry: tje_game::FunctionId,
    ) -> Option<tje_engine_runtime::timing_region::RegionUsage> {
        self.timing_services
            .as_ref()
            .and_then(|services| services.usage(entry))
    }
    pub fn context(&mut self) -> HookContext<'_> {
        self.inspection_context()
    }

    /// Inspect the typed VDP registers directly; unlike the snapshot encoding,
    /// this does not allocate and retains the upstream register field names.
    pub fn vdp_registers(&self) -> &genesis_components::vdp::registers::Registers {
        self.emulator.hybrid_bus().vdp.hybrid_registers()
    }

    fn tick(&mut self, inputs: &mut Inputs) -> Result<TickEffect> {
        self.emulator
            .tick(&mut self.output, &mut self.audio, inputs, &mut self.saves)
            .map_err(|e| HostError::Emulator(e.to_string()))
    }

    pub fn run_frame(&mut self, inputs: GenesisInputs) -> Result<FrameSnapshot> {
        if self.mode == Mode::Replace {
            self.registry.ensure_replacement_admission()?;
        }
        self.observe_retained_native_prefix()?;
        self.wait_clock.frame = self.frame;
        self.wait_clock.inputs = inputs;
        let mut inputs = Inputs(inputs);
        loop {
            let pc = self.emulator.hybrid_cpu().pc();
            let bus = self.emulator.hybrid_bus();
            let ready = bus.cycles.m68k_wait_cpu_cycles == 0
                && self.emulator.hybrid_cpu().hybrid_instruction_ready(bus);
            if ready {
                self.observe_live_instruction(pc);
                if self.mode != Mode::Reference
                    && let Some(hook) = self.registry.at(pc).cloned()
                    && self.terminal_entry != Some(hook.symbol.id)
                    && (self.disabled_check_roots.is_empty()
                        || !self.disabled_check_roots.contains(&pc))
                {
                    if self.mode == Mode::Check {
                        self.check_live_hook(&hook)?;
                    } else {
                        self.replace_hook(&hook)?;
                        inputs.0 = self.wait_clock.inputs;
                        continue;
                    }
                }
            }
            let effect = self.tick_live_original(&mut inputs)?;
            if effect == TickEffect::FrameRendered {
                break;
            }
        }
        let snapshot = self.finish_live_frame()?;
        self.wait_clock.frame = self.frame;
        Ok(snapshot)
    }

    fn snapshot(
        &mut self,
        idle_random_draws: u64,
        lag_events: Vec<LagEvent>,
    ) -> Result<FrameSnapshot> {
        let frame_counter = self.context().read_u16(catalogue::FRAME_COUNTER);
        let bus = self.emulator.hybrid_bus_mut();
        let (ram, _) = bus.memory.debug_ram_view();
        let mut ram_bytes = Vec::with_capacity(65_536);
        ram_bytes.extend(ram.iter().flat_map(|word| word.to_be_bytes()));
        let ram = ram_bytes.into_boxed_slice();
        let (vram, cram, vsram) = bus.vdp.hybrid_memories();
        let vdp = VdpSnapshot {
            vram: vram.into(),
            cram: cram.into(),
            vsram: vsram.into(),
            registers: bincode::encode_to_vec(
                bus.vdp.hybrid_registers(),
                bincode::config::standard(),
            )
            .map_err(|e| HostError::Emulator(e.to_string()))?,
        };
        let writes = bus
            .hybrid_writes
            .as_mut()
            .expect("trace enabled")
            .drain(..)
            .map(Into::into)
            .collect();
        Ok(FrameSnapshot {
            frame: self.frame,
            frame_counter,
            pc: self.emulator.hybrid_cpu().pc(),
            ram,
            vdp,
            audio_samples: std::mem::replace(&mut self.audio.samples, Vec::with_capacity(1024)),
            writes,
            idle_random_draws,
            lag_events,
        })
    }

    fn fork(&self) -> GenesisEmulator {
        let mut fork = self.emulator.partial_clone();
        fork.hybrid_bus_mut().hybrid_writes = Some(Vec::new());
        fork.hybrid_bus_mut().hybrid_accesses = Some(Vec::new());
        fork
    }

    /// Move, don't copy, the read-only ROM into a sequentially executed fork.
    /// Return the same asset on Result errors or unwind, preserving the exact
    /// unwind payload. CPU/SP/device state is never copied or restored here.
    fn execute_fork<T>(
        &mut self,
        fork: &mut GenesisEmulator,
        execute: impl FnOnce(&mut GenesisEmulator) -> Result<T>,
    ) -> Result<T> {
        Self::execute_rom_fork(&mut self.emulator, fork, execute)
    }

    fn execute_rom_fork<T>(
        origin: &mut GenesisEmulator,
        fork: &mut GenesisEmulator,
        execute: impl FnOnce(&mut GenesisEmulator) -> Result<T>,
    ) -> Result<T> {
        fork.hybrid_bus_mut()
            .cartridge
            .as_mut()
            .expect("standalone cartridge")
            .take_rom_from(
                origin
                    .hybrid_bus_mut()
                    .cartridge
                    .as_mut()
                    .expect("cartridge"),
            );
        // The fork stays owned by the caller outside the catch, so unwinding
        // cannot drop the leased cartridge before ownership is returned.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(fork)));
        origin
            .hybrid_bus_mut()
            .cartridge
            .as_mut()
            .expect("cartridge")
            .take_rom_from(fork.hybrid_bus_mut().cartridge.as_mut().expect("cartridge"));
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn check_hook(&mut self, hook: &Hook) -> Result<tje_game::exits::EntryOutcome> {
        if self.checks.actual_capture_enabled {
            self.checks.actual_capture = Some(Box::new(
                check_capture::CheckCaptureDiagnostic::new(hook.symbol.name),
            ));
        }
        // Do not copy this frame's accumulated event trace into either fork.
        let live_trace = self.emulator.hybrid_bus_mut().hybrid_writes.take();
        let result = self.check_hook_inner(hook);
        if self.checks.actual_capture_enabled
            && let Some(capture) = self.checks.actual_capture.as_deref_mut()
        {
            capture.comparison = Some(if result.is_ok() { "passed" } else { "failed" });
            capture.comparison_error = result
                .as_ref()
                .err()
                .map(|error| check_failure::bounded_detail(error.to_string()).0);
        }
        self.emulator.hybrid_bus_mut().hybrid_writes = live_trace;
        result
    }

    fn check_hook_inner(&mut self, hook: &Hook) -> Result<tje_game::exits::EntryOutcome> {
        let entry = exits::EntryState::capture(&self.context(), hook.exit_contract)?;
        let sp = entry.sp;
        let dma_plan = self.resolve_dependent_source(hook)?;
        let mut original = self.fork();
        let mut translated = self.fork();
        // The Rust outcome declares the finite original boundary, including
        // propagated resets, exception returns and persistent main-loop stages.
        let translated_run = self.run_check_side(
            &mut translated,
            hook,
            CheckSide::Rust,
            call_clock::ClockRunPurpose::Baseline,
            crate::future_frame_watch::ObservationOrigin::CheckFork,
        )?;
        let original_side = CheckSide::Original {
            boundary: exits::reference_boundary(
                hook,
                translated_run.outcome,
                entry,
                self.revision,
            )?,
            outcome: translated_run.outcome,
        };
        let original_run = self.run_check_side(
            &mut original,
            hook,
            original_side,
            call_clock::ClockRunPurpose::Baseline,
            crate::future_frame_watch::ObservationOrigin::CheckFork,
        )?;
        self.measurements.check_original_wait_frames += original_run.wait_frames;
        self.measurements.check_rust_wait_frames += translated_run.wait_frames;
        self.checks.calls += 1;
        self.checks.record_function(hook, true);
        self.checks.nested_original_calls += translated_run.nested_calls;
        if self.checks.actual_capture_enabled
            && let Some(capture) = self.checks.actual_capture.as_deref_mut()
        {
            capture.attach_counterpart_frames(&original, &translated);
        }
        if hook.check_route().uses_logic_projection() {
            let raw_writes_differ =
                original.hybrid_bus().hybrid_writes != translated.hybrid_bus().hybrid_writes;
            self.checks
                .time_aware
                .entry(hook.symbol.name.to_owned())
                .or_default()
                .record(
                    &original_run.logic_trace,
                    &translated_run.logic_trace,
                    raw_writes_differ,
                );
        }
        let comparison = if hook.dependent_proof.is_some() {
            self.check_dependent_equivalence(
                &mut original,
                &mut translated,
                &original_run,
                &translated_run,
                hook,
                dma_plan,
            )
        } else {
            // Unmarked entries still prove independence on each side separately.
            let original_independent =
                self.check_stack_dependencies(&mut original, &original_run, hook, original_side);
            let rust_independent = self.check_stack_dependencies(
                &mut translated,
                &translated_run,
                hook,
                CheckSide::Rust,
            );
            original_independent.and(rust_independent).and_then(|()| {
                let (entry_ram, _) = self.emulator.hybrid_bus_mut().memory.debug_ram_view();
                compare_check_runs(
                    &mut original,
                    &mut translated,
                    &original_run,
                    &translated_run,
                    hook,
                    entry_ram,
                    sp,
                )
            })
        };
        if hook.check_route().uses_logic_projection() {
            self.checks.last_time_aware = Some(check_projection::TimeAwareObservation {
                function: hook.symbol.name.to_owned(),
                original: original_run.logic_trace.events,
                translated: translated_run.logic_trace.events,
            });
        }

        match comparison {
            Ok(dead_bytes) => {
                self.checks.dead_stack_differing_bytes += dead_bytes;
                if dead_bytes != 0 {
                    self.checks.dead_stack_differing_calls += 1;
                }
                if let Some(entry) = exits::terminal(translated_run.outcome) {
                    self.terminal_entry = Some(entry);
                }
                Ok(translated_run.outcome)
            }
            Err(mut detail) => {
                self.record_vdp_mismatch(
                    &mut detail,
                    &original,
                    &translated,
                    hook,
                    diagnostics::VdpMismatchOrigin::Check,
                );
                self.checks.record_mismatch(hook);
                Err(HostError::Mismatch {
                    frame: self.frame,
                    function: hook.symbol.name.to_owned(),
                    detail,
                })
            }
        }
    }

    fn replace_hook(&mut self, hook: &Hook) -> Result<()> {
        self.registry.ensure_replacement_admission()?;
        hook.ensure_replacement_admission()?;
        let progresses_time = hook.execution_class != ExecutionClass::Atomic;
        let entry_usage = self.wait_clock.call_clock.usage();
        let mut context = HookContext::new(
            &mut self.emulator,
            self.revision,
            self.registry.dispatcher(),
            self.mode,
        )
        .with_timing_services(self.timing_services.as_mut());
        if progresses_time {
            context =
                context.with_execution(hook.execution_class, std::mem::take(&mut self.wait_clock));
            context.time_audio = std::mem::take(&mut self.audio.samples);
        }
        let mut result = hook.execute(&mut context).and_then(|outcome| {
            exits::finish(&mut context, hook, outcome)?;
            Ok(outcome)
        });
        self.checks.nested_original_calls += context.nested_calls;
        if progresses_time {
            let usage = context.wait_clock.call_clock.usage().since(entry_usage);
            if let Err(error) = usage
                && result.is_ok()
            {
                result = Err(HostError::Clock(error));
            }
            // Replacement/play has no per-call success journal. Only a cold
            // failure allocates a diagnostic receipt; ledger enforcement stays.
            if let (Err(error), Ok(usage)) = (&result, usage) {
                self.checks.call_clocks.push(CallClockRecord::new(
                    hook,
                    "Rust",
                    call_clock::ClockRunPurpose::Replacement,
                    usage,
                    (context.cpu().pc(), context.cpu().stack_pointer()),
                    Some(error.to_string()),
                ));
            }
            self.wait_clock = context.wait_clock;
            self.frame = self.wait_clock.frame;
            self.measurements.wait_video_frames += context.time_frames;
            self.audio.samples = context.time_audio;
        }
        if let Some(entry) = exits::terminal(result?) {
            self.terminal_entry = Some(entry);
        }
        self.checks.record_function(hook, false);
        Ok(())
    }

    /// ROM-backed function fixture. Does not advance hardware clocks. Useful for
    /// edge cases that the live game never generates (e.g. minimum signed seed).
    /// Requires a settled live boundary; settle before seeding fixture inputs.
    pub fn check_call(&mut self, address: u32, arguments: &[u32]) -> Result<u32> {
        self.check_call_inner(address, arguments, false)
    }

    fn check_call_inner(
        &mut self,
        address: u32,
        arguments: &[u32],
        isolated_fixture: bool,
    ) -> Result<u32> {
        self.observe_retained_native_prefix()?;
        self.require_fixture_boundary()?;
        let hook = self
            .registry
            .at(address)
            .cloned()
            .ok_or_else(|| HostError::Metadata(format!("no hook at {address:06X}")))?;
        if hook.exit_contract != tje_game::exits::ExitContract::Rts {
            return Err(HostError::Metadata(
                "non-RTS diagnostics require check_entry_fixture without a fabricated return"
                    .into(),
            ));
        }
        if (hook.check_route().progresses_time() || hook.execution_class != ExecutionClass::Atomic)
            && !isolated_fixture
        {
            return Err(HostError::Metadata(
                "non-atomic diagnostics require check_fixture isolation".into(),
            ));
        }
        let saved_pc = self.emulator.hybrid_cpu().pc();
        let saved_sp = self.emulator.hybrid_cpu().stack_pointer();
        let mut sp = saved_sp;
        for &argument in arguments.iter().rev() {
            sp = sp.wrapping_sub(4);
            self.context().write_u32(sp, argument);
        }
        let cleanup_sp = sp;
        sp = sp.wrapping_sub(4);
        self.context().write_u32(sp, abi::CALL_SENTINEL);
        let (cpu, bus) = self.emulator.hybrid_parts();
        set_sp(cpu, sp);
        cpu.set_pc(address, bus);
        self.check_hook(&hook)?;
        self.execute_fixture_original(&hook, cleanup_sp)?;
        let (cpu, bus) = self.emulator.hybrid_parts();
        let result = cpu.data_registers()[0];
        set_sp(cpu, saved_sp);
        cpu.set_pc(saved_pc, bus);
        self.observe_retained_native_prefix()?;
        Ok(result)
    }

    /// Differential fixture on a fork of the current live state. Restore CPU and
    /// every device/storage byte afterward, but retain the measured coverage.
    /// This prevents diagnostic writes from filling a frozen VDP FIFO across calls.
    pub fn check_fixture(&mut self, address: u32, arguments: &[u32]) -> Result<u32> {
        self.isolated_fixture(|host| host.check_call_inner(address, arguments, true))
    }
}

#[cfg(test)]
fn compare_calls(
    original: &mut GenesisEmulator,
    translated: &mut GenesisEmulator,
    entry_sp: u32,
    abi: &tje_game::Abi,
) -> std::result::Result<u64, String> {
    compare_calls_with_execution(original, translated, entry_sp, abi, ExecutionClass::Atomic)
}

fn compare_calls_with_execution(
    original: &mut GenesisEmulator,
    translated: &mut GenesisEmulator,
    entry_sp: u32,
    abi: &tje_game::Abi,
    execution_class: ExecutionClass,
) -> std::result::Result<u64, String> {
    let (a, b) = (original.hybrid_cpu(), translated.hybrid_cpu());
    if a.pc() != b.pc() || a.stack_pointer() != b.stack_pointer() {
        return Err(format!(
            "return PC/SP: original={:08X}/{:08X}, Rust={:08X}/{:08X}",
            a.pc(),
            a.stack_pointer(),
            b.pc(),
            b.stack_pointer()
        ));
    }
    if a.user_stack_pointer() != b.user_stack_pointer()
        || a.supervisor_stack_pointer() != b.supervisor_stack_pointer()
    {
        return Err("banked USP/SSP state differs".into());
    }
    if a.hybrid_exception_depth() != b.hybrid_exception_depth() {
        return Err("accepted exception nesting depth differs".into());
    }
    let (da, db) = (a.data_registers(), b.data_registers());
    if abi.returns_value && da[0] != db[0] {
        return Err(format!("D0: original={:08X}, Rust={:08X}", da[0], db[0]));
    }
    for index in 0..8 {
        if (abi.strict_registers || abi.preserved_data & (1 << index) != 0)
            && da[index] != db[index]
        {
            return Err(format!(
                "ABI-preserved D{index}: original={:08X}, Rust={:08X}",
                da[index], db[index]
            ));
        }
    }
    for index in 0..7 {
        if (abi.strict_registers || abi.preserved_address & (1 << index) != 0)
            && a.address_registers()[index] != b.address_registers()[index]
        {
            return Err(format!("ABI-preserved A{index} mismatch"));
        }
    }
    let sr_mask = if abi.strict_registers { 0xFFFF } else { 0xFFE0 };
    if a.status_register() & sr_mask != b.status_register() & sr_mask {
        return Err(format!(
            "SR control bits: original={:04X}, Rust={:04X}",
            a.status_register(),
            b.status_register()
        ));
    }
    let (a, b) = (original.hybrid_bus_mut(), translated.hybrid_bus_mut());
    let (ra, aa) = a.memory.debug_ram_view();
    let (rb, ab) = b.memory.debug_ram_view();
    let mut dead_bytes = 0;
    let dead_end = abi::canonical_ram_address(entry_sp).min(catalogue::STACK_TOP);
    for (index, (&wa, &wb)) in ra.iter().zip(rb.iter()).enumerate() {
        if wa == wb {
            continue;
        }
        let address = catalogue::STACK_BOTTOM + 2 * index as u32;
        for (offset, (ba, bb)) in wa
            .to_be_bytes()
            .into_iter()
            .zip(wb.to_be_bytes())
            .enumerate()
        {
            if ba == bb {
                continue;
            }
            let address = address + offset as u32;
            if address < dead_end {
                dead_bytes += 1;
            } else {
                return Err(format!(
                    "RAM {address:06X}: original={ba:02X}, Rust={bb:02X}"
                ));
            }
        }
    }
    if execution_class != ExecutionClass::TimeAware && aa != ab {
        return Err("Z80 audio RAM differs".into());
    }
    if a.z80_signals != b.z80_signals
        || (execution_class != ExecutionClass::TimeAware && a.z80_bank != b.z80_bank)
        || a.pending_writes != b.pending_writes
    {
        return Err("Z80 bus/reset/bank or pending CPU write state differs".into());
    }
    if execution_class == ExecutionClass::Functional
        && (a.hybrid_functional_violation.is_some() || b.hybrid_functional_violation.is_some())
    {
        return Err(format!(
            "functional contract observed VDP status/HV: original={:?}, Rust={:?}",
            a.hybrid_functional_violation, b.hybrid_functional_violation
        ));
    }
    let vdp_matches = if execution_class == ExecutionClass::TimeAware {
        a.vdp.hybrid_logic_state_eq(&b.vdp)
    } else {
        a.vdp.hybrid_state_eq(&b.vdp)
    };
    if !vdp_matches {
        return Err(
            "VDP state differs (ports/latches/FIFO/DMA/render state/memories/registers)".into(),
        );
    }
    if execution_class == ExecutionClass::TimeAware {
        // Owned main-thread/ISR logic effects are compared by check_projection;
        // asynchronous sound writes and verified ISR busy spins remain raw info.
        return Ok(dead_bytes);
    }
    let accesses_a = a.hybrid_accesses.as_deref().unwrap_or(&[]);
    let accesses_b = b.hybrid_accesses.as_deref().unwrap_or(&[]);
    if accesses_a.len() != accesses_b.len()
        || !accesses_a
            .iter()
            .zip(accesses_b)
            .all(|(a, b)| accesses_match(a, b))
    {
        let a = a.hybrid_accesses.as_deref().unwrap_or(&[]);
        let b = b.hybrid_accesses.as_deref().unwrap_or(&[]);
        let index = a
            .iter()
            .zip(b)
            .position(|(a, b)| !accesses_match(a, b))
            .unwrap_or(a.len().min(b.len()));
        return Err(format!(
            "ordered device read/write trace differs at {index}: original={:?}, Rust={:?}",
            a.get(index),
            b.get(index)
        ));
    }
    if a.hybrid_writes != b.hybrid_writes {
        return Err("VDP/YM2612/PSG write trace differs".into());
    }
    Ok(dead_bytes)
}

/// Compare only physically driven read bits; raw observations remain intact for
/// diagnostics. Unused open-bus bits are not a fabricated device output.
pub(crate) fn accesses_match(
    a: &genesis_core::HybridAccess,
    b: &genesis_core::HybridAccess,
) -> bool {
    use genesis_core::HybridAccess;
    if matches!(
        b,
        HybridAccess::Read {
            purpose: m68000_emu::ReadPurpose::Discarded,
            ..
        }
    ) && !matches!(
        a,
        HybridAccess::Read {
            purpose: m68000_emu::ReadPurpose::Discarded,
            ..
        }
    ) {
        // A value-consuming original cannot be disguised as a no-value read.
        return false;
    }
    match (*a, *b) {
        (
            HybridAccess::Read {
                address: aa,
                word: aw,
                value: av,
                ..
            },
            HybridAccess::Read {
                address: ba,
                word: bw,
                value: bv,
                ..
            },
        ) if aa == ba && aw == bw => {
            let mask = match aa & 0xffffff {
                0xc00004..=0xc00007 => {
                    if aw {
                        0x03ff
                    } else if aa & 1 == 0 {
                        0x03
                    } else {
                        0xff
                    }
                }
                0xa11100 => {
                    if aw {
                        0x0100
                    } else {
                        0x01
                    }
                }
                _ => {
                    if aw {
                        0xffff
                    } else {
                        0x00ff
                    }
                }
            };
            av & mask == bv & mask
        }
        (
            HybridAccess::Write {
                address: aa,
                word: aw,
                value: av,
            },
            HybridAccess::Write {
                address: ba,
                word: bw,
                value: bv,
            },
        ) => aa == ba && aw == bw && av == bv,
        _ => false,
    }
}

/// Compare RAM as observed at every device write, not merely after return. Sparse
/// overlays avoid copying 64 KiB at every port access and tolerate redundant stores.
fn compare_io_ram_order(
    original: &[OrderedWrite],
    translated: &[OrderedWrite],
    entry_ram: &[u16],
    entry_sp: u32,
) -> std::result::Result<(), String> {
    fn next_device<'a>(
        events: &mut impl Iterator<Item = &'a OrderedWrite>,
        overlay: &mut BTreeMap<u32, u8>,
        entry_sp: u32,
    ) -> Option<&'a OrderedWrite> {
        for event in events {
            if let OrderedWrite::Ram { address, value } = *event {
                if address >= abi::canonical_ram_address(entry_sp).min(catalogue::STACK_TOP) {
                    overlay.insert(address, value);
                }
            } else {
                return Some(event);
            }
        }
        None
    }
    let (mut a, mut b) = (original.iter(), translated.iter());
    let (mut ma, mut mb) = (BTreeMap::new(), BTreeMap::new());
    loop {
        let (ea, eb) = (
            next_device(&mut a, &mut ma, entry_sp),
            next_device(&mut b, &mut mb, entry_sp),
        );
        if ea.is_none() && eb.is_none() {
            return Ok(());
        }
        if ea != eb {
            return Err("RAM/device ordered write trace differs".into());
        }
        for address in ma.keys().chain(mb.keys()) {
            let offset = (address & 0xffff) as usize;
            let initial = entry_ram[offset / 2].to_be_bytes()[offset % 2];
            if ma.get(address).copied().unwrap_or(initial)
                != mb.get(address).copied().unwrap_or(initial)
            {
                return Err(format!("RAM {address:06X} differs at device access {ea:?}"));
            }
        }
    }
}

#[cfg(test)]
mod checker_regressions {
    use super::*;
    use genesis_core::GenesisBus;
    use jgenesis_common::{debug::DebugMemoryView, frontend::TimingMode};
    use m68000_emu::BusInterface;

    #[test]
    fn device_reads_compare_driven_bits_without_dropping_accesses() {
        use genesis_core::HybridAccess;
        use m68000_emu::ReadPurpose;
        let read = |address, word, value| HybridAccess::Read {
            address,
            word,
            value,
            purpose: ReadPurpose::Semantic,
        };
        for (address, word, mask) in [
            (0xc00004, true, 0x03ff_u16),
            (0xc00004, false, 0x0003),
            (0xc00005, false, 0x00ff),
            (0xa11100, true, 0x0100),
            (0xa11100, false, 0x0001),
        ] {
            let width = if word { 0xffff } else { 0x00ff };
            let baseline = read(address, word, 0);
            let open_bus_difference = read(address, word, width & !mask);
            assert!(accesses_match(&baseline, &open_bus_difference));
            let driven_difference = read(address, word, mask.isolate_lowest_one());
            assert!(!accesses_match(&baseline, &driven_difference));
            assert!(!accesses_match(&baseline, &read(address + 2, word, 0)));
            assert!(!accesses_match(&baseline, &read(address, !word, 0)));
        }
        let baseline = read(0xc00000, true, 0);
        assert!(!accesses_match(&baseline, &read(0xc00000, true, 1)));
        let write = HybridAccess::Write {
            address: 0xc00004,
            word: true,
            value: 0,
        };
        assert!(!accesses_match(&baseline, &write));
    }

    #[test]
    fn immutable_vdp_comparison_observes_memories_registers_and_command_latch() {
        let bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        let mutations: [fn(&mut GenesisBus); 4] = [
            |bus| bus.vdp.debug_vram_view().write(10, 1),
            |bus| bus.vdp.debug_cram_view().write(1, 2),
            |bus| bus.vdp.debug_vsram_view().write(1, 2),
            |bus| bus.vdp.write_control(0x8701),
        ];
        for mutate in mutations {
            let mut changed = bus.partial_clone();
            mutate(&mut changed);
            assert!(!bus.vdp.hybrid_state_eq(&changed.vdp));
        }
        let mut original = bus.partial_clone();
        original.vdp.write_control(0x4000);
        let mut changed = original.partial_clone();
        changed.vdp.read_status(0, 7);
        assert!(!original.vdp.hybrid_state_eq(&changed.vdp));
        let mut pending = bus.partial_clone();
        pending.write_word(0xc00004, 0x8701);
        assert_ne!(bus.pending_writes, pending.pending_writes);
    }

    #[test]
    fn z80_audio_ram_corruption_is_not_masked_as_dead_68000_stack() {
        let mut original = GenesisEmulator::create(
            None,
            GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        let mut changed = original.partial_clone();
        changed.hybrid_bus_mut().memory.debug_ram_view().1[0] ^= 1;
        let symbol = tje_game::ids::GET_RANDOM_NUMBER.metadata();
        let error = compare_calls(&mut original, &mut changed, 0xff7ffc, &symbol.abi).unwrap_err();
        assert_eq!(error, "Z80 audio RAM differs");
    }

    #[test]
    fn ram_io_timeline_compares_intermediate_not_only_final_values() {
        let ram = [0_u16; 32768];
        let device = OrderedWrite::DeviceWord {
            address: 0xc00000,
            value: 1,
        };
        let mirror = OrderedWrite::Ram {
            address: 0xff8000,
            value: 1,
        };
        assert!(
            compare_io_ram_order(
                &[device.clone(), mirror.clone()],
                &[device.clone(), mirror.clone()],
                &ram,
                0xff7ffc
            )
            .is_ok()
        );
        assert!(
            compare_io_ram_order(
                &[device.clone(), mirror.clone()],
                &[mirror, device],
                &ram,
                0xff7ffc
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod rom_lease_tests;
