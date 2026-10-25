//! Linear, typed, side-effect-explicit SSA at original instruction granularity.
use crate::{CpuState, ReadKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Value(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StateField {
    Data(u8),
    Address(u8),
    Usp,
    Ssp,
    Pc,
    Sr,
    Prefetch,
    PendingInterrupt,
    Stopped,
    Frozen,
    AddressError,
    LastMulDiv,
}
impl StateField {
    pub const COUNT: usize = 25;
    pub const fn from_index(index: usize) -> Option<Self> {
        Some(match index {
            0..=7 => Self::Data(index as u8),
            8..=14 => Self::Address((index - 8) as u8),
            15 => Self::Usp,
            16 => Self::Ssp,
            17 => Self::Pc,
            18 => Self::Sr,
            19 => Self::Prefetch,
            20 => Self::PendingInterrupt,
            21 => Self::Stopped,
            22 => Self::Frozen,
            23 => Self::AddressError,
            24 => Self::LastMulDiv,
            _ => return None,
        })
    }
    pub fn read(self, state: &CpuState) -> u32 {
        match self {
            Self::Data(n) => state.d[usize::from(n)],
            Self::Address(n) => state.a[usize::from(n)],
            Self::Usp => state.usp,
            Self::Ssp => state.ssp,
            Self::Pc => state.pc,
            Self::Sr => state.sr,
            Self::Prefetch => state.prefetch,
            Self::PendingInterrupt => state.pending_interrupt,
            Self::Stopped => state.stopped,
            Self::Frozen => state.frozen,
            Self::AddressError => state.address_error,
            Self::LastMulDiv => state.last_muldiv,
        }
    }
    pub const fn index(self) -> u32 {
        match self {
            Self::Data(n) => {
                assert!(n < 8);
                n as u32
            }
            Self::Address(n) => {
                assert!(n < 7);
                8 + n as u32
            }
            Self::Usp => 15,
            Self::Ssp => 16,
            Self::Pc => 17,
            Self::Sr => 18,
            Self::Prefetch => 19,
            Self::PendingInterrupt => 20,
            Self::Stopped => 21,
            Self::Frozen => 22,
            Self::AddressError => 23,
            Self::LastMulDiv => 24,
        }
    }
    pub const fn offset(self) -> u32 {
        4 * self.index()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    And,
    Or,
    Xor,
    ShiftLeft,
    ShiftRight,
    ArithmeticShiftRight,
    /// Divisor must be nonzero; MIN/-1 wraps. Guards establish this contract.
    DivideUnsigned,
    DivideSigned,
    RemainderUnsigned,
    RemainderSigned,
}
impl BinaryOp {
    /// Exact primitive semantics shared by compiler analyses and timing SSA.
    /// Division/remainder require a nonzero divisor, as established by IR guards.
    pub const fn evaluate(self, lhs: u32, rhs: u32) -> Option<u32> {
        Some(match self {
            Self::Add => lhs.wrapping_add(rhs),
            Self::Subtract => lhs.wrapping_sub(rhs),
            Self::Multiply => lhs.wrapping_mul(rhs),
            Self::And => lhs & rhs,
            Self::Or => lhs | rhs,
            Self::Xor => lhs ^ rhs,
            Self::ShiftLeft => lhs.wrapping_shl(rhs),
            Self::ShiftRight => lhs.wrapping_shr(rhs),
            Self::ArithmeticShiftRight => (lhs as i32).wrapping_shr(rhs) as u32,
            Self::DivideUnsigned
            | Self::DivideSigned
            | Self::RemainderUnsigned
            | Self::RemainderSigned
                if rhs == 0 =>
            {
                return None;
            }
            Self::DivideUnsigned => lhs / rhs,
            Self::DivideSigned => (lhs as i32).wrapping_div(rhs as i32) as u32,
            Self::RemainderUnsigned => lhs % rhs,
            Self::RemainderSigned => (lhs as i32).wrapping_rem(rhs as i32) as u32,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareOp {
    Equal,
    NotEqual,
    UnsignedLess,
    UnsignedLessEqual,
    SignedLess,
    SignedLessEqual,
}
impl CompareOp {
    pub const fn evaluate(self, lhs: u32, rhs: u32) -> u32 {
        (match self {
            Self::Equal => lhs == rhs,
            Self::NotEqual => lhs != rhs,
            Self::UnsignedLess => lhs < rhs,
            Self::UnsignedLessEqual => lhs <= rhs,
            Self::SignedLess => (lhs as i32) < rhs as i32,
            Self::SignedLessEqual => (lhs as i32) <= rhs as i32,
        }) as u32
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Width {
    Byte,
    Word,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExceptionExit {
    pub vector: u32,
    pub cycles: Value,
    pub saved_pc: Value,
    pub fault_address: Value,
    pub fault_status: Value,
}

/// Values are assigned exactly once. All arithmetic is 32-bit wrapping;
/// comparisons return 0/1; shift counts use their low five bits. IR contains no
/// runtime opcode or addressing-mode dispatch. Prefetches and discarded reads
/// remain observable and cannot be optimized away.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Op {
    Constant {
        dst: Value,
        value: u32,
    },
    LoadState {
        dst: Value,
        field: StateField,
    },
    StoreState {
        field: StateField,
        value: Value,
    },
    Binary {
        dst: Value,
        op: BinaryOp,
        lhs: Value,
        rhs: Value,
    },
    Compare {
        dst: Value,
        op: CompareOp,
        lhs: Value,
        rhs: Value,
    },
    Select {
        dst: Value,
        condition: Value,
        if_true: Value,
        if_false: Value,
    },
    ReadMemory {
        dst: Value,
        address: Value,
        width: Width,
        kind: ReadKind,
    },
    /// Used for taken-branch prefetch; returns zero without a bus access when
    /// condition is false. No speculative reads are permitted.
    ReadMemoryIf {
        dst: Value,
        condition: Value,
        address: Value,
        width: Width,
        kind: ReadKind,
    },
    WriteMemory {
        address: Value,
        value: Value,
        width: Width,
    },
    /// Advances only the reference VDP status-read compensation, not devices.
    SetReadOffset {
        cycles: Value,
    },
    /// Exits before subsequent operations if condition != 0. The scheduler
    /// performs the exception frame/vector sequence, then advances total cycles.
    ExitIf {
        condition: Value,
        exit: ExceptionExit,
    },
    /// The only normal interrupt-acceptance boundary. No batching is implied.
    Finish {
        cycles: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InstructionIr {
    pub pc: u32,
    pub next_pc: u32,
    pub opcode: u16,
    pub value_count: u32,
    pub ops: Vec<Op>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Program {
    pub instructions: Vec<InstructionIr>,
}
impl Program {
    pub fn instruction(&self, pc: u32) -> Option<&InstructionIr> {
        self.instructions
            .binary_search_by_key(&pc, |i| i.pc)
            .ok()
            .map(|i| &self.instructions[i])
    }
}
