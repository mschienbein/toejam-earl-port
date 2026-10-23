//! Original song-template copies and restart decisions.

use crate::{Context, Result, call, ids};
use tje_mem::{ram, rom};

/// G/src/asm/sound_schedule.s:24 (`MusicLoadSongChannelTemplates`, REV 02 0x03e47e).
/// Release FM before restoring the initial state, then use the low unsigned
/// argument word for MULU's 228-byte template stride and copy bytes in order.
pub fn music_load_song_channel_templates(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    call(context, ids::AUDIO_RELEASE_ALL_FM_CHANNELS, &[])?;
    call(context, ids::MUSIC_RESTORE_INITIAL_STATE, &[])?;
    let revision = context.revision();
    let mut source = rom::song_channel_templates(revision)
        .at(i32::from(arguments[0] as u16))
        .at(0)
        .cast::<u8>();
    let mut destination = ram::g_music_channels(revision).at(0).cast::<u8>();
    let mut remaining = 227_u16;
    loop {
        let value = source.read(context);
        source = source.offset(1);
        destination.write(context, value);
        destination = destination.offset(1);
        remaining = remaining.wrapping_sub(1);
        if remaining == u16::MAX {
            context.set_data_register(0, 0xffff);
            context.set_address_register(0, source.address());
            context.set_address_register(1, destination.address());
            let nz = if value == 0 {
                4
            } else if value & 0x80 != 0 {
                8
            } else {
                0
            };
            context.set_status_register((context.status_register() & !0x1f) | nz);
            return Ok(None);
        }
    }
}
