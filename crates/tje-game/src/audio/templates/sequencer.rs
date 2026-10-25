//! The original per-tick music bytecode interpreter, not a rescheduled audio VM.

use super::registers::{
    add_word, extend_byte, move_byte, move_long, move_word, nz, shift_word_left, shift_word_right,
    subtract_byte, subtract_word,
};
use crate::{
    Context, Error, FunctionId, Result, call_at,
    call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments},
    function_at, ids,
};
use tje_mem::{Address, ram, rom_symbol};

const STREAM: i32 = 4;
const CALLBACK: i32 = 8;
const FM_CHANNEL: i32 = 12;
const PRIORITY: i32 = 13;
const PATCH: i32 = 14;
const WAIT: i32 = 15;
const FREQUENCY_HI: i32 = 16;
const FREQUENCY_LO: i32 = 17;
const NOTE_ACTIVE: i32 = 18;
const SELECTED: i32 = 19;
const LOOP_DEPTH: i32 = 20;

fn channel(context: &dyn Context) -> Address<u8> {
    Address::new(context.address_register(0))
}

fn write_byte(context: &mut dyn Context, address: Address<u8>, value: u8) {
    address.write(context, value);
    nz(context, u32::from(value), 8);
}

fn load_byte(context: &mut dyn Context, register: usize, address: Address<u8>) {
    let value = address.read(context);
    move_byte(context, register, value);
}

fn store_byte(context: &mut dyn Context, address: Address<u8>, register: usize) {
    let value = context.data_register(register) as u8;
    write_byte(context, address, value);
}

fn write_long(context: &mut dyn Context, address: Address<u32>, value: u32) {
    address.write(context, value);
    nz(context, value, 32);
}

fn stream_byte(context: &mut dyn Context) -> u8 {
    let pointer = context.address_register(1);
    let value = context.read_u8(pointer);
    context.set_address_register(1, pointer.wrapping_add(1));
    value
}

fn read_byte_into(context: &mut dyn Context, register: usize) {
    let value = stream_byte(context);
    move_byte(context, register, value);
}

fn read_relative(context: &mut dyn Context) -> i16 {
    read_byte_into(context, 1);
    shift_word_left(context, 1, 8);
    read_byte_into(context, 1);
    context.data_register(1) as i16
}

/// Original JSR return offsets from both linked `sound.s` revisions. Every
/// original fallback sees the real suspended MOVEM frame and return address.
fn service(context: &mut dyn Context, id: FunctionId, return_offset: u32) -> Result<()> {
    let site = CallSite::relative(
        ids::MUSIC_UPDATE_SEQUENCER,
        context.revision(),
        return_offset,
    );
    call_at(context, id, &[], site)?;
    Ok(())
}

fn next_channel(context: &mut dyn Context) -> bool {
    context.set_address_register(0, context.address_register(0).wrapping_add(38));
    let counter = (context.data_register(0) as u16).wrapping_sub(1);
    // DBRA changes only D0.w, never CCR.
    context.set_data_register(0, (context.data_register(0) & !0xffff) | u32::from(counter));
    counter != u16::MAX
}

fn choose_owners(context: &mut dyn Context) -> Result<()> {
    let revision = context.revision();
    let owners = ram::g_music_fm_channel_owners(revision).at(0).cast::<u32>();
    context.set_address_register(3, owners.address());
    context.set_address_register(0, ram::g_music_channels(revision).address());
    let count = ram::g_music_channel_scan_count_minus_one(revision)
        .cast::<u16>()
        .read(context);
    move_word(context, 0, count);
    loop {
        let record = channel(context);
        let stream = record.byte_offset(STREAM).cast::<u32>().read(context);
        nz(context, stream, 32);
        if stream != 0 {
            load_byte(context, 1, record.byte_offset(FM_CHANNEL));
            extend_byte(context, 1);
            shift_word_left(context, 1, 2);
            load_byte(context, 2, record.byte_offset(PRIORITY));
            let owner_slot = owners.byte_offset(i32::from(context.data_register(1) as i16));
            let previous = owner_slot.read(context);
            move_long(context, 3, previous);
            if previous != 0 {
                context.set_address_register(4, previous);
                let previous = Address::<u8>::new(previous);
                let priority = previous.byte_offset(PRIORITY).read(context) as i8;
                if (context.data_register(2) as i8) < priority {
                    clear_byte(context, record.byte_offset(SELECTED))?;
                    write_byte(context, previous.byte_offset(SELECTED), 1);
                    if next_channel(context) {
                        continue;
                    }
                    break;
                }
                clear_byte(context, previous.byte_offset(SELECTED))?;
            }
            write_byte(context, record.byte_offset(SELECTED), 1);
            write_long(context, owner_slot, record.address());
        }
        if !next_channel(context) {
            break;
        }
    }
    Ok(())
}

