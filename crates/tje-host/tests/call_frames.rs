//! Exact stack-slot protocol using retail AllocateSpriteSlots and the canonical
//! pending framed Rust body through the same hook/native dispatch contract.
use std::{fs, path::PathBuf};
use tje_game::call_frame::{CallSite, FrameLayout, FrameState, LocalFrame, OutgoingArguments};
use tje_game::{Context, Dispatcher, Error, FramedTranslation, Result, ids};
use tje_host::{Host, Mode, abi::HookContext, hooks::HookRegistry};
use tje_mem::{AddressSpace, Bus, Revision, ram};

const CODE: u32 = 0xfff000;
const OUTPUT: u32 = 0xffe800;
const RESULT: u32 = 0xffe900;

static SELECTOR: &[FramedTranslation] = tje_game::sprites::PENDING_FRAMED_TRANSLATIONS;

fn caller(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    // Authored reference caller below uses precisely this LINK/MOVEM shape.
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 4, 0))?;
    let args = OutgoingArguments::push(context, &[1, 0, 0, OUTPUT])?;
    let dispatcher = context.dispatcher();
    args.call(
        context,
        dispatcher,
        ids::ALLOCATE_SPRITE_SLOTS,
        CallSite::new(CODE + 38),
    )?;
    args.call(
        context,
        dispatcher,
        ids::ALLOCATE_SPRITE_SLOTS,
        CallSite::new(CODE + 44),
    )?;
    let advanced = args.argument_slot(3).read(context);
    context.write_u32(RESULT, advanced);
    args.release(context)?;
    frame.leave(context)?;
    Ok(Some(advanced))
}
fn rom(revision: Revision) -> Vec<u8> {
    let (key, file) = match revision {
        Revision::Rev02 => ("TJE_REV02_ROM", "rev02.bin"),
        Revision::Rev00 => ("TJE_REV00_ROM", "rev00.bin"),
    };
    let path = std::env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../roms").join(file));
    fs::read(path).unwrap()
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn real_sprite_callee_and_rust_callee_share_mutable_slots_and_reuse_in_both_revisions() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for translated_callee in [false, true] {
            let mut registry = HookRegistry::from_entries(
                revision,
                &[],
                if translated_callee { SELECTOR } else { &[] },
            )
            .unwrap();
            let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
            symbol.name = "AuthoredSlotReuseCaller";
            symbol.addresses = [CODE; 2];
            symbol.abi.argument_count = 0;
            symbol.abi.returns_value = true;
            registry.register_contract(symbol, true, caller).unwrap();
            let mut host = Host::new(&rom(revision), Mode::Check, registry).unwrap();
            let target = ids::ALLOCATE_SPRITE_SLOTS.metadata().address(revision);
            // Authored caller: LINK/MOVEM; four argument pushes; two JSRs sharing
            // the same outgoing area; read mutated fourth slot; two ADDQ cleanups.
            let words = [
                0x4e56,
                0xfffc,
                0x48e7,
                0x2000,
                0x2f3c,
                (OUTPUT >> 16) as u16,
                OUTPUT as u16,
                0x2f3c,
                0,
                0,
                0x2f3c,
                0,
                0,
                0x2f3c,
                0,
                1,
                0x4eb9,
                (target >> 16) as u16,
                target as u16,
                0x4eb9,
                (target >> 16) as u16,
                target as u16,
                0x202f,
                12,
                0x23c0,
                (RESULT >> 16) as u16,
                RESULT as u16,
                0x508f,
                0x508f,
                0x4cee,
                4,
                0xfff8,
                0x4e5e,
                0x4e75,
            ];
            for (index, word) in words.into_iter().enumerate() {
                host.context().write_u16(CODE + 2 * index as u32, word);
            }
            for index in 0..54 {
                ram::g_viewport_sprite_slot_flags(revision)
                    .at(index)
                    .write(&mut host.context(), 0);
            }
            let old_sp = host.cpu().stack_pointer();
            let value = host.check_call(CODE, &[]).unwrap();
            assert_eq!(value, OUTPUT + 2);
            assert_eq!(host.context().read_u32(RESULT), OUTPUT + 2);
            assert_eq!(host.context().read_u8(OUTPUT), 25);
            assert_eq!(host.context().read_u8(OUTPUT + 1), 26);
            assert_eq!(host.cpu().stack_pointer(), old_sp);
            assert_eq!(host.checks.mismatches, 0);
            assert!(host.checks.uninitialized_stack_reads.is_empty());
            assert_eq!(
                host.checks.nested_original_calls,
                if translated_callee { 0 } else { 2 }
            );
        }
    }
}

