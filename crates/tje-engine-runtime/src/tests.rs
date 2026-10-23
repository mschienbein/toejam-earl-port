//! Authored instruction fixtures only; no retail instructions or state dumps.

use super::*;
use m68000_emu::{BusInterface, M68000, ReadPurpose, debug::DummyM68000Debugger};
use std::convert::Infallible;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Access {
    Read(u32, u8, u32),
    Write(u32, u8, u32),
    Ack(u8),
}

#[derive(Clone)]
struct TestDevices {
    memory: Vec<u8>,
    accesses: Vec<Access>,
    irq: u8,
    halt: bool,
    reset: bool,
    wait: u32,
    clocks: u64,
    read_offset: u32,
}

impl TestDevices {
    fn new(words: &[u16]) -> Self {
        let mut result = Self {
            memory: vec![0; 0x10000],
            accesses: Vec::new(),
            irq: 0,
            halt: false,
            reset: false,
            wait: 0,
            clocks: 0,
            read_offset: 0,
        };
        result.poke_long(0, 0x8000);
        result.poke_long(4, 0x100);
        for vector in 2..48 {
            result.poke_long(4 * vector, 0x200);
        }
        result.poke_word(0x200, 0x4E73); // Authored exception handler: RTE.
        result.poke_word(0x202, 0x4E71);
        for (index, word) in words.iter().enumerate() {
            result.poke_word(0x100 + 2 * index as u32, *word);
        }
        result
    }

    fn poke_word(&mut self, address: u32, value: u16) {
        let index = address as usize & 0xFFFF;
        self.memory[index..index + 2].copy_from_slice(&value.to_be_bytes());
    }

    fn poke_long(&mut self, address: u32, value: u32) {
        self.poke_word(address, (value >> 16) as u16);
        self.poke_word(address + 2, value as u16);
    }

    fn word(&self, address: u32) -> u16 {
        let index = address as usize & 0xFFFF;
        u16::from_be_bytes([self.memory[index], self.memory[index + 1]])
    }
}

impl Bus for TestDevices {
    fn read8(&mut self, address: u32, _kind: ReadKind) -> u8 {
        let value = self.memory[address as usize & 0xFFFF];
        self.accesses
            .push(Access::Read(address, 1, u32::from(value)));
        value
    }
    fn read16(&mut self, address: u32, _kind: ReadKind) -> u16 {
        let value = self.word(address);
        self.accesses
            .push(Access::Read(address, 2, u32::from(value)));
        value
    }
    fn write8(&mut self, address: u32, value: u8) {
        self.memory[address as usize & 0xFFFF] = value;
        self.accesses
            .push(Access::Write(address, 1, u32::from(value)));
    }
    fn write16(&mut self, address: u32, value: u16) {
        self.poke_word(address, value);
        self.accesses
            .push(Access::Write(address, 2, u32::from(value)));
    }
    fn set_read_offset_mclk(&mut self, offset: u32) {
        self.read_offset = offset;
    }
}

impl BusInterface for TestDevices {
    type DebugView<'a> = DummyM68000Debugger;

    fn read_byte(&mut self, address: u32) -> u8 {
        Bus::read8(self, address, ReadKind::Semantic)
    }
    fn read_word(&mut self, address: u32) -> u16 {
        Bus::read16(self, address, ReadKind::Semantic)
    }
    fn read_byte_tagged(&mut self, address: u32, _: ReadPurpose) -> u8 {
        Bus::read8(self, address, ReadKind::Semantic)
    }
    fn read_word_tagged(&mut self, address: u32, _: ReadPurpose) -> u16 {
        Bus::read16(self, address, ReadKind::Semantic)
    }
    fn write_byte(&mut self, address: u32, value: u8) {
        Bus::write8(self, address, value);
    }
    fn write_word(&mut self, address: u32, value: u16) {
        Bus::write16(self, address, value);
    }
    fn interrupt_level(&self) -> u8 {
        self.irq
    }
    fn acknowledge_interrupt(&mut self, level: u8) {
        self.accesses.push(Access::Ack(level));
        self.irq = 0;
    }
    fn halt(&self) -> bool {
        self.halt
    }
    fn reset(&self) -> bool {
        self.reset
    }
}

impl Devices for TestDevices {
    type Inputs = ();

