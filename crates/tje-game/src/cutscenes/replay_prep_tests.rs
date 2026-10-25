//! Root-level access regressions. Callee spies and scripted counter reads are
//! unit-test inputs, not ROM differential proofs or a shipping execution mode.
use super::*;
use crate::call_frame::{FrameError, FrameState};
use crate::{Dispatcher, FunctionId, execution};
use std::cell::RefCell;
use std::collections::VecDeque;
use tje_mem::{AddressSpace, Bus, Revision};

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Read(u32, u8),
    Discard(u32, u8),
    Write(u32, u8, u32),
    Call(FunctionId, Vec<u32>, i16),
    Wait(execution::WaitRequest),
    Yield,
}

const ENTRY_SP: u32 = 0xff7000;

#[derive(Debug, PartialEq, Eq)]
struct PreparedCall {
    function: FunctionId,
    entry_sp: u32,
    return_pc: u32,
    slots: Vec<u32>,
    following_slot: u32,
}

fn framed_wait_probe(context: &mut dyn Context) -> Result<Option<u32>> {
    framed_word_void_probe(context, ids::WAIT_FOR_NEXT_V_BLANK)
}

fn framed_border_probe(context: &mut dyn Context) -> Result<Option<u32>> {
    framed_word_void_probe(context, ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER)
}

fn framed_word_void_probe(context: &mut dyn Context, function: FunctionId) -> Result<Option<u32>> {
    let frame = context.frame_state().ok_or(FrameError::Unavailable)?;
    assert!(frame.is_prepared_for(function));
    let word = frame.argument_word(0).read(context);
    context.call_original_void_with_execution(
        function,
        &[word as i32 as u32],
        execution::ExecutionClass::TimeAware,
    )?;
    Ok(None)
}

fn framed_cadence_probe(context: &mut dyn Context) -> Result<Option<u32>> {
    let function = ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE;
    let frame = context.frame_state().ok_or(FrameError::Unavailable)?;
    assert!(frame.is_prepared_for(function));
    let advance = frame.argument_word(0).read(context) as u16;
    let animate = frame.argument_word(1).read(context);
    let value = context.call_original(function, &[u32::from(advance), animate as i32 as u32])?;
    Ok(Some(value))
}

const FRAMED_PROBES: &[crate::FramedTranslation] = &[
    crate::FramedTranslation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: framed_wait_probe,
    },
    crate::FramedTranslation {
        id: ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
        function: framed_border_probe,
    },
    crate::FramedTranslation {
        id: ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
        function: framed_cadence_probe,
    },
];

struct ScriptedContext {
    memory: AddressSpace,
    events: RefCell<Vec<Event>>,
    stack_events: RefCell<Vec<Event>>,
    prepared_calls: Vec<PreparedCall>,
    call_stack_pointers: Vec<(FunctionId, u32, i16)>,
    frame: FrameState,
    sp: u32,
    address: [u32; 8],
    counter_reads: Option<RefCell<VecDeque<u16>>>,
    controller1_reads: VecDeque<u32>,
    data: [u32; 8],
    status: u16,
    fail_on: Option<FunctionId>,
}

impl ScriptedContext {
    fn new(revision: Revision) -> Self {
        Self {
            memory: AddressSpace::new(Vec::new(), revision),
            events: RefCell::new(Vec::new()),
            stack_events: RefCell::new(Vec::new()),
            prepared_calls: Vec::new(),
            call_stack_pointers: Vec::new(),
            frame: FrameState::new(ENTRY_SP),
            sp: ENTRY_SP,
            address: [0; 8],
            counter_reads: None,
            controller1_reads: VecDeque::new(),
            data: [0; 8],
            status: 0x271f,
            fail_on: None,
        }
    }

    fn record(&self, event: Event) {
        let address = match event {
            Event::Read(address, _) | Event::Discard(address, _) | Event::Write(address, _, _) => {
                Some(address)
            }
            _ => None,
        };
        if address.is_some_and(|address| (ENTRY_SP - 0x800..ENTRY_SP + 0x100).contains(&address)) {
            self.stack_events.borrow_mut().push(event);
        } else {
            self.events.borrow_mut().push(event);
        }
    }

