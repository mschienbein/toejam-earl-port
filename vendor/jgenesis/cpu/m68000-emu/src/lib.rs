#[cfg(any(test, feature = "memorybus"))]
pub mod bus;
mod core;
pub mod debug;
pub mod disassemble;
pub mod traits;

pub use crate::core::{
    AddressRegister, DataRegister, HybridExecutedInstruction, HybridInstructionCounts,
    HybridTrapEntryError, IndexRegister, IndexSize, M68000, OpSize, cycles_if_move_btst_cmp,
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

#[cfg(test)]
mod hybrid_trap_entry_tests {
    use crate::debug::DummyM68000Debugger;
    use crate::{BusInterface, ExceptionBoundary, HybridTrapEntryError, M68000, ReadPurpose};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Access {
        Read(u32, u16, ReadPurpose),
        Write(u32, u16),
        Boundary(ExceptionBoundary),
    }
    #[derive(Clone)]
    struct Bus {
        bytes: Vec<u8>,
        accesses: Vec<Access>,
    }
    impl Bus {
        fn raw_word(&self, address: u32) -> u16 {
            u16::from_be_bytes([self.bytes[address as usize], self.bytes[address as usize + 1]])
        }
        fn put_word(&mut self, address: u32, value: u16) {
            self.bytes[address as usize..address as usize + 2]
                .copy_from_slice(&value.to_be_bytes());
        }
        fn setup(status: u16, ssp: u32, handler: u32) -> (M68000, Self) {
            let mut bus = Self { bytes: vec![0; 0x10000], accesses: Vec::new() };
            // Authored ISA-only DIVU/NOP/spin fixtures; no cartridge payload.
            bus.put_word(0x100, 0x82c2);
            bus.put_word(0x102, 0x4e71);
            bus.put_word(0x300, 0x60fe);
            bus.put_word(0x400, 0x60fe);
            bus.put_word(20, (handler >> 16) as u16);
            bus.put_word(22, handler as u16);
            bus.put_word(12, 0);
            bus.put_word(14, 0x400);
            let mut cpu = M68000::default();
            cpu.set_status_register(status);
            cpu.set_data_registers([0x12345678, 0xaa55, 0, 3, 4, 5, 6, 7]);
            cpu.set_address_registers([0x1111, 0x2222, 3, 4, 5, 6, 7], 0x7000, ssp);
            cpu.set_pc(0x100, &mut bus);
            cpu.hybrid_enable_instruction_counts();
            bus.accesses.clear();
            (cpu, bus)
        }
    }
    impl BusInterface for Bus {
        type DebugView<'a> = DummyM68000Debugger;
        fn read_byte(&mut self, address: u32) -> u8 {
            self.bytes[address as usize]
        }
        fn read_word(&mut self, address: u32) -> u16 {
            self.read_word_tagged(address, ReadPurpose::Semantic)
        }
        fn read_word_tagged(&mut self, address: u32, purpose: ReadPurpose) -> u16 {
            let value = self.raw_word(address);
            self.accesses.push(Access::Read(address, value, purpose));
            value
        }
        fn write_byte(&mut self, address: u32, value: u8) {
            self.bytes[address as usize] = value;
        }
        fn write_word(&mut self, address: u32, value: u16) {
            self.accesses.push(Access::Write(address, value));
            self.put_word(address, value);
        }
        fn interrupt_level(&self) -> u8 {
            0
        }
        fn acknowledge_interrupt(&mut self, _: u8) {
            panic!("synchronous trap must not acknowledge IRQ");
        }
        fn halt(&self) -> bool {
            false
        }
        fn reset(&self) -> bool {
            false
        }
        fn observe_exception_boundary(&mut self, boundary: ExceptionBoundary) {
            self.accesses.push(Access::Boundary(boundary));
        }
    }
    fn realize(cpu: &mut M68000, bus: &mut Bus) -> Result<(), HybridTrapEntryError> {
        // The Rust source produces DIVU-zero CCR, then its exact next-word
        // prefetch. This does NOT decode/execute or count an original opcode.
        cpu.set_status_register(cpu.status_register() & !0xf);
        cpu.set_pc(0x102, bus);
        cpu.hybrid_enter_trap(5, 0x102, 0x82c2, bus)
    }
    #[test]
    fn actual_vector_frame_prefetch_and_all_25_cpu_words_match_real_divu_fault() {
        for status in [0x201f, 0x001f, 0xa71f] {
            let (mut original, mut original_bus) = Bus::setup(status, 0x8000, 0x300);
            let mut rust = original.clone();
            let mut rust_bus = original_bus.clone();
            original.execute_instruction(&mut original_bus);
            realize(&mut rust, &mut rust_bus).unwrap();
            assert_eq!(rust.hybrid_state_words(), original.hybrid_state_words());
            assert_eq!(rust.hybrid_exception_depth(), 1);
            assert_eq!(rust.hybrid_last_exception_vector(), Some(5));
            assert_eq!(rust_bus.bytes, original_bus.bytes);
            assert_eq!(rust_bus.accesses, original_bus.accesses);
            assert_eq!(rust.pc(), 0x300);
            assert_eq!(rust.supervisor_stack_pointer(), 0x7ffa);
            assert_eq!(rust_bus.raw_word(0x7ffa), status & !0xf);
            assert_eq!(rust_bus.raw_word(0x7ffc), 0);
            assert_eq!(rust_bus.raw_word(0x7ffe), 0x102);
            assert_eq!(rust.hybrid_instruction_counts().unwrap().main_thread, 0);
            assert_eq!(rust.hybrid_instruction_counts().unwrap().faulted_opcode_attempts, 0);
            assert_eq!(original.hybrid_instruction_counts().unwrap().faulted_opcode_attempts, 1);
        }
    }
    #[test]
    fn odd_inactive_supervisor_stack_keeps_real_double_fault_without_success() {
        let (mut original, mut original_bus) = Bus::setup(0x001f, 0x8001, 0x300);
        let mut rust = original.clone();
        let mut rust_bus = original_bus.clone();
        original.execute_instruction(&mut original_bus);
        assert!(matches!(
            realize(&mut rust, &mut rust_bus),
            Err(HybridTrapEntryError::EntryFault { frozen: true, .. })
        ));
        assert_eq!(rust.hybrid_state_words(), original.hybrid_state_words());
        assert_eq!(rust_bus.bytes, original_bus.bytes);
        assert_eq!(rust_bus.accesses, original_bus.accesses);
        assert_eq!(rust.hybrid_exception_depth(), original.hybrid_exception_depth());
    }
    #[test]
    fn odd_handler_runs_real_address_entry_but_never_claims_standard_trap_success() {
        let (mut original, mut original_bus) = Bus::setup(0x201f, 0x8000, 0x301);
        let mut rust = original.clone();
        let mut rust_bus = original_bus.clone();
        original.execute_instruction(&mut original_bus);
        assert!(matches!(
            realize(&mut rust, &mut rust_bus),
            Err(HybridTrapEntryError::EntryFault { last_vector: Some(3), frozen: false })
        ));
        assert_eq!(rust.hybrid_state_words(), original.hybrid_state_words());
        assert_eq!(rust_bus.bytes, original_bus.bytes);
        assert_eq!(rust_bus.accesses, original_bus.accesses);
    }
    #[test]
    fn prior_vector_latch_does_not_claim_a_new_entry_after_double_fault() {
        let (mut seed, mut bus) = Bus::setup(0x001f, 0x8000, 0x300);
        bus.put_word(0x300, 0x4e73); // Authored real RTE from the first trap.
        seed.execute_instruction(&mut bus);
        seed.execute_instruction(&mut bus);
        assert_eq!(seed.hybrid_exception_depth(), 0);
        assert_eq!(seed.hybrid_last_exception_vector(), Some(5));
        seed.set_status_register(0x001f);
        seed.set_address_registers(seed.address_registers(), 0x7000, 0x8001);
        seed.set_pc(0x100, &mut bus);
        bus.accesses.clear();
        let mut original = seed.clone();
        let mut original_bus = bus.clone();
        original.execute_instruction(&mut original_bus);
        assert!(matches!(
            realize(&mut seed, &mut bus),
            Err(HybridTrapEntryError::EntryFault { last_vector: Some(5), frozen: true })
        ));
        assert_eq!(seed.hybrid_state_words(), original.hybrid_state_words());
        assert_eq!(bus.bytes, original_bus.bytes);
        assert_eq!(bus.accesses, original_bus.accesses);
        assert!(
            !bus.accesses
                .iter()
                .any(|event| matches!(event, Access::Boundary(ExceptionBoundary::Entered { .. })))
        );
    }
    #[test]
    fn unsupported_vector_or_unprepared_pc_is_rejected_without_effects() {
        let (mut cpu, mut bus) = Bus::setup(0x201f, 0x8000, 0x300);
        let before = cpu.hybrid_state_words();
        let bytes = bus.bytes.clone();
        assert_eq!(
            cpu.hybrid_enter_trap(3, 0x100, 0x82c2, &mut bus),
            Err(HybridTrapEntryError::UnsupportedVector(3))
        );
        assert_eq!(
            cpu.hybrid_enter_trap(5, 0x102, 0x82c2, &mut bus),
            Err(HybridTrapEntryError::InvalidFaultContext)
        );
        assert_eq!(cpu.hybrid_state_words(), before);
        assert_eq!(bus.bytes, bytes);
        assert!(bus.accesses.is_empty());
    }
}
