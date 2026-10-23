use tje_engine_core::{
    Bus, CpuState, InstructionIr, ReadKind, StepResult,
    ir::{BinaryOp, CompareOp, ExceptionExit, Op, StateField, Value, Width},
};
use tje_engine_verify::interpreter::Interpreter;

#[derive(Default)]
struct AuthoredBus {
    events: Vec<(char, u32, u32, ReadKind)>,
}
impl Bus for AuthoredBus {
    fn read8(&mut self, address: u32, kind: ReadKind) -> u8 {
        self.events.push(('r', address, 8, kind));
        0x81
    }
    fn read16(&mut self, address: u32, kind: ReadKind) -> u16 {
        self.events.push(('r', address, 16, kind));
        0x8123
    }
    fn write8(&mut self, address: u32, value: u8) {
        self.events
            .push(('b', address, u32::from(value), ReadKind::Semantic));
    }
    fn write16(&mut self, address: u32, value: u16) {
        self.events
            .push(('w', address, u32::from(value), ReadKind::Semantic));
    }
    fn set_read_offset_mclk(&mut self, offset: u32) {
        self.events.push(('o', offset, 0, ReadKind::Semantic));
    }
}
fn ir(ops: Vec<Op>, value_count: u32) -> InstructionIr {
    InstructionIr {
        pc: 0x100,
        next_pc: 0x102,
        opcode: 0,
        value_count,
        ops,
    }
}

#[test]
fn arithmetic_wraps_and_signed_boundaries_match_ir_contract() {
    use BinaryOp::*;
    let cases = [
        (Add, u32::MAX, 1, 0),
        (Subtract, 0, 1, u32::MAX),
        (Multiply, 0x80000000, 2, 0),
        (And, 0xf0, 0x38, 0x30),
        (Or, 0xf0, 0x38, 0xf8),
        (Xor, 0xf0, 0x38, 0xc8),
        (ShiftLeft, 1, 33, 2),
        (ShiftRight, 0x80000000, 63, 1),
        (ArithmeticShiftRight, 0x80000000, 31, u32::MAX),
        (DivideUnsigned, u32::MAX, 2, 0x7fffffff),
        (DivideSigned, 0x80000000, u32::MAX, 0x80000000),
        (RemainderUnsigned, u32::MAX, 2, 1),
        (RemainderSigned, 0x80000000, u32::MAX, 0),
    ];
    let mut interpreter = Interpreter::default();
    for (op, lhs, rhs, expected) in cases {
        let instruction = ir(
            vec![
                Op::Constant {
                    dst: Value(0),
                    value: lhs,
                },
                Op::Constant {
                    dst: Value(1),
                    value: rhs,
                },
                Op::Binary {
                    dst: Value(2),
                    op,
                    lhs: Value(0),
                    rhs: Value(1),
                },
                Op::StoreState {
                    field: StateField::Data(0),
                    value: Value(2),
                },
                Op::Constant {
                    dst: Value(3),
                    value: 8,
                },
                Op::Finish { cycles: Value(3) },
            ],
            4,
        );
        let mut state = CpuState::default();
        assert_eq!(
            interpreter
                .evaluate(&instruction, &mut state, &mut AuthoredBus::default())
                .unwrap(),
            StepResult::normal(8)
        );
        assert_eq!(state.d[0], expected, "{op:?}");
    }
}

