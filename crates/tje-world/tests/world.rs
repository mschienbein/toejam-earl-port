use tje_world::generation::{Generation, GenerationSpec, Phase};
use tje_world::population::{PopulationBaseline, PopulationBudget, StableBits};
use tje_world::random::StageRng;
use tje_world::world::{WorldState, codec, decode};
use tje_world::{
    Cell, Density, Dimensions, Error, LevelId, LevelSize, Resources, Result, Shape, WorldConfig,
};

// Entirely synthetic definitions: one solid terrain tile and one void tile.
// They exercise algorithms, not a copy or approximation of a cartridge table.
struct Solid;
impl Resources for Solid {
    fn parameter(&self, _: LevelId, parameter: u8) -> Result<(u8, u8)> {
        Ok(if parameter == 6 { (3, 6) } else { (55, 70) })
    }
    fn shape(&self, _: Shape, _: u8, alternate: bool) -> Result<u8> {
        Ok(if alternate { 2 } else { 1 })
    }
    fn alternative(&self, shape: u8, _: u32) -> Result<Cell> {
        Cell::new(shape, 0)
    }
    fn transition(&self, _: u8, _: u8, _: u32) -> Result<Option<Cell>> {
        Ok(None)
    }
    fn base_tile(&self, base: u8, _: u8, _: u8) -> Result<u16> {
        Ok(u16::from(base & 127 != 0))
    }
    fn overlay_tile(&self, _: u8, _: u8, _: u8) -> Result<u16> {
        Ok(0)
    }
    fn property(&self, tile: u16) -> Result<u8> {
        Ok(if tile == 1 { 0x10 } else { 0x90 })
    }
}
struct Disconnected;
impl Resources for Disconnected {
    fn parameter(&self, level: LevelId, parameter: u8) -> Result<(u8, u8)> {
        Solid.parameter(level, parameter)
    }
    fn shape(&self, kind: Shape, corners: u8, alternate: bool) -> Result<u8> {
        Solid.shape(kind, corners, alternate)
    }
    fn alternative(&self, shape: u8, draw: u32) -> Result<Cell> {
        Solid.alternative(shape, draw)
    }
    fn transition(&self, _: u8, _: u8, _: u32) -> Result<Option<Cell>> {
        Ok(None)
    }
    fn base_tile(&self, _: u8, row: u8, column: u8) -> Result<u16> {
        Ok(u16::from(row == 4 && column == 8))
    }
    fn overlay_tile(&self, _: u8, _: u8, _: u8) -> Result<u16> {
        Ok(0)
    }
    fn property(&self, tile: u16) -> Result<u8> {
        Solid.property(tile)
    }
}
fn id(value: u16) -> LevelId {
    LevelId::new(value).unwrap()
}
fn config(size: LevelSize, density: Density, seed: u64) -> WorldConfig {
    WorldConfig {
        root_seed: seed,
        rom_sha1: [0; 20],
        size,
        density,
    }
}
fn baseline() -> PopulationBaseline {
    PopulationBaseline {
        walkable_tiles: 19 * 26 * 64,
        ordinary_items: 12,
        earthlings: 10,
        reserved_items: 2,
        ship_piece: true,
        mailbox: true,
    }
}
fn spec(dimensions: Dimensions, seed: u64) -> GenerationSpec {
    GenerationSpec {
        config: config(LevelSize::Custom(dimensions), Density::Normal, seed),
        level: id(2),
        baseline: baseline(),
    }
}
fn complete(mut generation: Generation, quantum: u32, resources: &impl Resources) -> Generation {
    for _ in 0..1_000_000 {
        if matches!(
            generation.advance(resources, quantum).unwrap(),
            Phase::Complete | Phase::Rejected
        ) {
            return generation;
        }
    }
    panic!("bounded generation did not terminate");
}
fn load(world: &mut WorldState, level: LevelId, resources: &impl Resources) -> Phase {
    world.request_load(level, baseline()).unwrap();
    for _ in 0..100_000 {
        let phase = world.advance_loading(resources, 1024).unwrap();
        if matches!(phase, Phase::Complete | Phase::Rejected) {
            return phase;
        }
    }
    panic!("world load did not terminate");
}

