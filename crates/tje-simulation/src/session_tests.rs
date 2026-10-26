use crate::bridge::{self, Operation, PhaseServices};
use crate::session::{Roles, Session, SessionConfig, SetupRoot, SetupTransaction, SimulationState};
use crate::{Admission, Error, LevelId, Phase, PublicationStamp, Result, SeatMask};
use tje_four_player::{
    CoopState, SeatId,
    input::InputFrame,
    state::{Arrival, Config, Position},
};
use tje_world::{Cell, Density, LevelSize, Resources, Shape, WorldConfig, world::WorldState};

// Authored solid/void terrain only: no ROM map or recovered lookup table.
pub(crate) struct Solid;
impl Resources for Solid {
    fn parameter(&self, _: LevelId, parameter: u8) -> tje_world::Result<(u8, u8)> {
        Ok(if parameter == 6 { (3, 6) } else { (55, 70) })
    }
    fn shape(&self, _: Shape, _: u8, alternate: bool) -> tje_world::Result<u8> {
        Ok(if alternate { 2 } else { 1 })
    }
    fn alternative(&self, shape: u8, _: u32) -> tje_world::Result<Cell> {
        Cell::new(shape, 0)
    }
    fn transition(&self, _: u8, _: u8, _: u32) -> tje_world::Result<Option<Cell>> {
        Ok(None)
    }
    fn base_tile(&self, base: u8, _: u8, _: u8) -> tje_world::Result<u16> {
        Ok(u16::from(base & 127 != 0))
    }
    fn overlay_tile(&self, _: u8, _: u8, _: u8) -> tje_world::Result<u16> {
        Ok(0)
    }
    fn property(&self, tile: u16) -> tje_world::Result<u8> {
        Ok(if tile == 1 { 0x10 } else { 0x90 })
    }
}

pub(crate) fn session(seed: u64) -> Session {
    session_for_roles(seed, Roles::coop(SeatMask::ALL))
}

fn session_for_roles(seed: u64, roles: Roles) -> Session {
    use tje_world::generation::Phase as Loading;
    let world_config = WorldConfig {
        root_seed: seed,
        rom_sha1: [0; 20],
        size: LevelSize::Original,
        density: Density::Original,
    };
    let mut coop_config = Config::four_seats(seed);
    coop_config.participants = roles.heroes;
    let mut config = SessionConfig::new(
        world_config.clone(),
        coop_config.clone(),
        tje_world::original::OriginalRevision::Rev02,
        tje_world::GeneratorId::ExpandedV1,
        None,
        &[],
    )
    .unwrap();
    config.roles = roles;
    let mut world = WorldState::new(world_config).unwrap();
    let level = LevelId::new(2).unwrap();
    world
        .request_load(
            level,
            tje_world::population::PopulationBaseline {
                walkable_tiles: 19 * 26 * 64,
                ordinary_items: 12,
                earthlings: 10,
                reserved_items: 2,
                ship_piece: true,
                mailbox: true,
            },
        )
        .unwrap();
    let mut loaded = false;
    for _ in 0..100_000 {
        match world.advance_loading(&Solid, 1024).unwrap() {
            Loading::Complete => {
                loaded = true;
                break;
            }
            Loading::Rejected => panic!("synthetic aggregate floor rejected"),
            _ => {}
        }
    }
    assert!(loaded);
    let (x, y) = world
        .floor(level)
        .unwrap()
        .generated
        .geometry
        .anchors()
        .arrival
        .center();
    let mut players = CoopState::new(coop_config).unwrap();
    let arrivals: Vec<_> = roles
        .heroes
        .iter()
        .map(|seat| Arrival {
            seat,
            level,
            position: Position {
                x: x * 256,
                y: y * 256,
                elevation: 0,
            },
        })
        .collect();
    players.record_arrivals(&arrivals).unwrap();
    for seat in roles.heroes.iter() {
        world.set_active(seat.get(), level).unwrap();
    }
    let mut state = SimulationState::new(&config).unwrap();
    state.world = world;
    state.players = players;
    for seat in roles.heroes.iter() {
        state.services.runtime[seat.index()].checkpoint = Some(crate::players::Landing {
            level,
            position: state.players.players[seat.index()].motion.position,
        });
    }
    stage_population(&state.world, &mut state.population, level);
    for seat in roles.heroes.iter() {
        state.population.retain(seat, level).unwrap();
    }
    state.loading.initial_setup = true;
    SetupTransaction::prepare(config, state, vec![])
        .unwrap()
        .finish_setup()
        .unwrap()
}

