//! Authored memory observations, not a cartridge, oracle or implementation fallback.
//! This arithmetic/annotation/dataflow model deliberately keeps the default
//! no-op call observer, which mints no child token. It provides no Host/Native/
//! Replace invocation, lifetime, witness, admission or proof coverage.
use super::{dispatch, friendly, generic, interactions};
use crate::{Context, Dispatcher, FunctionId, Result, core::test_support::TestContext};
use std::{cell::RefCell, collections::VecDeque};
use tje_mem::{Bus, Revision, ram, rom};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Read {
    address: u32,
    width: u8,
    discarded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Write {
    address: u32,
    width: u8,
    value: u32,
}

pub(super) struct ObservedContext {
    inner: TestContext,
    reads: RefCell<Vec<Read>>,
    writes: RefCell<Vec<Write>>,
    guard: std::sync::Arc<crate::stack_read::Guard>,
    declared_locals: Vec<(crate::stack_read::UninitializedLocalRead, u8, u32)>,
    changing_byte: Option<(u32, RefCell<VecDeque<u8>>)>,
    waits: Vec<crate::execution::WaitRequest>,
    frame: crate::call_frame::FrameState,
    dispatcher: Dispatcher,
    class: crate::execution::ExecutionClass,
    clock: crate::execution::CallClock,
}
impl ObservedContext {
    pub(super) fn new(revision: Revision) -> Self {
        Self::with_rom(revision, vec![0; 0x100000])
    }
    pub(super) fn with_rom(revision: Revision, image: Vec<u8>) -> Self {
        let mut inner = TestContext::with_rom(revision, image);
        inner.set_address_register(7, 0xff7e00);
        Self {
            inner,
            reads: RefCell::new(Vec::new()),
            writes: RefCell::new(Vec::new()),
            guard: std::sync::Arc::new(crate::stack_read::Guard::default()),
            declared_locals: Vec::new(),
            changing_byte: None,
            waits: Vec::new(),
            frame: crate::call_frame::FrameState::new(0xff7e00),
            dispatcher: Dispatcher::hybrid(),
            class: crate::execution::ExecutionClass::Atomic,
            clock: crate::execution::CallClock::default(),
        }
    }
    pub(super) fn prepare(&mut self, arguments: &[u32]) {
        self.frame = crate::call_frame::FrameState::new(0xff7e00);
        self.inner.set_address_register(7, self.frame.entry_sp);
        for (index, &value) in arguments.iter().enumerate() {
            self.frame
                .argument_slot(index as i32)
                .write(&mut self.inner, value);
        }
    }
    fn byte_value(&self, address: u32) -> u8 {
        if let Some((observed, values)) = &self.changing_byte
            && *observed == address
        {
            return values
                .borrow_mut()
                .pop_front()
                .expect("each source byte observation");
        }
        self.inner.memory.read_u8(address)
    }
    fn record(&self, address: u32, width: u8, discarded: bool) {
        self.reads.borrow_mut().push(Read {
            address,
            width,
            discarded,
        });
    }
    fn outside_frame_reads(&self) -> Vec<Read> {
        self.reads
            .borrow()
            .iter()
            .filter(|read| !(0xff7c00..0xff7f00).contains(&read.address))
            .copied()
            .collect()
    }
}
impl Bus for ObservedContext {
    fn read_u8(&self, address: u32) -> u8 {
        self.guard
            .read(address, 1, crate::stack_read::Reader::RustMemory);
        self.record(address, 1, false);
        self.byte_value(address)
    }
    fn read_u16(&self, address: u32) -> u16 {
        self.guard
            .read(address, 2, crate::stack_read::Reader::RustMemory);
        self.record(address, 2, false);
        self.inner.memory.read_u16(address)
    }
    fn read_u32(&self, address: u32) -> u32 {
        (u32::from(self.read_u16(address)) << 16)
            | u32::from(self.read_u16(address.wrapping_add(2)))
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.writes.borrow_mut().push(Write {
            address,
            width: 1,
            value: u32::from(value),
        });
        self.inner.write_u8(address, value);
        self.guard.commit(address, 1, true);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.writes.borrow_mut().push(Write {
            address,
            width: 2,
            value: u32::from(value),
        });
        self.inner.write_u16(address, value);
        self.guard.commit(address, 2, true);
    }
}
impl tje_mem::Platform for ObservedContext {
    fn write_vdp_control(&mut self, value: u16) {
        tje_mem::Platform::write_vdp_control(&mut self.inner, value);
    }
    fn write_vdp_data(&mut self, value: u16) {
        tje_mem::Platform::write_vdp_data(&mut self.inner, value);
    }
}
impl crate::Platform for ObservedContext {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        crate::Platform::read_device_u8(&mut self.inner, address)
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        crate::Platform::read_device_u16(&mut self.inner, address)
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        crate::Platform::write_device_u8(&mut self.inner, address, value);
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        crate::Platform::write_device_u16(&mut self.inner, address, value);
    }
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.record(address, 1, true);
        let _ = self.byte_value(address);
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.record(address, 2, true);
        let _ = self.inner.memory.read_u16(address);
        Ok(())
    }
    fn wait_until(&mut self, request: crate::execution::WaitRequest) -> Result<()> {
        use crate::execution::{RamWidth, WaitCondition};
        self.waits.push(request);
        let WaitCondition::Ram {
            address,
            width: RamWidth::Byte,
            mask,
            value,
            comparison,
        } = request.condition
        else {
            panic!("authored upload fixture requires an actual byte RAM predicate");
        };
        // Authored successive samples model the wait's reads, not live time.
        // Pop exhaustion deliberately rejects an extra poll after the final zero.
        while !comparison.satisfied(u32::from(self.read_u8(address)), mask, value) {}
        Ok(())
    }
}
impl Context for ObservedContext {
    fn revision(&self) -> Revision {
        self.inner.revision()
    }
    fn data_register(&self, index: usize) -> u32 {
        self.inner.data_register(index)
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.inner.set_data_register(index, value);
    }
    fn address_register(&self, index: usize) -> u32 {
        self.inner.address_register(index)
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.inner.set_address_register(index, value);
    }
    fn status_register(&self) -> u16 {
        self.inner.status_register()
    }
    fn set_status_register(&mut self, value: u16) {
        self.inner.set_status_register(value);
    }
    fn dispatcher(&self) -> Dispatcher {
        self.dispatcher
    }
    fn call_original(&mut self, function: FunctionId, args: &[u32]) -> Result<u32> {
        self.inner.call_original(function, args)
    }
    fn stack_pointer(&self) -> Result<u32> {
        Ok(self.inner.address_register(7))
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.inner.set_address_register(7, value);
        Ok(())
    }
    fn frame_state(&self) -> Option<crate::call_frame::FrameState> {
        Some(self.frame)
    }
    fn set_frame_state(&mut self, state: crate::call_frame::FrameState) -> Result<()> {
        self.frame = state;
        Ok(())
    }
    fn read_uninitialized_local(
        &mut self,
        read: crate::stack_read::UninitializedLocalRead,
    ) -> Result<u32> {
        use crate::stack_read::LocalWidth;
        let address = self.guard.explicit_local(
            read,
            self.frame,
            self.inner.address_register(7),
            self.inner.status_register() & 0x2000 != 0,
        )?;
        let missing = self.guard.unwritten_mask(address, read.width.bytes());
        let value = match read.width {
            LocalWidth::Byte => u32::from(self.inner.memory.read_u8(address)),
            LocalWidth::Word => u32::from(self.inner.memory.read_u16(address)),
            LocalWidth::Long => self.inner.memory.read_u32(address),
        };
        self.record(address, read.width.bytes(), false);
        self.declared_locals.push((read, missing, value));
        Ok(value)
    }
    fn execution_class(&self) -> crate::execution::ExecutionClass {
        self.class
    }
    fn set_execution_class(&mut self, class: crate::execution::ExecutionClass) -> Result<()> {
        self.class = class;
        Ok(())
    }
    fn clocked_execution(&self) -> bool {
        self.clock.active() || self.class != crate::execution::ExecutionClass::Atomic
    }
    fn enter_call_ceiling(
        &mut self,
        function: FunctionId,
    ) -> Result<crate::execution::CallCeilingScope> {
        self.clock
            .enter(
                function,
                crate::execution_contract(function)
                    .and_then(|contract| contract.operational_ceiling)
                    .unwrap_or(crate::execution::CallCeiling::DEFAULT),
            )
            .map_err(crate::Error::Clock)
    }
    fn leave_call_ceiling(&mut self, scope: crate::execution::CallCeilingScope) -> Result<()> {
        self.clock
            .leave(scope)
            .map(|_| ())
            .map_err(crate::Error::Clock)
    }
}

