use tje_world::{
    Anchors, Cell, CellPosition, Density, Dimensions, LevelGeometry, LevelId, LevelSize, Resources,
    Result, Shape, WorldConfig,
    land::{LandState, PathResources},
    loading::{ImportKind, ImportProgress, ImportSpec, SpecialFloor},
    population::{
        Placement, PlacementRole, Population, PopulationBaseline, PopulationBudget, StableBits,
    },
    random::StageRng,
    terrain::{
        DoorwaySupport, ElevatorBarrier, ElevatorContact, ElevatorDoor, Footprint, PlacementSearch,
        SearchProgress, TerrainMask,
    },
    transitions::{
        Destination, Doorway, Journey, Landing, PhoneEvent, PhonePhase, Position, Telephone,
        TravelFailure, TravelKind, TravelPhase, TravelTiming,
    },
    world::{Discovery, WorldState, codec, decode},
};

struct Synthetic;
impl Resources for Synthetic {
    fn parameter(&self, _: LevelId, parameter: u8) -> Result<(u8, u8)> {
        Ok(if parameter == 6 { (3, 6) } else { (55, 70) })
    }
    fn shape(&self, kind: Shape, mask: u8, _: bool) -> Result<u8> {
        Ok(if matches!(kind, Shape::Path) { mask } else { 1 })
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
        Ok(if tile == 0 { 0x90 } else { 0x10 })
    }
}
impl PathResources for Synthetic {
    fn path_shape(&self, metatile: u8) -> Result<Option<u8>> {
        Ok((metatile < 16).then_some(metatile))
    }
    fn edge_boundary(&self, _: u8, _: bool) -> Result<u8> {
        Ok(1)
    }
}
fn id(value: u16) -> LevelId {
    LevelId::new(value).unwrap()
}
fn config() -> WorldConfig {
    WorldConfig {
        root_seed: 42,
        rom_sha1: [0; 20],
        size: LevelSize::Expanded,
        density: Density::Original,
    }
}
fn baseline() -> PopulationBaseline {
    PopulationBaseline {
        walkable_tiles: 19 * 26 * 64,
        ordinary_items: 12,
        earthlings: 10,
        reserved_items: 0,
        ship_piece: false,
        mailbox: false,
    }
}
fn geometry(dimensions: Dimensions) -> LevelGeometry {
    LevelGeometry::new(
        dimensions,
        vec![Cell::new(1, 0).unwrap(); dimensions.area()],
        vec![0; dimensions.area()],
        Anchors {
            arrival: CellPosition { x: 2, y: 2 },
            exit: CellPosition {
                x: dimensions.width() - 3,
                y: dimensions.height() - 3,
            },
            ship_piece: None,
            regions: vec![],
        },
    )
    .unwrap()
}
fn population(dimensions: Dimensions) -> Population {
    let (x, y) = CellPosition { x: 1, y: 1 }.center();
    let placement = |index: u16, role| Placement {
        id: index,
        x,
        y,
        role,
        dormant_ticks: 0,
        ai_rng: StageRng::new(u64::from(index)),
    };
    let population = Population {
        budget: PopulationBudget {
            items: 128,
            earthlings: 64,
            reserved_items: 0,
            promotion_numerator: 1,
            promotion_denominator: 1,
        },
        items: (0..128)
            .map(|index| placement(index, PlacementRole::Resource))
            .collect(),
        earthlings: (0..64)
            .map(|index| placement(index, PlacementRole::Earthling))
            .collect(),
        collected: StableBits::new(128).unwrap(),
        removed: StableBits::new(64).unwrap(),
    };
    population.validate(dimensions).unwrap();
    population
}
fn import_spec(
    world: &WorldState,
    level: LevelId,
    kind: ImportKind,
    source: &LevelGeometry,
) -> ImportSpec {
    ImportSpec {
        config: world.config().clone(),
        level,
        kind,
        dimensions: source.dimensions(),
        anchors: source.anchors().clone(),
        population: population(source.dimensions()),
        source_digest: ImportSpec::content_digest(source.dimensions(), source).unwrap(),
    }
}
fn import(world: &mut WorldState, level: LevelId, dimensions: Dimensions) {
    let source = geometry(dimensions);
    let kind = if matches!(level.get(), 0 | 1 | 26) {
        ImportKind::Special(match level.get() {
            0 => SpecialFloor::Zero,
            1 => SpecialFloor::One,
            _ => SpecialFloor::Final,
        })
    } else {
        ImportKind::Designed {
            course_identity: [7; 32],
        }
    };
    world
        .request_import(import_spec(world, level, kind, &source))
        .unwrap();
    while world.importing().is_some() {
        world.advance_import(&source, &Synthetic, 73).unwrap();
    }
}
fn timing() -> TravelTiming {
    TravelTiming {
        join_ticks: 2,
        departure_ticks: 1,
        arrival_ticks: 1,
    }
}
fn until_ready(journey: &mut Journey, world: &mut WorldState, start: u64) -> u64 {
    for tick in start..start + 1_000_000 {
        match journey
            .advance(world, &Synthetic, baseline(), tick, 127, &[])
            .unwrap()
        {
            TravelPhase::Ready => return tick,
            TravelPhase::Failed => panic!("travel failed: {:?}", journey.failure()),
            _ => (),
        }
    }
    panic!("travel did not reach finite ready state")
}