    fn record_prepared(&mut self, function: FunctionId) {
        let count = i32::from(function.metadata().abi.argument_count);
        let slots = (0..count)
            .map(|index| self.frame.argument_slot(index).read(&self.memory))
            .collect();
        self.prepared_calls.push(PreparedCall {
            function,
            entry_sp: self.frame.entry_sp,
            return_pc: self.memory.read_u32(self.frame.entry_sp),
            slots,
            following_slot: self.frame.argument_slot(count).read(&self.memory),
        });
    }

    fn original_prepared_effects(&mut self, function: FunctionId) -> Result<()> {
        assert_eq!(self.sp, self.frame.entry_sp);
        let arguments: Vec<_> = (0..i32::from(function.metadata().abi.argument_count))
            .map(|index| self.frame.argument_slot(index).read(&self.memory))
            .collect();
        if !self.frame.is_prepared_for(function) {
            self.record_prepared(function);
        }
        self.original_effects(function, &arguments)?;
        // The original-entry API consumes its own real RTS; the caller only
        // releases its outgoing argument area afterward.
        self.sp = self.sp.wrapping_add(4);
        Ok(())
    }

    fn calls(&self, function: FunctionId) -> Vec<(Vec<u32>, i16)> {
        self.events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Event::Call(id, arguments, state) if *id == function => {
                    Some((arguments.clone(), *state))
                }
                _ => None,
            })
            .collect()
    }

    fn counter_poll_sequence(&self) -> Vec<char> {
        let counter = ram::g_frame_counter(self.memory.revision()).address();
        self.events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Event::Read(address, 2) if *address == counter => Some('R'),
                Event::Wait(_) => Some('W'),
                Event::Yield => Some('Y'),
                _ => None,
            })
            .collect()
    }
    fn original_effects(&mut self, function: FunctionId, arguments: &[u32]) -> Result<()> {
        let state = ram::g_cutscene_script_state(self.revision()).read(&self.memory);
        if self.frame.is_prepared_for(function) {
            self.record_prepared(function);
        }
        if function == ids::GET_CTRL1_INPUT || function == ids::GET_CTRL2_INPUT {
            self.call_stack_pointers.push((function, self.sp, state));
        }
        self.events
            .borrow_mut()
            .push(Event::Call(function, arguments.to_vec(), state));
        if self.fail_on == Some(function) {
            return Err(crate::Error::Untranslated(function));
        }
        {
            let value = if function == ids::GET_CTRL1_INPUT {
                self.controller1_reads.pop_front().unwrap_or(0)
            } else {
                0
            };
            if function.metadata().abi.returns_value {
                self.set_data_register(0, value);
            }
            Ok(())
        }
    }
}

impl Bus for ScriptedContext {
    fn read_u8(&self, address: u32) -> u8 {
        self.record(Event::Read(address, 1));
        self.memory.read_u8(address)
    }

    fn read_u16(&self, address: u32) -> u16 {
        self.record(Event::Read(address, 2));
        if address == ram::g_frame_counter(self.memory.revision()).address()
            && let Some(values) = &self.counter_reads
        {
            return values.borrow_mut().pop_front().expect("extra counter read");
        }
        self.memory.read_u16(address)
    }

    fn read_u32(&self, address: u32) -> u32 {
        self.record(Event::Read(address, 4));
        self.memory.read_u32(address)
    }

    fn write_u8(&mut self, address: u32, value: u8) {
        self.record(Event::Write(address, 1, u32::from(value)));
        self.memory.write_u8(address, value);
    }

    fn write_u16(&mut self, address: u32, value: u16) {
        self.record(Event::Write(address, 2, u32::from(value)));
        self.memory.write_u16(address, value);
    }

    fn write_u32(&mut self, address: u32, value: u32) {
        self.record(Event::Write(address, 4, value));
        self.memory.write_u32(address, value);
    }
}

impl tje_mem::Platform for ScriptedContext {
    fn write_vdp_control(&mut self, value: u16) {
        crate::Platform::write_device_u16(self, 0xc00004, value);
    }

    fn write_vdp_data(&mut self, value: u16) {
        crate::Platform::write_device_u16(self, 0xc00000, value);
    }
}

