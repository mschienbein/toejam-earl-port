use super::remaining::*;
use super::test_support::{Access, TestContext, Write};
use super::wait_for_next_vblank_readable_framed;
use crate::Context;
use tje_mem::{Revision, ram, rom};

#[test]
fn starfield_sign_extends_bytes_and_repeats_every_tile_write() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut bytes = vec![0; 0x100000];
        bytes[rom::starfield_byte_map(revision).address() as usize] = 0x80;
        let mut context = TestContext::with_rom(revision, bytes);
        draw_starfield_plane(&mut context, &[]).unwrap();
        assert_eq!(context.writes.len(), 4098);
        assert_eq!(
            &context.writes[..5],
            &[
                Write::Control(0x8f40),
                Write::Control(0x6000),
                Write::Control(3),
                Write::Data(0xa00),
                Write::Data(0xa00),
            ]
        );
        assert_eq!(context.writes.last(), Some(&Write::Control(0x8f02)));
    }
}

#[test]
fn input_masks_keep_previous_values_and_group_precedence() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (group, state, alive, expected) in [
            (0, 0, true, 0xff),
            (0x20000, 0, true, 0x80),
            (0, 79, true, 0x80),
            (0, 78, true, 0x80),
            (0, 5, true, 0x80),
            (0, 6, true, 0x80),
            (0x20000000, 0, true, 0x8f),
            (0x20, 0, true, 0xcf),
            (0x40, 0, true, 0xcf),
            (0x07800000, 0, true, 0xef),
            (0, 2, true, 0xef),
            (0x20000060, 2, true, 0x8f),
            (0, 0, false, 0),
            (0x20000000, 0, false, 0),
        ] {
            let mut context = TestContext::new(revision);
            ram::g_controller1_raw_input(revision).write(&mut context, 0xff);
            ram::g_controller2_raw_input(revision).write(&mut context, 0xff);
            for player in 0..2 {
                let entity = ram::g_player_info(revision).at(player);
                entity.animation_group_mask().write(&mut context, group);
                entity.sprite_state().write(&mut context, state);
                ram::g_player_lives(revision)
                    .at(player)
                    .write(&mut context, if alive { 0 } else { -1 });
                ram::g_player_processed_input(revision)
                    .at(player)
                    .write(&mut context, 0x55);
            }
            context.set_data_register(0, 0xdeadbeef);
            context.set_data_register(1, 0xdeadbeef);
            context.set_data_register(2, 0x12345678);
            context.set_address_register(0, 0xff9000);
            context.set_address_register(1, 0xff9010);
            context.set_status_register(0x271f);
            get_player_input(&mut context, &[]).unwrap();
            let masks_input = group & (0x20000000 | 0x60 | 0x07800000) != 0 || state == 2;
            assert_eq!(context.data_register(0), u32::from(masks_input));
            let d1 = if group & 0x20000000 != 0 {
                if alive { 0xff } else { 1 }
            } else {
                0x60
            };
            assert_eq!(context.data_register(1), d1);
            assert_eq!(context.data_register(2), 0x12345678);
            assert_eq!(
                context.address_register(0),
                ram::g_player_lives(revision).address()
            );
            assert_eq!(context.address_register(1), 0xff9010);
            assert_eq!(context.status_register(), 0x2700);
            for player in 0..2 {
                assert_eq!(
                    ram::g_player_processed_input(revision)
                        .at(player)
                        .read(&context),
                    expected
                );
                assert_eq!(
                    ram::g_player_previous_processed_input(revision)
                        .at(player)
                        .read(&context),
                    0x55
                );
            }
        }
    }
}

#[test]
fn vblank_wait_preserves_disabled_and_nonpositive_returns() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (mode, count) in [
            (0, 0x12340001),
            (0x40, 0x12340000),
            (0x40, 0x1234ffff),
            (0x40, 0x12348000),
        ] {
            let mut context = TestContext::new(revision);
            let frame = context.prepare_entry(crate::ids::WAIT_FOR_NEXT_V_BLANK, &[count]);
            let shadow = ram::g_mode2_reg_softcopy(revision);
            shadow.write(&mut context, mode);
            context.trace_accesses = true;
            assert_eq!(
                wait_for_next_vblank_readable_framed(&mut context).unwrap(),
                None
            );
            assert_eq!(
                *context.accesses.borrow(),
                vec![
                    Access::Read(frame.argument_word(0).address(), 2),
                    Access::Read(shadow.address(), 1),
                ],
            );
        }
        let mut context = TestContext::new(revision);
        context.prepare_entry(crate::ids::WAIT_FOR_NEXT_V_BLANK, &[1]);
        ram::g_mode2_reg_softcopy(revision).write(&mut context, 0x40);
        // An Atomic unit context has no clock authority: don't fake the poll.
        assert!(wait_for_next_vblank_readable_framed(&mut context).is_err());
    }
}

#[test]
fn readable_vblank_uses_one_word_predicate_after_each_initial_counter_load() {
    use crate::execution::WaitRequest;
    for revision in [Revision::Rev02, Revision::Rev00] {
        for previous in [0, 0x7fff, 0xffff] {
            let mut context = TestContext::new(revision);
            let frame = context.prepare_entry(crate::ids::WAIT_FOR_NEXT_V_BLANK, &[1]);
            let mode = ram::g_mode2_reg_softcopy(revision);
            let counter = ram::g_frame_counter(revision);
            mode.write(&mut context, 0x40);
            counter.write(&mut context, previous);
            context.trace_accesses = true;
            // A real unsatisfied first CMP poll is observed. No synthetic clock
            // or IRQ is granted merely to finish this unit fixture.
            let error = wait_for_next_vblank_readable_framed(&mut context).unwrap_err();
            assert!(matches!(error, crate::Error::Platform(_)));
            assert_eq!(
                *context.accesses.borrow(),
                vec![
                    Access::Read(frame.argument_word(0).address(), 2),
                    Access::Read(mode.address(), 1),
                    Access::Read(counter.address(), 2),
                    Access::Wait(WaitRequest::ram_word_not_equal(
                        counter.address(),
                        0xffff,
                        previous,
                    )),
                    Access::Read(counter.address(), 2),
                ],
            );
        }
    }
}
