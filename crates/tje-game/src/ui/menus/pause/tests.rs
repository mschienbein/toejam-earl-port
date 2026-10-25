use super::apply_ship_piece_hint;
use crate::{Context, Dispatcher, FunctionId, Result};
use std::{cell::RefCell, collections::VecDeque};
use tje_mem::{AddressSpace, Bus, Revision, ram};

#[derive(Debug, PartialEq, Eq)]
enum Access {
    Read(u32, u8),
    Write(u32, u8),
    Discard(u32, u8),
}

struct Fixture {
    memory: AddressSpace,
    accesses: RefCell<Vec<Access>>,
    reads: RefCell<VecDeque<(u32, u8)>>,
    data: [u32; 8],
    addresses: [u32; 7],
    status: u16,
}
impl Fixture {
    fn new(revision: Revision, assignments: [i8; 10], level: i8) -> Self {
        let mut memory = AddressSpace::new(vec![0; 0x100000], revision);
        for (index, value) in assignments.into_iter().enumerate() {
            ram::g_ship_piece_level_assignments(revision)
                .at(index as i32)
                .write(&mut memory, value);
        }
        ram::g_player1_map_level(revision).write(&mut memory, level);
        Self {
            memory,
            accesses: RefCell::default(),
            reads: RefCell::default(),
            data: [0; 8],
            addresses: [0; 7],
            status: 0x2000,
        }
    }
    fn read_addresses(&self) -> Vec<u32> {
        self.accesses
            .borrow()
            .iter()
            .filter_map(|access| match access {
                Access::Read(address, _) => Some(*address),
                Access::Write(_, _) | Access::Discard(_, _) => None,
            })
            .collect()
    }
}
impl Bus for Fixture {
    fn read_u8(&self, address: u32) -> u8 {
        let value = {
            let mut reads = self.reads.borrow_mut();
            if reads
                .front()
                .is_some_and(|&(expected, _)| expected == address)
            {
                reads.pop_front().unwrap().1
            } else {
                self.memory.read_u8(address)
            }
        };
        self.accesses
            .borrow_mut()
            .push(Access::Read(address, value));
        value
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.accesses.get_mut().push(Access::Write(address, value));
        self.memory.write_u8(address, value);
    }
}
impl tje_mem::Platform for Fixture {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("pause cheat unexpectedly writes VDP control")
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("pause cheat unexpectedly writes VDP data")
    }
}
impl crate::Platform for Fixture {
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.accesses.get_mut().push(Access::Discard(address, 1));
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.accesses.get_mut().push(Access::Discard(address, 2));
        Ok(())
    }
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("pause cheat unexpectedly reads a device")
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("pause cheat unexpectedly reads a device")
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("pause cheat unexpectedly writes a device")
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("pause cheat unexpectedly writes a device")
    }
}
impl Context for Fixture {
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
        self.status
    }
    fn set_status_register(&mut self, value: u16) {
        self.status = value;
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid()
    }
    fn call_original(&mut self, id: FunctionId, _: &[u32]) -> Result<u32> {
        panic!("pause cheat unexpectedly calls {}", id.metadata().name)
    }
}

#[test]
fn hint_comparison_rechecks_candidate_after_minimum_then_reads_level_before_entry() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Fixture::new(revision, [4, 0, 2, 0, 0, 0, 0, 0, 0, 0], 5);
        let entries = ram::g_ship_piece_level_assignments(revision);
        let base = entries.address();
        let level = ram::g_player1_map_level(revision).address();
        assert_eq!(apply_ship_piece_hint(&mut context).unwrap(), 2);
        let mut expected = vec![base, base + 1, base + 2, base, base + 2];
        expected.extend((3..10).map(|index| base + index));
        expected.extend([level, base + 2, level]);
        assert_eq!(context.read_addresses(), expected);
        assert_eq!(entries.at(2).read(&context.memory), 6);
        for index in (0..10).filter(|&index| index != 2) {
            assert_eq!(entries.at(index).read(&context.memory), 0);
        }
    }
}

#[test]
fn hint_uses_the_comparison_read_not_the_earlier_nonzero_test_value() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Fixture::new(revision, [2, 1, 0, 0, 0, 0, 0, 0, 0, 0], 2);
        let entries = ram::g_ship_piece_level_assignments(revision);
        let base = entries.address();
        context
            .reads
            .get_mut()
            .extend([(base, 2), (base + 1, 1), (base, 2), (base + 1, 3)]);
        assert_eq!(apply_ship_piece_hint(&mut context).unwrap(), 0);
        assert!(context.reads.borrow().is_empty());
        assert_eq!(entries.at(0).read(&context.memory), 2);
        assert_eq!(entries.at(1).read(&context.memory), 0);
    }
}

#[test]
fn hint_preserves_signed_byte_minimum_and_all_zero_cross_array_index_ten() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let entries = ram::g_ship_piece_level_assignments(revision);
        let mut signed = Fixture::new(revision, [3, -4, 0, 0, 0, 0, 0, 0, 0, 0], i8::MIN);
        assert_eq!(apply_ship_piece_hint(&mut signed).unwrap(), 1);
        assert_eq!(entries.at(1).read(&signed.memory), i8::MIN + 1);
        let mut empty = Fixture::new(revision, [0; 10], 5);
        entries.at(10).write(&mut empty.memory, 7);
        assert_eq!(apply_ship_piece_hint(&mut empty).unwrap(), 10);
        assert_eq!(entries.at(10).read(&empty.memory), 6);
        assert!(empty.read_addresses().contains(&entries.at(10).address()));
    }
}

