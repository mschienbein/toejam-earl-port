use super::*;
use crate::players::{Landing, PlayerGeometry, PlayerResult, PlayerServices};
use crate::population::{
    Definition, DefinitionCatalog, Movement, PopulationLimits, PopulationState, ProjectileVectors,
    Thinker, TickRange, Touch,
};
use crate::resources::{ItemDefinitions, ResourceLimits};
use tje_world::{
    Cell, Density, Dimensions, LevelSize, Resources, Shape,
    original::resources::{OriginalResources, ShapeBank},
};

struct Authored;
impl Resources for Authored {
    fn parameter(&self, _: LevelId, index: u8) -> tje_world::Result<(u8, u8)> {
        Ok(if index == 6 { (3, 3) } else { (40, 40) })
    }
    fn shape(&self, _: Shape, _: u8, _: bool) -> tje_world::Result<u8> {
        Ok(30)
    }
    fn alternative(&self, _: u8, _: u32) -> tje_world::Result<Cell> {
        Cell::new(30, 0)
    }
    fn transition(&self, _: u8, _: u8, _: u32) -> tje_world::Result<Option<Cell>> {
        Ok(None)
    }
    fn base_tile(&self, _: u8, _: u8, _: u8) -> tje_world::Result<u16> {
        Ok(1)
    }
    fn overlay_tile(&self, _: u8, _: u8, _: u8) -> tje_world::Result<u16> {
        Ok(0)
    }
    fn property(&self, _: u16) -> tje_world::Result<u8> {
        Ok(0x10)
    }
}
impl OriginalResources for Authored {
    fn original_revision(&self) -> OriginalRevision {
        OriginalRevision::Rev02
    }
    fn original_shape(&self, _: ShapeBank, _: u8) -> tje_world::Result<u8> {
        Ok(30)
    }
    fn original_turn(&self, direction: u8, choice: u8) -> tje_world::Result<u8> {
        let directions = [1, 2, 4, 8];
        let index = directions
            .iter()
            .position(|value| *value == direction)
            .unwrap();
        Ok(directions[(index + usize::from(choice)) % 4])
    }
    fn original_transition_contains(&self, _: u8, _: u8) -> tje_world::Result<bool> {
        Ok(false)
    }
    fn original_special_word(
        &self,
        _: tje_world::loading::SpecialFloor,
        _: usize,
    ) -> tje_world::Result<u16> {
        Ok(30)
    }
    fn original_level1_item(&self, index: u8) -> tje_world::Result<(i32, i32)> {
        Ok((256 + i32::from(index) * 64, 256))
    }
    fn original_present_weight(&self, _: u8) -> tje_world::Result<u8> {
        Ok(1)
    }
    fn original_large_excluded(&self, _: u16) -> tje_world::Result<bool> {
        Ok(false)
    }
}
impl ItemDefinitions for Authored {
    fn food_health_delta(&self, _: u8) -> PlayerResult<i8> {
        Ok(4)
    }
}
fn world_config() -> WorldConfig {
    WorldConfig {
        root_seed: 42,
        rom_sha1: [0; 20],
        size: LevelSize::Original,
        density: Density::Original,
    }
}
fn catalog() -> DefinitionCatalog {
    let time = TickRange {
        minimum: 0,
        maximum: 0,
    };
    let definition = Definition {
        thinker: Thinker::None,
        movement: Movement::None,
        touch: Touch::Hurt,
        axes: 0,
        width: 16,
        height: 16,
        target_chance: 0,
        target_distance: 0,
        turn_chance: 0,
        companion: false,
        change_axis: false,
        radius: 0,
        contact_x: 0,
        contact_y: 0,
        damage: 0,
        target_time: time,
        turn_time: time,
        move_time: time,
        state_delay: 0,
        health: 1,
    };
    DefinitionCatalog::new(
        [definition; 25],
        ProjectileVectors {
            thresholds: [0; 8],
            velocities: [[0; 2]; 9],
        },
    )
    .unwrap()
}
struct Terrain<'a>(&'a WorldState);
impl PlayerGeometry for Terrain<'_> {
    fn dimensions(&self, level: LevelId) -> PlayerResult<Dimensions> {
        self.0
            .floor(level)
            .map(|floor| floor.generated.geometry.dimensions())
            .ok_or(crate::players::PlayerError::Geometry("fixture floor"))
    }
    fn terrain(&self, level: LevelId, x: i32, y: i32) -> PlayerResult<u8> {
        self.0
            .floor(level)
            .ok_or(crate::players::PlayerError::Geometry("fixture floor"))?
            .generated
            .geometry
            .terrain_at(level, x, y, &Authored)
            .map_err(|_| crate::players::PlayerError::Geometry("authored terrain"))
    }
    fn blocked(
        &self,
        _: SeatId,
        _: tje_four_player::Archetype,
        _: LevelId,
        _: Position,
    ) -> PlayerResult<bool> {
        Ok(false)
    }
    fn respawn(&self, _: SeatId, checkpoint: Landing) -> PlayerResult<Landing> {
        self.dimensions(checkpoint.level)?;
        Ok(checkpoint)
    }
}
fn fixture() -> (
    LoadingState,
    WorldState,
    CoopState,
    PlayerServices,
    PopulationState,
) {
    let world = WorldState::new(world_config()).unwrap();
    let coop = CoopState::new(tje_four_player::state::Config::four_seats(42)).unwrap();
    let loading = LoadingState::new(
        world.config(),
        coop.config().participants,
        OriginalRevision::Rev02,
    )
    .unwrap();
    let players = PlayerServices::new(42, ResourceLimits::default()).unwrap();
    let population = PopulationState::new(PopulationLimits::default(), [1; 32]).unwrap();
    (loading, world, coop, players, population)
}
fn load_first(
    loading: &mut LoadingState,
    world: &mut WorldState,
    players: &mut PlayerServices,
    population: &mut PopulationState,
) {
    let catalog = catalog();
    let sources = LoadingSources {
        terrain: &Authored,
        items: &Authored,
        catalog: &catalog,
        stock_kind: |_, _| Ok(Some(crate::population::Kind::Devil)),
    };
    loading
        .prepare_floor(world, LevelId::new(1).unwrap())
        .unwrap();
    for tick in 0..100 {
        loading
            .advance_sources(world, players, population, &sources, tick, 1024)
            .unwrap();
        if world.floor(LevelId::new(1).unwrap()).is_some() {
            return;
        }
    }
    panic!("bounded authored first floor did not publish");
}
#[test]
fn first_source_and_initial_arrival_are_independent_atomic_transactions() {
    let (mut loading, mut world, mut coop, mut players, mut population) = fixture();
    load_first(&mut loading, &mut world, &mut players, &mut population);
    assert!(
        coop.players
            .iter()
            .all(|hero| hero.life == LifeState::Unplaced)
    );
    assert_eq!(*world.active(), [None; 4]);
    assert_eq!(*population.retained(), [None; 4]);
    for tick in 100..200 {
        if loading
            .advance_initial_positions(&world, &coop, &Authored, tick, 32)
            .unwrap()
        {
            break;
        }
    }
    let prepared = loading
        .prepare_initial_arrivals(&world, &coop, &players, &population, &Terrain(&world))
        .unwrap();
    loading.commit_arrivals(
        prepared,
        &mut world,
        &mut coop,
        &mut players,
        &mut population,
    );
    assert!(loading.initial_setup);
    let level = LevelId::new(1).unwrap();
    for seat in SeatId::ALL {
        assert_eq!(coop.players[seat.index()].life, LifeState::Alive);
        assert_eq!(coop.levels[level.index()].arrivals[seat.index()], 1);
        assert_eq!(
            coop.players[seat.index()]
                .inventory
                .iter()
                .flatten()
                .count(),
            2
        );
        assert_eq!(
            players.runtime[seat.index()].checkpoint.unwrap().level,
            level
        );
    }
    assert_eq!(*world.active(), [Some(level); 4]);
    population.validate_world(&world).unwrap();
    players.validate(&coop).unwrap();
    loading.validate_world(&world, &coop).unwrap();
}
#[test]
fn cancelled_source_never_publishes_or_changes_a_seat() {
    let (mut loading, mut world, coop, mut players, mut population) = fixture();
    let before = coop.clone();
    let catalog = catalog();
    let sources = LoadingSources {
        terrain: &Authored,
        items: &Authored,
        catalog: &catalog,
        stock_kind: |_, _| Ok(None),
    };
    let level = LevelId::new(1).unwrap();
    loading.prepare_floor(&world, level).unwrap();
    loading
        .advance_sources(&mut world, &mut players, &mut population, &sources, 0, 17)
        .unwrap();
    loading.cancel_source(&mut world, level).unwrap();
    assert!(world.floors().is_empty());
    assert!(population.floors().is_empty());
    assert!(players.resources.items().is_empty());
    assert_eq!(coop, before);
}
#[test]
fn immutable_campaign_seed_and_revision_restore_checks_fail_closed() {
    let (mut loading, world, coop, _, _) = fixture();
    loading
        .validate_config(
            world.config(),
            coop.config().participants,
            OriginalRevision::Rev02,
            GeneratorId::OriginalV1,
        )
        .unwrap();
    assert!(
        loading
            .validate_config(
                world.config(),
                coop.config().participants,
                OriginalRevision::Rev00,
                GeneratorId::OriginalV1
            )
            .is_err()
    );
    loading
        .campaign
        .as_mut()
        .unwrap()
        .accepted_seeds
        .replace_accepted(LevelId::new(2).unwrap(), 999)
        .unwrap();
    assert!(loading.validate_world(&world, &coop).is_err());
}
#[test]
fn source_budget_and_codec_restore_keep_identical_progress() {
    let (mut loading, mut world, coop, mut players, mut population) = fixture();
    let level = LevelId::new(1).unwrap();
    loading.prepare_floor(&world, level).unwrap();
    let catalog = catalog();
    let sources = LoadingSources {
        terrain: &Authored,
        items: &Authored,
        catalog: &catalog,
        stock_kind: |_, _| Ok(None),
    };
    loading
        .advance_sources(&mut world, &mut players, &mut population, &sources, 0, 17)
        .unwrap();
    let bytes = bincode::encode_to_vec(&loading, tje_world::world::codec()).unwrap();
    let (mut restored, used): (LoadingState, usize) =
        bincode::decode_from_slice(&bytes, tje_world::world::codec()).unwrap();
    assert_eq!(used, bytes.len());
    restored.validate_world(&world, &coop).unwrap();
    let mut other_world = world.clone();
    let mut other_players = players.clone();
    let mut other_population = population.clone();
    for tick in 1..10 {
        loading
            .advance_sources(
                &mut world,
                &mut players,
                &mut population,
                &sources,
                tick,
                127,
            )
            .unwrap();
        restored
            .advance_sources(
                &mut other_world,
                &mut other_players,
                &mut other_population,
                &sources,
                tick,
                127,
            )
            .unwrap();
    }
    assert_eq!(loading, restored);
    assert_eq!(world, other_world);
    assert_eq!(players, other_players);
    assert_eq!(population, other_population);
}
#[test]
fn stored_geometry_epoch_changes_only_for_successful_geometry_mutations() {
    let (mut loading, mut world, _, mut players, mut population) = fixture();
    load_first(&mut loading, &mut world, &mut players, &mut population);
    let floor = world.floor_mut(LevelId::new(1).unwrap()).unwrap();
    assert_eq!(floor.geometry_epoch(), 0);
    assert!(
        floor
            .replace_cell(
                tje_world::CellPosition { x: 99, y: 0 },
                Cell::new(30, 0).unwrap(),
                0
            )
            .is_err()
    );
    assert_eq!(floor.geometry_epoch(), 0);
    floor
        .replace_cell(
            tje_world::CellPosition { x: 0, y: 0 },
            Cell::new(30, 0).unwrap(),
            0,
        )
        .unwrap();
    assert_eq!(floor.geometry_epoch(), 1);
    assert_eq!(floor.discovery.columns(), 8);
    assert_eq!(floor.discovery.rows(), 7);
}

