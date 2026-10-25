use super::*;
use tje_mem::Ptr;

fn fixture(revision: Revision) -> (AddressSpace, Vec<u8>, [u16; 64], PublishedView) {
    let mut bytes = vec![0; 0x100000];
    bytes[0x100..0x120].fill(0x33);
    bytes[0x120..0x140].fill(0x55);
    bytes[0x140..0x160].fill(0xff);
    bytes[0x200] = 1;
    bytes[0x204] = 1;
    bytes[0x205] = 1;
    bytes[0x209] = 0x40;
    bytes[0x20a..0x20e].copy_from_slice(&0x120u32.to_be_bytes());
    let mut memory = AddressSpace::new(bytes, revision);
    ram::g_player1_map_level(revision).write(&mut memory, 2);
    ram::g_player2_map_level(revision).write(&mut memory, 3);
    let mut state = PublishedView::new(400);
    state.world = true;
    state.published = true;
    state.views[0] = PublishedViewport {
        camera_x: 160,
        camera_y: 64,
        level: 2,
        inventory: false,
    };
    state.views[1] = PublishedViewport {
        camera_x: 160,
        camera_y: 64,
        level: 3,
        inventory: false,
    };
    let mut cram = [0; 64];
    cram[3] = 0x000e;
    cram[5] = 0x00e0;
    (memory, vec![0; 0x10000], cram, state)
}
fn entity(x: i16, y: i16, level: i8, source: u32) -> Entity {
    let mut entity = Entity {
        x,
        y,
        flags: 1,
        level,
        count: 1,
        ..Entity::default()
    };
    entity.pieces[0] = Piece {
        width: 1,
        height: 1,
        format: 0x40,
        source,
        ..Piece::default()
    };
    entity
}
fn original() -> Vec<Color> {
    vec![Color::rgb(1, 2, 3); 320 * 224]
}

#[test]
fn authored_scenes_are_exact_centered_and_world_wings_include_unallocated_sprites() {
    for revision in Revision::ALL {
        let (memory, vram, cram, mut state) = fixture(revision);
        state.entities[0][6] = entity(125, 84, 2, 0x100);
        state.entities[0][7] = entity(482, 84, 2, 0x120);
        let mut presenter = Presenter::new(ViewMode::Widescreen);
        let original = original();
        presenter
            .compose(&memory, &vram, &cram, &original, &state, &[])
            .unwrap();
        let (pixels, size) = presenter.frame();
        assert_eq!(size.width, 400);
        assert_eq!(pixels[20 * 400 + 5], Color::rgb(255, 0, 0));
        assert_eq!(pixels[20 * 400 + 362], Color::rgb(0, 255, 0));
        for y in 0..224 {
            assert_eq!(
                &pixels[y * 400 + 40..y * 400 + 360],
                &original[y * 320..(y + 1) * 320]
            );
        }
        state.world = false;
        presenter
            .compose(&memory, &vram, &cram, &original, &state, &[])
            .unwrap();
        assert_eq!(presenter.frame().0[20 * 400 + 5], Color::BLACK);
        assert_eq!(presenter.frame().0[20 * 400 + 362], Color::BLACK);
    }
}

#[test]
fn split_views_clip_pieces_at_separator_and_select_independent_levels() {
    let (memory, vram, cram, mut state) = fixture(Revision::Rev02);
    state.split = true;
    state.entities[0][6] = entity(125, 172, 2, 0x100);
    state.entities[0][7] = entity(125, 64, 3, 0x120);
    let mut presenter = Presenter::new(ViewMode::Widescreen);
    presenter
        .compose(&memory, &vram, &cram, &original(), &state, &[])
        .unwrap();
    let pixels = presenter.frame().0;
    assert_eq!(pixels[111 * 400 + 5], Color::rgb(255, 0, 0));
    assert_eq!(pixels[112 * 400 + 5], Color::rgb(0, 255, 0));
    assert_eq!(pixels[120 * 400 + 5], Color::BLACK);
    state.views[1].inventory = true;
    presenter
        .compose(&memory, &vram, &cram, &original(), &state, &[])
        .unwrap();
    assert_eq!(presenter.frame().0[112 * 400 + 5], Color::BLACK);
}

#[test]
fn sprite_order_is_independent_of_background_priority_and_shadow_order() {
    let (memory, mut vram, cram, mut state) = fixture(Revision::Rev02);
    state.entities[0][6] = entity(125, 84, 2, 0x100);
    state.entities[0][7] = entity(125, 84, 2, 0x120);
    state.entities[0][7].tile = 0x8000;
    // An opaque high-priority exterior tile hides low priority front sprites;
    // a later high-priority sprite cannot leak through that sprite's pixels.
    let mut memory = memory;
    ram::g_camera_tile_attributes(memory.revision()).write(&mut memory, 0x8001);
    vram[32..64].fill(0x55);
    let mut presenter = Presenter::new(ViewMode::Widescreen);
    presenter
        .compose(&memory, &vram, &cram, &original(), &state, &[])
        .unwrap();
    assert_eq!(presenter.frame().0[20 * 400 + 5], Color::rgb(0, 255, 0));
    state.entities[0][6].tile = 0x8000;
    presenter
        .compose(&memory, &vram, &cram, &original(), &state, &[])
        .unwrap();
    assert_eq!(presenter.frame().0[20 * 400 + 5], Color::rgb(255, 0, 0));
}

