use super::{predecrement_i8, predecrement_i16};
use tje_mem::{Address, AddressSpace, Bus, Revision};

// Compare the actual shared store/branch implementation to the independent
// 68000 SUBQ flag equations. A post-store sign test loses V at the minimum.
#[test]
fn every_byte_predecrement_matches_subq_signed_conditions() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut bus = AddressSpace::new(Vec::new(), revision);
        let counter = Address::<i8>::new(0xff1001);
        for bits in 0..=u8::MAX {
            bus.write_u8(0xff1000, 0x5a);
            bus.write_u8(0xff1002, 0xa5);
            let old = bits as i8;
            counter.write(&mut bus, old);
            let branch_value = predecrement_i8(&mut bus, counter);
            let result = bits.wrapping_sub(1);
            let negative = result & 0x80 != 0;
            let overflow = bits == 0x80;
            let zero = result == 0;
            assert_eq!(counter.read(&bus), result as i8, "{old}");
            assert_eq!(branch_value < 0, negative ^ overflow, "{old}: BLT");
            assert_eq!(
                branch_value <= 0,
                zero || (negative ^ overflow),
                "{old}: BLE"
            );
            assert_eq!(
                branch_value > 0,
                !zero && !(negative ^ overflow),
                "{old}: BGT"
            );
            assert_eq!(bus.read_u8(0xff1000), 0x5a);
            assert_eq!(bus.read_u8(0xff1002), 0xa5);
        }
    }
}

#[test]
fn every_word_predecrement_matches_subq_signed_conditions() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut bus = AddressSpace::new(Vec::new(), revision);
        let counter = Address::<i16>::new(0xff1002);
        for bits in 0..=u16::MAX {
            bus.write_u16(0xff1000, 0x5aa5);
            bus.write_u16(0xff1004, 0xa55a);
            let old = bits as i16;
            counter.write(&mut bus, old);
            let branch_value = predecrement_i16(&mut bus, counter);
            let result = bits.wrapping_sub(1);
            let negative = result & 0x8000 != 0;
            let overflow = bits == 0x8000;
            let zero = result == 0;
            assert_eq!(counter.read(&bus), result as i16, "{old}");
            assert_eq!(branch_value < 0, negative ^ overflow, "{old}: BLT");
            assert_eq!(
                branch_value <= 0,
                zero || (negative ^ overflow),
                "{old}: BLE"
            );
            assert_eq!(
                branch_value > 0,
                !zero && !(negative ^ overflow),
                "{old}: BGT"
            );
            assert_eq!(bus.read_u16(0xff1000), 0x5aa5);
            assert_eq!(bus.read_u16(0xff1004), 0xa55a);
        }
    }
}

#[test]
fn every_original_is_classified_and_pending_bodies_are_not_dispatched() {
    let mut count = 0;
    for function in crate::functions().filter(|function| function.subsystem == "presents") {
        let atomic = super::ATOMIC_CAPABLE.contains(&function.id);
        let other = super::EXECUTION_CONTRACTS
            .iter()
            .filter(|(id, _)| *id == function.id)
            .count();
        assert_eq!(usize::from(atomic) + other, 1, "{}", function.name);
        count += 1;
    }
    assert_eq!(count, 65);
    for candidate in super::PENDING {
        assert!(
            crate::Dispatcher::hybrid()
                .translation(candidate.id)
                .is_none(),
            "pending {} escaped into production",
            candidate.id.metadata().name
        );
    }
}
