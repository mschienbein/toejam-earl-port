//! Authored access-order regressions, independent of cartridge bytes.
use super::{dispatch, effects, hazard_frames, telephone};
use crate::{Context, Dispatcher, FunctionId, Result, call_frame::FrameState, execution, ids};
use std::cell::RefCell;
use tje_mem::{AddressSpace, Bus, Revision, ram};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Access {
    Read(u32, u8),
    Discard(u32, u8),
    Write(u32, u8, u32),
    Call(FunctionId, Vec<u32>),
}

struct Machine {
    memory: AddressSpace,
    accesses: RefCell<Vec<Access>>,
    data: [u32; 8],
    address: [u32; 8],
    status: u16,
    frame: FrameState,
    collisions_left: usize,
    active_sprite: i8,
}
impl Machine {
    fn new(revision: Revision) -> Self {
        Self {
            memory: AddressSpace::new(vec![0; 0x100000], revision),
            accesses: RefCell::new(Vec::new()),
            data: [0; 8],
            address: [0; 8],
            status: 0x2000,
            frame: FrameState::new(0xff7000),
            collisions_left: 0,
            active_sprite: -1,
        }
    }
    fn begin(&self) {
        self.accesses.borrow_mut().clear();
    }
    fn reads_at(&self, address: u32) -> Vec<Access> {
        self.accesses
            .borrow()
            .iter()
            .filter(|access| matches!(access, Access::Read(actual, _) if *actual == address))
            .cloned()
            .collect()
    }
    fn original_effects(&mut self, id: FunctionId, args: &[u32]) -> Result<()> {
        self.accesses
            .borrow_mut()
            .push(Access::Call(id, args.to_vec()));
        {
            let value = match id {
                ids::GET_ACTIVE_PRESENT_SPRITE => i32::from(self.active_sprite) as u32,
                ids::PRESENT_UPDATE_TRANSIENT_EFFECT => 1,
                ids::ALLOCATE_ENTITY_WITH_EVICTION => 6,
                ids::ABS => (args[0] as i32).wrapping_abs() as u32,
                ids::GET_RANDOM_NUMBER => 65535,
                ids::ENTITY_IS_WITHIN_RANGE => 1,
                ids::PLAYER_YOUCH => {
                    let source = tje_mem::Address::<tje_mem::EarthlingState>::new(args[1]);
                    assert_eq!(source.kind().read(self), 1);
                    assert_eq!(source.object().read(self), 6);
                    assert_eq!(source.x().read(self), 400);
                    assert_eq!(source.y().read(self), 400);
                    assert_eq!(
                        self.read_u8(source.address() + 2),
                        0xa5,
                        "unwritten source bytes must not be initialized"
                    );
                    0
                }
                ids::TELEPHONE_START_RING => {
                    ram::g_telephone_time_to_ring(self.revision())
                        .at(args[0] as i32)
                        .write(self, 60);
                    0
                }
                ids::ENTITY_FIND_NARROW_COLLISION_AT_POSITION => {
                    let collision = self.collisions_left != 0;
                    self.collisions_left = self.collisions_left.saturating_sub(1);
                    u32::from(collision)
                }
                ids::STOP_PLAYER
                | ids::HUD_LOAD_ACTION_ICON_SPRITE
                | ids::SETUP_SELECTED_PRESENT
                | ids::SET_TOMATO_RAIN_TOMATOES
                | ids::GENERATE_SPEECH
                | ids::SET_SPRITE
                | ids::SET_POSITION_ON_NEW_LEVEL
                | ids::PRESENT_FIND_DOORWAY_OWNER_AT_TILE
                | ids::PLAY_SOUND
                | ids::PLAY_SOUND_EFFECT
                | ids::START_DIALOGUE
                | ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT
                | ids::FREE_ENTITY
                | ids::DRAW_GAME_MAP
                | ids::PRESENT_START_TRANSIENT_EFFECT
                | ids::PRESENT_UPDATE_DECOY
                | ids::PRESENT_UPDATE_BOOM_BOX
                | ids::PRESENT_UPDATE_DOORWAY
                | ids::PRESENT_UPDATE_BURPS
                | ids::PRESENT_UPDATE_RAIN_CLOUD
                | ids::ENTITY_RELEASE_FINISHED_TIMED_EFFECT
                | ids::CHECK_FOR_TELEPHONE
                | ids::CHECK_TOMATO_RAIN_TIMER
                | ids::EARTHLING_RESET_STATE_TWO_BY_DRAW_MASK => 0,
                _ => panic!("unexpected isolated callee {}", id.metadata().name),
            };
            if id.metadata().abi.returns_value {
                self.set_data_register(0, value);
            }
            Ok(())
        }
    }
}
impl Bus for Machine {
    fn read_u8(&self, address: u32) -> u8 {
        self.accesses.borrow_mut().push(Access::Read(address, 1));
        self.memory.read_u8(address)
    }
    fn read_u16(&self, address: u32) -> u16 {
        self.accesses.borrow_mut().push(Access::Read(address, 2));
        self.memory.read_u16(address)
    }
    fn read_u32(&self, address: u32) -> u32 {
        self.accesses.borrow_mut().push(Access::Read(address, 4));
        self.memory.read_u32(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 1, u32::from(value)));
        self.memory.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 2, u32::from(value)));
        self.memory.write_u16(address, value);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 4, value));
        self.memory.write_u32(address, value);
    }
}
impl tje_mem::Platform for Machine {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("unexpected VDP control write")
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("unexpected VDP data write")
    }
}
impl crate::Platform for Machine {
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.accesses.borrow_mut().push(Access::Discard(address, 1));
        let _ = self.memory.read_u8(address);
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.accesses.borrow_mut().push(Access::Discard(address, 2));
        let _ = self.memory.read_u16(address);
        Ok(())
    }
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("unexpected device read")
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("unexpected device read")
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("unexpected device write")
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("unexpected device write")
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
        Ok(self.frame.entry_sp)
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.frame.entry_sp = value;
        Ok(())
    }
    fn frame_state(&self) -> Option<FrameState> {
        Some(self.frame)
    }
    fn set_frame_state(&mut self, frame: FrameState) -> Result<()> {
        self.frame = frame;
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with(&[])
    }
    fn execution_class(&self) -> execution::ExecutionClass {
        execution::ExecutionClass::TimeAware
    }
    fn enter_call_ceiling(&mut self, _: FunctionId) -> Result<execution::CallCeilingScope> {
        Ok(execution::CallCeilingScope::not_required())
    }
    fn call_original_with_execution(
        &mut self,
        id: FunctionId,
        args: &[u32],
        _: execution::ExecutionClass,
    ) -> Result<u32> {
        self.call_original(id, args)
    }
    fn call_original_entry(&mut self, id: FunctionId, _: execution::ExecutionClass) -> Result<u32> {
        let sp = self.stack_pointer()?;
        let args: Vec<_> = (0..u32::from(id.metadata().abi.argument_count))
            .map(|index| self.read_u32(sp + 4 + 4 * index))
            .collect();
        let result = self.call_original(id, &args)?;
        self.set_stack_pointer(sp + 4)?;
        Ok(result)
    }
    fn call_original_entry_void(
        &mut self,
        id: FunctionId,
        _: execution::ExecutionClass,
    ) -> Result<()> {
        let sp = self.stack_pointer()?;
        let args: Vec<_> = (0..u32::from(id.metadata().abi.argument_count))
            .map(|index| self.read_u32(sp + 4 + 4 * index))
            .collect();
        self.original_effects(id, &args)?;
        self.set_stack_pointer(sp + 4)?;
        Ok(())
    }
    fn call_original(&mut self, id: FunctionId, args: &[u32]) -> Result<u32> {
        self.original_effects(id, args)?;
        Ok(self.data_register(0))
    }
    fn call_original_void_with_execution(
        &mut self,
        id: FunctionId,
        args: &[u32],
        _class: crate::execution::ExecutionClass,
    ) -> Result<()> {
        self.original_effects(id, args)
    }
}