fn arrived_first_fixture() -> (LoadingState, WorldState, CoopState) {
    let (mut loading, mut world, mut coop, mut players, mut population) = fixture();
    load_first(&mut loading, &mut world, &mut players, &mut population);
    for tick in 100..200 {
        if loading
            .advance_initial_positions(&world, &coop, &Authored, tick, 32)
            .unwrap()
        {
            break;
        }
    }
    let prepared = loading
        .prepare_initial_arrivals(&world, &coop, &players, &population, &Terrain(&world))
        .unwrap();
    loading.commit_arrivals(
        prepared,
        &mut world,
        &mut coop,
        &mut players,
        &mut population,
    );
    (loading, world, coop)
}

#[test]
fn elevator_approach_opens_and_sensors_board_four_seats_in_one_visible_window() {
    let (mut loading, world, mut coop) = arrived_first_fixture();
    let level = LevelId::new(1).unwrap();
    let barrier = loading.elevator_barriers(&world, level).unwrap()[0];
    assert_eq!(barrier.door, ElevatorDoor::Closed);
    for hero in &mut coop.players {
        hero.motion.position = Position {
            x: barrier.x * 256,
            y: (barrier.y + 12) * 256,
            elevation: 0,
        };
    }
    let neutral = [crate::players::PlayerInput::default(); 4];
    for tick in 200..214 {
        loading.advance_elevator_doors(&world, &coop, tick).unwrap();
        assert_eq!(
            loading
                .poll_elevator_sensors(&world, &mut coop, &neutral, tick, 0)
                .unwrap(),
            SeatMask::NONE
        );
    }
    assert_eq!(
        loading.elevator_barriers(&world, level).unwrap()[0].door,
        ElevatorDoor::Open
    );
    for hero in &mut coop.players {
        hero.motion.position.y = (barrier.y + 7) * 256;
    }
    let up = [crate::players::PlayerInput {
        directions: 1,
        ..Default::default()
    }; 4];
    assert_eq!(
        loading
            .poll_elevator_sensors(&world, &mut coop, &up, 214, 0)
            .unwrap(),
        SeatMask::ALL
    );
    assert_eq!(loading.travelling_mask(), SeatMask::ALL);
    assert_eq!(loading.journeys.iter().flatten().count(), 1);
    let saved = loading.clone();
    assert_eq!(
        loading
            .poll_elevator_sensors(&world, &mut coop, &up, 214, 0)
            .unwrap(),
        SeatMask::NONE
    );
    assert_eq!(loading, saved);
    loading.validate_world(&world, &coop).unwrap();
    let props = loading.elevator_props(&world, level).unwrap();
    assert_eq!(props[0].unwrap().riders, SeatMask::ALL);
    assert!(!props[0].unwrap().arrival);
}

