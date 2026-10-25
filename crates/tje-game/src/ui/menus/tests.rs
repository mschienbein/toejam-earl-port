use super::{player_update_health_and_lives, text_write_decimal_tiles};
use crate::{
    Context, Dispatcher, FunctionId, Result, call_frame::FrameState, execution::ExecutionClass, ids,
};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
};
use tje_mem::{AddressSpace, Bus, Revision, ram};

struct Fixture {
    memory: AddressSpace,
    data: [u32; 8],
    addresses: [u32; 7],
    status: u16,
    sp: u32,
    frame: FrameState,
    class: ExecutionClass,
    kills: Vec<u32>,
    strings: Vec<(u32, Vec<u8>)>,
    clock: crate::execution::CallClock,
    watched_sprite: Option<u32>,
    sprite_reads: Cell<u32>,
    sprite_values: RefCell<VecDeque<u8>>,
    native_dependencies: bool,
    watched_health: Option<[u32; 2]>,
    health_reads_after_dma: RefCell<Vec<u32>>,
    vdp_controls: Vec<u16>,
    word_writes: Vec<(u32, u16)>,
    discarded_reads: Vec<(u32, u8)>,
    word_bus_events: Vec<(u32, Option<u16>)>,
    record_poll_reads: bool,
    poll_reads: RefCell<Vec<u32>>,
    upload_waits: usize,
}
impl Fixture {
    fn new(revision: Revision) -> Self {
        Self {
            memory: AddressSpace::new(vec![0; 0x100000], revision),
            data: std::array::from_fn(|index| 0x76540000 | index as u32),
            addresses: std::array::from_fn(|index| 0xff1000 + index as u32 * 4),
            status: 0x2000,
            sp: 0xff8000,
            frame: FrameState::new(0xff8000),
            class: ExecutionClass::TimeAware,
            kills: Vec::new(),
            strings: Vec::new(),
            clock: crate::execution::CallClock::default(),
            watched_sprite: None,
            sprite_reads: Cell::new(0),
            sprite_values: RefCell::new(VecDeque::new()),
            native_dependencies: false,
            watched_health: None,
            health_reads_after_dma: RefCell::new(Vec::new()),
            vdp_controls: Vec::new(),
            word_writes: Vec::new(),
            discarded_reads: Vec::new(),
            word_bus_events: Vec::new(),
            record_poll_reads: false,
            poll_reads: RefCell::new(Vec::new()),
            upload_waits: 0,
        }
    }
}
impl Bus for Fixture {
    fn read_u8(&self, address: u32) -> u8 {
        if self.record_poll_reads {
            self.poll_reads.borrow_mut().push(address);
        }
        if !self.vdp_controls.is_empty()
            && self
                .watched_health
                .is_some_and(|addresses| addresses.contains(&address))
        {
            self.health_reads_after_dma.borrow_mut().push(address);
        }
        if self.watched_sprite == Some(address) {
            self.sprite_reads.set(self.sprite_reads.get() + 1);
            if let Some(value) = self.sprite_values.borrow_mut().pop_front() {
                return value;
            }
        }
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.word_writes.push((address, value));
        self.word_bus_events.push((address, Some(value)));
        self.memory.write_u16(address, value);
    }
}
impl tje_mem::Platform for Fixture {
    fn write_vdp_control(&mut self, value: u16) {
        assert!(self.native_dependencies, "unexpected VDP control");
        self.vdp_controls.push(value);
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("unexpected VDP data");
    }
}
impl crate::Platform for Fixture {
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
    fn wait_until(&mut self, request: crate::execution::WaitRequest) -> Result<()> {
        assert!(self.record_poll_reads, "unexpected wait in isolated test");
        assert_eq!(
            request.condition,
            crate::execution::WaitCondition::Interrupt
        );
        self.upload_waits += 1;
        let pending = ram::g_sprite_graphics_upload_pending(self.revision()).address();
        self.memory.write_u8(pending, 0);
        Ok(())
    }
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.discarded_reads.push((address, 1));
        let _ = self.memory.read_u8(address);
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.discarded_reads.push((address, 2));
        self.word_bus_events.push((address, None));
        let _ = self.memory.read_u16(address);
        Ok(())
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
            .map_err(crate::Error::Clock)
    }
    fn leave_call_ceiling(&mut self, scope: crate::execution::CallCeilingScope) -> Result<()> {
        self.clock
            .leave(scope)
            .map(|_| ())
            .map_err(crate::Error::Clock)
    }
    fn dispatcher(&self) -> Dispatcher {
        static RESOURCE_CALLEES: &[crate::FramedTranslation] = &[crate::FramedTranslation {
            id: ids::CHECK_PROMOTION_SCORE,
            function: super::check_promotion_score,
        }];
        if self.native_dependencies {
            Dispatcher::hybrid_with_overrides_and_frames(&[], RESOURCE_CALLEES)
        } else {
            Dispatcher::hybrid_with_frames(&[], RESOURCE_CALLEES)
        }
    }
    fn call_original(&mut self, id: FunctionId, arguments: &[u32]) -> Result<u32> {
        self.call_original_with_execution(id, arguments, ExecutionClass::Atomic)
    }
    fn call_original_with_execution(
        &mut self,
        id: FunctionId,
        arguments: &[u32],
        _: ExecutionClass,
    ) -> Result<u32> {
        assert_eq!(
            id,
            ids::KILL_THE_PLAYER,
            "unexpected dependency in isolated arithmetic test"
        );
        self.kills.push(arguments[0]);
        Ok(self.data[0])
    }
    fn call_original_entry(&mut self, id: FunctionId, class: ExecutionClass) -> Result<u32> {
        if id == ids::KILL_THE_PLAYER {
            let return_pc = self.read_u32(self.sp);
            assert_eq!(
                return_pc,
                ids::PLAYER_UPDATE_HEALTH_AND_LIVES
                    .metadata()
                    .address(self.revision())
                    + 0x7e
            );
            let player = self.frame.argument_word(0).read(self);
            self.kills.push(i32::from(player) as u32);
            self.sp += 4;
            return Ok(self.data[0]);
        }
        assert_eq!(id, ids::TEXT_WRITE_STRING_TILES);
        assert_eq!(class, ExecutionClass::Functional);
        let return_pc = self.read_u32(self.sp);
        let expected = ids::TEXT_WRITE_DECIMAL_TILES
            .metadata()
            .address(self.revision())
            + 0x8a;
        assert_eq!(return_pc, expected);
        let pointer = self.frame.argument_slot(0).read(self);
        let mut text = Vec::new();
        let mut address = pointer;
        while self.read_u8(address) != 0 {
            text.push(self.read_u8(address));
            address += 1;
        }
        self.strings.push((pointer, text));
        self.sp += 4;
        Ok(self.data[0])
    }
}