#[test]
fn import_every_slice_roundtrips_and_wrong_content_cancel_never_publish() {
    let dimensions = Dimensions::new(64, 64).unwrap();
    let source = geometry(dimensions);
    let mut world = WorldState::new(config()).unwrap();
    let spec = import_spec(
        &world,
        id(2),
        ImportKind::Designed {
            course_identity: [3; 32],
        },
        &source,
    );
    world.request_import(spec.clone()).unwrap();
    for _ in 0..dimensions.area() - 1 {
        assert_eq!(
            world.advance_import(&source, &Synthetic, 1).unwrap(),
            ImportProgress::Loading
        );
        let bytes = bincode::encode_to_vec(&world, codec()).unwrap();
        world = decode(&bytes, world.config()).unwrap();
        assert!(world.floor(id(2)).is_none());
    }
    assert_eq!(
        world.advance_import(&source, &Synthetic, 1).unwrap(),
        ImportProgress::Ready
    );
    assert!(world.importing().is_none());
    world.validate().unwrap();
    assert!(world.request_import(spec.clone()).is_err());
    let mut rejected = WorldState::new(config()).unwrap();
    let mut bad = spec.clone();
    bad.source_digest[0] ^= 1;
    rejected.request_import(bad).unwrap();
    while rejected.importing().unwrap().progress() == ImportProgress::Loading {
        rejected.advance_import(&source, &Synthetic, 1024).unwrap();
    }
    assert_eq!(
        rejected.importing().unwrap().progress(),
        ImportProgress::Rejected
    );
    assert!(rejected.floors().is_empty());
    rejected.cancel_loading();
    assert!(rejected.importing().is_none());
    rejected.request_import(spec).unwrap();
    rejected.advance_import(&source, &Synthetic, 17).unwrap();
    rejected.cancel_loading();
    assert!(rejected.floors().is_empty());
    rejected.validate().unwrap();
}

