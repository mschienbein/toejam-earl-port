//! Semantic observations are captured from the readable Rust bodies over an
//! authored synthetic ROM in each revision. No retail tables or map are fixtures.
#![cfg(feature = "enhanced-mode")]

use tje_game::{Context, Dispatcher, FunctionId, ids};
use tje_mem::{AddressSpace, Bus, Revision, ram, rom};
use tje_world::{
    Anchors, Cell, CellPosition, Dimensions, LevelGeometry, LevelId, Resources,
    random::StageRng,
    rom::RomResources,
    terrain::{DoorwaySupport, Footprint, TerrainCache, TerrainMask, position_in_cell},
};

struct Observation {
    memory: AddressSpace,
    data: [u32; 8],
    address: [u32; 8],
    status: u16,
    rng: StageRng,
}
impl Observation {
    fn new(revision: Revision) -> Self {
        let mut bytes = vec![0; revision.rom_size()];
        // Authored checker support, not a cartridge property bank or graphic.
        for base in [1_u8, 30] {
            for row in 0..8 {
                for column in 0..16 {
                    let tile = if base == 30 || (row + column) % 3 != 0 {
                        1_u16
                    } else {
                        2
                    };
                    let offset = rom::level_metatile_maps(revision).address() as usize
                        + usize::from(base) * 256
                        + row * 32
                        + column * 2;
                    bytes[offset..offset + 2].copy_from_slice(&tile.to_be_bytes());
                }
            }
        }
        for row in 0..8 {
            for column in 0..16 {
                let tile = if column < 4 { 0_u16 } else { 2 };
                let offset =
                    rom::overlay_metatile_maps(revision).address() as usize + row * 32 + column * 2;
                bytes[offset..offset + 2].copy_from_slice(&tile.to_be_bytes());
            }
        }
        let properties = rom::player_tile_property_lookup(revision).address() as usize;
        bytes[properties + 1] = 0x10;
        bytes[properties + 2] = 0x90;
        bytes[rom::level0_map_data(revision).address() as usize] = 1;
        bytes[rom::level1_map_data(revision).address() as usize] = 30;
        let final_map = rom::funkatron_map_data(revision).address() as usize;
        bytes[final_map..final_map + 2].copy_from_slice(&1_u16.to_be_bytes());
        Self {
            memory: AddressSpace::new(bytes, revision),
            data: [0; 8],
            address: [0; 8],
            status: 0,
            rng: StageRng::new(1),
        }
    }
}
impl Bus for Observation {
    fn read_u8(&self, address: u32) -> u8 {
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
    }
}
impl tje_mem::Platform for Observation {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("geometry observation must not touch devices")
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("geometry observation must not touch devices")
    }
}
impl tje_game::Platform for Observation {
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("geometry observation must not touch devices")
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("geometry observation must not touch devices")
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("geometry observation must not touch devices")
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("geometry observation must not touch devices")
    }
}
impl Context for Observation {
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
        Dispatcher::hybrid_with(&[])
    }
    fn call_original(&mut self, id: FunctionId, args: &[u32]) -> tje_game::Result<u32> {
        let resources = RomResources::new(&self.memory).unwrap();
        match id {
            ids::GET_RANDOM_NUMBER => Ok(u32::from(self.rng.next_u16())),
            ids::GET_LEVEL_METATILE_TILE => Ok(u32::from(
                resources
                    .base_tile(args[0] as u8, args[1] as u8, args[2] as u8)
                    .unwrap(),
            )),
            ids::GET_OVERLAY_METATILE_TILE => Ok(u32::from(
                resources
                    .overlay_tile(args[0] as u8, args[1] as u8, args[2] as u8)
                    .unwrap(),
            )),
            ids::PRESENT_FIND_DOORWAY_OWNER_AT_TILE => {
                Ok(tje_game::presents::present_find_doorway_owner_at_tile(self, args)?.unwrap())
            }
            _ => Err(tje_game::Error::Untranslated(id)),
        }
    }
}
fn id(value: u16) -> LevelId {
    LevelId::new(value).unwrap()
}
fn geometry(word: u16) -> LevelGeometry {
    LevelGeometry::new(
        Dimensions::ORIGINAL,
        vec![Cell::from_word(word).unwrap(); 19 * 26],
        vec![0; 19 * 26],
        Anchors {
            arrival: CellPosition { x: 1, y: 1 },
            exit: CellPosition { x: 17, y: 24 },
            ship_piece: None,
            regions: vec![],
        },
    )
    .unwrap()
}

