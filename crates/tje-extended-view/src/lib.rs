//! Opt-in terrain rendering from the user's RAM map and ROM metatile tables.
//!
//! No shipping crate depends on this experiment. The renderer borrows state and
//! caller-owned output, never reads VDP name tables, and cannot mutate simulation.
//! Optional gameplay hooks live separately behind the `enhanced-mode` feature.
//! See docs/EXTENDED_VIEW.md for the scene/sprite composition boundary.
#![forbid(unsafe_code)]

use std::fmt;
use tje_mem::{AddressSpace, ram, rom};

#[cfg(feature = "enhanced-mode")]
pub mod enhanced;

pub const NATIVE_WIDTH: usize = 320;
pub const WIDESCREEN_WIDTH: usize = 400;
pub const FULL_HEIGHT: usize = 224;
pub const SPLIT_HEIGHT: usize = 112;
pub const MAX_EXPERIMENTAL_WIDTH: u16 = 512;

/// Wider views are deliberately separate from the ordinary widescreen choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExperimentalWidth(u16);

impl ExperimentalWidth {
    pub fn new(width: u16) -> Result<Self, RenderError> {
        if width <= WIDESCREEN_WIDTH as u16
            || width > MAX_EXPERIMENTAL_WIDTH
            || !width.is_multiple_of(8)
        {
            return Err(RenderError::InvalidExperimentalWidth(width));
        }
        Ok(Self(width))
    }

    pub const fn pixels(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Faithful,
    Widescreen,
    Experimental(ExperimentalWidth),
}

impl ViewMode {
    pub const fn width(self) -> usize {
        match self {
            Self::Faithful => NATIVE_WIDTH,
            Self::Widescreen => WIDESCREEN_WIDTH,
            Self::Experimental(width) => width.pixels(),
        }
    }

    pub const fn margin(self) -> i32 {
        ((self.width() - NATIVE_WIDTH) / 2) as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Viewport {
    Primary,
    Secondary,
}

/// Completed-frame camera selection. Pending cameras belong to simulation hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldView {
    viewport: Viewport,
    camera_x: i16,
    camera_y: i16,
    level: i8,
    height: usize,
}

impl WorldView {
    /// The caller must select a published world scene, not a map/menu/ride frame.
    pub fn committed(memory: &AddressSpace, viewport: Viewport) -> Result<Self, RenderError> {
        let revision = memory.revision();
        let split = ram::g_split_screen_enabled(revision).read(memory) != 0;
        let (camera_x, camera_y, level, height) = match viewport {
            Viewport::Primary => (
                ram::g_camera1_committed_x(revision).read(memory),
                ram::g_camera1_committed_y(revision).read(memory),
                ram::g_player1_map_level(revision).read(memory),
                if split { SPLIT_HEIGHT } else { FULL_HEIGHT },
            ),
            Viewport::Secondary if split => (
                ram::g_camera2_committed_x(revision).read(memory),
                ram::g_camera2_committed_y(revision).read(memory),
                ram::g_player2_map_level(revision).read(memory),
                SPLIT_HEIGHT,
            ),
            Viewport::Secondary => return Err(RenderError::InactiveViewport),
        };
        if !(0..=26).contains(&level) {
            return Err(RenderError::UnloadedLevel(level));
        }
        Ok(Self {
            viewport,
            camera_x,
            camera_y,
            level,
            height,
        })
    }

    pub const fn height(self) -> usize {
        self.height
    }

    pub const fn level(self) -> i8 {
        self.level
    }

    pub const fn camera(self) -> (i16, i16) {
        (self.camera_x, self.camera_y)
    }
}

/// Six-bit CRAM index and a separate priority bit. Color zero is transparent,
/// irrespective of palette. No palette or authored pattern data is stored here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct IndexedPixel(u8);

impl IndexedPixel {
    pub const fn palette_index(self) -> usize {
        (self.0 & 0x3f) as usize
    }

    pub const fn transparent(self) -> bool {
        self.0 & 0x0f == 0
    }