impl crate::Platform for ScriptedContext {
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("unexpected root device read");
    }

    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("unexpected root device read");
    }

    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("unexpected root device byte write");
    }

    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.events
            .borrow_mut()
            .push(Event::Write(address, 2, u32::from(value)));
    }

    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.record(Event::Discard(address, 1));
        Ok(())
    }

    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.record(Event::Discard(address, 2));
        Ok(())
    }

    fn wait_until(&mut self, request: execution::WaitRequest) -> Result<()> {
        self.events.borrow_mut().push(Event::Wait(request));
        let execution::WaitCondition::Ram {
            address,
            width: execution::RamWidth::Word,
            mask,
            value,
            comparison: execution::WaitComparison::NotEqual,
        } = request.condition
        else {
            panic!("logo must use its exact word-not-equal RAM predicate");
        };
        assert_eq!(address, ram::g_frame_counter(self.revision()).address());
        assert_eq!(mask, u32::from(u16::MAX));
        while u32::from(self.read_u16(address)) & mask == value {}
        Ok(())
    }

    fn poll_yield(&mut self) -> Result<()> {
        self.events.borrow_mut().push(Event::Yield);
        Ok(())
    }
}

impl Context for ScriptedContext {
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
        if index == 7 {
            self.sp
        } else {
            self.address[index]
        }
    }

    fn set_address_register(&mut self, index: usize, value: u32) {
        if index == 7 {
            self.sp = value;
        } else {
            self.address[index] = value;
        }
    }

    fn status_register(&self) -> u16 {
        self.status
    }

    fn set_status_register(&mut self, value: u16) {
        self.status = value;
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

    fn set_frame_state(&mut self, frame: FrameState) -> Result<()> {
        self.frame = frame;
        Ok(())
    }

    fn call_original_entry(
        &mut self,
        function: FunctionId,
        _: execution::ExecutionClass,
    ) -> Result<u32> {
        self.original_prepared_effects(function)?;
        Ok(self.data[0])
    }

    fn call_original_entry_void(
        &mut self,
        function: FunctionId,
        _: execution::ExecutionClass,
    ) -> Result<()> {
        self.original_prepared_effects(function)
    }

    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with_frames(&[], FRAMED_PROBES)
    }

    fn execution_class(&self) -> execution::ExecutionClass {
        execution::ExecutionClass::TimeAware
    }

    fn enter_call_ceiling(&mut self, _: FunctionId) -> Result<execution::CallCeilingScope> {
        Ok(execution::CallCeilingScope::not_required())
    }

    fn call_original(&mut self, function: FunctionId, arguments: &[u32]) -> Result<u32> {
        self.original_effects(function, arguments)?;
        Ok(self.data_register(0))
    }
    fn call_original_void_with_execution(
        &mut self,
        function: FunctionId,
        arguments: &[u32],
        _class: crate::execution::ExecutionClass,
    ) -> Result<()> {
        self.original_effects(function, arguments)
    }

    fn call_original_with_execution(
        &mut self,
        function: FunctionId,
        arguments: &[u32],
        _: execution::ExecutionClass,
    ) -> Result<u32> {
        self.call_original(function, arguments)
    }
}

