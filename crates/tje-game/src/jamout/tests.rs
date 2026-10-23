use super::*;
use crate::core::test_support::{TestContext, Write};
use tje_mem::{RenderFrame, Revision};

#[test]
fn positioning_and_hiding_keep_full_slots_and_word_handle_displacements() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (handles, position, hide) in [
            (
                ram::g_jam_toejam_entities(revision),
                position_toejam_copy as crate::TranslationFn,
                hide_toejam_copy as crate::TranslationFn,
            ),
            (
                ram::g_jam_earl_entities(revision),
                position_earl_copy,
                hide_earl_copy,
            ),
        ] {
            let mut context = TestContext::new(revision);
            position(&mut context, &[0, 10, 20]).unwrap();
            hide(&mut context, &[0]).unwrap();
            assert!(context.writes.is_empty());
            // 513 << 7 wraps as a word to 128, selecting object one.
            handles.at(0).write(&mut context, 513);
            let object = ram::g_player_info(revision).at(1);
            // Full int slot arithmetic wraps 0x80000000 * 2 to zero.
            position(&mut context, &[0x80000000, 0x12348001, 0xffff7fff]).unwrap();
            assert_eq!(object.x().read(&context), -32767);
            assert_eq!(object.y().read(&context), 32767);
            hide(&mut context, &[0]).unwrap();
            assert_eq!(object.x().read(&context), 160);
            assert_eq!(object.y().read(&context), -50);
            handles.at(8).write(&mut context, -1);
            position(&mut context, &[8, 123, 456]).unwrap();
            let before_first = ram::g_player_info(revision).at(-1);
            assert_eq!(before_first.x().read(&context), 123);
            assert_eq!(before_first.y().read(&context), 456);
        }
    }
}

#[test]
fn sequence_locks_and_missing_leads_preserve_cache_behavior() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (handles, lock, cached, set_sequence) in [
            (
                ram::g_jam_toejam_entities(revision),
                ram::g_jam_toejam_animation_locked(revision),
                ram::g_jam_toejam_animation_sequence(revision),
                set_toejam_sequence as crate::TranslationFn,
            ),
            (
                ram::g_jam_earl_entities(revision),
                ram::g_jam_earl_animation_locked(revision),
                ram::g_jam_earl_animation_sequence(revision),
                set_earl_sequence,
            ),
        ] {
            let mut context = TestContext::new(revision);
            lock.write(&mut context, -1);
            context.writes.clear();
            set_sequence(&mut context, &[0x12345678, 3]).unwrap();
            assert!(context.writes.is_empty());
            lock.write(&mut context, 0);
            set_sequence(&mut context, &[0x12345678, 3]).unwrap();
            assert_eq!(cached.read(&context).address(), 0x12345678);
            handles.at(0).write(&mut context, 6);
            set_sequence(&mut context, &[0x01000000, 0x12ff]).unwrap();
            let object = ram::g_player_info(revision).at(6);
            assert_eq!(object.sequence().read(&context).address(), 0x01000000);
            assert_eq!(cached.read(&context).address(), 0x01000000);
            assert_eq!(object.frame_duration().read(&context), 1);
            assert_eq!(object.frame().read(&context), -1);
            assert_eq!(object.frame_phase().read(&context), -1);
        }
    }
}

#[test]
fn lead_updates_keep_overwritten_partner_store_and_signed_frame_caches() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        let transition = Ptr::new(rom::jam_lead_idle_transition_sequence(revision).address());
        let idle = Ptr::new(rom::jam_lead_idle_loop_sequence(revision).address());
        let toejam = ram::g_player_info(revision).at(6);
        let earl = ram::g_player_info(revision).at(7);
        ram::g_jam_toejam_entities(revision)
            .at(0)
            .write(&mut context, 6);
        ram::g_jam_earl_entities(revision)
            .at(0)
            .write(&mut context, 7);
        ram::g_jam_toejam_animation_sequence(revision).write(&mut context, transition);
        ram::g_jam_earl_animation_sequence(revision).write(&mut context, idle);
        ram::g_jam_earl_animation_frame(revision).write(&mut context, 0x1234);
        ram::g_jam_toejam_animation_locked(revision).write(&mut context, 1);
        ram::g_jam_earl_animation_locked(revision).write(&mut context, -1);
        earl.frame_duration().write(&mut context, -1);
        earl.frame().write(&mut context, -128);
        context.writes.clear();
        update_lead_animations(&mut context, &[]).unwrap();
        assert_eq!(toejam.sequence().read(&context), idle);
        assert_eq!(toejam.frame().read(&context), -1);
        assert_eq!(toejam.frame_duration().read(&context), 1);
        assert_eq!(toejam.frame_phase().read(&context), 6);
        assert_eq!(
            ram::g_jam_toejam_animation_frame(revision).read(&context),
            -1
        );
        assert_eq!(
            ram::g_jam_earl_animation_frame(revision).read(&context),
            -128
        );
        assert_eq!(
            ram::g_jam_toejam_animation_locked(revision).read(&context),
            0
        );
        assert_eq!(ram::g_jam_earl_animation_locked(revision).read(&context), 0);
        let frame_writes = context
            .writes
            .iter()
            .filter_map(|event| match event {
                Write::Ram(address, value) if *address == toejam.frame().address() => Some(*value),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(frame_writes, [0x34, 0xff]);
        // Both finish in the same call; Earl observes ToeJam's newly cached idle.
        toejam.frame_duration().write(&mut context, 0);
        earl.frame_duration().write(&mut context, 0);
        ram::g_jam_toejam_animation_sequence(revision).write(&mut context, transition);
        ram::g_jam_earl_animation_sequence(revision).write(&mut context, transition);
        update_lead_animations(&mut context, &[]).unwrap();
        assert_eq!(toejam.sequence().read(&context), idle);
        assert_eq!(earl.sequence().read(&context), idle);
    }
}