    pub const fn high_priority(self) -> bool {
        self.0 & 0x40 != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    InvalidExperimentalWidth(u16),
    InactiveViewport,
    UnloadedLevel(i8),
    InvalidVramLength(usize),
    TruncatedRom,
    InvalidMetatile(u8),
    InvalidOverlay(u8),
    OutputLength { expected: usize, actual: usize },
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExperimentalWidth(width) => write!(
                formatter,
                "experimental width {width} must be an 8-pixel multiple from 408 through 512"
            ),
            Self::InactiveViewport => formatter.write_str("secondary world viewport is not active"),
            Self::UnloadedLevel(level) => {
                write!(formatter, "world map is not loaded (level {level})")
            }
            Self::InvalidVramLength(length) => {
                write!(
                    formatter,
                    "expected 65536 borrowed VRAM bytes, got {length}"
                )
            }
            Self::TruncatedRom => formatter.write_str("ROM does not contain the metatile tables"),
            Self::InvalidMetatile(id) => write!(formatter, "invalid base metatile {id}"),
            Self::InvalidOverlay(id) => write!(formatter, "invalid overlay selector {id}"),
            Self::OutputLength { expected, actual } => {
                write!(formatter, "expected {expected} output pixels, got {actual}")
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// The ROM/map and live VRAM patterns stay borrowed, never copied into a second
/// authoritative map or a per-frame decoded atlas. Construction performs no I/O.
pub struct MapRenderer<'a> {
    memory: &'a AddressSpace,
    vram: &'a [u8],
}

impl<'a> MapRenderer<'a> {
    pub fn new(memory: &'a AddressSpace, vram: &'a [u8]) -> Result<Self, RenderError> {
        if vram.len() != 0x10000 {
            return Err(RenderError::InvalidVramLength(vram.len()));
        }
        let revision = memory.revision();
        let required = [
            rom::level_metatile_maps(revision).address() as usize + 122 * 8 * 16 * 2,
            rom::overlay_metatile_maps(revision).address() as usize + 15 * 8 * 16 * 2,
            rom::starfield_byte_map(revision).address() as usize + 32 * 32,
        ]
        .into_iter()
        .max()
        .unwrap();
        if memory.rom().len() < required {
            return Err(RenderError::TruncatedRom);
        }
        Ok(Self { memory, vram })
    }

    /// G/src/camera/camera_tiles_and_scroll.c:22-52,159-193,347-392.
    /// Coordinates are world 8x8 tiles, not VDP ring-buffer coordinates.
    pub fn terrain_tile(
        &self,
        view: WorldView,
        tile_x: i32,
        tile_y: i32,
    ) -> Result<u16, RenderError> {
        let attributes = ram::g_camera_tile_attributes(self.memory.revision()).read(self.memory);
        self.terrain_tile_with_attributes(view, tile_x, tile_y, attributes)
    }

    fn terrain_tile_with_attributes(
        &self,
        view: WorldView,
        tile_x: i32,
        tile_y: i32,
        attributes: u16,
    ) -> Result<u16, RenderError> {
        let map_x = tile_x.div_euclid(16) - 1;
        let map_y = tile_y.div_euclid(8) - 1;
        let column = tile_x.rem_euclid(16);
        let row = tile_y.rem_euclid(8);
        let cell = if (0..19).contains(&map_x) && (0..26).contains(&map_y) {
            let map = match view.viewport {
                Viewport::Primary => ram::g_player1_level_map(self.memory.revision()),
                Viewport::Secondary => ram::g_player2_level_map(self.memory.revision()),
            };
            map.at(map_y).at(map_x).read(self.memory)
        } else if view.level == 1 || view.level == 26 {
            // The source's `wraps` flag chooses a fill; it does not wrap map cells.
            82
        } else {
            return Ok(attributes);
        };
        let overlay = ((cell & 0x1f00) >> 8) as u8;
        if overlay != 0 {
            let id = overlay & 15;
            if id == 0 {
                return Err(RenderError::InvalidOverlay(overlay));
            }
            let mirror = overlay & 16 != 0;
            let tile = rom::overlay_metatile_maps(self.memory.revision())
                .at(i32::from(id - 1))
                .at(row)
                .at(if mirror { 15 - column } else { column })
                .read(self.memory);
            if tile != 0 {
                return Ok((tile ^ if mirror { 0x0800 } else { 0 }) | attributes);
            }
        }
        let base = cell as u8;
        let id = base & 0x7f;
        if id >= 122 {
            return Err(RenderError::InvalidMetatile(id));
        }
        let mirror = base & 0x80 != 0;
        let tile = rom::level_metatile_maps(self.memory.revision())
            .at(i32::from(id))
            .at(row)
            .at(if mirror { 15 - column } else { column })
            .read(self.memory);
        Ok((tile ^ if mirror { 0x0800 } else { 0 }) | attributes)
    }

    /// Render only the foreground map; transparent pixels remain transparent.
    /// Each metatile lookup is shared by the up-to-8x8 visible tile span.
    pub fn render_terrain(
        &self,
        view: WorldView,
        mode: ViewMode,
        output: &mut [IndexedPixel],
    ) -> Result<(), RenderError> {
        let width = mode.width();
        let expected = width * view.height;
        if output.len() != expected {
            return Err(RenderError::OutputLength {
                expected,
                actual: output.len(),
            });
        }
        let attributes = ram::g_camera_tile_attributes(self.memory.revision()).read(self.memory);
        let origin_x = i32::from(view.camera_x) - mode.margin();
        let origin_y = i32::from(view.camera_y);
        let mut screen_y = 0;
        while screen_y < view.height {
            let world_y = origin_y + screen_y as i32;
            let local_y = world_y.rem_euclid(8) as usize;
            let rows = (8 - local_y).min(view.height - screen_y);
            let mut screen_x = 0;
            while screen_x < width {
                let world_x = origin_x + screen_x as i32;
                let local_x = world_x.rem_euclid(8) as usize;
                let columns = (8 - local_x).min(width - screen_x);
                let tile = self.terrain_tile_with_attributes(
                    view,
                    world_x.div_euclid(8),
                    world_y.div_euclid(8),
                    attributes,
                )?;
                for y in 0..rows {
                    let row_start = (screen_y + y) * width + screen_x;
                    self.paint_span(
                        tile,
                        local_y + y,
                        local_x,
                        &mut output[row_start..row_start + columns],
                        false,
                    );
                }
                screen_x += columns;
            }
            screen_y += rows;
        }
        Ok(())
    }

    /// Foreground plus the ordinary half-speed starfield. No entities, HUD or
    /// falling-priority patches are inferred from this completed-frame image.
    pub fn render_background(
        &self,
        view: WorldView,
        mode: ViewMode,
        output: &mut [IndexedPixel],
    ) -> Result<(), RenderError> {
        self.render_terrain(view, mode, output)?;
        if view.level == 26 {
            return Ok(());
        }
        let width = mode.width();
        // C's signed /2 truncates toward zero. Screen pixels are not scaled.
        let origin_x = i32::from(view.camera_x) / 2 - mode.margin();
        let origin_y = i32::from(view.camera_y) / 2;
        for screen_y in 0..view.height {
            let world_y = origin_y + screen_y as i32;
            let row = world_y.div_euclid(8).rem_euclid(32);
            let mut screen_x = 0;
            while screen_x < width {
                let world_x = origin_x + screen_x as i32;
                let local_x = world_x.rem_euclid(8) as usize;
                let columns = (8 - local_x).min(width - screen_x);
                let start = screen_y * width + screen_x;
                let span = &mut output[start..start + columns];
                if span.iter().any(|pixel| pixel.transparent()) {
                    let column = world_x.div_euclid(8).rem_euclid(64) / 2;
                    // G/src/core/startup_and_main_loop.c:87-102. Signed byte
                    // promotion preserves its high-bit horizontal-flip encoding.
                    let tile = (rom::starfield_byte_map(self.memory.revision())
                        .at(row)
                        .at(column)
                        .read(self.memory) as i16 as u16
                        & 0x087f)
                        + 0x0200;
                    self.paint_span(tile, world_y.rem_euclid(8) as usize, local_x, span, true);
                }
                screen_x += columns;
            }
        }
        Ok(())
    }

    fn paint_span(
        &self,
        tile: u16,
        row: usize,
        column: usize,
        output: &mut [IndexedPixel],
        transparent_only: bool,
    ) {
        let row = if tile & 0x1000 != 0 { 7 - row } else { row };
        let pattern_row = usize::from(tile & 0x07ff) * 32 + row * 4;
        let flags = ((tile >> 9) & 0x70) as u8;
        for (offset, pixel) in output.iter_mut().enumerate() {
            if transparent_only && !pixel.transparent() {
                continue;
            }
            let x = column + offset;
            let x = if tile & 0x0800 != 0 { 7 - x } else { x };
            let byte = self.vram[pattern_row + x / 2];
            let color = if x & 1 == 0 { byte >> 4 } else { byte & 15 };
            *pixel = IndexedPixel(flags | color);
        }
    }
}

#[cfg(test)]
mod tests;
