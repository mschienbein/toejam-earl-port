//! Authored SSA/physical-read streams only; no cartridge data or execution.
use super::*;
use crate::ir::{BinaryOp, CompareOp, ExceptionExit, Op, Value};

fn instruction(count: u32, body: impl IntoIterator<Item = Op>) -> InstructionIr {
    InstructionIr {
        pc: 0x200,
        next_pc: 0x202,
        opcode: 0x4e71,
        value_count: count,
        ops: std::iter::once(Op::Constant {
            dst: Value(0),
            value: 4,
        })
        .chain(body)
        .chain(std::iter::once(Op::Finish { cycles: Value(0) }))
        .collect(),
    }
}
fn before(ir: &InstructionIr) -> CpuState {
    CpuState {
        pc: ir.pc,
        prefetch: u32::from(ir.opcode),
        sr: 0x2000,
        ..CpuState::default()
    }
}
fn bind(
    ir: &InstructionIr,
    reads: &[ObservedRead],
    addresses: ObservedAddressDomain,
    scratch: &mut DependencyScratch,
) -> DependencyReport {
    let mut report = DependencyReport::default();
    analyze_instruction(
        ir,
        DependencyInput::Executed {
            before: &before(ir),
            reads,
            addresses,
        },
        scratch,
        &mut report,
    )
    .unwrap();
    report
}
fn read(address: u32, width: Width, kind: ReadKind, value: u32) -> ObservedRead {
    ObservedRead {
        address,
        width,
        kind,
        value,
    }
}
fn simple_word() -> InstructionIr {
    instruction(
        3,
        [
            Op::Constant {
                dst: Value(1),
                value: 0xff7000,
            },
            Op::ReadMemory {
                dst: Value(2),
                address: Value(1),
                width: Width::Word,
                kind: ReadKind::Semantic,
            },
            Op::StoreState {
                field: StateField::Data(0),
                value: Value(2),
            },
        ],
    )
}

#[test]
fn identical_addresses_keep_physical_read_ordinals_widths_and_purposes() {
    let ir = instruction(
        7,
        [
            Op::Constant {
                dst: Value(1),
                value: 0xff7000,
            },
            Op::ReadMemory {
                dst: Value(2),
                address: Value(1),
                width: Width::Word,
                kind: ReadKind::Prefetch,
            },
            Op::ReadMemory {
                dst: Value(3),
                address: Value(1),
                width: Width::Word,
                kind: ReadKind::Semantic,
            },
            Op::ReadMemory {
                dst: Value(4),
                address: Value(1),
                width: Width::Word,
                kind: ReadKind::Discarded,
            },
            Op::Constant {
                dst: Value(5),
                value: 0xff,
            },
            Op::Binary {
                dst: Value(6),
                op: BinaryOp::And,
                lhs: Value(3),
                rhs: Value(5),
            },
            Op::StoreState {
                field: StateField::Data(0),
                value: Value(6),
            },
        ],
    );
    let reads = [
        read(0xff7000, Width::Word, ReadKind::Prefetch, 0xa533),
        read(0xff7000, Width::Word, ReadKind::Semantic, 0xa533),
        read(0xff7000, Width::Word, ReadKind::Discarded, 0xa533),
    ];
    let mut scratch = DependencyScratch::default();
    bind(&ir, &reads, ObservedAddressDomain::Raw32, &mut scratch);
    let mut leaves = Vec::new();
    let old = scratch.output_reads(StateField::Data(0), u32::MAX).unwrap();
    let new = scratch
        .output_read_demands(StateField::Data(0), u32::MAX, &mut leaves)
        .unwrap();
    assert_eq!(old, new, "old summary semantics remain unchanged");
    assert_eq!(
        leaves,
        [ExecutedReadDemand {
            ordinal: 1,
            address: 0xff7000,
            width: Width::Word,
            kind: ReadKind::Semantic,
            data_bits: 0xff,
            control_bits: 0
        }]
    );
    scratch
        .value_read_demands(Value(4), u32::MAX, false, &mut leaves)
        .unwrap();
    assert_eq!(
        (leaves[0].ordinal, leaves[0].kind, leaves[0].data_bits),
        (2, ReadKind::Discarded, 0xffff)
    );
    scratch
        .value_read_demands(Value(2), u32::MAX, true, &mut leaves)
        .unwrap();
    assert_eq!(
        (
            leaves[0].ordinal,
            leaves[0].kind,
            leaves[0].data_bits,
            leaves[0].control_bits
        ),
        (0, ReadKind::Prefetch, 0, 0xffff)
    );
}

