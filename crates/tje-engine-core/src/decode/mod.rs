//! GPL-3.0 instruction vocabulary and encoding rules adapted from jgenesis.
//! No cartridge bytes or decoded cartridge instructions are embedded.
mod table;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    RegisterToMemory,
    MemoryToRegister,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UspDirection {
    RegisterToUsp,
    UspToRegister,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftDirection {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftCount {
    Constant(u8),
    Register(DataRegister),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchCondition {
    True,
    False,
    Higher,
    LowerOrSame,
    CarryClear,
    CarrySet,
    NotEqual,
    Equal,
    OverflowClear,
    OverflowSet,
    Plus,
    Minus,
    GreaterOrEqual,
    LessThan,
    GreaterThan,
    LessOrEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instruction {
    Add {
        size: OpSize,
        source: AddressingMode,
        dest: AddressingMode,
        with_extend: bool,
    },
    AddDecimal {
        source: AddressingMode,
        dest: AddressingMode,
    },
    And {
        size: OpSize,
        source: AddressingMode,
        dest: AddressingMode,
    },
    AndToCcr,
    AndToSr,
    ArithmeticShiftMemory(ShiftDirection, AddressingMode),
    ArithmeticShiftRegister(OpSize, ShiftDirection, DataRegister, ShiftCount),
    BitTest {
        source: AddressingMode,
        dest: AddressingMode,
    },
    BitTestAndChange {
        source: AddressingMode,
        dest: AddressingMode,
    },
    BitTestAndClear {
        source: AddressingMode,
        dest: AddressingMode,
    },
    BitTestAndSet {
        source: AddressingMode,
        dest: AddressingMode,
    },
    Branch(BranchCondition, i8),
    BranchDecrement(BranchCondition, DataRegister),
    BranchToSubroutine(i8),
    CheckRegister(DataRegister, AddressingMode),
    Clear(OpSize, AddressingMode),
    Compare {
        size: OpSize,
        source: AddressingMode,
        dest: AddressingMode,
    },
    DivideSigned(DataRegister, AddressingMode),
    DivideUnsigned(DataRegister, AddressingMode),
    ExchangeAddress(AddressRegister, AddressRegister),
    ExchangeData(DataRegister, DataRegister),
    ExchangeDataAddress(DataRegister, AddressRegister),
    ExclusiveOr {
        size: OpSize,
        source: AddressingMode,
        dest: AddressingMode,
    },
    ExclusiveOrToCcr,
    ExclusiveOrToSr,
    Extend(OpSize, DataRegister),
    Illegal {
        opcode: u16,
    },
    Jump(AddressingMode),
    JumpToSubroutine(AddressingMode),
    Link(AddressRegister),
    LoadEffectiveAddress(AddressingMode, AddressRegister),
    LogicalShiftMemory(ShiftDirection, AddressingMode),
    LogicalShiftRegister(OpSize, ShiftDirection, DataRegister, ShiftCount),
    Move {
        size: OpSize,
        source: AddressingMode,
        dest: AddressingMode,
    },
    MoveFromSr(AddressingMode),
    MoveMultiple(OpSize, AddressingMode, Direction),
    MovePeripheral(OpSize, DataRegister, AddressRegister, Direction),
    MoveQuick(i8, DataRegister),
    MoveToCcr(AddressingMode),
    MoveToSr(AddressingMode),
    MoveUsp(UspDirection, AddressRegister),
    MultiplySigned(DataRegister, AddressingMode),
    MultiplyUnsigned(DataRegister, AddressingMode),
    Negate {
        size: OpSize,
        dest: AddressingMode,
        with_extend: bool,
    },
    NegateDecimal(AddressingMode),
    NoOp,
    Not(OpSize, AddressingMode),
    Or {
        size: OpSize,
        source: AddressingMode,
        dest: AddressingMode,
    },
    OrToCcr,
    OrToSr,
    PushEffectiveAddress(AddressingMode),
    Reset,
    Return {
        restore_ccr: bool,
    },
    ReturnFromException,
    RotateMemory(ShiftDirection, AddressingMode),
    RotateRegister(OpSize, ShiftDirection, DataRegister, ShiftCount),
    RotateThruExtendMemory(ShiftDirection, AddressingMode),
    RotateThruExtendRegister(OpSize, ShiftDirection, DataRegister, ShiftCount),
    Set(BranchCondition, AddressingMode),
    Subtract {
        size: OpSize,
        source: AddressingMode,
        dest: AddressingMode,
        with_extend: bool,
    },
    SubtractDecimal {
        source: AddressingMode,
        dest: AddressingMode,
    },
    Swap(DataRegister),
    Stop,
    Test(OpSize, AddressingMode),
    TestAndSet(AddressingMode),
    Trap(u32),
    TrapOnOverflow,
    Unlink(AddressRegister),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpSize {
    Byte,
    Word,
    LongWord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressingMode {
    DataDirect(DataRegister),
    AddressDirect(AddressRegister),
    AddressIndirect(AddressRegister),
    AddressIndirectPostincrement(AddressRegister),
    AddressIndirectPredecrement(AddressRegister),
    AddressIndirectDisplacement(AddressRegister),
    AddressIndirectIndexed(AddressRegister),
    PcRelativeDisplacement,
    PcRelativeIndexed,
    AbsoluteShort,
    AbsoluteLong,
    Immediate,
    Quick(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataRegister(pub u8);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressRegister(pub u8);
impl DataRegister {
    pub const ALL: [Self; 8] = [
        Self(0),
        Self(1),
        Self(2),
        Self(3),
        Self(4),
        Self(5),
        Self(6),
        Self(7),
    ];
}
impl AddressRegister {
    pub const ALL: [Self; 8] = [
        Self(0),
        Self(1),
        Self(2),
        Self(3),
        Self(4),
        Self(5),
        Self(6),
        Self(7),
    ];
}
impl From<u8> for DataRegister {
    fn from(v: u8) -> Self {
        Self(v)
    }
}
impl From<u8> for AddressRegister {
    fn from(v: u8) -> Self {
        Self(v)
    }
}
impl OpSize {
    pub const ALL: [Self; 3] = [Self::Byte, Self::Word, Self::LongWord];
    pub const fn bytes(self) -> u32 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
            Self::LongWord => 4,
        }
    }
    pub const fn mask(self) -> u32 {
        match self {
            Self::Byte => 255,
            Self::Word => 65535,
            Self::LongWord => u32::MAX,
        }
    }
    pub const fn sign(self) -> u32 {
        match self {
            Self::Byte => 128,
            Self::Word => 32768,
            Self::LongWord => 1 << 31,
        }
    }
}
impl AddressingMode {
    pub const fn is_data_direct(self) -> bool {
        matches!(self, Self::DataDirect(_))
    }
    pub const fn is_address_direct(self) -> bool {
        matches!(self, Self::AddressDirect(_))
    }
    pub const fn is_memory(self) -> bool {
        !matches!(
            self,
            Self::DataDirect(_) | Self::AddressDirect(_) | Self::Immediate | Self::Quick(_)
        )
    }
}
impl BranchCondition {
    pub const ALL: [Self; 16] = [
        Self::True,
        Self::False,
        Self::Higher,
        Self::LowerOrSame,
        Self::CarryClear,
        Self::CarrySet,
        Self::NotEqual,
        Self::Equal,
        Self::OverflowClear,
        Self::OverflowSet,
        Self::Plus,
        Self::Minus,
        Self::GreaterOrEqual,
        Self::LessThan,
        Self::GreaterThan,
        Self::LessOrEqual,
    ];
}
pub fn decode(opcode: u16) -> Instruction {
    table::decode(opcode)
}
