//! Linear, typed, side-effect-explicit SSA at original instruction granularity.
use crate::ReadKind;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareOp {
    Equal,
    NotEqual,
    UnsignedLess,
    UnsignedLessEqual,
    SignedLess,
    SignedLessEqual,
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