#[test]
fn word_load_then_byte_extension_discards_only_the_dead_high_byte() {
    let ir = instruction(
        6,
        [
            Op::Constant {
                dst: Value(1),
                value: 0xff7000,
            },
            Op::ReadMemory {
                dst: Value(2),
                address: Value(1),
                width: Width::Word,
                kind: ReadKind::Semantic,
            },
            Op::Constant {
                dst: Value(3),
                value: 24,
            },
            Op::Binary {
                dst: Value(4),
                op: BinaryOp::ShiftLeft,
                lhs: Value(2),
                rhs: Value(3),
            },
            Op::Binary {
                dst: Value(5),
                op: BinaryOp::ArithmeticShiftRight,
                lhs: Value(4),
                rhs: Value(3),
            },
            Op::StoreState {
                field: StateField::Data(0),
                value: Value(5),
            },
        ],
    );
    let mut scratch = DependencyScratch::default();
    let mut leaves = Vec::new();
    for value in [0xab7f, 0xcd7f, 0xab80, 0xab01, 0xab02] {
        let reads = [read(0xff7000, Width::Word, ReadKind::Semantic, value)];
        bind(&ir, &reads, ObservedAddressDomain::Raw32, &mut scratch);
        scratch
            .output_read_demands(StateField::Data(0), u32::MAX, &mut leaves)
            .unwrap();
        assert_eq!(
            (leaves.len(), leaves[0].data_bits, leaves[0].control_bits),
            (1, 0xff, 0)
        );
        scratch
            .value_read_demands(Value(2), 0xffff, false, &mut leaves)
            .unwrap();
        assert_eq!(
            leaves[0].data_bits, 0xffff,
            "an earlier store/use still retains the high byte"
        );
        scratch
            .value_read_demands(Value(2), 0xff00, true, &mut leaves)
            .unwrap();
        assert_eq!((leaves[0].data_bits, leaves[0].control_bits), (0, 0xff00));
    }
}

#[test]
fn memory_selected_path_controls_are_inseparable_from_data_queries() {
    let ir = instruction(
        8,
        [
            Op::Constant {
                dst: Value(1),
                value: 0xff7000,
            },
            Op::ReadMemory {
                dst: Value(2),
                address: Value(1),
                width: Width::Byte,
                kind: ReadKind::Semantic,
            },
            Op::Constant {
                dst: Value(3),
                value: 0,
            },
            Op::Compare {
                dst: Value(4),
                op: CompareOp::Equal,
                lhs: Value(2),
                rhs: Value(3),
            },
            Op::Constant {
                dst: Value(5),
                value: 1,
            },
            Op::Constant {
                dst: Value(6),
                value: 2,
            },
            Op::Select {
                dst: Value(7),
                condition: Value(4),
                if_true: Value(5),
                if_false: Value(6),
            },
            Op::StoreState {
                field: StateField::Data(0),
                value: Value(7),
            },
        ],
    );
    let mut scratch = DependencyScratch::default();
    let mut leaves = Vec::new();
    for value in [0, 1, 255] {
        bind(
            &ir,
            &[read(0xff7000, Width::Byte, ReadKind::Semantic, value)],
            ObservedAddressDomain::Raw32,
            &mut scratch,
        );
        scratch
            .output_read_demands(StateField::Data(0), u32::MAX, &mut leaves)
            .unwrap();
        assert_eq!((leaves[0].data_bits, leaves[0].control_bits), (0, 255));
        scratch
            .value_read_demands(Value(5), u32::MAX, false, &mut leaves)
            .unwrap();
        assert_eq!(
            (leaves[0].data_bits, leaves[0].control_bits),
            (0, 255),
            "even a constant root retains the actual path controls"
        );
    }
}

