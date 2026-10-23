//! Explicitly synthetic, returning inputs on a fork of a reached gameplay state.
use super::check;
use tje_game::{Context, ids};
use tje_host::Host;
use tje_mem::{Address, Ptr, RenderFrame, RenderPiece, ram};

pub fn fixtures(host: &mut Host) {
    let revision = host.revision();
    let entity = ram::g_player_info(revision).at(39);
    let buffer = ram::g_sprite_tile_upload_buffer(revision).address();
    let frame = Address::<RenderFrame>::new(buffer + 7168);
    let piece = Address::<RenderPiece>::new(frame.address() + 4);
    let second = piece.offset(1);
    let stream = buffer + 7900;
    frame.piece_count().write(&mut host.context(), 2);
    for (index, part) in [piece, second].into_iter().enumerate() {
        part.width().write(&mut host.context(), 1);
        part.height().write(&mut host.context(), 1);
        part.x().write(&mut host.context(), -4 + index as i8);
        part.flipped_x().write(&mut host.context(), 4 - index as i8);
        part.y().write(&mut host.context(), -2 - index as i8 * 8);
        part.tile_data()
            .write(&mut host.context(), Ptr::new(stream));
    }
    entity
        .current_frame()
        .write(&mut host.context(), Ptr::new(frame.address()));
    entity.next().write(&mut host.context(), Ptr::new(0));
    entity.entity_index().write(&mut host.context(), 0);
    entity.sprite_ready().write(&mut host.context(), 1);
    entity.sprite_count().write(&mut host.context(), 2);
    for index in 0..2 {
        entity
            .primary_sprites()
            .at(index)
            .write(&mut host.context(), (25 + index) as i8);
        entity
            .secondary_sprites()
            .at(index)
            .write(&mut host.context(), (27 + index) as i8);
    }

    for level in [1, 26] {
        ram::g_player1_map_level(revision).write(&mut host.context(), level);
        for free in [None, Some(0), Some(19), Some(20), Some(33)] {
            for index in 0..34 {
                ram::g_dynamic_entity_slot_allocated(revision)
                    .at(index)
                    .write(&mut host.context(), i8::from(free != Some(index)));
            }
            check(host, ids::ALLOCATE_ENTITY, &[]);
        }
    }
    entity.sprite_count().write(&mut host.context(), 1);
    entity.tile_count().write(&mut host.context(), 8);
    for full in [false, true] {
        for index in 0..54 {
            ram::g_viewport_sprite_slot_flags(revision)
                .at(index)
                .write(&mut host.context(), if full { 3 } else { 0 });
        }
        for index in 0..80 {
            ram::g_sprite_tile_group_allocated(revision)
                .at(index)
                .write(&mut host.context(), 0);
        }
        for flags in [1, 3, 5, 7] {
            entity.draw_flags().write(&mut host.context(), flags);
            for shared in [0, 4] {
                entity.behavior_flags().write(&mut host.context(), shared);
                for viewport in [0, 1] {
                    for residual in [640, 648] {
                        host.context().set_data_register(4, 0xa5a5_0000 | residual);
                        check(
                            host,
                            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
                            &[entity.address(), viewport],
                        );
                    }
                }
            }
        }
    }
    for flags in [0, 1, 3, 5, 7] {
        entity.draw_flags().write(&mut host.context(), flags);
        entity.tile().write(&mut host.context(), 640);
        for shared in [0, 4] {
            entity.behavior_flags().write(&mut host.context(), shared);
            for state in [0, 1, 2, 3] {
                entity.primary_shadow().write(&mut host.context(), 29);
                entity.secondary_shadow().write(&mut host.context(), 30);
                ram::g_viewport_sprite_slot_flags(revision)
                    .at(4)
                    .write(&mut host.context(), state);
                ram::g_viewport_sprite_slot_flags(revision)
                    .at(5)
                    .write(&mut host.context(), state);
                check(host, ids::RELEASE_ENTITY_SHADOWS, &[entity.address()]);
                for viewport in [0, 1] {
                    check(
                        host,
                        ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                        &[entity.address(), viewport],
                    );
                }
            }
        }
    }

    for pending in [0, 255] {
        ram::g_sprite_graphics_upload_pending(revision).write(&mut host.context(), pending);
        for format in [0, -128] {
            piece.tile_format().write(&mut host.context(), format);
            let data = if format < 0 {
                vec![0xf8, 0x12, 0xf8, 0x34, 0xf8, 0x56, 0xf8, 0x78]
            } else {
                vec![0xe0, 0x12]
            };
            for (index, value) in data.into_iter().enumerate() {
                host.context().write_u8(stream + index as u32, value);
            }
            for tile in [0, 640, 0xffff] {
                ram::g_sprite_tile_buffer_offset(revision).write(&mut host.context(), 0);
                ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut host.context(), 0);
                check(
                    host,
                    ids::QUEUE_SPRITE_PIECE_TILES,
                    &[piece.address(), tile],
                );
            }
        }
    }
    entity.sprite_count().write(&mut host.context(), 2);
    entity.tile().write(&mut host.context(), 0xe280);
    for format in [0i8, -128, 0x43] {
        for part in [piece, second] {
            part.tile_format().write(&mut host.context(), format);
        }
        let data = if format < 0 {
            vec![0xf8, 0x12, 0xf8, 0x34, 0xf8, 0x56, 0xf8, 0x78]
        } else if format & 0x40 != 0 {
            (0..32).map(|index| index * 7).collect()
        } else {
            vec![0xe0, 0x12]
        };
        for (index, value) in data.into_iter().enumerate() {
            host.context().write_u8(stream + index as u32, value);
        }
        for elevation in [0i16, -1, -8, -20] {
            entity.height().write(&mut host.context(), elevation);
            for mask in [0, 0x20] {
                entity.behavior_flags().write(&mut host.context(), mask);
                for fall in [0, 0x20] {
                    entity
                        .animation_group_mask()
                        .write(&mut host.context(), fall);
                    for reuse in [0, 1] {
                        for viewport in [0, 1] {
                            ram::g_sprite_tile_buffer_offset(revision)
                                .write(&mut host.context(), 0);
                            ram::g_sprite_tile_upload_queue_word_count(revision)
                                .write(&mut host.context(), 0);
                            check(
                                host,
                                ids::PREPARE_ENTITY_SPRITE_FRAME,
                                &[entity.address(), frame.address(), reuse, viewport],
                            );
                        }
                    }
                }
            }
        }
    }

    entity.x().write(&mut host.context(), 300);
    entity.y().write(&mut host.context(), 200);
    entity.owner().write(&mut host.context(), -1);
    entity
        .attachment_x_or_latch()
        .write(&mut host.context(), -1);
    entity
        .attachment_y_or_latch()
        .write(&mut host.context(), -128);
    entity.primary_shadow().write(&mut host.context(), 29);
    entity.secondary_shadow().write(&mut host.context(), 30);
    for elevation in [-225i16, -224, -1, 0, 224, 225] {
        entity.height().write(&mut host.context(), elevation);
        for flags in [1, 3, 5, 7] {
            entity.draw_flags().write(&mut host.context(), flags);
            for hidden_or_attached in [0, 8, 0x40] {
                entity
                    .behavior_flags()
                    .write(&mut host.context(), hidden_or_attached);
                for flip in [0, 0x0800] {
                    entity.tile().write(&mut host.context(), flip);
                    for state in [0, 2] {
                        ram::g_viewport_sprite_slot_flags(revision)
                            .at(4)
                            .write(&mut host.context(), state);
                        ram::g_viewport_sprite_slot_flags(revision)
                            .at(5)
                            .write(&mut host.context(), state);
                        check(host, ids::POSITION_ENTITY_SPRITES, &[entity.address()]);
                    }
                }
            }
        }
    }
    // The signed-char piece-count branch is distinct from a 128-piece frame.
    frame.piece_count().write(&mut host.context(), 128);
    entity.behavior_flags().write(&mut host.context(), 0);
    check(host, ids::POSITION_ENTITY_SPRITES, &[entity.address()]);
}
