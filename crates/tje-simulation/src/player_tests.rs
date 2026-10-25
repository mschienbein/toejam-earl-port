use crate::{ActorId, players::*, resources::*};
use tje_four_player::{
    Archetype, Config, CoopState, SeatId, SeatMask,
    state::{LifeState, Overlay, Position},
};
use tje_world::{Dimensions, LevelId};

#[derive(Debug, Clone)]
struct Actor {
    level: LevelId,
    position: Position,
    health: u8,
    sleeping: Option<crate::EffectId>,
}
#[derive(Debug, Clone, Default)]
struct World {
    actors: Vec<Actor>,
    wall: Option<(i32, i32, i32, i32)>,
    water: Option<(i32, i32)>,
    doors: Vec<(SeatId, LevelId, Position)>,
    phones: Vec<(SeatId, LevelId, Position)>,
    revealed: u8,
    burps: u16,
    rejected: bool,
    service_collisions: Vec<(ActorId, bool)>,
}
impl PlayerGeometry for World {
    fn dimensions(&self, _: LevelId) -> PlayerResult<Dimensions> {
        Ok(Dimensions::EXPANDED)
    }
    fn terrain(&self, level: LevelId, x: i32, y: i32) -> PlayerResult<u8> {
        if x < 0 || y < 0 {
            return Ok(0x90);
        }
        if self
            .water
            .is_some_and(|(left, right)| x >= left && x < right)
        {
            return Ok(0x80);
        }
        Ok(if level.get() == 4 { 0x30 } else { 0x10 })
    }
    fn blocked(
        &self,
        _: SeatId,
        archetype: Archetype,
        _: LevelId,
        position: Position,
    ) -> PlayerResult<bool> {
        let (x, y) = (position.x >> 8, position.y >> 8);
        let radius = i32::from(archetype.dimensions().0) / 2;
        Ok(self.wall.is_some_and(|(left, top, right, bottom)| {
            x + radius >= left && x - radius <= right && y >= top && y <= bottom
        }))
    }
    fn respawn(&self, _: SeatId, checkpoint: Landing) -> PlayerResult<Landing> {
        Ok(checkpoint)
    }
}
impl PlayerWorld for World {
    fn commit_present(&mut self, present: WorldPresent) -> PlayerResult<()> {
        if self.rejected {
            return Err(PlayerError::Rejected("fixture world has no reservation"));
        }
        match present {
            WorldPresent::SpawnEarthling {
                level,
                mut position,
                choice,
                ..
            } => {
                position.x += if choice & 1 == 0 { 40 * 256 } else { -40 * 256 };
                position.y += 25 * 256;
                self.actors.push(Actor {
                    level,
                    position,
                    health: 3,
                    sleeping: None,
                });
            }
            WorldPresent::Togetherness { destination, .. } => {
                self.dimensions(destination.level)?;
            }
            WorldPresent::OpenDoorway {
                owner,
                level,
                position,
            } => self.doors.push((owner, level, position)),
            WorldPresent::OpenTelephone {
                owner,
                level,
                position,
            } => self.phones.push((owner, level, position)),
        }
        Ok(())
    }
    fn apply_actor_effect(&mut self, effect: ActorEffect) -> PlayerResult<bool> {
        let mut hit = false;
        match effect {
            ActorEffect::Projectile {
                level,
                position,
                damage,
                ..
            } => {
                if let Some(actor) = self.actors.iter_mut().find(|actor| {
                    actor.health != 0
                        && actor.level == level
                        && (actor.position.x - position.x).abs() < 24 * 256
                        && (actor.position.y - position.y).abs() < 24 * 256
                }) {
                    actor.health = actor.health.saturating_sub(damage);
                    hit = true;
                }
            }
            ActorEffect::Roses {
                level, position, ..
            } => {
                for actor in &mut self.actors {
                    if actor.level == level
                        && (actor.position.x - position.x).abs() < 24 * 256
                        && (actor.position.y - position.y).abs() < 24 * 256
                    {
                        actor.health = actor.health.saturating_sub(1);
                        hit = true;
                    }
                }
            }
            ActorEffect::Boombox {
                effect,
                level,
                position,
            } => {
                for actor in &mut self.actors {
                    if actor.level == level
                        && (actor.position.x - position.x).abs() < 320 * 256
                        && (actor.position.y - position.y).abs() < 256 * 256
                    {
                        actor.sleeping = Some(effect);
                        hit = true;
                    }
                }
            }
            ActorEffect::WakeBoombox { effect, level } => {
                for actor in &mut self.actors {
                    if actor.level == level && actor.sleeping == Some(effect) {
                        actor.sleeping = None;
                        hit = true;
                    }
                }
            }
            ActorEffect::Burp { .. } => {
                self.burps += 1;
                hit = true;
            }
            ActorEffect::ServiceCollision { actor, collidable } => {
                if let Some((_, value)) = self
                    .service_collisions
                    .iter_mut()
                    .find(|(existing, _)| *existing == actor)
                {
                    *value = collidable;
                } else {
                    self.service_collisions.push((actor, collidable));
                }
                hit = true;
            }
        }
        Ok(hit)
    }
    fn doorway_crossing(
        &mut self,
        _: crate::EffectId,
        owner: SeatId,
        _: SeatId,
        level: LevelId,
        _: Position,
    ) -> PlayerResult<Option<Landing>> {
        if !self
            .doors
            .iter()
            .any(|&(seat, floor, _)| seat == owner && floor == level)
        {
            return Err(PlayerError::Invalid("unregistered doorway"));
        }
        Ok(Some(Landing {
            level,
            position: point(900, 600),
        }))
    }
    fn answer_telephone(
        &mut self,
        _: crate::EffectId,
        hero: SeatId,
        level: LevelId,
    ) -> PlayerResult<()> {
        if !self.phones.iter().any(|&(_, floor, _)| floor == level) {
            return Err(PlayerError::Invalid("unregistered telephone"));
        }
        self.revealed |= 1 << hero.get();
        Ok(())
    }
    fn control_remap(&self, selection: u8) -> PlayerResult<[u8; 4]> {
        // Newly authored permutations; production resolves its user's catalogue.
        Ok(match selection {
            0 => [2, 1, 8, 4],
            1 => [4, 8, 2, 1],
            2 => [8, 4, 1, 2],
            3 => [1, 2, 8, 4],
            _ => return Err(PlayerError::Invalid("remap selection")),
        })
    }
}
fn point(x: i32, y: i32) -> Position {
    Position {
        x: x * 256,
        y: y * 256,
        elevation: 0,
    }
}
fn level(value: u16) -> LevelId {
    LevelId::new(value).unwrap()
}
fn fixture(archetype: Archetype, seat: SeatId) -> (CoopState, PlayerServices, World) {
    let mut config = Config::four_seats(123);
    config.roster[seat.index()].archetype = archetype;
    let mut coop = CoopState::new(config).unwrap();
    let mut services = PlayerServices::new(123, ResourceLimits::default()).unwrap();
    let world = World::default();
    for hero in SeatId::ALL {
        services
            .setup_seat(
                &mut coop,
                hero,
                Landing {
                    level: level(2),
                    position: point(400 + i32::from(hero.get()) * 80, 400),
                },
                2,
                &world,
            )
            .unwrap();
    }
    (coop, services, world)
}
fn equip(coop: &mut CoopState, services: &PlayerServices, seat: SeatId, kind: PresentKind) {
    coop.players[seat.index()].inventory[0] = Some(services.resources.presents.wrapper_for(kind));
}
fn input_for(seat: SeatId, input: PlayerInput) -> [PlayerInput; 4] {
    let mut inputs = [PlayerInput::default(); 4];
    inputs[seat.index()] = input;
    inputs
}