#[test]
fn dispatch_keeps_compiled_short_circuit_reads_and_btst_width() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        let player = ram::g_player_info(revision).at(0);
        player.sprite_state().write(&mut c, 6);
        c.begin();
        assert_eq!(
            dispatch::open_selected_present(&mut c, &[0, 8]).unwrap(),
            Some(1)
        );
        assert_eq!(
            c.reads_at(player.sprite_state().address()),
            vec![Access::Read(player.sprite_state().address(), 1); 2]
        );

        // Non-powerup presents return without reading the animation mask.
        c.begin();
        dispatch::setup_selected_present(&mut c, &[player.address(), 0, 8, 0x10]).unwrap();
        assert!(
            c.reads_at(player.animation_group_mask().address())
                .is_empty()
        );

        player.height().write(&mut c, 1);
        player.animation_group_mask().write(&mut c, 0x20);
        ram::g_player_powerups(revision).at(0).write(&mut c, 1);
        c.begin();
        dispatch::setup_selected_present(&mut c, &[player.address(), 0, 2, 0x10]).unwrap();
        assert!(
            c.reads_at(player.animation_group_mask().address())
                .is_empty()
        );
        assert_eq!(
            c.reads_at(player.animation_group_mask().address() + 3),
            vec![Access::Read(player.animation_group_mask().address() + 3, 1)]
        );

        player.height().write(&mut c, 0);
        player.sprite_state().write(&mut c, 76);
        player.animation_group_mask().write(&mut c, 0);
        c.begin();
        assert_eq!(
            dispatch::open_selected_present(&mut c, &[0, 22]).unwrap(),
            Some(0)
        );
        let predicate: Vec<_> = c.accesses.borrow().iter().filter(|a| matches!(a, Access::Read(address, _) if *address == player.sprite_state().address() || *address == player.animation_group_mask().address())).cloned().collect();
        assert_eq!(
            predicate,
            vec![
                Access::Read(player.sprite_state().address(), 1),
                Access::Read(player.sprite_state().address(), 1),
                Access::Read(player.sprite_state().address(), 1),
                Access::Read(player.sprite_state().address(), 1),
                Access::Read(player.animation_group_mask().address(), 4),
                Access::Read(player.sprite_state().address(), 1)
            ]
        );
    }
}