#[test]
fn life_request_handle_guard_precedes_every_player_state_read() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ObservedContext::new(revision);
        ram::g_life_request_display_entity(revision).write(&mut context, 6);
        context.prepare(&[1]);
        interactions::player_process_life_request(&mut context).unwrap();
        assert_eq!(
            context.outside_frame_reads(),
            [
                Read {
                    address: ram::g_player1_map_level(revision).address(),
                    width: 1,
                    discarded: false
                },
                Read {
                    address: ram::g_life_request_display_entity(revision).address(),
                    width: 2,
                    discarded: false
                },
                Read {
                    address: ram::g_life_request_pending(revision).address(),
                    width: 2,
                    discarded: false
                },
            ]
        );
    }
}

#[test]
fn life_request_compiled_comparisons_do_not_reuse_an_earlier_state_byte() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for states in [vec![1, 4], vec![1, 1, 76]] {
            let mut context = ObservedContext::new(revision);
            let player = ram::g_player_info(revision).at(0);
            ram::g_player_lives(revision).at(0).write(&mut context, 3);
            context.changing_byte = Some((
                player.sprite_state().address(),
                RefCell::new(states.clone().into()),
            ));
            context.prepare(&[1]);
            interactions::player_process_life_request(&mut context).unwrap();
            assert_eq!(
                context
                    .reads
                    .borrow()
                    .iter()
                    .filter(|read| read.address == player.sprite_state().address())
                    .count(),
                states.len()
            );
            assert!(
                context
                    .changing_byte
                    .as_ref()
                    .unwrap()
                    .1
                    .borrow()
                    .is_empty()
            );
        }
    }
}