#[test]
fn elevator_sensors_and_door_ticks_restore_and_admit_independently() {
    let (mut loading, world, mut coop) = arrived_first_fixture();
    let level = LevelId::new(1).unwrap();
    let barrier = loading.elevator_barriers(&world, level).unwrap()[0];
    let neutral = [crate::players::PlayerInput::default(); 4];
    for hero in &mut coop.players {
        hero.motion.position = Position {
            x: barrier.x * 256,
            y: (barrier.y + 12) * 256,
            elevation: 0,
        };
    }
    let closed = loading.elevators.clone();
    for tick in [200, 202] {
        assert_eq!(
            loading
                .poll_elevator_sensors(&world, &mut coop, &neutral, tick, 0)
                .unwrap(),
            SeatMask::NONE
        );
    }
    assert_eq!(loading.elevators, closed);
    loading.advance_elevator_doors(&world, &coop, 202).unwrap();
    assert_eq!(
        loading.elevators[0].doors[0],
        DoorMotion::Opening { remaining: 12 }
    );
    let saved = loading.clone();
    loading.advance_elevator_doors(&world, &coop, 202).unwrap();
    assert_eq!(loading, saved);
    for step in 1..=12 {
        let movement_zero = 200 + step * 4;
        loading
            .poll_elevator_sensors(&world, &mut coop, &neutral, movement_zero, 0)
            .unwrap();
        loading
            .advance_elevator_doors(&world, &coop, movement_zero + 2)
            .unwrap();
        loading
            .poll_elevator_sensors(&world, &mut coop, &neutral, movement_zero + 2, 0)
            .unwrap();
    }
    assert_eq!(
        loading.elevator_barriers(&world, level).unwrap()[0].door,
        ElevatorDoor::Open
    );
    let doors = loading.elevators.clone();
    for hero in &mut coop.players {
        hero.motion.position.y = (barrier.y + 7) * 256;
    }
    let up = [crate::players::PlayerInput {
        directions: 1,
        ..Default::default()
    }; 4];
    // Entry occurs after phase-zero XY, before another phase-two door tick.
    assert_eq!(
        loading
            .poll_elevator_sensors(&world, &mut coop, &up, 252, 0)
            .unwrap(),
        SeatMask::ALL
    );
    assert_eq!(loading.elevator_sensor_tick, Some(252));
    assert_eq!(loading.elevator_door_tick, Some(250));
    assert_eq!(loading.elevators, doors);
    assert_eq!(loading.journeys.iter().flatten().count(), 1);
    let journeys = loading.journeys.clone();
    loading.advance_elevator_doors(&world, &coop, 254).unwrap();
    assert_eq!(loading.journeys, journeys);
    let doors = loading.elevators.clone();
    let mut down = neutral;
    down[0].directions = 2;
    assert_eq!(
        loading
            .poll_elevator_sensors(&world, &mut coop, &down, 254, 0)
            .unwrap(),
        SeatMask::NONE
    );
    assert_eq!(loading.travelling_mask(), SeatMask::new(0b1110).unwrap());
    assert_eq!(coop.players[0].motion.position.y, (barrier.y + 12) * 256);
    assert_eq!(loading.elevators, doors);
    coop.players[0].motion.position.y = (barrier.y + 7) * 256;
    assert_eq!(
        loading
            .poll_elevator_sensors(&world, &mut coop, &up, 256, 0)
            .unwrap(),
        SeatMask::new(1).unwrap()
    );
    assert_eq!(loading.travelling_mask(), SeatMask::ALL);
    let bytes = bincode::encode_to_vec(&loading, tje_world::world::codec()).unwrap();
    let (mut restored, used): (LoadingState, usize) =
        bincode::decode_from_slice(&bytes, tje_world::world::codec()).unwrap();
    assert_eq!(used, bytes.len());
    restored.validate_world(&world, &coop).unwrap();
    assert_eq!(restored, loading);
    assert_eq!(
        restored
            .poll_elevator_sensors(&world, &mut coop, &up, 256, 0)
            .unwrap(),
        SeatMask::NONE
    );
    restored.advance_elevator_doors(&world, &coop, 254).unwrap();
    assert_eq!(restored, loading);
    assert!(
        restored
            .poll_elevator_sensors(&world, &mut coop, &up, 255, 0)
            .is_err()
    );
    assert!(restored.advance_elevator_doors(&world, &coop, 253).is_err());
    assert_eq!(restored, loading);
}