#[test]
fn four_independent_level_lifetimes_keep_all_deltas_through_atomic_travel_restore_and_swaps() {
    let mut world = WorldState::new(config()).unwrap();
    let dimensions = Dimensions::new(64, 64).unwrap();
    for seat in 0..4_u8 {
        let level = id(u16::from(seat) + 2);
        import(&mut world, level, dimensions);
        world.set_active(seat, level).unwrap();
        let floor = world.floor_mut(level).unwrap();
        floor
            .replace_cell(
                CellPosition { x: 40, y: 40 },
                Cell::new(2 + seat, 0).unwrap(),
                0,
            )
            .unwrap();
        floor
            .generated
            .population
            .collected
            .insert(100 + u16::from(seat))
            .unwrap();
        floor
            .generated
            .population
            .removed
            .insert(40 + u16::from(seat))
            .unwrap();
        floor.discovery.visit(6000, 3000, dimensions).unwrap();
        world.drop_item(level, 6000, 3000, u16::from(seat)).unwrap();
        import(&mut world, id(u16::from(seat) + 6), dimensions);
    }
    let before = world.clone();
    let mut journeys: [Journey; 4] = std::array::from_fn(|seat| {
        let mut destinations = [None; 4];
        destinations[seat] = Some(Destination {
            level: id(seat as u16 + 6),
            landing: Landing::Arrival,
        });
        Journey::new(&world, TravelKind::Reenter, timing(), destinations, 0).unwrap()
    });
    for journey in &mut journeys {
        until_ready(journey, &mut world, 0);
        let bytes = bincode::encode_to_vec(&*journey, codec()).unwrap();
        let (restored, _): (Journey, usize) = bincode::decode_from_slice(&bytes, codec()).unwrap();
        *journey = restored;
        journey.validate(&world).unwrap();
        journey.commit(&mut world, &Synthetic, &[]).unwrap();
    }
    assert_eq!(
        *world.active(),
        [Some(id(6)), Some(id(7)), Some(id(8)), Some(id(9))]
    );
    world = decode(
        &bincode::encode_to_vec(&world, codec()).unwrap(),
        world.config(),
    )
    .unwrap();
    for seat in 0..4 {
        let mut destinations = [None; 4];
        destinations[seat] = Some(Destination {
            level: id(seat as u16 + 2),
            landing: Landing::Search,
        });
        let mut journey =
            Journey::new(&world, TravelKind::Reenter, timing(), destinations, 0).unwrap();
        until_ready(&mut journey, &mut world, 0);
        journey.commit(&mut world, &Synthetic, &[]).unwrap();
    }
    for level in 2..6 {
        assert_eq!(world.floor(id(level)), before.floor(id(level)));
    }
    assert_eq!(world.dropped(), before.dropped());
    world.copy_active_level(0, 3).unwrap();
    world.swap_active_levels(1, 2).unwrap();
    assert_eq!(world.active()[3], Some(id(2)));
    assert_eq!(world.floor(id(2)), before.floor(id(2)));
    let drop = world.take_drop(2).unwrap();
    assert_eq!(drop.level, id(4));
    assert!(world.take_drop(2).is_err());
    world.validate().unwrap();
}

#[test]
fn loading_generation_cancel_failure_and_cohort_commit_are_atomic() {
    let mut world = WorldState::new(config()).unwrap();
    import(&mut world, id(2), Dimensions::EXPANDED);
    world.set_active(0, id(2)).unwrap();
    let before = world.clone();
    let mut journey = Journey::new(
        &world,
        TravelKind::Elevator,
        timing(),
        [
            Some(Destination {
                level: id(3),
                landing: Landing::Arrival,
            }),
            None,
            None,
            None,
        ],
        0,
    )
    .unwrap();
    assert!(journey.commit(&mut world, &Synthetic, &[]).is_err());
    until_ready(&mut journey, &mut world, 0);
    assert_eq!(world.active(), before.active());
    assert!(world.floor(id(3)).is_some());
    let saved = world.clone();
    assert!(
        world
            .commit_cohort(3, [Some(id(3)), Some(id(31)), None, None])
            .is_err()
    );
    assert_eq!(world, saved);
    let arrival = journey.arrivals()[0].unwrap();
    let cell = world
        .floor(id(3))
        .unwrap()
        .generated
        .geometry
        .dimensions()
        .world_cell(arrival.x, arrival.y)
        .unwrap();
    world
        .floor_mut(id(3))
        .unwrap()
        .replace_cell(cell, Cell::new(0, 0).unwrap(), 0)
        .unwrap();
    let saved = world.clone();
    assert!(journey.commit(&mut world, &Synthetic, &[]).is_err());
    assert_eq!(world, saved);
    journey.cancel(&mut world).unwrap();
    journey.validate(&world).unwrap();
    world.request_load(id(4), baseline()).unwrap();
    world.advance_loading(&Synthetic, 17).unwrap();
    world.cancel_loading();
    assert!(world.floor(id(4)).is_none());
}

