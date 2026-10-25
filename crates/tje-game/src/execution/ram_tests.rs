use super::*;

#[test]
fn ram_wait_constructors_preserve_width_mask_relation_and_operational_cap() {
    let requests = [
        (
            WaitRequest::ram_byte_equal(0xff9001, 0x80, 0),
            RamWidth::Byte,
            0x80,
            WaitComparison::Equal,
        ),
        (
            WaitRequest::ram_byte_not_equal(0xff9001, 0xff, 0),
            RamWidth::Byte,
            0xff,
            WaitComparison::NotEqual,
        ),
        (
            WaitRequest::ram_word_equal(0xff9002, 0x8000, 0),
            RamWidth::Word,
            0x8000,
            WaitComparison::Equal,
        ),
        (
            WaitRequest::ram_word_not_equal(0xff9002, 0xffff, 0),
            RamWidth::Word,
            0xffff,
            WaitComparison::NotEqual,
        ),
        (
            WaitRequest::ram_long_equal(0xff9004, 0x80000000, 0),
            RamWidth::Long,
            0x80000000,
            WaitComparison::Equal,
        ),
        (
            WaitRequest::ram_long_not_equal(0xff9004, u32::MAX, 0),
            RamWidth::Long,
            u32::MAX,
            WaitComparison::NotEqual,
        ),
    ];
    for (request, expected_width, expected_mask, expected_comparison) in requests {
        assert_eq!(request.max_ticks, WaitRequest::DEFAULT_MAX_TICKS);
        let WaitCondition::Ram {
            address,
            width,
            mask,
            value,
            comparison,
        } = request.condition
        else {
            panic!("not a RAM predicate")
        };
        assert_eq!(
            (width, mask, value, comparison),
            (expected_width, expected_mask, 0, expected_comparison)
        );
        assert!(width.valid(address, mask, value));
    }
}

#[test]
fn ram_wait_domains_reject_wrong_width_alignment_space_and_bits() {
    assert!(RamWidth::Byte.valid(0xff9001, 255, 0));
    assert!(RamWidth::Word.valid(0xe09002, 65535, 0));
    assert!(RamWidth::Long.valid(0xfffffc, u32::MAX, 0));
    assert!(!RamWidth::Byte.valid(0xff9001, 0x100, 0));
    assert!(!RamWidth::Byte.valid(0xff9001, 255, 256));
    assert!(!RamWidth::Word.valid(0xff9001, 65535, 0));
    assert!(!RamWidth::Word.valid(0xa11100, 65535, 0));
    assert!(!RamWidth::Long.valid(0xfffffe, u32::MAX, 0));
}

#[test]
fn equality_and_inequality_are_explicit_opposites_for_full_byte_samples() {
    for actual in 0..=255 {
        for value in 0..=255 {
            for mask in [0, 1, 0x80, 0xff] {
                let equal = WaitComparison::Equal.satisfied(actual, mask, value);
                let different = WaitComparison::NotEqual.satisfied(actual, mask, value);
                assert_eq!(equal, actual & mask == value);
                assert_eq!(different, !equal);
            }
        }
    }
}