#[test]
fn canonical_hud_frames_remain_pending_and_time_aware() {
    for entry in super::RESOURCE_FRAME_CANDIDATES
        .iter()
        .chain(crate::ui::hud_all::FRAME_CANDIDATES)
    {
        assert!(crate::ui::PENDING.contains(&entry.id));
        assert!(
            crate::ui::PENDING_FRAMED_TRANSLATIONS
                .iter()
                .any(|pending| pending.id == entry.id)
        );
        assert!(
            !crate::ui::TRANSLATIONS
                .iter()
                .any(|proven| proven.id == entry.id)
        );
        assert_eq!(
            crate::execution_class(entry.id),
            Some(ExecutionClass::TimeAware)
        );
    }
}

#[test]
fn hud_call_arguments_preserve_move_pea_and_clear_bus_beats() {
    use crate::ui::hud_all::frame as source;
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Fixture::new(revision);
        let sp = context.sp;
        source::push_move(&mut context, 0x12345678).unwrap();
        assert_eq!(context.word_writes, [(sp - 2, 0x5678), (sp - 4, 0x1234)]);
        source::release(&mut context, 4).unwrap();
        context.word_writes.clear();
        source::push_pea(&mut context, 0x12345678).unwrap();
        assert_eq!(context.word_writes, [(sp - 4, 0x1234), (sp - 2, 0x5678)]);
        source::release(&mut context, 4).unwrap();
        context.word_writes.clear();
        context.word_bus_events.clear();
        source::push_clear(&mut context).unwrap();
        assert_eq!(context.discarded_reads, [(sp - 4, 2), (sp - 2, 2)]);
        assert_eq!(context.word_writes, [(sp - 4, 0), (sp - 2, 0)]);
        assert_eq!(
            context.word_bus_events,
            [
                (sp - 4, None),
                (sp - 2, None),
                (sp - 4, Some(0)),
                (sp - 2, Some(0)),
            ]
        );
        source::release(&mut context, 4).unwrap();
        assert_eq!(context.sp, sp);
    }
}

