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
fn start_query_keeps_compiled_byte_read_order() {
    use super::test_support::Access;
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        let input = ram::g_player_processed_input(revision);
        let previous = ram::g_player_previous_processed_input(revision);
        previous.at(0).write(&mut context, 0x80);
        previous.at(1).write(&mut context, 0x80);
        for (first, second) in [
            (0, 0),
            (0x7f, 0x7f),
            (0x80, 0),
            (0, 0x80),
            (0x80, 0x80),
            (0xff, 0xff),
        ] {
            input.at(0).write(&mut context, first);
            input.at(1).write(&mut context, second);
            context.accesses.borrow_mut().clear();
            context.trace_accesses = true;
            let held = (first | second) & 0x80 != 0;
            assert_eq!(
                super::input::check_start_button(&mut context, &[]).unwrap(),
                Some(u32::from(held)),
            );
            let mut accesses = vec![
                Access::Read(input.at(1).address(), 1),
                Access::Read(input.at(0).address(), 1),
            ];
            if held {
                accesses.push(Access::Read(input.at(0).address(), 1));
                let player = i32::from(first & 0x80 == 0);
                accesses.push(Access::Read(previous.at(player).address(), 1));
            }
            assert_eq!(*context.accesses.borrow(), accesses);
            context.trace_accesses = false;
        }
    }
}

#[test]
fn split_mode_nonforced_early_return_does_not_publish_camera_mirrors() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        context.prepare_entry(crate::ids::UPDATE_SCREEN_SPLIT_MODE, &[0x100]);
        ram::g_player_lives(revision).at(0).write(&mut context, -1);
        ram::g_camera1_pending_x(revision).write(&mut context, 99);
        ram::g_camera1_pending_y(revision).write(&mut context, 77);
        ram::g_camera2_pending_x(revision).write(&mut context, 33);
        ram::g_camera2_pending_y(revision).write(&mut context, 22);
        context.writes.clear();
        // The declared char ignores the high argument-slot bits.
        super::views::update_screen_split_mode(&mut context).unwrap();
        assert!(context.writes.is_empty());
        assert_eq!(ram::g_camera2_pending_x(revision).read(&context), 33);
        assert_eq!(ram::g_camera2_pending_y(revision).read(&context), 22);
    }
}

#[test]
fn menu_continuation_clears_music_with_a_discarded_byte_before_vdp_output() {
    use super::test_support::Access;
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        context.prepare_entry(crate::ids::FURTHER_INITIALISATION, &[]);
        let requested = ram::g_requested_music_selection(revision);
        requested.write(&mut context, -1);
        context.trace_accesses = true;
        // Stop at the actual foreign Functional copy routine; this verifies
        // the owned prefix, not an invented transfer or clock.
        let error = super::main_loop::after_menu(&mut context, &[]).unwrap_err();
        assert!(matches!(error, crate::Error::ExecutionUnavailable { .. }));
        assert_eq!(
            &context.accesses.borrow()[..4],
            &[
                Access::Discard(requested.address(), 1),
                Access::Write(requested.address(), 1, 0),
                Access::Control(0x4000),
                Access::Control(0),
            ],
        );
    }
}

#[test]
fn saved_register_epilogue_records_a_discarded_terminal_word() {
    use super::test_support::Access;
    use crate::Context;
    use crate::call_frame::{FrameLayout, LocalFrame};
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        context.prepare_entry(crate::ids::PROCESS_INPUT, &[]);
        let entry_sp = context.sp;
        context.set_data_register(2, 0x12345678);
        let frame = LocalFrame::enter(&mut context, FrameLayout::movem(0, 1 << 2, 0)).unwrap();
        context.set_data_register(2, 0);
        context.trace_accesses = true;
        frame.leave(&mut context).unwrap();
        assert_eq!(context.data_register(2), 0x12345678);
        assert_eq!(context.sp, entry_sp);
        assert_eq!(
            *context.accesses.borrow(),
            vec![
                Access::Read(entry_sp - 4, 2),
                Access::Read(entry_sp - 2, 2),
                Access::Discard(entry_sp, 2),
            ],
        );
    }
}