#[test]
fn life_request_upload_predicate_owns_all_byte_polls_including_final_zero() {
    fn free_display(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
        Ok(None)
    }
    fn stop_after_upload(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
        Err(crate::Error::Platform(
            "authored post-upload boundary".into(),
        ))
    }
    static CALLEES: &[crate::Translation] = &[
        crate::Translation {
            id: crate::ids::FREE_ENTITY,
            function: free_display,
        },
        crate::Translation {
            id: crate::ids::DISABLE_BLANKING_GRAPHICS,
            function: stop_after_upload,
        },
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        // Cover all possible initial TST.B values, plus changing busy values.
        let sequences = (0..=u8::MAX)
            .map(|busy| if busy == 0 { vec![0] } else { vec![busy, 0] })
            .chain([vec![0x80, 1, 0xff, 0]]);
        for samples in sequences {
            let mut context = ObservedContext::new(revision);
            context.dispatcher = Dispatcher::hybrid_with_overrides(CALLEES);
            context.class = crate::execution::ExecutionClass::TimeAware;
            ram::g_life_request_display_entity(revision).write(&mut context, 6);
            ram::g_life_request_pending(revision).write(&mut context, 1);
            ram::g_player_interaction_responses(revision)
                .at(0)
                .write(&mut context, 2);
            let address = ram::g_sprite_graphics_upload_pending(revision).address();
            context.changing_byte = Some((address, RefCell::new(samples.clone().into())));
            context.prepare(&[1]);
            assert_eq!(
                interactions::player_process_life_request(&mut context),
                Err(crate::Error::Platform(
                    "authored post-upload boundary".into()
                ))
            );
            assert_eq!(
                context.waits,
                [crate::execution::WaitRequest::ram_byte_equal(
                    address,
                    u8::MAX,
                    0
                )]
            );
            assert_eq!(
                context
                    .reads
                    .borrow()
                    .iter()
                    .filter(|read| read.address == address)
                    .copied()
                    .collect::<Vec<_>>(),
                vec![
                    Read {
                        address,
                        width: 1,
                        discarded: false,
                    };
                    samples.len()
                ]
            );
            assert!(
                context
                    .changing_byte
                    .as_ref()
                    .unwrap()
                    .1
                    .borrow()
                    .is_empty()
            );
        }
    }
}

#[test]
fn wiseman_compares_second_player_level_before_first_player_level() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ObservedContext::new(revision);
        let record = ram::g_earthling_banks(revision).at(0).at(0);
        record.object().write(&mut context, 6);
        ram::g_player_wiseman_interaction_states(revision)
            .at(0)
            .write(&mut context, 1);
        ram::g_player_wiseman_interaction_states(revision)
            .at(1)
            .write(&mut context, 1);
        context.prepare(&[0, record.address()]);
        friendly::ai_wiseman_touch(&mut context).unwrap();
        let first = ram::g_player_info(revision).at(0).level().address();
        let second = ram::g_player_info(revision).at(1).level().address();
        let levels: Vec<_> = context
            .reads
            .borrow()
            .iter()
            .filter(|read| read.address == first || read.address == second)
            .map(|read| read.address)
            .collect();
        assert_eq!(&levels[levels.len() - 2..], &[second, first]);
    }
}