#[test]
fn park_miller_and_sha_stage_vectors_are_fixed() {
    let mut rng = StageRng::new(0);
    for (state, draw) in [
        (16807, 16807),
        (282475249, 15089),
        (1622650073, 44249),
        (984943658, 3114),
        (1144108930, 46978),
    ] {
        assert_eq!(rng.next_u16(), draw);
        assert_eq!(rng.state(), state);
    }
    assert_eq!(
        StageRng::derive(&[0; 32], id(2), 0, b"anchors").state(),
        1863217297
    );
    assert_ne!(
        StageRng::derive(&[0; 32], id(2), 0, b"population"),
        StageRng::derive(&[0; 32], id(2), 0, b"loot")
    );
    assert_ne!(
        StageRng::derive(&[0; 32], id(2), 0, b"anchors"),
        StageRng::derive(&[0; 32], id(2), 1, b"anchors")
    );
    for seed in [0, 1, u64::MAX] {
        assert!(StageRng::new(seed).valid());
    }
}

#[test]
fn independent_defaults_are_faithful_and_every_extra_is_fail_closed() {
    for size in [
        LevelSize::Original,
        LevelSize::Expanded,
        LevelSize::Large,
        LevelSize::Custom(Dimensions::new(8, 64).unwrap()),
    ] {
        for density in [
            Density::Original,
            Density::Low,
            Density::Normal,
            Density::Busy,
        ] {
            let value = config(size, density, 5);
            assert_eq!(
                tje_world::admit_native(&value).is_ok(),
                size == LevelSize::Original && density == Density::Original
            );
            assert_eq!(value.dimensions_for(id(1)), Dimensions::ORIGINAL);
            assert_eq!(value.dimensions_for(id(26)), Dimensions::ORIGINAL);
        }
    }
    assert_ne!(
        config(LevelSize::Expanded, Density::Original, 1).identity(),
        config(LevelSize::Expanded, Density::Busy, 1).identity()
    );
}

#[test]
fn geometry_bounds_and_camera_stock_clamps() {
    assert_eq!(Dimensions::ORIGINAL.world_envelope(), (2688, 1792));
    assert_eq!(
        Dimensions::ORIGINAL.camera_bounds(320, 224).unwrap(),
        ((8, 2360), (8, 1560))
    );
    assert_eq!(
        Dimensions::ORIGINAL.camera_bounds(320, 112).unwrap().1,
        (8, 1672)
    );
    let maximum = Dimensions::new(64, 64).unwrap();
    assert_eq!(maximum.world_envelope(), (8448, 4224));
    for (x, y) in [
        (-1, 0),
        (0, -1),
        (64, 0),
        (0, 64),
        (i32::MAX, i32::MAX),
        (i32::MIN, i32::MIN),
    ] {
        assert_eq!(maximum.index(x, y), None);
    }
    assert_eq!(maximum.index(63, 63), Some(4095));
    assert!(maximum.world_cell(-1, 128).is_none());
    assert!(maximum.world_cell(i32::MIN, i32::MAX).is_none());
    assert!(maximum.world_cell(127, 64).is_none());
    assert!(maximum.world_cell(128, 63).is_none());
    assert_eq!(
        maximum.world_cell(128, 64).unwrap().index(maximum).unwrap(),
        0
    );
    for width in [0, 7, 65, u16::MAX] {
        assert!(Dimensions::new(width, 26).is_err());
    }
    for height in [0, 7, 65, u16::MAX] {
        assert!(Dimensions::new(19, height).is_err());
    }
    assert!(maximum.camera_bounds(u16::MAX, 224).is_err());
    assert!(LevelId::new(32).is_none());
    assert!(LevelId::new(u16::MAX).is_none());
}

#[test]
fn final_cells_cannot_contain_generation_labels_or_illegal_overlay_mirrors() {
    for base in 0..122 {
        for mirrored in [0, 128] {
            for overlay in [0, 1, 15, 17, 31] {
                let cell = Cell::new(base | mirrored, overlay).unwrap();
                assert_eq!(cell.base(), base | mirrored);
                assert_eq!(cell.overlay(), overlay);
            }
        }
    }
    for word in [0x2000, 0x4000, 0x8000, 0x1000, 122, 255, 0xffff] {
        assert!(Cell::from_word(word).is_err());
    }
}

