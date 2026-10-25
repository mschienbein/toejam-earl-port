use super::*;
use tje_mem::{AddressSpace, Ptr};

#[test]
fn set_state_installs_both_revision_descriptors_and_keeps_early_returns() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut bytes = vec![0; 0x100000];
        for (table, sequence, frames, period, offset) in [
            (
                rom::elevator_primary_animations(revision),
                0x1234_u32,
                0x5678_u32,
                3_u8,
                0_u8,
            ),
            (
                rom::elevator_secondary_animations(revision),
                0x4321,
                0x8765,
                7,
                0xf0,
            ),
        ] {
            let address = table.at(2).address() as usize;
            bytes[address..address + 4].copy_from_slice(&sequence.to_be_bytes());
            bytes[address + 4..address + 8].copy_from_slice(&frames.to_be_bytes());
            bytes[address + 8] = period;
            bytes[address + 9] = offset;
        }
        let mut memory = AddressSpace::new(bytes, revision);
        let elevator = ram::g_elevator_records(revision).at(0).at(0);
        let primary = ram::g_player_info(revision).at(2);
        let secondary = ram::g_player_info(revision).at(4);
        primary.x().write(&mut memory, -123);
        primary.y().write(&mut memory, i16::MIN + 8);
        elevator.entity().write(&mut memory, 2);
        elevator_set_state(&mut memory, revision, elevator, 2);
        assert_eq!(primary.sequence().read(&memory), Ptr::new(0x1234));
        assert_eq!(primary.frames().read(&memory), Ptr::new(0x5678));
        assert_eq!(primary.frame_phase().read(&memory), 3);
        assert_eq!(primary.frame_duration().read(&memory), 1);
        assert_eq!(primary.frame().read(&memory), -1);
        assert_eq!(primary.sprite_state().read(&memory), 2);
        assert_eq!(secondary.sequence().read(&memory), Ptr::new(0x4321));
        assert_eq!(secondary.frames().read(&memory), Ptr::new(0x8765));
        assert_eq!(secondary.frame_phase().read(&memory), 7);
        assert_eq!(secondary.x().read(&memory), -123);
        assert_eq!(secondary.y().read(&memory), i16::MAX - 7);
        elevator.level().write(&mut memory, 9);
        let saved_sequence = primary.sequence().read(&memory);
        elevator_set_state(&mut memory, revision, elevator, 11);
        assert_eq!(elevator.state().read(&memory), 11);
        assert_eq!(elevator.level().read(&memory), -1);
        assert_eq!(primary.sequence().read(&memory), saved_sequence);
        elevator.entity().write(&mut memory, 0);
        elevator_set_state(&mut memory, revision, elevator, -1);
        assert_eq!(elevator.state().read(&memory), -1);
        assert_eq!(primary.sequence().read(&memory), saved_sequence);
    }
}

#[test]
fn map_cell_conversion_sign_extends_and_loads_both_coordinates_before_stores() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut memory = AddressSpace::new(vec![], revision);
        let player = ram::g_player_info(revision).at(0);
        for (map_x, map_y) in [(-128_i8, 127_i8), (-1, -2), (0, 0), (127, -128)] {
            ram::g_arrival_elevator_map_x(revision).write(&mut memory, map_x);
            ram::g_arrival_elevator_map_y(revision).write(&mut memory, map_y);
            player_set_position_from_saved_map_cell(&mut memory, revision, player);
            assert_eq!(
                player.x().read(&memory),
                ((i32::from(map_x) + 2) * 128 - 16) as i16
            );
            assert_eq!(
                player.y().read(&memory),
                ((i32::from(map_y) + 2) * 64 - 5) as i16
            );
        }
        ram::g_arrival_elevator_map_x(revision).write(&mut memory, 1);
        ram::g_arrival_elevator_map_y(revision).write(&mut memory, 2);
        let aliased_player = ram::g_arrival_elevator_map_x(revision).cast::<ObjectState>();
        player_set_position_from_saved_map_cell(&mut memory, revision, aliased_player);
        assert_eq!(aliased_player.x().read(&memory), 368);
        assert_eq!(aliased_player.y().read(&memory), 251);
    }
}

