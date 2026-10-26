use crate::*;
use crate::{
    command::{Command, Envelope},
    config::*,
    placement::{Context, Rejection, Scratch},
    proof::*,
    state::{MatchResult, SpawnStage},
    traps::{Hero, Sink, Stage, Trap, TrapKind},
    view::*,
};
use tje_four_player::{
    CoopState,
    input::InputFrame,
    state::{LifeState, Overlay},
};

fn roundtrip<T>(value: &T) -> T
where
    T: bincode::Encode + bincode::Decode<()>,
{
    let bytes = bincode::encode_to_vec(value, bincode::config::standard()).unwrap();
    let (decoded, consumed) =
        bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
    assert_eq!(consumed, bytes.len());
    decoded
}
#[test]
fn role_config_keeps_four_inputs_three_bodies_and_separate_owners() {
    let config = config(0);
    assert_eq!(config.shared.roles.inputs, SeatMask::ALL);
    assert_eq!(config.shared.roles.heroes, HEROES);
    assert_eq!(config.shared.roles.director, Some(DIRECTOR));
    let coop = CoopState::new(config.shared.coop.clone()).unwrap();
    assert_eq!(coop.players[3].life, LifeState::Disabled);
    assert!(coop.players[3].inventory.iter().all(Option::is_none));
    let mut bad = config.clone();
    bad.shared.roles.heroes = SeatMask::ALL;
    assert!(bad.validate().is_err());
    let mut remote = config.clone();
    remote.shared.roles.local_owners = HEROES;
    remote.validate().unwrap();
    assert_ne!(remote.identity(), config.identity());
    let mut state = OverlordState::new(remote).unwrap();
    let before = state.clone();
    let command = Envelope {
        seat: DIRECTOR,
        frame: 0,
        sequence: 0,
        command: Command::CycleTool,
    };
    assert_eq!(state.enqueue_local(command), Err(Error::Role));
    assert_eq!(state, before);
    assert_eq!(request_gameplay(), Err(Error::AdmissionClosed));
    assert_ne!(config.identity(), proof::config(2).identity());
}
#[test]
fn command_queue_is_finite_ordered_owned_and_duplicate_safe() {
    let mut state = OverlordState::new(config(0)).unwrap();
    let envelope = Envelope {
        seat: DIRECTOR,
        frame: 12,
        sequence: 0,
        command: Command::CycleTool,
    };
    assert_eq!(state.enqueue_local(envelope), Ok(true));
    assert_eq!(state.enqueue_local(envelope), Ok(false));
    let before = state.clone();
    assert_eq!(
        state.enqueue_local(Envelope {
            command: Command::CancelPlacement,
            ..envelope
        }),
        Err(Error::Order)
    );
    assert_eq!(
        state.enqueue_local(Envelope {
            seat: SeatId::ALL[2],
            ..envelope
        }),
        Err(Error::Role)
    );
    assert_eq!(
        state.enqueue_local(Envelope {
            sequence: 2,
            ..envelope
        }),
        Err(Error::Order)
    );
    assert_eq!(state, before);
    for sequence in 1..MAX_COMMANDS as u32 {
        state
            .enqueue_local(Envelope {
                sequence,
                ..envelope
            })
            .unwrap();
    }
    assert_eq!(
        state.enqueue_local(Envelope {
            sequence: MAX_COMMANDS as u32,
            ..envelope
        }),
        Err(Error::Full)
    );
    assert_eq!(state, roundtrip(&state));
    state.validate().unwrap();
}
#[test]
fn video_timers_are_integer_paused_and_exactly_once() {
    let mut state = OverlordState::new(config(0)).unwrap();
    for frame in 0..59 {
        state.video_tick(frame, false).unwrap();
    }
    assert_eq!((state.energy, state.energy_remainder), (40, 59));
    let saved = state.clone();
    assert_eq!(state.video_tick(58, false), Ok(false));
    assert_eq!(state, saved);
    assert_eq!(state.video_tick(58, true), Err(Error::Order));
    assert_eq!(state, saved);
    for frame in 59..119 {
        state.video_tick(frame, true).unwrap();
    }
    assert_eq!(
        (state.energy, state.energy_remainder, state.elapsed),
        (40, 59, 59)
    );
    state.video_tick(119, false).unwrap();
    assert_eq!((state.energy, state.energy_remainder), (41, 0));
    assert_eq!(state.video_tick(121, false), Err(Error::Order));
    state.validate().unwrap();
}
#[test]
fn role_input_routes_director_edges_without_inventing_a_fourth_hero() {
    let mut state = OverlordState::new(config(0)).unwrap();
    state.video_tick(10, false).unwrap();
    let input = InputFrame::new(10, [0x101, 0x102, 0x104, 0x190], [0; 4]).unwrap();
    let heroes = state.route_local_input(input, None).unwrap().unwrap();
    assert_eq!(heroes.pads[..3], input.pads[..3]);
    assert!(!heroes.pads[3].connected());
    assert_eq!(heroes.actions[3], 0);
    assert_eq!(state.route_local_input(input, None).unwrap(), None);
    assert!(
        state
            .pending()
            .iter()
            .flatten()
            .any(|event| event.command == Command::Pause(true))
    );
    let before = state.clone();
    let bad = InputFrame::new(10, [0x101, 0x102, 0x104, 0x180], [0, 0, 0, 1]).unwrap();
    assert!(state.route_local_input(bad, None).is_err());
    assert_eq!(state, before);
}
#[test]
fn possession_cost_stale_generation_protected_state_and_safe_release() {
    for kind in [
        Kind::Devil,
        Kind::Dentist,
        Kind::DentistVariant,
        Kind::Lawnmower,
        Kind::NerdHerd,
    ] {
        let mut fixture = Fixture::new(0);
        fixture.state.video_tick(0, false).unwrap();
        let body = fixture.control_body(kind);
        let id = fixture.population.resolve(body).unwrap().id;
        fixture.population.actor_mut(id).unwrap().set_state(99);
        let energy = fixture.state.energy;
        assert_eq!(
            fixture.command(0, Command::Possess(body)),
            Err(Error::Protected)
        );
        assert_eq!(fixture.state.energy, energy);
        assert!(fixture.state.control.is_none());
        fixture.population.actor_mut(id).unwrap().set_state(0);
        fixture.state.video_tick(1, false).unwrap();
        fixture.command(1, Command::Possess(body)).unwrap();
        assert_eq!(fixture.state.energy, energy - POSSESSION_COST);
        fixture.state.video_tick(2, false).unwrap();
        fixture.command(2, Command::Release).unwrap();
        assert_eq!(fixture.population.resolve(body).unwrap().timer, 0);
        fixture.state.video_tick(3, false).unwrap();
        assert_eq!(
            fixture.command(3, Command::Possess(body)),
            Err(Error::Cooldown)
        );
        fixture.population.remove(id, false).unwrap();
        fixture.state.video_tick(4, false).unwrap();
        assert_eq!(
            fixture.command(4, Command::Possess(body)),
            Err(Error::StaleActor)
        );
    }
}
#[test]
fn unsupported_spawn_and_bad_coordinates_do_not_spend_or_allocate() {
    let mut fixture = Fixture::new(0);
    fixture.state.video_tick(0, false).unwrap();
    let population = fixture.population.clone();
    let energy = fixture.state.energy;
    let rng = fixture.state.rng;
    assert_eq!(
        fixture.command(
            0,
            Command::PlaceSpawn {
                kind: Kind::ShipPiece,
                level: level(2),
                point: point(1120, 512)
            }
        ),
        Err(Error::UnsupportedKind)
    );
    assert_eq!(fixture.state.energy, energy);
    assert_eq!(fixture.state.rng, rng);
    assert_eq!(fixture.population, population);
    let before = fixture.state.clone();
    assert_eq!(
        fixture.state.enqueue_local(Envelope {
            seat: DIRECTOR,
            frame: 1,
            sequence: fixture.state.next_sequence(),
            command: Command::PlaceSpawn {
                kind: Kind::Devil,
                level: level(2),
                point: placement::Point { x: i32::MAX, y: -1 }
            }
        }),
        Err(Error::Placement(Rejection::Coordinates))
    );
    assert_eq!(fixture.state, before);
}
#[test]
fn placement_uses_exact_footprints_hero_interest_and_objective_reservations() {
    let fixture = Fixture::new(0);
    let heroes = fixture.heroes();
    let catalog = catalog();
    let mut scratch = Scratch::default();
    let context = Context {
        world: &fixture.world,
        resources: &Solid,
        heroes: &heroes,
        population: &fixture.population,
        catalog: &catalog,
    };
    context
        .validate(level(2), point(1120, 512), Some(Kind::Devil), &mut scratch)
        .unwrap();
    assert_eq!(
        context.validate(level(3), point(1120, 512), Some(Kind::Devil), &mut scratch),
        Err(Error::Placement(Rejection::DormantFloor))
    );
    assert_eq!(
        context.validate(level(2), point(768, 512), Some(Kind::Devil), &mut scratch),
        Err(Error::Placement(Rejection::Visible))
    );
    assert_eq!(
        context.validate(level(2), point(192, 96), None, &mut scratch),
        Err(Error::Placement(Rejection::SafeZone))
    );
    assert_eq!(
        context.validate(level(2), point(2000, 512), Some(Kind::Devil), &mut scratch),
        Err(Error::Placement(Rejection::TooFar))
    );
    let geometry_before = fixture.world.clone();
    let population_before = fixture.population.clone();
    for _ in 0..4 {
        context
            .validate(level(2), point(1120, 512), Some(Kind::Devil), &mut scratch)
            .unwrap();
    }
    assert_eq!(fixture.world, geometry_before);
    assert_eq!(fixture.population, population_before);
}
#[test]
fn reserved_spawn_telegraphs_then_revalidates_and_refunds_only_once() {
    let mut fixture = Fixture::new(0);
    fixture.state.video_tick(0, false).unwrap();
    fixture
        .command(
            0,
            Command::PlaceSpawn {
                kind: Kind::Devil,
                level: level(2),
                point: point(1120, 512),
            },
        )
        .unwrap();
    let spawn = fixture.state.spawns[0].unwrap();
    assert_eq!(fixture.state.energy, 20);
    let pending = fixture.population.actor(spawn.population).unwrap();
    assert!(pending.hidden);
    assert!(!pending.collidable);
    assert!(pending.body.is_some());
    assert_eq!(spawn.stage, SpawnStage::Telegraph);
    for frame in 1..60 {
        fixture.state.video_tick(frame, false).unwrap();
    }
    let mut scratch = Scratch::default();
    fixture.resolve_spawns(&mut scratch).unwrap();
    assert_eq!(
        fixture.state.spawns[0].unwrap().stage,
        SpawnStage::Telegraph
    );
    fixture.coop.players[0].motion.position = position(1120, 512);
    fixture.state.video_tick(60, false).unwrap();
    fixture.resolve_spawns(&mut scratch).unwrap();
    let refund = fixture.state.energy;
    assert_eq!(fixture.state.spawns[0].unwrap().stage, SpawnStage::Expired);
    assert!(fixture.state.spawns[0].unwrap().refunded);
    assert!(fixture.population.actor(spawn.population).unwrap().retired);
    fixture.resolve_spawns(&mut scratch).unwrap();
    assert_eq!(fixture.state.energy, refund);
    assert_eq!(fixture.state, roundtrip(&fixture.state));
}
#[test]
fn activated_spawn_death_is_not_a_refund() {
    let mut fixture = Fixture::new(0);
    fixture.state.video_tick(0, false).unwrap();
    fixture
        .command(
            0,
            Command::PlaceSpawn {
                kind: Kind::Devil,
                level: level(2),
                point: point(1120, 512),
            },
        )
        .unwrap();
    for frame in 1..=60 {
        fixture.state.video_tick(frame, false).unwrap();
    }
    let mut scratch = Scratch::default();
    fixture.resolve_spawns(&mut scratch).unwrap();
    let spawn = fixture.state.spawns[0].unwrap();
    assert_eq!(spawn.stage, SpawnStage::Active);
    assert!(!fixture.population.actor(spawn.population).unwrap().hidden);
    let energy = fixture.state.energy;
    fixture.population.remove(spawn.population, false).unwrap();
    fixture.resolve_spawns(&mut scratch).unwrap();
    assert_eq!(fixture.state.energy, energy);
    assert!(!fixture.state.spawns[0].unwrap().refunded);
}

