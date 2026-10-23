//! Independent timing formulas ported from CycleProto (GPL-3.0).
use crate::decode::{
    AddressingMode as Ea, BranchCondition, Direction, Instruction, OpSize, ShiftCount,
};

pub fn ea(a: Ea, s: OpSize) -> u32 {
    let extra = if s == OpSize::LongWord { 4 } else { 0 };
    match a {
        Ea::DataDirect(_) | Ea::AddressDirect(_) | Ea::Quick(_) => 0,
        Ea::AddressIndirect(_) | Ea::AddressIndirectPostincrement(_) | Ea::Immediate => 4 + extra,
        Ea::AddressIndirectPredecrement(_) => 6 + extra,
        Ea::AddressIndirectDisplacement(_) | Ea::PcRelativeDisplacement | Ea::AbsoluteShort => {
            8 + extra
        }
        Ea::AddressIndirectIndexed(_) | Ea::PcRelativeIndexed => 10 + extra,
        Ea::AbsoluteLong => 12 + extra,
    }
}
fn data(a: Ea) -> bool {
    matches!(a, Ea::DataDirect(_))
}
fn address(a: Ea) -> bool {
    matches!(a, Ea::AddressDirect(_))
}
fn memory(a: Ea) -> bool {
    !data(a) && !address(a) && !matches!(a, Ea::Immediate | Ea::Quick(_))
}
fn unary(s: OpSize, d: Ea) -> u32 {
    (if s == OpSize::LongWord {
        8 - 2 * u32::from(data(d))
    } else {
        4
    }) + ea(d, s)
        + 4 * u32::from(!data(d))
}
fn binary(s: OpSize, a: Ea, d: Ea) -> u32 {
    (if s == OpSize::LongWord { 8 } else { 4 })
        + 4 * u32::from(s == OpSize::Word && address(d))
        + ea(a, s)
        + ea(d, s)
        - 2 * u32::from(s == OpSize::LongWord && memory(a) && (data(d) || address(d)))
        + 4 * u32::from(memory(d))
}
fn compare(s: OpSize, source: Ea, dest: Ea) -> u32 {
    if matches!(
        (source, dest),
        (
            Ea::AddressIndirectPostincrement(_),
            Ea::AddressIndirectPostincrement(_)
        )
    ) {
        if s == OpSize::LongWord { 20 } else { 12 }
    } else if matches!(source, Ea::Immediate) {
        if data(dest) {
            if s == OpSize::LongWord { 14 } else { 8 }
        } else {
            binary(s, source, dest) - if s == OpSize::LongWord { 8 } else { 4 }
        }
    } else if s == OpSize::LongWord && (data(source) || address(source)) && data(dest) {
        6
    } else {
        binary(s, source, dest)
    }
}

/// The reference's VDP-read classifier is not always the instruction charge:
/// CMPA uses CMP timing here, but six-plus-EA timing when actually executed.
pub fn cycles_if_move_btst_cmp(opcode: u16) -> Option<u32> {
    Some(match crate::decode::decode(opcode) {
        Instruction::Move { size, source, dest } => {
            4 - 2 * u32::from(matches!(dest, Ea::AddressIndirectPredecrement(_)))
                + ea(source, size)
                + ea(dest, size)
        }
        Instruction::BitTest { source, dest } => {
            4 + 4 * u32::from(matches!(source, Ea::Immediate))
                + if data(dest) {
                    2
                } else if matches!(dest, Ea::Immediate) {
                    6
                } else {
                    ea(dest, OpSize::Byte)
                }
        }
        Instruction::Compare { size, source, dest } => compare(size, source, dest),
        _ => return None,
    })
}

