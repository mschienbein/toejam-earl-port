use super::*;
use crate::{
    decode::{self, BranchCondition, Instruction as I},
    ir::{BinaryOp, ExceptionExit, Op, Value},
    timing,
};
use std::collections::HashSet;

fn ir(opcode: u16, extensions: &[u16]) -> InstructionIr {
    let mut bytes = [0_u8; 12];
    bytes[..2].copy_from_slice(&opcode.to_be_bytes());
    for (index, word) in extensions.iter().enumerate() {
        bytes[2 + index * 2..4 + index * 2].copy_from_slice(&word.to_be_bytes());
    }
    compile_instruction(&bytes, 0).unwrap()
}
fn report(instruction: &InstructionIr) -> DependencyReport {
    let mut result = DependencyReport::default();
    analyze_instruction(
        instruction,
        DependencyInput::Conservative,
        &mut DependencyScratch::default(),
        &mut result,
    )
    .unwrap();
    result
}
fn before(instruction: &InstructionIr) -> CpuState {
    CpuState {
        pc: instruction.pc,
        prefetch: u32::from(instruction.opcode),
        sr: 0x2700,
        ssp: 0xff4000,
        usp: 0xff6000,
        ..CpuState::default()
    }
}
fn at_entry(instruction: &InstructionIr, before: &CpuState) -> DependencyReport {
    let mut result = DependencyReport::default();
    analyze_instruction(
        instruction,
        DependencyInput::Entry(before),
        &mut DependencyScratch::default(),
        &mut result,
    )
    .unwrap();
    result
}

#[test]
fn every_opcode_class_and_extension_pattern_has_a_valid_all_state_ir_footprint() {
    let mut scratch = DependencyScratch::default();
    let mut result = DependencyReport::default();
    let mut classes = HashSet::new();
    let mut bytes = [0_u8; 12];
    for pattern in 0..2 {
        for opcode in 0..=u16::MAX {
            bytes[..2].copy_from_slice(&opcode.to_be_bytes());
            for (index, byte) in bytes[2..].iter_mut().enumerate() {
                *byte = if pattern == 0 {
                    0
                } else {
                    opcode.rotate_left(index as u32) as u8 ^ 0xa5
                };
            }
            let decoded = decode::decode(opcode);
            classes.insert(std::mem::discriminant(&decoded));
            let instruction = compile_instruction(&bytes, 0).unwrap();
            analyze_instruction(
                &instruction,
                DependencyInput::Conservative,
                &mut scratch,
                &mut result,
            )
            .unwrap_or_else(|error| panic!("{opcode:04x} {decoded:?}: {error}"));
            let mut loads = StateMask::EMPTY;
            let mut stores = StateMask::EMPTY;
            for op in &instruction.ops {
                match *op {
                    Op::LoadState { field, .. } => loads.insert(field, u32::MAX),
                    Op::StoreState { field, .. } => stores.insert(field, u32::MAX),
                    _ => {}
                }
            }
            let dependencies = result.instruction;
            assert!(
                (dependencies.reads - loads).is_empty(),
                "{opcode:04x}: fabricated field use"
            );
            assert!((dependencies.physical_writes - stores).is_empty());
            assert!((dependencies.possible_writes - dependencies.physical_writes).is_empty());
            assert!((dependencies.definite_writes - dependencies.possible_writes).is_empty());
            assert!((dependencies.preserved & dependencies.definite_writes).is_empty());
            assert_eq!(dependencies.reads, result.selected_reads(|_| true));
            if matches!(decoded, I::Illegal { .. } | I::Trap(_)) {
                assert!(dependencies.obligations.exception_entry_outside_ir);
            }
            if dependencies.obligations.exception_taken.is_none()
                && matches!(
                    decoded,
                    I::Add {
                        with_extend: true,
                        ..
                    } | I::Subtract {
                        with_extend: true,
                        ..
                    } | I::Negate {
                        with_extend: true,
                        ..
                    } | I::AddDecimal { .. }
                        | I::SubtractDecimal { .. }
                        | I::NegateDecimal(_)
                        | I::RotateThruExtendMemory(..)
                        | I::RotateThruExtendRegister(..)
                )
            {
                assert_ne!(
                    dependencies.reads.get(StateField::Sr) & 16,
                    0,
                    "{opcode:04x}: missing X input"
                );
            }
        }
    }
    assert!(
        classes.len() > 60,
        "all decoder variants, not a narrowed synthetic opcode family"
    );
}