#[test]
fn logo_palette_deadline_repolls_without_inventing_frame_waits() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ScriptedContext::new(revision);
        ram::g_sega_rocket_pass_shown(revision).write(&mut context.memory, 1);
        let start = if revision == Revision::Rev02 { 0 } else { 20 };
        context.counter_reads = Some(RefCell::new(VecDeque::from([
            9,
            9,
            9,
            if revision == Revision::Rev02 {
                10
            } else {
                start
            },
            start,
            start,
            start,
            start,
            start,
            start + 3,
            start + 3,
            start + 120,
            start + 120,
            start + 120,
            start + 121,
        ])));
        sega_video(&mut context, &[]).unwrap();
        assert!(context.counter_reads.as_ref().unwrap().borrow().is_empty());
        let waits: Vec<_> = context
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Event::Wait(request) => Some(*request),
                _ => None,
            })
            .collect();
        let counter = ram::g_frame_counter(revision).address();
        assert_eq!(
            waits,
            [
                execution::WaitRequest::ram_word_not_equal(counter, u16::MAX, 9),
                execution::WaitRequest::ram_word_not_equal(counter, u16::MAX, start + 120),
            ]
        );
        assert_eq!(
            context.counter_poll_sequence(),
            [
                'R', 'W', 'R', 'R', 'R', // Entry snapshot; all three entry polls.
                'R', 'R', // Palette/start snapshots.
                'R', 'R', 'Y', // First compound iteration: no palette advance.
                'R', 'R', 'R',
                'Y', // Second iteration: counter is read again for palette_time.
                'R', // Final deadline test exits without yielding.
                'R', 'W', 'R', 'R', // Exit snapshot; failed and satisfying exit polls.
            ]
        );
        let events = context.events.borrow();
        let yields: Vec<_> = events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| matches!(event, Event::Yield).then_some(index))
            .collect();
        assert_eq!(yields.len(), 2);
        assert_eq!(
            &events[yields[0] - 2..=yields[0]],
            [
                Event::Read(counter, 2),
                Event::Read(counter, 2),
                Event::Yield,
            ]
        );
        let source = ram::g_sega_palette_upload_source(revision).address();
        assert_eq!(
            &events[yields[1] - 6..=yields[1]],
            [
                Event::Read(counter, 2),
                Event::Read(counter, 2),
                Event::Read(counter, 2),
                Event::Read(source, 4),
                Event::Write(source, 4, rom::sega_logo_pal(revision).address() + 32),
                Event::Write(ram::g_sega_palette_upload_pending(revision).address(), 2, 1),
                Event::Yield,
            ]
        );
        assert_eq!(
            ram::g_sega_palette_upload_source(revision)
                .read(&context.memory)
                .address(),
            rom::sega_logo_pal(revision).address() + 32
        );
        assert_eq!(context.status_register(), 0x2704);
        let discarded: Vec<_> = context
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Event::Discard(address, width) => Some((*address, *width)),
                _ => None,
            })
            .collect();
        let callback = ram::g_v_blank_func(revision).address();
        let mut expected = vec![
            (ram::g_sega_palette_upload_pending(revision).address(), 2),
            (
                ram::g_sega_v_blank_sprite_updates_enabled(revision).address(),
                2,
            ),
        ];
        if revision == Revision::Rev02 {
            expected.push((counter, 2));
        }
        expected.extend([(callback, 2), (callback + 2, 2)]);
        assert_eq!(discarded, expected);
    }
}

#[test]
fn logo_first_rocket_pass_keeps_all_steps_and_release_callee_extend() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ScriptedContext::new(revision);
        let start = if revision == Revision::Rev02 { 0 } else { 10 };
        let mut reads = VecDeque::from([
            9,
            9,
            10, // Entry snapshot and two primitive-owned polls.
            start,
            start,
            start + 120, // Palette/start snapshots; deadline already reached.
            start + 120,
            start + 120,
            start + 121, // Pre-rocket snapshot and wait.
            start + 121, // Palette time after scene setup.
        ]);
        let mut expected_polls = vec!['R', 'W', 'R', 'R', 'R', 'R', 'R', 'R', 'W', 'R', 'R', 'R'];
        let mut palette_time = start + 121;
        for frame in start + 121..=start + 450 {
            reads.extend([frame, frame, frame + 1, frame + 1]);
            expected_polls.extend(['R', 'W', 'R', 'R', 'R']);
            if frame + 1 >= palette_time + 3 {
                reads.push_back(frame + 1);
                expected_polls.push('R');
                palette_time = frame + 1;
            }
        }
        reads.extend([start + 451, start + 451, start + 452]);
        expected_polls.extend(['R', 'W', 'R', 'R']);
        context.counter_reads = Some(RefCell::new(reads));
        sega_video(&mut context, &[]).unwrap();
        assert!(context.counter_reads.as_ref().unwrap().borrow().is_empty());
        assert_eq!(context.counter_poll_sequence(), expected_polls);
        let positions = context.calls(ids::CUTSCENE_POSITION_ROCKETSHIP_PAIR);
        assert_eq!(positions.len(), 330);
        assert_eq!(positions.last(), Some(&(vec![(-50_i32) as u32, 150], 0)));
        assert_eq!(
            context.calls(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES).len(),
            330
        );
        assert_eq!(
            context.calls(ids::PLAY_SOUND_EFFECT),
            [(vec![rom::sfx_achoo(revision).address(), 50, 0], 0)]
        );
        assert_eq!(context.calls(ids::PLAY_SOUND), [(vec![21, 99], 0)]);
        let waits = context
            .events
            .borrow()
            .iter()
            .filter(|event| matches!(event, Event::Wait(_)))
            .count();
        assert_eq!(waits, 333);
        assert_eq!(context.status_register(), 0x2714);
    }
}

