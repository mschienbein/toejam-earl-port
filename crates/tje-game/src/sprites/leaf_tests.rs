use super::{frame_rendering, lifecycle, links, queue_update, rendering, slots, unpack, updates};
use crate::{Context, Dispatcher, FunctionId, Result};
use tje_mem::{Address, AddressSpace, Bus, Platform, Ptr, RenderPiece, Revision, ram, rom};

struct Machine {
    memory: AddressSpace,
    data: [u32; 8],
    address: [u32; 7],
    sr: u16,
    writes: Vec<(u8, u16)>,
}
impl Machine {
    fn new(revision: Revision) -> Self {
        Self {
            memory: AddressSpace::new(vec![0; 0x100000], revision),
            data: [0xa5a5_1234; 8],
            address: [0; 7],
            sr: 0x271f,
            writes: Vec::new(),
        }
    }
    fn buffer(&self) -> u32 {
        ram::g_sprite_tile_upload_buffer(self.revision()).address()
    }
    fn piece(&mut self) -> Address<RenderPiece> {
        let piece = Address::<RenderPiece>::new(self.buffer() + 7680);
        piece
            .tile_data()
            .write(self, Ptr::new(self.buffer() + 7800));
        piece
    }
}
impl Bus for Machine {
    fn read_u8(&self, address: u32) -> u8 {
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
    }
}
impl Platform for Machine {
    fn write_vdp_control(&mut self, value: u16) {
        self.writes.push((4, value));
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.writes.push((0, value));
    }
}
impl crate::Platform for Machine {
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("unexpected byte device read")
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("unexpected word device read")
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("unexpected byte device write")
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("unexpected word device write")
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
        self.sr
    }
    fn set_status_register(&mut self, value: u16) {
        self.sr = value;
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid()
    }
    fn call_original(&mut self, function: FunctionId, _: &[u32]) -> Result<u32> {
        Err(crate::Error::Untranslated(function))
    }
}

#[test]
fn rle_modes_overrun_and_zero_quotas_preserve_register_contract() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut machine = Machine::new(revision);
        let piece = machine.piece();
        let buffer = machine.buffer();
        let stream = piece.tile_data().read(&machine).address();
        let data = [0xf8, 0x12, 0xb8, 0x48, 8, 1, 2, 3, 4, 5, 6, 7, 8];
        for (index, value) in data.into_iter().enumerate() {
            machine.write_u8(stream + index as u32, value);
        }
        let before = machine.data;
        unpack::sprite_unpack_rle(&mut machine, &[piece.address(), 1]).unwrap();
        let result: Vec<_> = (0..32)
            .map(|index| machine.read_u8(buffer + index))
            .collect();
        assert_eq!(&result[..8], &[0x12; 8]);
        assert_eq!(&result[8..16], &[0; 8]);
        assert_eq!(&result[16..24], &[0xff; 8]);
        assert_eq!(&result[24..], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(machine.data[0], before[0]);
        assert_eq!(machine.data[1], 0xa5a5_0008);
        assert_eq!(&machine.data[2..], &before[2..]);
        assert_eq!(machine.address[0], buffer + 32);
        assert_eq!(machine.address[1], stream + data.len() as u32);
        assert_eq!(machine.sr, 0x2704);
        machine.write_u8(stream, 0xdf);
        machine.write_u8(stream + 1, 0x98);
        unpack::sprite_unpack_rle(&mut machine, &[piece.address(), 1]).unwrap();
        assert_eq!(machine.address[0], buffer + 33);
        assert_eq!(machine.sr, 0x2718); // SUB.W borrows, then TST.W sees -1.
        unpack::sprite_unpack_rle(&mut machine, &[piece.address(), 2048]).unwrap();
        assert_eq!(machine.address[0], buffer);
        assert_eq!(machine.sr, 0x2714); // ASL.W's last outgoing bit survives as X.
    }
}

