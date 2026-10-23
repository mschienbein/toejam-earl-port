use super::*;
use crate::decode::{ShiftCount, ShiftDirection};

impl Builder<'_> {
    pub(super) fn shift_register(
        &self,
        size: S,
        direction: ShiftDirection,
        n: u8,
        count: ShiftCount,
    ) -> Value {
        let (count, known) = match count {
            ShiftCount::Constant(n) => (self.k(n.into()), Some(n)),
            ShiftCount::Register(r) => (self.and(self.load(F::Data(r.0)), self.k(63)), None),
        };
        let old = self.trunc(self.load(F::Data(n)), size);
        let out = self.shift(old, size, direction, count, known);
        self.write(Operand::D(n), size, out);
        self.add(
            self.k(if size == S::LongWord { 8 } else { 6 }),
            self.shl(count, 1),
        )
    }
    pub(super) fn shift(
        &self,
        old: Value,
        size: S,
        direction: ShiftDirection,
        count: Value,
        known: Option<u8>,
    ) -> Value {
        let arithmetic = matches!(
            self.instruction,
            I::ArithmeticShiftRegister(..) | I::ArithmeticShiftMemory(..)
        );
        let rotate = matches!(
            self.instruction,
            I::RotateRegister(..) | I::RotateMemory(..)
        );
        let through = matches!(
            self.instruction,
            I::RotateThruExtendRegister(..) | I::RotateThruExtendMemory(..)
        );
        let left = direction == ShiftDirection::Left;
        let bits = size.bytes() * 8;
        let sr = self.load(F::Sr);
        let old_x = self.bit(sr, 4);
        let (out, carry, x, overflow) = if known.is_some() || through {
            let iterations = known.unwrap_or(63);
            let mut value = old;
            let mut x = old_x;
            let mut carry = if through { old_x } else { self.k(0) };
            let mut overflow = self.k(0);
            for i in 0..iterations {
                let active = if known.is_some() {
                    self.k(1)
                } else {
                    self.cmp(C::UnsignedLess, self.k(u32::from(i)), count)
                };
                let shifted_out = self.bit(value, if left { bits - 1 } else { 0 });
                let shifted = if left {
                    self.trunc(self.shl(value, 1), size)
                } else if arithmetic {
                    self.trunc(
                        self.bin(B::ArithmeticShiftRight, self.signed(value, size), self.k(1)),
                        size,
                    )
                } else {
                    self.shr(value, 1)
                };
                let shifted = if rotate || through {
                    let incoming = if through { x } else { shifted_out };
                    self.or(
                        shifted,
                        if left {
                            incoming
                        } else {
                            self.shl(incoming, bits - 1)
                        },
                    )
                } else {
                    shifted
                };
                if arithmetic {
                    let changed = self.xor(self.bit(value, bits - 1), self.bit(shifted, bits - 1));
                    overflow = self.or(overflow, self.and(active, changed));
                }
                value = self.select(active, shifted, value);
                carry = self.select(active, shifted_out, carry);
                if !rotate {
                    x = self.select(active, shifted_out, x);
                }
            }
            (value, carry, x, overflow)
        } else if rotate {
            let reduced = self.and(count, self.k(bits - 1));
            let inverse = self.sub(self.k(bits), reduced);
            let out = if left {
                self.or(
                    self.bin(B::ShiftLeft, old, reduced),
                    self.bin(B::ShiftRight, old, inverse),
                )
            } else {
                self.or(
                    self.bin(B::ShiftRight, old, reduced),
                    self.bin(B::ShiftLeft, old, inverse),
                )
            };
            let out = self.trunc(out, size);
            let carry = self.select(
                self.zero(count),
                self.k(0),
                self.bit(out, if left { 0 } else { bits - 1 }),
            );
            (out, carry, old_x, self.k(0))
        } else {
            let large = self.cmp(C::UnsignedLessEqual, self.k(bits), count);
            let nonzero = self.truth(count);
            let shifted = self.bin(
                if left {
                    B::ShiftLeft
                } else if arithmetic {
                    B::ArithmeticShiftRight
                } else {
                    B::ShiftRight
                },
                if arithmetic && !left {
                    self.signed(old, size)
                } else {
                    old
                },
                count,
            );
            let fill = if arithmetic && !left {
                self.select(self.bit(old, bits - 1), self.k(size.mask()), self.k(0))
            } else {
                self.k(0)
            };
            let out = self.trunc(self.select(large, fill, shifted), size);
            let carry_index = if left {
                self.sub(self.k(bits), count)
            } else {
                self.sub(count, self.k(1))
            };
            let carry = self.and(self.bin(B::ShiftRight, old, carry_index), self.k(1));
            let too_large = self.cmp(C::UnsignedLess, self.k(bits), count);
            let carry = self.select(
                too_large,
                if arithmetic && !left {
                    self.bit(old, bits - 1)
                } else {
                    self.k(0)
                },
                carry,
            );
            let carry = self.select(nonzero, carry, self.k(0));
            let x = self.select(nonzero, carry, old_x);
            let overflow = if arithmetic && left {
                let recovered = self.bin(B::ArithmeticShiftRight, self.signed(out, size), count);
                self.select(
                    large,
                    self.truth(old),
                    self.cmp(C::NotEqual, recovered, self.signed(old, size)),
                )
            } else {
                self.k(0)
            };
            (out, carry, x, overflow)
        };
        let flags = self.or(
            self.or(carry, self.shl(overflow, 1)),
            self.or(
                self.shl(self.zero(out), 2),
                self.or(self.shl(self.bit(out, bits - 1), 3), self.shl(x, 4)),
            ),
        );
        self.store(F::Sr, self.or(self.and(sr, self.k(!31)), flags));
        out
    }
}