#[test]
fn generation_is_deterministic_across_dimensions_seeds_and_work_quanta() {
    for (width, height) in [
        (8, 8),
        (19, 26),
        (28, 39),
        (38, 52),
        (64, 64),
        (8, 64),
        (64, 8),
        (9, 17),
    ] {
        let dimensions = Dimensions::new(width, height).unwrap();
        for seed in [0, 1, u64::MAX] {
            let a = complete(
                Generation::new(spec(dimensions, seed)).unwrap(),
                1024,
                &Solid,
            );
            let b = complete(
                Generation::new(spec(dimensions, seed)).unwrap(),
                173,
                &Solid,
            );
            assert_eq!(a, b, "{width}x{height} seed {seed}");
            let a = a.finish().unwrap();
            let b = b.finish().unwrap();
            assert_eq!(
                bincode::encode_to_vec(&a, codec()).unwrap(),
                bincode::encode_to_vec(&b, codec()).unwrap()
            );
            assert_eq!(a.geometry.cells().len(), dimensions.area());
            assert!(a.population.items.len() <= 128 && a.population.earthlings.len() <= 64);
            assert!(a.accepted_attempt < 64);
            a.validate().unwrap();
            for placement in a.population.items.iter().chain(&a.population.earthlings) {
                assert_eq!(
                    a.geometry
                        .terrain_at(id(2), placement.x, placement.y, &Solid)
                        .unwrap()
                        & 0xf0,
                    0x10
                );
            }
        }
    }
}

#[test]
fn generator_snapshot_round_trip_at_every_loading_phase() {
    let request = spec(Dimensions::new(8, 8).unwrap(), 71);
    let expected = complete(Generation::new(request.clone()).unwrap(), 1024, &Solid)
        .finish()
        .unwrap();
    let mut generation = Generation::new(request).unwrap();
    let mut phases = Vec::new();
    loop {
        let phase = generation.phase();
        if !phases.contains(&phase) {
            phases.push(phase);
            let bytes = bincode::encode_to_vec(&generation, codec()).unwrap();
            let (restored, used): (Generation, usize) =
                bincode::decode_from_slice(&bytes, codec()).unwrap();
            assert_eq!(used, bytes.len());
            restored.validate().unwrap();
            assert_eq!(complete(restored, 1024, &Solid).finish().unwrap(), expected);
        }
        if generation.advance(&Solid, 1).unwrap() == Phase::Complete {
            break;
        }
    }
    assert_eq!(
        phases,
        vec![
            Phase::Anchors,
            Phase::Paths,
            Phase::Regions,
            Phase::Features,
            Phase::Shapes,
            Phase::Alternatives,
            Phase::Terrain,
            Phase::Flood,
            Phase::Validate,
            Phase::Population
        ]
    );
    assert_eq!(generation.finish().unwrap(), expected);
}

#[test]
fn coarse_connected_but_tile_disconnected_maps_exhaust_exactly_64_attempts() {
    let request = spec(Dimensions::new(8, 8).unwrap(), 12);
    let generation = complete(Generation::new(request).unwrap(), 1024, &Disconnected);
    assert_eq!(generation.phase(), Phase::Rejected);
    assert_eq!(generation.attempt(), 63);
    assert!(matches!(
        generation.finish(),
        Err(Error::Generation { attempts: 64, .. })
    ));
}

#[test]
fn invalid_work_budgets_and_special_generation_fail_before_mutation() {
    let mut generation = Generation::new(spec(Dimensions::EXPANDED, 3)).unwrap();
    let before = generation.clone();
    assert!(generation.advance(&Solid, 0).is_err());
    assert_eq!(generation, before);
    assert!(generation.advance(&Solid, 1025).is_err());
    assert_eq!(generation, before);
    for special in [0, 1, 26, 31] {
        let mut request = generation.spec().clone();
        request.level = id(special);
        assert!(Generation::new(request).is_err());
    }
}