#[test]
fn guarded_off_reads_have_no_physical_ordinal_or_placeholder_leaf() {
    let ir = instruction(
        6,
        [
            Op::Constant {
                dst: Value(1),
                value: 0xff7000,
            },
            Op::ReadMemory {
                dst: Value(2),
                address: Value(1),
                width: Width::Byte,
                kind: ReadKind::Semantic,
            },
            Op::Constant {
                dst: Value(3),
                value: 0xff7002,
            },
            Op::ReadMemoryIf {
                dst: Value(4),
                condition: Value(2),
                address: Value(3),
                width: Width::Word,
                kind: ReadKind::Semantic,
            },
            Op::Constant {
                dst: Value(5),
                value: 9,
            },
            Op::StoreState {
                field: StateField::Data(0),
                value: Value(5),
            },
        ],
    );
    let mut scratch = DependencyScratch::default();
    let mut leaves = Vec::new();
    bind(
        &ir,
        &[read(0xff7000, Width::Byte, ReadKind::Semantic, 0)],
        ObservedAddressDomain::Raw32,
        &mut scratch,
    );
    scratch
        .value_read_demands(Value(4), u32::MAX, false, &mut leaves)
        .unwrap();
    assert_eq!(leaves.len(), 1);
    assert_eq!(
        (
            leaves[0].ordinal,
            leaves[0].data_bits,
            leaves[0].control_bits
        ),
        (0, 0, 255)
    );
    bind(
        &ir,
        &[
            read(0xff7000, Width::Byte, ReadKind::Semantic, 1),
            read(0xff7002, Width::Word, ReadKind::Semantic, 0x1234),
        ],
        ObservedAddressDomain::Raw32,
        &mut scratch,
    );
    scratch
        .value_read_demands(Value(4), u32::MAX, false, &mut leaves)
        .unwrap();
    assert_eq!(
        leaves
            .iter()
            .map(|leaf| (leaf.ordinal, leaf.data_bits, leaf.control_bits))
            .collect::<Vec<_>>(),
        [(0, 0, 255), (1, 0xffff, 0)]
    );
}

#[test]
fn generic_roots_must_belong_to_the_selected_executed_prefix() {
    let ir = instruction(
        6,
        [
            Op::Constant {
                dst: Value(1),
                value: 1,
            },
            Op::Constant {
                dst: Value(2),
                value: 0x200,
            },
            Op::ExitIf {
                condition: Value(1),
                exit: ExceptionExit {
                    vector: 5,
                    cycles: Value(0),
                    saved_pc: Value(2),
                    fault_address: Value(2),
                    fault_status: Value(2),
                },
            },
            Op::Constant {
                dst: Value(3),
                value: 0xff7000,
            },
            Op::ReadMemory {
                dst: Value(4),
                address: Value(3),
                width: Width::Word,
                kind: ReadKind::Semantic,
            },
            Op::Constant {
                dst: Value(5),
                value: 9,
            },
            Op::StoreState {
                field: StateField::Data(0),
                value: Value(5),
            },
        ],
    );
    let mut scratch = DependencyScratch::default();
    bind(&ir, &[], ObservedAddressDomain::Raw32, &mut scratch);
    let mut leaves = Vec::new();
    for value in [Value(3), Value(4), Value(5), Value(99)] {
        leaves.push(ExecutedReadDemand {
            ordinal: 999,
            address: 0,
            width: Width::Byte,
            kind: ReadKind::Semantic,
            data_bits: 255,
            control_bits: 0,
        });
        assert!(
            scratch
                .value_read_demands(value, u32::MAX, false, &mut leaves)
                .is_err()
        );
        assert!(leaves.is_empty());
    }
    scratch
        .value_read_demands(Value(1), u32::MAX, false, &mut leaves)
        .unwrap();
    assert!(leaves.is_empty());
}