#[test]
fn interleaved_lane_modes_and_odd_alternating_runs() {
    let mut machine = Machine::new(Revision::Rev02);
    let piece = machine.piece();
    let buffer = machine.buffer();
    let stream = piece.tile_data().read(&machine).address();
    let data = [
        0x63, 0x12, 0xfb, 0x34, 0xb8, 0x48, 8, 1, 2, 3, 4, 5, 6, 7, 8,
    ];
    for (index, value) in data.into_iter().enumerate() {
        machine.write_u8(stream + index as u32, value);
    }
    unpack::sprite_unpack_interleaved_rle(&mut machine, &[piece.address(), 1]).unwrap();
    for index in 0..8 {
        assert_eq!(
            machine.read_u8(buffer + index * 4),
            [0x12, 0x21, 0x12, 0x34, 0x34, 0x34, 0x34, 0x34][index as usize]
        );
        assert_eq!(machine.read_u8(buffer + index * 4 + 1), 0);
        assert_eq!(machine.read_u8(buffer + index * 4 + 2), 0xff);
        assert_eq!(machine.read_u8(buffer + index * 4 + 3), (index + 1) as u8);
    }
    assert_eq!(machine.data[0], 0xa5a5_0004);
    assert_eq!(machine.data[1], 0xa5a5_0000);
    assert_eq!(machine.address[0], buffer + 4);
    assert_eq!(machine.address[1], stream + data.len() as u32);
    assert_eq!(machine.sr, 0x2704);
}

#[test]
fn raw_copy_and_append_keep_forward_aliasing_and_word_count_wrap() {
    let mut machine = Machine::new(Revision::Rev00);
    let buffer = machine.buffer();
    let piece = machine.piece();
    piece.tile_data().write(&mut machine, Ptr::new(buffer));
    machine.write_u32(buffer, 0x89ab_cdef);
    ram::g_sprite_tile_buffer_offset(Revision::Rev00).write(&mut machine, 4);
    unpack::sprite_unpack_raw(&mut machine, &[piece.address(), 1]).unwrap();
    for index in 0..9 {
        assert_eq!(machine.read_u32(buffer + index * 4), 0x89ab_cdef);
    }
    assert_eq!(machine.data[0], 0xa5a5_ffff);
    assert_eq!(machine.data[1], 0xa5a5_0004);
    assert_eq!(machine.sr, 0x2708);
    let before = machine.read_u32(buffer + 36);
    rendering::append_sprite_tiles_to_buffer(&mut machine, &[buffer, 4096]).unwrap();
    assert_eq!(machine.read_u32(buffer + 36), before);
    assert_eq!(
        ram::g_sprite_tile_buffer_offset(Revision::Rev00).read(&machine),
        4
    );
}

#[test]
fn clipping_masks_only_boundary_and_rows_below_it() {
    let mut machine = Machine::new(Revision::Rev02);
    let piece = machine.piece();
    let buffer = machine.buffer();
    piece.width().write(&mut machine, 2);
    piece.height().write(&mut machine, 1);
    piece.y().write(&mut machine, 0);
    for index in 0..64 {
        machine.write_u8(buffer + index, 0xff);
    }
    assert_eq!(
        rendering::clip_sprite_pixels_below_cutoff(&mut machine, &[piece.address(), u32::MAX])
            .unwrap(),
        Some(0)
    );
    for column in [0, 32] {
        assert_eq!(machine.read_u32(buffer + column), u32::MAX);
        assert_eq!(machine.read_u32(buffer + column + 4), 0x0f00_0ff0);
        for row in 2..8 {
            assert_eq!(machine.read_u32(buffer + column + row * 4), 0);
        }
    }
    piece.y().write(&mut machine, 127);
    assert_eq!(
        rendering::clip_sprite_pixels_below_cutoff(
            &mut machine,
            &[piece.address(), i16::MIN as i32 as u32],
        )
        .unwrap(),
        Some(1),
        "SUB.W overflow still selects the original nonnegative branch",
    );
    for (index, value) in [0xff, 0xf0, 0x0f, 0x12, 1, 0xfe].into_iter().enumerate() {
        machine.write_u8(buffer + index as u32, value);
    }
    rendering::clear_sprite_mask_pixels(&mut machine, &[6]).unwrap();
    assert_eq!(
        (0..6)
            .map(|index| machine.read_u8(buffer + index))
            .collect::<Vec<_>>(),
        [0, 0, 0, 0x12, 1, 0xfe]
    );
}

