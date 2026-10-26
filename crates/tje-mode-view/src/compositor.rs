use crate::{
    CANVAS_HEIGHT, CANVAS_WIDTH, Error, HEIGHT, MAX_CACHE_BYTES, Packet, Result, WIDTH, overlay,
    packet::{Actor, Metasprite, Shadow, View, VramShadow},
    source::FrameSource,
};
use jgenesis_common::frontend::{Color, FrameSize};
use tje_extended_view::{
    IndexedPixel,
    presentation::Piece,
    resources::{MAX_PIECE_BYTES, decode_piece_into, piece_length},
};
use tje_four_player::Character;

const CACHE_SLOTS: usize = 4096;
#[derive(Clone)]
struct CacheEntry {
    key: Option<(u32, i8, u8, u8)>,
    bytes: [u8; MAX_PIECE_BYTES],
}
struct Cache {
    entries: Vec<CacheEntry>,
}
impl Cache {
    fn new() -> Self {
        let mut entries = Vec::with_capacity(CACHE_SLOTS);
        entries.resize_with(CACHE_SLOTS, || CacheEntry {
            key: None,
            bytes: [0; MAX_PIECE_BYTES],
        });
        Self { entries }
    }
    fn clear(&mut self) {
        for entry in &mut self.entries {
            entry.key = None;
        }
    }
    fn piece(&mut self, rom: &[u8], piece: Piece) -> Result<&[u8]> {
        let length = piece_length(piece)?;
        let key = (piece.source, piece.format, piece.width, piece.height);
        let hash = piece.source.wrapping_mul(0x9e3779b1)
            ^ (piece.format as u8 as u32).rotate_left(13)
            ^ (u32::from(piece.width) << 7)
            ^ u32::from(piece.height);
        let entry = &mut self.entries[hash as usize & (CACHE_SLOTS - 1)];
        if entry.key != Some(key) {
            entry.key = None;
            decode_piece_into(rom, piece, &mut entry.bytes[..length])?;
            entry.key = Some(key);
        }
        Ok(&entry.bytes[..length])
    }
}
#[derive(Clone, Copy, Default)]
struct SpritePixel {
    index: u8,
    high: bool,
    occupied: bool,
    character: Option<Character>,
}
/// Double-buffered final output and reusable per-view layers. Cache eviction is
/// bounded and lossless: a collision decodes into the same scratch slot, never
/// drops an actor or consults simulation allocation/contact state.
pub struct Compositor {
    canvases: [Vec<Color>; 2],
    active: usize,
    view: Vec<Color>,
    terrain: Vec<IndexedPixel>,
    sprites: Vec<SpritePixel>,
    cache: Cache,
    shadow_highlight: bool,
    revision: Option<tje_rom::Revision>,
    authored: [bool; 2],
}
impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}
impl Compositor {
    pub fn new() -> Self {
        assert!(CACHE_SLOTS * std::mem::size_of::<CacheEntry>() < MAX_CACHE_BYTES);
        Self {
            canvases: std::array::from_fn(|_| vec![Color::BLACK; CANVAS_WIDTH * CANVAS_HEIGHT]),
            active: 0,
            view: vec![Color::BLACK; WIDTH * HEIGHT],
            terrain: vec![IndexedPixel::default(); WIDTH * HEIGHT],
            sprites: vec![SpritePixel::default(); WIDTH * HEIGHT],
            cache: Cache::new(),
            shadow_highlight: false,
            revision: None,
            authored: [false; 2],
        }
    }
    pub fn frame(&self) -> (&[Color], FrameSize) {
        (
            &self.canvases[self.active],
            FrameSize {
                width: CANVAS_WIDTH as u32,
                height: CANVAS_HEIGHT as u32,
            },
        )
    }
    pub fn evict_resources(&mut self) {
        self.cache.clear();
    }
    pub fn set_shadow_highlight(&mut self, enabled: bool) {
        self.shadow_highlight = enabled;
    }
    /// Preserve genuine authored pixels at native density, centered on the
    /// candidate's fixed canvas. This is an explicit scene, not a world fallback.
    pub fn compose_authored(&mut self, original: &[Color]) -> Result<()> {
        if original.len() < 320 * 224 {
            return Err(Error::Invalid(
                "authored scene lacks its genuine 320x224 hardware surface",
            ));
        }
        let pending = self.active ^ 1;
        if !self.authored[pending] {
            self.canvases[pending].fill(Color::BLACK);
        }
        let left = (CANVAS_WIDTH - 320) / 2;
        let top = (CANVAS_HEIGHT - 224) / 2;
        for row in 0..224 {
            let target = (top + row) * CANVAS_WIDTH + left;
            self.canvases[pending][target..target + 320]
                .copy_from_slice(&original[row * 320..(row + 1) * 320]);
        }
        self.authored[pending] = true;
        self.active = pending;
        Ok(())
    }
    pub fn compose(&mut self, packet: &Packet, source: &impl FrameSource) -> Result<()> {
        if packet.stamp() != Some(source.stamp()) {
            return Err(Error::Invalid("packet and borrowed geometry epochs differ"));
        }
        if self.revision != Some(source.revision()) {
            self.cache.clear();
            self.revision = Some(source.revision());
        }
        for (index, &view) in packet.views().iter().enumerate() {
            self.view.fill(Color::rgb(12, 16, 24));
            self.sprites.fill(SpritePixel::default());
            if let View::World { level, origin, .. } = view {
                source.terrain(view, &mut self.terrain)?;
                for patch in packet.patches().iter().filter(|patch| patch.level == level) {
                    for y in 0..HEIGHT {
                        let row = (origin[1] + y as i32).div_euclid(8);
                        if !(patch.minimum[1]..=patch.maximum[1]).contains(&row) {
                            continue;
                        }
                        for x in 0..WIDTH {
                            let column = (origin[0] + x as i32).div_euclid(8);
                            if (patch.minimum[0]..=patch.maximum[0]).contains(&column) {
                                self.terrain[y * WIDTH + x].clear_priority();
                            }
                        }
                    }
                }
                for actor in packet.actors().iter().filter(|actor| actor.level == level) {
                    for &piece in actor.pieces.iter().take(usize::from(actor.count)) {
                        self.paint_actor(source.rom(), actor, piece, origin, false)?;
                    }
                }
                // Original chain semantics: all world bodies, then all shadows.
                for actor in packet.actors().iter().filter(|actor| actor.level == level) {
                    match &actor.shadow {
                        Some(Shadow::Rom(frame)) => {
                            for piece in frame.pieces.iter().take(usize::from(frame.count)) {
                                self.paint_actor(source.rom(), actor, *piece, origin, true)?;
                            }
                        }
                        Some(Shadow::Vram(shadow)) => {
                            self.paint_vram_shadow(source.vram(), actor, *shadow, origin)?
                        }
                        None => {}
                    }
                }
                for (target, (terrain, sprite)) in self
                    .view
                    .iter_mut()
                    .zip(self.terrain.iter().zip(&self.sprites))
                {
                    let visible = sprite.occupied
                        && (sprite.high || !terrain.high_priority() || terrain.transparent());
                    let mut modifier = if self.shadow_highlight && !terrain.foreground_priority() {
                        -1
                    } else {
                        0
                    };
                    let mut selected = if terrain.transparent() {
                        0
                    } else {
                        terrain.palette_index()
                    };
                    let mut character = None;
                    if visible {
                        if self.shadow_highlight && sprite.index == 62 {
                            modifier = if modifier == -1 { 0 } else { 1 };
                        } else if self.shadow_highlight && sprite.index == 63 {
                            modifier = -1;
                        } else {
                            selected = usize::from(sprite.index);
                            character = sprite.character;
                            if self.shadow_highlight && (sprite.high || sprite.index & 15 == 14) {
                                modifier = 0;
                            }
                        }
                    }
                    *target = source.color(selected, character, modifier);
                }
                for actor in packet.actors().iter().filter(|actor| actor.level == level) {
                    if let Some(seat) = actor.seat {
                        let x = actor.position[0] - origin[0] - 3;
                        let y = actor.position[1] - origin[1] - actor.elevation - 48;
                        overlay::rect(&mut self.view, [x - 2, y - 1, 9, 9], Color::rgb(8, 12, 20));
                        overlay::number(&mut self.view, x, y, u32::from(seat.get()) + 1);
                    }
                }
            } else {
                overlay::text(
                    &mut self.view,
                    130,
                    104,
                    b"SEAT NOT IN USE",
                    Color::rgb(180, 190, 200),
                );
            }
            packet.overlays()[index].paint(&mut self.view, source)?;
            if let Some(inventory) = &packet.overlays()[index].inventory {
                for (slot, item) in inventory.slots.iter().enumerate() {
                    if let Some(icon) = item.as_ref().and_then(|item| item.icon.as_ref()) {
                        let position = [23 + (slot % 8) as i32 * 46, 40 + (slot / 8) as i32 * 68];
                        self.paint_icon(source, icon, position)?;
                    }
                }
            }
            let offset_x = index % 2 * WIDTH;
            let offset_y = index / 2 * HEIGHT;
            let canvas = &mut self.canvases[self.active ^ 1];
            for row in 0..HEIGHT {
                let start = (offset_y + row) * CANVAS_WIDTH + offset_x;
                canvas[start..start + WIDTH]
                    .copy_from_slice(&self.view[row * WIDTH..(row + 1) * WIDTH]);
            }
        }
        self.authored[self.active ^ 1] = false;
        self.active ^= 1;
        Ok(())
    }
    fn paint_actor(
        &mut self,
        rom: &[u8],
        actor: &Actor,
        piece: Piece,
        origin: [i32; 2],
        shadow: bool,
    ) -> Result<()> {
        let flip_horizontal = actor.flip_horizontal && !shadow;
        let flip_vertical = actor.flip_vertical && !shadow;
        let x = actor.position[0] - origin[0]
            + i32::from(if flip_horizontal {
                piece.flipped_x
            } else {
                piece.x
            });
        let y = actor.position[1] - origin[1] - if shadow { 0 } else { actor.elevation }
            + i32::from(piece.y);
        let width = i32::from(piece.width) * 8;
        let height = i32::from(piece.height) * 8;
        if x >= WIDTH as i32 || x + width <= 0 || y >= HEIGHT as i32 || y + height <= 0 {
            return Ok(());
        }
        let cutoff = actor.elevation + 2 - i32::from(piece.y);
        let submerged = actor.submerged && !shadow;
        if submerged && cutoff <= 0 {
            return Ok(());
        }
        let bytes = self.cache.piece(rom, piece)?;
        let attributes = ((u16::from(if shadow { 0 } else { actor.palette_bank }) << 13)
            | if actor.high_priority || shadow {
                0x8000
            } else {
                0
            })
        .wrapping_add((u16::from(piece.format as u8 & 3)) << 13);
        for py in (-y).max(0)..height.min(HEIGHT as i32 - y) {
            if submerged && py > cutoff {
                continue;
            }
            let sy = y + py;
            for px in (-x).max(0)..width.min(WIDTH as i32 - x) {
                let sx = x + px;
                let tx = if flip_horizontal { width - 1 - px } else { px } as usize;
                let ty = if flip_vertical { height - 1 - py } else { py } as usize;
                let tile = tx / 8 * usize::from(piece.height) + ty / 8;
                let mut packed = bytes[tile * 32 + ty % 8 * 4 + tx % 8 / 2];
                if actor.mask_pixels && !shadow && matches!(packed, 0xff | 0xf0 | 0x0f) {
                    packed = 0;
                }
                if submerged && py == cutoff {
                    packed &= [0x0f, 0x00, 0x0f, 0xf0][tx % 8 / 2];
                }
                let color = if tx & 1 == 0 {
                    packed >> 4
                } else {
                    packed & 15
                };
                let target = &mut self.sprites[sy as usize * WIDTH + sx as usize];
                // Mark occupied before terrain priority is resolved. An earlier
                // obscured low-priority sprite still blocks a later high sprite.
                if color != 0 && !target.occupied {
                    *target = SpritePixel {
                        index: (((attributes >> 9) & 0x30) as u8) | color,
                        high: attributes & 0x8000 != 0,
                        occupied: true,
                        character: if shadow { None } else { actor.character },
                    };
                }
            }
        }
        Ok(())
    }
    fn paint_vram_shadow(
        &mut self,
        vram: &[u8],
        actor: &Actor,
        shadow: VramShadow,
        origin: [i32; 2],
    ) -> Result<()> {
        if vram.len() != 0x10000 {
            return Err(tje_extended_view::RenderError::InvalidVramLength(vram.len()).into());
        }
        let [tiles_x, tiles_y] = shadow.size.map(usize::from);
        let width = tiles_x * 8;
        let height = tiles_y * 8;
        let x = actor.position[0] - origin[0] + i32::from(shadow.offset[0]);
        let y = actor.position[1] - origin[1] + i32::from(shadow.offset[1]);
        for row in 0..height {
            let sy = y + row as i32;
            if !(0..HEIGHT as i32).contains(&sy) {
                continue;
            }
            for column in 0..width {
                let sx = x + column as i32;
                if !(0..WIDTH as i32).contains(&sx) {
                    continue;
                }
                let tx = if shadow.tile & 0x0800 != 0 {
                    width - 1 - column
                } else {
                    column
                };
                let ty = if shadow.tile & 0x1000 != 0 {
                    height - 1 - row
                } else {
                    row
                };
                let tile = tx / 8 * tiles_y + ty / 8;
                let address =
                    (usize::from(shadow.tile & 0x07ff) * 32 + tile * 32 + ty % 8 * 4 + tx % 8 / 2)
                        & 0xffff;
                let packed = vram[address];
                let color = if tx & 1 == 0 {
                    packed >> 4
                } else {
                    packed & 15
                };
                let target = &mut self.sprites[sy as usize * WIDTH + sx as usize];
                if color != 0 && !target.occupied {
                    *target = SpritePixel {
                        index: (((shadow.tile >> 9) & 0x30) as u8) | color,
                        high: shadow.tile & 0x8000 != 0,
                        occupied: true,
                        character: None,
                    };
                }
            }
        }
        Ok(())
    }

