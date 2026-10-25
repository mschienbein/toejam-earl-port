//! Authored SSA streams only; no cartridge data or instruction listings.
use super::*;
use crate::ir::{BinaryOp, ExceptionExit, Op, Value};

fn stream<const N: usize>(value_count: u32, body: [Op; N]) -> InstructionIr {
    InstructionIr {
        pc: 0x200,
        next_pc: 0x202,
        opcode: 0x4e71,
        value_count,
        ops: std::iter::once(Op::Constant {
            dst: Value(0),
            value: 4,
        })
        .chain(body)
        .chain(std::iter::once(Op::Finish { cycles: Value(0) }))
        .collect(),
    }
}
fn before(instruction: &InstructionIr) -> CpuState {
    CpuState {
        pc: instruction.pc,
        prefetch: u32::from(instruction.opcode),
        ssp: 0xff4000,
        usp: 0xff6000,
        sr: 0x2700,
        ..CpuState::default()
    }
}
fn bind(
    instruction: &InstructionIr,
    before: &CpuState,
    reads: &[ObservedRead],
    scratch: &mut DependencyScratch,
    report: &mut DependencyReport,
) -> Result<(), DependencyError> {
    analyze_instruction(
        instruction,
        DependencyInput::Executed {
            before,
            reads,
            addresses: ObservedAddressDomain::Raw32,
        },
        scratch,
        report,
    )
}

#[test]
fn aligned_stack_arithmetic_has_precise_low_bit_sources_without_an_opcode_rule() {
    let instruction = stream(
        4,
        [
            Op::LoadState {
                dst: Value(1),
                field: StateField::Ssp,
            },
            Op::Constant {
                dst: Value(2),
                value: 4,
            },
            Op::Binary {
                dst: Value(3),
                op: BinaryOp::Subtract,
                lhs: Value(1),
                rhs: Value(2),
            },
            Op::StoreState {
                field: StateField::Ssp,
                value: Value(3),
            },
        ],
    );
    let mut scratch = DependencyScratch::default();
    let mut report = DependencyReport::default();
    bind(
        &instruction,
        &before(&instruction),
        &[],
        &mut scratch,
        &mut report,
    )
    .unwrap();
    for (bits, expected) in [(1, 1), (2, 3), (0x80000000, u32::MAX)] {
        let output = scratch.output_reads(StateField::Ssp, bits).unwrap();
        assert_eq!(output.reads, StateMask::single(StateField::Ssp, expected));
        assert!(output.control_reads.is_empty());
        assert!(!output.memory_values);
        assert_eq!(
            (output.pc, output.opcode),
            (instruction.pc, instruction.opcode)
        );
        assert!(matches!(
            output.binding,
            PathBinding::ExecutedReads { consumed: 0, .. }
        ));
    }
    assert_eq!(
        scratch.output_reads(StateField::Ssp, 0).unwrap().reads,
        StateMask::EMPTY
    );
    for index in 0..StateField::COUNT {
        let field = StateField::from_index(index).unwrap();
        if field != StateField::Ssp {
            assert_eq!(
                scratch.output_reads(field, 0x80).unwrap().reads,
                StateMask::single(field, 0x80),
                "unchanged canonical domain {field:?}"
            );
        }
    }
}

#[test]
fn executed_selection_retains_its_controls_and_rebinds_to_the_new_actual_input() {
    let instruction = stream(
        5,
        [
            Op::LoadState {
                dst: Value(1),
                field: StateField::Ssp,
            },
            Op::LoadState {
                dst: Value(2),
                field: StateField::Usp,
            },
            Op::LoadState {
                dst: Value(3),
                field: StateField::Data(0),
            },
            Op::Select {
                dst: Value(4),
                condition: Value(3),
                if_true: Value(1),
                if_false: Value(2),
            },
            Op::StoreState {
                field: StateField::Ssp,
                value: Value(4),
            },
        ],
    );
    let mut scratch = DependencyScratch::default();
    let mut report = DependencyReport::default();
    for selected in [false, true, false] {
        let mut input = before(&instruction);
        input.d[0] = u32::from(selected);
        bind(&instruction, &input, &[], &mut scratch, &mut report).unwrap();
        let output = scratch.output_reads(StateField::Ssp, 1).unwrap();
        let controls = StateMask::single(StateField::Data(0), u32::MAX);
        assert_eq!(output.control_reads, controls);
        assert_eq!(
            output.reads,
            controls
                | StateMask::single(
                    if selected {
                        StateField::Ssp
                    } else {
                        StateField::Usp
                    },
                    1
                )
        );
        assert!(!output.memory_values);
    }
}

