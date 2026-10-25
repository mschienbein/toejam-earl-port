//! Readable mailbox protocol bodies, including both original retry fragments.
//!
//! These entries are PENDING, not approved zero-time replacements. ClockedStrict
//! must advance each original instruction through TimingSource, including the
//! no-I/O DBRA backedges and release/reacquire cadence. A condition-only wait
//! cannot replace these loops or their bounded timeout behavior.

use crate::{
    Context, FunctionId, Result,
    call_frame::{FrameLayout, LocalFrame},
    ids,
    open_bus::OpenBusSource,
};
use tje_mem::ram;

const BUS_TAKE: u16 = 0x100;
const BUS_POLL_LIMIT: u16 = 20;
const MAILBOX_LOCK: u16 = 0x0041;
const PSG_ACTIVE: u16 = 0x004b;

fn nz(context: &mut dyn Context, value: u32, bits: u32) {
    let value = value & (u32::MAX >> (32 - bits));
    let flags = if value == 0 {
        4
    } else if value & (1 << (bits - 1)) != 0 {
        8
    } else {
        0
    };
    context.set_status_register((context.status_register() & !0x0f) | flags);
}

fn move_byte(context: &mut dyn Context, register: usize, value: u8) {
    context.set_data_register(
        register,
        (context.data_register(register) & !0xff) | u32::from(value),
    );
    nz(context, u32::from(value), 8);
}

fn move_word(context: &mut dyn Context, register: usize, value: u16) {
    context.set_data_register(
        register,
        (context.data_register(register) & !0xffff) | u32::from(value),
    );
    nz(context, u32::from(value), 16);
}

fn move_long(context: &mut dyn Context, register: usize, value: u32) {
    context.set_data_register(register, value);
    nz(context, value, 32);
}

fn subtract_one_word(context: &mut dyn Context, register: usize) -> u16 {
    let old = context.data_register(register) as u16;
    let value = old.wrapping_sub(1);
    context.set_data_register(
        register,
        (context.data_register(register) & !0xffff) | u32::from(value),
    );
    let flags = (u16::from(old == 0) * 0x11)
        | (u16::from(value & 0x8000 != 0) << 3)
        | (u16::from(value == 0) << 2)
        | (u16::from(old == 0x8000) << 1);
    context.set_status_register((context.status_register() & !0x1f) | flags);
    value
}

fn shift_right_eight(context: &mut dyn Context, register: usize) {
    let old = context.data_register(register);
    move_long(context, register, old >> 8);
    context.set_status_register(
        (context.status_register() & !0x11) | if old & 0x80 != 0 { 0x11 } else { 0 },
    );
}

fn bus_request(context: &mut dyn Context, value: u16) {
    context.write_z80_bus_request(value);
    nz(context, u32::from(value), 16);
}

fn post_byte(context: &mut dyn Context, offset: u16, value: u8) {
    context.write_z80_ram(offset, value);
    nz(context, u32::from(value), 8);
}

fn post_register_byte(context: &mut dyn Context, offset: u16, register: usize) {
    let value = context.data_register(register) as u8;
    post_byte(context, offset, value);
}

/// Execute the actual MOVEQ/DBRA state changes; do not turn the released-bus
/// interval into a wait on inaccessible Z80 RAM or omit its inclusive iteration.
/// Yield only at the compound protocol's released-bus retry boundary. This
/// scheduler marker adds no read or source timing claim; the stronger profile
/// still needs its separate authenticated MOVEQ/DBRA delay receipt.
fn release_and_delay(context: &mut dyn Context, delay_minus_one: u8) -> Result<()> {
    bus_request(context, 0);
    move_long(context, 0, u32::from(delay_minus_one));
    loop {
        let remaining = (context.data_register(0) as u16).wrapping_sub(1);
        context.set_data_register(
            0,
            (context.data_register(0) & !0xffff) | u32::from(remaining),
        );
        if remaining == u16::MAX {
            break;
        }
    }
    context.poll_yield()
}