struct TrapWorld {
    heroes: [Option<Hero>; 4],
    supported: bool,
    bumps: Vec<(SeatId, i32, i32)>,
}
impl TrapWorld {
    fn empty() -> Self {
        Self {
            heroes: [None; 4],
            supported: true,
            bumps: vec![],
        }
    }
    fn set(&mut self, seat: usize, x: i32, y: i32, eligible: bool) {
        self.heroes[seat] = Some(Hero {
            seat: SeatId::ALL[seat],
            level: level(2),
            x,
            y,
            eligible,
        });
    }
}
impl Sink for TrapWorld {
    fn hero(&self, seat: SeatId) -> Option<Hero> {
        self.heroes[seat.index()]
    }
    fn may_arm(&mut self, _: &Trap) -> Result<bool> {
        Ok(true)
    }
    fn bump(&mut self, seat: SeatId, _: LevelId, x: i32, y: i32) -> Result<bool> {
        if self.supported {
            self.bumps.push((seat, x, y));
            self.heroes[seat.index()].as_mut().unwrap().x = x;
            self.heroes[seat.index()].as_mut().unwrap().y = y;
        }
        Ok(self.supported)
    }
}
#[test]
fn traps_do_not_arm_under_any_hero_and_nonstacking_trigger_is_seat_ordered() {
    let mut trap = Trap::new(1, TrapKind::Snare, level(2), point(768, 512));
    trap.arm_left = 0;
    let mut traps = [None; MAX_TRAPS];
    traps[0] = Some(trap);
    let mut snares = [0; 3];
    let mut sink = TrapWorld::empty();
    sink.set(0, 768, 512, false);
    traps::resolve(&mut traps, &mut snares, &mut sink).unwrap();
    assert_eq!(traps[0].unwrap().stage, Stage::Telegraph);
    sink.heroes[0] = None;
    traps::resolve(&mut traps, &mut snares, &mut sink).unwrap();
    assert_eq!(traps[0].unwrap().stage, Stage::Armed);
    sink.set(0, 768, 512, true);
    sink.set(1, 768, 512, true);
    snares[0] = 50;
    traps::resolve(&mut traps, &mut snares, &mut sink).unwrap();
    assert_eq!(snares, [50, SNARE_TICKS, 0]);
    assert_eq!(traps[0].unwrap().triggered.bits(), 2);
    traps::resolve(&mut traps, &mut snares, &mut sink).unwrap();
    assert_eq!(snares, [50, SNARE_TICKS, 0]);
    assert_eq!(traps, roundtrip(&traps));
}
#[test]
fn bump_is_twelve_pixels_supported_only_and_never_a_damage_payload() {
    let mut traps = [None; MAX_TRAPS];
    let mut trap = Trap::new(1, TrapKind::Bump, level(2), point(768, 512));
    trap.stage = Stage::Armed;
    trap.arm_left = 0;
    traps[0] = Some(trap);
    let mut sink = TrapWorld::empty();
    sink.set(0, 776, 512, true);
    sink.supported = false;
    traps::resolve(&mut traps, &mut [0; 3], &mut sink).unwrap();
    assert_eq!(traps[0].unwrap().stage, Stage::Armed);
    assert!(sink.bumps.is_empty());
    sink.supported = true;
    traps::resolve(&mut traps, &mut [0; 3], &mut sink).unwrap();
    assert_eq!(sink.bumps, [(SeatId::ALL[0], 788, 512)]);
    assert_eq!(traps[0].unwrap().stage, Stage::Triggered);
}
#[test]
fn trap_lifetime_cooldowns_and_three_per_floor_cap_are_exact() {
    let mut fixture = Fixture::new(0);
    fixture.state.video_tick(0, false).unwrap();
    fixture
        .command(
            0,
            Command::PlaceTrap {
                kind: TrapKind::Snare,
                level: level(2),
                point: point(768, 512),
            },
        )
        .unwrap();
    assert_eq!(fixture.state.traps[0].unwrap().arm_left, 60);
    for frame in 1..600 {
        fixture.state.video_tick(frame, false).unwrap();
    }
    assert!(fixture.state.traps[0].unwrap().active());
    fixture.state.video_tick(600, false).unwrap();
    assert_eq!(fixture.state.traps[0].unwrap().stage, Stage::Expired);
    for (offset, point) in [
        point(768, 512),
        point(1024, 512),
        point(768, 768),
        point(1024, 768),
    ]
    .into_iter()
    .enumerate()
    {
        let frame = 601 + offset as u64;
        fixture.state.video_tick(frame, false).unwrap();
        fixture.state.energy = 100;
        fixture.state.trap_cooldown = 0;
        let result = fixture.command(
            frame,
            Command::PlaceTrap {
                kind: TrapKind::Snare,
                level: level(2),
                point,
            },
        );
        if offset == 3 {
            assert_eq!(result, Err(Error::Full));
            assert_eq!(fixture.state.energy, 100);
        } else {
            result.unwrap();
        }
    }
    assert_eq!(
        fixture
            .state
            .traps
            .iter()
            .flatten()
            .filter(|trap| trap.active())
            .count(),
        3
    );
    fixture.state.validate().unwrap();
}
#[test]
fn validated_exit_wins_same_phase_terminal_death_and_time_policy() {
    let mut fixture = Fixture::new(0);
    for seat in HEROES.iter() {
        fixture.coop.players[seat.index()].life = LifeState::Eliminated;
        fixture.coop.players[seat.index()].level = None;
    }
    assert_eq!(
        fixture
            .state
            .arbitrate(&fixture.coop, 0x3ff, Some((SeatId::ALL[0], level(26))))
            .unwrap(),
        MatchResult::Heroes
    );
    assert_eq!(
        fixture.state.arbitrate(&fixture.coop, 0, None).unwrap(),
        MatchResult::Heroes
    );
    let mut state = OverlordState::new(config(0)).unwrap();
    assert_eq!(
        state.arbitrate(&fixture.coop, 0, None).unwrap(),
        MatchResult::DirectorExhaustion
    );
    let live = Fixture::new(0);
    let mut state = OverlordState::new(config(0)).unwrap();
    state.elapsed = u64::MAX - 1;
    assert_eq!(
        state.arbitrate(&live.coop, 0, None).unwrap(),
        MatchResult::Running
    ); // full campaign has no hidden timer
    assert!(
        state
            .arbitrate(&live.coop, 0, Some((DIRECTOR, level(26))))
            .is_err()
    );
}
#[test]
fn tactical_packets_redact_hero_ui_and_cannot_activate_dormant_floors() {
    let mut fixture = Fixture::new(0);
    let world = fixture.world.clone();
    let population = fixture.population.clone();
    let rng = fixture.state.rng;
    fixture.state.cursor_level = level(4);
    let packet = publish(
        &fixture.state,
        Authorization {
            owners: SeatMask::ALL,
            levels: &[level(2), level(4)],
            heroes: &[],
        },
    )
    .unwrap();
    assert!(packet.heroes.iter().all(Option::is_none));
    let mut pixels = vec![0; WIDTH * HEIGHT];
    draw_tactical(
        &packet,
        level(4),
        &fixture.world.floor(level(4)).unwrap().generated.geometry,
        &Solid,
        &mut pixels,
    )
    .unwrap();
    fixture.coop.players[0].health = 1;
    fixture.coop.players[0].ui.overlay = Overlay::Inventory;
    let after = publish(
        &fixture.state,
        Authorization {
            owners: SeatMask::ALL,
            levels: &[level(2), level(4)],
            heroes: &[],
        },
    )
    .unwrap();
    assert_eq!(packet, after);
    assert_eq!(fixture.world, world);
    assert_eq!(fixture.population, population);
    assert_eq!(fixture.state.rng, rng);
    let neutral = publish(
        &fixture.state,
        Authorization {
            owners: SeatMask::ALL,
            levels: &[level(2)],
            heroes: &[],
        },
    )
    .unwrap();
    assert_eq!(neutral.camera, Camera::Unavailable);
    assert_eq!(
        publish(
            &fixture.state,
            Authorization {
                owners: HEROES,
                levels: &[level(2)],
                heroes: &[]
            }
        ),
        Err(Error::Role)
    );
}
#[test]
fn complete_director_codec_fits_budget_and_restores_pending_commands_and_timers() {
    let mut fixture = Fixture::new(2);
    fixture.state.video_tick(0, false).unwrap();
    fixture
        .command(
            0,
            Command::Possess(fixture.control_body(Kind::DentistVariant)),
        )
        .unwrap();
    fixture
        .state
        .enqueue_local(Envelope {
            seat: DIRECTOR,
            sequence: fixture.state.next_sequence(),
            frame: 1,
            command: Command::UseAbility,
        })
        .unwrap();
    let mut bytes = [0; MAX_STATE_BYTES];
    let used = bincode::encode_into_slice(&fixture.state, &mut bytes, bincode::config::standard())
        .unwrap();
    assert!(used < MAX_STATE_BYTES);
    let mut restored: Fixture = roundtrip(&fixture);
    assert!(fixture == restored);
    fixture.state.video_tick(1, false).unwrap();
    restored.state.video_tick(1, false).unwrap();
    // Replay exactly the queued command through the real shared adapter.
    let mut a = [None; MAX_COMMANDS];
    let mut b = [None; MAX_COMMANDS];
    let catalog = catalog();
    let heroes = fixture.heroes();
    let mut scratch = Scratch::default();
    let environment = tje_simulation::population::WorldEnvironment {
        world: &fixture.world,
        resources: &Solid,
        animation,
        placement_allowed: |_, _, _| Ok(true),
    };
    let mut shared = bridge::Shared {
        world: &fixture.world,
        resources: &Solid,
        heroes: &heroes,
        population: &mut fixture.population,
        catalog: &catalog,
        roles: fixture.state.config().shared.roles,
        environment: &environment,
        scratch: &mut scratch,
    };
    fixture.state.commands(1, &mut shared, &mut a).unwrap();
    let environment = tje_simulation::population::WorldEnvironment {
        world: &restored.world,
        resources: &Solid,
        animation,
        placement_allowed: |_, _, _| Ok(true),
    };
    let mut shared = bridge::Shared {
        world: &restored.world,
        resources: &Solid,
        heroes: &heroes,
        population: &mut restored.population,
        catalog: &catalog,
        roles: restored.state.config().shared.roles,
        environment: &environment,
        scratch: &mut scratch,
    };
    restored.state.commands(1, &mut shared, &mut b).unwrap();
    assert_eq!(a, b);
    assert!(fixture == restored);
}
#[test]
fn authored_native_wasm_proof_scenarios_are_deterministic() {
    assert_eq!(scenario(0), scenario(0));
    assert_eq!(scenario(2), scenario(2));
    assert_ne!(scenario(0), scenario(2));
}

