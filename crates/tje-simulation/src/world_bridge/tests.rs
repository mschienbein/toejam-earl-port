use super::*;
use crate::players::{ActorEffect, PlayerWorld, WorldPresent};
use crate::population::{AuthoredActor, PopulationLimits};
use crate::resources::{PresentKind, ResourceLimits};
use tje_world::{
    Anchors, Cell, CellPosition, Density, LevelGeometry, LevelSize, Shape, WorldConfig,
    loading::{ImportKind, ImportSpec},
    original::OriginalRevision,
    population::{Population, PopulationBudget, StableBits},
};

struct Land;
impl Resources for Land {
    fn parameter(&self, _: LevelId, _: u8) -> tje_world::Result<(u8, u8)> {
        Ok((1, 1))
    }
    fn shape(&self, _: Shape, _: u8, _: bool) -> tje_world::Result<u8> {
        Ok(1)
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
fn level(id: u16) -> LevelId {
    LevelId::new(id).unwrap()
}
fn position(x: i32, y: i32) -> Position {
    Position {
        x: x * 256,
        y: y * 256,
        elevation: 0,
    }
}
fn import_floor(world: &mut WorldState, id: LevelId) {
    let dimensions = Dimensions::ORIGINAL;
    let anchors = Anchors {
        arrival: CellPosition { x: 0, y: 0 },
        exit: CellPosition { x: 18, y: 25 },
        ship_piece: None,
        regions: vec![],
    };
    let geometry = LevelGeometry::new(
        dimensions,
        vec![Cell::new(30, 0).unwrap(); dimensions.area()],
        vec![0; dimensions.area()],
        anchors.clone(),
    )
    .unwrap();
    let population = Population {
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
    };
    world
        .request_import(ImportSpec {
            config: world.config().clone(),
            level: id,
            kind: ImportKind::Designed {
                course_identity: [9; 32],
            },
            dimensions,
            anchors,
            population,
            source_digest: ImportSpec::content_digest(dimensions, &geometry).unwrap(),
        })
        .unwrap();
    while world.importing().is_some() {
        world.advance_import(&geometry, &Land, 1024).unwrap();
    }
}
fn fixture() -> (
    WorldState,
    PopulationState,
    LoadingState,
    CoopState,
    PlayerServices,
    DefinitionCatalog,
    ControlRemaps,
    PresentCatalog,
) {
    let config = WorldConfig {
        root_seed: 42,
        rom_sha1: [0; 20],
        size: LevelSize::Original,
        density: Density::Original,
    };
    let mut world = WorldState::new(config).unwrap();
    let catalog = population::tests::catalog();
    let mut population = PopulationState::new(
        PopulationLimits {
            per_floor: 64,
            active_bodies: 316,
        },
        [7; 32],
    )
    .unwrap();
    for id in [level(2), level(3)] {
        import_floor(&mut world, id);
        population.stage_floor(id, &[], &catalog).unwrap();
    }
    let mut coop = CoopState::new(tje_four_player::Config::four_seats(42)).unwrap();
    for seat in SeatId::ALL {
        let hero = &mut coop.players[seat.index()];
        hero.life = LifeState::Alive;
        hero.level = Some(level(2));
        hero.health = hero.character.archetype.base_health();
        hero.motion.position = position(800 + i32::from(seat.get()) * 100, 500);
        world.set_active(seat.get(), level(2)).unwrap();
        population.retain(seat, level(2)).unwrap();
    }
    let services = PlayerServices::new(42, ResourceLimits::default()).unwrap();
    let loading =
        LoadingState::new(world.config(), SeatMask::ALL, OriginalRevision::Rev02).unwrap();
    let remaps = ControlRemaps::from_rows(std::array::from_fn(|_| {
        std::array::from_fn(|input| {
            if matches!(input, 1 | 2 | 4 | 5 | 6 | 8 | 9 | 10) {
                input as u8
            } else {
                0
            }
        })
    }))
    .unwrap();
    let presents = PresentCatalog::new([1; 27]).unwrap();
    (
        world, population, loading, coop, services, catalog, remaps, presents,
    )
}
fn animation(_: Kind, _: u8, cursor: &mut AnimationCursor) -> tje_world::Result<AnimationSample> {
    cursor.started = true;
    Ok(AnimationSample {
        frame: 0,
        motion: None,
        finished: false,
    })
}
fn resources<'a>(
    catalog: &'a DefinitionCatalog,
    remaps: &'a ControlRemaps,
    presents: &'a PresentCatalog,
) -> BridgeResources<'a, Land> {
    BridgeResources {
        terrain: &Land,
        earthlings: catalog,
        presents,
        remaps,
        animation_step: &animation,
    }
}