/// A fresh acquisition always reloads D2.low16 and predecrements before reading:
/// at most nineteen BUSACK reads. No successful grant is invented on timeout.
fn take_bus(context: &mut dyn Context, owner: FunctionId, prefetch_offset: u32) -> Result<bool> {
    bus_request(context, BUS_TAKE);
    move_word(context, 2, BUS_POLL_LIMIT);
    let prefetch = OpenBusSource::prefetch(
        owner
            .metadata()
            .address(context.revision())
            .wrapping_add(prefetch_offset),
    );
    loop {
        if subtract_one_word(context, 2) == 0 {
            return Ok(false);
        }
        let status = context.read_z80_bus_request(prefetch)?;
        move_word(context, 1, status);
        let busy = status & 0x100 != 0;
        context.set_status_register((context.status_register() & !4) | if busy { 0 } else { 4 });
        if !busy {
            return Ok(true);
        }
    }
}

/// Read the big-endian sample header and publish the original little-endian
/// payload/count bytes, redundant PSG-ID byte, and command-last transaction.
fn publish_pcm(context: &mut dyn Context, sample: u32) {
    context.set_address_register(0, sample);
    let length = context.read_u16(context.address_register(0));
    context.set_address_register(0, context.address_register(0).wrapping_add(2));
    move_word(context, 1, length);
    move_long(context, 2, 0);
    move_word(context, 2, length);
    let payload = context.address_register(0);
    move_long(context, 0, payload);
    post_register_byte(context, 0x0044, 0);
    shift_right_eight(context, 0);
    post_register_byte(context, 0x0045, 0);
    shift_right_eight(context, 0);
    post_register_byte(context, 0x0046, 0);
    post_register_byte(context, 0x0048, 1);
    shift_right_eight(context, 1);
    post_register_byte(context, 0x0049, 1);
    post_register_byte(context, 0x0042, 0);
    post_byte(context, 0x0040, 2);
}

fn submit_pending_pcm(context: &mut dyn Context, begin_at_retry: bool) -> Result<Option<u32>> {
    if begin_at_retry {
        release_and_delay(context, 23)?;
    }
    loop {
        if !take_bus(context, ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE, 0x18)? {
            bus_request(context, 0);
            ram::g_pending_psg_effect(context.revision()).write(context, -1);
            nz(context, 0xffff, 16);
            return Ok(None);
        }
        let lock = context.read_z80_ram(MAILBOX_LOCK);
        move_byte(context, 0, lock);
        if lock != 0 {
            release_and_delay(context, 23)?;
            continue;
        }
        let sample = ram::g_pending_pcm_sample(context.revision()).read(context) as u32;
        publish_pcm(context, sample);
        let length = context.data_register(2);
        let quotient = length / 138;
        let remainder = length % 138;
        context.set_data_register(2, (remainder << 16) | quotient);
        nz(context, quotient, 16);
        let ticks = (quotient as u16).wrapping_add(5);
        let carry = quotient > 0xfffa;
        let overflow = (!(quotient as u16 ^ 5) & (quotient as u16 ^ ticks) & 0x8000) != 0;
        context.set_data_register(2, (context.data_register(2) & !0xffff) | u32::from(ticks));
        nz(context, u32::from(ticks), 16);
        context.set_status_register(
            (context.status_register() & !0x13)
                | if carry { 0x11 } else { 0 }
                | (u16::from(overflow) << 1),
        );
        ram::g_pcm_effect_cooldown_ticks(context.revision()).write(context, ticks as i16);
        nz(context, u32::from(ticks), 16);
        bus_request(context, 0);
        ram::g_pending_psg_effect(context.revision()).write(context, -1);
        nz(context, 0xffff, 16);
        // sound_pcm.s:60 CLR.L reads both words before writing zero.
        let pending = ram::g_pending_pcm_sample(context.revision()).address();
        context.discard_u16(pending)?;
        context.discard_u16(pending.wrapping_add(2))?;
        ram::g_pending_pcm_sample(context.revision()).write(context, 0);
        nz(context, 0, 32);
        return Ok(None);
    }
}