fn tick_and_callback(context: &mut dyn Context) -> Result<()> {
    let record = channel(context);
    let wait = record.byte_offset(WAIT);
    let old_wait = wait.read(context);
    let remaining = subtract_byte(context, old_wait, 1);
    wait.write(context, remaining);
    let callback = record.byte_offset(CALLBACK).cast::<u32>().read(context);
    move_long(context, 1, callback);
    if callback != 0 {
        context.set_address_register(1, callback);
        let id = function_at(context.revision(), callback).ok_or_else(|| {
            Error::Platform(format!("uncatalogued music callback {callback:08x}"))
        })?;
        // sound.s:1040-1044 pushes A0, then D0. These are actual saved
        // registers, not invented callback parameters (the callback takes none).
        let saved = [context.data_register(0), context.address_register(0)];
        nz(context, saved[0], 32);
        let outgoing = OutgoingArguments::push(context, &saved)?;
        let dispatcher = context.dispatcher();
        let site = CallSite::relative(ids::MUSIC_UPDATE_SEQUENCER, context.revision(), 0x2ea);
        outgoing.call(context, dispatcher, id, site)?;
        let restored = outgoing.argument_slot(0).read(context);
        move_long(context, 0, restored);
        context.set_address_register(0, outgoing.argument_slot(1).read(context));
        outgoing.release(context)?;
    }
    Ok(())
}

fn save_stream(context: &mut dyn Context) {
    let destination = channel(context).byte_offset(STREAM).cast::<u32>();
    let pointer = context.address_register(1);
    write_long(context, destination, pointer);
}

fn selected(context: &mut dyn Context) -> bool {
    let value = channel(context).byte_offset(SELECTED).read(context);
    nz(context, u32::from(value), 8);
    value != 0
}

fn fm_channel(context: &mut dyn Context) {
    let value = channel(context).byte_offset(FM_CHANNEL).read(context);
    move_byte(context, 1, value);
    extend_byte(context, 1);
}

/// `SUBQ.W #1,D3; BLE` consumes subtraction V. At -32768 it must exit,
/// despite the narrowed result becoming +32767. Only the positive branch
/// writes the counter back; the exhausted/negative value remains in RAM.
fn loop_continues(old: i16) -> bool {
    i32::from(old) - 1 > 0
}

