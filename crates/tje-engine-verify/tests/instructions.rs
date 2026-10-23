//! Authored 68000 programs, not extracted game instructions. Compare independent
//! semantics plus ordered physical reads/writes and base charges for each case.
use m68000_emu::{BusInterface, M68000, ReadPurpose, debug::DummyM68000Debugger};
use tje_engine_core::{Bus, CpuState, ReadKind, compile_instruction, ir::Program};
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
use tje_engine_cranelift::NativeProgram;
use tje_engine_verify::interpreter::Interpreter;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    Read(u32, u8, ReadKind, u16),
    Write(u32, u8, u16),
}
#[derive(Clone)]
struct Memory {
    bytes: Vec<u8>,
    events: Vec<Event>,
}
impl Memory {
    fn read(&mut self, address: u32, width: u8, kind: ReadKind) -> u16 {
        let offset = address as usize & 0xffff;
        let value = if width == 8 {
            u16::from(self.bytes[offset])
        } else {
            u16::from_be_bytes([self.bytes[offset], self.bytes[(offset + 1) & 0xffff]])
        };
        self.events.push(Event::Read(address, width, kind, value));
        value
    }
    fn write(&mut self, address: u32, width: u8, value: u16) {
        let offset = address as usize & 0xffff;
        if width == 8 {
            self.bytes[offset] = value as u8;
        } else {
            let [hi, lo] = value.to_be_bytes();
            self.bytes[offset] = hi;
            self.bytes[(offset + 1) & 0xffff] = lo;
        }
        self.events.push(Event::Write(address, width, value));
    }
}
impl Bus for Memory {
    fn read8(&mut self, address: u32, kind: ReadKind) -> u8 {
        self.read(address, 8, kind) as u8
    }
    fn read16(&mut self, address: u32, kind: ReadKind) -> u16 {
        self.read(address, 16, kind)
    }
    fn write8(&mut self, address: u32, value: u8) {
        self.write(address, 8, value.into());
    }
    fn write16(&mut self, address: u32, value: u16) {
        self.write(address, 16, value);
    }
    fn set_read_offset_mclk(&mut self, _: u32) {}
}
fn kind(purpose: ReadPurpose) -> ReadKind {
    match purpose {
        ReadPurpose::Semantic => ReadKind::Semantic,
        ReadPurpose::Prefetch => ReadKind::Prefetch,
        ReadPurpose::Discarded => ReadKind::Discarded,
    }
}
impl BusInterface for Memory {
    type DebugView<'a> = DummyM68000Debugger;
    fn read_byte(&mut self, address: u32) -> u8 {
        self.read(address, 8, ReadKind::Semantic) as u8
    }
    fn read_word(&mut self, address: u32) -> u16 {
        self.read(address, 16, ReadKind::Semantic)
    }
    fn read_byte_tagged(&mut self, address: u32, purpose: ReadPurpose) -> u8 {
        self.read(address, 8, kind(purpose)) as u8
    }
    fn read_word_tagged(&mut self, address: u32, purpose: ReadPurpose) -> u16 {
        self.read(address, 16, kind(purpose))
    }
    fn write_byte(&mut self, address: u32, value: u8) {
        self.write(address, 8, value.into());
    }
    fn write_word(&mut self, address: u32, value: u16) {
        self.write(address, 16, value);
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
fn ir_and_reference_match_authored_instruction_boundaries() {
    let cases: &[(&str, &[u16])] = &[
        ("MOVEQ #-1,D0", &[0x70ff]),
        ("ADDQ.W #1,D0", &[0x5240]),
        ("SUBQ.B #1,D0", &[0x5300]),
        ("SUBQ.W #1,D0", &[0x5340]),
        ("ADD.L D1,D0", &[0xd081]),
        ("SUB.L D1,D0", &[0x9081]),
        ("AND.W D1,D0", &[0xc041]),
        ("OR.B D1,D0", &[0x8001]),
        ("CMP.L D1,D0", &[0xb081]),
        ("EOR.W D1,D0", &[0xb340]),
        ("MULU.W #7,D0", &[0xc0fc, 7]),
        ("MULS.W #$8001,D0", &[0xc1fc, 0x8001]),
        ("DIVU.W #7,D0", &[0x80fc, 7]),
        ("DIVS.W #7,D0", &[0x81fc, 7]),
        ("LSL.W D1,D0", &[0xe368]),
        ("ASR.W D1,D0", &[0xe260]),
        ("ROR.L D1,D0", &[0xe2b8]),
        ("DBF D0,+4", &[0x51c8, 4]),
        ("BNE +4", &[0x6604]),
        ("BMI +4", &[0x6b04]),
        ("MOVE.W (A0)+,D0", &[0x3018]),
        ("MOVE.W D0,-(A0)", &[0x3100]),
        ("CLR.W (A0)", &[0x4250]),
        ("MOVEM.L D0-D2,-(A7)", &[0x48e7, 0xe000]),
        ("MOVEM.L (A0)+,D0-D2", &[0x4cd8, 7]),
        ("JSR $400.L", &[0x4eb9, 0, 0x0400]),
        ("BSR +4", &[0x6104]),
        ("BTST D1,(A0)", &[0x0310]),
    ];
    let mut bytes = vec![0; 65536];
    for chunk in bytes.as_chunks_mut::<2>().0 {
        chunk.copy_from_slice(&0x4e71_u16.to_be_bytes());
    }
    for (case, (_, words)) in cases.iter().enumerate() {
        for (offset, word) in words.iter().enumerate() {
            let pc = 0x100 + 16 * case + 2 * offset;
            bytes[pc..pc + 2].copy_from_slice(&word.to_be_bytes());
        }
    }
    let program = Program {
        instructions: (0..cases.len())
            .map(|case| compile_instruction(&bytes, 0x100 + 16 * case as u32).unwrap())
            .collect(),
    };
    #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
    let mut native = NativeProgram::compile([0; 20], &program).unwrap();
    let template = Memory {
        bytes,
        events: Vec::new(),
    };
    let mut interpreter = Interpreter::default();
    let values = [
        0,
        1,
        2,
        7,
        15,
        31,
        32,
        63,
        0x7f,
        0x80,
        0x7fff,
        0x8000,
        0xffff,
        0x7fff_ffff,
        0x8000_0000,
        u32::MAX,
    ];
    for (case, instruction) in program.instructions.iter().enumerate() {
        for value in values {
            for ccr in 0..32 {
                let mut state = CpuState {
                    d: [value; 8],
                    a: [0x2000; 7],
                    usp: 0x8000,
                    ssp: 0x8000,
                    pc: instruction.pc,
                    sr: 0x2700 | ccr,
                    prefetch: instruction.opcode.into(),
                    ..CpuState::default()
                };
                // Different source/count register; includes count0 and >31.
                state.d[1] = value.rotate_left(7) ^ value.wrapping_mul(17);
                let (mut reference_memory, mut ir_memory) = (template.clone(), template.clone());
                #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
                let mut native_memory = template.clone();
                let mut reference = M68000::builder().allow_tas_writes(false).build();
                reference.set_data_registers(state.d);
                reference.set_address_registers(state.a, state.usp, state.ssp);
                reference.set_status_register(state.sr as u16);
                reference.set_pc(state.pc, &mut reference_memory);
                reference_memory.events.clear();
                #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
                let mut compiled_state = state.clone();
                let ir_result = interpreter
                    .evaluate(instruction, &mut state, &mut ir_memory)
                    .unwrap();
                #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
                let native_result = native
                    .execute(&mut compiled_state, &mut native_memory)
                    .unwrap();
                let cycles = reference.execute_instruction(&mut reference_memory);
                let context = format!(
                    "{} pc={:06x} value={value:08x} ccr={ccr:02x}",
                    cases[case].0, instruction.pc
                );
                assert_eq!(ir_result.vector, 0, "{context}");
                #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
                assert_eq!(native_result, ir_result, "{context}");
                #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
                assert_eq!(compiled_state, state, "{context}");
                assert_eq!(state.d, reference.data_registers(), "{context}");
                assert_eq!(state.a, reference.address_registers(), "{context}");
                assert_eq!(state.usp, reference.user_stack_pointer(), "{context}");
                assert_eq!(state.ssp, reference.supervisor_stack_pointer(), "{context}");
                assert_eq!(state.pc, reference.pc(), "{context}");
                assert_eq!(state.sr as u16, reference.status_register(), "{context}");
                assert_eq!(state.prefetch as u16, reference.next_opcode(), "{context}");
                assert_eq!(
                    state.last_muldiv != 0,
                    reference.last_instruction_was_mul_or_div(),
                    "{context}"
                );
                assert_eq!(ir_result.cycles, cycles, "{context}");
                #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
                assert_eq!(native_memory.events, ir_memory.events, "{context}");
                assert_eq!(ir_memory.events, reference_memory.events, "{context}");
                #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
                assert_eq!(native_memory.bytes, ir_memory.bytes, "{context}");
                assert_eq!(ir_memory.bytes, reference_memory.bytes, "{context}");
            }
        }
    }
}

#[test]
fn short_index_offsets_wrap_before_address_sign_extension() {
    for words in [
        vec![0xeb40_u16, 0xd0c0, 0x4e71],
        vec![0xc1fc_u16, 32, 0xd0c0, 0x4e71],
    ] {
        let mut bytes = vec![0; 65536];
        for (index, word) in words.iter().enumerate() {
            bytes[0x100 + 2 * index..0x102 + 2 * index].copy_from_slice(&word.to_be_bytes());
        }
        let first = compile_instruction(&bytes, 0x100).unwrap();
        let second = compile_instruction(&bytes, first.next_pc).unwrap();
        let program = Program {
            instructions: vec![first, second],
        };
        #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
        let mut native = NativeProgram::compile([0; 20], &program).unwrap();
        for index in [512_i32, -512, 1024, -1024, 0x7fff, -0x8000] {
            let mut state = CpuState {
                d: [0; 8],
                a: [0; 7],
                usp: 0x8000,
                ssp: 0x8000,
                pc: 0x100,
                sr: 0x2700,
                prefetch: words[0].into(),
                ..CpuState::default()
            };
            state.d[0] = index as u32;
            state.a[0] = 0xffc000;
            let mut reference = M68000::builder().allow_tas_writes(false).build();
            let mut reference_memory = Memory {
                bytes: bytes.clone(),
                events: Vec::new(),
            };
            reference.set_data_registers(state.d);
            reference.set_address_registers(state.a, state.usp, state.ssp);
            reference.set_status_register(state.sr as u16);
            reference.set_pc(state.pc, &mut reference_memory);
            reference_memory.events.clear();
            let mut ir_memory = reference_memory.clone();
            #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
            let (mut compiled_state, mut compiled_memory) =
                (state.clone(), reference_memory.clone());
            let mut interpreter = Interpreter::default();
            for instruction in &program.instructions {
                let result = interpreter
                    .evaluate(instruction, &mut state, &mut ir_memory)
                    .unwrap();
                let cycles = reference.execute_instruction(&mut reference_memory);
                assert_eq!(result.cycles, cycles);
                assert_eq!(state.d, reference.data_registers());
                assert_eq!(state.a, reference.address_registers());
                assert_eq!(state.sr as u16, reference.status_register());
                assert_eq!(ir_memory.events, reference_memory.events);
                #[cfg(all(feature = "native", not(target_arch = "wasm32")))]
                {
                    let compiled = native
                        .execute(&mut compiled_state, &mut compiled_memory)
                        .unwrap();
                    assert_eq!(compiled, result);
                    assert_eq!(compiled_state, state);
                    assert_eq!(compiled_memory.events, ir_memory.events);
                }
            }
            let offset = (index as u16).wrapping_mul(32) as i16 as u32;
            assert_eq!(
                state.a[0],
                0xffc000_u32.wrapping_add(offset),
                "index={index}"
            );
        }
    }
}
