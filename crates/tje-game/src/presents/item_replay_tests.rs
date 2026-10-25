use super::*;
use crate::call_frame::FrameState;
use crate::{Dispatcher, Error, Translation};
use std::cell::RefCell;
use tje_mem::{AddressSpace, Bus, Revision};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    Byte(u32),
    Word(u32),
    DiscardByte(u32),
    DiscardWord(u32),
    StoreByte(u32, u8),
    StoreWord(u32, u16),
}

struct Fixture {
    memory: AddressSpace,
    accesses: RefCell<Vec<Access>>,
    revision: Revision,
    data: [u32; 8],
    address: [u32; 7],
    status: u16,
    sp: u32,
    frame: FrameState,
    dispatcher: Dispatcher,
}

const ENTRY_SP: u32 = 0xff7000;
const ABS_COUNT: u32 = 0xff9100;

impl Fixture {
    fn new(revision: Revision) -> Self {
        Self {
            memory: AddressSpace::new(Vec::new(), revision),
            accesses: RefCell::new(Vec::new()),
            revision,
            data: [0; 8],
            address: [0; 7],
            status: 0x201f,
            sp: ENTRY_SP,
            frame: FrameState::new(ENTRY_SP),
            dispatcher: Dispatcher::hybrid_with_overrides(&OVERRIDES),
        }
    }
    fn clear_trace(&self) {
        self.accesses.borrow_mut().clear();
    }
}

impl Bus for Fixture {
    fn read_u8(&self, address: u32) -> u8 {
        self.accesses.borrow_mut().push(Access::Byte(address));
        self.memory.read_u8(address)
    }
    fn read_u16(&self, address: u32) -> u16 {
        self.accesses.borrow_mut().push(Access::Word(address));
        self.memory.read_u16(address)
    }
    fn read_u32(&self, address: u32) -> u32 {
        (u32::from(self.read_u16(address)) << 16) | u32::from(self.read_u16(address + 2))
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.accesses
            .borrow_mut()
            .push(Access::StoreByte(address, value));
        self.memory.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.accesses
            .borrow_mut()
            .push(Access::StoreWord(address, value));
        self.memory.write_u16(address, value);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.write_u16(address, (value >> 16) as u16);
        self.write_u16(address + 2, value as u16);
    }
}

impl tje_mem::Platform for Fixture {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("item RAM regression unexpectedly wrote the VDP");
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("item RAM regression unexpectedly wrote the VDP");
    }
}
impl crate::Platform for Fixture {
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.accesses
            .borrow_mut()
            .push(Access::DiscardByte(address));
        let _ = self.memory.read_u8(address);
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.accesses
            .borrow_mut()
            .push(Access::DiscardWord(address));
        let _ = self.memory.read_u16(address);
        Ok(())
    }
    fn read_device_u8(&mut self, address: u32) -> u8 {
        panic!("unexpected device byte read {address:06x}");
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        panic!("unexpected device word read {address:06x}");
    }
    fn write_device_u8(&mut self, address: u32, _: u8) {
        panic!("unexpected device byte write {address:06x}");
    }
    fn write_device_u16(&mut self, address: u32, _: u16) {
        panic!("unexpected device word write {address:06x}");
    }
}
impl Context for Fixture {
    fn revision(&self) -> Revision {
        self.revision
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
    fn set_frame_state(&mut self, value: FrameState) -> Result<()> {
        self.frame = value;
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        self.dispatcher
    }
    fn call_original(&mut self, function: FunctionId, _: &[u32]) -> Result<u32> {
        Err(Error::Untranslated(function))
    }
}

fn counted_original_translation_abs(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let count = context.read_u16(ABS_COUNT);
    context.write_u16(ABS_COUNT, count + 1);
    let actual = crate::translations()
        .find(|entry| entry.id == ids::ABS)
        .unwrap();
    (actual.function)(context, arguments)
}

fn mutate_parent_record_slot(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    // A legal callee-side alias mutation: the byte must be loaded after the call.
    let sp = context.stack_pointer()?;
    assert_eq!(sp, ENTRY_SP - 56);
    assert_eq!(
        context.read_u32(sp),
        ids::PLAYER_COLLECT_NEARBY_ITEMS
            .metadata()
            .address(context.revision())
            + 0x11a,
    );
    assert_eq!(
        context.read_u32(sp + 4),
        ram::g_player_info(context.revision()).at(0).address(),
    );
    assert_eq!(context.read_u32(sp + 8), 1);
    assert_eq!(context.read_u32(sp + 12), 50);
    context.write_u8(ENTRY_SP + 19, 1);
    Ok(None)
}
static OVERRIDES: [Translation; 2] = [
    Translation {
        id: ids::ABS,
        function: counted_original_translation_abs,
    },
    Translation {
        id: ids::EARTHLING_CLEAR_MOLE_PLAYER_CONTACT_FLAGS,
        function: mutate_parent_record_slot,
    },
];

#[test]
fn nonzero_timer_matches_all_word_values_and_compiled_bus_widths() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut fixture = Fixture::new(revision);
        let timer = Address::<u16>::new(0xff9000);
        for old in 1_u16..=u16::MAX {
            timer.write(&mut fixture.memory, old);
            fixture.clear_trace();
            let show = timer_show_nonzero(&mut fixture, timer);
            let next = old - 1;
            let expected = next > 300
                || if next > 100 {
                    next & 12 != 0
                } else {
                    next & 2 != 0
                };
            assert_eq!(show, expected, "{revision:?}/{old}");
            assert_eq!(timer.read(&fixture.memory), next);
            let accesses = fixture.accesses.borrow();
            assert_eq!(
                &accesses[..3],
                &[
                    Access::Word(timer.address()),
                    Access::StoreWord(timer.address(), next),
                    Access::Word(timer.address()),
                ]
            );
            if next > 300 {
                assert_eq!(accesses.len(), 3);
            } else {
                assert_eq!(accesses[3], Access::Word(timer.address()));
                assert_eq!(
                    accesses[4],
                    if next > 100 {
                        Access::Word(timer.address())
                    } else {
                        Access::Byte(timer.address() + 1)
                    }
                );
                assert_eq!(accesses.len(), 5);
            }
        }
    }
}

