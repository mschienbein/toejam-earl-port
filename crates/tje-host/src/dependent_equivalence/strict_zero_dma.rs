//! Separately reviewed real-clock platform zero-count DMA consumer policy.
//! This does not widen the frozen Functional descriptor or certify its producers.

use super::{
    DependentContract, DependentEvidence, DependentProofConfig, DependentRead, DependentSource,
    ProofSetting, zero_dma::ZeroDmaPlan,
};
use crate::{
    CheckForkRoute, Host, HostError, Mode, Result,
    abi::AtomicRun,
    hooks::{Hook, HookRegistry},
    stack_dependency::CheckRun,
};
use genesis_core::{GenesisEmulator, HybridAccess};
use serde::Serialize;
use std::collections::BTreeSet;
use tje_game::{call_frame::FrameLayout, execution::ExecutionClass, ids};

mod discarded_read_value_variance;
pub use discarded_read_value_variance::DiscardedReadValueVariance;

pub(super) static CONTRACT: DependentContract = DependentContract {
    function: ids::DMA_COPY_WORDS_TO_VRAM,
    source: DependentSource::ClockedStrictWorkRamDma {
        frame: FrameLayout::movem(0, 0x1c, 0),
        producer_cohort: "Consumer-only: every historical work-RAM producer byte must be preserved, or supplied by exact cycle-engine state; immediate MOVEM preservation is insufficient.",
    },
    evidence: "G src/asm/dma.s:15-75; original Functional class with an explicit ClockedStrict proof envelope, actual D2-D4/argument/return reads, exactly 65,536 shared-device RAM source words and separately checked CPU/IRQ control.",
};

impl HookRegistry {
    /// Explicit opt-in for the one separately reviewed strict platform consumer.
    /// This never selects or broadens the frozen Functional descriptor.
    pub fn enable_clocked_strict_zero_dma(&mut self, config: DependentProofConfig) -> Result<()> {
        config.validate()?;
        let hook = self
            .at(ids::DMA_COPY_WORDS_TO_VRAM
                .metadata()
                .address(self.revision()))
            .ok_or_else(|| {
                HostError::Metadata("strict zero-count DMA root is not registered".into())
            })?;
        validate_hook(hook)?;
        self.set_dependent_proof(
            ids::DMA_COPY_WORDS_TO_VRAM,
            ProofSetting {
                contract: &CONTRACT,
                config,
            },
        );
        Ok(())
    }
}

impl Host {
    /// Enable only after independent natural/ordinary coverage, before the
    /// mandatory zero fixture. Counts, failures and machine state are retained.
    pub fn enable_clocked_strict_zero_dma(&mut self, config: DependentProofConfig) -> Result<()> {
        if self.mode != Mode::Check {
            return Err(HostError::Metadata(
                "strict zero-count DMA activation requires Check mode".into(),
            ));
        }
        validate_actual(&self.emulator)?;
        self.registry.enable_clocked_strict_zero_dma(config)
    }
}

/// One topology record per actual DMA word, in complete CPU/Z80/HostMutation
/// chronology. No data value is invented and no words coalesce across a write,
/// wrap, transaction or IRQ boundary. Actual values stay in the full read journal.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ControlLog {
    events: Vec<ControlEvent>,
}

#[derive(Debug, PartialEq, Eq)]
enum ControlEvent {
    DmaWord { raw_address: u32 },
    Access(HybridAccess),
}

impl ControlLog {
    pub(crate) fn observe(&mut self, access: HybridAccess) {
        match access {
            HybridAccess::DmaRead { address, .. } => {
                self.events.push(ControlEvent::DmaWord {
                    raw_address: address,
                });
            }
            // These indices are valid only while their payload buffer is owned.
            // Accepted IRQ envelopes are retained separately as actual events.
            HybridAccess::ExceptionBoundary { .. } => {}
            _ => self.events.push(ControlEvent::Access(access)),
        }
    }

    pub(crate) fn append(&mut self, mut other: Self) {
        self.events.append(&mut other.events);
    }
}

