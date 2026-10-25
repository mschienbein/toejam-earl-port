//! Source-order regressions use authored memory and explicit callee doubles.
//! They are unit tests, not differential or replay-environment proof evidence.
use super::*;
use crate::call_frame::FrameState;
use crate::execution::{CallCeilingScope, ExecutionClass, RamWidth, WaitCondition, WaitRequest};
use std::{cell::RefCell, collections::VecDeque};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Access {
    Read(u32, u8),
    Discard(u32, u8),
    Write(u32, u8, u32),
    Call(FunctionId, Vec<u32>),
    Stack(u32),
    Vdp(u32, u16),
    Wait(WaitRequest),
}

struct RootMachine {
    inner: Machine,
    accesses: RefCell<Vec<Access>>,
    frame: FrameState,
    allocations: VecDeque<u32>,
    validation_succeeds: bool,
    reject_generation: bool,
    load_rejections: u8,
    input: VecDeque<u32>,
    pending_samples: RefCell<Option<(u32, VecDeque<u8>)>>,
}
impl RootMachine {
    fn new(r: Revision) -> Self {
        Self::with_rom(r, vec![0; 0x100000])
    }
    fn with_rom(r: Revision, bytes: Vec<u8>) -> Self {
        let mut inner = Machine::with_rom(r, bytes);
        inner.address[7] = 0xff7000;
        Self {
            inner,
            accesses: RefCell::new(Vec::new()),
            frame: FrameState::new(0xff7000),
            allocations: VecDeque::new(),
            validation_succeeds: true,
            reject_generation: false,
            load_rejections: 0,
            input: VecDeque::from([0, 0x20, 0]),
            pending_samples: RefCell::new(None),
        }
    }
    fn clear_accesses(&self) {
        self.accesses.borrow_mut().clear();
    }
    fn reads(&self) -> Vec<(u32, u8)> {
        self.accesses
            .borrow()
            .iter()
            .filter_map(|access| match access {
                Access::Read(address, width) => Some((*address, *width)),
                _ => None,
            })
            .collect()
    }
    fn calls(&self) -> Vec<(FunctionId, Vec<u32>)> {
        self.accesses
            .borrow()
            .iter()
            .filter_map(|access| match access {
                Access::Call(id, args) => Some((*id, args.clone())),
                _ => None,
            })
            .collect()
    }
    fn pending_samples(&mut self, samples: &[u8]) {
        let address = ram::g_sprite_graphics_upload_pending(self.revision()).address();
        *self.pending_samples.borrow_mut() = Some((address, samples.iter().copied().collect()));
    }
    fn original_effects(&mut self, id: FunctionId, args: &[u32]) -> Result<()> {
        self.accesses
            .borrow_mut()
            .push(Access::Call(id, args.to_vec()));
        let value = match id {
            ids::STACK_BASED_RANDOM => 0x10203040,
            ids::LMULT => self.inner.data[0].wrapping_mul(self.inner.data[1]),
            ids::MAP_GENERATE_SEPARATED_ANCHORS => 2,
            ids::MAP_GET_RANDOM_GENERATION_PARAMETER => 25,
            ids::MAP_IS_ANCHOR_CONNECTION_MISSING => u32::from(self.reject_generation),
            ids::MAP_TRANSITION_CONTAINS_METATILE => 0,
            ids::ALLOCATE_SPRITE_TILE_RUN => self.allocations.pop_front().unwrap_or(8),
            ids::TERRAIN_VALIDATION => {
                if self.validation_succeeds {
                    self.write_i16(args[3], 4);
                    self.write_i16(args[4], 6);
                    1
                } else {
                    0
                }
            }
            ids::CHANGE_LEVEL => {
                let rejected = self.load_rejections != 0;
                self.load_rejections = self.load_rejections.saturating_sub(1);
                u32::from(rejected)
            }
            ids::READ_MAPPED_PLAYER_INPUT => self.input.pop_front().expect("finite input fixture"),
            ids::GET_RANDOM_NUMBER
            | ids::LOAD_FINAL_LEVEL
            | ids::MAP_SWAP_PLAYER_WITH_SAVED_BANK
            | ids::COPY_LEVEL_BETWEEN_PLAYERS
            | ids::MAP_SWAP_PLAYER_BANKS
            | ids::MAP_CONNECT_ANCHOR_POSITIONS
            | ids::MAP_GROW_TERRAIN_REGION
            | ids::MAP_STABILIZE_REGION_CONNECTIONS
            | ids::MAP_SEED_FILLED_TERRAIN_REGIONS
            | ids::MAP_BUILD_REGION_EDGE_SHAPES
            | ids::MAP_BUILD_FILLED_TERRAIN_SHAPES
            | ids::MAP_BUILD_DIRECTIONAL_PATH_SHAPES
            | ids::MAP_SCATTER_TERRAIN_FEATURES
            | ids::MAP_ASSIGN_TRANSITION_CHAIN
            | ids::MAP_CHOOSE_METATILE_ALTERNATIVES
            | ids::ENTITY_CLEAR_VIEWPORT_SPRITES
            | ids::FREE_SPRITE_TILE_RUN
            | ids::WAIT_FOR_NEXT_V_BLANK
            | ids::SPRITE_UNPACK_INTERLEAVED_RLE
            | ids::SPRITE_UNPACK_RLE
            | ids::DMA_COPY_WORDS_TO_VRAM
            | ids::UPLOAD_QUEUED_SPRITE_GRAPHICS
            | ids::ELEVATOR_CONTAINS_POSITION
            | ids::DISABLE_BLANKING_GRAPHICS
            | ids::MOVE_DATA_TO_VRAM
            | ids::MAP_INITIALIZE_SCREEN_SPRITES
            | ids::MAP_DRAW_BACKGROUND_AND_TERRAIN
            | ids::LOAD_MAP_INTO_VRAM
            | ids::COPY_PALETTE_TO_CRAM_SLOT
            | ids::MAP_DRAW_PLAYER
            | ids::MAP_DRAW_SHIP_PIECE
            | ids::MAP_DRAW_ELEVATOR
            | ids::MAP_UPLOAD_COVER_ANIMATION_FRAME
            | ids::PRE_GENERATE_RANDOMS
            | ids::DRAW_STARFIELD_PLANE
            | ids::DRAW_HUD
            | ids::ENABLE_BLANKING_GRAPHICS => 0,
            _ => return Err(Error::Untranslated(id)),
        };
        self.inner.data[0] = value;
        {
            if id.metadata().abi.returns_value {
                self.set_data_register(0, value);
            }
            Ok(())
        }
    }
}
impl Bus for RootMachine {
    fn read_u8(&self, address: u32) -> u8 {
        self.accesses.borrow_mut().push(Access::Read(address, 1));
        if let Some((poll, samples)) = self.pending_samples.borrow_mut().as_mut()
            && address == *poll
        {
            return samples
                .pop_front()
                .expect("unexpected poll after the final zero");
        }
        self.inner.memory.read_u8(address)
    }
    fn read_u16(&self, address: u32) -> u16 {
        self.accesses.borrow_mut().push(Access::Read(address, 2));
        self.inner.memory.read_u16(address)
    }
    fn read_u32(&self, address: u32) -> u32 {
        self.accesses.borrow_mut().push(Access::Read(address, 4));
        self.inner.memory.read_u32(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 1, u32::from(value)));
        self.inner.memory.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 2, u32::from(value)));
        self.inner.memory.write_u16(address, value);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 4, value));
        self.inner.memory.write_u32(address, value);
    }
}
impl tje_mem::Platform for RootMachine {
    fn write_vdp_control(&mut self, value: u16) {
        self.accesses
            .borrow_mut()
            .push(Access::Vdp(0xc00004, value));
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.accesses
            .borrow_mut()
            .push(Access::Vdp(0xc00000, value));
    }
}
impl crate::Platform for RootMachine {
    fn wait_until(&mut self, request: WaitRequest) -> Result<()> {
        self.accesses.borrow_mut().push(Access::Wait(request));
        let WaitCondition::Ram {
            address,
            width: RamWidth::Byte,
            mask,
            value,
            comparison,
        } = request.condition
        else {
            panic!("unexpected proxy wait {request:?}");
        };
        assert_eq!(request, WaitRequest::ram_byte_equal(address, 0xff, 0));
        for _ in 0..request.max_ticks {
            let sample = u32::from(self.read_u8(address));
            if comparison.satisfied(sample, mask, value) {
                return Ok(());
            }
        }
        Err(Error::Platform(
            "unit byte wait exhausted its real polls".into(),
        ))
    }
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.accesses.borrow_mut().push(Access::Discard(address, 1));
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.accesses.borrow_mut().push(Access::Discard(address, 2));
        Ok(())
    }
    fn read_device_u8(&mut self, address: u32) -> u8 {
        panic!("unexpected device read {address:08x}");
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        panic!("unexpected device read {address:08x}");
    }
    fn write_device_u8(&mut self, address: u32, _: u8) {
        panic!("unexpected device write {address:08x}");
    }
    fn write_device_u16(&mut self, address: u32, _: u16) {
        panic!("unexpected device write {address:08x}");
    }
}
impl Context for RootMachine {
    fn revision(&self) -> Revision {
        self.inner.memory.revision()
    }
    fn data_register(&self, index: usize) -> u32 {
        self.inner.data[index]
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.inner.data[index] = value;
    }
    fn address_register(&self, index: usize) -> u32 {
        self.inner.address[index]
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        if index == 7 {
            self.accesses.borrow_mut().push(Access::Stack(value));
        }
        self.inner.address[index] = value;
    }
    fn status_register(&self) -> u16 {
        self.inner.status
    }
    fn set_status_register(&mut self, value: u16) {
        self.inner.status = value;
    }
    fn stack_pointer(&self) -> Result<u32> {
        Ok(self.inner.address[7])
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.set_address_register(7, value);
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
        ExecutionClass::TimeAware
    }
    fn enter_call_ceiling(&mut self, _: FunctionId) -> Result<CallCeilingScope> {
        Ok(CallCeilingScope::not_required())
    }
    fn call_original_with_execution(
        &mut self,
        id: FunctionId,
        args: &[u32],
        _: ExecutionClass,
    ) -> Result<u32> {
        self.call_original(id, args)
    }
    fn call_original_entry(&mut self, id: FunctionId, _: ExecutionClass) -> Result<u32> {
        // The double reads its own arguments without adding callee accesses to
        // the root trace being asserted. No real ROM is executed in these tests.
        let args: Vec<_> = (0..u32::from(id.metadata().abi.argument_count))
            .map(|index| {
                self.inner
                    .memory
                    .read_u32(self.frame.entry_sp + 4 + index * 4)
            })
            .collect();
        self.call_original(id, &args)
    }
    fn call_original_entry_void(&mut self, id: FunctionId, _: ExecutionClass) -> Result<()> {
        // The double reads its own arguments without adding callee accesses to
        // the root trace being asserted. No real ROM is executed in these tests.
        let args: Vec<_> = (0..u32::from(id.metadata().abi.argument_count))
            .map(|index| {
                self.inner
                    .memory
                    .read_u32(self.frame.entry_sp + 4 + index * 4)
            })
            .collect();
        self.original_effects(id, &args)
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
fn change_level_reads_requested_before_each_bank_and_restores_the_local_rng_word() {
    for r in Revision::ALL {
        for (player, first, second, saved) in
            [(0, 1, 2, 3), (0, 0, 1, 3), (0, 0, 0, 1), (1, 1, 0, 3)]
        {
            let mut c = RootMachine::new(r);
            let requested = ram::g_requested_map_level(r);
            requested.write(&mut c, 1);
            ram::g_player1_map_level(r).write(&mut c, first);
            ram::g_player2_map_level(r).write(&mut c, second);
            ram::g_saved_map_level(r).write(&mut c, saved);
            c.clear_accesses();
            assert_eq!(change_level(&mut c, &[player]).unwrap(), Some(0));
            let reads = c.reads();
            for bank in [
                ram::g_player1_map_level(r),
                ram::g_player2_map_level(r),
                ram::g_saved_map_level(r),
            ] {
                if let Some(index) = reads.iter().position(|&read| read == (bank.address(), 1)) {
                    assert_eq!(reads[index - 1], (requested.address(), 1));
                }
            }
            assert!(reads.contains(&(0xff7000 - 12, 4)));
            assert_eq!(
                c.calls().last(),
                Some(&(ids::STACK_BASED_RANDOM, vec![0x10203040]))
            );
            assert_eq!(c.inner.address[7], 0xff7000);
        }
    }
}

#[test]
fn procedural_generation_reloads_count_and_reads_anchor_x_before_y() {
    for r in Revision::ALL {
        let mut c = RootMachine::new(r);
        ram::g_requested_map_level(r).write(&mut c, 2);
        ram::g_player1_map_level(r).write(&mut c, 1);
        ram::g_player2_map_level(r).write(&mut c, 1);
        ram::g_saved_map_level(r).write(&mut c, -1);
        c.clear_accesses();
        assert_eq!(change_level(&mut c, &[0]).unwrap(), Some(0));
        let reads = c.reads();
        let count = (0xff7000 - 6, 2);
        assert_eq!(reads.iter().filter(|&&read| read == count).count(), 7);
        for index in 0..2 {
            let anchor = ram::g_level_anchor_positions(r).at(index);
            let x = reads
                .iter()
                .position(|&read| read == (anchor.x().address(), 1))
                .unwrap();
            assert_eq!(reads[x + 1], (anchor.y().address(), 1));
            assert_eq!(reads[x + 2], count);
        }
        let calls = c.calls();
        assert_eq!(calls.iter().filter(|(id, _)| *id == ids::LMULT).count(), 3);
        let minima: Vec<_> = calls
            .iter()
            .filter(|(id, _)| *id == ids::MAP_GROW_TERRAIN_REGION)
            .map(|(_, args)| args[2])
            .collect();
        assert_eq!(minima, vec![61, 122]);
        for row in 0..26 {
            for column in 0..19 {
                assert_zero_prereads(
                    &c,
                    ram::g_player1_level_map(r).at(row).at(column).address(),
                    2,
                    1,
                );
            }
        }
        assert_eq!(c.inner.address[7], 0xff7000);
    }
}

#[test]
fn cover_unpack_reads_format_then_height_then_width_and_keeps_queue_byte_rmws() {
    for r in Revision::ALL {
        let mut bytes = vec![0; 0x100000];
        let table = rom::spr_map_tile_uncover(r).address() as usize;
        bytes[table..table + 4].copy_from_slice(&0x6000_u32.to_be_bytes());
        bytes[0x6004..0x6006].copy_from_slice(&[2, 3]);
        bytes[0x6009] = 0x80;
        bytes[0x600e..0x6010].copy_from_slice(&[4, 5]);
        let mut c = RootMachine::with_rom(r, bytes);
        let sprites = ram::g_saved_level_map(r).address();
        map_upload_cover_animation_frame(&mut c, &[sprites, 0, 0]).unwrap();
        let reads = c.reads();
        for base in [0x6004, 0x600e] {
            let start = reads
                .iter()
                .position(|&read| read == (base + 5, 1))
                .unwrap();
            assert_eq!(
                &reads[start..start + 3],
                &[(base + 5, 1), (base + 1, 1), (base, 1)]
            );
        }
        let queue = ram::g_sprite_tile_upload_queue_word_count(r).address();
        assert_eq!(reads.iter().filter(|&&read| read == (queue, 1)).count(), 4);
        let calls = c.calls();
        assert_eq!(
            calls[0],
            (ids::SPRITE_UNPACK_INTERLEAVED_RLE, vec![0x6004, 6])
        );
        assert_eq!(calls[1], (ids::SPRITE_UNPACK_RLE, vec![0x600e, 20]));
        assert_eq!(ram::g_sprite_tile_buffer_offset(r).read(&c), 26 * 32);
    }
}

#[test]
fn placement_reads_local_producers_and_push_order_without_replacing_old_outputs() {
    for r in Revision::ALL {
        for success in [true, false] {
            let mut c = RootMachine::new(r);
            c.validation_succeeds = success;
            for player in 0..2 {
                ram::g_player_lives(r).at(player).write(&mut c, 0);
                ram::g_player_info(r).at(player).level().write(&mut c, 1);
            }
            let base = 0xff7000 - 4;
            c.write_i16(base - 16, 1234);
            c.write_i16(base - 14, 567);
            c.clear_accesses();
            set_position_on_new_level(&mut c, &[2]).unwrap();
            let reads = c.reads();
            let attempts = if success { 1 } else { 504 };
            assert_eq!(
                reads.iter().filter(|&&read| read == (base - 12, 4)).count(),
                attempts
            );
            let start = reads
                .iter()
                .position(|&read| read == (base - 8, 2))
                .unwrap();
            assert_eq!(
                &reads[start..start + 3],
                &[(base - 8, 2), (base - 6, 2), (base - 2, 2)]
            );
            if success {
                let accesses = c.accesses.borrow();
                let query = accesses.iter().position(|access| matches!(access, Access::Call(id, _) if *id == ids::ELEVATOR_CONTAINS_POSITION)).unwrap();
                let preceding: Vec<_> = accesses[..query]
                    .iter()
                    .filter_map(|access| match access {
                        Access::Read(address, width) => Some((*address, *width)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    &preceding[preceding.len() - 2..],
                    &[(base - 14, 2), (base - 16, 2)]
                );
            } else {
                assert_eq!(
                    reads.iter().filter(|&&read| read == (base - 4, 2)).count(),
                    520
                );
                assert_eq!(ram::g_player_info(r).at(0).x().read(&c), 1234);
                assert_eq!(ram::g_player_info(r).at(0).y().read(&c), 567);
                assert_eq!(ram::g_player_info(r).at(1).x().read(&c), 1298);
            }
            assert_eq!(c.inner.address[7], 0xff7000);
        }
    }
}

#[test]
fn map_display_reads_mode_before_releasing_the_retained_dma_batch() {
    for r in Revision::ALL {
        let mut c = RootMachine::new(r);
        let idle = ram::g_player_idle_ticks(r).at(0);
        idle.write(&mut c, 0x1234);
        c.clear_accesses();
        draw_game_map(&mut c, &[0]).unwrap();
        assert_zero_prereads(&c, idle.address(), 2, usize::from(r == Revision::Rev02));
        assert_eq!(idle.read(&c), if r == Revision::Rev02 { 0 } else { 0x1234 });
        let accesses = c.accesses.borrow();
        let dma = accesses.iter().position(|access| matches!(access, Access::Call(id, _) if *id == ids::DMA_COPY_WORDS_TO_VRAM)).unwrap();
        let mode = ram::g_mode2_reg_softcopy(r).address();
        assert_eq!(accesses[dma + 1], Access::Stack(0xff7000 - 664 - 84));
        assert_eq!(accesses[dma + 2], Access::Read(mode, 1));
        assert_eq!(accesses[dma + 3], Access::Write(mode, 1, 0x40));
        assert_eq!(accesses[dma + 4], Access::Read(mode, 1));
        assert_eq!(accesses[dma + 5], Access::Stack(0xff7000 - 664));
    }
}

#[test]
fn map_sprite_recovery_keeps_missing_third_allocation_and_unwritten_x_fields() {
    for r in Revision::ALL {
        for allocations in [vec![9], vec![0, 11], vec![0, 0]] {
            let mut c = RootMachine::new(r);
            c.allocations = allocations.iter().copied().collect();
            let sprites =
                tje_mem::Address::<HardwareSprite>::new(ram::g_saved_level_map(r).address());
            for index in 0..80 {
                sprites.offset(index).x().write(&mut c, 0x1234);
            }
            c.clear_accesses();
            map_initialize_screen_sprites(&mut c, &[sprites.address()]).unwrap();
            let calls = c.calls();
            assert_eq!(
                calls
                    .iter()
                    .filter(|(id, _)| *id == ids::ALLOCATE_SPRITE_TILE_RUN)
                    .count(),
                allocations.len()
            );
            let first = *allocations.last().unwrap();
            assert_eq!(
                calls.last(),
                Some(&(ids::FREE_SPRITE_TILE_RUN, vec![first, 8]))
            );
            assert_eq!(
                calls
                    .iter()
                    .filter(|(id, _)| *id == ids::ENTITY_CLEAR_VIEWPORT_SPRITES)
                    .count(),
                allocations.iter().filter(|&&value| value == 0).count()
            );
            for index in 0..7 {
                assert_zero_prereads(&c, sprites.offset(index).size().address(), 1, 1);
            }
            assert_zero_prereads(&c, sprites.offset(79).link().address(), 1, 1);
            for index in 0..80 {
                assert_eq!(sprites.offset(index).x().read(&c), 0x1234);
                assert_eq!(sprites.offset(index).y().read(&c), 1);
            }
            assert_eq!(sprites.offset(0).tile().read(&c), first as u16);
            assert_eq!(sprites.offset(79).link().read(&c), 0);
        }
    }
}

#[test]
fn marker_blink_waits_then_reads_only_visible_y_words() {
    for r in Revision::ALL {
        for counter in [0, 8, 16, 24] {
            let mut c = RootMachine::new(r);
            let sprites =
                tje_mem::Address::<HardwareSprite>::new(ram::g_saved_level_map(r).address());
            ram::g_frame_counter(r).write(&mut c, counter);
            for index in 0..3 {
                sprites.offset(index).y().write(&mut c, 200 + index as i16);
            }
            c.clear_accesses();
            draw_map_characters(&mut c, &[sprites.address()]).unwrap();
            assert_eq!(c.calls(), vec![(ids::WAIT_FOR_NEXT_V_BLANK, vec![1])]);
            let mut expected = vec![(ram::g_frame_counter(r).address(), 2)];
            if counter != 0 {
                expected.extend((0..3).map(|index| (sprites.offset(index).y().address(), 2)));
            }
            assert_eq!(c.reads(), expected);
            let data: Vec<_> = c
                .accesses
                .borrow()
                .iter()
                .filter_map(|access| match access {
                    Access::Vdp(0xc00000, value) => Some(*value),
                    _ => None,
                })
                .collect();
            assert_eq!(
                data,
                if counter == 0 {
                    vec![1; 3]
                } else {
                    vec![200, 201, 202]
                }
            );
        }
    }
}

#[test]
fn player_level_loader_keeps_dead_player_override_retry_and_final_short_circuit() {
    for r in Revision::ALL {
        let mut c = RootMachine::new(r);
        ram::g_player_lives(r).at(0).write(&mut c, -1);
        c.load_rejections = 1;
        map_load_player_level(&mut c, &[1, 2]).unwrap();
        assert_eq!(
            c.calls(),
            vec![
                (ids::CHANGE_LEVEL, vec![0]),
                (ids::GET_RANDOM_NUMBER, vec![]),
                (ids::CHANGE_LEVEL, vec![0])
            ]
        );
        assert_eq!(ram::g_requested_map_level(r).read(&c), 2);
        assert_eq!(ram::g_player1_map_level(r).read(&c), -1);
        c.clear_accesses();
        ram::g_player1_map_level(r).write(&mut c, 26);
        map_load_player_level(&mut c, &[1, 26]).unwrap();
        assert!(c.calls().is_empty());
        map_load_player_level(&mut c, &[1, 27]).unwrap();
        assert_eq!(c.calls(), vec![(ids::LOAD_FINAL_LEVEL, vec![27])]);
        assert_eq!(c.inner.address[7], 0xff7000);
    }
}

#[test]
fn transition_assignment_clears_only_transition_bits_and_exhausts_both_passes() {
    for r in Revision::ALL {
        let mut c = RootMachine::new(r);
        let map = ram::g_player1_level_map(r);
        for row in 0..26 {
            for column in 0..19 {
                map.at(row).at(column).write(&mut c, 0xffff);
            }
        }
        c.clear_accesses();
        map_assign_transition_chain(&mut c, &[]).unwrap();
        let minimum = ram::g_transition_path_minimum_visits(r).address();
        let writes: Vec<_> = c
            .accesses
            .borrow()
            .iter()
            .filter_map(|access| match access {
                Access::Write(address, 2, value) if *address == minimum => Some(*value),
                _ => None,
            })
            .collect();
        assert_eq!(writes, vec![25, 16, 16]);
        assert_eq!(c.calls().len(), 988);
        assert!(
            c.calls()
                .iter()
                .all(|call| *call == (ids::MAP_TRANSITION_CONTAINS_METATILE, vec![1, 0xe0ff]))
        );
        assert_zero_prereads(&c, ram::g_transition_path_visit_count(r).address(), 2, 988);
        for row in 0..26 {
            for column in 0..19 {
                assert_eq!(map.at(row).at(column).read(&c), 0xe0ff);
            }
        }
    }
}

fn assert_zero_prereads(c: &RootMachine, address: u32, width: u8, expected: usize) {
    let accesses = c.accesses.borrow();
    let writes: Vec<_> = accesses
        .iter()
        .enumerate()
        .filter_map(|(index, access)| {
            (*access == Access::Write(address, width, 0)).then_some(index)
        })
        .collect();
    assert_eq!(writes.len(), expected);
    for index in writes {
        assert_eq!(accesses[index - 1], Access::Discard(address, width));
    }
}

#[test]
fn rejected_generation_returns_one_without_restoring_the_rng() {
    for r in Revision::ALL {
        let mut c = RootMachine::new(r);
        c.reject_generation = true;
        ram::g_requested_map_level(r).write(&mut c, 2);
        ram::g_player1_map_level(r).write(&mut c, 1);
        ram::g_player2_map_level(r).write(&mut c, 1);
        ram::g_saved_map_level(r).write(&mut c, -1);
        assert_eq!(change_level(&mut c, &[0]).unwrap(), Some(1));
        assert_eq!(
            c.calls()
                .iter()
                .filter(|(id, _)| *id == ids::STACK_BASED_RANDOM)
                .count(),
            1
        );
        assert_eq!(c.inner.address[7], 0xff7000);
    }
}

fn assert_pending_polls(c: &RootMachine, samples: &[u8], outer_test: bool) {
    let flag = ram::g_sprite_graphics_upload_pending(c.revision()).address();
    let frame_counter = ram::g_frame_counter(c.revision()).address();
    let accesses = c.accesses.borrow();
    let pending: Vec<_> = accesses
        .iter()
        .filter(|access| match access {
            Access::Read(address, _) => *address == flag || *address == (flag & !1),
            Access::Wait(_) => true,
            _ => false,
        })
        .cloned()
        .collect();
    let mut expected = Vec::new();
    let remaining = if outer_test {
        expected.push(Access::Read(flag, 1));
        &samples[1..]
    } else {
        samples
    };
    if !outer_test || samples[0] != 0 {
        expected.push(Access::Wait(WaitRequest::ram_byte_equal(flag, 0xff, 0)));
    }
    expected.extend(remaining.iter().map(|_| Access::Read(flag, 1)));
    assert_eq!(pending, expected);
    assert!(
        !accesses
            .iter()
            .any(|access| matches!(access, Access::Read(address, _) if *address == frame_counter))
    );
    let pending_samples = c.pending_samples.borrow();
    assert!(pending_samples.as_ref().unwrap().1.is_empty());
}

#[test]
fn level_pending_wait_owns_all_byte_samples_including_the_final_zero() {
    for r in Revision::ALL {
        for samples in [&[0][..], &[0x80, 0x80, 0], &[0xff, 1, 0x40, 0]] {
            let mut c = RootMachine::new(r);
            ram::g_requested_map_level(r).write(&mut c, 2);
            ram::g_player1_map_level(r).write(&mut c, 1);
            ram::g_player2_map_level(r).write(&mut c, 1);
            ram::g_saved_map_level(r).write(&mut c, -1);
            c.pending_samples(samples);
            c.clear_accesses();
            assert_eq!(change_level(&mut c, &[0]).unwrap(), Some(0));
            assert_pending_polls(&c, samples, false);
        }
    }
}

#[test]
fn map_pending_wait_keeps_the_outer_test_without_duplicating_inner_polls() {
    for r in Revision::ALL {
        for samples in [&[0][..], &[0x80, 0], &[0xff, 0x80, 1, 0]] {
            let mut c = RootMachine::new(r);
            c.pending_samples(samples);
            draw_game_map(&mut c, &[0]).unwrap();
            assert_pending_polls(&c, samples, true);
            let first_call = c.calls().into_iter().next().unwrap();
            assert_eq!(
                first_call,
                if samples[0] == 0 {
                    (ids::WAIT_FOR_NEXT_V_BLANK, vec![1])
                } else {
                    (ids::DISABLE_BLANKING_GRAPHICS, vec![])
                }
            );
        }
    }
}
