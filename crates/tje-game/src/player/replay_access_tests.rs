//! ROM-free regressions for the pending falling/respawn bodies' own bus accesses.
use crate::{
    Context, Dispatcher, Error, FunctionId, Result,
    call_frame::FrameState,
    execution::{ExecutionClass, RamWidth, WaitCondition, WaitRequest},
    ids,
};
use std::{cell::RefCell, collections::VecDeque};
use tje_mem::{Address, AddressSpace, Bus, ObjectState, Platform, Revision, ram};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Access {
    Read(u32, u8),
    Discard(u32, u8),
    Write(u32, u8, u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SpinStep {
    Poll(u32, u8, u32),
    Call(FunctionId, Vec<u32>),
    Yield,
}
struct PollSamples {
    address: u32,
    width: u8,
    values: VecDeque<u32>,
}
struct CalleeBoundary {
    function: FunctionId,
    arguments: Vec<u32>,
    result: Option<u32>,
}
struct Probe {
    memory: AddressSpace,
    accesses: RefCell<Vec<Access>>,
    calls: Vec<(FunctionId, Vec<u32>)>,
    address_writes: Vec<(usize, u32)>,
    data: [u32; 8],
    address: [u32; 8],
    sr: u16,
    frame: FrameState,
    class: ExecutionClass,
    rust_callees: bool,
    clock: crate::execution::CallClock,
    waits: Vec<WaitRequest>,
    polls: RefCell<Option<PollSamples>>,
    spin_steps: RefCell<Vec<SpinStep>>,
    boundaries: Option<VecDeque<CalleeBoundary>>,
}
impl Probe {
    fn new(revision: Revision) -> Self {
        let sp = 0xff7000;
        let mut address = [0; 8];
        address[6] = 0xff7300;
        address[7] = sp;
        Self {
            memory: AddressSpace::new(vec![0; 0x100000], revision),
            accesses: RefCell::new(Vec::new()),
            calls: Vec::new(),
            address_writes: Vec::new(),
            data: [0; 8],
            address,
            sr: 0x2000,
            frame: FrameState::new(sp),
            class: ExecutionClass::TimeAware,
            rust_callees: true,
            clock: crate::execution::CallClock::default(),
            waits: Vec::new(),
            polls: RefCell::new(None),
            spin_steps: RefCell::new(Vec::new()),
            boundaries: None,
        }
    }
    fn reset_log(&mut self) {
        self.accesses.get_mut().clear();
        self.calls.clear();
        self.address_writes.clear();
        self.waits.clear();
        self.spin_steps.get_mut().clear();
    }
    fn poll_sample(&self, address: u32, width: u8) -> Option<u32> {
        let mut script = self.polls.borrow_mut();
        let script = script.as_mut()?;
        if script.address != address {
            return None;
        }
        assert_eq!(script.width, width, "poll width changed");
        let value = script.values.pop_front().expect("unexpected extra poll");
        self.spin_steps
            .borrow_mut()
            .push(SpinStep::Poll(address, width, value));
        Some(value)
    }
    fn at(&self, address: u32) -> Vec<Access> {
        self.accesses
            .borrow()
            .iter()
            .filter(|access| match access {
                Access::Read(a, _) | Access::Discard(a, _) | Access::Write(a, _, _) => {
                    *a == address
                }
            })
            .cloned()
            .collect()
    }
    fn reads(&self, address: u32) -> usize {
        self.at(address)
            .iter()
            .filter(|a| matches!(a, Access::Read(..)))
            .count()
    }
    fn seed_player(&mut self, number: i32) -> Address<ObjectState> {
        let p = ram::g_player_info(self.revision()).at(number);
        p.entity_index().write(self, number as i8);
        p.level().write(self, 1);
        p.x().write(self, 512);
        p.y().write(self, 100);
        p.height().write(self, 0);
        p.sprite_state().write(self, 0);
        p.frame_duration().write(self, 1);
        p.animation_group_mask().write(self, 0);
        p.draw_flags().write(self, 1);
        ram::g_player_lives(self.revision())
            .at(number)
            .write(self, 2);
        ram::g_player_powerups(self.revision())
            .at(number)
            .write(self, -1);
        p
    }
}
impl Bus for Probe {
    fn read_u8(&self, address: u32) -> u8 {
        self.accesses.borrow_mut().push(Access::Read(address, 1));
        self.poll_sample(address, 1)
            .map_or_else(|| self.memory.read_u8(address), |value| value as u8)
    }
    fn read_u16(&self, address: u32) -> u16 {
        self.accesses.borrow_mut().push(Access::Read(address, 2));
        self.poll_sample(address, 2)
            .map_or_else(|| self.memory.read_u16(address), |value| value as u16)
    }
    // A logical long is deliberately distinguishable from two explicit words.
    fn read_u32(&self, address: u32) -> u32 {
        self.accesses.borrow_mut().push(Access::Read(address, 4));
        self.memory.read_u32(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.accesses
            .get_mut()
            .push(Access::Write(address, 1, u32::from(value)));
        self.memory.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.accesses
            .get_mut()
            .push(Access::Write(address, 2, u32::from(value)));
        self.memory.write_u16(address, value);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.accesses
            .get_mut()
            .push(Access::Write(address, 4, value));
        self.memory.write_u32(address, value);
    }
}
impl Platform for Probe {
    fn write_vdp_control(&mut self, value: u16) {
        self.accesses
            .get_mut()
            .push(Access::Write(0xc00004, 2, u32::from(value)));
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.accesses
            .get_mut()
            .push(Access::Write(0xc00000, 2, u32::from(value)));
    }
}
impl crate::Platform for Probe {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        self.read_u8(address)
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        self.read_u16(address)
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.write_u8(address, value);
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.write_u16(address, value);
    }
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
    fn wait_until(&mut self, request: WaitRequest) -> Result<()> {
        self.waits.push(request);
        if self.polls.borrow().is_none() {
            return Err(Error::Platform("unit wait boundary".into()));
        }
        let WaitCondition::Ram {
            address,
            width: RamWidth::Word,
            mask,
            value,
            comparison,
        } = request.condition
        else {
            return Err(Error::Platform("unit expected a RAM word predicate".into()));
        };
        // Synthetic recorded poll values model replay, not live IRQ timing.
        loop {
            let sample = u32::from(self.read_u16(address));
            if comparison.satisfied(sample, mask, value) {
                return Ok(());
            }
        }
    }
    fn poll_yield(&mut self) -> Result<()> {
        if self.boundaries.is_none() {
            return Err(Error::Platform("unit scheduler boundary".into()));
        }
        self.spin_steps.get_mut().push(SpinStep::Yield);
        Ok(())
    }
}
impl Context for Probe {
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
        self.address_writes.push((index, value));
        self.address[index] = value;
    }
    fn status_register(&self) -> u16 {
        self.sr
    }
    fn set_status_register(&mut self, value: u16) {
        self.sr = value;
    }
    fn stack_pointer(&self) -> Result<u32> {
        Ok(self.address[7])
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.address[7] = value;
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
        self.class
    }
    fn set_execution_class(&mut self, value: ExecutionClass) -> Result<()> {
        self.class = value;
        Ok(())
    }
    fn clocked_execution(&self) -> bool {
        self.clock.active() || self.class != ExecutionClass::Atomic
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
            .map_err(Error::Clock)
    }
    fn leave_call_ceiling(&mut self, scope: crate::execution::CallCeilingScope) -> Result<()> {
        self.clock.leave(scope).map(|_| ()).map_err(Error::Clock)
    }
    fn dispatcher(&self) -> Dispatcher {
        if self.rust_callees {
            Dispatcher::hybrid_with_overrides(super::PENDING_TRANSLATIONS)
        } else {
            Dispatcher::hybrid_with(&[])
        }
    }
    fn call_original(&mut self, id: FunctionId, arguments: &[u32]) -> Result<u32> {
        self.calls.push((id, arguments.to_vec()));
        self.spin_steps
            .get_mut()
            .push(SpinStep::Call(id, arguments.to_vec()));
        if let Some(boundaries) = &mut self.boundaries {
            // Catalogue-boundary responses isolate the caller's access protocol;
            // they are not implementations or proofs of those game callees.
            let expected = boundaries.pop_front().expect("unexpected extra callee");
            assert_eq!(id, expected.function);
            assert_eq!(arguments, expected.arguments);
            return expected.result.ok_or(Error::Untranslated(id));
        }
        Err(Error::Untranslated(id))
    }
    fn call_original_with_execution(
        &mut self,
        id: FunctionId,
        arguments: &[u32],
        _: ExecutionClass,
    ) -> Result<u32> {
        self.call_original(id, arguments)
    }
}

#[test]
fn falling_array_uses_a6_minus_two_and_passes_that_actual_address() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Probe::new(revision);
        let base = c.stack_pointer().unwrap() - 4;
        c.write_u8(base - 4, 0x12);
        c.write_u8(base - 3, 0x34);
        ram::g_player_lives(revision).at(0).write(&mut c, -1);
        ram::g_player_lives(revision).at(1).write(&mut c, -1);
        c.reset_log();
        super::falling_check::check_player_falling(&mut c, &[]).unwrap();
        assert!(c.accesses.borrow().contains(&Access::Write(base - 2, 1, 0)));
        assert!(c.accesses.borrow().contains(&Access::Write(base - 1, 1, 0)));
        assert_eq!(c.memory.read_u8(base - 4), 0x12);
        assert_eq!(c.memory.read_u8(base - 3), 0x34);
        assert!(
            c.accesses
                .borrow()
                .iter()
                .any(|a| matches!(a,Access::Write(_,4,value) if *value==base-2))
        );
        assert_eq!(c.reads(base - 2), 2);
        assert_eq!(c.reads(base - 1), 1);
        assert_eq!(c.stack_pointer().unwrap(), 0xff7000);
        assert_eq!(c.address_register(6), 0xff7300);
    }
}