fn execute_commands(context: &mut dyn Context) -> Result<()> {
    loop {
        let record = channel(context);
        read_byte_into(context, 1);
        let opcode = context.data_register(1) as u8;
        if opcode & 0x80 != 0 {
            let ticks = opcode & 0x7f;
            move_byte(context, 1, ticks);
            write_byte(context, record.byte_offset(WAIT), ticks);
            save_stream(context);
            return tick_and_callback(context);
        }
        // The SUBQ.B dispatch chain leaves zero for opcodes 1..12 and
        // opcode-12 for 13..127. All subtractions have V=C=X=0, so one
        // width-preserving subtraction gives the same residue and CCR.
        if opcode != 0 {
            let value = subtract_byte(context, opcode, opcode.min(12));
            context.set_data_register(1, (context.data_register(1) & !0xff) | u32::from(value));
        }
        match opcode {
            0 => {
                if selected(context) {
                    fm_channel(context);
                    let hardware_channel = context.data_register(1) as u16;
                    move_word(context, 6, hardware_channel);
                    shift_word_left(context, 6, 2);
                    let slot = Address::<u32>::new(context.address_register(3))
                        .byte_offset(i32::from(context.data_register(6) as i16));
                    clear_long(context, slot)?;
                    service(context, ids::AUDIO_WRITE_FM_CHANNEL_RELEASE_MASK, 0x110)?;
                }
                clear_long(context, record.byte_offset(STREAM).cast())?;
                clear_long(context, record.byte_offset(CALLBACK).cast())?;
                clear_byte(context, record.byte_offset(NOTE_ACTIVE))?;
                clear_byte(context, record.byte_offset(SELECTED))?;
                let template_status = ram::g_fm_effect_template_status(context.revision()).cast();
                clear_word(context, template_status)?;
                return Ok(());
            }
            1 => {
                load_byte(context, 1, record.byte_offset(FM_CHANNEL));
                load_byte(context, 3, record.byte_offset(PATCH));
                read_byte_into(context, 4);
                read_byte_into(context, 5);
                store_byte(context, record.byte_offset(FREQUENCY_HI), 4);
                store_byte(context, record.byte_offset(FREQUENCY_LO), 5);
                write_byte(context, record.byte_offset(NOTE_ACTIVE), 1);
                extend_byte(context, 1);
                if selected(context) {
                    service(context, ids::SETUP_YM2612, 0x158)?;
                }
            }
            2 => {
                if selected(context) {
                    fm_channel(context);
                    service(context, ids::AUDIO_WRITE_FM_CHANNEL_RELEASE_MASK, 0x172)?;
                }
                clear_byte(context, record.byte_offset(NOTE_ACTIVE))?;
            }
            3 => {
                read_byte_into(context, 4);
                read_byte_into(context, 5);
                store_byte(context, record.byte_offset(FREQUENCY_HI), 4);
                store_byte(context, record.byte_offset(FREQUENCY_LO), 5);
                if selected(context) {
                    fm_channel(context);
                    service(context, ids::YM2612_UPDATE_FREQUENCY, 0x19e)?;
                }
            }
            4 => {
                read_byte_into(context, 1);
                extend_byte(context, 1);
                let song = ram::g_current_music_selection(context.revision()).read(context);
                move_byte(context, 2, song as u8);
                extend_byte(context, 2);
                shift_word_left(context, 2, 4);
                let song_offset = context.data_register(2) as u16;
                let selection = context.data_register(1) as u16;
                let offset = add_word(context, song_offset, selection);
                context
                    .set_data_register(2, (context.data_register(2) & !0xffff) | u32::from(offset));
                let maps = rom_symbol(context.revision(), "InstrumentSelectionMaps")
                    .expect("instrument selection maps exist in both revisions")
                    .address;
                context.set_address_register(5, maps);
                let patch = Address::<u8>::new(maps)
                    .byte_offset(i32::from(offset as i16))
                    .read(context);
                move_byte(context, 1, patch);
                write_byte(context, record.byte_offset(PATCH), patch);
            }
            5 => {
                load_byte(context, 1, record.byte_offset(LOOP_DEPTH));
                extend_byte(context, 1);
                let depth = context.data_register(1) as u16;
                move_word(context, 2, depth);
                let index = context.data_register(2) as u16;
                let offset = add_word(context, index, index);
                context
                    .set_data_register(2, (context.data_register(2) & !0xffff) | u32::from(offset));
                let storage = record.byte_offset(i32::from(offset as i16));
                let high = stream_byte(context);
                write_byte(context, storage.byte_offset(22), high);
                let low = stream_byte(context);
                write_byte(context, storage.byte_offset(23), low);
                let old_depth = context.data_register(1) as u16;
                let depth = add_word(context, old_depth, 1);
                context
                    .set_data_register(1, (context.data_register(1) & !0xffff) | u32::from(depth));
                write_byte(context, record.byte_offset(LOOP_DEPTH), depth as u8);
            }
            6 => {
                let relative = read_relative(context);
                load_byte(context, 2, record.byte_offset(LOOP_DEPTH));
                extend_byte(context, 2);
                let index = context.data_register(2) as u16;
                let offset = add_word(context, index, index);
                context
                    .set_data_register(2, (context.data_register(2) & !0xffff) | u32::from(offset));
                let storage = record.byte_offset(i32::from(offset as i16));
                load_byte(context, 3, storage.byte_offset(20));
                shift_word_left(context, 3, 8);
                load_byte(context, 3, storage.byte_offset(21));
                let old = context.data_register(3) as u16;
                let remaining = subtract_word(context, old, 1);
                context.set_data_register(
                    3,
                    (context.data_register(3) & !0xffff) | u32::from(remaining),
                );
                if loop_continues(old as i16) {
                    write_byte(context, storage.byte_offset(21), remaining as u8);
                    shift_word_right(context, 3, 8);
                    store_byte(context, storage.byte_offset(20), 3);
                    let pointer = context
                        .address_register(1)
                        .wrapping_add(3)
                        .wrapping_add(relative as u32);
                    context.set_address_register(1, pointer);
                }
            }
            7 => {
                let pointer = context.address_register(1);
                move_long(context, 1, pointer);
                if pointer & 1 != 0 {
                    context.set_address_register(1, pointer.wrapping_add(1));
                }
                for byte in 0..4 {
                    let value = stream_byte(context);
                    write_byte(context, record.byte_offset(CALLBACK + byte), value);
                }
            }
            8 => {
                let relative = read_relative(context);
                context.set_address_register(
                    1,
                    context.address_register(1).wrapping_add(relative as u32),
                );
            }
            9..=12 => {
                let (name, return_offset) = match opcode {
                    9 => ("Sfx_JamRecordScratch", 0x26c),
                    10 => ("Sfx_JamClap", 0x284),
                    11 => ("Sfx_JamBassDrum", 0x29c),
                    _ => ("Sfx_JamSnareDrum", 0x2bc),
                };
                let sample = rom_symbol(context.revision(), name)
                    .expect("sequencer PCM symbols exist in both revisions")
                    .address;
                let site = CallSite::relative(
                    ids::MUSIC_UPDATE_SEQUENCER,
                    context.revision(),
                    return_offset,
                );
                call_at(
                    context,
                    ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
                    &[sample],
                    site,
                )?;
                if opcode == 11 {
                    ram::g_music_animation_event(context.revision()).write(context, 1);
                    nz(context, 1, 8);
                }
            }
            _ => {}
        }
        save_stream(context);
    }
}

