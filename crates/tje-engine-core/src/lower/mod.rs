//! Compile once from the user's immutable ROM; never dispatch decoded opcodes at execution.
mod arithmetic;
mod semantics;
mod shifts;

use crate::{
    CpuState, ReadKind,
    decode::{self, AddressingMode as Ea, BranchCondition, Instruction as I, OpSize as S},
    ir::*,
};
use BinaryOp as B;
use CompareOp as C;
use StateField as F;
use std::cell::{Cell, RefCell};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    OddPc(u32),
    Truncated(u32),
    InvalidMetadata(String),
}
impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OddPc(pc) => write!(f, "odd instruction PC {pc:08x}"),
            Self::Truncated(pc) => write!(f, "instruction extends beyond ROM at {pc:08x}"),
            Self::InvalidMetadata(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for CompileError {}

#[derive(Clone, Copy)]
enum Operand {
    D(u8),
    A(u8),
    Memory(Value, Option<(u8, Value)>),
    Immediate(Value),
}
enum WordSource<'a> {
    Bytes(&'a [u8]),
    Reader(&'a dyn Fn(u32) -> Option<u16>),
}
impl WordSource<'_> {
    fn word(&self, pc: u32) -> Option<u16> {
        match self {
            Self::Bytes(bytes) => {
                let start = pc as usize;
                let end = start.checked_add(2)?;
                bytes
                    .get(start..end)
                    .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
            }
            Self::Reader(reader) => reader(pc),
        }
    }
}

struct Builder<'a> {
    rom: WordSource<'a>,
    start: u32,
    position: Cell<u32>,
    opcode: u16,
    instruction: I,
    values: Cell<u32>,
    ops: RefCell<Vec<Op>>,
}

/// No linear scan over embedded data is performed here. `metadata::compile`
/// supplies vetted reachable instruction starts and rejects metadata conflicts.
pub fn compile_instruction(rom: &[u8], pc: u32) -> Result<InstructionIr, CompileError> {
    compile_instruction_with_divider(rom, pc, 7)
}

pub fn compile_instruction_with_divider(
    rom: &[u8],
    pc: u32,
    m68k_divider: u32,
) -> Result<InstructionIr, CompileError> {
    compile_instruction_source(WordSource::Bytes(rom), pc, m68k_divider)
}

/// Compile from a caller-owned immutable, big-endian word view without
/// retaining or copying a second ROM byte image. `None` has the same exact
/// truncation error as the slice APIs. Addresses are not masked or relocated.
///
/// This is compilation input, not an emulated bus read or an instruction/time
/// grant. The caller must supply stable ROM words, not mutable device samples.
pub fn compile_instruction_with_word_reader(
    reader: &dyn Fn(u32) -> Option<u16>,
    pc: u32,
    m68k_divider: u32,
) -> Result<InstructionIr, CompileError> {
    compile_instruction_source(WordSource::Reader(reader), pc, m68k_divider)
}

