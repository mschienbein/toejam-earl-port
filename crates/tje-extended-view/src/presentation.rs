//! Published-state host composition. No authored pixels or palette data live here.
use crate::resources::{decode_piece, palette_color};
use crate::{
    FULL_HEIGHT, IndexedPixel, MapRenderer, NATIVE_WIDTH, RenderError, ViewMode, Viewport,
    WorldView,
};
use jgenesis_common::frontend::{Color, FrameSize};
use std::collections::{BTreeMap, btree_map::Entry};
use tje_mem::{Address, AddressSpace, Bus, RenderFrame, RenderPiece, Revision, ram, rom};

#[derive(Debug, Clone, Copy)]
pub struct FrameOverride {
    pub entity: u8,
    pub frame_address: u32,
    pub horizontal_flip: bool,
    pub palette_index: Option<u8>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "enhanced-mode", derive(bincode::Encode, bincode::Decode))]
pub struct Piece {
    pub width: u8,
    pub height: u8,
    pub x: i8,
    pub y: i8,
    pub flipped_x: i8,
    pub format: i8,
    pub source: u32,
}
impl Piece {
    fn read(bus: &(impl Bus + ?Sized), address: Address<RenderPiece>) -> Self {
        Self {
            width: address.width().read(bus),
            height: address.height().read(bus),
            x: address.x().read(bus),
            y: address.y().read(bus),
            flipped_x: address.flipped_x().read(bus),
            format: address.tile_format().read(bus),
            source: address.tile_data().read(bus).address(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "enhanced-mode", derive(bincode::Encode, bincode::Decode))]
pub struct Entity {
    pub x: i16,
    pub y: i16,
    pub elevation: i16,
    pub tile: u16,
    pub flags: u8,
    pub behavior: u8,
    pub level: i8,
    pub owner: i8,
    pub animation_group: u32,
    pub state: i8,
    pub ready: bool,
    pub count: u8,
    pub pieces: [Piece; 12],
    pub primary_tiles: [u16; 12],
    pub secondary_tiles: [u16; 12],
}

#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "enhanced-mode", derive(bincode::Encode, bincode::Decode))]
pub struct HudSprite {
    pub x: i16,
    pub y: i16,
    pub tile: u16,
    pub size: u8,
}

#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "enhanced-mode", derive(bincode::Encode, bincode::Decode))]
pub struct PublishedViewport {
    pub camera_x: i16,
    pub camera_y: i16,
    pub level: i8,
    pub inventory: bool,
}

#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "enhanced-mode", derive(bincode::Encode, bincode::Decode))]
pub struct PriorityPatch {
    pub active: bool,
    pub center_x: i16,
    pub center_y: i16,
    pub viewport: u8,
    pub level: i8,
}

pub struct Publication {
    pub world: bool,
    pub split: bool,
    pub views: [PublishedViewport; 2],
    pub hud: [HudSprite; 12],
}

/// This state is captured by the enhanced publication callback, not reconstructed
/// from next-frame positions. The initial width is part of the save identity.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "enhanced-mode", derive(bincode::Encode, bincode::Decode))]
pub struct PublishedView {
    pub width: u16,
    #[cfg(feature = "enhanced-mode")]
    pub(crate) width_identity: [u8; 2],
    pub world_seen: bool,
    pub world: bool,
    pub published: bool,
    pub split: bool,
    pub views: [PublishedViewport; 2],
    pub entities: [[Entity; 40]; 2],
    pub published_index: usize,
    pub pending_ready: bool,
    pub hud: [HudSprite; 12],
    pub priority_patches: [PriorityPatch; 2],
}
impl PublishedView {
    pub fn new(width: u16) -> Self {
        Self {
            width,
            #[cfg(feature = "enhanced-mode")]
            width_identity: width.to_le_bytes(),
            world_seen: false,
            world: false,
            published: false,
            split: false,
            views: [PublishedViewport::default(); 2],
            entities: [[Entity::default(); 40]; 2],
            published_index: 0,
            pending_ready: false,
            hud: [HudSprite::default(); 12],
            priority_patches: [PriorityPatch::default(); 2],
        }
    }
    pub fn capture(&mut self, bus: &(impl Bus + ?Sized), revision: Revision, sprites: bool) {
        let publication = Self::publication(bus, revision, self.world_seen);
        self.world = publication.world;
        if !self.world {
            return;
        }
        self.split = publication.split;
        self.views = publication.views;
        self.hud = publication.hud;
        if sprites {
            self.entities[self.published_index] = Self::capture_entities(bus, revision);
            self.published = true;
        }
    }
    pub fn publication(
        bus: &(impl Bus + ?Sized),
        revision: Revision,
        world_seen: bool,
    ) -> Publication {
        let world = world_seen
            && ram::g_v_blank_func(revision).read(bus).address() == 0
            && ram::g_v_blank_graphics_enabled(revision).read(bus) != 0
            && ram::g_mode2_reg_softcopy(revision).read(bus) & 0x40 != 0
            && (0..26).contains(&ram::g_player1_map_level(revision).read(bus));
        let split = ram::g_split_screen_enabled(revision).read(bus) != 0;
        let mut views = [
            PublishedViewport {
                camera_x: ram::g_camera1_committed_x(revision).read(bus),
                camera_y: ram::g_camera1_committed_y(revision).read(bus),
                level: ram::g_player1_map_level(revision).read(bus),
                inventory: ram::g_player_inventory_open(revision).at(0).read(bus) != 0,
            },
            PublishedViewport {
                camera_x: ram::g_camera2_committed_x(revision).read(bus),
                camera_y: ram::g_camera2_committed_y(revision).read(bus),
                level: ram::g_player2_map_level(revision).read(bus),
                inventory: ram::g_player2_inventory_open(revision).read(bus) != 0,
            },
        ];
        if !split && views[1].inventory {
            views[0].inventory = true;
        }
        let hardware = ram::g_hardware_sprite_table(revision);
        let mut hud = [HudSprite::default(); 12];
        for (index, target) in hud.iter_mut().enumerate() {
            let sprite = hardware.at(index as i32 + 11);
            *target = HudSprite {
                x: sprite.x().read(bus),
                y: sprite.y().read(bus),
                tile: sprite.tile().read(bus),
                size: sprite.size().read(bus),
            };
        }
        Publication {
            world,
            split,
            views,
            hud,
        }
    }
    pub fn capture_entities(bus: &(impl Bus + ?Sized), revision: Revision) -> [Entity; 40] {
        let hardware = ram::g_hardware_sprite_table(revision);
        let mut entities = [Entity::default(); 40];
        for (index, target) in entities.iter_mut().enumerate() {
            let entity = ram::g_player_info(revision).at(index as i32);
            let frame = entity.current_frame().read(bus).target();
            let count = if frame.address() == 0 {
                0
            } else {
                frame.piece_count().read(bus).min(12)
            };
            *target = Entity {
                x: entity.x().read(bus),
                y: entity.y().read(bus),
                elevation: entity.height().read(bus),
                tile: entity.tile().read(bus),
                flags: entity.draw_flags().read(bus),
                behavior: entity.behavior_flags().read(bus),
                level: entity.level().read(bus),
                owner: entity.owner().read(bus),
                animation_group: entity.animation_group_mask().read(bus) as u32,
                state: entity.sprite_state().read(bus),
                ready: entity.sprite_ready().read(bus) != 0,
                count,
                ..Entity::default()
            };
            for number in 0..usize::from(count) {
                target.pieces[number] = Piece::read(bus, frame.piece(number as i32));
                let primary = entity.primary_sprites().at(number as i32).read(bus);
                let secondary = entity.secondary_sprites().at(number as i32).read(bus);
                if primary > 0 {
                    target.primary_tiles[number] = hardware.at(i32::from(primary)).tile().read(bus);
                }
                if secondary > 0 {
                    target.secondary_tiles[number] =
                        hardware.at(i32::from(secondary)).tile().read(bus);
                }
            }
        }
        entities
    }
}

/// Reusable frame/layer buffers. Atlas entries are lazily decoded only from the
/// user's ROM; RAM-authored speech pieces use their published live VRAM tiles.
/// Additional columns do not allocate native SAT slots or consume its tile budget.
pub struct Presenter {
    mode: ViewMode,
    pixels: Vec<Color>,
    terrain: Vec<IndexedPixel>,
    sprites: Vec<IndexedPixel>,
    occupied: Vec<bool>,
    atlas: BTreeMap<(u32, i8, u8, u8), Box<[u8]>>,
    shadow_highlight: bool,
}
impl Presenter {
    pub fn new(mode: ViewMode) -> Self {
        let length = mode.width() * FULL_HEIGHT;
        Self {
            mode,
            pixels: vec![Color::BLACK; length],
            terrain: vec![IndexedPixel::default(); length],
            sprites: vec![IndexedPixel::default(); length],
            occupied: vec![false; length],
            atlas: BTreeMap::new(),
            shadow_highlight: false,
        }
    }
    pub fn frame(&self) -> (&[Color], FrameSize) {
        (
            &self.pixels,
            FrameSize {
                width: self.mode.width() as u32,
                height: FULL_HEIGHT as u32,
            },
        )
    }
    pub fn set_shadow_highlight(&mut self, enabled: bool) {
        self.shadow_highlight = enabled;
    }
    pub fn compose(
        &mut self,
        memory: &AddressSpace,
        vram: &[u8],
        cram: &[u16],
        original: &[Color],
        state: &PublishedView,
        overrides: &[FrameOverride],
    ) -> Result<(), RenderError> {
        if original.len() < NATIVE_WIDTH * FULL_HEIGHT {
            return Err(RenderError::OutputLength {
                expected: NATIVE_WIDTH * FULL_HEIGHT,
                actual: original.len(),
            });
        }
        self.pixels.fill(Color::BLACK);
        if state.world && state.published {
            self.render_world(memory, vram, cram, state, overrides)?;
        }
        // Authored scenes and inventory halves keep their exact authored 320px
        // image. Ordinary widescreen also retains the hardware-rendered center,
        // including raster timing, speech and falling-priority patches.
        let width = self.mode.width();
        let margin = self.mode.margin() as usize;
        for y in 0..FULL_HEIGHT {
            let viewport = usize::from(state.split && y >= 112);
            if overrides.is_empty()
                || !state.world
                || !state.published
                || state.views[viewport].inventory
                || (!state.split && y >= 184)
            {
                self.pixels[y * width + margin..y * width + margin + NATIVE_WIDTH]
                    .copy_from_slice(&original[y * NATIVE_WIDTH..(y + 1) * NATIVE_WIDTH]);
            }
        }
        Ok(())
    }
    fn render_world(
        &mut self,
        memory: &AddressSpace,
        vram: &[u8],
        cram: &[u16],
        state: &PublishedView,
        overrides: &[FrameOverride],
    ) -> Result<(), RenderError> {
        let width = self.mode.width();
        let height = if state.split { 112 } else { 224 };
        self.sprites.fill(IndexedPixel::default());
        self.occupied.fill(false);
        let entities = &state.entities[state.published_index];
        let renderer = MapRenderer::new(memory, vram)?;
        for viewport in 0..if state.split { 2 } else { 1 } {
            let view = state.views[viewport];
            if view.inventory {
                continue;
            }
            let world = WorldView {
                viewport: if viewport == 0 {
                    Viewport::Primary
                } else {
                    Viewport::Secondary
                },
                camera_x: view.camera_x,
                camera_y: view.camera_y,
                level: view.level,
                height,
            };
            let range = viewport * height * width..(viewport + 1) * height * width;
            renderer.render_background(world, self.mode, &mut self.terrain[range.clone()])?;
            for patch in state.priority_patches {
                if !patch.active
                    || usize::from(patch.viewport) != viewport
                    || patch.level != view.level
                {
                    continue;
                }
                for column in i32::from(patch.center_x) - 4..=i32::from(patch.center_x) + 5 {
                    for row in i32::from(patch.center_y) - 6..=i32::from(patch.center_y) + 6 {
                        // The native patch stops below the player's row at the
                        // first supported tile, independently in every column.
                        let tile = renderer.terrain_tile(world, column, row)?;
                        if row > i32::from(patch.center_y)
                            && rom::player_tile_property_lookup(memory.revision())
                                .at(i32::from(tile & 0x07ff))
                                .read(memory)
                                < 0x90
                        {
                            break;
                        }
                        // Only honor a world-keyed patch while its actual native
                        // ring slot still holds that tile with priority cleared.
                        // Camera streaming naturally expires individual cells.
                        let address = (if viewport == 0 { 0xa000 } else { 0xc000 })
                            + row.rem_euclid(32) as usize * 128
                            + column.rem_euclid(64) as usize * 2;
                        let live = u16::from_be_bytes([vram[address], vram[address + 1]]);
                        if live & 0x8000 != 0 || live & 0x7fff != tile & 0x7fff {
                            continue;
                        }
                        let x = column * 8 - i32::from(view.camera_x) + self.mode.margin();
                        let y = row * 8 - i32::from(view.camera_y);
                        for sy in y.max(0)..(y + 8).min(height as i32) {
                            for sx in x.max(0)..(x + 8).min(width as i32) {
                                self.terrain
                                    [(viewport * height + sy as usize) * width + sx as usize]
                                    .0 &= !0xc0;
                            }
                        }
                    }
                }
            }
            // HUD slots precede world pieces in the original SAT chain.
            if state.split {
                for hud in &state.hud {
                    self.paint_hardware(vram, *hud, viewport, height);
                }
            }
            let mut order: [usize; 40] = std::array::from_fn(|i| i);
            order.sort_unstable_by_key(|&i| {
                (
                    entities[i].flags & 0x80 != 0,
                    std::cmp::Reverse(entities[i].y),
                    i,
                )
            });
            for &index in &order {
                let mut entity = entities[index];
                if entity.flags & 1 == 0
                    || entity.behavior & 0x40 != 0
                    || entity.level != view.level
                    || !(-224..=224).contains(&entity.elevation)
                    || index >= 26
                {
                    continue;
                }
                if let Some(replacement) = overrides.iter().find(|o| usize::from(o.entity) == index)
                {
                    entity.ready = false;
                    let frame = Address::<RenderFrame>::new(replacement.frame_address);
                    entity.count = frame.piece_count().read(memory).min(12);
                    entity.tile = (entity.tile & !0x0800)
                        | if replacement.horizontal_flip {
                            0x0800
                        } else {
                            0
                        };
                    if replacement.palette_index.is_some() {
                        entity.tile &= !0x6000;
                    }
                    for number in 0..usize::from(entity.count) {
                        entity.pieces[number] = Piece::read(memory, frame.piece(number as i32));
                        if let Some(palette) = replacement.palette_index {
                            entity.pieces[number].format =
                                (entity.pieces[number].format & !3) | (palette & 3) as i8;
                        }
                    }
                }
                self.paint_entity(memory, vram, entity, index, world)?;
            }
            // Shadows come after all world pieces, just like BuildViewportSpriteLinks.
            for &index in &order {
                let entity = entities[index];
                if entity.flags & 0x41 != 0x41
                    || entity.behavior & 0x40 != 0
                    || entity.level != view.level
                    || index >= 26
                {
                    continue;
                }
                self.paint_shadow(memory, vram, entity, viewport, height, view);
            }
            for pixel_index in range {
                let background = self.terrain[pixel_index];
                let sprite = self.sprites[pixel_index];
                let visible_sprite = self.occupied[pixel_index]
                    && (sprite.high_priority()
                        || !background.high_priority()
                        || background.transparent());
                let mut modifier = if self.shadow_highlight && background.0 & 0xc0 == 0 {
                    -1
                } else {
                    0
                };
                let mut pixel = background;
                if visible_sprite {
                    if self.shadow_highlight && sprite.palette_index() == 62 {
                        modifier = if modifier == -1 { 0 } else { 1 };
                    } else if self.shadow_highlight && sprite.palette_index() == 63 {
                        modifier = -1;
                    } else {
                        pixel = sprite;
                        if self.shadow_highlight
                            && (sprite.high_priority() || sprite.palette_index() & 15 == 14)
                        {
                            modifier = 0;
                        }
                    }
                }
                self.pixels[pixel_index] = palette_color(
                    cram,
                    if pixel.transparent() {
                        0
                    } else {
                        pixel.palette_index()
                    },
                    modifier,
                );
            }
        }
        // Preserve the native combined panel, extending only its neutral surround.
        if !state.split {
            for y in 184..FULL_HEIGHT {
                self.pixels[y * width..(y + 1) * width].fill(Color::BLACK);
            }
        }
        Ok(())
    }
    fn paint_entity(
        &mut self,
        memory: &AddressSpace,
        vram: &[u8],
        entity: Entity,
        index: usize,
        view: WorldView,
    ) -> Result<(), RenderError> {
        let viewport = usize::from(view.viewport == Viewport::Secondary);
        let height = view.height;
        let mirror = entity.tile & 0x0800 != 0;
        let screen_x = i32::from(entity.x) - i32::from(view.camera_x) + self.mode.margin();
        let screen_y = i32::from(entity.y) - i32::from(view.camera_y) - i32::from(entity.elevation);
        for number in 0..usize::from(entity.count) {
            let piece = entity.pieces[number];
            let x = screen_x + i32::from(if mirror { piece.flipped_x } else { piece.x });
            let y = screen_y + i32::from(piece.y);
            let pw = usize::from(piece.width) * 8;
            let ph = usize::from(piece.height) * 8;
            if x >= self.mode.width() as i32
                || x + pw as i32 <= 0
                || y >= height as i32
                || y + ph as i32 <= 0
            {
                continue;
            }
            if !(1..=4).contains(&piece.width) || !(1..=4).contains(&piece.height) {
                return Err(RenderError::InvalidSprite(
                    "piece dimensions exceed 4x4 tiles",
                ));
            }
            let key = (piece.source, piece.format, piece.width, piece.height);
            let tile = if viewport == 0 {
                entity.primary_tiles[number]
            } else {
                entity.secondary_tiles[number]
            };
            let use_vram = entity.ready && tile != 0;
            let rom_source = piece.source != 0 && (piece.source as usize) < memory.rom().len();
            let decoded = if !use_vram && rom_source {
                let cached = match self.atlas.entry(key) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) => {
                        entry.insert(decode_piece(memory.rom(), piece)?.into_boxed_slice())
                    }
                };
                Some(&**cached)
            } else {
                None
            };
            let attributes = (entity.tile & 0xf800).wrapping_add(((piece.format as u16) & 3) << 13);
            let clip = index <= 1 && entity.elevation < 0 && entity.animation_group & 0x20 == 0;
            let cutoff = i32::from(entity.elevation) + 2 - i32::from(piece.y);
            if clip && cutoff <= 0 {
                continue;
            }
            for py in 0..ph {
                if clip && py as i32 > cutoff {
                    continue;
                }
                let sy = y + py as i32;
                if !(0..height as i32).contains(&sy) {
                    continue;
                }
                for px in 0..pw {
                    let sx = x + px as i32;
                    if !(0..self.mode.width() as i32).contains(&sx) {
                        continue;
                    }
                    let source_x = if mirror { pw - 1 - px } else { px };
                    let source_y = if entity.tile & 0x1000 != 0 {
                        ph - 1 - py
                    } else {
                        py
                    };
                    let tile_index = source_x / 8 * usize::from(piece.height) + source_y / 8;
                    let byte_index = tile_index * 32 + source_y % 8 * 4 + source_x % 8 / 2;
                    let mut packed = if use_vram {
                        vram[(usize::from(tile & 0x07ff) * 32 + byte_index) & 0xffff]
                    } else if let Some(decoded) = decoded {
                        decoded[byte_index]
                    } else {
                        continue;
                    };
                    if entity.behavior & 0x20 != 0 && matches!(packed, 0xff | 0xf0 | 0x0f) {
                        packed = 0;
                    }
                    if clip && py as i32 == cutoff {
                        packed &= [0x0f, 0x00, 0x0f, 0xf0][source_x % 8 / 2];
                    }
                    let color = if source_x & 1 == 0 {
                        packed >> 4
                    } else {
                        packed & 15
                    };
                    let target =
                        (sy as usize + viewport * height) * self.mode.width() + sx as usize;
                    if color != 0 && !self.occupied[target] {
                        self.occupied[target] = true;
                        self.sprites[target] =
                            IndexedPixel((((attributes >> 9) & 0x70) as u8) | color);
                    }
                }
            }
        }
        Ok(())
    }
    fn emit(&mut self, x: usize, y: usize, color: u8, attributes: u16) {
        if color == 0 {
            return;
        }
        let index = y * self.mode.width() + x;
        if !self.occupied[index] {
            self.occupied[index] = true;
            self.sprites[index] = IndexedPixel((((attributes >> 9) & 0x70) as u8) | color);
        }
    }
    fn paint_hardware(&mut self, vram: &[u8], sprite: HudSprite, viewport: usize, height: usize) {
        let w = usize::from((sprite.size >> 2) & 3) * 8 + 8;
        let h = usize::from(sprite.size & 3) * 8 + 8;
        let x = i32::from(sprite.x) - 128 + self.mode.margin();
        let y = i32::from(sprite.y) - 128 - (viewport * height) as i32;
        for py in 0..h {
            let sy = y + py as i32;
            if !(0..height as i32).contains(&sy) {
                continue;
            }
            for px in 0..w {
                let sx = x + px as i32;
                if !(0..self.mode.width() as i32).contains(&sx) {
                    continue;
                }
                let tx = if sprite.tile & 0x0800 != 0 {
                    w - 1 - px
                } else {
                    px
                };
                let ty = if sprite.tile & 0x1000 != 0 {
                    h - 1 - py
                } else {
                    py
                };
                let tile_index = tx / 8 * (h / 8) + ty / 8;
                let packed = vram[(usize::from(sprite.tile & 0x07ff) * 32
                    + tile_index * 32
                    + ty % 8 * 4
                    + tx % 8 / 2)
                    & 0xffff];
                let color = if tx & 1 == 0 {
                    packed >> 4
                } else {
                    packed & 15
                };
                self.emit(
                    sx as usize,
                    sy as usize + viewport * height,
                    color,
                    sprite.tile,
                );
            }
        }
    }
    fn paint_shadow(
        &mut self,
        memory: &AddressSpace,
        vram: &[u8],
        entity: Entity,
        viewport: usize,
        height: usize,
        view: PublishedViewport,
    ) {
        let (size, tile) = if entity.elevation < 0 && entity.state != -1 {
            (12, 0x87f9)
        } else {
            let depth = if entity.elevation >= 128 || entity.state == -1 {
                1
            } else {
                ((127 - i32::from(entity.elevation)) as u8 & 0x70) >> 4
            };
            let tile = rom::sprite_tile_allocation_lookup(memory.revision())
                .at(i32::from(depth) - 1)
                .read(memory);
            ((depth << 1) & 12, u16::from(tile).wrapping_add(0xe7a0))
        };
        let shadow = HudSprite {
            x: entity
                .x
                .wrapping_sub(view.camera_x)
                .wrapping_add(128)
                .wrapping_sub(i16::from((size & 12) + 4)),
            y: entity
                .y
                .wrapping_sub(view.camera_y)
                .wrapping_add(128 + (viewport * height) as i16)
                .wrapping_sub(if entity.elevation < 0 { 2 } else { 4 }),
            size,
            tile,
        };
        self.paint_hardware(vram, shadow, viewport, height);
    }
}

#[cfg(test)]
mod tests;