// Native readable context: address space plus register/SP state, no CPU emulator.
struct Native {
    memory: AddressSpace,
    data: [u32; 8],
    address: [u32; 7],
    sp: u32,
    frame: FrameState,
    dispatcher: Dispatcher,
    word_writes: Vec<(u32, u16)>,
    forbidden_read: Option<std::ops::Range<u32>>,
}
impl Native {
    fn new(revision: Revision, dispatcher: Dispatcher) -> Self {
        let sp = 0xff8000;
        Self {
            memory: AddressSpace::new(vec![0; 64], revision),
            data: [0; 8],
            address: [0; 7],
            sp,
            frame: FrameState::new(sp),
            dispatcher,
            word_writes: Vec::new(),
            forbidden_read: None,
        }
    }
}
impl Bus for Native {
    fn read_u8(&self, address: u32) -> u8 {
        assert!(
            !self
                .forbidden_read
                .as_ref()
                .is_some_and(|range| range.contains(&(address & 0xff_ffff))),
            "unused fourth argument was eagerly read at {address:06X}"
        );
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.word_writes.push((address & 0xff_ffff, value));
        self.memory.write_u16(address, value);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.write_u16(address, (value >> 16) as u16);
        self.write_u16(address.wrapping_add(2), value as u16);
    }
}

// A bus observer for authored instructions, independent of the frame helpers.
impl m68000_emu::BusInterface for Native {
    type DebugView<'a> = m68000_emu::debug::DummyM68000Debugger;
    fn read_byte(&mut self, address: u32) -> u8 {
        self.read_u8(address)
    }
    fn read_word(&mut self, address: u32) -> u16 {
        self.read_u16(address)
    }
    fn write_byte(&mut self, address: u32, value: u8) {
        self.write_u8(address, value);
    }
    fn write_word(&mut self, address: u32, value: u16) {
        self.write_u16(address, value);
    }
    fn interrupt_level(&self) -> u8 {
        0
    }
    fn acknowledge_interrupt(&mut self, _: u8) {
        panic!("RAM-only instruction fixture received an interrupt");
    }
    fn halt(&self) -> bool {
        false
    }
    fn reset(&self) -> bool {
        false
    }
}
impl tje_mem::Platform for Native {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("RAM-only frame test wrote VDP")
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("RAM-only frame test wrote VDP")
    }
}
impl tje_game::Platform for Native {
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("RAM-only frame test read device")
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("RAM-only frame test read device")
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("RAM-only frame test wrote device")
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("RAM-only frame test wrote device")
    }
}
impl Context for Native {
    fn revision(&self) -> Revision {
        self.memory.revision()
    }
    fn data_register(&self, index: usize) -> u32 {
        self.data[index]
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.data[index] = value;
    }
    fn address_register(&self, index: usize) -> u32 {
        self.address[index]
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.address[index] = value;
    }
    fn status_register(&self) -> u16 {
        0x2700
    }
    fn set_status_register(&mut self, _: u16) {
        panic!("frame helpers must not alter CCR/SR")
    }
    fn stack_pointer(&self) -> Result<u32> {
        Ok(self.sp)
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.sp = value;
        Ok(())
    }
    fn frame_state(&self) -> Option<FrameState> {
        Some(self.frame)
    }
    fn set_frame_state(&mut self, value: FrameState) -> Result<()> {
        self.frame = value;
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        self.dispatcher
    }
    fn call_original(&mut self, id: tje_game::FunctionId, _: &[u32]) -> Result<u32> {
        Err(Error::Untranslated(id))
    }
}