#[test]
fn rank_display_rereads_incremented_index_and_tests_only_low_cache_byte() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for index in [0, 57, 58, u16::MAX] {
            let mut c = Machine::new(revision);
            let player = ram::g_player_info(revision).at(0);
            let display = ram::g_player_info(revision).at(6);
            ram::g_player_promotion_display_entities(revision)
                .at(0)
                .write(&mut c, 6);
            display.sprite_state().write(&mut c, 11);
            display.frame_duration().write(&mut c, 1);
            let cache_index = ram::g_random_cache_index(revision);
            cache_index.write(&mut c, index);
            let next = if index == 58 {
                0
            } else {
                index.wrapping_add(1)
            };
            let cached = ram::g_cached_random_values(revision).at(i32::from(next));
            cached.write(&mut c, 0xff00);
            c.begin();
            effects::bummer_life_promo(&mut c, &[player.address(), 0x10]).unwrap();
            assert_eq!(
                c.reads_at(cache_index.address()).len(),
                if index == 58 { 1 } else { 3 }
            );
            assert!(c.reads_at(cached.address()).is_empty());
            assert_eq!(
                c.reads_at(cached.address() + 1),
                vec![Access::Read(cached.address() + 1, 1)]
            );
            assert_eq!(cache_index.read(&c), next);
        }
    }
}

