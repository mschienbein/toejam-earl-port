//! Reset and exception entry prefixes; completion is an explicit non-RTS event.
use crate::{
    Context, Result, call_at,
    call_frame::{CallSite, FrameLayout, LocalFrame},
    execution::WaitRequest,
    exits, ids,
};
use tje_mem::{ram, rom_symbol};

fn test_flags(context: &mut dyn Context, value: u32, sign: u32) {
    let nz = if value == 0 {
        4
    } else if value & sign != 0 {
        8
    } else {
        0
    };
    context.set_status_register((context.status_register() & !0xf) | nz);
}

fn read_device_long(context: &mut dyn Context, address: u32) -> u32 {
    let high = context.read_device_u16(address);
    let low = context.read_device_u16(address.wrapping_add(2));
    (u32::from(high) << 16) | u32::from(low)
}

fn write_device_long(context: &mut dyn Context, address: u32, value: u32) {
    context.write_device_u16(address, (value >> 16) as u16);
    context.write_device_u16(address.wrapping_add(2), value as u16);
}

/// G/src/asm/interrupts.s:49 (`Level6Isr`): save all D0-D7/A0-A6, invoke the
/// original-dispatch VBlank service, restore MOVEM, and RTE from the exception frame.
pub fn level6_isr(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0xff, 0x7f))?;
    let site = CallSite::relative(ids::LEVEL6_ISR, context.revision(), 10);
    call_at(context, ids::HANDLE_V_BLANK, &[], site)?;
    frame.leave(context)?;
    exits::exception_return(ids::LEVEL6_ISR)
}

/// G/src/asm/interrupts.s:59 (`Level4Isr`): the explicit save set is D0-D1/A0-A1;
/// the C handler preserves the remaining registers, then RTE consumes saved SR/PC.
pub fn level4_isr(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x03, 0x03))?;
    let site = CallSite::relative(ids::LEVEL4_ISR, context.revision(), 10);
    call_at(context, ids::HANDLE_H_BLANK, &[], site)?;
    frame.leave(context)?;
    exits::exception_return(ids::LEVEL4_ISR)
}

/// G/src/asm/interrupts.s:69 (`RestartFromResetEntry`): mask interrupts, reload
/// the user's ROM vector SP, and tail-transfer to START without unwinding a caller.
pub fn restart_from_reset_entry(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let site = CallSite::relative(ids::RESTART_FROM_RESET_ENTRY, context.revision(), 4);
    call_at(context, ids::DISABLE_ALL_INTERRUPTS, &[], site)?;
    let vector = rom_symbol(context.revision(), "StartOfRom").expect("catalogued reset vectors");
    let initial_sp = context.read_u32(vector.address);
    context.set_stack_pointer(initial_sp)?;
    exits::tail(ids::RESTART_FROM_RESET_ENTRY, ids::START)
}

/// G/src/asm/interrupts.s:14 (`GameProgram`): clear all work RAM, retaining the
/// full postincrement A0 value, and expose the final real JSR boundary, not an RTS.
pub fn game_program(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let status = context.read_vdp_status();
    test_flags(context, u32::from(status), 0x8000);
    let site = CallSite::relative(ids::GAME_PROGRAM, context.revision(), 10);
    call_at(context, ids::DISABLE_ALL_INTERRUPTS, &[], site)?;
    let mut destination = ram::g_work_ram_start(context.revision()).address();
    for _ in 0..0x4000 {
        context.write_u32(destination, 0);
        destination = destination.wrapping_add(4);
    }
    context.set_address_register(0, destination);
    context.set_data_register(0, (context.data_register(0) & 0xffff_0000) | 0xffff);
    test_flags(context, 0, 0x8000_0000);
    let entry = ids::GAME_PROGRAM.metadata();
    let return_pc = entry
        .address(context.revision())
        .wrapping_add(entry.size(context.revision()));
    exits::call_transfer(ids::GAME_PROGRAM, ids::VDP_INITIALISATION, return_pc)
}

