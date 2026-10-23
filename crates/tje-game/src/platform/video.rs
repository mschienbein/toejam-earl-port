//! Ordered Functional VDP transactions; no clock/readiness shortcuts in game code.
use crate::{Context, Result};
use tje_mem::ram;

fn count_words(value: u32) -> u32 {
    let low = u32::from(value as u16);
    if low == 0 { 0x1_0000 } else { low }
}

fn move_ccr(context: &mut dyn Context, value: u32, sign_bit: u32, extend: bool) {
    let nz = if value == 0 {
        4
    } else if value & sign_bit != 0 {
        8
    } else {
        0
    };
    context
        .set_status_register((context.status_register() & !0x1f) | nz | (u16::from(extend) << 4));
}

fn clear(context: &mut dyn Context, command: u32, count: u32) -> Result<Option<u32>> {
    let extend = context.status_register() & 0x10 != 0;
    context.write_vdp_command(command);
    for _ in 0..count {
        context.write_vdp_data(0);
    }
    context.set_data_register(0, (context.data_register(0) & 0xffff_0000) | 0xffff);
    move_ccr(context, 0, 0x8000, extend);
    Ok(None)
}

/// G/src/asm/vdp.s:20 (`ClearVideoRAM`): 32,768 word stores, residual D0 high word.
pub fn clear_video_ram(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    clear(context, 0x4000_0000, 0x8000)
}

/// G/src/asm/vdp.s:32 (`ClearVerticalScrollRam`): forty VSRAM words, not save RAM.
pub fn clear_vertical_scroll_ram(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    clear(context, 0x4000_0010, 40)
}

/// G/src/asm/vdp.s:44 (`ClearColourRAM`): sixty-four CRAM word stores.
pub fn clear_colour_ram(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    clear(context, 0xc000_0000, 64)
}

/// G/src/asm/vdp.s:58 (`CopyLongwordRunToVramTile`): inclusive low-word DBRA,
/// ordered longword port aliases, and the count shift's surviving X flag.
pub fn copy_longword_run_to_vram_tile(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let shifted = arguments[1] << 7;
    let shifted = (shifted & 0xffff_0000) | u32::from((shifted as u16) >> 2);
    let command = shifted.rotate_left(16) | 0x4000_0000;
    context.write_vdp_command(command);
    let counter = arguments[2] << 3;
    let extend = arguments[2] & 0x2000_0000 != 0;
    let mut source = arguments[0];
    let mut value = 0;
    for _ in 0..=u32::from(counter as u16) {
        value = context.read_u32(source);
        source = source.wrapping_add(4);
        context.write_device_u16(0xc00000, (value >> 16) as u16);
        context.write_device_u16(0xc00002, value as u16);
    }
    context.set_data_register(0, command);
    context.set_data_register(1, (counter & 0xffff_0000) | 0xffff);
    context.set_address_register(0, source);
    move_ccr(context, value, 0x8000_0000, extend);
    Ok(None)
}

/// G/src/asm/vdp.s:77 (`FillVdpDataWords`): count zero performs 65,536 writes.
pub fn fill_vdp_data_words(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let value = arguments[0];
    let count = arguments[1];
    for _ in 0..count_words(count) {
        context.write_vdp_data(value as u16);
    }
    context.set_data_register(0, value);
    context.set_data_register(1, (count & 0xffff_0000) | 0xffff);
    move_ccr(context, u32::from(value as u16), 0x8000, count as u16 == 0);
    Ok(None)
}

/// G/src/asm/vdp.s:90 (`FillVdpDataBytes`): preserve byte-width VDP writes.
pub fn fill_vdp_data_bytes(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let value = arguments[0];
    let count = arguments[1];
    for _ in 0..count_words(count) {
        context.write_device_u8(0xc00000, value as u8);
    }
    context.set_data_register(0, value);
    context.set_data_register(1, (count & 0xffff_0000) | 0xffff);
    move_ccr(context, u32::from(value as u8), 0x80, count as u16 == 0);
    Ok(None)
}

