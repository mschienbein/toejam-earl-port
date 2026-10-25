use super::access_tests::Machine;
use crate::Context;
use tje_mem::{Revision, ram};

#[test]
fn layout_countdown_uses_z_not_signed_overflow_for_every_word() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Machine::new(revision);
        for old in i16::MIN..=i16::MAX {
            ram::g_jam_layout_countdown(revision).write(&mut context, old);
            // Expiry advances 2 -> 3, a scroll-only state with no scene calls.
            ram::g_jam_layout_phase(revision).write(&mut context, 2);
            ram::g_jam_scroll_step(revision).write(&mut context, 0);
            context.clear_accesses();
            context.run_framed(super::scene_layouts::update_layouts_and_scroll, &[0]);
            assert_eq!(
                ram::g_jam_layout_countdown(revision).read(&context),
                if old == 1 { 68 } else { old.wrapping_sub(1) },
                "old countdown {old}"
            );
            assert_eq!(
                ram::g_jam_layout_phase(revision).read(&context),
                if old == 1 { 3 } else { 2 }
            );
            assert_eq!(
                ram::g_jam_scroll_step(revision).read(&context),
                i16::from(old == 1)
            );
            let result = old.wrapping_sub(1);
            let expected_ccr = if old == 1 {
                0 // The expiry path ends with MOVE.W #68,countdown.
            } else {
                (u16::from(old == 0) * 0x11)
                    | (u16::from(result < 0) << 3)
                    | (u16::from(result == 0) << 2)
                    | (u16::from(old == i16::MIN) << 1)
            };
            assert_eq!(context.status_register(), 0x2700 | expected_ccr);
        }
    }
}

#[test]
fn layout_phase_wrap_and_signed_scroll_keep_word_storage() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Machine::new(revision);
        ram::g_jam_layout_countdown(revision).write(&mut context, 1);
        ram::g_jam_layout_phase(revision).write(&mut context, i16::MAX);
        ram::g_jam_scroll_step(revision).write(&mut context, -1);
        ram::g_jam_horizontal_scroll(revision).write(&mut context, i16::MAX);
        ram::g_jam_vertical_scroll(revision).write(&mut context, i16::MIN);
        ram::g_jam_toejam_animation_locked(revision).write(&mut context, 1);
        ram::g_jam_earl_animation_locked(revision).write(&mut context, 1);
        context.run_framed(super::scene_layouts::update_layouts_and_scroll, &[0]);
        assert_eq!(ram::g_jam_layout_phase(revision).read(&context), i16::MIN);
        assert_eq!(ram::g_jam_layout_countdown(revision).read(&context), 170);
        assert_eq!(
            ram::g_jam_horizontal_scroll(revision).read(&context),
            i16::MIN
        );
        assert_eq!(
            ram::g_jam_vertical_scroll(revision).read(&context),
            i16::MAX
        );
        assert_eq!(
            ram::g_jam_toejam_animation_locked(revision).read(&context),
            0
        );
        assert_eq!(ram::g_jam_earl_animation_locked(revision).read(&context), 0);
    }
}