#[test]
fn byte_word_and_long_writes_distinguish_physical_new_definition_and_passthrough_bits() {
    let byte = report(&ir(0x103c, &[0x80])).instruction;
    assert_eq!(byte.physical_writes.get(StateField::Data(0)), u32::MAX);
    assert_eq!(byte.possible_writes.get(StateField::Data(0)), 0xff);
    assert_eq!(byte.definite_writes.get(StateField::Data(0)), 0xff);
    assert_eq!(byte.preserved.get(StateField::Data(0)), !0xff);
    assert_eq!(byte.reads.get(StateField::Data(0)), !0xff);
    let word = report(&ir(0x303c, &[0x8001])).instruction;
    assert_eq!(word.possible_writes.get(StateField::Data(0)), 0xffff);
    assert_eq!(word.definite_writes.get(StateField::Data(0)), 0xffff);
    assert_eq!(word.preserved.get(StateField::Data(0)), !0xffff);
    let long = report(&ir(0x203c, &[0x1234, 0x5678])).instruction;
    assert_eq!(long.definite_writes.get(StateField::Data(0)), u32::MAX);
    assert_eq!(long.reads.get(StateField::Data(0)), 0);
    let same = report(&ir(0x2000, &[])).instruction;
    assert_eq!(same.physical_writes.get(StateField::Data(0)), u32::MAX);
    assert_eq!(same.preserved.get(StateField::Data(0)), u32::MAX);
    assert_eq!(same.definite_writes.get(StateField::Data(0)), 0);
}
#[test]
fn ccr_preservation_conditions_sticky_zero_and_extend_consumers_are_not_whole_sr_guesses() {
    let lea = report(&ir(0x43d0, &[])).instruction;
    assert_eq!(lea.reads.get(StateField::Address(0)), u32::MAX);
    assert_eq!(lea.physical_writes.get(StateField::Sr), 0);
    assert_eq!(lea.reads.get(StateField::Sr), 0);
    let cmp = report(&ir(0xb041, &[])).instruction;
    assert_eq!(cmp.effect_reads.get(StateField::Data(0)), 0xffff);
    assert_eq!(cmp.effect_reads.get(StateField::Data(1)), 0xffff);
    assert_eq!(cmp.possible_writes.get(StateField::Sr), 15);
    assert_eq!(cmp.preserved.get(StateField::Sr) & 31, 16);
    let bit = report(&ir(0x0800, &[9])).instruction;
    assert_eq!(bit.effect_reads.get(StateField::Data(0)), 1 << 9);
    assert_eq!(bit.possible_writes.get(StateField::Sr), 4);
    assert_eq!(bit.possible_writes.get(StateField::Data(0)), 0);
    for opcode in [0xd101_u16, 0x9101, 0x4000, 0xc101, 0x8101, 0x4800] {
        let instruction = ir(opcode, &[]);
        let dependencies = report(&instruction).instruction;
        assert_ne!(
            dependencies.effect_reads.get(StateField::Sr) & 16,
            0,
            "{opcode:04x} X"
        );
        assert_ne!(
            dependencies.effect_reads.get(StateField::Sr) & 4,
            0,
            "{opcode:04x} sticky Z"
        );
    }
    assert!(matches!(
        decode::decode(0xe310),
        I::RotateThruExtendRegister(..)
    ));
    assert!(matches!(
        decode::decode(0xe210),
        I::RotateThruExtendRegister(..)
    ));
    assert_ne!(
        report(&ir(0xe310, &[]))
            .instruction
            .effect_reads
            .get(StateField::Sr)
            & 16,
        0
    );
    assert_ne!(
        report(&ir(0xe210, &[]))
            .instruction
            .effect_reads
            .get(StateField::Sr)
            & 16,
        0
    );
}
fn condition_mask(condition: BranchCondition) -> u32 {
    (0..5).fold(0, |mask, bit| {
        mask | if (0..32).any(|sr| {
            timing::condition(condition, sr) != timing::condition(condition, sr ^ (1 << bit))
        }) {
            1 << bit
        } else {
            0
        }
    })
}
#[test]
fn every_bcc_scc_dbcc_condition_reads_exactly_the_live_condition_bits() {
    for (code, condition) in BranchCondition::ALL.into_iter().enumerate() {
        let scc = report(&ir(0x50c0 | (code as u16) << 8, &[])).instruction;
        assert_eq!(
            scc.effect_reads.get(StateField::Sr),
            condition_mask(condition),
            "Scc {condition:?}"
        );
        let dbcc = report(&ir(0x50c8 | (code as u16) << 8, &[2])).instruction;
        assert_eq!(
            dbcc.effect_reads.get(StateField::Sr),
            condition_mask(condition),
            "DBcc {condition:?}"
        );
        assert_eq!(dbcc.possible_writes.get(StateField::Data(0)) & !0xffff, 0);
        if code != 1 {
            let branch = report(&ir(0x6002 | (code as u16) << 8, &[])).instruction;
            assert_eq!(
                branch.effect_reads.get(StateField::Sr),
                condition_mask(condition),
                "Bcc {condition:?}"
            );
        }
    }
}