pub(crate) fn stage_population(
    world: &WorldState,
    population: &mut crate::population::PopulationState,
    level: LevelId,
) {
    use crate::population::{
        AuthoredActor, Definition, DefinitionCatalog, Kind, Movement, Thinker, TickRange, Touch,
    };
    let range = TickRange {
        minimum: 1,
        maximum: 1,
    };
    let definition = Definition {
        thinker: Thinker::None,
        movement: Movement::None,
        touch: Touch::Hurt,
        axes: 0,
        width: 24,
        height: 32,
        target_chance: 0,
        target_distance: 0,
        turn_chance: 0,
        companion: false,
        change_axis: false,
        radius: 0,
        contact_x: 0,
        contact_y: 0,
        damage: 0,
        target_time: range,
        turn_time: range,
        move_time: range,
        state_delay: 0,
        health: 1,
    };
    let catalog = DefinitionCatalog::new(
        [definition; 25],
        crate::population::ProjectileVectors {
            thresholds: [0; 8],
            velocities: [[0; 2]; 9],
        },
    )
    .unwrap();
    let actors: Vec<_> = world
        .floor(level)
        .unwrap()
        .generated
        .population
        .earthlings
        .iter()
        .map(|actor| AuthoredActor {
            local: actor.id,
            kind: if actor.role == tje_world::population::PlacementRole::Mailbox {
                Kind::Mailbox
            } else {
                Kind::Devil
            },
            position: Position {
                x: actor.x * 256,
                y: actor.y * 256,
                elevation: 0,
            },
            mail_order: actor.role == tje_world::population::PlacementRole::Mailbox,
        })
        .collect();
    population.stage_floor(level, &actors, &catalog).unwrap();
}

#[test]
fn aggregate_rejects_cross_service_references_and_config_tampering() {
    let initial = session(123);
    initial.validate().unwrap();
    let mut bad = initial.clone();
    bad.players.players[3].level = Some(LevelId::new(31).unwrap());
    assert!(bad.validate().is_err());
    assert_eq!(initial.hero_actor(SeatId::ALL[3]).unwrap().slot(), 3);
    let mut altered = initial.config().clone();
    altered.world.root_seed += 1;
    assert!(altered.validate().is_err());
    let encoded = bincode::encode_to_vec(&bad, bincode::config::standard()).unwrap();
    let (decoded, _): (Session, _) =
        bincode::decode_from_slice(&encoded, bincode::config::standard()).unwrap();
    assert!(decoded.validate().is_err());
    assert!(initial.admit().is_err());
    assert_eq!(initial, session(123));
}

#[test]
fn original_and_expanded_generator_capabilities_cannot_alias() {
    let expanded = session(123);
    assert_eq!(
        expanded.config().generator,
        tje_world::GeneratorId::ExpandedV1
    );
    let original = SessionConfig::stock(
        expanded.world.config().clone(),
        expanded.players.config().clone(),
        expanded.config().revision,
        &[],
    )
    .unwrap();
    assert_eq!(original.generator, tje_world::GeneratorId::OriginalV1);
    assert_ne!(original.identity(), expanded.config().identity());
    assert!(
        SetupTransaction::prepare(
            original,
            SimulationState {
                world: expanded.world.clone(),
                players: expanded.players.clone(),
                services: expanded.services.clone(),
                population: expanded.population.clone(),
                loading: expanded.loading.clone(),
            },
            vec![]
        )
        .is_err()
    );
}

