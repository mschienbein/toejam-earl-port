//! Functional transaction proofs: stable RAM sources, no timing-equivalence claim.
use super::check;
use tje_game::ids;
use tje_host::Host;
use tje_mem::{Address, Ptr, RenderFrame, ram};

pub fn fixtures(host: &mut Host) {
    let revision = host.revision();
    let buffer = ram::g_sprite_tile_upload_buffer(revision).address();
    let first = Address::<RenderFrame>::new(buffer + 7000);
    let second = Address::<RenderFrame>::new(buffer + 7010);
    let toejam = ram::g_player_info(revision).at(0);
    let earl = ram::g_player_info(revision).at(1);
    toejam
        .current_frame()
        .write(&mut host.context(), Ptr::new(first.address()));
    earl.current_frame()
        .write(&mut host.context(), Ptr::new(second.address()));
    for index in 0..12 {
        toejam
            .primary_sprites()
            .at(index)
            .write(&mut host.context(), (25 + index) as i8);
        earl.primary_sprites()
            .at(index)
            .write(&mut host.context(), (37 + index) as i8);
    }
    for first_count in [0, 1, 2, 12, 128] {
        first.piece_count().write(&mut host.context(), first_count);
        for second_count in [0, 1, 2, 12, 128] {
            second
                .piece_count()
                .write(&mut host.context(), second_count);
            check(host, ids::LINK_PLAYER_SPRITES_FOR_SPLIT_WIPE, &[]);
        }
    }
    for index in 0..96 {
        host.context().write_u8(
            buffer + index,
            (index as u8).wrapping_mul(7).wrapping_add(3),
        );
    }
    let runs = ram::g_sprite_tile_upload_runs(revision);
    for (index, word) in [640, 1, 648, 2].into_iter().enumerate() {
        runs.at(index as i32).write(&mut host.context(), word);
    }
    for pending in [0, 1, 255] {
        ram::g_sprite_graphics_upload_pending(revision).write(&mut host.context(), pending);
        for force in [0i16, 1, -1] {
            for tiles_only in [0i16, 1] {
                for queue in [0, 2, 4] {
                    ram::g_sprite_tile_upload_queue_word_count(revision)
                        .write(&mut host.context(), queue);
                    ram::g_sprite_tile_buffer_offset(revision).write(&mut host.context(), 96);
                    check(
                        host,
                        ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
                        &[force as i32 as u32, tiles_only as i32 as u32],
                    );
                }
            }
        }
    }
    // A zero encoded DMA length really transfers 65,536 words; do not invent a no-op.
    runs.at(0).write(&mut host.context(), 640);
    runs.at(1).write(&mut host.context(), 0);
    ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut host.context(), 2);
    check(host, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, &[0, 1]);
}
