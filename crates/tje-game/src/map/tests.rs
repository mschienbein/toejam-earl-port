use super::*;
use crate::{Dispatcher, Error, FunctionId};
use tje_mem::{AddressSpace, Bus, HardwareSprite, Revision, rom};

struct Machine {
    memory: AddressSpace,
    data: [u32; 8],
    address: [u32; 8],
    status: u16,
    writes: Vec<(u32, u16)>,
}
impl Machine {
    fn new(revision: Revision) -> Self {
        Self::with_rom(revision, vec![0; 0x100000])
    }
    fn with_rom(revision: Revision, bytes: Vec<u8>) -> Self {
        Self {
            memory: AddressSpace::new(bytes, revision),
            data: [0; 8],
            address: [0; 8],
            status: 0,
            writes: Vec::new(),
        }
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
impl tje_mem::Platform for Machine {
    fn write_vdp_control(&mut self, value: u16) {
        self.writes.push((0xc00004, value));
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.writes.push((0xc00000, value));
    }
}
impl crate::Platform for Machine {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        panic!("unexpected byte device read {address:08x}");
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        panic!("unexpected word device read {address:08x}");
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.writes.push((address, u16::from(value)));
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.writes.push((address, value));
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
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid()
    }
    fn call_original(&mut self, id: FunctionId, _: &[u32]) -> Result<u32> {
        Err(Error::Untranslated(id))
    }
}

#[test]
fn bank_words_and_packed_directions_round_trip() {
    for revision in Revision::ALL {
        let mut c = Machine::new(revision);
        let a = ram::g_player1_level_map(revision).at(0).at(0);
        let b = ram::g_player2_level_map(revision).at(0).at(0);
        let saved = ram::g_saved_level_map(revision).at(0).at(0);
        let directions = ram::g_map_path_direction_banks(revision).at(0).at(0);
        let saved_directions = ram::g_saved_map_path_directions(revision).at(0).at(0);
        a.write(&mut c, 0x1234);
        b.write(&mut c, 0x5678);
        saved.write(&mut c, 0xabcd);
        directions.write(&mut c, 0xa5);
        saved_directions.write(&mut c, 0x3c);
        map_swap_player_banks(&mut c, &[]).unwrap();
        assert_eq!(
            (a.read(&c), b.read(&c), directions.read(&c)),
            (0x5678, 0x1234, 0x5a)
        );
        copy_level_between_players(&mut c, &[0]).unwrap();
        assert_eq!((b.read(&c), directions.read(&c)), (0x5678, 0xaa));
        ram::g_player2_map_level(revision).write(&mut c, 9);
        map_swap_player_with_saved_bank(&mut c, &[1]).unwrap();
        assert_eq!((b.read(&c), saved.read(&c)), (0xabcd, 0x5678));
        assert_eq!(
            (directions.read(&c), saved_directions.read(&c)),
            (0xca, 0x0a)
        );
        assert_eq!(ram::g_saved_map_level(revision).read(&c), 9);
    }
}

#[test]
fn widening_keeps_the_label_across_a_gap() {
    for revision in Revision::ALL {
        for vertical in [false, true] {
            let mut c = Machine::new(revision);
            let map = ram::g_player1_level_map(revision);
            for column in [1, 3, 4, 5] {
                let (row, column) = if vertical { (column, 8) } else { (8, column) };
                map.at(row).at(column).write(&mut c, 0x1100);
            }
            if vertical {
                map_widen_vertical_region_edges(&mut c, &[]).unwrap();
                assert_eq!(map.at(3).at(7).read(&c), 0x1100);
                assert_eq!(map.at(4).at(7).read(&c), 0x1100);
            } else {
                map_widen_horizontal_region_edges(&mut c, &[]).unwrap();
                assert_eq!(map.at(7).at(3).read(&c), 0x1100);
                assert_eq!(map.at(7).at(4).read(&c), 0x1100);
            }
        }
    }
}

#[test]
fn filled_shape_passes_mutate_in_order_without_clipping() {
    let revision = Revision::Rev02;
    let mut c = Machine::new(revision);
    let map = ram::g_player1_level_map(revision);
    map.at(0).at(-1).write(&mut c, 19);
    map.at(0).at(0).write(&mut c, 0x0803);
    map.at(5).at(4).write(&mut c, 4);
    map.at(5).at(5).write(&mut c, 0x080c);
    map.at(9).at(9).write(&mut c, 0x880f);
    map.at(8).at(9).write(&mut c, 14);
    map.at(10).at(9).write(&mut c, 7);
    map_refine_filled_shape_masks(&mut c, &[]).unwrap();
    assert_eq!(map.at(0).at(0).read(&c), 0x0813);
    assert_eq!(map.at(5).at(5).read(&c), 0x081c);
    assert_eq!(map.at(9).at(9).read(&c), 0x8817);
}

#[test]
fn interior_growth_preserves_outputs_on_failure_and_merges_global_labels() {
    let revision = Revision::Rev02;
    let mut c = Machine::new(revision);
    let map = ram::g_player1_level_map(revision);
    for row in 0..26 {
        for column in 0..19 {
            map.at(row).at(column).write(&mut c, 0x1100);
        }
    }
    let row = ram::g_saved_level_map(revision).cast::<i16>();
    let column = row.offset(1);
    row.write(&mut c, 5);
    column.write(&mut c, 5);
    let arguments = [row.address(), column.address(), 1, 0x3100];
    assert_eq!(
        map_find_interior_growth_origin(&mut c, &arguments).unwrap(),
        Some(1)
    );
    assert_eq!((row.read(&c), column.read(&c)), (4, 5));
    ram::g_level_anchor_positions(revision)
        .at(2)
        .x()
        .write(&mut c, 5);
    ram::g_level_anchor_positions(revision)
        .at(2)
        .y()
        .write(&mut c, 3);
    assert_eq!(
        map_find_interior_growth_origin(&mut c, &arguments).unwrap(),
        Some(0)
    );
    assert_eq!((row.read(&c), column.read(&c)), (4, 5));
    map_fill_rectangle_and_merge_regions(&mut c, &[4, 5, 1, 1, 0x1200]).unwrap();
    assert_eq!(map.at(25).at(18).read(&c), 0x1200);
}

#[test]
fn cartridge_views_are_revision_selected_and_narrow_rom_bytes_zero_extend() {
    for revision in Revision::ALL {
        let mut bytes = vec![0; 0x100000];
        let data = rom::level1_map_data(revision).address() as usize;
        bytes[data] = 0xf1;
        bytes[data + 493] = 0xa6;
        let data = rom::funkatron_map_data(revision).address() as usize;
        bytes[data..data + 2].copy_from_slice(&0x91ab_u16.to_be_bytes());
        let mut c = Machine::with_rom(revision, bytes);
        load_level1_tiles(&mut c, &[]).unwrap();
        assert_eq!(
            ram::g_player1_level_map(revision).at(0).at(0).read(&c),
            0xf1
        );
        assert_eq!(
            ram::g_player2_level_map(revision).at(25).at(18).read(&c),
            0xa6
        );
        load_final_level(&mut c, &[0x1234]).unwrap();
        assert_eq!(
            ram::g_player2_level_map(revision).at(0).at(0).read(&c),
            0x91ab
        );
        assert_eq!(ram::g_player1_map_level(revision).read(&c), 0x34);
    }
}

#[test]
fn path_alternatives_use_counted_rom_lists_and_return_not_found_sixteen() {
    for revision in Revision::ALL {
        let mut bytes = vec![0; 0x100000];
        for shape in 0..16 {
            let table = rom::map_generation_lookup_tables(revision)
                .tiles()
                .at(shape)
                .address() as usize;
            bytes[table] = (shape * 2) as u8;
            let slot = rom::tile_alternative_pointers(revision)
                .at(shape * 2)
                .address() as usize;
            let list = 0x4000 + shape * 4;
            bytes[slot..slot + 4].copy_from_slice(&(list as u32).to_be_bytes());
            bytes[list as usize..list as usize + 3].copy_from_slice(&[
                2,
                (0x80 + shape) as u8,
                shape as u8,
            ]);
        }
        let mut c = Machine::with_rom(revision, bytes);
        for shape in 0..16 {
            assert_eq!(
                map_find_path_shape_for_metatile(&mut c, &[0x180 + shape]).unwrap(),
                Some(shape)
            );
        }
        assert_eq!(
            map_find_path_shape_for_metatile(&mut c, &[0x7f]).unwrap(),
            Some(16)
        );
        for (input, output) in [
            (-1_i32, -1_i32),
            (1, 4),
            (2, 8),
            (4, 1),
            (8, 2),
            (15, 1),
            (i32::MIN, 1),
        ] {
            assert_eq!(
                map_reverse_path_direction(&mut c, &[input as u32]).unwrap(),
                Some(output as u32)
            );
        }
    }
}

#[test]
fn recursive_path_walk_keeps_outside_map_accesses_and_stores_before_anchor() {
    let revision = Revision::Rev02;
    let mut c = Machine::new(revision);
    let map = ram::g_player1_level_map(revision);
    map.at(0).at(0).write(&mut c, 0x2001);
    map.at(-1).at(0).write(&mut c, 0x1000);
    ram::g_level_anchor_positions(revision)
        .at(1)
        .x()
        .write(&mut c, 0);
    ram::g_level_anchor_positions(revision)
        .at(1)
        .y()
        .write(&mut c, -1);
    assert_eq!(
        map_find_route_to_second_anchor(&mut c, &[0, 0]).unwrap(),
        Some(1)
    );
    assert_eq!(map.at(-1).at(0).read(&c), 0x1800);
    assert_eq!(
        map_find_route_to_second_anchor(&mut c, &[0, 0]).unwrap(),
        Some(0)
    );
}

#[test]
fn cover_sprite_limit_and_index_alias_keep_original_store_order() {
    let revision = Revision::Rev02;
    let mut c = Machine::new(revision);
    let sprites = tje_mem::Address::<HardwareSprite>::new(0xffa000);
    let index = ram::g_map_screen_next_sprite_index(revision);
    index.write(&mut c, 78);
    assert_eq!(
        map_append_cover_block_sprites(&mut c, &[6, 7, sprites.address(), 0]).unwrap(),
        Some(0)
    );
    assert_eq!(index.read(&c), 80);
    assert_eq!(sprites.offset(78).x().read(&c), 408);
    assert_eq!(sprites.offset(79).tile().read(&c), 0xc146);
    assert_eq!(
        map_append_cover_block_sprites(&mut c, &[0, 0, sprites.address(), 0]).unwrap(),
        Some(1)
    );
    index.write(&mut c, 0);
    let alias = index.address() - 6;
    map_append_cover_block_sprites(&mut c, &[0, (-3_i32) as u32, alias, 0]).unwrap();
    assert_eq!(index.read(&c), 10);
    assert_eq!(
        tje_mem::Address::<HardwareSprite>::new(alias)
            .offset(8)
            .y()
            .read(&c),
        128
    );
}

#[test]
fn edge_thresholds_and_signed_elevator_parity_are_explicit() {
    let revision = Revision::Rev02;
    let mut c = Machine::new(revision);
    let terrain = ram::g_player2_nearby_terrain(revision).cast::<u8>();
    terrain.offset(17).write(&mut c, 0x4f);
    assert_eq!(player_is_near_terrain_edge(&mut c, &[1]).unwrap(), Some(0));
    terrain.offset(17).write(&mut c, 0xff);
    assert_eq!(player_is_near_terrain_edge(&mut c, &[1]).unwrap(), Some(1));
    for (state, expected) in [(0_i8, 0), (-1, 1), (-2, 0), (3, 1)] {
        ram::g_player_elevator_flags(revision)
            .at(0)
            .write(&mut c, state);
        assert_eq!(
            player_has_odd_elevator_state(&mut c, &[0]).unwrap(),
            Some(expected)
        );
    }
}

#[test]
fn directional_shape_priority_pending_flags_and_mode_use_the_rom_views() {
    for revision in Revision::ALL {
        for (mode, pending) in [(0, false), (0, true), (1, true)] {
            let mut bytes = vec![0; 0x100000];
            for shape in 0..16 {
                let address = rom::map_generation_lookup_tables(revision)
                    .tiles()
                    .at(shape)
                    .address();
                bytes[address as usize] = 0x20 + shape as u8;
            }
            let alternate = rom::map_boundary_shapes_alternate(revision)
                .at(16)
                .address();
            bytes[alternate as usize] = 0x59;
            let mut c = Machine::with_rom(revision, bytes);
            let map = ram::g_player1_level_map(revision);
            map.at(4).at(4).write(&mut c, 0x9000);
            map.at(3).at(4).write(&mut c, 0x2004);
            map.at(4).at(5).write(&mut c, 0x2008);
            let directions = ram::g_map_path_direction_banks(revision);
            directions
                .at(3)
                .at(4)
                .write(&mut c, if pending { 4 } else { 0 });
            directions.at(4).at(5).write(&mut c, 8);
            ram::g_map_directional_path_mode(revision).write(&mut c, mode);
            map_build_directional_path_shapes(&mut c, &[]).unwrap();
            assert_eq!(
                map.at(4).at(4).read(&c),
                if pending { 0xd000 } else { 0x9059 }
            );
            assert_eq!(
                map.at(3).at(4).read(&c),
                if pending && mode == 0 { 0x6020 } else { 0x2024 }
            );
        }
    }
}

#[test]
fn cover_reveal_gate_and_base_mask_and_word_row_wrap_are_preserved() {
    let revision = Revision::Rev02;
    let mut c = Machine::new(revision);
    let map = ram::g_player1_level_map(revision);
    for row in 0..26 {
        for column in 0..19 {
            map.at(row).at(column).write(&mut c, 34);
        }
    }
    assert_eq!(
        map_cover_block_contains_terrain(&mut c, &[map.address(), 0, 0]).unwrap(),
        Some(0)
    );
    ram::g_map_reveal_mode(revision).write(&mut c, 1);
    map.at(0).at(0).write(&mut c, 0xa2);
    assert_eq!(
        map_cover_block_contains_terrain(&mut c, &[map.address(), 0, 0]).unwrap(),
        Some(0)
    );
    map.at(0).at(0).write(&mut c, 0x7f);
    assert_eq!(
        map_cover_block_contains_terrain(&mut c, &[map.address(), 0, 0]).unwrap(),
        Some(1)
    );
    let wrapped_row = ram::g_map_path_direction_banks(revision)
        .cast::<u8>()
        .byte_offset(-27536);
    wrapped_row.write(&mut c, 0xa7);
    assert_eq!(
        map_get_bank_path_directions(&mut c, &[0, 2000, 0]).unwrap(),
        Some(7)
    );
    assert_eq!(
        map_get_bank_path_directions(&mut c, &[1, 2000, 0]).unwrap(),
        Some(10)
    );
}

#[test]
fn transition_membership_narrows_slots_and_observes_the_count_prefix() {
    for revision in Revision::ALL {
        let mut bytes = vec![0; 0x100000];
        let shapes = rom::map_transition_descriptors(revision)
            .at(0)
            .shapes()
            .address() as usize;
        bytes[shapes..shapes + 4].copy_from_slice(&0x5000_u32.to_be_bytes());
        bytes[0x5000..0x5004].copy_from_slice(&[2, 0xaa, 0xbb, 0xcc]);
        let mut c = Machine::with_rom(revision, bytes);
        assert_eq!(
            map_transition_contains_metatile(&mut c, &[0x101, 0x1aa]).unwrap(),
            Some(1)
        );
        assert_eq!(
            map_transition_contains_metatile(&mut c, &[1, 0xbb]).unwrap(),
            Some(1)
        );
        assert_eq!(
            map_transition_contains_metatile(&mut c, &[1, 0xcc]).unwrap(),
            Some(0)
        );
    }
}

#[path = "tests_review.rs"]
mod review_indexes;
#[test]
fn original_classes_are_complete_and_pending_entries_never_register() {
    let count = crate::functions()
        .filter(|entry| entry.subsystem == "map")
        .count();
    assert_eq!(ATOMIC_CAPABLE.len() + EXECUTION_CONTRACTS.len(), count);
    for entry in crate::functions().filter(|entry| entry.subsystem == "map") {
        let atomic = ATOMIC_CAPABLE.iter().filter(|&&id| id == entry.id).count();
        let non_atomic = EXECUTION_CONTRACTS
            .iter()
            .filter(|contract| contract.function == entry.id)
            .count();
        assert_eq!(atomic + non_atomic, 1, "{}", entry.name);
    }
    assert_eq!(
        PENDING.len(),
        PENDING_TRANSLATIONS.len()
            + FRAME_PENDING_TRANSLATIONS.len()
            + PENDING_FRAMED_TRANSLATIONS.len()
    );
    for id in PENDING {
        assert_eq!(
            PENDING_TRANSLATIONS
                .iter()
                .chain(FRAME_PENDING_TRANSLATIONS)
                .map(|entry| entry.id)
                .chain(PENDING_FRAMED_TRANSLATIONS.iter().map(|entry| entry.id))
                .filter(|entry| entry == id)
                .count(),
            1
        );
    }
    for id in PENDING_TRANSLATIONS
        .iter()
        .chain(FRAME_PENDING_TRANSLATIONS)
        .map(|entry| entry.id)
        .chain(PENDING_FRAMED_TRANSLATIONS.iter().map(|entry| entry.id))
    {
        assert!(!TRANSLATIONS.iter().any(|registered| registered.id == id));
        assert_eq!(id.metadata().subsystem, "map");
    }
}

#[path = "tests_indexing.rs"]
mod index_and_register_lowering;
