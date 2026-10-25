use super::{advance_player_parity, handle_vblank_inner};
use crate::core::test_support::{Access, TestContext};
use crate::ids;
use tje_mem::{Ptr, Revision, ram};

#[test]
fn callback_test_and_pointer_load_are_distinct_long_reads() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        let callback = ram::g_v_blank_func(revision);
        callback.write(
            &mut context,
            Ptr::new(ids::JAM_OUT_V_BLANK.metadata().address(revision)),
        );
        context.trace_accesses = true;
        // The synthetic context cannot enter a TimeAware foreign callback.
        // Assert the complete owned prefix before that real capability error.
        assert!(handle_vblank_inner(&mut context, &[]).is_err());
        assert_eq!(
            *context.accesses.borrow(),
            vec![
                Access::Read(callback.address(), 2),
                Access::Read(callback.address() + 2, 2),
                Access::Read(callback.address(), 2),
                Access::Read(callback.address() + 2, 2),
            ],
        );
    }
}

#[test]
fn parity_add_byte_has_its_compiled_readback_for_every_byte() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        let parity = ram::g_v_blank_player_update_parity(revision);
        for previous in 0..=u8::MAX {
            context.trace_accesses = false;
            parity.write(&mut context, previous as i8);
            context.accesses.borrow_mut().clear();
            context.trace_accesses = true;
            let next = previous.wrapping_add(1);
            assert_eq!(advance_player_parity(&mut context), next as i8);
            assert_eq!(
                *context.accesses.borrow(),
                vec![
                    Access::Read(parity.address(), 1),
                    Access::Write(parity.address(), 1, u32::from(next)),
                    Access::Read(parity.address(), 1),
                ],
            );
        }
    }
}
