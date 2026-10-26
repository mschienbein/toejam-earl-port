//! Frame-private traffic is causal, not an oracle copy. Original allocation
//! ownership is evaluated at each actual own instruction; IRQ/callee frames do
//! not enlarge the root's region. Outside-frame accesses remain cursor-exact.
use super::{
    Failure, Unsupported, UnsupportedKind,
    record::{Access, Direction, Instruction, Invocation},
};
use crate::{
    abi::canonical_ram_address,
    catalogue::{STACK_BOTTOM, STACK_TOP},
};

pub(super) use tje_mem::written_ram::WrittenRam as Written;

pub(super) struct FrameMemory {
    root: tje_game::FunctionId,
    entry_sp: u32,
    ram: Box<[u8; 65536]>,
    shadow: Box<[u8; 65536]>,
    low: u32,
    high: u32,
    original_sp: u32,
    rust_sp: u32,
    original_written: Written,
    original_history: Written,
    rust_written: Written,
    rust_history: Written,
    opaque_callee_private: Written,
    environment_written: Written,
}
fn unsupported(detail: impl Into<String>) -> Failure {
    Failure::Unsupported(Unsupported {
        kind: UnsupportedKind::Frame,
        detail: detail.into(),
    })
}
fn address(address: u32) -> u32 {
    canonical_ram_address(address)
}
fn value(access: Access, mut byte: impl FnMut(u32) -> u8) -> u16 {
    if access.word {
        u16::from_be_bytes([byte(access.address), byte(access.address.wrapping_add(1))])
    } else {
        u16::from(byte(access.address))
    }
}
pub(super) fn private_access(entry_sp: u32, instruction: &Instruction, access: Access) -> bool {
    let high = address(entry_sp);
    let low = address(instruction.before.cpu.sp())
        .min(address(instruction.after_sp))
        .min(high);
    access.ram()
        && access
            .bytes()
            .all(|(a, _)| (low..high).contains(&address(a)))
}
impl FrameMemory {
    pub fn new(
        invocation: &Invocation,
        ram: Box<[u8; 65536]>,
        poison: u8,
    ) -> Result<Self, Failure> {
        let root = invocation
            .entry
            .function
            .ok_or_else(|| unsupported("unclassified root has no owned frame"))?;
        let high = address(invocation.entry.boundary.cpu.sp());
        let mut low = high;
        for instruction in &invocation.trace.instructions {
            for raw in [instruction.before.cpu.sp(), instruction.after_sp] {
                let physical = address(raw);
                if !(STACK_BOTTOM..=STACK_TOP).contains(&physical) {
                    return Err(unsupported(
                        "own stack movement leaves the reviewed RAM stack domain",
                    ));
                }
                if physical > high && !(instruction.actual.opcode == 0x4e75 && physical == high + 4)
                {
                    return Err(unsupported(
                        "own stack movement abandons its ordinary frame",
                    ));
                }
                low = low.min(physical);
            }
        }
        let mut shadow = ram.clone();
        for physical in low..high {
            shadow[(physical & 65535) as usize] ^= poison;
        }
        Ok(Self {
            root,
            entry_sp: invocation.entry.boundary.cpu.sp(),
            ram,
            shadow,
            low,
            high,
            original_sp: high,
            rust_sp: high,
            original_written: Written::default(),
            original_history: Written::default(),
            rust_written: Written::default(),
            rust_history: Written::default(),
            environment_written: Written::default(),
            opaque_callee_private: Written::default(),
        })
    }
    pub fn contains(&self, access: Access) -> bool {
        access.ram()
            && access
                .bytes()
                .all(|(a, _)| (self.low..self.high).contains(&address(a)))
    }
    pub fn original_private(&self, access: Access, instruction: &Instruction) -> bool {
        private_access(self.high, instruction, access)
    }
    pub fn begin_original(&mut self, instruction: &Instruction) -> Result<(), Failure> {
        let before = address(instruction.before.cpu.sp());
        if before != self.original_sp {
            return Err(unsupported(
                "own frame allocation lineage differs from its actual call/return history",
            ));
        }
        let after = address(instruction.after_sp).min(self.high);
        self.original_sp = before.min(after);
        Ok(())
    }
    pub fn end_original(&mut self, instruction: &Instruction) {
        self.original_stack(address(instruction.after_sp).min(self.high));
    }
    pub fn original_stack(&mut self, sp: u32) {
        let sp = address(sp).min(self.high);
        if sp > self.original_sp {
            self.original_written.clear(self.original_sp, sp);
        }
        self.original_sp = sp;
    }
    pub fn rust_stack(&mut self, sp: u32) -> Result<(), Failure> {
        let physical = address(sp);
        if !(self.low..=self.high).contains(&physical) {
            return Err(unsupported(
                "Rust private frame exceeds the actual root's own frame region",
            ));
        }
        if physical > self.rust_sp {
            self.rust_written.clear(self.rust_sp, physical);
        }
        self.rust_sp = physical;
        Ok(())
    }
    pub fn original_store(&mut self, access: Access) {
        for (a, _) in access.bytes() {
            self.original_written.set(a);
            self.original_history.set(a);
        }
    }
    pub fn original_unwritten(&self, access: Access) -> u8 {
        access.bytes().enumerate().fold(0, |mask, (index, (a, _))| {
            mask | (u8::from(!self.original_written.get(a)) << index)
        })
    }
    pub fn rust_unwritten(&self, access: Access) -> u8 {
        access.bytes().enumerate().fold(0, |mask, (index, (a, _))| {
            mask | (u8::from(!self.rust_written.get(a)) << index)
        })
    }
    pub fn write_private(&mut self, access: Access) {
        for (a, byte) in access.bytes() {
            self.shadow[(a & 65535) as usize] = byte;
            self.opaque_callee_private.clear(a, a + 1);
            self.ram[(a & 65535) as usize] = byte;
            self.rust_written.set(a);
            self.rust_history.set(a);
        }
    }
    pub fn initialized_read(&self, access: Access) -> Option<u16> {
        (self.rust_unwritten(access) == 0)
            .then(|| value(access, |a| self.shadow[(a & 65535) as usize]))
    }
    pub fn explicit_unwritten_mask(
        &self,
        physical: u32,
        bytes: u8,
    ) -> Result<u8, tje_game::stack_read::UnsupportedReason> {
        let mut mask = 0;
        for lane in 0..bytes {
            let physical = physical.wrapping_add(u32::from(lane));
            let a = address(physical);
            if !(self.original_sp..self.high).contains(&a)
                || !(self.rust_sp..self.high).contains(&a)
            {
                return Err(tje_game::stack_read::UnsupportedReason::InvalidExplicitLocal);
            }
            if self.environment_written.get(physical) || self.opaque_callee_private.get(physical) {
                return Err(tje_game::stack_read::UnsupportedReason::InterveningNonOwnWrite);
            }
            if self.original_history.get(physical) != self.rust_history.get(physical) {
                return Err(tje_game::stack_read::UnsupportedReason::PairedInputMismatch);
            }
            mask |= u8::from(!self.rust_written.get(physical)) << lane;
        }
        if mask == 0 {
            return Err(tje_game::stack_read::UnsupportedReason::AlreadyWritten);
        }
        Ok(mask)
    }
    pub fn uninitialized_read(
        &self,
        access: Access,
        original_unwritten: u8,
    ) -> Result<u16, Failure> {
        if access
            .bytes()
            .any(|(a, _)| self.environment_written.get(a) || self.opaque_callee_private.get(a))
        {
            return Err(Failure::StackRead(tje_game::stack_read::Unsupported {
                root: self.root,
                entry_sp: self.entry_sp,
                raw_address: access.address & tje_mem::ADDRESS_MASK,
                address: address(access.address),
                width: if access.word { 2 } else { 1 },
                reader: tje_game::stack_read::Reader::ExplicitLocal,
                reason: tje_game::stack_read::UnsupportedReason::InterveningNonOwnWrite,
            }));
        }
        if self.rust_unwritten(access) != original_unwritten {
            return Err(Failure::Mismatch(format!(
                "frame read at {:06x} has a missing/extra Rust store; original never-written mask={original_unwritten:#x}, Rust={:#x}",
                access.address,
                self.rust_unwritten(access)
            )));
        }
        for (index, (a, expected)) in access.bytes().enumerate() {
            let original_history = self.original_history.get(a);
            let rust_history = self.rust_history.get(a);
            if original_history != rust_history {
                return Err(Failure::Mismatch(format!(
                    "explicit local at {a:06x} cannot erase an earlier-lifetime missing/extra Rust store"
                )));
            }
            if (original_unwritten & (1 << index) == 0 || original_history)
                && self.shadow[(a & 65535) as usize] != expected
            {
                return Err(Failure::Mismatch(format!(
                    "explicit mixed local read would repair a wrong Rust store at {a:06x}"
                )));
            }
        }
        // Only lanes never written anywhere in this own-root history supply
        // entry input. Reallocated old stores remain Rust-owned causal values.
        // Neither path changes initialization/history/proof-status bits.
        let sampled = access.value.to_be_bytes();
        Ok(value(access, |a| {
            let index = a.wrapping_sub(access.address) as usize;
            if original_unwritten & (1 << index) != 0 && !self.original_history.get(a) {
                sampled[if access.word { index } else { 1 }]
            } else {
                self.shadow[(a & 65535) as usize]
            }
        }))
    }
    /// Snapshot only Rust-owned input data at this call boundary. An unavailable
    /// byte remains an error even if a later call/environment overwrites it.
    pub fn call_input_byte(&self, physical: u32) -> Result<u8, Failure> {
        let access = Access {
            address: physical,
            word: false,
            direction: Direction::Read(m68000_emu::ReadPurpose::Semantic),
            value: 0,
        };
        self.check_observed(access)?;
        self.check_initialized(access)?;
        let canonical = address(physical);
        if (self.original_sp..self.high).contains(&canonical) {
            if !self.rust_written.get(physical) {
                return Err(Failure::Mismatch(format!(
                    "callee consumes frame byte {physical:06x} without a Rust store"
                )));
            }
            Ok(self.shadow[(physical & 65535) as usize])
        } else {
            if self.original_history.get(physical)
                && !self.rust_history.get(physical)
                && !self.environment_written.get(physical)
            {
                return Err(Failure::Mismatch(format!(
                    "callee observes old root-frame residue {physical:06x} whose original store Rust omitted"
                )));
            }
            Ok(self.ram[(physical & 65535) as usize])
        }
    }
    pub fn check_call_byte(&self, physical: u32, expected: u8) -> Result<(), Failure> {
        self.check_call_bits(physical, expected, u8::MAX)
    }
    pub fn check_call_bits(&self, physical: u32, expected: u8, bits: u8) -> Result<(), Failure> {
        let actual = self.call_input_byte(physical)?;
        if (actual ^ expected) & bits != 0 {
            return Err(Failure::Mismatch(format!(
                "callee RAM/frame input {physical:06x}/{bits:02x}: original={:02x}, Rust={:02x}",
                expected & bits,
                actual & bits
            )));
        }
        Ok(())
    }
    pub fn read_ram(&self, access: Access) -> u16 {
        value(access, |a| self.ram[(a & 65535) as usize])
    }
    pub fn commit_own(&mut self, access: Access) {
        debug_assert_eq!(access.direction, Direction::Write);
        for (a, byte) in access.bytes() {
            self.ram[(a & 65535) as usize] = byte;
            self.rust_history.set(a);
            self.opaque_callee_private.clear(a, a + 1);
        }
    }
    pub fn environment_write(&mut self, access: Access) {
        for (a, byte) in access.bytes() {
            self.ram[(a & 65535) as usize] = byte;
            self.environment_written.set(a);
            self.opaque_callee_private.clear(a, a + 1);
            if (self.original_sp..self.high).contains(&address(a)) {
                self.shadow[(a & 65535) as usize] = byte;
                self.rust_written.set(a);
                self.original_written.set(a);
            }
        }
    }
    pub fn mark_private_residue(&mut self, address: u32) {
        self.opaque_callee_private.set(address);
    }
    pub fn check_observed(&self, access: Access) -> Result<(), Failure> {
        if access.ram()
            && access
                .bytes()
                .any(|(a, _)| self.opaque_callee_private.get(a))
        {
            return Err(unsupported(format!(
                "caller observes substituted callee/ISR-private residue at {:06x}/{} without an independent residue-output proof; no original stack value is imported",
                access.address,
                if access.word { 16 } else { 8 }
            )));
        }
        Ok(())
    }
    fn unwritten(&self, access: Access, reader: tje_game::stack_read::Reader) -> Failure {
        Failure::StackRead(tje_game::stack_read::Unsupported {
            root: self.root,
            entry_sp: self.entry_sp,
            raw_address: access.address & tje_mem::ADDRESS_MASK,
            address: address(access.address),
            width: if access.word { 2 } else { 1 },
            reader,
            reason: tje_game::stack_read::UnsupportedReason::Unwritten,
        })
    }
    fn check_history(
        &self,
        access: Access,
        reader: tje_game::stack_read::Reader,
    ) -> Result<(), Failure> {
        if access.ram()
            && access.bytes().any(|(a, _)| {
                address(a) < self.high
                    && !self.rust_history.get(a)
                    && !self.environment_written.get(a)
            })
        {
            return Err(self.unwritten(access, reader));
        }
        Ok(())
    }
    pub fn check_initialized(&self, access: Access) -> Result<(), Failure> {
        self.check_history(access, tje_game::stack_read::Reader::RustMemory)?;
        if self.contains(access) && self.rust_unwritten(access) != 0 {
            return Err(self.unwritten(access, tje_game::stack_read::Reader::RustMemory));
        }
        Ok(())
    }
    pub fn check_environment_read(
        &self,
        access: Access,
        reader: tje_game::stack_read::Reader,
    ) -> Result<(), Failure> {
        self.check_observed(access)?;
        self.check_history(access, reader)?;
        if access.ram()
            && access
                .bytes()
                .any(|(a, _)| (self.original_sp..self.high).contains(&address(a)))
        {
            return Err(unsupported(
                "IRQ/DMA observes the root's private frame outside an authenticated callee boundary; an explicit residue/control proof is required",
            ));
        }
        Ok(())
    }
    pub fn clear_temporary(&mut self, low: u32, high: u32) {
        self.rust_written.clear(address(low), address(high));
    }
    pub fn ram(&self) -> &[u8; 65536] {
        &self.ram
    }
    pub fn private_bounds(&self) -> (u32, u32) {
        (self.low, self.high)
    }
    pub fn rust_sp(&self) -> u32 {
        self.rust_sp
    }
}