#[test]
fn exact_address_policy_and_physical_word_alignment_are_retained() {
    let mut ir = instruction(
        3,
        [
            Op::Constant {
                dst: Value(1),
                value: 0xffff9000,
            },
            Op::ReadMemory {
                dst: Value(2),
                address: Value(1),
                width: Width::Word,
                kind: ReadKind::Semantic,
            },
            Op::StoreState {
                field: StateField::Data(0),
                value: Value(2),
            },
        ],
    );
    let reads = [read(0xff9000, Width::Word, ReadKind::Semantic, 0x1234)];
    let mut scratch = DependencyScratch::default();
    let mut leaves = Vec::new();
    bind(
        &ir,
        &reads,
        ObservedAddressDomain::M68000Bus24,
        &mut scratch,
    );
    let summary = scratch
        .output_read_demands(StateField::Data(0), u32::MAX, &mut leaves)
        .unwrap();
    assert_eq!(leaves[0].address, 0xff9000);
    assert!(matches!(
        summary.binding,
        PathBinding::ExecutedReads {
            consumed: 1,
            addresses: ObservedAddressDomain::M68000Bus24
        }
    ));
    assert!(
        analyze_instruction(
            &ir,
            DependencyInput::Executed {
                before: &before(&ir),
                reads: &reads,
                addresses: ObservedAddressDomain::Raw32
            },
            &mut scratch,
            &mut DependencyReport::default()
        )
        .is_err()
    );
    assert!(
        scratch
            .output_read_demands(StateField::Data(0), u32::MAX, &mut leaves)
            .is_err()
    );
    assert!(leaves.is_empty());
    ir.ops[1] = Op::Constant {
        dst: Value(1),
        value: 0xffff9001,
    };
    assert!(
        analyze_instruction(
            &ir,
            DependencyInput::Executed {
                before: &before(&ir),
                reads: &reads,
                addresses: ObservedAddressDomain::M68000Bus24,
            },
            &mut scratch,
            &mut DependencyReport::default()
        )
        .is_err(),
        "bus24 policy must not silently align a distinct odd SSA EA"
    );
    assert!(
        scratch
            .value_read_demands(Value(2), u32::MAX, false, &mut leaves)
            .is_err()
    );
    assert!(leaves.is_empty());
}

#[test]
fn repeated_queries_reset_data_control_and_unwritten_field_leaves() {
    let ir = simple_word();
    let mut scratch = DependencyScratch::default();
    let mut leaves = Vec::new();
    bind(
        &ir,
        &[read(0xff7000, Width::Word, ReadKind::Semantic, 0x1234)],
        ObservedAddressDomain::Raw32,
        &mut scratch,
    );
    scratch
        .value_read_demands(Value(2), u32::MAX, true, &mut leaves)
        .unwrap();
    assert_eq!((leaves[0].data_bits, leaves[0].control_bits), (0, 0xffff));
    scratch
        .output_read_demands(StateField::Data(0), u32::MAX, &mut leaves)
        .unwrap();
    assert_eq!((leaves[0].data_bits, leaves[0].control_bits), (0xffff, 0));
    scratch
        .output_read_demands(StateField::Data(1), u32::MAX, &mut leaves)
        .unwrap();
    assert!(
        leaves.is_empty(),
        "no stale leaves from the preceding output"
    );
    scratch
        .output_read_demands(StateField::Data(0), u32::MAX, &mut leaves)
        .unwrap();
    assert!(
        scratch
            .output_read_demands(StateField::Data(8), u32::MAX, &mut leaves)
            .is_err()
    );
    assert!(leaves.is_empty());
    assert!(
        scratch
            .value_read_demands(Value(99), u32::MAX, false, &mut leaves)
            .is_err()
    );
    assert!(leaves.is_empty());
}