#[test]
fn sinking_preserves_each_short_circuit_state_read_and_long_word_beats() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (state, count) in [(79, 1), (78, 2), (5, 3), (6, 4), (0, 4)] {
            let mut c = Probe::new(revision);
            let p = c.seed_player(0);
            p.sprite_state().write(&mut c, state);
            c.reset_log();
            super::falling::player_update_terrain_sinking(&mut c, &[p.address(), 0x10]).unwrap();
            assert_eq!(c.reads(p.sprite_state().address()), count);
            assert_eq!(
                c.at(p.animation_group_mask().address()),
                vec![Access::Read(p.animation_group_mask().address(), 2)]
            );
            assert_eq!(
                c.at(p.animation_group_mask().address() + 2),
                vec![Access::Read(p.animation_group_mask().address() + 2, 2)]
            );
        }
    }
}

#[test]
fn descent_btst_is_one_byte_before_the_conditional_long_load() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for group in [0x20, 0x10000000, 0] {
            let mut c = Probe::new(revision);
            let p = c.seed_player(0);
            p.animation_group_mask().write(&mut c, group);
            p.height().write(&mut c, 100);
            c.reset_log();
            super::falling::player_resolve_descending_terrain(&mut c, &[p.address(), 0x10])
                .unwrap();
            let address = p.animation_group_mask().address();
            let actual: Vec<_> = c
                .accesses
                .borrow()
                .iter()
                .filter(|a| matches!(a,Access::Read(at,_) if *at>=address && *at<address+4))
                .cloned()
                .collect();
            let mut expected = vec![Access::Read(address + 3, 1)];
            if group & 0x20 == 0 {
                expected.extend([Access::Read(address, 2), Access::Read(address + 2, 2)]);
            }
            assert_eq!(actual, expected);
        }
    }
}

