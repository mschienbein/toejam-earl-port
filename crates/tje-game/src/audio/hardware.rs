//! Functional logic translations of the original FM entries.
//! Poll operands remain real device reads. A busy timed device needs the platform
//! wait scheduler; the Functional checker quotients out only its fixed latency.

use crate::{
    Context, Result,
    execution::{WaitCondition, WaitRequest},
};
use tje_mem::{Address, ram, rom_symbol};

fn symbol(context: &dyn Context, name: &str) -> u32 {
    rom_symbol(context.revision(), name)
        .expect("audio ROM symbol is present in both revision catalogues")
        .address
}

pub(super) fn compare_word(context: &mut dyn Context, left: u16, right: u16) {
    let difference = left.wrapping_sub(right);
    let flags = (u16::from(difference & 0x8000 != 0) << 3)
        | (u16::from(difference == 0) << 2)
        | (u16::from((left ^ right) & (left ^ difference) & 0x8000 != 0) << 1)
        | u16::from(left < right);
    context.set_status_register((context.status_register() & !0x0f) | flags);
}

fn select_ym(context: &mut dyn Context, address: u32, register: u8) -> Result<()> {
    while context.read_device_u8(address) & 0x80 != 0 {
        context.wait_until(WaitRequest::new(WaitCondition::DeviceByte {
            address,
            mask: 0x80,
            value: 0,
        }))?;
    }
    context.write_device_u8(address, register);
    Ok(())
}

fn finish_bus_release(context: &mut dyn Context, borrow: bool) {
    context.write_z80_bus_request(0);
    context.set_status_register(
        (context.status_register() & !0x1f) | 4 | if borrow { 0x10 } else { 0 },
    );
}

/// G/src/asm/sound.s:670 (`AudioWriteFmChannelReleaseMask`, REV 02 0x03ec74).
/// Retain signed word channel indexing, byte-only register changes, DAC pacing,
/// real YM status reads, the redundant DAC-latch restore, and final CCR residue.
pub fn audio_write_fm_channel_release_mask(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let mut channel = context.data_register(1);
    if channel as i16 >= 6 {
        compare_word(context, channel as u16, 6);
        return Ok(None);
    }
    let masks = ram::g_fm_channel_key_masks(context.revision()).address();
    let mask = context.read_u8(masks.wrapping_add_signed(i32::from(channel as i16)));
    let operators = mask.wrapping_shl(4);
    context.set_data_register(3, (context.data_register(3) & !255) | u32::from(operators));
    context.set_address_register(5, masks);
    if channel as i16 >= 3 {
        channel = (channel & !255) | u32::from((channel as u8).wrapping_add(1));
    }
    channel = (channel & !255) | u32::from((channel as u8 & 7) | operators);
    context.set_data_register(1, channel);
    context.write_z80_bus_request(0x100);
    let delay = context.read_z80_ram(0x004a);
    context.write_z80_ram(0x004a, delay.wrapping_sub(1));
    select_ym(context, 0xa04000, 0x28)?;
    context.write_ym2612(1, channel as u8);
    context.write_ym2612(0, 0x2a);
    finish_bus_release(context, delay == 0);
    Ok(None)
}

/// G/src/asm/sound.s:709 (`YM2612UpdateFrequency`, REV 02 0x03ecda).
/// Write the selected bank's frequency pair without modifying D4/D5; restore the
/// bank-zero DAC latch even when the frequency used bank one.
pub fn ym2612_update_frequency(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let mut channel = context.data_register(1);
    if channel as i16 >= 6 {
        compare_word(context, channel as u16, 6);
        return Ok(None);
    }
    let port = if channel as i16 >= 3 {
        channel = (channel & !0xffff) | u32::from((channel as u16).wrapping_sub(3));
        0xa04002
    } else {
        0xa04000
    };
    context.set_data_register(1, channel);
    context.set_address_register(5, port);
    context.write_z80_bus_request(0x100);
    let delay = context.read_z80_ram(0x004a);
    context.write_z80_ram(0x004a, delay.wrapping_sub(1));
    for (register, data) in [(0xa4, 4), (0xa0, 5)] {
        let register = register | channel as u8;
        context.set_data_register(7, (context.data_register(7) & !255) | u32::from(register));
        select_ym(context, port, register)?;
        context.write_device_u8(port + 1, context.data_register(data) as u8);
    }
    context.write_ym2612(0, 0x2a);
    finish_bus_release(context, delay == 0);
    Ok(None)
}

