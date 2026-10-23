//! MCC68K 3.3K integer/runtime behavior, from G/src/asm/runtime.s and tools/semantics.c.

/// A real 68000 integer divide-by-zero exception (vector 5), never a zero result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithmeticTrap {
    DivideByZero,
}

impl ArithmeticTrap {
    pub const fn vector(self) -> u8 {
        match self {
            Self::DivideByZero => 5,
        }
    }
}

/// IDIV's magnitude/sign algorithm truncates toward zero; MIN/-1 wraps to MIN.
pub fn div_i32(dividend: i32, divisor: i32) -> Result<i32, ArithmeticTrap> {
    if divisor == 0 {
        return Err(ArithmeticTrap::DivideByZero);
    }
    Ok(dividend.wrapping_div(divisor))
}

/// The sign of the remainder is the dividend's; MIN%-1 is zero.
pub fn rem_i32(dividend: i32, divisor: i32) -> Result<i32, ArithmeticTrap> {
    if divisor == 0 {
        return Err(ArithmeticTrap::DivideByZero);
    }
    Ok(dividend.wrapping_rem(divisor))
}

pub fn div_u32(dividend: u32, divisor: u32) -> Result<u32, ArithmeticTrap> {
    if divisor == 0 {
        return Err(ArithmeticTrap::DivideByZero);
    }
    Ok(dividend / divisor)
}

pub fn rem_u32(dividend: u32, divisor: u32) -> Result<u32, ArithmeticTrap> {
    if divisor == 0 {
        return Err(ArithmeticTrap::DivideByZero);
    }
    Ok(dividend % divisor)
}

/// Stock ABS's NEG.L wraps for the most-negative input, preserving the original bug.
pub const fn abs_i32(value: i32) -> i32 {
    value.wrapping_abs()
}

/// 68000 register-count shifts use the low six count bits, not Rust's low five.
/// These implement the value only; translated logic needing CCR must retain it separately.
pub const fn lsl_u32(value: u32, count: u32) -> u32 {
    let count = count & 63;
    if count >= 32 { 0 } else { value << count }
}
pub const fn lsr_u32(value: u32, count: u32) -> u32 {
    let count = count & 63;
    if count >= 32 { 0 } else { value >> count }
}
pub const fn asr_i32(value: i32, count: u32) -> i32 {
    let count = count & 63;
    if count >= 32 {
        value >> 31
    } else {
        value >> count
    }
}

/// MCC68K uses unsigned-preserving promotions, not modern value-preserving C rules.
pub const fn promote_char(value: i8) -> i32 {
    value as i32
}
pub const fn promote_short(value: i16) -> i32 {
    value as i32
}
pub const fn promote_unsigned_char(value: u8) -> u32 {
    value as u32
}
pub const fn promote_unsigned_short(value: u16) -> u32 {
    value as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_division_and_remainder_follow_runtime_helpers() {
        for (n, d, q, r) in [
            (-7, 3, -2, -1),
            (7, -3, -2, 1),
            (-7, -3, 2, -1),
            (7, 3, 2, 1),
            (i32::MIN, -1, i32::MIN, 0),
            (i32::MIN, 1, i32::MIN, 0),
            (i32::MIN, i32::MIN, 1, 0),
            (i32::MAX, 65537, 32767, 32768),
        ] {
            assert_eq!(div_i32(n, d), Ok(q));
            assert_eq!(rem_i32(n, d), Ok(r));
        }
        assert_eq!(abs_i32(i32::MIN), i32::MIN);
        assert_eq!(abs_i32(-7), 7);
    }

    #[test]
    fn unsigned_helpers_cover_both_sides_of_native_divu_word_path() {
        for d in [1, 2, 3, 65535, 65536, 65537, 0x80000000, u32::MAX] {
            for n in [0, 1, 65535, 65536, 0x80000000, u32::MAX] {
                assert_eq!(div_u32(n, d), Ok(n / d));
                assert_eq!(rem_u32(n, d), Ok(n % d));
            }
        }
    }

    #[test]
    fn division_by_zero_is_vector_five_for_every_helper() {
        assert_eq!(div_i32(1, 0), Err(ArithmeticTrap::DivideByZero));
        assert_eq!(rem_i32(-1, 0), Err(ArithmeticTrap::DivideByZero));
        assert_eq!(div_u32(1, 0), Err(ArithmeticTrap::DivideByZero));
        assert_eq!(rem_u32(1, 0), Err(ArithmeticTrap::DivideByZero));
        assert_eq!(ArithmeticTrap::DivideByZero.vector(), 5);
    }

    #[test]
    fn shifts_match_68000_register_count_edges() {
        for count in [0, 1, 7, 16, 31, 32, 33, 63, 64, 65, 255, u32::MAX] {
            let effective = count & 63;
            let expected_left = if effective >= 32 {
                0
            } else {
                0x81234567_u32 << effective
            };
            let expected_right = if effective >= 32 {
                0
            } else {
                0x81234567_u32 >> effective
            };
            let expected_signed = if effective >= 32 {
                -1
            } else {
                (0x81234567_u32 as i32) >> effective
            };
            assert_eq!(lsl_u32(0x81234567, count), expected_left);
            assert_eq!(lsr_u32(0x81234567, count), expected_right);
            assert_eq!(asr_i32(0x81234567_u32 as i32, count), expected_signed);
        }
        assert_eq!(asr_i32(7, 32), 0);
        assert_eq!(asr_i32(-7, 2), -2);
        assert_eq!(lsl_u32(1, 32), 0);
        assert_eq!(lsl_u32(1, 64), 1);
    }

    #[test]
    fn unsigned_preserving_promotions_match_dynamic_compiler_probes() {
        assert_eq!(promote_char(-1), -1);
        assert_eq!(promote_short(i16::MIN), -32768);
        assert_eq!(promote_unsigned_char(255), 255_u32);
        assert_eq!(promote_unsigned_short(65535), 65535_u32);
        assert!(promote_unsigned_short(65535) < (-1_i32 as u32));
        assert!(promote_unsigned_char(255) < (-1_i32 as u32));
        assert!((-1_i32 as u32) > 1);
        assert_eq!(promote_unsigned_short(65535).wrapping_add(1), 65536);
    }
}
