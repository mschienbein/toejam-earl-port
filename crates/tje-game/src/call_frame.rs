//! Source-backed MCC68K frames for frame-sensitive readable translations.
//!
//! Values are emulated addresses, never host stack pointers. Unwritten locals and
//! omitted arguments retain existing RAM. Ordinary proven value-only calls may
//! elide these frames; pointer/slot/residual-dependent calls must use this API.
use crate::{Context, Dispatcher, Error, FunctionId, Result};
use tje_mem::Address;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    Unavailable,
    UnalignedStack(u32),
    InvalidAddressMask(u8),
    UnexpectedStack { expected: u32, actual: u32 },
    UnexpectedReturn { expected: u32, actual: u32 },
    TooManyArguments(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameState {
    pub entry_sp: u32,
}
impl FrameState {
    pub const fn new(entry_sp: u32) -> Self {
        Self { entry_sp }
    }
    pub const fn argument_slot(self, index: i32) -> Address<u32> {
        Address::new(
            self.entry_sp
                .wrapping_add(4)
                .wrapping_add((index as u32).wrapping_mul(4)),
        )
    }
    pub const fn argument_word(self, index: i32) -> Address<i16> {
        self.argument_slot(index).byte_offset(2).cast()
    }
    pub const fn argument_byte(self, index: i32) -> Address<i8> {
        self.argument_slot(index).byte_offset(3).cast()
    }
}

/// Return PC immediately after the original JSR/BSR, from matching compiler
/// output/link metadata in each revision. Never substitute a made-up sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallSite {
    pub return_pc: u32,
}
impl CallSite {
    pub const fn new(return_pc: u32) -> Self {
        Self { return_pc }
    }
    pub fn relative(caller: FunctionId, revision: tje_mem::Revision, byte_offset: u32) -> Self {
        Self::new(
            caller
                .metadata()
                .address(revision)
                .wrapping_add(byte_offset),
        )
    }
}

