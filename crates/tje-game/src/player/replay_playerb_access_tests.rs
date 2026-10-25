//! Source-access regressions. Callee bodies are isolated; entry arguments and
//! selected returns are explicit test inputs, not claims of a replay proof.
//! This arithmetic/annotation/dataflow model deliberately keeps the default
//! no-op call observer, which mints no child token. It provides no Host/Native/
//! Replace invocation, lifetime, witness, admission or proof coverage.
use super::{frames, hottub, jump, reactions};
use crate::call_frame::FrameState;
use crate::execution::ExecutionClass;
use crate::stack_read::{Guard, LocalWidth, Reader, UninitializedLocalRead};
use crate::{Context, Dispatcher, FunctionId, Result, ids};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;
use tje_mem::{AddressSpace, Bus, Platform, Revision, ram};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Access {
    Read(u32, u8),
    Discard(u32, u8),
    Write(u32, u8, u32),
}

#[derive(Debug)]
struct Callee {
    id: FunctionId,
    arguments: Vec<u32>,
    entry_sp: u32,
    return_pc: u32,
}

struct AccessContext {
    memory: AddressSpace,
    accesses: RefCell<Vec<Access>>,
    calls: Vec<Callee>,
    data: [u32; 8],
    address: [u32; 8],
    status: u16,
    sp: u32,
    frame: FrameState,
    random: VecDeque<u32>,
    guard: Arc<Guard>,
    declared_locals: Vec<(UninitializedLocalRead, u8, u32)>,
}

impl AccessContext {
    fn new(revision: Revision, arguments: &[u32]) -> Self {
        let mut memory = AddressSpace::new(vec![0; 0x100000], revision);
        let sp = 0xfff000;
        memory.write_u32(sp, 0x1000);
        for (index, &value) in arguments.iter().enumerate() {
            memory.write_u32(sp + 4 + index as u32 * 4, value);
        }
        Self {
            memory,
            accesses: RefCell::new(Vec::new()),
            calls: Vec::new(),
            data: std::array::from_fn(|i| 0x3456_0000 | i as u32),
            address: std::array::from_fn(|i| 0x5678_0000 | i as u32),
            status: 0x201f,
            sp,
            frame: FrameState::new(sp),
            random: VecDeque::new(),
            guard: Arc::new(Guard::default()),
            declared_locals: Vec::new(),
        }
    }

    fn prepare(&mut self) {
        let revision = self.revision();
        let player = ram::g_player_info(revision).at(0);
        player.entity_index().write(&mut self.memory, 0);
        player.level().write(&mut self.memory, 1);
        player.x().write(&mut self.memory, 512);
        player.y().write(&mut self.memory, 512);
        player.animation_group_mask().write(&mut self.memory, 1);
        let earthling = ram::g_earthling_banks(revision).at(0).at(0);
        earthling.kind().write(&mut self.memory, 1);
        earthling.object().write(&mut self.memory, 25);
        earthling.x().write(&mut self.memory, 512);
        earthling.y().write(&mut self.memory, 516);
        let attacker = ram::g_player_info(revision).at(25);
        attacker.x().write(&mut self.memory, 512);
        attacker.y().write(&mut self.memory, 512);
        ram::g_random_cache_index(revision).write(&mut self.memory, 58);
    }

    fn at(&self, address: u32) -> Vec<Access> {
        self.accesses
            .borrow()
            .iter()
            .filter(|event| match event {
                Access::Read(a, _) | Access::Discard(a, _) | Access::Write(a, _, _) => {
                    *a == address
                }
            })
            .cloned()
            .collect()
    }

    fn assert_cleared(&self, address: u32, bits: u8) {
        let events = self.at(address);
        assert!(
            events.windows(2).any(|pair| pair
                == [
                    Access::Discard(address, bits),
                    Access::Write(address, bits, 0),
                ]),
            "missing CLR read/write at {address:06x}: {events:?}"
        );
    }