#[test]
fn teleport_reads_primary_camera_first_and_collision_coordinates_right_to_left() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        let player = ram::g_player_info(revision).at(1);
        player.x().write(&mut c, 200);
        player.y().write(&mut c, 100);
        ram::g_camera2_pending_x(revision).write(&mut c, 0);
        ram::g_camera2_pending_y(revision).write(&mut c, 0);
        c.collisions_left = 1;
        c.begin();
        effects::player_teleport_through_doorway(&mut c, &[1, player.address()]).unwrap();
        let accesses = c.accesses.borrow();
        assert_eq!(
            &accesses[..3],
            &[
                Access::Read(ram::g_camera1_pending_x(revision).address(), 2),
                Access::Read(ram::g_camera1_pending_y(revision).address(), 2),
                Access::Read(ram::g_player_lives(revision).address(), 1)
            ]
        );
        let first_call = accesses
            .iter()
            .position(|a| matches!(a, Access::Call(id, _) if *id == ids::SET_POSITION_ON_NEW_LEVEL))
            .unwrap();
        assert_eq!(
            &accesses[first_call + 1..first_call + 4],
            &[
                Access::Read(player.y().address(), 2),
                Access::Read(player.x().address(), 2),
                Access::Call(
                    ids::ENTITY_FIND_NARROW_COLLISION_AT_POSITION,
                    vec![player.address(), 200, 100, 2]
                )
            ]
        );
        assert_eq!(
            accesses
                .iter()
                .filter(
                    |a| matches!(a, Access::Call(id, _) if *id == ids::SET_POSITION_ON_NEW_LEVEL)
                )
                .count(),
            2
        );
    }
}

#[test]
fn doorway_tile_arguments_read_column_before_row() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        let doorway = ram::g_player_info(revision).at(6);
        doorway.draw_flags().write(&mut c, 6);
        ram::g_player_doorway_entities(revision)
            .at(0)
            .write(&mut c, 6);
        c.begin();
        effects::present_update_doorway(&mut c, &[0]).unwrap();
        let accesses = c.accesses.borrow();
        for index in 0..2 {
            let player = ram::g_player_info(revision).at(index);
            let call = accesses.iter().position(|a| matches!(a, Access::Call(id, args) if *id == ids::PRESENT_FIND_DOORWAY_OWNER_AT_TILE && args[0] == player.address())).unwrap();
            assert_eq!(
                &accesses[call - 2..call],
                &[
                    Access::Read(player.x().address(), 2),
                    Access::Read(player.y().address(), 2)
                ]
            );
        }
    }
}

#[test]
fn promotion_and_terrain_keep_each_compiled_state_comparison() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        let player = ram::g_player_info(revision).at(0);
        player.sprite_state().write(&mut c, 77);
        c.begin();
        effects::perform_promotion(&mut c, &[0, 17, 0x10]).unwrap();
        assert_eq!(c.reads_at(player.sprite_state().address()).len(), 5);

        ram::g_player_powerups(revision).at(0).write(&mut c, 6);
        player.sprite_state().write(&mut c, 6);
        c.begin();
        hazard_frames::present_update_terrain_effects(&mut c, &[player.address(), 0x10]).unwrap();
        assert_eq!(c.reads_at(player.sprite_state().address()).len(), 2);
    }
}

fn assert_clear(accesses: &[Access], address: u32, width: u8) {
    let store = accesses
        .iter()
        .position(|a| *a == Access::Write(address, width, 0))
        .unwrap();
    assert_eq!(accesses[store - 1], Access::Discard(address, width));
}