#[test]
fn every_present_has_real_semantics_in_every_seat_and_archetype() {
    for archetype in [Archetype::ToeJam, Archetype::Earl] {
        for seat in SeatId::ALL {
            for kind in PresentKind::ALL {
                let (mut coop, mut services, mut world) = fixture(archetype, seat);
                let index = seat.index();
                coop.players[index].health = 12;
                coop.players[index].bucks = 10;
                let before = coop.players[index].clone();
                equip(&mut coop, &services, seat, kind);
                let receipt = services
                    .open_present(&mut coop, seat, 0, &mut world)
                    .unwrap_or_else(|error| panic!("{archetype:?} {seat:?} {kind:?}: {error}"));
                assert_eq!(receipt.requested, kind);
                assert!(coop.players[index].inventory[0].is_none());
                assert_eq!(coop.players[index].points, before.points + 2);
                let applied = receipt.applied;
                match applied {
                    PresentKind::IcarusWings | PresentKind::SpringShoes => {
                        services
                            .tick_players(
                                &mut coop,
                                0,
                                input_for(
                                    seat,
                                    PlayerInput {
                                        jump: true,
                                        ..PlayerInput::default()
                                    },
                                ),
                                &mut world,
                            )
                            .unwrap();
                        assert!(coop.players[index].motion.position.elevation > 0);
                    }
                    PresentKind::Innertube => {
                        world.water = Some((300, 800));
                        services
                            .tick_players(
                                &mut coop,
                                0,
                                input_for(
                                    seat,
                                    PlayerInput {
                                        directions: 8,
                                        ..PlayerInput::default()
                                    },
                                ),
                                &mut world,
                            )
                            .unwrap();
                        assert_eq!(services.runtime[index].mode, PlayerMode::Swimming);
                        assert_eq!(coop.players[index].health, 12);
                    }
                    PresentKind::Tomatoes | PresentKind::Slingshot | PresentKind::RoseBushes => {
                        world.actors.push(Actor {
                            level: level(2),
                            position: before.motion.position,
                            health: 3,
                            sleeping: None,
                        });
                        services
                            .tick_players(
                                &mut coop,
                                0,
                                input_for(
                                    seat,
                                    PlayerInput {
                                        action: true,
                                        ..PlayerInput::default()
                                    },
                                ),
                                &mut world,
                            )
                            .unwrap();
                        assert_eq!(services.resources.effects().count(), 1);
                        services.tick_effects(&mut coop, 0, &mut world).unwrap();
                        assert_eq!(world.actors[0].health, 2, "{seat:?} {applied:?}");
                    }
                    PresentKind::RocketSkates
                    | PresentKind::SuperHiTops
                    | PresentKind::BonusHiTops => {
                        services
                            .tick_players(
                                &mut coop,
                                0,
                                input_for(
                                    seat,
                                    PlayerInput {
                                        directions: 8,
                                        action: true,
                                        ..PlayerInput::default()
                                    },
                                ),
                                &mut world,
                            )
                            .unwrap();
                        assert!(
                            coop.players[index].motion.position.x - before.motion.position.x >= 768
                        );
                    }
                    PresentKind::Doorway => {
                        services.tick_effects(&mut coop, 0, &mut world).unwrap();
                        assert_eq!(coop.players[index].motion.position, point(900, 600));
                        assert_eq!(coop.players[index].level, Some(level(2)));
                    }
                    PresentKind::Food => assert!(matches!(coop.players[index].health, 4 | 20)),
                    PresentKind::RootBeer => {
                        assert!((12..=18).contains(&services.runtime[index].burps_left));
                        assert_eq!(coop.players[index].health, 16);
                        services.tick_effects(&mut coop, 0, &mut world).unwrap();
                        assert_eq!(world.burps, 1);
                    }
                    PresentKind::Promotion => assert_eq!(coop.players[index].rank, 1),
                    PresentKind::Togetherness => {
                        assert!(coop.players[index].motion.position.elevation > 0)
                    }
                    PresentKind::RainCloud => assert_eq!(
                        services
                            .resources
                            .effect(receipt.effect.unwrap())
                            .unwrap()
                            .kind,
                        EffectKind::RainCloud
                    ),
                    PresentKind::FudgeSundae => {
                        assert_eq!(coop.players[index].health, 24.min(archetype.base_health()))
                    }
                    PresentKind::Decoy => {
                        let id = receipt.effect.unwrap();
                        assert!(services.resources.hit_decoy(id));
                        assert!(services.resources.hit_decoy(id));
                        assert!(services.resources.effect(id).is_some());
                        assert!(services.resources.hit_decoy(id));
                        assert!(services.resources.effect(id).is_none());
                    }
                    PresentKind::TotalBummer => {
                        assert_eq!(coop.players[index].health, 0);
                        services
                            .tick_players(&mut coop, 0, [PlayerInput::default(); 4], &mut world)
                            .unwrap();
                        assert_eq!(services.runtime[index].mode, PlayerMode::Dying);
                    }
                    PresentKind::ExtraLife => assert_eq!(coop.players[index].lives, 3),
                    PresentKind::Randomizer => assert_eq!(services.resources.presents.epoch, 2),
                    PresentKind::Telephone => {
                        services
                            .tick_players(
                                &mut coop,
                                0,
                                input_for(
                                    seat,
                                    PlayerInput {
                                        action: true,
                                        ..PlayerInput::default()
                                    },
                                ),
                                &mut world,
                            )
                            .unwrap();
                        services.tick_effects(&mut coop, 0, &mut world).unwrap();
                        assert_eq!(world.revealed, 1 << seat.get());
                    }
                    PresentKind::ExtraBuck => assert_eq!(coop.players[index].bucks, 11),
                    PresentKind::Jackpot => assert_eq!(coop.players[index].bucks, 15),
                    PresentKind::TomatoRain => {
                        let rain = services.runtime[index].rain_left;
                        assert!((50..=80).contains(&rain));
                        services.tick_effects(&mut coop, 0, &mut world).unwrap();
                        assert_eq!(services.runtime[index].rain_left, rain - 1);
                        assert_eq!(services.resources.effects().count(), 1);
                    }
                    PresentKind::Earthling => assert_eq!(world.actors.len(), 1),
                    PresentKind::SchoolBook => {
                        for tick in 0..60 {
                            services
                                .tick_players(
                                    &mut coop,
                                    tick,
                                    [PlayerInput::default(); 4],
                                    &mut world,
                                )
                                .unwrap();
                        }
                        assert_eq!(services.runtime[index].mode, PlayerMode::Sleeping);
                    }
                    PresentKind::Boombox => {
                        world.actors.push(Actor {
                            level: level(2),
                            position: before.motion.position,
                            health: 3,
                            sleeping: None,
                        });
                        services.tick_effects(&mut coop, 0, &mut world).unwrap();
                        assert_eq!(world.actors[0].sleeping, receipt.effect);
                        for tick in 1..300 {
                            services.tick_effects(&mut coop, tick, &mut world).unwrap();
                        }
                        assert!(world.actors[0].sleeping.is_none());
                    }
                    PresentKind::Mystery => panic!("mystery must resolve to a real effect"),
                }
                services
                    .validate(&coop)
                    .unwrap_or_else(|error| panic!("{archetype:?} {seat:?} {kind:?}: {error}"));
            }
        }
    }
}

