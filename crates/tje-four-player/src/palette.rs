//! Per-instance palettes derived at runtime from the user's verified cartridge.
//! Both hero archetypes use ROM-selected piece palette banks; do not assume a
//! spare CRAM slot or rewrite global CRAM to recolor a particular seat.

use crate::{Character, Error, PaletteVariant, Result};
use tje_mem::rom;
use tje_rom::{Revision, Rom};

pub type Color = [u8; 4];
pub type PaletteBanks = [[Color; 16]; 4];

/// Rebuildable presentation data, intentionally NOT encoded in the sidecar.
/// All three variants together occupy 768 bytes and need no heap allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePalettes {
    variants: [PaletteBanks; 3],
}
impl RuntimePalettes {
    pub fn load(rom: &Rom) -> Result<Self> {
        Self::read(rom.bytes(), rom.revision())
    }
    fn read(bytes: &[u8], revision: Revision) -> Result<Self> {
        let addresses = [
            rom::palette_game_line1(revision).address(),
            rom::palette_game_line2(revision).address(),
            rom::palette_game_line3(revision).address(),
            rom::palette_game_line4(revision).address(),
        ];
        let mut words = [[0; 16]; 4];
        for (bank, address) in words.iter_mut().zip(addresses) {
            let bytes =
                bytes
                    .get(address as usize..address as usize + 32)
                    .ok_or(Error::Invalid(
                        "cartridge is missing a complete gameplay palette bank",
                    ))?;
            for (word, source) in bank.iter_mut().zip(bytes.as_chunks::<2>().0) {
                *word = u16::from_be_bytes(*source);
            }
        }
        Ok(Self::from_words(words))
    }
    fn from_words(words: [[u16; 16]; 4]) -> Self {
        let variants = [
            PaletteVariant::Original,
            PaletteVariant::RotateForward,
            PaletteVariant::RotateBackward,
        ]
        .map(|variant| {
            std::array::from_fn(|bank| {
                std::array::from_fn(|index| decode(words[bank][index], index == 0, variant))
            })
        });
        Self { variants }
    }
    pub fn for_character(&self, character: Character) -> &PaletteBanks {
        &self.variants[match character.palette {
            PaletteVariant::Original => 0,
            PaletteVariant::RotateForward => 1,
            PaletteVariant::RotateBackward => 2,
        }]
    }
    /// Bank is the ROM piece's format&3; color index zero stays transparent in
    /// every bank. An invalid resource/index is rejected, never masked to fit.
    pub fn color(&self, character: Character, bank: u8, index: u8) -> Option<Color> {
        self.for_character(character)
            .get(usize::from(bank))?
            .get(usize::from(index))
            .copied()
    }
}