#[test]
fn invalidation_changes_only_arrival_level_and_state() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut memory = AddressSpace::new(vec![], revision);
        let records = ram::g_elevator_records(revision);
        for bank in 0..2 {
            for kind in 0..2 {
                let record = records.at(bank).at(kind);
                record.level().write(&mut memory, 6);
                record.state().write(&mut memory, 3);
                record.entity().write(&mut memory, 2);
            }
        }
        elevator_invalidate_secondary_records(&mut memory, revision);
        for bank in 0..2 {
            assert_eq!(records.at(bank).at(0).state().read(&memory), 3);
            assert_eq!(records.at(bank).at(0).level().read(&memory), 6);
            assert_eq!(records.at(bank).at(1).state().read(&memory), 11);
            assert_eq!(records.at(bank).at(1).level().read(&memory), -1);
            assert_eq!(records.at(bank).at(1).entity().read(&memory), 2);
        }
    }
}

#[test]
fn query_decodes_every_byte_handle_including_wrapped_and_negative_values() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut memory = AddressSpace::new(vec![], revision);
        let records = ram::g_elevator_records(revision);
        for byte in 0..=255_u32 {
            let handle = byte as i8;
            let zero_based = handle.wrapping_sub(1);
            let offset = i32::from(zero_based >> 1) * 16 + i32::from(zero_based & 1) * 8;
            let state = records.cast::<u8>().byte_offset(offset + 5);
            state.write(&mut memory, 3);
            assert_eq!(process_elevator(&memory, revision, handle), 1);
            state.write(&mut memory, 2);
            assert_eq!(process_elevator(&memory, revision, handle), 0);
        }
    }
}

#[test]
fn byte_delay_predecrement_consumes_subtraction_overflow_before_reload() {
    for value in i8::MIN..=i8::MAX {
        let (stored, expired) = decrement_byte_and_test_le_zero(value);
        let overflow = value == i8::MIN;
        let negative = stored < 0;
        let zero = stored == 0;
        let bgt = !zero && negative == overflow;
        assert_eq!(stored, value.wrapping_sub(1));
        assert_eq!(expired, !bgt, "delay {value}");
    }
    assert_eq!(decrement_byte_and_test_le_zero(i8::MIN), (i8::MAX, true));
    assert_eq!(decrement_byte_and_test_le_zero(1), (0, true));
    assert_eq!(decrement_byte_and_test_le_zero(2), (1, false));
}

use crate::{
    Context, Dispatcher, Error, FunctionId, Platform, Result, Translation,
    call_frame::FrameState,
    execution::{CallCeilingScope, ExecutionClass, RamWidth, WaitCondition, WaitRequest},
};
use std::{cell::RefCell, collections::VecDeque};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Access {
    Read(u32, u8),
    Discard(u32, u8),
    Write(u32, u8, u32),
    Wait(WaitRequest),
    Yield,
}

struct SceneContext {
    memory: AddressSpace,
    trace: RefCell<Vec<Access>>,
    data: [u32; 8],
    addresses: [u32; 8],
    sp: u32,
    frame: FrameState,
    velocity_wakes: VecDeque<i16>,
}

const SPEECH_COUNT: u32 = 0xff1000;
const QUEUE_READY: u32 = 0xff1004;
const CAPTURE_D3: u32 = 0xff1008;
const CAPTURE_A3: u32 = 0xff100c;
const QUEUE_CALLS: u32 = 0xff1010;

impl SceneContext {
    fn new(revision: Revision) -> Self {
        let mut bytes = vec![0; 0x100000];
        let table = rom::speech_string_pointers(revision).at(0).address() as usize;
        bytes[table..table + 4].copy_from_slice(&0x300_u32.to_be_bytes());
        bytes[table + 8..table + 12].copy_from_slice(&0x400_u32.to_be_bytes());
        Self {
            memory: AddressSpace::new(bytes, revision),
            trace: RefCell::default(),
            data: [0x12345678; 8],
            addresses: [0xff2000; 8],
            sp: 0xff7000,
            frame: FrameState::new(0xff7000),
            velocity_wakes: VecDeque::new(),
        }
    }

    fn accesses(&self, address: u32) -> Vec<Access> {
        self.trace
            .borrow()
            .iter()
            .filter(|event| match event {
                Access::Read(at, _) | Access::Discard(at, _) | Access::Write(at, _, _) => {
                    *at == address
                }
                Access::Wait(_) | Access::Yield => false,
            })
            .cloned()
            .collect()
    }

    fn advance_poll(&mut self) {
        let revision = self.memory.revision();
        if let Some(velocity) = self.velocity_wakes.pop_front() {
            ram::g_elevator_ride_scroll_velocity(revision).write(&mut self.memory, velocity);
        } else {
            ram::g_sprite_graphics_upload_pending(revision).write(&mut self.memory, 0);
            let requested = ram::g_requested_music_selection(revision).read(&self.memory);
            ram::g_current_music_selection(revision).write(&mut self.memory, requested);
        }
    }
}