#[test]
fn terrain_collision_hurt_death_respawn_and_overlays_are_seat_owned() {
    for archetype in [Archetype::ToeJam, Archetype::Earl] {
        for seat in SeatId::ALL {
            let (mut coop, mut services, mut world) = fixture(archetype, seat);
            let index = seat.index();
            let original = coop.players[index].motion.position;
            world.wall = Some((
                (original.x >> 8) + i32::from(archetype.dimensions().0) / 2 + 1,
                350,
                2000,
                450,
            ));
            services
                .tick_players(
                    &mut coop,
                    0,
                    input_for(
                        seat,
                        PlayerInput {
                            directions: 8,
                            ..PlayerInput::default()
                        },
                    ),
                    &mut world,
                )
                .unwrap();
            assert_eq!(coop.players[index].motion.position.x, original.x);
            assert_eq!(services.runtime[index].blocked_axes & 1, 1);
            world.wall = None;
            services
                .ui(&mut coop, seat, UiIntent::OpenInventory)
                .unwrap();
            let other = coop.players[(index + 1) % 4].clone();
            assert!(
                services
                    .damage(
                        &mut coop,
                        seat,
                        DamageRequest {
                            source: DamageSource::Terrain,
                            amount: 8,
                            style: DamageStyle::Hurt,
                            origin: point(0, 0)
                        },
                        &world
                    )
                    .unwrap()
            );
            for tick in 1..=12 {
                services
                    .tick_players(&mut coop, tick, [PlayerInput::default(); 4], &mut world)
                    .unwrap();
            }
            assert_eq!(coop.players[index].health, archetype.base_health() - 8);
            assert_eq!(coop.players[(index + 1) % 4].health, other.health);
            for tick in 13..=42 {
                services
                    .tick_players(&mut coop, tick, [PlayerInput::default(); 4], &mut world)
                    .unwrap();
            }
            assert!(
                services
                    .damage(
                        &mut coop,
                        seat,
                        DamageRequest {
                            source: DamageSource::Terrain,
                            amount: 127,
                            style: DamageStyle::Squish,
                            origin: point(0, 0)
                        },
                        &world
                    )
                    .unwrap()
            );
            for tick in 43..=103 {
                services
                    .tick_players(&mut coop, tick, [PlayerInput::default(); 4], &mut world)
                    .unwrap();
            }
            assert_eq!(coop.players[index].health, archetype.base_health());
            assert_eq!(coop.players[index].lives, 1);
            assert_eq!(coop.players[index].motion.position, original);
            assert!(services.runtime[index].invulnerable_ticks > 0);
            services.validate(&coop).unwrap();
        }
    }
}

#[test]
fn four_independent_terrain_caches_and_fall_cohort_commit_atomically() {
    let (mut coop, mut services, mut world) = fixture(Archetype::Earl, SeatId::ALL[3]);
    let cohort = FallCohort {
        token: 13,
        seats: SeatMask::ALL,
    };
    services.begin_fall(&mut coop, cohort).unwrap();
    for tick in 0..40 {
        services
            .tick_players(&mut coop, tick, [PlayerInput::default(); 4], &mut world)
            .unwrap();
    }
    assert!(services.fall_ready(13));
    let mut destinations = std::array::from_fn(|index| {
        Some(Landing {
            level: level(index as u16 + 2),
            position: point(400, 400),
        })
    });
    let before = (coop.clone(), services.clone());
    destinations[3] = None;
    assert!(
        services
            .finish_fall(&mut coop, 13, destinations, &world)
            .is_err()
    );
    assert_eq!((coop.clone(), services.clone()), before);
    destinations[3] = Some(Landing {
        level: level(5),
        position: point(400, 400),
    });
    services
        .finish_fall(&mut coop, 13, destinations, &world)
        .unwrap();
    for seat in SeatId::ALL {
        assert_eq!(
            coop.players[seat.index()].level,
            Some(level(seat.get() as u16 + 2))
        );
    }
    assert_eq!(coop.players[2].terrain[2][2], 0x30);
    assert_eq!(coop.players[3].terrain[2][2], 0x10);
    assert!(services.fall_cohorts.iter().all(Option::is_none));
    services.validate(&coop).unwrap();
}

