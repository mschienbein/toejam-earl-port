use super::*;
use crate::{Dispatcher, FunctionId, call_frame::FrameState, execution::ExecutionClass};
use std::cell::RefCell;
use tje_mem::{AddressSpace, Bus, Revision};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    Read(u32, u8),
    Discard(u32, u8),
    Write(u32, u8, u32),
}
#[derive(Debug)]
pub(crate) struct Call {
    pub(crate) id: FunctionId,
    pub(crate) sp: u32,
    pub(crate) return_pc: u32,
    pub(crate) argument: u32,
    d2: u32,
    pub(crate) d4: u32,
    a2: u32,
    a3: u32,
}
pub(crate) struct Machine {
    pub(crate) memory: AddressSpace,
    accesses: RefCell<Vec<Access>>,
    pub(crate) calls: Vec<Call>,
    pub(crate) data: [u32; 8],
    pub(crate) address: [u32; 7],
    pub(crate) devices: Vec<(u32, u16)>,
    status: u16,
    pub(crate) sp: u32,
    frame: FrameState,
    class: ExecutionClass,
}
impl Machine {
    pub(crate) const SP: u32 = 0xff7f00;
    pub(crate) fn new(revision: Revision) -> Self {
        Self {
            memory: AddressSpace::new(Vec::new(), revision),
            accesses: RefCell::default(),
            calls: Vec::new(),
            devices: Vec::new(),
            data: [0x12345678; 8],
            address: [0xff6100; 7],
            status: 0x2000,
            sp: Self::SP,
            frame: FrameState::new(Self::SP),
            class: ExecutionClass::TimeAware,
        }
    }
    pub(crate) fn main_accesses(&self) -> Vec<Access> {
        self.accesses
            .borrow()
            .iter()
            .filter(|access| match access {
                Access::Read(address, _)
                | Access::Discard(address, _)
                | Access::Write(address, _, _) => *address >= 0xff8000,
            })
            .cloned()
            .collect()
    }
}
impl Bus for Machine {
    fn read_u8(&self, address: u32) -> u8 {
        self.accesses.borrow_mut().push(Access::Read(address, 1));
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.accesses
            .get_mut()
            .push(Access::Write(address, 1, u32::from(value)));
        self.memory.write_u8(address, value);
    }
    fn read_u16(&self, address: u32) -> u16 {
        self.accesses.borrow_mut().push(Access::Read(address, 2));
        self.memory.read_u16(address)
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.accesses
            .get_mut()
            .push(Access::Write(address, 2, u32::from(value)));
        self.memory.write_u16(address, value);
    }
    fn read_u32(&self, address: u32) -> u32 {
        self.accesses.borrow_mut().push(Access::Read(address, 4));
        self.memory.read_u32(address)
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.accesses
            .get_mut()
            .push(Access::Write(address, 4, value));
        self.memory.write_u32(address, value);
    }
}
impl tje_mem::Platform for Machine {
    fn write_vdp_control(&mut self, value: u16) {
        self.devices.push((0xc00004, value));
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.devices.push((0xc00000, value));
    }
    fn write_vdp_command(&mut self, value: u32) {
        self.devices.push((0xc00004, (value >> 16) as u16));
        self.devices.push((0xc00006, value as u16));
    }
}
impl crate::Platform for Machine {
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.accesses.get_mut().push(Access::Discard(address, 1));
        let _ = self.memory.read_u8(address);
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.accesses.get_mut().push(Access::Discard(address, 2));
        let _ = self.memory.read_u16(address);
        Ok(())
    }
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("unexpected device read");
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("unexpected device read");
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("unexpected device write");
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("unexpected device write");
    }
}
impl Context for Machine {
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
    fn execution_class(&self) -> ExecutionClass {
        self.class
    }
    fn set_execution_class(&mut self, class: ExecutionClass) -> Result<()> {
        self.class = class;
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with(&[])
    }
    fn call_original_entry(&mut self, id: FunctionId, _: ExecutionClass) -> Result<u32> {
        // Unit-only callee boundary: inspect the actual prepared RAM, not a
        // host argument slice. ROM/check proofs use the real callees instead.
        assert!(matches!(
            id,
            ids::WAIT_FOR_NEXT_V_BLANK
                | ids::DISABLE_BLANKING_GRAPHICS
                | ids::ENABLE_BLANKING_GRAPHICS
                | ids::UPDATE_CAMERA_X
                | ids::UPDATE_CAMERA_Y
                | ids::TRY_UPDATE_CAMERA_X
                | ids::TRY_UPDATE_CAMERA_Y
                | ids::GET_PLAYER_CAMERA_VELOCITY
                | ids::ABS
                | ids::LDIVT
                | ids::POSITION_ENTITY_SPRITES
                | ids::UPLOAD_QUEUED_SPRITE_GRAPHICS
                | ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES
                | ids::ENTITY_RELEASE_ONE_BY_PRIORITY
                | ids::UPDATE_AND_QUEUE_ENTITY_SPRITES
                | ids::LINK_PLAYER_SPRITES_FOR_SPLIT_WIPE
                | ids::SELECT_SPRITE_CHAIN_START
        ));
        let return_pc = self.memory.read_u32(self.sp);
        self.calls.push(Call {
            id,
            sp: self.sp,
            return_pc,
            argument: self.memory.read_u32(self.sp + 4),
            d2: self.data[2],
            d4: self.data[4],
            a2: self.address[2],
            a3: self.address[3],
        });
        let value = match id {
            ids::GET_PLAYER_CAMERA_VELOCITY => {
                // Isolate the parent; this is not a ROM/callee proof.
                let dx = self.memory.read_u32(self.sp + 8);
                let dy = self.memory.read_u32(self.sp + 12);
                self.write_u16(dx, 0);
                self.write_u16(dy, 0);
                0
            }
            ids::ABS => (self.memory.read_u32(self.sp + 4) as i32).wrapping_abs() as u32,
            ids::LDIVT => {
                tje_mem::arithmetic::div_i32(self.data[0] as i32, self.data[1] as i32)? as u32
            }
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES => {
                let entity = tje_mem::Address::<tje_mem::ObjectState>::new(
                    self.memory.read_u32(self.sp + 4),
                );
                let flags = entity.draw_flags().read(&self.memory) | 2;
                entity.draw_flags().write(&mut self.memory, flags);
                entity.sprite_ready().write(&mut self.memory, 1);
                0
            }
            ids::UPDATE_AND_QUEUE_ENTITY_SPRITES => {
                for player in 0..2 {
                    ram::g_player_info(self.revision())
                        .at(player)
                        .sprite_ready()
                        .write(&mut self.memory, 1);
                }
                0
            }
            _ => 0,
        };
        if id.metadata().abi.returns_value {
            self.data[0] = value;
        }
        self.sp += 4;
        Ok(value)
    }
    fn call_original(&mut self, id: FunctionId, _: &[u32]) -> Result<u32> {
        panic!("value-only call instead of the compiled prepared frame: {id:?}")
    }
}