fn compile_instruction_source(
    rom: WordSource<'_>,
    pc: u32,
    m68k_divider: u32,
) -> Result<InstructionIr, CompileError> {
    if pc & 1 != 0 {
        return Err(CompileError::OddPc(pc));
    }
    let opcode = rom.word(pc).ok_or(CompileError::Truncated(pc))?;
    let instruction = decode::decode(opcode);
    let b = Builder {
        rom,
        start: pc,
        position: Cell::new(pc),
        opcode,
        instruction,
        values: Cell::new(0),
        ops: RefCell::new(Vec::with_capacity(96)),
    };
    let compensation =
        crate::timing::cycles_if_move_btst_cmp(opcode).map_or(8, |cycles| cycles - 4);
    let offset = b.k(compensation * m68k_divider);
    b.emit(Op::SetReadOffset { cycles: offset });
    b.store(F::AddressError, b.k(0));
    b.store(F::LastMulDiv, b.k(0));
    b.word()?;
    b.store(
        F::LastMulDiv,
        b.k(u32::from(matches!(
            instruction,
            I::MultiplySigned(..)
                | I::MultiplyUnsigned(..)
                | I::DivideSigned(..)
                | I::DivideUnsigned(..)
        ))),
    );
    let cycles = b.instruction()?;
    b.emit(Op::Finish { cycles });
    Ok(InstructionIr {
        pc,
        next_pc: b.position.get(),
        opcode,
        value_count: b.values.get(),
        ops: b.ops.into_inner(),
    })
}
impl Builder<'_> {
    fn emit(&self, op: Op) {
        self.ops.borrow_mut().push(op);
    }
    fn value(&self) -> Value {
        let n = self.values.get();
        self.values.set(n + 1);
        Value(n)
    }
    fn k(&self, value: u32) -> Value {
        let dst = self.value();
        self.emit(Op::Constant { dst, value });
        dst
    }
    fn load(&self, field: F) -> Value {
        let dst = self.value();
        self.emit(Op::LoadState { dst, field });
        dst
    }
    fn store(&self, field: F, value: Value) {
        self.emit(Op::StoreState { field, value });
    }
    fn bin(&self, op: B, lhs: Value, rhs: Value) -> Value {
        let dst = self.value();
        self.emit(Op::Binary { dst, op, lhs, rhs });
        dst
    }
    fn cmp(&self, op: C, lhs: Value, rhs: Value) -> Value {
        let dst = self.value();
        self.emit(Op::Compare { dst, op, lhs, rhs });
        dst
    }
    fn select(&self, condition: Value, if_true: Value, if_false: Value) -> Value {
        let dst = self.value();
        self.emit(Op::Select {
            dst,
            condition,
            if_true,
            if_false,
        });
        dst
    }
    fn add(&self, a: Value, b: Value) -> Value {
        self.bin(B::Add, a, b)
    }
    fn sub(&self, a: Value, b: Value) -> Value {
        self.bin(B::Subtract, a, b)
    }
    fn and(&self, a: Value, b: Value) -> Value {
        self.bin(B::And, a, b)
    }
    fn or(&self, a: Value, b: Value) -> Value {
        self.bin(B::Or, a, b)
    }
    fn xor(&self, a: Value, b: Value) -> Value {
        self.bin(B::Xor, a, b)
    }
    fn shl(&self, a: Value, n: u32) -> Value {
        self.bin(B::ShiftLeft, a, self.k(n))
    }
    fn shr(&self, a: Value, n: u32) -> Value {
        self.bin(B::ShiftRight, a, self.k(n))
    }
    fn not(&self, a: Value) -> Value {
        self.xor(a, self.k(u32::MAX))
    }
    fn truth(&self, a: Value) -> Value {
        self.cmp(C::NotEqual, a, self.k(0))
    }
    fn zero(&self, a: Value) -> Value {
        self.cmp(C::Equal, a, self.k(0))
    }
    fn bit(&self, a: Value, n: u32) -> Value {
        self.and(self.shr(a, n), self.k(1))
    }
    fn trunc(&self, a: Value, s: S) -> Value {
        if s == S::LongWord {
            a
        } else {
            self.and(a, self.k(s.mask()))
        }
    }
    fn signed(&self, a: Value, s: S) -> Value {
        if s == S::LongWord {
            a
        } else {
            self.bin(
                B::ArithmeticShiftRight,
                self.shl(a, 32 - s.bytes() * 8),
                self.k(32 - s.bytes() * 8),
            )
        }
    }
    fn a(&self, n: u8) -> Value {
        if n < 7 {
            self.load(F::Address(n))
        } else {
            self.select(
                self.and(self.load(F::Sr), self.k(0x2000)),
                self.load(F::Ssp),
                self.load(F::Usp),
            )
        }
    }
    fn set_a(&self, n: u8, value: Value) {
        if n < 7 {
            self.store(F::Address(n), value);
        } else {
            let supervisor = self.and(self.load(F::Sr), self.k(0x2000));
            self.store(F::Ssp, self.select(supervisor, value, self.load(F::Ssp)));
            self.store(F::Usp, self.select(supervisor, self.load(F::Usp), value));
        }
    }
    fn set_sr(&self, value: Value) {
        self.store(F::Sr, self.and(value, self.k(CpuState::SR_MASK)));
    }
    fn set_ccr(&self, value: Value) {
        self.store(
            F::Sr,
            self.or(
                self.and(self.load(F::Sr), self.k(!31)),
                self.and(value, self.k(31)),
            ),
        );
    }
    fn word(&self) -> Result<u16, CompileError> {
        let pc = self.position.get();
        let word = self.rom.word(pc).ok_or(CompileError::Truncated(pc))?;
        let next = pc.wrapping_add(2);
        let prefetch = self.read_raw(self.k(next), Width::Word, ReadKind::Prefetch);
        self.store(F::Prefetch, prefetch);
        self.store(F::Pc, self.k(next));
        self.position.set(next);
        Ok(word)
    }
    fn long(&self) -> Result<u32, CompileError> {
        Ok((u32::from(self.word()?) << 16) | u32::from(self.word()?))
    }
    fn read_raw(&self, address: Value, width: Width, kind: ReadKind) -> Value {
        let dst = self.value();
        self.emit(Op::ReadMemory {
            dst,
            address,
            width,
            kind,
        });
        dst
    }
    fn exception(&self, condition: Value, vector: u32, cycles: Value, saved_pc: Value) {
        let zero = self.k(0);
        self.emit(Op::ExitIf {
            condition,
            exit: ExceptionExit {
                vector,
                cycles,
                saved_pc,
                fault_address: zero,
                fault_status: zero,
            },
        });
    }
    fn privilege(&self) {
        self.exception(
            self.zero(self.and(self.load(F::Sr), self.k(0x2000))),
            8,
            self.k(34),
            self.k(self.start),
        );
    }
    fn address_check(&self, address: Value, write: bool, jump: bool, enabled: Option<Value>) {
        let odd = self.and(address, self.k(1));
        let condition = enabled.map_or(odd, |e| self.and(odd, e));
        let (dest, source) = self.address_modes();
        let correction = if write
            && matches!(dest, Some(Ea::AddressIndirectPredecrement(_)))
            && source.is_some()
        {
            0
        } else if write
            && matches!(dest, Some(Ea::AbsoluteLong))
            && source.is_some_and(Ea::is_memory)
        {
            4
        } else {
            2
        };
        let saved_pc = self.sub(self.load(F::Pc), self.k(correction));
        let rw = (!write) ^ matches!(self.instruction, I::MoveFromSr(_));
        let status = if jump {
            self.select(
                self.and(self.load(F::Sr), self.k(0x2000)),
                self.k(0x0e),
                self.k(0x0a),
            )
        } else {
            self.k(5)
        };
        let status = self.or(
            status,
            self.k(u32::from(self.opcode & 0xffe0) | (u32::from(rw) << 4)),
        );
        self.emit(Op::ExitIf {
            condition,
            exit: ExceptionExit {
                vector: 3,
                cycles: self.k(50),
                saved_pc,
                fault_address: address,
                fault_status: status,
            },
        });
    }
    fn read_mem(&self, address: Value, s: S, kind: ReadKind) -> Value {
        if s != S::Byte {
            self.address_check(address, false, false, None);
        }
        match s {
            S::Byte => self.read_raw(address, Width::Byte, kind),
            S::Word => self.read_raw(address, Width::Word, kind),
            S::LongWord => {
                let high = self.read_raw(address, Width::Word, kind);
                let low = self.read_raw(self.add(address, self.k(2)), Width::Word, kind);
                self.or(self.shl(high, 16), low)
            }
        }
    }
    fn write_mem(&self, address: Value, s: S, value: Value) {
        if s != S::Byte {
            self.address_check(address, true, false, None);
        }
        match s {
            S::Byte => self.emit(Op::WriteMemory {
                address,
                value: self.trunc(value, S::Byte),
                width: Width::Byte,
            }),
            S::Word => self.emit(Op::WriteMemory {
                address,
                value: self.trunc(value, S::Word),
                width: Width::Word,
            }),
            S::LongWord => {
                self.emit(Op::WriteMemory {
                    address,
                    value: self.shr(value, 16),
                    width: Width::Word,
                });
                self.emit(Op::WriteMemory {
                    address: self.add(address, self.k(2)),
                    value: self.trunc(value, S::Word),
                    width: Width::Word,
                });
            }
        }
    }
    fn index(&self, ext: u16) -> Value {
        let n = ((ext >> 12) & 7) as u8;
        let value = if ext & 0x8000 != 0 {
            self.a(n)
        } else {
            self.load(F::Data(n))
        };
        if ext & 0x800 != 0 {
            value
        } else {
            self.signed(value, S::Word)
        }
    }
    fn resolve(&self, e: Ea, s: S, apply_post: bool) -> Result<Operand, CompileError> {
        let operand = match e {
            Ea::DataDirect(r) => Operand::D(r.0),
            Ea::AddressDirect(r) => Operand::A(r.0),
            Ea::Quick(n) => Operand::Immediate(self.k(n.into())),
            Ea::Immediate => Operand::Immediate(self.k(if s == S::LongWord {
                self.long()?
            } else {
                u32::from(self.word()?) & s.mask()
            })),
            Ea::AbsoluteShort => Operand::Memory(self.k(self.word()? as i16 as u32), None),
            Ea::AbsoluteLong => Operand::Memory(self.k(self.long()?), None),
            Ea::AddressIndirect(r) => Operand::Memory(self.a(r.0), None),
            Ea::AddressIndirectPredecrement(r) => {
                let a = self.sub(
                    self.a(r.0),
                    self.k(if r.0 == 7 && s == S::Byte {
                        2
                    } else {
                        s.bytes()
                    }),
                );
                self.set_a(r.0, a);
                Operand::Memory(a, None)
            }
            Ea::AddressIndirectPostincrement(r) => {
                let a = self.a(r.0);
                Operand::Memory(
                    a,
                    Some((
                        r.0,
                        self.add(
                            a,
                            self.k(if r.0 == 7 && s == S::Byte {
                                2
                            } else {
                                s.bytes()
                            }),
                        ),
                    )),
                )
            }
            Ea::AddressIndirectDisplacement(r) => {
                let ext = self.word()?;
                Operand::Memory(self.add(self.a(r.0), self.k(ext as i16 as u32)), None)
            }
            Ea::AddressIndirectIndexed(r) => {
                let ext = self.word()?;
                Operand::Memory(
                    self.add(
                        self.add(self.a(r.0), self.index(ext)),
                        self.k(ext as i8 as u32),
                    ),
                    None,
                )
            }
            Ea::PcRelativeDisplacement => {
                let pc = self.position.get();
                let ext = self.word()?;
                Operand::Memory(self.k(pc.wrapping_add(ext as i16 as u32)), None)
            }
            Ea::PcRelativeIndexed => {
                let pc = self.position.get();
                let ext = self.word()?;
                Operand::Memory(
                    self.add(self.k(pc.wrapping_add(ext as i8 as u32)), self.index(ext)),
                    None,
                )
            }
        };
        if apply_post {
            self.post(operand);
        }
        Ok(operand)
    }
    fn post(&self, o: Operand) {
        if let Operand::Memory(_, Some((r, a))) = o {
            self.set_a(r, a);
        }
    }
    fn read(&self, o: Operand, s: S, kind: ReadKind) -> Value {
        self.trunc(
            match o {
                Operand::D(n) => self.load(F::Data(n)),
                Operand::A(n) => self.a(n),
                Operand::Immediate(v) => v,
                Operand::Memory(a, _) => self.read_mem(a, s, kind),
            },
            s,
        )
    }
    fn read_ea(&self, e: Ea, s: S) -> Result<Value, CompileError> {
        let o = self.resolve(e, s, true)?;
        Ok(self.read(o, s, ReadKind::Semantic))
    }
    fn write(&self, o: Operand, s: S, value: Value) {
        let value = self.trunc(value, s);
        match o {
            Operand::D(n) => self.store(
                F::Data(n),
                if s == S::LongWord {
                    value
                } else {
                    self.or(self.and(self.load(F::Data(n)), self.k(!s.mask())), value)
                },
            ),
            Operand::A(n) => self.set_a(
                n,
                if s == S::Word {
                    self.signed(value, S::Word)
                } else {
                    value
                },
            ),
            Operand::Memory(a, _) => self.write_mem(a, s, value),
            Operand::Immediate(_) => unreachable!("decoder forbids immediate destination"),
        }
    }
    fn write_ea(&self, e: Ea, s: S, value: Value) -> Result<(), CompileError> {
        let o = self.resolve(e, s, false)?;
        self.write(o, s, value);
        self.post(o);
        Ok(())
    }
    fn memory_address(&self, e: Ea) -> Result<Value, CompileError> {
        match self.resolve(e, S::LongWord, false)? {
            Operand::Memory(a, _) => Ok(a),
            _ => unreachable!("decoder forbids noncontrol EA"),
        }
    }
    fn push(&self, value: Value, s: S) {
        let a = self.sub(self.a(7), self.k(s.bytes()));
        self.set_a(7, a);
        self.write_mem(a, s, value);
    }
    fn pop(&self, s: S) -> Value {
        let a = self.a(7);
        let value = self.read_mem(a, s, ReadKind::Semantic);
        self.set_a(7, self.add(a, self.k(s.bytes())));
        value
    }
    fn jump(&self, target: Value, condition: Option<Value>) {
        let old_pc = self.load(F::Pc);
        let before = self.sub(target, self.k(2));
        self.store(
            F::Pc,
            condition.map_or(before, |c| self.select(c, before, old_pc)),
        );
        self.address_check(target, false, true, condition);
        let pref = if let Some(condition) = condition {
            let dst = self.value();
            self.emit(Op::ReadMemoryIf {
                dst,
                condition,
                address: target,
                width: Width::Word,
                kind: ReadKind::Prefetch,
            });
            self.select(condition, dst, self.load(F::Prefetch))
        } else {
            self.read_raw(target, Width::Word, ReadKind::Prefetch)
        };
        self.store(F::Prefetch, pref);
        self.store(
            F::Pc,
            condition.map_or(target, |c| self.select(c, target, old_pc)),
        );
    }
    fn nz(&self, value: Value, s: S) {
        let value = self.trunc(value, s);
        self.store(
            F::Sr,
            self.or(
                self.and(self.load(F::Sr), self.k(!15)),
                self.or(
                    self.shl(self.zero(value), 2),
                    self.shl(self.truth(self.and(value, self.k(s.sign()))), 3),
                ),
            ),
        );
    }
    fn condition(&self, c: BranchCondition) -> Value {
        use BranchCondition::*;
        let sr = self.load(F::Sr);
        let carry = self.bit(sr, 0);
        let overflow = self.bit(sr, 1);
        let zero = self.bit(sr, 2);
        let negative = self.bit(sr, 3);
        match c {
            True => self.k(1),
            False => self.k(0),
            Higher => self.zero(self.or(carry, zero)),
            LowerOrSame => self.or(carry, zero),
            CarryClear => self.zero(carry),
            CarrySet => carry,
            NotEqual => self.zero(zero),
            Equal => zero,
            OverflowClear => self.zero(overflow),
            OverflowSet => overflow,
            Plus => self.zero(negative),
            Minus => negative,
            GreaterOrEqual => self.cmp(C::Equal, negative, overflow),
            LessThan => self.xor(negative, overflow),
            GreaterThan => self.and(self.zero(zero), self.cmp(C::Equal, negative, overflow)),
            LessOrEqual => self.or(zero, self.xor(negative, overflow)),
        }
    }
    fn address_modes(&self) -> (Option<Ea>, Option<Ea>) {
        use I::*;
        let dest = match self.instruction {
            Add { dest, .. }
            | AddDecimal { dest, .. }
            | And { dest, .. }
            | Clear(_, dest)
            | Compare { dest, .. }
            | ExclusiveOr { dest, .. }
            | Move { dest, .. }
            | MoveFromSr(dest)
            | Negate { dest, .. }
            | Not(_, dest)
            | Or { dest, .. }
            | Set(_, dest)
            | Subtract { dest, .. }
            | SubtractDecimal { dest, .. }
            | TestAndSet(dest) => Some(dest),
            _ => None,
        };
        let source = match self.instruction {
            Add { source, .. }
            | AddDecimal { source, .. }
            | And { source, .. }
            | ArithmeticShiftMemory(_, source)
            | BitTest { source, .. }
            | BitTestAndChange { source, .. }
            | BitTestAndClear { source, .. }
            | BitTestAndSet { source, .. }
            | CheckRegister(_, source)
            | Compare { source, .. }
            | DivideSigned(_, source)
            | DivideUnsigned(_, source)
            | ExclusiveOr { source, .. }
            | LoadEffectiveAddress(source, _)
            | LogicalShiftMemory(_, source)
            | Jump(source)
            | JumpToSubroutine(source)
            | Move { source, .. }
            | MoveToCcr(source)
            | MoveToSr(source)
            | MultiplySigned(_, source)
            | MultiplyUnsigned(_, source)
            | Or { source, .. }
            | PushEffectiveAddress(source)
            | RotateMemory(_, source)
            | RotateThruExtendMemory(_, source)
            | Subtract { source, .. }
            | SubtractDecimal { source, .. }
            | Test(_, source) => Some(source),
            _ => None,
        };
        (dest, source)
    }
}
