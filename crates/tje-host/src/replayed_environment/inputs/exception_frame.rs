//! Data-only identity transport for an actual accepted hardware frame and its
//! matching own RTE. No saved Original SR value is exposed/imported into Rust.
use super::super::{
    Failure, Unsupported, UnsupportedKind,
    record::{
        Access, Chronology, Direction, EntryEvidence, EntryKind, Event, IdleStep, Instruction,
        Invocation, InvocationId,
    },
};
use crate::{abi::canonical_ram_address, call_observation::invocation::ExceptionContextId};
use m68000_emu::{ExceptionBoundary, ReadPurpose};

fn unsupported(detail: &'static str) -> Failure {
    Failure::Unsupported(Unsupported {
        kind: UnsupportedKind::TypedControlProfile,
        detail: detail.into(),
    })
}
pub(in crate::replayed_environment) struct ExceptionFrame {
    owner: InvocationId,
    context: ExceptionContextId,
    address: u32,
    caller_pc: u32,
    caller_sp: u32,
    caller_sr: u16,
    caller_usp: u32,
    caller_ssp: u32,
    caller_depth: u32,
    modified: bool,
    returning: bool,
    returned: bool,
}
impl ExceptionFrame {
    pub fn capture(owner: &Invocation, index: usize, idle: &IdleStep) -> Result<Self, Failure> {
        let EntryEvidence::Interrupt(context) = owner.entry.evidence else {
            return Err(unsupported(
                "hardware frame lacks its sealed actual exception context",
            ));
        };
        let EntryKind::Interrupt { vector } = owner.entry.kind else {
            return Err(unsupported(
                "hardware frame was assigned to a call rather than an IRQ",
            ));
        };
        let before = idle.before.cpu;
        let after = idle.after.cpu;
        if !matches!(owner.trace.events.first(), Some(Event::Idle(first)) if *first == index)
            || idle.after != owner.entry.boundary
            || !(25..=31).contains(&vector)
            || owner.completion_evidence != EntryEvidence::Interrupt(context)
            || before.sr() & 0x2000 == 0
            || after.sr() & 0x2000 == 0
            || before.exception_depth.checked_add(1) != Some(after.exception_depth)
            || after.sp().wrapping_add(6) != before.sp()
            || before.words[15] != after.words[15]
            || owner.exit.cpu.pc() != before.pc()
            || owner.exit.cpu.sp() != before.sp()
            || owner.exit.cpu.sr() != before.sr()
            || owner.exit.cpu.words[15] != before.words[15]
            || owner.exit.cpu.words[16] != before.words[16]
            || owner.exit.cpu.exception_depth != before.exception_depth
        {
            return Err(unsupported(
                "hardware save/return is not this exact ordinary IRQ owner, frame, bank, depth and control point",
            ));
        }
        let frame = Self {
            owner: owner.entry.identity,
            context,
            address: canonical_ram_address(after.sp()),
            caller_pc: before.pc(),
            caller_sp: before.sp(),
            caller_sr: before.sr() as u16,
            caller_usp: before.words[15],
            caller_ssp: before.words[16],
            caller_depth: before.exception_depth,
            modified: false,
            returning: false,
            returned: false,
        };
        let mut issues = 0_u8;
        for &access in owner
            .trace
            .issues
            .get(idle.issues.clone())
            .ok_or_else(|| unsupported("IRQ hardware save has a missing raw issue span"))?
        {
            if access.direction != Direction::Write {
                continue;
            }
            let lane = frame.header_word(access).ok_or_else(|| {
                unsupported("IRQ entry has a changed/unknown hardware-frame producer")
            })?;
            if issues & (1 << lane) != 0 {
                return Err(unsupported("IRQ hardware save duplicated a frame word"));
            }
            // Pinned reference producer: PC high, PC low, saved SR.
            if [1, 2, 0].get(issues.count_ones() as usize).copied() != Some(lane) {
                return Err(unsupported(
                    "IRQ hardware-save physical issue order changed",
                ));
            }
            issues |= 1 << lane;
        }
        let mut committed = 0_u8;
        let mut acknowledge = None;
        let mut entered = 0;
        for event in owner
            .trace
            .chronology
            .get(idle.chronology.clone())
            .ok_or_else(|| {
                unsupported("IRQ hardware save has a missing physical chronology span")
            })?
        {
            match *event {
                Chronology::CpuCommit { access, issue } => {
                    let lane = frame.header_word(access).ok_or_else(|| {
                        unsupported("IRQ entry has an unknown physical frame commit")
                    })?;
                    if !idle.issues.contains(&issue)
                        || owner.trace.issues.get(issue).copied() != Some(access)
                        || committed & (1 << lane) != 0
                    {
                        return Err(unsupported(
                            "IRQ hardware save lost its exact single physical issue/commit pairing",
                        ));
                    }
                    if [1, 2, 0].get(committed.count_ones() as usize).copied() != Some(lane) {
                        return Err(unsupported(
                            "IRQ hardware-save physical commit order changed",
                        ));
                    }
                    committed |= 1 << lane;
                }
                Chronology::Acknowledge { level } => {
                    if acknowledge.replace(level).is_some() {
                        return Err(unsupported(
                            "IRQ hardware save has duplicate acknowledgements",
                        ));
                    }
                }
                Chronology::Z80Write(access) | Chronology::HostMutation(access) => {
                    if access.ram()
                        && access.bytes().any(|(address, _)| {
                            canonical_ram_address(address).wrapping_sub(frame.address) < 6
                        })
                    {
                        return Err(unsupported(
                            "a non-hardware writer changed the exception header during entry",
                        ));
                    }
                }
                Chronology::Exception(payload) => {
                    if matches!(payload.boundary, ExceptionBoundary::Entered { vector: actual, handler, depth }
                        if actual == vector && handler == after.pc() && depth == after.exception_depth)
                    {
                        entered += 1;
                    } else {
                        return Err(unsupported(
                            "IRQ entry contains a foreign/coalesced exception boundary",
                        ));
                    }
                }
                _ => {}
            }
        }
        if issues != 7
            || committed != 7
            || entered != 1
            || acknowledge.map(u32::from).map(|level| level + 24) != Some(vector)
        {
            return Err(unsupported(
                "IRQ hardware save lacks all three committed words and its actual acknowledgement",
            ));
        }
        Ok(frame)
    }
    fn header_word(&self, access: Access) -> Option<u8> {
        if !access.word || !access.ram() {
            return None;
        }
        let (lane, expected) =
            match canonical_ram_address(access.address).wrapping_sub(self.address) {
                0 => (0, self.caller_sr),
                2 => (1, (self.caller_pc >> 16) as u16),
                4 => (2, self.caller_pc as u16),
                _ => return None,
            };
        (access.value == expected).then_some(lane)
    }
    /// Every software, callee and non-main writer invalidates the identity,
    /// even if it happens to put back the same Original bytes.
    pub fn observe_write(&mut self, access: Access) {
        if !self.returned
            && access.ram()
            && access
                .bytes()
                .any(|(address, _)| canonical_ram_address(address).wrapping_sub(self.address) < 6)
        {
            self.modified = true;
        }
    }
    pub fn begin_return(
        &mut self,
        owner: &Invocation,
        index: usize,
        instruction: &Instruction,
    ) -> Result<(), Failure> {
        if self.owner != owner.entry.identity
            || owner.entry.evidence != EntryEvidence::Interrupt(self.context)
            || owner.completion_evidence != EntryEvidence::Interrupt(self.context)
            || self.modified
            || self.returning
            || self.returned
            || !matches!(owner.trace.events.last(), Some(Event::Instruction(last)) if *last == index)
            || instruction.actual.opcode != 0x4e73
            || !instruction.actual.completed_successfully
            || instruction.before.cpu.exception_depth != self.caller_depth + 1
            || canonical_ram_address(instruction.before.cpu.sp()) != self.address
            || instruction.after_pc != self.caller_pc
            || instruction.after_sp != self.caller_sp
            || instruction.after_sr != u32::from(self.caller_sr)
            || owner.exit.cpu.words[15] != self.caller_usp
            || owner.exit.cpu.words[16] != self.caller_ssp
        {
            return Err(unsupported(
                "ordinary RTE requires this untouched actual hardware-save identity; modified/foreign frames need their explicit control profile",
            ));
        }
        let mut reads = 0_u8;
        for &access in owner
            .trace
            .issues
            .get(instruction.issues.clone())
            .ok_or_else(|| unsupported("own RTE has a missing raw read span"))?
        {
            if access.direction != Direction::Read(ReadPurpose::Semantic) {
                continue;
            }
            let lane = self.header_word(access).ok_or_else(|| {
                unsupported("own RTE read does not match its actual hardware-save word")
            })?;
            if reads & (1 << lane) != 0 {
                return Err(unsupported("own RTE duplicated a hardware-header read"));
            }
            // Pinned reference consumer: saved SR, PC high, PC low.
            if [0, 1, 2].get(reads.count_ones() as usize).copied() != Some(lane) {
                return Err(unsupported("own RTE hardware-header read order changed"));
            }
            reads |= 1 << lane;
        }
        let mut returns = 0;
        let mut physical_reads = 0_u8;
        for event in owner
            .trace
            .chronology
            .get(instruction.chronology.clone())
            .ok_or_else(|| unsupported("own RTE has a missing actual return chronology"))?
        {
            match *event {
                Chronology::CpuRead { access, issue }
                    if access.direction == Direction::Read(ReadPurpose::Semantic) =>
                {
                    let lane = self.header_word(access).ok_or_else(|| {
                        unsupported("own RTE physical read is outside its hardware header")
                    })?;
                    if returns != 0
                        || !instruction.issues.contains(&issue)
                        || owner.trace.issues.get(issue).copied() != Some(access)
                        || [0, 1, 2].get(physical_reads.count_ones() as usize).copied()
                            != Some(lane)
                    {
                        return Err(unsupported(
                            "own RTE physical read order/issue span changed across its return marker",
                        ));
                    }
                    physical_reads |= 1 << lane;
                }
                Chronology::Exception(payload) => {
                    if !matches!(payload.boundary, ExceptionBoundary::Returned { depth } if depth == self.caller_depth)
                        || returns != 0
                        || physical_reads != 7
                    {
                        return Err(unsupported(
                            "own RTE return marker is foreign, duplicated or precedes its physical reads",
                        ));
                    }
                    returns += 1;
                }
                _ => {}
            }
        }
        if reads != 7 || physical_reads != 7 || returns != 1 {
            return Err(unsupported(
                "own RTE lacks its three header reads or single matched actual return boundary",
            ));
        }
        self.returning = true;
        Ok(())
    }
    pub fn finish_return(&mut self, depth: u32) -> Result<(), Failure> {
        if !self.returning || self.returned || self.modified || depth != self.caller_depth {
            return Err(unsupported(
                "own hardware-header identity was lost before its actual return marker",
            ));
        }
        self.returned = true;
        Ok(())
    }
    pub fn transports(
        &self,
        owner: &Invocation,
        instruction: &Instruction,
        access: Access,
    ) -> bool {
        self.owner == owner.entry.identity
            && !self.modified
            && self.returning
            && instruction.actual.opcode == 0x4e73
            && access.direction == Direction::Read(ReadPurpose::Semantic)
            && self.header_word(access).is_some()
    }
    pub fn finishes(&self, owner: &Invocation) -> bool {
        self.owner == owner.entry.identity && self.returned && !self.modified
    }
}
