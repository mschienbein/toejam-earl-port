//! Authored real-core access counts, independent of the capacity formula.
use super::*;

struct AccessCounter {
    memory: Vec<u8>,
    accesses: usize,
    nmi: InterruptLine,
    int: InterruptLine,
}
impl AccessCounter {
    fn new() -> Self {
        Self {
            memory: vec![0; 0x10000],
            accesses: 0,
            nmi: InterruptLine::High,
            int: InterruptLine::High,
        }
    }
}
impl BusInterface for AccessCounter {
    type DebugView<'a> = DummyZ80Debugger;
    fn read_memory(&mut self, address: u16) -> u8 {
        self.accesses += 1;
        self.memory[usize::from(address)]
    }
    fn write_memory(&mut self, address: u16, value: u8) {
        self.accesses += 1;
        self.memory[usize::from(address)] = value;
    }
    fn read_io(&mut self, _: u16) -> u8 {
        0xff
    }
    fn write_io(&mut self, _: u16, _: u8) {}
    fn nmi(&self) -> InterruptLine {
        self.nmi
    }
    fn int(&self) -> InterruptLine {
        self.int
    }
    fn busreq(&self) -> bool {
        false
    }
    fn reset(&self) -> bool {
        false
    }
}
#[derive(Debug, Clone, Copy)]
enum Family {
    Base,
    Cb,
    Ed,
    Ix,
    Iy,
    IxCb,
    IyCb,
    IxEd,
    IyEd,
}
impl Family {
    fn instruction(self, opcode: u8) -> [u8; 6] {
        match self {
            Self::Base => [opcode, 0, 0x90, 0, 0, 0],
            Self::Cb => [0xcb, opcode, 0, 0x90, 0, 0],
            Self::Ed => [0xed, opcode, 0, 0x90, 0, 0],
            Self::Ix => [0xdd, opcode, 0, 0x90, 0, 0],
            Self::Iy => [0xfd, opcode, 0, 0x90, 0, 0],
            Self::IxCb => [0xdd, 0xcb, 0, opcode, 0, 0],
            Self::IyCb => [0xfd, 0xcb, 0, opcode, 0, 0],
            Self::IxEd => [0xdd, 0xed, opcode, 0, 0x90, 0],
            Self::IyEd => [0xfd, 0xed, opcode, 0, 0x90, 0],
        }
    }
}

#[test]
fn every_opcode_family_and_branch_polarity_has_at_most_six_nonprefix_accesses() {
    let mut bus = AccessCounter::new();
    let mut observed_maximum = 0;
    let mut cases = 0;
    for flags in [0_u8, 0xff] {
        // Authored register/flag setup through real instructions; all pointer
        // registers and SP reside in the bank window, including indexed modes.
        let setup = [
            0x31, 0, 0x93, 0x01, 0, 0x90, 0x11, 0, 0x91, 0x21, flags, 0x12, 0xe5, 0xf1, 0x21, 0,
            0x92, 0xdd, 0x21, 0, 0x94, 0xfd, 0x21, 0, 0x95,
        ];
        bus.memory.fill(0);
        bus.memory[..setup.len()].copy_from_slice(&setup);
        let mut prepared = Z80::new();
        for _ in 0..9 {
            prepared.execute_instruction(&mut bus);
        }
        assert_eq!(usize::from(prepared.pc()), setup.len());
        for family in [
            Family::Base,
            Family::Cb,
            Family::Ed,
            Family::Ix,
            Family::Iy,
            Family::IxCb,
            Family::IyCb,
            Family::IxEd,
            Family::IyEd,
        ] {
            for opcode in 0_u8..=u8::MAX {
                bus.memory.fill(0);
                let code = family.instruction(opcode);
                bus.memory[0x8000..0x8000 + code.len()].copy_from_slice(&code);
                let prefixes = code
                    .iter()
                    .take_while(|&&byte| matches!(byte, 0xdd | 0xfd))
                    .count();
                let mut cpu = prepared.clone();
                cpu.set_pc(0x8000);
                bus.accesses = 0;
                cpu.execute_instruction(&mut bus);
                let nonprefix = bus.accesses.checked_sub(prefixes).unwrap();
                assert!(
                    nonprefix <= 6,
                    "{family:?} {opcode:02X} flags={flags:02X}: {nonprefix}"
                );
                observed_maximum = observed_maximum.max(nonprefix);
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 256 * 9 * 2);
    assert_eq!(
        observed_maximum, 6,
        "ED absolute word transfers attain the bound"
    );
}

#[test]
fn actual_irq_and_nmi_entry_have_two_memory_accesses() {
    for nmi in [false, true] {
        let mut bus = AccessCounter::new();
        bus.memory[..2].copy_from_slice(&[0xfb, 0]); // EI; NOP clears EI delay.
        let mut cpu = Z80::new();
        cpu.set_sp(0x9300);
        cpu.execute_instruction(&mut bus);
        cpu.execute_instruction(&mut bus);
        bus.accesses = 0;
        if nmi {
            bus.nmi = InterruptLine::Low;
        } else {
            bus.int = InterruptLine::Low;
        }
        cpu.execute_instruction(&mut bus);
        assert_eq!(bus.accesses, 2);
        assert_eq!(cpu.pc(), if nmi { 0x66 } else { 0x38 });
    }
}