#[test]
fn movem_every_mask_reads_only_selected_data_register_lanes_and_reverses_predecrement() {
    let mut scratch = DependencyScratch::default();
    let mut result = DependencyReport::default();
    for mask in 0..=u16::MAX {
        let instruction = ir(0x4890, &[mask]); // MOVEM.W list,(A0)
        analyze_instruction(
            &instruction,
            DependencyInput::Conservative,
            &mut scratch,
            &mut result,
        )
        .unwrap();
        for register in 0..8 {
            assert_eq!(
                result.instruction.reads.get(StateField::Data(register)),
                if mask & (1 << register) != 0 {
                    0xffff
                } else {
                    0
                },
                "mask {mask:04x} D{register}"
            );
        }
    }
    for bit in 0..16 {
        let mask = 1 << bit;
        let normal = report(&ir(0x48d0, &[mask])).instruction;
        let predecrement = report(&ir(0x48e7, &[mask])).instruction;
        for register in 0..8 {
            assert_eq!(
                normal.reads.get(StateField::Data(register)),
                if bit == register { u32::MAX } else { 0 }
            );
            assert_eq!(
                predecrement.reads.get(StateField::Data(register)),
                if 15 - bit == register { u32::MAX } else { 0 }
            );
        }
        let load = ir(0x4c98, &[mask]);
        let mut state = before(&load);
        state.a[0] = 0x1000;
        let load = at_entry(&load, &state).instruction;
        for register in 0..8 {
            assert_eq!(
                load.definite_writes.get(StateField::Data(register)),
                if bit == register { u32::MAX } else { 0 }
            );
        }
    }
}
#[test]
fn a7_banks_postincrement_and_index_word_inputs_are_explicit_and_path_bound() {
    let push = ir(0x2f00, &[]); // MOVE.L D0,-(A7)
    let unknown = report(&push).instruction;
    assert_eq!(unknown.physical_writes.get(StateField::Ssp), u32::MAX);
    assert_eq!(unknown.physical_writes.get(StateField::Usp), u32::MAX);
    assert_eq!(unknown.definite_writes.get(StateField::Ssp), 0);
    assert_eq!(unknown.definite_writes.get(StateField::Usp), 0);
    let supervisor = at_entry(&push, &before(&push)).instruction;
    assert_eq!(supervisor.effect_reads.get(StateField::Ssp), u32::MAX);
    assert_eq!(supervisor.effect_reads.get(StateField::Usp), 0);
    assert_ne!(supervisor.effect_reads.get(StateField::Sr) & 0x2000, 0);
    let mut user = before(&push);
    user.sr = 0;
    let user = at_entry(&push, &user).instruction;
    assert_eq!(user.effect_reads.get(StateField::Usp), u32::MAX);
    assert_eq!(user.effect_reads.get(StateField::Ssp), 0);
    let indexed = report(&ir(0x3030, &[0x1004])).instruction;
    assert_eq!(indexed.effect_reads.get(StateField::Data(1)), 0xffff);
    assert_eq!(indexed.effect_reads.get(StateField::Address(0)), u32::MAX);
    let post = report(&ir(0x3018, &[])).instruction;
    assert_eq!(post.possible_writes.get(StateField::Address(0)), u32::MAX);
    assert_eq!(post.reads.get(StateField::Address(0)), u32::MAX);
    let usp = report(&ir(0x4e68, &[])).instruction;
    assert_eq!(usp.reads.get(StateField::Usp), u32::MAX);
    assert_eq!(usp.physical_writes.get(StateField::Address(0)), u32::MAX);
}

