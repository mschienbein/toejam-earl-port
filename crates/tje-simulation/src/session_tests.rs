use crate::bridge::{self, Operation, PhaseServices};
use crate::session::{Roles, Session, SessionConfig, SetupRoot, SetupTransaction};
use crate::{Admission, Error, LevelId, Phase, PublicationStamp, Result, SeatMask};
use tje_four_player::{
    CoopState, SeatId,
    input::InputFrame,
    state::{Arrival, Config, Position},
};
use tje_world::{Cell, Density, LevelSize, Resources, Shape, WorldConfig, world::WorldState};

// Authored solid/void terrain only: no ROM map or recovered lookup table.
struct Solid;
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
    use tje_world::generation::Phase as Loading;
    let world_config = WorldConfig {
        root_seed: seed,
        rom_sha1: [0; 20],
        size: LevelSize::Original,
        density: Density::Original,
    };
    let coop_config = Config::four_seats(seed);
    let config = SessionConfig::stock(world_config.clone(), coop_config.clone(), &[]).unwrap();
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
    let arrivals = std::array::from_fn::<_, 4, _>(|index| Arrival {
        seat: SeatId::ALL[index],
        level,
        position: Position {
            x: x * 256,
            y: y * 256,
            elevation: 0,
        },
    });
    players.record_arrivals(&arrivals).unwrap();
    for seat in SeatId::ALL {
        world.set_active(seat.get(), level).unwrap();
    }
    SetupTransaction::prepare(config, world, players, vec![])
        .unwrap()
        .finish_setup()
        .unwrap()
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
fn roles_never_turn_the_director_into_a_fourth_hero() {
    let roles = Roles {
        inputs: SeatMask::ALL,
        heroes: SeatMask::new(7).unwrap(),
        local_owners: SeatMask::ALL,
        director: Some(SeatId::ALL[3]),
    };
    roles.validate().unwrap();
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