#[test]
fn density_caps_preserve_reservations_and_do_not_depend_on_viewports() {
    let stock = PopulationBudget::calculate(baseline(), 400000, Density::Original).unwrap();
    assert_eq!(
        (stock.items, stock.earthlings, stock.reserved_items),
        (14, 10, 2)
    );
    let busy = PopulationBudget::calculate(baseline(), 524288, Density::Busy).unwrap();
    assert_eq!((busy.items, busy.earthlings), (128, 64));
    let low =
        PopulationBudget::calculate(baseline(), baseline().walkable_tiles, Density::Low).unwrap();
    let normal =
        PopulationBudget::calculate(baseline(), baseline().walkable_tiles, Density::Normal)
            .unwrap();
    assert!(low.items < normal.items && low.earthlings < normal.earthlings);
    assert!(PopulationBudget::calculate(baseline(), 0, Density::Normal).is_err());
    let mut bad = baseline();
    bad.walkable_tiles = 0;
    assert!(bad.validate().is_err());
}

#[test]
fn stable_ids_beyond_bit_31_and_all_cell_bits_round_trip() {
    let mut bits = StableBits::new(4096).unwrap();
    for bit in [0, 31, 32, 63, 64, 127, 128, 4095] {
        assert!(bits.insert(bit).unwrap());
        assert!(!bits.insert(bit).unwrap());
        assert!(bits.contains(bit));
    }
    assert!(bits.insert(4096).is_err());
    bits.validate(4096).unwrap();
    let bytes = bincode::encode_to_vec(&bits, codec()).unwrap();
    let (restored, _): (StableBits, _) = bincode::decode_from_slice(&bytes, codec()).unwrap();
    assert_eq!(restored, bits);
    assert!(StableBits::new(4097).is_err());
}

#[test]
fn leave_elevator_fall_reenter_and_snapshot_preserve_all_floor_deltas() {
    let configuration = config(LevelSize::Expanded, Density::Busy, 42);
    let mut world = WorldState::new(configuration.clone()).unwrap();
    world
        .request_load(
            id(2),
            PopulationBaseline {
                walkable_tiles: 1000,
                ..baseline()
            },
        )
        .unwrap();
    while world.pending().is_some() {
        world.advance_loading(&Solid, 1024).unwrap();
    }
    world.set_active(0, id(2)).unwrap();
    let position = world
        .floor(id(2))
        .unwrap()
        .generated
        .geometry
        .anchors()
        .arrival;
    let (x, y) = position.center();
    let floor = world.floor_mut(id(2)).unwrap();
    floor
        .replace_cell(position, Cell::new(2, 0).unwrap(), 3)
        .unwrap();
    floor.generated.population.collected.insert(103).unwrap();
    floor.generated.population.removed.insert(40).unwrap();
    assert!(floor.discovery.visit(x, y, Dimensions::EXPANDED).unwrap());
    assert!(!floor.discovery.visit(x, y, Dimensions::EXPANDED).unwrap());
    let drop_id = world.drop_item(id(2), x, y, 5).unwrap();
    world.request_load(id(3), baseline()).unwrap();
    assert_eq!(world.active()[0], Some(id(2)));
    assert!(
        world
            .commit_cohort(1, [Some(id(3)), None, None, None])
            .is_err()
    );
    while world.pending().is_some() {
        world.advance_loading(&Solid, 1024).unwrap();
    }
    world
        .commit_cohort(1, [Some(id(3)), None, None, None])
        .unwrap();
    assert_eq!(world.active()[0], Some(id(3)));
    world
        .commit_cohort(1, [Some(id(2)), None, None, None])
        .unwrap();
    for level in 4..=7 {
        assert_eq!(load(&mut world, id(level), &Solid), Phase::Complete);
    }
    let bytes = bincode::encode_to_vec(&world, codec()).unwrap();
    let mut restored = decode(&bytes, &configuration).unwrap();
    assert_eq!(restored, world);
    restored
        .commit_cohort(1, [Some(id(2)), None, None, None])
        .unwrap();
    let floor = restored.floor(id(2)).unwrap();
    assert_eq!(
        floor
            .generated
            .geometry
            .cell(i32::from(position.x), i32::from(position.y))
            .unwrap()
            .base(),
        2
    );
    assert_eq!(
        floor.generated.geometry.path_state()[position.index(Dimensions::EXPANDED).unwrap()],
        3
    );
    assert!(floor.generated.population.collected.contains(103));
    assert!(floor.generated.population.removed.contains(40));
    assert_eq!(floor.discovery.awards(), 1);
    assert_eq!(restored.dropped()[0].id, drop_id);
    restored.validate().unwrap();
}

