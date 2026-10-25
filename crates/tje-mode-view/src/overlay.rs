//! Bounded per-seat UI packets. These are display data, not blocking menu loops.
use crate::{Error, HEIGHT, Result, WIDTH, source::FrameSource};
use jgenesis_common::frontend::Color;
use tje_extended_view::presentation::Piece;
use tje_four_player::SeatId;
use tje_world::LevelId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hud {
    pub seat: SeatId,
    pub level: LevelId,
    pub health: u16,
    pub maximum_health: u16,
    pub lives: u8,
    pub rank: u8,
    pub bucks: u16,
    pub pause_requested: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryKind {
    Inventory,
    MailOrder,
    Identify,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub wrapper: u8,
    pub identified: bool,
    pub icon: Option<Piece>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    pub kind: InventoryKind,
    pub selected: u8,
    pub slots: [Option<Item>; 16],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub position: [i32; 2],
    pub seat: Option<SeatId>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Map {
    pub level: LevelId,
    pub dimensions: [u16; 2],
    /// Shared discovery bins are 320x256 world units, anchored at (72,8).
    pub discovery: [u64; 9],
    pub markers: [Option<Marker>; 68],
}
impl Map {
    fn discovered(&self, position: [i32; 2]) -> bool {
        if position[0] < 72 || position[1] < 8 {
            return false;
        }
        let columns = (i32::from(self.dimensions[0]) + 2) * 128;
        let columns = ((columns - 72 + 319) / 320) as usize;
        let x = ((position[0] - 72) / 320) as usize;
        let y = ((position[1] - 8) / 256) as usize;
        let index = y * columns + x;
        x < columns && index < 576 && self.discovery[index / 64] & (1 << (index % 64)) != 0
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speech {
    pub text: [u8; 96],
    pub length: u8,
    pub choices: bool,
    pub accept_selected: bool,
}
impl Default for Speech {
    fn default() -> Self {
        Self {
            text: [0; 96],
            length: 0,
            choices: false,
            accept_selected: true,
        }
    }
}
impl Speech {
    pub fn new(text: &[u8], choices: bool, accept_selected: bool) -> Result<Self> {
        if text.len() > 96
            || !text
                .iter()
                .all(|byte| byte.is_ascii_graphic() || *byte == b' ' || *byte == b'\n')
        {
            return Err(Error::Invalid(
                "speech exceeds bounded ASCII display packet",
            ));
        }
        let mut result = Self {
            length: text.len() as u8,
            choices,
            accept_selected,
            ..Self::default()
        };
        result.text[..text.len()].copy_from_slice(text);
        Ok(result)
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeatOverlay {
    pub hud: Option<Hud>,
    pub inventory: Option<Inventory>,
    pub map: Option<Map>,
    pub speech: Speech,
}
impl SeatOverlay {
    pub fn validate(&self) -> Result<()> {
        if self.hud.is_some_and(|hud| {
            hud.maximum_health == 0
                || hud.health > hud.maximum_health
                || hud.rank > 8
                || hud.level.validate().is_err()
        }) {
            return Err(Error::Invalid("invalid per-seat HUD"));
        }
        if let Some(inventory) = &self.inventory {
            if inventory.selected >= 16 {
                return Err(Error::Invalid("inventory selection exceeds 16 slots"));
            }
            for item in inventory.slots.iter().flatten() {
                if item.wrapper >= 28 {
                    return Err(Error::Invalid("invalid wrapper display identity"));
                }
                if let Some(piece) = item.icon {
                    tje_extended_view::resources::piece_length(piece)?;
                }
            }
        }
        if let Some(map) = &self.map {
            if map.level.validate().is_err()
                || map.dimensions.iter().any(|axis| !(8..=64).contains(axis))
            {
                return Err(Error::Invalid("invalid map overlay geometry"));
            }
            if map.markers.iter().flatten().any(|marker| {
                marker
                    .position
                    .iter()
                    .any(|value| value.unsigned_abs() > 32767)
            }) {
                return Err(Error::Invalid("map marker exceeds world envelope"));
            }
        }
        if self.inventory.is_some() && self.map.is_some() {
            return Err(Error::Invalid(
                "a seat cannot publish inventory and map simultaneously",
            ));
        }
        if usize::from(self.speech.length) > self.speech.text.len() {
            return Err(Error::Invalid("invalid speech extent"));
        }
        Speech::new(
            &self.speech.text[..usize::from(self.speech.length)],
            self.speech.choices,
            self.speech.accept_selected,
        )?;
        Ok(())
    }
    pub(crate) fn paint(&self, output: &mut [Color], source: &impl FrameSource) -> Result<()> {
        if let Some(map) = &self.map {
            rect(output, [8, 8, 384, 184], Color::rgb(12, 16, 24));
            let envelope = [
                (i32::from(map.dimensions[0]) + 2) * 128,
                (i32::from(map.dimensions[1]) + 2) * 64,
            ];
            for y in (24..184).step_by(4) {
                for x in (16..384).step_by(4) {
                    let world = [(x - 16) * envelope[0] / 368, (y - 24) * envelope[1] / 160];
                    let color = if map.discovered(world) {
                        source.map_color(map.level, world)?
                    } else {
                        Color::rgb(28, 32, 40)
                    };
                    rect(output, [x, y, 4, 4], color);
                }
            }
            for marker in map.markers.iter().flatten() {
                if !map.discovered(marker.position) {
                    continue;
                }
                let x = 16 + marker.position[0] * 368 / envelope[0];
                let y = 24 + marker.position[1] * 160 / envelope[1];
                rect(output, [x - 2, y - 2, 5, 5], Color::rgb(255, 255, 255));
                if let Some(seat) = marker.seat {
                    number(output, x + 4, y - 3, u32::from(seat.get()) + 1);
                }
            }
            text(output, 16, 12, b"MAP", Color::rgb(240, 240, 240));
        }
        if let Some(inventory) = &self.inventory {
            rect(output, [12, 12, 376, 176], Color::rgb(16, 20, 32));
            text(
                output,
                20,
                20,
                match inventory.kind {
                    InventoryKind::Inventory => b"INVENTORY",
                    InventoryKind::MailOrder => b"MAIL ORDER",
                    InventoryKind::Identify => b"IDENTIFY",
                },
                Color::rgb(240, 240, 240),
            );
            for (index, item) in inventory.slots.iter().enumerate() {
                let x = 20 + (index % 8) as i32 * 46;
                let y = 40 + (index / 8) as i32 * 68;
                rect(
                    output,
                    [x - 2, y - 2, 42, 60],
                    if index == usize::from(inventory.selected) {
                        Color::rgb(220, 200, 80)
                    } else {
                        Color::rgb(48, 56, 72)
                    },
                );
                rect(output, [x, y, 38, 56], Color::rgb(12, 16, 24));
                if let Some(item) = item {
                    if item.identified {
                        number(output, x + 10, y + 40, u32::from(item.wrapper) + 1);
                    } else {
                        text(output, x + 10, y + 40, b"??", Color::rgb(240, 240, 240));
                    }
                }
            }
        }
        if let Some(hud) = self.hud {
            rect(output, [0, 198, 400, 26], Color::rgb(8, 12, 20));
            number(output, 6, 203, u32::from(hud.seat.get()) + 1);
            text(output, 20, 203, b"HP", Color::rgb(230, 230, 230));
            rect(output, [36, 202, 100, 8], Color::rgb(64, 24, 32));
            rect(
                output,
                [
                    36,
                    202,
                    i32::from(hud.health) * 100 / i32::from(hud.maximum_health),
                    8,
                ],
                Color::rgb(64, 220, 112),
            );
            text(output, 146, 203, b"LIVES", Color::rgb(230, 230, 230));
            number(output, 182, 203, u32::from(hud.lives));
            text(output, 212, 203, b"LV", Color::rgb(230, 230, 230));
            number(output, 230, 203, u32::from(hud.level.get()));
            text(output, 266, 203, b"$", Color::rgb(230, 230, 230));
            number(output, 276, 203, u32::from(hud.bucks));
            text(output, 326, 203, b"R", Color::rgb(230, 230, 230));
            number(output, 338, 203, u32::from(hud.rank));
            if hud.pause_requested {
                text(
                    output,
                    150,
                    215,
                    b"PAUSE REQUESTED",
                    Color::rgb(250, 220, 100),
                );
            }
        }
        if self.speech.length != 0 {
            rect(output, [16, 140, 368, 52], Color::rgb(236, 236, 220));
            let mut x = 22;
            let mut y = 146;
            for &byte in &self.speech.text[..usize::from(self.speech.length)] {
                if byte == b'\n' || x > 374 {
                    x = 22;
                    y += 9;
                    if byte == b'\n' {
                        continue;
                    }
                }
                text(output, x, y, &[byte], Color::rgb(12, 16, 24));
                x += 6;
            }
            if self.speech.choices {
                text(
                    output,
                    22,
                    180,
                    if self.speech.accept_selected {
                        b">A OK   B NO"
                    } else {
                        b" A OK  >B NO"
                    },
                    Color::rgb(12, 16, 24),
                );
            }
        }
        Ok(())
    }
}
pub(crate) fn rect(output: &mut [Color], bounds: [i32; 4], color: Color) {
    let [x, y, width, height] = bounds;
    for row in y.max(0)..(y + height).min(HEIGHT as i32) {
        let start = row as usize * WIDTH;
        let left = x.clamp(0, WIDTH as i32) as usize;
        let right = (x + width).clamp(0, WIDTH as i32) as usize;
        if left < right {
            output[start + left..start + right].fill(color);
        }
    }
}
pub(crate) fn number(output: &mut [Color], x: i32, y: i32, mut value: u32) {
    let mut digits = [b'0'; 10];
    let mut start = 9;
    while value >= 10 {
        digits[start] = b'0' + (value % 10) as u8;
        value /= 10;
        start -= 1;
    }
    digits[start] = b'0' + value as u8;
    text(output, x, y, &digits[start..], Color::rgb(255, 255, 255));
}
pub(crate) fn text(output: &mut [Color], x: i32, y: i32, bytes: &[u8], color: Color) {
    for (column, byte) in bytes.iter().enumerate() {
        for (row, bits) in glyph(byte.to_ascii_uppercase()).iter().enumerate() {
            for (bit, pixel) in bits.bytes().enumerate() {
                if pixel == b'#' {
                    rect(
                        output,
                        [x + column as i32 * 6 + bit as i32, y + row as i32, 1, 1],
                        color,
                    );
                }
            }
        }
    }
}
// Newly authored compact host UI font, not cartridge glyph data.
fn glyph(byte: u8) -> &'static [&'static str; 7] {
    match byte {
        b'A' => &[
            ".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
        b'B' => &[
            "####.", "#...#", "#...#", "####.", "#...#", "#...#", "####.",
        ],
        b'C' => &[
            ".###.", "#...#", "#....", "#....", "#....", "#...#", ".###.",
        ],
        b'D' => &[
            "####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####.",
        ],
        b'E' => &[
            "#####", "#....", "#....", "####.", "#....", "#....", "#####",
        ],
        b'F' => &[
            "#####", "#....", "#....", "####.", "#....", "#....", "#....",
        ],
        b'G' => &[
            ".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".###.",
        ],
        b'H' => &[
            "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
        b'I' => &[
            ".###.", "..#..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ],
        b'J' => &[
            "..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##..",
        ],
        b'K' => &[
            "#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#",
        ],
        b'L' => &[
            "#....", "#....", "#....", "#....", "#....", "#....", "#####",
        ],
        b'M' => &[
            "#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#",
        ],
        b'N' => &[
            "#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#",
        ],
        b'O' => &[
            ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
        b'P' => &[
            "####.", "#...#", "#...#", "####.", "#....", "#....", "#....",
        ],
        b'Q' => &[
            ".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#",
        ],
        b'R' => &[
            "####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#",
        ],
        b'S' => &[
            ".####", "#....", "#....", ".###.", "....#", "....#", "####.",
        ],
        b'T' => &[
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..",
        ],
        b'U' => &[
            "#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
        b'V' => &[
            "#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#..",
        ],
        b'W' => &[
            "#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#",
        ],
        b'X' => &[
            "#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#",
        ],
        b'Y' => &[
            "#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#..",
        ],
        b'Z' => &[
            "#####", "....#", "...#.", "..#..", ".#...", "#....", "#####",
        ],
        b'0' => &[
            ".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###.",
        ],
        b'1' => &[
            "..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ],
        b'2' => &[
            ".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####",
        ],
        b'3' => &[
            "####.", "....#", "....#", ".###.", "....#", "....#", "####.",
        ],
        b'4' => &[
            "...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.",
        ],
        b'5' => &[
            "#####", "#....", "#....", "####.", "....#", "....#", "####.",
        ],
        b'6' => &[
            ".###.", "#....", "#....", "####.", "#...#", "#...#", ".###.",
        ],
        b'7' => &[
            "#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#...",
        ],
        b'8' => &[
            ".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.",
        ],
        b'9' => &[
            ".###.", "#...#", "#...#", ".####", "....#", "....#", ".###.",
        ],
        b'?' => &[
            ".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#..",
        ],
        b'!' => &[
            "..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#..",
        ],
        b'.' => &[
            ".....", ".....", ".....", ".....", ".....", ".....", "..#..",
        ],
        b',' => &[
            ".....", ".....", ".....", ".....", ".....", "..#..", ".#...",
        ],
        b':' => &[
            ".....", "..#..", ".....", ".....", "..#..", ".....", ".....",
        ],
        b'\'' => &[
            "..#..", "..#..", ".....", ".....", ".....", ".....", ".....",
        ],
        b'>' => &[
            "#....", ".#...", "..#..", "...#.", "..#..", ".#...", "#....",
        ],
        b'-' => &[
            ".....", ".....", ".....", "#####", ".....", ".....", ".....",
        ],
        b'/' => &[
            "....#", "...#.", "...#.", "..#..", ".#...", ".#...", "#....",
        ],
        b'$' => &[
            "..#..", ".####", "#.#..", ".###.", "..#.#", "####.", "..#..",
        ],
        b' ' => &[
            ".....", ".....", ".....", ".....", ".....", ".....", ".....",
        ],
        _ => &[
            "#####", "#...#", "..#.#", "..#..", "..#..", ".....", "..#..",
        ],
    }
}
