//! Original song-template copies and restart decisions.

use crate::{
    Context, Result, call, call_at,
    call_frame::{CallSite, FrameError},
    ids,
};
use tje_mem::{Address, ram, rom};

#[path = "templates/registers.rs"]
mod registers;
#[path = "templates/sequencer.rs"]
mod sequencer;

pub use sequencer::music_update_sequencer;

use registers::{add_byte, move_byte, move_word, nz, subtract_word};

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

/// G/src/asm/sound_schedule.s:12-18 (`MusicSelectSongTemplateBank`).
/// The alternate-bank bias changes the caller's actual low argument word;
/// the loader is a fallthrough, sharing this entry frame and return address.
pub fn music_select_song_template_bank(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    ram::g_music_template_restart_state(revision).write(context, 2);
    nz(context, 2, 16);
    let alternate = ram::g_music_alternate_template_bank_enabled(revision).read(context);
    nz(context, alternate as u16 as u32, 16);
    let frame = context.frame_state().ok_or(FrameError::Unavailable)?;
    if alternate != 0 {
        let slot = frame.argument_word(0).cast::<u16>();
        let old = slot.read(context);
        let value = registers::add_word(context, old, 7);
        slot.write(context, value);
    }
    let selection = frame.argument_slot(0).read(context);
    call(
        context,
        ids::MUSIC_LOAD_SONG_CHANNEL_TEMPLATES,
        &[selection],
    )?;
    Ok(None)
}

/// G/src/asm/sound_schedule.s:44-76 (`MusicProcessTemplateRestartHook`).
/// A0 is the callback's channel record, not a stack argument. BSR return
/// offsets are +0x2e / +0x40 in both matching linked revisions.
pub fn music_process_template_restart_hook(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    Address::<u32>::new(context.address_register(0).wrapping_add(8)).write(context, 0);
    nz(context, 0, 32);
    context.set_data_register(0, 0);
    nz(context, 0, 32);
    let selection = ram::g_current_music_selection(revision).read(context) as u8;
    move_byte(context, 0, selection);
    let state = ram::g_music_template_restart_state(revision);
    let old_state = state.read(context) as u16;
    let remaining = subtract_word(context, old_state, 1);
    state.write(context, remaining as i16);
    if remaining != 0 {
        move_word(context, 1, remaining);
        registers::compare_word(context, remaining, 0xfffe);
        if remaining != 0xfffe {
            return Ok(None);
        }
        let alternate = ram::g_music_alternate_template_bank_enabled(revision).read(context);
        nz(context, alternate as u16 as u32, 16);
        if alternate == 0 {
            // MOVE.L D0,-(SP) supplies N/Z before BSR; ADDQ to SP leaves CCR.
            let argument = context.data_register(0);
            nz(context, argument, 32);
            call_at(
                context,
                ids::MUSIC_SELECT_SONG_TEMPLATE_BANK,
                &[argument],
                CallSite::relative(ids::MUSIC_PROCESS_TEMPLATE_RESTART_HOOK, revision, 0x2e),
            )?;
            return Ok(None);
        }
    }
    let hold = ram::g_boom_box_music_template_hold(revision).read(context);
    nz(context, hold as u16 as u32, 16);
    if hold != 0 {
        state.write(context, 1);
        nz(context, 1, 16);
    } else {
        let next = add_byte(context, selection, 7);
        move_byte(context, 0, next);
        let argument = context.data_register(0);
        nz(context, argument, 32);
        call_at(
            context,
            ids::MUSIC_LOAD_SONG_CHANNEL_TEMPLATES,
            &[argument],
            CallSite::relative(ids::MUSIC_PROCESS_TEMPLATE_RESTART_HOOK, revision, 0x40),
        )?;
    }
    Ok(None)
}