#[test]
fn snapshots_and_geometry_borrow_without_mutable_player_services() {
    let (mut world, mut population, mut loading, coop, services, catalog, remaps, presents) =
        fixture();
    let heroes = capture_heroes(&coop, &services);
    let effects = capture_effects(&services.resources);
    let actors = capture_actor_bodies(&population, &catalog).unwrap();
    let frozen = GeometryView::frozen(
        &world,
        &loading,
        &Land,
        &actors,
        BridgeViews {
            heroes: &heroes,
            effects: &effects,
        },
    )
    .unwrap();
    assert!(
        !frozen
            .blocked(
                SeatId::ALL[0],
                Archetype::ToeJam,
                level(2),
                position(800, 500)
            )
            .unwrap()
    );
    assert!(
        frozen
            .blocked(
                SeatId::ALL[0],
                Archetype::ToeJam,
                level(2),
                position(900, 500)
            )
            .unwrap()
    );
    let bridge = WorldBridge::new(
        &mut world,
        &mut population,
        &mut loading,
        resources(&catalog, &remaps, &presents),
        BridgeViews {
            heroes: &heroes,
            effects: &effects,
        },
        1,
    )
    .unwrap();
    assert_eq!(bridge.terrain(level(3), 1000, 500).unwrap(), 0x10);
    assert!(bridge.terrain(level(4), 1000, 500).is_err());
    assert_eq!(
        bridge
            .terrain_cache(Landing {
                level: level(2),
                position: position(800, 500)
            })
            .unwrap(),
        [[0x10; 5]; 5]
    );
}
#[test]
fn same_tick_present_creation_uses_returned_pose_and_collision_overlay() {
    let (mut world, mut population, mut loading, coop, services, catalog, remaps, presents) =
        fixture();
    let heroes = capture_heroes(&coop, &services);
    let effects = capture_effects(&services.resources);
    let mut bridge = WorldBridge::new(
        &mut world,
        &mut population,
        &mut loading,
        resources(&catalog, &remaps, &presents),
        BridgeViews {
            heroes: &heroes,
            effects: &effects,
        },
        1,
    )
    .unwrap();
    let origin = coop.players[0].motion.position;
    let receipt = bridge
        .commit_present(WorldPresent::OpenDecoy {
            owner: SeatId::ALL[0],
            level: level(2),
            position: origin,
        })
        .unwrap();
    let accepted = receipt.effect_position.unwrap();
    assert_eq!(accepted, position(800, 501));
    let id = EffectId::new(0, 1).unwrap();
    let effect = Effect::stationary(
        SeatId::ALL[0],
        PresentKind::Decoy,
        EffectKind::Decoy,
        level(2),
        accepted,
        100,
    );
    bridge.effect_created(id, effect).unwrap();
    assert!(
        bridge
            .blocked(SeatId::ALL[0], Archetype::ToeJam, level(2), origin)
            .unwrap()
    );
    assert!(
        !bridge
            .movement_blocked(
                SeatId::ALL[0],
                Archetype::ToeJam,
                level(2),
                origin,
                position(780, 500)
            )
            .unwrap()
    );
    bridge
        .apply_actor_effect(ActorEffect::Retire {
            effect: id,
            level: level(2),
        })
        .unwrap();
    assert!(
        !bridge
            .blocked(SeatId::ALL[0], Archetype::ToeJam, level(2), origin)
            .unwrap()
    );
    assert!(
        bridge
            .apply_actor_effect(ActorEffect::Retire {
                effect: id,
                level: level(2)
            })
            .is_err()
    );
}
#[test]
fn togetherness_commits_shared_floor_ledgers_without_borrowing_hero_state() {
    let (mut world, mut population, mut loading, mut coop, services, catalog, remaps, presents) =
        fixture();
    coop.players[3].level = Some(level(3));
    coop.players[3].motion.position = position(1000, 600);
    world.set_active(3, level(3)).unwrap();
    population.retain(SeatId::ALL[3], level(3)).unwrap();
    let heroes = capture_heroes(&coop, &services);
    let effects = capture_effects(&services.resources);
    let mut bridge = WorldBridge::new(
        &mut world,
        &mut population,
        &mut loading,
        resources(&catalog, &remaps, &presents),
        BridgeViews {
            heroes: &heroes,
            effects: &effects,
        },
        1,
    )
    .unwrap();
    let destination = Landing {
        level: level(3),
        position: Position {
            elevation: 70 * 256,
            ..position(1000, 600)
        },
    };
    bridge
        .commit_present(WorldPresent::Togetherness {
            owner: SeatId::ALL[0],
            target: SeatId::ALL[3],
            destination,
        })
        .unwrap();
    assert_eq!(bridge.world.active()[0], Some(level(3)));
    assert_eq!(bridge.population.retained()[0], Some(level(3)));
    assert_eq!(
        bridge.hero(SeatId::ALL[0]).unwrap().position,
        destination.position
    );
    assert_eq!(coop.players[0].level, Some(level(2)));
    bridge.release_hero(SeatId::ALL[0]).unwrap();
    assert_eq!(bridge.world.active()[0], None);
    assert_eq!(bridge.population.retained()[0], None);
    let landing = bridge
        .respawn(
            SeatId::ALL[0],
            Landing {
                level: level(3),
                position: position(1400, 600),
            },
        )
        .unwrap();
    bridge.commit_arrival(SeatId::ALL[0], landing).unwrap();
    assert_eq!(bridge.world.active()[0], Some(level(3)));
}
#[test]
fn real_actor_effects_damage_sleep_and_retire_scoped_state() {
    let (mut world, mut population, mut loading, coop, mut services, catalog, remaps, presents) =
        fixture();
    population
        .stage_floor(
            level(4),
            &[AuthoredActor {
                local: 0,
                kind: Kind::Devil,
                position: position(1000, 700),
                mail_order: false,
            }],
            &catalog,
        )
        .unwrap();
    import_floor(&mut world, level(4));
    // Active-floor insertion is prepared separately, exactly as the loader does.
    let source = EffectId::new(0, 1).unwrap();
    let effect = Effect::stationary(
        SeatId::ALL[0],
        PresentKind::Boombox,
        EffectKind::Boombox,
        level(4),
        position(1000, 700),
        300,
    );
    let id = services.resources.spawn_effect(effect).unwrap();
    assert_eq!(id, source);
    let mut heroes = capture_heroes(&coop, &services);
    heroes[0].as_mut().unwrap().level = Some(level(4));
    heroes[0].as_mut().unwrap().position = position(1000, 700);
    world.set_active(0, level(4)).unwrap();
    population.retain(SeatId::ALL[0], level(4)).unwrap();
    let effects = capture_effects(&services.resources);
    let hero_views = SeatId::ALL.map(|seat| {
        heroes[seat.index()]
            .filter(|hero| hero.life == LifeState::Alive)
            .map(|hero| population::HeroView {
                seat,
                level: hero.level.unwrap(),
                position: hero.position,
                width: hero.archetype.dimensions().0,
                velocity: [0; 3],
                targetable: true,
                active_motion: false,
                receives_effects: true,
                swimming: false,
                elevator: false,
                suppressed: false,
                has_inventory: true,
                remapped: false,
                sneaking: false,
                burping: false,
                service_eligible: true,
                response: None,
            })
    });
    population.refresh_activity(&hero_views).unwrap();
    let mut bridge = WorldBridge::new(
        &mut world,
        &mut population,
        &mut loading,
        resources(&catalog, &remaps, &presents),
        BridgeViews {
            heroes: &heroes,
            effects: &effects,
        },
        1,
    )
    .unwrap();
    assert!(
        bridge
            .apply_actor_effect(ActorEffect::Boombox {
                effect: source,
                level: level(4),
                position: position(1000, 700)
            })
            .unwrap()
    );
    assert_eq!(
        bridge
            .population
            .floors()
            .iter()
            .find(|floor| floor.level == level(4))
            .unwrap()
            .actors[0]
            .state,
        population::SLEEP_STATE
    );
    bridge
        .apply_actor_effect(ActorEffect::Retire {
            effect: source,
            level: level(4),
        })
        .unwrap();
    assert_eq!(
        bridge
            .population
            .floors()
            .iter()
            .find(|floor| floor.level == level(4))
            .unwrap()
            .actors[0]
            .state,
        0
    );
}
#[test]
fn remaps_keep_exact_diagonal_lookup_and_reject_invalid_rows() {
    let mut rows = [[0; 16]; 4];
    for row in &mut rows {
        for input in [1_usize, 2, 4, 5, 6, 8, 9, 10] {
            row[input] = input as u8;
        }
    }
    rows[0][1] = 5;
    rows[0][4] = 1;
    rows[0][5] = 2;
    let remaps = ControlRemaps::from_rows(rows).unwrap();
    assert_eq!(remaps.directions(0).unwrap()[5], 2);
    assert_ne!(
        remaps.directions(0).unwrap()[1] | remaps.directions(0).unwrap()[4],
        2
    );
    assert!(remaps.directions(4).is_err());
    rows[2][0] = 1;
    assert!(ControlRemaps::from_rows(rows).is_err());
}

