use crate::core::test_support::TestContext;
use tje_mem::{Revision, ram};

#[test]
fn layout_countdown_uses_z_not_signed_overflow_for_every_word() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        for old in i16::MIN..=i16::MAX {
            ram::g_jam_layout_countdown(revision).write(&mut context, old);
            // Expiry advances 2 -> 3, a scroll-only state with no scene calls.
            ram::g_jam_layout_phase(revision).write(&mut context, 2);
            ram::g_jam_scroll_step(revision).write(&mut context, 0);
            context.writes.clear();
            super::scene_layouts::update_layouts_and_scroll(&mut context, &[0]).unwrap();
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
        }
    }
}

#[test]
fn layout_phase_wrap_and_signed_scroll_keep_word_storage() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        ram::g_jam_layout_countdown(revision).write(&mut context, 1);
        ram::g_jam_layout_phase(revision).write(&mut context, i16::MAX);
        ram::g_jam_scroll_step(revision).write(&mut context, -1);
        ram::g_jam_horizontal_scroll(revision).write(&mut context, i16::MAX);
        ram::g_jam_vertical_scroll(revision).write(&mut context, i16::MIN);
        ram::g_jam_toejam_animation_locked(revision).write(&mut context, 1);
        ram::g_jam_earl_animation_locked(revision).write(&mut context, 1);
        super::scene_layouts::update_layouts_and_scroll(&mut context, &[0]).unwrap();
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
