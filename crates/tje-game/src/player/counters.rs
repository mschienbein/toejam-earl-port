//! MCC68K SUBQ followed immediately by a signed branch consumes N xor V,
//! not the sign of the truncated stored byte/word (G's matching compiler output).

#[inline]
pub(super) fn predecrement_byte(previous: i8) -> (i8, i16) {
    let result = i16::from(previous) - 1;
    (result as i8, result)
}

#[inline]
pub(super) fn predecrement_word(previous: i16) -> (i16, i32) {
    let result = i32::from(previous) - 1;
    (result as i16, result)
}

#[cfg(test)]
mod tests {
    use super::{predecrement_byte, predecrement_word};

    #[test]
    fn every_byte_preserves_subq_store_and_signed_branch_flags() {
        for bits in 0..=u8::MAX {
            let old = bits as i8;
            let (stored, wide) = predecrement_byte(old);
            assert_eq!(stored, old.wrapping_sub(1));
            let n = stored < 0;
            let z = stored == 0;
            let v = old == i8::MIN;
            assert_eq!(wide < 0, n != v, "byte {old}");
            assert_eq!(wide <= 0, z || n != v, "byte {old}");
        }
    }

    #[test]
    fn every_word_preserves_subq_store_and_signed_branch_flags() {
        for bits in 0..=u16::MAX {
            let old = bits as i16;
            let (stored, wide) = predecrement_word(old);
            assert_eq!(stored, old.wrapping_sub(1));
            let n = stored < 0;
            let z = stored == 0;
            let v = old == i16::MIN;
            assert_eq!(wide < 0, n != v, "word {old}");
            assert_eq!(wide <= 0, z || n != v, "word {old}");
        }
    }
}