#[test]
fn director_binding_is_stable_and_cannot_share_a_hero_source() {
    use tje_four_player::input::{PadWord, SourceId, SourceSample};
    let mut fixture = Fixture::new(0);
    let director = SourceId([9; 16]);
    let hero_source = SourceId([4; 16]);
    fixture
        .state
        .director_router
        .reassign(&[(DIRECTOR, Some(director))])
        .unwrap();
    fixture
        .coop
        .router
        .reassign(&[(SeatId::ALL[0], Some(hero_source))])
        .unwrap();
    fixture.state.video_tick(0, false).unwrap();
    let samples = [
        SourceSample {
            source: director,
            pad: PadWord::new(0x180).unwrap(),
            actions: 0,
        },
        SourceSample {
            source: hero_source,
            pad: PadWord::new(0x101).unwrap(),
            actions: 0,
        },
    ];
    let heroes = fixture
        .state
        .route_local_samples(&fixture.coop, 0, &samples, None)
        .unwrap()
        .unwrap();
    assert_eq!(heroes.pads[0].bits(), 0x101);
    assert!(!heroes.pads[3].connected());
    let router = fixture.state.director_router.clone();
    fixture.state.video_tick(1, false).unwrap();
    fixture
        .state
        .route_local_samples(&fixture.coop, 1, &samples[1..], None)
        .unwrap();
    assert_eq!(fixture.state.director_router, router);
    fixture
        .coop
        .router
        .reassign(&[(SeatId::ALL[0], Some(director))])
        .unwrap();
    fixture.state.video_tick(2, false).unwrap();
    let before = fixture.state.clone();
    assert!(
        fixture
            .state
            .route_local_samples(&fixture.coop, 2, &samples, None)
            .is_err()
    );
    assert_eq!(fixture.state, before);
}