#[test]
fn elevator_join_special_floors_ship_objectives_and_falling_adapters_are_explicit() {
    let mut world = WorldState::new(config()).unwrap();
    for level in [0, 1, 2, 25, 26] {
        import(&mut world, id(level), Dimensions::ORIGINAL);
    }
    for seat in 0..4 {
        world.set_active(seat, id(1)).unwrap();
    }
    let mut elevator = Journey::new(
        &world,
        TravelKind::Elevator,
        timing(),
        [
            Some(Destination {
                level: id(2),
                landing: Landing::Arrival,
            }),
            None,
            None,
            None,
        ],
        0,
    )
    .unwrap();
    for seat in 1..4 {
        elevator.join(&world, seat).unwrap();
    }
    assert_eq!(elevator.cohort(), 15);
    let tick = until_ready(&mut elevator, &mut world, 0);
    assert!(elevator.join(&world, 1).is_err());
    let arrivals = elevator.commit(&mut world, &Synthetic, &[]).unwrap();
    assert!(arrivals.iter().all(Option::is_some));
    assert!(elevator.commit(&mut world, &Synthetic, &[]).is_err());
    elevator
        .advance(&mut world, &Synthetic, baseline(), tick + 1, 1, &[])
        .unwrap();
    elevator.validate(&world).unwrap();
    assert_eq!(
        Destination::campaign_fall(id(0), id(25)).unwrap().level,
        id(25)
    );
    assert_eq!(
        Destination::campaign_fall(id(1), id(25)).unwrap().level,
        id(0)
    );
    assert_eq!(
        Destination::campaign_fall(id(26), id(25)).unwrap().level,
        id(25)
    );
    for seat in 0..4 {
        world.set_active(seat, id(25)).unwrap();
    }
    let destinations = std::array::from_fn(|_| {
        Some(Destination {
            level: id(26),
            landing: Landing::Arrival,
        })
    });
    assert!(
        Journey::new(
            &world,
            TravelKind::Ship {
                required_parts: 0x3ff
            },
            timing(),
            destinations,
            0x1ff
        )
        .is_err()
    );
    assert!(Journey::new(&world, TravelKind::Elevator, timing(), destinations, 0x3ff).is_err());
    let mut ship = Journey::new(
        &world,
        TravelKind::Ship {
            required_parts: 0x3ff,
        },
        timing(),
        destinations,
        0x3ff,
    )
    .unwrap();
    until_ready(&mut ship, &mut world, 0);
    ship.commit(&mut world, &Synthetic, &[]).unwrap();
    assert_eq!(*world.active(), [Some(id(26)); 4]);
    for level in [0, 1, 26] {
        assert_eq!(
            world
                .floor(id(level))
                .unwrap()
                .generated
                .geometry
                .dimensions(),
            Dimensions::ORIGINAL
        );
    }
}

#[test]
fn marked_path_traversal_mutates_once_in_cardinal_order_and_survives_continuations() {
    let dimensions = Dimensions::EXPANDED;
    let mut geometry = geometry(dimensions);
    let start = CellPosition { x: 22, y: 31 };
    let right = CellPosition { x: 23, y: 31 };
    let down = CellPosition { x: 22, y: 32 };
    for (position, directions) in [(start, 6), (right, 8), (down, 1)] {
        geometry
            .replace_cell(position, Cell::new(0, 0).unwrap(), directions)
            .unwrap();
    }
    let mut land = LandState::from_geometry(&geometry).unwrap();
    let mut dirty = StableBits::new(dimensions.area() as u16).unwrap();
    let mut rng = StageRng::new(42);
    land.begin(start).unwrap();
    let mut order = vec![];
    for _ in 0..10 {
        if let Some(change) = land
            .step(&mut geometry, &mut dirty, &mut rng, &Synthetic)
            .unwrap()
        {
            order.push(change.position);
        }
        land.validate(&geometry).unwrap();
        let bytes = bincode::encode_to_vec(&land, codec()).unwrap();
        let (restored, _): (LandState, usize) =
            bincode::decode_from_slice(&bytes, codec()).unwrap();
        land = restored;
        if !land.busy() {
            break;
        }
    }
    assert_eq!(order, vec![start, right, start, down]);
    assert!(!land.busy());
    assert_eq!(dirty.count(), 3);
    for position in [start, right, down] {
        assert_eq!(
            geometry.path_state()[position.index(dimensions).unwrap()],
            0
        );
    }
    assert!(LandState::near_edge(&[0x90; 25]));
    assert!(!LandState::near_edge(&[0x10; 25]));
}

