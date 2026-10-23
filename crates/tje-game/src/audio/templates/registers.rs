//! Width/CCR operations used by the handwritten scheduling and sequencer code.

use crate::Context;

pub(super) fn nz(context: &mut dyn Context, value: u32, bits: u32) {
    let sign = 1_u32 << (bits - 1);
    let mask = u32::MAX >> (32 - bits);
    let value = value & mask;
    let flags = if value == 0 {
        4
    } else if value & sign != 0 {
        8
    } else {
        0
    };
    context.set_status_register((context.status_register() & !0x0f) | flags);
}

pub(super) fn move_byte(context: &mut dyn Context, register: usize, value: u8) {
    context.set_data_register(
        register,
        (context.data_register(register) & !0xff) | u32::from(value),
    );
    nz(context, u32::from(value), 8);
}

pub(super) fn move_word(context: &mut dyn Context, register: usize, value: u16) {
    context.set_data_register(
        register,
        (context.data_register(register) & !0xffff) | u32::from(value),
    );
    nz(context, u32::from(value), 16);
}

pub(super) fn move_long(context: &mut dyn Context, register: usize, value: u32) {
    context.set_data_register(register, value);
    nz(context, value, 32);
}

pub(super) fn extend_byte(context: &mut dyn Context, register: usize) {
    let word = context.data_register(register) as u8 as i8 as i16 as u16;
    move_word(context, register, word);
}

fn subtract_flags(
    context: &mut dyn Context,
    left: u32,
    right: u32,
    bits: u32,
    extend: bool,
) -> u32 {
    let mask = u32::MAX >> (32 - bits);
    let sign = 1_u32 << (bits - 1);
    let result = left.wrapping_sub(right) & mask;
    let borrow = left < right;
    let overflow = ((left ^ right) & (left ^ result) & sign) != 0;
    let flags = (u16::from(result & sign != 0) << 3)
        | (u16::from(result == 0) << 2)
        | (u16::from(overflow) << 1)
        | u16::from(borrow);
    let status = context.status_register();
    let status = if extend {
        (status & !0x1f) | (u16::from(borrow) << 4)
    } else {
        status & !0x0f
    };
    context.set_status_register(status | flags);
    result
}

pub(super) fn subtract_byte(context: &mut dyn Context, left: u8, right: u8) -> u8 {
    subtract_flags(context, u32::from(left), u32::from(right), 8, true) as u8
}

pub(super) fn subtract_word(context: &mut dyn Context, left: u16, right: u16) -> u16 {
    subtract_flags(context, u32::from(left), u32::from(right), 16, true) as u16
}

pub(super) fn compare_word(context: &mut dyn Context, left: u16, right: u16) {
    subtract_flags(context, u32::from(left), u32::from(right), 16, false);
}

fn add_flags(context: &mut dyn Context, left: u32, right: u32, bits: u32) -> u32 {
    let mask = u32::MAX >> (32 - bits);
    let sign = 1_u32 << (bits - 1);
    let sum = left + right;
    let result = sum & mask;
    let carry = sum > mask;
    let overflow = (!(left ^ right) & (left ^ result) & sign) != 0;
    let flags = (u16::from(carry) << 4)
        | (u16::from(result & sign != 0) << 3)
        | (u16::from(result == 0) << 2)
        | (u16::from(overflow) << 1)
        | u16::from(carry);
    context.set_status_register((context.status_register() & !0x1f) | flags);
    result
}

pub(super) fn add_byte(context: &mut dyn Context, left: u8, right: u8) -> u8 {
    add_flags(context, u32::from(left), u32::from(right), 8) as u8
}

pub(super) fn add_word(context: &mut dyn Context, left: u16, right: u16) -> u16 {
    add_flags(context, u32::from(left), u32::from(right), 16) as u16
}

pub(super) fn shift_word_left(context: &mut dyn Context, register: usize, count: u32) {
    let old = context.data_register(register) as u16;
    let value = old.wrapping_shl(count);
    let carry = old & (1 << (16 - count)) != 0;
    move_word(context, register, value);
    context.set_status_register((context.status_register() & !0x11) | if carry { 0x11 } else { 0 });
}

pub(super) fn shift_word_right(context: &mut dyn Context, register: usize, count: u32) {
    let old = context.data_register(register) as u16;
    let value = old >> count;
    let carry = old & (1 << (count - 1)) != 0;
    move_word(context, register, value);
    context.set_status_register((context.status_register() & !0x11) | if carry { 0x11 } else { 0 });
}