#[test]
fn missing_leads_unlock_only_cached_idle_sequences() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        ram::g_jam_toejam_animation_locked(revision).write(&mut context, -1);
        ram::g_jam_earl_animation_locked(revision).write(&mut context, -1);
        let idle = Ptr::new(rom::jam_lead_idle_loop_sequence(revision).address());
        ram::g_jam_toejam_animation_sequence(revision).write(&mut context, idle);
        ram::g_jam_earl_animation_sequence(revision).write(&mut context, Ptr::new(0));
        update_lead_animations(&mut context, &[]).unwrap();
        assert_eq!(
            ram::g_jam_toejam_animation_locked(revision).read(&context),
            0
        );
        assert_eq!(
            ram::g_jam_earl_animation_locked(revision).read(&context),
            -1
        );
    }
}

#[test]
fn shared_copies_keep_signed_indices_and_forward_dependency_order() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        let parent = ram::g_player_info(revision).at(-1);
        let first = ram::g_player_info(revision).at(0);
        let second = ram::g_player_info(revision).at(1);
        let frame = Address::<RenderFrame>::new(0xfff000);
        frame.piece_count().write(&mut context, 2);
        parent
            .current_frame()
            .write(&mut context, Ptr::new(frame.address()));
        parent.frame().write(&mut context, -7);
        parent.sprite_ready().write(&mut context, -1);
        for (object, owner) in [(first, -1), (second, 0)] {
            object.draw_flags().write(&mut context, 1);
            object.behavior_flags().write(&mut context, 4);
            object.owner().write(&mut context, owner);
            object.x().write(&mut context, 1234);
        }
        for piece in 0..2 {
            parent
                .primary_sprites()
                .at(piece)
                .write(&mut context, (piece - 1) as i8);
            first
                .primary_sprites()
                .at(piece)
                .write(&mut context, (piece + 4) as i8);
            second
                .primary_sprites()
                .at(piece)
                .write(&mut context, (piece + 8) as i8);
            let sprite = ram::g_hardware_sprite_table(revision).at(piece - 1);
            sprite.size().write(&mut context, (10 + piece) as u8);
            sprite.tile().write(&mut context, (0x8123 + piece) as u16);
        }
        synchronize_entity_copies(&mut context, &[]).unwrap();
        for object in [first, second] {
            assert_eq!(object.frame().read(&context), -7);
            assert_eq!(object.sprite_ready().read(&context), -1);
            assert_eq!(
                object.current_frame().read(&context).address(),
                frame.address()
            );
            assert_eq!(object.x().read(&context), 1234);
            for piece in 0..2 {
                let index = object.primary_sprites().at(piece).read(&context);
                let sprite = ram::g_hardware_sprite_table(revision).at(i32::from(index));
                assert_eq!(sprite.size().read(&context), (10 + piece) as u8);
                assert_eq!(sprite.tile().read(&context), (0x8123 + piece) as u16);
            }
        }
        // Counts above the twelve-piece C access view intentionally cross into
        // adjacent fields rather than clamping or indexing a host slice.
        frame.piece_count().write(&mut context, 13);
        synchronize_entity_copies(&mut context, &[]).unwrap();
    }
}

#[test]
fn packed_background_reads_rom_and_expands_both_flip_bits() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut bytes = vec![0; 0x100000];
        bytes[rom::crash_packed_tilemap_b(revision).address() as usize] = 0xc0;
        let mut context = TestContext::with_rom(revision, bytes);
        super::layouts::draw_packed_background_block(&mut context, &[0, 0]).unwrap();
        assert_eq!(context.writes.len(), 14 * 22);
        assert_eq!(
            &context.writes[..4],
            &[
                Write::Control(0x6000),
                Write::Control(3),
                Write::Data(0xba33),
                Write::Data(0xa233),
            ]
        );
    }
}