#[test]
fn placement_exhaustion_bounds_and_quanta_are_deterministic_at_stock_and_maximum_size() {
    for dimensions in [Dimensions::ORIGINAL, Dimensions::new(64, 64).unwrap()] {
        let mut marked = geometry(dimensions);
        for index in 0..dimensions.area() {
            marked
                .replace_cell(
                    dimensions.position(index).unwrap(),
                    Cell::new(1, 0).unwrap(),
                    15,
                )
                .unwrap();
        }
        let mut expected = None;
        for work in [1, 7, 1024] {
            let mut search = PlacementSearch::new(
                dimensions,
                Footprint::new(4, 2).unwrap(),
                TerrainMask::LAND_OR_SAND,
                StageRng::new(42),
            )
            .unwrap();
            for _ in 0..dimensions.area() + 11 {
                if search
                    .advance(&marked, id(2), &Synthetic, work, &[])
                    .unwrap()
                    == SearchProgress::Exhausted
                {
                    break;
                }
            }
            search.validate().unwrap();
            assert_eq!(search.progress(), SearchProgress::Exhausted);
            if let Some(expected) = &expected {
                assert_eq!(&search, expected);
            } else {
                expected = Some(search);
            }
        }
    }
    assert!(Footprint::new(0, 2).is_err());
    assert!(Footprint::new(17, 2).is_err());
    assert!(TerrainMask::new(4).is_err());
}

#[test]
fn telephone_doorway_and_elevator_contacts_never_depend_on_camera_or_extra_seat_sentinels() {
    let mut world = WorldState::new(config()).unwrap();
    import(&mut world, id(2), Dimensions::EXPANDED);
    let position = Position {
        level: id(2),
        x: 500,
        y: 500,
    };
    let mut phone = Telephone::new(position);
    let mut heroes = [None; 4];
    heroes[3] = Some(Position { x: 600, ..position });
    assert_eq!(phone.advance(&heroes, false, 0).unwrap(), PhoneEvent::Ring);
    let saved = phone.clone();
    assert_eq!(phone.advance(&heroes, false, 0).unwrap(), PhoneEvent::None);
    assert_eq!(phone, saved);
    heroes[3] = Some(position);
    assert_eq!(
        phone.advance(&heroes, false, 1).unwrap(),
        PhoneEvent::Reveal {
            seat: 3,
            level: id(2)
        }
    );
    let bytes = bincode::encode_to_vec(&phone, codec()).unwrap();
    let (restored, _): (Telephone, usize) = bincode::decode_from_slice(&bytes, codec()).unwrap();
    phone = restored;
    assert!(phone.finish_reveal(0).is_err());
    phone.finish_reveal(3).unwrap();
    assert_eq!(phone.phase(), PhonePhase::Spent);
    phone.validate(&world).unwrap();
    let mut doorway = Doorway::new(DoorwaySupport {
        owner: 3,
        level: id(2),
        x: 500,
        y: 500,
        open: false,
    });
    let heroes = [Some(Position { y: 508, ..position }); 4];
    assert_eq!(doorway.advance(&heroes, 0).unwrap(), 0);
    let heroes = [Some(Position { y: 499, ..position }); 4];
    assert_eq!(doorway.advance(&heroes, 1).unwrap(), 15);
    assert_eq!(doorway.advance(&heroes, 1).unwrap(), 0);
    assert_eq!(doorway.advance(&heroes, 2).unwrap(), 0);
    doorway.validate(&world).unwrap();
    let barrier = ElevatorBarrier {
        level: id(2),
        x: 500,
        y: 500,
        door: ElevatorDoor::Open,
    };
    assert_eq!(
        barrier.player_contact(id(2), (500, 510), (500, 507), 0, 0, false),
        ElevatorContact::Enter
    );
    assert_eq!(
        barrier.player_contact(id(2), (500, 507), (500, 510), 0, 0, true),
        ElevatorContact::Exit
    );
    assert_eq!(
        barrier.player_contact(id(2), (500, 510), (500, 499), 0, 0, false),
        ElevatorContact::Blocked
    );
    assert!(barrier.contains(id(2), (500, 505)));
    assert!(!barrier.contains(id(3), (500, 505)));
    assert!(barrier.blocks_entity(id(2), (500, 510), (500, 499)));
    assert!(!barrier.blocks_entity(id(2), (500, 510), (530, 499)));
}

