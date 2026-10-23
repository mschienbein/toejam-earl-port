//! Explicit, reviewed equivalence for genuinely dependent residual frame bytes.
//!
//! Unmarked hooks still require same-side independence. A marked hook instead
//! reproduces the actual residual reads and tracks original outputs under coupled
//! perturbations; neither a LocalFrame nor an address list alone is approval.

pub(crate) mod journal;
pub use journal::DependentRead;

use crate::{
    Host, HostError, Result, StackReadSite,
    abi::{OrderedWrite, canonical_ram_address},
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
    /// Additional coupled seeded assignments, besides every byte's four boundary
    /// probes. Values below 16 are rejected before any entry executes.
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
pub struct DependentContract {
    pub function: FunctionId,
    pub frame: FrameLayout,
    pub ranges: &'static [FrameByteRange],
    pub producer: CompiledProducer,
    pub source: &'static str,
}
impl DependentContract {
    fn permits(self, entry_sp: u32, address: u32) -> bool {
        let base = canonical_ram_address(entry_sp);
        self.ranges.iter().any(|range| {
            let start = i64::from(base) + i64::from(range.entry_sp_offset);
            let end = start + i64::from(range.bytes);
            range.bytes != 0
                && start >= i64::from(STACK_BOTTOM)
                && end <= i64::from(base.min(STACK_TOP))
                && (start..end).contains(&i64::from(address))
        })
    }
}

/// Source-reviewed eligibility only. Explicit per-registry activation and an
/// actual successful sensitivity proof are both still required for every entry.
pub static REVIEWED_CONTRACTS: &[DependentContract] = &[DependentContract {
    function: ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
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
    source: "G src/earthlings/targeting.c:18-122; c_targeting.s LINK #-12 and selected delta words -8/-6/-4/-2(A6)",
}];

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProofSetting {
    pub contract: &'static DependentContract,
    pub config: DependentProofConfig,
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
    pub probe_pairs_matched: u64,
    pub sensitive_probe_pairs: u64,
    pub dependent_bytes: BTreeSet<u32>,
    pub last_read_trace: Vec<DependentRead>,
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
}
fn perturb(emulator: &mut GenesisEmulator, addresses: &[u32], probe: Probe, seed: u64) {
    let memory = &mut emulator.hybrid_bus_mut().memory;
    match probe {
        Probe::Byte { address, value } => memory.write_main_ram_byte(address, value),
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
    pub(super) fn check_dependent_equivalence(
        &mut self,
        original: &mut GenesisEmulator,
        translated: &mut GenesisEmulator,
        original_run: &CheckRun,
        translated_run: &CheckRun,
        hook: &Hook,
    ) -> std::result::Result<u64, String> {
        let setting = hook.dependent_proof.expect("marked hook");
        let mut evidence = DependentEvidence {
            seed: setting.config.seed,
            ..DependentEvidence::default()
        };
        let mut sites = BTreeSet::new();
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
        self.checks
            .dependent_equivalence
            .entry(hook.symbol.name.to_owned())
            .or_default()
            .merge(evidence);
        result
    }

    fn prove_dependent(
        &mut self,
        baseline: &mut Baselines<'_>,
        hook: &Hook,
        evidence: &mut DependentEvidence,
        sites: &mut BTreeSet<crate::abi::UninitializedStackRead>,
    ) -> std::result::Result<u64, String> {
        let setting = hook.dependent_proof.expect("marked hook");
        setting
            .config
            .validate()
            .map_err(|error| error.to_string())?;
        let entry = exits::EntryState::capture(&self.context(), hook.exit_contract)
            .map_err(|error| error.to_string())?;
        let sp = entry.sp;
        let frame_sp = hook.dependent_frame_sp(sp);
        let original_reads = DerivedReads::derive(&baseline.original_run.audit, sp);
        let translated_reads = DerivedReads::derive(&baseline.translated_run.audit, sp);
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
            evidence.dependent_bytes.extend(&bytes);
            for &address in &bytes {
                if !setting.contract.permits(frame_sp, address) {
                    return Err(format!(
                        "dependent residual byte {address:06X} is outside the reviewed source frame ranges"
                    ));
                }
            }
            evidence.last_read_trace = compare_reads(&original_reads, &translated_reads, &bytes)?;
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
            for index in 0..seeded + 4 * addresses.len() {
                let probe = if index < seeded {
                    Probe::Seeded(index as u16)
                } else {
                    let byte_index = index - seeded;
                    Probe::Byte {
                        address: addresses[byte_index / 4],
                        value: [0x00, 0xff, 0x7f, 0x80][byte_index % 4],
                    }
                };
                let mut rust = self.fork();
                perturb(&mut rust, &addresses, probe, setting.config.seed);
                self.checks.stack_dependency_reruns += 1;
                let rust_run = self
                    .run_check_side(&mut rust, hook, CheckSide::Rust)
                    .map_err(|error| format!("dependent Rust probe did not complete: {error}"))?;
                evidence.rust_runs_completed += 1;
                let boundary =
                    exits::reference_boundary(hook, rust_run.outcome, entry, self.revision)
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
                    )
                    .map_err(|error| {
                        format!("dependent original probe did not complete: {error}")
                    })?;
                evidence.original_runs_completed += 1;
                match probe {
                    Probe::Seeded(_) => evidence.seeded_probes_completed += 1,
                    Probe::Byte { .. } => evidence.byte_probes_completed += 1,
                }
                let a = DerivedReads::derive(&original_run.audit, sp);
                let b = DerivedReads::derive(&rust_run.audit, sp);
                sites.extend(a.sites.iter().chain(&b.sites).copied());
                let previous = bytes.len();
                bytes.extend(a.bytes.iter().chain(&b.bytes));
                if bytes.len() != previous {
                    grew = true;
                    break;
                }
                compare_reads(&a, &b, &bytes)?;
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
                evidence.probe_pairs_matched += 1;
                // Use actual original D0/registers/SR/RAM/writes. Original
                // CheckRun.outcome is the Rust-declared boundary, not a result.
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