/// Kept separate from legacy frozen counters, including when one host exercises
/// both policies. These finite probes do not establish native producer history.
#[derive(Debug, Default, Serialize)]
pub struct StrictDmaEvidence {
    pub qualification: &'static str,
    pub probe_control_contract: &'static str,
    pub producer_history: &'static str,
    pub seed: u64,
    pub proof_calls_started: u64,
    pub calls_proven: u64,
    pub calls_without_residual_reads: u64,
    pub calls_rejected: u64,
    pub original_runs_completed: u64,
    pub rust_runs_completed: u64,
    pub seeded_probes_completed: u64,
    pub collective_probes_completed: u64,
    pub probe_pairs_matched: u64,
    /// Qualified counts advance only when the whole stable-domain call passes.
    pub qualified_final_probe_pairs: u64,
    pub qualified_source_sensitive_pairs: u64,
    pub qualified_vram_sensitive_pairs: u64,
    pub original_control_checks: u64,
    pub source_sensitive_pairs: u64,
    pub vram_sensitive_pairs: u64,
    pub last_dependent_byte_count: usize,
    pub last_source: Option<super::zero_dma::DmaSourceSummary>,
    pub discarded_read_journal_checks: u64,
    pub discarded_read_value_variances: Vec<DiscardedReadValueVariance>,
}

#[derive(Clone, Copy)]
struct ProofCallIdentity {
    seed: u64,
    ordinal: u64,
}

#[derive(Default)]
pub(super) struct ProbeProgress {
    identity: Option<ProofCallIdentity>,
    pub control_checks: u64,
    pub source_sensitive: u64,
    pub vram_sensitive: u64,
    baseline_control: Option<Vec<u8>>,
    baseline_vdp_control: Option<Vec<u8>>,
    pub discarded_read_journal_checks: u64,
    pub discarded_read_value_variances: Vec<DiscardedReadValueVariance>,
}

impl StrictDmaEvidence {
    pub(super) fn begin_call(&mut self, seed: u64) -> std::result::Result<ProbeProgress, String> {
        let ordinal = self
            .proof_calls_started
            .checked_add(1)
            .ok_or("strict zero-DMA proof-call identity exhausted")?;
        self.proof_calls_started = ordinal;
        Ok(ProbeProgress {
            identity: Some(ProofCallIdentity { seed, ordinal }),
            ..Default::default()
        })
    }

    pub(super) fn merge(&mut self, other: DependentEvidence, progress: ProbeProgress) {
        self.qualification = "finite probe evidence, not a formal proof";
        self.probe_control_contract = discarded_read_value_variance::CONTRACT;
        self.producer_history = "Consumer-only: all historical work-RAM producers must be preserved or supplied by exact cycle-engine state; native producer/ISR composition is not certified.";
        self.seed = other.seed;
        self.calls_proven += other.calls_proven;
        self.calls_without_residual_reads += other.calls_without_residual_reads;
        self.calls_rejected += other.calls_rejected;
        self.original_runs_completed += other.original_runs_completed;
        self.rust_runs_completed += other.rust_runs_completed;
        self.seeded_probes_completed += other.seeded_probes_completed;
        self.collective_probes_completed += other.collective_dma_probes_completed;
        self.probe_pairs_matched += other.probe_pairs_matched;
        self.original_control_checks += progress.control_checks;
        self.source_sensitive_pairs += progress.source_sensitive;
        self.vram_sensitive_pairs += progress.vram_sensitive;
        self.discarded_read_journal_checks += progress.discarded_read_journal_checks;
        self.discarded_read_value_variances
            .extend(progress.discarded_read_value_variances);
        if other.calls_proven != 0 {
            self.qualified_final_probe_pairs += other.probe_pairs_matched;
            self.qualified_source_sensitive_pairs += progress.source_sensitive;
            self.qualified_vram_sensitive_pairs += progress.vram_sensitive;
        }
        self.last_dependent_byte_count = other.last_dma_dependent_byte_count;
        self.last_source = other.last_dma_source;
    }
}