impl Bus for SceneContext {
    fn read_u8(&self, address: u32) -> u8 {
        self.trace.borrow_mut().push(Access::Read(address, 1));
        self.memory.read_u8(address)
    }
    fn read_u16(&self, address: u32) -> u16 {
        self.trace.borrow_mut().push(Access::Read(address, 2));
        self.memory.read_u16(address)
    }
    fn read_u32(&self, address: u32) -> u32 {
        self.trace.borrow_mut().push(Access::Read(address, 4));
        self.memory.read_u32(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.trace
            .borrow_mut()
            .push(Access::Write(address, 1, u32::from(value)));
        self.memory.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.trace
            .borrow_mut()
            .push(Access::Write(address, 2, u32::from(value)));
        self.memory.write_u16(address, value);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.trace
            .borrow_mut()
            .push(Access::Write(address, 4, value));
        self.memory.write_u32(address, value);
    }
}

impl tje_mem::Platform for SceneContext {
    fn write_vdp_control(&mut self, value: u16) {
        self.write_device_u16(0xc00004, value);
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.write_device_u16(0xc00000, value);
    }
    fn write_vdp_command(&mut self, value: u32) {
        self.write_device_u16(0xc00004, (value >> 16) as u16);
        self.write_device_u16(0xc00006, value as u16);
    }
}

impl Platform for SceneContext {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        panic!("unexpected device byte read {address:x}");
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        panic!("unexpected device word read {address:x}");
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.write_u8(address, value);
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.write_u16(address, value);
    }
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        let _ = self.memory.read_u8(address);
        self.trace.borrow_mut().push(Access::Discard(address, 1));
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        let _ = self.memory.read_u16(address);
        self.trace.borrow_mut().push(Access::Discard(address, 2));
        Ok(())
    }
    fn wait_until(&mut self, request: WaitRequest) -> Result<()> {
        self.trace.borrow_mut().push(Access::Wait(request));
        let WaitCondition::Ram {
            address,
            width: RamWidth::Byte,
            mask,
            value,
            comparison,
        } = request.condition
        else {
            panic!("unexpected proxy or non-byte wait: {request:?}");
        };
        loop {
            let sample = u32::from(self.read_u8(address));
            if comparison.satisfied(sample, mask, value) {
                break;
            }
            self.advance_poll();
        }
        Ok(())
    }
    fn poll_yield(&mut self) -> Result<()> {
        self.trace.borrow_mut().push(Access::Yield);
        self.advance_poll();
        Ok(())
    }
}

impl Context for SceneContext {
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
        self.addresses[index]
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.addresses[index] = value;
    }
    fn status_register(&self) -> u16 {
        0x2000
    }
    fn set_status_register(&mut self, _: u16) {}
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
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::TimeAware
    }
    fn set_execution_class(&mut self, _: ExecutionClass) -> Result<()> {
        Ok(())
    }
    fn enter_call_ceiling(&mut self, _: FunctionId) -> Result<CallCeilingScope> {
        Ok(CallCeilingScope::not_required())
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with_overrides(SCENE_CALLEES)
    }
    fn call_original(&mut self, id: FunctionId, _: &[u32]) -> Result<u32> {
        panic!("unexpected original call {}", id.metadata().name);
    }
}

fn no_effect(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    Ok(Some(0))
}

fn stop_at_callee(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    Err(Error::Platform("unit-test call boundary".into()))
}

fn speech_line(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let count = context.read_u32(SPEECH_COUNT);
    context.write_u32(SPEECH_COUNT, count + 1);
    Ok(Some(count + 2))
}

fn queue_ready(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let mode = context.read_u8(QUEUE_READY);
    if mode != 0 {
        let revision = context.revision();
        let calls = context.read_u32(QUEUE_CALLS);
        context.write_u32(QUEUE_CALLS, calls + 1);
        for slot in [2, 4] {
            let ready = !(calls == 0 && ((mode == 2 && slot == 2) || (mode == 3 && slot == 4)));
            ram::g_player_info(revision)
                .at(slot)
                .sprite_ready()
                .write(context, i8::from(ready));
        }
        ram::g_sprite_graphics_upload_pending(revision).write(context, u8::from(mode == 1));
    }
    Ok(None)
}

fn record_advance(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    context.write_u32(CAPTURE_D3, context.data_register(3));
    context.write_u32(CAPTURE_A3, context.address_register(3));
    stop_at_callee(context, &[])
}