#[test]
fn shared_setup_allocates_exactly_three_hero_actor_identities() {
    let config = config(0).shared;
    let state = tje_simulation::session::SimulationState::new(&config).unwrap();
    let session = tje_simulation::session::SetupTransaction::prepare(config, state, vec![])
        .unwrap()
        .into_loading()
        .unwrap();
    for seat in HEROES.iter() {
        assert!(session.hero_actor(seat).is_some());
    }
    assert_eq!(session.hero_actor(DIRECTOR), None);
    assert!(session.admit().is_err());
}

#[test]
fn short_preset_declares_three_floor_objectives_and_exact_time_tie_policy() {
    use tje_simulation::session::{CourseManifest, SourceBinding};
    let mut shared = config(0).shared;
    shared.generator = tje_world::GeneratorId::DesignedV1;
    shared.course = Some(CourseManifest {
        identity: [7; 32],
        sources: [2, 3, 4]
            .map(|floor| SourceBinding {
                level: level(floor),
                source_digest: [floor as u8; 32],
                definitions_digest: [17; 32],
            })
            .to_vec(),
    });
    let config = Config::new(
        shared,
        Preset::ShortCourse {
            required: 3,
            exit: level(4),
        },
        91,
    )
    .unwrap();
    let coop = CoopState::new(config.shared.coop.clone()).unwrap();
    let mut state = OverlordState::new(config.clone()).unwrap();
    state.elapsed = 15 * 60 * 60;
    assert_eq!(
        state
            .arbitrate(&coop, 3, Some((SeatId::ALL[2], level(4))))
            .unwrap(),
        MatchResult::Heroes
    );
    let mut state = OverlordState::new(config).unwrap();
    state.elapsed = 15 * 60 * 60;
    assert_eq!(
        state.arbitrate(&coop, 0, None).unwrap(),
        MatchResult::DirectorTime
    );
}