/// G/src/asm/sound_pcm.s:10-15 (`AudioRetryPendingPcmMailbox`, REV 02 0x03f61c).
/// Retry fragment: release, 24 DBRA iterations, then share the owner's entry and
/// existing return frame. It must not be registered as an independent RTS hook.
pub fn audio_retry_pending_pcm_mailbox(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    submit_pending_pcm(context, true)
}

/// G/src/asm/sound_pcm.s:21-68 (`AudioSubmitPendingPcmSample`, REV 02 0x03f62a).
/// Timeout cancels only pending PSG; acceptance posts the PCM packet, starts the
/// length/138+5 cooldown and clears the pending pointer, in the original order.
pub fn audio_submit_pending_pcm_sample(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    submit_pending_pcm(context, false)
}

/// G/src/asm/sound_pcm.s:74-131 (`AudioTrySubmitSequencerPcmSample`, REV 02 0x03f6c6).
/// Preserve the exact LINK/MOVEM frame and D0-D2/A0, reject cooldown/inhibit or
/// PSG-active requests, and post an immediate sample without starting cooldown.
/// Keep the entry framed: sound_pcm.s:106 reads the actual pointer slot only on
/// acceptance, never through an eager adapter before cooldown/inhibit/active gates.
pub fn audio_try_submit_sequencer_pcm_sample(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(0, 0x07, 0x01))?;
    let cooldown = ram::g_pcm_effect_cooldown_ticks(context.revision()).read(context) as u16;
    nz(context, u32::from(cooldown), 16);
    if cooldown == 0 {
        let inhibited =
            ram::g_music_alternate_template_bank_enabled(context.revision()).read(context) as u16;
        nz(context, u32::from(inhibited), 16);
        if inhibited == 0 {
            loop {
                if !take_bus(context, ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE, 0x44)? {
                    bus_request(context, 0);
                    break;
                }
                let lock = context.read_z80_ram(MAILBOX_LOCK);
                move_byte(context, 0, lock);
                if lock != 0 {
                    release_and_delay(context, 23)?;
                    continue;
                }
                let active = context.read_z80_ram(PSG_ACTIVE);
                move_byte(context, 0, active);
                if active == 0 {
                    let sample = frame.argument_slot(0).read(context);
                    publish_pcm(context, sample);
                }
                bus_request(context, 0);
                break;
            }
        }
    }
    frame.leave(context)?;
    Ok(None)
}

fn submit_pending_psg(context: &mut dyn Context, begin_at_retry: bool) -> Result<Option<u32>> {
    if begin_at_retry {
        release_and_delay(context, 123)?;
    }
    loop {
        if !take_bus(context, ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND, 0x18)? {
            break;
        }
        let lock = context.read_z80_ram(MAILBOX_LOCK);
        move_byte(context, 0, lock);
        if lock != 0 {
            release_and_delay(context, 123)?;
            continue;
        }
        let effect = ram::g_pending_psg_effect(context.revision()).read(context) as u16;
        move_word(context, 0, effect);
        post_register_byte(context, 0x0042, 0);
        post_byte(context, 0x0040, 1);
        ram::g_psg_effect_status(context.revision()).write(context, 15);
        nz(context, 15, 16);
        break;
    }
    bus_request(context, 0);
    ram::g_pending_psg_effect(context.revision()).write(context, -1);
    nz(context, 0xffff, 16);
    Ok(None)
}

/// G/src/asm/sound_psg.s:9-14 (`AudioRetryPendingPsgMailbox`, REV 02 0x03f79c).
/// Retry fragment: release and 124 DBRA iterations, then fall through to the
/// indexed-command owner without adding a return address or a new argument area.
pub fn audio_retry_pending_psg_mailbox(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    submit_pending_psg(context, true)
}

/// G/src/asm/sound_psg.s:20-45 (`AudioSubmitPendingIndexedCommand`, REV 02 0x03f7aa).
/// On acceptance post ID then command1 and status15; consume pending PSG even on
/// timeout, preserving the raw BUSACK word and the original scratch-register bits.
pub fn audio_submit_pending_indexed_command(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    submit_pending_psg(context, false)
}

#[cfg(test)]
mod tests;