    fn ids(&self) -> Vec<FunctionId> {
        self.calls.iter().map(|call| call.id).collect()
    }
    fn original_entry_effects(&mut self, id: FunctionId, _: ExecutionClass) -> Result<()> {
        let count = usize::from(id.metadata().abi.argument_count);
        let arguments: Vec<_> = (0..count)
            .map(|index| self.memory.read_u32(self.sp + 4 + index as u32 * 4))
            .collect();
        let result = match id {
            ids::ABS => (arguments[0] as i32).unsigned_abs(),
            ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT => 1,
            ids::GET_RANDOM_NUMBER => self.random.pop_front().expect("scripted RNG call"),
            ids::ELEVATOR_BLOCKS_PLAYER_MOVEMENT
            | ids::GET_ACTIVE_PRESENT_SPRITE
            | ids::PLAYER_YOUCH
            | ids::SET_SPRITE
            | ids::SET_EARTHLING_STATE
            | ids::GENERATE_SPEECH
            | ids::PLAY_SOUND_EFFECT
            | ids::PLAY_SOUND
            | ids::ENTITY_UPDATE_TERRAIN_DRAW_FLAG
            | ids::PLAYER_START_FALLING
            | ids::PLAYER_APPLY_DIRECTIONAL_MOVEMENT_ANIMATION
            | ids::PLAYER_APPLY_PHASED_JUMP_INPUT => 0,
            _ => panic!("unexpected callee {}", id.metadata().name),
        };
        self.calls.push(Callee {
            id,
            arguments,
            entry_sp: self.sp,
            return_pc: self.memory.read_u32(self.sp),
        });
        self.status = 0x2010;
        {
            let value = result;
            if id.metadata().abi.returns_value {
                self.set_data_register(0, value);
            }
            Ok(())
        }
    }
}

impl Bus for AccessContext {
    fn read_u8(&self, address: u32) -> u8 {
        self.guard.read(address, 1, Reader::RustMemory);
        self.accesses.borrow_mut().push(Access::Read(address, 8));
        self.memory.read_u8(address)
    }
    fn read_u16(&self, address: u32) -> u16 {
        self.guard.read(address, 2, Reader::RustMemory);
        self.accesses.borrow_mut().push(Access::Read(address, 16));
        self.memory.read_u16(address)
    }
    fn read_u32(&self, address: u32) -> u32 {
        (u32::from(self.read_u16(address)) << 16) | u32::from(self.read_u16(address + 2))
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 8, u32::from(value)));
        self.memory.write_u8(address, value);
        self.guard.commit(address, 1, true);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 16, u32::from(value)));
        self.memory.write_u16(address, value);
        self.guard.commit(address, 2, true);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.write_u16(address, (value >> 16) as u16);
        self.write_u16(address + 2, value as u16);
    }
}
impl Platform for AccessContext {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("unexpected VDP control access")
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("unexpected VDP data access")
    }
}
impl crate::Platform for AccessContext {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        panic!("unexpected device read {address:06x}")
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        panic!("unexpected device read {address:06x}")
    }
    fn write_device_u8(&mut self, address: u32, _: u8) {
        panic!("unexpected device write {address:06x}")
    }
    fn write_device_u16(&mut self, address: u32, _: u16) {
        panic!("unexpected device write {address:06x}")
    }
    // As in HookContext::read_bus_tagged, a tagged discarded bus read has
    // no exposed operand; it is recorded, not treated as a semantic fetch.
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.accesses.borrow_mut().push(Access::Discard(address, 8));
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.accesses
            .borrow_mut()
            .push(Access::Discard(address, 16));
        Ok(())
    }
}
impl Context for AccessContext {
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
    fn set_frame_state(&mut self, value: FrameState) -> Result<()> {
        self.frame = value;
        Ok(())
    }
    fn read_uninitialized_local(&mut self, read: UninitializedLocalRead) -> Result<u32> {
        let address =
            self.guard
                .explicit_local(read, self.frame, self.sp, self.status & 0x2000 != 0)?;
        let mask = self.guard.unwritten_mask(address, read.width.bytes());
        let value = match read.width {
            LocalWidth::Byte => u32::from(self.memory.read_u8(address)),
            LocalWidth::Word => u32::from(self.memory.read_u16(address)),
            LocalWidth::Long => self.memory.read_u32(address),
        };
        self.accesses
            .borrow_mut()
            .push(Access::Read(address, read.width.bytes() * 8));
        self.declared_locals.push((read, mask, value));
        Ok(value)
    }
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::TimeAware
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with(&[])
    }
    fn call_original(&mut self, id: FunctionId, _: &[u32]) -> Result<u32> {
        panic!("unprepared call {}", id.metadata().name)
    }
    fn call_original_entry(&mut self, id: FunctionId, class: ExecutionClass) -> Result<u32> {
        self.original_entry_effects(id, class)?;
        Ok(self.data_register(0))
    }
    fn call_original_entry_void(&mut self, id: FunctionId, class: ExecutionClass) -> Result<()> {
        self.original_entry_effects(id, class)
    }
}

