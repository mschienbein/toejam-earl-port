use super::*;
use tje_engine_core::ir::ExceptionExit;
use tje_engine_core::{CpuState, ExceptionOut, ReadKind};

fn single(ops: Vec<Op>, value_count: u32) -> Program {
    Program {
        instructions: vec![InstructionIr {
            pc: 0x100,
            next_pc: 0x102,
            opcode: 0,
            value_count,
            ops,
        }],
    }
}
fn constant(dst: u32, value: u32) -> Op {
    Op::Constant {
        dst: Value(dst),
        value,
    }
}

#[test]
fn every_primitive_encodes_as_valid_wasm() {
    let mut ops = vec![
        constant(0, 0x8000_0000),
        constant(1, u32::MAX),
        constant(2, 1),
    ];
    let mut next = 3;
    for op in [
        BinaryOp::Add,
        BinaryOp::Subtract,
        BinaryOp::Multiply,
        BinaryOp::And,
        BinaryOp::Or,
        BinaryOp::Xor,
        BinaryOp::ShiftLeft,
        BinaryOp::ShiftRight,
        BinaryOp::ArithmeticShiftRight,
        BinaryOp::DivideUnsigned,
        BinaryOp::DivideSigned,
        BinaryOp::RemainderUnsigned,
        BinaryOp::RemainderSigned,
    ] {
        ops.push(Op::Binary {
            dst: Value(next),
            op,
            lhs: Value(0),
            rhs: Value(1),
        });
        next += 1;
    }
    for op in [
        CompareOp::Equal,
        CompareOp::NotEqual,
        CompareOp::UnsignedLess,
        CompareOp::UnsignedLessEqual,
        CompareOp::SignedLess,
        CompareOp::SignedLessEqual,
    ] {
        ops.push(Op::Compare {
            dst: Value(next),
            op,
            lhs: Value(0),
            rhs: Value(1),
        });
        next += 1;
    }
    ops.push(Op::Select {
        dst: Value(next),
        condition: Value(2),
        if_true: Value(0),
        if_false: Value(1),
    });
    next += 1;
    for field in [
        StateField::Data(7),
        StateField::Address(6),
        StateField::Usp,
        StateField::Ssp,
        StateField::Pc,
        StateField::Sr,
        StateField::Prefetch,
        StateField::PendingInterrupt,
        StateField::Stopped,
        StateField::Frozen,
        StateField::AddressError,
        StateField::LastMulDiv,
    ] {
        ops.push(Op::LoadState {
            dst: Value(next),
            field,
        });
        ops.push(Op::StoreState {
            field,
            value: Value(next),
        });
        next += 1;
    }
    for width in [Width::Byte, Width::Word] {
        for kind in [ReadKind::Semantic, ReadKind::Prefetch, ReadKind::Discarded] {
            ops.push(Op::ReadMemory {
                dst: Value(next),
                address: Value(0),
                width,
                kind,
            });
            next += 1;
            ops.push(Op::ReadMemoryIf {
                dst: Value(next),
                condition: Value(2),
                address: Value(0),
                width,
                kind,
            });
            next += 1;
        }
        ops.push(Op::WriteMemory {
            address: Value(0),
            value: Value(1),
            width,
        });
    }
    ops.push(Op::SetReadOffset { cycles: Value(1) });
    ops.push(Op::ExitIf {
        condition: Value(2),
        exit: ExceptionExit {
            vector: 3,
            cycles: Value(1),
            saved_pc: Value(0),
            fault_address: Value(0),
            fault_status: Value(1),
        },
    });
    ops.push(Op::Finish { cycles: Value(2) });
    let module = generate(&single(ops, next)).unwrap();
    wasmparser::validate(&module.bytes).unwrap();
    assert_eq!(module.instruction_count, 1);
    assert_eq!(module.table_slots, 0x81);
    assert!(module.largest_function_bytes < MAX_FUNCTION_BYTES);
}

