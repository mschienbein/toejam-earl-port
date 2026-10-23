//! Assembly decoders, including residual register halves, DBRA counts, and CCR.
use crate::{Context, Result};
use tje_mem::{Address, RenderPiece, ram};

fn output_base(context: &dyn Context) -> (u32, u16) {
    let revision = context.revision();
    let offset = ram::g_sprite_tile_buffer_offset(revision).read(context);
    (
        ram::g_sprite_tile_upload_buffer(revision)
            .address()
            .wrapping_add(offset as i16 as i32 as u32),
        offset,
    )
}

fn read_next(context: &dyn Context, source: &mut u32) -> u8 {
    let value = context.read_u8(*source);
    *source = source.wrapping_add(1);
    value
}

fn emit(context: &mut dyn Context, destination: &mut u32, stride: u32, value: u8) {
    context.write_u8(*destination, value);
    *destination = destination.wrapping_add(stride);
}

fn replace_word(context: &mut dyn Context, register: usize, value: u16) {
    let high = context.data_register(register) & 0xffff_0000;
    context.set_data_register(register, high | u32::from(value));
}

/// G/src/asm/rle.s:22, SpriteUnpackInterleavedRLE.
/// Each lane consumes its own signed word quota. Zero-token DBRA underflow is not corrected.
pub fn sprite_unpack_interleaved_rle(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let piece = Address::<RenderPiece>::new(arguments[0]);
    let quota = (arguments[1] as u16).wrapping_shl(3);
    let mut source = piece.tile_data().read(context).address();
    let (base, _) = output_base(context);
    let mut remaining = quota;
    for lane in 0..4 {
        let mut destination = base.wrapping_add(lane);
        remaining = quota;
        while remaining as i16 > 0 {
            let token = read_next(context, &mut source);
            let count;
            if (token as i8) < 0 {
                let value = if token & 0x40 != 0 {
                    read_next(context, &mut source)
                } else {
                    0
                };
                count = (i16::from((token | 0x40) as i8)).wrapping_neg() as u16;
                for _ in 0..u32::from(count.wrapping_sub(1)) + 1 {
                    emit(context, &mut destination, 4, value);
                }
            } else if token & 0x40 == 0 {
                count = u16::from(token);
                for _ in 0..u32::from(count.wrapping_sub(1)) + 1 {
                    let value = read_next(context, &mut source);
                    emit(context, &mut destination, 4, value);
                }
            } else if token & 0x20 != 0 {
                let value = read_next(context, &mut source);
                let swapped = value.rotate_left(4);
                count = u16::from(token & 0x1f);
                for _ in 0..u32::from((count >> 1).wrapping_sub(1)) + 1 {
                    emit(context, &mut destination, 4, value);
                    emit(context, &mut destination, 4, swapped);
                }
                if count & 1 != 0 {
                    emit(context, &mut destination, 4, value);
                }
            } else {
                count = u16::from(token & 0x1f);
                for _ in 0..u32::from(count.wrapping_sub(1)) + 1 {
                    emit(context, &mut destination, 4, 0xff);
                }
            }
            remaining = remaining.wrapping_sub(count);
        }
    }
    replace_word(context, 0, 4);
    replace_word(context, 1, remaining);
    context.set_address_register(0, base.wrapping_add(4));
    context.set_address_register(1, source);
    // The final ADDQ.W 3->4 clears X; CMP.W #4 then sets only Z.
    context.set_status_register((context.status_register() & !0x1f) | 4);
    Ok(None)
}

/// G/src/asm/rle.s:139, SpriteUnpackRLE.
/// Restore D2-D4 and retain D0; D1's upper word survives every token operation.
pub fn sprite_unpack_rle(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let piece = Address::<RenderPiece>::new(arguments[0]);
    let count = arguments[1] as u16;
    let mut source = piece.tile_data().read(context).address();
    let (mut destination, offset) = output_base(context);
    let mut remaining = count.wrapping_shl(5);
    let mut d1_word = offset;
    let mut extend = count & 0x0800 != 0;
    while remaining as i16 > 0 {
        let token = read_next(context, &mut source);
        let run;
        if (token as i8) < 0 {
            let value = if token & 0x40 != 0 {
                read_next(context, &mut source)
            } else {
                0
            };
            run = (i16::from((token | 0x40) as i8)).wrapping_neg() as u16;
            for _ in 0..u32::from(run.wrapping_sub(1)) + 1 {
                emit(context, &mut destination, 1, value);
            }
        } else if token & 0x40 != 0 {
            run = u16::from(token & 0x3f);
            for _ in 0..u32::from(run.wrapping_sub(1)) + 1 {
                emit(context, &mut destination, 1, 0xff);
            }
        } else {
            run = u16::from(token);
            for _ in 0..u32::from(run.wrapping_sub(1)) + 1 {
                let value = read_next(context, &mut source);
                emit(context, &mut destination, 1, value);
            }
        }
        d1_word = run;
        extend = remaining < run;
        remaining = remaining.wrapping_sub(run);
    }
    replace_word(context, 1, d1_word);
    context.set_address_register(0, destination);
    context.set_address_register(1, source);
    let flags = if remaining == 0 {
        4
    } else if remaining & 0x8000 != 0 {
        8
    } else {
        0
    };
    context.set_status_register(
        (context.status_register() & !0x1f) | flags | if extend { 0x10 } else { 0 },
    );
    Ok(None)
}

/// G/src/asm/rle.s:210, SpriteUnpackRaw.
/// A zero tile count executes 65,536 tiles because the DBRA body precedes its test.
pub fn sprite_unpack_raw(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let piece = Address::<RenderPiece>::new(arguments[0]);
    let count = arguments[1] as u16;
    let mut source = piece.tile_data().read(context).address();
    let (mut destination, offset) = output_base(context);
    let mut last = 0;
    for _ in 0..u32::from(count.wrapping_sub(1)) + 1 {
        for _ in 0..8 {
            last = context.read_u32(source);
            context.write_u32(destination, last);
            source = source.wrapping_add(4);
            destination = destination.wrapping_add(4);
        }
    }
    replace_word(context, 0, 0xffff);
    replace_word(context, 1, offset);
    context.set_address_register(0, destination);
    context.set_address_register(1, source);
    let flags = if last == 0 {
        4
    } else if last & 0x8000_0000 != 0 {
        8
    } else {
        0
    };
    context.set_status_register(
        (context.status_register() & !0x1f) | flags | if count == 0 { 0x10 } else { 0 },
    );
    Ok(None)
}