fn context(revision: Revision, root: FunctionId, tail: &[u32]) -> AccessContext {
    let player = ram::g_player_info(revision).at(0).address();
    let earthling = ram::g_earthling_banks(revision).at(0).at(0).address();
    let arguments = match root {
        ids::PLAYER_YOUCH | ids::HULA_ABILITY | ids::HOTTUB_ABILITY | ids::CHICKEN_ABILITY => {
            vec![0, earthling]
        }
        ids::PLAYER_APPLY_PHASED_JUMP_INPUT => vec![player, 0, tail[0], tail[1]],
        _ => vec![player, tail[0]],
    };
    let mut context = AccessContext::new(revision, &arguments);
    context.prepare();
    context
}

#[test]
fn youch_shark_rejection_keeps_three_distinct_sprite_reads_and_abi() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = context(revision, ids::PLAYER_YOUCH, &[]);
        let p = ram::g_player_info(revision).at(0);
        ram::g_earthling_banks(revision)
            .at(0)
            .at(0)
            .kind()
            .write(&mut c.memory, 14);
        let saved = (c.data, c.address, c.sp);
        assert_eq!(reactions::player_youch(&mut c).unwrap(), Some(1));
        assert_eq!(
            c.at(p.sprite_state().address()),
            vec![Access::Read(p.sprite_state().address(), 8); 3]
        );
        assert_eq!(
            c.at(p.animation_group_mask().address()),
            vec![Access::Read(p.animation_group_mask().address(), 16)]
        );
        assert_eq!(
            c.at(p.animation_group_mask().address() + 2),
            vec![Access::Read(p.animation_group_mask().address() + 2, 16)]
        );
        assert_eq!((c.data, c.address, c.sp), saved);
        assert!(c.calls.is_empty());
    }
}

#[test]
fn youch_dentist_keeps_kind_rereads_abs_and_discarded_clears() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = context(revision, ids::PLAYER_YOUCH, &[]);
        let p = ram::g_player_info(revision).at(0);
        let e = ram::g_earthling_banks(revision).at(0).at(0);
        e.kind().write(&mut c.memory, 21);
        assert_eq!(reactions::player_youch(&mut c).unwrap(), Some(0));
        assert_eq!(
            c.ids(),
            vec![
                ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
                ids::ABS,
                ids::ELEVATOR_BLOCKS_PLAYER_MOVEMENT,
                ids::SET_SPRITE,
                ids::SET_EARTHLING_STATE,
                ids::GENERATE_SPEECH,
                ids::PLAY_SOUND_EFFECT
            ]
        );
        assert_eq!(
            c.at(e.kind().address()),
            vec![Access::Read(e.kind().address(), 8); 4]
        );
        c.assert_cleared(p.height().address(), 16);
        c.assert_cleared(
            ram::g_player_info(revision).at(25).velocity_x().address(),
            16,
        );
        c.assert_cleared(
            ram::g_player_info(revision).at(25).velocity_y().address(),
            16,
        );
    }
}