/// G/src/asm/vdp.s:103 (`MoveDataToVRAM`): forward source-word reads and stores;
/// the predecrement's X survives every MOVE and DBRA, including count zero.
pub fn move_data_to_vram(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let count = arguments[1];
    let mut source = arguments[0];
    let mut value = 0;
    for _ in 0..count_words(count) {
        value = context.read_u16(source);
        source = source.wrapping_add(2);
        context.write_vdp_data(value);
    }
    context.set_data_register(0, (count & 0xffff_0000) | 0xffff);
    context.set_address_register(0, source);
    move_ccr(context, u32::from(value), 0x8000, count as u16 == 0);
    Ok(None)
}

/// G/src/asm/vdp.s:117 (`UploadVramWordRectangle`): nested inclusive word counters,
/// wrapping row stride and final MOVE.L restore flags from the saved entry D2.
pub fn upload_vram_word_rectangle(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let saved_d2 = context.data_register(2);
    let mut destination = arguments[0];
    let mut source = arguments[1];
    let width = arguments[2];
    let rows = arguments[3];
    let stride = arguments[4] << 1;
    for _ in 0..count_words(rows) {
        let shifted = destination << 2;
        let shifted = (shifted & 0xffff_0000) | u32::from((shifted as u16) >> 2);
        let command = (shifted.rotate_left(16) | 0x4000_0000) & 0x7fff_0003;
        context.write_vdp_command(command);
        destination = destination.wrapping_add(stride);
        for _ in 0..count_words(width) {
            let value = context.read_u16(source);
            source = source.wrapping_add(2);
            context.write_vdp_data(value);
        }
    }
    context.set_data_register(0, (width & 0xffff_0000) | 0xffff);
    context.set_data_register(1, stride);
    context.set_address_register(0, source);
    context.set_address_register(1, destination);
    move_ccr(context, saved_d2, 0x8000_0000, width as u16 == 0);
    Ok(None)
}

/// G/src/asm/vdp.s:153 (`CopyPaletteToCRAMSlot`): slot masking, sixteen words,
/// and D0's retained packed-command high word after MOVE.W/DBRA.
pub fn copy_palette_to_cram_slot(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let command = (((arguments[1] & 3) << 5) | 0xc000).rotate_left(16);
    context.write_vdp_command(command);
    let mut source = arguments[0];
    let mut value = 0;
    for _ in 0..16 {
        value = context.read_u16(source);
        source = source.wrapping_add(2);
        context.write_vdp_data(value);
    }
    context.set_data_register(0, (command & 0xffff_0000) | 0xffff);
    context.set_address_register(0, source);
    move_ccr(context, u32::from(value), 0x8000, false);
    Ok(None)
}

/// G/src/asm/dma.s:15 (`DmaCopyWordsToVram`): ordered count/source register writes,
/// RAM command publication, split command words, preserved D2–D4, exact D0/D1/CCR.
pub fn dma_copy_words_to_vram(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let source = arguments[0];
    let destination = arguments[1];
    let count = arguments[2];
    let revision = context.revision();
    let mode2 = ram::g_mode2_reg_softcopy(revision).read(context);
    context.write_vdp_control(0x8104 + (u16::from(mode2) | 0x10));
    context.write_vdp_control(0x9300 + (count & 0xff) as u16);
    context.write_vdp_control(0x9400 + ((count >> 8) & 0xff) as u16);
    context.write_vdp_control(0x9500 + ((source >> 1) & 0xff) as u16);
    context.write_vdp_control(0x9600 + ((source >> 9) & 0xff) as u16);
    context.write_vdp_control(0x9700 + ((source >> 17) & 0x7f) as u16);
    let low_command = ((destination & 0x3fff) << 16).wrapping_add(0x4000_0080);
    let command = low_command.wrapping_add((destination & 0xc000) >> 14);
    ram::g_vdp_dma_command(revision)
        .cast::<u32>()
        .write(context, command);
    let high = ram::g_vdp_dma_command(revision).cast::<u16>().read(context);
    context.write_vdp_control(high);
    let low = ram::g_vdp_dma_command_low_word(revision)
        .cast::<u16>()
        .read(context);
    context.write_vdp_control(low);
    context.set_data_register(0, command);
    context.set_data_register(1, low_command);
    // Packed ADD.L cannot carry; the final MOVE.W is in the range 0x0080..0x0083.
    move_ccr(context, u32::from(low), 0x8000, false);
    Ok(None)
}
