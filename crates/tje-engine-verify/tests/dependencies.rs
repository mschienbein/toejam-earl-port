//! Independent evaluator properties for compiler dependency analysis. Streams,
//! CPU seeds and memory are authored synthetic fixtures, never cartridge data.
use tje_engine_core::{
    Bus, CpuState, InstructionIr, ReadKind, StepResult, compile_instruction,
    dependencies::{
        DependencyFold, DependencyInput, DependencyReport, DependencyScratch,
        ObservedAddressDomain, ObservedRead, StateMask,
    },
    ir::{BinaryOp, Op, StateField, Value, Width},
};
use tje_engine_verify::interpreter::Interpreter;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Access {
    Read {
        address: u32,
        width: Width,
        kind: ReadKind,
        value: u32,
    },
    Write {
        address: u32,
        width: Width,
        value: u32,
    },
    Offset(u32),
}
#[derive(Default)]
struct SyntheticBus {
    initial: Vec<u8>,
    bytes: Vec<(u32, u8)>,
    accesses: Vec<Access>,
    physical24: bool,
}
impl SyntheticBus {
    fn new(initial: &[u8]) -> Self {
        Self {
            initial: initial.to_vec(),
            ..Self::default()
        }
    }
    fn journal24(initial: &[u8]) -> Self {
        Self {
            physical24: true,
            ..Self::new(initial)
        }
    }
    fn address(&self, address: u32) -> u32 {
        if self.physical24 {
            address & 0x00ff_ffff
        } else {
            address
        }
    }
    fn byte(&self, address: u32) -> u8 {
        self.bytes
            .iter()
            .rev()
            .find(|(at, _)| *at == address)
            .map(|(_, value)| *value)
            .or_else(|| self.initial.get(address as usize).copied())
            .unwrap_or_else(|| address.wrapping_mul(0x045d9f3b).rotate_right(7) as u8)
    }
    fn put(&mut self, address: u32, value: u8) {
        self.bytes.push((address, value));
    }
    fn observations(&self) -> Vec<ObservedRead> {
        self.accesses
            .iter()
            .filter_map(|access| match *access {
                Access::Read {
                    address,
                    width,
                    kind,
                    value,
                } => Some(ObservedRead {
                    address,
                    width,
                    kind,
                    value,
                }),
                _ => None,
            })
            .collect()
    }
}
impl Bus for SyntheticBus {
    fn read8(&mut self, address: u32, kind: ReadKind) -> u8 {
        let address = self.address(address);
        let value = self.byte(address);
        self.accesses.push(Access::Read {
            address,
            width: Width::Byte,
            kind,
            value: u32::from(value),
        });
        value
    }
    fn read16(&mut self, address: u32, kind: ReadKind) -> u16 {
        let address = self.address(address);
        let value = u16::from_be_bytes([
            self.byte(address),
            self.byte(self.address(address.wrapping_add(1))),
        ]);
        self.accesses.push(Access::Read {
            address,
            width: Width::Word,
            kind,
            value: u32::from(value),
        });
        value
    }
    fn write8(&mut self, address: u32, value: u8) {
        let address = self.address(address);
        self.put(address, value);
        self.accesses.push(Access::Write {
            address,
            width: Width::Byte,
            value: u32::from(value),
        });
    }
    fn write16(&mut self, address: u32, value: u16) {
        let address = self.address(address);
        let bytes = value.to_be_bytes();
        self.put(address, bytes[0]);
        self.put(self.address(address.wrapping_add(1)), bytes[1]);
        self.accesses.push(Access::Write {
            address,
            width: Width::Word,
            value: u32::from(value),
        });
    }
    fn set_read_offset_mclk(&mut self, value: u32) {
        self.accesses.push(Access::Offset(value));
    }
}
fn mutate(state: &mut CpuState, field: StateField, bits: u32) {
    match field {
        StateField::Data(n) => state.d[n as usize] ^= bits,
        StateField::Address(n) => state.a[n as usize] ^= bits,
        StateField::Usp => state.usp ^= bits,
        StateField::Ssp => state.ssp ^= bits,
        StateField::Pc => state.pc ^= bits,
        StateField::Sr => state.sr ^= bits,
        StateField::Prefetch => state.prefetch ^= bits,
        StateField::PendingInterrupt => state.pending_interrupt ^= bits,
        StateField::Stopped => state.stopped ^= bits,
        StateField::Frozen => state.frozen ^= bits,
        StateField::AddressError => state.address_error ^= bits,
        StateField::LastMulDiv => state.last_muldiv ^= bits,
    }
}
fn state(opcode: u16, seed: u32) -> CpuState {
    CpuState {
        d: std::array::from_fn(|n| seed.rotate_left(n as u32 * 3) ^ 0x97a6_152b),
        a: std::array::from_fn(|n| 0x1000 + n as u32 * 0x100),
        usp: 0x8000,
        ssp: 0x9000,
        pc: 0,
        prefetch: u32::from(opcode),
        sr: if seed & 1 == 0 { 0x2700 } else { 0 },
        ..CpuState::default()
    }
}
fn compared_inputs(report: &DependencyReport, step: StepResult) -> StateMask {
    if step.vector == 0 {
        return report.instruction.reads;
    }
    // The comparison below imports ALL potentially stored fields. On an early
    // exit some of those stores never happened: their identity output is still
    // an input to this observation, not an instruction effect. The normal-path
    // property stays on the unaugmented instruction read mask.
    let mut fold = DependencyFold::default();
    fold.push(&report.instruction);
    fold.observe_outputs(report.instruction.physical_writes);
    fold.read_before_write
}