#[test]
fn timer_eligibility_retains_both_sprite_comparison_loads() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut fixture = Fixture::new(revision);
        let player = ram::g_player_info(revision).at(0);
        ram::g_player_lives(revision)
            .at(0)
            .write(&mut fixture.memory, 3);
        ram::g_player_powerups(revision)
            .at(0)
            .write(&mut fixture.memory, 1);
        ram::g_player_inventory_open(revision)
            .at(0)
            .write(&mut fixture.memory, 0);
        for state in [0, 5, 6, 77] {
            player.sprite_state().write(&mut fixture.memory, state);
            fixture.clear_trace();
            assert_eq!(
                timer_player_eligible(&fixture, revision, 0, player),
                state != 5 && state != 6
            );
            let accesses = fixture.accesses.borrow();
            let count = accesses
                .iter()
                .filter(|access| **access == Access::Byte(player.sprite_state().address()))
                .count();
            assert_eq!(count, if state == 5 { 1 } else { 2 });
        }
    }
}

#[test]
fn collection_eligibility_retains_three_separate_sprite_loads() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut fixture = Fixture::new(revision);
        let player = ram::g_player_info(revision).at(0);
        ram::g_player_lives(revision)
            .at(0)
            .write(&mut fixture.memory, 3);
        player.height().write(&mut fixture.memory, 0);
        player.animation_group_mask().write(&mut fixture.memory, 1);
        for (state, expected_reads) in [(0, 3), (3, 1), (7, 2), (77, 3)] {
            player.sprite_state().write(&mut fixture.memory, state);
            fixture.clear_trace();
            assert_eq!(
                collection_player_eligible(&mut fixture, revision, 0, player),
                state == 0
            );
            let accesses = fixture.accesses.borrow();
            assert_eq!(
                accesses
                    .iter()
                    .filter(|access| **access == Access::Byte(player.sprite_state().address()))
                    .count(),
                expected_reads
            );
        }
    }
}

#[test]
fn bank_level_is_read_before_player_level_and_unused_bank_short_circuits() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut fixture = Fixture::new(revision);
        let users = ram::g_item_bank_users(revision).at(1);
        let level = ram::g_item_bank_levels(revision).at(1);
        let player = ram::g_player_info(revision).at(0);
        users.write(&mut fixture.memory, 1);
        level.write(&mut fixture.memory, 7);
        player.level().write(&mut fixture.memory, 7);
        fixture.clear_trace();
        assert!(bank_matches_player(&fixture, revision, 1, 0));
        assert_eq!(
            fixture.accesses.borrow().as_slice(),
            &[
                Access::Byte(users.address()),
                Access::Byte(level.address()),
                Access::Byte(player.level().address()),
            ]
        );
        users.write(&mut fixture.memory, 0);
        fixture.clear_trace();
        assert!(!bank_matches_player(&fixture, revision, 1, 0));
        assert_eq!(
            fixture.accesses.borrow().as_slice(),
            &[Access::Byte(users.address())]
        );
    }
}

