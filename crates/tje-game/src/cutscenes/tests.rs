use super::*;
use crate::{Dispatcher, Error, FunctionId, ship, speech};
use tje_mem::{AddressSpace, Bus, Revision};

struct MemoryContext {
    memory: AddressSpace,
}

impl MemoryContext {
    fn new(revision: Revision) -> Self {
        Self {
            memory: AddressSpace::new(vec![0; 64], revision),
        }
    }
}

impl Bus for MemoryContext {
    fn read_u8(&self, address: u32) -> u8 {
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
    }
}

impl tje_mem::Platform for MemoryContext {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("leaf accessed VDP");
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("leaf accessed VDP");
    }
}

impl crate::Platform for MemoryContext {
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("leaf accessed device");
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("leaf accessed device");
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("leaf accessed device");
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("leaf accessed device");
    }
}

impl Context for MemoryContext {
    fn revision(&self) -> Revision {
        self.memory.revision()
    }
    fn data_register(&self, _: usize) -> u32 {
        panic!("leaf read residual register");
    }
    fn set_data_register(&mut self, _: usize, _: u32) {
        panic!("leaf wrote register");
    }
    fn address_register(&self, _: usize) -> u32 {
        panic!("leaf read residual register");
    }
    fn set_address_register(&mut self, _: usize, _: u32) {
        panic!("leaf wrote register");
    }
    fn status_register(&self) -> u16 {
        panic!("leaf read status register");
    }
    fn set_status_register(&mut self, _: u16) {
        panic!("leaf wrote status register");
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid()
    }
    fn call_original(&mut self, function: FunctionId, _: &[u32]) -> Result<u32> {
        Err(Error::Untranslated(function))
    }
}

#[test]
fn all_owned_leaf_roots_have_native_closures() {
    let roots: Vec<_> = TRANSLATIONS
        .iter()
        .chain(speech::TRANSLATIONS)
        .chain(ship::TRANSLATIONS)
        .map(|translation| translation.id)
        .collect();
    assert!(Dispatcher::native(&roots).is_ok());
    for root in roots {
        assert!(crate::is_atomic_capable(root));
        assert!(root.metadata().callees.is_empty());
    }
}

#[test]
fn remapping_covers_every_packed_byte_and_signed_count_wrap() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = MemoryContext::new(revision);
        let source = 0xff0100;
        let destination = ram::g_sprite_tile_upload_buffer(revision).address();
        for byte in 0..256_u32 {
            c.write_u8(source + byte, byte as u8);
        }
        cutscene_append_remapped_font_tiles(&mut c, &[source, 8]).unwrap();
        let palette = [0, 0, 0, 0, 0, 0, 0, 0, 15, 2, 2, 1, 0, 0, 0, 0];
        for byte in 0..256_u32 {
            assert_eq!(
                c.read_u8(destination + byte),
                palette[(byte & 15) as usize] | (palette[(byte >> 4) as usize] << 4)
            );
        }
        assert_eq!(ram::g_sprite_tile_buffer_offset(revision).read(&c), 256);
        for (count, offset) in [(1024_i16, 0x8000), (2048, 0), (-1, 0xffe0)] {
            ram::g_sprite_tile_buffer_offset(revision).write(&mut c, 0);
            c.write_u8(destination, 0x6d);
            cutscene_append_remapped_font_tiles(&mut c, &[source, count as i32 as u32]).unwrap();
            assert_eq!(c.read_u8(destination), 0x6d);
            assert_eq!(ram::g_sprite_tile_buffer_offset(revision).read(&c), offset);
        }
        // Forward overlap must consume the previously written destination byte.
        ram::g_sprite_tile_buffer_offset(revision).write(&mut c, 1);
        c.write_u8(destination, 0xbb);
        cutscene_append_remapped_font_tiles(&mut c, &[destination, 1]).unwrap();
        assert_eq!(c.read_u8(destination + 1), 0x11);
        assert_eq!(c.read_u8(destination + 2), 0);
    }
}

#[test]
fn sequence_selectors_are_unchecked_and_periods_narrow() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = MemoryContext::new(revision);
        for (slots, function) in [
            (
                ram::g_intro_large_player_part_entities(revision).cast::<u8>(),
                intro_set_large_player_part_sequence as crate::TranslationFn,
            ),
            (
                ram::g_intro_driving_part_entities(revision).cast::<u8>(),
                intro_set_driving_part_sequence as crate::TranslationFn,
            ),
        ] {
            for index in [-1_i16, 0, 7, 8] {
                slots.offset(i32::from(index)).write(&mut c, 9);
                function(&mut c, &[index as i32 as u32, 0x12345678, 0x180]).unwrap();
                let entity = ram::g_player_info(revision).at(9);
                assert_eq!(entity.sequence().read(&c).address(), 0x12345678);
                assert_eq!(entity.frame_phase().read(&c), -128);
                assert_eq!(entity.frame_duration().read(&c), 1);
                assert_eq!(entity.frame().read(&c), -1);
            }
        }
    }
}