#[test]
fn discovery_matches_original_translated_boundaries_and_does_not_double_award() {
    let mut discovery = Discovery::new(Dimensions::ORIGINAL).unwrap();
    assert!(discovery.visit(72, 8, Dimensions::ORIGINAL).unwrap());
    assert!(!discovery.visit(72, 8, Dimensions::ORIGINAL).unwrap());
    assert!(discovery.visit(2631, 1799, Dimensions::ORIGINAL).unwrap());
    assert!(!discovery.visit(2632, 1799, Dimensions::ORIGINAL).unwrap());
    assert!(!discovery.visit(2631, 1800, Dimensions::ORIGINAL).unwrap());
    assert!(
        !discovery
            .visit(i32::MIN, i32::MAX, Dimensions::ORIGINAL)
            .unwrap()
    );
    discovery.validate(Dimensions::ORIGINAL).unwrap();
    assert_eq!(discovery.awards(), 2);
}

#[test]
fn unsafe_landing_failure_leaves_all_four_sources_unchanged() {
    let mut world = WorldState::new(config()).unwrap();
    import(&mut world, id(2), Dimensions::EXPANDED);
    for seat in 0..4 {
        world.set_active(seat, id(2)).unwrap();
    }
    let destinations = std::array::from_fn(|_| {
        Some(Destination {
            level: id(2),
            landing: Landing::Exact(128, 64),
        })
    });
    let mut journey = Journey::new(&world, TravelKind::Doorway, timing(), destinations, 0).unwrap();
    let before = world.clone();
    for tick in 0..20 {
        journey
            .advance(&mut world, &Synthetic, baseline(), tick, 8, &[])
            .unwrap();
        if journey.phase() == TravelPhase::Failed {
            break;
        }
    }
    assert_eq!(journey.failure(), Some(TravelFailure::NoLanding));
    assert_eq!(world, before);
    journey.validate(&world).unwrap();
}

#[test]
fn every_journey_phase_roundtrips_and_duplicate_ticks_do_not_advance() {
    let mut world = WorldState::new(config()).unwrap();
    import(&mut world, id(2), Dimensions::EXPANDED);
    world.set_active(0, id(2)).unwrap();
    let mut journey = Journey::new(
        &world,
        TravelKind::Elevator,
        timing(),
        [
            Some(Destination {
                level: id(3),
                landing: Landing::Arrival,
            }),
            None,
            None,
            None,
        ],
        0,
    )
    .unwrap();
    let mut seen = [false; 8];
    for tick in 0..100_000 {
        seen[journey.phase() as usize] = true;
        let bytes = bincode::encode_to_vec((&world, &journey), codec()).unwrap();
        let ((restored_world, restored_journey), used): ((WorldState, Journey), usize) =
            bincode::decode_from_slice(&bytes, codec()).unwrap();
        assert_eq!(used, bytes.len());
        restored_world.validate().unwrap();
        restored_journey.validate(&restored_world).unwrap();
        world = restored_world;
        journey = restored_journey;
        journey
            .advance(&mut world, &Synthetic, baseline(), tick, 127, &[])
            .unwrap();
        let before = (world.clone(), journey.clone());
        journey
            .advance(&mut world, &Synthetic, baseline(), tick, 127, &[])
            .unwrap();
        assert_eq!((&world, &journey), (&before.0, &before.1));
        if journey.phase() == TravelPhase::Ready {
            journey.commit(&mut world, &Synthetic, &[]).unwrap();
            seen[TravelPhase::Ready as usize] = true;
        }
        if journey.phase() == TravelPhase::Complete {
            seen[TravelPhase::Complete as usize] = true;
            break;
        }
    }
    assert_eq!(&seen[..7], &[true; 7]);
    assert!(!seen[TravelPhase::Failed as usize]);
}

