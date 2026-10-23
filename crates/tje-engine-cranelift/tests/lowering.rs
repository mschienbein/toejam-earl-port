//! Synthetic IR only: no opcode tables, ROM-derived code, or private fixtures.
use tje_engine_core::{
    Bus, CpuState, InstructionIr, Program, ReadKind, StepResult,
    ir::{BinaryOp, CompareOp, ExceptionExit, Op, StateField, Value, Width},
};
use tje_engine_cranelift::{CacheConfig, CacheStatus, Error, NativeProgram};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Access {
    Read8(u32, ReadKind),
    Read16(u32, ReadKind),
    Write8(u32, u8),
    Write16(u32, u16),
    Offset(u32),
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
struct TestBus {
    accesses: Vec<Access>,
}
impl Bus for TestBus {
    fn read8(&mut self, address: u32, kind: ReadKind) -> u8 {
        self.accesses.push(Access::Read8(address, kind));
        (address ^ 0x5a) as u8
    }
    fn read16(&mut self, address: u32, kind: ReadKind) -> u16 {
        self.accesses.push(Access::Read16(address, kind));
        (address ^ 0xa55a) as u16
    }
    fn write8(&mut self, address: u32, value: u8) {
        self.accesses.push(Access::Write8(address, value));
    }
    fn write16(&mut self, address: u32, value: u16) {
        self.accesses.push(Access::Write16(address, value));
    }
    fn set_read_offset_mclk(&mut self, offset: u32) {
        self.accesses.push(Access::Offset(offset));
    }
}

fn instruction(pc: u32, value_count: u32, ops: Vec<Op>) -> InstructionIr {
    InstructionIr {
        pc,
        next_pc: pc + 2,
        opcode: 0,
        value_count,
        ops,
    }
}

fn binary_program() -> (Program, Vec<BinaryOp>) {
    let operations = vec![
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
    ];
    let instructions = operations
        .iter()
        .enumerate()
        .map(|(index, &op)| {
            instruction(
                0x100 + 2 * index as u32,
                4,
                vec![
                    Op::LoadState {
                        dst: Value(0),
                        field: StateField::Data(0),
                    },
                    Op::LoadState {
                        dst: Value(1),
                        field: StateField::Data(1),
                    },
                    Op::Binary {
                        dst: Value(2),
                        op,
                        lhs: Value(0),
                        rhs: Value(1),
                    },
                    Op::StoreState {
                        field: StateField::Data(2),
                        value: Value(2),
                    },
                    Op::Constant {
                        dst: Value(3),
                        value: 14,
                    },
                    Op::Finish { cycles: Value(3) },
                ],
            )
        })
        .collect();
    (Program { instructions }, operations)
}

fn expected_binary(operation: BinaryOp, lhs: u32, rhs: u32) -> u32 {
    match operation {
        BinaryOp::Add => lhs.wrapping_add(rhs),
        BinaryOp::Subtract => lhs.wrapping_sub(rhs),
        BinaryOp::Multiply => lhs.wrapping_mul(rhs),
        BinaryOp::And => lhs & rhs,
        BinaryOp::Or => lhs | rhs,
        BinaryOp::Xor => lhs ^ rhs,
        BinaryOp::ShiftLeft => lhs.wrapping_shl(rhs & 31),
        BinaryOp::ShiftRight => lhs.wrapping_shr(rhs & 31),
        BinaryOp::ArithmeticShiftRight => (lhs as i32).wrapping_shr(rhs & 31) as u32,
        BinaryOp::DivideUnsigned => lhs / rhs,
        BinaryOp::DivideSigned => (lhs as i32).wrapping_div(rhs as i32) as u32,
        BinaryOp::RemainderUnsigned => lhs % rhs,
        BinaryOp::RemainderSigned => (lhs as i32).wrapping_rem(rhs as i32) as u32,
    }
}

#[test]
fn lowers_all_arithmetic_with_wrapping_and_masked_shift_semantics() {
    let (program, operations) = binary_program();
    let mut native = NativeProgram::compile([0; 20], &program).unwrap();
    let samples = [
        0,
        1,
        2,
        7,
        31,
        32,
        33,
        63,
        64,
        0x7fff,
        0x8000,
        0xffff,
        0x7fff_ffff,
        0x8000_0000,
        0xffff_fffe,
        u32::MAX,
    ];
    for (index, operation) in operations.into_iter().enumerate() {
        for lhs in samples {
            for rhs in samples {
                if rhs == 0
                    && matches!(
                        operation,
                        BinaryOp::DivideUnsigned
                            | BinaryOp::DivideSigned
                            | BinaryOp::RemainderUnsigned
                            | BinaryOp::RemainderSigned
                    )
                {
                    continue;
                }
                let mut cpu = CpuState {
                    pc: program.instructions[index].pc,
                    sr: 0xa71f,
                    ..CpuState::default()
                };
                cpu.d[0] = lhs;
                cpu.d[1] = rhs;
                let before = cpu.clone();
                let mut bus = TestBus::default();
                assert_eq!(
                    native.execute(&mut cpu, &mut bus).unwrap(),
                    StepResult::normal(14)
                );
                let mut expected = before;
                expected.d[2] = expected_binary(operation, lhs, rhs);
                assert_eq!(cpu, expected, "{operation:?} {lhs:#x} {rhs:#x}");
                assert!(bus.accesses.is_empty());
            }
        }
    }
}

#[test]
fn comparisons_normalize_to_boolean_and_select_accepts_any_nonzero() {
    let comparisons = [
        CompareOp::Equal,
        CompareOp::NotEqual,
        CompareOp::UnsignedLess,
        CompareOp::UnsignedLessEqual,
        CompareOp::SignedLess,
        CompareOp::SignedLessEqual,
    ];
    let mut instructions: Vec<_> = comparisons
        .iter()
        .enumerate()
        .map(|(index, &op)| {
            instruction(
                0x100 + 2 * index as u32,
                4,
                vec![
                    Op::LoadState {
                        dst: Value(0),
                        field: StateField::Data(0),
                    },
                    Op::LoadState {
                        dst: Value(1),
                        field: StateField::Data(1),
                    },
                    Op::Compare {
                        dst: Value(2),
                        op,
                        lhs: Value(0),
                        rhs: Value(1),
                    },
                    Op::StoreState {
                        field: StateField::Data(2),
                        value: Value(2),
                    },
                    Op::Constant {
                        dst: Value(3),
                        value: 4,
                    },
                    Op::Finish { cycles: Value(3) },
                ],
            )
        })
        .collect();
    instructions.push(instruction(
        0x200,
        5,
        vec![
            Op::LoadState {
                dst: Value(0),
                field: StateField::Data(0),
            },
            Op::Constant {
                dst: Value(1),
                value: 0x1234,
            },
            Op::Constant {
                dst: Value(2),
                value: 0x5678,
            },
            Op::Select {
                dst: Value(3),
                condition: Value(0),
                if_true: Value(1),
                if_false: Value(2),
            },
            Op::StoreState {
                field: StateField::Data(1),
                value: Value(3),
            },
            Op::Constant {
                dst: Value(4),
                value: 4,
            },
            Op::Finish { cycles: Value(4) },
        ],
    ));
    let mut native = NativeProgram::compile([0; 20], &Program { instructions }).unwrap();
    for (index, comparison) in comparisons.into_iter().enumerate() {
        for lhs in [0_u32, 1, 0x7fff_ffff, 0x8000_0000, u32::MAX] {
            for rhs in [0_u32, 1, 0x7fff_ffff, 0x8000_0000, u32::MAX] {
                let mut cpu = CpuState {
                    pc: 0x100 + index as u32 * 2,
                    ..CpuState::default()
                };
                cpu.d[0] = lhs;
                cpu.d[1] = rhs;
                native.execute(&mut cpu, &mut TestBus::default()).unwrap();
                let expected = match comparison {
                    CompareOp::Equal => lhs == rhs,
                    CompareOp::NotEqual => lhs != rhs,
                    CompareOp::UnsignedLess => lhs < rhs,
                    CompareOp::UnsignedLessEqual => lhs <= rhs,
                    CompareOp::SignedLess => (lhs as i32) < rhs as i32,
                    CompareOp::SignedLessEqual => (lhs as i32) <= rhs as i32,
                };
                assert_eq!(cpu.d[2], u32::from(expected), "{comparison:?}");
            }
        }
    }
    for condition in [0, 1, 2, 0x8000_0000, u32::MAX] {
        let mut cpu = CpuState {
            pc: 0x200,
            ..CpuState::default()
        };
        cpu.d[0] = condition;
        native.execute(&mut cpu, &mut TestBus::default()).unwrap();
        assert_eq!(cpu.d[1], if condition == 0 { 0x5678 } else { 0x1234 });
    }
}

fn bus_program() -> Program {
    Program {
        instructions: vec![instruction(
            0x100,
            7,
            vec![
                Op::LoadState {
                    dst: Value(0),
                    field: StateField::Data(0),
                },
                Op::Constant {
                    dst: Value(1),
                    value: 0xc00004,
                },
                Op::Constant {
                    dst: Value(2),
                    value: 0x1234_5678,
                },
                Op::Constant {
                    dst: Value(3),
                    value: 28,
                },
                Op::SetReadOffset { cycles: Value(3) },
                Op::WriteMemory {
                    address: Value(1),
                    value: Value(2),
                    width: Width::Word,
                },
                Op::ReadMemory {
                    dst: Value(4),
                    address: Value(1),
                    width: Width::Byte,
                    kind: ReadKind::Semantic,
                },
                Op::ReadMemoryIf {
                    dst: Value(5),
                    condition: Value(0),
                    address: Value(1),
                    width: Width::Word,
                    kind: ReadKind::Prefetch,
                },
                Op::StoreState {
                    field: StateField::Data(1),
                    value: Value(5),
                },
                Op::WriteMemory {
                    address: Value(1),
                    value: Value(2),
                    width: Width::Byte,
                },
                Op::ReadMemory {
                    dst: Value(6),
                    address: Value(1),
                    width: Width::Word,
                    kind: ReadKind::Discarded,
                },
                Op::Finish { cycles: Value(3) },
            ],
        )],
    }
}

#[test]
fn bus_calls_preserve_order_width_kind_and_conditional_nonexecution() {
    let mut native = NativeProgram::compile([0; 20], &bus_program()).unwrap();
    for condition in [0, 1, u32::MAX] {
        let mut cpu = CpuState {
            pc: 0x100,
            ..CpuState::default()
        };
        cpu.d[0] = condition;
        let mut bus = TestBus::default();
        assert_eq!(native.execute(&mut cpu, &mut bus).unwrap().cycles, 28);
        let mut accesses = vec![
            Access::Offset(28),
            Access::Write16(0xc00004, 0x5678),
            Access::Read8(0xc00004, ReadKind::Semantic),
        ];
        if condition != 0 {
            accesses.push(Access::Read16(0xc00004, ReadKind::Prefetch));
        }
        accesses.extend([
            Access::Write8(0xc00004, 0x78),
            Access::Read16(0xc00004, ReadKind::Discarded),
        ]);
        assert_eq!(bus.accesses, accesses);
        assert_eq!(cpu.d[1], if condition == 0 { 0 } else { 0xa55e });
    }
}

#[test]
fn exception_exits_publish_details_before_any_later_side_effect() {
    let program = Program {
        instructions: vec![instruction(
            0x100,
            5,
            vec![
                Op::LoadState {
                    dst: Value(0),
                    field: StateField::Data(0),
                },
                Op::Constant {
                    dst: Value(1),
                    value: 50,
                },
                Op::Constant {
                    dst: Value(2),
                    value: 0x1234,
                },
                Op::Constant {
                    dst: Value(3),
                    value: 0x5679,
                },
                Op::Constant {
                    dst: Value(4),
                    value: 0xa55a,
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
                    field: StateField::Pc,
                    value: Value(2),
                },
                Op::WriteMemory {
                    address: Value(3),
                    value: Value(4),
                    width: Width::Word,
                },
                Op::Finish { cycles: Value(4) },
            ],
        )],
    };
    let mut native = NativeProgram::compile([0; 20], &program).unwrap();
    for condition in [0, 1, 0x8000_0000] {
        let mut cpu = CpuState {
            pc: 0x100,
            ..CpuState::default()
        };
        cpu.d[0] = condition;
        let mut bus = TestBus::default();
        let result = native.execute(&mut cpu, &mut bus).unwrap();
        if condition == 0 {
            assert_eq!(result, StepResult::normal(0xa55a));
            assert_eq!(cpu.pc, 0x1234);
            assert_eq!(bus.accesses, [Access::Write16(0x5679, 0xa55a)]);
        } else {
            assert_eq!(
                result,
                StepResult {
                    cycles: 50,
                    vector: 3,
                    saved_pc: 0x1234,
                    fault_address: 0x5679,
                    fault_status: 0xa55a
                }
            );
            assert_eq!(cpu.pc, 0x100);
            assert!(bus.accesses.is_empty());
        }
    }
}

#[test]
fn cached_code_reloads_and_corruption_regenerates_without_execution() {
    let directory = tempfile::tempdir().unwrap();
    let cache = CacheConfig {
        data_dir: directory.path().to_path_buf(),
    };
    let program = bus_program();
    let first = NativeProgram::load_or_compile([1; 20], &program, &cache).unwrap();
    assert_eq!(first.stats().cache_status, CacheStatus::Miss);
    let path = first.stats().cache_path.clone().unwrap();
    let mut second = NativeProgram::load_or_compile([1; 20], &program, &cache).unwrap();
    assert_eq!(second.stats().cache_status, CacheStatus::Hit);
    assert_eq!(second.stats().compilation_time, std::time::Duration::ZERO);
    let mut cpu = CpuState {
        pc: 0x100,
        ..CpuState::default()
    };
    second.execute(&mut cpu, &mut TestBus::default()).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    let index = bytes.len() - 33;
    bytes[index] ^= 0x80;
    std::fs::write(&path, &bytes).unwrap();
    let rebuilt = NativeProgram::load_or_compile([1; 20], &program, &cache).unwrap();
    assert_eq!(rebuilt.stats().cache_status, CacheStatus::Rebuilt);
    assert_eq!(rebuilt.stats().cache_rejection.as_deref(), Some("checksum"));
    let another_rom = NativeProgram::load_or_compile([2; 20], &program, &cache).unwrap();
    assert_eq!(another_rom.stats().cache_status, CacheStatus::Miss);
    assert_ne!(another_rom.stats().cache_path, Some(path));
}

#[test]
fn malformed_ir_and_missing_continuations_fail_closed() {
    let (mut program, _) = binary_program();
    let mut native = NativeProgram::compile([0; 20], &program).unwrap();
    for pc in [0, 0x101, 0x400, u32::MAX] {
        assert!(!native.contains(pc));
        assert!(
            matches!(native.execute(&mut CpuState { pc, ..CpuState::default() }, &mut TestBus::default()), Err(Error::MissingContinuation(address)) if address == pc)
        );
    }
    program.instructions[0].ops.insert(
        0,
        Op::StoreState {
            field: StateField::Data(0),
            value: Value(0),
        },
    );
    assert!(matches!(
        NativeProgram::compile([0; 20], &program),
        Err(Error::InvalidIr { .. })
    ));
    program.instructions.reverse();
    assert!(matches!(
        NativeProgram::compile([0; 20], &program),
        Err(Error::InvalidIr { .. })
    ));
}

#[test]
fn canonical_state_layout_and_all_field_offsets_match_generated_accesses() {
    use std::mem::{offset_of, size_of};
    assert_eq!(size_of::<CpuState>(), 100);
    assert_eq!(offset_of!(CpuState, d), 0);
    assert_eq!(offset_of!(CpuState, a), 32);
    assert_eq!(offset_of!(CpuState, usp), 60);
    assert_eq!(offset_of!(CpuState, last_muldiv), 96);
    let fields: Vec<_> = (0..8)
        .map(StateField::Data)
        .chain((0..7).map(StateField::Address))
        .chain([
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
        ])
        .collect();
    let program = Program {
        instructions: fields
            .iter()
            .enumerate()
            .map(|(index, &field)| {
                instruction(
                    0x100 + index as u32 * 2,
                    3,
                    vec![
                        Op::Constant {
                            dst: Value(0),
                            value: 0x1234_0000 + index as u32,
                        },
                        Op::StoreState {
                            field,
                            value: Value(0),
                        },
                        Op::LoadState {
                            dst: Value(1),
                            field,
                        },
                        Op::StoreState {
                            field: StateField::Data(0),
                            value: Value(1),
                        },
                        Op::Constant {
                            dst: Value(2),
                            value: 4,
                        },
                        Op::Finish { cycles: Value(2) },
                    ],
                )
            })
            .collect(),
    };
    let mut native = NativeProgram::compile([0; 20], &program).unwrap();
    for (index, field) in fields.into_iter().enumerate() {
        let mut cpu = CpuState {
            pc: 0x100 + index as u32 * 2,
            ..CpuState::default()
        };
        native.execute(&mut cpu, &mut TestBus::default()).unwrap();
        let actual = match field {
            StateField::Data(n) => cpu.d[n as usize],
            StateField::Address(n) => cpu.a[n as usize],
            StateField::Usp => cpu.usp,
            StateField::Ssp => cpu.ssp,
            StateField::Pc => cpu.pc,
            StateField::Sr => cpu.sr,
            StateField::Prefetch => cpu.prefetch,
            StateField::PendingInterrupt => cpu.pending_interrupt,
            StateField::Stopped => cpu.stopped,
            StateField::Frozen => cpu.frozen,
            StateField::AddressError => cpu.address_error,
            StateField::LastMulDiv => cpu.last_muldiv,
        };
        assert_eq!(actual, 0x1234_0000 + index as u32);
        assert_eq!(cpu.d[0], actual);
    }
}

#[test]
fn code_publication_survives_loading_on_another_thread_and_cache_reuse() {
    let directory = tempfile::tempdir().unwrap();
    let cache = CacheConfig {
        data_dir: directory.path().to_path_buf(),
    };
    for value in [0x1234_5678, 0x8765_4321] {
        let program = Program {
            instructions: vec![instruction(
                0x100,
                2,
                vec![
                    Op::Constant {
                        dst: Value(0),
                        value,
                    },
                    Op::StoreState {
                        field: StateField::Data(0),
                        value: Value(0),
                    },
                    Op::Constant {
                        dst: Value(1),
                        value: 4,
                    },
                    Op::Finish { cycles: Value(1) },
                ],
            )],
        };
        for expected_status in [CacheStatus::Miss, CacheStatus::Hit] {
            let cache = cache.clone();
            let program = program.clone();
            let mut native = std::thread::spawn(move || {
                let mut native = NativeProgram::load_or_compile([4; 20], &program, &cache).unwrap();
                assert_eq!(native.stats().cache_status, expected_status);
                let mut cpu = CpuState {
                    pc: 0x100,
                    ..CpuState::default()
                };
                assert_eq!(
                    native.execute(&mut cpu, &mut TestBus::default()).unwrap(),
                    StepResult::normal(4)
                );
                assert_eq!(cpu.d[0], value);
                native
            })
            .join()
            .unwrap();
            let mut cpu = CpuState {
                pc: 0x100,
                ..CpuState::default()
            };
            assert_eq!(
                native.execute(&mut cpu, &mut TestBus::default()).unwrap(),
                StepResult::normal(4)
            );
            assert_eq!(cpu.d[0], value);
        }
    }
}