#[test]
fn seeded_special_prefix_is_one_candidate_per_unit_and_snapshot_safe() {
    use tje_world::original::items::{StockSpecialJob, StockSpecialPositions};
    let dimensions = Dimensions::ORIGINAL;
    let geometry = tje_world::LevelGeometry::new(
        dimensions,
        vec![Cell::new(34, 0).unwrap(); dimensions.area()],
        vec![0; dimensions.area()],
        tje_world::Anchors {
            arrival: tje_world::CellPosition { x: 2, y: 2 },
            exit: tje_world::CellPosition { x: 5, y: 5 },
            ship_piece: None,
            regions: vec![],
        },
    )
    .unwrap();
    let level = LevelId::new(4).unwrap();
    let expected = StockSpecialPositions::prepare(level, 42, &geometry, None, &Authored).unwrap();
    assert_eq!(expected.failed, 1);
    for quantum in [1, 7, 64, 1024] {
        let mut job = StockSpecialJob::new(level, 42, &geometry, None).unwrap();
        let mut calls = 0u32;
        loop {
            let done = job.advance(&geometry, &Authored, quantum).unwrap();
            calls += 1;
            let bytes = bincode::encode_to_vec(&job, tje_world::world::codec()).unwrap();
            let (restored, used): (StockSpecialJob, usize) =
                bincode::decode_from_slice(&bytes, tje_world::world::codec()).unwrap();
            assert_eq!(used, bytes.len());
            job = restored;
            job.validate().unwrap();
            if done {
                break;
            }
        }
        assert_eq!(job.finish().unwrap(), expected);
        assert_eq!(calls, 504u32.div_ceil(quantum));
    }
}