#[test]
fn stock_count_growth_and_exhaustion_leave_world_rng_and_ids_atomic() {
    let (mut world, mut population, mut loading, coop, services, catalog, remaps, presents) =
        fixture();
    let heroes = capture_heroes(&coop, &services);
    let effects = capture_effects(&services.resources);
    // Fill original stable slots on a third, already loaded shared floor.
    import_floor(&mut world, level(4));
    let environment = PopulationEnvironment {
        world: &world,
        loading: &loading,
        resources: &Land,
        animation_step: &animation,
    };
    let prepared = population
        .prepare_stock_floor(
            level(4),
            &[Some(Kind::Devil); 20],
            &[],
            &environment,
            &catalog,
        )
        .unwrap();
    assert_eq!(prepared.actors.len(), 20);
    population.install_prepared_floor(prepared).unwrap();
    world.set_active(0, level(4)).unwrap();
    population.retain(SeatId::ALL[0], level(4)).unwrap();
    let mut heroes = heroes;
    heroes[0].as_mut().unwrap().level = Some(level(4));
    let mut bridge = WorldBridge::new(
        &mut world,
        &mut population,
        &mut loading,
        resources(&catalog, &remaps, &presents),
        BridgeViews {
            heroes: &heroes,
            effects: &effects,
        },
        1,
    )
    .unwrap();
    let request = WorldPresent::SpawnEarthling {
        owner: SeatId::ALL[0],
        level: level(4),
        position: position(800, 500),
        choice: 0,
    };
    bridge.commit_present(request).unwrap();
    assert_eq!(
        bridge
            .population
            .floors()
            .iter()
            .find(|floor| floor.level == level(4))
            .unwrap()
            .actors
            .len(),
        21
    );
    while bridge
        .population
        .floors()
        .iter()
        .find(|floor| floor.level == level(4))
        .unwrap()
        .declared_slots
        != u64::MAX
    {
        bridge.commit_present(request).unwrap();
    }
    let world_before = bridge.world.clone();
    let population_before = bridge.population.clone();
    let loading_before = bridge.loading.clone();
    assert!(bridge.commit_present(request).is_err());
    assert_eq!(*bridge.world, world_before);
    assert_eq!(*bridge.population, population_before);
    assert_eq!(*bridge.loading, loading_before);
}

