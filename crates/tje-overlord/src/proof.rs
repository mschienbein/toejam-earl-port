//! ROM-free authored fixture shared by adversarial tests and native/wasm proof.
//! This is not a cartridge campaign, a faithful receipt, or an admission bypass.
use crate::*;
use crate::{
    bridge::Shared,
    command::{Command, Envelope},
    placement::{Point, Scratch},
    state::Receipt,
};
use bincode::{Decode, Encode};
use sha2::{Digest, Sha256};
use tje_four_player::{
    CoopState,
    state::{Arrival, Config as HeroConfig, Position},
};
use tje_simulation::{
    contact::{ContactBindings, ContactFrame, PlayerContacts},
    players::{Landing, PlayerError, PlayerGeometry, PlayerResult, PlayerServices},
    population::*,
    resources::{PresentCatalog, ResourceLimits},
    session::SessionConfig,
};
use tje_world::{
    Anchors, Cell, CellPosition, Density, Dimensions, GeneratorId, LevelGeometry, LevelSize,
    Resources, Shape, WorldConfig,
    loading::{ImportKind, ImportProgress, ImportSpec},
    population::{Population, PopulationBudget, StableBits},
    world::WorldState,
};

pub struct Solid;
impl Resources for Solid {
    fn parameter(&self, _: LevelId, _: u8) -> tje_world::Result<(u8, u8)> {
        Ok((1, 1))
    }
    fn shape(&self, _: Shape, _: u8, _: bool) -> tje_world::Result<u8> {
        Ok(1)
    }
    fn alternative(&self, _: u8, _: u32) -> tje_world::Result<Cell> {
        Cell::new(1, 0)
    }
    fn transition(&self, _: u8, _: u8, _: u32) -> tje_world::Result<Option<Cell>> {
        Ok(None)
    }
    fn base_tile(&self, base: u8, _: u8, _: u8) -> tje_world::Result<u16> {
        Ok(u16::from(base != 0))
    }
    fn overlay_tile(&self, _: u8, _: u8, _: u8) -> tje_world::Result<u16> {
        Ok(0)
    }
    fn property(&self, tile: u16) -> tje_world::Result<u8> {
        Ok(if tile == 0 { 0x90 } else { 0x10 })
    }
}
pub fn level(value: u16) -> LevelId {
    LevelId::new(value).expect("authored floor")
}
pub fn point(x: i32, y: i32) -> Point {
    Point::new(x, y).expect("authored tile point")
}
pub fn position(x: i32, y: i32) -> Position {
    Position {
        x: x * 256,
        y: y * 256,
        elevation: 0,
    }
}
pub fn hero(seat: usize, floor: u16, x: i32, y: i32) -> HeroView {
    HeroView {
        seat: SeatId::ALL[seat],
        level: level(floor),
        position: position(x, y),
        width: 24,
        velocity: [0; 3],
        targetable: true,
        active_motion: true,
        receives_effects: true,
        swimming: false,
        elevator: false,
        suppressed: false,
        has_inventory: false,
        remapped: false,
        sneaking: false,
        burping: false,
        service_eligible: true,
        response: None,
    }
}
pub fn catalog() -> DefinitionCatalog {
    // Intentionally authored values, not any cartridge definition dump.
    let definition = Definition {
        thinker: Thinker::Generic,
        movement: Movement::AllowedTerrain,
        touch: Touch::Hurt,
        axes: 3,
        width: 24,
        height: 24,
        target_chance: 100,
        target_distance: 100,
        turn_chance: 0,
        companion: false,
        change_axis: false,
        radius: 2,
        contact_x: 8,
        contact_y: 8,
        damage: 1,
        target_time: TickRange {
            minimum: 3,
            maximum: 5,
        },
        turn_time: TickRange {
            minimum: 2,
            maximum: 3,
        },
        move_time: TickRange {
            minimum: 3,
            maximum: 5,
        },
        state_delay: 0,
        health: 3,
    };
    DefinitionCatalog::new(
        [definition; 25],
        ProjectileVectors {
            thresholds: [8, 16, 24, 32, 40, 48, 56, 64],
            velocities: [[256, 256]; 9],
        },
    )
    .expect("authored catalogue")
}
pub fn config(revision: u8) -> Config {
    let world = WorldConfig {
        root_seed: 19,
        rom_sha1: [revision; 20],
        size: LevelSize::Original,
        density: Density::Original,
    };
    let mut heroes = HeroConfig::four_seats(19);
    heroes.participants = HEROES;
    let revision = match revision {
        0 => tje_world::original::OriginalRevision::Rev00,
        2 => tje_world::original::OriginalRevision::Rev02,
        _ => panic!("authored proof requires revision 0 or 2"),
    };
    let mut shared =
        SessionConfig::new(world, heroes, revision, GeneratorId::OriginalV1, None, &[])
            .expect("shared config");
    shared.roles = Config::roles(SeatMask::ALL);
    shared.capacities.earthlings_per_floor = shared.capacities.earthlings_per_floor.max(26);
    Config::new(shared, Preset::Campaign, 91).expect("director config")
}
pub fn geometry() -> LevelGeometry {
    LevelGeometry::new(
        Dimensions::ORIGINAL,
        vec![Cell::new(1, 0).unwrap(); Dimensions::ORIGINAL.area()],
        vec![0; Dimensions::ORIGINAL.area()],
        Anchors {
            arrival: CellPosition { x: 0, y: 0 },
            exit: CellPosition { x: 18, y: 25 },
            ship_piece: None,
            regions: vec![],
        },
    )
    .expect("authored solid floor")
}
#[derive(Clone, PartialEq, Eq, Encode, Decode)]
pub struct Fixture {
    pub state: OverlordState,
    pub world: WorldState,
    pub population: PopulationState,
    pub coop: CoopState,
    pub players: PlayerServices,
}
impl Fixture {
    pub fn new(revision: u8) -> Self {
        let config = config(revision);
        let mut world = WorldState::new(config.shared.world.clone()).unwrap();
        let geometry = geometry();
        for floor in 2..=4 {
            let spec = ImportSpec {
                config: config.shared.world.clone(),
                level: level(floor),
                kind: ImportKind::Stock,
                dimensions: Dimensions::ORIGINAL,
                anchors: geometry.anchors().clone(),
                population: Population {
                    budget: PopulationBudget {
                        items: 0,
                        earthlings: 0,
                        reserved_items: 0,
                        promotion_numerator: 1,
                        promotion_denominator: 1,
                    },
                    items: vec![],
                    earthlings: vec![],
                    collected: StableBits::new(128).unwrap(),
                    removed: StableBits::new(64).unwrap(),
                },
                source_digest: ImportSpec::content_digest(Dimensions::ORIGINAL, &geometry).unwrap(),
            };
            world.request_import(spec).unwrap();
            loop {
                if world.advance_import(&geometry, &Solid, 1024).unwrap() == ImportProgress::Ready {
                    break;
                }
            }
        }
        let catalog = catalog();
        let mut population = PopulationState::new(
            PopulationLimits {
                per_floor: 64,
                active_bodies: 128,
            },
            config.identity(),
        )
        .unwrap();
        for floor in 2..=4 {
            population
                .stage_floor(
                    level(floor),
                    &[
                        (0, Kind::Devil),
                        (1, Kind::Dentist),
                        (2, Kind::DentistVariant),
                        (3, Kind::Lawnmower),
                        (4, Kind::NerdHerd),
                    ]
                    .map(|(local, kind)| AuthoredActor {
                        local,
                        kind,
                        position: position(800 + i32::from(local) * 32, 600),
                        mail_order: false,
                    }),
                    &catalog,
                )
                .unwrap();
        }
        let mut coop = CoopState::new(config.shared.coop.clone()).unwrap();
        coop.record_arrivals(&[0, 1, 2].map(|seat| Arrival {
            seat: SeatId::ALL[seat],
            level: level(2),
            position: position(768, 512),
        }))
        .unwrap();
        for seat in HEROES.iter() {
            world.set_active(seat.get(), level(2)).unwrap();
            population.retain(seat, level(2)).unwrap();
        }
        population
            .refresh_activity(&[
                Some(hero(0, 2, 768, 512)),
                Some(hero(1, 2, 768, 512)),
                Some(hero(2, 2, 768, 512)),
                None,
            ])
            .unwrap();
        Self {
            state: OverlordState::new(config).unwrap(),
            world,
            population,
            coop,
            players: PlayerServices::new(19, ResourceLimits::default()).unwrap(),
        }
    }
    pub fn heroes(&self) -> [Option<HeroView>; 4] {
        SeatId::ALL.map(|seat| {
            if HEROES.contains(seat) {
                self.coop.players[seat.index()].level.map(|floor| {
                    hero(
                        seat.index(),
                        floor.get(),
                        self.coop.players[seat.index()].motion.position.x >> 8,
                        self.coop.players[seat.index()].motion.position.y >> 8,
                    )
                })
            } else {
                None
            }
        })
    }
    pub fn command(&mut self, frame: u64, command: Command) -> Result<()> {
        self.state.enqueue_local(Envelope {
            seat: DIRECTOR,
            frame,
            sequence: self.state.next_sequence(),
            command,
        })?;
        let mut receipts: [Option<Receipt>; MAX_COMMANDS] = [None; MAX_COMMANDS];
        let heroes = self.heroes();
        let catalog = catalog();
        let environment = WorldEnvironment {
            world: &self.world,
            resources: &Solid,
            animation,
            placement_allowed: |_, _, _| Ok(true),
        };
        let mut scratch = Scratch::default();
        let mut shared = Shared {
            world: &self.world,
            resources: &Solid,
            heroes: &heroes,
            population: &mut self.population,
            catalog: &catalog,
            roles: self.state.config().shared.roles,
            environment: &environment,
            scratch: &mut scratch,
        };
        self.state.commands(frame, &mut shared, &mut receipts)?;
        receipts[0]
            .ok_or(Error::Invalid("missing authored command receipt"))?
            .result
    }
    pub fn resolve_spawns(&mut self, scratch: &mut Scratch) -> Result<()> {
        let heroes = self.heroes();
        let catalog = catalog();
        let environment = WorldEnvironment {
            world: &self.world,
            resources: &Solid,
            animation,
            placement_allowed: |_, _, _| Ok(true),
        };
        self.state.resolve_spawns(&mut Shared {
            world: &self.world,
            resources: &Solid,
            heroes: &heroes,
            population: &mut self.population,
            catalog: &catalog,
            roles: self.state.config().shared.roles,
            environment: &environment,
            scratch,
        })
    }
    pub fn population_tick(&mut self, frame: u64, scratch: &mut Scratch) {
        let catalog = catalog();
        let environment = WorldEnvironment {
            world: &self.world,
            resources: &Solid,
            animation,
            placement_allowed: |_, _, _| Ok(true),
        };
        let geometry = HeroGeometry { world: &self.world };
        let identity = *self.state.identity();
        let presents = PresentCatalog::new([1; 27]).unwrap();
        let mut sink = PlayerContacts::new(
            &mut self.coop,
            &mut self.players,
            &geometry,
            ContactFrame {
                tick: frame,
                hero_ids: SeatId::ALL.map(|seat| {
                    HEROES
                        .contains(seat)
                        .then(|| ActorId::new(u16::from(seat.get()), 1).unwrap())
                }),
                elevator: SeatMask::NONE,
                suppressed: SeatMask::NONE,
            },
            ContactBindings {
                identity: &identity,
                presents: &presents,
                ship_parts: &[],
            },
        )
        .unwrap();
        let intent = bridge::intent(&self.state);
        let receipt = self
            .population
            .step_controlled(frame, &environment, &catalog, &[], &mut sink, intent)
            .unwrap();
        if let Some(intent) = intent {
            bridge::acknowledge(&mut self.state, intent.actor, receipt).unwrap();
        }
        self.population
            .advance_motion(frame, &environment, &catalog, &[], &mut sink)
            .unwrap();
        self.population
            .advance_animation(frame, &environment, &catalog, &[], &mut sink)
            .unwrap();
        // This fixture contains only the five contact-only supported kinds:
        // no projectile/service/spawn operation may be silently discarded.
        assert!(sink.finish().is_empty());
        let heroes = self.heroes();
        let roles = self.state.config().shared.roles;
        self.state.reconcile(&Shared {
            world: &self.world,
            resources: &Solid,
            heroes: &heroes,
            population: &mut self.population,
            catalog: &catalog,
            roles,
            environment: &environment,
            scratch,
        });
    }
    pub fn trap_tick(&mut self, frame: u64, scratch: &mut Scratch) {
        let heroes = self.heroes();
        let catalog = catalog();
        let geometry = HeroGeometry { world: &self.world };
        let placement = placement::Context {
            world: &self.world,
            resources: &Solid,
            heroes: &heroes,
            population: &self.population,
            catalog: &catalog,
        };
        self.state
            .resolve_traps(
                frame,
                &mut bridge::TrapAdapter {
                    coop: &mut self.coop,
                    players: &mut self.players,
                    geometry: &geometry,
                    placement,
                    scratch,
                },
            )
            .unwrap();
    }
    pub fn control_body(&self, kind: Kind) -> ActorId {
        self.population
            .floors()
            .iter()
            .find(|floor| floor.level == level(2))
            .unwrap()
            .actors
            .iter()
            .find(|actor| actor.kind == kind)
            .unwrap()
            .body
            .unwrap()
    }
}
pub struct HeroGeometry<'a> {
    pub world: &'a WorldState,
}
impl PlayerGeometry for HeroGeometry<'_> {
    fn dimensions(&self, level: LevelId) -> PlayerResult<Dimensions> {
        self.world
            .floor(level)
            .map(|floor| floor.generated.geometry.dimensions())
            .ok_or(PlayerError::Geometry("authored floor absent"))
    }
    fn terrain(&self, level: LevelId, x: i32, y: i32) -> PlayerResult<u8> {
        self.world
            .floor(level)
            .ok_or(PlayerError::Geometry("authored floor absent"))?
            .generated
            .geometry
            .terrain_at(level, x, y, &Solid)
            .map_err(|_| PlayerError::Geometry("authored terrain"))
    }
    fn blocked(
        &self,
        _: SeatId,
        _: tje_four_player::Archetype,
        level: LevelId,
        position: Position,
    ) -> PlayerResult<bool> {
        Ok(self
            .dimensions(level)?
            .world_cell(position.x >> 8, position.y >> 8)
            .is_none()
            || !matches!(
                self.terrain(level, position.x >> 8, position.y >> 8)?,
                0x10 | 0x20
            ))
    }
    fn respawn(&self, _: SeatId, checkpoint: Landing) -> PlayerResult<Landing> {
        if self.blocked(
            SeatId::ALL[0],
            tje_four_player::Archetype::ToeJam,
            checkpoint.level,
            checkpoint.position,
        )? {
            return Err(PlayerError::Geometry("authored checkpoint unsupported"));
        }
        Ok(checkpoint)
    }
}
pub fn animation(
    _: Kind,
    _: u8,
    cursor: &mut AnimationCursor,
) -> tje_world::Result<AnimationSample> {
    cursor.started = true;
    cursor.sequence_index = (cursor.sequence_index + 1) % 4;
    Ok(AnimationSample {
        frame: cursor.sequence_index,
        motion: Some([1, 0]),
        finished: cursor.sequence_index == 0,
    })
}
/// A platform-independent authoritative fixture hash; not a core/audio receipt.
pub fn scenario(revision: u8) -> [u8; 32] {
    let mut fixture = Fixture::new(revision);
    let mut scratch = Scratch::default();
    let body = fixture.control_body(Kind::Dentist);
    for frame in 0..1800 {
        fixture
            .state
            .video_tick(frame, (500..520).contains(&frame))
            .unwrap();
        match frame {
            0 => fixture.command(frame, Command::Possess(body)).unwrap(),
            20 => fixture
                .command(frame, Command::Move(command::Direction::Left))
                .unwrap(),
            25 => fixture.command(frame, Command::UseAbility).unwrap(),
            40 => fixture.command(frame, Command::Release).unwrap(),
            100 => fixture
                .command(
                    frame,
                    Command::PlaceSpawn {
                        kind: Kind::Devil,
                        level: level(2),
                        point: point(1120, 512),
                    },
                )
                .unwrap(),
            1000 => fixture
                .command(
                    frame,
                    Command::PlaceTrap {
                        kind: traps::TrapKind::Snare,
                        level: level(2),
                        point: point(1024, 768),
                    },
                )
                .unwrap(),
            1300 => fixture
                .command(
                    frame,
                    Command::PlaceTrap {
                        kind: traps::TrapKind::Bump,
                        level: level(2),
                        point: point(768, 768),
                    },
                )
                .unwrap(),
            _ => {}
        }
        fixture.resolve_spawns(&mut scratch).unwrap();
        if !(500..520).contains(&frame) {
            fixture.population_tick(frame, &mut scratch);
            fixture.trap_tick(frame, &mut scratch);
        }
        // Every authoritative phase-boundary state is codec checked. Restore
        // midway through telegraphs/cooldowns/pause, with no host iterator state.
        if [25, 121, 510, 1021, 1350].contains(&frame) {
            let bytes = bincode::encode_to_vec(&fixture, bincode::config::standard()).unwrap();
            let (restored, used): (Fixture, _) =
                bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
            assert_eq!(used, bytes.len());
            fixture = restored;
        }
    }
    fixture.state.validate().unwrap();
    fixture.population.validate().unwrap();
    Sha256::digest(bincode::encode_to_vec(&fixture, bincode::config::standard()).unwrap()).into()
}