pub(super) fn require_stable_domain(
    assigned: &BTreeSet<u32>,
    original: &BTreeSet<u32>,
    translated: &BTreeSet<u32>,
) -> std::result::Result<(), String> {
    if assigned != original || assigned != translated {
        return Err("ClockedStrict zero-DMA dependency domain changed (growth or shrink); unsupported source shape, never restarted or approved from earlier sensitivity".into());
    }
    Ok(())
}

pub(super) fn validate_hook(hook: &Hook) -> Result<()> {
    if hook.symbol.id != ids::DMA_COPY_WORDS_TO_VRAM
        || hook.execution_class != ExecutionClass::Functional
        || hook.check_route() != CheckForkRoute::ClockedStrict
    {
        return Err(HostError::Metadata(
            "strict zero-count DMA requires the platform root, its original Functional class and exactly ClockedStrict".into(),
        ));
    }
    Ok(())
}

/// Pure, pre-effect inspection. Commuting/TimeAware and experimental retiming
/// cannot acquire this policy through a shared 'progresses time' predicate.
pub(super) fn validate_actual(machine: &GenesisEmulator) -> Result<()> {
    let bus = machine.hybrid_bus();
    if bus.hybrid_functional_mode()
        || bus
            .timing_survey
            .as_ref()
            .is_some_and(|survey| survey.policy.is_some())
    {
        return Err(HostError::Metadata(
            "strict zero-count DMA requires actual unnormalized real clocks without experimental retiming".into(),
        ));
    }
    if bus.vdp.timing_dma_active() || bus.vdp.hybrid_checkpoint().pending_dma_port_writes != 0 {
        return Err(HostError::Metadata(
            "strict zero-count DMA rejects unowned pending DMA/port writes at entry".into(),
        ));
    }
    Ok(())
}

pub(super) fn resolve(host: &mut Host, hook: &Hook) -> Result<ZeroDmaPlan> {
    validate_hook(hook)?;
    validate_actual(&host.emulator)?;
    let context = host.context();
    if context.argument_u32(2) & 0xffff != 0 {
        return Err(HostError::Metadata(
            "strict zero-count DMA rejects nonzero encoded counts".into(),
        ));
    }
    let source = context.argument_u32(0) & 0xfffffe;
    if source < 0xe00000 {
        return Err(HostError::Metadata(
            "strict zero-count DMA admits only an original whole-work-RAM source".into(),
        ));
    }
    Ok(ZeroDmaPlan::single_transfer(source))
}

pub(super) fn reject_cpu_residual(audit: &AtomicRun) -> std::result::Result<(), String> {
    for event in audit.stack_reads.as_deref().unwrap_or_default() {
        match event.read.origin {
            "vdp_dma" => {}
            "cpu" if event.uninitialized_mask != 0 && event.read.purpose == "semantic" => {
                return Err(format!(
                    "strict DMA-only source rejects genuine CPU residual at {:06X}",
                    event.read.raw_address
                ));
            }
            "cpu" => {}
            _ => return Err("strict DMA-only source observed an unreviewed origin".into()),
        }
    }
    Ok(())
}