#[test]
fn hula_random_increment_reloads_the_index_after_its_rmw() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = context(revision, ids::HULA_ABILITY, &[]);
        let index = ram::g_random_cache_index(revision);
        index.write(&mut c.memory, 57);
        reactions::hula_ability(&mut c).unwrap();
        assert_eq!(
            c.at(index.address()),
            vec![
                Access::Read(index.address(), 16),
                Access::Read(index.address(), 16),
                Access::Write(index.address(), 16, 58),
                Access::Read(index.address(), 16)
            ]
        );
        let random = ram::g_cached_random_values(revision).at(58).address();
        assert_eq!(c.at(random), vec![Access::Read(random, 16)]);
        assert_eq!(
            c.ids(),
            vec![ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT, ids::ABS, ids::ABS]
        );
    }
}

#[test]
fn hula_keeps_speech_arguments_live_under_the_sound_call() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = context(revision, ids::HULA_ABILITY, &[]);
        ram::g_cached_random_values(revision)
            .at(0)
            .write(&mut c.memory, 3);
        reactions::hula_ability(&mut c).unwrap();
        let speech = c
            .calls
            .iter()
            .find(|call| call.id == ids::GENERATE_SPEECH)
            .unwrap();
        let sound = c
            .calls
            .iter()
            .find(|call| call.id == ids::PLAY_SOUND_EFFECT)
            .unwrap();
        assert_eq!(sound.entry_sp + 12, speech.entry_sp);
        assert_eq!(speech.arguments[0], 0);
        assert_eq!(speech.arguments[2], 1);
        assert_eq!(sound.arguments[1..], [50, 1]);
        assert_eq!(
            ram::g_player_hula_contact_delay(revision)
                .at(0)
                .read(&c.memory),
            4
        );
    }
}

#[test]
fn whirlwind_direction_uses_a_cache_byte_but_duration_uses_a_word() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = context(revision, ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY, &[0x10]);
        let p = ram::g_player_info(revision).at(0);
        p.sprite_state().write(&mut c.memory, 79);
        ram::g_player_whirlwind_timers(revision)
            .at(0)
            .write(&mut c.memory, 100);
        ram::g_cached_random_values(revision)
            .at(0)
            .write(&mut c.memory, 3);
        ram::g_cached_random_values(revision)
            .at(1)
            .write(&mut c.memory, 11);
        reactions::player_update_whirlwind_and_recovery(&mut c).unwrap();
        let cache = ram::g_cached_random_values(revision).at(0).address();
        assert!(c.at(cache).is_empty());
        assert_eq!(c.at(cache + 1), vec![Access::Read(cache + 1, 8)]);
        assert_eq!(c.at(cache + 2), vec![Access::Read(cache + 2, 16)]);
        assert_eq!(p.velocity_x().read(&c.memory), 192);
        assert_eq!(p.velocity_y().read(&c.memory), 192);
        assert_eq!(
            ram::g_player_whirlwind_direction_timers(revision)
                .at(0)
                .read(&c.memory),
            25
        );
        assert_eq!(c.ids(), vec![ids::ENTITY_UPDATE_TERRAIN_DRAW_FLAG]);
    }
}

#[test]
fn whirlwind_recovery_zero_present_preserves_the_axes_clear_read() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = context(revision, ids::PLAYER_UPDATE_WHIRLWIND_AND_RECOVERY, &[0x10]);
        let p = ram::g_player_info(revision).at(0);
        p.sprite_state().write(&mut c.memory, 78);
        reactions::player_update_whirlwind_and_recovery(&mut c).unwrap();
        c.assert_cleared(p.animation_motion_axes().address(), 8);
        assert_eq!(p.velocity_height().read(&c.memory), -1);
        assert_eq!(c.calls[0].arguments, [p.address(), 61]);
    }
}

