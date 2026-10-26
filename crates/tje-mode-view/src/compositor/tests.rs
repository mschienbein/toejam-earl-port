use super::*;
use crate::{
    overlay::{Hud, Inventory, InventoryKind, Item, Map, SeatOverlay, Speech},
    packet::{PriorityPatch, Publisher},
};
use tje_four_player::{Character, SeatId};
use tje_simulation::{ActorId, Phase, PublicationStamp};
use tje_world::LevelId;

struct Source {
    stamp: PublicationStamp,
    rom: Vec<u8>,
    high_terrain: bool,
}
impl Source {
    fn new() -> Self {
        let mut rom = vec![0; 512];
        for (index, value) in [0x11, 0x22, 0x33, 0xff, 0xf0, 0x0f, 0x44]
            .into_iter()
            .enumerate()
        {
            rom[32 * (index + 1)..32 * (index + 2)].fill(value);
        }
        Self {
            stamp: stamp(1),
            rom,
            high_terrain: false,
        }
    }
}
impl FrameSource for Source {
    fn stamp(&self) -> PublicationStamp {
        self.stamp
    }
    fn rom(&self) -> &[u8] {
        &self.rom
    }
    fn vram(&self) -> &[u8] {
        &[0; 0x10000]
    }
    fn revision(&self) -> tje_rom::Revision {
        tje_rom::Revision::Rev02
    }
    fn terrain(&self, view: View, output: &mut [IndexedPixel]) -> Result<()> {
        let View::World { level, .. } = view else {
            unreachable!()
        };
        output.fill(IndexedPixel::new(
            8 + level.get() as u8,
            0,
            self.high_terrain,
        ));
        Ok(())
    }
    fn color(&self, index: usize, hero: Option<Character>, _modifier: i8) -> Color {
        let shift = hero.map_or(0, |hero| match hero.palette {
            tje_four_player::PaletteVariant::Original => 0,
            tje_four_player::PaletteVariant::RotateForward => 70,
            tje_four_player::PaletteVariant::RotateBackward => 140,
        });
        Color::rgb(index as u8 * 3, shift, index as u8)
    }
    fn map_color(&self, level: LevelId, _position: [i32; 2]) -> Result<Color> {
        Ok(Color::rgb(level.get() as u8, 64, 32))
    }
}
fn stamp(epoch: u64) -> PublicationStamp {
    PublicationStamp {
        hardware_frame: epoch,
        phase: Phase::PresentationA,
        epoch,
    }
}
fn views(disjoint: bool) -> [View; 4] {
    std::array::from_fn(|seat| View::World {
        seat: SeatId::ALL[seat],
        level: LevelId::new(if disjoint { seat as u16 + 2 } else { 2 }).unwrap(),
        origin: [0, 0],
        attributes: 0,
        starfield: true,
    })
}
fn one_piece(piece: Piece) -> crate::packet::Metasprite {
    crate::packet::Metasprite {
        count: 1,
        pieces: [piece; 12],
    }
}
fn actor(slot: u16, level: u16, x: i32, y: i32, source: u32) -> Actor {
    let mut pieces = [Piece::default(); 12];
    pieces[0] = Piece {
        width: 1,
        height: 1,
        source,
        format: 64,
        ..Piece::default()
    };
    Actor {
        id: crate::packet::DrawId::Actor(ActorId::new(slot, 1).unwrap()),
        level: LevelId::new(level).unwrap(),
        position: [x, y],
        elevation: 0,
        draw_late: false,
        flip_horizontal: false,
        flip_vertical: false,
        high_priority: false,
        palette_bank: 0,
        character: None,
        seat: None,
        mask_pixels: false,
        submerged: false,
        count: 1,
        pieces,
        shadow: None,
    }
}
fn overlays() -> [SeatOverlay; 4] {
    std::array::from_fn(|_| SeatOverlay::default())
}
fn pixel(compositor: &Compositor, view: usize, x: usize, y: usize) -> Color {
    compositor.frame().0[(view / 2 * HEIGHT + y) * CANVAS_WIDTH + view % 2 * WIDTH + x]
}