#[test]
fn analysis_and_compile_errors_invalidate_both_query_bindings() {
    let ir = simple_word();
    let reads = [read(0xff7000, Width::Word, ReadKind::Semantic, 0x1234)];
    let mut scratch = DependencyScratch::default();
    let mut leaves = Vec::new();
    for failure in 0..5 {
        bind(&ir, &reads, ObservedAddressDomain::Raw32, &mut scratch);
        scratch
            .output_read_demands(StateField::Data(0), u32::MAX, &mut leaves)
            .unwrap();
        match failure {
            0 => assert!(
                analyze_rom_instruction(
                    &[],
                    0x200,
                    DependencyInput::Conservative,
                    &mut scratch,
                    &mut DependencyReport::default()
                )
                .is_err()
            ),
            1 => {
                analyze_instruction(
                    &ir,
                    DependencyInput::Conservative,
                    &mut scratch,
                    &mut DependencyReport::default(),
                )
                .unwrap();
            }
            2 => {
                analyze_instruction(
                    &ir,
                    DependencyInput::Entry(&before(&ir)),
                    &mut scratch,
                    &mut DependencyReport::default(),
                )
                .unwrap();
            }
            3 => assert!(
                analyze_instruction(
                    &ir,
                    DependencyInput::Executed {
                        before: &before(&ir),
                        reads: &[],
                        addresses: ObservedAddressDomain::Raw32
                    },
                    &mut scratch,
                    &mut DependencyReport::default()
                )
                .is_err()
            ),
            4 => assert!(
                analyze_instruction(
                    &ir,
                    DependencyInput::Executed {
                        before: &before(&ir),
                        reads: &[read(0xff7002, Width::Word, ReadKind::Semantic, 0x1234)],
                        addresses: ObservedAddressDomain::Raw32
                    },
                    &mut scratch,
                    &mut DependencyReport::default()
                )
                .is_err()
            ),
            _ => unreachable!(),
        }
        assert!(
            scratch
                .output_read_demands(StateField::Data(0), u32::MAX, &mut leaves)
                .is_err()
        );
        assert!(leaves.is_empty());
        assert!(
            scratch
                .value_read_demands(Value(2), u32::MAX, false, &mut leaves)
                .is_err()
        );
        assert!(leaves.is_empty());
    }
}

#[test]
fn combined_physical_words_keep_independent_big_endian_leaf_masks() {
    let ir = instruction(
        8,
        [
            Op::Constant {
                dst: Value(1),
                value: 0xff7000,
            },
            Op::ReadMemory {
                dst: Value(2),
                address: Value(1),
                width: Width::Word,
                kind: ReadKind::Semantic,
            },
            Op::Constant {
                dst: Value(3),
                value: 0xff7002,
            },
            Op::ReadMemory {
                dst: Value(4),
                address: Value(3),
                width: Width::Word,
                kind: ReadKind::Semantic,
            },
            Op::Constant {
                dst: Value(5),
                value: 16,
            },
            Op::Binary {
                dst: Value(6),
                op: BinaryOp::ShiftLeft,
                lhs: Value(2),
                rhs: Value(5),
            },
            Op::Binary {
                dst: Value(7),
                op: BinaryOp::Or,
                lhs: Value(6),
                rhs: Value(4),
            },
            Op::StoreState {
                field: StateField::Data(0),
                value: Value(7),
            },
        ],
    );
    let mut scratch = DependencyScratch::default();
    let mut leaves = Vec::new();
    bind(
        &ir,
        &[
            read(0xff7000, Width::Word, ReadKind::Semantic, 0xabcd),
            read(0xff7002, Width::Word, ReadKind::Semantic, 0x1234),
        ],
        ObservedAddressDomain::Raw32,
        &mut scratch,
    );
    scratch
        .output_read_demands(StateField::Data(0), 0x00ff0000, &mut leaves)
        .unwrap();
    assert_eq!(
        leaves
            .iter()
            .map(|leaf| (leaf.ordinal, leaf.address, leaf.data_bits))
            .collect::<Vec<_>>(),
        [(0, 0xff7000, 0xff)]
    );
    scratch
        .output_read_demands(StateField::Data(0), 0x0000ff00, &mut leaves)
        .unwrap();
    assert_eq!(
        leaves
            .iter()
            .map(|leaf| (leaf.ordinal, leaf.address, leaf.data_bits))
            .collect::<Vec<_>>(),
        [(1, 0xff7002, 0xff00)]
    );
    scratch
        .output_read_demands(StateField::Data(0), u32::MAX, &mut leaves)
        .unwrap();
    assert_eq!(
        leaves
            .iter()
            .map(|leaf| (leaf.ordinal, leaf.data_bits))
            .collect::<Vec<_>>(),
        [(0, 0xffff), (1, 0xffff)]
    );
}

