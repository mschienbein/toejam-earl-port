//! ROM-independent 68000 compiler front end. Execution belongs to compiled backends.
//! The instruction evaluator is deliberately a separate, development-only crate.

pub mod decode;
pub mod ir;
pub mod lower;
pub mod metadata;
pub mod timing;
pub mod timing_region;

#[cfg(test)]
mod tests;

pub use ir::{InstructionIr, Program};
pub use lower::{CompileError, compile_instruction, compile_instruction_with_divider};

/// Canonical state shared by both compiled backends and the scheduler.
/// A7 is selected from USP/SSP by SR.S, never stored in `a`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[repr(C)]
pub struct CpuState {
    pub d: [u32; 8],
    pub a: [u32; 7],
    pub usp: u32,
    pub ssp: u32,
    pub pc: u32,
    pub sr: u32,
    pub prefetch: u32,
    pub pending_interrupt: u32,
    pub stopped: u32,
    pub frozen: u32,
    pub address_error: u32,
    pub last_muldiv: u32,
}

impl CpuState {
    pub const SR_MASK: u32 = 0xA71F;
    pub fn read_address(&self, register: u8) -> u32 {
        match register {
            0..=6 => self.a[usize::from(register)],
            7 if self.sr & 0x2000 != 0 => self.ssp,
            7 => self.usp,
            _ => panic!("invalid address register"),
        }
    }
    pub fn write_address(&mut self, register: u8, value: u32) {
        match register {
            0..=6 => self.a[usize::from(register)] = value,
            7 if self.sr & 0x2000 != 0 => self.ssp = value,
            7 => self.usp = value,
            _ => panic!("invalid address register"),
        }
    }
    pub fn set_sr(&mut self, value: u32) {
        self.sr = value & Self::SR_MASK;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ReadKind {
    Semantic = 0,
    Prefetch = 1,
    Discarded = 2,
}

/// Bus accesses occur in IR order. Writes may be deferred by the device layer
/// until its instruction step has advanced; the compiler never coalesces them.
pub trait Bus {
    fn read8(&mut self, address: u32, kind: ReadKind) -> u8;
    fn read16(&mut self, address: u32, kind: ReadKind) -> u16;
    fn write8(&mut self, address: u32, value: u8);
    fn write16(&mut self, address: u32, value: u16);
    fn set_read_offset_mclk(&mut self, offset: u32);
}

/// Native ABI: the continuation returns cycles in bits 0..31 and vector in
/// bits 32..39, and writes this record when vector != 0. All cycle charges are
/// complete base charges, including exception entry (not hardware bus stalls).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct StepResult {
    pub cycles: u32,
    pub vector: u32,
    pub saved_pc: u32,
    pub fault_address: u32,
    pub fault_status: u32,
}
impl StepResult {
    pub const fn normal(cycles: u32) -> Self {
        Self {
            cycles,
            vector: 0,
            saved_pc: 0,
            fault_address: 0,
            fault_status: 0,
        }
    }
    pub const fn packed(self) -> u64 {
        self.cycles as u64 | ((self.vector as u64) << 32)
    }
}

/// Exception side channel for cached relocation-free native continuations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct ExceptionOut {
    pub saved_pc: u32,
    pub fault_address: u32,
    pub status_word: u32,
}