/// Data probes preserve every non-data machine field and all physical RAM outside
/// the exact assigned input domain. No cloned machine is rewritten for comparison.
pub(super) fn compare_original_control(
    baseline: &GenesisEmulator,
    probe: &GenesisEmulator,
    baseline_run: &CheckRun,
    probe_run: &CheckRun,
    domain: &BTreeSet<u32>,
    identity: super::Probe,
    progress: &mut ProbeProgress,
) -> std::result::Result<(), String> {
    let (a, b) = (baseline.hybrid_bus(), probe.hybrid_bus());
    let verified = discarded_read_value_variance::verify(
        a.native_cpu_accesses.as_deref(),
        b.native_cpu_accesses.as_deref(),
        identity,
        progress,
    )?;
    validate_pair_structure(baseline, probe, baseline_run, probe_run)
        .map_err(|error| format!("strict DMA baseline-to-original probe control: {error}"))?;
    match (
        &baseline_run.audit.strict_control,
        &probe_run.audit.strict_control,
    ) {
        (Some(a), Some(b)) => verified.compare_controls(a, b),
        _ => Err("strict DMA actual CPU/Z80/HostMutation/DMA chronology is missing".into()),
    }
    .map_err(|error| format!("strict DMA baseline-to-original probe control: {error}"))?;
    match (&a.native_ram_writes, &b.native_ram_writes) {
        (Some(a), Some(b)) if a == b => {}
        _ => return Err("strict DMA input changed original actual RAM commit control".into()),
    }
    match (&a.native_writes, &b.native_writes) {
        (Some(a), Some(b)) if a == b => {}
        _ => return Err("strict DMA input changed original timed chip dispatch control".into()),
    }
    if progress.baseline_control.is_none() {
        progress.baseline_control = Some(
            baseline
                .hybrid_encode_non_vdp_main_ram_control_without_cpu_journal()
                .map_err(|error| format!("strict DMA original control encoding failed: {error}"))?,
        );
    }
    let original_control = progress
        .baseline_control
        .as_ref()
        .expect("captured control");
    let probe_control = probe
        .hybrid_encode_non_vdp_main_ram_control_without_cpu_journal()
        .map_err(|error| format!("strict DMA probe control encoding failed: {error}"))?;
    if original_control != &probe_control {
        return Err("strict DMA input changed original non-VDP CPU/Z80/audio/input/clock/provenance control".into());
    }
    if progress.baseline_vdp_control.is_none() {
        progress.baseline_vdp_control = Some(
            bincode::encode_to_vec(
                baseline.hybrid_bus().vdp.hybrid_control_view(),
                bincode::config::standard(),
            )
            .map_err(|error| format!("strict DMA original VDP control encoding failed: {error}"))?,
        );
    }
    let original_vdp = progress
        .baseline_vdp_control
        .as_ref()
        .expect("captured VDP control");
    let probe_vdp = bincode::encode_to_vec(
        probe.hybrid_bus().vdp.hybrid_control_view(),
        bincode::config::standard(),
    )
    .map_err(|error| format!("strict DMA probe VDP control encoding failed: {error}"))?;
    if original_vdp != &probe_vdp {
        return Err("strict DMA input changed original conservative VDP control superset (including FIFO/data latches); review required, never normalized".into());
    }
    let original_ram = a.memory.hybrid_raw_main_ram();
    let probe_ram = b.memory.hybrid_raw_main_ram();
    for (index, (&left, &right)) in original_ram.iter().zip(probe_ram).enumerate() {
        if left == right {
            continue;
        }
        for (offset, (left, right)) in left
            .to_be_bytes()
            .into_iter()
            .zip(right.to_be_bytes())
            .enumerate()
        {
            let address = 0xff0000 + 2 * index as u32 + offset as u32;
            if left != right && !domain.contains(&address) {
                return Err(format!(
                    "strict DMA input changed physical RAM outside its exact assigned domain at {address:06X}"
                ));
            }
        }
    }
    if baseline_run.wait_frames != probe_run.wait_frames
        || !baseline_run
            .audio
            .iter()
            .flatten()
            .map(|sample| sample.to_bits())
            .eq(probe_run
                .audio
                .iter()
                .flatten()
                .map(|sample| sample.to_bits()))
    {
        return Err(
            "strict DMA input changed actual frame progression or emitted audio bits".into(),
        );
    }
    Ok(())
}

/// Incoming RAM identity alone is not sensitivity. Require actual consumed DMA
/// values and actual mutable VDP output/state to change, without advancing or
/// normalizing either machine. Full cross-language equality is checked separately.
pub(super) fn sensitivity(
    baseline: &GenesisEmulator,
    probe: &GenesisEmulator,
    baseline_audit: &AtomicRun,
    probe_audit: &AtomicRun,
) -> (bool, bool) {
    let source = !dma_reads(baseline_audit).eq(dma_reads(probe_audit));
    let device =
        baseline.hybrid_bus().vdp.hybrid_memories().0 != probe.hybrid_bus().vdp.hybrid_memories().0;
    (source, device)
}