#[test]
fn contact_range_arguments_load_y_before_x_for_each_player() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let definition = rom::earthling_definitions(revision).at(1);
        let mut image = vec![0_u8; 0x100000];
        image[definition.contact_distance_x().address() as usize] = 13;
        image[definition.contact_distance_y().address() as usize] = 15;
        let mut context = ObservedContext::with_rom(revision, image);
        ram::g_earthling_bank_users(revision)
            .at(0)
            .write(&mut context, 1);
        for slot in 0..20 {
            ram::g_earthling_banks(revision)
                .at(0)
                .at(slot)
                .kind()
                .write(&mut context, -1);
        }
        let record = ram::g_earthling_banks(revision).at(0).at(0);
        record.kind().write(&mut context, 1);
        record.object().write(&mut context, 6);
        for number in 0..2 {
            ram::g_player_info(revision)
                .at(number)
                .x()
                .write(&mut context, 1000);
        }
        context.prepare(&[]);
        dispatch::earthling_dispatch_player_contacts(&mut context).unwrap();
        let x = definition.contact_distance_x().address();
        let y = definition.contact_distance_y().address();
        let dimensions: Vec<_> = context
            .reads
            .borrow()
            .iter()
            .filter(|read| read.address == x || read.address == y)
            .map(|read| read.address)
            .collect();
        assert_eq!(dimensions, [y, x, y, x]);
    }
}

#[test]
fn spawn_reloads_y_output_pointer_after_an_aliasing_x_store() {
    fn random_origin(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
        Ok(Some(0))
    }
    fn place_at_origin(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
        tje_mem::Address::<i16>::new(arguments[3]).write(context, 0);
        tje_mem::Address::<i16>::new(arguments[4]).write(context, 0);
        Ok(Some(1))
    }
    fn clear_overlap(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
        Ok(Some(1))
    }
    static CALLEES: &[crate::Translation] = &[
        crate::Translation {
            id: crate::ids::GET_RANDOM_NUMBER,
            function: random_origin,
        },
        crate::Translation {
            id: crate::ids::TERRAIN_VALIDATION,
            function: place_at_origin,
        },
        crate::Translation {
            id: crate::ids::CHECK_EARTHLING_OVERLAP,
            function: clear_overlap,
        },
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ObservedContext::new(revision);
        context.dispatcher = Dispatcher::hybrid_with_overrides(CALLEES);
        ram::g_player1_map_level(revision).write(&mut context, 1);
        // Authored callee fixtures isolate caller aliasing, not a ROM check.
        // X replaces Y-pointer's low word: FF:F002 becomes FF:0080.
        let output_x = context.frame.argument_slot(3).address() + 2;
        context.prepare(&[8, 0, output_x, 0xfff002, 1]);
        let result = super::framed::find_spawn_location_for_earthling(&mut context).unwrap();
        assert_eq!(result, Some(0));
        assert_eq!(context.inner.memory.read_u16(output_x), 128);
        assert_eq!(context.inner.memory.read_u16(0xff0080), 64);
        assert_eq!(context.inner.memory.read_u16(0xfff002), 0);
        assert_eq!(context.stack_pointer().unwrap(), context.frame.entry_sp);
    }
}

#[test]
fn dispatcher_empty_banks_and_inventory_guard_keep_source_read_order() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for function in [
            dispatch::earthling_dispatch_abilities as crate::FramedTranslationFn,
            dispatch::earthling_dispatch_player_contacts,
        ] {
            let mut context = ObservedContext::new(revision);
            context.prepare(&[]);
            function(&mut context).unwrap();
            assert_eq!(
                context.outside_frame_reads(),
                [
                    Read {
                        address: ram::g_earthling_bank_users(revision).at(0).address(),
                        width: 1,
                        discarded: false
                    },
                    Read {
                        address: ram::g_earthling_bank_users(revision).at(1).address(),
                        width: 1,
                        discarded: false
                    },
                ]
            );
        }
        let mut context = ObservedContext::new(revision);
        context.prepare(&[]);
        ram::g_player_inventory_open(revision)
            .at(0)
            .write(&mut context, 1);
        ram::g_player2_inventory_open(revision).write(&mut context, -1);
        dispatch::process_earthlings(&mut context).unwrap();
        assert_eq!(
            context.outside_frame_reads(),
            [
                Read {
                    address: ram::g_player_inventory_open(revision).at(0).address(),
                    width: 1,
                    discarded: false
                },
                Read {
                    address: ram::g_player2_inventory_open(revision).address(),
                    width: 1,
                    discarded: false
                },
            ]
        );
    }
}

#[test]
fn nearest_early_return_keeps_the_last_emitted_extend_source() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (sprite, maximum, separation, decoy, expected_x) in [
            (2, 1, 0_i16, 0, false),
            (2, 1, 2, 0, true),
            (2, 1, -2, 0, true),
            (0, 1, 2, 0, false),
            (0, 0x101, 0, 0, true),
            (0, 0x201, 2, 0, false),
            (2, 1, 0, -1, true),
            (2, 1, 2, 2, false),
        ] {
            let mut context = ObservedContext::new(revision);
            let state = ram::g_player_info(revision).at(6);
            state.level().write(&mut context, 3);
            state.sprite_state().write(&mut context, sprite);
            ram::g_player_decoy_entities(revision)
                .at(0)
                .write(&mut context, decoy);
            context.prepare(&[state.address(), maximum, separation as u32, 1]);
            context.set_status_register(0x201f);
            assert_eq!(
                super::framed::earthling_steer_toward_nearest_target(&mut context).unwrap(),
                Some(u32::MAX),
            );
            assert_eq!(
                context.status_register(),
                0x2008 | if expected_x { 0x10 } else { 0 }
            );
        }
    }
}