#[test]
fn final_intro_state_23_uses_held_frame_eighteen() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ScriptedContext::new(revision);
        assert_eq!(
            super::super::intro_part11::intro_part11(&mut context, &[]).unwrap(),
            Some(0)
        );
        assert_eq!(context.status_register(), 0x2704);
        let calls = context.calls(ids::INTRO_SET_LARGE_PLAYER_PART_SEQUENCE);
        let at_23: Vec<_> = calls.iter().filter(|(_, state)| *state == 23).collect();
        assert_eq!(
            at_23,
            [
                &(
                    vec![
                        6,
                        rom::intro_large_player_part_six_held_frame_fourteen(revision).address(),
                        6,
                    ],
                    23,
                ),
                &(
                    vec![
                        7,
                        rom::intro_large_player_part_seven_held_frame_eighteen(revision).address(),
                        6,
                    ],
                    23,
                ),
            ]
        );
        let discarded: Vec<_> = context
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Event::Discard(address, width) => Some((*address, *width)),
                _ => None,
            })
            .collect();
        assert_eq!(
            discarded,
            [
                (ram::g_cutscene_script_state(revision).address(), 2),
                (
                    ram::g_cutscene_music_actor_animation_enabled(revision).address(),
                    2
                ),
                (
                    ram::g_cutscene_music_actor_animation_enabled(revision).address(),
                    2
                ),
            ]
        );
    }
}

#[test]
fn final_intro_action_assignment_uses_store_flags_before_the_loop_reread() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ScriptedContext::new(revision);
        // Loop guard released, A held for one cadence, released next cadence,
        // then Start aborts at the following loop guard.
        context.controller1_reads = VecDeque::from([0, 0x10, 0, 0x80]);
        assert_eq!(
            super::super::intro_part11::intro_part11(&mut context, &[]).unwrap(),
            Some(1)
        );
        assert_eq!(context.status_register(), 0x2710);
        let mask = ram::g_cutscene_advance_button_mask(revision).address();
        let accesses: Vec<_> = context
            .events
            .borrow()
            .iter()
            .filter(|event| match event {
                Event::Read(address, _) | Event::Write(address, _, _) => *address == mask,
                _ => false,
            })
            .map(|event| match event {
                Event::Read(_, width) => (false, *width),
                Event::Write(_, width, _) => (true, *width),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(accesses.len(), 4);
        for pair in accesses.as_chunks::<2>().0 {
            assert_eq!(*pair, [(true, 2), (false, 2)]);
        }
        assert_eq!(accesses.len() % 2, 0);
        assert!(context.controller1_reads.is_empty());
        let controller_calls: Vec<_> = context
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Event::Call(id, _, _)
                    if *id == ids::GET_CTRL1_INPUT || *id == ids::GET_CTRL2_INPUT =>
                {
                    Some(*id)
                }
                _ => None,
            })
            .collect();
        for pair in controller_calls.as_chunks::<2>().0 {
            assert_eq!(*pair, [ids::GET_CTRL2_INPUT, ids::GET_CTRL1_INPUT]);
        }
        assert_eq!(controller_calls.len(), 8);
    }
}