/// G/src/asm/sound.s:71 (`SetupYM2612`, REV 02 0x03e59a).
/// Upload the revision's patch bytes in original order. Both frequency modes,
/// byte/word residual registers, unchecked signed indices, and the distinct
/// initial/tail operator-mask offsets follow the original assembly.
pub fn setup_ym2612(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut channel = context.data_register(1);
    if channel as u8 == ram::g_music_animation_channel(revision).read(context) as u8 {
        ram::g_music_animation_event(revision).write(context, 1);
    }
    channel = (channel & !0xffff) | u32::from(channel as u8 as i8 as i16 as u16);
    context.set_data_register(1, channel);
    if channel as i16 >= 6 {
        compare_word(context, channel as u16, 6);
        return Ok(None);
    }
    let patch_index = u32::from(context.data_register(3) as u8 as i8 as i16 as u16);
    let mut patch = Address::<u8>::new(symbol(context, "FMPatches").wrapping_add(patch_index * 38));
    let masks = ram::g_fm_channel_key_masks(revision).address();
    let mask = patch.offset(37).read(context);
    context.write_u8(masks.wrapping_add_signed(i32::from(channel as i16)), mask);
    let mut key_channel = (context.data_register(2) & !0xffff) | (channel & 0xffff);
    let mode = if channel as i16 == 2 {
        patch.offset(1).read(context)
    } else {
        0
    };
    let port = if channel as i16 >= 3 {
        key_channel = (key_channel & !0xffff) | u32::from((key_channel as u16).wrapping_add(1));
        channel = (channel & !0xffff) | u32::from((channel as u16).wrapping_sub(3));
        0xa04002
    } else {
        0xa04000
    };
    context.set_data_register(1, channel);
    context.set_address_register(5, port);
    context.write_z80_bus_request(0x100);
    let delay = context.read_z80_ram(0x004a);
    context.write_z80_ram(0x004a, delay.wrapping_sub(1));
    select_ym(context, 0xa04000, 0x22)?;
    let lfo = patch.read(context);
    patch = patch.offset(2);
    context.write_ym2612(1, lfo);
    select_ym(context, 0xa04000, 0x27)?;
    context.write_ym2612(1, mode);
    for register in [0xb0, 0xb4] {
        let register = register | channel as u8;
        context.set_data_register(7, (context.data_register(7) & !255) | u32::from(register));
        select_ym(context, port, register)?;
        let value = patch.read(context);
        patch = patch.offset(1);
        context.write_device_u8(port + 1, value);
    }
    for slot in [0, 4, 8, 12] {
        for register in [0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90] {
            let selected = (register + slot) | channel as u8;
            context.set_data_register(7, (context.data_register(7) & !255) | u32::from(selected));
            select_ym(context, port, selected)?;
            let value = if register == 0x90 {
                0
            } else {
                let value = patch.read(context);
                patch = patch.offset(1);
                value
            };
            context.write_device_u8(port + 1, value);
        }
    }
    if mode == 0 {
        for (register, data) in [(0xa4, 4), (0xa0, 5)] {
            let register = register | channel as u8;
            context.set_data_register(7, (context.data_register(7) & !255) | u32::from(register));
            select_ym(context, port, register)?;
            context.write_device_u8(port + 1, context.data_register(data) as u8);
        }
    } else {
        for (register, offset) in [
            (0xa6, 1),
            (0xa2, 0),
            (0xac, 3),
            (0xa8, 2),
            (0xad, 5),
            (0xa9, 4),
            (0xae, 7),
            (0xaa, 6),
        ] {
            select_ym(context, port, register)?;
            context.write_device_u8(port + 1, patch.offset(offset).read(context));
        }
    }
    let operators = patch.offset(8).read(context).wrapping_shl(4);
    key_channel = (key_channel & !255) | u32::from((key_channel as u8 & 7) | operators);
    context.set_data_register(2, key_channel);
    context.set_data_register(3, u32::from(operators));
    context.set_address_register(4, patch.address());
    select_ym(context, 0xa04000, 0x28)?;
    context.write_ym2612(1, key_channel as u8);
    let delay = context.read_z80_ram(0x004a);
    context.write_z80_ram(0x004a, delay.wrapping_sub(5));
    context.write_ym2612(0, 0x2a);
    finish_bus_release(context, delay < 5);
    Ok(None)
}
