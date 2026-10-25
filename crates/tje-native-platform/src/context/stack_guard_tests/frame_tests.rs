//! Authored logical calls through the real native adapter and framework. No
//! original CPU, cartridge body approval, or inferred missing parent lineage.
use super::*;
use std::cell::Cell;
use tje_game::{
    FramedTranslation,
    call_frame::{self, CallSite, FrameLayout, LocalFrame},
    stack_read::LocalWidth,
};
const OUTPUT: u32 = 0xff9000;
thread_local! { static COPIED_ENTRY: Cell<Option<FrameState>> = const { Cell::new(None) }; }
fn mixed_child(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(8, 0, 0))?;
    frame.local::<u8>(-3).write(context, 0x7f);
    let value = context.read_uninitialized_local(frame.entry().uninitialized_local(
        ids::GET_RANDOM_NUMBER,
        -8,
        LocalWidth::Word,
        "authored native child's own entry SP-8 input",
    ))?;
    context.write_u16(OUTPUT, value as u16);
    frame.leave(context)?;
    Ok(Some(value))
}
fn capture_entry(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(8, 0, 0))?;
    COPIED_ENTRY.with(|entry| entry.set(Some(frame.entry())));
    frame.leave(context)?;
    Ok(Some(0))
}
fn returning_child(_: &mut dyn Context) -> Result<Option<u32>> {
    Ok(None)
}
fn parent_with_sibling(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(8, 0, 0))?;
    call_frame::call_at_void(
        context,
        dispatcher(),
        ids::NO_OP,
        &[],
        CallSite::new(0x2004),
    )?;
    let value = call_frame::call_at(
        context,
        dispatcher(),
        ids::GET_RANDOM_NUMBER,
        &[],
        CallSite::new(0x200a),
    )?;
    frame.leave(context)?;
    Ok(Some(value))
}
static FRAMES: &[FramedTranslation] = &[
    FramedTranslation {
        id: ids::GET_RANDOM_NUMBER,
        function: mixed_child,
    },
    FramedTranslation {
        id: ids::ABS,
        function: parent_with_sibling,
    },
    FramedTranslation {
        id: ids::NO_OP,
        function: returning_child,
    },
];
static CAPTURE: &[FramedTranslation] = &[FramedTranslation {
    id: ids::GET_RANDOM_NUMBER,
    function: capture_entry,
}];
fn dispatcher() -> Dispatcher {
    Dispatcher::hybrid_with_frames(&[], FRAMES)
}
fn seed(platform: &mut NativePlatform) {
    for address in SP - 96..SP {
        platform.devices.memory_mut().write_u8(address, 0x12);
    }
    platform.devices.memory_mut().write_u16(OUTPUT, 0xbeef);
}

#[test]
fn real_prepared_native_child_reads_unchanged_entry_residue_without_an_original() {
    let mut platform = platform();
    seed(&mut platform);
    let value = platform
        .run_readable_root(ids::GET_RANDOM_NUMBER, |context| {
            let frame = LocalFrame::enter(context, FrameLayout::link(8, 0, 0))?;
            let value = call_frame::call_at(
                context,
                dispatcher(),
                ids::GET_RANDOM_NUMBER,
                &[],
                CallSite::new(0x100a),
            )?;
            frame.leave(context)?;
            Ok(value)
        })
        .unwrap();
    assert_eq!(value, 0x127f);
    assert_eq!(platform.devices.memory().read_u16(OUTPUT), 0x127f);
}

#[test]
fn copied_returned_prepared_frame_dto_cannot_mint_another_native_input() {
    let mut platform = platform();
    seed(&mut platform);
    let error = platform
        .run_readable_root(ids::GET_RANDOM_NUMBER, |context| {
            let _frame = LocalFrame::enter(context, FrameLayout::link(8, 0, 0))?;
            call_frame::call_at(
                context,
                Dispatcher::hybrid_with_frames(&[], CAPTURE),
                ids::GET_RANDOM_NUMBER,
                &[],
                CallSite::new(0x100a),
            )?;
            let copied = COPIED_ENTRY.with(|entry| entry.take()).unwrap();
            context.set_frame_state(copied)?;
            context.set_stack_pointer(copied.entry_sp - 16)?;
            context.read_uninitialized_local(copied.uninitialized_local(
                ids::GET_RANDOM_NUMBER,
                -8,
                LocalWidth::Word,
                "copied returned frame cannot revive its input permission",
            ))
        })
        .unwrap_err();
    let read = error.unsupported_stack_read().unwrap();
    assert_eq!(
        (read.reader, read.reason),
        (
            Reader::ExplicitLocal,
            UnsupportedReason::InvalidExplicitLocal
        )
    );
    assert_eq!(platform.devices.memory().read_u16(OUTPUT), 0xbeef);
}

#[test]
fn native_grandchild_return_without_a_sealed_parent_record_keeps_sibling_unsupported() {
    let mut platform = platform();
    seed(&mut platform);
    let error = platform
        .run_readable_root(ids::GET_RANDOM_NUMBER, |context| {
            let _frame = LocalFrame::enter(context, FrameLayout::link(8, 0, 0))?;
            call_frame::call_at(context, dispatcher(), ids::ABS, &[0], CallSite::new(0x100c))
        })
        .unwrap_err();
    let read = error.unsupported_stack_read().unwrap();
    assert_eq!(
        (read.reader, read.reason),
        (
            Reader::ExplicitLocal,
            UnsupportedReason::InvalidExplicitLocal
        )
    );
    assert_eq!(
        platform.devices.memory().read_u16(OUTPUT),
        0xbeef,
        "no fallback operand was fetched or published"
    );
}

#[test]
fn native_bank_round_trip_does_not_rearm_the_outer_frame_read() {
    let mut platform = platform();
    seed(&mut platform);
    let error = platform
        .run_readable_root(ids::GET_RANDOM_NUMBER, |context| {
            let frame = LocalFrame::enter(context, FrameLayout::link(8, 0, 0))?;
            let status = context.registers.status;
            context.set_status_register(status ^ 0x2000);
            context.set_status_register(status);
            context.read_uninitialized_local(frame.entry().uninitialized_local(
                ids::GET_RANDOM_NUMBER,
                -8,
                LocalWidth::Word,
                "real stack-bank round trip does not restore read permission",
            ))
        })
        .unwrap_err();
    assert_eq!(
        error.unsupported_stack_read().unwrap().reason,
        UnsupportedReason::InvalidExplicitLocal
    );
    assert_eq!(platform.devices.memory().read_u16(OUTPUT), 0xbeef);
}
