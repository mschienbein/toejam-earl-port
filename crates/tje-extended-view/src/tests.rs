use super::*;
use tje_mem::Revision;

fn put_word(bytes: &mut [u8], address: u32, value: u16) {
    bytes[address as usize..address as usize + 2].copy_from_slice(&value.to_be_bytes());
}

fn fixture(revision: Revision, setup: impl FnOnce(&mut [u8])) -> (AddressSpace, Vec<u8>) {
    // Entirely authored synthetic storage; no game patterns, palette or maps.
    let mut bytes = vec![0; 0x100000];
    setup(&mut bytes);
    let mut memory = AddressSpace::new(bytes, revision);
    ram::g_player1_map_level(revision).write(&mut memory, 2);
    ram::g_player2_map_level(revision).write(&mut memory, 3);
    ram::g_camera1_committed_x(revision).write(&mut memory, 128);
    ram::g_camera1_committed_y(revision).write(&mut memory, 64);
    ram::g_camera2_committed_x(revision).write(&mut memory, 128);
    ram::g_camera2_committed_y(revision).write(&mut memory, 64);
    (memory, vec![0; 0x10000])
}

fn base_address(revision: Revision, id: i32, row: i32, column: i32) -> u32 {
    rom::level_metatile_maps(revision)
        .at(id)
        .at(row)
        .at(column)
        .address()
}

fn fill_metatile(bytes: &mut [u8], revision: Revision, id: i32, tile: u16) {
    for row in 0..8 {
        for column in 0..16 {
            put_word(bytes, base_address(revision, id, row, column), tile);
        }
    }
}

#[test]
fn default_is_faithful_and_wider_requires_explicit_experimental_width() {
    assert_eq!(ViewMode::default(), ViewMode::Faithful);
    assert_eq!(ViewMode::default().width(), 320);
    assert_eq!(ViewMode::Widescreen.width(), 400);
    assert_eq!(ViewMode::Widescreen.margin(), 40);
    for width in [0, 320, 399, 400, 401, 407, 409, 513, u16::MAX] {
        assert!(ExperimentalWidth::new(width).is_err());
    }
    for width in [408, 416, 504, 512] {
        let mode = ViewMode::Experimental(ExperimentalWidth::new(width).unwrap());
        assert_eq!(mode.width(), usize::from(width));
        assert_eq!(mode.margin(), i32::from(width - 320) / 2);
    }
}

#[test]
fn metatile_mirror_and_zero_overlay_follow_the_source_in_both_revisions() {
    for revision in Revision::ALL {
        let (mut memory, vram) = fixture(revision, |bytes| {
            put_word(bytes, base_address(revision, 3, 2, 11), 0x0017);
            let overlay = rom::overlay_metatile_maps(revision).at(0).at(2);
            put_word(bytes, overlay.at(10).address(), 0x1023);
        });
        ram::g_player1_level_map(revision)
            .at(0)
            .at(0)
            .write(&mut memory, 0xd183);
        ram::g_camera_tile_attributes(revision).write(&mut memory, 0x2000);
        let view = WorldView::committed(&memory, Viewport::Primary).unwrap();
        let renderer = MapRenderer::new(&memory, &vram).unwrap();
        // Overlay selector 0x11 is mirrored overlay 1; zero stays zero,
        // allowing mirrored base 3 to show through. Other generation bits ignored.
        assert_eq!(renderer.terrain_tile(view, 16 + 4, 8 + 2), Ok(0x2817));
        assert_eq!(renderer.terrain_tile(view, 16 + 5, 8 + 2), Ok(0x3823));
    }
}

#[test]
fn overlay_replaces_a_whole_tile_word_even_when_its_pixels_are_transparent() {
    let revision = Revision::Rev02;
    let (mut memory, vram) = fixture(revision, |bytes| {
        fill_metatile(bytes, revision, 4, 7);
        put_word(
            bytes,
            rom::overlay_metatile_maps(revision)
                .at(0)
                .at(0)
                .at(0)
                .address(),
            0x1000,
        );
    });
    ram::g_player1_level_map(revision)
        .at(0)
        .at(0)
        .write(&mut memory, 0x0104);
    let view = WorldView::committed(&memory, Viewport::Primary).unwrap();
    let renderer = MapRenderer::new(&memory, &vram).unwrap();
    assert_eq!(renderer.terrain_tile(view, 16, 8), Ok(0x1000));
    assert_eq!(renderer.terrain_tile(view, 17, 8), Ok(7));
}