#[test]
fn claims_ship_parts_purchases_drops_and_identification_conserve_resources() {
    let (mut coop, mut services, world) = fixture(Archetype::ToeJam, SeatId::ALL[0]);
    for hero in &mut coop.players {
        hero.motion.position = point(400, 400);
        hero.bucks = 10;
    }
    let present = services
        .resources
        .presents
        .wrapper_for(PresentKind::ExtraLife);
    for id in 1..=80 {
        services
            .resources
            .add_item(PlacedItem {
                id,
                level: level(2),
                position: point(400, 400),
                kind: ItemKind::Present(present),
                collected: false,
            })
            .unwrap();
    }
    services
        .resources
        .add_item(PlacedItem {
            id: 81,
            level: level(2),
            position: point(400, 400),
            kind: ItemKind::ShipPart(9),
            collected: false,
        })
        .unwrap();
    let mut output = [ResourceOutcome::default(); 4];
    let mut requests = SeatId::ALL.map(|seat| ResourceRequest {
        seat,
        action: ResourceAction::Claim(ItemRef::Placed(64)),
    });
    services
        .resources
        .transact(&mut coop, 0, &requests, &mut output)
        .unwrap();
    assert_eq!(
        output
            .iter()
            .filter(|outcome| outcome.result.is_ok())
            .count(),
        1
    );
    assert!(coop.players[0].inventory[0].is_some());
    requests = SeatId::ALL.map(|seat| ResourceRequest {
        seat,
        action: ResourceAction::Claim(ItemRef::Placed(65)),
    });
    services
        .resources
        .transact(&mut coop, 1, &requests, &mut output)
        .unwrap();
    assert!(coop.players[1].inventory[0].is_some());
    requests = SeatId::ALL.map(|seat| ResourceRequest {
        seat,
        action: ResourceAction::Claim(ItemRef::Placed(81)),
    });
    services
        .resources
        .transact(&mut coop, 2, &requests, &mut output)
        .unwrap();
    assert_eq!(services.resources.ship_parts, 1 << 9);
    assert_eq!(
        output
            .iter()
            .filter(|outcome| outcome.result == Ok(ResourceReceipt::ShipPart(9)))
            .count(),
        1
    );
    let mail = ActorId::new(32, 1).unwrap();
    let identify = ActorId::new(33, 1).unwrap();
    services
        .resources
        .add_service(TradeService {
            actor: mail,
            kind: ServiceKind::MailOrder,
            level: level(2),
            position: point(400, 400),
        })
        .unwrap();
    services
        .resources
        .add_service(TradeService {
            actor: identify,
            kind: ServiceKind::Identify,
            level: level(2),
            position: point(400, 400),
        })
        .unwrap();
    services
        .resources
        .add_offer(MailOffer {
            id: 12,
            service: mail,
            present,
            price: 3,
            stock: 1,
        })
        .unwrap();
    requests = SeatId::ALL.map(|seat| ResourceRequest {
        seat,
        action: ResourceAction::Purchase {
            offer: 12,
            service: mail,
        },
    });
    services
        .resources
        .transact(&mut coop, 3, &requests, &mut output)
        .unwrap();
    assert_eq!(
        coop.players
            .iter()
            .map(|hero| u16::from(hero.bucks))
            .sum::<u16>(),
        37
    );
    assert_eq!(coop.players[3].bucks, 7);
    let count = coop
        .players
        .iter()
        .map(|hero| hero.inventory.iter().flatten().count())
        .sum::<usize>();
    let drop_request = [ResourceRequest {
        seat: SeatId::ALL[3],
        action: ResourceAction::Drop { slot: 0 },
    }];
    services
        .resources
        .transact(&mut coop, 4, &drop_request, &mut output)
        .unwrap();
    let drop = match output[0].result.unwrap() {
        ResourceReceipt::Dropped(id) => id,
        _ => unreachable!(),
    };
    assert_eq!(services.resources.drops().count(), 1);
    let claim_request = [ResourceRequest {
        seat: SeatId::ALL[2],
        action: ResourceAction::Claim(ItemRef::Dropped(drop)),
    }];
    services
        .resources
        .transact(&mut coop, 5, &claim_request, &mut output)
        .unwrap();
    assert!(output[0].result.is_ok());
    assert_eq!(services.resources.drops().count(), 0);
    assert_eq!(
        coop.players
            .iter()
            .map(|hero| hero.inventory.iter().flatten().count())
            .sum::<usize>(),
        count
    );
    let identify_request = [ResourceRequest {
        seat: SeatId::ALL[2],
        action: ResourceAction::Identify {
            slot: 0,
            service: identify,
        },
    }];
    services
        .resources
        .transact(&mut coop, 6, &identify_request, &mut output)
        .unwrap();
    assert_eq!(coop.players[2].bucks, 8);
    services
        .resources
        .transact(&mut coop, 7, &identify_request, &mut output)
        .unwrap();
    assert_eq!(output[0].result, Err(ResourceFailure::AlreadyIdentified));
    assert_eq!(coop.players[2].bucks, 8);
    let before = (coop.clone(), services.clone());
    assert!(
        services
            .resources
            .transact(&mut coop, 7, &identify_request, &mut output)
            .is_err()
    );
    assert_eq!((coop.clone(), services.clone()), before);
    services.validate(&coop).unwrap();
    assert_eq!(world.actors.len(), 0);
}

