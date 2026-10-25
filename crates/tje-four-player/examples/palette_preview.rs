//! Writes a private runtime-derived BMP contact sheet, never a gameplay claim.
//! Usage: cargo run -p tje-four-player --example palette_preview -- ROM OUTPUT.bmp
use std::{error::Error, io::Write, path::PathBuf};
use tje_four_player::{Character, palette::RuntimePalettes};
use tje_rom::Rom;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let source = PathBuf::from(args.next().ok_or("missing private ROM path")?);
    let destination = PathBuf::from(args.next().ok_or("missing private output path")?);
    if args.next().is_some() {
        return Err("expected ROM OUTPUT.bmp".into());
    }
    let rom = Rom::import(&std::fs::read(source)?)?;
    let palettes = RuntimePalettes::load(&rom)?;
    const WIDTH: usize = 1056;
    const HEIGHT: usize = 128;
    let mut pixels = vec![0; WIDTH * HEIGHT * 4];
    // Authored tiny seat-number glyphs, not the cartridge font.
    const DIGITS: [[u8; 7]; 4] = [
        [4, 12, 4, 4, 4, 4, 14],
        [14, 17, 1, 2, 4, 8, 31],
        [30, 1, 1, 14, 1, 1, 30],
        [2, 6, 10, 18, 31, 2, 2],
    ];
    for (seat, (digit, character)) in DIGITS.iter().zip(Character::DEFAULT_ROSTER).enumerate() {
        for y in 0..32 {
            for x in 0..WIDTH {
                let color = if x < 32 {
                    let gx = x / 3;
                    let gy = y / 4;
                    if gx < 5 && gy < 7 && digit[gy] & (1 << (4 - gx)) != 0 {
                        [255; 4]
                    } else {
                        [24, 24, 24, 255]
                    }
                } else {
                    let cell = (x - 32) / 16;
                    let bank = (cell / 16) as u8;
                    let index = (cell % 16) as u8;
                    let color = palettes
                        .color(character, bank, index)
                        .ok_or("invalid palette sample")?;
                    if color[3] == 0 {
                        let shade = if ((x / 4) + (y / 4)) % 2 == 0 {
                            64
                        } else {
                            112
                        };
                        [shade, shade, shade, 255]
                    } else {
                        color
                    }
                };
                let offset = ((HEIGHT - 1 - (seat * 32 + y)) * WIDTH + x) * 4;
                pixels[offset..offset + 4].copy_from_slice(&[color[2], color[1], color[0], 0]);
            }
        }
    }
    let mut file = std::fs::File::create(destination)?;
    file.write_all(b"BM")?;
    file.write_all(&(54 + pixels.len() as u32).to_le_bytes())?;
    file.write_all(&[0; 4])?;
    file.write_all(&54_u32.to_le_bytes())?;
    file.write_all(&40_u32.to_le_bytes())?;
    file.write_all(&(WIDTH as i32).to_le_bytes())?;
    file.write_all(&(HEIGHT as i32).to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&32_u16.to_le_bytes())?;
    file.write_all(&[0; 24])?;
    file.write_all(&pixels)?;
    println!(
        "{} runtime palette preview written (four rows, ROM piece banks 0..3; not gameplay)",
        rom.revision()
    );
    Ok(())
}
