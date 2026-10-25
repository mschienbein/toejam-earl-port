use super::*;
use crate::camera::movement::tests::{Access, Machine};
use tje_mem::Revision;

#[test]
fn player_split_restores_coordinates_and_orders_height_reads() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut machine = Machine::new(revision);
        let data = machine.data;
        let addresses = machine.address;
        let first = ram::g_player_info(revision).at(0);
        let second = first.offset(1);
        for (entity, value) in [(first, 200), (second, 250)] {
            entity.x().write(&mut machine.memory, value);
            entity.y().write(&mut machine.memory, value);
            entity.height_pixels().write(&mut machine.memory, 32);
            entity.height().write(&mut machine.memory, 0);
        }
        ram::g_camera1_pending_x(revision).write(&mut machine.memory, 100);
        ram::g_camera1_pending_y(revision).write(&mut machine.memory, 100);
        animate_players_into_split_views(&mut machine, &[]).unwrap();
        assert_eq!(
            [
                first.x().read(&machine.memory),
                first.y().read(&machine.memory),
                second.x().read(&machine.memory),
                second.y().read(&machine.memory)
            ],
            [200, 200, 250, 250]
        );
        let reads = machine.main_accesses();
        let expected = [
            Access::Read(first.x().address(), 2),
            Access::Read(first.y().address(), 2),
            Access::Read(second.x().address(), 2),
            Access::Read(second.y().address(), 2),
            Access::Read(first.x().address(), 2),
            Access::Read(first.y().address(), 2),
            Access::Read(second.x().address(), 2),
            Access::Read(second.y().address(), 2),
            Access::Read(ram::g_camera1_pending_x(revision).address(), 2),
            Access::Read(first.height_pixels().address(), 1),
            Access::Read(ram::g_camera1_pending_y(revision).address(), 2),
            Access::Read(first.height().address(), 2),
            Access::Read(ram::g_camera1_pending_x(revision).address(), 2),
            Access::Read(second.height_pixels().address(), 1),
            Access::Read(ram::g_camera1_pending_y(revision).address(), 2),
            Access::Read(second.height().address(), 2),
        ];
        assert_eq!(&reads[..expected.len()], expected);
        let calls = machine
            .calls
            .iter()
            .filter(|call| {
                matches!(
                    call.id,
                    ids::POSITION_ENTITY_SPRITES
                        | ids::WAIT_FOR_NEXT_V_BLANK
                        | ids::UPLOAD_QUEUED_SPRITE_GRAPHICS
                )
            })
            .collect::<Vec<_>>();
        assert!(!calls.is_empty());
        assert_eq!(calls.len() % 3, 0);
        let entry = ids::ANIMATE_PLAYERS_INTO_SPLIT_VIEWS
            .metadata()
            .address(revision);
        for group in calls.as_chunks::<3>().0 {
            assert_eq!(
                [group[0].id, group[1].id, group[2].id],
                [
                    ids::POSITION_ENTITY_SPRITES,
                    ids::WAIT_FOR_NEXT_V_BLANK,
                    ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
                ]
            );
            assert_eq!(
                [group[0].sp, group[1].sp, group[2].sp],
                [Machine::SP - 84, Machine::SP - 88, Machine::SP - 96]
            );
            assert_eq!(
                [group[0].return_pc, group[1].return_pc, group[2].return_pc],
                [entry + 0x264, entry + 0x26e, entry + 0x27a]
            );
        }
        assert_eq!(&machine.data[2..], &data[2..]);
        assert_eq!(&machine.address[2..], &addresses[2..]);
        assert_eq!(machine.sp, Machine::SP);
    }
}

#[test]
fn split_division_uses_the_software_register_helper_only_when_emitted() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (numerator, divisor, software) in [
            (100_i32, 2_i32, false),
            (100000, 1, true),
            (i32::MIN, -1, true),
            (5, 65536, true),
        ] {
            let mut machine = Machine::new(revision);
            let expected = arithmetic::div_i32(numerator, divisor).unwrap();
            assert_eq!(
                split_divide(&mut machine, numerator, divisor, 0x1ac).unwrap(),
                expected
            );
            assert_eq!(machine.calls.len(), usize::from(software));
            if software {
                assert_eq!(machine.calls[0].id, ids::LDIVT);
                assert_eq!(
                    machine.calls[0].return_pc,
                    ids::ANIMATE_PLAYERS_INTO_SPLIT_VIEWS
                        .metadata()
                        .address(revision)
                        + 0x1ac
                );
            }
        }
    }
}