    fn set_inputs(&mut self, (): ()) {}
    fn begin_instruction(&mut self, _: u32, _: u16) {
        self.read_offset = 0;
    }
    fn prepare_noninstruction(&mut self) {
        // This flat synthetic bus has no VDP; retain an observable phase marker.
        self.read_offset = 56;
    }
    fn take_wait_step(&mut self, pc: u32) -> Option<DeviceStep> {
        let cycles = std::mem::take(&mut self.wait);
        (cycles != 0).then(|| {
            self.prepare_noninstruction();
            self.finish_instruction(pc, cycles)
        })
    }
    fn finish_instruction(&mut self, _: u32, cpu_cycles: u32) -> DeviceStep {
        let master_clocks = u64::from(cpu_cycles) * 7;
        let previous_frame = self.clocks / 1000;
        self.clocks += master_clocks;
        DeviceStep {
            cpu_cycles,
            master_clocks,
            frame_complete: self.clocks / 1000 != previous_frame,
        }
    }
    fn cpu_halted(&self) -> bool {
        self.halt
    }
    fn pending_interrupt(&self) -> u8 {
        self.irq
    }
    fn acknowledge_interrupt(&mut self, level: u8) {
        BusInterface::acknowledge_interrupt(self, level);
    }
    fn reset_asserted(&self) -> bool {
        self.reset
    }
}

/// A few deliberately authored continuation bodies to isolate the scheduler.
/// General ISA decoding/interpreting is not part of the production runtime.
#[derive(Default)]
struct Fixtures {
    calls: usize,
}

fn fetch(cpu: &mut CpuState, bus: &mut dyn Bus) -> u32 {
    let value = cpu.prefetch;
    cpu.pc = cpu.pc.wrapping_add(2);
    cpu.prefetch = u32::from(bus.read16(cpu.pc, ReadKind::Prefetch));
    value
}

impl Backend for Fixtures {
    type Error = Infallible;