fn single_support_cell_fixture() -> (
    LoadingState,
    WorldState,
    CoopState,
    PlayerServices,
    PopulationState,
) {
    let (mut loading, mut world, coop, mut players, mut population) = fixture();
    load_first(&mut loading, &mut world, &mut players, &mut population);
    let floor = world.floor_mut(LevelId::new(1).unwrap()).unwrap();
    let dimensions = floor.generated.geometry.dimensions();
    for index in 0..dimensions.area() {
        let position = dimensions.position(index).unwrap();
        let base = if position == (tje_world::CellPosition { x: 3, y: 4 }) {
            30
        } else {
            34
        };
        floor
            .replace_cell(position, Cell::new(base, 0).unwrap(), 0)
            .unwrap();
    }
    (loading, world, coop, players, population)
}

#[test]
fn four_initial_heroes_share_one_full_support_cell_with_saved_subcell_work() {
    let (loading, world, coop, players, population) = single_support_cell_fixture();
    let level = LevelId::new(1).unwrap();
    let support = tje_world::CellPosition { x: 3, y: 4 };
    let mut expected = None;
    for quantum in [1, 7, 128] {
        let mut state = loading.clone();
        let mut ready = false;
        let mut saved_subcell = false;
        for tick in 100..10000 {
            ready = state
                .advance_initial_positions(&world, &coop, &Authored, tick, quantum)
                .unwrap();
            if state.initial_subcell.iter().any(|cursor| *cursor != 0) {
                saved_subcell = true;
                let bytes = bincode::encode_to_vec(&state, tje_world::world::codec()).unwrap();
                let (restored, used): (LoadingState, usize) =
                    bincode::decode_from_slice(&bytes, tje_world::world::codec()).unwrap();
                assert_eq!(used, bytes.len());
                state = restored;
                state.validate_world(&world, &coop).unwrap();
            }
            if ready {
                break;
            }
        }
        assert!(ready);
        if quantum == 1 {
            assert!(saved_subcell);
        }
        if let Some(expected) = expected {
            assert_eq!(state.initial_arrivals, expected);
        } else {
            expected = Some(state.initial_arrivals);
        }
        let dimensions = world.floor(level).unwrap().generated.geometry.dimensions();
        for (index, landing) in state.initial_arrivals.iter().enumerate() {
            let position = landing.unwrap().position;
            let (x, y) = (position.x >> 8, position.y >> 8);
            assert_eq!(dimensions.world_cell(x - 16, y - 14), Some(support));
            assert_eq!(dimensions.world_cell(x + 15, y + 1), Some(support));
            for other in state.initial_arrivals[..index].iter().flatten() {
                assert!(
                    (i64::from(other.position.x >> 8) - i64::from(x)).abs() >= 32
                        || (i64::from(other.position.y >> 8) - i64::from(y)).abs() >= 16
                );
            }
        }
        let mut committed_world = world.clone();
        let mut committed_coop = coop.clone();
        let mut committed_players = players.clone();
        let mut committed_population = population.clone();
        let prepared = state
            .prepare_initial_arrivals(&world, &coop, &players, &population, &Terrain(&world))
            .unwrap();
        state.commit_arrivals(
            prepared,
            &mut committed_world,
            &mut committed_coop,
            &mut committed_players,
            &mut committed_population,
        );
        assert!(
            committed_coop
                .players
                .iter()
                .all(|hero| hero.life == LifeState::Alive)
        );
        assert_eq!(*committed_world.active(), [Some(level); 4]);
        state
            .validate_world(&committed_world, &committed_coop)
            .unwrap();
    }
}