#[test]
fn first_player_frame_query_retains_second_only_handle_bug() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = MemoryContext::new(revision);
        ram::g_player_info(revision)
            .at(0)
            .frame()
            .write(&mut c, -128);
        ram::g_player_info(revision).at(9).frame().write(&mut c, 37);
        assert_eq!(
            ending_get_first_player_animation_frame(&mut c, &[]).unwrap(),
            Some(0)
        );
        ram::g_ending_player2_entity(revision).write(&mut c, 9);
        assert_eq!(
            ending_get_first_player_animation_frame(&mut c, &[]).unwrap(),
            Some(0xffffff80)
        );
        ram::g_ending_player1_entity(revision).write(&mut c, 9);
        assert_eq!(
            ending_get_first_player_animation_frame(&mut c, &[]).unwrap(),
            Some(37)
        );
    }
}

#[test]
fn reveal_and_pair_motion_keep_zero_and_aliased_handles() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = MemoryContext::new(revision);
        for index in 0..10 {
            ram::g_intro_component_entities(revision)
                .at(index)
                .write(&mut c, index as u8);
            ram::g_player_info(revision)
                .at(index)
                .behavior_flags()
                .write(&mut c, 0xff);
        }
        intro_reveal_frame_entities(&mut c, &[]).unwrap();
        for index in 0..10 {
            assert_eq!(
                ram::g_player_info(revision)
                    .at(index)
                    .behavior_flags()
                    .read(&c),
                0xbf
            );
        }
        let ship = ram::g_player_info(revision).at(6);
        let exhaust = ram::g_player_info(revision).at(7);
        ram::g_cutscene_rocketship_entity(revision).write(&mut c, 6);
        ram::g_cutscene_rocketship_exhaust_entity(revision).write(&mut c, 7);
        for (start, distance, expected) in [
            (-60, 1, 320),
            (-59, 1, -60),
            (i16::MIN, 1, i16::MAX),
            (42, 0x10002, 40),
        ] {
            ship.x().write(&mut c, start);
            exhaust.x().write(&mut c, -60);
            cutscene_move_rocketship_pair_left(&mut c, &[distance]).unwrap();
            assert_eq!(ship.x().read(&c), expected);
            assert_eq!(exhaust.x().read(&c), 320);
        }
        ram::g_cutscene_rocketship_exhaust_entity(revision).write(&mut c, 6);
        ship.x().write(&mut c, -59);
        cutscene_move_rocketship_pair_left(&mut c, &[1]).unwrap();
        assert_eq!(ship.x().read(&c), 320);
        cutscene_position_rocketship_pair(&mut c, &[0x12348000, 0xffff7fff]).unwrap();
        assert_eq!(ship.x().read(&c), i16::MIN);
        assert_eq!(ship.y().read(&c), i16::MAX);
    }
}