#[test]
fn allocation_zero_count_and_reset_extent_retain_original_behavior() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    assert_eq!(
        slots::allocate_sprite_tile_run(&mut machine, &[0]).unwrap(),
        Some(640)
    );
    assert_eq!(
        slots::allocate_sprite_tile_run(&mut machine, &[9]).unwrap(),
        Some(648)
    );
    assert_eq!(
        ram::g_sprite_tile_group_allocated(revision)
            .at(2)
            .read(&machine),
        1
    );
    slots::free_sprite_tile_run(&mut machine, &[648, 9]).unwrap();
    assert_eq!(
        ram::g_sprite_tile_group_allocated(revision)
            .at(0)
            .read(&machine),
        1
    );
    assert_eq!(
        ram::g_sprite_tile_group_allocated(revision)
            .at(1)
            .read(&machine),
        0
    );
    let entity = ram::g_player_info(revision).at(5);
    for index in 0..128 {
        machine.write_u8(entity.address() + index, 0xa5);
    }
    slots::reset_entity_runtime_state(&mut machine, &[entity.address()]).unwrap();
    for index in 0..128 {
        assert_eq!(
            machine.read_u8(entity.address() + index),
            if index < 94 { 0 } else { 0xa5 }
        );
    }
}

#[test]
fn sort_is_stable_and_late_entities_follow_every_normal_entity() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    for (index, y, flags) in [(0, 5, 1), (1, 5, 1), (2, 10, 1), (3, 300, 0x81)] {
        let entity = ram::g_player_info(revision).at(index);
        entity.y().write(&mut machine, y);
        entity.draw_flags().write(&mut machine, flags);
    }
    rendering::sort_entities_for_sprite_draw(&mut machine, &[]).unwrap();
    let mut next = ram::g_entity_draw_list_head(revision).read(&machine);
    for index in [2, 0, 1, 3] {
        assert_eq!(
            next.address(),
            ram::g_player_info(revision).at(index).address()
        );
        next = next.target().next().read(&machine);
    }
    assert_eq!(next.address(), 0);
}

#[test]
fn vdp_chain_selection_sign_extends_mirror_link_bytes() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    ram::g_primary_viewport_uploaded_link(revision).write(&mut machine, 0x80);
    links::select_primary_viewport_sprites(&mut machine, &[]).unwrap();
    assert_eq!(
        machine.writes,
        [
            (4, 0x78c2),
            (4, 3),
            (0, 79),
            (4, 0x78ba),
            (4, 3),
            (0, 0xff80)
        ]
    );
}

#[test]
fn descriptor_reads_are_revision_selected_and_negative_state_is_noop() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut machine = Machine::new(revision);
        let definition = rom::player_animation_descriptors(revision).at(4);
        let base = definition.address() as usize;
        let mut rom_bytes = vec![0; 0x100000];
        rom_bytes[base..base + 4].copy_from_slice(&0x123456u32.to_be_bytes());
        rom_bytes[base + 4..base + 8].copy_from_slice(&0x789abcu32.to_be_bytes());
        rom_bytes[base + 8] = 0xfe;
        machine.memory = AddressSpace::new(rom_bytes, revision);
        let entity = ram::g_player_info(revision).at(0);
        rendering::set_sprite(&mut machine, &[entity.address(), 4]).unwrap();
        assert_eq!(entity.sequence().read(&machine).address(), 0x123456);
        assert_eq!(entity.frames().read(&machine).address(), 0x789abc);
        assert_eq!(entity.frame_phase().read(&machine), -2);
        assert_eq!(entity.frame().read(&machine), -1);
        assert_eq!(entity.animation_group_mask().read(&machine), 2);
        let before = machine.memory.ram().to_vec();
        rendering::set_sprite(&mut machine, &[entity.address(), u32::MAX]).unwrap();
        assert_eq!(machine.memory.ram().as_slice(), before);
    }
}