#[test]
fn effect_generation_quotas_and_failed_present_are_transactional() {
    let (mut coop, mut services, mut world) = fixture(Archetype::ToeJam, SeatId::ALL[0]);
    let seat = SeatId::ALL[0];
    let effect = Effect::stationary(
        seat,
        PresentKind::Decoy,
        EffectKind::Decoy,
        level(2),
        point(400, 400),
        10,
    );
    let stale = services.resources.spawn_effect(effect).unwrap();
    assert!(services.resources.release_effect(stale));
    assert!(!services.resources.release_effect(stale));
    let fresh = services.resources.spawn_effect(effect).unwrap();
    assert_ne!(stale, fresh);
    assert!(services.resources.effect(stale).is_none());
    for _ in 1..16 {
        services.resources.spawn_effect(effect).unwrap();
    }
    equip(&mut coop, &services, seat, PresentKind::RainCloud);
    let before = (coop.clone(), services.clone());
    assert!(
        services
            .open_present(&mut coop, seat, 0, &mut world)
            .is_err()
    );
    assert_eq!((coop.clone(), services.clone()), before);
    equip(&mut coop, &services, SeatId::ALL[3], PresentKind::Earthling);
    world.rejected = true;
    let before = (coop.clone(), services.clone());
    assert!(
        services
            .open_present(&mut coop, SeatId::ALL[3], 0, &mut world)
            .is_err()
    );
    assert_eq!((coop.clone(), services.clone()), before);
    services.validate(&coop).unwrap();
}

#[test]
fn identified_donations_are_conservative_and_cannot_chain() {
    let (mut coop, mut services, world) = fixture(Archetype::Earl, SeatId::ALL[3]);
    for hero in &mut coop.players {
        hero.motion.position = point(400, 400);
    }
    coop.players[0].health = 20;
    coop.players[3].health = 4;
    let total = coop
        .players
        .iter()
        .map(|hero| u16::from(hero.health))
        .sum::<u16>();
    services
        .donate_health(&mut coop, 10, SeatId::ALL[0], SeatId::ALL[3], 7)
        .unwrap();
    assert_eq!(coop.players[0].health, 13);
    assert_eq!(coop.players[3].health, 11);
    assert_eq!(
        coop.players
            .iter()
            .map(|hero| u16::from(hero.health))
            .sum::<u16>(),
        total
    );
    assert!(
        services
            .donate_health(&mut coop, 10, SeatId::ALL[3], SeatId::ALL[2], 1)
            .is_err()
    );
    coop.players[2].life = LifeState::Eliminated;
    coop.players[2].level = None;
    coop.players[2].health = 0;
    coop.players[2].lives = 0;
    let lives = coop.players[1].lives;
    services
        .donate_life(&mut coop, 11, SeatId::ALL[1], SeatId::ALL[2], &world)
        .unwrap();
    assert_eq!(coop.players[1].lives, lives - 1);
    assert_eq!(coop.players[2].life, LifeState::Alive);
    assert_eq!(coop.players[2].lives, 0);
    assert!(
        services
            .donate_life(&mut coop, 11, SeatId::ALL[2], SeatId::ALL[3], &world)
            .is_err()
    );
    services.validate(&coop).unwrap();
}

#[test]
fn overlays_intents_and_pause_do_not_alias_or_grant_immunity() {
    let (mut coop, mut services, mut world) = fixture(Archetype::ToeJam, SeatId::ALL[0]);
    for seat in SeatId::ALL {
        services
            .ui(&mut coop, seat, UiIntent::OpenInventory)
            .unwrap();
        services
            .ui(&mut coop, seat, UiIntent::Select(seat.get() + 4))
            .unwrap();
        services.ui(&mut coop, seat, UiIntent::Activate).unwrap();
        assert_eq!(
            services.take_ui_command(seat),
            Some(UiCommand::OpenPresent(seat.get() + 4))
        );
        assert!(services.ui(&mut coop, seat, UiIntent::Activate).is_err());
        services.release_ui_action(&mut coop, seat);
        services.ui(&mut coop, seat, UiIntent::Pause(true)).unwrap();
    }
    assert!(services.pause_agreed(&coop));
    services
        .ui(&mut coop, SeatId::ALL[2], UiIntent::Pause(false))
        .unwrap();
    assert!(!services.pause_agreed(&coop));
    let before = std::array::from_fn::<_, 4, _>(|index| coop.players[index].motion.position);
    services
        .tick_players(
            &mut coop,
            0,
            [PlayerInput {
                directions: 8,
                ..PlayerInput::default()
            }; 4],
            &mut world,
        )
        .unwrap();
    for seat in SeatId::ALL {
        assert_eq!(
            coop.players[seat.index()].motion.position,
            before[seat.index()]
        );
        assert_eq!(coop.players[seat.index()].ui.overlay, Overlay::Inventory);
    }
}

#[test]
fn player_effect_status_snapshot_roundtrip_and_duplicate_phase_do_not_advance() {
    let (mut coop, mut services, mut world) = fixture(Archetype::Earl, SeatId::ALL[3]);
    equip(&mut coop, &services, SeatId::ALL[3], PresentKind::RainCloud);
    services
        .open_present(&mut coop, SeatId::ALL[3], 0, &mut world)
        .unwrap();
    services
        .status(
            &mut coop,
            SeatId::ALL[2],
            StatusRequest::Remap {
                ticks: 100,
                directions: [8, 4, 1, 2],
            },
        )
        .unwrap();
    services
        .begin_fall(
            &mut coop,
            FallCohort {
                token: 123,
                seats: SeatMask::new(1).unwrap(),
            },
        )
        .unwrap();
    services
        .tick_players(&mut coop, 0, [PlayerInput::default(); 4], &mut world)
        .unwrap();
    services.tick_effects(&mut coop, 0, &mut world).unwrap();
    let codec = bincode::config::standard()
        .with_fixed_int_encoding()
        .with_limit::<{ 4 * 1024 * 1024 }>();
    let bytes = bincode::encode_to_vec((&coop, &services), codec).unwrap();
    let ((decoded_coop, decoded_services), used): ((CoopState, PlayerServices), usize) =
        bincode::decode_from_slice(&bytes, codec).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!((&decoded_coop, &decoded_services), (&coop, &services));
    decoded_services.validate(&decoded_coop).unwrap();
    assert_eq!(
        bincode::encode_to_vec((&decoded_coop, &decoded_services), codec).unwrap(),
        bytes
    );
    let before = (coop.clone(), services.clone());
    assert!(
        services
            .tick_players(&mut coop, 0, [PlayerInput::default(); 4], &mut world)
            .is_err()
    );
    assert!(services.tick_effects(&mut coop, 0, &mut world).is_err());
    assert_eq!((coop.clone(), services.clone()), before);
    assert!(bytes.len() < 16 * 1024);
}