#[test]
fn pending_designed_imports_authenticate_course_and_definition_provenance() {
    use crate::session::{CourseManifest, SourceBinding};
    use tje_world::loading::{ImportKind, ImportSpec};
    let template = session(123);
    let floor = template.world.floor(LevelId::new(2).unwrap()).unwrap();
    let geometry = &floor.generated.geometry;
    let spec = ImportSpec {
        config: template.world.config().clone(),
        level: LevelId::new(2).unwrap(),
        kind: ImportKind::Designed {
            course_identity: [3; 32],
        },
        dimensions: geometry.dimensions(),
        anchors: geometry.anchors().clone(),
        population: floor.generated.population.clone(),
        source_digest: ImportSpec::content_digest(geometry.dimensions(), geometry).unwrap(),
    };
    let provenance = spec.provenance();
    let config = SessionConfig::new(
        template.world.config().clone(),
        template.players.config().clone(),
        template.config().revision,
        tje_world::GeneratorId::DesignedV1,
        Some(CourseManifest {
            identity: [3; 32],
            sources: vec![SourceBinding {
                level: spec.level,
                source_digest: spec.source_digest,
                definitions_digest: provenance.definitions_digest,
            }],
        }),
        &[],
    )
    .unwrap();
    let prepare = |candidate: ImportSpec| {
        let mut state = SimulationState::new(&config).unwrap();
        state.world.request_import(candidate).unwrap();
        SetupTransaction::prepare(config.clone(), state, vec![])
    };
    assert!(prepare(spec.clone()).is_ok());
    let mut wrong_course = spec.clone();
    wrong_course.kind = ImportKind::Designed {
        course_identity: [4; 32],
    };
    assert!(prepare(wrong_course).is_err());
    let mut wrong_definitions = spec;
    wrong_definitions.population.items[0].x += 8;
    assert!(prepare(wrong_definitions).is_err());
}

#[test]
fn hero_input_latching_follows_players_phase_not_boot_relative_codec_frames() {
    let mut state = session(123);
    for (index, phase) in [
        Phase::PresentationA,
        Phase::Population,
        Phase::PresentationB,
        Phase::Players,
    ]
    .into_iter()
    .cycle()
    .take(8)
    .enumerate()
    {
        let input = InputFrame::neutral(500 + index as u64);
        state.clock.admit(phase, input).unwrap();
        if phase == Phase::Players {
            assert!(state.latch_phase_inputs().unwrap());
            assert!(!state.latch_phase_inputs().unwrap());
        } else {
            assert!(state.latch_phase_inputs().is_err());
        }
    }
    assert_eq!(state.players.inputs.unwrap().frame, 507);
}

#[test]
fn input_and_publication_crossrefs_are_validated_together() {
    let mut state = session(123);
    let phase_input = InputFrame::neutral(0);
    let Admission::Step(stamp) = state
        .clock
        .admit(Phase::PresentationA, phase_input)
        .unwrap()
    else {
        panic!("first phase was not admitted");
    };
    state.publish(stamp).unwrap();
    state
        .players
        .latch_inputs(InputFrame::new(0, [0x101, 0, 0, 0], [0; 4]).unwrap())
        .unwrap();
    assert!(state.validate().is_err());
    state.players.inputs = Some(phase_input);
    state.validate().unwrap();
    for (frame, phase) in [
        Phase::Population,
        Phase::PresentationB,
        Phase::Players,
        Phase::PresentationA,
    ]
    .into_iter()
    .enumerate()
    {
        state
            .clock
            .admit(phase, InputFrame::neutral(frame as u64 + 1))
            .unwrap();
    }
    assert!(state.validate().is_err());
}

#[test]
fn roles_never_turn_the_director_into_a_fourth_hero() {
    let roles = Roles {
        inputs: SeatMask::ALL,
        heroes: SeatMask::new(7).unwrap(),
        local_owners: SeatMask::ALL,
        director: Some(SeatId::ALL[3]),
    };
    roles.validate().unwrap();
    let mut state = session_for_roles(123, roles);
    for (frame, phase) in [
        Phase::PresentationA,
        Phase::Population,
        Phase::PresentationB,
        Phase::Players,
    ]
    .into_iter()
    .enumerate()
    {
        state
            .clock
            .admit(
                phase,
                InputFrame::new(frame as u64, [0x100; 4], [1, 2, 4, 0x8000]).unwrap(),
            )
            .unwrap();
    }
    state.validate().unwrap();
    assert!(state.latch_phase_inputs().unwrap());
    let hero = state.players.inputs.unwrap();
    assert!(!hero.pads[3].connected());
    assert_eq!(hero.actions[3], 0);
    assert_eq!(state.clock.input().unwrap().actions[3], 0x8000);
    state.validate().unwrap();
    assert!(state.hero_actor(SeatId::ALL[3]).is_none());
    assert!(
        Roles {
            heroes: SeatMask::ALL,
            ..roles
        }
        .validate()
        .is_err()
    );
    assert!(
        Roles {
            director: None,
            ..roles
        }
        .validate()
        .is_err()
    );
    assert_ne!(roles.inputs, roles.heroes);
}

