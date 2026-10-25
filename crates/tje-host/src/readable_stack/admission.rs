//! A cited occurrence is an entry input, not a byte/address waiver.
use crate::{
    Hook,
    abi::{AtomicRun, UninitializedStackRead, canonical_ram_address},
    dependent_equivalence::journal::JournalRead,
};
use serde::Serialize;
use std::collections::BTreeSet;
use tje_game::stack_read::{LocalWidth, UninitializedLocalRead, Unsupported, UnsupportedReason};
use tje_rom::Revision;

#[derive(Debug, Clone)]
pub(crate) struct AdmittedRead {
    pub token: JournalRead,
    pub declaration: UninitializedLocalRead,
}
/// A later actual exception is diagnostic history, not the read's owner.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ExceptionFrameEvidence {
    pub vector: u32,
    pub handler: u32,
    pub depth: u32,
    pub occurrence: u64,
    pub parent_occurrence: Option<u64>,
    pub call_parent_owner: Option<&'static str>,
    pub call_parent_entry_sp: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct InvocationFrameEvidence {
    pub owner: &'static str,
    pub owner_id: &'static str,
    pub entry_pc: u32,
    pub entry_sp: u32,
    pub observed_cpu_pc: u32,
    pub observed_active_sp: u32,
    pub entry_status: u16,
    pub entry_usp: u32,
    pub entry_ssp: u32,
    pub supervisor: bool,
    pub exception_depth: u32,
    pub invocation_depth: u32,
    pub occurrence: u64,
    pub return_pc: Option<u32>,
    pub return_site: Option<u32>,
    pub caller_pc: Option<u32>,
    pub call_opcode: Option<u16>,
    pub parent_owner: Option<&'static str>,
    pub parent_entry_sp: Option<u32>,
    pub parent_occurrence: Option<u64>,
    pub first_later_exception: Option<ExceptionFrameEvidence>,
}

fn frame_evidence(
    scope: &super::OriginalRootScope,
    witness: super::OriginalRootWitness,
) -> Option<InvocationFrameEvidence> {
    let frame = scope.record(witness)?;
    let parent = scope.parent(witness);
    let point = frame.entry_point();
    let owner = frame.owner()?;
    Some(InvocationFrameEvidence {
        owner: owner.metadata().name,
        owner_id: owner.metadata().inventory_id,
        entry_pc: frame.entry_pc(),
        entry_sp: frame.entry_sp()?,
        observed_cpu_pc: point.pc,
        observed_active_sp: point.sp,
        entry_status: point.status,
        entry_usp: point.usp,
        entry_ssp: point.ssp,
        supervisor: frame.supervisor(),
        exception_depth: frame.exception_depth(),
        invocation_depth: frame.invocation_depth(),
        occurrence: frame.occurrence(),
        return_pc: frame.return_pc(),
        return_site: frame.return_site(),
        caller_pc: frame.caller_pc(),
        call_opcode: frame.opcode(),
        parent_owner: parent
            .as_ref()
            .and_then(|parent| parent.owner())
            .map(|owner| owner.metadata().name),
        parent_entry_sp: parent.as_ref().and_then(|parent| parent.entry_sp()),
        parent_occurrence: parent.as_ref().map(|parent| parent.occurrence()),
        first_later_exception: scope.first_later_exception(),
    })
}