fn jump(a: Ea) -> u32 {
    match a {
        Ea::AddressIndirect(_) => 8,
        Ea::AddressIndirectIndexed(_) | Ea::PcRelativeIndexed => 14,
        Ea::AbsoluteLong => 12,
        _ => 10,
    }
}
pub fn condition(c: BranchCondition, sr: u16) -> bool {
    let carry = sr & 1 != 0;
    let overflow = sr & 2 != 0;
    let zero = sr & 4 != 0;
    let negative = sr & 8 != 0;
    match c {
        BranchCondition::True => true,
        BranchCondition::False => false,
        BranchCondition::Higher => !carry && !zero,
        BranchCondition::LowerOrSame => carry || zero,
        BranchCondition::CarryClear => !carry,
        BranchCondition::CarrySet => carry,
        BranchCondition::NotEqual => !zero,
        BranchCondition::Equal => zero,
        BranchCondition::OverflowClear => !overflow,
        BranchCondition::OverflowSet => overflow,
        BranchCondition::Plus => !negative,
        BranchCondition::Minus => negative,
        BranchCondition::GreaterOrEqual => negative == overflow,
        BranchCondition::LessThan => negative != overflow,
        BranchCondition::GreaterThan => !zero && negative == overflow,
        BranchCondition::LessOrEqual => zero || negative != overflow,
    }
}
pub fn divu_cycles(mut dividend: u32, divisor: u16) -> u32 {
    if divisor == 0 {
        return 38;
    }
    if dividend / u32::from(divisor) > 65535 {
        return 10;
    }
    let divisor = u32::from(divisor) << 16;
    let mut cycles = 76;
    for _ in 0..15 {
        let high = dividend >> 31 != 0;
        dividend <<= 1;
        let borrow = dividend < divisor;
        if high || !borrow {
            dividend = dividend.wrapping_sub(divisor);
        }
        if !high {
            cycles += 2 + 2 * u32::from(borrow);
        }
    }
    cycles
}
pub fn divs_cycles(n: i32, d: i16) -> u32 {
    if d == 0 {
        return 38;
    }
    let d = i32::from(d);
    let q = n.wrapping_div(d);
    if !(i16::MIN as i32..=i16::MAX as i32).contains(&q) && n.wrapping_abs() >> 16 >= d.abs() {
        return 16 + 2 * u32::from(n < 0);
    }
    let mut q = q.wrapping_abs() as i16;
    let mut cycles = 120
        + 2 * u32::from(n < 0)
        + if d < 0 {
            2
        } else if n < 0 {
            4
        } else {
            0
        };
    for _ in 0..15 {
        cycles += 2 * u32::from(q >= 0);
        q = q.wrapping_shl(1);
    }
    cycles
}
/// Independently sums documented instruction timing. `operand` is the source
/// value read by the native operation; it is not a measured cycle count.
pub fn cycles(i: Instruction, cpu: &crate::CpuState, extension: u16, operand: u32) -> Option<u32> {
    use Instruction::*;
    let sr = cpu.sr as u16;
    if sr & 0x2000 == 0
        && matches!(
            i,
            MoveToSr(_)
                | AndToSr
                | OrToSr
                | ExclusiveOrToSr
                | MoveUsp(..)
                | Stop
                | ReturnFromException
        )
    {
        return Some(34);
    }
    let d = cpu.d;
    Some(match i {
        Move { size, source, dest } => {
            4 - 2 * u32::from(matches!(dest, Ea::AddressIndirectPredecrement(_)))
                + ea(source, size)
                + ea(dest, size)
        }
        MoveQuick(..) | Extend(..) | Swap(_) | NoOp | Stop => 4,
        Add {
            size,
            source,
            dest,
            with_extend,
        }
        | Subtract {
            size,
            source,
            dest,
            with_extend,
        } => {
            if with_extend {
                match (size, data(source)) {
                    (OpSize::LongWord, true) => 8,
                    (OpSize::LongWord, false) => 30,
                    (_, true) => 4,
                    (_, false) => 18,
                }
            } else {
                binary(size, source, dest)
            }
        }
        And { size, source, dest }
        | Or { size, source, dest }
        | ExclusiveOr { size, source, dest } => binary(size, source, dest),
        Compare { size, source, dest } => {
            if address(dest) {
                6 + ea(source, size)
            } else {
                compare(size, source, dest)
            }
        }
        Clear(s, a)
        | Not(s, a)
        | Negate {
            size: s, dest: a, ..
        } => unary(s, a),
        Test(s, a) => 4 + ea(a, s),
        Branch(c, disp) => {
            if condition(c, sr) {
                10
            } else if disp == 0 {
                12
            } else {
                8
            }
        }
        BranchDecrement(c, r) => {
            if condition(c, sr) {
                12
            } else if d[r.0 as usize] as u16 == 0 {
                14
            } else {
                10
            }
        }
        BranchToSubroutine(_) => 18,
        Jump(a) => jump(a),
        JumpToSubroutine(a) => 8 + jump(a),
        Return { restore_ccr } => {
            if restore_ccr {
                20
            } else {
                16
            }
        }
        ReturnFromException => 20,
        Link(_) => 16,
        Unlink(_) => 12,
        LoadEffectiveAddress(a, _) | PushEffectiveAddress(a) => {
            let n = if matches!(a, Ea::AddressIndirectIndexed(_) | Ea::PcRelativeIndexed) {
                12
            } else {
                ea(a, OpSize::Byte)
            };
            n + 8 * u32::from(matches!(i, PushEffectiveAddress(_)))
        }
        MultiplyUnsigned(_, a) => 38 + 2 * (operand as u16).count_ones() + ea(a, OpSize::Word),
        MultiplySigned(_, a) => {
            let v = operand as u16;
            38 + 2 * (v ^ (v << 1)).count_ones() + ea(a, OpSize::Word)
        }
        DivideUnsigned(r, a) => divu_cycles(d[r.0 as usize], operand as u16) + ea(a, OpSize::Word),
        DivideSigned(r, a) => {
            divs_cycles(d[r.0 as usize] as i32, operand as i16) + ea(a, OpSize::Word)
        }
        ArithmeticShiftRegister(s, _, _, c)
        | LogicalShiftRegister(s, _, _, c)
        | RotateRegister(s, _, _, c)
        | RotateThruExtendRegister(s, _, _, c) => {
            let count = match c {
                ShiftCount::Constant(n) => u32::from(n),
                ShiftCount::Register(r) => d[r.0 as usize] & 63,
            };
            (if s == OpSize::LongWord { 8 } else { 6 }) + 2 * count
        }
        ArithmeticShiftMemory(_, a)
        | LogicalShiftMemory(_, a)
        | RotateMemory(_, a)
        | RotateThruExtendMemory(_, a) => 8 + ea(a, OpSize::Word),
        MoveMultiple(s, a, dir) => {
            let count = extension.count_ones() * (if s == OpSize::LongWord { 8 } else { 4 });
            if matches!(a, Ea::AddressIndirectPredecrement(_)) {
                8 + count
            } else {
                (if dir == Direction::MemoryToRegister {
                    8
                } else {
                    4
                }) + ea(a, OpSize::Word)
                    + count
            }
        }
        BitTest { source, dest } => {
            4 + if matches!(source, Ea::Immediate) {
                4
            } else {
                0
            } + if data(dest) {
                2
            } else if matches!(dest, Ea::Immediate) {
                6
            } else {
                ea(dest, OpSize::Byte)
            }
        }
        BitTestAndChange { source, dest }
        | BitTestAndClear { source, dest }
        | BitTestAndSet { source, dest } => {
            4 + if matches!(source, Ea::Immediate) {
                4
            } else {
                0
            } + if data(dest) {
                2 + 2 * u32::from(operand % 32 >= 16)
                    + 2 * u32::from(matches!(i, BitTestAndClear { .. }))
            } else {
                4 + ea(dest, OpSize::Byte)
            }
        }
        Set(c, a) => {
            if data(a) {
                4 + 2 * u32::from(condition(c, sr))
            } else {
                8 + ea(a, OpSize::Byte)
            }
        }
        MoveFromSr(a) => {
            if data(a) {
                6
            } else {
                8 + ea(a, OpSize::Word)
            }
        }
        MoveToSr(a) | MoveToCcr(a) => 12 + ea(a, OpSize::Word),
        AndToSr | OrToSr | ExclusiveOrToSr | AndToCcr | OrToCcr | ExclusiveOrToCcr => 20,
        ExchangeAddress(..) | ExchangeData(..) | ExchangeDataAddress(..) => 6,
        MoveUsp(..) => 4,
        MovePeripheral(s, ..) => {
            if s == OpSize::LongWord {
                24
            } else {
                16
            }
        }
        Reset => 132,
        TestAndSet(a) => {
            if data(a) {
                4
            } else {
                10 + ea(a, OpSize::Byte)
            }
        }
        AddDecimal { source, .. } | SubtractDecimal { source, .. } => {
            if data(source) {
                6
            } else {
                18
            }
        }
        NegateDecimal(a) => {
            if data(a) {
                6
            } else {
                unary(OpSize::Byte, a)
            }
        }
        Trap(_) => 34,
        TrapOnOverflow => {
            if sr & 2 != 0 {
                34
            } else {
                4
            }
        }
        CheckRegister(r, a) => {
            let v = d[r.0 as usize] as i16;
            ea(a, OpSize::Word)
                + if v > operand as i16 {
                    38
                } else if v < 0 {
                    40
                } else {
                    10
                }
        }
        Illegal { .. } => 34,
    })
}