#[test]
fn flush_keeps_wait_argument_through_disable_and_upload_in_both_revisions() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for vertical in [true, false] {
            for camera in [0_i32, 1, 256, -256] {
                let mut machine = Machine::new(revision);
                let saved_data = machine.data;
                let saved_addresses = machine.address;
                let pending = if vertical {
                    ram::g_camera_row_upload_pending(revision)
                } else {
                    ram::g_camera_column_upload_pending(revision)
                };
                let other = pending.at(camera ^ 1);
                other.write(&mut machine.memory, 0x7e);
                let caller = if vertical {
                    ids::TRY_UPDATE_CAMERA_Y
                } else {
                    ids::TRY_UPDATE_CAMERA_X
                };
                flush(&mut machine, camera, vertical).unwrap();
                assert_eq!(
                    machine.calls.iter().map(|call| call.id).collect::<Vec<_>>(),
                    [
                        ids::WAIT_FOR_NEXT_V_BLANK,
                        ids::DISABLE_BLANKING_GRAPHICS,
                        if vertical {
                            ids::UPDATE_CAMERA_Y
                        } else {
                            ids::UPDATE_CAMERA_X
                        },
                        ids::ENABLE_BLANKING_GRAPHICS,
                    ]
                );
                let start = caller.metadata().address(revision);
                for (index, offset) in [0x3a, 0x40, 0x44, 0x56].into_iter().enumerate() {
                    let call = &machine.calls[index];
                    assert_eq!(call.return_pc, start + offset);
                    assert_eq!(call.sp, Machine::SP - if index < 3 { 20 } else { 16 });
                    assert_eq!(call.argument, if index < 3 { 1 } else { saved_data[2] });
                    assert_eq!(call.d2, camera as u32);
                    assert_eq!(call.a2, pending.address());
                }
                assert_eq!(
                    machine.main_accesses(),
                    [
                        Access::Read(other.address(), 1),
                        Access::Discard(other.address(), 1),
                        Access::Write(other.address(), 1, 0),
                        Access::Write(
                            ram::g_v_blank_audio_input_override(revision).address(),
                            1,
                            1
                        ),
                        Access::Write(ram::g_v_blank_graphics_paused(revision).address(), 1, 1),
                        Access::Write(other.address(), 1, 0x7e),
                        Access::Discard(ram::g_v_blank_graphics_paused(revision).address(), 1),
                        Access::Write(ram::g_v_blank_graphics_paused(revision).address(), 1, 0),
                        Access::Discard(ram::g_v_blank_audio_input_override(revision).address(), 1),
                        Access::Write(
                            ram::g_v_blank_audio_input_override(revision).address(),
                            1,
                            0
                        ),
                    ]
                );
                assert_eq!(machine.data, saved_data);
                assert_eq!(machine.address, saved_addresses);
                assert_eq!(machine.sp, Machine::SP);
                assert_eq!(machine.frame, FrameState::new(Machine::SP));
            }
        }
    }
}