#[test]
fn native_slots_locals_missing_arguments_and_saved_register_aliases_are_real_ram() {
    let dispatcher = Dispatcher::native_with_frames(&[ids::ALLOCATE_SPRITE_SLOTS], &[], SELECTOR)
        .unwrap_or_else(|_| panic!("test selector has a closed native leaf"));
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Native::new(revision, dispatcher);
        context.data[2] = OUTPUT;
        context.address[6] = 0x12345678;
        let frame = LocalFrame::enter(&mut context, FrameLayout::link(4, 4, 0)).unwrap();
        assert_eq!(frame.frame_pointer(), 0xff7ffc);
        assert_eq!(context.sp, 0xff7ff4);
        assert_eq!(context.read_u32(0xff7ffc), 0x12345678);
        assert_eq!(context.read_u32(0xff7ff4), OUTPUT);
        // Fourth argument deliberately omitted: at callee SP+16 lies the caller's
        // saved D2. It is read as a pointer, incremented, then restored from RAM.
        let outgoing = OutgoingArguments::push(&mut context, &[1, 0, 0]).unwrap();
        outgoing
            .call(
                &mut context,
                dispatcher,
                ids::ALLOCATE_SPRITE_SLOTS,
                CallSite::new(0x1234),
            )
            .unwrap();
        outgoing.release(&mut context).unwrap();
        assert_eq!(context.read_u8(OUTPUT), 25);
        frame.leave(&mut context).unwrap();
        assert_eq!(context.data[2], OUTPUT + 1);
        assert_eq!(context.address[6], 0x12345678);
        assert_eq!(context.sp, 0xff8000);
        assert_eq!(context.frame.entry_sp, 0xff8000);
        // A nonescaping local passed by its emulated address is also writable by
        // the callee; no host slice/pointer ABI or zero-filled residual frame.
        let frame = LocalFrame::enter(&mut context, FrameLayout::link(4, 0, 0)).unwrap();
        let output = frame.local::<i8>(-1);
        dispatcher
            .call_at(
                &mut context,
                ids::ALLOCATE_SPRITE_SLOTS,
                &[1, 0, 0, output.address()],
                CallSite::new(0x5678),
            )
            .unwrap();
        assert_eq!(output.read(&context), 26);
        frame.leave(&mut context).unwrap();
        assert_eq!(context.sp, 0xff8000);
    }
}

#[test]
fn movem_saves_match_real_instruction_word_order_without_reversing_link() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for linked in [false, true] {
            let dispatcher = Dispatcher::hybrid_with(&[]);
            let mut translated = Native::new(revision, dispatcher);
            translated.data = std::array::from_fn(|index| 0x89ab_4567 + index as u32);
            translated.address = std::array::from_fn(|index| 0x1234_5678 + index as u32);
            let mut reference_bus = Native::new(revision, dispatcher);
            let mut cpu = m68000_emu::M68000::default();
            cpu.set_status_register(0x2700);
            cpu.set_data_registers(translated.data);
            cpu.set_address_registers(translated.address, 0xff4000, translated.sp);
            let data_mask = 0x89;
            let address_mask = 0x42;
            let ordinary_mask = u16::from(data_mask) | (u16::from(address_mask) << 8);
            // Authored LINK A6,#-8 or LEA -8(SP),SP, then MOVEM.L
            // D0/D3/D7/A1/A6,-(SP). Opcode masks reverse ordinary register order.
            let words = [
                if linked { 0x4e56 } else { 0x4fef },
                0xfff8,
                0x48e7,
                ordinary_mask.reverse_bits(),
                0x4e71,
                0x4e71,
            ];
            for (index, word) in words.into_iter().enumerate() {
                reference_bus
                    .memory
                    .write_u16(CODE + 2 * index as u32, word);
            }
            cpu.set_pc(CODE, &mut reference_bus);
            cpu.execute_instruction(&mut reference_bus);
            cpu.execute_instruction(&mut reference_bus);
            let layout = if linked {
                FrameLayout::link(8, data_mask, address_mask)
            } else {
                FrameLayout::movem(8, data_mask, address_mask)
            };
            let frame = LocalFrame::enter(&mut translated, layout).unwrap();
            assert_eq!(translated.word_writes, reference_bus.word_writes);
            assert_eq!(translated.sp, cpu.stack_pointer());
            assert_eq!(translated.address, cpu.address_registers());
            assert_eq!(translated.data, cpu.data_registers());
            assert_eq!(frame.save_base(), cpu.stack_pointer());
            for &(address, value) in &reference_bus.word_writes {
                assert_eq!(translated.read_u16(address), value);
            }
        }
    }
}