#[test]
fn cancelled_journey_discards_only_its_unpublished_load_and_remains_restorable() {
    let mut world = WorldState::new(config()).unwrap();
    import(&mut world, id(2), Dimensions::EXPANDED);
    world.set_active(0, id(2)).unwrap();
    let mut journey = Journey::new(
        &world,
        TravelKind::Reenter,
        timing(),
        [
            Some(Destination {
                level: id(3),
                landing: Landing::Search,
            }),
            None,
            None,
            None,
        ],
        0,
    )
    .unwrap();
    for tick in 0..3 {
        journey
            .advance(&mut world, &Synthetic, baseline(), tick, 1, &[])
            .unwrap();
    }
    assert!(world.pending().is_some());
    let floors = world.floors().to_vec();
    let active = *world.active();
    journey.cancel(&mut world).unwrap();
    assert!(world.pending().is_none());
    assert!(world.floor(id(3)).is_none());
    assert_eq!(world.floors(), floors);
    assert_eq!(*world.active(), active);
    journey.validate(&world).unwrap();
    let bytes = bincode::encode_to_vec(&journey, codec()).unwrap();
    let (restored, _): (Journey, usize) = bincode::decode_from_slice(&bytes, codec()).unwrap();
    restored.validate(&world).unwrap();
    world.request_load(id(4), baseline()).unwrap();
    let pending = world.pending().unwrap().clone();
    let mut unrelated = Journey::new(
        &world,
        TravelKind::Reenter,
        timing(),
        [
            Some(Destination {
                level: id(3),
                landing: Landing::Search,
            }),
            None,
            None,
            None,
        ],
        0,
    )
    .unwrap();
    unrelated.cancel(&mut world).unwrap();
    assert_eq!(world.pending(), Some(&pending));
}

#[test]
fn simultaneous_disjoint_loading_and_level_qualified_exclusions_keep_floors_independent() {
    use tje_world::terrain::Exclusion;
    let mut world = WorldState::new(config()).unwrap();
    for seat in 0..4 {
        import(&mut world, id(seat as u16 + 2), Dimensions::EXPANDED);
        world.set_active(seat, id(seat as u16 + 2)).unwrap();
    }
    let mut journeys: [Journey; 4] = std::array::from_fn(|seat| {
        let mut destinations = [None; 4];
        destinations[seat] = Some(Destination {
            level: id(seat as u16 + 6),
            landing: Landing::Arrival,
        });
        Journey::new(&world, TravelKind::Reenter, timing(), destinations, 0).unwrap()
    });
    // Identical coordinates on a dormant unrelated level must block no arrival.
    let exclusions = [Exclusion {
        level: id(2),
        x: 0,
        y: 0,
        half_width: u16::MAX,
        half_height: u16::MAX,
    }];
    for tick in 0..100_000 {
        for journey in &mut journeys {
            if journey.committed() {
                continue;
            }
            journey
                .advance(&mut world, &Synthetic, baseline(), tick, 127, &exclusions)
                .unwrap();
            assert_ne!(journey.phase(), TravelPhase::Failed);
            if journey.phase() == TravelPhase::Ready {
                journey.commit(&mut world, &Synthetic, &exclusions).unwrap();
            }
        }
        if journeys.iter().all(Journey::committed) {
            break;
        }
    }
    assert!(journeys.iter().all(Journey::committed));
    assert_eq!(
        *world.active(),
        [Some(id(6)), Some(id(7)), Some(id(8)), Some(id(9))]
    );
    world.validate().unwrap();
}