#[test]
fn four_dense_and_disjoint_views_exceed_sat_and_scanline_caps_without_dropout() {
    for disjoint in [false, true] {
        let source = Source::new();
        let mut actors = Vec::new();
        for seat in 0..if disjoint { 4 } else { 1 } {
            for index in 0..256 {
                // All 256 actors lie on four scanlines: no 20/80-sprite limit.
                actors.push(actor(
                    (seat * 256 + index) as u16,
                    seat as u16 + 2,
                    (index % 48) * 8,
                    (index / 48) * 8,
                    32,
                ));
            }
        }
        let mut publisher = Publisher::new();
        publisher
            .publish(stamp(1), views(disjoint), &actors, &overlays(), &[])
            .unwrap();
        let mut renderer = Compositor::new();
        renderer.compose(publisher.current(), &source).unwrap();
        assert_eq!(
            renderer.frame().1,
            FrameSize {
                width: 800,
                height: 448
            }
        );
        for view in 0..4 {
            for index in 0..256 {
                assert_eq!(
                    pixel(&renderer, view, index % 48 * 8, index / 48 * 8),
                    source.color(1, None, 0)
                );
            }
        }
        let before = renderer.frame().0.to_vec();
        renderer.evict_resources();
        renderer.compose(publisher.current(), &source).unwrap();
        assert_eq!(renderer.frame().0, before);
    }
}
#[test]
fn descending_ground_y_draw_late_stable_ids_and_terrain_priority_are_independent() {
    let mut source = Source::new();
    let mut publisher = Publisher::new();
    let mut renderer = Compositor::new();
    let early = actor(10, 2, 10, 20, 32);
    let mut later = actor(1, 2, 10, 19, 64);
    later.high_priority = true;
    for kind in 0..4 {
        let mut first = early.clone();
        let second = later.clone();
        match kind {
            1 => {
                first.draw_late = true;
            }
            2 => {
                first.position[1] = 19;
            }
            3 => {
                source.high_terrain = true;
            }
            _ => {}
        }
        source.stamp = stamp(kind + 1);
        publisher
            .publish(
                source.stamp,
                views(false),
                &[second, first],
                &overlays(),
                &[],
            )
            .unwrap();
        renderer.compose(publisher.current(), &source).unwrap();
        let expected = match kind {
            1 | 2 => source.color(2, None, 0),
            3 => source.color(10, None, 0),
            _ => source.color(1, None, 0),
        };
        assert_eq!(pixel(&renderer, 0, 10, 20), expected);
    }
}
#[test]
fn masks_flips_submerged_cutoff_shadows_and_each_quadrant_clip_are_explicit() {
    let source = Source::new();
    let mut publisher = Publisher::new();
    let mut renderer = Compositor::new();
    let mut body = actor(1, 2, -4, 10, 96);
    body.shadow = Some(crate::packet::Shadow::Rom(one_piece(Piece {
        source: 64,
        y: 8,
        ..body.pieces[0]
    })));
    let mut masked = actor(2, 2, 20, 10, 128);
    masked.mask_pixels = true;
    let mut submerged = actor(3, 2, 40, 10, 32);
    submerged.elevation = -1;
    submerged.submerged = true;
    submerged.pieces[0].y = -4;
    let mut edge = actor(4, 2, 398, 222, 32);
    edge.flip_horizontal = true;
    publisher
        .publish(
            stamp(1),
            views(true),
            &[body, masked, submerged, edge],
            &overlays(),
            &[],
        )
        .unwrap();
    renderer.compose(publisher.current(), &source).unwrap();
    assert_eq!(pixel(&renderer, 0, 0, 10), source.color(3, None, 0));
    assert_eq!(pixel(&renderer, 0, 0, 18), source.color(2, None, 0));
    assert_eq!(pixel(&renderer, 0, 20, 10), source.color(10, None, 0));
    assert_eq!(pixel(&renderer, 0, 40, 7), source.color(1, None, 0));
    assert_eq!(pixel(&renderer, 0, 40, 14), source.color(10, None, 0));
    assert_eq!(pixel(&renderer, 0, 399, 223), source.color(1, None, 0));
    assert_eq!(pixel(&renderer, 1, 0, 223), source.color(11, None, 0));
    assert_eq!(pixel(&renderer, 2, 399, 0), source.color(12, None, 0));
}
#[test]
fn publication_rejects_partial_stale_and_duplicate_packets_without_changing_output() {
    let mut source = Source::new();
    let mut publisher = Publisher::new();
    let mut renderer = Compositor::new();
    let body = actor(1, 2, 10, 10, 32);
    publisher
        .publish(
            stamp(1),
            views(false),
            std::slice::from_ref(&body),
            &overlays(),
            &[],
        )
        .unwrap();
    let before = publisher.current().clone();
    renderer.compose(&before, &source).unwrap();
    let pixels = renderer.frame().0.to_vec();
    assert!(
        publisher
            .publish(stamp(1), views(false), &[], &overlays(), &[])
            .is_err()
    );
    assert!(
        publisher
            .publish(
                stamp(2),
                views(false),
                &[body.clone(), body],
                &overlays(),
                &[]
            )
            .is_err()
    );
    assert_eq!(publisher.current(), &before);
    source.stamp = stamp(2);
    assert!(renderer.compose(&before, &source).is_err());
    assert_eq!(renderer.frame().0, pixels);
    publisher.invalidate();
    publisher
        .publish(stamp(1), views(false), &[], &overlays(), &[])
        .unwrap();
}
#[test]
fn per_instance_palettes_four_overlays_and_level_keyed_priority_patches_are_local() {
    let mut source = Source::new();
    source.high_terrain = true;
    let mut actors = Vec::new();
    let mut ui = overlays();
    for (seat, overlay) in ui.iter_mut().enumerate() {
        let mut body = actor(seat as u16, seat as u16 + 2, 10, 10, 32);
        body.character = Some(Character::DEFAULT_ROSTER[seat]);
        actors.push(body);
        overlay.hud = Some(Hud {
            seat: SeatId::ALL[seat],
            level: LevelId::new(seat as u16 + 2).unwrap(),
            health: 12,
            maximum_health: 24,
            lives: 3,
            rank: 0,
            bucks: 9,
            pause_requested: seat == 3,
        });
    }
    ui[0].inventory = Some(Inventory {
        kind: InventoryKind::Inventory,
        selected: 15,
        slots: [Some(Item {
            wrapper: 2,
            identified: true,
            icon: None,
            price: None,
            available: true,
        }); 16],
    });
    ui[1].map = Some(Map {
        level: LevelId::new(3).unwrap(),
        dimensions: [19, 26],
        discovery: [u64::MAX; 9],
        discovery_columns: 8,
        discovery_rows: 7,
        markers: [None; 68],
    });
    ui[2].speech = Speech::new(b"NEWLY AUTHORED TEST", true, true).unwrap();
    let patch = PriorityPatch {
        level: LevelId::new(5).unwrap(),
        minimum: [1, 1],
        maximum: [1, 1],
    };
    let mut publisher = Publisher::new();
    publisher
        .publish(stamp(1), views(true), &actors, &ui, &[patch])
        .unwrap();
    let mut renderer = Compositor::new();
    renderer.compose(publisher.current(), &source).unwrap();
    assert_eq!(
        pixel(&renderer, 3, 10, 10),
        source.color(1, actors[3].character, 0)
    );
    assert_eq!(pixel(&renderer, 2, 10, 10), source.color(12, None, 0));
    assert_ne!(pixel(&renderer, 0, 20, 40), pixel(&renderer, 1, 20, 40));
    for view in 0..4 {
        assert_eq!(pixel(&renderer, view, 1, 199), Color::rgb(8, 12, 20));
    }
}