const SCENE_CALLEES: &[Translation] = &[
    Translation {
        id: ids::GET_RANDOM_NUMBER,
        function: no_effect,
    },
    Translation {
        id: ids::GENERATE_SPEECH,
        function: speech_line,
    },
    Translation {
        id: ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
        function: queue_ready,
    },
    Translation {
        id: ids::BUILD_VIEWPORT_SPRITE_LINKS,
        function: no_effect,
    },
    Translation {
        id: ids::ELEVATOR_SET_STATE,
        function: no_effect,
    },
    Translation {
        id: ids::CLEAR_TRANSIENT_ENTITIES,
        function: no_effect,
    },
    Translation {
        id: ids::CLEAR_SPEECH_BUBBLES_AND_REPLIES,
        function: stop_at_callee,
    },
    Translation {
        id: ids::DISABLE_BLANKING_GRAPHICS,
        function: no_effect,
    },
    Translation {
        id: ids::WAIT_FOR_V_BLANK_WITH_GRAPHICS_PAUSED,
        function: stop_at_callee,
    },
    Translation {
        id: ids::ELEVATOR_PREPARE_RIDE_DIALOGUE,
        function: no_effect,
    },
    Translation {
        id: ids::ELEVATOR_BEGIN_RIDE_SCENE,
        function: stop_at_callee,
    },
    Translation {
        id: ids::ELEVATOR_ADVANCE_RIDERS_TO_NEXT_LEVEL,
        function: record_advance,
    },
    Translation {
        id: ids::PLAY_SOUND,
        function: no_effect,
    },
    Translation {
        id: ids::MAP_LOAD_PLAYER_LEVEL,
        function: stop_at_callee,
    },
];

#[test]
fn final_level_frame_is_present_at_the_first_original_call() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = SceneContext::new(revision);
        ram::g_player_lives(revision)
            .at(0)
            .write(&mut context.memory, 3);
        assert!(map_enter_final_level(&mut context, &[]).is_err());
        let return_pc = ids::MAP_ENTER_FINAL_LEVEL.metadata().address(revision) + 0x2a;
        assert_eq!(context.memory.read_u32(0xff6fec), return_pc);
        assert_eq!(context.memory.read_u32(0xff6ff0), 0);
        assert_eq!(context.memory.read_u32(0xff6ff4), 26);
        assert_eq!(context.memory.read_u32(0xff6ffc), 0xff2000);
        assert_eq!(context.memory.read_u32(0xff6ff8), 0xff2000);
        assert_eq!(context.sp, 0xff7000);
    }
}

#[test]
fn dialogue_pushes_interleave_reads_and_reply_local_reloads_byte_then_word() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = SceneContext::new(revision);
        let entity = ram::g_player_info(revision).at(2);
        entity.entity_index().write(&mut context.memory, 2);
        context.memory.write_u32(0xff7004, entity.address());
        elevator_prepare_ride_dialogue(&mut context).unwrap();
        let trace = context.trace.borrow();
        let descriptor = rom::speech_string_pointers(revision).at(0);
        let first_pointer = trace
            .iter()
            .position(|event| *event == Access::Read(descriptor.first_line().address(), 4))
            .unwrap();
        assert_eq!(trace[first_pointer - 1], Access::Write(0xff6fd4, 4, 0));
        assert_eq!(trace[first_pointer + 1], Access::Write(0xff6fd0, 4, 0x300));
        assert_eq!(
            trace[first_pointer + 2],
            Access::Read(entity.entity_index().address(), 1)
        );
        let reply_byte = trace
            .iter()
            .position(|event| *event == Access::Read(0xff6ffb, 1))
            .unwrap();
        assert_eq!(
            trace[reply_byte + 1],
            Access::Write(
                ram::g_elevator_ride_speech_entities(revision)
                    .at(2)
                    .address(),
                1,
                3
            )
        );
        assert_eq!(trace[reply_byte + 2], Access::Read(0xff6ffa, 2));
        assert_eq!(context.sp, 0xff7000);
    }
}