#[test]
fn retaining_an_existing_bank_loads_only_the_short_argument_and_wraps_the_counter() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for old in [1_u8, 127, 128, 255] {
            let mut context = ObservedContext::new(revision);
            ram::g_earthling_bank_users(revision)
                .at(0)
                .write(&mut context, old as i8);
            ram::g_earthling_bank_levels(revision)
                .at(0)
                .write(&mut context, 3);
            context.prepare(&[0x7654_0003]);
            super::framed::earthling_retain_level_bank(&mut context).unwrap();
            let entry = context.frame.entry_sp;
            assert!(
                context
                    .reads
                    .borrow()
                    .iter()
                    .any(|read| read.address == entry + 6 && read.width == 2)
            );
            assert!(
                !context
                    .reads
                    .borrow()
                    .iter()
                    .any(|read| read.address == entry + 4)
            );
            let stored = old.wrapping_add(1);
            assert_eq!(
                ram::g_earthling_bank_users(revision).at(0).read(&context),
                stored as i8
            );
        }
    }
}

#[test]
fn invalid_mailbox_contact_uses_a_discarded_clr_byte_read() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ObservedContext::new(revision);
        let record = ram::g_earthling_banks(revision).at(0).at(0);
        record.object().write(&mut context, 6);
        ram::g_player_info(revision)
            .at(0)
            .sprite_state()
            .write(&mut context, 6);
        let contact = ram::g_mailbox_level_states(revision).at(0).contact().at(0);
        contact.write(&mut context, 2);
        context.prepare(&[0, record.address()]);
        friendly::ai_mailbox_touch(&mut context).unwrap();
        assert!(context.reads.borrow().iter().any(|read| *read
            == Read {
                address: contact.address(),
                width: 1,
                discarded: true,
            }));
        assert_eq!(contact.read(&context), 0);
    }
}

#[test]
fn mailbox_speaker_load_occurs_after_the_earlier_outgoing_argument_pushes() {
    fn within_range(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
        Ok(Some(1))
    }
    fn observe_speaker(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
        tje_mem::Address::<u32>::new(0xff0100).write(context, arguments[0]);
        Ok(Some(0))
    }
    fn choice(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
        Ok(None)
    }
    static CALLEES: &[crate::Translation] = &[
        crate::Translation {
            id: crate::ids::ENTITY_IS_WITHIN_RANGE,
            function: within_range,
        },
        crate::Translation {
            id: crate::ids::GENERATE_SPEECH,
            function: observe_speaker,
        },
        crate::Translation {
            id: crate::ids::PLAYER_CREATE_INTERACTION_CHOICE_DISPLAY,
            function: choice,
        },
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ObservedContext::new(revision);
        context.dispatcher = Dispatcher::hybrid_with_overrides(CALLEES);
        context.class = crate::execution::ExecutionClass::TimeAware;
        // Authored alias: the object byte is the second byte of the future
        // text slot. The earlier range-call X argument first writes zero there.
        let record = tje_mem::Address::<tje_mem::EarthlingState>::new(context.frame.entry_sp - 32);
        record.object().write(&mut context, 6);
        context.prepare(&[0, record.address()]);
        friendly::ai_mailbox_touch(&mut context).unwrap();
        let metadata = crate::ids::AI_MAILBOX_TOUCH.metadata();
        let expected = ((metadata.address(revision) + metadata.size(revision)) >> 16) as u8;
        assert_ne!(expected, 0);
        assert_eq!(context.inner.memory.read_u32(0xff0100), u32::from(expected));
    }
}

#[test]
fn generic_supplies_the_compiled_a3_for_nearest_targets_residual_bug() {
    fn inspect_target_input(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
        assert_eq!(
            context.address_register(3),
            ram::g_random_cache_index(context.revision()).address()
        );
        Ok(Some(u32::MAX))
    }
    static CALLEE: &[crate::Translation] = &[crate::Translation {
        id: crate::ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
        function: inspect_target_input,
    }];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let definition = rom::earthling_definitions(revision).at(14);
        let mut image = vec![0_u8; 0x100000];
        image[definition.target_chance().address() as usize] = 1;
        image[definition.target_distance().address() as usize] = 255;
        let mut context = ObservedContext::with_rom(revision, image);
        context.dispatcher = Dispatcher::hybrid_with_overrides(CALLEE);
        let record = ram::g_earthling_banks(revision).at(0).at(0);
        record.kind().write(&mut context, 14);
        record.object().write(&mut context, 6);
        record.state().write(&mut context, 2);
        ram::g_player_info(revision)
            .at(6)
            .sprite_state()
            .write(&mut context, 2);
        context.prepare(&[record.address()]);
        context.set_address_register(3, 0xff1234);
        generic::ai_generic_enemy_think(&mut context).unwrap();
        assert_eq!(context.address_register(3), 0xff1234);
    }
}