#[test]
fn state_and_exception_layouts_match_the_numeric_abi() {
    assert_eq!(std::mem::size_of::<CpuState>(), 25 * 4);
    assert_eq!(
        std::mem::offset_of!(CpuState, d),
        StateField::Data(0).offset() as usize
    );
    assert_eq!(
        std::mem::offset_of!(CpuState, a),
        StateField::Address(0).offset() as usize
    );
    assert_eq!(
        std::mem::offset_of!(CpuState, usp),
        StateField::Usp.offset() as usize
    );
    assert_eq!(
        std::mem::offset_of!(CpuState, ssp),
        StateField::Ssp.offset() as usize
    );
    assert_eq!(
        std::mem::offset_of!(CpuState, pc),
        StateField::Pc.offset() as usize
    );
    assert_eq!(
        std::mem::offset_of!(CpuState, sr),
        StateField::Sr.offset() as usize
    );
    assert_eq!(
        std::mem::offset_of!(CpuState, prefetch),
        StateField::Prefetch.offset() as usize
    );
    assert_eq!(
        std::mem::offset_of!(CpuState, last_muldiv),
        StateField::LastMulDiv.offset() as usize
    );
    assert_eq!(std::mem::size_of::<ExceptionOut>(), 12);
    assert_eq!(std::mem::offset_of!(ExceptionOut, saved_pc), 0);
    assert_eq!(std::mem::offset_of!(ExceptionOut, fault_address), 4);
    assert_eq!(std::mem::offset_of!(ExceptionOut, status_word), 8);
}

#[test]
fn invalid_ir_fails_before_encoding() {
    assert!(matches!(
        generate(&Program::default()),
        Err(GenerateError::EmptyProgram)
    ));
    let valid = single(vec![constant(0, 4), Op::Finish { cycles: Value(0) }], 1);
    let mut program = valid.clone();
    program.instructions[0].pc |= 1;
    assert!(matches!(
        generate(&program),
        Err(GenerateError::InvalidPc(_))
    ));
    program.instructions[0].pc = MAX_CODE_ADDRESS;
    assert!(matches!(
        generate(&program),
        Err(GenerateError::InvalidPc(_))
    ));
    let mut program = valid.clone();
    program.instructions.push(program.instructions[0].clone());
    assert!(matches!(
        generate(&program),
        Err(GenerateError::UnsortedPc(_))
    ));
    for (ops, count) in [
        (vec![Op::Finish { cycles: Value(0) }], 1),
        (
            vec![
                constant(0, 4),
                constant(0, 5),
                Op::Finish { cycles: Value(0) },
            ],
            1,
        ),
        (vec![constant(1, 4), Op::Finish { cycles: Value(1) }], 1),
        (vec![constant(0, 4)], 1),
        (
            vec![
                constant(0, 4),
                Op::Finish { cycles: Value(0) },
                Op::Finish { cycles: Value(0) },
            ],
            1,
        ),
        (
            vec![
                Op::LoadState {
                    dst: Value(0),
                    field: StateField::Data(8),
                },
                Op::Finish { cycles: Value(0) },
            ],
            1,
        ),
        (
            vec![
                Op::LoadState {
                    dst: Value(0),
                    field: StateField::Address(7),
                },
                Op::Finish { cycles: Value(0) },
            ],
            1,
        ),
        (
            vec![constant(0, 4), Op::Finish { cycles: Value(0) }],
            MAX_VALUES + 1,
        ),
    ] {
        assert!(matches!(
            generate(&single(ops, count)),
            Err(GenerateError::InvalidIr { .. })
        ));
    }
}

#[test]
fn emission_is_reproducible_and_does_not_define_linear_memory() {
    let program = single(vec![constant(0, 4), Op::Finish { cycles: Value(0) }], 1);
    let first = generate(&program).unwrap();
    let second = generate(&program).unwrap();
    assert_eq!(first.bytes, second.bytes);
    for payload in wasmparser::Parser::new(0).parse_all(&first.bytes) {
        assert!(!matches!(
            payload.unwrap(),
            wasmparser::Payload::MemorySection(_)
                | wasmparser::Payload::DataSection(_)
                | wasmparser::Payload::StartSection { .. }
        ));
    }
}
