//! Own issue order is exact; physical commits and genuine environment segments
//! keep their actual order. A callee marker is never skipped by ordinary reads.
use super::{
    Failure, Unsupported, UnsupportedKind,
    frame::FrameMemory,
    record::{Access, Chronology, Direction, Event, Instruction, Invocation},
};
use m68000_emu::ReadPurpose;

#[derive(Clone, Copy)]
pub(super) enum Token<'a> {
    Access {
        access: Access,
        issue: usize,
        pc: u32,
        opcode: u16,
    },
    FrameRead {
        access: Access,
        unwritten: u8,
        pc: u32,
        opcode: u16,
    },
    OptionalAbi(Access),
    Callee(&'a Invocation),
    Interrupt(&'a Invocation),
    End,
}
// 68000 MOVEM can store all 16 registers as two physical words each.
const MAX_INSTRUCTION_WORD_STORES: usize = 16 * 2;
#[derive(Clone, Copy)]
struct AbiDiscard {
    access: Access,
    instruction: usize,
    issue: usize,
}
pub(super) struct Cursor<'a> {
    pub original: &'a Invocation,
    pub memory: FrameMemory,
    event: usize,
    issue: usize,
    started: bool,
    matched_writes: [Option<(usize, Access)>; MAX_INSTRUCTION_WORD_STORES],
    retained_abi: Option<AbiDiscard>,
    pub own_accesses: u64,
    pub original_pc: Option<u32>,
    pub original_opcode: Option<u16>,
    rom_len: usize,
    own_code: std::ops::Range<u32>,
}
fn unsupported(kind: UnsupportedKind, detail: impl Into<String>) -> Failure {
    Failure::Unsupported(Unsupported {
        kind,
        detail: detail.into(),
    })
}
fn access_json(access: Access, sampled: bool) -> serde_json::Value {
    serde_json::json!({
        "address": access.address,
        "width_bits": if access.word { 16 } else { 8 },
        "kind": match access.direction { Direction::Write => "write", Direction::Read(_) => "read" },
        "purpose": match access.direction { Direction::Write => None,
            Direction::Read(purpose) => Some(format!("{purpose:?}")) },
        "value": if sampled || access.direction == Direction::Write { Some(access.value) } else { None },
    })
}
fn token_json(token: Token<'_>) -> serde_json::Value {
    match token {
        Token::Access {
            access, pc, opcode, ..
        }
        | Token::FrameRead {
            access, pc, opcode, ..
        } => {
            let mut value = access_json(access, true);
            value["pc"] = pc.into();
            value["opcode"] = opcode.into();
            value
        }
        Token::OptionalAbi(access) => access_json(access, true),
        Token::Callee(call) => {
            serde_json::json!({"callee":call.entry.function.map(|id| id.metadata().inventory_id),
            "invocation_id":call.entry.identity.serial()})
        }
        Token::Interrupt(irq) => {
            serde_json::json!({"interrupt_invocation_id":irq.entry.identity.serial()})
        }
        Token::End => serde_json::json!({"end":true}),
    }
}
fn optional_abi(instruction: &Instruction, access: Access, entry_sp: u32) -> bool {
    // The real nonempty stack MOVEM's terminal discarded WORD may be exactly
    // at entry SP (the return slot), outside the private allocation interval.
    let opcode = instruction.actual.opcode;
    let stack_movem = opcode & 0xfb80 == 0x4880 && opcode & 0x0400 != 0 && opcode & 63 == 0x1f;
    stack_movem
        && access.word
        && access.direction == Direction::Read(ReadPurpose::Discarded)
        && access.ram()
        && crate::abi::canonical_ram_address(instruction.before.cpu.sp())
            < crate::abi::canonical_ram_address(entry_sp)
        && crate::abi::canonical_ram_address(instruction.after_sp)
            == crate::abi::canonical_ram_address(entry_sp)
        && crate::abi::canonical_ram_address(access.address)
            == crate::abi::canonical_ram_address(entry_sp)
}
impl<'a> Cursor<'a> {
    fn compared(
        &self,
        point: &'static str,
        detail: String,
        expected: serde_json::Value,
        actual: serde_json::Value,
    ) -> Failure {
        Failure::Compared(Box::new(super::report::ComparisonDetail {
            point,
            original_pc: self.original_pc,
            original_opcode: self.original_opcode,
            own_access_index: Some(self.own_accesses),
            expected: Some(expected),
            actual: Some(actual),
            detail,
        }))
    }
    pub fn new(
        original: &'a Invocation,
        ram: Box<[u8; 65536]>,
        rom_len: usize,
        poison: u8,
        revision: tje_mem::Revision,
    ) -> Result<Self, Failure> {
        let owner = original.entry.function.ok_or_else(|| {
            unsupported(
                UnsupportedKind::EntryBoundary,
                "own root has no committed function extent",
            )
        })?;
        let start = owner.metadata().address(revision);
        let own_code = start
            ..start
                .checked_add(owner.metadata().size(revision))
                .ok_or_else(|| {
                    unsupported(
                        UnsupportedKind::EntryBoundary,
                        "committed function extent overflows",
                    )
                })?;
        Ok(Self {
            original,
            memory: FrameMemory::new(original, ram, poison)?,
            event: 0,
            issue: 0,
            started: false,
            matched_writes: [None; MAX_INSTRUCTION_WORD_STORES],
            retained_abi: None,
            own_accesses: 0,
            original_pc: None,
            original_opcode: None,
            rom_len,
            own_code,
        })
    }
    /// Structural elision closes this occurrence before another observable
    /// frontier. It is never a RAM-read/initialization or IRQ/callee waiver.
    pub fn elide_retained_abi(&mut self) {
        self.retained_abi = None;
    }
    /// The immediately preceding successful call belongs to this exact parent
    /// event, not another execution with an equal PC, SP or target name.
    pub fn call_instruction(&self, callee: &Invocation) -> Result<&'a Instruction, Failure> {
        let Some(Event::Callee(current)) = self.original.trace.events.get(self.event) else {
            return Err(unsupported(
                UnsupportedKind::EntryBoundary,
                "no current actual callee event",
            ));
        };
        let Some(Event::Instruction(index)) = self
            .event
            .checked_sub(1)
            .and_then(|index| self.original.trace.events.get(index))
        else {
            return Err(unsupported(
                UnsupportedKind::Observation,
                "callee lacks its actual source call instruction",
            ));
        };
        let instruction = &self.original.trace.instructions[*index];
        let super::record::EntryKind::Call {
            caller_pc, opcode, ..
        } = callee.entry.kind
        else {
            return Err(unsupported(
                UnsupportedKind::EntryBoundary,
                "callee is not a successful ordinary call",
            ));
        };
        if current.entry.identity != callee.entry.identity
            || callee.entry.parent != Some(self.original.entry.identity)
            || !crate::call_observation::successful_call_instruction(instruction.actual)
            || instruction.actual.pc != caller_pc
            || instruction.actual.opcode != opcode
            || instruction.actual.exception_depth != callee.entry.boundary.cpu.exception_depth
            || instruction.after_pc != callee.entry.boundary.cpu.pc()
            || instruction.after_sp != callee.entry.boundary.cpu.sp()
            || instruction.after_sr != callee.entry.boundary.cpu.sr()
            || instruction.before.cpu.sp().wrapping_sub(4) != instruction.after_sp
        {
            return Err(unsupported(
                UnsupportedKind::EntryBoundary,
                "actual call instruction/target/return-frame/lineage witness differs",
            ));
        }
        Ok(instruction)
    }
    fn require_read_mask(
        &self,
        issue: usize,
        access: Access,
        required: u16,
    ) -> Result<(), Failure> {
        let driven = self.original.trace.driven_masks.get(issue).ok_or_else(|| {
            unsupported(
                UnsupportedKind::Observation,
                "actual read has no bound driven-mask metadata",
            )
        })?;
        if required & !driven != 0 {
            return Err(unsupported(
                UnsupportedKind::Observation,
                format!(
                    "software read {:06x}/{} observes undefined bits: required={required:04x}, actual driven={driven:04x}; explicit source-backed read is unsupported in this profile",
                    access.address,
                    if access.word { 16 } else { 8 }
                ),
            ));
        }
        Ok(())
    }
    fn root_rts_read(&self, instruction: &Instruction, access: Access) -> bool {
        instruction.actual.opcode == 0x4e75
            && instruction.actual.completed_successfully
            && access.word
            && access.direction == Direction::Read(ReadPurpose::Semantic)
            && matches!(
                access
                    .address
                    .wrapping_sub(self.original.entry.boundary.cpu.sp() & 0xffffff),
                0 | 2
            )
    }
    fn root_control(&self, instruction: &Instruction) -> Result<(), Failure> {
        if !self.own_code.contains(&instruction.actual.pc) {
            return Err(unsupported(
                UnsupportedKind::TypedControlProfile,
                "own instruction/read owner PC is outside its committed function extent",
            ));
        }
        if (instruction.before.cpu.sr() ^ instruction.after_sr) & !0x1f != 0
            || super::ordinary_control_instruction(instruction.actual.opcode)
        {
            return Err(unsupported(
                UnsupportedKind::TypedControlProfile,
                "own privileged/system/bank/STOP/reset/RTE effect requires its explicit control profile",
            ));
        }
        Ok(())
    }
    pub fn peek(&mut self, include_optional: bool) -> Result<Token<'a>, Failure> {
        self.peek_with_frame_reads(include_optional, false)
    }
    pub fn peek_with_frame_reads(
        &mut self,
        include_optional: bool,
        frame_reads: bool,
    ) -> Result<Token<'a>, Failure> {
        if let Some(marker) = self.retained_abi {
            return Ok(Token::OptionalAbi(marker.access));
        }
        let original = self.original;
        loop {
            let Some(event) = original.trace.events.get(self.event) else {
                return Ok(Token::End);
            };
            match event {
                Event::Instruction(index) => {
                    let instruction = &original.trace.instructions[*index];
                    self.original_pc = Some(instruction.actual.pc);
                    self.original_opcode = Some(instruction.actual.opcode);
                    if !self.started {
                        self.root_control(instruction)?;
                        self.memory.begin_original(instruction)?;
                        self.issue = instruction.issues.start;
                        self.started = true;
                    }
                    while self.issue < instruction.issues.end {
                        let access = original.trace.issues[self.issue];
                        if access.direction == Direction::Read(ReadPurpose::Prefetch) {
                            if !access.word || access.address as usize >= self.rom_len {
                                return Err(unsupported(
                                    UnsupportedKind::Observation,
                                    "instruction traffic is not a certified immutable ROM word",
                                ));
                            }
                            self.issue += 1;
                            continue;
                        }
                        if self.root_rts_read(instruction, access) {
                            self.issue += 1;
                            continue;
                        }
                        if optional_abi(instruction, access, original.entry.boundary.cpu.sp()) {
                            if include_optional {
                                return Ok(Token::OptionalAbi(access));
                            }
                            self.retained_abi = Some(AbiDiscard {
                                access,
                                instruction: *index,
                                issue: self.issue,
                            });
                            self.issue += 1;
                            return Ok(Token::OptionalAbi(access));
                        }
                        if self.memory.original_private(access, instruction) {
                            match access.direction {
                                Direction::Write => self.memory.original_store(access),
                                Direction::Read(ReadPurpose::Semantic) => {
                                    let unwritten = self.memory.original_unwritten(access);
                                    if unwritten != 0 || frame_reads {
                                        return Ok(Token::FrameRead {
                                            access,
                                            unwritten,
                                            pc: instruction.actual.pc,
                                            opcode: instruction.actual.opcode,
                                        });
                                    }
                                }
                                Direction::Read(_) => {}
                            }
                            self.issue += 1;
                            continue;
                        }
                        return Ok(Token::Access {
                            access,
                            issue: self.issue,
                            pc: instruction.actual.pc,
                            opcode: instruction.actual.opcode,
                        });
                    }
                    self.own_chronology(instruction)?;
                    self.memory.end_original(instruction);
                    self.event += 1;
                    self.started = false;
                }
                Event::Idle(index) => {
                    self.environment_chronology(
                        &original.trace.chronology[original.trace.idles[*index].chronology.clone()],
                    )?;
                    self.event += 1;
                }
                Event::Interrupt(interrupt) => return Ok(Token::Interrupt(interrupt)),
                Event::Callee(callee) => return Ok(Token::Callee(callee)),
            }
        }
    }
    fn own_chronology(&mut self, instruction: &Instruction) -> Result<(), Failure> {
        let original = self.original;
        for &event in &original.trace.chronology[instruction.chronology.clone()] {
            match event {
                Chronology::CpuCommit { access, issue } => {
                    if self.memory.original_private(access, instruction) {
                        continue;
                    }
                    let actual = self
                        .matched_writes
                        .iter_mut()
                        .find_map(|slot| {
                            if slot.as_ref().is_some_and(|(index, _)| *index == issue) {
                                slot.take().map(|(_, access)| access)
                            } else {
                                None
                            }
                        })
                        .ok_or_else(|| {
                            Failure::Mismatch(format!(
                                "original commit at {:06x} has no matched Rust store",
                                access.address
                            ))
                        })?;
                    if actual != access {
                        return Err(Failure::Mismatch(
                            "matched issue/physical commit changed".into(),
                        ));
                    }
                    if access.ram() {
                        self.memory.commit_own(actual);
                    }
                }
                event => self.environment_chronology(std::slice::from_ref(&event))?,
            }
        }
        for slot in &mut self.matched_writes {
            if let Some((_, access)) = *slot {
                // The supported cartridge's immutable ROM ignores CPU stores;
                // their own issue still compared exactly above.
                if access.address < 0x400000 {
                    *slot = None;
                } else {
                    return Err(unsupported(
                        UnsupportedKind::Observation,
                        "matched non-ROM store lacks a physical commit",
                    ));
                }
            }
        }
        Ok(())
    }
    fn environment_chronology(&mut self, events: &[Chronology]) -> Result<(), Failure> {
        for &event in events {
            match event {
                Chronology::CpuCommit { access, .. }
                | Chronology::Z80Write(access)
                | Chronology::HostMutation(access) => {
                    if access.ram() {
                        self.memory.environment_write(access);
                    }
                }
                Chronology::DmaRead { address, value } => {
                    let access = Access {
                        address,
                        word: true,
                        direction: Direction::Read(ReadPurpose::Semantic),
                        value,
                    };
                    self.memory
                        .check_environment_read(access, tje_game::stack_read::Reader::VdpDma)?;
                    if access.ram() && self.memory.read_ram(access) != value {
                        return Err(Failure::Mismatch(format!(
                            "actual DMA observed different Rust RAM at {address:06x}"
                        )));
                    }
                }
                Chronology::CpuRead { .. }
                | Chronology::Exception(_)
                | Chronology::Acknowledge { .. } => {}
            }
        }
        Ok(())
    }
    fn import_environment(&mut self, invocation: &Invocation) -> Result<(), Failure> {
        let mut private = super::frame::Written::default();
        self.import_frame_environment(invocation, &mut Vec::new(), &mut private)?;
        // Internal justified saves/restores can read their own writes. Only
        // after the environment returns do its unproved private bytes become
        // opaque to the caller, subsequent IRQs, calls and DMA.
        for address in private.addresses() {
            self.memory.mark_private_residue(address);
        }
        Ok(())
    }
    fn import_chronology(
        &mut self,
        events: &[Chronology],
        frames: &[std::ops::Range<u32>],
        private: &mut super::frame::Written,
    ) -> Result<(), Failure> {
        self.environment_chronology(events)?;
        for &event in events {
            let (access, cpu) = match event {
                Chronology::CpuCommit { access, .. } => (access, true),
                Chronology::Z80Write(access) | Chronology::HostMutation(access) => (access, false),
                _ => continue,
            };
            if access.ram() {
                for (address, _) in access.bytes() {
                    let address = crate::abi::canonical_ram_address(address);
                    if cpu && frames.iter().any(|frame| frame.contains(&address)) {
                        private.set(address);
                    } else {
                        private.clear(address, address + 1);
                    }
                }
            }
        }
        Ok(())
    }
    fn import_frame_environment(
        &mut self,
        invocation: &Invocation,
        frames: &mut Vec<std::ops::Range<u32>>,
        private: &mut super::frame::Written,
    ) -> Result<(), Failure> {
        let high = crate::abi::canonical_ram_address(invocation.entry.boundary.cpu.sp());
        let hardware = matches!(
            invocation.entry.kind,
            super::record::EntryKind::Interrupt { .. }
        );
        if hardware {
            let Some(Event::Idle(index)) = invocation.trace.events.first() else {
                return Err(unsupported(
                    UnsupportedKind::Observation,
                    "actual IRQ has no hardware-entry chronology",
                ));
            };
            let before = &invocation.trace.idles[*index].before;
            let top = crate::abi::canonical_ram_address(before.cpu.sp());
            if before.cpu.sr() & 0x2000 == 0 || top != high + 6 {
                return Err(unsupported(
                    UnsupportedKind::TypedControlProfile,
                    "IRQ bank/frame transport requires its explicit control profile",
                ));
            }
            frames.push(high..top);
        }
        frames.push(high..high);
        for event in &invocation.trace.events {
            match event {
                Event::Instruction(index) => {
                    let instruction = &invocation.trace.instructions[*index];
                    frames.last_mut().expect("actual environment frame").start =
                        crate::abi::canonical_ram_address(instruction.before.cpu.sp())
                            .min(crate::abi::canonical_ram_address(instruction.after_sp))
                            .min(high);
                    self.import_chronology(
                        &invocation.trace.chronology[instruction.chronology.clone()],
                        frames,
                        private,
                    )?;
                    frames.last_mut().expect("actual environment frame").start =
                        crate::abi::canonical_ram_address(instruction.after_sp).min(high);
                }
                Event::Idle(index) => self.import_chronology(
                    &invocation.trace.chronology[invocation.trace.idles[*index].chronology.clone()],
                    frames,
                    private,
                )?,
                Event::Callee(child) | Event::Interrupt(child) => {
                    self.import_frame_environment(child, frames, private)?
                }
            }
        }
        frames.pop().expect("actual environment frame closes");
        if hardware {
            frames.pop().expect("actual hardware frame closes");
        }
        Ok(())
    }
    pub fn take_interrupt(&mut self, interrupt: &Invocation) -> Result<(), Failure> {
        match self.peek(false)? {
            Token::Interrupt(expected) if expected.entry.identity == interrupt.entry.identity => {
                self.import_environment(interrupt)?;
                self.event += 1;
                Ok(())
            }
            _ => Err(Failure::Mismatch(
                "IRQ is not the next exact environment invocation".into(),
            )),
        }
    }
    pub fn take_callee(&mut self, callee: &Invocation) -> Result<(), Failure> {
        match self.peek(false)? {
            Token::Callee(expected) if expected.entry.identity == callee.entry.identity => {
                self.import_environment(callee)?;
                self.memory.original_stack(callee.exit.cpu.sp());
                self.event += 1;
                Ok(())
            }
            _ => Err(Failure::Mismatch(
                "modular callee segment is not the next actual call".into(),
            )),
        }
    }
    pub fn read(&mut self, address: u32, word: bool) -> Result<u16, Failure> {
        self.read_for_predicate(address, word, if word { 0xffff } else { 0xff })
    }
    /// Only the dedicated wait implementation may request fewer consumed bits.
    /// The raw beat is retained for predicate authentication, never returned to
    /// software through Bus or Platform.
    pub fn read_for_predicate(
        &mut self,
        address: u32,
        word: bool,
        required: u16,
    ) -> Result<u16, Failure> {
        let purpose = ReadPurpose::Semantic;
        let requested = Access {
            address: address & 0xffffff,
            word,
            direction: Direction::Read(purpose),
            value: 0,
        };
        let token = self.peek(false)?;
        self.memory.check_observed(requested)?;
        self.memory.check_initialized(requested)?;
        if self.memory.contains(requested)
            && !matches!(token, Token::Access { access, .. } | Token::FrameRead { access, .. }
                if access.address == requested.address && access.word == word && access.direction == requested.direction)
            && let Some(value) = self.memory.initialized_read(requested)
        {
            return Ok(value);
        }
        match token {
            Token::Access { access, .. }
                if access.address == requested.address
                    && access.word == word
                    && access.direction == requested.direction =>
            {
                self.require_read_mask(self.issue, access, required)?;
                if access.ram() && self.memory.read_ram(access) != access.value {
                    let actual = self.memory.read_ram(access);
                    return Err(self.compared(
                        "own_ram_sample",
                        format!(
                            "own RAM sample differs at {:06x}: original={:04x}, Rust={actual:04x}",
                            access.address, access.value
                        ),
                        access_json(access, true),
                        access_json(
                            Access {
                                value: actual,
                                ..access
                            },
                            true,
                        ),
                    ));
                }
                self.issue += 1;
                self.own_accesses += 1;
                Ok(access.value)
            }
            Token::FrameRead { .. } => Err(unsupported(
                UnsupportedKind::Frame,
                "generic reads never import an original uninitialized-local sample; use the explicit own-frame API",
            )),
            other => Err(self.compared(
                "own_read_order",
                format!(
                    "extra/reordered own read: address={:06x}, word={word}, purpose={purpose:?}",
                    requested.address
                ),
                token_json(other),
                access_json(requested, false),
            )),
        }
    }
    /// Explicit bug intent still requires the next actual own-frame read.
    /// A fully initialized beat is permitted only as part of a mixed-width
    /// request whose caller has already required at least one unwritten lane.
    pub fn read_uninitialized(&mut self, address: u32, word: bool) -> Result<u16, Failure> {
        let requested = Access {
            address: address & 0xffffff,
            word,
            direction: Direction::Read(ReadPurpose::Semantic),
            value: 0,
        };
        let token = self.peek_with_frame_reads(false, true)?;
        self.memory.check_observed(requested)?;
        match token {
            Token::FrameRead {
                access, unwritten, ..
            } if access.address == requested.address
                && access.word == word
                && access.direction == requested.direction =>
            {
                self.require_read_mask(self.issue, access, if word { 0xffff } else { 0xff })?;
                let value = self.memory.uninitialized_read(access, unwritten)?;
                self.issue += 1;
                self.own_accesses += 1;
                Ok(value)
            }
            other => Err(self.compared(
                "explicit_uninitialized_read",
                "explicit local read has no next exact original own-frame semantic read".into(),
                token_json(other),
                access_json(requested, false),
            )),
        }
    }
    pub fn read_uninitialized_long(&mut self, address: u32) -> Result<u32, Failure> {
        let address = address & 0xffffff;
        let token = self.peek_with_frame_reads(false, true)?;
        let event = self.event;
        let high = if matches!(token, Token::FrameRead { access, .. }
            if access.address == address.wrapping_add(2) && access.word
                && access.direction == Direction::Read(ReadPurpose::Semantic))
        {
            let Some(Event::Instruction(index)) = self.original.trace.events.get(event) else {
                return Err(unsupported(
                    UnsupportedKind::Observation,
                    "mixed LONG read has no actual instruction",
                ));
            };
            let instruction = &self.original.trace.instructions[*index];
            if self.issue <= instruction.issues.start {
                return Err(unsupported(
                    UnsupportedKind::Observation,
                    "mixed LONG read has no preceding original WORD",
                ));
            }
            let issue = self.issue - 1;
            let prior = self.original.trace.issues[issue];
            if prior.address != address
                || !prior.word
                || prior.direction != Direction::Read(ReadPurpose::Semantic)
                || !self.memory.original_private(prior, instruction)
                || self.memory.original_unwritten(prior) != 0
            {
                return Err(unsupported(
                    UnsupportedKind::Observation,
                    "mixed LONG high WORD is not the same-instruction skipped initialized read",
                ));
            }
            self.memory.check_observed(prior)?;
            self.require_read_mask(issue, prior, 0xffff)?;
            let value = self.memory.uninitialized_read(prior, 0)?;
            self.own_accesses += 1;
            value
        } else {
            self.read_uninitialized(address, true)?
        };
        let low = self.read_uninitialized(address.wrapping_add(2), true)?;
        if self.event != event {
            return Err(unsupported(
                UnsupportedKind::Observation,
                "explicit LONG words crossed an original instruction/frontier",
            ));
        }
        Ok((u32::from(high) << 16) | u32::from(low))
    }
    pub fn discard(&mut self, address: u32, word: bool) -> Result<(), Failure> {
        let requested = Access {
            address: address & 0xffffff,
            word,
            direction: Direction::Read(ReadPurpose::Discarded),
            value: 0,
        };
        if let Some(marker) = self.retained_abi {
            self.memory.check_observed(requested)?;
            let source = &self.original.trace.instructions[marker.instruction];
            if self.original.trace.issues.get(marker.issue) != Some(&marker.access)
                || !optional_abi(source, marker.access, self.original.entry.boundary.cpu.sp())
            {
                return Err(unsupported(
                    UnsupportedKind::Observation,
                    "retained ABI discard lost its exact original occurrence",
                ));
            }
            if marker.access.address != requested.address
                || marker.access.word != word
                || marker.access.direction != requested.direction
            {
                return Err(self.compared(
                    "discarded_abi_occurrence",
                    "discard does not match the retained original MOVEM terminal occurrence".into(),
                    access_json(marker.access, true),
                    access_json(requested, false),
                ));
            }
            self.original_pc = Some(source.actual.pc);
            self.original_opcode = Some(source.actual.opcode);
            self.retained_abi = None;
            return Ok(());
        }
        let token = self.peek(true)?;
        self.memory.check_observed(requested)?;
        if self.memory.contains(requested)
            && !matches!(token, Token::Access { access, .. }
                if access.address == requested.address && access.word == word && access.direction == requested.direction)
        {
            return Ok(());
        }
        match token {
            Token::OptionalAbi(access)
                if access.address == requested.address
                    && access.word == word
                    && access.direction == requested.direction =>
            {
                self.issue += 1;
                Ok(())
            }
            Token::Access { access, .. }
                if access.address == requested.address
                    && access.word == word
                    && access.direction == requested.direction =>
            {
                if access.ram() && self.memory.read_ram(access) != access.value {
                    return Err(Failure::Mismatch(format!(
                        "discarded RAM sample differs at {:06x}",
                        access.address
                    )));
                }
                self.issue += 1;
                self.own_accesses += 1;
                Ok(())
            }
            _ => Err(Failure::Mismatch(format!(
                "extra/reordered discarded read: address={:06x}, word={word}",
                requested.address
            ))),
        }
    }
    pub fn write(&mut self, address: u32, word: bool, value: u16) -> Result<(), Failure> {
        let access = Access {
            address: address & 0xffffff,
            word,
            direction: Direction::Write,
            value,
        };
        if self.memory.contains(access) {
            self.memory.write_private(access);
            return Ok(());
        }
        match self.peek(false)? {
            Token::Access {
                access: expected,
                issue,
                ..
            } if expected == access => {
                let slot = self
                    .matched_writes
                    .iter_mut()
                    .find(|slot| slot.is_none())
                    .ok_or_else(|| {
                        unsupported(
                            UnsupportedKind::Observation,
                            "one opcode exceeded the 68000 MOVEM store capacity",
                        )
                    })?;
                *slot = Some((issue, access));
                self.issue += 1;
                self.own_accesses += 1;
                Ok(())
            }
            other => Err(self.compared(
                "own_write_order",
                format!("dropped/changed/reordered own store: {access:?}"),
                token_json(other),
                access_json(access, true),
            )),
        }
    }
    pub fn finish(&mut self, exit_ram: &[u8; 65536]) -> Result<(), Failure> {
        let mut remaining = self.peek(false)?;
        while matches!(remaining, Token::OptionalAbi(_)) {
            self.elide_retained_abi();
            remaining = self.peek(false)?;
        }
        if !matches!(remaining, Token::End) {
            return Err(self.compared(
                "unconsumed_own_effect",
                "Rust returned with an unconsumed original own access, frame input or callee"
                    .into(),
                token_json(remaining),
                serde_json::json!({"returned":true}),
            ));
        }
        let (low, high) = self.memory.private_bounds();
        for (index, (&expected, &actual)) in
            exit_ram.iter().zip(self.memory.ram().iter()).enumerate()
        {
            let address = 0xff0000 + index as u32;
            if !(low..high).contains(&address) && expected != actual {
                return Err(self.compared(
                    "return_ram",
                    format!("return RAM {address:06x}: original={expected:02x}, Rust={actual:02x}"),
                    serde_json::json!({"address":address,"value":expected}),
                    serde_json::json!({"address":address,"value":actual}),
                ));
            }
        }
        Ok(())
    }
    pub fn instruction_span(&self) -> [Option<&'a Instruction>; 4] {
        let mut result = [None; 4];
        let mut count = 0;
        for event in self.original.trace.events.iter().skip(self.event) {
            match event {
                Event::Instruction(index) => {
                    result[count] = Some(&self.original.trace.instructions[*index]);
                    count += 1;
                    if count == result.len() {
                        break;
                    }
                }
                Event::Idle(_) | Event::Interrupt(_) => {}
                Event::Callee(_) => break,
            }
        }
        result
    }
}