#[test]
fn release_helpers_keep_selector_order_and_refresh_boundary() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ScriptedContext::new(revision);
        let selectors = [0, 6, 7, 0, 9, 10, 0, 12, 13, 14];
        for (index, value) in selectors.into_iter().enumerate() {
            ram::g_intro_component_entities(revision)
                .at(index as i32)
                .write(&mut context.memory, value);
        }
        super::super::intro_frame_release::intro_release_frame_entities_and_refresh(
            &mut context,
            &[],
        )
        .unwrap();
        let events = context.events.borrow();
        let mut expected = Vec::new();
        for (index, value) in selectors.into_iter().enumerate() {
            let address = ram::g_intro_component_entities(revision)
                .at(index as i32)
                .address();
            expected.push(Event::Read(address, 1));
            if value != 0 {
                expected.push(Event::Call(ids::FREE_ENTITY, vec![u32::from(value)], 0));
            }
            expected.push(Event::Discard(address, 1));
            expected.push(Event::Write(address, 1, 0));
        }
        expected.extend([
            Event::Discard(ram::g_sprite_graphics_upload_pending(revision).address(), 1),
            Event::Write(
                ram::g_sprite_graphics_upload_pending(revision).address(),
                1,
                0,
            ),
            Event::Call(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, vec![], 0),
            Event::Call(ids::WAIT_FOR_NEXT_V_BLANK, vec![1], 0),
            Event::Call(ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, vec![0, 0], 0),
        ]);
        assert_eq!(*events, expected);
        drop(events);
        context.events.borrow_mut().clear();
        super::super::large_player_parts::intro_release_large_player_sprites_and_refresh(
            &mut context,
            &[],
        )
        .unwrap();
        let mut expected: Vec<_> = [0, 4, 1, 2, 5, 6, 7]
            .into_iter()
            .map(|index| Event::Call(ids::INTRO_RELEASE_LARGE_PLAYER_PART_SPRITES, vec![index], 0))
            .collect();
        expected.extend([
            Event::Discard(ram::g_sprite_graphics_upload_pending(revision).address(), 1),
            Event::Write(
                ram::g_sprite_graphics_upload_pending(revision).address(),
                1,
                0,
            ),
            Event::Call(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, vec![], 0),
            Event::Call(ids::WAIT_FOR_NEXT_V_BLANK, vec![1], 0),
            Event::Call(ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, vec![0, 0], 0),
        ]);
        assert_eq!(*context.events.borrow(), expected);
    }
}

#[test]
fn logo_callback_orders_publications_sound_and_wrapping_counter() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for palette_pending in [0, 1] {
            for sprites_enabled in [0, 1] {
                let mut context = ScriptedContext::new(revision);
                let pending = ram::g_sega_palette_upload_pending(revision);
                let source = ram::g_sega_palette_upload_source(revision);
                let enabled = ram::g_sega_v_blank_sprite_updates_enabled(revision);
                let counter = ram::g_frame_counter(revision);
                pending.write(&mut context.memory, palette_pending);
                source.write(&mut context.memory, Ptr::new(0xff0100));
                enabled.write(&mut context.memory, sprites_enabled);
                counter.write(&mut context.memory, u16::MAX);
                super::super::sega_logo_vblank::sega_logo_vblank(&mut context, &[]).unwrap();
                let mut expected = vec![Event::Read(pending.address(), 2)];
                if palette_pending != 0 {
                    expected.extend([
                        Event::Discard(pending.address(), 2),
                        Event::Write(pending.address(), 2, 0),
                        Event::Read(source.address(), 4),
                        Event::Call(ids::COPY_PALETTE_TO_CRAM_SLOT, vec![0xff0100, 3], 0),
                    ]);
                }
                expected.push(Event::Read(enabled.address(), 2));
                if sprites_enabled != 0 {
                    expected.extend([
                        Event::Call(ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, vec![0, 0], 0),
                        Event::Call(ids::SELECT_VIEWPORT_SPRITE_CHAIN, vec![0], 0),
                    ]);
                }
                expected.extend([
                    Event::Call(ids::MUSIC_UPDATE_SEQUENCER, vec![], 0),
                    Event::Read(counter.address(), 2),
                    Event::Write(counter.address(), 2, 0),
                ]);
                assert_eq!(*context.events.borrow(), expected);
                assert_eq!(counter.read(&context.memory), 0);
                assert_eq!(context.status_register(), 0x2715);
            }
        }
    }
}

#[test]
fn frame_release_move_restore_uses_the_full_entry_d2_for_flags() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (saved, flags) in [(0, 4), (0x7fff_ffff, 0), (0x8000_0000, 8), (0x8000, 0)] {
            let mut context = ScriptedContext::new(revision);
            context.data[2] = saved;
            super::super::intro_frame_release::intro_release_frame_entities_and_refresh(
                &mut context,
                &[],
            )
            .unwrap();
            assert_eq!(context.data[2], saved);
            assert_eq!(context.status_register(), 0x2710 | flags);
        }
    }
}