#[test]
fn hottub_windup_probes_one_byte_and_keeps_signed_counter_wrap() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for previous in [127_i8, -1] {
            let mut c = context(revision, ids::HOTTUB_ABILITY, &[]);
            let p = ram::g_player_info(revision).at(0);
            p.sprite_state().write(&mut c.memory, 28);
            p.animation_group_mask().write(&mut c.memory, 0x80);
            ram::g_player_jump_counters(revision)
                .at(0)
                .write(&mut c.memory, previous);
            hottub::hottub_ability(&mut c).unwrap();
            let group_byte = p.animation_group_mask().address() + 3;
            assert_eq!(c.at(group_byte), vec![Access::Read(group_byte, 8)]);
            assert!(c.at(p.animation_group_mask().address()).is_empty());
            assert_eq!(c.ids(), vec![ids::ABS]);
            assert_eq!(
                ram::g_player_jump_counters(revision).at(0).read(&c.memory),
                previous.wrapping_add(1),
            );
        }
    }
}

#[test]
fn hottub_chatter_then_healing_keeps_post_call_counter_and_health_rmw() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = context(revision, ids::HOTTUB_ABILITY, &[]);
        let p = ram::g_player_info(revision).at(0);
        p.sprite_state().write(&mut c.memory, 119);
        c.random.push_back(0);
        ram::g_player_jump_counters(revision)
            .at(0)
            .write(&mut c.memory, 5);
        ram::g_player_health(revision)
            .at(0)
            .write(&mut c.memory, -1);
        hottub::hottub_ability(&mut c).unwrap();
        assert_eq!(
            c.ids(),
            vec![ids::ABS, ids::GET_RANDOM_NUMBER, ids::GENERATE_SPEECH]
        );
        assert_eq!(p.x().read(&c.memory), 512);
        let counter = ram::g_player_jump_counters(revision).at(0).address();
        assert_eq!(
            c.at(counter),
            vec![
                Access::Read(counter, 8),
                Access::Write(counter, 8, 6),
                Access::Read(counter, 8)
            ]
        );
        assert_eq!(ram::g_player_health(revision).at(0).read(&c.memory), 0);
    }
}

#[test]
fn phased_input_minus_one_never_reads_the_unused_jump_argument() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = context(
            revision,
            ids::PLAYER_APPLY_PHASED_JUMP_INPUT,
            &[u32::MAX, 64],
        );
        let p = ram::g_player_info(revision).at(0);
        p.velocity_x().write(&mut c.memory, 600);
        p.velocity_y().write(&mut c.memory, -600);
        ram::g_player_phased_jump_speech_used(revision)
            .at(0)
            .write(&mut c.memory, 1);
        let unused = c.frame.argument_byte(3).address();
        jump::player_apply_phased_jump_input(&mut c).unwrap();
        assert!(c.at(unused).is_empty());
        assert_eq!(c.ids(), vec![ids::ABS, ids::ABS]);
        assert_eq!(p.velocity_x().read(&c.memory), 384);
        assert_eq!(p.velocity_y().read(&c.memory), -256);
    }
}

#[test]
fn phased_input_high_jump_keeps_the_clr_read_and_speech_store_order() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = context(revision, ids::PLAYER_APPLY_PHASED_JUMP_INPUT, &[1, 64]);
        let p = ram::g_player_info(revision).at(0);
        p.height().write(&mut c.memory, 50);
        jump::player_apply_phased_jump_input(&mut c).unwrap();
        c.assert_cleared(p.velocity_height().address(), 16);
        assert_eq!(c.ids(), vec![ids::ABS, ids::ABS, ids::GENERATE_SPEECH]);
        assert_eq!(
            ram::g_player_phased_jump_speech_used(revision)
                .at(0)
                .read(&c.memory),
            1
        );
    }
}