#[test]
fn final_pause_retains_sound_arguments_and_lazy_signed_player() {
    use crate::{ids, ui::menus::test_support::Fixture as FrameFixture};
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context =
            FrameFixture::new(revision, ids::RUN_FINAL_LEVEL_PAUSE_MENU, &[0xaabbffff]);
        context.expect(ids::DRAW_PAUSE_CHOICE_PANEL, 0x1a, &[0], 0);
        context.expect(ids::PLAY_SOUND, 0x28, &[6, 99], 0);
        context.expect(ids::WAIT_FOR_NEXT_V_BLANK, 0x32, &[1], 0);
        context.expect(ids::PLAY_SOUND, 0x40, &[20, 99], 0);
        context.expect(ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, 0x48, &[0], 0);
        context.expect(ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, 0x4e, &[], 0);
        context.expect(ids::MUSIC_REQUEST_TRACK_POINTER_RESET, 0x54, &[], 0);
        context.expect(ids::READ_MAPPED_PLAYER_INPUT, 0x60, &[u32::MAX], 0);
        context.expect(ids::READ_MAPPED_PLAYER_INPUT, 0x74, &[u32::MAX], 0);
        context.expect(ids::READ_MAPPED_PLAYER_INPUT, 0xae, &[u32::MAX], 0x80);
        context.expect(ids::DRAW_PAUSE_CHOICE_PANEL, 0xbe, &[u32::MAX], 0);
        context.expect(ids::READ_MAPPED_PLAYER_INPUT, 0xda, &[u32::MAX], 1);
        super::run_final_level_pause_menu(&mut context).unwrap();
        context.complete();
        assert_eq!(context.calls[6].1, 0xff7fd0, "28-byte retained arguments");
    }
}

#[test]
fn game_pause_early_returns_preserve_lazy_word_arguments_and_saved_registers() {
    use crate::{ids, ui::menus::test_support::Fixture as FrameFixture};
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut opened = FrameFixture::new(revision, ids::GAME_PAUSED, &[0xaabb0001]);
        ram::g_player_inventory_open(revision)
            .at(1)
            .write(&mut opened.memory, -1);
        let saved = opened.data;
        super::game_paused(&mut opened).unwrap();
        opened.complete();
        assert_eq!(&opened.data[2..7], &saved[2..7]);
        let mut failed = FrameFixture::new(revision, ids::GAME_PAUSED, &[0xaabb0000]);
        failed.expect(ids::ALLOCATE_ENTITY_WITH_EVICTION, 0x54, &[0], 0);
        super::game_paused(&mut failed).unwrap();
        failed.complete();
    }
}

#[test]
fn game_pause_uses_real_constructor_and_sound_frames_before_freeing_the_saved_handle() {
    use crate::{ids, ui::menus::test_support::Fixture as FrameFixture};
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = FrameFixture::new(revision, ids::GAME_PAUSED, &[0xaabb0000]);
        let object = ram::g_player_info(revision).at(3);
        object.draw_flags().write(&mut context.memory, 2);
        object.sprite_ready().write(&mut context.memory, 1);
        context.expect(ids::ALLOCATE_ENTITY_WITH_EVICTION, 0x54, &[0], 3);
        context.expect(
            ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
            0x12e,
            &[object.address(), 0],
            0,
        );
        context.expect(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, 0x144, &[], 0);
        context.expect(ids::PLAY_SOUND, 0x158, &[6, 99], 0);
        context.expect(ids::WAIT_FOR_NEXT_V_BLANK, 0x162, &[1], 0);
        context.expect(ids::PLAY_SOUND, 0x170, &[20, 99], 0);
        context.expect(ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, 0x178, &[0], 0);
        context.expect(ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, 0x17e, &[], 0);
        context.expect(ids::MUSIC_REQUEST_TRACK_POINTER_RESET, 0x184, &[], 0);
        context.expect(ids::READ_MAPPED_PLAYER_INPUT, 0x18e, &[1], 0);
        context.expect(ids::READ_MAPPED_PLAYER_INPUT, 0x196, &[0], 0);
        context.expect(ids::READ_MAPPED_PLAYER_INPUT, 0x1a4, &[0], 0x80);
        context.expect(ids::READ_MAPPED_PLAYER_INPUT, 0x1ae, &[1], 0);
        context.expect(ids::FREE_ENTITY, 0x2aa, &[3], 0);
        context.expect(ids::READ_MAPPED_PLAYER_INPUT, 0x2b2, &[1], 0);
        context.expect(ids::READ_MAPPED_PLAYER_INPUT, 0x2ba, &[0], 1);
        super::game_paused(&mut context).unwrap();
        context.complete();
        assert_eq!(
            context.calls[8].1, 0xff7fbc,
            "24-byte retained sound arguments"
        );
    }
}