#[test]
fn unused_omitted_formal_is_never_loaded_and_value_only_entry_fails_closed() {
    let dispatcher = Dispatcher::native_with_frames(&[ids::ALLOCATE_SPRITE_SLOTS], &[], SELECTOR)
        .unwrap_or_else(|_| panic!("test selector has a closed native leaf"));
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Native::new(revision, dispatcher);
        for index in 0..54 {
            ram::g_viewport_sprite_slot_flags(revision)
                .at(index)
                .write(&mut context, 1);
        }
        let frame = LocalFrame::enter(&mut context, FrameLayout::link(4, 0, 0)).unwrap();
        let unused = frame.local::<u32>(-4).address();
        context.forbidden_read = Some(unused..unused + 4);
        let outgoing = OutgoingArguments::push(&mut context, &[1, 0, 0]).unwrap();
        let value = outgoing
            .call(
                &mut context,
                dispatcher,
                ids::ALLOCATE_SPRITE_SLOTS,
                CallSite::new(0x1234),
            )
            .unwrap();
        assert_eq!(value, 1);
        outgoing.release(&mut context).unwrap();
        frame.leave(&mut context).unwrap();
        assert_eq!(context.sp, 0xff8000);
        assert_eq!(
            dispatcher.call(&mut context, ids::ALLOCATE_SPRITE_SLOTS, &[1, 0, 0]),
            Err(Error::Frame(
                tje_game::call_frame::FrameError::PreparedEntryRequired(ids::ALLOCATE_SPRITE_SLOTS)
            ))
        );
    }
}

#[test]
fn odd_return_pc_fails_before_materialization_and_legacy_prepared_calls_still_work() {
    let dispatcher = Dispatcher::native(&[ids::ABS]).unwrap();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Native::new(revision, dispatcher);
        let parent = context.frame;
        assert_eq!(
            dispatcher.call_at(&mut context, ids::ABS, &[5], CallSite::new(0x1235)),
            Err(Error::Frame(
                tje_game::call_frame::FrameError::UnalignedReturnPc(0x1235)
            ))
        );
        assert_eq!(context.sp, parent.entry_sp);
        assert!(context.word_writes.is_empty());
        let outgoing = OutgoingArguments::push(&mut context, &[5]).unwrap();
        let writes = context.word_writes.clone();
        assert_eq!(
            outgoing.call(&mut context, dispatcher, ids::ABS, CallSite::new(0x1235)),
            Err(Error::Frame(
                tje_game::call_frame::FrameError::UnalignedReturnPc(0x1235)
            ))
        );
        assert_eq!(context.word_writes, writes);
        assert_eq!(
            outgoing.call(&mut context, dispatcher, ids::ABS, CallSite::new(0x1234)),
            Ok(5)
        );
        assert_eq!(context.sp, parent.entry_sp - 4);
        assert_eq!(context.frame, parent);
        outgoing.release(&mut context).unwrap();
    }
}

