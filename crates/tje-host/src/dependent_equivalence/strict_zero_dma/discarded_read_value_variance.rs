//! DiscardedReadValueVariance applies only to original baseline/probe controls.
//! The complete core-issued journal authenticates every duplicate read by ordinal.
//! Neither the paired Rust comparison nor a caller-authored purpose grants it.

use super::{ControlEvent, ControlLog, ProbeProgress, control_difference};
use crate::abi::canonical_ram_address;
use genesis_core::{HybridAccess, native::CpuAccess};
use m68000_emu::ReadPurpose;
use serde::Serialize;

pub(super) const CONTRACT: &str = "DiscardedReadValueVariance";

#[derive(Debug, Serialize)]
pub struct DiscardedReadValueVariance {
    pub seed: u64,
    pub proof_call_ordinal: u64,
    pub seeded_probe_index: Option<u16>,
    pub collective_fill: Option<u8>,
    pub raw_cpu_ordinal: usize,
    pub raw_address: u32,
    pub canonical_ram_address: Option<u32>,
    pub width_bytes: u8,
    pub purpose: &'static str,
    pub baseline_value: u16,
    pub probe_value: u16,
}

pub(super) struct VerifiedJournal<'a> {
    baseline: &'a [CpuAccess],
    probe: &'a [CpuAccess],
}

pub(super) fn verify<'a>(
    baseline: Option<&'a [CpuAccess]>,
    probe: Option<&'a [CpuAccess]>,
    identity: super::super::Probe,
    progress: &mut ProbeProgress,
) -> Result<VerifiedJournal<'a>, String> {
    let proof_call = progress
        .identity
        .ok_or("DiscardedReadValueVariance requires an allocated proof-call identity")?;
    let (seeded_probe_index, collective_fill) = match identity {
        super::super::Probe::Seeded(index) => (Some(index), None),
        super::super::Probe::DmaFill(value) => (None, Some(value)),
        super::super::Probe::Byte { .. } => {
            return Err("DiscardedReadValueVariance is not a Frame byte-probe contract".into());
        }
    };
    let (Some(baseline), Some(probe)) = (baseline, probe) else {
        return Err("DiscardedReadValueVariance requires both actual CPU journals".into());
    };
    if baseline.len() != probe.len() {
        return Err(format!(
            "DiscardedReadValueVariance CPU journal counts differ: baseline={}, probe={}",
            baseline.len(),
            probe.len()
        ));
    }
    for (ordinal, (&left, &right)) in baseline.iter().zip(probe).enumerate() {
        if left == right {
            continue;
        }
        match (left, right) {
            (
                CpuAccess::Read {
                    address,
                    word,
                    value: baseline_value,
                    purpose: 2,
                },
                CpuAccess::Read {
                    address: right_address,
                    word: right_word,
                    value: probe_value,
                    purpose: 2,
                },
            ) if address == right_address && word == right_word => {
                progress
                    .discarded_read_value_variances
                    .push(DiscardedReadValueVariance {
                        seed: proof_call.seed,
                        proof_call_ordinal: proof_call.ordinal,
                        seeded_probe_index,
                        collective_fill,
                        raw_cpu_ordinal: ordinal,
                        raw_address: address,
                        canonical_ram_address: (0xe00000..=0xffffff)
                            .contains(&address)
                            .then(|| canonical_ram_address(address)),
                        width_bytes: if word { 2 } else { 1 },
                        purpose: "Discarded",
                        baseline_value,
                        probe_value,
                    });
            }
            _ => {
                return Err(format!(
                    "DiscardedReadValueVariance preserves semantic/prefetch reads, writes, acknowledgements and every access identity: raw CPU ordinal {ordinal}, baseline={left:?}, probe={right:?}"
                ));
            }
        }
    }
    progress.discarded_read_journal_checks += 1;
    Ok(VerifiedJournal { baseline, probe })
}

fn matches_core(access: HybridAccess, core: CpuAccess) -> bool {
    match (access, core) {
        (
            HybridAccess::Read {
                address,
                word,
                value,
                purpose,
            },
            CpuAccess::Read {
                address: core_address,
                word: core_word,
                value: core_value,
                purpose: core_purpose,
            },
        ) => {
            let purpose = match purpose {
                ReadPurpose::Semantic => 0,
                ReadPurpose::Prefetch => 1,
                ReadPurpose::Discarded => 2,
            };
            address == core_address
                && word == core_word
                && value == core_value
                && purpose == core_purpose
        }
        _ => false,
    }
}

impl VerifiedJournal<'_> {
    pub(super) fn compare_controls(
        &self,
        left: &ControlLog,
        right: &ControlLog,
    ) -> Result<(), String> {
        let mut core_reads = self.baseline.iter().zip(self.probe).enumerate().filter(
            |(_, (access, _))| matches!(access, CpuAccess::Read { address, .. } if *address >= 0x800000),
        );
        for (index, (a, b)) in left.events.iter().zip(&right.events).enumerate() {
            match (a, b) {
                (
                    ControlEvent::Access(a @ HybridAccess::Read { .. }),
                    ControlEvent::Access(b @ HybridAccess::Read { .. }),
                ) => {
                    let Some((ordinal, (&core_a, &core_b))) = core_reads.next() else {
                        return Err(format!(
                            "DiscardedReadValueVariance control read at event {index} has no actual core ordinal: baseline={a:?}, probe={b:?}"
                        ));
                    };
                    if !matches_core(*a, core_a) || !matches_core(*b, core_b) {
                        return Err(format!(
                            "DiscardedReadValueVariance duplicate read does not bind to actual core ordinal {ordinal} at control event {index}: baseline={a:?}/{core_a:?}, probe={b:?}/{core_b:?}"
                        ));
                    }
                    // Every value change was already authenticated at THIS raw
                    // ordinal by verify(); no address search or purpose inference.
                }
                _ if a == b => {}
                _ => return Err(control_difference(left, right, index)),
            }
        }
        if left.events.len() != right.events.len() {
            return Err(control_difference(
                left,
                right,
                left.events.len().min(right.events.len()),
            ));
        }
        if let Some((ordinal, _)) = core_reads.next() {
            return Err(format!(
                "DiscardedReadValueVariance actual core read ordinal {ordinal} is missing from the chronological control stream"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