fn decode(word: u16, transparent: bool, variant: PaletteVariant) -> Color {
    let rgb = [
        ((word >> 1) & 7) as u8,
        ((word >> 5) & 7) as u8,
        ((word >> 9) & 7) as u8,
    ];
    let rgb = transform(rgb, transparent, variant);
    let expand = |component: u8| (u16::from(component) * 255 / 7) as u8;
    [
        expand(rgb[0]),
        expand(rgb[1]),
        expand(rgb[2]),
        if transparent { 0 } else { 255 },
    ]
}
fn transform(rgb: [u8; 3], transparent: bool, variant: PaletteVariant) -> [u8; 3] {
    let maximum = rgb[0].max(rgb[1]).max(rgb[2]);
    let minimum = rgb[0].min(rgb[1]).min(rgb[2]);
    // Preserve dark outlines and achromatic/near-neutral highlights. The cyclic
    // transform preserves every channel's range, ordering contrast and sum;
    // it does not promise identical perceptual luminance.
    if transparent || maximum <= 1 || maximum - minimum <= 1 {
        return rgb;
    }
    match variant {
        PaletteVariant::Original => rgb,
        PaletteVariant::RotateForward => [rgb[2], rgb[0], rgb[1]],
        PaletteVariant::RotateBackward => [rgb[1], rgb[2], rgb[0]],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Archetype, SeatId};

    #[test]
    fn every_genesis_color_preserves_transparency_neutrals_and_contrast_range() {
        for r in 0..8 {
            for g in 0..8 {
                for b in 0..8 {
                    let source = [r, g, b];
                    let maximum = r.max(g).max(b);
                    let minimum = r.min(g).min(b);
                    for variant in [
                        PaletteVariant::Original,
                        PaletteVariant::RotateForward,
                        PaletteVariant::RotateBackward,
                    ] {
                        let transformed = transform(source, false, variant);
                        assert_eq!(transformed.iter().sum::<u8>(), source.iter().sum::<u8>());
                        assert_eq!(transformed.iter().max(), source.iter().max());
                        assert_eq!(transformed.iter().min(), source.iter().min());
                        assert_eq!(transform(source, true, variant), source);
                        if maximum <= 1 || maximum - minimum <= 1 {
                            assert_eq!(transformed, source);
                        }
                    }
                    let forward = transform(source, false, PaletteVariant::RotateForward);
                    assert_eq!(
                        transform(forward, false, PaletteVariant::RotateBackward),
                        source
                    );
                }
            }
        }
    }
    #[test]
    fn variants_are_per_character_instances_not_global_cram_assignments() {
        // Authored synthetic colors, not an extracted game palette.
        let words = std::array::from_fn(|bank| {
            std::array::from_fn(|index| {
                let r = (index & 7) as u16;
                let g = (bank & 7) as u16;
                (r << 1) | (g << 5) | (2 << 9)
            })
        });
        let palettes = RuntimePalettes::from_words(words);
        assert_eq!(std::mem::size_of::<RuntimePalettes>(), 768);
        let roster = Character::DEFAULT_ROSTER;
        for seat in SeatId::ALL {
            for bank in 0..4 {
                assert_eq!(palettes.color(roster[seat.index()], bank, 0).unwrap()[3], 0);
                for index in 1..16 {
                    assert_eq!(
                        palettes.color(roster[seat.index()], bank, index).unwrap()[3],
                        255
                    );
                }
            }
        }
        assert_ne!(
            palettes.for_character(roster[0]),
            palettes.for_character(roster[2])
        );
        assert_ne!(
            palettes.for_character(roster[1]),
            palettes.for_character(roster[3])
        );
        let mut earl = roster[2];
        earl.archetype = Archetype::Earl;
        assert_eq!(
            palettes.for_character(earl),
            palettes.for_character(roster[2])
        );
        assert!(palettes.color(roster[0], 4, 1).is_none());
        assert!(palettes.color(roster[0], 0, 16).is_none());
    }
    #[test]
    fn both_revision_addresses_read_runtime_data_and_reject_truncation() {
        for revision in Revision::ALL {
            assert!(RuntimePalettes::read(&[], revision).is_err());
            let mut bytes = vec![0; revision.rom_size()];
            for address in [
                rom::palette_game_line1(revision).address(),
                rom::palette_game_line2(revision).address(),
                rom::palette_game_line3(revision).address(),
                rom::palette_game_line4(revision).address(),
            ] {
                for index in 0..16_u32 {
                    let offset = (address + 2 * index) as usize;
                    let word = ((index & 7) << 1) as u16;
                    bytes[offset..offset + 2].copy_from_slice(&word.to_be_bytes());
                }
            }
            let palettes = RuntimePalettes::read(&bytes, revision).unwrap();
            assert_eq!(
                palettes.color(Character::DEFAULT_ROSTER[0], 0, 7),
                Some([255, 0, 0, 255])
            );
            assert_eq!(
                palettes.color(Character::DEFAULT_ROSTER[2], 0, 7),
                Some([0, 255, 0, 255])
            );
            assert_eq!(
                palettes.color(Character::DEFAULT_ROSTER[3], 0, 7),
                Some([0, 0, 255, 255])
            );
            let last = rom::palette_game_line4(revision).address() as usize;
            assert!(RuntimePalettes::read(&bytes[..last + 31], revision).is_err());
        }
    }
}
