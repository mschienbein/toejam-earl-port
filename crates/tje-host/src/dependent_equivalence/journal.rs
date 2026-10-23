use crate::{
    abi::{AtomicRun, OrderedWrite, UninitializedStackRead, canonical_ram_address},
    catalogue::{STACK_BOTTOM, STACK_TOP},
};
use genesis_core::HybridAccess;
use m68000_emu::ReadPurpose;
use serde::Serialize;
use std::collections::BTreeSet;

/// An observed 68000 bus read. A long is its two ordered word reads, never four
/// invented byte accesses. Raw aliases remain distinct from backing RAM bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DependentRead {
    pub raw_address: u32,
    pub canonical_address: u32,
    pub width: u8,
    pub purpose: &'static str,
    pub value: u16,
}

#[derive(Debug, Clone)]
pub(crate) struct JournalRead {
    pub pc: u32,
    pub read: DependentRead,
    pub writes_before: usize,
}

impl JournalRead {
    pub(crate) fn new(
        pc: u32,
        address: u32,
        word: bool,
        purpose: ReadPurpose,
        value: u16,
        writes_before: usize,
    ) -> Option<Self> {
        let raw_address = address & 0xffffff;
        if raw_address < 0xe00000 {
            return None;
        }
        let canonical_address = canonical_ram_address(raw_address);
        if !(STACK_BOTTOM..STACK_TOP).contains(&canonical_address) {
            return None;
        }
        Some(Self {
            pc,
            read: DependentRead {
                raw_address,
                canonical_address,
                width: if word { 2 } else { 1 },
                purpose: match purpose {
                    ReadPurpose::Semantic => "semantic",
                    ReadPurpose::Discarded => "discarded",
                    ReadPurpose::Prefetch => "prefetch",
                },
                value,
            },
            writes_before,
        })
    }
}

pub(crate) fn from_clock_accesses(accesses: &[(u32, HybridAccess)]) -> Vec<JournalRead> {
    let mut reads = Vec::new();
    let mut writes = 0;
    for &(pc, access) in accesses {
        match access {
            HybridAccess::Read {
                address,
                word,
                value,
                purpose,
            } => {
                if let Some(read) = JournalRead::new(pc, address, word, purpose, value, writes) {
                    reads.push(read);
                }
            }
            // This is exactly audit_accesses/merge_clock_run's byte-RAM and
            // single-device write cardinality, not cross-language bus ordering.
            HybridAccess::Write { address, word, .. } => {
                writes += if address >= 0xe00000 && word { 2 } else { 1 };
            }
        }
    }
    reads
}

pub(crate) fn append(
    target: &mut Vec<JournalRead>,
    reads: Vec<JournalRead>,
    preceding_writes: usize,
) {
    target.extend(reads.into_iter().map(|mut read| {
        read.writes_before += preceding_writes;
        read
    }));
}

pub(crate) struct DerivedReads<'a> {
    reads: &'a [JournalRead],
    pub bytes: BTreeSet<u32>,
    pub sites: BTreeSet<UninitializedStackRead>,
}

impl<'a> DerivedReads<'a> {
    /// Replay one side's own chronological stores. The write indices must never
    /// be compared between languages: MOVEM/predecrement word order can differ
    /// under the readable-layer write-footprint contract.
    pub(crate) fn derive(audit: &'a AtomicRun, entry_sp: u32) -> Self {
        let mut initialized = [0_u64; 512];
        let end = canonical_ram_address(entry_sp).min(STACK_TOP);
        let mut applied = 0;
        let mut result = Self {
            reads: audit.stack_reads.as_deref().unwrap_or_default(),
            bytes: BTreeSet::new(),
            sites: BTreeSet::new(),
        };
        for event in audit.stack_reads.as_deref().unwrap_or_default() {
            for write in &audit.writes[applied..event.writes_before] {
                if let OrderedWrite::Ram { address, .. } = *write
                    && (STACK_BOTTOM..end).contains(&address)
                {
                    let index = (address - STACK_BOTTOM) as usize;
                    initialized[index / 64] |= 1 << (index % 64);
                }
            }
            applied = event.writes_before;
            for offset in 0..event.read.width {
                let address = canonical_ram_address(event.read.raw_address + u32::from(offset));
                if !(STACK_BOTTOM..end).contains(&address) {
                    continue;
                }
                let index = (address - STACK_BOTTOM) as usize;
                if initialized[index / 64] & (1 << (index % 64)) == 0
                    && event.read.purpose == "semantic"
                {
                    result.bytes.insert(address);
                    result.sites.insert(UninitializedStackRead {
                        pc: event.pc,
                        address,
                    });
                }
            }
        }
        result
    }

    /// Keep every purpose when it touches a genuinely semantic-dependent byte.
    /// A discarded/prefetch-only byte is not itself evidence of dependence.
    pub(crate) fn trace(&self, bytes: &BTreeSet<u32>) -> Vec<DependentRead> {
        self.reads
            .iter()
            .filter(|event| {
                (0..event.read.width).any(|offset| {
                    bytes.contains(&canonical_ram_address(
                        event.read.raw_address + u32::from(offset),
                    ))
                })
            })
            .map(|event| event.read.clone())
            .collect()
    }
}