#[test]
fn trap_global_cap_counts_telegraphs_on_three_disjoint_hero_floors() {
    use tje_four_player::state::Arrival;
    let mut fixture = Fixture::new(0);
    for index in 0..3 {
        let floor = level(2 + index as u16);
        let seat = SeatId::ALL[index];
        fixture
            .coop
            .record_arrivals(&[Arrival {
                seat,
                level: floor,
                position: position(768, 512),
            }])
            .unwrap();
        fixture.world.set_active(seat.get(), floor).unwrap();
        fixture.population.retain(seat, floor).unwrap();
    }
    for index in 0..9 {
        fixture.state.video_tick(index, false).unwrap();
        fixture.state.energy = 100;
        fixture.state.trap_cooldown = 0;
        let result = fixture.command(
            index,
            Command::PlaceTrap {
                kind: TrapKind::Snare,
                level: level(2 + (index / 3) as u16),
                point: point(768 + (index % 3) as i32 * 256, 512),
            },
        );
        if index == 8 {
            assert_eq!(result, Err(Error::Full));
            assert_eq!(fixture.state.energy, 100);
        } else {
            result.unwrap();
        }
    }
    assert_eq!(
        fixture
            .state
            .traps
            .iter()
            .flatten()
            .filter(|trap| trap.active())
            .count(),
        8
    );
    fixture.state.validate().unwrap();
}