#[test]
fn phased_motion_walk_early_exit_keeps_discarded_height_read() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = context(revision, ids::PLAYER_UPDATE_PHASED_JUMP_MOTION, &[0x10]);
        let p = ram::g_player_info(revision).at(0);
        p.animation_group_mask().write(&mut c.memory, 4);
        p.draw_flags().write(&mut c.memory, 0x40);
        jump::player_update_phased_jump_motion(&mut c).unwrap();
        c.assert_cleared(p.height().address(), 16);
        assert!(c.calls.is_empty());
    }
}

#[test]
fn phased_motion_void_landing_keeps_all_interleaved_clear_reads() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = context(revision, ids::PLAYER_UPDATE_PHASED_JUMP_MOTION, &[0x90]);
        let p = ram::g_player_info(revision).at(0);
        p.sprite_state().write(&mut c.memory, 61);
        p.animation_group_mask().write(&mut c.memory, 0x8000);
        p.height().write(&mut c.memory, 2);
        p.velocity_height().write(&mut c.memory, -1);
        ram::g_player_phased_jump_phase(revision)
            .at(0)
            .write(&mut c.memory, 2);
        jump::player_update_phased_jump_motion(&mut c).unwrap();
        for address in [
            p.velocity_x().address(),
            p.acceleration_x().address(),
            p.velocity_y().address(),
            p.acceleration_y().address(),
            p.height().address(),
            p.velocity_height().address(),
            p.acceleration_height().address(),
        ] {
            c.assert_cleared(address, 16);
        }
        c.assert_cleared(ram::g_player_jump_counters(revision).at(0).address(), 8);
        assert!(c.calls.is_empty());
    }
}

#[test]
fn arithmetic_helpers_preserve_control_bits_and_68000_overflow_flags() {
    let mut c = AccessContext::new(Revision::Rev02, &[]);
    assert_eq!(frames::subtraction_flags(&mut c, 0x80, 1, 8, true), 0x7f);
    assert_eq!(c.status, 0x2002);
    assert_eq!(frames::addition_flags(&mut c, 0xff, 1, 8), 0);
    assert_eq!(c.status, 0x2015);
    frames::subtraction_flags(&mut c, 0, 0xff, 8, false);
    assert_eq!(c.status, 0x2011);
    frames::logic_flags(&mut c, 0x80, 8);
    assert_eq!(c.status, 0x2018);
}

#[test]
fn chicken_reads_only_the_source_argument_lanes_before_its_first_field_access() {
    let entry = super::PENDING_FRAMED_TRANSLATIONS
        .iter()
        .find(|entry| entry.id == ids::CHICKEN_ABILITY)
        .expect("ChickenAbility must use a canonical framed pending entry");
    assert!(
        !super::PENDING_TRANSLATIONS
            .iter()
            .any(|entry| entry.id == ids::CHICKEN_ABILITY)
    );
    for revision in [Revision::Rev02, Revision::Rev00] {
        for number in 0..2 {
            for high_word in [0xa55a_u32, 0xffff] {
                let mut c = context(revision, ids::CHICKEN_ABILITY, &[]);
                let p = ram::g_player_info(revision).at(number);
                p.x().write(&mut c.memory, 512);
                p.y().write(&mut c.memory, 512);
                c.frame
                    .argument_slot(0)
                    .write(&mut c.memory, (high_word << 16) | number as u32);
                let saved_sp = c.sp;
                let saved_data = c.data;
                let saved_address = c.address;
                assert_eq!((entry.function)(&mut c).unwrap(), None);
                let argument_reads: Vec<_> = c.accesses.borrow().iter().filter(|event| {
                    matches!(event, Access::Read(address, _) if (saved_sp + 4..saved_sp + 12).contains(address))
                }).cloned().collect();
                assert_eq!(
                    argument_reads,
                    vec![
                        Access::Read(saved_sp + 6, 16),
                        Access::Read(saved_sp + 8, 16),
                        Access::Read(saved_sp + 10, 16),
                    ]
                );
                assert!(
                    c.at(saved_sp + 4).is_empty(),
                    "unused high word was observed"
                );
                let reads: Vec<_> = c
                    .accesses
                    .borrow()
                    .iter()
                    .filter(|event| matches!(event, Access::Read(_, _)))
                    .take(4)
                    .cloned()
                    .collect();
                assert_eq!(
                    reads,
                    vec![
                        Access::Read(saved_sp + 6, 16),
                        Access::Read(saved_sp + 8, 16),
                        Access::Read(saved_sp + 10, 16),
                        Access::Read(p.x().address(), 16),
                    ]
                );
                assert_eq!(c.ids(), vec![ids::ABS, ids::ABS, ids::PLAYER_YOUCH]);
                assert_eq!(
                    c.calls[2].arguments,
                    vec![
                        number as u32,
                        ram::g_earthling_banks(revision).at(0).at(0).address()
                    ]
                );
                for (call, offset) in c.calls.iter().zip([0x34, 0x4e, 0x6a]) {
                    assert_eq!(
                        call.return_pc,
                        ids::CHICKEN_ABILITY.metadata().address(revision) + offset
                    );
                }
                assert_eq!(c.sp, saved_sp);
                assert_eq!(c.data[2..], saved_data[2..]);
                assert_eq!(c.address[2..], saved_address[2..]);
            }
        }
    }
}