#[test]
fn pending_damage_is_loaded_before_the_health_subtraction() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Probe::new(revision);
        let p = c.seed_player(0);
        p.animation_group_mask().write(&mut c, 0x2000);
        p.frame_duration().write(&mut c, 0);
        p.height().write(&mut c, 10);
        let health = ram::g_player_health(revision).at(0);
        let damage = ram::g_player_pending_hurt_damage(revision).at(0);
        health.write(&mut c, 20);
        damage.write(&mut c, 7);
        c.reset_log();
        super::falling::player_update_hurt_recovery_and_idle(&mut c, &[p.address(), 0x10]).unwrap();
        let actual:Vec<_>=c.accesses.borrow().iter().filter(|a|matches!(a,Access::Read(at,_) if *at==health.address() || *at==damage.address())).cloned().collect();
        assert_eq!(
            actual,
            vec![
                Access::Read(damage.address(), 1),
                Access::Read(health.address(), 1)
            ]
        );
        assert_eq!(health.read(&c), 13);
    }
}

#[test]
fn death_bne_uses_rmw_result_without_an_extra_counter_reload() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for previous in [-128, 0, 1, 2] {
            let mut c = Probe::new(revision);
            let p = c.seed_player(0);
            p.sprite_state().write(&mut c, 5);
            p.height().write(&mut c, -1);
            let counter = ram::g_player_jump_counters(revision).at(0);
            counter.write(&mut c, previous);
            c.reset_log();
            super::update::player_update_death_animation(&mut c, &[p.address(), 0x10]).unwrap();
            assert_eq!(c.reads(counter.address()), 2);
        }
        let mut c = Probe::new(revision);
        let p = c.seed_player(0);
        let suppression = ram::g_player_respawn_suppression_ticks(revision).at(0);
        suppression.write(&mut c, 1);
        p.behavior_flags().write(&mut c, 0x10);
        c.reset_log();
        super::update::player_update_death_animation(&mut c, &[p.address(), 0x10]).unwrap();
        assert_eq!(c.reads(suppression.address()), 2);
        assert_eq!(p.behavior_flags().read(&c) & 0x10, 0);
        assert_eq!(c.reads(p.sprite_state().address()), 2);
    }
}