#[test]
fn automatic_services_complete_for_both_archetypes_in_every_seat() {
    for archetype in [Archetype::ToeJam, Archetype::Earl] {
        for seat in SeatId::ALL {
            let (mut coop, mut services, mut world) = fixture(archetype, seat);
            let index = seat.index();
            let actor = ActorId::new(32, 1).unwrap();
            let center = point(1000, 700);
            coop.players[index].motion.position = point(960, 696);
            coop.players[index].health = 1;
            services.runtime[index].previous_input.directions = 8;
            assert!(
                services
                    .service_contact(&mut coop, seat, actor, HeroServiceKind::HotTub, center)
                    .unwrap()
            );
            for tick in 0..40 {
                finish_authored_animation(&mut services, &mut coop, seat);
                services
                    .tick_players(&mut coop, tick, [PlayerInput::default(); 4], &mut world)
                    .unwrap();
            }
            assert_eq!(
                services.runtime[index].service.unwrap().stage,
                ServiceStage::Bathing
            );
            assert!(coop.players[index].health > 1);
            assert_eq!(world.service_collisions, [(actor, true)]);
            services
                .tick_players(
                    &mut coop,
                    40,
                    input_for(
                        seat,
                        PlayerInput {
                            directions: 4,
                            ..PlayerInput::default()
                        },
                    ),
                    &mut world,
                )
                .unwrap();
            finish_authored_animation(&mut services, &mut coop, seat);
            services
                .tick_players(&mut coop, 41, [PlayerInput::default(); 4], &mut world)
                .unwrap();
            assert!(services.runtime[index].service.is_none());
            assert!(coop.players[index].motion.position.x < center.x);
            assert_eq!(services.runtime[index].mode, PlayerMode::Jumping);
            coop.players[index].motion.position = point(1000, 710);
            services.runtime[index].previous_input.directions = 1;
            let lives = coop.players[index].lives;
            assert!(
                services
                    .service_contact(&mut coop, seat, actor, HeroServiceKind::Lemonade, center)
                    .unwrap()
            );
            finish_authored_animation(&mut services, &mut coop, seat);
            services
                .tick_players(&mut coop, 42, [PlayerInput::default(); 4], &mut world)
                .unwrap();
            assert_eq!(coop.players[index].lives, lives + 1);
            assert_eq!(world.burps, 1);
            services.runtime[index].previous_input.directions = 1;
            assert!(
                services
                    .service_contact(&mut coop, seat, actor, HeroServiceKind::Lemonade, center)
                    .unwrap()
            );
            finish_authored_animation(&mut services, &mut coop, seat);
            services
                .tick_players(&mut coop, 43, [PlayerInput::default(); 4], &mut world)
                .unwrap();
            assert_eq!(coop.players[index].lives, lives + 1);
            assert_eq!(world.burps, 2);
            services.validate(&coop).unwrap();
        }
    }
}

#[test]
fn randomizer_is_a_saved_sixty_step_shared_resource_operation() {
    let (mut coop, mut services, mut world) = fixture(Archetype::ToeJam, SeatId::ALL[0]);
    for seat in SeatId::ALL {
        for slot in 0..16 {
            coop.players[seat.index()].inventory[slot] = Some(
                services
                    .resources
                    .presents
                    .wrapper_for(PresentKind::ExtraBuck),
            );
        }
    }
    equip(
        &mut coop,
        &services,
        SeatId::ALL[0],
        PresentKind::Randomizer,
    );
    services
        .open_present(&mut coop, SeatId::ALL[0], 0, &mut world)
        .unwrap();
    let count = coop
        .players
        .iter()
        .map(|hero| hero.inventory.iter().flatten().count())
        .sum::<usize>();
    let before = coop.players[3].inventory;
    for tick in 0..30 {
        services.tick_effects(&mut coop, tick, &mut world).unwrap();
    }
    assert_eq!(services.randomization.unwrap().steps_left, 30);
    assert_ne!(before, coop.players[3].inventory);
    let codec = bincode::config::standard().with_fixed_int_encoding();
    let bytes = bincode::encode_to_vec((&coop, &services), codec).unwrap();
    let ((mut restored_coop, mut restored), _): ((CoopState, PlayerServices), _) =
        bincode::decode_from_slice(&bytes, codec).unwrap();
    let mut restored_world = world.clone();
    for tick in 30..60 {
        services.tick_effects(&mut coop, tick, &mut world).unwrap();
        restored
            .tick_effects(&mut restored_coop, tick, &mut restored_world)
            .unwrap();
    }
    assert!(services.randomization.is_none());
    assert_eq!((&coop, &services), (&restored_coop, &restored));
    assert_eq!(
        coop.players
            .iter()
            .map(|hero| hero.inventory.iter().flatten().count())
            .sum::<usize>(),
        count
    );
}

fn finish_authored_animation(services: &mut PlayerServices, coop: &mut CoopState, seat: SeatId) {
    let descriptor = coop.players[seat.index()].animation.descriptor;
    services
        .apply_animation(
            coop,
            seat,
            PlayerAnimationStep {
                descriptor,
                sequence_index: 1,
                ticks_left: 0,
                flip_horizontal: false,
                held: true,
                motion: None,
            },
        )
        .unwrap();
}

#[test]
fn enemy_and_rain_projectiles_damage_every_hero_in_overlays() {
    for archetype in [Archetype::ToeJam, Archetype::Earl] {
        for seat in SeatId::ALL {
            let (mut coop, mut services, mut world) = fixture(archetype, seat);
            services
                .ui(&mut coop, seat, UiIntent::OpenInventory)
                .unwrap();
            let mut projectile = Effect::stationary(
                seat,
                PresentKind::TomatoRain,
                EffectKind::RainTomato,
                level(2),
                coop.players[seat.index()].motion.position,
                30,
            );
            projectile.position.elevation = 10 * 256;
            projectile.owner = EffectOwner::Actor(ActorId::new(32, 1).unwrap());
            projectile.source = EffectSource::Actor(crate::DefinitionId::new(3).unwrap());
            let id = services.resources.spawn_effect(projectile).unwrap();
            services.tick_effects(&mut coop, 0, &mut world).unwrap();
            assert_eq!(
                coop.players[seat.index()].health,
                archetype.base_health() - 6
            );
            assert!(services.resources.effect(id).is_none());
            assert_eq!(coop.players[seat.index()].ui.overlay, Overlay::Inventory);
            services.validate(&coop).unwrap();
        }
    }
}

