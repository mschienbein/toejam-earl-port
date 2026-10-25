use super::{
    clear_display_and_publish, enter_shared_screen, set_display_and_publish,
    update_screen_split_mode,
};
use crate::core::test_support::{Access, TestContext};
use crate::{Context, Error};
use tje_mem::{Revision, ram};

#[test]
fn mode2_memory_rmw_is_followed_by_a_distinct_publish_read() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (initial, stored, control, publish) in [
            (
                0xe7,
                0x24,
                0x8124,
                clear_display_and_publish as fn(&mut dyn Context) -> u8,
            ),
            (
                0x20,
                0x60,
                0x8164,
                set_display_and_publish as fn(&mut dyn Context) -> u8,
            ),
        ] {
            let mut context = TestContext::new(revision);
            let shadow = ram::g_mode2_reg_softcopy(revision);
            shadow.write(&mut context, initial);
            context.accesses.borrow_mut().clear();
            context.trace_accesses = true;
            assert_eq!(u32::from(publish(&mut context)), stored);
            assert_eq!(
                *context.accesses.borrow(),
                vec![
                    Access::Read(shadow.address(), 1),
                    Access::Write(shadow.address(), 1, stored),
                    Access::Read(shadow.address(), 1),
                    Access::Control(control),
                ]
            );
        }
    }
}

#[test]
fn shared_midpoint_reads_earl_before_toejam_and_reloads_the_selected_fields() {
    // Matching c_startup_and_main_loop.s:878-926: each CMP consumes Earl's
    // coordinate first, then ToeJam's, followed by the chosen field's new load.
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (toejam_x, earl_x, toejam_y, earl_y) in [
            (480, 160, 640, 200),
            (160, 480, 200, 640),
            (160, 160, 200, 200),
        ] {
            let mut context = TestContext::new(revision);
            let arguments = context.prepare_entry(crate::ids::ENTER_SHARED_SCREEN, &[0, 0]);
            let toejam = ram::g_player_info(revision).at(0);
            let earl = ram::g_player_info(revision).at(1);
            for (entity, x, y) in [(toejam, toejam_x, toejam_y), (earl, earl_x, earl_y)] {
                entity.x().write(&mut context, x);
                entity.y().write(&mut context, y);
                entity.height_pixels().write(&mut context, 48);
                entity.height().write(&mut context, 20);
                entity
                    .animation_group_mask()
                    .write(&mut context, 0x04008000);
            }
            let selected_x = if toejam_x > earl_x { earl } else { toejam };
            let selected_y = if toejam_y > earl_y { earl } else { toejam };
            context.trace_accesses = true;
            // This Atomic synthetic context deliberately cannot enter the
            // foreign Functional camera callee. The test covers the complete
            // owned prefix, not a mock camera or a claimed callee proof.
            let error = enter_shared_screen(&mut context).unwrap_err();
            assert!(matches!(error, Error::ExecutionUnavailable { .. }));
            assert_eq!(
                *context.accesses.borrow(),
                vec![
                    Access::Read(arguments.argument_word(0).address(), 2),
                    Access::Read(arguments.argument_word(1).address(), 2),
                    Access::Read(earl.x().address(), 2),
                    Access::Read(toejam.x().address(), 2),
                    Access::Read(selected_x.x().address(), 2),
                    Access::Read(earl.y().address(), 2),
                    Access::Read(toejam.y().address(), 2),
                    Access::Read(selected_y.y().address(), 2),
                    Access::Read(selected_y.height_pixels().address(), 1),
                    Access::Read(selected_y.height().address(), 2),
                    Access::Read(selected_y.animation_group_mask().address(), 2),
                    Access::Read(selected_y.animation_group_mask().address() + 2, 2),
                    Access::Read(selected_y.height().address(), 2),
                    Access::Read(ram::g_mode2_reg_softcopy(revision).address(), 1),
                    Access::Discard(ram::g_split_screen_enabled(revision).address(), 1),
                    Access::Write(ram::g_split_screen_enabled(revision).address(), 1, 0),
                ]
            );
        }
    }
}

#[test]
fn split_hysteresis_keeps_compiled_level_and_flying_coordinate_read_order() {
    // G/build/rev{02,00}/asm/c_startup_and_main_loop.s:1171-1266.
    for revision in [Revision::Rev02, Revision::Rev00] {
        for flying_player in [0, 1] {
            let mut context = TestContext::new(revision);
            let arguments = context.prepare_entry(crate::ids::UPDATE_SCREEN_SPLIT_MODE, &[0]);
            let toejam = ram::g_player_info(revision).at(0);
            let earl = ram::g_player_info(revision).at(1);
            for (player, entity) in [(0, toejam), (1, earl)] {
                ram::g_player_lives(revision)
                    .at(player)
                    .write(&mut context, 1);
                entity.level().write(&mut context, 1);
                entity.x().write(&mut context, 160 + player as i16 * 40);
                entity.y().write(
                    &mut context,
                    if player == flying_player { 100 } else { 120 },
                );
                entity
                    .height()
                    .write(&mut context, if player == flying_player { 10 } else { 0 });
                entity
                    .animation_group_mask()
                    .write(&mut context, 0x04008000);
            }
            context.trace_accesses = true;
            update_screen_split_mode(&mut context).unwrap();
            let mut expected = vec![
                Access::Read(arguments.argument_byte(0).address(), 1),
                Access::Read(ram::g_player_lives(revision).at(0).address(), 1),
                Access::Read(ram::g_player_lives(revision).at(1).address(), 1),
                Access::Read(earl.level().address(), 1),
                Access::Read(toejam.level().address(), 1),
                Access::Read(toejam.y().address(), 2),
                Access::Read(earl.y().address(), 2),
                Access::Read(toejam.x().address(), 2),
                Access::Read(earl.x().address(), 2),
                Access::Read(toejam.height().address(), 2),
            ];
            let (flying, first_y, second_y) = if flying_player == 0 {
                (toejam, earl, toejam)
            } else {
                expected.push(Access::Read(earl.height().address(), 2));
                (earl, toejam, earl)
            };
            expected.extend([
                Access::Read(flying.animation_group_mask().address(), 2),
                Access::Read(flying.animation_group_mask().address() + 2, 2),
                Access::Read(first_y.y().address(), 2),
                Access::Read(second_y.y().address(), 2),
                Access::Read(flying.height().address(), 2),
                Access::Read(ram::g_split_screen_enabled(revision).address(), 1),
                Access::Read(ram::g_split_screen_enabled(revision).address(), 1),
                Access::Read(ram::g_split_screen_enabled(revision).address(), 1),
                Access::Read(ram::g_camera1_pending_x(revision).address(), 2),
                Access::Write(ram::g_camera2_pending_x(revision).address(), 2, 0),
                Access::Read(ram::g_camera1_pending_y(revision).address(), 2),
                Access::Write(ram::g_camera2_pending_y(revision).address(), 2, 0),
            ]);
            assert_eq!(*context.accesses.borrow(), expected);
        }
    }
}
