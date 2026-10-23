//! MCC68K C ABI: caller-owned 32-bit argument slots, D0 result, D2-D7 and
//! A2-A6 preserved. Short/byte arguments occupy the low part of their slot.
//! Differential calls are atomic: peripheral clocks are frozen and interrupts
//! masked on both forks; the live check-mode machine retains original timing.

use crate::{
    HostError, Result,
    catalogue::{STACK_BOTTOM, STACK_TOP},
};
use genesis_core::{GenesisBus, GenesisEmulator};
use m68000_emu::{BusInterface, M68000, debug::DummyM68000Debugger};
use serde::Serialize;
use std::collections::BTreeSet;

pub const CALL_SENTINEL: u32 = 0x003F_FFFE;
const MAX_CALL_INSTRUCTIONS: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct UninitializedStackRead {
    pub pc: u32,
    pub address: u32,
}

#[derive(Debug, Default)]
pub(crate) struct AtomicRun {
    pub instructions: u64,
    pub cycles: u64,
    pub uninitialized_reads: BTreeSet<UninitializedStackRead>,
}

/// CPU bus observer. Reading an old local before writing it is significant even
/// when that byte lies in the normally ignored dead-stack comparison window.
struct StackAuditBus<'a> {
    inner: &'a mut GenesisBus,
    entry_sp: u32,
    pc: u32,
    written: [u64; 512],
    reads: BTreeSet<UninitializedStackRead>,
}

impl StackAuditBus<'_> {
    fn stack_offset(&self, address: u32) -> Option<usize> {
        let address = address & 0xFF_FFFF;
        (STACK_BOTTOM..self.entry_sp.min(STACK_TOP))
            .contains(&address)
            .then_some(address.wrapping_sub(STACK_BOTTOM) as usize)
    }

    fn read(&mut self, address: u32) {
        if let Some(offset) = self.stack_offset(address)
            && self.written[offset / 64] & (1 << (offset % 64)) == 0
        {
            self.reads.insert(UninitializedStackRead {
                pc: self.pc,
                address: address & 0xFF_FFFF,
            });
        }
    }

    fn write(&mut self, address: u32) {
        if let Some(offset) = self.stack_offset(address) {
            self.written[offset / 64] |= 1 << (offset % 64);
        }
    }
}

impl BusInterface for StackAuditBus<'_> {
    type DebugView<'a>
        = DummyM68000Debugger
    where
        Self: 'a;
    fn read_byte(&mut self, address: u32) -> u8 {
        self.read(address);
        self.inner.read_byte(address)
    }
    fn read_word(&mut self, address: u32) -> u16 {
        self.read(address);
        self.read(address.wrapping_add(1));
        self.inner.read_word(address)
    }
    fn write_byte(&mut self, address: u32, value: u8) {
        self.write(address);
        self.inner.write_byte(address, value);
    }
    fn write_word(&mut self, address: u32, value: u16) {
        self.write(address);
        self.write(address.wrapping_add(1));
        self.inner.write_word(address, value);
    }
    fn interrupt_level(&self) -> u8 {
        0
    }
    fn acknowledge_interrupt(&mut self, level: u8) {
        self.inner.acknowledge_interrupt(level);
    }
    fn halt(&self) -> bool {
        self.inner.halt()
    }
    fn reset(&self) -> bool {
        <GenesisBus as BusInterface>::reset(self.inner)
    }
}

pub(crate) fn run_original_atomic(
    emulator: &mut GenesisEmulator,
    return_pc: u32,
    return_sp: u32,
) -> Result<AtomicRun> {
    let (cpu, bus) = emulator.hybrid_parts();
    let mut audit = StackAuditBus {
        inner: bus,
        entry_sp: cpu.stack_pointer() & 0xFF_FFFF,
        pc: cpu.pc(),
        written: [0; 512],
        reads: BTreeSet::new(),
    };
    let mut result = AtomicRun::default();
    while cpu.pc() != return_pc || cpu.stack_pointer() != return_sp {
        if result.instructions == MAX_CALL_INSTRUCTIONS {
            return Err(HostError::CallDidNotReturn {
                pc: cpu.pc(),
                return_pc,
            });
        }
        audit.pc = cpu.pc();
        result.cycles += u64::from(cpu.execute_instruction(&mut audit));
        audit.inner.apply_pending_writes();
        result.instructions += 1;
    }
    result.uninitialized_reads = audit.reads;
    Ok(result)
}

pub(crate) fn set_sp(cpu: &mut M68000, value: u32) {
    let (mut usp, mut ssp) = (cpu.user_stack_pointer(), cpu.supervisor_stack_pointer());
    if cpu.status_register() & 0x2000 != 0 {
        ssp = value;
    } else {
        usp = value;
    }
    cpu.set_address_registers(cpu.address_registers(), usp, ssp);
}

/// A Rust translation sees the actual emulated address space and stack arguments.
/// It keeps its own locals in Rust; it never recreates the compiler's save area.
pub struct HookContext<'a> {
    emulator: &'a mut GenesisEmulator,
    pub nested_calls: u64,
}

impl<'a> HookContext<'a> {
    pub(crate) fn new(emulator: &'a mut GenesisEmulator) -> Self {
        Self {
            emulator,
            nested_calls: 0,
        }
    }

