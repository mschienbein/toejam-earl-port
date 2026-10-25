//! Compiled operations shared only by pending Earthlings bodies.
//! Production value-only helpers deliberately keep their existing proof contract.
use crate::Context;
use tje_mem::ram;

/// Nearest-target's separately reviewed strict residual proof also checks CCR.
pub(super) fn extend(context: &mut dyn Context, set: bool) {
    context.set_status_register((context.status_register() & !0x10) | if set { 0x10 } else { 0 });
}

pub(super) fn char_flags(context: &mut dyn Context, value: u32) {
    let value = value as u8;
    let nz = (u16::from(value & 0x80 != 0) * 8) | (u16::from(value == 0) * 4);
    context.set_status_register((context.status_register() & 0xfff0) | nz);
}

/// G c_generic_enemy.s:156-178,276-298 and c_spawning_and_banks.s:1913-1928.
/// CMPI.W; either MOVE.W zero, or ADDQ.W's read/write followed by MOVE.W reread.
/// Consumers choose the source's byte or word cache access separately.
pub(super) fn next_cached_index(context: &mut dyn Context) -> u16 {
    let index = ram::g_random_cache_index(context.revision());
    if index.read(context) == 58 {
        index.write(context, 0);
        0
    } else {
        let next = index.read(context).wrapping_add(1);
        index.write(context, next);
        index.read(context)
    }
}