/// G/src/asm/startup.s:23 (`_start`): cold/warm reset prefix. Setup words,
/// hardware addresses, Z80 bootstrap and PSG bytes are read from the user's ROM.
/// The last branch transfers to GameProgram; it never recursively runs the game.
pub fn start(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let ports_ab = read_device_long(context, 0xa10008);
    test_flags(context, ports_ab, 0x8000_0000);
    if ports_ab != 0 {
        return exits::tail(ids::START, ids::GAME_PROGRAM);
    }
    let port_c = context.read_device_u16(0xa1000c);
    test_flags(context, u32::from(port_c), 0x8000);
    if port_c != 0 {
        return exits::tail(ids::START, ids::GAME_PROGRAM);
    }

    let mut data: [u32; 8] = std::array::from_fn(|index| context.data_register(index));
    let mut addresses: [u32; 7] = std::array::from_fn(|index| context.address_register(index));
    let setup = rom_symbol(context.revision(), "SetupValues").expect("catalogued reset setup");
    let mut cursor = setup.address;
    for register in &mut data[5..8] {
        *register = i32::from(context.read_u16(cursor) as i16) as u32;
        cursor = cursor.wrapping_add(2);
    }
    for register in &mut addresses[..5] {
        *register = context.read_u32(cursor);
        cursor = cursor.wrapping_add(4);
    }
    let version = context.read_device_u8(addresses[1].wrapping_sub(0x10ff)) & 0xf;
    if version != 0 {
        write_device_long(context, addresses[1].wrapping_add(0x2f00), 0x5345_4741);
    }
    let _status = context.read_device_u16(addresses[4]);
    data[0] = 0;
    addresses[6] = 0;
    let mut banks = context.stack_banks()?;
    banks.user = 0;
    context.set_stack_banks(banks)?;

    let mut extend = false;
    for _ in 0..24 {
        data[5] = (data[5] & !0xff) | u32::from(context.read_u8(cursor));
        cursor = cursor.wrapping_add(1);
        context.write_device_u16(addresses[4], data[5] as u16);
        let (word, carry) = (data[5] as u16).overflowing_add(data[7] as u16);
        data[5] = (data[5] & 0xffff_0000) | u32::from(word);
        extend = carry;
    }
    data[1] = 0xffff;
    let command = context.read_u32(cursor);
    cursor = cursor.wrapping_add(4);
    write_device_long(context, addresses[4], command);
    context.write_device_u16(addresses[3], 0);
    context.write_device_u16(addresses[1], data[7] as u16);
    context.write_device_u16(addresses[2], data[7] as u16);
    addresses[5] = cursor;
    for (index, &value) in data.iter().enumerate() {
        context.set_data_register(index, value);
    }
    for (index, &value) in addresses.iter().enumerate() {
        context.set_address_register(index, value);
    }
    let move_n = if data[7] & 0x8000 != 0 { 8 } else { 0 };
    loop {
        let busy = context.read_device_u8(addresses[1]) & 1 != 0;
        let flags = move_n | (u16::from(!busy) << 2) | (u16::from(extend) << 4);
        context.set_status_register((context.status_register() & !0x1f) | flags);
        if !busy {
            break;
        }
        context.wait_until(WaitRequest::z80_busack())?;
    }

    for _ in 0..38 {
        let byte = context.read_u8(cursor);
        cursor = cursor.wrapping_add(1);
        context.write_device_u8(addresses[0], byte);
        addresses[0] = addresses[0].wrapping_add(1);
    }
    context.write_device_u16(addresses[2], 0);
    context.write_device_u16(addresses[1], 0);
    context.write_device_u16(addresses[2], data[7] as u16);
    for _ in 0..=u32::from(data[6] as u16) {
        addresses[6] = addresses[6].wrapping_sub(4);
        // MOVE.L to a predecrement destination transfers its low word first.
        context.write_u16(addresses[6].wrapping_add(2), 0);
        context.write_u16(addresses[6], 0);
    }
    for _ in 0..2 {
        let command = context.read_u32(cursor);
        cursor = cursor.wrapping_add(4);
        write_device_long(context, addresses[4], command);
    }
    for _ in 0..32 {
        write_device_long(context, addresses[3], 0);
    }
    let command = context.read_u32(cursor);
    cursor = cursor.wrapping_add(4);
    write_device_long(context, addresses[4], command);
    for _ in 0..20 {
        write_device_long(context, addresses[3], 0);
    }
    for _ in 0..4 {
        let byte = context.read_u8(cursor);
        cursor = cursor.wrapping_add(1);
        context.write_device_u8(addresses[3].wrapping_add(0x11), byte);
    }
    context.write_device_u16(addresses[2], 0);
    // MOVEM uses the old A6 effective address even though A6 is in the load list.
    let mut restored = addresses[6];
    for index in 0..8 {
        let value = context.read_u32(restored);
        restored = restored.wrapping_add(4);
        context.set_data_register(index, value);
    }
    for index in 0..7 {
        let value = context.read_u32(restored);
        restored = restored.wrapping_add(4);
        context.set_address_register(index, value);
    }
    context.set_status_register(0x2700);
    exits::tail(ids::START, ids::GAME_PROGRAM)
}