#[test]
fn animation_zero_duration_is_not_an_unstarted_or_completed_sequence() {
    let (mut coop, mut services, mut world) = fixture(Archetype::ToeJam, SeatId::ALL[2]);
    let seat = SeatId::ALL[2];
    coop.players[2].motion.position = point(1000, 710);
    services.runtime[2].previous_input.directions = 1;
    assert!(
        services
            .service_contact(
                &mut coop,
                seat,
                ActorId::new(32, 1).unwrap(),
                HeroServiceKind::Lemonade,
                point(1000, 700)
            )
            .unwrap()
    );
    services
        .tick_players(&mut coop, 0, [PlayerInput::default(); 4], &mut world)
        .unwrap();
    assert!(services.runtime[2].service.is_some());
    let descriptor = coop.players[2].animation.descriptor;
    services
        .apply_animation(
            &mut coop,
            seat,
            PlayerAnimationStep {
                descriptor,
                sequence_index: 0,
                ticks_left: 0,
                flip_horizontal: false,
                held: false,
                motion: Some(AuthoredMotion {
                    axes: 3,
                    multiplier: [1, 2],
                }),
            },
        )
        .unwrap();
    services
        .tick_players(&mut coop, 1, [PlayerInput::default(); 4], &mut world)
        .unwrap();
    assert!(services.runtime[2].service.is_some());
    finish_authored_animation(&mut services, &mut coop, seat);
    services
        .tick_players(&mut coop, 2, [PlayerInput::default(); 4], &mut world)
        .unwrap();
    assert!(services.runtime[2].service.is_none());
    assert_eq!(services.runtime[2].authored_motion.multiplier, [1, 2]);
}

#[test]
fn remap_projectiles_apply_status_not_damage_to_each_seat() {
    for seat in SeatId::ALL {
        let (mut coop, mut services, mut world) = fixture(Archetype::Earl, seat);
        let health = coop.players[seat.index()].health;
        let mut effect = Effect::stationary(
            seat,
            PresentKind::Tomatoes,
            EffectKind::ControlRemap,
            level(2),
            coop.players[seat.index()].motion.position,
            20,
        );
        effect.position.elevation = 10 * 256;
        effect.owner = EffectOwner::Actor(ActorId::new(32, 1).unwrap());
        effect.source = EffectSource::Actor(crate::DefinitionId::new(3).unwrap());
        services.resources.spawn_effect(effect).unwrap();
        services.tick_effects(&mut coop, 0, &mut world).unwrap();
        assert_eq!(coop.players[seat.index()].health, health);
        assert_eq!(
            coop.players[seat.index()].counters.controls_remap_ticks,
            240
        );
        assert_ne!(services.runtime[seat.index()].remap, [1, 2, 4, 8]);
        services.validate(&coop).unwrap();
    }
}

struct AuthoredItemDefinitions;
impl ItemDefinitions for AuthoredItemDefinitions {
    fn food_health_delta(&self, kind: u8) -> PlayerResult<i8> {
        match kind {
            64..=79 => Ok(4),
            81 => Ok(12),
            82 => Ok(0),
            83 => Ok(-8),
            _ => Err(PlayerError::Invalid("authored food kind")),
        }
    }
}

#[test]
fn original_item_binding_preserves_all_present_kinds_source_holes_and_large_items() {
    let (mut coop, mut services, world) = fixture(Archetype::ToeJam, SeatId::ALL[0]);
    let mut items: [StockItem; 32] = std::array::from_fn(|index| StockItem {
        slot: index as u8,
        kind: index as i8,
        level: level(2),
        x: 400,
        y: 400,
    });
    items[28].kind = -1;
    items[29].kind = 80;
    items[30].kind = 82;
    items[31].kind = 83;
    services
        .resources
        .import_stock_items(level(2), &items, &AuthoredItemDefinitions)
        .unwrap();
    assert_eq!(services.resources.items().len(), 31);
    for kind in PresentKind::ALL {
        let item = services.resources.item(2 * 128 + kind as u32 + 1).unwrap();
        let ItemKind::Present(wrapper) = item.kind else {
            panic!("present source kind");
        };
        assert_eq!(services.resources.presents.kind(wrapper), kind);
    }
    assert!(services.resources.item(2 * 128 + 29).is_none());
    assert_eq!(
        services.resources.item(2 * 128 + 30).unwrap().kind,
        ItemKind::Buck(1)
    );
    assert_eq!(
        services.resources.item(2 * 128 + 31).unwrap().kind,
        ItemKind::Food(0)
    );
    let mut output = [ResourceOutcome::default()];
    services
        .resources
        .transact(
            &mut coop,
            0,
            &[ResourceRequest {
                seat: SeatId::ALL[0],
                action: ResourceAction::Claim(ItemRef::Placed(2 * 128 + 32)),
            }],
            &mut output,
        )
        .unwrap();
    assert_eq!(coop.players[0].health, Archetype::ToeJam.base_health() - 8);
    assert!(services.resources.item(2 * 128 + 32).unwrap().collected);
    let before = services.resources.clone();
    assert!(
        services
            .resources
            .import_stock_items(level(2), &items, &AuthoredItemDefinitions)
            .is_err()
    );
    assert_eq!(services.resources, before);
    let mut invalid = items;
    for item in &mut invalid {
        item.level = level(3);
    }
    invalid[31].kind = 63;
    assert!(
        services
            .resources
            .import_stock_items(level(3), &invalid, &AuthoredItemDefinitions)
            .is_err()
    );
    assert!(!services.resources.stock_imported(level(3)));
    assert_eq!(services.resources, before);
    services.validate(&coop).unwrap();
    assert!(world.actors.is_empty());
}

#[test]
fn randomizing_wrappers_does_not_change_ground_or_dropped_present_contents() {
    let (mut coop, mut services, mut world) = fixture(Archetype::ToeJam, SeatId::ALL[0]);
    let seat = SeatId::ALL[0];
    let wrapper = services
        .resources
        .presents
        .wrapper_for(PresentKind::ExtraLife);
    services
        .resources
        .add_item(PlacedItem {
            id: 42,
            level: level(2),
            position: point(400, 400),
            kind: ItemKind::Present(wrapper),
            collected: false,
        })
        .unwrap();
    coop.players[0].inventory[0] = Some(wrapper);
    let mut output = [ResourceOutcome::default()];
    services
        .resources
        .transact(
            &mut coop,
            0,
            &[ResourceRequest {
                seat,
                action: ResourceAction::Drop { slot: 0 },
            }],
            &mut output,
        )
        .unwrap();
    equip(&mut coop, &services, seat, PresentKind::Randomizer);
    services
        .open_present(&mut coop, seat, 0, &mut world)
        .unwrap();
    let ItemKind::Present(ground) = services.resources.item(42).unwrap().kind else {
        panic!("ground present");
    };
    assert_eq!(
        services.resources.presents.kind(ground),
        PresentKind::ExtraLife
    );
    let (_, dropped) = services.resources.drops().next().unwrap();
    assert_eq!(
        services.resources.presents.kind(dropped.present),
        PresentKind::ExtraLife
    );
    services.validate(&coop).unwrap();
}