#[test]
fn pending_cached_index_keeps_cmpi_rmw_and_post_store_reread_distinct() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for old in [0_u16, 58, u16::MAX] {
            let mut context = ObservedContext::new(revision);
            let index = ram::g_random_cache_index(revision);
            index.write(&mut context, old);
            let actual = super::pending_access::next_cached_index(&mut context);
            assert_eq!(actual, if old == 58 { 0 } else { old.wrapping_add(1) });
            assert_eq!(context.reads.borrow().len(), if old == 58 { 1 } else { 3 });
            assert!(context.reads.borrow().iter().all(|read| *read
                == Read {
                    address: index.address(),
                    width: 2,
                    discarded: false,
                }));
        }
    }
}

#[test]
fn open_present_declares_only_the_original_mixed_player_word() {
    fn stop_at_first_random(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
        Err(crate::Error::Platform(
            "authored opener prefix boundary".into(),
        ))
    }
    static CALLEE: &[crate::Translation] = &[crate::Translation {
        id: crate::ids::GET_RANDOM_NUMBER,
        function: stop_at_first_random,
    }];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for high in [0_u8, 0x55, 0xaa, 0xff] {
            for number in [i8::MIN, -1, 0, 1, i8::MAX] {
                let mut context = ObservedContext::new(revision);
                context.dispatcher = Dispatcher::hybrid_with_overrides(CALLEE);
                let player = ram::g_player_info(revision).at(0);
                player.entity_index().write(&mut context.inner, number);
                context.prepare(&[player.address(), 0x10]);
                let entry_sp = context.frame.entry_sp;
                context
                    .inner
                    .write_u16(entry_sp - 10, (u16::from(high) << 8) | 0x5a);
                let guard = std::sync::Arc::clone(&context.guard);
                let result = guard
                    .run_root(
                        crate::ids::OPEN_EARTHLING_PRESENT,
                        entry_sp,
                        context.inner.status_register() & 0x2000 != 0,
                        || {
                            super::open_present::open_earthling_present(
                                &mut context,
                                &[player.address(), 0x10],
                            )
                        },
                    )
                    .unwrap();
                assert_eq!(
                    result,
                    Err(crate::Error::Platform(
                        "authored opener prefix boundary".into()
                    ))
                );
                assert!(guard.failure().is_none());
                assert_eq!(context.declared_locals.len(), 1);
                let (read, missing, value) = context.declared_locals[0];
                assert_eq!(read.owner, crate::ids::OPEN_EARTHLING_PRESENT);
                assert_eq!(read.frame.entry_sp, entry_sp);
                assert_eq!(
                    (read.offset, read.width),
                    (-10, crate::stack_read::LocalWidth::Word)
                );
                assert!(read.citation.contains("1431-1434"));
                assert_eq!(missing, 1, "only the untouched high byte is an input");
                assert_eq!(value, (u32::from(high) << 8) | u32::from(number as u8));
                assert_eq!(
                    context
                        .reads
                        .borrow()
                        .iter()
                        .filter(|read| read.address == entry_sp - 10)
                        .copied()
                        .collect::<Vec<_>>(),
                    [Read {
                        address: entry_sp - 10,
                        width: 2,
                        discarded: false,
                    }]
                );
                assert_eq!(
                    context
                        .writes
                        .borrow()
                        .iter()
                        .filter(|write| (entry_sp - 10..entry_sp - 8).contains(&write.address))
                        .copied()
                        .collect::<Vec<_>>(),
                    [
                        Write {
                            address: entry_sp - 9,
                            width: 1,
                            value: u32::from(number as u8),
                        },
                        Write {
                            address: entry_sp - 10,
                            width: 2,
                            value: u32::from(number as i16 as u16),
                        },
                    ]
                );
                assert_eq!(
                    context.inner.memory.read_u16(entry_sp - 10),
                    number as i16 as u16
                );
                assert_eq!(guard.unwritten_mask(entry_sp - 10, 2), 0);
                assert_eq!(context.stack_pointer().unwrap(), entry_sp);
            }
        }
    }
}

