#[cfg(any(test, feature = "memorybus"))]
pub mod bus;
mod core;
pub mod debug;
pub mod disassemble;
pub mod traits;

pub use crate::core::{
    AddressRegister, DataRegister, HybridExecutedInstruction, HybridInstructionCounts,
    IndexRegister, IndexSize, M68000, OpSize, cycles_if_move_btst_cmp,
};

#[cfg(test)]
mod instruction_observation_tests {
    use crate::{BusInterface, M68000, debug::DummyM68000Debugger};
    struct Bus {
        memory: Vec<u8>,
        irq: u8,
        halt: bool,
        reset: bool,
    }
    impl Bus {
        fn machine(words: &[u16]) -> (Self, M68000) {
            let mut bus = Self { memory: vec![0; 0x10000], irq: 0, halt: false, reset: false };
            for (i, &word) in words.iter().enumerate() {
                bus.write_word(0x100 + 2 * i as u32, word);
            }
            for vector in [30, 32] {
                bus.write_long_word(4 * vector, 0x200);
            }
            bus.write_word(0x200, 0x4e71);
            bus.write_word(0x202, 0x4e73);
            let mut cpu = M68000::default();
            cpu.set_status_register(0x2000);
            cpu.set_address_registers([0; 7], 0x8000, 0x8000);
            cpu.set_pc(0x100, &mut bus);
            (bus, cpu)
        }
    }
    impl BusInterface for Bus {
        type DebugView<'a> = DummyM68000Debugger;
        fn read_byte(&mut self, address: u32) -> u8 {
            self.memory[address as usize]
        }
        fn read_word(&mut self, address: u32) -> u16 {
            u16::from_be_bytes([self.read_byte(address), self.read_byte(address + 1)])
        }
        fn write_byte(&mut self, address: u32, value: u8) {
            self.memory[address as usize] = value;
        }
        fn write_word(&mut self, address: u32, value: u16) {
            self.memory[address as usize..address as usize + 2]
                .copy_from_slice(&value.to_be_bytes());
        }
        fn interrupt_level(&self) -> u8 {
            self.irq
        }
        fn acknowledge_interrupt(&mut self, _: u8) {}
        fn halt(&self) -> bool {
            self.halt
        }
        fn reset(&self) -> bool {
            self.reset
        }
    }
    #[test]
    fn actual_counter_excludes_irq_recognition_entry_halt_reset_and_stop_polling() {
        let (mut bus, mut cpu) = Bus::machine(&[0x4e71, 0x4e72, 0x2700]);
        cpu.hybrid_enable_instruction_counts();
        bus.halt = true;
        cpu.execute_instruction(&mut bus);
        bus.halt = false;
        assert_eq!(cpu.hybrid_instruction_counts().unwrap().main_thread, 0);
        bus.irq = 6;
        cpu.execute_instruction(&mut bus); // recognition only
        assert_eq!(cpu.hybrid_instruction_counts().unwrap().irq_entries[6], 0);
        cpu.execute_instruction(&mut bus); // successful entry, no opcode
        bus.irq = 0;
        assert_eq!(cpu.hybrid_instruction_counts().unwrap().irq_entries[6], 1);
        assert_eq!(cpu.hybrid_instruction_counts().unwrap().main_thread, 0);
        cpu.execute_instruction(&mut bus); // handler NOP
        cpu.execute_instruction(&mut bus); // handler RTE, counted before depth falls
        let counts = cpu.hybrid_instruction_counts().unwrap();
        assert_eq!(counts.exception_handler, 2);
        assert_eq!(counts.last_instruction.unwrap().exception_depth, 1);
        cpu.execute_instruction(&mut bus); // main NOP
        cpu.execute_instruction(&mut bus); // STOP is one actual opcode
        for _ in 0..8 {
            cpu.execute_instruction(&mut bus);
        }
        assert_eq!(cpu.hybrid_instruction_counts().unwrap().main_thread, 2);
        bus.reset = true;
        cpu.execute_instruction(&mut bus);
        assert_eq!(cpu.hybrid_instruction_counts().unwrap().main_thread, 2);
    }
    #[test]
    fn trap_opcode_belongs_to_its_origin_not_the_new_handler() {
        let (mut bus, mut cpu) = Bus::machine(&[0x4e40, 0x4e71]);
        cpu.hybrid_enable_instruction_counts();
        cpu.execute_instruction(&mut bus);
        let counts = cpu.hybrid_instruction_counts().unwrap();
        assert_eq!(counts.main_thread, 1);
        assert_eq!(counts.exception_handler, 0);
        assert_eq!(counts.other_exceptions, 1);
        assert_eq!(counts.last_instruction.unwrap().exception_depth, 0);
        cpu.execute_instruction(&mut bus);
        cpu.execute_instruction(&mut bus);
        assert_eq!(cpu.hybrid_instruction_counts().unwrap().exception_handler, 2);
    }
    #[cfg(feature = "bincode")]
    #[test]
    fn observation_is_absent_from_console_serialization_and_architectural_state() {
        let (mut bus, mut observed) = Bus::machine(&[0x4e71]);
        let mut ordinary = observed.clone();
        observed.hybrid_enable_instruction_counts();
        observed.execute_instruction(&mut bus);
        ordinary.execute_instruction(&mut bus);
        assert_eq!(observed.hybrid_state_words(), ordinary.hybrid_state_words());
        let config = bincode::config::standard();
        let encoded = bincode::encode_to_vec(&observed, config).unwrap();
        assert_eq!(encoded, bincode::encode_to_vec(&ordinary, config).unwrap());
        let (decoded, _): (M68000, _) = bincode::decode_from_slice(&encoded, config).unwrap();
        assert!(decoded.hybrid_instruction_counts().is_none());
    }

    #[test]
    fn faulting_jsr_and_rts_are_counted_attempts_not_successful_boundaries() {
        for words in [&[0x4eb9, 0, 0x121][..], &[0x4e75][..]] {
            let (mut bus, mut cpu) = Bus::machine(words);
            bus.write_long_word(0x8000, 0x107); // RTS target is deliberately odd
            cpu.hybrid_enable_instruction_counts();
            cpu.execute_instruction(&mut bus);
            let counts = cpu.hybrid_instruction_counts().unwrap();
            assert_eq!(counts.main_thread, 1);
            assert_eq!(counts.faulted_opcode_attempts, 1);
            assert!(!counts.last_instruction.unwrap().completed_successfully);
            assert_eq!(counts.other_exceptions, 1);
        }
    }
}
pub use traits::{BusInterface, ExceptionBoundary, ReadPurpose};

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