#[test]
fn movers_preserve_word_access_sequence_and_prepare_the_selected_flush() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (direction, caller, offsets, initial, expected, bit) in [
            (
                Direction::Up,
                ids::MOVE_CAMERA_UP,
                [0x46, 0x82],
                17_i16,
                16_i16,
                1,
            ),
            (
                Direction::Down,
                ids::MOVE_CAMERA_DOWN,
                [0x56, 0xa0],
                15,
                16,
                4,
            ),
            (
                Direction::Left,
                ids::MOVE_CAMERA_LEFT,
                [0x46, 0x82],
                17,
                16,
                8,
            ),
            (
                Direction::Right,
                ids::MOVE_CAMERA_RIGHT,
                [0x46, 0x82],
                15,
                16,
                2,
            ),
        ] {
            for camera in [0_i16, 1, -1] {
                let mut machine = Machine::new(revision);
                let data = machine.data;
                let addresses = machine.address;
                let selected = i32::from(camera != 0);
                let vertical = matches!(direction, Direction::Up | Direction::Down);
                let positions = if vertical {
                    [
                        ram::g_camera1_pending_y(revision),
                        ram::g_camera2_pending_y(revision),
                    ]
                } else {
                    [
                        ram::g_camera1_pending_x(revision),
                        ram::g_camera2_pending_x(revision),
                    ]
                };
                let position = positions[selected as usize];
                position.write(&mut machine.memory, initial);
                ram::g_player_lives(revision)
                    .at(0)
                    .write(&mut machine.memory, 3);
                let pending = if vertical {
                    ram::g_camera_row_upload_pending(revision)
                } else {
                    ram::g_camera_column_upload_pending(revision)
                };
                pending.at(selected).write(&mut machine.memory, 1);
                let cursor = if vertical {
                    super::super::tiles::row_cursor(revision, camera)
                } else {
                    super::super::tiles::column_cursor(revision, camera)
                };
                cursor.write(&mut machine.memory, 0);
                let directions = if selected == 0 {
                    ram::g_camera1_pending_tile_directions(revision)
                } else {
                    ram::g_camera2_pending_tile_directions(revision)
                };
                assert_eq!(
                    move_pixel(&mut machine, camera, direction).unwrap(),
                    Some(0)
                );
                assert_eq!(position.read(&machine.memory), expected);
                assert_eq!(directions.read(&machine.memory), bit);
                let call = &machine.calls[0];
                assert_eq!(
                    call.id,
                    if vertical {
                        ids::TRY_UPDATE_CAMERA_Y
                    } else {
                        ids::TRY_UPDATE_CAMERA_X
                    }
                );
                assert_eq!(call.sp, Machine::SP - 20);
                assert_eq!(call.argument, selected as u32);
                assert_eq!(
                    call.return_pc,
                    caller.metadata().address(revision) + offsets[selected as usize]
                );
                assert_eq!(call.d2, (data[2] & 0xffff0000) | 1);
                assert_eq!([call.a2, call.a3], positions.map(|value| value.address()));
                let accesses = machine.main_accesses();
                let at = accesses
                    .iter()
                    .position(|access| *access == Access::Read(position.address(), 2))
                    .unwrap();
                assert_eq!(
                    &accesses[at..at + 5],
                    [
                        Access::Read(position.address(), 2),
                        Access::Read(position.address(), 2),
                        Access::Write(position.address(), 2, expected as u32),
                        Access::Read(position.address(), 2),
                        Access::Read(cursor.address(), 2),
                    ]
                );
                assert_eq!(machine.data, data);
                assert_eq!(machine.address, addresses);
                assert_eq!(machine.sp, Machine::SP);
            }
        }
    }
}
