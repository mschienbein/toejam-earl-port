//! Explicit, reviewed equivalence for genuinely dependent residual frame bytes.
//!
//! Unmarked hooks still require same-side independence. A marked hook instead
//! reproduces the actual residual reads and tracks original outputs under coupled
//! perturbations; neither a LocalFrame nor an address list alone is approval.

pub(crate) mod journal;
mod strict_zero_dma;
mod zero_dma;
pub use journal::DependentRead;
pub(crate) use strict_zero_dma::ControlLog;
pub use strict_zero_dma::{DiscardedReadValueVariance, StrictDmaEvidence};
pub use zero_dma::{DmaSourceSummary, ZeroDmaRoot};

use crate::{
    Host, HostError, Mode, Result, StackReadSite,
    abi::{OrderedWrite, canonical_ram_address},
    call_clock::ClockRunPurpose,
    catalogue::{STACK_BOTTOM, STACK_TOP},
    exits,
    hooks::{Hook, HookRegistry},
    stack_dependency::{CheckRun, CheckSide, compare_check_runs},
};
use genesis_core::GenesisEmulator;
use journal::DerivedReads;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use tje_game::{FunctionId, call_frame::FrameLayout, ids};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DependentProofConfig {
    pub seed: u64,
    /// Coupled seeded assignments in addition to the source kind's boundary
    /// probes: individual bytes for frames, collective fills for zero DMA.
    /// Values below 16 are rejected before any entry executes.
    pub seeded_probes: u16,
}
impl Default for DependentProofConfig {
    fn default() -> Self {
        Self {
            seed: 0x544a_452d_7265_7369,
            seeded_probes: 16,
        }
    }
}
impl DependentProofConfig {
    fn validate(self) -> Result<()> {
        if self.seeded_probes < 16 {
            return Err(HostError::Metadata(
                "dependent equivalence requires at least 16 seeded probes".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FrameByteRange {
    pub entry_sp_offset: i16,
    pub bytes: u16,
}

/// The actual compiled producer, not a promise that a Rust constant is the old
/// value. Authored producer fixtures and natural last-writer observations must
/// be distinguished in evidence.
#[derive(Debug, Clone, Copy)]
pub struct CompiledProducer {
    pub function: FunctionId,
    pub caller: FunctionId,
    pub caller_frame: FrameLayout,
    pub frame: FrameLayout,
    pub entry_sp_delta: i16,
    pub source: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub enum DependentSource {
    FrameResidual {
        frame: FrameLayout,
        ranges: &'static [FrameByteRange],
        producer: CompiledProducer,
    },
    /// Exact original zero-encoded RAM-to-VRAM data plane, not a generic RAM
    /// span. The whole historical RAM producer state remains an obligation.
    ZeroEncodedWorkRamDma {
        frame: FrameLayout,
        root: ZeroDmaRoot,
        producer_cohort: &'static str,
    },
    /// Separately reviewed real-clock platform data plane. Neither the legacy
    /// frozen contract nor a commuting envelope can acquire this policy.
    ClockedStrictWorkRamDma {
        frame: FrameLayout,
        producer_cohort: &'static str,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct DependentContract {
    pub function: FunctionId,
    pub source: DependentSource,
    pub evidence: &'static str,
}
impl DependentContract {
    fn permits_frame_byte(self, entry_sp: u32, address: u32) -> bool {
        let DependentSource::FrameResidual { ranges, .. } = self.source else {
            return false;
        };
        let base = canonical_ram_address(entry_sp);
        ranges.iter().any(|range| {
            let start = i64::from(base) + i64::from(range.entry_sp_offset);
            let end = start + i64::from(range.bytes);
            range.bytes != 0
                && start >= i64::from(STACK_BOTTOM)
                && end <= i64::from(base.min(STACK_TOP))
                && (start..end).contains(&i64::from(address))
        })
    }
}

/// Source-reviewed eligibility only, never automatic activation or proof of
/// replacement composition. DMA whole-RAM inputs require their entire producer
/// history, not merely preservation of the immediate call's save area.
pub static REVIEWED_CONTRACTS: &[DependentContract] = &[
    DependentContract {
        function: ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
        source: DependentSource::FrameResidual {
            frame: FrameLayout::link(12, 0x7c, 0x1c),
            ranges: &[FrameByteRange {
                entry_sp_offset: -12,
                bytes: 8,
            }],
            producer: CompiledProducer {
                function: ids::SET_EARTHLING_STATE,
                caller: ids::AI_GENERIC_ENEMY_THINK,
                caller_frame: FrameLayout::link(4, 0x7c, 0x3c),
                frame: FrameLayout::movem(0, 0x0c, 0x1c),
                entry_sp_delta: 8,
                source: "G c_generic_enemy.s:550-555 and c_spawning_and_banks.s:975-976; same steady SP, two versus four argument slots",
            },
        },
        evidence: "G src/earthlings/targeting.c:18-122; c_targeting.s LINK #-12 and selected delta words -8/-6/-4/-2(A6)",
    },
    DependentContract {
        function: ids::DMA_COPY_WORDS_TO_VRAM,
        source: DependentSource::ZeroEncodedWorkRamDma {
            frame: FrameLayout::movem(0, 0x1c, 0),
            root: ZeroDmaRoot::Platform,
            producer_cohort: "All historical work-RAM producers must be preserved, or entry state must come from the exact cycle engine; immediate MOVEM preservation alone is insufficient.",
        },
        evidence: "G src/asm/dma.s:15-75; low sixteen count bits zero means 65,536 source words, with original VDP 128-KiB raw-bank wrapping.",
    },
    DependentContract {
        function: ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
        source: DependentSource::ZeroEncodedWorkRamDma {
            frame: FrameLayout::movem(0, 0x1c, 0x04),
            root: ZeroDmaRoot::QueuedSpriteGraphics,
            producer_cohort: "All historical work-RAM producers must be preserved, or entry state must come from the exact cycle engine; the Upload/DMA frame cohort alone is insufficient.",
        },
        evidence: "G src/sprites/load_sprites.c:8-31 and c_load_sprites.s:4-68; tiles_only excludes the nonzero table prefix; byte queue bound and every narrowed run count must encode zero.",
    },
];

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProofSetting {
    pub contract: &'static DependentContract,
    pub config: DependentProofConfig,
}

impl ProofSetting {
    pub(crate) fn captures_all_dma(self) -> bool {
        matches!(
            self.contract.source,
            DependentSource::ZeroEncodedWorkRamDma { .. }
                | DependentSource::ClockedStrictWorkRamDma { .. }
        )
    }

    pub(crate) fn is_clocked_strict(self) -> bool {
        matches!(
            self.contract.source,
            DependentSource::ClockedStrictWorkRamDma { .. }
        )
    }

    pub(crate) fn requires_frozen(self) -> bool {
        matches!(
            self.contract.source,
            DependentSource::ZeroEncodedWorkRamDma { .. }
        )
    }

    pub(crate) fn validate_route(self, hook: &Hook) -> Result<()> {
        if self.is_clocked_strict() {
            strict_zero_dma::validate_hook(hook)
        } else if self.requires_frozen()
            && hook.check_route() != crate::CheckForkRoute::FrozenFunctional
        {
            Err(HostError::Metadata(
                "zero-count DMA proof requires its reviewed ClassDefined frozen Functional fork"
                    .into(),
            ))
        } else {
            Ok(())
        }
    }

    pub(crate) fn validate_actual_strict(self, machine: &GenesisEmulator) -> Result<()> {
        if self.is_clocked_strict() {
            strict_zero_dma::validate_actual(machine)
        } else {
            Ok(())
        }
    }

    fn validate_dma_pair(
        self,
        plan: zero_dma::ZeroDmaPlan,
        original: &GenesisEmulator,
        translated: &GenesisEmulator,
        original_run: &CheckRun,
        translated_run: &CheckRun,
        bytes: &BTreeSet<u32>,
    ) -> std::result::Result<(), String> {
        plan.compare(&original_run.audit, &translated_run.audit, bytes)?;
        if self.is_clocked_strict() {
            strict_zero_dma::validate_pair(original, translated, original_run, translated_run)
        } else {
            plan.reject_feedback(original, &original_run.audit)?;
            plan.reject_feedback(translated, &translated_run.audit)
        }
    }
}

impl HookRegistry {
    /// Activate only named, registered, source-reviewed roots. An empty slice
    /// enables nothing; this is never a global dead-stack waiver.
    pub fn enable_dependent_equivalence(
        &mut self,
        roots: &[FunctionId],
        config: DependentProofConfig,
    ) -> Result<()> {
        config.validate()?;
        // Validate the whole request before changing any hook.
        for id in roots {
            if !REVIEWED_CONTRACTS
                .iter()
                .any(|contract| contract.function == *id)
            {
                return Err(HostError::Metadata(format!(
                    "no reviewed dependent-equivalence contract for {}",
                    id.metadata().name
                )));
            }
            if !self
                .at(id.metadata().address(self.revision()))
                .is_some_and(|hook| hook.symbol.id == *id)
            {
                return Err(HostError::Metadata(format!(
                    "dependent-equivalence root is not registered: {}",
                    id.metadata().name
                )));
            }
            let reviewed = REVIEWED_CONTRACTS
                .iter()
                .find(|contract| contract.function == *id)
                .expect("validated dependent contract");
            if matches!(
                reviewed.source,
                DependentSource::ZeroEncodedWorkRamDma { .. }
            ) && self
                .at(id.metadata().address(self.revision()))
                .is_none_or(|hook| hook.check_route() != crate::CheckForkRoute::FrozenFunctional)
            {
                return Err(HostError::Metadata(
                    "zero-count DMA approval is ClassDefined frozen Functional only; stronger envelopes require separate review".into(),
                ));
            }
        }
        for id in roots {
            let contract = REVIEWED_CONTRACTS
                .iter()
                .find(|contract| contract.function == *id)
                .expect("validated contract");
            self.set_dependent_proof(*id, ProofSetting { contract, config });
        }
        Ok(())
    }
}

#[derive(Debug, Default, Serialize)]
pub struct DependentEvidence {
    pub seed: u64,
    pub calls_proven: u64,
    pub calls_without_residual_reads: u64,
    pub calls_rejected: u64,
    pub original_runs_completed: u64,
    pub rust_runs_completed: u64,
    pub seeded_probes_completed: u64,
    pub byte_probes_completed: u64,
    pub collective_dma_probes_completed: u64,
    pub last_dma_dependent_byte_count: usize,
    pub last_dma_source: Option<DmaSourceSummary>,
    pub probe_pairs_matched: u64,
    pub sensitive_probe_pairs: u64,
    /// Exact small frame-source sites; zero DMA reports bounded cardinality instead.
    pub dependent_bytes: BTreeSet<u32>,
    /// Small frame-source trace only. Full DMA values remain in the internal journal.
    pub last_read_trace: Vec<DependentRead>,
    /// Distinct policy counters; frozen successes never backfill strict proof.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clocked_strict: Option<StrictDmaEvidence>,
}
impl DependentEvidence {
    fn merge(&mut self, other: Self) {
        self.seed = other.seed;
        self.calls_proven += other.calls_proven;
        self.calls_without_residual_reads += other.calls_without_residual_reads;
        self.calls_rejected += other.calls_rejected;
        self.original_runs_completed += other.original_runs_completed;
        self.rust_runs_completed += other.rust_runs_completed;
        self.seeded_probes_completed += other.seeded_probes_completed;
        self.byte_probes_completed += other.byte_probes_completed;
        self.collective_dma_probes_completed += other.collective_dma_probes_completed;
        self.last_dma_dependent_byte_count = other.last_dma_dependent_byte_count;
        self.last_dma_source = other.last_dma_source;
        self.probe_pairs_matched += other.probe_pairs_matched;
        self.sensitive_probe_pairs += other.sensitive_probe_pairs;
        self.dependent_bytes.extend(other.dependent_bytes);
        self.last_read_trace = other.last_read_trace;
    }
}

pub(crate) type RamWriteFootprint = BTreeMap<(u32, u8), u64>;
pub(crate) fn ram_write_footprint(writes: &[OrderedWrite]) -> RamWriteFootprint {
    let mut footprint = BTreeMap::new();
    for write in writes {
        if let OrderedWrite::Ram { address, value } = *write {
            *footprint.entry((address, value)).or_default() += 1;
        }
    }
    footprint
}

fn compare_outputs(
    original: &mut GenesisEmulator,
    translated: &mut GenesisEmulator,
    original_run: &CheckRun,
    translated_run: &CheckRun,
    hook: &Hook,
    entry_ram: &[u16],
    entry_sp: u32,
) -> std::result::Result<u64, String> {
    let a = original_run
        .dependent_writes
        .as_ref()
        .expect("enabled original footprint");
    let b = translated_run
        .dependent_writes
        .as_ref()
        .expect("enabled Rust footprint");
    if a != b {
        let &(address, value) = a
            .keys()
            .chain(b.keys())
            .find(|key| a.get(key) != b.get(key))
            .expect("different footprints");
        return Err(format!(
            "dependent-equivalence RAM write/value footprint differs (including dead stack) at {address:06X}, value {value:02X}: original count {}, Rust count {}",
            a.get(&(address, value)).copied().unwrap_or_default(),
            b.get(&(address, value)).copied().unwrap_or_default(),
        ));
    }
    if original.hybrid_cpu().status_register() != translated.hybrid_cpu().status_register() {
        return Err(format!(
            "dependent-equivalence full SR/CCR: original={:04X}, Rust={:04X}",
            original.hybrid_cpu().status_register(),
            translated.hybrid_cpu().status_register()
        ));
    }
    compare_check_runs(
        original,
        translated,
        original_run,
        translated_run,
        hook,
        entry_ram,
        entry_sp,
    )
}

struct Baselines<'a> {
    original: &'a mut GenesisEmulator,
    translated: &'a mut GenesisEmulator,
    original_run: &'a CheckRun,
    translated_run: &'a CheckRun,
}

#[derive(Clone, Copy)]
enum Probe {
    Seeded(u16),
    Byte { address: u32, value: u8 },
    DmaFill(u8),
}
fn perturb(emulator: &mut GenesisEmulator, addresses: &[u32], probe: Probe, seed: u64) {
    let memory = &mut emulator.hybrid_bus_mut().memory;
    match probe {
        Probe::Byte { address, value } => memory.write_main_ram_byte(address, value),
        Probe::DmaFill(value) => {
            for &address in addresses {
                memory.write_main_ram_byte(address, value);
            }
        }
        Probe::Seeded(index) => {
            let mut state = seed.wrapping_add(u64::from(index).wrapping_mul(0x9e37_79b9_7f4a_7c15));
            for &address in addresses {
                state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut value = state;
                value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                value ^= value >> 31;
                memory.write_main_ram_byte(address, value as u8);
            }
        }
    }
}

fn compare_reads(
    a: &DerivedReads<'_>,
    b: &DerivedReads<'_>,
    bytes: &BTreeSet<u32>,
) -> std::result::Result<Vec<DependentRead>, String> {
    let original = a.trace(bytes);
    let translated = b.trace(bytes);
    if original != translated {
        let first = original
            .iter()
            .zip(&translated)
            .position(|(a, b)| a != b)
            .unwrap_or(original.len().min(translated.len()));
        return Err(format!(
            "dependent residual read trace differs at access {first}: original={:?}, Rust={:?}",
            original.get(first),
            translated.get(first)
        ));
    }
    Ok(original)
}

impl Host {
    /// Activate a reviewed proof scope after natural check coverage and before
    /// fixture seeding. Only settings change; prior coverage/failures and every
    /// CPU/device/clock byte are retained. Restricted DMA markers never auto-skip
    /// a later nonzero transfer.
    pub fn enable_dependent_equivalence(
        &mut self,
        roots: &[FunctionId],
        config: DependentProofConfig,
    ) -> Result<()> {
        if self.mode != Mode::Check {
            return Err(HostError::Metadata(
                "live dependent-proof activation requires check mode".into(),
            ));
        }
        self.registry.enable_dependent_equivalence(roots, config)
    }

    pub(super) fn resolve_dependent_source(
        &mut self,
        hook: &Hook,
    ) -> Result<Option<zero_dma::ZeroDmaPlan>> {
        let Some(setting) = hook.dependent_proof else {
            return Ok(None);
        };
        setting.config.validate()?;
        match setting.contract.source {
            DependentSource::FrameResidual { .. } => Ok(None),
            DependentSource::ZeroEncodedWorkRamDma { root, .. } => {
                root.resolve(self, hook).map(Some)
            }
            DependentSource::ClockedStrictWorkRamDma { .. } => {
                strict_zero_dma::resolve(self, hook).map(Some)
            }
        }
    }

    pub(super) fn check_dependent_equivalence(
        &mut self,
        original: &mut GenesisEmulator,
        translated: &mut GenesisEmulator,
        original_run: &CheckRun,
        translated_run: &CheckRun,
        hook: &Hook,
        dma_plan: Option<zero_dma::ZeroDmaPlan>,
    ) -> std::result::Result<u64, String> {
        let setting = hook.dependent_proof.expect("marked hook");
        let mut evidence = DependentEvidence {
            seed: setting.config.seed,
            ..DependentEvidence::default()
        };
        let mut sites = BTreeSet::new();
        let mut strict_progress = if setting.is_clocked_strict() {
            Some(
                self.checks
                    .dependent_equivalence
                    .entry(hook.symbol.name.to_owned())
                    .or_default()
                    .clocked_strict
                    .get_or_insert_with(Default::default)
                    .begin_call(setting.config.seed)?,
            )
        } else {
            None
        };
        let result = self.prove_dependent(
            &mut Baselines {
                original,
                translated,
                original_run,
                translated_run,
            },
            hook,
            &mut evidence,
            &mut sites,
            dma_plan,
            strict_progress.as_mut(),
        );
        if result.is_err() {
            evidence.calls_rejected += 1;
        }
        let destination = if result.is_ok() {
            &mut self.checks.dependent_stack_reads
        } else {
            &mut self.checks.uninitialized_stack_reads
        };
        for site in sites {
            destination.insert(StackReadSite {
                function: hook.symbol.name.to_owned(),
                pc: site.pc,
                address: site.address,
            });
        }
        if let Some(progress) = strict_progress {
            self.checks
                .dependent_equivalence
                .get_mut(hook.symbol.name)
                .and_then(|entry| entry.clocked_strict.as_mut())
                .expect("strict proof-call entry was reserved before execution")
                .merge(evidence, progress);
        } else {
            self.checks
                .dependent_equivalence
                .entry(hook.symbol.name.to_owned())
                .or_default()
                .merge(evidence);
        }
        result
    }

    fn prove_dependent(
        &mut self,
        baseline: &mut Baselines<'_>,
        hook: &Hook,
        evidence: &mut DependentEvidence,
        sites: &mut BTreeSet<crate::abi::UninitializedStackRead>,
        dma_plan: Option<zero_dma::ZeroDmaPlan>,
        mut strict_progress: Option<&mut strict_zero_dma::ProbeProgress>,
    ) -> std::result::Result<u64, String> {
        let setting = hook.dependent_proof.expect("marked hook");
        setting
            .config
            .validate()
            .map_err(|error| error.to_string())?;
        setting
            .validate_route(hook)
            .map_err(|error| error.to_string())?;
        setting
            .validate_actual_strict(&self.emulator)
            .map_err(|error| error.to_string())?;
        let entry = exits::EntryState::capture(&self.context(), hook.exit_contract)
            .map_err(|error| error.to_string())?;
        let sp = entry.sp;
        let frame_sp = hook.dependent_frame_sp(sp);
        let original_reads =
            DerivedReads::derive_with_sites(&baseline.original_run.audit, sp, dma_plan.is_none());
        let translated_reads =
            DerivedReads::derive_with_sites(&baseline.translated_run.audit, sp, dma_plan.is_none());
        sites.extend(
            original_reads
                .sites
                .iter()
                .chain(&translated_reads.sites)
                .copied(),
        );
        let mut bytes: BTreeSet<_> = original_reads
            .bytes
            .union(&translated_reads.bytes)
            .copied()
            .collect();
        if let Some(plan) = dma_plan {
            evidence.last_dma_dependent_byte_count = bytes.len();
            setting.validate_dma_pair(
                plan,
                baseline.original,
                baseline.translated,
                baseline.original_run,
                baseline.translated_run,
                &bytes,
            )?;
            evidence.last_dma_source = Some(plan.summary());
        }
        let mut baseline_dead = None;
        if bytes.is_empty() {
            evidence.calls_without_residual_reads += 1;
            let (entry_ram, _) = self.emulator.hybrid_bus_mut().memory.debug_ram_view();
            return compare_outputs(
                baseline.original,
                baseline.translated,
                baseline.original_run,
                baseline.translated_run,
                hook,
                entry_ram,
                sp,
            );
        }
        let mut sensitive = false;
        loop {
            if dma_plan.is_some() {
                evidence.last_dma_dependent_byte_count = bytes.len();
            } else {
                evidence.dependent_bytes.extend(&bytes);
            }
            for &address in &bytes {
                if dma_plan.is_none() && !setting.contract.permits_frame_byte(frame_sp, address) {
                    return Err(format!(
                        "dependent residual byte {address:06X} is outside the reviewed source frame ranges"
                    ));
                }
            }
            if dma_plan.is_none() {
                evidence.last_read_trace =
                    compare_reads(&original_reads, &translated_reads, &bytes)?;
            }
            if baseline_dead.is_none() {
                let (entry_ram, _) = self.emulator.hybrid_bus_mut().memory.debug_ram_view();
                baseline_dead = Some(compare_outputs(
                    baseline.original,
                    baseline.translated,
                    baseline.original_run,
                    baseline.translated_run,
                    hook,
                    entry_ram,
                    sp,
                )?);
            }
            let addresses: Vec<_> = bytes.iter().copied().collect();
            let seeded = usize::from(setting.config.seeded_probes);
            let mut grew = false;
            let boundary_probes = if dma_plan.is_some() {
                4
            } else {
                4 * addresses.len()
            };
            for index in 0..seeded + boundary_probes {
                let probe = if index < seeded {
                    Probe::Seeded(index as u16)
                } else if dma_plan.is_some() {
                    Probe::DmaFill([0x00, 0xff, 0x7f, 0x80][index - seeded])
                } else {
                    let byte_index = index - seeded;
                    Probe::Byte {
                        address: addresses[byte_index / 4],
                        value: [0x00, 0xff, 0x7f, 0x80][byte_index % 4],
                    }
                };
                let purpose = match probe {
                    Probe::Seeded(index) => ClockRunPurpose::DependentSeed {
                        seed: setting.config.seed,
                        index,
                    },
                    Probe::Byte { address, value } => {
                        ClockRunPurpose::DependentByte { address, value }
                    }
                    Probe::DmaFill(value) => ClockRunPurpose::DependentDmaFill { value },
                };
                setting
                    .validate_route(hook)
                    .map_err(|error| error.to_string())?;
                setting
                    .validate_actual_strict(&self.emulator)
                    .map_err(|error| error.to_string())?;
                let mut rust = self.fork();
                perturb(&mut rust, &addresses, probe, setting.config.seed);
                self.checks.stack_dependency_reruns += 1;
                let rust_run = self
                    .run_check_side(
                        &mut rust,
                        hook,
                        CheckSide::Rust,
                        purpose,
                        crate::future_frame_watch::ObservationOrigin::DependencyProbe,
                    )
                    .map_err(|error| format!("dependent Rust probe did not complete: {error}"))?;
                evidence.rust_runs_completed += 1;
                let boundary =
                    exits::reference_boundary(hook, rust_run.outcome, entry, self.revision)
                        .map_err(|error| error.to_string())?;
                setting
                    .validate_route(hook)
                    .map_err(|error| error.to_string())?;
                setting
                    .validate_actual_strict(&self.emulator)
                    .map_err(|error| error.to_string())?;
                let mut original = self.fork();
                perturb(&mut original, &addresses, probe, setting.config.seed);
                self.checks.stack_dependency_reruns += 1;
                let original_run = self
                    .run_check_side(
                        &mut original,
                        hook,
                        CheckSide::Original {
                            boundary,
                            outcome: rust_run.outcome,
                        },
                        purpose,
                        crate::future_frame_watch::ObservationOrigin::DependencyProbe,
                    )
                    .map_err(|error| {
                        format!("dependent original probe did not complete: {error}")
                    })?;
                evidence.original_runs_completed += 1;
                match probe {
                    Probe::Seeded(_) => evidence.seeded_probes_completed += 1,
                    Probe::Byte { .. } => evidence.byte_probes_completed += 1,
                    Probe::DmaFill(_) => evidence.collective_dma_probes_completed += 1,
                }
                if let Some(plan) = dma_plan {
                    setting.validate_dma_pair(
                        plan,
                        &original,
                        &rust,
                        &original_run,
                        &rust_run,
                        &bytes,
                    )?;
                }
                let a =
                    DerivedReads::derive_with_sites(&original_run.audit, sp, dma_plan.is_none());
                let b = DerivedReads::derive_with_sites(&rust_run.audit, sp, dma_plan.is_none());
                sites.extend(a.sites.iter().chain(&b.sites).copied());
                if setting.is_clocked_strict() {
                    strict_zero_dma::require_stable_domain(&bytes, &a.bytes, &b.bytes)?;
                }
                let previous = bytes.len();
                bytes.extend(a.bytes.iter().chain(&b.bytes));
                if bytes.len() != previous {
                    grew = true;
                    break;
                }
                if dma_plan.is_none() {
                    compare_reads(&a, &b, &bytes)?;
                }
                let (entry_ram, _) = self.emulator.hybrid_bus_mut().memory.debug_ram_view();
                compare_outputs(
                    &mut original,
                    &mut rust,
                    &original_run,
                    &rust_run,
                    hook,
                    entry_ram,
                    sp,
                )?;
                if let Some(progress) = strict_progress.as_deref_mut() {
                    strict_zero_dma::compare_original_control(
                        baseline.original,
                        &original,
                        baseline.original_run,
                        &original_run,
                        &bytes,
                        probe,
                        progress,
                    )?;
                    progress.control_checks += 1;
                    let (source, vram) = strict_zero_dma::sensitivity(
                        baseline.original,
                        &original,
                        &baseline.original_run.audit,
                        &original_run.audit,
                    );
                    progress.source_sensitive += u64::from(source);
                    progress.vram_sensitive += u64::from(vram);
                    if source && vram {
                        sensitive = true;
                        evidence.sensitive_probe_pairs += 1;
                    }
                } else {
                    // Legacy policies retain their original output-sensitivity test.
                    if compare_outputs(
                        baseline.original,
                        &mut original,
                        baseline.original_run,
                        &original_run,
                        hook,
                        entry_ram,
                        sp,
                    )
                    .is_err()
                    {
                        sensitive = true;
                        evidence.sensitive_probe_pairs += 1;
                    }
                }
                evidence.probe_pairs_matched += 1;
            }
            if grew {
                continue;
            }
            if !sensitive {
                return Err("dependent-equivalence proof found no original output sensitivity; true dependence was not established".into());
            }
            evidence.calls_proven += 1;
            return Ok(baseline_dead.expect("baseline compared"));
        }
    }
}

#[cfg(test)]
mod tests;
