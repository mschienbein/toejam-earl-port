//! Authored ISA fixtures matching the development verifier's native test matrix.
//! No instruction words or initial states in this module come from a game ROM.
use m68000_emu::{BusInterface, M68000, ReadPurpose, debug::DummyM68000Debugger};
use tje_engine_core::{Bus, CpuState, Program, ReadKind, compile_instruction};
use tje_engine_runtime::Backend;
use wasm_bindgen::prelude::*;

use super::{Metrics, failure};
use crate::{BrowserBackend, browser::now};

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
            let [high, low] = value.to_be_bytes();
            self.bytes[offset] = high;
            self.bytes[(offset + 1) & 0xffff] = low;
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

/// Independent reference-vs-compiled sampled states, including every initial CCR
/// combination and operand boundaries. The emitted code is actually instantiated
/// in Chromium and called through the application's Wasm function table.
#[wasm_bindgen]
pub async fn browser_verify_instructions() -> Result<String, JsValue> {
    let cases: &[(&str, &[u16])] = &[
        ("moveq-negative", &[0x70ff]),
        ("addq-word", &[0x5240]),
        ("subq-byte", &[0x5300]),
        ("subq-word", &[0x5340]),
        ("add-long", &[0xd081]),
        ("subtract-long", &[0x9081]),
        ("and-word", &[0xc041]),
        ("or-byte", &[0x8001]),
        ("compare-long", &[0xb081]),
        ("exclusive-or-word", &[0xb340]),
        ("multiply-unsigned-immediate", &[0xc0fc, 7]),
        ("multiply-signed-immediate", &[0xc1fc, 0x8001]),
        ("divide-unsigned-immediate", &[0x80fc, 7]),
        ("divide-signed-immediate", &[0x81fc, 7]),
        ("logical-shift-left-word", &[0xe368]),
        ("arithmetic-shift-right-word", &[0xe260]),
        ("rotate-right-long", &[0xe2b8]),
        ("decrement-branch-false", &[0x51c8, 4]),
        ("branch-not-equal", &[0x6604]),
        ("branch-minus", &[0x6b04]),
        ("move-postincrement-word", &[0x3018]),
        ("move-predecrement-word", &[0x3100]),
        ("clear-indirect-word", &[0x4250]),
        ("movem-predecrement-long", &[0x48e7, 0xe000]),
        ("movem-postincrement-long", &[0x4cd8, 7]),
        ("jump-subroutine-absolute", &[0x4eb9, 0, 0x0400]),
        ("branch-subroutine", &[0x6104]),
        ("bit-test-indirect", &[0x0310]),
    ];
    let mut bytes = vec![0; 65_536];
    for chunk in bytes.chunks_exact_mut(2) {
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
            .map(|case| compile_instruction(&bytes, 0x100 + 16 * case as u32))
            .collect::<Result<_, _>>()
            .map_err(failure)?,
    };
    let mut backend = BrowserBackend::uncached(&program).await.map_err(failure)?;
    let template = Memory {
        bytes,
        events: Vec::new(),
    };
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
    let mut samples = 0_u64;
    let start = now();
    for (instruction, (name, _)) in program.instructions.iter().zip(cases) {
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
                state.d[1] = value.rotate_left(7) ^ value.wrapping_mul(17);
                let (mut reference_memory, mut compiled_memory) =
                    (template.clone(), template.clone());
                let mut reference = M68000::builder().allow_tas_writes(false).build();
                reference.set_data_registers(state.d);
                reference.set_address_registers(state.a, state.usp, state.ssp);
                reference.set_status_register(state.sr as u16);
                reference.set_pc(state.pc, &mut reference_memory);
                reference_memory.events.clear();
                let result = backend
                    .execute(&mut state, &mut compiled_memory)
                    .map_err(failure)?;
                let cycles = reference.execute_instruction(&mut reference_memory);
                if result.vector != 0
                    || result.cycles != cycles
                    || state.d != reference.data_registers()
                    || state.a != reference.address_registers()
                    || state.usp != reference.user_stack_pointer()
                    || state.ssp != reference.supervisor_stack_pointer()
                    || state.pc != reference.pc()
                    || state.sr as u16 != reference.status_register()
                    || state.prefetch as u16 != reference.next_opcode()
                    || (state.last_muldiv != 0) != reference.last_instruction_was_mul_or_div()
                    || compiled_memory.events != reference_memory.events
                    || compiled_memory.bytes != reference_memory.bytes
                {
                    return Err(failure(format!(
                        "compiled/reference {name} mismatch: PC={:06x} value={value:08x} CCR={ccr:02x}; compiled={state:?}/{result:?}, reference D={:?} A={:?} SR={:04x} PC={:06x} cycles={cycles}",
                        instruction.pc,
                        reference.data_registers(),
                        reference.address_registers(),
                        reference.status_register(),
                        reference.pc()
                    )));
                }
                samples += 1;
            }
        }
    }
    serde_json::to_string(&serde_json::json!({ "cases": samples,
        "functions": program.instructions.len(), "generation": Metrics::from(backend.metrics()),
        "execution_ms": now() - start }))
    .map_err(failure)
}
