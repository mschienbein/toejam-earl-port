use crate::ir::{BinaryOp, CompareOp, StateField, Value};

pub(super) const ENTRY_BASE: u32 = 2;
pub(super) const VALUE_BASE: u32 = ENTRY_BASE + StateField::COUNT as u32 * 32;
#[derive(Clone, Copy, Debug)]
pub(super) struct Fact {
    /// Proven independently of an observed entry/read value. Observed choices
    /// must never masquerade as constants and erase their controlling inputs.
    pub zeros: u32,
    pub ones: u32,
    pub observed: Option<u32>,
    pub origins: [u32; 32],
}
impl Default for Fact {
    fn default() -> Self {
        Self::constant(0)
    }
}
impl Fact {
    pub fn constant(value: u32) -> Self {
        Self {
            zeros: !value,
            ones: value,
            observed: Some(value),
            origins: std::array::from_fn(|bit| (value >> bit) & 1),
        }
    }
    pub fn entry(field: StateField, observed: Option<u32>) -> Self {
        Self {
            zeros: 0,
            ones: 0,
            observed,
            origins: std::array::from_fn(|bit| ENTRY_BASE + field.index() * 32 + bit as u32),
        }
    }
    pub fn value(value: Value) -> Self {
        Self {
            zeros: 0,
            ones: 0,
            observed: None,
            origins: std::array::from_fn(|bit| VALUE_BASE + value.0 * 32 + bit as u32),
        }
    }
    pub fn constant_value(&self) -> Option<u32> {
        (self.zeros | self.ones == u32::MAX).then_some(self.ones)
    }
    pub fn same_bits(&self, other: &Self) -> u32 {
        self.origins
            .iter()
            .zip(other.origins)
            .enumerate()
            .fold(0, |mask, (bit, (&a, b))| mask | (u32::from(a == b) << bit))
    }
    pub fn initial_bits(&self, field: StateField) -> u32 {
        self.origins
            .iter()
            .enumerate()
            .fold(0, |mask, (bit, &origin)| {
                mask | (u32::from(origin == ENTRY_BASE + field.index() * 32 + bit as u32) << bit)
            })
    }
    pub fn memory(value: Value, mask: u32, observed: Option<u32>) -> Self {
        let mut result = Self::value(value);
        result.zeros = !mask;
        result.observed = observed;
        for bit in 0..32 {
            if mask & (1 << bit) == 0 {
                result.origins[bit] = 0;
            }
        }
        result
    }
    pub fn compare(dst: Value, op: CompareOp, lhs: &Self, rhs: &Self) -> Self {
        if let (Some(lhs), Some(rhs)) = (lhs.constant_value(), rhs.constant_value()) {
            return Self::constant(op.evaluate(lhs, rhs));
        }
        let mut result = Self::value(dst);
        result.zeros = !1;
        result.observed = lhs
            .observed
            .zip(rhs.observed)
            .map(|(a, b)| op.evaluate(a, b));
        result.origins[1..].fill(0);
        result
    }
    pub fn select(dst: Value, condition: &Self, yes: &Self, no: &Self) -> Self {
        if let Some(condition) = condition.constant_value() {
            return if condition != 0 { *yes } else { *no };
        }
        let mut result = Self::value(dst);
        // Constants remain path-independent even when an actual condition is
        // available. Only identity and value facts may refine the accepted path.
        result.zeros = yes.zeros & no.zeros;
        result.ones = yes.ones & no.ones;
        result.observed = condition.observed.and_then(|condition| {
            if condition != 0 {
                yes.observed
            } else {
                no.observed
            }
        });
        for bit in 0..32 {
            result.origins[bit] = if let Some(condition) = condition.observed {
                if condition != 0 {
                    yes.origins[bit]
                } else {
                    no.origins[bit]
                }
            } else if yes.origins[bit] == no.origins[bit] {
                yes.origins[bit]
            } else {
                result.origins[bit]
            };
        }
        result.observed = result.observed.or(result.constant_value());
        result
    }
    pub fn binary(dst: Value, op: BinaryOp, lhs: &Self, rhs: &Self) -> Self {
        if let (Some(a), Some(b)) = (lhs.constant_value(), rhs.constant_value())
            && let Some(value) = op.evaluate(a, b)
        {
            return Self::constant(value);
        }
        let mut result = Self::value(dst);
        result.observed = lhs
            .observed
            .zip(rhs.observed)
            .and_then(|(a, b)| op.evaluate(a, b));
        match op {
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                for bit in 0..32 {
                    let a = lhs.origins[bit];
                    let b = rhs.origins[bit];
                    result.origins[bit] = match op {
                        BinaryOp::And if a == 0 || b == 0 => 0,
                        BinaryOp::And if a == 1 || a == b => b,
                        BinaryOp::And if b == 1 => a,
                        BinaryOp::Or if a == 1 || b == 1 => 1,
                        BinaryOp::Or if a == 0 || a == b => b,
                        BinaryOp::Or if b == 0 => a,
                        BinaryOp::Xor if a == b => 0,
                        BinaryOp::Xor if a == 0 => b,
                        BinaryOp::Xor if b == 0 => a,
                        BinaryOp::Xor if a <= 1 && b <= 1 => a ^ b,
                        _ => result.origins[bit],
                    };
                    // Only unconditional constant origins may define bit facts.
                    // An observed Select can have a constant origin on one path.
                    let mask = 1 << bit;
                    let unconditional = match op {
                        BinaryOp::And => (lhs.zeros | rhs.zeros, lhs.ones & rhs.ones),
                        BinaryOp::Or => (lhs.zeros & rhs.zeros, lhs.ones | rhs.ones),
                        BinaryOp::Xor => (
                            (lhs.ones & rhs.ones) | (lhs.zeros & rhs.zeros),
                            (lhs.ones & rhs.zeros) | (lhs.zeros & rhs.ones),
                        ),
                        _ => unreachable!(),
                    };
                    result.zeros |= unconditional.0 & mask;
                    result.ones |= unconditional.1 & mask;
                }
            }
            BinaryOp::Add if lhs.constant_value() == Some(0) => result = *rhs,
            BinaryOp::Add | BinaryOp::Subtract if rhs.constant_value() == Some(0) => result = *lhs,
            BinaryOp::Multiply
                if lhs.constant_value() == Some(0) || rhs.constant_value() == Some(0) =>
            {
                result = Self::constant(0)
            }
            BinaryOp::Multiply if lhs.constant_value() == Some(1) => result = *rhs,
            BinaryOp::Multiply if rhs.constant_value() == Some(1) => result = *lhs,
            BinaryOp::DivideSigned | BinaryOp::DivideUnsigned
                if rhs.constant_value() == Some(1) =>
            {
                result = *lhs
            }
            BinaryOp::ShiftLeft | BinaryOp::ShiftRight | BinaryOp::ArithmeticShiftRight => {
                if let Some(count) = rhs.observed.or(rhs.constant_value()) {
                    let count = count & 31;
                    for bit in 0..32 {
                        result.origins[bit] = match op {
                            BinaryOp::ShiftLeft if bit >= count as usize => {
                                lhs.origins[bit - count as usize]
                            }
                            BinaryOp::ShiftLeft => 0,
                            BinaryOp::ShiftRight if bit + (count as usize) < 32 => {
                                lhs.origins[bit + count as usize]
                            }
                            BinaryOp::ShiftRight => 0,
                            BinaryOp::ArithmeticShiftRight => {
                                lhs.origins[(bit + count as usize).min(31)]
                            }
                            _ => unreachable!(),
                        };
                    }
                }
                if let Some(count) = rhs.constant_value() {
                    let count = count & 31;
                    match op {
                        BinaryOp::ShiftLeft => {
                            result.ones = lhs.ones.wrapping_shl(count);
                            result.zeros =
                                lhs.zeros.wrapping_shl(count) | !(u32::MAX.wrapping_shl(count));
                        }
                        BinaryOp::ShiftRight => {
                            result.ones = lhs.ones.wrapping_shr(count);
                            result.zeros =
                                lhs.zeros.wrapping_shr(count) | !(u32::MAX.wrapping_shr(count));
                        }
                        BinaryOp::ArithmeticShiftRight => {
                            result.ones = (lhs.ones as i32).wrapping_shr(count) as u32;
                            result.zeros = (lhs.zeros as i32).wrapping_shr(count) as u32;
                        }
                        _ => unreachable!(),
                    }
                }
            }
            _ => {}
        }
        result.observed = result.observed.or(result.constant_value());
        result
    }
}