    fn paint_icon(
        &mut self,
        source: &impl FrameSource,
        icon: &Metasprite,
        origin: [i32; 2],
    ) -> Result<()> {
        let pieces = &icon.pieces[..usize::from(icon.count)];
        let left = pieces
            .iter()
            .map(|piece| i32::from(piece.x))
            .min()
            .expect("validated icon");
        let top = pieces
            .iter()
            .map(|piece| i32::from(piece.y))
            .min()
            .expect("validated icon");
        let right = pieces
            .iter()
            .map(|piece| i32::from(piece.x) + i32::from(piece.width) * 8)
            .max()
            .expect("validated icon");
        let bottom = pieces
            .iter()
            .map(|piece| i32::from(piece.y) + i32::from(piece.height) * 8)
            .max()
            .expect("validated icon");
        let offset = [
            (32 - (right - left)).div_euclid(2) - left,
            (32 - (bottom - top)).div_euclid(2) - top,
        ];
        // Paint back-to-front so the first authored piece retains sprite order.
        for &piece in pieces.iter().rev() {
            let bytes = self.cache.piece(source.rom(), piece)?;
            let width = usize::from(piece.width) * 8;
            let height = usize::from(piece.height) * 8;
            for y in 0..height {
                for x in 0..width {
                    let sx = origin[0] + offset[0] + i32::from(piece.x) + x as i32;
                    let sy = origin[1] + offset[1] + i32::from(piece.y) + y as i32;
                    if !(origin[0]..origin[0] + 32).contains(&sx)
                        || !(origin[1]..origin[1] + 32).contains(&sy)
                        || !(0..WIDTH as i32).contains(&sx)
                        || !(0..HEIGHT as i32).contains(&sy)
                    {
                        continue;
                    }
                    let packed = bytes
                        [(x / 8 * usize::from(piece.height) + y / 8) * 32 + y % 8 * 4 + x % 8 / 2];
                    let color = if x & 1 == 0 { packed >> 4 } else { packed & 15 };
                    if color != 0 {
                        self.view[sy as usize * WIDTH + sx as usize] = source.color(
                            usize::from(piece.format as u8 & 3) * 16 + usize::from(color),
                            None,
                            0,
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