#[test]
fn logo_counter_addq_flags_match_every_word_input() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ScriptedContext::new(revision);
        let counter = ram::g_frame_counter(revision);
        for old in 0..=u16::MAX {
            counter.write(&mut context.memory, old);
            context.status = 0x271f;
            context.events.borrow_mut().clear();
            super::super::sega_logo_vblank::sega_logo_vblank(&mut context, &[]).unwrap();
            let full = u32::from(old) + 1;
            let result = full as u16;
            let carry = full > u32::from(u16::MAX);
            let negative = (result as i16) < 0;
            let overflow = (old as i16) >= 0 && negative;
            let flags = (u16::from(carry) << 4)
                | (u16::from(negative) << 3)
                | (u16::from(result == 0) << 2)
                | (u16::from(overflow) << 1)
                | u16::from(carry);
            assert_eq!(counter.read(&context.memory), result);
            assert_eq!(
                context.status_register(),
                0x2700 | flags,
                "{revision:?} {old}"
            );
        }
    }
}

#[test]
fn final_intro_prepares_real_callee_frames_and_retains_cadence_arguments() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = ScriptedContext::new(revision);
        context.data[2] = 0x1234_abcd;
        context.data[3] = 0x5678_dcba;
        context.address[2] = 0x1100_2200;
        context.address[3] = 0x3300_4400;
        context.address[4] = 0x5500_6600;
        let saved_data = context.data;
        let saved_address = context.address;
        context.controller1_reads = VecDeque::from([0, 0x10, 0, 0x80]);
        assert_eq!(
            super::super::intro_part11::intro_part11(&mut context, &[]).unwrap(),
            Some(1)
        );
        assert_eq!(context.sp, ENTRY_SP);
        assert_eq!(context.frame, FrameState::new(ENTRY_SP));
        assert_eq!(&context.data[2..], &saved_data[2..]);
        assert_eq!(&context.address[2..7], &saved_address[2..7]);
        let base = ids::INTRO_PART11.metadata().address(revision);
        let calls: Vec<_> = context
            .prepared_calls
            .iter()
            .map(|call| {
                (
                    call.function,
                    call.entry_sp,
                    call.return_pc,
                    call.slots.clone(),
                )
            })
            .collect();
        assert_eq!(
            calls,
            [
                (
                    ids::INITIALIZE_TITLE_TRANSITION_TILEMAP,
                    ENTRY_SP - 24,
                    base + 0x4e,
                    vec![]
                ),
                (
                    ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
                    ENTRY_SP - 28,
                    base + 0x56,
                    vec![0]
                ),
                (
                    ids::WAIT_FOR_NEXT_V_BLANK,
                    ENTRY_SP - 32,
                    base + 0x66,
                    vec![1]
                ),
                (
                    ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
                    ENTRY_SP - 32,
                    base + 0xae,
                    vec![0, 1]
                ),
                (
                    ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
                    ENTRY_SP - 32,
                    base + 0xae,
                    vec![0, 1]
                ),
                (
                    ids::INTRO_RELEASE_LARGE_PLAYER_SPRITES_AND_REFRESH,
                    ENTRY_SP - 24,
                    base + 0x528,
                    vec![]
                ),
                (
                    ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
                    ENTRY_SP - 32,
                    base + 0x538,
                    vec![0, 0]
                ),
            ]
        );
        assert_eq!(context.prepared_calls[2].following_slot, 0);
        let first_controller_stack: Vec<_> = context
            .call_stack_pointers
            .iter()
            .filter_map(|&(id, sp, _)| (id == ids::GET_CTRL1_INPUT).then_some(sp))
            .collect();
        assert_eq!(
            first_controller_stack,
            [ENTRY_SP - 20, ENTRY_SP - 28, ENTRY_SP - 28, ENTRY_SP - 20]
        );
        let stack = context.stack_events.borrow();
        assert!(stack.contains(&Event::Read(ENTRY_SP - 26, 2)));
        assert!(stack.contains(&Event::Read(ENTRY_SP - 22, 2)));
    }
}

