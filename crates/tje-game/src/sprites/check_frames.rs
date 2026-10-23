//! Source-backed stack-local and mutable-argument sprite proofs.
use super::check;
use tje_game::ids;
use tje_host::Host;
use tje_mem::{Address, Ptr, RenderFrame, ram};

pub fn fixtures(host: &mut Host) {
    let revision = host.revision();
    let buffer = ram::g_sprite_tile_upload_buffer(revision).address();
    let output = buffer + 5000;
    let flags = ram::g_viewport_sprite_slot_flags(revision);
    for pattern in 0..4 {
        for count in [1i16, 2, 12, 13, 15] {
            for mode in [0i16, 1, -1, 2] {
                for secondary in [0u32, 1, 256] {
                    for index in 0..54 {
                        let state = match pattern {
                            0 => 0,
                            1 => 1,
                            2 => 3,
                            _ => [0, 1, 3][index as usize % 3],
                        };
                        flags.at(index).write(&mut host.context(), state);
                    }
                    check(
                        host,
                        ids::ALLOCATE_SPRITE_SLOTS,
                        &[count as i32 as u32, mode as i32 as u32, secondary, output],
                    );
                }
            }
        }
    }
    for state in [0, 1, 2, 3, 4, 7] {
        for mode in [i16::MIN, -1, 0, 1, i16::MAX] {
            for secondary in [0u32, 1, 256, u32::MAX] {
                for index in 0..54 {
                    flags.at(index).write(&mut host.context(), state);
                }
                check(
                    host,
                    ids::ALLOCATE_SPRITE_SLOT,
                    &[mode as i32 as u32, secondary],
                );
            }
        }
    }

    let header = Address::<RenderFrame>::new(buffer + 6200);
    let first = ram::g_player_info(revision).at(6);
    let second = ram::g_player_info(revision).at(7);
    let inventory = ram::g_player_info(revision).at(39);
    ram::g_entity_draw_list_head(revision).write(&mut host.context(), Ptr::new(0));
    ram::g_inventory_draw_list_head(revision).write(&mut host.context(), Ptr::new(0));
    check(host, ids::BUILD_VIEWPORT_SPRITE_LINKS, &[]);
    for entity in [first, second, inventory] {
        entity
            .current_frame()
            .write(&mut host.context(), Ptr::new(header.address()));
        entity.sprite_ready().write(&mut host.context(), 1);
        entity.next().write(&mut host.context(), Ptr::new(0));
    }
    first
        .next()
        .write(&mut host.context(), Ptr::new(second.address()));
    first.draw_flags().write(&mut host.context(), 0x42); // World list does not re-test ACTIVE.
    second.draw_flags().write(&mut host.context(), 0x45);
    first.primary_shadow().write(&mut host.context(), 29);
    first.secondary_shadow().write(&mut host.context(), 30);
    second.primary_shadow().write(&mut host.context(), 31);
    second.secondary_shadow().write(&mut host.context(), 32);
    for index in 0..12 {
        first
            .primary_sprites()
            .at(index)
            .write(&mut host.context(), (25 + index) as i8);
        second
            .secondary_sprites()
            .at(index)
            .write(&mut host.context(), (27 + index) as i8);
        inventory
            .primary_sprites()
            .at(index)
            .write(&mut host.context(), (33 + index) as i8);
        inventory
            .secondary_sprites()
            .at(index)
            .write(&mut host.context(), (35 + index) as i8);
    }
    ram::g_entity_draw_list_head(revision).write(&mut host.context(), Ptr::new(first.address()));
    ram::g_inventory_draw_list_head(revision)
        .write(&mut host.context(), Ptr::new(inventory.address()));
    for count in [0, 1, 2, 12] {
        header.piece_count().write(&mut host.context(), count);
        for ready in [0, 1] {
            first.sprite_ready().write(&mut host.context(), ready);
            second.sprite_ready().write(&mut host.context(), ready);
            for active in [0, 1, 7] {
                inventory.draw_flags().write(&mut host.context(), active);
                check(host, ids::BUILD_VIEWPORT_SPRITE_LINKS, &[]);
            }
        }
    }
    // Exactly 54 primary handles fill the real local array without a host bound.
    header.piece_count().write(&mut host.context(), 9);
    ram::g_inventory_draw_list_head(revision).write(&mut host.context(), Ptr::new(0));
    for index in 0..6 {
        let entity = ram::g_player_info(revision).at(6 + index);
        entity
            .current_frame()
            .write(&mut host.context(), Ptr::new(header.address()));
        entity.sprite_ready().write(&mut host.context(), 1);
        entity.draw_flags().write(&mut host.context(), 3);
        entity.next().write(
            &mut host.context(),
            Ptr::new(if index == 5 {
                0
            } else {
                ram::g_player_info(revision).at(7 + index).address()
            }),
        );
        for piece in 0..9 {
            entity
                .primary_sprites()
                .at(piece)
                .write(&mut host.context(), (25 + index * 9 + piece) as i8);
        }
    }
    check(host, ids::BUILD_VIEWPORT_SPRITE_LINKS, &[]);
    // 55/56 primary handles overwrite the saved old A6 but not the return PC.
    // The original UNLK restoration is therefore an observable unchecked overflow.
    let last_header = Address::<RenderFrame>::new(header.address() + 16);
    let last_entity = ram::g_player_info(revision).at(11);
    last_entity
        .current_frame()
        .write(&mut host.context(), Ptr::new(last_header.address()));
    for count in [10, 11] {
        last_header.piece_count().write(&mut host.context(), count);
        last_entity
            .primary_sprites()
            .at(9)
            .write(&mut host.context(), 25);
        last_entity
            .primary_sprites()
            .at(10)
            .write(&mut host.context(), 26);
        check(host, ids::BUILD_VIEWPORT_SPRITE_LINKS, &[]);
    }
}