#[test]
fn props_bind_accepted_poses_and_retire_only_the_exact_live_generation() {
    let (mut world, mut population, mut loading, coop, services, catalog, remaps, presents) =
        fixture();
    let heroes = capture_heroes(&coop, &services);
    let effects = capture_effects(&services.resources);
    let mut bridge = WorldBridge::new(
        &mut world,
        &mut population,
        &mut loading,
        resources(&catalog, &remaps, &presents),
        BridgeViews {
            heroes: &heroes,
            effects: &effects,
        },
        1,
    )
    .unwrap();
    let receipt = bridge
        .commit_present(WorldPresent::OpenDoorway {
            owner: SeatId::ALL[0],
            level: level(2),
            position: position(800, 500),
            choice: 1,
        })
        .unwrap();
    let accepted = receipt.effect_position.unwrap();
    assert_eq!(accepted, position(784, 482));
    let id = EffectId::new(0, 1).unwrap();
    bridge
        .effect_created(
            id,
            Effect::stationary(
                SeatId::ALL[0],
                PresentKind::Doorway,
                EffectKind::Doorway,
                level(2),
                accepted,
                3000,
            ),
        )
        .unwrap();
    assert_eq!(bridge.loading.doors[0].as_ref().unwrap().effect, Some(id));
    assert!(
        bridge
            .apply_actor_effect(ActorEffect::Retire {
                effect: EffectId::new(0, 2).unwrap(),
                level: level(2)
            })
            .is_err()
    );
    assert!(bridge.loading.doors[0].is_some());
    bridge
        .apply_actor_effect(ActorEffect::Retire {
            effect: id,
            level: level(2),
        })
        .unwrap();
    assert!(bridge.loading.doors[0].is_none());
    let receipt = bridge
        .commit_present(WorldPresent::OpenTelephone {
            owner: SeatId::ALL[3],
            level: level(2),
            position: position(1100, 500),
        })
        .unwrap();
    let accepted = receipt.effect_position.unwrap();
    assert_eq!(accepted, position(1100, 510));
    let id = EffectId::new(1, 1).unwrap();
    bridge
        .effect_created(
            id,
            Effect::stationary(
                SeatId::ALL[3],
                PresentKind::Telephone,
                EffectKind::Telephone,
                level(2),
                accepted,
                3000,
            ),
        )
        .unwrap();
    assert!(
        bridge
            .answer_telephone(id, SeatId::ALL[2], level(2))
            .is_err()
    );
    assert!(bridge.loading.reveal(SeatId::ALL[2]).is_none());
}