#[test]
fn publication_uses_committed_cameras_and_freezes_positions() {
    let revision = Revision::Rev00;
    let (mut memory, _, _, _) = fixture(revision);
    ram::g_v_blank_graphics_enabled(revision).write(&mut memory, 1);
    ram::g_mode2_reg_softcopy(revision).write(&mut memory, 0x40);
    ram::g_camera1_pending_x(revision).write(&mut memory, 999);
    ram::g_camera1_committed_x(revision).write(&mut memory, 160);
    let actor = ram::g_player_info(revision).at(6);
    actor.x().write(&mut memory, 125);
    actor.draw_flags().write(&mut memory, 1);
    let frame = ram::g_sprite_tile_upload_buffer(revision).address() + 7000;
    actor.current_frame().write(&mut memory, Ptr::new(frame));
    Address::<RenderFrame>::new(frame)
        .piece_count()
        .write(&mut memory, 1);
    let mut state = PublishedView::new(400);
    state.world_seen = true;
    state.capture(&memory, revision, true);
    actor.x().write(&mut memory, 500);
    assert_eq!(state.views[0].camera_x, 160);
    assert_eq!(state.entities[0][6].x, 125);
    assert_eq!(state.entities[0][6].count, 1);
    state.world_seen = false;
    state.capture(&memory, revision, false);
    assert!(!state.world);
}

#[test]
fn all_runtime_compression_modes_decode_without_rom_content() {
    let mut rom = vec![0; 128];
    rom[16..48].fill(0x35);
    let raw = Piece {
        width: 1,
        height: 1,
        format: 0x40,
        source: 16,
        ..Piece::default()
    };
    assert_eq!(decode_piece(&rom, raw).unwrap(), vec![0x35; 32]);
    rom[16..18].copy_from_slice(&[0xe0, 0x35]);
    let rle = Piece { format: 0, ..raw };
    assert_eq!(decode_piece(&rom, rle).unwrap(), vec![0x35; 32]);
    rom[16..24].copy_from_slice(&[0xf8, 0x12, 0xf8, 0x34, 0xf8, 0x56, 0xf8, 0x78]);
    let interleaved = Piece {
        format: -128,
        ..raw
    };
    let expected: Vec<u8> = [0x12, 0x34, 0x56, 0x78]
        .into_iter()
        .cycle()
        .take(32)
        .collect();
    assert_eq!(decode_piece(&rom, interleaved).unwrap(), expected);
    rom[16..24].copy_from_slice(&[0x68, 0x12, 0x68, 0x34, 0x68, 0x56, 0x68, 0x78]);
    let decoded = decode_piece(&rom, interleaved).unwrap();
    assert_eq!(
        &decoded[..8],
        &[0x12, 0x34, 0x56, 0x78, 0x21, 0x43, 0x65, 0x87]
    );
    rom[16] = 0;
    assert!(decode_piece(&rom, interleaved).is_err());
}

#[test]
fn native_width_frame_overrides_replace_only_opted_in_world_presentation() {
    let (memory, vram, cram, mut state) = fixture(Revision::Rev02);
    state.width = 320;
    state.entities[0][6] = entity(200, 84, 2, 0x100);
    let mut presenter = Presenter::new(ViewMode::Faithful);
    let replacement = FrameOverride {
        entity: 6,
        frame_address: 0x200,
        horizontal_flip: false,
        palette_index: None,
    };
    presenter
        .compose(&memory, &vram, &cram, &original(), &state, &[replacement])
        .unwrap();
    assert_eq!(presenter.frame().0[20 * 320 + 40], Color::rgb(0, 255, 0));
    state.world = false;
    presenter
        .compose(&memory, &vram, &cram, &original(), &state, &[replacement])
        .unwrap();
    assert_eq!(presenter.frame().0[20 * 320 + 40], Color::rgb(1, 2, 3));
}

#[test]
fn special_shadow_sprite_uses_the_hardware_dac_not_cram_color_sixty_three() {
    let (mut memory, mut vram, cram, mut state) = fixture(Revision::Rev02);
    ram::g_camera_tile_attributes(memory.revision()).write(&mut memory, 0x8001);
    vram[32..64].fill(0x55);
    state.entities[0][6] = entity(125, 84, 2, 0x140);
    state.entities[0][6].tile = 0xe000;
    let mut presenter = Presenter::new(ViewMode::Widescreen);
    presenter.set_shadow_highlight(true);
    presenter
        .compose(&memory, &vram, &cram, &original(), &state, &[])
        .unwrap();
    assert_eq!(presenter.frame().0[20 * 400 + 5], Color::rgb(0, 130, 0));
}

#[test]
fn world_priority_patch_expires_per_native_streamed_tile() {
    let revision = Revision::Rev02;
    let (mut memory, mut vram, cram, mut state) = fixture(revision);
    ram::g_camera_tile_attributes(revision).write(&mut memory, 0x8001);
    vram[32..64].fill(0x55);
    state.entities[0][6] = entity(125, 84, 2, 0x100);
    state.priority_patches[0] = PriorityPatch {
        active: true,
        center_x: 15,
        center_y: 10,
        viewport: 0,
        level: 2,
    };
    let address = 0xa000 + 10 * 128 + 15 * 2;
    vram[address..address + 2].copy_from_slice(&1u16.to_be_bytes());
    let mut presenter = Presenter::new(ViewMode::Widescreen);
    presenter
        .compose(&memory, &vram, &cram, &original(), &state, &[])
        .unwrap();
    assert_eq!(presenter.frame().0[20 * 400 + 5], Color::rgb(255, 0, 0));
    vram[address..address + 2].copy_from_slice(&0x8001u16.to_be_bytes());
    presenter
        .compose(&memory, &vram, &cram, &original(), &state, &[])
        .unwrap();
    assert_eq!(presenter.frame().0[20 * 400 + 5], Color::rgb(0, 255, 0));
}