#[test]
fn chicken_distant_target_keeps_rng_reread_and_earthling_first_facing_reads() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (x, y, next, vertical) in [(700, 650, 10, false), (600, 700, 8, true)] {
            let mut c = context(revision, ids::CHICKEN_ABILITY, &[]);
            let p = ram::g_player_info(revision).at(0);
            let e = ram::g_earthling_banks(revision).at(0).at(0);
            e.x().write(&mut c.memory, x);
            e.y().write(&mut c.memory, y);
            let index = ram::g_random_cache_index(revision);
            index.write(&mut c.memory, 57);
            ram::g_cached_random_values(revision)
                .at(58)
                .write(&mut c.memory, 99);
            reactions::chicken_ability(&mut c).unwrap();
            assert_eq!(
                c.ids(),
                vec![
                    ids::ABS,
                    ids::ABS,
                    ids::PLAYER_CAN_RECEIVE_EARTHLING_EFFECT,
                    ids::SET_EARTHLING_STATE,
                ]
            );
            assert_eq!(c.calls[3].arguments, vec![e.address(), next]);
            assert_eq!(
                c.calls[2].return_pc,
                ids::CHICKEN_ABILITY.metadata().address(revision) + 0xd8
            );
            assert_eq!(
                c.calls[3].return_pc,
                ids::CHICKEN_ABILITY.metadata().address(revision) + 0x114
            );
            assert_eq!(
                c.at(index.address()),
                vec![
                    Access::Read(index.address(), 16),
                    Access::Read(index.address(), 16),
                    Access::Write(index.address(), 16, 58),
                    Access::Read(index.address(), 16),
                ]
            );
            let coordinates = [
                p.x().address(),
                e.x().address(),
                p.y().address(),
                e.y().address(),
            ];
            let reads: Vec<_> = c.accesses.borrow().iter().filter(|event| {
                matches!(event, Access::Read(address, _) if coordinates.contains(address))
            }).cloned().collect();
            let (source, target) = if vertical {
                (e.y().address(), p.y().address())
            } else {
                (e.x().address(), p.x().address())
            };
            assert_eq!(
                reads,
                vec![
                    Access::Read(p.x().address(), 16),
                    Access::Read(e.x().address(), 16),
                    Access::Read(p.y().address(), 16),
                    Access::Read(e.y().address(), 16),
                    Access::Read(source, 16),
                    Access::Read(target, 16),
                ]
            );
        }
    }
}