#[test]
fn source_delta_sync_never_writes_dynamic_lifetime_ids() {
    let (mut world, population, _, _, _, catalog, _, _) = fixture();
    // A newly authored source placement, not a map/actor RAM dump.
    let source = tje_world::population::Placement {
        id: 0,
        x: 640,
        y: 400,
        role: tje_world::population::PlacementRole::Earthling,
        dormant_ticks: 0,
        ai_rng: StageRng::new(1),
    };
    world
        .floor_mut(level(2))
        .unwrap()
        .generated
        .population
        .earthlings
        .push(source);
    world
        .floor_mut(level(2))
        .unwrap()
        .generated
        .population
        .budget
        .earthlings = 1;
    // Prepare matching persistent source separately rather than mutating the
    // already-published floor's actor identities through a test-only setter.
    let mut matching = PopulationState::new(
        PopulationLimits {
            per_floor: 64,
            active_bodies: 316,
        },
        [7; 32],
    )
    .unwrap();
    matching
        .stage_floor(
            level(2),
            &[AuthoredActor {
                local: 0,
                kind: Kind::Devil,
                position: position(640, 400),
                mail_order: false,
            }],
            &catalog,
        )
        .unwrap();
    matching.stage_floor(level(3), &[], &catalog).unwrap();
    assert!(validate_population_deltas(&world, &matching).is_ok());
    matching
        .remove(
            population::PopulationId {
                level: level(2),
                local: 0,
            },
            false,
        )
        .unwrap();
    assert!(validate_population_deltas(&world, &matching).is_err());
    sync_population_deltas(&mut world, &matching).unwrap();
    assert!(
        world
            .floor(level(2))
            .unwrap()
            .generated
            .population
            .removed
            .contains(0)
    );
    assert!(
        !world
            .floor(level(2))
            .unwrap()
            .generated
            .population
            .removed
            .contains(20)
    );
    // A different owner ledger may not resurrect that source.
    assert!(sync_population_deltas(&mut world, &population).is_err());
}