#[derive(Default)]
struct Recorder {
    calls: Vec<Operation>,
    reject: bool,
    fail: bool,
}
impl PhaseServices for Recorder {
    fn supports(&self, _: Operation) -> bool {
        !self.reject
    }
    fn execute(
        &mut self,
        operation: Operation,
        _: &mut Session,
        _: PublicationStamp,
    ) -> Result<()> {
        if self.fail {
            return Err(Error::State("authored service failure"));
        }
        self.calls.push(operation);
        Ok(())
    }
}

#[test]
fn bounded_bridge_preserves_setup_and_never_replays_a_phase() {
    let mut session = session(123);
    let unchanged = session.clone();
    for root in [
        SetupRoot::Player,
        SetupRoot::MainGame,
        SetupRoot::Inventory,
        SetupRoot::Earthlings,
        SetupRoot::Elevator,
        SetupRoot::Seeds,
    ] {
        assert!(session.acknowledge_setup(root).unwrap());
        assert!(!session.acknowledge_setup(root).unwrap());
    }
    assert_eq!(session.world, unchanged.world);
    assert_eq!(session.players, unchanged.players);
    let mut recorder = Recorder::default();
    for (frame, phase) in [
        Phase::PresentationA,
        Phase::Population,
        Phase::PresentationB,
        Phase::Players,
    ]
    .into_iter()
    .cycle()
    .take(16)
    .enumerate()
    {
        let input =
            InputFrame::new(frame as u64, [0x101, 0x102, 0x104, 0x108], [1, 2, 4, 8]).unwrap();
        assert!(matches!(
            bridge::run_phase(&mut session, &mut recorder, phase, input).unwrap(),
            Admission::Step(_)
        ));
        let calls = recorder.calls.len();
        let checkpoint = session.clone();
        assert_eq!(
            bridge::run_phase(&mut session, &mut recorder, phase, input).unwrap(),
            Admission::Repeated
        );
        assert_eq!(recorder.calls.len(), calls);
        assert_eq!(session, checkpoint);
        session.validate().unwrap();
    }
    assert_eq!(recorder.calls.len(), 96);
}

#[test]
fn missing_operation_rejects_before_mutation_and_execution_errors_latch() {
    let mut session = session(123);
    let before = session.clone();
    let input = InputFrame::neutral(0);
    let mut recorder = Recorder {
        reject: true,
        ..Recorder::default()
    };
    assert!(bridge::run_phase(&mut session, &mut recorder, Phase::PresentationA, input).is_err());
    assert_eq!(session, before);
    recorder.reject = false;
    recorder.fail = true;
    assert!(bridge::run_phase(&mut session, &mut recorder, Phase::PresentationA, input).is_err());
    assert!(session.failed());
    let failed = session.clone();
    recorder.fail = false;
    assert!(
        bridge::run_phase(
            &mut session,
            &mut recorder,
            Phase::Population,
            InputFrame::neutral(1)
        )
        .is_err()
    );
    assert_eq!(session, failed);
}

#[test]
fn aggregate_checks_hero_and_population_pose_bindings_before_restore() {
    use crate::visual::{EarthlingBinding, ProgramContract};
    let template = session(123);
    let mut config = template.config().clone();
    let animation = crate::AnimationId::new(0).unwrap();
    config.catalogue_identity = [29; 32];
    config.frame_count = 2;
    config.animations.programs = vec![ProgramContract {
        selections: 2,
        frame_indices: vec![0, 1],
        flips: [0; 4],
        period: Some(2),
    }];
    config.animations.heroes = std::array::from_fn(|_| vec![animation; 121]);
    config.animations.earthlings = crate::population::Kind::ALL
        .into_iter()
        .map(|kind| EarthlingBinding {
            kind: kind.index() as u8,
            state: 0,
            animation,
        })
        .collect();
    let mut state = SimulationState::new(&config).unwrap();
    state.world = template.world;
    state.players = template.players;
    state.services = template.services;
    let level = LevelId::new(2).unwrap();
    stage_population(&state.world, &mut state.population, level);
    for seat in config.roles.heroes.iter() {
        state.population.retain(seat, level).unwrap();
    }
    state.loading.initial_setup = true;
    let valid = SetupTransaction::prepare(config, state, vec![])
        .unwrap()
        .finish_setup()
        .unwrap();
    valid.validate().unwrap();
    let mut bad = valid.clone();
    bad.players.players[0].animation.sequence_index = 2;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.players.players[0].animation.ticks_left = 3;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.services.runtime[0].animation.descriptor = 1;
    assert!(bad.validate().is_err());
    let id = valid.population.floors()[0].actors[0].id;
    let mut bad = valid.clone();
    bad.population.actor_mut(id).unwrap().state = 3;
    assert!(bad.validate().is_err());
    let mut bad = valid;
    let actor = bad.population.actor_mut(id).unwrap();
    actor.animation.started = true;
    actor.animation.ticks_left = 2;
    actor.animation_frame = 1;
    assert!(bad.validate().is_err());
}

