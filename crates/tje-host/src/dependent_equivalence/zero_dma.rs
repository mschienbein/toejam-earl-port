//! Closed source plans for the original zero-encoded memory-to-VRAM transfer.
//! This is not an arbitrary RAM-span exception or a CPU residual-read contract.

use super::journal::JournalRead;
use crate::{Host, HostError, Result, abi::AtomicRun, hooks::Hook};
use genesis_core::{GenesisEmulator, HybridAccess};
use serde::Serialize;
use std::collections::BTreeSet;
use tje_game::{execution::ExecutionClass, ids};
use tje_mem::ram;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZeroDmaRoot {
    Platform,
    QueuedSpriteGraphics,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ZeroDmaPlan {
    source: u32,
    transfers: u16,
}

/// Bounded matched source-plan evidence, never a RAM/ROM value dump.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DmaSourceSummary {
    pub words_per_execution: u64,
    pub transfers_per_execution: u64,
    pub raw_first_address: u32,
    pub raw_last_address: u32,
    pub raw_bank_bytes: u32,
    pub canonical_work_ram_bytes: u32,
}

impl ZeroDmaRoot {
    pub(crate) fn resolve(self, host: &mut Host, hook: &Hook) -> Result<ZeroDmaPlan> {
        let expected_id = match self {
            Self::Platform => ids::DMA_COPY_WORDS_TO_VRAM,
            Self::QueuedSpriteGraphics => ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
        };
        if hook.symbol.id != expected_id
            || hook.execution_class != ExecutionClass::Functional
            || hook.check_route() != crate::CheckForkRoute::FrozenFunctional
        {
            return Err(HostError::Metadata(
                "zero-count DMA proof requires its reviewed root under frozen Functional execution"
                    .into(),
            ));
        }
        let revision = host.revision();
        let context = host.context();
        let (source, transfers) = match self {
            Self::Platform => {
                if context.argument_u32(2) & 0xffff != 0 {
                    return Err(HostError::Metadata(
                        "zero-count DMA source contract rejects nonzero encoded counts".into(),
                    ));
                }
                (context.argument_u32(0) & 0xfffffe, 1)
            }
            Self::QueuedSpriteGraphics => {
                // c_load_sprites.s:5-6,16-17: signed low-word tiles_only; no
                // nonzero 320-word sprite-table prefix is admitted here.
                if context.argument_i16(1) == 0 {
                    return Err(HostError::Metadata("zero-count upload source contract requires tiles_only and excludes the sprite-table DMA".into()));
                }
                // c_load_sprites.s:58-63: BYTE queue bound, word index. Each
                // run is shifted as a long and then narrowed to its word count.
                let count = ram::g_sprite_tile_upload_queue_word_count(revision).read(&context);
                let runs = ram::g_sprite_tile_upload_runs(revision);
                let mut index = 0_i16;
                let mut transfers = 0_u16;
                while i32::from(index) < i32::from(count) {
                    if runs.at(i32::from(index) + 1).read(&context).wrapping_shl(4) != 0 {
                        return Err(HostError::Metadata("zero-count upload source contract rejects nonzero or mixed encoded runs".into()));
                    }
                    transfers += 1;
                    index = index.wrapping_add(2);
                }
                if transfers == 0 {
                    return Err(HostError::Metadata("zero-count upload source contract requires an actual zero-encoded transfer".into()));
                }
                // source += encoded_count * 2, so every admitted run retains the
                // same original upload-buffer pointer, including 4096 << 4.
                (
                    ram::g_sprite_tile_upload_buffer(revision).address(),
                    transfers,
                )
            }
        };
        if source < 0xe00000 {
            return Err(HostError::Metadata(
                "zero-count DMA source contract admits only the original whole-work-RAM transfer"
                    .into(),
            ));
        }
        Ok(ZeroDmaPlan { source, transfers })
    }
}

impl ZeroDmaPlan {
    pub(crate) fn transfers(self) -> u64 {
        u64::from(self.transfers)
    }
    pub(crate) fn words(self) -> u64 {
        self.transfers() * 65536
    }