fn failure_without_output(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0, 0))?;
    let dispatcher = context.dispatcher();
    let outgoing = OutgoingArguments::push(context, &[1, 0, 0])?;
    let value = outgoing.call(
        context,
        dispatcher,
        ids::ALLOCATE_SPRITE_SLOTS,
        CallSite::new(CODE + 28),
    )?;
    outgoing.release(context)?;
    frame.leave(context)?;
    Ok(Some(value))
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn original_and_framed_selector_do_not_consume_an_unused_omitted_output_pointer() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for translated_callee in [false, true] {
            let mut registry = HookRegistry::from_entries(
                revision,
                &[],
                if translated_callee { SELECTOR } else { &[] },
            )
            .unwrap();
            let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
            symbol.name = "AuthoredOmittedUnusedSpriteOutput";
            symbol.addresses = [CODE; 2];
            symbol.abi.argument_count = 0;
            symbol.abi.returns_value = true;
            registry
                .register_contract(symbol, true, failure_without_output)
                .unwrap();
            let mut host = Host::new(&rom(revision), Mode::Check, registry).unwrap();
            let target = ids::ALLOCATE_SPRITE_SLOTS.metadata().address(revision);
            // Authored LINK, three immediate pushes, JSR, caller cleanup, UNLK, RTS.
            let words = [
                0x4e56,
                0xfffc,
                0x2f3c,
                0,
                0,
                0x2f3c,
                0,
                0,
                0x2f3c,
                0,
                1,
                0x4eb9,
                (target >> 16) as u16,
                target as u16,
                0x4fef,
                12,
                0x4e5e,
                0x4e75,
            ];
            for (index, word) in words.into_iter().enumerate() {
                host.context().write_u16(CODE + 2 * index as u32, word);
            }
            for index in 0..54 {
                ram::g_viewport_sprite_slot_flags(revision)
                    .at(index)
                    .write(&mut host.context(), 1);
            }
            assert_eq!(host.check_call(CODE, &[]).unwrap(), 1);
            assert_eq!(host.checks.mismatches, 0);
            assert!(host.checks.uninitialized_stack_reads.is_empty());
            assert!(host.checks.nondependent_stack_reads.is_empty());
        }
    }
}

#[test]
fn explicit_framed_entry_never_preloads_slots_or_ignores_value_arguments() {
    fn records_entry(context: &mut dyn Context) -> Result<Option<u32>> {
        context.set_data_register(7, 0x1234);
        Ok(Some(9))
    }
    static ENTRY: &[FramedTranslation] = &[FramedTranslation {
        id: ids::ABS,
        function: records_entry,
    }];
    let dispatcher = Dispatcher::native_with_frames(&[ids::ABS], &[], ENTRY).unwrap();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Native::new(revision, dispatcher);
        let entry = context.frame;
        context.forbidden_read = Some(entry.entry_sp + 4..entry.entry_sp + 20);
        assert_eq!(
            tje_game::exits::enter(&mut context, ids::ABS, &[5]),
            Err(Error::Frame(
                tje_game::call_frame::FrameError::UnexpectedValueArguments {
                    id: ids::ABS,
                    count: 1,
                }
            ))
        );
        assert_eq!(context.data, [0; 8]);
        assert_eq!(
            context.frame, entry,
            "value-only admission cannot mark a prepared frame"
        );
        assert!(context.word_writes.is_empty());
        assert_eq!(
            tje_game::exits::enter(&mut context, ids::ABS, &[]).unwrap(),
            tje_game::exits::EntryOutcome::Returned(Some(9))
        );
        assert_eq!(context.data[0], 9);
        assert_eq!(context.data[7], 0x1234);
        assert_eq!(context.frame.entry_sp, entry.entry_sp);
        assert!(context.frame.is_prepared_for(ids::ABS));
        assert_eq!(context.sp, entry.entry_sp);
        assert!(context.word_writes.is_empty());
    }
}