/// G/src/asm/sound.s:762-1053 (`MusicUpdateSequencer`), both revisions.
/// Retains request order, inclusive DBRA channel scans, signed ownership, every
/// opcode, per-tick callbacks and the original TimeAware PCM/PSG submissions.
/// No mailbox poll, retry, delay or clock observation is replaced here.
pub fn music_update_sequencer(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    // sound.s:763: MOVEM.L D2-D7/A2-A6,-(SP); locals/save bytes are real RAM.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0xfc, 0x7c))?;
    let revision = context.revision();
    let release = ram::g_fm_channel_release_requested(revision).read(context);
    nz(context, release as u16 as u32, 16);
    if release != 0 {
        service(context, ids::AUDIO_RELEASE_ALL_FM_CHANNELS, 0x10)?;
    }
    let reset = ram::g_music_track_reset_requested(revision).read(context);
    nz(context, reset as u16 as u32, 16);
    if reset != 0 {
        service(context, ids::MUSIC_CLEAR_TRACK_POINTERS, 0x1c)?;
    }
    let effect = ram::g_pending_fm_effect(revision).read(context);
    nz(context, effect as u16 as u32, 16);
    if effect >= 0 {
        service(
            context,
            ids::AUDIO_LOAD_PENDING_CHANNEL_TEMPLATE_EFFECT,
            0x28,
        )?;
    }
    let pcm = ram::g_pending_pcm_sample(revision).read(context) as u32;
    nz(context, pcm, 32);
    if pcm != 0 {
        service(context, ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE, 0x36)?;
    } else {
        let psg = ram::g_pending_psg_effect(revision).read(context);
        nz(context, psg as u16 as u32, 16);
        if psg >= 0 {
            service(context, ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND, 0x46)?;
        }
    }
    let status = ram::g_psg_effect_status(revision);
    let old_status = status.read(context);
    nz(context, old_status as u16 as u32, 16);
    if old_status != 0 {
        // TST.W and the memory SUBQ.W are separate reads (sound.s:789-791).
        let operand = status.read(context) as u16;
        let value = subtract_word(context, operand, 1);
        status.write(context, value as i16);
    }
    let cooldown = ram::g_pcm_effect_cooldown_ticks(revision);
    let old_cooldown = cooldown.read(context);
    nz(context, old_cooldown as u16 as u32, 16);
    if old_cooldown != 0 {
        // The speech timer has the same distinct test/read-modify-write pair.
        let operand = cooldown.read(context) as u16;
        let value = subtract_word(context, operand, 1);
        cooldown.write(context, value as i16);
        let blocks = ram::g_pcm_blocks_sequencer(revision);
        let blocking = blocks.read(context);
        nz(context, blocking as u16 as u32, 16);
        if blocking != 0 {
            let remaining = cooldown.read(context);
            nz(context, remaining as u16 as u32, 16);
            if remaining != 0 {
                frame.leave(context)?;
                return Ok(None);
            }
            blocks.write(context, 0);
            nz(context, 0, 16);
        }
    }
    choose_owners(context)?;
    context.set_address_register(0, ram::g_music_channels(revision).address());
    let count = ram::g_music_channel_scan_count_minus_one(revision)
        .cast::<u16>()
        .read(context);
    move_word(context, 0, count);
    loop {
        let record = channel(context);
        let stream = record.byte_offset(STREAM).cast::<u32>().read(context);
        move_long(context, 1, stream);
        if stream != 0 {
            context.set_address_register(1, stream);
            let wait = record.byte_offset(WAIT).read(context) as i8;
            nz(context, u32::from(wait as u8), 8);
            if wait > 0 {
                tick_and_callback(context)?;
            } else {
                execute_commands(context)?;
            }
        }
        if !next_channel(context) {
            break;
        }
    }
    frame.leave(context)?;
    Ok(None)
}

