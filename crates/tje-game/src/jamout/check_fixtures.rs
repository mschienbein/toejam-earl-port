//! Live-state fixtures for Jam Out leaves; included by core/check_fixtures.rs.
//! RAM inputs are authored boundary cases, never recordings or ROM-derived data.

use tje_game::{FunctionId, ids};
use tje_host::Host;
use tje_mem::{Address, ObjectState, Ptr, RenderFrame, ram, rom};

fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| {
            panic!(
                "live-state fixture {} {arguments:?}: {error}",
                id.metadata().name
            )
        });
}

pub fn fixtures(host: &mut Host) {
    let revision = host.revision();
    for (handles, position, hide) in [
        (
            ram::g_jam_toejam_entities(revision),
            ids::JAM_OUT_POSITION_TOEJAM_COPY,
            ids::JAM_OUT_HIDE_TOEJAM_COPY,
        ),
        (
            ram::g_jam_earl_entities(revision),
            ids::JAM_OUT_POSITION_EARL_COPY,
            ids::JAM_OUT_HIDE_EARL_COPY,
        ),
    ] {
        for slot in [0, 8, -1, i32::MIN] {
            for handle in [0, 6, -1, 513] {
                handles.at(slot).write(&mut host.context(), handle);
                check(host, position, &[slot as u32, 0x12348001, 0xffff7fff]);
                check(host, hide, &[slot as u32]);
            }
        }
    }
    let idle = rom::jam_lead_idle_loop_sequence(revision).address();
    let transition = rom::jam_lead_idle_transition_sequence(revision).address();
    for (handles, lock, set_sequence) in [
        (
            ram::g_jam_toejam_entities(revision),
            ram::g_jam_toejam_animation_locked(revision),
            ids::JAM_OUT_SET_TOEJAM_SEQUENCE,
        ),
        (
            ram::g_jam_earl_entities(revision),
            ram::g_jam_earl_animation_locked(revision),
            ids::JAM_OUT_SET_EARL_SEQUENCE,
        ),
    ] {
        for locked in [-1, 0, 1] {
            lock.write(&mut host.context(), locked);
            for handle in [0, 6, 513] {
                handles.at(0).write(&mut host.context(), handle);
                for sequence in [0, idle, 0x01000000] {
                    for period in [0x8000, 0xffff, 0, 6, 0x7fff] {
                        check(host, set_sequence, &[sequence, period]);
                    }
                }
            }
        }
    }
    for (first, second) in [(0, 0), (6, 7), (6, 6), (513, 7), (-1, 7)] {
        ram::g_jam_toejam_entities(revision)
            .at(0)
            .write(&mut host.context(), first);
        ram::g_jam_earl_entities(revision)
            .at(0)
            .write(&mut host.context(), second);
        for first_sequence in [0, idle, transition] {
            for second_sequence in [0, idle, transition] {
                for duration in [0, 1, -1] {
                    ram::g_jam_toejam_animation_sequence(revision)
                        .write(&mut host.context(), Ptr::new(first_sequence));
                    ram::g_jam_earl_animation_sequence(revision)
                        .write(&mut host.context(), Ptr::new(second_sequence));
                    ram::g_jam_toejam_animation_frame(revision).write(&mut host.context(), 0x1234);
                    ram::g_jam_earl_animation_frame(revision).write(&mut host.context(), -128);
                    ram::g_jam_toejam_animation_locked(revision).write(&mut host.context(), -1);
                    ram::g_jam_earl_animation_locked(revision).write(&mut host.context(), 1);
                    for handle in [first, second] {
                        if handle != 0 {
                            let object = ram::g_player_info(revision)
                                .cast::<ObjectState>()
                                .byte_offset(i32::from(handle.wrapping_shl(7)));
                            object.frame_duration().write(&mut host.context(), duration);
                            object.frame().write(&mut host.context(), -7);
                        }
                    }
                    check(host, ids::JAM_OUT_UPDATE_LEAD_ANIMATIONS, &[]);
                }
            }
        }
    }
    // The live machine is never resumed after these deliberately authored RAM
    // inputs. Clearing the arena prevents unrelated active gameplay entities
    // from introducing uncontrolled parent/frame pointers into this fixture.
    let objects = ram::g_player_info(revision).address();
    for offset in 0..40 * 128 {
        host.context().write_u8(objects + offset, 0);
    }
    let frame = Address::<RenderFrame>::new(0xffe800);
    for owner in [-1, 2, 3] {
        for piece_count in [0, 1, 2, 12, 13, 255] {
            let parent = ram::g_player_info(revision).at(i32::from(owner));
            let object = ram::g_player_info(revision).at(3);
            parent.frame().write(&mut host.context(), -7);
            parent.sprite_ready().write(&mut host.context(), -1);
            parent
                .current_frame()
                .write(&mut host.context(), Ptr::new(frame.address()));
            frame.piece_count().write(&mut host.context(), piece_count);
            object.draw_flags().write(&mut host.context(), 1);
            object.behavior_flags().write(&mut host.context(), 4);
            object.owner().write(&mut host.context(), owner);
            for piece in 0..12 {
                parent
                    .primary_sprites()
                    .at(piece)
                    .write(&mut host.context(), (piece - 1) as i8);
                object
                    .primary_sprites()
                    .at(piece)
                    .write(&mut host.context(), (piece + 12) as i8);
                let sprite = ram::g_hardware_sprite_table(revision).at(piece - 1);
                sprite.size().write(&mut host.context(), (piece + 1) as u8);
                sprite
                    .tile()
                    .write(&mut host.context(), (0x8123 + piece) as u16);
            }
            check(host, ids::JAM_OUT_SYNCHRONIZE_ENTITY_COPIES, &[]);
        }
    }
    for active in [0, 1] {
        for shared in [0, 4] {
            let object = ram::g_player_info(revision).at(3);
            object.draw_flags().write(&mut host.context(), active);
            object.behavior_flags().write(&mut host.context(), shared);
            check(host, ids::JAM_OUT_SYNCHRONIZE_ENTITY_COPIES, &[]);
        }
    }
}