    /// Side-effect-free ROM/RAM read. For hardware reads use `bus()` so that
    /// read side effects and open-bus behavior remain explicit.
    pub fn read_u8(&self, address: u32) -> u8 {
        let address = address & 0xFF_FFFF;
        let bus = self.emulator.hybrid_bus();
        match address {
            0..=0x3F_FFFF => bus
                .cartridge
                .as_ref()
                .expect("cartridge")
                .peek_word(address)
                .to_be_bytes()[(address & 1) as usize],
            0xE00000..=0xFFFFFF => bus.memory.read_main_ram_byte(address),
            _ => panic!("read_u8 is ROM/RAM-only; use bus() for hardware address {address:06X}"),
        }
    }
    pub fn read_u16(&self, address: u32) -> u16 {
        u16::from_be_bytes([self.read_u8(address), self.read_u8(address.wrapping_add(1))])
    }
    pub fn read_u32(&self, address: u32) -> u32 {
        (u32::from(self.read_u16(address)) << 16)
            | u32::from(self.read_u16(address.wrapping_add(2)))
    }
    pub fn write_u8(&mut self, address: u32, value: u8) {
        let address = address & 0xFF_FFFF;
        let bus = self.emulator.hybrid_bus_mut();
        match address {
            0..=0x3F_FFFF => {} // Cartridge ROM is immutable.
            0xE00000..=0xFFFFFF => bus.memory.write_main_ram_byte(address, value),
            _ => panic!("write_u8 is ROM/RAM-only; use bus() for hardware address {address:06X}"),
        }
    }
    pub fn write_u16(&mut self, address: u32, value: u16) {
        let [hi, lo] = value.to_be_bytes();
        self.write_u8(address, hi);
        self.write_u8(address.wrapping_add(1), lo);
    }
    pub fn write_u32(&mut self, address: u32, value: u32) {
        self.write_u16(address, (value >> 16) as u16);
        self.write_u16(address.wrapping_add(2), value as u16);
    }
    pub fn argument_u32(&self, index: u32) -> u32 {
        self.read_u32(
            self.emulator
                .hybrid_cpu()
                .stack_pointer()
                .wrapping_add(4 + 4 * index),
        )
    }
    pub fn argument_i16(&self, index: u32) -> i16 {
        self.argument_u32(index) as i16
    }

    /// Read/write hardware through the real bus, recording the same VDP/YM/PSG
    /// events as emulated code. Work RAM writes above bypass deferred CPU writes.
    pub fn bus(&mut self) -> &mut GenesisBus {
        self.emulator.hybrid_bus_mut()
    }
    pub fn cpu(&mut self) -> &mut M68000 {
        self.emulator.hybrid_parts().0
    }

    /// Execute an untranslated MCC68K function, bypassing the Rust registry.
    /// Push arguments right-to-left, run to an unexecuted sentinel with matching
    /// SP (so nested RTS cannot end the call), then perform caller cleanup.
    /// This atomic call is for functions that do not wait for peripheral time.
    pub fn call_original(&mut self, address: u32, arguments: &[u32]) -> Result<u32> {
        let saved_sp = self.emulator.hybrid_cpu().stack_pointer();
        let saved_pc = self.emulator.hybrid_cpu().pc();
        let mut sp = saved_sp;
        for &argument in arguments.iter().rev() {
            sp = sp.wrapping_sub(4);
            self.write_u32(sp, argument);
        }
        let cleanup_sp = sp;
        sp = sp.wrapping_sub(4);
        self.write_u32(sp, CALL_SENTINEL);
        let (cpu, bus) = self.emulator.hybrid_parts();
        set_sp(cpu, sp);
        cpu.set_pc(address, bus);
        run_original_atomic(self.emulator, CALL_SENTINEL, cleanup_sp)?;
        self.nested_calls += 1;
        let (cpu, bus) = self.emulator.hybrid_parts();
        let result = cpu.data_registers()[0];
        set_sp(cpu, saved_sp);
        cpu.set_pc(saved_pc, bus);
        Ok(result)
    }

    pub(crate) fn return_to_caller(&mut self, value: Option<u32>) {
        let sp = self.emulator.hybrid_cpu().stack_pointer();
        let return_pc = self.read_u32(sp);
        let (cpu, bus) = self.emulator.hybrid_parts();
        if let Some(value) = value {
            let mut registers = cpu.data_registers();
            registers[0] = value;
            cpu.set_data_registers(registers);
        }
        set_sp(cpu, sp.wrapping_add(4));
        cpu.set_pc(return_pc, bus);
        bus.apply_pending_writes();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genesis_config::GenesisEmulatorConfig;
    use jgenesis_common::frontend::TimingMode;

    #[test]
    fn stack_audit_records_reads_before_writes_not_caller_arguments() {
        let mut bus = GenesisBus::new(TimingMode::Ntsc, None, &GenesisEmulatorConfig::default());
        let mut audit = StackAuditBus {
            inner: &mut bus,
            entry_sp: 0xFF7FF0,
            pc: 0x1234,
            written: [0; 512],
            reads: BTreeSet::new(),
        };
        audit.read_word(0xFF7FEC);
        assert_eq!(audit.reads.len(), 2);
        audit.reads.clear();
        audit.write_word(0xFF7FEC, 0x4321);
        audit.read_word(0xFF7FEC);
        audit.read_word(0xFF7FF0); // caller return address: not a callee local
        audit.read_word(0xFF8000); // allocated globals: not stack
        assert!(audit.reads.is_empty());
        audit.read_byte(0xFFFF7FEE); // address-space wrapping is still stack
        assert_eq!(
            audit.reads.into_iter().collect::<Vec<_>>(),
            [UninitializedStackRead {
                pc: 0x1234,
                address: 0xFF7FEE
            }]
        );
    }
}