fn assert_unchanged(
    instruction: &InstructionIr,
    expected: (&CpuState, StepResult, &SyntheticBus),
    changed: (&CpuState, StepResult, &SyntheticBus),
    report: &DependencyReport,
    input: (StateField, u32),
) {
    let (expected_state, expected_step, expected_bus) = expected;
    let (altered, altered_step, altered_bus) = changed;
    let (field, bit) = input;
    assert_eq!(
        altered_step, expected_step,
        "{:04x} {field:?}/{bit}: cycles/exit",
        instruction.opcode
    );
    assert_eq!(
        altered_bus.accesses, expected_bus.accesses,
        "{:04x} {field:?}/{bit}: ordered bus",
        instruction.opcode
    );
    assert_eq!(altered_bus.bytes, expected_bus.bytes);
    for (output, mask) in report.instruction.physical_writes.iter() {
        assert_eq!(
            (output.read(altered) ^ output.read(expected_state)) & mask,
            0,
            "{:04x} input {field:?}/{bit} changed output {output:?}",
            instruction.opcode
        );
    }
}
#[test]
fn every_opcode_ir_result_is_invariant_under_an_unreported_entry_bit() {
    let mut evaluator = Interpreter::default();
    let mut scratch = DependencyScratch::default();
    let mut report = DependencyReport::default();
    let mut mutations = 0;
    for opcode in 0..=u16::MAX {
        let mut bytes = [0; 12];
        bytes[..2].copy_from_slice(&opcode.to_be_bytes());
        for (n, byte) in bytes[2..].iter_mut().enumerate() {
            *byte = opcode.rotate_right(n as u32) as u8 ^ 0x52;
        }
        let instruction = compile_instruction(&bytes, 0).unwrap();
        let before = state(opcode, u32::from(opcode).wrapping_mul(0x9e37_79b9));
        let mut expected = before.clone();
        let mut bus = SyntheticBus::new(&bytes);
        let step = evaluator
            .evaluate(&instruction, &mut expected, &mut bus)
            .unwrap();
        tje_engine_core::dependencies::analyze_instruction(
            &instruction,
            DependencyInput::Conservative,
            &mut scratch,
            &mut report,
        )
        .unwrap();
        // Rotate through all canonical domains and bit positions across the full
        // opcode space. Targeted fixtures below exhaust every bit of IR operands.
        let field = StateField::from_index(opcode as usize % StateField::COUNT).unwrap();
        let bit = (u32::from(opcode) / StateField::COUNT as u32) & 31;
        if compared_inputs(&report, step).get(field) & (1 << bit) == 0 {
            let mut altered = before.clone();
            mutate(&mut altered, field, 1 << bit);
            let mut altered_bus = SyntheticBus::new(&bytes);
            let altered_step = evaluator
                .evaluate(&instruction, &mut altered, &mut altered_bus)
                .unwrap();
            assert_unchanged(
                &instruction,
                (&expected, step, &bus),
                (&altered, altered_step, &altered_bus),
                &report,
                (field, bit),
            );
            mutations += 1;
        }
        let observations = bus.observations();
        tje_engine_core::dependencies::analyze_instruction(
            &instruction,
            DependencyInput::Executed {
                before: &before,
                addresses: ObservedAddressDomain::Raw32,
                reads: &observations,
            },
            &mut scratch,
            &mut report,
        )
        .unwrap();
        // Actual-path precision is bound to this exact accepted read sequence.
        // A7 bank/condition changes must be reported, never healed by the oracle.
        let field = StateField::Data(((opcode >> 9) & 7) as u8);
        let bit = (u32::from(opcode) >> 3) & 31;
        if compared_inputs(&report, step).get(field) & (1 << bit) == 0 {
            let mut altered = before.clone();
            mutate(&mut altered, field, 1 << bit);
            let mut altered_bus = SyntheticBus::new(&bytes);
            let altered_step = evaluator
                .evaluate(&instruction, &mut altered, &mut altered_bus)
                .unwrap();
            assert_unchanged(
                &instruction,
                (&expected, step, &bus),
                (&altered, altered_step, &altered_bus),
                &report,
                (field, bit),
            );
            mutations += 1;
        }
    }
    assert!(mutations > 65536, "not a vacuous all-inputs-full property");
}
fn primitive(op: BinaryOp, mask: u32, count_constant: Option<u32>) -> InstructionIr {
    InstructionIr {
        pc: 0,
        next_pc: 2,
        opcode: 0x4e71,
        value_count: 6,
        ops: vec![
            Op::LoadState {
                dst: Value(0),
                field: StateField::Data(0),
            },
            if let Some(value) = count_constant {
                Op::Constant {
                    dst: Value(1),
                    value,
                }
            } else {
                Op::LoadState {
                    dst: Value(1),
                    field: StateField::Data(1),
                }
            },
            Op::Binary {
                dst: Value(2),
                op,
                lhs: Value(0),
                rhs: Value(1),
            },
            Op::Constant {
                dst: Value(3),
                value: mask,
            },
            Op::Binary {
                dst: Value(4),
                op: BinaryOp::And,
                lhs: Value(2),
                rhs: Value(3),
            },
            Op::StoreState {
                field: StateField::Data(2),
                value: Value(4),
            },
            Op::Constant {
                dst: Value(5),
                value: 4,
            },
            Op::Finish { cycles: Value(5) },
        ],
    }
}
#[test]
fn independent_all_bit_mutations_cover_boolean_carry_sign_extension_and_variable_shift_masks() {
    let mut evaluator = Interpreter::default();
    let mut scratch = DependencyScratch::default();
    let mut report = DependencyReport::default();
    for op in [
        BinaryOp::And,
        BinaryOp::Or,
        BinaryOp::Xor,
        BinaryOp::Add,
        BinaryOp::Subtract,
        BinaryOp::Multiply,
        BinaryOp::ShiftLeft,
        BinaryOp::ShiftRight,
        BinaryOp::ArithmeticShiftRight,
    ] {
        for mask in [0xff, 0xffff, u32::MAX] {
            for count in [
                None,
                Some(0),
                Some(1),
                Some(16),
                Some(31),
                Some(32),
                Some(63),
            ] {
                let instruction = primitive(op, mask, count);
                for seed in [0x8000_0001, 0x7fff_ffff, 0xa55a_5aa5] {
                    let mut before = state(instruction.opcode, seed);
                    before.d[1] = 17;
                    let mut expected = before.clone();
                    let mut bus = SyntheticBus::default();
                    let step = evaluator
                        .evaluate(&instruction, &mut expected, &mut bus)
                        .unwrap();
                    for exact in [false, true] {
                        let input = if exact {
                            DependencyInput::Entry(&before)
                        } else {
                            DependencyInput::Conservative
                        };
                        tje_engine_core::dependencies::analyze_instruction(
                            &instruction,
                            input,
                            &mut scratch,
                            &mut report,
                        )
                        .unwrap();
                        for field in [StateField::Data(0), StateField::Data(1)] {
                            for bit in 0..32 {
                                if report.instruction.reads.get(field) & (1 << bit) != 0 {
                                    continue;
                                }
                                let mut altered = before.clone();
                                mutate(&mut altered, field, 1 << bit);
                                let mut altered_bus = SyntheticBus::default();
                                let changed = evaluator
                                    .evaluate(&instruction, &mut altered, &mut altered_bus)
                                    .unwrap();
                                assert_unchanged(
                                    &instruction,
                                    (&expected, step, &bus),
                                    (&altered, changed, &altered_bus),
                                    &report,
                                    (field, bit),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn executed_physical_journals_bind_high_ir_pointers_without_ram_mirror_normalization() {
    let mut evaluator = Interpreter::default();
    let mut scratch = DependencyScratch::default();
    let mut report = DependencyReport::default();
    for (opcode, extension) in [(0x3038_u16, 0x8000_u16), (0x3010, 0)] {
        let mut bytes = [0; 12];
        bytes[..2].copy_from_slice(&opcode.to_be_bytes());
        bytes[2..4].copy_from_slice(&extension.to_be_bytes());
        let instruction = compile_instruction(&bytes, 0).unwrap();
        let mut before = state(opcode, 0x1234);
        before.a[0] = 0xffff_8000;
        let mut actual = before.clone();
        let mut bus = SyntheticBus::journal24(&bytes);
        evaluator
            .evaluate(&instruction, &mut actual, &mut bus)
            .unwrap();
        let observations = bus.observations();
        assert!(
            observations
                .iter()
                .any(|read| read.address == 0x00ff_8000 && read.kind == ReadKind::Semantic)
        );
        tje_engine_core::dependencies::analyze_instruction(
            &instruction,
            DependencyInput::Executed {
                before: &before,
                reads: &observations,
                addresses: ObservedAddressDomain::M68000Bus24,
            },
            &mut scratch,
            &mut report,
        )
        .unwrap();
        assert!(matches!(
            report.instruction.binding,
            tje_engine_core::dependencies::PathBinding::ExecutedReads {
                addresses: ObservedAddressDomain::M68000Bus24,
                ..
            }
        ));
        assert!(
            tje_engine_core::dependencies::analyze_instruction(
                &instruction,
                DependencyInput::Executed {
                    before: &before,
                    reads: &observations,
                    addresses: ObservedAddressDomain::Raw32
                },
                &mut scratch,
                &mut report
            )
            .is_err()
        );
        for wrong in [0x00e0_8000_u32, 0x00ff_8002, 0xffff_8000] {
            let mut invalid = observations.clone();
            invalid
                .iter_mut()
                .find(|read| read.address == 0x00ff_8000)
                .unwrap()
                .address = wrong;
            assert!(
                tje_engine_core::dependencies::analyze_instruction(
                    &instruction,
                    DependencyInput::Executed {
                        before: &before,
                        reads: &invalid,
                        addresses: ObservedAddressDomain::M68000Bus24
                    },
                    &mut scratch,
                    &mut report
                )
                .is_err(),
                "wrong physical line or high journal bits were silently canonicalized"
            );
        }
    }
}

#[test]
fn early_data_address_fault_before_status_store_requires_passthrough_output_observation() {
    // Authored ADDI.W #$3967,-$3761(A3), matching the full-space property shape.
    let mut bytes = [0; 12];
    bytes[..2].copy_from_slice(&0x066b_u16.to_be_bytes());
    bytes[2..4].copy_from_slice(&0x3967_u16.to_be_bytes());
    bytes[4..6].copy_from_slice(&0xc89f_u16.to_be_bytes());
    let instruction = compile_instruction(&bytes, 0).unwrap();
    let before = state(instruction.opcode, 0x1234);
    let mut actual = before.clone();
    let mut bus = SyntheticBus::new(&bytes);
    let mut evaluator = Interpreter::default();
    let step = evaluator
        .evaluate(&instruction, &mut actual, &mut bus)
        .unwrap();
    assert_eq!(step.vector, 3);
    assert_eq!(
        actual.sr, before.sr,
        "normal-path SR store was not executed"
    );
    let mut report = DependencyReport::default();
    tje_engine_core::dependencies::analyze_instruction(
        &instruction,
        DependencyInput::Conservative,
        &mut DependencyScratch::default(),
        &mut report,
    )
    .unwrap();
    assert_ne!(
        report.instruction.physical_writes.get(StateField::Sr) & 2,
        0
    );
    assert_eq!(report.instruction.reads.get(StateField::Sr) & 2, 0);
    assert_eq!(
        report.instruction.definite_writes.get(StateField::Sr) & 2,
        0
    );
    assert_ne!(compared_inputs(&report, step).get(StateField::Sr) & 2, 0);
    assert!(
        report.instruction.obligations.exception_entry_outside_ir,
        "full exception-frame/SR/SSP scheduler effects are outside this IR oracle"
    );
    let mut changed = before.clone();
    changed.sr ^= 2;
    let mut changed_bus = SyntheticBus::new(&bytes);
    let changed_step = evaluator
        .evaluate(&instruction, &mut changed, &mut changed_bus)
        .unwrap();
    assert_eq!(changed_step, step);
    assert_eq!(changed_bus.accesses, bus.accesses);
    assert_eq!(
        changed.sr ^ actual.sr,
        2,
        "untouched output retains its own input bit"
    );
}