#[test]
fn cleared_game_storage_keeps_discarded_68000_pre_reads() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        let player = ram::g_player_info(revision).at(0);
        c.active_sprite = 0;
        c.begin();
        dispatch::setup_selected_present(&mut c, &[player.address(), 0, 2, 0x10]).unwrap();
        assert_clear(&c.accesses.borrow(), player.velocity_x().address(), 2);
        assert_clear(&c.accesses.borrow(), player.velocity_y().address(), 2);

        ram::g_player_promotion_display_entities(revision)
            .at(0)
            .write(&mut c, 6);
        c.begin();
        effects::perform_promotion(&mut c, &[0, 16, 0x10]).unwrap();
        assert_clear(
            &c.accesses.borrow(),
            ram::g_player_health(revision).address(),
            1,
        );

        let display = ram::g_player_info(revision).at(6);
        display.sprite_state().write(&mut c, 16);
        display.frame_duration().write(&mut c, 0);
        c.begin();
        effects::bummer_life_promo(&mut c, &[player.address(), 0x10]).unwrap();
        assert_clear(
            &c.accesses.borrow(),
            ram::g_player_health(revision).address(),
            1,
        );
        assert_clear(
            &c.accesses.borrow(),
            ram::g_player_promotion_display_entities(revision).address(),
            1,
        );

        ram::g_player_doorway_entities(revision)
            .at(0)
            .write(&mut c, 6);
        display.draw_flags().write(&mut c, 0);
        c.begin();
        effects::present_update_doorway(&mut c, &[0]).unwrap();
        assert_clear(
            &c.accesses.borrow(),
            ram::g_player_doorway_entities(revision).address(),
            1,
        );

        ram::g_player_one_shot_terrain_effect_entities(revision)
            .at(0)
            .write(&mut c, 6);
        c.begin();
        hazard_frames::present_update_terrain_effects(&mut c, &[player.address(), 0x10]).unwrap();
        assert_clear(
            &c.accesses.borrow(),
            ram::g_player_one_shot_terrain_effect_entities(revision).address(),
            1,
        );
    }
}

#[test]
fn telephone_preserves_operand_order_abs_calls_and_answer_clear_reads() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        let phone = ram::g_telephone_states(revision).at(0);
        let player = ram::g_player_info(revision).at(0);
        phone.level().write(&mut c, 1);
        phone.object().write(&mut c, 6);
        phone.x().write(&mut c, 100);
        phone.y().write(&mut c, 100);
        player.level().write(&mut c, 1);
        player.x().write(&mut c, 105);
        player.y().write(&mut c, 98);
        ram::g_player1_map_level(revision).write(&mut c, 1);
        ram::g_telephone_time_to_ring(revision)
            .at(0)
            .write(&mut c, 60);
        c.begin();
        telephone::check_for_telephone(&mut c, &[0]).unwrap();
        let accesses = c.accesses.borrow();
        let map_read = accesses
            .iter()
            .position(|a| *a == Access::Read(ram::g_player1_map_level(revision).address(), 1))
            .unwrap();
        assert_eq!(
            accesses[map_read + 1],
            Access::Read(phone.level().address(), 1)
        );
        let calls: Vec<_> = accesses
            .iter()
            .filter_map(|a| {
                if let Access::Call(id, args) = a {
                    Some((*id, args.clone()))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            calls,
            vec![
                (ids::ABS, vec![(-5_i32) as u32]),
                (ids::ABS, vec![2]),
                (ids::TELEPHONE_START_RING, vec![0, phone.address()]),
                (ids::ABS, vec![(-5_i32) as u32]),
                (ids::ABS, vec![2]),
                (ids::FREE_ENTITY, vec![6]),
                (ids::DRAW_GAME_MAP, vec![0]),
            ]
        );
        assert_clear(&accesses, phone.object().address(), 1);
        assert_clear(&accesses, ram::g_map_reveal_mode(revision).address(), 2);
        assert_clear(&accesses, phone.level().address(), 1);
    }
}