#[test]
fn refresh_callers_keep_the_wait_slot_below_upload_arguments() {
    let roots: [(FunctionId, crate::TranslationFn, u32, u32); 2] = [
        (
            ids::INTRO_RELEASE_FRAME_ENTITIES_AND_REFRESH,
            super::super::intro_frame_release::intro_release_frame_entities_and_refresh,
            0x4e,
            0x58,
        ),
        (
            ids::INTRO_RELEASE_LARGE_PLAYER_SPRITES_AND_REFRESH,
            super::super::large_player_parts::intro_release_large_player_sprites_and_refresh,
            0x4a,
            0x54,
        ),
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (root, function, wait_offset, upload_offset) in roots {
            let mut context = ScriptedContext::new(revision);
            let saved = 0x1234_abcd;
            context.data[2] = saved;
            context.address[2] = saved;
            assert_eq!(function(&mut context, &[]).unwrap(), None);
            assert_eq!(context.sp, ENTRY_SP);
            assert_eq!(context.data[2], saved);
            assert_eq!(context.address[2], saved);
            let base = root.metadata().address(revision);
            let wait = context
                .prepared_calls
                .iter()
                .find(|call| call.function == ids::WAIT_FOR_NEXT_V_BLANK)
                .unwrap();
            assert_eq!(wait.entry_sp, ENTRY_SP - 12);
            assert_eq!(wait.return_pc, base + wait_offset);
            assert_eq!(wait.slots, [1]);
            assert_eq!(wait.following_slot, saved);
            let upload = context.prepared_calls.last().unwrap();
            assert_eq!(upload.function, ids::UPLOAD_QUEUED_SPRITE_GRAPHICS);
            assert_eq!(upload.entry_sp, ENTRY_SP - 20);
            assert_eq!(upload.return_pc, base + upload_offset);
            assert_eq!(upload.slots, [0, 0]);
            assert_eq!(upload.following_slot, 1);
            let stack = context.stack_events.borrow();
            assert_eq!(
                &stack[..2],
                [
                    Event::Write(ENTRY_SP - 2, 2, saved & 0xffff),
                    Event::Write(ENTRY_SP - 4, 2, saved >> 16),
                ]
            );
            assert!(stack.contains(&Event::Read(ENTRY_SP - 6, 2)));
            if root == ids::INTRO_RELEASE_LARGE_PLAYER_SPRITES_AND_REFRESH {
                let selectors = [0, 4, 1, 2, 5, 6, 7];
                let offsets = [0x0c, 0x12, 0x18, 0x1e, 0x24, 0x2a, 0x30];
                for (index, (&selector, &offset)) in selectors.iter().zip(&offsets).enumerate() {
                    let call = &context.prepared_calls[index];
                    assert_eq!(call.function, ids::INTRO_RELEASE_LARGE_PLAYER_PART_SPRITES);
                    assert_eq!(call.entry_sp, ENTRY_SP - 12 - 4 * index as u32);
                    assert_eq!(call.return_pc, base + offset);
                    assert_eq!(call.slots, [selector]);
                    assert_eq!(
                        call.following_slot,
                        if index == 0 {
                            saved
                        } else {
                            selectors[index - 1]
                        }
                    );
                }
            }
        }
    }
}

#[test]
fn prepared_caller_diagnostic_errors_restore_the_owned_save_frame() {
    let roots: &[crate::TranslationFn] = &[
        super::super::intro_part11::intro_part11,
        super::super::intro_frame_release::intro_release_frame_entities_and_refresh,
        super::super::large_player_parts::intro_release_large_player_sprites_and_refresh,
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for function in roots {
            let mut context = ScriptedContext::new(revision);
            context.data[2] = 0x1234_abcd;
            context.data[3] = 0x5678_dcba;
            context.address[2] = 0x1100_2200;
            context.address[3] = 0x3300_4400;
            context.address[4] = 0x5500_6600;
            let saved_data = context.data;
            let saved_address = context.address;
            context.fail_on = Some(ids::WAIT_FOR_NEXT_V_BLANK);
            let error = function(&mut context, &[]).unwrap_err();
            assert!(
                matches!(error, crate::Error::Untranslated(id) if id == ids::WAIT_FOR_NEXT_V_BLANK)
            );
            assert_eq!(context.sp, ENTRY_SP);
            assert_eq!(context.frame, FrameState::new(ENTRY_SP));
            assert_eq!(&context.data[2..], &saved_data[2..]);
            assert_eq!(&context.address[2..7], &saved_address[2..7]);
        }
    }
}
