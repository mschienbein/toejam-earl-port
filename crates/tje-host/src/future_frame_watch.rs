//! Rejection-only provenance for private interrupt-frame bytes.
//!
//! A commutation result stays pending unless both sides' actual current values
//! are attested equal before any read. The host observes only a persistent
//! Original stream; it has no Rust-current authority, so its debts cannot be
//! approved. This model never reads or changes the machine. Actual CPU/DMA
//! observations and speculative branches remain separate.
use crate::abi::canonical_ram_address;
use genesis_core::HybridAccess;
use m68000_emu::ReadPurpose;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct ApprovalId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct ReceiptId(u64);

impl ApprovalId {
    pub fn value(self) -> u64 {
        self.0
    }
}

impl ReceiptId {
    pub fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Original,
    Rust,
}

impl Side {
    fn index(self) -> usize {
        match self {
            Self::Original => 0,
            Self::Rust => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueOrigin {
    EntrySnapshot,
    CpuStore,
    Z80Store,
    NativeStore,
    ExternalRecipe,
    ExternalSetup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CurrentByteValues {
    pub original: u8,
    pub translated: u8,
    pub original_current_attested: bool,
    pub translated_current_attested: bool,
    pub original_origin: ValueOrigin,
    pub translated_origin: ValueOrigin,
}

impl CurrentByteValues {
    fn write(&mut self, side: Side, value: u8, actual_current: bool, origin: ValueOrigin) {
        match side {
            Side::Original => {
                self.original = value;
                self.original_current_attested = actual_current;
                self.original_origin = origin;
            }
            Side::Rust => {
                self.translated = value;
                self.translated_current_attested = actual_current;
                self.translated_origin = origin;
            }
        }
    }

    fn equal(self) -> bool {
        self.original_current_attested
            && self.translated_current_attested
            && self.original == self.translated
    }
}

/// Identity of an actually accepted exception, not a pending interrupt level.
/// `interrupted_state` is the CPU's complete `hybrid_state_words()` before the
/// tick that accepted it (including GPRs, banked stacks, SR, PC and prefetch).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IrqIdentity {
    pub vector: u32,
    pub handler: u32,
    pub exception_depth: u32,
    pub entry_master_clock: u64,
    pub interrupted_state: [u32; 25],
}

impl IrqIdentity {
    pub fn interrupted_pc(&self) -> u32 {
        self.interrupted_state[17]
    }

    pub fn interrupted_sp(&self) -> u32 {
        // hybrid_state_words: USP, SSP, PC, SR start at indices 15..19.
        self.interrupted_state[if self.interrupted_state[18] & 0x2000 != 0 {
            16
        } else {
            15
        }]
    }

    pub fn supervisor_sp(&self) -> u32 {
        self.interrupted_state[16]
    }
}

/// Source-declared physical stack ownership, not a general CPU memory rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StackRegion {
    pub start: u32,
    pub end: u32,
}

/// Tags come from actual exception-entry or decoded save instructions. A write
/// merely below SSP is not a frame save (it may be a global/scratch store).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameCommitKind {
    ExceptionEntry,
    RegisterSave,
    CallReturn,
    ArgumentSave,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameCommit {
    pub kind: FrameCommitKind,
    pub access: HybridAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DifferingByte {
    pub address: u32,
    pub original: u8,
    pub translated: u8,
}

impl DifferingByte {
    fn value(self, side: Side) -> u8 {
        match side {
            Side::Original => self.original,
            Side::Rust => self.translated,
        }
    }
}

/// A source-reviewed commutation's exact dynamic frame debt. Construction checks
/// the private-byte ownership witness from actual commits in that IRQ envelope;
/// a caller-provided frame size alone is deliberately insufficient.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WatchReceipt {
    owner_side: Side,
    irq: IrqIdentity,
    frame_start: u32,
    frame_end: u32,
    bytes: Vec<DifferingByte>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WatchError {
    #[error("the provenance campaign is already closed")]
    CampaignClosed,
    #[error("a private-frame receipt must contain differing bytes")]
    EmptyReceipt,
    #[error("invalid actual interrupt identity or supervisor stack boundary")]
    InvalidIdentity,
    #[error("private-frame byte {0:06X} has equal original and translated values")]
    EqualByte(u32),
    #[error("private-frame byte {0:06X} occurs more than once through RAM aliases")]
    DuplicateByte(u32),
    #[error("byte {0:06X} was not an observed private RAM commit of this interrupt")]
    UnownedByte(u32),
    #[error("actual interrupt chronology is missing, ambiguous or out of order")]
    InterruptChronology,
    #[error("a speculative branch cannot be merged into this provenance campaign")]
    ForeignBranch,
    #[error("actual RTE byte values do not attest the queued private-frame receipt")]
    ActivationValues,
    #[error("unknown provenance approval id")]
    UnknownApproval,
    #[error("provenance approval identifiers are exhausted")]
    ApprovalIdsExhausted,
    #[error("missing indexed actual exception boundary payload")]
    BoundaryPayload,
}

impl WatchReceipt {
    pub fn from_observed_irq(
        owner_side: Side,
        irq: IrqIdentity,
        mut bytes: Vec<DifferingByte>,
        stack: StackRegion,
        committed_irq_accesses: &[FrameCommit],
    ) -> Result<Self, WatchError> {
        let boundary = canonical_ram_address(irq.supervisor_sp());
        if irq.exception_depth == 0
            || !(24..=31).contains(&irq.vector)
            || irq.supervisor_sp() & 0xffffff < 0xe00000
            || stack.start < 0xff0000
            || stack.end > 0x1000000
            || stack.start >= stack.end
            || !(stack.start + 1..=stack.end).contains(&boundary)
        {
            return Err(WatchError::InvalidIdentity);
        }
        if bytes.is_empty() {
            return Err(WatchError::EmptyReceipt);
        }
        let mut owned = BTreeSet::new();
        for &access in committed_irq_accesses {
            if access.kind != FrameCommitKind::Other
                && let HybridAccess::Write { address, word, .. } = access.access
            {
                for lane in 0..if word { 2 } else { 1 } {
                    if let Some(address) = ram_byte(address.wrapping_add(lane))
                        && (stack.start..boundary).contains(&address)
                    {
                        owned.insert(address);
                    }
                }
            }
        }
        let mut seen = BTreeSet::new();
        for byte in &mut bytes {
            let Some(address) = ram_byte(byte.address) else {
                return Err(WatchError::UnownedByte(byte.address & 0xffffff));
            };
            if byte.original == byte.translated {
                return Err(WatchError::EqualByte(address));
            }
            if !seen.insert(address) {
                return Err(WatchError::DuplicateByte(address));
            }
            if !owned.contains(&address) {
                return Err(WatchError::UnownedByte(address));
            }
            byte.address = address;
        }
        bytes.sort_unstable_by_key(|byte| byte.address);
        Ok(Self {
            owner_side,
            irq,
            frame_start: *owned.first().expect("owned differing bytes"),
            frame_end: owned.last().expect("owned differing bytes") + 1,
            bytes,
        })
    }

    pub fn identity(&self) -> &IrqIdentity {
        &self.irq
    }

    pub fn bytes(&self) -> &[DifferingByte] {
        &self.bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationOrigin {
    Live,
    CheckFork,
    DependencyProbe,
    CanonicalReplay,
    FixtureReplay,
}

/// A real observation checkpoint. DMA has no invented CPU operand PC; callers
/// retain its surrounding checkpoint clock without claiming an exact bus cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AccessCheckpoint {
    pub frame: u64,
    pub master_clock: u64,
    pub cpu_pc: Option<u32>,
    pub origin: ObservationOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadOrigin {
    CpuSemantic,
    CpuDiscarded,
    CpuPrefetch,
    NativeSemantic,
    NativeDiscarded,
    NativePrefetch,
    VdpDma,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadViolation {
    pub receipt: ReceiptId,
    pub side: Side,
    pub current_values: CurrentByteValues,
    pub checkpoint: AccessCheckpoint,
    pub origin: ReadOrigin,
    pub raw_address: u32,
    pub width: u8,
    pub value: u16,
    pub byte: DifferingByte,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignEnd {
    Completed,
    Reset,
    ObserverLost,
    EvidenceMismatch,
    NotLiveExecution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReceiptOutcome {
    PendingIrq,
    InIrq,
    Active,
    Discharged,
    Invalidated {
        violation: ReadViolation,
    },
    Provisional {
        reason: CampaignEnd,
        unresolved_bytes: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ApprovalOutcome {
    NotTracked,
    Provisional { receipts: Vec<ReceiptId> },
    Discharged,
    Invalidated { receipts: Vec<ReceiptId> },
}

/// Allocation-free observation only; no variant is an approval capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DependencyOutcome {
    NotTracked,
    Provisional,
    Discharged,
    Invalidated,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReceiptEvidence {
    pub id: ReceiptId,
    pub receipt: WatchReceipt,
    pub approvals: BTreeSet<ApprovalId>,
    pub outcome: ReceiptOutcome,
    pub activated_master_clock: Option<u64>,
    pub remaining: BTreeSet<u32>,
    /// Requested lanes with a CPU-attributed M68k producer in the matched IRQ.
    pub observed_frame_producers: BTreeSet<u32>,
    /// Values actually committed to requested lanes after the matching entry.
    pub observed_frame_values: BTreeMap<u32, u8>,
    pub current_values: BTreeMap<u32, CurrentByteValues>,
}

#[derive(Debug, Clone, Serialize)]
struct WatchEvidenceSnapshot {
    next_approval: u64,
    receipts: Vec<ReceiptEvidence>,
    closed: Option<CampaignEnd>,
}

/// Cloneable analysis state. A clone's commits discharge only that clone; merging
/// it propagates sticky rejections, never overwrites the live remaining set.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FutureFrameWatches {
    next_approval: u64,
    #[serde(skip)]
    campaign_identity: Option<Arc<()>>,
    receipts: Vec<ReceiptEvidence>,
    #[serde(skip)]
    active: BTreeMap<u32, BTreeSet<ReceiptId>>,
    #[serde(skip)]
    attesting: BTreeMap<u32, BTreeSet<ReceiptId>>,
    #[serde(skip)]
    interrupt_stack: [Vec<IrqIdentity>; 2],
    #[serde(skip)]
    entry_prefix: [Vec<HybridAccess>; 2],
    #[serde(skip)]
    rejections_pending: bool,
    closed: Option<CampaignEnd>,
}

impl FutureFrameWatches {
    pub fn new_approval(&mut self) -> Result<ApprovalId, WatchError> {
        if self.closed.is_some() {
            return Err(WatchError::CampaignClosed);
        }
        let id = ApprovalId(self.next_approval);
        self.next_approval = self
            .next_approval
            .checked_add(1)
            .ok_or(WatchError::ApprovalIdsExhausted)?;
        self.campaign_identity.get_or_insert_with(|| Arc::new(()));
        Ok(id)
    }

    /// Queue a receipt before its expected live IRQ. It cannot become active
    /// merely because a ready PC, pending IRQ level or frame number agrees.
    pub fn arm(
        &mut self,
        approval: ApprovalId,
        receipt: WatchReceipt,
    ) -> Result<ReceiptId, WatchError> {
        if self.closed.is_some() {
            return Err(WatchError::CampaignClosed);
        }
        if approval.0 >= self.next_approval {
            return Err(WatchError::UnknownApproval);
        }
        let id = ReceiptId(self.receipts.len() as u64);
        let remaining = receipt.bytes.iter().map(|byte| byte.address).collect();
        let current_values = receipt
            .bytes
            .iter()
            .map(|byte| {
                (
                    byte.address,
                    CurrentByteValues {
                        original: byte.original,
                        translated: byte.translated,
                        original_current_attested: false,
                        translated_current_attested: false,
                        original_origin: ValueOrigin::EntrySnapshot,
                        translated_origin: ValueOrigin::EntrySnapshot,
                    },
                )
            })
            .collect();
        self.receipts.push(ReceiptEvidence {
            id,
            receipt,
            approvals: BTreeSet::from([approval]),
            outcome: ReceiptOutcome::PendingIrq,
            activated_master_clock: None,
            remaining,
            observed_frame_producers: BTreeSet::new(),
            observed_frame_values: BTreeMap::new(),
            current_values,
        });
        Ok(id)
    }

    fn hold_receipts(&mut self, ids: &[ReceiptId], reason: CampaignEnd) {
        for id in ids {
            let entry = &mut self.receipts[id.0 as usize];
            entry.outcome = ReceiptOutcome::Provisional {
                reason,
                unresolved_bytes: entry.remaining.len(),
            };
        }
    }

    fn take_rejections_pending(&mut self) -> bool {
        std::mem::take(&mut self.rejections_pending)
    }

    fn observe_setup_write(&mut self, side: Side, address: u32, value: u8) {
        if self.closed.is_some() {
            return;
        }
        self.record_attesting_write(
            side,
            HybridAccess::HostMutation {
                address,
                word: false,
                value: u16::from(value),
            },
            None,
            false,
        );
        if let Some(address) = ram_byte(address) {
            self.overwrite(side, address, value, false, ValueOrigin::ExternalSetup);
        }
    }

    /// Feed actual accepted exception entries, including nested exceptions that
    /// have no receipt. Keeping their nesting prevents a child's RTE from
    /// activating its parent's debt.
    pub fn enter_irq(&mut self, side: Side, actual: IrqIdentity) -> Result<(), WatchError> {
        if self.closed.is_some() {
            return Err(WatchError::CampaignClosed);
        }
        if actual.exception_depth == 0
            || self.interrupt_stack[side.index()]
                .last()
                .is_some_and(|parent| {
                    actual.exception_depth != parent.exception_depth + 1
                        || actual.entry_master_clock < parent.entry_master_clock
                })
        {
            return Err(WatchError::InterruptChronology);
        }
        for entry in &mut self.receipts {
            if entry.outcome == ReceiptOutcome::PendingIrq
                && entry.receipt.owner_side == side
                && entry.receipt.irq == actual
            {
                entry.outcome = ReceiptOutcome::InIrq;
                for &address in &entry.remaining {
                    self.attesting.entry(address).or_default().insert(entry.id);
                }
            }
        }
        self.interrupt_stack[side.index()].push(actual);
        Ok(())
    }

    pub fn current_irq(&self, side: Side) -> Option<&IrqIdentity> {
        self.interrupt_stack[side.index()].last()
    }

    /// Activate only after the observed RTE of the innermost accepted exception.
    /// `actual` is the previously observed entry identity, not a guessed owner.
    fn activate(
        &mut self,
        side: Side,
        actual: &IrqIdentity,
        rte_master_clock: u64,
    ) -> Result<(), WatchError> {
        if self.closed.is_some() {
            return Err(WatchError::CampaignClosed);
        }
        if self.interrupt_stack[side.index()].last() != Some(actual)
            || rte_master_clock < actual.entry_master_clock
        {
            return Err(WatchError::InterruptChronology);
        }
        self.interrupt_stack[side.index()].pop();
        let mut mismatch = false;
        for entry in &mut self.receipts {
            if entry.outcome == ReceiptOutcome::InIrq
                && entry.receipt.owner_side == side
                && entry.receipt.irq == *actual
            {
                let attested = entry.receipt.bytes.iter().all(|expected| {
                    entry.observed_frame_producers.contains(&expected.address)
                        && entry.observed_frame_values.get(&expected.address)
                            == Some(&expected.value(side))
                });
                if attested {
                    entry.outcome = ReceiptOutcome::Active;
                    entry.activated_master_clock = Some(rte_master_clock);
                    for &address in &entry.remaining {
                        self.active.entry(address).or_default().insert(entry.id);
                    }
                } else {
                    entry.outcome = ReceiptOutcome::Provisional {
                        reason: CampaignEnd::EvidenceMismatch,
                        unresolved_bytes: entry.remaining.len(),
                    };
                    mismatch = true;
                }
            }
        }
        self.attesting.retain(|_, ids| {
            ids.retain(|id| self.receipts[id.0 as usize].outcome == ReceiptOutcome::InIrq);
            !ids.is_empty()
        });
        if mismatch {
            Err(WatchError::ActivationValues)
        } else {
            Ok(())
        }
    }

    /// Begin one actual CPU/device step, never a ready-PC guess. Immediate bus
    /// providers may commit the six-byte IRQ header before reporting successful
    /// entry; only that exact current-step header is associated afterward.
    pub fn begin_access_batch(&mut self, side: Side) {
        self.entry_prefix[side.index()].clear();
    }

    /// Consume an indexed architectural boundary at its actual place among bus
    /// accesses. Requested frame values come from preceding actual commits, not
    /// from final RAM or an instruction-issue reconstruction.
    pub fn observe_stream(
        &mut self,
        side: Side,
        checkpoint: AccessCheckpoint,
        interrupted_state: &[u32; 25],
        access: HybridAccess,
        boundaries: &[genesis_core::HybridExceptionBoundary],
    ) -> Result<bool, WatchError> {
        self.observe_stream_inner(
            side,
            checkpoint,
            interrupted_state,
            access,
            boundaries,
            false,
        )
    }

    pub(crate) fn observe_live_stream(
        &mut self,
        checkpoint: AccessCheckpoint,
        interrupted_state: &[u32; 25],
        access: HybridAccess,
        boundaries: &[genesis_core::HybridExceptionBoundary],
    ) -> Result<bool, WatchError> {
        self.observe_stream_inner(
            Side::Original,
            checkpoint,
            interrupted_state,
            access,
            boundaries,
            true,
        )
    }

    fn observe_stream_inner(
        &mut self,
        side: Side,
        checkpoint: AccessCheckpoint,
        interrupted_state: &[u32; 25],
        access: HybridAccess,
        boundaries: &[genesis_core::HybridExceptionBoundary],
        actual_current: bool,
    ) -> Result<bool, WatchError> {
        let HybridAccess::ExceptionBoundary { index } = access else {
            if matches!(access, HybridAccess::Write { .. })
                && checkpoint.cpu_pc.is_some()
                && self.receipts.iter().any(|entry| {
                    entry.outcome == ReceiptOutcome::PendingIrq && entry.receipt.owner_side == side
                })
            {
                self.entry_prefix[side.index()].push(access);
            }
            return Ok(self.observe_inner(side, checkpoint, access, actual_current));
        };
        let Some(payload) = boundaries.get(index as usize) else {
            self.finish(CampaignEnd::ObserverLost);
            return Err(WatchError::BoundaryPayload);
        };
        match payload.boundary {
            m68000_emu::ExceptionBoundary::Entered {
                vector,
                handler,
                depth,
            } => {
                self.enter_irq(
                    side,
                    IrqIdentity {
                        vector,
                        handler,
                        exception_depth: depth,
                        entry_master_clock: payload.master_clock,
                        interrupted_state: *interrupted_state,
                    },
                )?;
                if (24..=31).contains(&vector) {
                    let end = canonical_ram_address(interrupted_state[16]);
                    let header = end.saturating_sub(6)..end;
                    for index in 0..self.entry_prefix[side.index()].len() {
                        self.record_attesting_write(
                            side,
                            self.entry_prefix[side.index()][index],
                            Some(&header),
                            true,
                        );
                    }
                }
                self.entry_prefix[side.index()].clear();
            }
            m68000_emu::ExceptionBoundary::Returned { depth } => {
                let Some(owner) = self.current_irq(side).cloned() else {
                    // An exception entered before this observer began has no
                    // matching receipt and cannot activate one. Its real frame
                    // reads were still observed. Never invent a missing owner.
                    if !self.receipts.iter().any(|entry| {
                        entry.outcome == ReceiptOutcome::InIrq && entry.receipt.owner_side == side
                    }) {
                        return Ok(false);
                    }
                    self.finish(CampaignEnd::ObserverLost);
                    return Err(WatchError::InterruptChronology);
                };
                if depth.checked_add(1) != Some(owner.exception_depth) {
                    self.finish(CampaignEnd::ObserverLost);
                    return Err(WatchError::InterruptChronology);
                }
                self.activate(side, &owner, payload.master_clock)?;
            }
        }
        Ok(false)
    }

    /// Consume actual chronological reads and committed RAM writes. ReadPurpose
    /// never discharges a version debt: even prefetch/discarded reads reject.
    /// Returns whether this access invalidated at least one receipt.
    pub fn observe(
        &mut self,
        side: Side,
        checkpoint: AccessCheckpoint,
        access: HybridAccess,
    ) -> bool {
        self.observe_inner(side, checkpoint, access, false)
    }

    // There is deliberately no production Rust-current authority provider:
    // check-mode Rust forks are speculative, not a persistent live trajectory.
    fn observe_inner(
        &mut self,
        side: Side,
        checkpoint: AccessCheckpoint,
        access: HybridAccess,
        actual_current: bool,
    ) -> bool {
        if self.closed.is_some() {
            return false;
        }
        let (address, width, value, origin) = match access {
            HybridAccess::Write {
                address,
                word,
                value,
            }
            | HybridAccess::Z80Write {
                address,
                word,
                value,
            }
            | HybridAccess::HostMutation {
                address,
                word,
                value,
            } => {
                let (writer, actual_current) = match access {
                    HybridAccess::HostMutation { .. } => (ValueOrigin::ExternalRecipe, false),
                    HybridAccess::Z80Write { .. } => (ValueOrigin::Z80Store, actual_current),
                    HybridAccess::Write { .. } => (
                        if checkpoint.cpu_pc.is_some() {
                            ValueOrigin::CpuStore
                        } else {
                            ValueOrigin::NativeStore
                        },
                        actual_current && checkpoint.cpu_pc.is_some(),
                    ),
                    HybridAccess::Read { .. }
                    | HybridAccess::DmaRead { .. }
                    | HybridAccess::ExceptionBoundary { .. } => unreachable!("write observation"),
                };
                self.record_attesting_write(
                    side,
                    access,
                    None,
                    matches!(access, HybridAccess::Write { .. }) && checkpoint.cpu_pc.is_some(),
                );
                for lane in 0..if word { 2 } else { 1 } {
                    if let Some(address) = ram_byte(address.wrapping_add(lane)) {
                        let byte = if word && lane == 0 {
                            (value >> 8) as u8
                        } else {
                            value as u8
                        };
                        self.overwrite(side, address, byte, actual_current, writer);
                    }
                }
                return false;
            }
            HybridAccess::Read {
                address,
                word,
                value,
                purpose,
            } => (
                address,
                if word { 2 } else { 1 },
                value,
                match (purpose, checkpoint.cpu_pc.is_some()) {
                    (ReadPurpose::Semantic, true) => ReadOrigin::CpuSemantic,
                    (ReadPurpose::Discarded, true) => ReadOrigin::CpuDiscarded,
                    (ReadPurpose::Prefetch, true) => ReadOrigin::CpuPrefetch,
                    (ReadPurpose::Semantic, false) => ReadOrigin::NativeSemantic,
                    (ReadPurpose::Discarded, false) => ReadOrigin::NativeDiscarded,
                    (ReadPurpose::Prefetch, false) => ReadOrigin::NativePrefetch,
                },
            ),
            HybridAccess::DmaRead { address, value } => (address, 2, value, ReadOrigin::VdpDma),
            HybridAccess::ExceptionBoundary { .. } => {
                // An unresolved indexed marker is missing observer evidence;
                // only observe_stream may consume it with its actual payload.
                self.finish(CampaignEnd::ObserverLost);
                return false;
            }
        };
        let mut invalidated = false;
        for lane in 0..width {
            let Some(canonical) = ram_byte(address.wrapping_add(u32::from(lane))) else {
                continue;
            };
            let Some(ids) = self.active.remove(&canonical) else {
                continue;
            };
            for id in ids {
                let entry = &mut self.receipts[id.0 as usize];
                if entry.outcome != ReceiptOutcome::Active {
                    continue;
                }
                let byte = *entry
                    .receipt
                    .bytes
                    .iter()
                    .find(|byte| byte.address == canonical)
                    .expect("indexed private-frame byte");
                entry.outcome = ReceiptOutcome::Invalidated {
                    violation: ReadViolation {
                        receipt: id,
                        side,
                        current_values: entry.current_values[&canonical],
                        checkpoint: AccessCheckpoint {
                            cpu_pc: if origin == ReadOrigin::VdpDma {
                                None
                            } else {
                                checkpoint.cpu_pc
                            },
                            ..checkpoint
                        },
                        origin,
                        raw_address: address & 0xffffff,
                        width,
                        value,
                        byte,
                    },
                };
                invalidated = true;
            }
        }
        if invalidated {
            self.rejections_pending = true;
            self.prune_invalidated();
        }
        invalidated
    }

    #[cfg(test)]
    fn observe_paired_actual_for_model_test(
        &mut self,
        side: Side,
        checkpoint: AccessCheckpoint,
        access: HybridAccess,
    ) -> bool {
        self.observe_inner(side, checkpoint, access, true)
    }

    fn record_attesting_write(
        &mut self,
        side: Side,
        access: HybridAccess,
        range: Option<&std::ops::Range<u32>>,
        producer_commit: bool,
    ) {
        let (address, word, value) = match access {
            HybridAccess::Write {
                address,
                word,
                value,
            }
            | HybridAccess::Z80Write {
                address,
                word,
                value,
            }
            | HybridAccess::HostMutation {
                address,
                word,
                value,
            } => (address, word, value),
            HybridAccess::Read { .. }
            | HybridAccess::DmaRead { .. }
            | HybridAccess::ExceptionBoundary { .. } => return,
        };
        for lane in 0..if word { 2 } else { 1 } {
            if let Some(address) = ram_byte(address.wrapping_add(lane))
                && range.is_none_or(|range| range.contains(&address))
                && let Some(ids) = self.attesting.get(&address)
            {
                let byte = if word && lane == 0 {
                    (value >> 8) as u8
                } else {
                    value as u8
                };
                for id in ids {
                    let entry = &mut self.receipts[id.0 as usize];
                    // Only a real CPU-attributed M68k commit can establish the
                    // producer witness. Other physical writers update an
                    // already witnessed lane, never invent a missing save.
                    if entry.receipt.owner_side == side {
                        if producer_commit {
                            entry.observed_frame_producers.insert(address);
                        }
                        if entry.observed_frame_producers.contains(&address) {
                            entry.observed_frame_values.insert(address, byte);
                        }
                    }
                }
            }
        }
    }

    fn overwrite(
        &mut self,
        side: Side,
        address: u32,
        value: u8,
        actual_current: bool,
        origin: ValueOrigin,
    ) {
        if let Some(ids) = self.active.get_mut(&address) {
            ids.retain(|id| {
                let entry = &mut self.receipts[id.0 as usize];
                if entry.outcome != ReceiptOutcome::Active {
                    return false;
                }
                let versions = entry
                    .current_values
                    .get_mut(&address)
                    .expect("indexed byte versions");
                versions.write(side, value, actual_current, origin);
                if !versions.equal() {
                    return true;
                }
                entry.remaining.remove(&address);
                if entry.remaining.is_empty() {
                    entry.outcome = ReceiptOutcome::Discharged;
                }
                false
            });
            if ids.is_empty() {
                self.active.remove(&address);
            }
        }
    }

    fn prune_invalidated(&mut self) {
        self.active.retain(|_, ids| {
            ids.retain(|id| self.receipts[id.0 as usize].outcome == ReceiptOutcome::Active);
            !ids.is_empty()
        });
        self.attesting.retain(|_, ids| {
            ids.retain(|id| self.receipts[id.0 as usize].outcome == ReceiptOutcome::InIrq);
            !ids.is_empty()
        });
    }

    /// Propagate branch rejections without allowing speculative writes to clear
    /// the live debt. Branches may not add/rebind the campaign's receipts.
    pub fn merge_branch_rejections(&mut self, branch: &Self) -> Result<bool, WatchError> {
        let same_campaign = match (&self.campaign_identity, &branch.campaign_identity) {
            (Some(live), Some(fork)) => Arc::ptr_eq(live, fork),
            (None, None) => self.receipts.is_empty() && branch.receipts.is_empty(),
            _ => false,
        };
        if !same_campaign
            || branch.next_approval != self.next_approval
            || branch.receipts.len() != self.receipts.len()
            || self
                .receipts
                .iter()
                .zip(&branch.receipts)
                .any(|(live, fork)| {
                    live.id != fork.id
                        || live.receipt != fork.receipt
                        || live.approvals != fork.approvals
                })
        {
            return Err(WatchError::ForeignBranch);
        }
        let mut invalidated = false;
        for (live, fork) in self.receipts.iter_mut().zip(&branch.receipts) {
            if !matches!(live.outcome, ReceiptOutcome::Invalidated { .. })
                && let ReceiptOutcome::Invalidated { violation } = &fork.outcome
            {
                live.outcome = ReceiptOutcome::Invalidated {
                    violation: violation.clone(),
                };
                invalidated = true;
            }
        }
        self.rejections_pending |= invalidated;
        self.prune_invalidated();
        Ok(invalidated)
    }

    pub fn outcome(&self, approval: ApprovalId) -> ApprovalOutcome {
        let mut known = false;
        let mut provisional = Vec::new();
        let mut invalidated = Vec::new();
        for entry in &self.receipts {
            if !entry.approvals.contains(&approval) {
                continue;
            }
            known = true;
            match entry.outcome {
                ReceiptOutcome::Invalidated { .. } => invalidated.push(entry.id),
                ReceiptOutcome::Discharged => {}
                _ => provisional.push(entry.id),
            }
        }
        if !invalidated.is_empty() {
            ApprovalOutcome::Invalidated {
                receipts: invalidated,
            }
        } else if !provisional.is_empty() {
            ApprovalOutcome::Provisional {
                receipts: provisional,
            }
        } else if known {
            ApprovalOutcome::Discharged
        } else if approval.0 < self.next_approval {
            ApprovalOutcome::Provisional {
                receipts: Vec::new(),
            }
        } else {
            ApprovalOutcome::NotTracked
        }
    }
    pub(crate) fn dependency_outcome(&self, approval: ApprovalId) -> DependencyOutcome {
        let mut known = false;
        let mut provisional = false;
        for entry in &self.receipts {
            if !entry.approvals.contains(&approval) {
                continue;
            }
            known = true;
            match entry.outcome {
                ReceiptOutcome::Invalidated { .. } => return DependencyOutcome::Invalidated,
                ReceiptOutcome::Discharged => {}
                _ => provisional = true,
            }
        }
        if provisional {
            DependencyOutcome::Provisional
        } else if known {
            DependencyOutcome::Discharged
        } else if approval.0 < self.next_approval {
            DependencyOutcome::Provisional
        } else {
            DependencyOutcome::NotTracked
        }
    }

    pub fn evidence(&self) -> &[ReceiptEvidence] {
        &self.receipts
    }

    fn evidence_snapshot(&self) -> WatchEvidenceSnapshot {
        WatchEvidenceSnapshot {
            next_approval: self.next_approval,
            receipts: self.receipts.clone(),
            closed: self.closed,
        }
    }

    pub fn needs_observation(&self) -> bool {
        self.closed.is_none()
            && self.receipts.iter().any(|entry| {
                matches!(
                    entry.outcome,
                    ReceiptOutcome::PendingIrq | ReceiptOutcome::InIrq | ReceiptOutcome::Active
                )
            })
    }

    /// End/reset never upgrades unresolved version debt to success. Callers keep
    /// this report before creating a fresh tracker for another campaign.
    pub fn finish(&mut self, reason: CampaignEnd) {
        if self.closed.is_some() {
            return;
        }
        self.closed = Some(reason);
        for entry in &mut self.receipts {
            if matches!(
                entry.outcome,
                ReceiptOutcome::PendingIrq | ReceiptOutcome::InIrq | ReceiptOutcome::Active
            ) {
                entry.outcome = ReceiptOutcome::Provisional {
                    reason,
                    unresolved_bytes: entry.remaining.len(),
                };
            }
        }
        self.active.clear();
        self.attesting.clear();
        for stack in &mut self.interrupt_stack {
            stack.clear();
        }
        for prefix in &mut self.entry_prefix {
            prefix.clear();
        }
    }
}

fn ram_byte(address: u32) -> Option<u32> {
    let physical = address & 0xffffff;
    (physical >= 0xe00000).then(|| canonical_ram_address(physical))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_tests;

mod host;
pub(crate) use host::{BranchProvenance, LiveObserverLease, ProofDependency};
pub use host::{ProvenanceReport, ProvenanceStatus, ProvenanceSummary};