#[test]
fn end_ride_preserves_signed_thresholds_and_yields_only_on_spin_backedges() {
    let cases: &[(i16, &[i16])] = &[
        (128, &[2048, 0]),
        (128, &[2053, -1]),
        (i16::MIN, &[i16::MAX, i16::MIN]),
        (2048, &[-1]),
        (2053, &[0]),
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for &(initial, wakes) in cases {
            let mut context = SceneContext::new(revision);
            let velocity = ram::g_elevator_ride_scroll_velocity(revision);
            let acceleration = ram::g_elevator_ride_scroll_acceleration(revision);
            velocity.write(&mut context.memory, initial);
            context.velocity_wakes = wakes.iter().copied().collect();
            assert!(elevator_end_ride_scene(&mut context, &[]).is_err());
            let sequence: Vec<_> = context
                .trace
                .borrow()
                .iter()
                .filter(|event| match event {
                    Access::Read(address, _) => *address == velocity.address(),
                    Access::Write(address, _, _) => *address == acceleration.address(),
                    Access::Wait(_) | Access::Yield => true,
                    _ => false,
                })
                .cloned()
                .collect();
            let mut expected = vec![Access::Read(velocity.address(), 2)];
            if initial < 2048 {
                expected.extend([Access::Yield, Access::Read(velocity.address(), 2)]);
            }
            expected.extend([
                Access::Write(acceleration.address(), 2, u32::from((-2048_i16) as u16)),
                Access::Read(velocity.address(), 2),
                Access::Yield,
                Access::Read(velocity.address(), 2),
            ]);
            assert_eq!(sequence, expected, "initial={initial}, wakes={wakes:?}");
            assert_eq!(acceleration.read(&context.memory), -2048);
        }
    }
}

#[test]
fn begin_ride_waits_without_a_containing_word_read() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = SceneContext::new(revision);
        let elevator = ram::g_elevator_records(revision).at(0).at(0);
        elevator.entity().write(&mut context.memory, 2);
        context.memory.write_u8(QUEUE_READY, 1);
        context.memory.write_u32(0xff7004, elevator.address());
        assert!(elevator_begin_ride_scene(&mut context).is_err());
        let pending = ram::g_sprite_graphics_upload_pending(revision).address();
        assert_eq!(
            context.accesses(pending),
            vec![
                Access::Write(pending, 1, 1),
                Access::Read(pending, 1),
                Access::Read(pending, 1)
            ]
        );
        assert!(
            !context
                .trace
                .borrow()
                .contains(&Access::Read(pending & !1, 2))
        );
        let sequence: Vec<_> = context
            .trace
            .borrow()
            .iter()
            .filter(|event| match event {
                Access::Read(address, _) | Access::Write(address, _, _) => *address == pending,
                Access::Wait(_) | Access::Yield => true,
                _ => false,
            })
            .cloned()
            .collect();
        assert_eq!(
            sequence,
            vec![
                Access::Write(pending, 1, 1),
                Access::Wait(WaitRequest::ram_byte_equal(pending, u8::MAX, 0)),
                Access::Read(pending, 1),
                Access::Read(pending, 1),
            ]
        );
        assert!(
            context
                .accesses(ram::g_frame_counter(revision).address())
                .is_empty()
        );
    }
}

#[test]
fn begin_readiness_loop_keeps_short_circuit_order_and_yields_before_requeue() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for mode in [2, 3] {
            let mut context = SceneContext::new(revision);
            let elevator = ram::g_elevator_records(revision).at(0).at(0);
            elevator.entity().write(&mut context.memory, 2);
            context.memory.write_u8(QUEUE_READY, mode);
            context.memory.write_u32(0xff7004, elevator.address());
            assert!(elevator_begin_ride_scene(&mut context).is_err());
            let first = ram::g_player_info(revision).at(2).sprite_ready().address();
            let second = ram::g_player_info(revision).at(4).sprite_ready().address();
            let sequence: Vec<_> = context.trace.borrow().iter().filter(|event| {
                matches!(event, Access::Read(address, _) if *address == first || *address == second)
                    || matches!(event, Access::Yield)
            }).cloned().collect();
            let mut expected = vec![Access::Read(first, 1)];
            if mode == 3 {
                expected.push(Access::Read(second, 1));
            }
            expected.extend([
                Access::Yield,
                Access::Read(first, 1),
                Access::Read(second, 1),
            ]);
            assert_eq!(sequence, expected);
            let pending = ram::g_sprite_graphics_upload_pending(revision).address();
            assert_eq!(
                context.accesses(pending),
                vec![
                    Access::Write(pending, 1, 0),
                    Access::Read(pending, 1),
                    Access::Write(pending, 1, 0),
                    Access::Read(pending, 1),
                ]
            );
            assert_eq!(
                context
                    .trace
                    .borrow()
                    .iter()
                    .filter(|event| {
                        **event == Access::Wait(WaitRequest::ram_byte_equal(pending, u8::MAX, 0))
                    })
                    .count(),
                2
            );
        }
    }
}