#[test]
fn open_present_mixed_word_without_declaration_is_still_rejected() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ObservedContext::new(revision);
        let guard = std::sync::Arc::clone(&context.guard);
        let entry_sp = context.frame.entry_sp;
        let failure = guard
            .run_root(
                crate::ids::OPEN_EARTHLING_PRESENT,
                entry_sp,
                context.inner.status_register() & 0x2000 != 0,
                || {
                    let frame = crate::call_frame::LocalFrame::enter(
                        &mut context,
                        crate::call_frame::FrameLayout::link(8, 0x7c, 0x1c),
                    )
                    .unwrap();
                    frame.local::<i8>(-5).write(&mut context, 1);
                    let _ = frame.local::<i16>(-6).read(&context);
                },
            )
            .unwrap_err();
        assert_eq!(failure.root, crate::ids::OPEN_EARTHLING_PRESENT);
        assert_eq!(failure.address, entry_sp - 10);
        assert_eq!(failure.width, 2);
        assert_eq!(
            failure.reason,
            crate::stack_read::UnsupportedReason::Unwritten
        );
    }
}

const NEAREST_CAPTURE: u32 = 0xff7000;

fn capture_nearest_deltas(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    context.write_u32(NEAREST_CAPTURE, arguments[1]);
    context.write_u32(NEAREST_CAPTURE + 4, arguments[2]);
    Ok(Some(0x25))
}

fn capture_nearest_horizontal(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    capture_nearest_deltas(context, &[arguments[0], arguments[2], arguments[3]])
}

fn nearest_authored_abs(_: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    Ok(Some((arguments[0] as i32).unsigned_abs()))
}

fn nearest_authored_status(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    Ok(None)
}

fn nearest_guard_context(revision: Revision, maximum: u16, mode: u8) -> ObservedContext {
    static CALLEES: &[crate::Translation] = &[
        crate::Translation {
            id: crate::ids::ABS,
            function: nearest_authored_abs,
        },
        crate::Translation {
            id: crate::ids::ENTITY_SET_VELOCITY_TOWARD_DELTA,
            function: capture_nearest_deltas,
        },
        crate::Translation {
            id: crate::ids::ENTITY_STEER_WITH_VERTICAL_DEAD_ZONE,
            function: capture_nearest_deltas,
        },
        crate::Translation {
            id: crate::ids::ENTITY_STEER_WITH_HORIZONTAL_DRIFT,
            function: capture_nearest_horizontal,
        },
        crate::Translation {
            id: crate::ids::ENTITY_TRIGGER_TARGET_STATUS_EVENT_ONCE,
            function: nearest_authored_status,
        },
    ];
    let mut context = ObservedContext::new(revision);
    context.dispatcher = Dispatcher::hybrid_with_overrides(CALLEES);
    let state = ram::g_player_info(revision).at(6);
    state.level().write(&mut context, 3);
    for player in 0..2 {
        ram::g_player_info(revision)
            .at(player)
            .level()
            .write(&mut context, 1);
    }
    context.prepare(&[state.address(), u32::from(maximum), 0, u32::from(mode)]);
    context
}

#[test]
fn nearest_declares_unwritten_own_delta_words_without_fabricating_stores() {
    // Authored callees only capture inputs; this is not a cartridge proof.
    for revision in [Revision::Rev02, Revision::Rev00] {
        for mode in [1, 7, 12, 14, 18] {
            for (dx, dy) in [
                (0_i16, 0_i16),
                (i16::MIN, i16::MAX),
                (-19, 23),
                (0x2468, 0xcdef_u16 as i16),
            ] {
                for rejected_other_player in [false, true] {
                    let mut context = nearest_guard_context(revision, 255, mode);
                    let entry_sp = context.frame.entry_sp;
                    context.inner.write_u16(entry_sp - 10, dx as u16);
                    context.inner.write_u16(entry_sp - 6, dy as u16);
                    if rejected_other_player {
                        // Writing/rejecting player0's deltas must not mark
                        // player1's still-unwritten words as initialized.
                        ram::g_player_decoy_entities(revision)
                            .at(0)
                            .write(&mut context, 8);
                        let decoy = ram::g_player_info(revision).at(8);
                        decoy.level().write(&mut context, 3);
                        decoy.x().write(&mut context, 800);
                        decoy.y().write(&mut context, 200);
                    }
                    let guard = std::sync::Arc::clone(&context.guard);
                    let result = guard
                        .run_root(
                            crate::ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
                            entry_sp,
                            context.inner.status_register() & 0x2000 != 0,
                            || super::framed::earthling_steer_toward_nearest_target(&mut context),
                        )
                        .unwrap()
                        .unwrap();
                    assert_eq!(result, Some(0x25));
                    assert!(guard.failure().is_none());
                    assert_eq!(context.inner.memory.read_u32(NEAREST_CAPTURE), dx as u32);
                    assert_eq!(
                        context.inner.memory.read_u32(NEAREST_CAPTURE + 4),
                        dy as u32
                    );
                    assert_eq!(context.declared_locals.len(), 2);
                    for ((read, missing, value), offset, expected) in [
                        (context.declared_locals[0], -6, dy),
                        (context.declared_locals[1], -10, dx),
                    ] {
                        assert_eq!(
                            read.owner,
                            crate::ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET
                        );
                        assert_eq!(read.frame.entry_sp, entry_sp);
                        assert_eq!(read.offset, offset);
                        assert_eq!(read.width, crate::stack_read::LocalWidth::Word);
                        assert_eq!(missing, 3, "both original word bytes are unwritten");
                        assert_eq!(value, u32::from(expected as u16));
                        assert!(read.citation.contains("c_targeting.s"));
                    }
                    assert!(context.writes.borrow().iter().all(|write| {
                        write.address != entry_sp - 10 && write.address != entry_sp - 6
                    }));
                    assert_eq!(context.stack_pointer().unwrap(), entry_sp);
                }
            }
        }
    }
}