#[test]
fn upload_wait_reads_only_the_source_pending_byte() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Fixture::new(revision);
        let pending = ram::g_sprite_graphics_upload_pending(revision);
        pending.write(&mut context, 1);
        context.record_poll_reads = true;
        crate::ui::animations::wait_upload(&mut context).unwrap();
        assert_eq!(context.upload_waits, 1);
        assert_eq!(
            *context.poll_reads.borrow(),
            [pending.address(), pending.address()]
        );
    }
}

fn call_health_update(context: &mut Fixture, object: u32, terrain: u16) {
    let sp = context.sp;
    let parent = context.frame;
    context.write_u32(sp + 4, object);
    context.write_u16(sp + 10, terrain);
    player_update_health_and_lives(context).unwrap();
    assert_eq!(context.sp, sp);
    assert_eq!(context.frame, parent);
}

#[test]
fn health_predecrement_matches_subq_byte_n_xor_v_for_every_life_byte() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for player in 0..2 {
            let mut context = Fixture::new(revision);
            let object = ram::g_player_info(revision).at(player);
            object.entity_index().write(&mut context, player as i8);
            for bits in 0..=255_u8 {
                let lives = bits as i8;
                for saved in [-1, 0, 100] {
                    context.kills.clear();
                    ram::g_player_health(revision)
                        .at(player)
                        .write(&mut context, 0);
                    ram::g_player_lives(revision)
                        .at(player)
                        .write(&mut context, lives);
                    ram::g_player_health_before_swimming(revision)
                        .at(player)
                        .write(&mut context, saved);
                    call_health_update(&mut context, object.address(), 0);
                    let decremented = lives.wrapping_sub(1);
                    let overflow = lives == i8::MIN;
                    let negative_branch = (decremented < 0) != overflow;
                    let expected_lives = if negative_branch { lives } else { decremented };
                    let expected_health = if negative_branch { 0 } else { 100 };
                    let expected_saved = if negative_branch || saved == -1 {
                        -1
                    } else {
                        100
                    };
                    assert_eq!(
                        ram::g_player_lives(revision).at(player).read(&context),
                        expected_lives
                    );
                    assert_eq!(
                        ram::g_player_health(revision).at(player).read(&context),
                        expected_health
                    );
                    assert_eq!(
                        ram::g_player_health_before_swimming(revision)
                            .at(player)
                            .read(&context),
                        expected_saved
                    );
                    assert_eq!(context.kills, [player as u32]);
                }
            }
        }
    }
}

#[test]
fn health_ghost_comparisons_consume_distinct_source_byte_reads() {
    // G c_hud.s:1654-1657 compares memory twice if the first comparison fails.
    // Simulate an environment update between those two semantic reads.
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (values, expected_reads) in [(vec![5], 1), (vec![0, 6], 2)] {
            let mut context = Fixture::new(revision);
            let object = ram::g_player_info(revision).at(0);
            object.entity_index().write(&mut context, 0);
            context.watched_sprite = Some(object.sprite_state().address());
            *context.sprite_values.borrow_mut() = values.into();
            call_health_update(&mut context, object.address(), 0);
            assert_eq!(context.sprite_reads.get(), expected_reads);
            assert!(context.sprite_values.borrow().is_empty());
            assert!(
                context.kills.is_empty(),
                "the second ghost observation must return"
            );
        }
    }
}