/// Exact prologue shape, cited by the translation, not inferred from a C type.
/// Masks are D0-D7/A0-A6 bitsets in ordinary register order, not opcode bit order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameLayout {
    linked: bool,
    local_bytes: u16,
    data_mask: u8,
    address_mask: u8,
}
impl FrameLayout {
    pub const fn link(local_bytes: u16, data_mask: u8, address_mask: u8) -> Self {
        Self {
            linked: true,
            local_bytes,
            data_mask,
            address_mask,
        }
    }
    pub const fn movem(local_bytes: u16, data_mask: u8, address_mask: u8) -> Self {
        Self {
            linked: false,
            local_bytes,
            data_mask,
            address_mask,
        }
    }
    const fn saved_bytes(self) -> u32 {
        4 * (self.data_mask.count_ones() + self.address_mask.count_ones())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct LocalFrame {
    entry: FrameState,
    layout: FrameLayout,
    base: u32,
    save_base: u32,
}
impl LocalFrame {
    pub fn enter(context: &mut dyn Context, layout: FrameLayout) -> Result<Self> {
        if layout.address_mask & 0x80 != 0 {
            return Err(FrameError::InvalidAddressMask(layout.address_mask).into());
        }
        let entry = context.frame_state().ok_or(FrameError::Unavailable)?;
        let mut sp = context.stack_pointer()?;
        aligned(sp)?;
        if layout.linked {
            sp = sp.wrapping_sub(4);
            let old_a6 = context.address_register(6);
            context.write_u32(sp, old_a6);
            context.set_address_register(6, sp);
        }
        let base = sp;
        sp = sp.wrapping_sub(u32::from(layout.local_bytes));
        aligned(sp)?;
        // 68000 predecrement MOVEM transfers highest selected register first.
        for index in (0..7).rev() {
            if layout.address_mask & (1 << index) != 0 {
                sp = sp.wrapping_sub(4);
                let value = context.address_register(index);
                save_movem_long(context, sp, value);
            }
        }
        for index in (0..8).rev() {
            if layout.data_mask & (1 << index) != 0 {
                sp = sp.wrapping_sub(4);
                let value = context.data_register(index);
                save_movem_long(context, sp, value);
            }
        }
        context.set_stack_pointer(sp)?;
        Ok(Self {
            entry,
            layout,
            base,
            save_base: sp,
        })
    }
    pub const fn entry(&self) -> FrameState {
        self.entry
    }
    pub const fn frame_pointer(&self) -> u32 {
        self.base
    }
    pub const fn save_base(&self) -> u32 {
        self.save_base
    }
    pub const fn argument_slot(&self, index: i32) -> Address<u32> {
        self.entry.argument_slot(index)
    }
    pub const fn argument_word(&self, index: i32) -> Address<i16> {
        self.entry.argument_word(index)
    }
    pub const fn argument_byte(&self, index: i32) -> Address<i8> {
        self.entry.argument_byte(index)
    }
    /// Offsets are from A6 after LINK, or the pre-local-allocation SP without LINK.
    /// This intentionally does not bound-check an emulated C array.
    pub const fn local<T>(&self, byte_offset: i32) -> Address<T> {
        Address::new(self.base.wrapping_add(byte_offset as u32))
    }
    /// MOVEM restores from actual RAM; overwritten save slots affect registers
    /// exactly as on the original. Local/save bytes are never cleared on exit.
    pub fn leave(self, context: &mut dyn Context) -> Result<()> {
        let base = if self.layout.linked {
            context.address_register(6)
        } else {
            self.base
        };
        let mut cursor = if self.layout.linked {
            base.wrapping_sub(u32::from(self.layout.local_bytes))
                .wrapping_sub(self.layout.saved_bytes())
        } else {
            context.stack_pointer()?
        };
        for index in 0..8 {
            if self.layout.data_mask & (1 << index) != 0 {
                let value = context.read_u32(cursor);
                context.set_data_register(index, value);
                cursor = cursor.wrapping_add(4);
            }
        }
        for index in 0..7 {
            if self.layout.address_mask & (1 << index) != 0 {
                let value = context.read_u32(cursor);
                context.set_address_register(index, value);
                cursor = cursor.wrapping_add(4);
            }
        }
        if self.layout.linked {
            let fp = context.address_register(6);
            let old_a6 = context.read_u32(fp);
            context.set_address_register(6, old_a6);
            context.set_stack_pointer(fp.wrapping_add(4))?;
        } else {
            context.set_stack_pointer(cursor.wrapping_add(u32::from(self.layout.local_bytes)))?;
        }
        Ok(())
    }
}

// MOVEM.L -(An) transfers its low word first, unlike LINK/JSR stack pushes.
// Authority: vendored m68000 core/instructions/load.rs::movem_predecrement.
fn save_movem_long(context: &mut dyn Context, address: u32, value: u32) {
    context.write_u16(address.wrapping_add(2), value as u16);
    context.write_u16(address, (value >> 16) as u16);
}

/// A caller-owned outgoing argument area. It stays allocated after each callee
/// return, so callers can inspect a mutated slot or reuse it for another call.
#[derive(Debug, PartialEq, Eq)]
pub struct OutgoingArguments {
    base: u32,
    caller_sp: u32,
}
impl OutgoingArguments {
    pub fn push(context: &mut dyn Context, values: &[u32]) -> Result<Self> {
        let caller_sp = context.stack_pointer()?;
        aligned(caller_sp)?;
        let mut sp = caller_sp;
        for &value in values.iter().rev() {
            sp = sp.wrapping_sub(4);
            context.write_u32(sp, value);
        }
        context.set_stack_pointer(sp)?;
        Ok(Self {
            base: sp,
            caller_sp,
        })
    }
    /// Adopt an outgoing area the compiler already reserved. The area is not
    /// initialized or rewritten; this also preserves intentionally omitted args.
    pub fn existing(context: &dyn Context, slot_count: u32) -> Result<Self> {
        let base = context.stack_pointer()?;
        aligned(base)?;
        Ok(Self {
            base,
            caller_sp: base.wrapping_add(slot_count.wrapping_mul(4)),
        })
    }
    pub const fn argument_slot(&self, index: i32) -> Address<u32> {
        Address::new(self.base.wrapping_add((index as u32).wrapping_mul(4)))
    }
    pub fn call(
        &self,
        context: &mut dyn Context,
        dispatcher: Dispatcher,
        id: FunctionId,
        site: CallSite,
    ) -> Result<u32> {
        let parent = context.frame_state().ok_or(FrameError::Unavailable)?;
        let current_sp = context.stack_pointer()?;
        if current_sp != self.base {
            return Err(FrameError::UnexpectedStack {
                expected: self.base,
                actual: current_sp,
            }
            .into());
        }
        let translation = dispatcher.translation(id);
        if translation.is_none() && !dispatcher.allows_original_fallback() {
            return Err(Error::Untranslated(id));
        }
        let class = crate::execution_class(id).ok_or(Error::Unclassified(id))?;
        let caller_class = context.execution_class();
        if !caller_class.permits(class) {
            return Err(Error::ExecutionUnavailable {
                caller: caller_class,
                callee: class,
            });
        }
        let count = usize::from(id.metadata().abi.argument_count);
        if count > 16 {
            return Err(FrameError::TooManyArguments(count).into());
        }
        let entry = FrameState::new(self.base.wrapping_sub(4));
        context.write_u32(entry.entry_sp, site.return_pc);
        context.set_stack_pointer(entry.entry_sp)?;
        context.set_frame_state(entry)?;
        let result = if translation.is_some() {
            let mut arguments = [0_u32; 16];
            for (index, argument) in arguments[..count].iter_mut().enumerate() {
                *argument = entry.argument_slot(index as i32).read(context);
            }
            dispatcher.call(context, id, &arguments[..count])
        } else {
            context.call_original_entry(id, class)
        };
        if result
            .as_ref()
            .is_err_and(|error| error.preserves_call_frame())
        {
            return result;
        }
        let result = match result {
            Ok(value) if translation.is_some() => {
                let sp = context.stack_pointer()?;
                if sp != entry.entry_sp {
                    Err(FrameError::UnexpectedStack {
                        expected: entry.entry_sp,
                        actual: sp,
                    }
                    .into())
                } else {
                    let actual = context.read_u32(sp);
                    context.set_stack_pointer(sp.wrapping_add(4))?;
                    if actual != site.return_pc {
                        Err(FrameError::UnexpectedReturn {
                            expected: site.return_pc,
                            actual,
                        }
                        .into())
                    } else {
                        Ok(value)
                    }
                }
            }
            other => other,
        };
        // Errors are returned, never hidden; restore the suspended caller's
        // frame bookkeeping so a checked diagnostic can report them safely.
        context.set_stack_pointer(self.base)?;
        context.set_frame_state(parent)?;
        result
    }
    pub fn release(self, context: &mut dyn Context) -> Result<()> {
        let actual = context.stack_pointer()?;
        if actual != self.base {
            return Err(FrameError::UnexpectedStack {
                expected: self.base,
                actual,
            }
            .into());
        }
        context.set_stack_pointer(self.caller_sp)
    }
}

fn aligned(sp: u32) -> Result<()> {
    if sp & 1 != 0 {
        Err(FrameError::UnalignedStack(sp).into())
    } else {
        Ok(())
    }
}

pub fn call_at(
    context: &mut dyn Context,
    dispatcher: Dispatcher,
    id: FunctionId,
    arguments: &[u32],
    site: CallSite,
) -> Result<u32> {
    let outgoing = OutgoingArguments::push(context, arguments)?;
    let result = outgoing.call(context, dispatcher, id, site);
    if result
        .as_ref()
        .is_err_and(|error| error.preserves_call_frame())
    {
        return result;
    }
    let release = outgoing.release(context);
    match (result, release) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(value), Ok(())) => Ok(value),
    }
}