struct Unsupported;
impl Resources for Unsupported {
    fn parameter(&self, level: LevelId, index: u8) -> tje_world::Result<(u8, u8)> {
        Authored.parameter(level, index)
    }
    fn shape(&self, shape: Shape, directions: u8, mirrored: bool) -> tje_world::Result<u8> {
        Authored.shape(shape, directions, mirrored)
    }
    fn alternative(&self, base: u8, random: u32) -> tje_world::Result<Cell> {
        Authored.alternative(base, random)
    }
    fn transition(&self, base: u8, directions: u8, random: u32) -> tje_world::Result<Option<Cell>> {
        Authored.transition(base, directions, random)
    }
    fn base_tile(&self, base: u8, row: u8, column: u8) -> tje_world::Result<u16> {
        Authored.base_tile(base, row, column)
    }
    fn overlay_tile(&self, overlay: u8, row: u8, column: u8) -> tje_world::Result<u16> {
        Authored.overlay_tile(overlay, row, column)
    }
    fn property(&self, _: u16) -> tje_world::Result<u8> {
        Ok(0x80)
    }
}
#[test]
fn initial_cohort_never_accepts_a_full_cell_without_real_footprint_support() {
    let (mut loading, world, coop, _, _) = single_support_cell_fixture();
    let mut exhausted = false;
    for tick in 100..10000 {
        match loading.advance_initial_positions(&world, &coop, &Unsupported, tick, 1) {
            Ok(ready) => assert!(!ready),
            Err(Error::Capacity("initial four-hero safe placement")) => {
                exhausted = true;
                break;
            }
            Err(error) => panic!("unexpected initial placement error: {error:?}"),
        }
    }
    assert!(exhausted);
    assert_eq!(loading.initial_arrivals, [None; 4]);
    assert!(
        coop.players
            .iter()
            .all(|hero| hero.life == LifeState::Unplaced)
    );
    assert_eq!(*world.active(), [None; 4]);
}