#[test]
fn the_single_actual_early_exit_uses_its_binding_not_the_unreachable_normal_store() {
    let instruction = stream(
        5,
        [
            Op::LoadState {
                dst: Value(1),
                field: StateField::Data(0),
            },
            Op::LoadState {
                dst: Value(2),
                field: StateField::Data(1),
            },
            Op::LoadState {
                dst: Value(3),
                field: StateField::Data(2),
            },
            Op::Constant {
                dst: Value(4),
                value: 0,
            },
            Op::StoreState {
                field: StateField::Ssp,
                value: Value(1),
            },
            Op::ExitIf {
                condition: Value(2),
                exit: ExceptionExit {
                    vector: 5,
                    cycles: Value(0),
                    saved_pc: Value(4),
                    fault_address: Value(4),
                    fault_status: Value(4),
                },
            },
            Op::StoreState {
                field: StateField::Ssp,
                value: Value(3),
            },
        ],
    );
    let mut scratch = DependencyScratch::default();
    let mut report = DependencyReport::default();
    for taken in [true, false] {
        let mut input = before(&instruction);
        input.d[1] = u32::from(taken);
        bind(&instruction, &input, &[], &mut scratch, &mut report).unwrap();
        let output = scratch.output_reads(StateField::Ssp, 1).unwrap();
        let controls = StateMask::single(StateField::Data(1), u32::MAX);
        assert_eq!(output.control_reads, controls);
        assert_eq!(
            output.reads,
            controls | StateMask::single(StateField::Data(if taken { 0 } else { 2 }), 1)
        );
        assert_eq!(
            report.instruction.obligations.exception_taken,
            taken.then_some(5)
        );
    }
    analyze_instruction(
        &instruction,
        DependencyInput::Conservative,
        &mut scratch,
        &mut report,
    )
    .unwrap();
    assert_eq!(
        scratch.output_reads(StateField::Ssp, 1),
        Err(DependencyError::OutputQueryUnavailable),
        "never substitute only the normal binding for several permitted exits"
    );
    analyze_instruction(
        &instruction,
        DependencyInput::Entry(&before(&instruction)),
        &mut scratch,
        &mut report,
    )
    .unwrap();
    assert_eq!(
        scratch.output_reads(StateField::Ssp, 1),
        Err(DependencyError::OutputQueryUnavailable)
    );
}

#[test]
fn observed_memory_values_and_memory_path_controls_remain_explicit_obligations() {
    let memory_value = stream(
        3,
        [
            Op::LoadState {
                dst: Value(1),
                field: StateField::Address(0),
            },
            Op::ReadMemory {
                dst: Value(2),
                address: Value(1),
                width: Width::Word,
                kind: ReadKind::Semantic,
            },
            Op::StoreState {
                field: StateField::Ssp,
                value: Value(2),
            },
        ],
    );
    let mut input = before(&memory_value);
    input.a[0] = 0xff9000;
    let reads = [ObservedRead {
        address: input.a[0],
        width: Width::Word,
        kind: ReadKind::Semantic,
        value: 0,
    }];
    let mut scratch = DependencyScratch::default();
    let mut report = DependencyReport::default();
    bind(&memory_value, &input, &reads, &mut scratch, &mut report).unwrap();
    let output = scratch.output_reads(StateField::Ssp, 1).unwrap();
    assert!(output.reads.is_empty());
    assert!(
        output.memory_values,
        "observed zero is not a constant or a causality proof"
    );
    assert!(
        !scratch
            .output_reads(StateField::Ssp, 0)
            .unwrap()
            .memory_values
    );

    let memory_control = stream(
        4,
        [
            Op::LoadState {
                dst: Value(1),
                field: StateField::Address(0),
            },
            Op::ReadMemory {
                dst: Value(2),
                address: Value(1),
                width: Width::Word,
                kind: ReadKind::Semantic,
            },
            Op::Constant {
                dst: Value(3),
                value: 7,
            },
            Op::ExitIf {
                condition: Value(2),
                exit: ExceptionExit {
                    vector: 5,
                    cycles: Value(0),
                    saved_pc: Value(0),
                    fault_address: Value(0),
                    fault_status: Value(0),
                },
            },
            Op::StoreState {
                field: StateField::Ssp,
                value: Value(3),
            },
        ],
    );
    bind(&memory_control, &input, &reads, &mut scratch, &mut report).unwrap();
    assert!(
        scratch
            .output_reads(StateField::Ssp, 1)
            .unwrap()
            .memory_values,
        "a memory-selected path remains a memory obligation even for a constant output"
    );
}