#[test]
fn direct_damage_and_remap_preserve_existing_motion_and_facing() {
    let (mut coop, mut services, world) = fixture(Archetype::Earl, SeatId::ALL[3]);
    let seat = SeatId::ALL[3];
    coop.players[3].motion.velocity = [400, -200, 128];
    coop.players[3].motion.acceleration = [0, 0, -32];
    services.runtime[3].facing = 3;
    let motion = coop.players[3].motion;
    services
        .damage(
            &mut coop,
            seat,
            DamageRequest {
                source: DamageSource::Terrain,
                amount: 4,
                style: DamageStyle::Direct,
                origin: point(0, 0),
            },
            &world,
        )
        .unwrap();
    assert_eq!(coop.players[3].motion, motion);
    assert_eq!(services.runtime[3].facing, 3);
    services
        .status(
            &mut coop,
            seat,
            StatusRequest::Remap {
                ticks: 240,
                directions: [8, 4, 1, 2],
            },
        )
        .unwrap();
    assert_eq!(coop.players[3].motion, motion);
    assert_eq!(services.runtime[3].facing, 3);
}

#[test]
fn runtime_food_catalogue_is_bounded_for_both_supported_private_roms() {
    for revision in tje_mem::Revision::ALL {
        assert!(RomItemDefinitions::new(&[], revision).is_err());
        let name = match revision {
            tje_mem::Revision::Rev00 => "rev00.bin",
            tje_mem::Revision::Rev02 => "rev02.bin",
        };
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../roms")
            .join(name);
        let Ok(rom) = std::fs::read(path) else {
            continue;
        };
        let definitions = RomItemDefinitions::new(&rom, revision).unwrap();
        for kind in 64_u8..=83 {
            if kind == 80 {
                assert!(definitions.food_health_delta(kind).is_err());
                continue;
            }
            let address = tje_mem::rom::present_auxiliary_signed_lookup(revision)
                .at(i32::from(kind - 64))
                .address() as usize;
            assert_eq!(
                definitions.food_health_delta(kind).unwrap(),
                rom[address] as i8
            );
        }
        assert!(definitions.food_health_delta(63).is_err());
        assert!(definitions.food_health_delta(84).is_err());
    }
}

#[test]
fn initial_stock_supply_is_atomic_for_one_to_four_heroes() {
    for mask in 1..=15 {
        let mut config = Config::four_seats(1);
        config.participants = SeatMask::new(mask).unwrap();
        let mut coop = CoopState::new(config).unwrap();
        let services = PlayerServices::new(1, ResourceLimits::default()).unwrap();
        services.initialize_stock_inventory(&mut coop).unwrap();
        let count = if mask.count_ones() == 1 { 4 } else { 2 };
        for seat in SeatId::ALL {
            let inventory = &coop.players[seat.index()].inventory;
            assert_eq!(
                inventory.iter().flatten().count(),
                if mask & (1 << seat.get()) == 0 {
                    0
                } else {
                    count
                }
            );
            for &wrapper in inventory.iter().flatten() {
                assert_eq!(
                    services.resources.presents.kind(wrapper),
                    PresentKind::BonusHiTops
                );
            }
        }
        let before = coop.clone();
        assert!(services.initialize_stock_inventory(&mut coop).is_err());
        assert_eq!(coop, before);
        services.validate(&coop).unwrap();
    }
}

#[test]
fn walking_sleep_and_present_pose_ids_follow_archetype_independent_source_families() {
    for archetype in [Archetype::ToeJam, Archetype::Earl] {
        for seat in SeatId::ALL {
            let (mut coop, mut services, mut world) = fixture(archetype, seat);
            services
                .tick_players(
                    &mut coop,
                    0,
                    input_for(
                        seat,
                        PlayerInput {
                            directions: 8,
                            jump: true,
                            ..PlayerInput::default()
                        },
                    ),
                    &mut world,
                )
                .unwrap();
            assert_eq!(coop.players[seat.index()].animation.descriptor, 11);
            assert_eq!(
                coop.players[seat.index()].motion.position.elevation,
                0,
                "ordinary heroes gain no invented jump"
            );
            assert!(
                services
                    .status(&mut coop, seat, StatusRequest::Sleep(20))
                    .unwrap()
            );
            assert_eq!(coop.players[seat.index()].animation.descriptor, 117);
            services.validate(&coop).unwrap();
        }
    }
}

#[test]
fn overlay_actions_do_not_also_answer_a_world_telephone() {
    let (mut coop, mut services, mut world) = fixture(Archetype::ToeJam, SeatId::ALL[0]);
    let seat = SeatId::ALL[0];
    equip(&mut coop, &services, seat, PresentKind::Telephone);
    services
        .open_present(&mut coop, seat, 0, &mut world)
        .unwrap();
    services
        .ui(&mut coop, seat, UiIntent::OpenInventory)
        .unwrap();
    services
        .tick_players(
            &mut coop,
            0,
            input_for(
                seat,
                PlayerInput {
                    action: true,
                    ..PlayerInput::default()
                },
            ),
            &mut world,
        )
        .unwrap();
    services.tick_effects(&mut coop, 0, &mut world).unwrap();
    assert_eq!(world.revealed, 0);
    services.ui(&mut coop, seat, UiIntent::Close).unwrap();
    services
        .tick_players(
            &mut coop,
            1,
            input_for(
                seat,
                PlayerInput {
                    action: true,
                    ..PlayerInput::default()
                },
            ),
            &mut world,
        )
        .unwrap();
    services.tick_effects(&mut coop, 1, &mut world).unwrap();
    assert_eq!(world.revealed, 1);
}