#[test]
fn elevator_props_publish_internal_door_and_moving_descriptor_states_readonly() {
    let (mut loading, mut world, coop) = arrived_first_fixture();
    let level = LevelId::new(1).unwrap();
    assert_eq!(
        loading.elevator_props(&world, level).unwrap()[0]
            .unwrap()
            .animation_state,
        1
    );
    let slot = loading.ensure_elevator(level);
    for (motion, expected, physics) in [
        (DoorMotion::Closed, 1, ElevatorDoor::Closed),
        (
            DoorMotion::Opening { remaining: 12 },
            2,
            ElevatorDoor::Closed,
        ),
        (DoorMotion::Open { remaining: 30 }, 3, ElevatorDoor::Open),
        (
            DoorMotion::Closing { remaining: 12 },
            4,
            ElevatorDoor::Closed,
        ),
    ] {
        loading.elevators[slot].doors = [motion; 2];
        let before = loading.clone();
        let props = loading.elevator_props(&world, level).unwrap();
        assert_eq!(props[0].unwrap().animation_state, expected);
        assert_eq!(props[0].unwrap().door, physics);
        assert!(!props[0].unwrap().moving);
        if motion != DoorMotion::Closed {
            assert_eq!(props[1].unwrap().animation_state, expected);
            assert_eq!(props[1].unwrap().door, physics);
        } else {
            assert!(props[1].is_none());
        }
        assert_eq!(loading, before);
    }
    loading.elevators[slot].doors = [DoorMotion::Open { remaining: 30 }; 2];
    let index = loading.begin_elevator(&world, SeatMask::ALL, 0).unwrap();
    assert_eq!(
        loading.elevator_props(&world, level).unwrap()[0]
            .unwrap()
            .animation_state,
        3
    );
    let baseline = tje_world::population::PopulationBaseline {
        walkable_tiles: Dimensions::ORIGINAL.area() as u32 * 128,
        ordinary_items: 12,
        earthlings: 0,
        reserved_items: 0,
        ship_piece: false,
        mailbox: false,
    };
    for tick in 0..30 {
        loading.journeys[index]
            .as_mut()
            .unwrap()
            .advance(&mut world, &Authored, baseline, tick, 1, &[])
            .unwrap();
    }
    assert_eq!(
        loading.journeys[index].as_ref().unwrap().phase(),
        tje_world::transitions::TravelPhase::Departing
    );
    let before = loading.clone();
    let props = loading.elevator_props(&world, level).unwrap();
    assert_eq!(props[0].unwrap().animation_state, 5);
    assert!(props[0].unwrap().moving);
    assert_eq!(props[1].unwrap().animation_state, 3);
    assert!(!props[1].unwrap().moving);
    assert_eq!(loading, before);
    loading.validate_world(&world, &coop).unwrap();
}
