//! Authored unit triggers execute the canonical faithful Rust bodies and the
//! corrected callbacks. Callee doubles are explicit; these are not ROM proofs.
use super::*;
use std::cell::RefCell;
use tje_engine_runtime::enhanced::{EnhancedInputs, Sidecars};
use tje_game::{
    Context as GameContext, Dispatcher, Platform, TranslatedEntry,
    call_frame::FrameState,
    execution::{CallCeilingScope, ExecutionClass},
};
use tje_mem::{AddressSpace, Bus};

type Callback = fn(&mut dyn Context, &mut Continuation) -> Result<OverrideStep>;

struct Fixture {
    memory: AddressSpace,
    revision: Revision,
    data: [u32; 8],
    address: [u32; 8],
    status: u16,
    frame: FrameState,
    class: ExecutionClass,
    args: Vec<u32>,
    sidecars: Sidecars,
    calls: Vec<(FunctionId, Vec<u32>)>,
    reads: RefCell<Vec<u32>>,
    limit: usize,
    validation: bool,
}
impl Fixture {
    fn new(revision: Revision, args: &[u32]) -> Self {
        let mut result = Self {
            memory: AddressSpace::new(vec![0; 0x100000], revision),
            revision,
            data: [0; 8],
            address: [0; 8],
            status: 0x2000,
            frame: FrameState::new(0xff7000),
            class: ExecutionClass::TimeAware,
            args: args.to_vec(),
            sidecars: Sidecars::default(),
            calls: vec![],
            reads: RefCell::new(vec![]),
            limit: 20000,
            validation: false,
        };
        result.address[7] = result.frame.entry_sp;
        result.memory.write_u32(result.frame.entry_sp, 0x200);
        for (index, &value) in args.iter().enumerate() {
            result
                .memory
                .write_u32(result.frame.entry_sp + 4 + index as u32 * 4, value);
        }
        result
            .sidecars
            .register(Configuration {
                fixes: super::super::ALL,
                identity: super::super::ALL.to_le_bytes(),
            })
            .unwrap();
        result
    }
    fn faithful(&mut self, id: FunctionId, args: &[u32]) -> Result<Option<u32>> {
        if let Some(entry) = tje_game::translations()
            .chain(tje_game::pending_translations())
            .chain(tje_game::replayed_translations())
            .find(|entry| entry.id == id)
        {
            (entry.function)(self, args)
        } else {
            let entry = tje_game::framed_translations()
                .chain(tje_game::pending_framed_translations())
                .chain(tje_game::replayed_framed_translations())
                .find(|entry| entry.id == id)
                .expect("canonical faithful body");
            TranslatedEntry::Framed(entry).invoke_prepared(self)
        }
    }
    fn effects(&mut self, id: FunctionId, args: &[u32]) -> Result<u32> {
        self.calls.push((id, args.to_vec()));
        if self.calls.len() > self.limit {
            return Err(tje_game::Error::Platform(
                "authored fixture call budget exhausted".into(),
            ));
        }
        let value = match id {
            ids::GET_RANDOM_NUMBER => 0,
            ids::GET_SCORE_FOR_PROMO => tje_game::ui::get_score_for_promo(self, args)?.unwrap(),
            ids::PLAYER_GET_MAXIMUM_HEALTH => {
                if args[0] == 0 {
                    23
                } else {
                    31
                }
            }
            ids::GET_MAP_TERRAIN_AT_WORLD_POSITION => 0x10,
            ids::ALLOCATE_SPRITE_SLOTS
            | ids::PLAYER_IS_NEAR_TERRAIN_EDGE
            | ids::ELEVATOR_ALLOWS_WORLD_PLACEMENT => 1,
            ids::TERRAIN_VALIDATION => {
                if self.validation {
                    self.write_i16(args[3], 3);
                    self.write_i16(args[4], 4);
                }
                u32::from(self.validation)
            }
            ids::GET_ACTIVE_PRESENT_SPRITE => 10,
            ids::HUD_ANIMATE_HEALTH_BAR => {
                self.faithful(id, args)?;
                0
            }
            _ => 0,
        };
        self.data[0] = value;
        Ok(value)
    }
    fn run_callback(&mut self, callback: Callback) -> Result<OverrideStep> {
        let mut k = Continuation::default();
        for _ in 0..64 {
            match callback(self, &mut k)? {
                OverrideStep::CallOriginal {
                    function,
                    arguments,
                } => {
                    self.effects(function, &arguments)?;
                }
                step => return Ok(step),
            }
        }
        panic!("bounded corrected continuation")
    }
}
impl Bus for Fixture {
    fn read_u8(&self, address: u32) -> u8 {
        self.reads.borrow_mut().push(address);
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
    }
}
impl tje_mem::Platform for Fixture {
    fn write_vdp_control(&mut self, _: u16) {}
    fn write_vdp_data(&mut self, _: u16) {}
}
impl Platform for Fixture {
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        let _ = self.read_u8(address);
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        let _ = self.read_u16(address);
        Ok(())
    }
    fn read_device_u8(&mut self, _: u32) -> u8 {
        0
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        0
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {}
    fn write_device_u16(&mut self, _: u32, _: u16) {}
}
impl GameContext for Fixture {
    fn read_uninitialized_local(
        &mut self,
        read: tje_game::stack_read::UninitializedLocalRead,
    ) -> Result<u32> {
        // This authored context models the one cited faithful mixed word. It
        // grants no production proof authority and never zero-fills the byte.
        assert_eq!(read.owner, ids::OPEN_EARTHLING_PRESENT);
        assert_eq!(read.offset, -10);
        assert_eq!(read.width, tje_game::stack_read::LocalWidth::Word);
        assert_eq!(read.frame.entry_sp, self.frame.entry_sp);
        assert!(!read.citation.is_empty());
        Ok(u32::from(self.read_u16(read.address())))
    }
    fn call_original_entry(&mut self, id: FunctionId, _: ExecutionClass) -> Result<u32> {
        let args: Vec<_> = (0..u32::from(id.metadata().abi.argument_count))
            .map(|i| self.memory.read_u32(self.frame.entry_sp + 4 + i * 4))
            .collect();
        self.effects(id, &args)
    }
    fn call_original_entry_void(&mut self, id: FunctionId, class: ExecutionClass) -> Result<()> {
        self.call_original_entry(id, class).map(|_| ())
    }
    fn revision(&self) -> Revision {
        self.revision
    }
    fn data_register(&self, i: usize) -> u32 {
        self.data[i]
    }
    fn set_data_register(&mut self, i: usize, value: u32) {
        self.data[i] = value;
    }
    fn address_register(&self, i: usize) -> u32 {
        self.address[i]
    }
    fn set_address_register(&mut self, i: usize, value: u32) {
        self.address[i] = value;
    }
    fn status_register(&self) -> u16 {
        self.status
    }
    fn set_status_register(&mut self, value: u16) {
        self.status = value;
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
    fn set_frame_state(&mut self, state: FrameState) -> Result<()> {
        self.frame = state;
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with(&[])
    }
    fn execution_class(&self) -> ExecutionClass {
        self.class
    }
    fn set_execution_class(&mut self, value: ExecutionClass) -> Result<()> {
        self.class = value;
        Ok(())
    }
    fn enter_call_ceiling(&mut self, _: FunctionId) -> Result<CallCeilingScope> {
        Ok(CallCeilingScope::not_required())
    }
    fn call_original(&mut self, id: FunctionId, args: &[u32]) -> Result<u32> {
        self.effects(id, args)
    }
    fn call_original_with_execution(
        &mut self,
        id: FunctionId,
        args: &[u32],
        _: ExecutionClass,
    ) -> Result<u32> {
        self.effects(id, args)
    }
    fn call_original_void_with_execution(
        &mut self,
        id: FunctionId,
        args: &[u32],
        _: ExecutionClass,
    ) -> Result<()> {
        self.effects(id, args).map(|_| ())
    }
}
impl Context for Fixture {
    fn sidecars(&self) -> &Sidecars {
        &self.sidecars
    }
    fn sidecars_mut(&mut self) -> &mut Sidecars {
        &mut self.sidecars
    }
    fn state_and_memory(&mut self) -> (&mut Sidecars, &AddressSpace) {
        (&mut self.sidecars, &self.memory)
    }
    fn inputs(&self) -> EnhancedInputs {
        EnhancedInputs::default()
    }
    fn frame_count(&self) -> u64 {
        0
    }
    fn entry_function(&self) -> Result<FunctionId> {
        tje_game::functions()
            .find(|entry| self.frame.is_prepared_for(entry.id))
            .map(|entry| entry.id)
            .ok_or_else(|| {
                tje_game::Error::Platform("fixture has no prepared entry identity".into())
            })
    }
    fn entry_argument(&self, index: usize) -> Result<u32> {
        self.args
            .get(index)
            .copied()
            .ok_or_else(|| tje_game::Error::Platform("fixture entry argument index".into()))
    }
    fn entry_arguments(&self) -> Result<Vec<u32>> {
        Ok(self.args.clone())
    }
}

#[test]
fn b017_b018_source_loops_have_bounded_fixed_outcomes() {
    for r in Revision::ALL {
        let mut faithful = Fixture::new(r, &[0]);
        faithful.limit = 40;
        ram::g_player_points(r).at(0).write(&mut faithful, 880);
        assert!(
            faithful
                .faithful(ids::PLAYER_RESET_RANK_AND_RESOURCES, &[0])
                .is_err()
        );
        assert!(
            faithful
                .calls
                .iter()
                .any(|(id, args)| *id == ids::GET_SCORE_FOR_PROMO && args[0] == 8)
        );
        let mut fixed = Fixture::new(r, &[0]);
        ram::g_player_points(r).at(0).write(&mut fixed, 880);
        resources::rank_reset(&mut fixed, &mut Continuation::default()).unwrap();
        assert_eq!(ram::g_player_rank(r).at(0).read(&fixed), 8);
        assert!(
            !fixed
                .calls
                .iter()
                .any(|(id, args)| *id == ids::GET_SCORE_FOR_PROMO && args[0] == 8)
        );
        for (lives, sprite, level) in [(-1, 0, 1), (3, 5, 1), (3, 6, 1), (3, 0, 26)] {
            let mut faithful = Fixture::new(r, &[0, 1]);
            faithful.limit = 20;
            ram::g_player_lives(r).at(0).write(&mut faithful, lives);
            ram::g_hud_displayed_lives(r)
                .at(0)
                .write(&mut faithful, lives);
            ram::g_player_info(r)
                .at(0)
                .sprite_state()
                .write(&mut faithful, sprite);
            ram::g_player1_map_level(r).write(&mut faithful, level);
            ram::g_player_health(r).at(0).write(&mut faithful, 1);
            assert!(
                faithful
                    .faithful(ids::HUD_INITIALIZE_HEALTH_BAR, &[0, 1])
                    .is_err()
            );
            faithful.calls.clear();
            faithful.address[7] = 0xff7000;
            faithful.frame = FrameState::new(0xff7000);
            assert!(matches!(
                faithful.run_callback(resources::health_bar).unwrap(),
                OverrideStep::Return(None)
            ));
            assert_eq!(ram::g_hud_displayed_health(r).at(0).read(&faithful), 0);
            assert_eq!(ram::g_player_health(r).at(0).read(&faithful), 1);
        }
    }
}

#[test]
fn b004_b005_b006_exhausted_maps_never_publish_residual_coordinates() {
    for r in Revision::ALL {
        let mut old = Fixture::new(r, &[0xff5000, 0xff5002, 3, 0]);
        old.limit = 40;
        ram::g_player1_map_level(r).write(&mut old, 3);
        for i in 0..494 {
            ram::g_player1_level_map(r)
                .cast::<u16>()
                .offset(i)
                .write(&mut old, 0x2000);
        }
        assert!(
            old.faithful(
                ids::MAP_FIND_ITEM_PLACEMENT_POSITION,
                &[0xff5000, 0xff5002, 3, 0]
            )
            .is_err()
        );
        old.calls.clear();
        old.limit = 4096;
        old.address[7] = 0xff7000;
        old.frame = FrameState::new(0xff7000);
        old.write_i16(0xff5000, 777);
        old.write_i16(0xff5002, 888);
        assert_eq!(
            placement::item_position(&mut old, &[0xff5000, 0xff5002, 3, 0]).unwrap(),
            Some(0)
        );
        assert_eq!((old.read_i16(0xff5000), old.read_i16(0xff5002)), (777, 888));
        assert_eq!(
            old.calls
                .iter()
                .filter(|(id, _)| *id == ids::GET_RANDOM_NUMBER)
                .count(),
            22
        );
        let mut old_player = Fixture::new(r, &[0]);
        ram::g_player_info(r)
            .at(0)
            .level()
            .write(&mut old_player, 3);
        ram::g_player_info(r).at(0).x().write(&mut old_player, 777);
        old_player.write_i16(0xff6fec, 500);
        old_player.write_i16(0xff6fee, 600);
        old_player
            .faithful(ids::SET_POSITION_ON_NEW_LEVEL, &[0])
            .unwrap();
        assert_eq!(ram::g_player_info(r).at(0).x().read(&old_player), 500);
        let mut player = Fixture::new(r, &[0]);
        ram::g_player_info(r).at(0).level().write(&mut player, 3);
        ram::g_player_info(r).at(0).x().write(&mut player, 777);
        assert!(placement::player_position(&mut player, &[0]).is_err());
        assert_eq!(ram::g_player_info(r).at(0).x().read(&player), 777);
        let mut items = Fixture::new(r, &[0xff5000, 3]);
        ram::g_player1_map_level(r).write(&mut items, 3);
        // The faithful LINK #-12 frame's unwritten coordinate words.
        items.write_i16(0xff6ff0, 500);
        items.write_i16(0xff6ff2, 500);
        items
            .faithful(ids::MAP_PLACE_LARGE_LEVEL_ITEMS, &[0xff5000, 3])
            .unwrap();
        let record = Address::<tje_mem::LevelItem>::new(0xff5000).offset(28);
        assert_ne!(
            record.kind().read(&items),
            -1,
            "faithful consumes unwritten placement outputs"
        );
        placement::large_items(&mut items, &[0xff5000, 3]).unwrap();
        for i in 28..32 {
            assert_eq!(
                Address::<tje_mem::LevelItem>::new(0xff5000)
                    .offset(i)
                    .kind()
                    .read(&items),
                -1
            );
        }
        ram::g_player_info(r).at(0).level().write(&mut items, 3);
        let phone = ram::g_telephone_states(r).at(0);
        phone.x().write(&mut items, 777);
        assert_eq!(placement::telephone(&mut items, &[0]).unwrap(), Some(0));
        assert_eq!(phone.x().read(&items), 777);
        let mut bank = Fixture::new(r, &[3]);
        ram::g_player1_map_level(r).write(&mut bank, 3);
        placement::load_bank(&mut bank, &[3]).unwrap();
        for slot in 0..17 {
            assert_eq!(ram::g_object_table(r).at(0).at(slot).kind().read(&bank), -1);
        }
        assert_eq!(ram::g_item_bank_users(r).at(0).read(&bank), 1);
        assert_eq!(bank.calls.last().unwrap().0, ids::STACK_BASED_RANDOM);
    }
}

#[test]
fn b007_small_item_recovery_has_explicit_small_dimensions() {
    for r in Revision::ALL {
        let mut fixture = Fixture::new(r, &[0xff5000]);
        fixture.address[2] = 0xfeed_ffff;
        let item = Address::<tje_mem::LevelItem>::new(0xff5000);
        item.kind().write(&mut fixture, 64);
        item.level().write(&mut fixture, 3);
        fixture
            .faithful(ids::ENTITY_CREATE_LEVEL_ITEM, &[item.address()])
            .unwrap();
        let old = &fixture
            .calls
            .iter()
            .find(|(id, _)| *id == ids::MAP_FIND_ITEM_PLACEMENT_POSITION)
            .unwrap()
            .1;
        assert_eq!(
            old[3], 0xfeed_ffff,
            "faithful omitted flag consumes saved A2"
        );
        fixture.calls.clear();
        ram::g_player1_map_level(r).write(&mut fixture, 3);
        fixture.validation = true;
        placement::create_item(&mut fixture, &[item.address()]).unwrap();
        let args = &fixture
            .calls
            .iter()
            .find(|(id, _)| *id == ids::TERRAIN_VALIDATION)
            .unwrap()
            .1;
        assert_eq!(&args[1..3], &[3, 1]);
    }
}

#[test]
fn b008_b011_exhaustion_preserves_unowned_storage() {
    for r in Revision::ALL {
        let mut old = Fixture::new(r, &[]);
        let player = ram::g_player_info(r).at(0);
        player.level().write(&mut old, 3);
        ram::g_earthling_bank_users(r).at(0).write(&mut old, 1);
        ram::g_earthling_bank_levels(r).at(0).write(&mut old, 3);
        for i in 0..20 {
            let record = ram::g_earthling_banks(r).at(0).at(i);
            record.kind().write(&mut old, 1);
            record.object().write(&mut old, 6);
        }
        let next = ram::g_earthling_banks(r).at(1).at(0);
        next.kind().write(&mut old, 33);
        old.faithful(ids::OPEN_EARTHLING_PRESENT, &[player.address(), 0x10])
            .unwrap();
        assert_ne!(next.kind().read(&old), 33);
        next.kind().write(&mut old, 33);
        old.reads.borrow_mut().clear();
        assert_eq!(
            resources::earthling_present(&mut old, &[player.address(), 0x10]).unwrap(),
            Some(0)
        );
        assert!(!old.reads.borrow().contains(&next.kind().address()));
        assert_eq!(next.kind().read(&old), 33);
        // The first scan's one-past read is also forbidden when fallback succeeds.
        ram::g_earthling_banks(r)
            .at(0)
            .at(19)
            .object()
            .write(&mut old, 0);
        old.reads.borrow_mut().clear();
        resources::earthling_present(&mut old, &[player.address(), 0x10]).unwrap();
        assert!(!old.reads.borrow().contains(&next.kind().address()));
        let mut sprite = Fixture::new(r, &[]);
        let entity = ram::g_player_info(r).at(6);
        entity.draw_flags().write(&mut sprite, 1);
        entity.behavior_flags().write(&mut sprite, 4);
        entity.sprite_count().write(&mut sprite, 1);
        entity.tile_count().write(&mut sprite, 12);
        sprite.data[4] = 77;
        sprite
            .faithful(
                ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
                &[entity.address(), 0],
            )
            .unwrap();
        assert!(
            sprite
                .calls
                .iter()
                .any(|(id, args)| *id == ids::FREE_SPRITE_TILE_RUN && args[0] == 77)
        );
        sprite.calls.clear();
        resources::viewport_sprites(&mut sprite, &[entity.address(), 0]).unwrap();
        assert!(
            !sprite
                .calls
                .iter()
                .any(|(id, _)| *id == ids::FREE_SPRITE_TILE_RUN)
        );
    }
}

#[test]
fn b001_b002_correct_two_axis_and_capacity_boundaries() {
    for r in Revision::ALL {
        let player = ram::g_player_info(r).at(0);
        let mut fixture = Fixture::new(r, &[player.address(), 0, 0, 0]);
        player
            .animation_group_mask()
            .write(&mut fixture, 0x0400_0000);
        player.velocity_x().write(&mut fixture, 0x700);
        player.velocity_y().write(&mut fixture, 0x500);
        fixture
            .faithful(
                ids::PLAYER_APPLY_ROCKET_SKATES_INPUT,
                &[player.address(), 0, 0, 0],
            )
            .unwrap();
        assert_eq!(player.velocity_y().read(&fixture), 0x500);
        let mut k = Continuation {
            stage: 1,
            ..Default::default()
        };
        k.locals[0] = player.address();
        skates(&mut fixture, &mut k).unwrap();
        assert_eq!(
            (
                player.velocity_x().read(&fixture),
                player.velocity_y().read(&fixture)
            ),
            (0x600, 0x480)
        );
        let mut sharing = Fixture::new(r, &[]);
        let earl = ram::g_player_info(r).at(1);
        earl.sprite_state().write(&mut sharing, 90);
        earl.frame_duration().write(&mut sharing, 0);
        ram::g_player_health(r).at(0).write(&mut sharing, 23);
        ram::g_player_health(r).at(1).write(&mut sharing, 30);
        sharing
            .faithful(ids::PLAYER_UPDATE_HEALTH_SHARING_CLAP, &[])
            .unwrap();
        assert_eq!(ram::g_player_health(r).at(0).read(&sharing), 27);
        assert_eq!(share_health(53, 23, 31), (23, 30));
        ram::g_player_health(r).at(0).write(&mut sharing, 23);
        ram::g_player_health(r).at(1).write(&mut sharing, 30);
        sharing.run_callback(clap).unwrap();
        assert_eq!(ram::g_player_health(r).at(0).read(&sharing), 23);
        assert_eq!(ram::g_player_health(r).at(1).read(&sharing), 30);
        assert_eq!(share_health(31, 23, 31), (16, 15));
    }
}

#[test]
fn b014_b015_correct_only_the_original_bad_callsites() {
    for r in Revision::ALL {
        let mut reunion = Fixture::new(r, &[]);
        reunion.data[2] = 0xbeef;
        ram::g_reunion_dialogue_pending(r).write(&mut reunion, 1);
        reunion
            .faithful(ids::UPDATE_SPEECH_AND_DIALOGUE, &[])
            .unwrap();
        let omitted = &reunion
            .calls
            .iter()
            .find(|(id, _)| *id == ids::START_DIALOGUE)
            .unwrap()
            .1;
        assert_eq!(
            omitted,
            &[1, 0xbeef],
            "faithful reunion consumes saved incoming D2"
        );
        let mut speech = Fixture::new(r, &[1, 11, 0]);
        ram::g_player_promotion_display_entities(r)
            .at(1)
            .write(&mut speech, 6);
        speech
            .faithful(ids::PERFORM_PROMOTION, &[1, 11, 0])
            .unwrap();
        let args = speech
            .calls
            .iter()
            .find(|(id, _)| *id == ids::GENERATE_SPEECH)
            .unwrap()
            .1
            .clone();
        assert_eq!(args[0], ram::g_player_info(r).at(1).address());
        speech.args = args;
        speech.address[7] = 0xff7000;
        speech.write_u32(0xff7000, ids::PERFORM_PROMOTION.metadata().address(r) + 2);
        match promotion_speaker(&mut speech, &mut Continuation::default()).unwrap() {
            OverrideStep::CallOriginal { arguments, .. } => assert_eq!(arguments[0], 1),
            _ => panic!("corrected promotion arguments"),
        }
        for (caller, offset, dialogue, speaker) in [
            (ids::UPDATE_SPEECH_AND_DIALOGUE, 0x1de, 1, 0),
            (ids::UPDATE_SPEECH_AND_DIALOGUE, 0x1de, 3, 0),
            (ids::UPDATE_SPEECH_AND_DIALOGUE, 0x1de, 4, 1),
            (ids::SHIP_PIECE_ABILITY, 0x86, 2, 1),
        ] {
            let mut fixture = Fixture::new(r, &[dialogue, 0xbeef]);
            fixture.data[2] = 1;
            fixture.write_u32(0xff7000, caller.metadata().address(r) + offset);
            match dialogue_speaker(&mut fixture, &mut Continuation::default()).unwrap() {
                OverrideStep::CallOriginal { arguments, .. } => {
                    assert_eq!(arguments, vec![dialogue, speaker])
                }
                _ => panic!("explicit caller policy"),
            }
        }
    }
}

#[test]
fn r001_through_r008_are_rev00_only_and_individually_selected() {
    for r in Revision::ALL {
        assert_eq!(
            new_mode(r, Settings::default()).unwrap().profile_id(),
            "faithful"
        );
        let mode = new_mode(
            r,
            Settings {
                enabled: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(mode.profile_id(), PROFILE_ID);
        for fix in &super::super::MANIFEST[12..] {
            let mut settings = Settings {
                enabled: true,
                fixes: 0,
            };
            settings.set(fix.id, true).unwrap();
            new_mode(r, settings).unwrap();
        }
    }
    let r = Revision::Rev00;
    let player = ram::g_player_info(r).at(0);
    let mut falling = Fixture::new(r, &[player.address(), 0, 5, 0]);
    ram::g_player_fall_states(r).at(0).write(&mut falling, 1);
    falling
        .faithful(ids::SETUP_SELECTED_PRESENT, &[player.address(), 0, 5, 0])
        .unwrap();
    assert!(
        falling
            .calls
            .iter()
            .any(|(id, _)| *id == ids::OPEN_ROCKET_SKATES)
    );
    falling.calls.clear();
    falling.run_callback(dispatch).unwrap();
    assert!(
        !falling
            .calls
            .iter()
            .any(|(id, _)| *id == ids::OPEN_ROCKET_SKATES)
    );
    let mut pose = Fixture::new(r, &[player.address(), 0, 1, 0]);
    player.animation_group_mask().write(&mut pose, 0x0800_0000);
    player.sprite_state().write(&mut pose, 70);
    pose.faithful(ids::SETUP_SELECTED_PRESENT, &[player.address(), 0, 1, 0])
        .unwrap();
    assert!(!pose.calls.iter().any(|(id, _)| *id == ids::SET_SPRITE));
    pose.calls.clear();
    pose.run_callback(dispatch).unwrap();
    assert!(pose.calls.iter().any(|(id, _)| *id == ids::SET_SPRITE));
    for sprite in [78, 79] {
        let mut land_fixture = Fixture::new(r, &[]);
        player.level().write(&mut land_fixture, 3);
        player.x().write(&mut land_fixture, 256);
        player.y().write(&mut land_fixture, 128);
        player.sprite_state().write(&mut land_fixture, sprite);
        ram::g_player_lives(r).at(1).write(&mut land_fixture, -1);
        ram::g_player1_level_map(r)
            .at(1)
            .at(1)
            .write(&mut land_fixture, 0x4000);
        ram::g_player1_terrain_underfoot(r).write(&mut land_fixture, 0x90);
        land_fixture.faithful(ids::PROCESS_LAND, &[]).unwrap();
        assert!(
            land_fixture
                .calls
                .iter()
                .any(|(id, _)| *id == ids::PLAYER_START_FALLING)
        );
        land_fixture.calls.clear();
        land_fixture.run_callback(land).unwrap();
        assert!(
            !land_fixture
                .calls
                .iter()
                .any(|(id, _)| *id == ids::PLAYER_START_FALLING)
        );
    }
    let mut sleeping = Fixture::new(r, &[player.address()]);
    ram::g_player_interaction_choice_active(r)
        .at(0)
        .write(&mut sleeping, 1);
    ram::g_player_idle_ticks(r).at(0).write(&mut sleeping, 300);
    sleeping
        .faithful(ids::PROCESS_SLEEP, &[player.address()])
        .unwrap();
    assert!(
        sleeping
            .calls
            .iter()
            .any(|(id, args)| *id == ids::SET_SPRITE && args[1] == 117)
    );
    sleeping.calls.clear();
    sleeping.run_callback(sleep).unwrap();
    assert_eq!(ram::g_player_idle_ticks(r).at(0).read(&sleeping), 0);
    for (id, callback) in [
        (ids::DRAW_GAME_MAP, map_entry as Callback),
        (ids::DRAW_INVENTORY, inventory_entry),
    ] {
        let mut fixture = Fixture::new(r, &[0, 1]);
        fixture.limit = 0;
        ram::g_player_idle_ticks(r).at(0).write(&mut fixture, 299);
        assert!(fixture.faithful(id, &[0, 1]).is_err());
        assert_eq!(ram::g_player_idle_ticks(r).at(0).read(&fixture), 299);
        callback(&mut fixture, &mut Continuation::default()).unwrap();
        assert_eq!(ram::g_player_idle_ticks(r).at(0).read(&fixture), 0);
    }
    for (id, callback) in [
        (ids::TITLE_SCREEN_VIDEO, title_entry as Callback),
        (ids::SEGA_VIDEO, sega_entry),
    ] {
        let mut fixture = Fixture::new(r, &[]);
        ram::g_frame_counter(r).write(&mut fixture, 65500);
        fixture.limit = 0;
        assert!(fixture.faithful(id, &[]).is_err());
        assert_eq!(ram::g_frame_counter(r).read(&fixture), 65500);
        assert!(u32::from(ram::g_frame_counter(r).read(&fixture)) + 120 > 65535);
        callback(&mut fixture, &mut Continuation::default()).unwrap();
        assert_eq!(ram::g_frame_counter(r).read(&fixture), 0);
    }
    let mut lean = Fixture::new(r, &[0, 0xff5000]);
    player.animation_group_mask().write(&mut lean, 0x10);
    lean.faithful(ids::PLAYER_PROCESS_TERRAIN_MOTION, &[0, 0xff5000])
        .unwrap();
    assert!(
        lean.calls
            .iter()
            .any(|(id, _)| *id == ids::PLAYER_APPLY_TERRAIN_DESTINATION)
    );
    assert!(matches!(
        terrain_motion(&mut lean, &mut Continuation::default()).unwrap(),
        OverrideStep::Return(None)
    ));
}
