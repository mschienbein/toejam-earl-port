//! Authored synthetic opcode/extension streams only; no cartridge-derived data.
use crate::{
    CompileError, compile_instruction, compile_instruction_with_divider,
    compile_instruction_with_word_reader,
};
use std::cell::Cell;

#[test]
fn every_opcode_and_divider_keeps_exact_ir_with_an_immutable_word_view() {
    const PC: u32 = 0x10;
    let mut words = [0_u16; 16];
    let mut bytes = [0_u8; 32];
    for extension in [0x0000_u16, 0xa55a] {
        words.fill(extension);
        for chunk in bytes.as_chunks_mut::<2>().0 {
            chunk.copy_from_slice(&extension.to_be_bytes());
        }
        for opcode in 0..=u16::MAX {
            words[PC as usize / 2] = opcode;
            bytes[PC as usize..PC as usize + 2].copy_from_slice(&opcode.to_be_bytes());
            let reader = |address: u32| {
                if address & 1 != 0 {
                    None
                } else {
                    words.get(address as usize / 2).copied()
                }
            };
            for divider in [1, 7, 64] {
                assert_eq!(
                    compile_instruction_with_word_reader(&reader, PC, divider),
                    compile_instruction_with_divider(&bytes, PC, divider),
                    "opcode={opcode:04x}, extension={extension:04x}, divider={divider}",
                );
            }
        }
    }
}

#[test]
fn every_synthetic_extension_truncation_keeps_exact_slice_error_and_pc() {
    let opcodes = [
        0x4e71_u16, 0x4eb9, 0x203c, 0x48e7, 0x4e72, 0x4e73, 0x6000, 0x0139, 0x23fc,
    ];
    let mut bytes = [0xa5_u8; 32];
    for pc in [0_u32, 8] {
        for opcode in opcodes {
            bytes[pc as usize..pc as usize + 2].copy_from_slice(&opcode.to_be_bytes());
            for length in 0..=bytes.len() {
                let available = &bytes[..length];
                let reader = |address: u32| {
                    available
                        .get(address as usize..address as usize + 2)
                        .map(|word| u16::from_be_bytes([word[0], word[1]]))
                };
                for divider in [1, 7, 64] {
                    assert_eq!(
                        compile_instruction_with_word_reader(&reader, pc, divider),
                        compile_instruction_with_divider(available, pc, divider),
                        "pc={pc:x}, opcode={opcode:04x}, bytes={length}, divider={divider}",
                    );
                }
            }
        }
    }
}

#[test]
fn odd_pc_rejects_before_reader_invocation_and_missing_words_never_default() {
    let calls = Cell::new(0);
    let missing = |_: u32| {
        calls.set(calls.get() + 1);
        None
    };
    for pc in [1_u32, 3, u32::MAX] {
        assert_eq!(
            compile_instruction_with_word_reader(&missing, pc, 7),
            Err(CompileError::OddPc(pc))
        );
        assert_eq!(compile_instruction(&[], pc), Err(CompileError::OddPc(pc)));
    }
    assert_eq!(calls.get(), 0);
    for pc in [0_u32, 2, 0xffff_fffe] {
        assert_eq!(
            compile_instruction_with_word_reader(&missing, pc, 7),
            Err(CompileError::Truncated(pc))
        );
        assert_eq!(
            compile_instruction(&[], pc),
            Err(CompileError::Truncated(pc))
        );
    }
    assert_eq!(calls.get(), 3);
}

#[test]
fn a_missing_second_absolute_extension_reports_its_original_address() {
    let mut bytes = [0_u8; 12];
    bytes[8..10].copy_from_slice(&0x4eb9_u16.to_be_bytes());
    bytes[10..12].copy_from_slice(&0x1234_u16.to_be_bytes());
    let reader = |address| match address {
        8 => Some(0x4eb9),
        10 => Some(0x1234),
        _ => None,
    };
    assert_eq!(
        compile_instruction_with_word_reader(&reader, 8, 7),
        Err(CompileError::Truncated(12))
    );
    assert_eq!(
        compile_instruction(&bytes, 8),
        Err(CompileError::Truncated(12))
    );
}