#[test]
fn nearest_keeps_ordinary_delta_reads_after_candidate_stores_even_on_rejection() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for mode in [1, 7, 12, 14, 18] {
            for (player, x, y, expected_dx, expected_dy) in [
                (0, 80, 32, 20_i16, 8_i16),
                (1, 80, 32, 20, 8),
                (1, 800, 200, 200, 50),
            ] {
                let mut context = nearest_guard_context(revision, 255, mode);
                ram::g_player_decoy_entities(revision)
                    .at(player)
                    .write(&mut context, 8);
                let decoy = ram::g_player_info(revision).at(8);
                decoy.level().write(&mut context, 3);
                decoy.x().write(&mut context, x);
                decoy.y().write(&mut context, y);
                let entry_sp = context.frame.entry_sp;
                let guard = std::sync::Arc::clone(&context.guard);
                assert_eq!(
                    guard
                        .run_root(
                            crate::ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
                            entry_sp,
                            context.inner.status_register() & 0x2000 != 0,
                            || {
                                super::framed::earthling_steer_toward_nearest_target(&mut context)
                            }
                        )
                        .unwrap()
                        .unwrap(),
                    Some(0x25)
                );
                assert!(guard.failure().is_none());
                assert!(context.declared_locals.is_empty());
                assert_eq!(
                    context.inner.memory.read_u32(NEAREST_CAPTURE),
                    expected_dx as u32
                );
                assert_eq!(
                    context.inner.memory.read_u32(NEAREST_CAPTURE + 4),
                    expected_dy as u32
                );
            }
        }
    }
}

#[test]
fn nearest_early_distance_return_never_reads_or_declares_delta_words() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for mode in [1, 7, 12, 14, 18] {
            let mut context = nearest_guard_context(revision, 0x7f, mode);
            let entry_sp = context.frame.entry_sp;
            let guard = std::sync::Arc::clone(&context.guard);
            assert_eq!(
                guard
                    .run_root(
                        crate::ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
                        entry_sp,
                        context.inner.status_register() & 0x2000 != 0,
                        || { super::framed::earthling_steer_toward_nearest_target(&mut context) }
                    )
                    .unwrap()
                    .unwrap(),
                Some(u32::MAX)
            );
            assert!(guard.failure().is_none());
            assert!(context.declared_locals.is_empty());
            assert!(context.reads.borrow().iter().all(|read| {
                ![entry_sp - 12, entry_sp - 10, entry_sp - 8, entry_sp - 6].contains(&read.address)
            }));
        }
    }
}

#[test]
fn nearest_unwritten_delta_without_declaration_is_still_rejected() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ObservedContext::new(revision);
        let entry_sp = context.frame.entry_sp;
        let guard = std::sync::Arc::clone(&context.guard);
        let failure = guard
            .run_root(
                crate::ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET,
                entry_sp,
                context.inner.status_register() & 0x2000 != 0,
                || {
                    let frame = crate::call_frame::LocalFrame::enter(
                        &mut context,
                        crate::call_frame::FrameLayout::link(12, 0x7c, 0x1c),
                    )
                    .unwrap();
                    let _ = frame.local::<i16>(-2).read(&context);
                },
            )
            .unwrap_err();
        assert_eq!(
            failure.root,
            crate::ids::EARTHLING_STEER_TOWARD_NEAREST_TARGET
        );
        assert_eq!(failure.address, entry_sp - 6);
        assert_eq!(failure.width, 2);
        assert_eq!(
            failure.reason,
            crate::stack_read::UnsupportedReason::Unwritten
        );
    }
}
