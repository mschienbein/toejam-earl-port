use tje_mem::{AddressSpace, Revision, rom};
use tje_world::rom::RomResources;
use tje_world::{Cell, Resources};

#[test]
fn imported_mirrors_overlay_transparency_and_terrain_indices_are_checked() {
    for revision in Revision::ALL {
        let mut bytes = vec![0; revision.rom_size()];
        let base = rom::level_metatile_maps(revision).address() as usize + 256;
        let overlay = rom::overlay_metatile_maps(revision).address() as usize;
        for row in 0..8 {
            for column in 0..16 {
                let value = (row * 16 + column + 1) as u16;
                let offset = base + row * 32 + column * 2;
                bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
                let offset = overlay + row * 32 + column * 2;
                bytes[offset..offset + 2]
                    .copy_from_slice(&(if column == 0 { 0 } else { value }).to_be_bytes());
            }
        }
        let memory = AddressSpace::new(bytes, revision);
        let resources = RomResources::new(&memory).unwrap();
        for row in 0..8 {
            for column in 0..16 {
                assert_eq!(
                    resources.base_tile(0x81, row, column).unwrap(),
                    resources.base_tile(1, row, 15 - column).unwrap() ^ 0x0800
                );
            }
        }
        assert_eq!(resources.overlay_tile(17, 0, 15).unwrap(), 0);
        assert_eq!(
            resources.overlay_tile(17, 0, 0).unwrap(),
            resources.overlay_tile(1, 0, 15).unwrap() ^ 0x0800
        );
        assert!(resources.base_tile(122, 0, 0).is_err());
        assert!(resources.base_tile(1, 8, 0).is_err());
        assert!(resources.overlay_tile(16, 0, 0).is_err());
        assert!(resources.overlay_tile(1, 0, 16).is_err());
        assert!(resources.property(398).is_err());
        assert!(resources.property(2047).is_err());
    }
}

#[test]
fn transition_shape_lists_are_counted_but_alternative_pairs_have_no_count_byte() {
    for revision in Revision::ALL {
        let mut bytes = vec![0; revision.rom_size()];
        let descriptor = rom::map_transition_descriptors(revision).address() as usize;
        let shapes = 0x1000_u32;
        let alternatives = 0x1100_u32;
        bytes[descriptor] = 1;
        bytes[descriptor + 2..descriptor + 6].copy_from_slice(&shapes.to_be_bytes());
        bytes[descriptor + 6..descriptor + 10].copy_from_slice(&alternatives.to_be_bytes());
        bytes[shapes as usize..shapes as usize + 3].copy_from_slice(&[2, 7, 8]);
        bytes[alternatives as usize..alternatives as usize + 4].copy_from_slice(&[10, 11, 12, 13]);
        let memory = AddressSpace::new(bytes, revision);
        let resources = RomResources::new(&memory).unwrap();
        assert_eq!(
            resources.transition(7, 0, 0).unwrap(),
            Some(Cell::new(10, 1).unwrap())
        );
        assert_eq!(
            resources.transition(7, 0, 1).unwrap(),
            Some(Cell::new(11, 1).unwrap())
        );
        assert_eq!(
            resources.transition(8, 0, 0).unwrap(),
            Some(Cell::new(12, 8).unwrap())
        );
        assert_eq!(
            resources.transition(8, 0, 1).unwrap(),
            Some(Cell::new(13, 8).unwrap())
        );
        assert_eq!(resources.transition(9, 0, 0).unwrap(), None);
        assert!(resources.transition(7, 16, 0).is_err());
    }
}

#[test]
fn malformed_import_pointers_and_counts_never_access_ram_or_unmapped_rom() {
    let revision = Revision::Rev02;
    for pointer in [0, revision.rom_size() as u32, 0xff0000, u32::MAX] {
        let mut bytes = vec![0; revision.rom_size()];
        let address = rom::tile_alternative_pointers(revision).address() as usize;
        bytes[address..address + 4].copy_from_slice(&pointer.to_be_bytes());
        let memory = AddressSpace::new(bytes, revision);
        let resources = RomResources::new(&memory).unwrap();
        assert!(resources.alternative(0, 0).is_err());
    }
    assert!(RomResources::new(&AddressSpace::new(vec![0; 100], revision)).is_err());
}