#[test]
fn pickup_loads_record_slot_after_callee_alias_mutation_and_calls_real_abs() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut fixture = Fixture::new(revision);
        let player = ram::g_player_info(revision).at(0);
        let item = ram::g_dropped_present_table(revision).at(0);
        ram::g_player_lives(revision)
            .at(0)
            .write(&mut fixture.memory, 3);
        player.height().write(&mut fixture.memory, 0);
        player.sprite_state().write(&mut fixture.memory, 0);
        player.animation_group_mask().write(&mut fixture.memory, 1);
        player.x().write(&mut fixture.memory, 200);
        player.y().write(&mut fixture.memory, 100);
        item.object().write(&mut fixture.memory, 1);
        item.kind().write(&mut fixture.memory, 1);
        item.level().write(&mut fixture.memory, 3);
        item.x().write(&mut fixture.memory, 200);
        item.y().write(&mut fixture.memory, 100);
        ram::g_player_inventory(revision)
            .at(0)
            .at(0)
            .write(&mut fixture.memory, -1);
        let bits = ram::g_collected_level_item_bits(revision).at(3);
        bits.write(&mut fixture.memory, 0);
        fixture.memory.write_u32(ENTRY_SP + 4, 0);
        fixture.memory.write_u32(ENTRY_SP + 8, item.address());
        fixture.memory.write_u32(ENTRY_SP + 12, 1);
        fixture.memory.write_u32(ENTRY_SP + 16, 0);
        player_collect_nearby_items(&mut fixture).unwrap();
        assert_eq!(fixture.memory.read_u16(ABS_COUNT), 2);
        assert_eq!(bits.read(&fixture.memory), 0x8000_0000);
        assert_eq!(fixture.memory.read_u8(ENTRY_SP + 19), 1);
        assert_eq!(fixture.sp, ENTRY_SP);
        assert_eq!(fixture.status & 0x1f, 4);
    }
}

#[test]
fn hazards_call_abs_before_distance_rejection_instead_of_inlining_it() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut fixture = Fixture::new(revision);
        let player = ram::g_player_info(revision).at(0);
        let items = ram::g_dropped_present_table(revision);
        player.height().write(&mut fixture.memory, 0);
        player.x().write(&mut fixture.memory, 100);
        let item = items.at(0);
        item.kind().write(&mut fixture.memory, 83);
        item.object().write(&mut fixture.memory, 1);
        item.x().write(&mut fixture.memory, 124);
        super::super::hazard_frames::player_check_level_item_hazards(
            &mut fixture,
            &[0, items.address()],
        )
        .unwrap();
        assert_eq!(fixture.memory.read_u16(ABS_COUNT), 1);
        assert_eq!(fixture.sp, ENTRY_SP);
        assert_eq!(fixture.status & 0x1f, 4);
    }
}

#[test]
fn splash_clr_has_a_tagged_byte_preread_and_preserves_extend() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut fixture = Fixture::new(revision);
        let player = ram::g_player_info(revision).at(0);
        let address = player.animation_motion_axes().address();
        for byte in 0..=u8::MAX {
            for ccr in 0..32 {
                fixture.memory.write_u8(address, byte);
                fixture.status = 0x2700 | ccr;
                fixture.clear_trace();
                super::super::expiry::clear_splash_motion(&mut fixture, player).unwrap();
                assert_eq!(fixture.memory.read_u8(address), 0);
                assert_eq!(fixture.status, 0x2704 | (ccr & 0x10));
                assert_eq!(
                    fixture.accesses.borrow().as_slice(),
                    &[Access::DiscardByte(address), Access::StoreByte(address, 0),]
                );
            }
        }
    }
}

#[test]
fn retained_full_food_speech_keeps_declared_scalar_and_frame_mutations() {
    fn speech(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
        assert_eq!(arguments, &[1, 0xff9100, 1]);
        let sp = context.stack_pointer()?;
        assert_eq!(
            context.read_u32(sp),
            ids::PLAYER_COLLECT_NEARBY_ITEMS
                .metadata()
                .address(context.revision())
                + 0x1e6,
        );
        context.write_u32(sp + 12, 2);
        context.set_data_register(1, 0x5566_7788);
        Ok(Some(23))
    }
    static CALLEE: &[Translation] = &[Translation {
        id: ids::GENERATE_SPEECH,
        function: speech,
    }];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = crate::core::test_support::TestContext::with_rom(revision, Vec::new());
        let caller = ids::PLAYER_COLLECT_NEARBY_ITEMS;
        let parent = context.prepare_entry(caller, &[]);
        context.execution = crate::execution::ExecutionClass::TimeAware;
        context.dispatcher_override = Some(Dispatcher::hybrid_with(CALLEE));
        let sp = context.stack_pointer().unwrap();
        let area = super::item_retained_call(
            &mut context,
            caller,
            ids::GENERATE_SPEECH,
            &[1, 0xff9100, 1],
            0x1e6,
        )
        .unwrap();
        assert_eq!(context.stack_pointer().unwrap(), sp - 12);
        assert_eq!(context.frame_state(), Some(parent));
        assert_eq!(area.argument_slot(0).read(&context), 1);
        assert_eq!(area.argument_slot(1).read(&context), 0xff9100);
        assert_eq!(area.argument_slot(2).read(&context), 2);
        assert_eq!(context.data_register(0), 23);
        assert_eq!(context.data_register(1), 0x5566_7788);
        area.release(&mut context).unwrap();
        assert_eq!(context.stack_pointer().unwrap(), sp);
    }
}