#[test]
fn shared_snare_scales_voluntary_motion_only_and_bump_uses_real_player_transaction() {
    use tje_simulation::players::{PlayerMode, VoluntaryScale};
    let mut fixture = Fixture::new(0);
    fixture.state.snares[0] = SNARE_TICKS;
    assert_eq!(
        bridge::voluntary_scales(&fixture.state),
        [
            VoluntaryScale::Snared,
            VoluntaryScale::Normal,
            VoluntaryScale::Normal,
            VoluntaryScale::Normal
        ]
    );
    let geometry = HeroGeometry {
        world: &fixture.world,
    };
    let mut normal = fixture.coop.clone();
    let mut normal_services = fixture.players.clone();
    for seat in HEROES.iter() {
        fixture.coop.players[seat.index()].motion.velocity[0] = 256;
        normal.players[seat.index()].motion.velocity[0] = 256;
        fixture.players.runtime[seat.index()]
            .effective_input
            .directions = 8;
        normal_services.runtime[seat.index()]
            .effective_input
            .directions = 8;
    }
    let before = fixture.coop.players[0].motion.position.x;
    fixture
        .players
        .advance_movement(
            &mut fixture.coop,
            0,
            bridge::voluntary_scales(&fixture.state),
            &geometry,
        )
        .unwrap();
    normal_services
        .advance_movement(&mut normal, 0, [VoluntaryScale::Normal; 4], &geometry)
        .unwrap();
    let snared = fixture.coop.players[0].motion.position.x - before;
    let ordinary = normal.players[0].motion.position.x - before;
    assert_eq!(snared * 4, ordinary * 3);
    assert_eq!(
        fixture.coop.players[0].motion.velocity,
        normal.players[0].motion.velocity
    );
    assert_eq!(fixture.coop.players[1].motion, normal.players[1].motion);
    let destination = tje_simulation::players::Landing {
        level: level(2),
        position: position(780, 512),
    };
    fixture.players.runtime[0].mode = PlayerMode::Flying;
    let before = fixture.coop.clone();
    assert!(
        !fixture
            .players
            .try_ground_bump(&mut fixture.coop, SeatId::ALL[0], destination, &geometry)
            .unwrap()
    );
    assert_eq!(fixture.coop, before);
    fixture.players.runtime[0].mode = PlayerMode::Grounded;
    assert!(
        fixture
            .players
            .try_ground_bump(&mut fixture.coop, SeatId::ALL[0], destination, &geometry)
            .unwrap()
    );
    assert_eq!(
        fixture.coop.players[0].motion.position,
        destination.position
    );
}
