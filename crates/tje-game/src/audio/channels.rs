//! Functional channel-release and auxiliary-template logic.

use crate::{Context, Result, call_void, ids};
use tje_mem::{Address, ram, rom_symbol};

/// G/src/asm/sound_channels.s:9 (`AudioReleaseAllFmChannels`, REV 02 0x03e446).
/// Preserve D3/A5 across all six calls and D1 across each individual key-mask
/// write. The final DBRA low word is -1, not a corrected channel-zero result.
pub fn audio_release_all_fm_channels(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let saved_d3 = context.data_register(3);
    let saved_a5 = context.address_register(5);
    let mut channel = 5_u16;
    loop {
        context.set_data_register(1, u32::from(channel));
        call_void(context, ids::AUDIO_WRITE_FM_CHANNEL_RELEASE_MASK, &[])?;
        channel = channel.wrapping_sub(1);
        context.set_data_register(1, u32::from(channel));
        if channel == u16::MAX {
            break;
        }
    }
    context.set_data_register(3, saved_d3);
    context.set_address_register(5, saved_a5);
    ram::g_fm_channel_release_requested(context.revision()).write(context, 0);
    context.set_status_register((context.status_register() & !0x0f) | 4);
    Ok(None)
}

/// G/src/asm/sound.s:9 (`AudioLoadPendingChannelTemplateEffect`, REV 02 0x03e504).
/// Release active auxiliary streams in their original 2/3/1 order, clear exactly
/// 57 words, then copy the unchecked ROM pointer/count-selected channel words.
pub fn audio_load_pending_channel_template_effect(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for (stream, channel, owner) in [
        (
            ram::g_fm_effect_channel1_current_stream(revision),
            2,
            ram::g_music_fm_channel2_owner(revision),
        ),
        (
            ram::g_fm_effect_channel2_current_stream(revision),
            3,
            ram::g_music_fm_channel3_owner(revision),
        ),
        (
            ram::g_fm_effect_channel3_current_stream(revision),
            1,
            ram::g_music_fm_channel1_owner(revision),
        ),
    ] {
        if stream.cast::<u32>().read(context) != 0 {
            context.set_data_register(1, (context.data_register(1) & !0xffff) | channel);
            call_void(context, ids::AUDIO_WRITE_FM_CHANNEL_RELEASE_MASK, &[])?;
            owner.cast::<u32>().write(context, 0);
        }
    }
    let mut destination = ram::g_fm_effect_channels(revision).cast::<u16>();
    for _ in 0..57 {
        destination.write(context, 0);
        destination = destination.offset(1);
    }
    let effect = ram::g_pending_fm_effect(revision).read(context) as u16;
    context.set_data_register(0, (context.data_register(0) & !0xffff) | u32::from(effect));
    ram::g_fm_effect_template_status(revision).write(context, effect as i16);
    context.set_address_register(0, destination.address());
    if effect != 0 {
        let counts = rom_symbol(revision, "SfxChannelTemplateCounts")
            .expect("audio template count symbol exists in both revisions")
            .address;
        let count = context.read_u8(counts.wrapping_add_signed(i32::from(effect as i16))) as i8;
        let scaled = effect.wrapping_shl(2);
        context.set_data_register(0, (context.data_register(0) & !0xffff) | u32::from(scaled));
        let pointers = rom_symbol(revision, "SfxChannelTemplatePointers")
            .expect("audio template pointer symbol exists in both revisions")
            .address;
        let mut source = Address::<u16>::new(
            context.read_u32(pointers.wrapping_add_signed(i32::from(scaled as i16))),
        );
        let words = u32::from(count as i16 as u16) * 19;
        let mut remaining = (words as u16).wrapping_sub(1);
        let borrow = words as u16 == 0;
        destination = ram::g_fm_effect_channels(revision).cast::<u16>();
        loop {
            let value = source.read(context);
            source = source.offset(1);
            destination.write(context, value);
            destination = destination.offset(1);
            remaining = remaining.wrapping_sub(1);
            if remaining == u16::MAX {
                break;
            }
        }
        context.set_data_register(1, (words & !0xffff) | 0xffff);
        context.set_address_register(0, destination.address());
        context.set_address_register(1, source.address());
        context.set_status_register(
            (context.status_register() & !0x10) | if borrow { 0x10 } else { 0 },
        );
    }
    ram::g_pending_fm_effect(revision).write(context, -1);
    context.set_status_register((context.status_register() & !0x0f) | 8);
    Ok(None)
}