#[test]
fn health_swim_comparisons_preserve_second_observation_and_short_circuit() {
    // G c_hud.s:1715-1718 also reloads sprite_state for the near-shore test.
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (values, expected_reads) in [(vec![0, 0, 3], 3), (vec![0, 0, 0, 7], 4)] {
            let mut context = Fixture::new(revision);
            let object = ram::g_player_info(revision).at(0);
            object.entity_index().write(&mut context, 0);
            ram::g_player_lives(revision).at(0).write(&mut context, -1);
            ram::g_hud_displayed_health(revision)
                .at(0)
                .write(&mut context, 1);
            ram::g_player_health(revision).at(0).write(&mut context, 5);
            ram::g_player_health_before_swimming(revision)
                .at(0)
                .write(&mut context, -1);
            context.watched_sprite = Some(object.sprite_state().address());
            *context.sprite_values.borrow_mut() = values.into();
            call_health_update(&mut context, object.address(), 0);
            assert_eq!(context.sprite_reads.get(), expected_reads);
            assert!(context.sprite_values.borrow().is_empty());
            assert_eq!(ram::g_player_health(revision).at(0).read(&context), 4);
            assert_eq!(
                ram::g_player_health_before_swimming(revision)
                    .at(0)
                    .read(&context),
                5
            );
            assert!(context.kills.is_empty());
        }
    }
}

#[test]
fn immediate_health_initializer_reads_actual_before_displayed_after_native_dma() {
    // Both c_hud.s:598,601: MOVE.B actual, then CMP.B displayed.
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Fixture::new(revision);
        context.native_dependencies = true;
        let health = ram::g_player_health(revision).at(0);
        let displayed = ram::g_hud_displayed_health(revision).at(0);
        context.watched_health = Some([health.address(), displayed.address()]);
        health.write(&mut context, 0);
        displayed.write(&mut context, 99);
        let sp = context.sp;
        context.write_u16(sp + 6, 0);
        context.write_u16(sp + 10, 1);
        crate::ui::hud_all::hud_initialize_health_bar(&mut context).unwrap();
        assert!(
            !context.vdp_controls.is_empty(),
            "the real Rust DMA body must run"
        );
        assert_eq!(
            *context.health_reads_after_dma.borrow(),
            [health.address(), displayed.address()],
        );
        let discarded_bytes: Vec<_> = context
            .discarded_reads
            .iter()
            .filter_map(|&(address, width)| (width == 1).then_some(address))
            .collect();
        let buffer = ram::g_sprite_tile_upload_buffer(revision).address();
        let expected: Vec<_> = std::iter::once(displayed.address())
            .chain((0..256).map(|offset| buffer + offset))
            .collect();
        assert_eq!(discarded_bytes, expected);
    }
}

#[test]
fn queued_health_initializer_retains_count_byte_alias_between_postincrements() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Fixture::new(revision);
        context.record_poll_reads = true;
        let count = ram::g_sprite_tile_upload_queue_word_count(revision);
        let runs = ram::g_sprite_tile_upload_runs(revision);
        assert_eq!(runs.at(41).address(), count.address());
        count.write(&mut context, 41);
        let sp = context.sp;
        context.write_u16(sp + 6, 0);
        context.write_u16(sp + 10, 0);
        crate::ui::hud_all::hud_initialize_health_bar(&mut context).unwrap();
        // The first queued tile writes 0x07b4 over the count byte. The second
        // postincrement therefore consumes 7, not the pre-store value 42.
        assert_eq!(count.read(&context), 8);
        assert_eq!(runs.at(7).read(&context), 8);
        assert_eq!(runs.at(41).read(&context), 0x08b4);
        assert_eq!(
            ram::g_sprite_tile_buffer_offset(revision).read(&context),
            256
        );
        assert_eq!(context.upload_waits, 1);
        assert_eq!(context.sp, sp);
    }
}

#[test]
fn decimal_uses_original_emulated_array_and_preserves_minimum_short_bug() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (value, expected) in [
            (i16::MIN, "--.)*("),
            (-10000, "-10000"),
            (-1, "-1"),
            (0, "0"),
            (9, "9"),
            (10, "10"),
            (100, "100"),
            (32767, "32767"),
        ] {
            let mut context = Fixture::new(revision);
            let data = context.data;
            let addresses = context.addresses;
            text_write_decimal_tiles(&mut context, &[i32::from(value) as u32]).unwrap();
            assert_eq!(context.strings, [(0xff7ff0, expected.as_bytes().to_vec())]);
            assert_eq!(context.sp, 0xff8000);
            assert_eq!(context.frame, FrameState::new(0xff8000));
            assert_eq!(context.data, data);
            assert_eq!(context.addresses, addresses);
        }
    }
}