#[test]
fn visibility_uses_signed_emitted_bounds_and_distinct_split_view_height() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut machine = Machine::new(revision);
        let entity = ram::g_player_info(revision).at(0);
        entity.x().write(&mut machine, -10);
        entity.y().write(&mut machine, -9);
        entity.width().write(&mut machine, 10);
        let arguments = [entity.address(), 0];
        assert_eq!(
            rendering::is_entity_visible_in_viewport(&mut machine, &arguments).unwrap(),
            Some(1)
        );
        entity.x().write(&mut machine, -11);
        assert_eq!(
            rendering::is_entity_visible_in_viewport(&mut machine, &arguments).unwrap(),
            Some(0)
        );
        entity.x().write(&mut machine, 0);
        entity.y().write(&mut machine, 113);
        assert_eq!(
            rendering::is_entity_visible_in_viewport(&mut machine, &arguments).unwrap(),
            Some(1)
        );
        ram::g_split_screen_enabled(revision).write(&mut machine, 1);
        assert_eq!(
            rendering::is_entity_visible_in_viewport(&mut machine, &arguments).unwrap(),
            Some(0)
        );
        entity.y().write(&mut machine, 112);
        assert_eq!(
            rendering::is_entity_visible_in_viewport(&mut machine, &[entity.address(), 1]).unwrap(),
            Some(1)
        );
        ram::g_player2_inventory_open(revision).write(&mut machine, 1);
        assert_eq!(
            rendering::is_entity_visible_in_viewport(&mut machine, &[entity.address(), 1]).unwrap(),
            Some(0)
        );
    }
}

#[test]
fn flagged_hiding_prefers_primary_list_and_does_not_mutate_the_ram_table() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    let entity = ram::g_player_info(revision).at(0);
    entity.behavior_flags().write(&mut machine, 0x10);
    entity.draw_flags().write(&mut machine, 6);
    entity.sprite_ready().write(&mut machine, 1);
    entity.sprite_count().write(&mut machine, 1);
    entity.primary_sprites().at(0).write(&mut machine, 25);
    entity.secondary_sprites().at(0).write(&mut machine, 78);
    let before = machine.memory.ram().to_vec();
    links::hide_flagged_player_sprites(&mut machine, &[0]).unwrap();
    assert_eq!(machine.writes, [(4, 0x78c8), (4, 3), (0, 1)]);
    assert_eq!(machine.memory.ram().as_slice(), before);
}

#[test]
fn entity_allocation_respects_gameplay_limit_and_keeps_stable_index() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    let flags = ram::g_dynamic_entity_slot_allocated(revision);
    for index in 0..34 {
        flags.at(index).write(&mut machine, 1);
    }
    flags.at(20).write(&mut machine, 0);
    assert_eq!(
        lifecycle::allocate_entity(&mut machine, &[]).unwrap(),
        Some(0)
    );
    ram::g_player1_map_level(revision).write(&mut machine, 26);
    let entity = ram::g_player_info(revision).at(26);
    entity.entity_index().write(&mut machine, 26);
    entity.x().write(&mut machine, 123);
    assert_eq!(
        lifecycle::allocate_entity(&mut machine, &[]).unwrap(),
        Some(26)
    );
    assert_eq!(entity.x().read(&machine), 0);
    assert_eq!(entity.draw_flags().read(&machine), 1);
    assert_eq!(entity.entity_index().read(&machine), 26);
}

#[test]
fn viewport_release_reclaims_only_last_unshared_view_and_its_shadow() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    let entity = ram::g_player_info(revision).at(6);
    entity.draw_flags().write(&mut machine, 3);
    entity.tile().write(&mut machine, 640);
    entity.tile_count().write(&mut machine, 8);
    entity.sprite_count().write(&mut machine, 1);
    entity.primary_sprites().at(0).write(&mut machine, 25);
    entity.primary_shadow().write(&mut machine, 29);
    ram::g_viewport_sprite_slot_flags(revision)
        .at(0)
        .write(&mut machine, 3);
    ram::g_viewport_sprite_slot_flags(revision)
        .at(4)
        .write(&mut machine, 1);
    ram::g_sprite_tile_group_allocated(revision)
        .at(0)
        .write(&mut machine, 1);
    lifecycle::release_entity_viewport_sprites(&mut machine, &[entity.address(), 0]).unwrap();
    assert_eq!(entity.draw_flags().read(&machine), 1);
    assert_eq!(entity.primary_shadow().read(&machine), 0);
    assert_eq!(
        ram::g_viewport_sprite_slot_flags(revision)
            .at(0)
            .read(&machine),
        0
    );
    assert_eq!(
        ram::g_viewport_sprite_slot_flags(revision)
            .at(4)
            .read(&machine),
        0
    );
    assert_eq!(
        ram::g_sprite_tile_group_allocated(revision)
            .at(0)
            .read(&machine),
        0
    );
}