fn dma_reads(audit: &AtomicRun) -> impl Iterator<Item = &DependentRead> {
    audit
        .stack_reads
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|event| event.read.origin == "vdp_dma")
        .map(|event| &event.read)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IrqObservation {
    Enter {
        vector: u32,
        handler: u32,
        depth: u32,
        clock: u64,
    },
    Return {
        depth: u32,
        clock: u64,
    },
    Read {
        pc: u32,
        clock: u64,
        address: u32,
        word: bool,
        value: u16,
        purpose: u8,
    },
}

fn irq_observation(event: &crate::check_projection::LogicEvent) -> Option<IrqObservation> {
    use crate::check_projection::LogicEvent;
    match *event {
        LogicEvent::ExceptionEntered {
            vector,
            handler,
            depth,
            master_clock,
        } => Some(IrqObservation::Enter {
            vector,
            handler,
            depth,
            clock: master_clock,
        }),
        LogicEvent::ExceptionReturned {
            depth,
            master_clock,
        } => Some(IrqObservation::Return {
            depth,
            clock: master_clock,
        }),
        LogicEvent::Access {
            pc,
            master_clock,
            access:
                HybridAccess::Read {
                    address,
                    word,
                    value,
                    purpose,
                },
            ..
        } if (0x400000..0xe00000).contains(&(address & 0xffffff)) => Some(IrqObservation::Read {
            pc,
            clock: master_clock,
            address: address & 0xffffff,
            word,
            value,
            purpose: purpose as u8,
        }),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RawIrqRead {
    Acknowledge(u8),
    Read {
        address: u32,
        word: bool,
        value: u16,
        purpose: u8,
    },
}

fn validate_irq_reads(
    machine: &GenesisEmulator,
    run: &CheckRun,
) -> std::result::Result<(), String> {
    use genesis_core::native::CpuAccess;
    let raw = machine
        .hybrid_bus()
        .native_cpu_accesses
        .as_deref()
        .ok_or("strict DMA lacks actual CPU-issued IRQ/read evidence")?;
    let mut expected = raw.iter().filter_map(|event| match *event {
        CpuAccess::AcknowledgeInterrupt { level } => Some(RawIrqRead::Acknowledge(level)),
        CpuAccess::Read {
            address,
            word,
            value,
            purpose,
        } if (0x400000..0xe00000).contains(&(address & 0xffffff)) => Some(RawIrqRead::Read {
            address: address & 0xffffff,
            word,
            value,
            purpose,
        }),
        _ => None,
    });
    let mut depth = 0_u32;
    let mut previous_clock = 0;
    for event in run.logic_trace.events.iter().filter_map(irq_observation) {
        let clock = match event {
            IrqObservation::Enter {
                vector,
                depth: entered,
                clock,
                ..
            } => {
                if !matches!(vector, 28 | 30)
                    || entered != depth + 1
                    || expected.next() != Some(RawIrqRead::Acknowledge((vector - 24) as u8))
                {
                    return Err(
                        "strict DMA has an unsupported/unwitnessed relative IRQ entry".into(),
                    );
                }
                depth = entered;
                clock
            }
            IrqObservation::Return {
                depth: returned,
                clock,
            } => {
                if depth == 0 || returned + 1 != depth {
                    return Err("strict DMA has an unbalanced relative IRQ return".into());
                }
                depth = returned;
                clock
            }
            IrqObservation::Read {
                address,
                word,
                value,
                purpose,
                clock,
                ..
            } => {
                if depth == 0 {
                    return Err("strict DMA rejects root-thread device feedback reads, including an already-entered caller ISR".into());
                }
                if expected.next()
                    != Some(RawIrqRead::Read {
                        address,
                        word,
                        value,
                        purpose,
                    })
                {
                    return Err("strict DMA IRQ read lacks its exact actual raw CPU witness".into());
                }
                clock
            }
        };
        if clock < previous_clock {
            return Err("strict DMA IRQ/read clocks are not monotonic".into());
        }
        previous_clock = clock;
    }
    if depth != 0 || expected.next().is_some() {
        return Err("strict DMA has unfinished IRQs or omitted actual IRQ/device reads".into());
    }
    Ok(())
}

fn validate_root_start(
    machine: &GenesisEmulator,
    run: &CheckRun,
) -> std::result::Result<(), String> {
    use crate::check_projection::LogicEvent;
    use genesis_core::timing_survey::{TimingCpu, TimingKind};
    let bus = machine.hybrid_bus();
    let capture = bus
        .hybrid_dma_capture()
        .ok_or("strict DMA lacks its owned actual dispatch-scope capture")?;
    if capture.invalid_depth_sequence
        || capture.missing_dispatch_context
        || capture.active_dma_port_write_seen
        || capture.deferred_dma_port_write_seen
    {
        return Err("strict DMA has unknown exception scope or an unowned/deferred DMA port write; internal chaining is unsupported".into());
    }
    if capture.current_exception_depth != capture.entry_exception_depth
        || capture.current_exception_depth != machine.hybrid_cpu().hybrid_exception_depth()
    {
        return Err(
            "strict DMA actual exception scope did not return to its real entry depth".into(),
        );
    }
    let mut logical = run
        .logic_trace
        .events
        .iter()
        .filter_map(irq_observation)
        .filter(|event| !matches!(event, IrqObservation::Read { .. }));
    let mut previous_clock = 0;
    for owned in &capture.boundaries {
        if owned.master_clock < previous_clock {
            return Err("strict DMA owned exception boundaries are not chronological".into());
        }
        previous_clock = owned.master_clock;
        let matched = match (owned.boundary, logical.next()) {
            (
                m68000_emu::ExceptionBoundary::Entered {
                    vector,
                    handler,
                    depth,
                },
                Some(IrqObservation::Enter {
                    vector: v,
                    handler: h,
                    depth: relative,
                    ..
                }),
            ) => {
                vector == v
                    && handler == h
                    && depth.checked_sub(capture.entry_exception_depth) == Some(relative)
            }
            (
                m68000_emu::ExceptionBoundary::Returned { depth },
                Some(IrqObservation::Return {
                    depth: relative, ..
                }),
            ) => depth.checked_sub(capture.entry_exception_depth) == Some(relative),
            _ => false,
        };
        if !matched {
            return Err(
                "strict DMA has missing/mismatched actual owned IRQ-boundary evidence".into(),
            );
        }
    }
    if logical.next().is_some() {
        return Err("strict DMA IRQ envelope lacks its actual successful boundary markers".into());
    }
    let [owned] = capture.starts.as_slice() else {
        return Err(
            "strict DMA requires one actual owned external root start; extra/foreign starts reject"
                .into(),
        );
    };
    if !root_dispatch_scope(capture, owned) || owned.origin != genesis_core::native::Origin::Main {
        return Err(
            "strict DMA actual start was not in the invocation's root-relative exception scope"
                .into(),
        );
    }
    let survey = bus
        .timing_survey
        .as_ref()
        .ok_or("strict DMA lacks actual DMA-start evidence")?;
    let mut starts = survey
        .accesses
        .iter()
        .filter(|event| event.kind == TimingKind::DmaStart);
    let start = starts
        .next()
        .ok_or("strict DMA never started its actual root transfer")?;
    if starts.next().is_some()
        || start.cpu != TimingCpu::M68k
        || !start.write
        || start.pc != owned.pc
        || start.clock != owned.master_clock
        || start.address != owned.address
        || start.value != owned.value
    {
        return Err("strict DMA actual start and owned dispatch witness differ".into());
    }
    // This only ties the already-qualified actual dispatch to its control stream.
    // Similar PC/value writes elsewhere never establish dispatch scope.
    let mut depth = 0;
    let mut root_writes = 0;
    for event in &run.logic_trace.events {
        match *event {
            LogicEvent::ExceptionEntered { depth: entered, .. } => depth = entered,
            LogicEvent::ExceptionReturned {
                depth: returned, ..
            } => depth = returned,
            LogicEvent::Access {
                pc,
                access:
                    HybridAccess::Write {
                        address,
                        word: true,
                        value,
                    },
                ..
            } if depth == 0
                && pc == owned.pc
                && address == owned.address
                && value == owned.value =>
            {
                root_writes += 1
            }
            _ => {}
        }
    }
    if root_writes != 1 {
        return Err("strict DMA owned dispatch lacks one matching root control write".into());
    }
    let dispatched = bus
        .native_writes
        .as_deref()
        .ok_or("strict DMA lacks actual chip dispatch evidence")?
        .iter()
        .filter(|event| {
            event.clock == owned.master_clock
                && event.pc == owned.pc
                && event.origin == owned.origin
                && event.write
                    == genesis_core::native::DeviceWrite::Vdp {
                        port: 4,
                        value: owned.value,
                    }
        })
        .count();
    if dispatched != 1 {
        return Err("strict DMA start lacks its actual timed control-port dispatch".into());
    }
    Ok(())
}

fn root_dispatch_scope(
    capture: &genesis_core::HybridDmaCapture,
    start: &genesis_core::HybridDmaStart,
) -> bool {
    start.entry_exception_depth == capture.entry_exception_depth
        && start.exception_depth == capture.entry_exception_depth
}

fn validate_pair_structure(
    original: &GenesisEmulator,
    translated: &GenesisEmulator,
    original_run: &CheckRun,
    translated_run: &CheckRun,
) -> std::result::Result<(), String> {
    if original_run.nested_calls != 0 || translated_run.nested_calls != 0 {
        return Err(
            "strict platform DMA has no original callees; forwarded main execution is not proof"
                .into(),
        );
    }
    reject_cpu_residual(&original_run.audit)?;
    reject_cpu_residual(&translated_run.audit)?;
    validate_irq_reads(original, original_run)?;
    validate_irq_reads(translated, translated_run)?;
    validate_root_start(original, original_run)?;
    validate_root_start(translated, translated_run)?;
    if !original_run
        .logic_trace
        .events
        .iter()
        .filter_map(irq_observation)
        .eq(translated_run
            .logic_trace
            .events
            .iter()
            .filter_map(irq_observation))
    {
        return Err("strict DMA exact relative IRQ/read envelope differs".into());
    }
    Ok(())
}

pub(super) fn validate_pair(
    original: &GenesisEmulator,
    translated: &GenesisEmulator,
    original_run: &CheckRun,
    translated_run: &CheckRun,
) -> std::result::Result<(), String> {
    validate_pair_structure(original, translated, original_run, translated_run)?;
    match (
        &original_run.audit.strict_control,
        &translated_run.audit.strict_control,
    ) {
        (Some(a), Some(b)) => {
            let first_difference = a
                .events
                .iter()
                .zip(&b.events)
                .position(|(left, right)| left != right)
                .or_else(|| {
                    (a.events.len() != b.events.len()).then_some(a.events.len().min(b.events.len()))
                });
            if let Some(index) = first_difference {
                return Err(control_difference(a, b, index));
            }
            Ok(())
        }
        _ => Err("strict DMA actual CPU/Z80/HostMutation/DMA chronology is missing".into()),
    }
}

fn control_difference(a: &ControlLog, b: &ControlLog, index: usize) -> String {
    let start = index.saturating_sub(2);
    let end = index.saturating_add(3);
    format!(
        "strict DMA actual CPU/Z80/HostMutation/DMA chronology differs at event {index}: left={:?}, right={:?}; lengths left={}, right={}; context left={:?}, right={:?}",
        a.events.get(index),
        b.events.get(index),
        a.events.len(),
        b.events.len(),
        &a.events[start..end.min(a.events.len())],
        &b.events[start..end.min(b.events.len())]
    )
}

#[cfg(test)]
mod tests;
