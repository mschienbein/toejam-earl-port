use super::*;
use crate::decode::{Direction, UspDirection};

impl Builder<'_> {
    pub(super) fn instruction(&self) -> Result<Value, CompileError> {
        use I::*;
        let static_cycles = || {
            self.k(crate::timing::cycles(
                self.instruction,
                &CpuState {
                    sr: 0x2700,
                    ..CpuState::default()
                },
                0,
                0,
            )
            .expect("full ISA timing"))
        };
        Ok(match self.instruction {
            Move { size, source, dest } => {
                let value = self.read_ea(source, size)?;
                if !dest.is_address_direct() {
                    self.nz(value, size);
                }
                if let (S::LongWord, Ea::AddressIndirectPredecrement(r)) = (size, dest) {
                    let a = self.sub(self.a(r.0), self.k(2));
                    self.set_a(r.0, a);
                    self.write_mem(a, S::Word, value);
                    let a = self.sub(a, self.k(2));
                    self.set_a(r.0, a);
                    self.write_mem(a, S::Word, self.shr(value, 16));
                } else {
                    self.write_ea(dest, size, value)?;
                }
                static_cycles()
            }
            MoveQuick(value, r) => {
                let value = self.k(value as u32);
                self.store(F::Data(r.0), value);
                self.nz(value, S::LongWord);
                static_cycles()
            }
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
                let subtract = matches!(self.instruction, Subtract { .. });
                if let Ea::AddressDirect(r) = dest {
                    let source = self.read_ea(source, size)?;
                    let source = if size == S::Word {
                        self.signed(source, S::Word)
                    } else {
                        source
                    };
                    let out = if subtract {
                        self.sub(self.a(r.0), source)
                    } else {
                        self.add(self.a(r.0), source)
                    };
                    self.set_a(r.0, out);
                } else {
                    let (o, l, r) = if with_extend {
                        let (_, r) = self.extend_read(source, size)?;
                        let (o, l) = self.extend_read(dest, size)?;
                        (o, l, r)
                    } else {
                        let r = self.read_ea(source, size)?;
                        let o = self.resolve(dest, size, true)?;
                        (o, self.read(o, size, ReadKind::Semantic), r)
                    };
                    let out = self.arithmetic(l, r, size, subtract, with_extend, false);
                    self.write(o, size, out);
                }
                static_cycles()
            }
            Compare { size, source, dest } => {
                let r = self.read_ea(source, size)?;
                if let Ea::AddressDirect(n) = dest {
                    self.arithmetic(
                        self.a(n.0),
                        if size == S::Word {
                            self.signed(r, S::Word)
                        } else {
                            r
                        },
                        S::LongWord,
                        true,
                        false,
                        true,
                    );
                } else {
                    let l = self.read_ea(dest, size)?;
                    self.arithmetic(l, r, size, true, false, true);
                }
                static_cycles()
            }
            And { size, source, dest }
            | Or { size, source, dest }
            | ExclusiveOr { size, source, dest } => {
                let source = self.read_ea(source, size)?;
                let dest = self.resolve(dest, size, true)?;
                let old = self.read(dest, size, ReadKind::Semantic);
                let out = self.bin(
                    match self.instruction {
                        And { .. } => B::And,
                        Or { .. } => B::Or,
                        _ => B::Xor,
                    },
                    old,
                    source,
                );
                self.nz(out, size);
                self.write(dest, size, out);
                static_cycles()
            }
            Clear(size, dest) | Not(size, dest) | Negate { size, dest, .. } => {
                let o = self.resolve(dest, size, true)?;
                let out = if matches!(self.instruction, Clear(..)) {
                    if matches!(o, Operand::Memory(..)) {
                        self.read(o, size, ReadKind::Discarded);
                    }
                    let value = self.k(0);
                    self.nz(value, size);
                    value
                } else {
                    let value = self.read(o, size, ReadKind::Semantic);
                    if let Negate { with_extend, .. } = self.instruction {
                        self.arithmetic(self.k(0), value, size, true, with_extend, false)
                    } else {
                        let value = self.trunc(self.not(value), size);
                        self.nz(value, size);
                        value
                    }
                };
                self.write(o, size, out);
                static_cycles()
            }
            Test(size, source) => {
                let value = self.read_ea(source, size)?;
                self.nz(value, size);
                static_cycles()
            }
            Extend(size, r) => {
                let source = if size == S::Word { S::Byte } else { S::Word };
                let value = self.signed(self.load(F::Data(r.0)), source);
                self.write(Operand::D(r.0), size, value);
                self.nz(value, size);
                static_cycles()
            }
            Swap(r) => {
                let old = self.load(F::Data(r.0));
                let value = self.or(self.shl(old, 16), self.shr(old, 16));
                self.store(F::Data(r.0), value);
                self.nz(value, S::LongWord);
                static_cycles()
            }
            ExchangeData(x, y) => {
                let a = self.load(F::Data(x.0));
                let b = self.load(F::Data(y.0));
                self.store(F::Data(x.0), b);
                self.store(F::Data(y.0), a);
                static_cycles()
            }
            ExchangeAddress(x, y) => {
                let a = self.a(x.0);
                let b = self.a(y.0);
                self.set_a(x.0, b);
                self.set_a(y.0, a);
                static_cycles()
            }
            ExchangeDataAddress(x, y) => {
                let a = self.load(F::Data(x.0));
                let b = self.a(y.0);
                self.store(F::Data(x.0), b);
                self.set_a(y.0, a);
                static_cycles()
            }
            MultiplyUnsigned(r, source) => self.multiply(r.0, source, false)?,
            MultiplySigned(r, source) => self.multiply(r.0, source, true)?,
            DivideUnsigned(r, source) => self.divide(r.0, source, false)?,
            DivideSigned(r, source) => self.divide(r.0, source, true)?,
            AddDecimal { source, dest } | SubtractDecimal { source, dest } => {
                let r = self.read_ea(source, S::Byte)?;
                let o = self.resolve(dest, S::Byte, false)?;
                let l = self.read(o, S::Byte, ReadKind::Semantic);
                let out = self.decimal(l, r, matches!(self.instruction, SubtractDecimal { .. }));
                self.write(o, S::Byte, out);
                static_cycles()
            }
            NegateDecimal(dest) => {
                let o = self.resolve(dest, S::Byte, true)?;
                let r = self.read(o, S::Byte, ReadKind::Semantic);
                let out = self.decimal(self.k(0), r, true);
                self.write(o, S::Byte, out);
                static_cycles()
            }
            Branch(condition, displacement) => {
                let displacement = if displacement == 0 {
                    self.word()? as i16 as u32
                } else {
                    displacement as u32
                };
                let take = self.condition(condition);
                self.jump(
                    self.k(self.start.wrapping_add(2).wrapping_add(displacement)),
                    Some(take),
                );
                self.select(
                    take,
                    self.k(10),
                    self.k(if self.opcode & 255 == 0 { 12 } else { 8 }),
                )
            }
            BranchDecrement(condition, r) => {
                let displacement = self.word()? as i16 as u32;
                let condition = self.condition(condition);
                let old = self.load(F::Data(r.0));
                let word = self.trunc(old, S::Word);
                let updated = self.or(
                    self.and(old, self.k(!65535)),
                    self.trunc(self.sub(word, self.k(1)), S::Word),
                );
                self.store(F::Data(r.0), self.select(condition, old, updated));
                let take = self.and(self.zero(condition), self.truth(word));
                self.jump(
                    self.k(self.start.wrapping_add(2).wrapping_add(displacement)),
                    Some(take),
                );
                self.select(
                    condition,
                    self.k(12),
                    self.select(self.zero(word), self.k(14), self.k(10)),
                )
            }
            BranchToSubroutine(displacement) => {
                let displacement = if displacement == 0 {
                    self.word()? as i16 as u32
                } else {
                    displacement as u32
                };
                self.push(self.load(F::Pc), S::LongWord);
                self.jump(
                    self.k(self.start.wrapping_add(2).wrapping_add(displacement)),
                    None,
                );
                static_cycles()
            }
            Jump(source) | JumpToSubroutine(source) => {
                let target = self.memory_address(source)?;
                let pc = self.load(F::Pc);
                self.jump(target, None);
                if matches!(self.instruction, JumpToSubroutine(_)) {
                    self.push(pc, S::LongWord);
                }
                static_cycles()
            }
            LoadEffectiveAddress(source, r) => {
                let value = self.memory_address(source)?;
                self.set_a(r.0, value);
                static_cycles()
            }
            PushEffectiveAddress(source) => {
                let value = self.memory_address(source)?;
                self.push(value, S::LongWord);
                static_cycles()
            }
            Link(r) => {
                let displacement = self.word()? as i16 as u32;
                let value = if r.0 == 7 {
                    self.sub(self.a(7), self.k(4))
                } else {
                    self.a(r.0)
                };
                self.push(value, S::LongWord);
                let sp = self.a(7);
                self.set_a(r.0, sp);
                self.set_a(7, self.add(sp, self.k(displacement)));
                static_cycles()
            }
            Unlink(r) => {
                self.set_a(7, self.a(r.0));
                let value = self.pop(S::LongWord);
                self.set_a(r.0, value);
                static_cycles()
            }
            Return { restore_ccr } => {
                if restore_ccr {
                    let ccr = self.pop(S::Word);
                    self.set_ccr(ccr);
                }
                let pc = self.pop(S::LongWord);
                self.jump(pc, None);
                static_cycles()
            }
            ReturnFromException => {
                self.privilege();
                let sr = self.pop(S::Word);
                let pc = self.pop(S::LongWord);
                self.set_sr(sr);
                self.jump(pc, None);
                static_cycles()
            }
            MoveFromSr(dest) => {
                let o = self.resolve(dest, S::Word, true)?;
                self.write(o, S::Word, self.load(F::Sr));
                static_cycles()
            }
            MoveToCcr(source) => {
                let value = self.read_ea(source, S::Word)?;
                self.set_ccr(value);
                static_cycles()
            }
            MoveToSr(source) => {
                self.privilege();
                let value = self.read_ea(source, S::Word)?;
                self.set_sr(value);
                static_cycles()
            }
            MoveUsp(direction, r) => {
                self.privilege();
                match direction {
                    UspDirection::RegisterToUsp => self.store(F::Usp, self.a(r.0)),
                    UspDirection::UspToRegister => self.set_a(r.0, self.load(F::Usp)),
                };
                static_cycles()
            }
            AndToCcr | OrToCcr | ExclusiveOrToCcr | AndToSr | OrToSr | ExclusiveOrToSr => {
                let full = matches!(self.instruction, AndToSr | OrToSr | ExclusiveOrToSr);
                if full {
                    self.privilege();
                }
                let value = self.k(self.word()?.into());
                let op = match self.instruction {
                    AndToCcr | AndToSr => B::And,
                    OrToCcr | OrToSr => B::Or,
                    _ => B::Xor,
                };
                let value = self.bin(op, self.load(F::Sr), value);
                if full {
                    self.set_sr(value);
                } else {
                    self.set_ccr(value);
                }
                static_cycles()
            }
            Stop => {
                self.privilege();
                let sr = self.k(self.word()?.into());
                self.set_sr(sr);
                self.store(F::Stopped, self.k(1));
                static_cycles()
            }
            NoOp | Reset => static_cycles(),
            Trap(vector) => {
                self.exception(self.k(1), 32 + vector, self.k(34), self.load(F::Pc));
                self.k(34)
            }
            TrapOnOverflow => {
                self.exception(
                    self.and(self.load(F::Sr), self.k(2)),
                    7,
                    self.k(34),
                    self.load(F::Pc),
                );
                self.k(4)
            }
            Illegal { opcode } => {
                let vector = match opcode >> 12 {
                    10 => 10,
                    15 => 11,
                    _ => 4,
                };
                self.exception(self.k(1), vector, self.k(34), self.k(self.start));
                self.k(34)
            }
            CheckRegister(r, source) => {
                let upper = self.signed(self.read_ea(source, S::Word)?, S::Word);
                let value = self.signed(self.load(F::Data(r.0)), S::Word);
                let negative = self.bit(value, 31);
                let above = self.cmp(C::SignedLess, upper, value);
                let trap = self.or(above, negative);
                let sr = self.load(F::Sr);
                self.store(
                    F::Sr,
                    self.or(
                        self.and(sr, self.k(!15)),
                        self.select(trap, self.shl(negative, 3), self.and(sr, self.k(8))),
                    ),
                );
                let cycles = self.add(
                    self.k(crate::timing::ea(source, S::Word)),
                    self.select(above, self.k(38), self.k(40)),
                );
                self.exception(trap, 6, cycles, self.load(F::Pc));
                self.k(10 + crate::timing::ea(source, S::Word))
            }
            Set(condition, dest) => {
                let condition = self.condition(condition);
                self.write_ea(
                    dest,
                    S::Byte,
                    self.select(condition, self.k(255), self.k(0)),
                )?;
                if dest.is_data_direct() {
                    self.add(self.k(4), self.shl(condition, 1))
                } else {
                    self.k(8 + crate::timing::ea(dest, S::Byte))
                }
            }
            BitTest { source, dest }
            | BitTestAndChange { source, dest }
            | BitTestAndClear { source, dest }
            | BitTestAndSet { source, dest } => {
                let index = self.read_ea(source, S::Byte)?;
                let size = if dest.is_data_direct() {
                    S::LongWord
                } else {
                    S::Byte
                };
                let o = self.resolve(dest, size, true)?;
                let old = self.read(o, size, ReadKind::Semantic);
                let bit = self.and(index, self.k(if size == S::LongWord { 31 } else { 7 }));
                let mask = self.bin(B::ShiftLeft, self.k(1), bit);
                let zero = self.zero(self.and(old, mask));
                self.store(
                    F::Sr,
                    self.or(self.and(self.load(F::Sr), self.k(!4)), self.shl(zero, 2)),
                );
                if !matches!(self.instruction, BitTest { .. }) {
                    let value = match self.instruction {
                        BitTestAndChange { .. } => self.xor(old, mask),
                        BitTestAndClear { .. } => self.and(old, self.not(mask)),
                        _ => self.or(old, mask),
                    };
                    self.write(o, size, value);
                }
                if dest.is_data_direct() && !matches!(self.instruction, BitTest { .. }) {
                    self.add(static_cycles(), self.shl(self.bit(index, 4), 1))
                } else {
                    static_cycles()
                }
            }
            TestAndSet(dest) => {
                let o = self.resolve(dest, S::Byte, true)?;
                let old = self.read(o, S::Byte, ReadKind::Semantic);
                self.nz(old, S::Byte);
                if dest.is_data_direct() {
                    self.write(o, S::Byte, self.or(old, self.k(128)));
                }
                static_cycles()
            }
            MoveMultiple(size, ea, direction) => self.movem(size, ea, direction)?,
            MovePeripheral(size, d, a, direction) => {
                let ext = self.word()? as i16 as u32;
                let address = self.add(self.a(a.0), self.k(ext));
                let old = self.load(F::Data(d.0));
                let mut out = self.k(0);
                for i in 0..size.bytes() {
                    let address = self.add(address, self.k(2 * i));
                    let shift = 8 * (size.bytes() - 1 - i);
                    if direction == Direction::RegisterToMemory {
                        self.write_mem(address, S::Byte, self.shr(old, shift));
                    } else {
                        out = self.or(
                            self.shl(out, 8),
                            self.read_mem(address, S::Byte, ReadKind::Semantic),
                        );
                    }
                }
                if direction == Direction::MemoryToRegister {
                    self.write(Operand::D(d.0), size, out);
                }
                static_cycles()
            }
            ArithmeticShiftRegister(size, direction, r, count)
            | LogicalShiftRegister(size, direction, r, count)
            | RotateRegister(size, direction, r, count)
            | RotateThruExtendRegister(size, direction, r, count) => {
                self.shift_register(size, direction, r.0, count)
            }
            ArithmeticShiftMemory(direction, dest)
            | LogicalShiftMemory(direction, dest)
            | RotateMemory(direction, dest)
            | RotateThruExtendMemory(direction, dest) => {
                let o = self.resolve(dest, S::Word, true)?;
                let old = self.read(o, S::Word, ReadKind::Semantic);
                let out = self.shift(old, S::Word, direction, self.k(1), Some(1));
                self.write(o, S::Word, out);
                static_cycles()
            }
        })
    }
    fn movem(&self, size: S, ea: Ea, direction: Direction) -> Result<Value, CompileError> {
        let mask = self.word()?;
        if let Ea::AddressIndirectPredecrement(r) = ea {
            let mut address = self.a(r.0);
            for bit in 0..16 {
                if mask & (1 << bit) == 0 {
                    continue;
                }
                let n = 15 - bit;
                let value = if n < 8 {
                    self.load(F::Data(n))
                } else {
                    self.a(n - 8)
                };
                if size == S::LongWord {
                    address = self.sub(address, self.k(2));
                    self.write_mem(address, S::Word, value);
                    address = self.sub(address, self.k(2));
                    self.write_mem(address, S::Word, self.shr(value, 16));
                } else {
                    address = self.sub(address, self.k(2));
                    self.write_mem(address, S::Word, value);
                }
            }
            self.set_a(r.0, address);
        } else {
            let resolved = self.resolve(ea, size, false)?;
            let Operand::Memory(mut address, post) = resolved else {
                unreachable!()
            };
            for n in 0..16 {
                if mask & (1 << n) == 0 {
                    continue;
                }
                if direction == Direction::RegisterToMemory {
                    let value = if n < 8 {
                        self.load(F::Data(n))
                    } else {
                        self.a(n - 8)
                    };
                    self.write_mem(address, size, value);
                } else {
                    if let Some((r, _)) = post {
                        self.set_a(r, self.add(address, self.k(2)));
                    }
                    let value = self.read_mem(address, size, ReadKind::Semantic);
                    let value = if size == S::Word {
                        self.signed(value, S::Word)
                    } else {
                        value
                    };
                    if n < 8 {
                        self.store(F::Data(n), value);
                    } else if post.is_none_or(|(r, _)| r != n - 8) {
                        self.set_a(n - 8, value);
                    }
                }
                address = self.add(address, self.k(size.bytes()));
            }
            if direction == Direction::MemoryToRegister {
                self.read_mem(address, S::Word, ReadKind::Discarded);
                if let Some((r, _)) = post {
                    self.set_a(r, address);
                }
            }
        }
        Ok(self.k(
            crate::timing::cycles(self.instruction, &CpuState::default(), mask, 0)
                .expect("MOVEM timing"),
        ))
    }
}