#[test]
fn exterior_fill_does_not_wrap_map_coordinates() {
    for level in [0, 1, 2, 25, 26] {
        let revision = Revision::Rev00;
        let (mut memory, vram) = fixture(revision, |bytes| {
            fill_metatile(bytes, revision, 82, 0x0009);
            fill_metatile(bytes, revision, 3, 0x0007);
        });
        ram::g_player1_map_level(revision).write(&mut memory, level);
        ram::g_camera_tile_attributes(revision).write(&mut memory, 0x8000);
        ram::g_player1_level_map(revision)
            .at(0)
            .at(0)
            .write(&mut memory, 3);
        let view = WorldView::committed(&memory, Viewport::Primary).unwrap();
        let renderer = MapRenderer::new(&memory, &vram).unwrap();
        assert_eq!(renderer.terrain_tile(view, 16, 8), Ok(0x8007));
        let expected = if level == 1 || level == 26 {
            0x8009
        } else {
            0x8000
        };
        for (x, y) in [(-1, 8), (15, 8), (320, 8), (16, 7), (16, 216), (336, 224)] {
            assert_eq!(renderer.terrain_tile(view, x, y), Ok(expected));
        }
    }
}

#[test]
fn pattern_flips_palette_priority_and_unaligned_columns_are_decoded() {
    for revision in Revision::ALL {
        let (mut memory, mut vram) = fixture(revision, |bytes| {
            fill_metatile(bytes, revision, 3, 0xf801);
        });
        ram::g_player1_level_map(revision)
            .at(0)
            .at(0)
            .write(&mut memory, 3);
        ram::g_camera1_committed_x(revision).write(&mut memory, 131);
        ram::g_camera1_committed_y(revision).write(&mut memory, 66);
        for row in 0..8 {
            for column in 0..8 {
                let color = ((row * 3 + column) % 15 + 1) as u8;
                let byte = &mut vram[32 + row * 4 + column / 2];
                *byte |= if column & 1 == 0 { color << 4 } else { color };
            }
        }
        let view = WorldView::committed(&memory, Viewport::Primary).unwrap();
        let renderer = MapRenderer::new(&memory, &vram).unwrap();
        let mut output = vec![IndexedPixel::default(); 400 * 224];
        renderer
            .render_terrain(view, ViewMode::Widescreen, &mut output)
            .unwrap();
        // The original center begins at x=40. World (131,66) is tile
        // local (3,2), flipped to (4,5); authored test color=(5*3+4)%15+1=5.
        let pixel = output[40];
        assert_eq!(pixel.palette_index(), 3 * 16 + 5);
        assert!(pixel.high_priority());
        assert!(!pixel.transparent());
        // Original top-left map boundary appears at x=37, with sub-tile
        // clipping correctly handled rather than rounded to an 8px boundary.
        assert!(output[36].transparent());
        assert!(!output[37].transparent());
        // VDP foreground name-table bytes are irrelevant to this renderer.
        vram[0xa000..0xd000].fill(0x7f);
        let mut changed_planes = vec![IndexedPixel::default(); output.len()];
        MapRenderer::new(&memory, &vram)
            .unwrap()
            .render_terrain(view, ViewMode::Widescreen, &mut changed_planes)
            .unwrap();
        assert_eq!(changed_planes, output);
    }
}

