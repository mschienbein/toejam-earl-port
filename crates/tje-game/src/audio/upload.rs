//! Original bounded Z80 upload; all driver bytes remain in the user's ROM.

use crate::{
    Context, Result,
    call_frame::{FrameLayout, LocalFrame},
    ids,
    open_bus::OpenBusSource,
};
use tje_mem::{Address, ram, rom_symbol};

fn symbol(context: &dyn Context, name: &str) -> u32 {
    rom_symbol(context.revision(), name)
        .expect("audio ROM symbol is present in both revision catalogues")
        .address
}

fn move_word_flags(context: &mut dyn Context, value: u16) {
    let nz = if value == 0 {
        4
    } else if value & 0x8000 != 0 {
        8
    } else {
        0
    };
    context.set_status_register((context.status_register() & !0x0f) | nz);
}

/// G/src/asm/z80bus.s:10-39 (`LoadSoundDriverToZ80`, REV 02 0x03f57a).
/// Preserve D2's actual saved slot, nineteen-read acquisition, inclusive DBRA
/// byte count, reset/release order and both MULU residues. The BUSACK operand's
/// unused bits use the real BTST prefetch at entry+0x1a in both ROM revisions.
pub fn load_sound_driver_to_z80(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x04, 0))?;
    context.write_z80_bus_request(0x100);
    move_word_flags(context, 0x100);
    let mut remaining = 20_u16;
    context.set_data_register(
        2,
        (context.data_register(2) & !0xffff) | u32::from(remaining),
    );
    move_word_flags(context, remaining);
    let acquired = loop {
        let old = remaining;
        remaining = remaining.wrapping_sub(1);
        context.set_data_register(
            2,
            (context.data_register(2) & !0xffff) | u32::from(remaining),
        );
        let flags = (u16::from(old == 0) * 0x11)
            | (u16::from(remaining & 0x8000 != 0) << 3)
            | (u16::from(remaining == 0) << 2)
            | (u16::from(old == 0x8000) << 1);
        context.set_status_register((context.status_register() & !0x1f) | flags);
        if remaining == 0 {
            break false;
        }
        let status = context.read_z80_bus_request(OpenBusSource::prefetch(
            ids::LOAD_SOUND_DRIVER_TO_Z80
                .metadata()
                .address(context.revision())
                .wrapping_add(0x1a),
        ))?;
        context.set_data_register(1, (context.data_register(1) & !0xffff) | u32::from(status));
        move_word_flags(context, status);
        context.set_status_register(
            (context.status_register() & !4) | if status & 0x100 == 0 { 4 } else { 0 },
        );
        if status & 0x100 == 0 {
            break true;
        }
    };
    if acquired {
        let mut source = Address::<u8>::new(symbol(context, "SoundDriverProgram"));
        let mut destination = 0xa00000_u32;
        context.set_address_register(0, source.address());
        context.set_address_register(1, destination);
        let mut counter = context.read_u16(symbol(context, "SoundDriverProgramSize"));
        context.set_data_register(0, (context.data_register(0) & !0xffff) | u32::from(counter));
        move_word_flags(context, counter);
        loop {
            let value = source.read(context);
            source = source.offset(1);
            context.set_address_register(0, source.address());
            context.write_device_u8(destination, value);
            destination = destination.wrapping_add(1);
            context.set_address_register(1, destination);
            let nz = if value == 0 {
                4
            } else if value & 0x80 != 0 {
                8
            } else {
                0
            };
            context.set_status_register((context.status_register() & !0x0f) | nz);
            counter = counter.wrapping_sub(1);
            context.set_data_register(0, (context.data_register(0) & !0xffff) | u32::from(counter));
            if counter == u16::MAX {
                break;
            }
        }
        context.write_z80_reset(0);
        move_word_flags(context, 0);
        context.write_z80_bus_request(0);
        move_word_flags(context, 0);
        let first = u32::from(context.data_register(0) as u16)
            .wrapping_mul(u32::from(context.data_register(1) as u16));
        context.set_data_register(0, first);
        let nz = if first == 0 {
            4
        } else if first & 0x8000_0000 != 0 {
            8
        } else {
            0
        };
        context.set_status_register((context.status_register() & !0x0f) | nz);
        context.write_z80_reset(0x100);
        move_word_flags(context, 0x100);
        let second =
            u32::from(first as u16).wrapping_mul(u32::from(context.data_register(1) as u16));
        context.set_data_register(0, second);
        let nz = if second == 0 {
            4
        } else if second & 0x8000_0000 != 0 {
            8
        } else {
            0
        };
        context.set_status_register((context.status_register() & !0x0f) | nz);
    }
    frame.leave(context)?;
    ram::g_music_template_restart_state(context.revision()).write(context, 2);
    move_word_flags(context, 2);
    Ok(None)
}
