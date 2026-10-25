use crate::{
    abi::{AtomicRun, UninitializedStackRead, canonical_ram_address},
    catalogue::{STACK_BOTTOM, STACK_TOP},
};
use genesis_core::HybridAccess;
use m68000_emu::ReadPurpose;
use serde::Serialize;
use std::collections::BTreeSet;

/// An actual CPU or VDP source read. A long is two ordered words, never four
/// invented byte accesses. Raw aliases and source-device identity are retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DependentRead {
    pub raw_address: u32,
    pub canonical_address: u32,
    pub width: u8,
    pub origin: &'static str,
    pub purpose: &'static str,
    pub value: u16,
}

#[derive(Debug, Clone)]
pub(crate) struct JournalRead {
    /// For DMA this is the surrounding execution checkpoint, not an invented
    /// 68000 read instruction.
    pub pc: u32,
    pub ordinal: usize,
    pub read: DependentRead,
    /// Classified by the sole root bitmap at the actual read, after preceding
    /// RAM commits. Never reconstructed from instruction-issue write indices.
    pub uninitialized_mask: u8,
    pub original_root: Option<crate::readable_stack::OriginalRootWitness>,
}

impl JournalRead {
    pub(crate) fn new(pc: u32, access: HybridAccess, uninitialized_mask: u8) -> Option<Self> {
        let read = Self::new_work_ram(pc, access, uninitialized_mask)?;
        if read.read.origin == "cpu"
            && !(STACK_BOTTOM..STACK_TOP).contains(&read.read.canonical_address)
        {
            return None;
        }
        Some(read)
    }
    pub(crate) fn new_work_ram(
        pc: u32,
        access: HybridAccess,
        uninitialized_mask: u8,
    ) -> Option<Self> {
        let (address, word, purpose, value, origin) = match access {
            HybridAccess::Read {
                address,
                word,
                purpose,
                value,
            } => (address, word, purpose, value, "cpu"),
            HybridAccess::DmaRead { address, value } => {
                (address, true, ReadPurpose::Semantic, value, "vdp_dma")
            }
            HybridAccess::Write { .. }
            | HybridAccess::Z80Write { .. }
            | HybridAccess::HostMutation { .. }
            | HybridAccess::ExceptionBoundary { .. } => return None,
        };
        let raw_address = address & 0xffffff;
        if origin == "cpu" && raw_address < 0xe00000 {
            return None;
        }
        let canonical_address = canonical_ram_address(raw_address);
        Some(Self {
            ordinal: 0,
            pc,
            read: DependentRead {
                raw_address,
                canonical_address,
                width: if word { 2 } else { 1 },
                origin,
                purpose: match purpose {
                    ReadPurpose::Semantic => "semantic",
                    ReadPurpose::Discarded => "discarded",
                    ReadPurpose::Prefetch => "prefetch",
                },
                value,
            },
            uninitialized_mask,
            original_root: None,
        })
    }
}

pub(crate) struct DerivedReads<'a> {
    reads: &'a [JournalRead],
    pub bytes: BTreeSet<u32>,
    pub sites: BTreeSet<UninitializedStackRead>,
}

impl<'a> DerivedReads<'a> {
    /// The read's mask was classified in actual commit/read chronology, shared
    /// across the root, nested originals, waits, and endpoint normalization.
    pub(crate) fn derive_with_sites(
        audit: &'a AtomicRun,
        entry_sp: u32,
        retain_sites: bool,
    ) -> Self {
        let end = canonical_ram_address(entry_sp).min(STACK_TOP);
        let mut result = Self {
            reads: audit.stack_reads.as_deref().unwrap_or_default(),
            bytes: BTreeSet::new(),
            sites: BTreeSet::new(),
        };
        for event in audit.stack_reads.as_deref().unwrap_or_default() {
            if audit
                .admitted_stack_inputs
                .iter()
                .any(|read| read.token.ordinal == event.ordinal)
            {
                continue;
            }
            for offset in 0..event.read.width {
                let address = canonical_ram_address(event.read.raw_address + u32::from(offset));
                if !(STACK_BOTTOM..end).contains(&address) {
                    continue;
                }
                if event.uninitialized_mask & (1 << offset) != 0 && event.read.purpose == "semantic"
                {
                    result.bytes.insert(address);
                    if retain_sites {
                        result.sites.insert(UninitializedStackRead {
                            pc: event.pc,
                            address,
                        });
                    }
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