#[test]
fn music_wait_reads_requested_before_current_and_never_their_word() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = SceneContext::new(revision);
        for player in 0..2 {
            ram::g_player_lives(revision)
                .at(player)
                .write(&mut context.memory, -1);
        }
        context.memory.write_u32(0xff7004, 1);
        assert!(elevator_advance_riders_to_next_level(&mut context).is_err());
        let requested = ram::g_requested_music_selection(revision).address();
        let current = ram::g_current_music_selection(revision).address();
        let polls: Vec<_> = context.trace.borrow().iter().filter(|event| {
            matches!(event, Access::Read(address, _) if *address == requested || *address == current)
                || matches!(event, Access::Yield | Access::Wait(_))
        }).cloned().collect();
        assert_eq!(
            polls,
            vec![
                Access::Read(requested, 1),
                Access::Read(current, 1),
                Access::Yield,
                Access::Read(requested, 1),
                Access::Read(current, 1)
            ]
        );
        assert_eq!(context.accesses(0xff7007), vec![Access::Read(0xff7007, 1)]);
        assert!(context.accesses(0xff7004).is_empty());
    }
}

#[test]
fn departure_rereads_entity_and_forwards_last_attached_player_d3_residue() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for attached in [None, Some(0), Some(1)] {
            let mut context = SceneContext::new(revision);
            let elevator = ram::g_elevator_records(revision).at(0).at(0);
            elevator.state().write(&mut context.memory, 7);
            elevator.entity().write(&mut context.memory, 2);
            ram::g_elevator_records(revision)
                .at(1)
                .at(0)
                .state()
                .write(&mut context.memory, 11);
            if let Some(player) = attached {
                ram::g_player_elevator_flags(revision)
                    .at(player)
                    .write(&mut context.memory, 1);
                ram::g_player_info(revision)
                    .at(1)
                    .level()
                    .write(&mut context.memory, 2);
            }
            assert!(process_elevator_step1(&mut context, &[]).is_err());
            assert_eq!(
                context.accesses(elevator.entity().address()),
                vec![
                    Access::Read(elevator.entity().address(), 1),
                    Access::Read(elevator.entity().address(), 1)
                ]
            );
            let expected = attached.map_or(0x12345607, |player| {
                ram::g_player_info(revision).at(player).address()
            });
            assert_eq!(context.memory.read_u32(CAPTURE_D3), expected);
            assert_eq!(
                context.memory.read_u32(CAPTURE_A3),
                ram::g_player_info(revision).address()
            );
        }
    }
}

#[test]
fn blocked_departure_tests_arrival_levels_before_each_departure_level_reload() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = SceneContext::new(revision);
        let elevator = ram::g_elevator_records(revision).at(0).at(0);
        elevator.state().write(&mut context.memory, 13);
        elevator.level().write(&mut context.memory, 3);
        ram::g_elevator_records(revision)
            .at(1)
            .at(0)
            .state()
            .write(&mut context.memory, 11);
        process_elevator_step1(&mut context, &[]).unwrap();
        let primary = ram::g_primary_arrival_elevator_level(revision).address();
        let secondary = ram::g_secondary_arrival_elevator_level(revision).address();
        let accesses: Vec<_> = context.trace.borrow().iter().filter(|event| {
            matches!(event, Access::Read(address, _) if [primary, secondary, elevator.level().address()].contains(address))
        }).cloned().collect();
        assert_eq!(
            &accesses[..4],
            &[
                Access::Read(primary, 1),
                Access::Read(elevator.level().address(), 1),
                Access::Read(secondary, 1),
                Access::Read(elevator.level().address(), 1),
            ]
        );
    }
}

#[test]
fn skipped_dialogue_clears_each_nonframe_byte_with_its_discarded_preread() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = SceneContext::new(revision);
        ram::g_player_lives(revision)
            .at(0)
            .write(&mut context.memory, -1);
        context
            .memory
            .write_u32(0xff7004, ram::g_player_info(revision).at(0).address());
        elevator_prepare_ride_dialogue(&mut context).unwrap();
        for index in 0..4 {
            let address = ram::g_elevator_ride_speech_entities(revision)
                .at(index)
                .address();
            assert_eq!(
                context.accesses(address),
                vec![Access::Discard(address, 1), Access::Write(address, 1, 0),]
            );
        }
        assert_eq!(context.accesses(0xff7004), vec![Access::Read(0xff7004, 4)]);
    }
}
