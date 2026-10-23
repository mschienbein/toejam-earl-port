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
use tje_game::{Dispatcher, FunctionId};
use tje_rom::Revision;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OrderedWrite {
    Ram { address: u32, value: u8 },
    DeviceByte { address: u32, value: u8 },
    DeviceWord { address: u32, value: u16 },
}

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
    pub writes: Vec<OrderedWrite>,
}

/// CPU bus observer. Reading an old local before writing it is significant even
/// when that byte lies in the normally ignored dead-stack comparison window.
struct StackAuditBus<'a> {
    inner: &'a mut GenesisBus,
    entry_sp: u32,
    pc: u32,
    written: [u64; 512],
    reads: BTreeSet<UninitializedStackRead>,
    writes: Vec<OrderedWrite>,
}

impl StackAuditBus<'_> {
    fn stack_offset(&self, address: u32) -> Option<usize> {
        let address = address & 0xFF_FFFF;
        if address < 0xE00000 {
            return None;
        }
        let address = 0xFF0000 | (address & 0xFFFF);
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
                address: 0xFF0000 | (address & 0xFFFF),
            });
        }
    }

    fn write(&mut self, address: u32, value: u8) {
        if let Some(offset) = self.stack_offset(address) {
            self.written[offset / 64] |= 1 << (offset % 64);
        }
        let address = address & 0xFF_FFFF;
        if address >= 0xE00000 {
            self.writes.push(OrderedWrite::Ram {
                address: 0xFF0000 | (address & 0xFFFF),
                value,
            });
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
        self.write(address, value);
        if !(0..=0x3F_FFFF).contains(&(address & 0xFF_FFFF)) && (address & 0xFF_FFFF) < 0xE00000 {
            self.writes.push(OrderedWrite::DeviceByte {
                address: address & 0xFF_FFFF,
                value,
            });
        }
        self.inner.write_byte(address, value);
    }
    fn write_word(&mut self, address: u32, value: u16) {
        self.write(address, (value >> 8) as u8);
        self.write(address.wrapping_add(1), value as u8);
        if !(0..=0x3F_FFFF).contains(&(address & 0xFF_FFFF)) && (address & 0xFF_FFFF) < 0xE00000 {
            self.writes.push(OrderedWrite::DeviceWord {
                address: address & 0xFF_FFFF,
                value,
            });
        }
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
        writes: Vec::new(),
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
    result.writes = audit.writes;
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
    revision: Revision,
    dispatcher: Dispatcher,
    pub(crate) writes: Vec<OrderedWrite>,
    pub(crate) uninitialized_reads: BTreeSet<UninitializedStackRead>,
}

impl<'a> HookContext<'a> {
    pub(crate) fn new(
        emulator: &'a mut GenesisEmulator,
        revision: Revision,
        dispatcher: Dispatcher,
    ) -> Self {
        Self {
            emulator,
            revision,
            dispatcher,
            writes: Vec::new(),
            nested_calls: 0,
            uninitialized_reads: BTreeSet::new(),
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
            0xE00000..=0xFFFFFF => {
                self.writes.push(OrderedWrite::Ram {
                    address: 0xFF0000 | (address & 0xFFFF),
                    value,
                });
                bus.memory.write_main_ram_byte(address, value);
            }
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

    /// Ordered immediate CPU-bus facade. Unlike the emulator's instruction-local
    /// queues, consecutive Rust byte/word accesses are never regrouped by width.
    pub fn bus(&mut self) -> HardwareBus<'_, 'a> {
        HardwareBus { context: self }
    }
    pub fn cpu(&mut self) -> &mut M68000 {
        self.emulator.hybrid_parts().0
    }

    /// Execute an untranslated MCC68K function, bypassing the Rust registry.
    /// Push arguments right-to-left, run to an unexecuted sentinel with matching
    /// SP (so nested RTS cannot end the call), then perform caller cleanup.
    /// This atomic call is for functions that do not wait for peripheral time.
    pub fn call_original(&mut self, address: u32, arguments: &[u32]) -> Result<u32> {
        let id = tje_game::function_at(self.revision, address).ok_or_else(|| {
            HostError::Metadata(format!("no declared atomic call contract at {address:06X}"))
        })?;
        self.call_original_with_contract(id.metadata(), arguments)
    }

    /// Explicit contracts also support authored, ROM-free machine-code fixtures.
    pub fn call_original_with_contract(
        &mut self,
        contract: &tje_game::FunctionMetadata,
        arguments: &[u32],
    ) -> Result<u32> {
        if !contract.atomic_capable {
            return Err(HostError::NotAtomicCapable {
                function: contract.name.to_owned(),
            });
        }
        let address = contract.address(self.revision);
        self.emulator.hybrid_bus_mut().apply_pending_writes();
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
        let audit = run_original_atomic(self.emulator, CALL_SENTINEL, cleanup_sp)?;
        self.writes.extend(audit.writes);
        self.uninitialized_reads.extend(audit.uninitialized_reads);
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

pub struct HardwareBus<'context, 'emulator> {
    context: &'context mut HookContext<'emulator>,
}
impl BusInterface for HardwareBus<'_, '_> {
    type DebugView<'a>
        = DummyM68000Debugger
    where
        Self: 'a;
    fn read_byte(&mut self, address: u32) -> u8 {
        tje_game::Platform::read_device_u8(self.context, address)
    }
    fn read_word(&mut self, address: u32) -> u16 {
        tje_game::Platform::read_device_u16(self.context, address)
    }
    fn write_byte(&mut self, address: u32, value: u8) {
        let physical = address & 0xFFFFFF;
        if physical >= 0xE00000 || physical <= 0x3FFFFF {
            self.context.write_u8(address, value);
        } else {
            tje_game::Platform::write_device_u8(self.context, address, value);
        }
    }
    fn write_word(&mut self, address: u32, value: u16) {
        let physical = address & 0xFFFFFF;
        if physical >= 0xE00000 || physical <= 0x3FFFFF {
            self.context.write_u16(address, value);
        } else {
            tje_game::Platform::write_device_u16(self.context, address, value);
        }
    }
    fn interrupt_level(&self) -> u8 {
        self.context.emulator.hybrid_bus().interrupt_level()
    }
    fn acknowledge_interrupt(&mut self, level: u8) {
        self.context
            .emulator
            .hybrid_bus_mut()
            .acknowledge_interrupt(level);
    }
    fn halt(&self) -> bool {
        self.context.emulator.hybrid_bus().halt()
    }
    fn reset(&self) -> bool {
        <GenesisBus as BusInterface>::reset(self.context.emulator.hybrid_bus())
    }
}

impl tje_mem::Bus for HookContext<'_> {
    fn read_u8(&self, address: u32) -> u8 {
        HookContext::read_u8(self, address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        HookContext::write_u8(self, address, value);
    }
}
impl tje_mem::Platform for HookContext<'_> {
    fn write_vdp_control(&mut self, value: u16) {
        tje_game::Platform::write_device_u16(self, 0xC00004, value);
    }
    fn write_vdp_data(&mut self, value: u16) {
        tje_game::Platform::write_device_u16(self, 0xC00000, value);
    }
    fn write_vdp_command(&mut self, value: u32) {
        // MOVE.L to the command port uses its two word-address aliases.
        tje_game::Platform::write_device_u16(self, 0xC00004, (value >> 16) as u16);
        tje_game::Platform::write_device_u16(self, 0xC00006, value as u16);
    }
}
impl tje_game::Platform for HookContext<'_> {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        let bus = self.emulator.hybrid_bus_mut();
        bus.apply_pending_writes();
        bus.read_byte(address)
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        let bus = self.emulator.hybrid_bus_mut();
        bus.apply_pending_writes();
        bus.read_word(address)
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.writes.push(OrderedWrite::DeviceByte {
            address: address & 0xFF_FFFF,
            value,
        });
        let bus = self.emulator.hybrid_bus_mut();
        bus.write_byte(address, value);
        bus.apply_pending_writes();
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.writes.push(OrderedWrite::DeviceWord {
            address: address & 0xFF_FFFF,
            value,
        });
        let bus = self.emulator.hybrid_bus_mut();
        bus.write_word(address, value);
        bus.apply_pending_writes();
    }
}
impl tje_game::Context for HookContext<'_> {
    fn revision(&self) -> Revision {
        self.revision
    }
    fn data_register(&self, index: usize) -> u32 {
        self.emulator.hybrid_cpu().data_registers()[index]
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        let mut registers = self.emulator.hybrid_cpu().data_registers();
        registers[index] = value;
        self.emulator.hybrid_parts().0.set_data_registers(registers);
    }
    fn address_register(&self, index: usize) -> u32 {
        self.emulator.hybrid_cpu().address_registers()[index]
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        let cpu = self.emulator.hybrid_parts().0;
        let mut addresses = cpu.address_registers();
        addresses[index] = value;
        cpu.set_address_registers(
            addresses,
            cpu.user_stack_pointer(),
            cpu.supervisor_stack_pointer(),
        );
    }
    fn status_register(&self) -> u16 {
        self.emulator.hybrid_cpu().status_register()
    }
    fn set_status_register(&mut self, value: u16) {
        self.emulator.hybrid_parts().0.set_status_register(value);
    }
    fn dispatcher(&self) -> Dispatcher {
        self.dispatcher
    }
    fn call_original(&mut self, id: FunctionId, arguments: &[u32]) -> tje_game::Result<u32> {
        HookContext::call_original(self, id.metadata().address(self.revision), arguments)
            .map_err(|error| tje_game::Error::Platform(error.to_string()))
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
            writes: Vec::new(),
        };
        audit.read_word(0xFF7FEC);
        assert_eq!(audit.reads.len(), 2);
        audit.reads.clear();
        audit.write_word(0xFF7FEC, 0x4321);
        audit.read_word(0xFF7FEC);
        audit.read_word(0xFF7FF0); // caller return address: not a callee local
        audit.read_word(0xFF8000); // allocated globals: not stack
        assert!(audit.reads.is_empty());
        audit.read_word(0xE07FEC); // mirrored writes/read audit the same physical bytes
        assert!(audit.reads.is_empty());
        audit.read_byte(0xE07FEB);
        assert_eq!(audit.reads.iter().next().unwrap().address, 0xFF7FEB);
        audit.reads.clear();
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
