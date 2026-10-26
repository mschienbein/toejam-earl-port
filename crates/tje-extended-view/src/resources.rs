//! Bounded cartridge resource decoding shared by stock and owned-world sources.
use crate::{RenderError, presentation::Piece};
use jgenesis_common::frontend::Color;

pub const MAX_PIECE_BYTES: usize = 4 * 4 * 32;

/// Validates a ROM frame's variable-length piece table before exposing it.
pub fn frame_pieces(
    rom: &[u8],
    address: u32,
    output: &mut [Piece; 12],
) -> Result<usize, RenderError> {
    let frame = rom
        .get(address as usize..)
        .ok_or(RenderError::InvalidSprite("truncated frame"))?;
    let prefix = frame
        .get(..4)
        .ok_or(RenderError::InvalidSprite("truncated frame"))?;
    let count = usize::from(prefix[0]);
    if count == 0 || count > output.len() {
        return Err(RenderError::InvalidSprite("frame piece count exceeds 12"));
    }
    let table = frame
        .get(4..4 + count * 10)
        .ok_or(RenderError::InvalidSprite("truncated frame pieces"))?;
    for (target, bytes) in output.iter_mut().zip(table.as_chunks::<10>().0) {
        *target = Piece {
            width: bytes[0],
            height: bytes[1],
            x: bytes[2] as i8,
            y: bytes[3] as i8,
            flipped_x: bytes[4] as i8,
            format: bytes[5] as i8,
            source: u32::from_be_bytes(bytes[6..10].try_into().unwrap()),
        };
        piece_length(*target)?;
    }
    Ok(count)
}

pub fn piece_length(piece: Piece) -> Result<usize, RenderError> {
    if !(1..=4).contains(&piece.width) || !(1..=4).contains(&piece.height) {
        return Err(RenderError::InvalidSprite(
            "piece dimensions exceed 4x4 tiles",
        ));
    }
    Ok(usize::from(piece.width) * usize::from(piece.height) * 32)
}

/// Same electrical DAC as the VDP, including shadow/highlight levels.
pub fn palette_color(cram: &[u16], index: usize, modifier: i8) -> Color {
    color_word(cram[index & 63], modifier)
}

pub fn color_word(color: u16, modifier: i8) -> Color {
    let channel = match modifier {
        -1 => [0, 29, 52, 70, 87, 101, 116, 130],
        1 => [130, 144, 158, 172, 187, 206, 228, 255],
        _ => [0, 52, 87, 116, 144, 172, 206, 255],
    };
    Color::rgb(
        channel[((color >> 1) & 7) as usize],
        channel[((color >> 5) & 7) as usize],
        channel[((color >> 9) & 7) as usize],
    )
}

pub fn decode_piece(rom: &[u8], piece: Piece) -> Result<Vec<u8>, RenderError> {
    let mut output = vec![0; piece_length(piece)?];
    decode_piece_into(rom, piece, &mut output)?;
    Ok(output)
}

/// Decodes into caller-owned scratch. Tile reuse is a cache choice, never an
/// animation/motion decision. No stream can write beyond one 4x4-tile piece.
pub fn decode_piece_into(rom: &[u8], piece: Piece, output: &mut [u8]) -> Result<(), RenderError> {
    let length = piece_length(piece)?;
    if output.len() != length {
        return Err(RenderError::OutputLength {
            expected: length,
            actual: output.len(),
        });
    }
    let mut source = piece.source as usize;
    let next = |source: &mut usize| -> Result<u8, RenderError> {
        let value = *rom
            .get(*source)
            .ok_or(RenderError::InvalidSprite("truncated piece stream"))?;
        *source += 1;
        Ok(value)
    };
    if piece.format >= 0 && piece.format & 0x40 != 0 {
        let data = rom
            .get(source..)
            .and_then(|data| data.get(..length))
            .ok_or(RenderError::InvalidSprite("truncated raw tiles"))?;
        output.copy_from_slice(data);
        return Ok(());
    }
    let lanes = if piece.format < 0 { 4 } else { 1 };
    for lane in 0..lanes {
        let mut produced = 0;
        let quota = length / lanes;
        while produced < quota {
            let token = next(&mut source)?;
            let count = if token & 0x80 != 0 {
                usize::from((token | 0x40).wrapping_neg())
            } else if token & 0x40 != 0 {
                usize::from(token & if lanes == 4 { 0x1f } else { 0x3f })
            } else {
                usize::from(token)
            };
            // The cartridge emits the whole run before checking its signed
            // remaining quota (G/src/asm/rle.s:119-124,196-201). Some authored
            // pieces deliberately exhaust a lane part-way through that run.
            // Consume every literal/value byte, but retain only this piece's
            // declared tile window rather than reproducing staging-buffer spill.
            // Zero runs and alternating count one cause DBRA underflow in the
            // original; they are not bounded resources this host can admit.
            if count == 0 || (lanes == 4 && token & 0xe0 == 0x60 && count == 1) {
                return Err(RenderError::InvalidSprite("invalid RLE run"));
            }
            let repeated = if token & 0x80 != 0 {
                Some(if token & 0x40 != 0 {
                    next(&mut source)?
                } else {
                    0
                })
            } else if token & 0x40 != 0 {
                Some(if lanes == 4 && token & 0x20 != 0 {
                    next(&mut source)?
                } else {
                    0xff
                })
            } else {
                None
            };
            for i in 0..count {
                let value = match repeated {
                    Some(value) if lanes == 4 && token & 0xe0 == 0x60 && i & 1 != 0 => {
                        value.rotate_left(4)
                    }
                    Some(value) => value,
                    None => next(&mut source)?,
                };
                if produced + i < quota {
                    output[lane + (produced + i) * lanes] = value;
                }
            }
            produced += count;
        }
    }
    Ok(())
}