#[test]
fn failed_load_and_special_transition_leave_playable_floors_and_seats_intact() {
    let mut world = WorldState::new(config(
        LevelSize::Custom(Dimensions::new(8, 8).unwrap()),
        Density::Normal,
        7,
    ))
    .unwrap();
    assert_eq!(load(&mut world, id(2), &Solid), Phase::Complete);
    world.set_active(0, id(2)).unwrap();
    let before = world.floors().to_vec();
    let active = *world.active();
    assert_eq!(load(&mut world, id(3), &Disconnected), Phase::Rejected);
    assert_eq!(world.floors(), before);
    assert_eq!(*world.active(), active);
    assert_eq!(world.last_failure().unwrap().attempts, 64);
    let before = bincode::encode_to_vec(&world, codec()).unwrap();
    assert!(
        world
            .commit_cohort(1, [Some(id(1)), None, None, None])
            .is_err()
    );
    assert_eq!(bincode::encode_to_vec(&world, codec()).unwrap(), before);
    let restored = decode(&before, world.config()).unwrap();
    assert_eq!(restored, world);
}

#[test]
fn world_pending_snapshot_preserves_generator_and_atomic_publication() {
    let configuration = config(LevelSize::Expanded, Density::Normal, 19);
    let mut world = WorldState::new(configuration.clone()).unwrap();
    world.request_load(id(2), baseline()).unwrap();
    world.advance_loading(&Solid, 377).unwrap();
    assert!(world.floors().is_empty());
    let bytes = bincode::encode_to_vec(&world, codec()).unwrap();
    let mut restored = decode(&bytes, &configuration).unwrap();
    while world.pending().is_some() {
        world.advance_loading(&Solid, 1024).unwrap();
    }
    while restored.pending().is_some() {
        restored.advance_loading(&Solid, 13).unwrap();
    }
    assert_eq!(world, restored);
    let mut mismatch = configuration.clone();
    mismatch.root_seed += 1;
    assert!(decode(&bytes, &mismatch).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(decode(&trailing, &configuration).is_err());
}

#[test]
fn four_active_floors_have_fixed_interest_and_fit_the_logical_arena() {
    let mut world = WorldState::new(config(
        LevelSize::Custom(Dimensions::new(64, 64).unwrap()),
        Density::Busy,
        9,
    ))
    .unwrap();
    for seat in 0..4 {
        assert_eq!(
            load(&mut world, id(u16::from(seat) + 2), &Solid),
            Phase::Complete
        );
        world.set_active(seat, id(u16::from(seat) + 2)).unwrap();
    }
    assert!(world.logical_entities_reserved() <= 4 * (128 + 64) + 4);
    let population = &world.floor(id(2)).unwrap().generated.population;
    let center = world
        .floor(id(2))
        .unwrap()
        .generated
        .geometry
        .anchors()
        .arrival
        .center();
    let mut active = Vec::with_capacity(64);
    population.active_earthlings(&[center], &mut active);
    let expected = active.clone();
    for _presentation in [320, 400, 512] {
        population.active_earthlings(&[center], &mut active);
        assert_eq!(active, expected);
    }
    let bytes = bincode::encode_to_vec(&world, codec()).unwrap();
    assert!(bytes.len() < tje_world::MAX_STATE_BYTES);
    for _ in 0..128 {
        world.drop_item(id(2), center.0, center.1, 0).unwrap();
    }
    assert!(world.drop_item(id(2), center.0, center.1, 0).is_err());
    world.validate().unwrap();
}

#[cfg(feature = "enhanced-mode")]
#[test]
fn native_admission_does_not_register_or_mutate_sidecars() {
    use tje_engine_runtime::enhanced::EnhancedMode;
    let configuration = config(LevelSize::Expanded, Density::Normal, 9);
    let state = WorldState::new(configuration.clone()).unwrap();
    let mut mode = EnhancedMode::default();
    mode.sidecars_mut().register(state.clone()).unwrap();
    assert!(tje_world::enhanced::configure_native(&mut mode, &configuration).is_err());
    assert_eq!(mode.profile_id(), "faithful");
    assert_eq!(mode.sidecars().get::<WorldState>().unwrap(), &state);
    tje_world::enhanced::configure_native(
        &mut mode,
        &config(LevelSize::Original, Density::Original, 0),
    )
    .unwrap();
    assert_eq!(mode.sidecars().get::<WorldState>().unwrap(), &state);
}

#[test]
fn maximum_codec_envelope_reserves_room_for_pending_loading() {
    let configuration = config(
        LevelSize::Custom(Dimensions::new(64, 64).unwrap()),
        Density::Busy,
        25,
    );
    let mut world = WorldState::new(configuration.clone()).unwrap();
    for level in 2..=24 {
        assert_eq!(load(&mut world, id(level), &Solid), Phase::Complete);
    }
    let mut maximum_geometry = 0;
    let mut maximum_population = 0;
    for floor in world.floors() {
        maximum_geometry = maximum_geometry.max(
            bincode::encode_to_vec(&floor.generated.geometry, codec())
                .unwrap()
                .len(),
        );
        maximum_population = maximum_population.max(
            bincode::encode_to_vec(&floor.generated.population, codec())
                .unwrap()
                .len(),
        );
    }
    world.request_load(id(25), baseline()).unwrap();
    world.advance_loading(&Solid, 1024).unwrap();
    let bytes = bincode::encode_to_vec(&world, codec()).unwrap();
    assert!(maximum_geometry < 32 * 1024);
    assert!(maximum_population < 16 * 1024);
    // Reserve the design's entire 32-floor envelope, not merely these 23
    // ordinary floor IDs, plus a maximum pending generation, registry and seats.
    assert!(
        bytes.len() + (32 - world.floors().len()) * (32 + 16) * 1024 + 64 * 1024
            < tje_world::MAX_STATE_BYTES
    );
    eprintln!(
        "world cap: geometry={maximum_geometry} population={maximum_population} 23 floors+pending={} bytes",
        bytes.len()
    );
    assert_eq!(decode(&bytes, &configuration).unwrap(), world);
}

#[test]
fn invalid_persistent_ids_and_population_caps_reject_snapshot_restore() {
    let configuration = config(LevelSize::Expanded, Density::Normal, 8);
    let mut world = WorldState::new(configuration.clone()).unwrap();
    assert_eq!(load(&mut world, id(2), &Solid), Phase::Complete);
    world
        .floor_mut(id(2))
        .unwrap()
        .generated
        .population
        .budget
        .items = 129;
    let bytes = bincode::encode_to_vec(&world, codec()).unwrap();
    assert!(decode(&bytes, &configuration).is_err());
}

#[test]
fn density_and_population_baseline_do_not_reseed_candidate_geometry() {
    let request = spec(Dimensions::EXPANDED, 77);
    let mut other = request.clone();
    other.config.density = Density::Busy;
    other.baseline.walkable_tiles = 12000;
    other.baseline.ordinary_items = 8;
    assert_eq!(request.geometry_identity(), other.geometry_identity());
    assert_ne!(request.identity(), other.identity());
    let first = complete(Generation::new(request).unwrap(), 1024, &Solid)
        .finish()
        .unwrap();
    let second = complete(Generation::new(other).unwrap(), 1024, &Solid)
        .finish()
        .unwrap();
    assert_eq!(first.accepted_attempt, second.accepted_attempt);
    assert_eq!(first.geometry, second.geometry);
    assert_ne!(first.population.budget, second.population.budget);
}
