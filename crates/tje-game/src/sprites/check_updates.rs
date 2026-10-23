//! Live-state fixtures for residency, shadows, and animation-controller branches.
use super::check;
use tje_game::{Context, ids};
use tje_host::Host;
use tje_mem::{Address, EarthlingState, Ptr, RenderFrame, ram};

pub fn fixtures(host: &mut Host) {
    let revision = host.revision();
    let flags = ram::g_viewport_sprite_slot_flags(revision);
    for number in [0, 1, 2, 6, 39] {
        let entity = ram::g_player_info(revision).at(number);
        entity.sprite_count().write(&mut host.context(), 1);
        entity.tile_count().write(&mut host.context(), 8);
        entity.tile().write(&mut host.context(), 640);
        entity
            .primary_sprites()
            .at(0)
            .write(&mut host.context(), 25);
        entity
            .secondary_sprites()
            .at(0)
            .write(&mut host.context(), 26);
        entity.primary_shadow().write(&mut host.context(), 29);
        entity.secondary_shadow().write(&mut host.context(), 30);
        for active in [0, 1, 3, 5, 7] {
            entity.draw_flags().write(&mut host.context(), active);
            for shared in [0, 4] {
                entity.behavior_flags().write(&mut host.context(), shared);
                check(host, ids::FREE_ENTITY, &[number as u32]);
            }
        }
    }
    let entity = ram::g_player_info(revision).at(39);
    for active in [1, 3, 5, 7] {
        entity.draw_flags().write(&mut host.context(), active);
        for initial in [0, 29] {
            entity.primary_shadow().write(&mut host.context(), initial);
            entity
                .secondary_shadow()
                .write(&mut host.context(), initial);
            for unavailable in [false, true] {
                for index in 0..54 {
                    flags
                        .at(index)
                        .write(&mut host.context(), if unavailable { 3 } else { 0 });
                }
                for height in [i16::MIN, -1, 0, 111, 112, 127, 128, i16::MAX] {
                    entity.height().write(&mut host.context(), height);
                    for state in [-1, 0] {
                        entity.sprite_state().write(&mut host.context(), state);
                        for old_depth in [0, 0x70] {
                            ram::g_hardware_sprite_table(revision)
                                .at(29)
                                .size()
                                .write(&mut host.context(), old_depth);
                            check(host, ids::UPDATE_ENTITY_SHADOWS, &[entity.address()]);
                        }
                    }
                }
            }
        }
    }
    for index in 0..40 {
        ram::g_player_info(revision)
            .at(index)
            .draw_flags()
            .write(&mut host.context(), 0);
    }
    check(host, ids::UPDATE_ALL_ENTITY_SHADOWS, &[]);
    let first = ram::g_player_info(revision).at(6);
    first.draw_flags().write(&mut host.context(), 0x43);
    first.primary_shadow().write(&mut host.context(), 29);
    first.sprite_state().write(&mut host.context(), 0);
    first.height().write(&mut host.context(), 16);
    let second = ram::g_player_info(revision).at(7);
    second.draw_flags().write(&mut host.context(), 1);
    second.primary_shadow().write(&mut host.context(), 30);
    check(host, ids::UPDATE_ALL_ENTITY_SHADOWS, &[]);

    ram::g_player1_map_level(revision).write(&mut host.context(), 1);
    ram::g_player2_map_level(revision).write(&mut host.context(), 1);
    ram::g_split_screen_enabled(revision).write(&mut host.context(), 1);
    ram::g_player_inventory_open(revision)
        .at(0)
        .write(&mut host.context(), 0);
    ram::g_player2_inventory_open(revision).write(&mut host.context(), 0);
    ram::g_camera1_pending_x(revision).write(&mut host.context(), 0);
    ram::g_camera1_pending_y(revision).write(&mut host.context(), 0);
    ram::g_camera2_pending_x(revision).write(&mut host.context(), 0);
    ram::g_camera2_pending_y(revision).write(&mut host.context(), 0);
    for number in [0, 1, 39] {
        let entity = ram::g_player_info(revision).at(number);
        entity.x().write(&mut host.context(), 100);
        entity.y().write(&mut host.context(), 80);
        entity.height().write(&mut host.context(), 0);
        entity.width().write(&mut host.context(), 16);
        entity.height_pixels().write(&mut host.context(), 16);
        entity.level().write(&mut host.context(), 1);
        entity.sprite_count().write(&mut host.context(), 1);
        entity.tile_count().write(&mut host.context(), 8);
        entity.tile().write(&mut host.context(), 640);
        entity
            .primary_sprites()
            .at(0)
            .write(&mut host.context(), 25);
        entity
            .secondary_sprites()
            .at(0)
            .write(&mut host.context(), 26);
        entity.primary_shadow().write(&mut host.context(), 0);
        entity.secondary_shadow().write(&mut host.context(), 0);
        for active in [0, 1, 3, 5, 7] {
            for shared in [0, 4] {
                for unavailable in [false, true] {
                    entity.draw_flags().write(&mut host.context(), active);
                    entity.behavior_flags().write(&mut host.context(), shared);
                    for index in 0..54 {
                        flags
                            .at(index)
                            .write(&mut host.context(), if unavailable { 3 } else { 0 });
                    }
                    host.context().set_data_register(4, 0xa5a5_0280);
                    check(host, ids::UPDATE_ENTITY_VIEWPORT_SPRITES, &[number as u32]);
                }
            }
        }
    }
    for level in [1, 26] {
        for index in 0..40 {
            ram::g_player_info(revision)
                .at(index)
                .draw_flags()
                .write(&mut host.context(), 0);
        }
        ram::g_player1_map_level(revision).write(&mut host.context(), level);
        entity.level().write(&mut host.context(), level);
        entity.draw_flags().write(&mut host.context(), 1);
        entity.behavior_flags().write(&mut host.context(), 0);
        ram::g_inventory_draw_list_head(revision).write(&mut host.context(), Ptr::new(0x123456));
        for index in 0..54 {
            flags.at(index).write(&mut host.context(), 0);
        }
        check(host, ids::UPDATE_ALL_ENTITY_VIEWPORT_SPRITES, &[]);
    }

    for index in 0..40 {
        ram::g_player_info(revision)
            .at(index)
            .draw_flags()
            .write(&mut host.context(), 0);
    }
    for index in 0..4 {
        ram::g_speech_bubble_entities(revision)
            .at(index)
            .write(&mut host.context(), 0);
    }
    let buffer = ram::g_sprite_tile_upload_buffer(revision).address();
    let frame = Address::<RenderFrame>::new(buffer + 7168);
    let sequence = buffer + 7600;
    let frame_table = buffer + 7620;
    entity.draw_flags().write(&mut host.context(), 7);
    entity
        .sequence()
        .write(&mut host.context(), Ptr::new(sequence));
    entity
        .frames()
        .write(&mut host.context(), Ptr::new(frame_table));
    entity.animation_motion_axes().write(&mut host.context(), 0);
    entity.tile_count().write(&mut host.context(), 1);
    host.context().write_u32(frame_table, frame.address());
    host.context().write_u8(sequence, 0);
    for duration in [0, 1, 2] {
        entity.frame_duration().write(&mut host.context(), duration);
        for current in [-1, 0] {
            entity.frame().write(&mut host.context(), current);
            for command in [0, 254, 255] {
                host.context().write_u8(sequence + 1, command);
                for reuse in [0, 1] {
                    frame.reuse_tiles().write(&mut host.context(), reuse);
                    for ready in [0, 1] {
                        entity.sprite_ready().write(&mut host.context(), ready);
                        check(host, ids::PROCESS_ENTITY_ANIMATION_MOTION, &[]);
                    }
                }
            }
        }
    }
    let earthling = Address::<EarthlingState>::new(buffer + 6000);
    earthling
        .health_visibility()
        .at(1)
        .write(&mut host.context(), 0);
    entity
        .earthling()
        .write(&mut host.context(), Ptr::new(earthling.address()));
    ram::g_player_inventory_open(revision)
        .at(0)
        .write(&mut host.context(), 1);
    ram::g_player2_inventory_open(revision).write(&mut host.context(), 1);
    entity.frame_duration().write(&mut host.context(), 1);
    entity.frame().write(&mut host.context(), -1);
    entity.animation_motion_axes().write(&mut host.context(), 3);
    for callback in [
        ids::PLAYER1_PROCESS_TERRAIN_MOTION,
        ids::PLAYER2_PROCESS_TERRAIN_MOTION,
        ids::EARTHLING_MOVE_ON_ALLOWED_TERRAIN,
        ids::EARTHLING_MOVE_WITHIN_MAP_BOUNDS,
    ] {
        entity.process().write(
            &mut host.context(),
            Ptr::new(callback.metadata().address(revision)),
        );
        check(host, ids::PROCESS_ENTITY_ANIMATION_MOTION, &[]);
    }
    // Distinguish the original over-budget not-ready bug from the corrected <= test.
    for index in 0..40 {
        ram::g_player_info(revision)
            .at(index)
            .draw_flags()
            .write(&mut host.context(), 0);
    }
    for index in 0..4 {
        ram::g_speech_bubble_entities(revision)
            .at(index)
            .write(&mut host.context(), -1);
    }
    let first = ram::g_player_info(revision).at(0);
    first.draw_flags().write(&mut host.context(), 0x37);
    first.tile_count().write(&mut host.context(), 255);
    first.frame_duration().write(&mut host.context(), 2);
    first.sprite_ready().write(&mut host.context(), 0);
    first.animation_motion_axes().write(&mut host.context(), 0);
    first.animation_group_mask().write(&mut host.context(), 0);
    first.sprite_state().write(&mut host.context(), 0);
    first.x().write(&mut host.context(), 300);
    first.y().write(&mut host.context(), 300);
    let second = ram::g_player_info(revision).at(1);
    second.draw_flags().write(&mut host.context(), 7);
    second.tile_count().write(&mut host.context(), 1);
    second.frame_duration().write(&mut host.context(), 1);
    second.frame().write(&mut host.context(), -1);
    second
        .sequence()
        .write(&mut host.context(), Ptr::new(sequence));
    second
        .frames()
        .write(&mut host.context(), Ptr::new(frame_table));
    second.animation_motion_axes().write(&mut host.context(), 3);
    second.process().write(
        &mut host.context(),
        Ptr::new(
            ids::PLAYER1_PROCESS_TERRAIN_MOTION
                .metadata()
                .address(revision),
        ),
    );
    ram::g_player_lives(revision)
        .at(0)
        .write(&mut host.context(), 1);
    ram::g_player_inventory_open(revision)
        .at(0)
        .write(&mut host.context(), 0);
    check(host, ids::PROCESS_ENTITY_ANIMATION_MOTION, &[]);
}