#[test]
fn frame_preparation_masks_pixels_then_appends_the_original_tile_run() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    let buffer = machine.buffer();
    let frame = Address::<tje_mem::RenderFrame>::new(buffer + 7168);
    let piece = Address::<RenderPiece>::new(frame.address() + 4);
    let source = buffer + 7900;
    frame.piece_count().write(&mut machine, 1);
    piece.width().write(&mut machine, 1);
    piece.height().write(&mut machine, 1);
    piece.tile_format().write(&mut machine, 2);
    piece.tile_data().write(&mut machine, Ptr::new(source));
    machine.write_u8(source, 0x60);
    let entity = ram::g_player_info(revision).at(6);
    entity.entity_index().write(&mut machine, 6);
    entity.tile().write(&mut machine, 640);
    entity.behavior_flags().write(&mut machine, 0x20);
    entity.primary_sprites().at(0).write(&mut machine, 25);
    frame_rendering::prepare_entity_sprite_frame(
        &mut machine,
        &[entity.address(), frame.address(), 0, 0],
    )
    .unwrap();
    assert!((0..32).all(|index| machine.read_u8(buffer + index) == 0));
    assert_eq!(
        ram::g_sprite_tile_buffer_offset(revision).read(&machine),
        32
    );
    assert_eq!(
        ram::g_sprite_tile_upload_queue_word_count(revision).read(&machine),
        2
    );
    assert_eq!(
        ram::g_sprite_tile_upload_runs(revision)
            .at(0)
            .read(&machine),
        640
    );
    assert_eq!(
        ram::g_sprite_tile_upload_runs(revision)
            .at(1)
            .read(&machine),
        1
    );
    assert_eq!(
        ram::g_hardware_sprite_table(revision)
            .at(25)
            .tile()
            .read(&machine),
        0x4280
    );
}

#[test]
fn shadow_depth_zero_keeps_the_original_preceding_rom_byte_read() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    let entity = ram::g_player_info(revision).at(6);
    entity.draw_flags().write(&mut machine, 3);
    entity.primary_shadow().write(&mut machine, 29);
    entity.height().write(&mut machine, 0);
    updates::update_entity_shadows(&mut machine, &[entity.address()]).unwrap();
    let hardware = ram::g_hardware_sprite_table(revision).at(29);
    assert_eq!(hardware.size().read(&machine), 0x7c);
    assert_eq!(hardware.tile().read(&machine), 0xe7a0);
    entity.height().write(&mut machine, 127);
    updates::update_entity_shadows(&mut machine, &[entity.address()]).unwrap();
    assert_eq!(hardware.size().read(&machine), 0);
    assert_eq!(hardware.tile().read(&machine), 0xe7a0);
    entity.height().write(&mut machine, -1);
    updates::update_entity_shadows(&mut machine, &[entity.address()]).unwrap();
    assert_eq!(hardware.size().read(&machine), 12);
    assert_eq!(hardware.tile().read(&machine), 0x87f9);
}

#[test]
fn entity_release_retains_players_and_frees_the_dynamic_flag() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    let player = ram::g_player_info(revision).at(0);
    player.draw_flags().write(&mut machine, 7);
    updates::free_entity(&mut machine, &[0]).unwrap();
    assert_eq!(player.draw_flags().read(&machine), 7);
    let entity = ram::g_player_info(revision).at(6);
    entity.draw_flags().write(&mut machine, 1);
    ram::g_dynamic_entity_slot_allocated(revision)
        .at(0)
        .write(&mut machine, 1);
    updates::free_entity(&mut machine, &[6]).unwrap();
    assert_eq!(entity.draw_flags().read(&machine), 0);
    assert_eq!(
        ram::g_dynamic_entity_slot_allocated(revision)
            .at(0)
            .read(&machine),
        0
    );
}

#[test]
fn original_not_ready_budget_bug_suppresses_the_later_motion_callback() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    for index in 0..4 {
        ram::g_speech_bubble_entities(revision)
            .at(index)
            .write(&mut machine, -1);
    }
    let first = ram::g_player_info(revision).at(0);
    first.draw_flags().write(&mut machine, 0x37);
    first.tile_count().write(&mut machine, 255);
    first.frame_duration().write(&mut machine, 2);
    first.sprite_ready().write(&mut machine, 0);
    let second = ram::g_player_info(revision).at(1);
    second.draw_flags().write(&mut machine, 7);
    second.tile_count().write(&mut machine, 1);
    second.frame_duration().write(&mut machine, 1);
    second.animation_motion_axes().write(&mut machine, 3);
    second.process().write(
        &mut machine,
        Ptr::new(
            crate::ids::PLAYER1_PROCESS_TERRAIN_MOTION
                .metadata()
                .address(revision),
        ),
    );
    // The original adds the over-budget 255 first, so no callback is invoked.
    updates::process_entity_animation_motion(&mut machine, &[]).unwrap();
    assert_eq!(first.draw_flags().read(&machine), 0x37);
}