#[test]
fn split_selects_each_complete_map_and_committed_camera_independently() {
    let revision = Revision::Rev02;
    let (mut memory, mut vram) = fixture(revision, |bytes| {
        fill_metatile(bytes, revision, 3, 1);
        fill_metatile(bytes, revision, 4, 2);
    });
    assert_eq!(
        WorldView::committed(&memory, Viewport::Secondary),
        Err(RenderError::InactiveViewport)
    );
    ram::g_split_screen_enabled(revision).write(&mut memory, 1);
    ram::g_camera1_pending_x(revision).write(&mut memory, 1000);
    ram::g_camera2_pending_x(revision).write(&mut memory, 2000);
    ram::g_player1_level_map(revision)
        .at(0)
        .at(0)
        .write(&mut memory, 3);
    ram::g_player2_level_map(revision)
        .at(0)
        .at(0)
        .write(&mut memory, 4);
    vram[32..64].fill(0x33);
    vram[64..96].fill(0x55);
    let before_ram = memory.ram().to_vec();
    let before_vram = vram.clone();
    let renderer = MapRenderer::new(&memory, &vram).unwrap();
    for (viewport, level, color) in [(Viewport::Primary, 2, 3), (Viewport::Secondary, 3, 5)] {
        let view = WorldView::committed(&memory, viewport).unwrap();
        assert_eq!(view.camera(), (128, 64));
        assert_eq!(view.level(), level);
        assert_eq!(view.height(), 112);
        let mut output = vec![IndexedPixel::default(); 400 * 112];
        renderer
            .render_terrain(view, ViewMode::Widescreen, &mut output)
            .unwrap();
        assert_eq!(output[40].palette_index(), color);
    }
    assert_eq!(memory.ram().as_slice(), before_ram);
    assert_eq!(vram, before_vram);
}

#[test]
fn starfield_repeats_tile_columns_and_scrolls_without_scaling_pixels() {
    let revision = Revision::Rev00;
    let (mut memory, mut vram) = fixture(revision, |bytes| {
        let map = rom::starfield_byte_map(revision);
        bytes[map.at(0).at(0).address() as usize] = 1;
        bytes[map.at(0).at(1).address() as usize] = 2;
    });
    ram::g_camera1_committed_x(revision).write(&mut memory, 0);
    ram::g_camera1_committed_y(revision).write(&mut memory, 0);
    vram[513 * 32..514 * 32].fill(0x33);
    vram[514 * 32..515 * 32].fill(0x55);
    let view = WorldView::committed(&memory, Viewport::Primary).unwrap();
    let mut output = vec![IndexedPixel::default(); 400 * 224];
    MapRenderer::new(&memory, &vram)
        .unwrap()
        .render_background(view, ViewMode::Widescreen, &mut output)
        .unwrap();
    assert_eq!(output[40].palette_index(), 3);
    assert_eq!(output[40 + 8].palette_index(), 3);
    assert_eq!(output[40 + 16].palette_index(), 5);
    ram::g_camera1_committed_x(revision).write(&mut memory, 16);
    let shifted = WorldView::committed(&memory, Viewport::Primary).unwrap();
    MapRenderer::new(&memory, &vram)
        .unwrap()
        .render_background(shifted, ViewMode::Widescreen, &mut output)
        .unwrap();
    assert_eq!(output[40].palette_index(), 3);
    assert_eq!(output[40 + 8].palette_index(), 5);
}

#[test]
fn invalid_storage_and_map_selectors_fail_without_aliasing_other_tables() {
    let revision = Revision::Rev02;
    let (mut memory, vram) = fixture(revision, |_| {});
    assert!(matches!(
        MapRenderer::new(&memory, &[]),
        Err(RenderError::InvalidVramLength(0))
    ));
    let small = AddressSpace::new(vec![0; 8], revision);
    assert!(matches!(
        MapRenderer::new(&small, &vram),
        Err(RenderError::TruncatedRom)
    ));
    ram::g_player1_level_map(revision)
        .at(0)
        .at(0)
        .write(&mut memory, 127);
    let view = WorldView::committed(&memory, Viewport::Primary).unwrap();
    assert_eq!(
        MapRenderer::new(&memory, &vram)
            .unwrap()
            .terrain_tile(view, 16, 8),
        Err(RenderError::InvalidMetatile(127))
    );
    ram::g_player1_level_map(revision)
        .at(0)
        .at(0)
        .write(&mut memory, 0x1000);
    assert_eq!(
        MapRenderer::new(&memory, &vram)
            .unwrap()
            .terrain_tile(view, 16, 8),
        Err(RenderError::InvalidOverlay(16))
    );
    let mut short = [IndexedPixel::default(); 1];
    assert_eq!(
        MapRenderer::new(&memory, &vram).unwrap().render_terrain(
            view,
            ViewMode::Widescreen,
            &mut short
        ),
        Err(RenderError::OutputLength {
            expected: 400 * 224,
            actual: 1
        })
    );
    ram::g_player1_map_level(revision).write(&mut memory, -1);
    assert_eq!(
        WorldView::committed(&memory, Viewport::Primary),
        Err(RenderError::UnloadedLevel(-1))
    );
}