fn matching_witnesses(
    audit: &AtomicRun,
    witness: Option<super::OriginalRootWitness>,
    other: &AtomicRun,
    other_witness: Option<super::OriginalRootWitness>,
) -> bool {
    match (witness, other_witness) {
        (None, None) => true,
        (Some(witness), Some(other_witness)) => audit
            .invocation_scope
            .as_deref()
            .zip(other.invocation_scope.as_deref())
            .is_some_and(|(a, b)| a.matches(witness, b, other_witness)),
        _ => false,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DeclaredStackInput {
    pub root: &'static str,
    pub frame_entry_sp: u32,
    pub offset: i32,
    pub logical_width: u8,
    pub beat: u8,
    pub physical_width: u8,
    pub original_site: u32,
    pub original_ordinal: usize,
    pub rust_ordinal: usize,
    pub raw_address: u32,
    pub address: u32,
    pub unwritten_mask: u8,
    pub value: u16,
    pub citation: &'static str,
    pub owner: &'static str,
    pub original_frame: InvocationFrameEvidence,
    pub rust_frame: InvocationFrameEvidence,
}

fn fail(
    hook: &Hook,
    entry_sp: u32,
    read: UninitializedLocalRead,
    reason: UnsupportedReason,
) -> Unsupported {
    Unsupported::paired(hook.symbol.id, entry_sp, read, reason)
}
fn input_bytes(admitted: &[AdmittedRead]) -> BTreeSet<u32> {
    admitted
        .iter()
        .flat_map(|read| {
            (0..read.token.read.width)
                .filter(|lane| read.token.uninitialized_mask & (1 << lane) != 0)
                .map(|lane| {
                    canonical_ram_address(read.token.read.raw_address.wrapping_add(u32::from(lane)))
                })
        })
        .collect()
}
fn reject_uncited(
    hook: &Hook,
    entry_sp: u32,
    audit: &AtomicRun,
    admitted: &[AdmittedRead],
) -> Result<(), Unsupported> {
    let Some(first) = admitted.first() else {
        return Ok(());
    };
    let bytes = input_bytes(admitted);
    let ordinals: BTreeSet<_> = admitted.iter().map(|read| read.token.ordinal).collect();
    for read in &audit.unwritten_stack_reads {
        if ordinals.contains(&read.ordinal) {
            continue;
        }
        if (0..read.read.width).any(|lane| {
            read.uninitialized_mask & (1 << lane) != 0
                && bytes.contains(&canonical_ram_address(
                    read.read.raw_address.wrapping_add(u32::from(lane)),
                ))
        }) {
            return Err(fail(
                hook,
                entry_sp,
                first.declaration,
                UnsupportedReason::UncitedInputDependence,
            ));
        }
    }
    Ok(())
}
fn remove_admitted_candidates(audit: &mut AtomicRun) {
    let ordinals: BTreeSet<_> = audit
        .admitted_stack_inputs
        .iter()
        .map(|read| read.token.ordinal)
        .collect();
    audit.uninitialized_reads.clear();
    for read in &audit.unwritten_stack_reads {
        if ordinals.contains(&read.ordinal) {
            continue;
        }
        for lane in 0..read.read.width {
            if read.uninitialized_mask & (1 << lane) != 0 {
                audit.uninitialized_reads.insert(UninitializedStackRead {
                    pc: read.pc,
                    address: canonical_ram_address(
                        read.read.raw_address.wrapping_add(u32::from(lane)),
                    ),
                });
            }
        }
    }
}

pub(crate) fn pair(
    hook: &Hook,
    revision: Revision,
    entry_sp: u32,
    original: &mut AtomicRun,
    rust: &mut AtomicRun,
) -> Result<Vec<DeclaredStackInput>, Unsupported> {
    if rust.explicit_stack_reads.is_empty() {
        return Ok(Vec::new());
    }
    let original_reads = original.stack_reads.as_deref().unwrap_or_default();
    let rust_reads = rust.stack_reads.as_deref().unwrap_or_default();
    let mut original_admitted = Vec::new();
    let mut rust_admitted = Vec::new();
    let mut evidence = Vec::new();
    let mut cursor = 0;
    for occurrence in &rust.explicit_stack_reads {
        let read = occurrence.declaration;
        let Some((rust_scope, rust_witness)) =
            rust.invocation_scope.as_deref().zip(occurrence.invocation)
        else {
            return Err(fail(
                hook,
                entry_sp,
                read,
                UnsupportedReason::MissingOriginalFrameWitness,
            ));
        };
        let Some(rust_frame) = frame_evidence(rust_scope, rust_witness).filter(|frame| {
            frame.owner_id == read.owner.metadata().inventory_id
                && frame.entry_sp == read.frame.entry_sp
        }) else {
            return Err(fail(
                hook,
                entry_sp,
                read,
                UnsupportedReason::MissingOriginalFrameWitness,
            ));
        };
        if read.citation.trim().is_empty() || occurrence.mask == 0 {
            return Err(fail(
                hook,
                entry_sp,
                read,
                UnsupportedReason::PairedInputMismatch,
            ));
        }
        let beats = if read.width == LocalWidth::Long { 2 } else { 1 };
        if occurrence.beat_count != beats {
            return Err(fail(
                hook,
                entry_sp,
                read,
                UnsupportedReason::PairedInputMismatch,
            ));
        }
        for beat in 0..beats {
            let raw = read.address().wrapping_add(2 * beat as u32) & tje_mem::ADDRESS_MASK;
            let width = if read.width == LocalWidth::Byte { 1 } else { 2 };
            let mask = (occurrence.mask >> (2 * beat)) & if width == 1 { 1 } else { 3 };
            let Some(rust_token) = rust_reads.get(occurrence.first_read + beat) else {
                return Err(fail(
                    hook,
                    entry_sp,
                    read,
                    UnsupportedReason::PairedInputMismatch,
                ));
            };
            if rust_token.ordinal != occurrence.ordinals[beat]
                || rust_token.read.raw_address != raw
                || rust_token.read.width != width
                || rust_token.read.origin != "cpu"
                || rust_token.read.purpose != "semantic"
                || rust_token.read.value != occurrence.beats[beat]
                || rust_token.uninitialized_mask != mask
            {
                return Err(fail(
                    hook,
                    entry_sp,
                    read,
                    UnsupportedReason::PairedInputMismatch,
                ));
            }
            let found = original_reads
                .iter()
                .enumerate()
                .skip(cursor)
                .find(|(_, token)| {
                    token.read.raw_address == raw
                        && token.read.width == width
                        && token.read.origin == "cpu"
                        && token.read.purpose == "semantic"
                });
            let Some((index, original_token)) = found else {
                return Err(fail(
                    hook,
                    entry_sp,
                    read,
                    UnsupportedReason::PairedInputMismatch,
                ));
            };
            let Some((original_scope, original_witness)) = original
                .invocation_scope
                .as_deref()
                .zip(original_token.original_root)
            else {
                return Err(fail(
                    hook,
                    entry_sp,
                    read,
                    UnsupportedReason::MissingOriginalFrameWitness,
                ));
            };
            if !original_scope.matches(original_witness, rust_scope, rust_witness) {
                return Err(fail(
                    hook,
                    entry_sp,
                    read,
                    UnsupportedReason::MissingOriginalFrameWitness,
                ));
            }
            let Some(original_frame) = frame_evidence(original_scope, original_witness) else {
                return Err(fail(
                    hook,
                    entry_sp,
                    read,
                    UnsupportedReason::MissingOriginalFrameWitness,
                ));
            };
            let start = original_frame.entry_pc;
            let size = if read.owner == hook.symbol.id && start == hook.symbol.address(revision) {
                hook.symbol.size(revision)
            } else {
                read.owner.metadata().size(revision)
            };
            let end = start.saturating_add(size);
            if !(start..end).contains(&original_token.pc)
                || original_token.uninitialized_mask != mask
                || original_token.read.value != rust_token.read.value
            {
                return Err(fail(
                    hook,
                    entry_sp,
                    read,
                    UnsupportedReason::PairedInputMismatch,
                ));
            }
            cursor = index + 1;
            original_admitted.push(AdmittedRead {
                token: original_token.clone(),
                declaration: read,
            });
            rust_admitted.push(AdmittedRead {
                token: rust_token.clone(),
                declaration: read,
            });
            evidence.push(DeclaredStackInput {
                root: hook.symbol.name,
                frame_entry_sp: read.frame.entry_sp,
                offset: read.offset,
                logical_width: read.width.bytes(),
                beat: beat as u8,
                physical_width: width,
                original_site: original_token.pc,
                original_ordinal: original_token.ordinal,
                rust_ordinal: rust_token.ordinal,
                raw_address: raw,
                address: canonical_ram_address(raw),
                unwritten_mask: mask,
                value: original_token.read.value,
                citation: read.citation,
                owner: read.owner.metadata().name,
                original_frame,
                rust_frame,
            });
        }
    }
    reject_uncited(hook, entry_sp, original, &original_admitted)?;
    reject_uncited(hook, entry_sp, rust, &rust_admitted)?;
    // A reviewed dependent proof still owns its complete residual domain and
    // coupled sensitivity obligation. Validating an explicit read cannot turn
    // that stronger proof into ordinary entry-input equivalence.
    if hook.dependent_proof.is_none() {
        original.admitted_stack_inputs = original_admitted;
        rust.admitted_stack_inputs = rust_admitted;
        remove_admitted_candidates(original);
        remove_admitted_candidates(rust);
    }
    Ok(evidence)
}

/// Probe all OTHER dependencies with declared entry-input bytes held fixed.
/// Every admitted token/site/value/mask/occurrence remains exact on the replay.
pub(crate) fn revalidate(
    hook: &Hook,
    entry_sp: u32,
    baseline: &AtomicRun,
    probe: &mut AtomicRun,
) -> Result<(), Unsupported> {
    if baseline.admitted_stack_inputs.is_empty() {
        return Ok(());
    }
    let reads = probe.stack_reads.as_deref().unwrap_or_default();
    let mut admitted = Vec::with_capacity(baseline.admitted_stack_inputs.len());
    for expected in &baseline.admitted_stack_inputs {
        let actual = reads
            .iter()
            .find(|read| read.ordinal == expected.token.ordinal);
        if !actual.is_some_and(|actual| {
            actual.pc == expected.token.pc
                && actual.read == expected.token.read
                && actual.uninitialized_mask == expected.token.uninitialized_mask
                && matching_witnesses(
                    baseline,
                    expected.token.original_root,
                    probe,
                    actual.original_root,
                )
        }) {
            return Err(fail(
                hook,
                entry_sp,
                expected.declaration,
                UnsupportedReason::PairedInputMismatch,
            ));
        }
        admitted.push(AdmittedRead {
            token: actual.expect("validated replay occurrence").clone(),
            declaration: expected.declaration,
        });
    }
    if !baseline.explicit_stack_reads.is_empty()
        && (baseline.explicit_stack_reads.len() != probe.explicit_stack_reads.len()
            || baseline
                .explicit_stack_reads
                .iter()
                .zip(&probe.explicit_stack_reads)
                .any(|(a, b)| {
                    a.declaration != b.declaration
                        || a.mask != b.mask
                        || a.beats != b.beats
                        || a.ordinals != b.ordinals
                        || a.beat_count != b.beat_count
                        || !matching_witnesses(baseline, a.invocation, probe, b.invocation)
                }))
    {
        let first = &baseline.admitted_stack_inputs[0];
        return Err(fail(
            hook,
            entry_sp,
            first.declaration,
            UnsupportedReason::PairedInputMismatch,
        ));
    }
    reject_uncited(hook, entry_sp, probe, &baseline.admitted_stack_inputs)?;
    probe.admitted_stack_inputs = admitted;
    remove_admitted_candidates(probe);
    Ok(())
}