#[test]
fn comparisons_and_select_use_signedness_and_nonzero_conditions() {
    use CompareOp::*;
    for (op, expected) in [
        (Equal, 0),
        (NotEqual, 1),
        (UnsignedLess, 0),
        (UnsignedLessEqual, 0),
        (SignedLess, 1),
        (SignedLessEqual, 1),
    ] {
        let instruction = ir(
            vec![
                Op::Constant {
                    dst: Value(0),
                    value: u32::MAX,
                },
                Op::Constant {
                    dst: Value(1),
                    value: 2,
                },
                Op::Compare {
                    dst: Value(2),
                    op,
                    lhs: Value(0),
                    rhs: Value(1),
                },
                Op::Select {
                    dst: Value(3),
                    condition: Value(2),
                    if_true: Value(1),
                    if_false: Value(0),
                },
                Op::StoreState {
                    field: StateField::Data(1),
                    value: Value(3),
                },
                Op::Finish { cycles: Value(2) },
            ],
            4,
        );
        let mut state = CpuState::default();
        let result = Interpreter::default()
            .evaluate(&instruction, &mut state, &mut AuthoredBus::default())
            .unwrap();
        assert_eq!(result.cycles, expected);
        assert_eq!(state.d[1], if expected == 0 { u32::MAX } else { 2 });
    }
}

#[test]
fn bus_order_width_and_read_purpose_survive_conditional_reads() {
    let instruction = ir(
        vec![
            Op::Constant {
                dst: Value(0),
                value: 4,
            },
            Op::Constant {
                dst: Value(1),
                value: 0,
            },
            Op::Constant {
                dst: Value(2),
                value: 1,
            },
            Op::ReadMemoryIf {
                dst: Value(3),
                condition: Value(1),
                address: Value(0),
                width: Width::Word,
                kind: ReadKind::Prefetch,
            },
            Op::ReadMemory {
                dst: Value(4),
                address: Value(0),
                width: Width::Byte,
                kind: ReadKind::Discarded,
            },
            Op::ReadMemoryIf {
                dst: Value(5),
                condition: Value(2),
                address: Value(0),
                width: Width::Word,
                kind: ReadKind::Prefetch,
            },
            Op::WriteMemory {
                address: Value(0),
                value: Value(5),
                width: Width::Byte,
            },
            Op::WriteMemory {
                address: Value(0),
                value: Value(4),
                width: Width::Word,
            },
            Op::SetReadOffset { cycles: Value(0) },
            Op::Finish { cycles: Value(3) },
        ],
        6,
    );
    let mut bus = AuthoredBus::default();
    let result = Interpreter::default()
        .evaluate(&instruction, &mut CpuState::default(), &mut bus)
        .unwrap();
    assert_eq!(result.cycles, 0);
    assert_eq!(
        bus.events,
        vec![
            ('r', 4, 8, ReadKind::Discarded),
            ('r', 4, 16, ReadKind::Prefetch),
            ('b', 4, 0x23, ReadKind::Semantic),
            ('w', 4, 0x81, ReadKind::Semantic),
            ('o', 4, 0, ReadKind::Semantic)
        ]
    );
}

#[test]
fn exception_exit_does_not_execute_later_effects() {
    let instruction = ir(
        vec![
            Op::Constant {
                dst: Value(0),
                value: 1,
            },
            Op::Constant {
                dst: Value(1),
                value: 38,
            },
            Op::Constant {
                dst: Value(2),
                value: 0x102,
            },
            Op::Constant {
                dst: Value(3),
                value: 0xff0001,
            },
            Op::Constant {
                dst: Value(4),
                value: 0x15,
            },
            Op::ExitIf {
                condition: Value(0),
                exit: ExceptionExit {
                    vector: 3,
                    cycles: Value(1),
                    saved_pc: Value(2),
                    fault_address: Value(3),
                    fault_status: Value(4),
                },
            },
            Op::StoreState {
                field: StateField::Data(0),
                value: Value(0),
            },
            Op::Finish { cycles: Value(0) },
        ],
        5,
    );
    let mut state = CpuState::default();
    let result = Interpreter::default()
        .evaluate(&instruction, &mut state, &mut AuthoredBus::default())
        .unwrap();
    assert_eq!(
        result,
        StepResult {
            cycles: 38,
            vector: 3,
            saved_pc: 0x102,
            fault_address: 0xff0001,
            fault_status: 0x15
        }
    );
    assert_eq!(state.d[0], 0);
}