#[test]
fn observed_zero_and_equal_entry_values_never_become_memory_identities() {
    let ir = simple_word();
    let mut scratch = DependencyScratch::default();
    let mut leaves = Vec::new();
    for value in [0, 1, 0xffff] {
        bind(
            &ir,
            &[read(0xff7000, Width::Word, ReadKind::Semantic, value)],
            ObservedAddressDomain::Raw32,
            &mut scratch,
        );
        scratch
            .output_read_demands(StateField::Data(0), u32::MAX, &mut leaves)
            .unwrap();
        assert_eq!(
            (leaves.len(), leaves[0].data_bits, leaves[0].control_bits),
            (1, 0xffff, 0)
        );
    }
}

#[test]
fn width_purpose_missing_and_extra_read_errors_leave_no_query_leaves() {
    let ir = simple_word();
    let good = [read(0xff7000, Width::Word, ReadKind::Semantic, 0x1234)];
    let mut scratch = DependencyScratch::default();
    let mut leaves = Vec::new();
    for bad in [
        &[read(0xff7000, Width::Byte, ReadKind::Semantic, 0x34)][..],
        &[read(0xff7000, Width::Word, ReadKind::Discarded, 0x1234)][..],
        &[][..],
        &[
            read(0xff7000, Width::Word, ReadKind::Semantic, 0x1234),
            read(0xff7000, Width::Word, ReadKind::Semantic, 0x1234),
        ][..],
    ] {
        bind(&ir, &good, ObservedAddressDomain::Raw32, &mut scratch);
        scratch
            .output_read_demands(StateField::Data(0), u32::MAX, &mut leaves)
            .unwrap();
        assert!(
            analyze_instruction(
                &ir,
                DependencyInput::Executed {
                    before: &before(&ir),
                    reads: bad,
                    addresses: ObservedAddressDomain::Raw32
                },
                &mut scratch,
                &mut DependencyReport::default()
            )
            .is_err()
        );
        assert!(
            scratch
                .output_read_demands(StateField::Data(0), u32::MAX, &mut leaves)
                .is_err()
        );
        assert!(leaves.is_empty());
        assert!(
            scratch
                .value_read_demands(Value(2), u32::MAX, false, &mut leaves)
                .is_err()
        );
        assert!(leaves.is_empty());
    }
}

#[test]
fn one_physical_read_can_have_disjoint_data_and_control_bit_demands() {
    let ir = instruction(
        11,
        [
            Op::Constant {
                dst: Value(1),
                value: 0xff7000,
            },
            Op::ReadMemory {
                dst: Value(2),
                address: Value(1),
                width: Width::Word,
                kind: ReadKind::Semantic,
            },
            Op::Constant {
                dst: Value(3),
                value: 0xff00,
            },
            Op::Binary {
                dst: Value(4),
                op: BinaryOp::And,
                lhs: Value(2),
                rhs: Value(3),
            },
            Op::Constant {
                dst: Value(5),
                value: 0,
            },
            Op::Compare {
                dst: Value(6),
                op: CompareOp::Equal,
                lhs: Value(4),
                rhs: Value(5),
            },
            Op::Constant {
                dst: Value(7),
                value: 0xff,
            },
            Op::Binary {
                dst: Value(8),
                op: BinaryOp::And,
                lhs: Value(2),
                rhs: Value(7),
            },
            Op::Constant {
                dst: Value(9),
                value: 1,
            },
            Op::Select {
                dst: Value(10),
                condition: Value(6),
                if_true: Value(8),
                if_false: Value(9),
            },
            Op::StoreState {
                field: StateField::Data(0),
                value: Value(10),
            },
        ],
    );
    let mut scratch = DependencyScratch::default();
    let mut leaves = Vec::new();
    for value in [0x0055, 0xab55] {
        bind(
            &ir,
            &[read(0xff7000, Width::Word, ReadKind::Semantic, value)],
            ObservedAddressDomain::Raw32,
            &mut scratch,
        );
        scratch
            .value_read_demands(Value(8), u32::MAX, false, &mut leaves)
            .unwrap();
        assert_eq!(
            (leaves.len(), leaves[0].data_bits, leaves[0].control_bits),
            (1, 0xff, 0xff00)
        );
    }
}
