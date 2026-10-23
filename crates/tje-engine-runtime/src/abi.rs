//! Numeric callbacks shared by native continuations and generated WebAssembly.
//! A context may only be used synchronously, while its `BusContext` is alive.

use crate::{Bus, ReadKind};
use std::ffi::c_void;

pub struct BusContext<'a> {
    bus: &'a mut dyn Bus,
}

impl<'a> BusContext<'a> {
    pub fn new(bus: &'a mut dyn Bus) -> Self {
        Self { bus }
    }

    pub fn as_mut_ptr(&mut self) -> *mut c_void {
        (self as *mut Self).cast()
    }
}

/// The layout and callback signatures are identical for all native backends.
/// WebAssembly imports the same numeric functions directly, not this table.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct BusCallbacks {
    pub read8: unsafe extern "C" fn(*mut c_void, u32, u32) -> u32,
    pub read16: unsafe extern "C" fn(*mut c_void, u32, u32) -> u32,
    pub write8: unsafe extern "C" fn(*mut c_void, u32, u32),
    pub write16: unsafe extern "C" fn(*mut c_void, u32, u32),
    pub set_read_offset_mclk: unsafe extern "C" fn(*mut c_void, u32),
}

pub const BUS_CALLBACKS: BusCallbacks = BusCallbacks {
    read8: tje_engine_bus_read8,
    read16: tje_engine_bus_read16,
    write8: tje_engine_bus_write8,
    write16: tje_engine_bus_write16,
    set_read_offset_mclk: tje_engine_bus_set_read_offset_mclk,
};

fn read_kind(kind: u32) -> ReadKind {
    match kind {
        0 => ReadKind::Semantic,
        1 => ReadKind::Prefetch,
        2 => ReadKind::Discarded,
        _ => panic!("invalid generated bus read kind: {kind}"),
    }
}

/// # Safety
/// `context` must point to a live, exclusively borrowed `BusContext` for the
/// duration of this synchronous call. `kind` must be a `ReadKind` discriminant.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub unsafe extern "C" fn tje_engine_bus_read8(
    context: *mut c_void,
    address: u32,
    kind: u32,
) -> u32 {
    // SAFETY: the generated-call ABI requires a live stack-scoped context.
    let context = unsafe { &mut *context.cast::<BusContext<'_>>() };
    u32::from(context.bus.read8(address, read_kind(kind)))
}

/// # Safety
/// Same context and read-kind requirements as `tje_engine_bus_read8`.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub unsafe extern "C" fn tje_engine_bus_read16(
    context: *mut c_void,
    address: u32,
    kind: u32,
) -> u32 {
    // SAFETY: the generated-call ABI requires a live stack-scoped context.
    let context = unsafe { &mut *context.cast::<BusContext<'_>>() };
    u32::from(context.bus.read16(address, read_kind(kind)))
}

/// # Safety
/// `context` must point to a live, exclusively borrowed `BusContext` for the
/// duration of this synchronous call. The value is truncated to one bus byte.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub unsafe extern "C" fn tje_engine_bus_write8(context: *mut c_void, address: u32, value: u32) {
    // SAFETY: the generated-call ABI requires a live stack-scoped context.
    let context = unsafe { &mut *context.cast::<BusContext<'_>>() };
    context.bus.write8(address, value as u8);
}

/// # Safety
/// `context` must point to a live, exclusively borrowed `BusContext` for the
/// duration of this synchronous call. The value is truncated to one bus word.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub unsafe extern "C" fn tje_engine_bus_write16(context: *mut c_void, address: u32, value: u32) {
    // SAFETY: the generated-call ABI requires a live stack-scoped context.
    let context = unsafe { &mut *context.cast::<BusContext<'_>>() };
    context.bus.write16(address, value as u16);
}

/// # Safety
/// `context` must point to a live, exclusively borrowed `BusContext` for the
/// duration of this synchronous call.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub unsafe extern "C" fn tje_engine_bus_set_read_offset_mclk(context: *mut c_void, offset: u32) {
    // SAFETY: the generated-call ABI requires a live stack-scoped context.
    let context = unsafe { &mut *context.cast::<BusContext<'_>>() };
    context.bus.set_read_offset_mclk(offset);
}
