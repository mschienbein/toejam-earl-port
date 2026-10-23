//! Frame construction follows the reference CPU's bus order, including its
//! approximate address-error frames and fixed 10 + 44 cycle interrupt entry.

use crate::{Bus, CpuState, ReadKind, StepResult};

const SUPERVISOR: u32 = 0x2000;
const TRACE: u32 = 0x8000;

#[derive(Clone, Copy)]
struct Fault {
    address: u32,
    write: bool,
    jump: bool,
}

pub(super) fn reset(cpu: &mut CpuState, bus: &mut dyn Bus) {
    // A reset does not clear CCR, data/address registers, or the latched pending
    // IRQ in the reference. Power-on starts these at zero in Machine::new.
    cpu.sr = (cpu.sr & 0x1F) | 0x2700;
    cpu.stopped = 0;
    cpu.frozen = 0;
    cpu.ssp = read_long(bus, 0);
    cpu.pc = read_long(bus, 4);
    if let Err(fault) = jump(cpu, bus, cpu.pc) {
        secondary_address_error(cpu, bus, 0, fault);
    }
}

pub(super) fn interrupt(cpu: &mut CpuState, bus: &mut dyn Bus, level: u8) -> u32 {
    let sr = cpu.sr;
    cpu.sr = ((sr & !TRACE) | SUPERVISOR) & !0x0700;
    cpu.sr |= u32::from(level) << 8;
    if let Err(fault) = trap_frame(cpu, bus, 24 + u32::from(level), cpu.pc, sr) {
        secondary_address_error(cpu, bus, 0, fault);
        50
    } else {
        // Admission already consumed ten cycles in the previous scheduler step.
        44
    }
}

pub(super) fn finish(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    opcode: u16,
    result: StepResult,
) -> u32 {
    if result.vector == 0 {
        return result.cycles;
    }
    if result.vector == 3 {
        address_error(
            cpu,
            bus,
            opcode,
            result.saved_pc,
            result.fault_address,
            result.fault_status as u16,
        );
        return 50;
    }
    let sr = cpu.sr;
    cpu.sr = (sr & !TRACE) | SUPERVISOR;
    match trap_frame(cpu, bus, result.vector, result.saved_pc, sr) {
        Ok(()) => result.cycles,
        Err(fault) => {
            secondary_address_error(cpu, bus, opcode, fault);
            50
        }
    }
}

fn trap_frame(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    vector: u32,
    saved_pc: u32,
    sr: u32,
) -> Result<(), Fault> {
    push_long(cpu, bus, saved_pc)?;
    push_word(cpu, bus, sr as u16)?;
    let target = read_long(bus, vector * 4);
    jump(cpu, bus, target)
}

fn secondary_address_error(cpu: &mut CpuState, bus: &mut dyn Bus, opcode: u16, fault: Fault) {
    // Frame-construction faults have no instruction EA to adjust the PC. The
    // compiled instruction supplies the exact saved-PC/status for primary faults.
    let saved_pc = cpu.pc.wrapping_sub(2);
    let code = if fault.jump {
        if cpu.sr & SUPERVISOR != 0 { 0x0E } else { 0x0A }
    } else {
        0x05
    };
    let rw = u16::from(!fault.write) << 4;
    let status = (opcode & 0xFFE0) | rw | code;
    address_error(cpu, bus, opcode, saved_pc, fault.address, status);
}

fn address_error(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    opcode: u16,
    saved_pc: u32,
    address: u32,
    status: u16,
) {
    cpu.address_error = 1;
    let sr = cpu.sr;
    cpu.sr = (sr & !TRACE) | SUPERVISOR;
    let result = (|| {
        push_long(cpu, bus, saved_pc)?;
        push_word(cpu, bus, sr as u16)?;
        push_word(cpu, bus, opcode)?;
        push_long(cpu, bus, address)?;
        push_word(cpu, bus, status)?;
        let target = read_long(bus, 12);
        jump(cpu, bus, target)
    })();
    if result.is_err() {
        // A second address error freezes the CPU, rather than recursively
        // inventing another frame. Devices continue to run until external reset.
        cpu.frozen = 1;
    }
}

fn push_word(cpu: &mut CpuState, bus: &mut dyn Bus, value: u16) -> Result<(), Fault> {
    cpu.ssp = cpu.ssp.wrapping_sub(2);
    checked_write(bus, cpu.ssp, value)
}

fn push_long(cpu: &mut CpuState, bus: &mut dyn Bus, value: u32) -> Result<(), Fault> {
    cpu.ssp = cpu.ssp.wrapping_sub(4);
    checked_write(bus, cpu.ssp, (value >> 16) as u16)?;
    checked_write(bus, cpu.ssp.wrapping_add(2), value as u16)
}

fn checked_write(bus: &mut dyn Bus, address: u32, value: u16) -> Result<(), Fault> {
    if address & 1 != 0 {
        return Err(Fault {
            address,
            write: true,
            jump: false,
        });
    }
    bus.write16(address, value);
    Ok(())
}

fn read_long(bus: &mut dyn Bus, address: u32) -> u32 {
    let high = bus.read16(address, ReadKind::Semantic);
    let low = bus.read16(address.wrapping_add(2), ReadKind::Semantic);
    (u32::from(high) << 16) | u32::from(low)
}

fn jump(cpu: &mut CpuState, bus: &mut dyn Bus, target: u32) -> Result<(), Fault> {
    cpu.pc = target.wrapping_sub(2);
    if target & 1 != 0 {
        return Err(Fault {
            address: target,
            write: false,
            jump: true,
        });
    }
    cpu.prefetch = u32::from(bus.read16(target, ReadKind::Prefetch));
    cpu.pc = target;
    Ok(())
}
