use super::*;

impl Builder<'_> {
    pub(super) fn arithmetic(
        &self,
        l: Value,
        r: Value,
        s: S,
        subtract: bool,
        extend: bool,
        compare: bool,
    ) -> Value {
        let l = self.trunc(l, s);
        let r = self.trunc(r, s);
        let old_sr = self.load(F::Sr);
        let x = if extend {
            self.bit(old_sr, 4)
        } else {
            self.k(0)
        };
        let first = if subtract {
            self.sub(l, r)
        } else {
            self.add(l, r)
        };
        let out = self.trunc(
            if subtract {
                self.sub(first, x)
            } else {
                self.add(first, x)
            },
            s,
        );
        let carry = if subtract {
            self.or(
                self.cmp(C::UnsignedLess, l, r),
                self.cmp(C::UnsignedLess, self.trunc(first, s), x),
            )
        } else {
            let first = self.trunc(first, s);
            self.or(
                self.cmp(C::UnsignedLess, first, l),
                self.cmp(C::UnsignedLess, out, first),
            )
        };
        let low_mask = self.k(s.sign() - 1);
        let low_l = self.and(l, low_mask);
        let low_r = self.add(self.and(r, low_mask), x);
        let low_carry = if subtract {
            self.cmp(C::UnsignedLess, low_l, low_r)
        } else {
            self.cmp(C::UnsignedLess, low_mask, self.add(low_l, low_r))
        };
        let overflow = self.xor(low_carry, carry);
        let zero = self.and(
            self.zero(out),
            if extend {
                self.bit(old_sr, 2)
            } else {
                self.k(1)
            },
        );
        let flags = self.or(
            self.or(carry, self.shl(overflow, 1)),
            self.or(
                self.shl(zero, 2),
                self.shl(self.truth(self.and(out, self.k(s.sign()))), 3),
            ),
        );
        let x = if compare {
            self.and(old_sr, self.k(16))
        } else {
            self.shl(carry, 4)
        };
        self.store(
            F::Sr,
            self.or(self.and(old_sr, self.k(!31)), self.or(flags, x)),
        );
        out
    }
    pub(super) fn extend_read(&self, e: Ea, s: S) -> Result<(Operand, Value), CompileError> {
        if let (Ea::AddressIndirectPredecrement(r), S::LongWord) = (e, s) {
            let a = self.sub(self.a(r.0), self.k(2));
            self.set_a(r.0, a);
            let low = self.read_mem(a, S::Word, ReadKind::Semantic);
            let a = self.sub(a, self.k(2));
            self.set_a(r.0, a);
            let high = self.read_mem(a, S::Word, ReadKind::Semantic);
            Ok((Operand::Memory(a, None), self.or(self.shl(high, 16), low)))
        } else {
            let o = self.resolve(e, s, true)?;
            Ok((o, self.read(o, s, ReadKind::Semantic)))
        }
    }
    pub(super) fn decimal(&self, l: Value, r: Value, subtract: bool) -> Value {
        let sr = self.load(F::Sr);
        let x = self.bit(sr, 4);
        let uncorrected = if subtract {
            self.sub(self.sub(l, r), x)
        } else {
            self.add(self.add(l, r), x)
        };
        let raw = self.trunc(uncorrected, S::Byte);
        let carry = if subtract {
            self.or(
                self.cmp(C::UnsignedLess, l, r),
                self.cmp(C::UnsignedLess, self.trunc(self.sub(l, r), S::Byte), x),
            )
        } else {
            self.cmp(C::UnsignedLess, self.k(255), uncorrected)
        };
        let low = if subtract {
            self.cmp(
                C::UnsignedLess,
                self.and(l, self.k(15)),
                self.add(self.and(r, self.k(15)), x),
            )
        } else {
            self.or(
                self.cmp(
                    C::UnsignedLess,
                    self.k(15),
                    self.add(
                        self.add(self.and(l, self.k(15)), self.and(r, self.k(15))),
                        x,
                    ),
                ),
                self.cmp(C::UnsignedLess, self.k(9), self.and(raw, self.k(15))),
            )
        };
        let high = if subtract {
            carry
        } else {
            self.or(carry, self.cmp(C::UnsignedLess, self.k(0x99), raw))
        };
        let correction = self.add(
            self.select(low, self.k(6), self.k(0)),
            self.select(high, self.k(0x60), self.k(0)),
        );
        let corrected = if subtract {
            self.sub(raw, correction)
        } else {
            self.add(raw, correction)
        };
        let carry2 = if subtract {
            self.cmp(C::UnsignedLess, raw, correction)
        } else {
            self.cmp(C::UnsignedLess, self.k(255), corrected)
        };
        let low_carry = if subtract {
            self.cmp(
                C::UnsignedLess,
                self.and(raw, self.k(127)),
                self.and(correction, self.k(127)),
            )
        } else {
            self.cmp(
                C::UnsignedLess,
                self.k(127),
                self.add(
                    self.and(raw, self.k(127)),
                    self.and(correction, self.k(127)),
                ),
            )
        };
        let overflow = self.xor(low_carry, carry2);
        let carry = self.or(carry, carry2);
        let out = self.trunc(corrected, S::Byte);
        let flags = self.or(
            self.or(carry, self.shl(carry, 4)),
            self.or(
                self.shl(overflow, 1),
                self.or(
                    self.shl(self.and(self.bit(sr, 2), self.zero(out)), 2),
                    self.shl(self.bit(out, 7), 3),
                ),
            ),
        );
        self.store(F::Sr, self.or(self.and(sr, self.k(!31)), flags));
        out
    }
    pub(super) fn multiply(&self, n: u8, source: Ea, signed: bool) -> Result<Value, CompileError> {
        let operand = self.read_ea(source, S::Word)?;
        let old = self.trunc(self.load(F::Data(n)), S::Word);
        let value = if signed {
            self.bin(
                B::Multiply,
                self.signed(operand, S::Word),
                self.signed(old, S::Word),
            )
        } else {
            self.bin(B::Multiply, operand, old)
        };
        self.store(F::Data(n), value);
        self.nz(value, S::LongWord);
        let timing_bits = if signed {
            self.trunc(self.xor(operand, self.shl(operand, 1)), S::Word)
        } else {
            operand
        };
        let mut count = self.k(0);
        for n in 0..16 {
            count = self.add(count, self.bit(timing_bits, n));
        }
        Ok(self.add(
            self.k(38 + crate::timing::ea(source, S::Word)),
            self.shl(count, 1),
        ))
    }
    pub(super) fn divide(&self, n: u8, source: Ea, signed: bool) -> Result<Value, CompileError> {
        // The reference samples the dividend before resolving a source EA.
        let dividend = self.load(F::Data(n));
        let operand = self.read_ea(source, S::Word)?;
        let ea = crate::timing::ea(source, S::Word);
        // DIV-by-zero clears NZVC, retaining X, before stacking SR.
        let zero = self.zero(operand);
        let sr = self.load(F::Sr);
        self.store(F::Sr, self.select(zero, self.and(sr, self.k(!15)), sr));
        self.exception(zero, 5, self.k(38 + ea), self.load(F::Pc));
        let divisor = if signed {
            self.signed(operand, S::Word)
        } else {
            operand
        };
        let quotient = self.bin(
            if signed {
                B::DivideSigned
            } else {
                B::DivideUnsigned
            },
            dividend,
            divisor,
        );
        let remainder = self.bin(
            if signed {
                B::RemainderSigned
            } else {
                B::RemainderUnsigned
            },
            dividend,
            divisor,
        );
        let overflow = if signed {
            self.or(
                self.cmp(C::SignedLess, quotient, self.k(i16::MIN as u32)),
                self.cmp(C::SignedLess, self.k(i16::MAX as u32), quotient),
            )
        } else {
            self.cmp(C::UnsignedLess, self.k(65535), quotient)
        };
        let out = self.or(self.trunc(quotient, S::Word), self.shl(remainder, 16));
        self.store(F::Data(n), self.select(overflow, dividend, out));
        let flags = self.or(
            self.shl(self.zero(quotient), 2),
            self.shl(self.bit(quotient, 15), 3),
        );
        self.store(
            F::Sr,
            self.or(
                self.and(self.load(F::Sr), self.k(!15)),
                self.select(overflow, self.k(10), flags),
            ),
        );
        let cycles = if signed {
            let negative = self.bit(dividend, 31);
            let div_negative = self.bit(divisor, 31);
            let abs_dividend = self.select(negative, self.sub(self.k(0), dividend), dividend);
            let abs_divisor = self.select(div_negative, self.sub(self.k(0), divisor), divisor);
            // Arithmetic shift on wrapping_abs mirrors the reference, including MIN.
            let absolute_overflow = self.cmp(
                C::SignedLessEqual,
                abs_divisor,
                self.bin(B::ArithmeticShiftRight, abs_dividend, self.k(16)),
            );
            let abs_q = self.select(
                self.bit(quotient, 31),
                self.sub(self.k(0), quotient),
                quotient,
            );
            let mut cycles = self.add(self.k(120), self.shl(negative, 1));
            cycles = self.add(
                cycles,
                self.select(div_negative, self.k(2), self.shl(negative, 2)),
            );
            for bit in 1..=15 {
                cycles = self.add(cycles, self.shl(self.zero(self.bit(abs_q, bit)), 1));
            }
            self.select(
                self.and(overflow, absolute_overflow),
                self.add(self.k(16), self.shl(negative, 1)),
                cycles,
            )
        } else {
            let shifted = self.shl(operand, 16);
            let mut value = dividend;
            let mut cycles = self.k(76);
            for _ in 0..15 {
                let high = self.bit(value, 31);
                value = self.shl(value, 1);
                let borrow = self.cmp(C::UnsignedLess, value, shifted);
                value = self.select(
                    self.or(high, self.zero(borrow)),
                    self.sub(value, shifted),
                    value,
                );
                cycles = self.add(
                    cycles,
                    self.select(high, self.k(0), self.add(self.k(2), self.shl(borrow, 1))),
                );
            }
            self.select(overflow, self.k(10), cycles)
        };
        Ok(self.add(cycles, self.k(ea)))
    }
}