#[test]
fn successful_pickup_clears_only_its_seat_mole_contacts_on_the_retained_floor() {
    let (mut world, _, mut loading, coop, services, catalog, remaps, presents) = fixture();
    let mut population = PopulationState::new(
        PopulationLimits {
            per_floor: 64,
            active_bodies: 316,
        },
        [7; 32],
    )
    .unwrap();
    for floor in [level(2), level(3)] {
        population
            .stage_floor(
                floor,
                &[AuthoredActor {
                    local: 0,
                    kind: Kind::Mole,
                    position: position(640, 400),
                    mail_order: false,
                }],
                &catalog,
            )
            .unwrap();
        population
            .actor_mut(population::PopulationId {
                level: floor,
                local: 0,
            })
            .unwrap()
            .mole_contacts = SeatMask::ALL;
    }
    for seat in SeatId::ALL {
        population.retain(seat, level(2)).unwrap();
    }
    let heroes = capture_heroes(&coop, &services);
    let effects = capture_effects(&services.resources);
    let mut bridge = WorldBridge::new(
        &mut world,
        &mut population,
        &mut loading,
        resources(&catalog, &remaps, &presents),
        BridgeViews {
            heroes: &heroes,
            effects: &effects,
        },
        1,
    )
    .unwrap();
    let event = ActorEffect::ClearMoleContacts {
        seat: SeatId::ALL[2],
        level: level(2),
    };
    assert!(bridge.apply_actor_effect(event).unwrap());
    assert!(!bridge.apply_actor_effect(event).unwrap());
    assert_eq!(
        bridge
            .population
            .actor(population::PopulationId {
                level: level(2),
                local: 0
            })
            .unwrap()
            .mole_contacts
            .bits(),
        0b1011
    );
    assert_eq!(
        bridge
            .population
            .actor(population::PopulationId {
                level: level(3),
                local: 0
            })
            .unwrap()
            .mole_contacts,
        SeatMask::ALL
    );
    let before = bridge.population.clone();
    assert!(
        bridge
            .apply_actor_effect(ActorEffect::ClearMoleContacts {
                seat: SeatId::ALL[1],
                level: level(3)
            })
            .is_err()
    );
    assert_eq!(*bridge.population, before);
    bridge.release_hero(SeatId::ALL[1]).unwrap();
    assert!(
        bridge
            .apply_actor_effect(ActorEffect::ClearMoleContacts {
                seat: SeatId::ALL[1],
                level: level(2)
            })
            .is_err()
    );
}
