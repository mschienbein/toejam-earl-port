#[cfg(any(test, feature = "memorybus"))]
pub mod bus;
mod core;
pub mod debug;
pub mod disassemble;
pub mod traits;

pub use crate::core::{
    AddressRegister, DataRegister, IndexRegister, IndexSize, M68000, OpSize,
    cycles_if_move_btst_cmp,
};
pub use traits::{BusInterface, ReadPurpose};

#[cfg(test)]
mod read_purpose_tests {
    //! TJE synthetic instruction fixtures; no cartridge code or data.
    use crate::{BusInterface, M68000, ReadPurpose, debug::DummyM68000Debugger};

    struct Bus {
        memory: Vec<u8>,
        reads: Vec<(u32, ReadPurpose)>,
        bus_reads: usize,
    }
    impl Bus {
        fn new(words: &[u16]) -> (Self, M68000) {
            let mut bus = Self { memory: vec![0; 0x10000], reads: Vec::new(), bus_reads: 0 };
            for (index, word) in words.iter().enumerate() {
                bus.write_word(0x100 + 2 * index as u32, *word);
            }
            let mut cpu = M68000::default();
            let mut addresses = [0; 7];
            addresses[6] = 0x7000;
            cpu.set_address_registers(addresses, 0x8000, 0x8000);
            cpu.set_pc(0x100, &mut bus);
            bus.reads.clear();
            bus.bus_reads = 0;
            (bus, cpu)
        }
    }
    impl BusInterface for Bus {
        type DebugView<'a> = DummyM68000Debugger;
        fn read_byte(&mut self, address: u32) -> u8 {
            self.bus_reads += 1;
            self.memory[address as usize]
        }
        fn read_word(&mut self, address: u32) -> u16 {
            self.bus_reads += 1;
            u16::from_be_bytes([self.memory[address as usize], self.memory[address as usize + 1]])
        }
        fn read_byte_tagged(&mut self, address: u32, purpose: ReadPurpose) -> u8 {
            self.reads.push((address, purpose));
            self.read_byte(address)
        }
        fn read_word_tagged(&mut self, address: u32, purpose: ReadPurpose) -> u16 {
            self.reads.push((address, purpose));
            self.read_word(address)
        }
        fn write_byte(&mut self, address: u32, value: u8) {
            self.memory[address as usize] = value;
        }
        fn write_word(&mut self, address: u32, value: u16) {
            self.memory[address as usize..address as usize + 2]
                .copy_from_slice(&value.to_be_bytes());
        }
        fn interrupt_level(&self) -> u8 {
            0
        }
        fn acknowledge_interrupt(&mut self, _: u8) {}
        fn halt(&self) -> bool {
            false
        }
        fn reset(&self) -> bool {
            false
        }
    }

    #[test]
    fn clr_discarded_read_still_accesses_bus_and_next_real_read_is_semantic() {
        // CLR.W -2(A6); MOVE.W -2(A6),D0; NOP.
        let (mut bus, mut cpu) = Bus::new(&[0x426e, 0xfffe, 0x302e, 0xfffe, 0x4e71]);
        bus.write_word(0x6ffe, 0xabcd);
        cpu.execute_instruction(&mut bus);
        assert!(bus.reads.contains(&(0x6ffe, ReadPurpose::Discarded)));
        assert!(!bus.reads.contains(&(0x6ffe, ReadPurpose::Semantic)));
        assert!(bus.reads.iter().any(|(_, kind)| *kind == ReadPurpose::Prefetch));
        assert_eq!(bus.bus_reads, bus.reads.len());
        cpu.execute_instruction(&mut bus);
        assert!(bus.reads.contains(&(0x6ffe, ReadPurpose::Semantic)));
        assert_eq!(cpu.data_registers()[0] & 0xffff, 0);
    }

    #[test]
    fn movem_terminal_word_is_discarded_but_a_real_read_of_it_is_semantic() {
        // MOVEM.L -28(A6),D2-D6/A2; MOVE.W -4(A6),D0; NOP.
        let (mut bus, mut cpu) = Bus::new(&[0x4cee, 0x047c, 0xffe4, 0x302e, 0xfffc, 0x4e71]);
        for index in 0..6 {
            bus.write_long_word(0x6fe4 + 4 * index, 0x10000000 + index);
        }
        bus.write_word(0x6ffc, 0xbeef);
        cpu.execute_instruction(&mut bus);
        assert!(bus.reads.contains(&(0x6ffc, ReadPurpose::Discarded)));
        assert!(!bus.reads.contains(&(0x6ffc, ReadPurpose::Semantic)));
        assert_eq!(
            bus.reads.iter().filter(|(_, purpose)| *purpose == ReadPurpose::Semantic).count(),
            12
        );
        assert_eq!(bus.bus_reads, bus.reads.len());
        cpu.execute_instruction(&mut bus);
        assert!(bus.reads.contains(&(0x6ffc, ReadPurpose::Semantic)));
        assert_eq!(cpu.data_registers()[0] & 0xffff, 0xbeef);
    }
}