#[test]
fn kill_keeps_distinct_cmpi_reads_before_the_height_store() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (state, count) in [(3, 1), (7, 2), (0, 2)] {
            let mut c = Probe::new(revision);
            let p = c.seed_player(0);
            p.sprite_state().write(&mut c, state);
            c.reset_log();
            super::state::kill_the_player(&mut c, &[0]).unwrap();
            let count_actual = c
                .accesses
                .borrow()
                .iter()
                .take_while(
                    |a| !matches!(a,Access::Write(address,2,_) if *address==p.height().address()),
                )
                .filter(
                    |a| matches!(a,Access::Read(address,1) if *address==p.sprite_state().address()),
                )
                .count();
            assert_eq!(count_actual, count);
            for address in [
                p.animation_motion_axes().address(),
                ram::g_player_jump_counters(revision).at(0).address(),
                ram::g_player_terrain_sinking(revision).at(0).address(),
            ] {
                assert!(
                    c.at(address)
                        .windows(2)
                        .any(|a| a == [Access::Discard(address, 1), Access::Write(address, 1, 0)])
                );
            }
        }
    }
}

#[test]
fn rng_index_reload_and_cache_low_byte_are_not_merged() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for previous in [57, 58, u16::MAX] {
            let mut c = Probe::new(revision);
            let index = ram::g_random_cache_index(revision);
            index.write(&mut c, previous);
            let next = if previous == 58 {
                0
            } else {
                previous.wrapping_add(1)
            };
            let cache = ram::g_cached_random_values(revision).at(i32::from(next));
            cache.write(&mut c, 0xab01);
            c.reset_log();
            assert_eq!(super::falling::sinking_help_random_bit(&mut c), 1);
            let mut expected = vec![Access::Read(index.address(), 2)];
            if previous != 58 {
                expected.push(Access::Read(index.address(), 2));
            }
            expected.push(Access::Write(index.address(), 2, u32::from(next)));
            if previous != 58 {
                expected.push(Access::Read(index.address(), 2));
            }
            expected.push(Access::Read(cache.address() + 1, 1));
            assert_eq!(*c.accesses.borrow(), expected);
        }
    }
}

#[test]
fn transient_effect_arguments_read_level_height_y_x_before_the_call() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Probe::new(revision);
        let p = c.seed_player(0);
        c.rust_callees = false;
        c.reset_log();
        assert!(
            matches!(super::levels::player_advance_to_next_level(&mut c,&[p.address(),0]),Err(Error::Untranslated(id)) if id==ids::PRESENT_START_TRANSIENT_EFFECT)
        );
        assert_eq!(
            *c.accesses.borrow(),
            vec![
                Access::Read(p.level().address(), 1),
                Access::Read(p.height().address(), 2),
                Access::Read(p.y().address(), 2),
                Access::Read(p.x().address(), 2)
            ]
        );
        assert_eq!(c.calls[0].1, vec![0, 512, 101, 9, 1]);
    }
}