// The original CLR instructions (sound.s:821-825,856-863,893) first perform
// discarded reads. A long operand is read in high-word, low-word bus order.
fn clear_byte(context: &mut dyn Context, address: Address<u8>) -> Result<()> {
    context.discard_u8(address.address())?;
    write_byte(context, address, 0);
    Ok(())
}

fn clear_word(context: &mut dyn Context, address: Address<u16>) -> Result<()> {
    context.discard_u16(address.address())?;
    address.write(context, 0);
    nz(context, 0, 16);
    Ok(())
}

fn clear_long(context: &mut dyn Context, address: Address<u32>) -> Result<()> {
    context.discard_u16(address.address())?;
    context.discard_u16(address.address().wrapping_add(2))?;
    write_long(context, address, 0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::loop_continues;

    #[test]
    fn loop_branch_matches_subq_word_ble_for_all_word_values() {
        for bits in 0..=u16::MAX {
            let result = bits.wrapping_sub(1);
            let n = result & 0x8000 != 0;
            let z = result == 0;
            let v = ((bits ^ 1) & (bits ^ result) & 0x8000) != 0;
            assert_eq!(loop_continues(bits as i16), !z && n == v, "{bits:04x}");
        }
    }
}

#[cfg(test)]
#[path = "sequencer/access_tests.rs"]
mod access_tests;
