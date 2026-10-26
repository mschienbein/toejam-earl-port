//! Actual-original witnesses. Only the live call tracker constructs invocation
//! identities; diagnostic serials, snapshots and translated code cannot mint one.
use super::{Trajectory, Unsupported, fresh};
use crate::call_observation::invocation::{
    ActualCallId, ExceptionContextId, InvocationId as ActualInvocationId,
};
use genesis_core::{GenesisEmulator, HybridExceptionBoundary};
use m68000_emu::{HybridExecutedInstruction, ReadPurpose};
use std::{ops::Range, sync::Arc};
use tje_engine_core::CpuState;
use tje_game::FunctionId;

mod journal;
mod tracker;
pub(super) use journal::{Before, Journal, ObservedStep};
pub(super) use tracker::{Target, Tracker};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct InvocationId {
    trajectory: Trajectory,
    serial: u64,
}
impl InvocationId {
    pub(super) fn trajectory(self) -> Trajectory {
        self.trajectory
    }
    pub(super) fn serial(self) -> u64 {
        self.serial
    }
    fn new(trajectory: Trajectory) -> Self {
        Self {
            trajectory,
            serial: fresh(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CpuSnapshot {
    pub words: [u32; 25],
    pub exception_depth: u32,
    pub opcode: u16,
}
impl CpuSnapshot {
    pub fn capture(emulator: &GenesisEmulator) -> Self {
        Self::from_cpu(emulator.hybrid_cpu())
    }
    fn from_cpu(cpu: &m68000_emu::M68000) -> Self {
        Self {
            words: cpu.hybrid_state_words(),
            exception_depth: cpu.hybrid_exception_depth(),
            opcode: cpu.next_opcode(),
        }
    }
    pub fn pc(self) -> u32 {
        self.words[17]
    }
    pub fn sr(self) -> u32 {
        self.words[18]
    }
    pub fn sp(self) -> u32 {
        self.words[if self.sr() & 0x2000 != 0 { 16 } else { 15 }]
    }
    pub fn engine(self) -> CpuState {
        CpuState {
            d: self.words[..8].try_into().expect("canonical D registers"),
            a: self.words[8..15]
                .try_into()
                .expect("canonical A0-A6 registers"),
            usp: self.words[15],
            ssp: self.words[16],
            pc: self.words[17],
            sr: self.words[18],
            prefetch: self.words[19],
            pending_interrupt: self.words[20],
            stopped: self.words[21],
            frozen: self.words[22],
            address_error: self.words[23],
            last_muldiv: self.words[24],
        }
    }
}

/// Passive evidence only. The ordinary replay profile cannot adopt or prove
/// ownership of unresolved DMA or a command still waiting to become DMA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DmaBoundary {
    pub active: bool,
    pub queued_port_writes: usize,
    pub partial_control: bool,
    pub queued_cpu_bytes: usize,
    pub queued_cpu_words: usize,
}
impl DmaBoundary {
    pub fn capture(bus: &genesis_core::GenesisBus) -> Self {
        let vdp = bus.vdp.hybrid_checkpoint();
        let cpu = bus.hybrid_checkpoint();
        Self {
            active: vdp.dma_active,
            queued_port_writes: vdp.pending_dma_port_writes,
            partial_control: vdp.control_word_pending,
            queued_cpu_bytes: cpu.pending_byte_writes,
            queued_cpu_words: cpu.pending_word_writes,
        }
    }
    pub fn unresolved(self) -> bool {
        self.active
            || self.queued_port_writes != 0
            || self.partial_control
            || self.queued_cpu_bytes != 0
            || self.queued_cpu_words != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Boundary {
    pub cpu: CpuSnapshot,
    pub frame: u64,
    pub master_clock: u64,
    pub ticks: u64,
    pub instructions: u64,
    pub dma: DmaBoundary,
}
impl Boundary {
    pub fn capture(emulator: &GenesisEmulator, frame: u64, ticks: u64, instructions: u64) -> Self {
        Self {
            cpu: CpuSnapshot::capture(emulator),
            frame,
            ticks,
            instructions,
            master_clock: emulator.hybrid_bus().cycles.m68k_mclk_cycles,
            dma: DmaBoundary::capture(emulator.hybrid_bus()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Direction {
    Read(ReadPurpose),
    Write,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Access {
    pub address: u32,
    pub word: bool,
    pub direction: Direction,
    pub value: u16,
}
impl Access {
    pub fn bytes(self) -> impl Iterator<Item = (u32, u8)> {
        let [high, low] = self.value.to_be_bytes();
        (0..if self.word { 2 } else { 1 }).map(move |index| {
            (
                self.address.wrapping_add(index),
                if self.word && index == 0 { high } else { low },
            )
        })
    }
    pub fn ram(self) -> bool {
        self.address >= 0xe00000 && self.address <= 0xffffff
    }
}

/// CPU issue and physical commit are deliberately distinct. A MOVEM terminal
/// read, for example, occurs after queued issues but before their commits.
#[derive(Debug, Clone, Copy)]
pub(super) enum Chronology {
    CpuRead { access: Access, issue: usize },
    CpuCommit { access: Access, issue: usize },
    DmaRead { address: u32, value: u16 },
    Z80Write(Access),
    HostMutation(Access),
    Exception(HybridExceptionBoundary),
    Acknowledge { level: u8 },
}

#[derive(Debug, Clone)]
pub(super) struct Instruction {
    pub before: Boundary,
    pub actual: HybridExecutedInstruction,
    pub after_sp: u32,
    pub after_sr: u32,
    pub after_pc: u32,
    pub issues: Range<usize>,
    pub chronology: Range<usize>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum EntryKind {
    Call {
        caller_pc: u32,
        opcode: u16,
        return_pc: u32,
    },
    Interrupt {
        vector: u32,
    },
}

/// Opaque historical evidence minted only by the shared actual-call recorder.
/// Diagnostic serials above remain map/report identities, not call authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EntryEvidence {
    Call {
        raw: ActualInvocationId,
        actual: ActualCallId,
    },
    Interrupt(ExceptionContextId),
}
#[derive(Debug, Clone, Copy)]
pub(super) struct Entry {
    pub identity: InvocationId,
    pub function: Option<FunctionId>,
    pub boundary: Boundary,
    pub kind: EntryKind,
    pub parent: Option<InvocationId>,
    pub evidence: EntryEvidence,
}

#[derive(Debug)]
pub(super) struct IdleStep {
    pub before: Boundary,
    pub after: Boundary,
    pub issues: Range<usize>,
    pub chronology: Range<usize>,
}

#[derive(Debug)]
pub(super) enum Event {
    Instruction(usize),
    Idle(usize),
    Callee(Arc<Invocation>),
    Interrupt(Arc<Invocation>),
}

/// Flat vectors avoid per-instruction journal allocations. A completed witness
/// is immutable and excludes the disposable analysis-entry emulator snapshot.
#[derive(Debug, Default)]
pub(super) struct Trace {
    pub instructions: Vec<Instruction>,
    pub idles: Vec<IdleStep>,
    pub issues: Vec<Access>,
    pub driven_masks: Vec<u16>,
    pub chronology: Vec<Chronology>,
    pub events: Vec<Event>,
}
impl Trace {
    fn append(
        &mut self,
        issues: &[Access],
        driven_masks: &[u16],
        chronology: &[Chronology],
    ) -> (Range<usize>, Range<usize>) {
        let issue_start = self.issues.len();
        let chronology_start = self.chronology.len();
        self.issues.extend_from_slice(issues);
        assert_eq!(
            issues.len(),
            driven_masks.len(),
            "validated actual-read mask alignment"
        );
        self.driven_masks.extend_from_slice(driven_masks);
        self.chronology
            .extend(chronology.iter().copied().map(|event| match event {
                Chronology::CpuCommit { access, issue } => Chronology::CpuCommit {
                    access,
                    issue: issue + issue_start,
                },
                Chronology::CpuRead { access, issue } => Chronology::CpuRead {
                    access,
                    issue: issue + issue_start,
                },
                event => event,
            }));
        (
            issue_start..self.issues.len(),
            chronology_start..self.chronology.len(),
        )
    }
    pub fn instruction(
        &mut self,
        before: Boundary,
        after: Boundary,
        actual: HybridExecutedInstruction,
        issues: &[Access],
        driven_masks: &[u16],
        chronology: &[Chronology],
    ) {
        let (issues, chronology) = self.append(issues, driven_masks, chronology);
        self.events
            .push(Event::Instruction(self.instructions.len()));
        self.instructions.push(Instruction {
            before,
            actual,
            after_sp: after.cpu.sp(),
            after_sr: after.cpu.sr(),
            after_pc: after.cpu.pc(),
            issues,
            chronology,
        });
    }
    pub fn idle(
        &mut self,
        before: Boundary,
        after: Boundary,
        issues: &[Access],
        driven_masks: &[u16],
        chronology: &[Chronology],
    ) {
        if issues.is_empty() && chronology.is_empty() {
            return;
        }
        let (issues, chronology) = self.append(issues, driven_masks, chronology);
        self.events.push(Event::Idle(self.idles.len()));
        self.idles.push(IdleStep {
            before,
            after,
            issues,
            chronology,
        });
    }
}

#[derive(Debug)]
pub(super) struct Invocation {
    pub entry: Entry,
    pub exit: Boundary,
    /// The actual shared return update, checked against entry before grouping.
    /// Historical identity only; never a read permission or a new call seal.
    pub completion_evidence: EntryEvidence,
    pub trace: Trace,
    pub unsupported: Option<Unsupported>,
}

pub(super) fn raw_ram(emulator: &GenesisEmulator) -> Box<[u8; 65536]> {
    let mut bytes = Vec::with_capacity(65536);
    for word in emulator.hybrid_bus().memory.hybrid_raw_main_ram() {
        bytes.extend_from_slice(&word.to_be_bytes());
    }
    bytes
        .into_boxed_slice()
        .try_into()
        .expect("Genesis work RAM is exactly 64 KiB")
}