#[test]
fn relocate_compares_global_and_partner_before_player_level() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Probe::new(revision);
        let p = c.seed_player(0);
        c.rust_callees = false;
        ram::g_player_lives(revision).at(1).write(&mut c, -1);
        ram::g_highest_level(revision).write(&mut c, 2);
        c.reset_log();
        assert!(
            matches!(super::levels::player_relocate_to_partner_or_next_level(&mut c,&[0]),Err(Error::Untranslated(id)) if id==ids::PLAYER_ADVANCE_TO_NEXT_LEVEL)
        );
        let highest = ram::g_highest_level(revision).address();
        let actual:Vec<_>=c.accesses.borrow().iter().filter(|a|matches!(a,Access::Read(address,1) if *address==highest || *address==p.level().address())).cloned().collect();
        assert_eq!(
            actual,
            vec![
                Access::Read(highest, 1),
                Access::Read(p.level().address(), 1)
            ]
        );
        let mut c = Probe::new(revision);
        let p = c.seed_player(0);
        let other = c.seed_player(1);
        c.rust_callees = false;
        ram::g_split_screen_enabled(revision).write(&mut c, 1);
        other.level().write(&mut c, 2);
        c.reset_log();
        assert!(
            matches!(super::levels::player_relocate_to_partner_or_next_level(&mut c,&[0]),Err(Error::Untranslated(id)) if id==ids::EARTHLING_RELEASE_LEVEL_BANK)
        );
        let actual:Vec<_>=c.accesses.borrow().iter().filter(|a|matches!(a,Access::Read(address,1) if *address==other.level().address() || *address==p.level().address())).cloned().collect();
        assert_eq!(
            &actual[..2],
            &[
                Access::Read(other.level().address(), 1),
                Access::Read(p.level().address(), 1)
            ]
        );
    }
}

#[test]
fn process_players_keeps_the_unused_final_player_two_terrain_read() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Probe::new(revision);
        for number in 0..2 {
            c.seed_player(number);
            ram::g_player_inventory_open(revision)
                .at(number)
                .write(&mut c, 1);
        }
        c.reset_log();
        super::update::process_players(&mut c, &[]).unwrap();
        assert_eq!(
            c.reads(ram::g_player1_terrain_underfoot(revision).address()),
            1
        );
        assert_eq!(
            c.reads(ram::g_player2_terrain_underfoot(revision).address()),
            2
        );
    }
}

#[test]
fn clr_helpers_retain_discarded_width_and_keep_extend() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Probe::new(revision);
        c.set_status_register(0x271f);
        super::frames::clear_byte(&mut c, 0xff9000).unwrap();
        super::frames::clear_word(&mut c, 0xff9002).unwrap();
        assert_eq!(
            *c.accesses.borrow(),
            vec![
                Access::Discard(0xff9000, 1),
                Access::Write(0xff9000, 1, 0),
                Access::Discard(0xff9002, 2),
                Access::Write(0xff9002, 2, 0)
            ]
        );
        assert_eq!(c.status_register(), 0x2714);
    }
}

#[test]
fn start_fall_delegates_every_poll_to_the_exact_word_zero_predicate() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Probe::new(revision);
        let p = c.seed_player(0);
        c.reset_log();
        assert!(matches!(
            super::falling::player_start_falling(&mut c, &[p.address(), 0]),
            Err(Error::Platform(_))
        ));
        let pending = ram::g_terrain_priority_update_pending(revision).address();
        assert_eq!(c.at(pending), vec![Access::Write(pending, 2, 1)]);
        assert_eq!(
            c.waits,
            vec![WaitRequest::ram_word_equal(pending, u16::MAX, 0)]
        );
        assert_eq!(c.reads(ram::g_frame_counter(revision).address()), 0);
    }
}

