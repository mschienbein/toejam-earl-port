use crate::{
    CpuState, compile_instruction,
    decode::{BranchCondition, DataRegister, Instruction},
    ir::*,
    metadata::{CodeRange, ProgramMetadata, compile_program},
    timing,
};

#[test]
fn every_opcode_has_complete_ssa_and_a_boundary() {
    let mut rom = [0_u8; 12];
    for opcode in 0..=u16::MAX {
        rom[..2].copy_from_slice(&opcode.to_be_bytes());
        let ir = compile_instruction(&rom, 0).unwrap();
        let mut defined = vec![false; ir.value_count as usize];
        let check = |v: Value, defined: &[bool]| {
            assert!(
                defined[v.0 as usize],
                "opcode {opcode:04x}: value before definition {v:?}"
            )
        };
        for op in &ir.ops {
            let dst = match *op {
                Op::Constant { dst, .. } | Op::LoadState { dst, .. } => Some(dst),
                Op::Binary { dst, lhs, rhs, .. } | Op::Compare { dst, lhs, rhs, .. } => {
                    check(lhs, &defined);
                    check(rhs, &defined);
                    Some(dst)
                }
                Op::Select {
                    dst,
                    condition,
                    if_true,
                    if_false,
                } => {
                    check(condition, &defined);
                    check(if_true, &defined);
                    check(if_false, &defined);
                    Some(dst)
                }
                Op::ReadMemory { dst, address, .. } => {
                    check(address, &defined);
                    Some(dst)
                }
                Op::ReadMemoryIf {
                    dst,
                    condition,
                    address,
                    ..
                } => {
                    check(condition, &defined);
                    check(address, &defined);
                    Some(dst)
                }
                Op::StoreState { value, .. } => {
                    check(value, &defined);
                    None
                }
                Op::WriteMemory { address, value, .. } => {
                    check(address, &defined);
                    check(value, &defined);
                    None
                }
                Op::SetReadOffset { cycles } | Op::Finish { cycles } => {
                    check(cycles, &defined);
                    None
                }
                Op::ExitIf { condition, exit } => {
                    for v in [
                        condition,
                        exit.cycles,
                        exit.saved_pc,
                        exit.fault_address,
                        exit.fault_status,
                    ] {
                        check(v, &defined);
                    }
                    None
                }
            };
            if let Some(dst) = dst {
                assert!(!defined[dst.0 as usize]);
                defined[dst.0 as usize] = true;
            }
        }
        assert!(matches!(ir.ops.last(), Some(Op::Finish { .. })));
        assert!(defined.into_iter().all(|v| v));
    }
}

#[test]
fn status_read_compensation_matches_reference_for_every_opcode() {
    for opcode in 0..=u16::MAX {
        let ours = timing::cycles_if_move_btst_cmp(opcode);
        assert_eq!(
            ours,
            m68000_emu::cycles_if_move_btst_cmp(opcode),
            "{opcode:04x}"
        );
    }
}

#[test]
fn conditions_and_dbcc_all_paths_are_operand_dependent() {
    for sr in 0..32 {
        let cpu = CpuState {
            sr,
            ..CpuState::default()
        };
        for condition in BranchCondition::ALL {
            let branch = Instruction::Branch(condition, 2);
            let take = timing::condition(condition, sr as u16);
            assert_eq!(
                timing::cycles(branch, &cpu, 0, 0),
                Some(if take { 10 } else { 8 })
            );
            assert_eq!(
                timing::cycles(Instruction::Branch(condition, 0), &cpu, 0, 0),
                Some(if take { 10 } else { 12 })
            );
            for low in [0, 1, 0xffff] {
                let mut cpu = cpu.clone();
                cpu.d[0] = low;
                assert_eq!(
                    timing::cycles(
                        Instruction::BranchDecrement(condition, DataRegister(0)),
                        &cpu,
                        0,
                        0
                    ),
                    Some(if take {
                        12
                    } else if low == 0 {
                        14
                    } else {
                        10
                    })
                );
            }
        }
    }
}

#[test]
fn metadata_disallows_code_data_crossing_and_overlapping_decodes() {
    let rom = [0x4e, 0x71, 0x4e, 0x75];
    let mut metadata = ProgramMetadata {
        code_ranges: vec![CodeRange { start: 0, end: 4 }],
        entry_points: vec![0],
        ..ProgramMetadata::default()
    };
    assert_eq!(
        compile_program(&rom, &metadata).unwrap().instructions.len(),
        2
    );
    metadata.code_ranges[0].end = 2;
    assert!(compile_program(&rom, &metadata).is_err());
    assert!(compile_instruction(&rom, 1).is_err());
    assert!(compile_instruction(&rom, 4).is_err());
}

#[test]
fn state_layout_is_exact_and_user_supervisor_stacks_are_independent() {
    assert_eq!(std::mem::size_of::<CpuState>(), 100);
    assert_eq!(
        std::mem::offset_of!(CpuState, sr),
        StateField::Sr.offset() as usize
    );
    assert_eq!(
        std::mem::offset_of!(CpuState, last_muldiv),
        StateField::LastMulDiv.offset() as usize
    );
    let mut cpu = CpuState {
        usp: 0x1000,
        ssp: 0x2000,
        ..CpuState::default()
    };
    assert_eq!(cpu.read_address(7), 0x1000);
    cpu.write_address(7, 0x1002);
    cpu.set_sr(0xffff);
    assert_eq!(cpu.sr, 0xa71f);
    assert_eq!(cpu.read_address(7), 0x2000);
    cpu.write_address(7, 0x2004);
    cpu.set_sr(0);
    assert_eq!((cpu.read_address(7), cpu.ssp), (0x1002, 0x2004));
}

#[test]
fn privileged_timing_exits_before_operand_costs() {
    let user = CpuState::default();
    let supervisor = CpuState {
        sr: 0x2700,
        ..CpuState::default()
    };
    for (opcode, normal_cycles) in [
        (0x46C0, 12),
        (0x027C, 20),
        (0x007C, 20),
        (0x0A7C, 20),
        (0x4E60, 4),
        (0x4E72, 4),
        (0x4E73, 20),
    ] {
        let instruction = crate::decode::decode(opcode);
        assert_eq!(timing::cycles(instruction, &user, 0, 0), Some(34));
        assert_eq!(
            timing::cycles(instruction, &supervisor, 0, 0),
            Some(normal_cycles)
        );
    }
    // The pinned reference intentionally treats RESET as an unprivileged long NOP.
    assert_eq!(timing::cycles(Instruction::Reset, &user, 0, 0), Some(132));
}