#[test]
fn hero_service_sources_must_have_issued_population_generations() {
    let mut bad = session(123);
    let source = bad.population.floors()[0].actors[0].id;
    bad.services.runtime[0].contacts.offer = Some(crate::contact::OfferedService {
        source,
        actor: crate::ActorId::new(crate::ActorId::POPULATION_START, 1).unwrap(),
        service: crate::population::Service::Heal,
        cost: 1,
        active: false,
    });
    bad.services.validate(&bad.players).unwrap();
    assert!(bad.validate().is_err());
}

#[test]
fn queued_hardware_input_and_lobby_assignment_commit_as_one_transaction() {
    use crate::recording::Assignment;
    use tje_four_player::input::SourceId;
    let mut state = session(123);
    let input = InputFrame::new(0, [0x101, 0x102, 0x104, 0x108], [1, 2, 4, 8]).unwrap();
    let initial = state.clone();
    assert!(state.queue_input(input, None).is_err());
    assert_eq!(state, initial);
    let sources = std::array::from_fn(|index| Some(SourceId([index as u8; 16])));
    let mut bad = Assignment {
        before_frame: 0,
        epoch: 1,
        sources,
    };
    bad.sources[1] = bad.sources[0];
    assert!(state.queue_input(input, Some(&bad)).is_err());
    assert_eq!(state, initial);
    let event = Assignment {
        before_frame: 0,
        epoch: 1,
        sources,
    };
    assert!(state.queue_input(input, Some(&event)).unwrap());
    let committed = state.clone();
    assert!(!state.queue_input(input, None).unwrap());
    assert_eq!(state, committed);
    let unknown = InputFrame::new(1, [0x100; 4], [512, 0, 0, 0]).unwrap();
    assert!(state.queue_input(unknown, None).is_err());
    assert_eq!(state, committed);
    let next = InputFrame::new(1, [0x100; 4], [0; 4]).unwrap();
    let skipped = Assignment {
        before_frame: 1,
        epoch: 3,
        sources,
    };
    assert!(state.queue_input(next, Some(&skipped)).is_err());
    assert_eq!(state, committed);
    let swap = Assignment {
        before_frame: 1,
        epoch: 2,
        sources: [sources[1], sources[0], sources[2], sources[3]],
    };
    assert!(state.queue_input(next, Some(&swap)).unwrap());
    state.validate().unwrap();
    assert_eq!(state.players.router.assignment_epoch(), 2);
    let bytes = bincode::encode_to_vec(&state, tje_world::world::codec()).unwrap();
    let (restored, used): (Session, usize) =
        bincode::decode_from_slice(&bytes, tje_world::world::codec()).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(state, restored);
}

#[test]
fn split_role_assignment_and_raw_frame_are_atomic_and_globally_unique() {
    use crate::recording::Assignment;
    use tje_four_player::input::{Router, SourceId};
    let roles = Roles {
        inputs: SeatMask::ALL,
        heroes: SeatMask::new(7).unwrap(),
        local_owners: SeatMask::ALL,
        director: Some(SeatId::ALL[3]),
    };
    let mut state = session_for_roles(123, roles);
    let mut director = Router::new(SeatMask::new(8).unwrap(), state.config().coop.swap_ac).unwrap();
    let sources = std::array::from_fn(|index| Some(SourceId([index as u8; 16])));
    let raw = InputFrame::new(0, [0x100; 4], [1, 2, 4, 0x8000]).unwrap();
    let original = (state.clone(), director.clone());
    assert!(state.queue_input(raw, None).is_err());
    assert_eq!((state.clone(), director.clone()), original);
    let mut duplicate = Assignment {
        before_frame: 0,
        epoch: 1,
        sources,
    };
    duplicate.sources[3] = sources[0];
    assert!(
        state
            .queue_role_input(raw, Some(&duplicate), &mut director)
            .is_err()
    );
    assert_eq!((state.clone(), director.clone()), original);
    let event = Assignment {
        before_frame: 0,
        epoch: 1,
        sources,
    };
    assert!(
        state
            .queue_role_input(raw, Some(&event), &mut director)
            .unwrap()
    );
    assert_eq!(state.bridge.pending_input, Some(raw));
    assert_eq!(
        state.players.router.sources(),
        &[sources[0], sources[1], sources[2], None]
    );
    assert_eq!(director.sources(), &[None, None, None, sources[3]]);
    assert_eq!(
        state.players.router.assignment_epoch(),
        director.assignment_epoch()
    );
    state.validate().unwrap();
}