#[test]
fn make_fall_level_comparison_reads_player_two_before_player_one() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Probe::new(revision);
        let first = c.seed_player(0);
        let second = c.seed_player(1);
        for (left, right) in [(-1, -1), (0, 1), (1, 0), (127, -128)] {
            first.level().write(&mut c, left);
            second.level().write(&mut c, right);
            c.reset_log();
            assert_eq!(
                super::levels::players_share_level(&c, first, second),
                left == right
            );
            assert_eq!(
                *c.accesses.borrow(),
                vec![
                    Access::Read(second.level().address(), 1),
                    Access::Read(first.level().address(), 1)
                ]
            );
        }
    }
}

#[test]
fn sinking_cancellation_keeps_the_real_clr_reads_not_semantic_reloads() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Probe::new(revision);
        let p = c.seed_player(0);
        p.height().write(&mut c, -3);
        ram::g_player_terrain_sinking(revision)
            .at(0)
            .write(&mut c, 1);
        c.reset_log();
        super::falling::player_update_terrain_sinking(&mut c, &[p.address(), 0x10]).unwrap();
        let h = p.height().address();
        assert_eq!(
            c.at(h),
            vec![
                Access::Read(h, 2),
                Access::Discard(h, 2),
                Access::Write(h, 2, 0),
                Access::Read(h, 2)
            ]
        );
        for address in [
            ram::g_player_terrain_sinking(revision).at(0).address(),
            p.sprite_ready().address(),
        ] {
            assert!(
                c.at(address)
                    .windows(2)
                    .any(|a| a == [Access::Discard(address, 1), Access::Write(address, 1, 0)])
            );
        }
    }
}

#[test]
fn falling_partner_guard_and_query_fields_keep_the_compiled_order() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Probe::new(revision);
        let player = c.seed_player(0);
        let other = c.seed_player(1);
        ram::g_player_lives(revision).at(1).write(&mut c, -1);
        ram::g_player_fall_states(revision).at(0).write(&mut c, 2);
        ram::g_player_jump_counters(revision).at(0).write(&mut c, 0);
        other.sprite_state().write(&mut c, 76);
        other.x().write(&mut c, 2000);
        c.reset_log();
        super::falling_check::check_player_falling(&mut c, &[]).unwrap();
        let state = other.sprite_state().address();
        let mask = other.animation_group_mask().address();
        let actual:Vec<_>=c.accesses.borrow().iter().filter(|a|matches!(a,Access::Read(address,_) if *address==state || *address==mask || *address==mask+2)).take(5).cloned().collect();
        assert_eq!(
            actual,
            vec![
                Access::Read(state, 1),
                Access::Read(state, 1),
                Access::Read(mask, 2),
                Access::Read(mask + 2, 2),
                Access::Read(state, 1)
            ]
        );
        let actual:Vec<_>=c.accesses.borrow().iter().filter(|a|matches!(a,Access::Read(address,_) if *address==player.level().address() || *address==player.y().address() || *address==player.x().address())).cloned().collect();
        assert_eq!(
            actual,
            vec![
                Access::Read(player.level().address(), 1),
                Access::Read(player.y().address(), 2),
                Access::Read(player.x().address(), 2)
            ]
        );
    }
}

#[test]
fn finalizer_publishes_its_actual_lives_base_before_terminal_callee_boundary() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Probe::new(revision);
        let p = c.seed_player(0);
        ram::g_player_lives(revision).at(1).write(&mut c, -1);
        c.reset_log();
        assert!(
            matches!(super::finalize::player_finalize_last_life_exit(&mut c,&[p.address(),0]),Err(Error::Untranslated(id)) if id==ids::SHOW_GAME_OVER_AND_RESET)
        );
        assert!(
            c.address_writes
                .contains(&(0, ram::g_player_lives(revision).address()))
        );
    }
}