#[test]
fn root_provenance_does_not_globally_subtract_a_private_saved_register() {
    let instruction = ir(0x48f0, &[1 << 3, 0x3000]); // MOVEM.L D3,d8(A0,D3.W)
    let result = report(&instruction);
    assert_eq!(result.instruction.reads.get(StateField::Data(3)), u32::MAX);
    let selected = result.selected_reads(|use_| {
        !matches!(
            use_.kind,
            DependencyUseKind::MemoryWriteValue { .. } | DependencyUseKind::PreservedState { .. }
        )
    });
    assert_eq!(
        selected.get(StateField::Data(3)),
        0xffff,
        "index use survives private save omission"
    );
    assert_eq!(selected.get(StateField::Address(0)), u32::MAX);
}
fn early_exit_ir() -> InstructionIr {
    InstructionIr {
        pc: 0,
        next_pc: 2,
        opcode: 0x4e71,
        value_count: 3,
        ops: vec![
            Op::LoadState {
                dst: Value(0),
                field: StateField::Data(0),
            },
            Op::Constant {
                dst: Value(1),
                value: 4,
            },
            Op::ExitIf {
                condition: Value(0),
                exit: ExceptionExit {
                    vector: 4,
                    cycles: Value(1),
                    saved_pc: Value(1),
                    fault_address: Value(1),
                    fault_status: Value(1),
                },
            },
            Op::Constant {
                dst: Value(2),
                value: 0,
            },
            Op::StoreState {
                field: StateField::Data(1),
                value: Value(2),
            },
            Op::Finish { cycles: Value(1) },
        ],
    }
}
#[test]
fn sequence_folding_kills_only_proved_definitions_and_closes_passthrough_outputs() {
    let mut fold = DependencyFold::default();
    fold.push(&report(&ir(0x103c, &[1])).instruction);
    fold.push(&report(&ir(0x4a80, &[])).instruction); // TST.L D0
    assert_eq!(fold.read_before_write.get(StateField::Data(0)), !0xff);
    let mut full = DependencyFold::default();
    full.push(&report(&ir(0x203c, &[0x1234, 0x5678])).instruction);
    full.push(&report(&ir(0x4a80, &[])).instruction);
    assert_eq!(full.read_before_write.get(StateField::Data(0)), 0);
    let exit = early_exit_ir();
    let conservative = report(&exit).instruction;
    assert_eq!(conservative.definite_writes.get(StateField::Data(1)), 0);
    let mut fold = DependencyFold::default();
    fold.push(&conservative);
    fold.observe_outputs(StateMask::single(StateField::Data(1), u32::MAX));
    assert_eq!(fold.read_before_write.get(StateField::Data(1)), u32::MAX);
    assert!(fold.requires_scheduler_accounting);
    let exact = at_entry(&exit, &before(&exit)).instruction;
    assert_eq!(exact.definite_writes.get(StateField::Data(1)), u32::MAX);
    let mut fold = DependencyFold::default();
    fold.push(&exact);
    fold.observe_outputs(StateMask::single(StateField::Data(1), u32::MAX));
    assert_eq!(fold.read_before_write.get(StateField::Data(1)), 0);
    assert!(fold.observed_path_input_equality_required);
}
fn conditional_read_ir() -> InstructionIr {
    InstructionIr {
        pc: 0,
        next_pc: 2,
        opcode: 0x4e71,
        value_count: 4,
        ops: vec![
            Op::LoadState {
                dst: Value(0),
                field: StateField::Data(0),
            },
            Op::Constant {
                dst: Value(1),
                value: 0x1000,
            },
            Op::ReadMemoryIf {
                dst: Value(2),
                condition: Value(0),
                address: Value(1),
                width: Width::Word,
                kind: ReadKind::Semantic,
            },
            Op::StoreState {
                field: StateField::Data(1),
                value: Value(2),
            },
            Op::Constant {
                dst: Value(3),
                value: 4,
            },
            Op::Finish { cycles: Value(3) },
        ],
    }
}
#[test]
fn executed_read_binding_is_exact_and_errors_never_leave_a_stale_success_report() {
    let instruction = conditional_read_ir();
    let mut state = before(&instruction);
    let mut scratch = DependencyScratch::default();
    let mut result = DependencyReport::default();
    analyze_instruction(
        &instruction,
        DependencyInput::Executed {
            before: &state,
            addresses: ObservedAddressDomain::Raw32,
            reads: &[],
        },
        &mut scratch,
        &mut result,
    )
    .unwrap();
    assert_eq!(
        result.instruction.binding,
        PathBinding::ExecutedReads {
            consumed: 0,
            addresses: ObservedAddressDomain::Raw32
        }
    );
    assert_eq!(result.instruction.reads.get(StateField::Data(0)), u32::MAX);
    state.d[0] = 1;
    let read = ObservedRead {
        address: 0x1000,
        width: Width::Word,
        kind: ReadKind::Semantic,
        value: 0x1234,
    };
    analyze_instruction(
        &instruction,
        DependencyInput::Executed {
            before: &state,
            addresses: ObservedAddressDomain::Raw32,
            reads: &[read],
        },
        &mut scratch,
        &mut result,
    )
    .unwrap();
    for invalid in [
        ObservedRead {
            address: 0x1002,
            ..read
        },
        ObservedRead {
            width: Width::Byte,
            ..read
        },
        ObservedRead {
            kind: ReadKind::Discarded,
            ..read
        },
        ObservedRead {
            value: 0x10000,
            ..read
        },
    ] {
        assert!(
            analyze_instruction(
                &instruction,
                DependencyInput::Executed {
                    before: &state,
                    addresses: ObservedAddressDomain::Raw32,
                    reads: &[invalid]
                },
                &mut scratch,
                &mut result
            )
            .is_err()
        );
        assert_eq!(result.instruction, InstructionDependencies::default());
        assert!(result.uses.is_empty());
    }
    assert!(
        analyze_instruction(
            &instruction,
            DependencyInput::Executed {
                before: &state,
                addresses: ObservedAddressDomain::Raw32,
                reads: &[]
            },
            &mut scratch,
            &mut result
        )
        .is_err()
    );
    assert!(
        analyze_instruction(
            &instruction,
            DependencyInput::Executed {
                before: &state,
                addresses: ObservedAddressDomain::Raw32,
                reads: &[read, read]
            },
            &mut scratch,
            &mut result
        )
        .is_err()
    );
    state.pc = 2;
    assert!(
        analyze_instruction(
            &instruction,
            DependencyInput::Entry(&state),
            &mut scratch,
            &mut result
        )
        .is_err()
    );
    assert!(
        analyze_rom_instruction(
            &[],
            0,
            DependencyInput::Conservative,
            &mut scratch,
            &mut result
        )
        .is_err()
    );
    assert_eq!(result.instruction, InstructionDependencies::default());
}
#[test]
fn malformed_ir_all_state_domains_and_outside_scheduler_obligations_fail_closed() {
    let mut scratch = DependencyScratch::default();
    let mut result = DependencyReport::default();
    let mut bad = early_exit_ir();
    bad.ops[0] = Op::LoadState {
        dst: Value(0),
        field: StateField::Data(8),
    };
    assert!(
        analyze_instruction(
            &bad,
            DependencyInput::Conservative,
            &mut scratch,
            &mut result
        )
        .is_err()
    );
    let mut bad = early_exit_ir();
    bad.ops[1] = Op::Binary {
        dst: Value(1),
        op: BinaryOp::Add,
        lhs: Value(2),
        rhs: Value(0),
    };
    assert!(
        analyze_instruction(
            &bad,
            DependencyInput::Conservative,
            &mut scratch,
            &mut result
        )
        .is_err()
    );
    let trap = report(&ir(0x4e40, &[])).instruction;
    assert!(trap.obligations.exception_entry_outside_ir);
    assert_eq!(trap.obligations.exception_taken, Some(32));
    let stop = report(&ir(0x4e72, &[0x2700])).instruction;
    assert!(stop.obligations.halt_or_status_control_write);
    for index in 0..StateField::COUNT {
        let field = StateField::from_index(index).unwrap();
        assert_eq!(field.index() as usize, index);
        assert_eq!(StateMask::single(field, 0x12345678).get(field), 0x12345678);
    }
    assert_eq!(StateField::from_index(StateField::COUNT), None);
}

#[test]
fn proven_zero_branch_address_guards_drop_payload_but_real_odd_targets_keep_system_inputs() {
    let even = report(&ir(0x51c8, &[2])).instruction;
    assert_eq!(even.effect_reads.get(StateField::Sr), 0);
    assert!(!even.obligations.exception_entry_outside_ir);
    let odd = report(&ir(0x51c8, &[3])).instruction;
    assert_ne!(odd.effect_reads.get(StateField::Sr) & 0x2000, 0);
    assert_eq!(odd.effect_reads.get(StateField::Sr) & 31, 0);
    assert!(odd.obligations.exception_entry_outside_ir);
}
