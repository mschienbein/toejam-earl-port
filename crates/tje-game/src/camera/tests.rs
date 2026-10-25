use super::*;
use tje_mem::AddressSpace;

#[test]
fn row_and_column_fills_keep_counts_flags_signed_height_and_other_camera() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut memory = AddressSpace::new(vec![], revision);
        ram::g_camera_tile_attributes(revision).write(&mut memory, 0x8123);
        ram::g_camera_row_upload_words(revision)
            .at(0)
            .at(0)
            .write(&mut memory, 0x4567);
        clear_camera_tile_row(&mut memory, revision, 1);
        for column in 0..45 {
            assert_eq!(
                ram::g_camera_row_upload_words(revision)
                    .at(1)
                    .at(column)
                    .read(&memory),
                0x8123
            );
        }
        assert_eq!(
            ram::g_camera_row_upload_words(revision)
                .at(0)
                .at(0)
                .read(&memory),
            0x4567
        );
        assert_eq!(
            ram::g_camera_row_upload_pending(revision)
                .at(1)
                .read(&memory),
            1
        );
        assert_eq!(
            ram::g_camera_row_upload_word_count(revision)
                .at(1)
                .read(&memory),
            45
        );
        for height in [-1, 0, 19, 32] {
            ram::g_camera_column_upload_height(revision).write(&mut memory, height);
            for row in 0..32 {
                ram::g_camera_column_upload_words(revision)
                    .at(0)
                    .at(row)
                    .write(&mut memory, 0x4567);
            }
            clear_camera_tile_column(&mut memory, revision, 0);
            for row in 0..32 {
                assert_eq!(
                    ram::g_camera_column_upload_words(revision)
                        .at(0)
                        .at(row)
                        .read(&memory),
                    if row < i32::from(height) {
                        0x8123
                    } else {
                        0x4567
                    },
                );
            }
            assert_eq!(
                ram::g_camera_column_upload_pending(revision)
                    .at(0)
                    .read(&memory),
                1
            );
            assert_eq!(
                ram::g_camera_column_upload_word_count(revision)
                    .at(0)
                    .read(&memory),
                height as i8
            );
        }
    }
}

#[test]
fn metatile_lookup_reads_revision_rom_and_preserves_word_effective_addresses() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let level = rom::level_metatile_maps(revision).address();
        let overlay = rom::overlay_metatile_maps(revision).address();
        let mut bytes = vec![0; 0x100000];
        // Synthetic words, not game assets. Include locations just before each table.
        for (address, value) in [
            (level + 256 + 64 + 6, 0x1234_u16),
            (level + 256 + 64 + 24, 0x4321),
            (level - 32, 0x369a),
            (overlay + 64 + 6, 0x5678),
            (overlay + 64 + 24, 0x7654),
            (overlay - 256, 0x1357),
        ] {
            bytes[address as usize..address as usize + 2].copy_from_slice(&value.to_be_bytes());
        }
        let memory = AddressSpace::new(bytes, revision);
        assert_eq!(get_level_metatile_tile(&memory, revision, 1, 2, 3), 0x1234);
        assert_eq!(
            get_level_metatile_tile(&memory, revision, 0x81, 2, 3),
            0x4b21
        );
        assert_eq!(
            get_level_metatile_tile(&memory, revision, 0x7f01, 2, 3),
            0x1234
        );
        assert_eq!(
            get_level_metatile_tile(&memory, revision, 0, 0x7ff, 0),
            0x369a
        );
        assert_eq!(
            get_overlay_metatile_tile(&memory, revision, 1, 2, 3),
            0x5678
        );
        assert_eq!(
            get_overlay_metatile_tile(&memory, revision, 0x11, 2, 3),
            0x7e54
        );
        assert_eq!(get_overlay_metatile_tile(&memory, revision, 0x11, 0, 0), 0);
        assert_eq!(
            get_overlay_metatile_tile(&memory, revision, 0, 0, 0),
            0x1357
        );
    }
}

#[test]
fn corpus_interrupt_timing_failures_remain_pending_and_unregistered() {
    for id in [
        ids::UPDATE_CAMERA_FOLLOWING,
        ids::MOVE_CAMERA_UP,
        ids::MOVE_CAMERA_DOWN,
        ids::MOVE_CAMERA_LEFT,
        ids::MOVE_CAMERA_RIGHT,
        ids::TRY_UPDATE_CAMERA_X,
        ids::TRY_UPDATE_CAMERA_Y,
    ] {
        assert!(
            PENDING_TRANSLATIONS.iter().any(|entry| entry.id == id)
                || PENDING_FRAMED_TRANSLATIONS
                    .iter()
                    .any(|entry| entry.id == id)
        );
        assert!(!TRANSLATIONS.iter().any(|entry| entry.id == id));
        assert!(
            crate::Dispatcher::hybrid()
                .resolve_entry(id)
                .unwrap()
                .is_none()
        );
        assert_eq!(crate::execution_class(id), Some(ExecutionClass::TimeAware));
    }
}

#[test]
fn argument_taking_camera_candidates_have_only_lazy_framed_pending_bodies() {
    for id in [
        ids::TRY_UPDATE_CAMERA_Y,
        ids::TRY_UPDATE_CAMERA_X,
        ids::MOVE_CAMERA_UP,
        ids::MOVE_CAMERA_DOWN,
        ids::MOVE_CAMERA_LEFT,
        ids::MOVE_CAMERA_RIGHT,
    ] {
        assert!(
            PENDING_FRAMED_TRANSLATIONS
                .iter()
                .any(|entry| entry.id == id)
        );
        assert!(!PENDING_TRANSLATIONS.iter().any(|entry| entry.id == id));
        assert!(!TRANSLATIONS.iter().any(|entry| entry.id == id));
        assert!(id.metadata().abi.argument_count > 0);
    }
}