    fn address(self, ordinal: u64) -> u32 {
        // jgenesis VDP increment_dma_source_address: wraps inside the original
        // 128-KiB raw bank, retaining FE/FF aliases. Canonical 64-KiB RAM is read
        // twice, not once. No allocated expected-address table is needed.
        (self.source & !0x1ffff)
            | (self.source.wrapping_add(((ordinal & 0xffff) as u32) * 2) & 0x1ffff)
    }

    pub(crate) fn summary(self) -> DmaSourceSummary {
        DmaSourceSummary {
            words_per_execution: self.words(),
            transfers_per_execution: self.transfers(),
            raw_first_address: self.source,
            raw_last_address: self.address(self.words() - 1),
            raw_bank_bytes: 0x20000,
            canonical_work_ram_bytes: 0x10000,
        }
    }

    pub(crate) fn reject_feedback(
        self,
        machine: &GenesisEmulator,
        audit: &AtomicRun,
    ) -> std::result::Result<(), String> {
        for event in audit.stack_reads.as_deref().unwrap_or_default() {
            match event.read.origin {
                "vdp_dma" => {}
                "cpu" if event.uninitialized_mask != 0 && event.read.purpose == "semantic" => {
                    return Err(format!(
                        "DMA-only source contract rejects genuine CPU residual read at {:06X}",
                        event.read.raw_address
                    ));
                }
                "cpu" => {}
                _ => {
                    return Err(
                        "DMA-only source contract observed an unreviewed read origin".into(),
                    );
                }
            }
        }
        if machine.hybrid_bus().hybrid_accesses.as_deref().unwrap_or_default().iter().any(|access| {
            matches!(*access, HybridAccess::Read { address, .. } if (0x400000..0xe00000).contains(&(address & 0xffffff)))
        }) {
            return Err("DMA-only source contract rejects CPU/device feedback reads".into());
        }
        Ok(())
    }

    /// Compare the entire actual DMA stream, including non-RAM/extra beats, and
    /// every CPU access touching the genuine dependent union in that same order.
    /// `uninitialized_mask` discovers candidates; it never drops later accesses.
    pub(crate) fn compare(
        self,
        original: &AtomicRun,
        translated: &AtomicRun,
        bytes: &BTreeSet<u32>,
    ) -> std::result::Result<(), String> {
        let relevant = |event: &&JournalRead| {
            event.read.origin == "vdp_dma"
                || (0..event.read.width).any(|offset| {
                    bytes.contains(&crate::abi::canonical_ram_address(
                        event.read.raw_address + u32::from(offset),
                    ))
                })
        };
        let mut a = original
            .stack_reads
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(relevant);
        let mut b = translated
            .stack_reads
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(relevant);
        let mut dma_words = 0;
        let mut accesses = 0;
        loop {
            match (a.next(), b.next()) {
                (None, None) => break,
                (Some(a), Some(b)) if a.read == b.read => {
                    if a.read.origin == "vdp_dma" {
                        if dma_words == self.words()
                            || a.read.width != 2
                            || a.read.purpose != "semantic"
                            || a.read.raw_address != self.address(dma_words)
                            || a.read.canonical_address
                                != crate::abi::canonical_ram_address(self.address(dma_words))
                        {
                            return Err(format!(
                                "zero-count DMA source sequence differs at word {dma_words}: {:?}",
                                a.read
                            ));
                        }
                        dma_words += 1;
                    }
                }
                (a, b) => {
                    return Err(format!(
                        "DMA/CPU source read trace differs at access {accesses}: original={:?}, Rust={:?}",
                        a.map(|event| &event.read),
                        b.map(|event| &event.read)
                    ));
                }
            }
            accesses += 1;
        }
        if dma_words != self.words() {
            return Err(format!(
                "zero-count DMA source sequence has {dma_words} words, expected {}",
                self.words()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod observer_tests;