#[test]
fn start_fall_wait_owns_busy_polls_and_the_only_terminal_zero_read() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for values in [vec![0], vec![0x0100, 2, 0]] {
            let mut c = Probe::new(revision);
            let p = c.seed_player(0);
            let pending = ram::g_terrain_priority_update_pending(revision).address();
            c.reset_log();
            c.polls.replace(Some(PollSamples {
                address: pending,
                width: 2,
                values: values.clone().into(),
            }));
            assert!(matches!(
                super::falling::player_start_falling(&mut c, &[p.address(), 0]),
                Err(Error::Untranslated(id)) if id == ids::GENERATE_SPEECH
            ));
            assert_eq!(
                c.waits,
                vec![WaitRequest::ram_word_equal(pending, u16::MAX, 0)]
            );
            assert!(c.polls.borrow().as_ref().unwrap().values.is_empty());
            let mut expected = vec![Access::Write(pending, 2, 1)];
            expected.extend(values.iter().map(|_| Access::Read(pending, 2)));
            assert_eq!(c.at(pending), expected);
            assert_eq!(c.reads(ram::g_frame_counter(revision).address()), 0);
        }
    }
}

#[test]
fn transient_effect_loop_keeps_calls_and_yields_before_each_next_byte_poll() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Probe::new(revision);
        let player = c.seed_player(0);
        player.behavior_flags().write(&mut c, 1);
        let effect = c.seed_player(1);
        effect.draw_flags().write(&mut c, 2);
        c.rust_callees = false;
        c.reset_log();
        let handle = ram::g_player_transient_effect_entities(revision)
            .at(0)
            .address();
        c.polls.replace(Some(PollSamples {
            address: handle,
            width: 1,
            values: vec![1, 1, 1, 1, 0].into(),
        }));
        let boundary = |function, arguments, result| CalleeBoundary {
            function,
            arguments,
            result,
        };
        c.boundaries = Some(VecDeque::from([
            boundary(
                ids::PRESENT_START_TRANSIENT_EFFECT,
                vec![0, 512, 101, 9, 1],
                Some(0),
            ),
            boundary(ids::UPDATE_ENTITY_VIEWPORT_SPRITES, vec![1], Some(0)),
            boundary(ids::PRESENT_UPDATE_TRANSIENT_EFFECT, vec![0], Some(0)),
            boundary(ids::WAIT_FOR_NEXT_V_BLANK, vec![2], Some(0)),
            boundary(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, vec![], Some(0)),
            boundary(ids::PRESENT_UPDATE_TRANSIENT_EFFECT, vec![0], Some(1)),
            boundary(ids::WAIT_FOR_NEXT_V_BLANK, vec![2], Some(0)),
            boundary(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, vec![], Some(0)),
            boundary(ids::EARTHLING_RELEASE_LEVEL_BANK, vec![1], None),
        ]));
        assert!(matches!(
            super::levels::player_advance_to_next_level(&mut c, &[player.address(), 0]),
            Err(Error::Untranslated(id)) if id == ids::EARTHLING_RELEASE_LEVEL_BANK
        ));
        let mut expected = vec![
            SpinStep::Call(ids::PRESENT_START_TRANSIENT_EFFECT, vec![0, 512, 101, 9, 1]),
            SpinStep::Poll(handle, 1, 1),
            SpinStep::Poll(handle, 1, 1),
            SpinStep::Call(ids::UPDATE_ENTITY_VIEWPORT_SPRITES, vec![1]),
        ];
        for _ in 0..2 {
            expected.extend([
                SpinStep::Poll(handle, 1, 1),
                SpinStep::Call(ids::PRESENT_UPDATE_TRANSIENT_EFFECT, vec![0]),
                SpinStep::Call(ids::WAIT_FOR_NEXT_V_BLANK, vec![2]),
                SpinStep::Call(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, vec![]),
                SpinStep::Yield,
            ]);
        }
        expected.extend([
            SpinStep::Poll(handle, 1, 0),
            SpinStep::Call(ids::EARTHLING_RELEASE_LEVEL_BANK, vec![1]),
        ]);
        assert_eq!(*c.spin_steps.borrow(), expected);
        assert!(c.boundaries.as_ref().unwrap().is_empty());
        assert!(c.polls.borrow().as_ref().unwrap().values.is_empty());
        assert_eq!(c.reads(ram::g_frame_counter(revision).address()), 0);
        assert_eq!(
            c.at(player.behavior_flags().address())
                .into_iter()
                .filter(|a| matches!(a, Access::Write(..)))
                .collect::<Vec<_>>(),
            vec![
                Access::Write(player.behavior_flags().address(), 1, 0x41),
                Access::Write(player.behavior_flags().address(), 1, 1)
            ]
        );
    }
}
