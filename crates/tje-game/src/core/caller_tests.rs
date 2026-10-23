use super::test_support::TestContext;
use tje_mem::{Revision, ram};

#[test]
fn held_start_returns_one_and_toejam_has_simultaneous_priority() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        assert_eq!(
            super::input::check_start_button(&mut context, &[]).unwrap(),
            Some(0)
        );
        ram::g_player_processed_input(revision)
            .at(0)
            .write(&mut context, 0x80);
        ram::g_player_processed_input(revision)
            .at(1)
            .write(&mut context, 0x80);
        ram::g_player_previous_processed_input(revision)
            .at(0)
            .write(&mut context, 0x80);
        ram::g_player_previous_processed_input(revision)
            .at(1)
            .write(&mut context, 0);
        // Earl's new press must not cause GamePaused while ToeJam is holding it.
        assert_eq!(
            super::input::check_start_button(&mut context, &[]).unwrap(),
            Some(1)
        );
        ram::g_player_processed_input(revision)
            .at(0)
            .write(&mut context, 0);
        ram::g_player_previous_processed_input(revision)
            .at(1)
            .write(&mut context, 0x80);
        assert_eq!(
            super::input::check_start_button(&mut context, &[]).unwrap(),
            Some(1)
        );
    }
}

#[test]
fn split_mode_nonforced_early_return_does_not_publish_camera_mirrors() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        ram::g_player_lives(revision).at(0).write(&mut context, -1);
        ram::g_camera1_pending_x(revision).write(&mut context, 99);
        ram::g_camera1_pending_y(revision).write(&mut context, 77);
        ram::g_camera2_pending_x(revision).write(&mut context, 33);
        ram::g_camera2_pending_y(revision).write(&mut context, 22);
        context.writes.clear();
        // The declared char ignores the high argument-slot bits.
        super::views::update_screen_split_mode(&mut context, &[0x100]).unwrap();
        assert!(context.writes.is_empty());
        assert_eq!(ram::g_camera2_pending_x(revision).read(&context), 33);
        assert_eq!(ram::g_camera2_pending_y(revision).read(&context), 22);
    }
}