#[test]
fn zero_distance_keeps_original_divide_exception_instead_of_a_fallback() {
    let mut machine = Machine::new(Revision::Rev02);
    for (player, y) in [(0, 72), (1, 184)] {
        let entity = ram::g_player_info(Revision::Rev02).at(player);
        entity.x().write(&mut machine.memory, 160);
        entity.y().write(&mut machine.memory, y);
        entity.height_pixels().write(&mut machine.memory, 32);
    }
    assert!(matches!(
        animate_players_into_split_views(&mut machine, &[]),
        Err(crate::Error::Arithmetic(
            arithmetic::ArithmeticTrap::DivideByZero
        ))
    ));
    assert!(
        !machine
            .calls
            .iter()
            .any(|call| call.id == ids::WAIT_FOR_NEXT_V_BLANK)
    );
}

#[test]
fn wipe_preserves_allocator_d4_and_materializes_retained_wait_arguments() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for allocate in [false, true] {
            let mut machine = Machine::new(revision);
            let data = machine.data;
            let addresses = machine.address;
            for player in 0..2 {
                let entity = ram::g_player_info(revision).at(player);
                entity
                    .draw_flags()
                    .write(&mut machine.memory, if allocate { 1 } else { 3 });
                entity.sprite_ready().write(&mut machine.memory, 1);
            }
            ram::g_camera1_tile_row_cursor(revision).write(&mut machine.memory, 10);
            animate_split_screen_wipe(&mut machine, &[]).unwrap();
            let entry = ids::ANIMATE_SPLIT_SCREEN_WIPE.metadata().address(revision);
            let allocations = machine
                .calls
                .iter()
                .filter(|call| call.id == ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES)
                .collect::<Vec<_>>();
            assert_eq!(allocations.len(), if allocate { 2 } else { 0 });
            for call in allocations {
                assert_eq!(call.d4, data[4]);
                assert_eq!(call.sp, Machine::SP - 36);
                assert_eq!(call.return_pc, entry + 0x20);
            }
            if allocate {
                let accesses = machine.main_accesses();
                for player in 0..2 {
                    let ready = ram::g_player_info(revision)
                        .at(player)
                        .sprite_ready()
                        .address();
                    assert!(accesses.windows(2).any(
                        |pair| pair == [Access::Discard(ready, 1), Access::Write(ready, 1, 0),]
                    ));
                }
            }
            let link = machine
                .calls
                .iter()
                .find(|call| call.id == ids::LINK_PLAYER_SPRITES_FOR_SPLIT_WIPE)
                .unwrap();
            assert_eq!(link.sp, Machine::SP - 32);
            assert_eq!(link.argument, 1);
            assert_eq!(link.return_pc, entry + 0x80);
            assert_eq!(
                machine
                    .calls
                    .iter()
                    .filter(|call| call.id == ids::WAIT_FOR_NEXT_V_BLANK)
                    .count(),
                16
            );
            assert_eq!(
                machine
                    .devices
                    .iter()
                    .filter(|value| **value == (0xc00000, 0x25b4))
                    .count(),
                15 * 128
            );
            assert_eq!(
                machine
                    .devices
                    .iter()
                    .filter(|value| **value == (0xc00000, 0x05ab))
                    .count(),
                14 * 128
            );
            assert_eq!(
                machine
                    .devices
                    .iter()
                    .filter(|value| **value == (0xc00000, 0xa5b4))
                    .count(),
                28 * 40
            );
            assert_eq!(machine.devices.last(), Some(&(0xc00004, 0x921c)));
            assert_eq!(&machine.data[2..], &data[2..]);
            assert_eq!(&machine.address[2..], &addresses[2..]);
            assert_eq!(machine.sp, Machine::SP);
        }
    }
}