    fn execute(&mut self, cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<StepResult, Infallible> {
        self.calls += 1;
        let opcode = fetch(cpu, bus);
        let mut result = StepResult::normal(4);
        match opcode {
            0x4E71 => {}
            0x4E73 => {
                if cpu.sr & 0x2000 == 0 {
                    result.cycles = 34;
                    result.vector = 8;
                    result.saved_pc = cpu.pc.wrapping_sub(2);
                } else {
                    let sr = bus.read16(cpu.ssp, ReadKind::Semantic);
                    cpu.ssp = cpu.ssp.wrapping_add(2);
                    let high = bus.read16(cpu.ssp, ReadKind::Semantic);
                    let low = bus.read16(cpu.ssp.wrapping_add(2), ReadKind::Semantic);
                    cpu.ssp = cpu.ssp.wrapping_add(4);
                    cpu.set_sr(u32::from(sr));
                    let pc = (u32::from(high) << 16) | u32::from(low);
                    cpu.pc = pc;
                    cpu.prefetch = u32::from(bus.read16(pc, ReadKind::Prefetch));
                    result.cycles = 20;
                }
            }
            0x4E72 if cpu.sr & 0x2000 != 0 => {
                let sr = fetch(cpu, bus);
                cpu.set_sr(sr);
                cpu.stopped = 1;
            }
            0x4E72 => {
                result.cycles = 34;
                result.vector = 8;
                result.saved_pc = cpu.pc.wrapping_sub(2);
            }
            0x4E40..=0x4E4F => {
                result.cycles = 34;
                result.vector = 32 + (opcode & 15);
                result.saved_pc = cpu.pc;
            }
            0x3010 => {
                result.cycles = 50;
                result.vector = 3;
                result.saved_pc = cpu.pc.wrapping_sub(2);
                result.fault_address = cpu.a[0];
                result.fault_status = (opcode & 0xFFE0) | 0x15;
            }
            0x4AFC => {
                result.cycles = 34;
                result.vector = 4;
                result.saved_pc = cpu.pc.wrapping_sub(2);
            }
            _ => panic!("fixture has no authored body for {opcode:04X}"),
        }
        Ok(result)
    }
}

fn pair(words: &[u16]) -> (Machine<TestDevices, Fixtures>, M68000, TestDevices) {
    let devices = TestDevices::new(words);
    let mut reference_devices = devices.clone();
    let mut reference = M68000::default();
    reference_devices.reset = true;
    assert_eq!(reference.execute_instruction(&mut reference_devices), 132);
    reference_devices.reset = false;
    let machine = Machine::new(devices, Fixtures::default());
    assert_equal(&machine, &reference, &reference_devices);
    (machine, reference, reference_devices)
}

fn assert_equal(machine: &Machine<TestDevices, Fixtures>, cpu: &M68000, bus: &TestDevices) {
    assert_eq!(machine.cpu.d, cpu.data_registers());
    assert_eq!(machine.cpu.a, cpu.address_registers());
    assert_eq!(machine.cpu.usp, cpu.user_stack_pointer());
    assert_eq!(machine.cpu.ssp, cpu.supervisor_stack_pointer());
    assert_eq!(machine.cpu.pc, cpu.pc());
    assert_eq!(machine.cpu.sr, u32::from(cpu.status_register()));
    assert_eq!(machine.cpu.prefetch, u32::from(cpu.next_opcode()));
    assert_eq!(machine.cpu.address_error != 0, cpu.address_error());
    assert_eq!(
        machine.cpu.last_muldiv != 0,
        cpu.last_instruction_was_mul_or_div()
    );
    assert_eq!(machine.devices.memory, bus.memory);
    assert_eq!(machine.devices.accesses, bus.accesses);
}

fn one(
    machine: &mut Machine<TestDevices, Fixtures>,
    cpu: &mut M68000,
    bus: &mut TestDevices,
) -> RuntimeStep {
    let cycles = cpu.execute_instruction(bus);
    let step = machine.step(()).unwrap();
    assert_eq!(step.devices.cpu_cycles, cycles);
    assert_equal(machine, cpu, bus);
    step
}

#[test]
fn power_on_prefetch_and_all_masked_irq_levels_match_reference() {
    for mask in 0..8 {
        for level in 0..8 {
            let (mut machine, mut cpu, mut bus) = pair(&[0x4E71, 0x4E71, 0x4E71]);
            machine.cpu.set_sr(0x2000 | (mask << 8));
            cpu.set_status_register((0x2000 | (mask << 8)) as u16);
            machine.devices.irq = level;
            bus.irq = level;
            let step = one(&mut machine, &mut cpu, &mut bus);
            if u32::from(level) > mask {
                assert_eq!(step.kind, StepKind::InterruptAdmission { level });
                assert_eq!(machine.cpu.pc, 0x100);
                assert_eq!(machine.backend.calls, 0);
                // The accepted level is latched despite the line deasserting.
                machine.devices.irq = 0;
                bus.irq = 0;
                let ack = one(&mut machine, &mut cpu, &mut bus);
                assert_eq!(ack.kind, StepKind::InterruptAcknowledge { level });
                assert_eq!(ack.devices.cpu_cycles, 44);
                one(&mut machine, &mut cpu, &mut bus); // RTE.
            } else {
                assert_eq!(step.kind, StepKind::Instruction);
                assert_eq!(machine.cpu.pc, 0x102);
            }
        }
    }
}

#[test]
fn supervisor_entry_and_rte_restore_the_user_stack_and_trace_bit() {
    let (mut machine, mut cpu, mut bus) = pair(&[0x4E71, 0x4E71]);
    machine.cpu.usp = 0x7000;
    machine.cpu.set_sr(0x8015);
    cpu.set_address_registers([0; 7], 0x7000, 0x8000);
    cpu.set_status_register(0x8015);
    machine.devices.irq = 6;
    bus.irq = 6;
    for _ in 0..3 {
        one(&mut machine, &mut cpu, &mut bus);
    }
    assert_eq!(machine.cpu.usp, 0x7000);
    assert_eq!(machine.cpu.ssp, 0x8000);
    assert_eq!(machine.cpu.sr, 0x8015);
    assert_eq!(machine.cpu.pc, 0x100);
}

#[test]
fn stop_polls_in_four_cycles_and_interrupts_wake_it() {
    let (mut machine, mut cpu, mut bus) = pair(&[0x4E72, 0x2000, 0x4E71, 0x4E71]);
    one(&mut machine, &mut cpu, &mut bus);
    let stopped = one(&mut machine, &mut cpu, &mut bus);
    assert_eq!(stopped.kind, StepKind::Stopped);
    assert_eq!(machine.backend.calls, 1);
    machine.devices.irq = 4;
    bus.irq = 4;
    for _ in 0..4 {
        one(&mut machine, &mut cpu, &mut bus);
    }
    assert_eq!(machine.cpu.stopped, 0);
    assert_eq!(machine.cpu.pc, 0x106);
}

#[test]
fn trap_illegal_privilege_and_address_frames_match_reference_bus_order() {
    for opcode in [0x4E40, 0x4E4F, 0x4AFC, 0x4E72, 0x3010] {
        let (mut machine, mut cpu, mut bus) = pair(&[opcode, 0x4E71, 0x4E71]);
        machine.cpu.a[0] = 0x3001;
        let mut addresses = [0; 7];
        addresses[0] = 0x3001;
        cpu.set_address_registers(addresses, 0, 0x8000);
        if opcode == 0x4E72 {
            machine.cpu.set_sr(0x001F);
            cpu.set_status_register(0x001F);
        }
        one(&mut machine, &mut cpu, &mut bus);
    }
}

#[test]
fn double_address_errors_freeze_until_reset_with_no_extra_frame() {
    let (mut machine, mut cpu, mut bus) = pair(&[0x3010, 0x4E71]);
    machine.cpu.a[0] = 1;
    machine.cpu.ssp = 0x8001;
    let mut addresses = [0; 7];
    addresses[0] = 1;
    cpu.set_address_registers(addresses, 0, 0x8001);
    one(&mut machine, &mut cpu, &mut bus);
    assert_eq!(machine.cpu.frozen, 1);
    let stopped = one(&mut machine, &mut cpu, &mut bus);
    assert_eq!(stopped.kind, StepKind::Halted);
    assert_eq!(stopped.devices.cpu_cycles, 1);
    machine.devices.reset = true;
    bus.reset = true;
    let reset = one(&mut machine, &mut cpu, &mut bus);
    assert_eq!(reset.kind, StepKind::Reset);
    assert_eq!(machine.cpu.frozen, 0);
}

#[test]
fn odd_interrupt_and_exception_vectors_match_reference_fault_frames() {
    for vector in [30, 32] {
        let (mut machine, mut cpu, mut bus) = pair(&[0x4E40, 0x4E71]);
        machine.devices.poke_long(vector * 4, 0x301);
        bus.poke_long(vector * 4, 0x301);
        if vector == 30 {
            machine.cpu.set_sr(0x2000);
            cpu.set_status_register(0x2000);
            machine.devices.irq = 6;
            bus.irq = 6;
            one(&mut machine, &mut cpu, &mut bus);
        }
        one(&mut machine, &mut cpu, &mut bus);
    }
}

#[test]
fn hardware_waits_precede_irq_and_halt_has_one_cycle_granularity() {
    let (mut machine, _, _) = pair(&[0x4E71, 0x4E71]);
    machine.cpu.set_sr(0x2000);
    machine.devices.irq = 6;
    machine.devices.wait = 3;
    let wait = machine.step(()).unwrap();
    assert_eq!(wait.kind, StepKind::Wait);
    assert_eq!(wait.devices.master_clocks, 21);
    assert_eq!(machine.cpu.pending_interrupt, 0);
    assert_eq!(machine.cpu.pc, 0x100);
    machine.devices.halt = true;
    assert_eq!(machine.step(()).unwrap().devices.cpu_cycles, 1);
    assert_eq!(machine.cpu.pending_interrupt, 0);
    machine.devices.halt = false;
    assert_eq!(
        machine.step(()).unwrap().kind,
        StepKind::InterruptAdmission { level: 6 }
    );
}

#[test]
fn numeric_callbacks_preserve_widths_purpose_and_offsets() {
    let mut bus = TestDevices::new(&[0x4E71]);
    let mut context = abi::BusContext::new(&mut bus);
    let pointer = context.as_mut_ptr();
    // SAFETY: context exclusively borrows the bus and outlives all calls here.
    unsafe {
        (abi::BUS_CALLBACKS.write8)(pointer, 0x300, 0x1234);
        (abi::BUS_CALLBACKS.write16)(pointer, 0x302, 0x123456);
        assert_eq!((abi::BUS_CALLBACKS.read8)(pointer, 0x300, 0), 0x34);
        assert_eq!((abi::BUS_CALLBACKS.read16)(pointer, 0x302, 1), 0x3456);
        (abi::BUS_CALLBACKS.set_read_offset_mclk)(pointer, 42);
    }
    assert_eq!(bus.read_offset, 42);
}

#[test]
fn odd_boundary_pc_faults_before_decoding_or_prefetch_access() {
    let devices = TestDevices::new(&[0x4E71]);
    let cpu = CpuState {
        pc: 0x301,
        sr: 0x2700,
        ssp: 0x8000,
        prefetch: 0x4E71,
        ..CpuState::default()
    };
    let mut machine = Machine::from_state(cpu, devices, Fixtures::default(), 0);
    let step = machine.step(()).unwrap();
    assert_eq!(step.devices.cpu_cycles, 50);
    assert_eq!(machine.backend.calls, 0);
    assert_eq!(machine.cpu.address_error, 1);
    assert_eq!(machine.cpu.pc, 0x200);
    assert_eq!(machine.cpu.ssp, 0x7FF2);
    assert_eq!(machine.devices.word(0x7FF2), 0x15);
    assert_eq!(machine.devices.word(0x7FF6), 0x303);
    assert_eq!(machine.devices.word(0x7FF8), 0); // Opcode was not fetched.
    assert_eq!(machine.devices.word(0x7FFC), 0);
    assert_eq!(machine.devices.word(0x7FFE), 0x2FF);
    assert!(!machine.devices.accesses.iter().any(|access| {
        matches!(access, Access::Read(address, _, _) if *address == 0x301 || *address == 0x303)
    }));
}

#[test]
fn hardware_reset_preserves_ccr_registers_and_the_already_admitted_irq() {
    let (mut machine, mut cpu, mut bus) = pair(&[0x4E71, 0x4E71]);
    machine.cpu.d[0] = 0x12345678;
    let mut data = [0; 8];
    data[0] = 0x12345678;
    cpu.set_data_registers(data);
    machine.cpu.set_sr(0x201F);
    cpu.set_status_register(0x201F);
    machine.devices.irq = 6;
    bus.irq = 6;
    one(&mut machine, &mut cpu, &mut bus);
    machine.devices.reset = true;
    bus.reset = true;
    assert_eq!(one(&mut machine, &mut cpu, &mut bus).kind, StepKind::Reset);
    assert_eq!(machine.cpu.sr, 0x271F);
    assert_eq!(machine.cpu.d[0], 0x12345678);
    assert_eq!(machine.cpu.pending_interrupt, 6);
    machine.devices.reset = false;
    bus.reset = false;
    machine.devices.irq = 0;
    bus.irq = 0;
    assert_eq!(
        one(&mut machine, &mut cpu, &mut bus).kind,
        StepKind::InterruptAcknowledge { level: 6 }
    );
    assert_eq!(machine.backend.calls, 0);
}

#[test]
fn slow_boundaries_prepare_read_phase_without_redecoding_normal_continuations() {
    for case in 0..9 {
        let (mut machine, _, _) = pair(&[0x4E71, 0x4E71]);
        let expected = match case {
            0 => StepKind::Instruction,
            1 => {
                machine.devices.wait = 3;
                StepKind::Wait
            }
            2 => {
                machine.devices.reset = true;
                StepKind::Reset
            }
            3 => {
                machine.devices.halt = true;
                StepKind::Halted
            }
            4 => {
                machine.cpu.frozen = 1;
                StepKind::Halted
            }
            5 => {
                machine.cpu.stopped = 1;
                StepKind::Stopped
            }
            6 => {
                machine.cpu.set_sr(0x2000);
                machine.devices.irq = 6;
                StepKind::InterruptAdmission { level: 6 }
            }
            7 => {
                machine.cpu.pending_interrupt = 6;
                StepKind::InterruptAcknowledge { level: 6 }
            }
            _ => {
                machine.cpu.pc = 0x301;
                StepKind::Instruction
            }
        };
        assert_eq!(machine.step(()).unwrap().kind, expected);
        assert_eq!(machine.devices.read_offset, if case == 0 { 0 } else { 56 });
    }
}
