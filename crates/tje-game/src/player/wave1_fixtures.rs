use super::check;
use tje_game::ids;
use tje_host::Host;
use tje_mem::ram;
pub(super) fn run(host: &mut Host) {
    let revision = host.revision();
    let p = ram::g_player_info(revision).at(0);
    let q = ram::g_player_info(revision).at(1);
    for level in [0, 1, 26, -1] {
        p.level().write(&mut host.context(), level);
        for (x, y) in [(0, 0), (15, 4), (16, 5), (-15, -4), (i16::MIN, i16::MAX)] {
            p.x().write(&mut host.context(), x);
            p.y().write(&mut host.context(), y);
            check(
                host,
                ids::ENTITY_IS_NEAR_POSITION,
                &[p.address(), 0, 0, level as u32],
            );
            q.level().write(&mut host.context(), level);
            q.x().write(&mut host.context(), 0);
            q.y().write(&mut host.context(), 0);
            check(
                host,
                ids::ENTITY_IS_WITHIN_RANGE,
                &[p.address(), q.address(), 16, 5],
            );
            check(
                host,
                ids::ENTITY_IS_WITHIN_RANGE,
                &[p.address(), q.address(), u32::MAX, u32::MAX],
            );
        }
    }
    for number in 0..2 {
        let p = ram::g_player_info(revision).at(number);
        p.entity_index().write(&mut host.context(), number as i8);
        p.level().write(&mut host.context(), 1);
        for terrain in [0x10, 0x50, 0x80, 0x90] {
            for (state, height, mask) in [
                (8, 0, 4),
                (3, -5, 1),
                (44, -1, 0x800),
                (60, 40, 0x8000),
                (104, 0, 0x400_0000),
            ] {
                p.sprite_state().write(&mut host.context(), state);
                p.animation_group_mask().write(&mut host.context(), mask);
                p.height().write(&mut host.context(), height);
                check(
                    host,
                    ids::OPEN_ROCKET_SKATES,
                    &[p.address(), number as u32, terrain],
                );
                for input in [0, 0x40] {
                    ram::g_player_processed_input(revision)
                        .at(number)
                        .write(&mut host.context(), input);
                    check(
                        host,
                        ids::PLAYER_UPDATE_ROCKET_SKATES_SPRITE,
                        &[p.address(), number as u32, terrain],
                    );
                }
            }
        }
        for direction in 0..4 {
            p.sprite_state().write(&mut host.context(), 92 + direction);
            for (mask, flags, vx, vy) in [
                (0x80_0000, 0, 0, 0),
                (0x400_0000, 0, 4000, 3000),
                (0x400_0000, 0, -4000, -3000),
                (0x400_0000, 0, 0, 3000),
                (0x400_0000, 0x30, 100, -200),
            ] {
                p.animation_group_mask().write(&mut host.context(), mask);
                p.draw_flags().write(&mut host.context(), flags);
                p.velocity_x().write(&mut host.context(), vx);
                p.velocity_y().write(&mut host.context(), vy);
                for input in [0, 1, 2, 4, 8, 5, 6, 9, 10, 15, 0xff] {
                    for changed in [0, 1] {
                        check(
                            host,
                            ids::PLAYER_APPLY_ROCKET_SKATES_INPUT,
                            &[p.address(), number as u32, changed, input],
                        );
                    }
                }
            }
            check(host, ids::PLAYER_START_TEETERING_FALL, &[p.address()]);
        }
        check(host, ids::OPEN_SCHOOL_BOOK, &[p.address()]);
        for secondary in [0, 1] {
            ram::g_split_screen_enabled(revision).write(&mut host.context(), secondary);
            ram::g_terrain_priority_update_player(revision)
                .write(&mut host.context(), number as i16);
            p.x().write(&mut host.context(), 512);
            p.y().write(&mut host.context(), 512);
            check(host, ids::MAP_CLEAR_TILE_PRIORITY_NEAR_PLAYER, &[]);
        }
    }
    let packet = ram::g_player_info(revision).at(39).x().address();
    for number in 0..2 {
        let p = ram::g_player_info(revision).at(number);
        ram::g_player1_map_level(revision).write(&mut host.context(), 1);
        for terrain in [0x10, 0x50, 0x80, 0x90] {
            let cache = if number == 0 {
                ram::g_player1_nearby_terrain(revision)
            } else {
                ram::g_player2_nearby_terrain(revision)
            }
            .cast::<u8>();
            for index in 0..25 {
                cache.offset(index).write(&mut host.context(), terrain);
            }
            for (mask, state, height) in [(4, 8, 0), (0x800, 44, 0), (0x40000, 72, 0), (4, 8, 5)] {
                for (tx, ty) in [(0, 0), (2, 2), (4, 4)] {
                    p.animation_group_mask().write(&mut host.context(), mask);
                    p.sprite_state().write(&mut host.context(), state);
                    p.height().write(&mut host.context(), height);
                    p.x().write(&mut host.context(), 512);
                    p.y().write(&mut host.context(), 512);
                    p.velocity_x().write(&mut host.context(), 256);
                    p.velocity_y().write(&mut host.context(), -256);
                    let mut c = host.context();
                    tje_mem::Address::<i16>::new(packet).write(&mut c, 513);
                    tje_mem::Address::<i16>::new(packet + 2).write(&mut c, 511);
                    tje_mem::Address::<i16>::new(packet + 6).write(&mut c, tx);
                    tje_mem::Address::<i16>::new(packet + 8).write(&mut c, ty);
                    tje_mem::Address::<u8>::new(packet + 10).write(&mut c, 200);
                    tje_mem::Address::<u8>::new(packet + 11).write(&mut c, 100);
                    check(
                        host,
                        ids::PLAYER_APPLY_TERRAIN_DESTINATION,
                        &[number as u32, p.address(), packet],
                    );
                }
            }
        }
    }
}