#[test]
fn failed_and_nonexecuted_analyses_clear_every_previous_output_query_binding() {
    let instruction = stream(
        2,
        [
            Op::LoadState {
                dst: Value(1),
                field: StateField::Data(0),
            },
            Op::StoreState {
                field: StateField::Ssp,
                value: Value(1),
            },
        ],
    );
    let input = before(&instruction);
    let mut scratch = DependencyScratch::default();
    let mut report = DependencyReport::default();
    assert_eq!(
        scratch.output_reads(StateField::Ssp, 1),
        Err(DependencyError::OutputQueryUnavailable)
    );
    for failure in 0..7 {
        bind(&instruction, &input, &[], &mut scratch, &mut report).unwrap();
        assert_eq!(
            scratch.output_reads(StateField::Ssp, 1).unwrap().reads,
            StateMask::single(StateField::Data(0), 1)
        );
        match failure {
            0 => {
                let mut wrong = before(&instruction);
                wrong.pc += 2;
                assert_eq!(
                    bind(&instruction, &wrong, &[], &mut scratch, &mut report),
                    Err(DependencyError::EntryMismatch)
                );
            }
            1 => {
                let extra = [ObservedRead {
                    address: 0,
                    width: Width::Word,
                    kind: ReadKind::Semantic,
                    value: 0,
                }];
                assert!(matches!(
                    bind(&instruction, &input, &extra, &mut scratch, &mut report),
                    Err(DependencyError::ExtraReads { .. })
                ));
            }
            2 => {
                let mut invalid = instruction.clone();
                invalid.ops.pop();
                assert!(matches!(
                    bind(&invalid, &input, &[], &mut scratch, &mut report),
                    Err(DependencyError::InvalidIr(_))
                ));
            }
            3 => analyze_instruction(
                &instruction,
                DependencyInput::Conservative,
                &mut scratch,
                &mut report,
            )
            .unwrap(),
            4 => analyze_instruction(
                &instruction,
                DependencyInput::Entry(&input),
                &mut scratch,
                &mut report,
            )
            .unwrap(),
            5 | 6 => {
                let rom = [0x4e, 0x71];
                let (bytes, pc) = if failure == 5 {
                    (&[][..], 0)
                } else {
                    (&rom[..], 1)
                };
                assert!(matches!(
                    analyze_rom_instruction(
                        bytes,
                        pc,
                        DependencyInput::Executed {
                            before: &input,
                            reads: &[],
                            addresses: ObservedAddressDomain::Raw32
                        },
                        &mut scratch,
                        &mut report
                    ),
                    Err(DependencyError::Compile(_))
                ));
            }
            _ => unreachable!(),
        }
        assert_eq!(
            scratch.output_reads(StateField::Ssp, 1),
            Err(DependencyError::OutputQueryUnavailable)
        );
        assert_eq!(
            scratch.output_reads(StateField::Ssp, 0),
            Err(DependencyError::OutputQueryUnavailable)
        );
    }
}

#[test]
fn invalid_output_fields_neither_alias_a_canonical_bank_nor_panic() {
    let instruction = stream(1, []);
    let mut scratch = DependencyScratch::default();
    let mut report = DependencyReport::default();
    for bound in [false, true] {
        if bound {
            bind(
                &instruction,
                &before(&instruction),
                &[],
                &mut scratch,
                &mut report,
            )
            .unwrap();
        }
        for field in [
            StateField::Data(8),
            StateField::Data(255),
            StateField::Address(7),
            StateField::Address(255),
        ] {
            assert!(matches!(
                scratch.output_reads(field, 1),
                Err(DependencyError::InvalidIr(_))
            ));
        }
    }
    assert_eq!(
        scratch.output_reads(StateField::Data(0), 1).unwrap().reads,
        StateMask::single(StateField::Data(0), 1),
        "rejected queries do not mutate the bound SSA"
    );
}