fn chooser_residue_context(revision: Revision, high: u8, axes: i8) -> AccessContext {
    // Keep the test stack below the game's globals, as on the real machine.
    let mut c = AccessContext::new(revision, &[]);
    c.prepare();
    c.sp = 0xff7000;
    c.frame = FrameState::new(c.sp);
    c.memory.write_u32(c.sp, 0x1000);
    let p = ram::g_player_info(revision).at(0);
    for (index, argument) in [p.address(), 0, 1, 0x10].into_iter().enumerate() {
        c.frame
            .argument_slot(index as i32)
            .write(&mut c.memory, argument);
    }
    p.sprite_state().write(&mut c.memory, 28);
    p.animation_group_mask().write(&mut c.memory, 0x80);
    p.animation_motion_axes().write(&mut c.memory, axes);
    c.memory.write_u16(c.sp - 10, (u16::from(high) << 8) | 0x5a);
    c
}

#[test]
fn chooser_declares_only_the_original_mixed_mode_word_without_initializing_its_high_byte() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for high in [0x00, 0x55, 0xaa, 0xff] {
            for axes in [i8::MIN, -1, 0, 3, i8::MAX] {
                let mut c = chooser_residue_context(revision, high, axes);
                let guard = Arc::clone(&c.guard);
                let entry_sp = c.sp;
                let saved_data = c.data;
                let saved_address = c.address;
                let result = guard
                    .run_root(
                        ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT,
                        entry_sp,
                        c.status & 0x2000 != 0,
                        || super::directional::player_choose_directional_movement(&mut c),
                    )
                    .unwrap()
                    .unwrap();
                assert_eq!(result, None);
                assert!(guard.failure().is_none());
                assert_eq!(c.declared_locals.len(), 1);
                let (read, missing, value) = c.declared_locals[0];
                assert_eq!(read.owner, ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT);
                assert_eq!(read.frame.entry_sp, entry_sp);
                assert_eq!((read.offset, read.width), (-10, LocalWidth::Word));
                assert!(read.citation.contains("286-289"));
                assert_eq!(missing, 1, "only the old high byte is an entry input");
                assert_eq!(value, (u32::from(high) << 8) | u32::from(axes as u8));
                assert_eq!(
                    c.at(entry_sp - 10),
                    vec![
                        Access::Read(entry_sp - 10, 16),
                        Access::Write(entry_sp - 10, 16, u32::from(axes as i16 as u16)),
                    ]
                );
                assert_eq!(
                    c.at(entry_sp - 9),
                    vec![Access::Write(entry_sp - 9, 8, u32::from(axes as u8)),]
                );
                assert_eq!(c.memory.read_u16(entry_sp - 10), axes as i16 as u16);
                assert_eq!(guard.unwritten_mask(entry_sp - 10, 2), 0);
                assert_eq!(c.sp, entry_sp);
                assert_eq!(c.data[2..], saved_data[2..]);
                assert_eq!(c.address[2..], saved_address[2..]);
                assert!(c.calls.is_empty());
            }
        }
    }
}

#[test]
fn chooser_mode_word_without_an_explicit_declaration_still_trips_the_guard() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = chooser_residue_context(revision, 0xaa, 3);
        let guard = Arc::clone(&c.guard);
        let entry_sp = c.sp;
        let failure = guard
            .run_root(
                ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT,
                entry_sp,
                c.status & 0x2000 != 0,
                || {
                    let frame = crate::call_frame::LocalFrame::enter(
                        &mut c,
                        crate::call_frame::FrameLayout::link(8, 0x7c, 0),
                    )
                    .unwrap();
                    frame.local::<i8>(-5).write(&mut c, 3);
                    let _ = frame.local::<i16>(-6).read(&c);
                },
            )
            .unwrap_err();
        assert_eq!(failure.root, ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT);
        assert_eq!(failure.address, entry_sp - 10);
        assert_eq!(failure.width, 2);
        assert_eq!(
            failure.reason,
            crate::stack_read::UnsupportedReason::Unwritten
        );
    }
}