fn stock_item_session() -> Session {
    use crate::resources::{ItemDefinitions, StockItem};
    struct Bucks;
    impl ItemDefinitions for Bucks {
        fn food_health_delta(&self, _: u8) -> crate::players::PlayerResult<i8> {
            panic!("authored fixture imports only bucks")
        }
    }
    let mut state = session(123);
    let level = LevelId::new(2).unwrap();
    let source: Vec<_> = state
        .world
        .floor(level)
        .unwrap()
        .generated
        .population
        .items
        .iter()
        .map(|item| StockItem {
            slot: u8::try_from(item.id).unwrap(),
            kind: 80,
            level,
            x: item.x,
            y: item.y,
        })
        .collect();
    state
        .services
        .resources
        .import_stock_items(level, &source, &Bucks)
        .unwrap();
    state.validate().unwrap();
    state
}

#[test]
fn committed_item_claims_reconcile_the_persistent_world_source_without_resurrection() {
    use crate::resources::{
        ItemRef, ResourceAction, ResourceOutcome, ResourceReceipt, ResourceRequest,
    };
    let mut state = stock_item_session();
    let item = *state.services.resources.items().first().unwrap();
    state.players.players[0].motion.position = item.position;
    let mut output = [ResourceOutcome::default()];
    state
        .services
        .resources
        .transact(
            &mut state.players,
            0,
            &[ResourceRequest {
                seat: SeatId::ALL[0],
                action: ResourceAction::Claim(ItemRef::Placed(item.id)),
            }],
            &mut output,
            |_| unreachable!("buck claims do not choose presents"),
        )
        .unwrap();
    assert_eq!(output[0].result, Ok(ResourceReceipt::Claimed));
    assert!(state.validate().is_err());
    state.sync_resource_deltas().unwrap();
    let local = (item.id - u32::from(item.level.get()) * 128 - 1) as u16;
    assert!(
        state
            .world
            .floor(item.level)
            .unwrap()
            .generated
            .population
            .collected
            .contains(local)
    );
    state.validate().unwrap();
    let restored: Session = bincode::decode_from_slice(
        &bincode::encode_to_vec(&state, bincode::config::standard()).unwrap(),
        bincode::config::standard(),
    )
    .unwrap()
    .0;
    restored.validate().unwrap();
    assert_eq!(restored, state);
}

#[test]
fn source_item_preflight_rejects_orphans_and_reverse_collection_before_mutation() {
    use crate::resources::{ItemAppearance, ItemKind, PlacedItem};
    let initial = stock_item_session();
    let level = LevelId::new(2).unwrap();
    let mut orphan = initial.clone();
    orphan
        .services
        .resources
        .add_item(PlacedItem {
            id: u32::from(level.get()) * 128 + 128,
            level,
            position: initial.services.resources.items()[0].position,
            kind: ItemKind::Buck(1),
            appearance: ItemAppearance::Ground(16),
            collected: false,
        })
        .unwrap();
    let before = orphan.clone();
    assert!(orphan.sync_resource_deltas().is_err());
    assert_eq!(orphan, before);
    let mut reversed = initial;
    let local = reversed
        .world
        .floor(level)
        .unwrap()
        .generated
        .population
        .items[0]
        .id;
    reversed
        .world
        .floor_mut(level)
        .unwrap()
        .generated
        .population
        .collected
        .insert(local)
        .unwrap();
    let before = reversed.clone();
    assert!(reversed.sync_resource_deltas().is_err());
    assert_eq!(reversed, before);
}