#[test]
fn cutoff_subtraction_branch_keeps_v_for_every_signed_word_input() {
    let mut machine = Machine::new(Revision::Rev02);
    let piece = machine.piece();
    piece.width().write(&mut machine, 1);
    piece.height().write(&mut machine, 1);
    piece.y().write(&mut machine, 127);
    for bits in 0..=u16::MAX {
        let cutoff = bits as i16;
        let result = rendering::clip_sprite_pixels_below_cutoff(
            &mut machine,
            &[piece.address(), cutoff as i32 as u32],
        )
        .unwrap();
        assert_eq!(
            result,
            Some(u32::from(125 >= i32::from(cutoff))),
            "cutoff={cutoff}"
        );
    }
}

#[test]
fn guarded_frame_duration_predecrement_matches_every_byte_and_zero_test() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    let entity = ram::g_player_info(revision).at(39);
    for bits in 0..=u8::MAX {
        let old = bits as i8;
        entity.frame_duration().write(&mut machine, old);
        let expired = queue_update::tick_frame_duration(&mut machine, entity);
        assert_eq!(expired, old == 1, "old={old}");
        assert_eq!(
            entity.frame_duration().read(&machine),
            if old == 0 { 0 } else { old.wrapping_sub(1) },
            "old={old}"
        );
    }
}

#[test]
fn short_player_selectors_wrap_the_word_offset_before_address_addition() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    let player = ram::g_player_info(revision).at(0);
    player.draw_flags().write(&mut machine, 3);
    player.behavior_flags().write(&mut machine, 0x10);
    player.sprite_ready().write(&mut machine, 1);
    player.sprite_count().write(&mut machine, 1);
    player.primary_sprites().at(0).write(&mut machine, 25);
    for selector in [512i16, -512, 1024, -1024] {
        machine.writes.clear();
        links::hide_flagged_player_sprites(&mut machine, &[selector as i32 as u32]).unwrap();
        assert_eq!(
            machine.writes,
            [(4, 0x78c8), (4, 3), (0, 1)],
            "selector={selector}"
        );
    }
    // FreeEntity's player guard uses the input number, not the aliased pointer.
    player.draw_flags().write(&mut machine, 1);
    ram::g_dynamic_entity_slot_allocated(revision)
        .at(506)
        .write(&mut machine, 1);
    updates::free_entity(&mut machine, &[512]).unwrap();
    assert_eq!(player.draw_flags().read(&machine), 0);
    assert_eq!(
        ram::g_dynamic_entity_slot_allocated(revision)
            .at(506)
            .read(&machine),
        0
    );
}

#[test]
fn int_entity_selector_does_not_reuse_the_short_selector_word_wrap() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    let player = ram::g_player_info(revision).at(0);
    player.draw_flags().write(&mut machine, 1);
    let before = machine.memory.ram().to_vec();
    updates::update_entity_viewport_sprites(&mut machine, &[512]).unwrap();
    assert_eq!(machine.memory.ram().as_slice(), before);
    assert_eq!(machine.data[4], 0xa5a5_1234);
}

#[test]
fn int_slot_query_keeps_its_long_index_even_after_a_full_word_of_slots() {
    let revision = Revision::Rev02;
    let mut machine = Machine::new(revision);
    ram::g_viewport_sprite_slot_flags(revision)
        .at(0)
        .write(&mut machine, 2);
    assert_eq!(
        slots::is_sprite_slot_locked(&mut machine, &[25]).unwrap(),
        Some(1)
    );
    // The full longword displacement reaches synthetic ROM, not the RAM flag alias.
    assert_eq!(
        slots::is_sprite_slot_locked(&mut machine, &[25 + 65536]).unwrap(),
        Some(0)
    );
}
