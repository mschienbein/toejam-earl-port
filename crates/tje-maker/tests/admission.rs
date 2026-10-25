use tje_maker::*;
use tje_mem::{AddressSpace, Bus, Revision, rom};

fn memory(revision: Revision) -> AddressSpace {
    // Newly authored semantic fixture, not extracted tables or original code.
    let mut bytes = vec![0; 0x10_0000];
    let tiles = rom::level_metatile_maps(revision).address() as usize;
    for base in [30, 33] {
        for tile in 0..128 {
            bytes[tiles + base * 256 + tile * 2..tiles + base * 256 + tile * 2 + 2]
                .copy_from_slice(&1_u16.to_be_bytes());
        }
    }
    let properties = rom::player_tile_property_lookup(revision).address() as usize;
    bytes[properties] = 0x90;
    bytes[properties + 1] = 0x10;
    AddressSpace::new(bytes, revision)
}
fn sample() -> Course {
    ValidatedCourse::from_json(include_bytes!("../../../courses/meadow.tjemaker.json"))
        .unwrap()
        .course()
        .clone()
}
#[test]
fn native_admission_checks_both_explicit_catalogs_without_writing_memory() {
    for (revision, catalog) in [
        (Revision::Rev02, Catalog::Rev02),
        (Revision::Rev00, Catalog::Rev00),
    ] {
        let memory = memory(revision);
        let before = memory.ram().to_vec();
        let mut course = sample();
        course.catalog = catalog;
        admit_native(&ValidatedCourse::new(course).unwrap(), &memory).unwrap();
        assert_eq!(memory.ram().as_slice(), before);
    }
}
#[test]
fn unsupported_capabilities_fail_before_any_mutation_or_stock_fallback() {
    let memory = memory(Revision::Rev02);
    for change in [
        |c: &mut Course| c.catalog = Catalog::Rev00,
        |c: &mut Course| c.rules.ruleset = Ruleset::OverlordV1,
        |c: &mut Course| c.levels[0].music = Music::Quiet,
        |c: &mut Course| {
            c.rules.heroes = 3;
            c.start.spawns.push(3);
            c.levels[0].spawn_points.push(Spawn {
                id: 3,
                position: Point { x: 640, y: 800 },
            });
        },
        |c: &mut Course| {
            c.levels[0].width = 20;
            c.levels[0].cells[0].0 = 520;
        },
        |c: &mut Course| c.levels[0].spawn_points[0].position.x = 128,
    ] {
        let mut course = sample();
        change(&mut course);
        let before = memory.ram().to_vec();
        assert!(admit_native(&ValidatedCourse::new(course).unwrap(), &memory).is_err());
        assert_eq!(memory.ram().as_slice(), before);
    }
}
#[test]
fn unsafe_actual_terrain_and_actor_capacity_are_clear_admission_errors() {
    let mut c = sample();
    c.levels[0].cells = vec![(494, 34)];
    let memory = memory(Revision::Rev02);
    assert!(
        admit_native(&ValidatedCourse::new(c).unwrap(), &memory)
            .unwrap_err()
            .to_string()
            .contains("unsafe mandatory arrival")
    );
    let mut c = sample();
    for id in 1000..1008 {
        c.levels[0].objects.push(Object::Item {
            id,
            definition: Item::Buck,
            position: Point { x: 1504, y: 1000 },
        });
    }
    assert!(
        admit_native(&ValidatedCourse::new(c).unwrap(), &memory)
            .unwrap_err()
            .to_string()
            .contains("dynamic bodies")
    );
    assert_eq!(memory.read_u8(0xff0000), 0);
}