#[test]
fn process_presents_keeps_all_nine_callees_for_each_player() {
    let expected_ids = [
        ids::PRESENT_UPDATE_DECOY,
        ids::PRESENT_UPDATE_BOOM_BOX,
        ids::PRESENT_UPDATE_DOORWAY,
        ids::PRESENT_UPDATE_BURPS,
        ids::PRESENT_UPDATE_RAIN_CLOUD,
        ids::PRESENT_UPDATE_TRANSIENT_EFFECT,
        ids::ENTITY_RELEASE_FINISHED_TIMED_EFFECT,
        ids::CHECK_FOR_TELEPHONE,
        ids::CHECK_TOMATO_RAIN_TIMER,
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        dispatch::process_presents(&mut c, &[]).unwrap();
        let expected: Vec<_> = (0..2)
            .flat_map(|player| expected_ids.map(|id| Access::Call(id, vec![player])))
            .collect();
        assert_eq!(*c.accesses.borrow(), expected);
    }
}

#[test]
fn burp_start_and_expiry_preserve_call_and_store_order() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        let timer = ram::g_player_burp_timers(revision).at(0);
        ram::g_player_burps_left(revision).at(0).write(&mut c, 2);
        timer.write(&mut c, -1);
        c.begin();
        effects::present_update_burps(&mut c, &[0]).unwrap();
        assert_eq!(
            &c.accesses.borrow()[2..],
            &[
                Access::Call(ids::START_DIALOGUE, vec![24, 0]),
                Access::Call(ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, vec![14]),
                Access::Write(timer.address(), 1, 50),
            ]
        );
        timer.write(&mut c, 0);
        c.begin();
        effects::present_update_burps(&mut c, &[0]).unwrap();
        let accesses = c.accesses.borrow();
        let calls: Vec<_> = accesses
            .iter()
            .filter_map(|a| {
                if let Access::Call(id, _) = a {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            calls,
            vec![
                ids::EARTHLING_RESET_STATE_TWO_BY_DRAW_MASK,
                ids::GENERATE_SPEECH,
                ids::GET_RANDOM_NUMBER,
                ids::PLAY_SOUND_EFFECT
            ]
        );
        let delay = (20 + 65535 % 81) as u32;
        let timer_store = accesses
            .iter()
            .position(|a| *a == Access::Write(timer.address(), 1, delay))
            .unwrap();
        assert_eq!(
            accesses[timer_store + 1],
            Access::Read(ram::g_pcm_effect_cooldown_ticks(revision).address(), 2)
        );
    }
}

#[test]
fn grown_terrain_effect_writes_only_source_fields_before_both_hurt_calls() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        for address in 0xff6fe8..0xff6ffc {
            c.memory.write_u8(address, 0xa5);
        }
        let player = ram::g_player_info(revision).at(0);
        let effect = ram::g_player_info(revision).at(6);
        effect.x().write(&mut c, 400);
        effect.y().write(&mut c, 400);
        effect.draw_flags().write(&mut c, 6);
        ram::g_player_powerups(revision).at(0).write(&mut c, 6);
        ram::g_player_active_terrain_effect_entities(revision)
            .at(0)
            .write(&mut c, 6);
        c.begin();
        hazard_frames::present_update_terrain_effects(&mut c, &[player.address(), 0x10]).unwrap();
        let hurt_players: Vec<_> = c
            .accesses
            .borrow()
            .iter()
            .filter_map(|a| {
                if let Access::Call(id, args) = a
                    && *id == ids::PLAYER_YOUCH
                {
                    Some(args[0])
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(hurt_players, vec![0, 1]);
        assert_eq!(c.stack_pointer().unwrap(), 0xff7000);
    }
}

#[test]
fn promotion_fallback_preserves_original_pointer_as_speaker_bug() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        ram::g_player_promotion_display_entities(revision)
            .at(1)
            .write(&mut c, 6);
        ram::g_player_rank(revision).at(1).write(&mut c, 2);
        c.begin();
        effects::perform_promotion(&mut c, &[1, 11, 0x10]).unwrap();
        let speech = c
            .accesses
            .borrow()
            .iter()
            .find_map(|a| {
                if let Access::Call(id, args) = a
                    && *id == ids::GENERATE_SPEECH
                {
                    Some(args.clone())
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(
            speech,
            vec![ram::g_player_info(revision).at(1).address(), 0, 1]
        );
    }
}