#[test]
fn shadows_keep_their_own_priority_bank_and_orientation() {
    let mut source = Source::new();
    source.high_terrain = true;
    let mut body = actor(1, 2, 10, 10, 32);
    body.palette_bank = 2;
    body.character = Some(Character::DEFAULT_ROSTER[2]);
    body.flip_horizontal = true;
    body.mask_pixels = true;
    body.shadow = Some(crate::packet::Shadow::Rom(one_piece(Piece {
        width: 1,
        height: 1,
        source: 64,
        format: 67,
        y: 8,
        flipped_x: 20,
        ..Piece::default()
    })));
    let mut publisher = Publisher::new();
    publisher
        .publish(stamp(1), views(false), &[body], &overlays(), &[])
        .unwrap();
    let mut renderer = Compositor::new();
    renderer.compose(publisher.current(), &source).unwrap();
    assert_eq!(pixel(&renderer, 0, 10, 18), source.color(50, None, 0));
    assert_eq!(pixel(&renderer, 0, 30, 18), source.color(10, None, 0));
}

#[test]
fn static_ids_do_not_consume_or_alias_simulation_body_slots() {
    use crate::packet::DrawId;
    let mut actors = Vec::new();
    for level in 2..6 {
        for id in 0..128 {
            let mut item = actor(0, level, 10, 10, 32);
            item.id = DrawId::Item {
                level: item.level,
                id,
            };
            actors.push(item);
        }
    }
    let mut hero = actor(0, 2, 10, 10, 64);
    actors.push(hero.clone());
    let mut publisher = Publisher::new();
    publisher
        .publish(stamp(1), views(true), &actors, &overlays(), &[])
        .unwrap();
    let saved = publisher.current().clone();
    actors.push(actors[0].clone());
    assert!(
        publisher
            .publish(stamp(2), views(true), &actors, &overlays(), &[])
            .is_err()
    );
    assert_eq!(publisher.current(), &saved);
    actors.pop();
    hero.id = DrawId::Actor(ActorId::new(0, 2).unwrap());
    actors.push(hero);
    assert!(
        publisher
            .publish(stamp(2), views(true), &actors, &overlays(), &[])
            .is_err()
    );
}

#[test]
fn static_vram_shadow_uses_its_own_tile_attributes_and_clips() {
    let mut vram = vec![0; 0x10000];
    vram[32..64].fill(0x22);
    let body = actor(1, 2, -3, 10, 32);
    let mut renderer = Compositor::new();
    renderer
        .paint_vram_shadow(
            &vram,
            &body,
            crate::packet::VramShadow {
                tile: 0xe001,
                size: [1, 1],
                offset: [0, 0],
            },
            [0, 0],
        )
        .unwrap();
    assert_eq!(renderer.sprites[10 * WIDTH].index, 50);
    assert!(renderer.sprites[10 * WIDTH].high);
    assert!(!renderer.sprites[10 * WIDTH + 5].occupied);
    assert!(
        renderer
            .paint_vram_shadow(
                &[],
                &body,
                crate::packet::VramShadow {
                    tile: 1,
                    size: [1, 1],
                    offset: [0, 0],
                },
                [0, 0]
            )
            .is_err()
    );
}