#[test]
fn terrain_validation_matches_recorded_readable_outputs_rng_and_mirroring() {
    for revision in Revision::ALL {
        for word in [1_u16, 0x81, 30, 34, 82, 84, 0x101] {
            for (width, height) in [(1, 1), (4, 2), (8, 2), (12, 2), (16, 8)] {
                for mask in [1_u8, 3, 8] {
                    let mut source = Observation::new(revision);
                    source.rng = StageRng::new(42);
                    let mut rng = source.rng;
                    let x = ram::g_saved_level_map(revision).cast::<i16>();
                    let y = x.offset(1);
                    x.write(&mut source, -123);
                    y.write(&mut source, -456);
                    let returned = tje_game::map::terrain_validation(
                        &mut source,
                        &[
                            u32::from(word),
                            width,
                            height,
                            x.address(),
                            y.address(),
                            u32::from(mask),
                        ],
                    )
                    .unwrap()
                    .unwrap();
                    let observed = if returned == 0 {
                        None
                    } else {
                        Some((i32::from(x.read(&source)), i32::from(y.read(&source))))
                    };
                    let generalized = position_in_cell(
                        Cell::from_word(word).unwrap(),
                        Footprint::new(width as u8, height as u8).unwrap(),
                        TerrainMask::new(mask).unwrap(),
                        false,
                        &mut rng,
                        &RomResources::new(&source.memory).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(
                        generalized, observed,
                        "{revision:?} {word:#x} {width}x{height} mask{mask}"
                    );
                    assert_eq!(rng, source.rng);
                }
            }
        }
    }
}

#[test]
fn four_neighborhoods_match_two_original_seats_overlay_and_doorway_observations() {
    for revision in Revision::ALL {
        for word in [1_u16, 0x81, 0x101, 0x1101] {
            let geometry = geometry(word);
            let mut source = Observation::new(revision);
            ram::g_split_screen_enabled(revision).write(&mut source, 1);
            for seat in 0..2 {
                ram::g_player_lives(revision).at(seat).write(&mut source, 3);
                for row in 0..26 {
                    for column in 0..19 {
                        let map = if seat == 0 {
                            ram::g_player1_level_map(revision)
                        } else {
                            ram::g_player2_level_map(revision)
                        };
                        map.at(row).at(column).write(&mut source, word);
                    }
                }
            }
            for (x, y) in [(128_i16, 64_i16), (256, 128), (2559, 1727), (100, 48)] {
                for seat in 0..2 {
                    let player = ram::g_player_info(revision).at(seat);
                    player.x().write(&mut source, x);
                    player.y().write(&mut source, y);
                    player.level().write(&mut source, 2);
                    tje_game::map::update_land_around_player(&mut source, &[seat as u32]).unwrap();
                    let nearby = if seat == 0 {
                        ram::g_player1_nearby_terrain(revision)
                    } else {
                        ram::g_player2_nearby_terrain(revision)
                    }
                    .cast::<u8>();
                    let mut observed = [0; 25];
                    for (index, value) in observed.iter_mut().enumerate() {
                        *value = nearby.offset(index as i32).read(&source);
                    }
                    let generalized = TerrainCache::sample(
                        &geometry,
                        id(2),
                        (i32::from(x), i32::from(y)),
                        &RomResources::new(&source.memory).unwrap(),
                        &[],
                    )
                    .unwrap();
                    assert_eq!(generalized.tiles, observed);
                    for extra_seat in [2, 3] {
                        let doorway = DoorwaySupport {
                            owner: extra_seat,
                            level: id(2),
                            x: i32::from(x),
                            y: i32::from(y),
                            open: true,
                        };
                        let cache = TerrainCache::sample(
                            &geometry,
                            id(2),
                            (i32::from(x), i32::from(y)),
                            &RomResources::new(&source.memory).unwrap(),
                            &[doorway],
                        )
                        .unwrap();
                        assert_eq!(cache.underfoot(), 0x10);
                    }
                }
            }
        }
    }
}

#[test]
fn special_rom_sources_read_authored_extents_in_both_revisions_without_ram_banks() {
    use tje_world::loading::{CellSource, SpecialFloor};
    for revision in Revision::ALL {
        let source = Observation::new(revision);
        let resources = RomResources::new(&source.memory).unwrap();
        assert_eq!(
            resources
                .special_source(SpecialFloor::Zero)
                .cell(0)
                .unwrap()
                .cell
                .word(),
            34
        );
        assert_eq!(
            resources
                .special_source(SpecialFloor::Zero)
                .cell(12 * 19 + 9)
                .unwrap()
                .cell
                .word(),
            1
        );
        assert_eq!(
            resources
                .special_source(SpecialFloor::One)
                .cell(0)
                .unwrap()
                .cell
                .word(),
            30
        );
        assert_eq!(
            resources
                .special_source(SpecialFloor::Final)
                .cell(0)
                .unwrap()
                .cell
                .word(),
            1
        );
        assert!(
            resources
                .special_source(SpecialFloor::Final)
                .cell(19 * 26)
                .is_err()
        );
    }
}

#[test]
fn original_scoped_rng_matches_readable_seed_normalization_and_every_draw() {
    use tje_world::original::OriginalRng;
    for revision in Revision::ALL {
        for seed in [0, 1, 42, 65535, u32::MAX, 0x8000_0000, 0x7fff_ffff] {
            let mut source = Observation::new(revision);
            ram::g_rng_state(revision)
                .write(&mut source, tje_game::map::normalize_seed(seed as i32));
            let mut generalized = OriginalRng::new(seed);
            for _ in 0..128 {
                let observed = tje_game::map::get_random_number(&mut source, &[])
                    .unwrap()
                    .unwrap() as u16;
                assert_eq!(generalized.next_u16(), observed);
                assert_eq!(
                    generalized.state(),
                    ram::g_rng_state(revision).read(&source)
                );
            }
        }
    }
}