#[test]
fn composites_preserve_guards_offsets_and_word_indexing() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = MemoryContext::new(revision);
        let entity = ram::g_player_info(revision).at(10);
        let before = c.memory.ram().to_vec();
        ending_set_pose_composite(&mut c, &[0, 1, 2]).unwrap();
        ending_position_six_piece_composite(&mut c, &[1, 2]).unwrap();
        ending_position_right_offset_composite(&mut c, &[1, 2]).unwrap();
        intro_position_falling_rocketship(&mut c, &[1, 2]).unwrap();
        ending_restart_elevated_composite_sequence(&mut c, &[]).unwrap();
        assert_eq!(c.memory.ram().as_slice(), before);
        ram::g_ending_pose_composite_entity(revision).write(&mut c, 10);
        entity.tile().write(&mut c, 0x1234);
        entity.draw_flags().write(&mut c, 0x03);
        ending_set_pose_composite(&mut c, &[0x4000, 0x8000, 0x7fff]).unwrap();
        assert_eq!(entity.x().read(&c), 32763);
        assert_eq!(entity.y().read(&c), 32762);
        assert_eq!(entity.tile().read(&c), 0x9234);
        assert_eq!(entity.draw_flags().read(&c), 0x83);
        assert_eq!(
            entity.sequence().read(&c).address(),
            rom::intro_held_frame_streams(revision)
                .address()
                .wrapping_sub(32768)
        );
        assert_eq!(entity.frame_phase().read(&c), 6);
        ram::g_ending_six_piece_composite_entity(revision).write(&mut c, 10);
        ending_position_six_piece_composite(&mut c, &[0xffff, 0x7fff]).unwrap();
        assert_eq!(entity.x().read(&c), -1);
        assert_eq!(entity.y().read(&c), -32669);
        assert_eq!(entity.height().read(&c), 100);
        ram::g_ending_right_offset_composite_entity(revision).write(&mut c, 10);
        ending_position_right_offset_composite(&mut c, &[0x7fff, 0x8000]).unwrap();
        assert_eq!(entity.x().read(&c), -32719);
        assert_eq!(entity.y().read(&c), i16::MIN);
        ram::g_intro_falling_rocketship_entity(revision).write(&mut c, 10);
        intro_position_falling_rocketship(&mut c, &[0x8000, 0x7fff]).unwrap();
        assert_eq!(entity.x().read(&c), i16::MIN);
        assert_eq!(entity.y().read(&c), i16::MAX);
        ram::g_ending_elevated_three_piece_composite_entity(revision).write(&mut c, 10);
        entity.sprite_ready().write(&mut c, -1);
        ending_restart_elevated_composite_sequence(&mut c, &[]).unwrap();
        assert_eq!(entity.sprite_ready().read(&c), 0);
        assert_eq!(
            entity.sequence().read(&c).address(),
            rom::ending_frame_index_streams(revision).at(249).address()
        );
        assert_eq!(entity.frame().read(&c), -1);
        assert_eq!(entity.frame_duration().read(&c), 1);
        assert_eq!(entity.frame_phase().read(&c), 4);
    }
}

#[test]
fn speech_queries_keep_signed_handles_and_reply_truth_tests() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = MemoryContext::new(revision);
        ram::g_speech_bubble_entities(revision)
            .at(3)
            .write(&mut c, 7);
        for (argument, expected) in [(5, 0), (6, 0), (7, 1), (0xffff, 0), (0x10007, 1)] {
            assert_eq!(
                speech::is_speech_bubble_entity(&mut c, &[argument]).unwrap(),
                Some(expected)
            );
        }
        let bubble = ram::g_player_info(revision).at(7);
        bubble.owner().write(&mut c, -1);
        bubble.draw_flags().write(&mut c, 1);
        assert_eq!(
            speech::has_speaker_speech(&mut c, &[u32::MAX]).unwrap(),
            Some(0)
        );
        bubble.draw_flags().write(&mut c, 8);
        assert_eq!(
            speech::has_speaker_speech(&mut c, &[u32::MAX]).unwrap(),
            Some(1)
        );
        assert_eq!(speech::has_speaker_speech(&mut c, &[255]).unwrap(), Some(0));
        bubble.draw_flags().write(&mut c, 0);
        for slot in 0..2 {
            ram::g_delayed_speech_reply_timers(revision)
                .at(slot)
                .write(&mut c, -1);
            ram::g_delayed_speech_reply_speakers(revision)
                .at(slot)
                .write(&mut c, -128);
            assert_eq!(
                speech::has_speaker_speech(&mut c, &[0xffffff80]).unwrap(),
                Some(1)
            );
            ram::g_delayed_speech_reply_timers(revision)
                .at(slot)
                .write(&mut c, 0);
        }
        ram::g_speech_bubble_entities(revision)
            .at(0)
            .write(&mut c, -1);
        let negative = ram::g_player_info(revision).at(-1);
        negative.draw_flags().write(&mut c, 8);
        negative.owner().write(&mut c, 1);
        assert_eq!(speech::has_speaker_speech(&mut c, &[1]).unwrap(), Some(1));
    }
}

#[test]
fn opaque_fill_covers_all_pixels_without_advancing_offset() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = MemoryContext::new(revision);
        let buffer = ram::g_sprite_tile_upload_buffer(revision).address();
        for byte in 0..256_u32 {
            c.write_u8(buffer + byte, byte as u8);
        }
        ship::fill_sprite_opaque_pixels(&mut c, &[8]).unwrap();
        for byte in 0..256_u32 {
            let expected =
                if byte & 15 != 0 { 15 } else { 0 } | if byte & 240 != 0 { 240 } else { 0 };
            assert_eq!(c.read_u8(buffer + byte), expected);
        }
        assert_eq!(ram::g_sprite_tile_buffer_offset(revision).read(&c), 0);
        for count in [-1_i16, 0, 1024, 2048] {
            c.write_u8(buffer, 0x42);
            ship::fill_sprite_opaque_pixels(&mut c, &[count as i32 as u32]).unwrap();
            assert_eq!(c.read_u8(buffer), 0x42);
        }
    }
}
