use super::*;
use tje_game::ids;
use tje_mem::{AddressSpace, Revision};
use tje_world::Density;

fn position(x: i32, y: i32) -> Position {
    Position {
        x: x * 256,
        y: y * 256,
        elevation: 0,
    }
}
fn level(id: u16) -> LevelId {
    LevelId::new(id).unwrap()
}
fn hero(seat: usize, floor: u16, x: i32, y: i32) -> HeroView {
    HeroView {
        seat: SeatId::ALL[seat],
        level: level(floor),
        position: position(x, y),
        width: 24,
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
    }
}
fn base_definition() -> Definition {
    // Authored parameters, deliberately not a cartridge definition/table golden.
    Definition {
        thinker: Thinker::Generic,
        movement: Movement::AllowedTerrain,
        touch: Touch::Hurt,
        axes: 3,
        width: 20,
        height: 24,
        target_chance: 100,
        target_distance: 100,
        turn_chance: 0,
        companion: false,
        change_axis: false,
        radius: 2,
        contact_x: 100,
        contact_y: 75,
        damage: 2,
        target_time: TickRange {
            minimum: 2,
            maximum: 3,
        },
        turn_time: TickRange {
            minimum: 1,
            maximum: 2,
        },
        move_time: TickRange {
            minimum: 2,
            maximum: 4,
        },
        state_delay: 0,
        health: 3,
    }
}
pub(crate) fn catalog() -> DefinitionCatalog {
    let mut values = [base_definition(); 25];
    for kind in Kind::ALL {
        let value = &mut values[kind.index()];
        value.touch = match kind {
            Kind::ShipPiece => Touch::ShipPiece,
            Kind::HulaDancer => Touch::Hula,
            Kind::Cupid => Touch::Cupid,
            Kind::Whirlwind => Touch::Whirlwind,
            Kind::Mailbox => Touch::Mailbox,
            Kind::Wizard => Touch::Wizard,
            Kind::Wiseman => Touch::Wiseman,
            Kind::Chickens => Touch::Chicken,
            Kind::Boogieman => Touch::Boogie,
            Kind::OperaSinger => Touch::Opera,
            Kind::Mole => Touch::Mole,
            Kind::Santa => Touch::Santa,
            Kind::LemonadeStand => Touch::Lemonade,
            Kind::HotTub => Touch::HotTub,
            Kind::Hamster
            | Kind::Shopper
            | Kind::NerdHerd
            | Kind::Lawnmower
            | Kind::IceCreamTruck => Touch::Squish,
            _ => Touch::Hurt,
        };
        value.thinker = match kind {
            Kind::ShipPiece | Kind::LemonadeStand | Kind::HotTub => Thinker::None,
            Kind::HulaDancer => Thinker::HorizontalWander,
            Kind::Wizard => Thinker::HorizontalTurn,
            Kind::Wiseman => Thinker::HorizontalReversal,
            Kind::Bees | Kind::BeesVariant | Kind::Cupid | Kind::IceCreamTruck => Thinker::Timed,
            Kind::Whirlwind => Thinker::Whirlwind,
            Kind::Santa => Thinker::Santa,
            _ => Thinker::Generic,
        };
        if matches!(
            kind,
            Kind::Bees | Kind::BeesVariant | Kind::Cupid | Kind::IceCreamTruck | Kind::Santa
        ) {
            value.movement = Movement::MapBounds;
        }
    }
    DefinitionCatalog::new(
        values,
        ProjectileVectors {
            thresholds: [10, 20, 30, 40, 50, 60, 70, 80],
            velocities: [[320, 320]; 9],
        },
    )
    .unwrap()
}
struct Ground {
    terrain: u8,
    frames_finished: bool,
    no_placement: bool,
    emit_motion: bool,
}
impl Default for Ground {
    fn default() -> Self {
        Self {
            terrain: 0x10,
            frames_finished: false,
            no_placement: false,
            emit_motion: true,
        }
    }
}
impl Environment for Ground {
    fn dimensions(&self, _: LevelId) -> Result<Dimensions> {
        Ok(Dimensions::ORIGINAL)
    }
    fn terrain(&self, _: LevelId, _: i32, _: i32) -> Result<u8> {
        Ok(self.terrain)
    }
    fn base_metatile(&self, _: LevelId, _: i32, _: i32) -> Result<Option<u8>> {
        Ok(Some(30))
    }
    fn placement(
        &self,
        _: LevelId,
        _: Kind,
        column: u16,
        row: u16,
        _: &mut StageRng,
    ) -> Result<Option<(i32, i32)>> {
        Ok((!self.no_placement).then_some((
            (i32::from(column) + 1) * 128 + 63,
            (i32::from(row) + 1) * 64 + 31,
        )))
    }
    fn allows_placement(&self, _: LevelId, _: i32, _: i32) -> Result<bool> {
        Ok(true)
    }
    fn animation_step(
        &self,
        _: Kind,
        _: u8,
        cursor: &mut AnimationCursor,
    ) -> Result<AnimationSample> {
        cursor.sequence_index = (cursor.sequence_index + 1) % 8;
        cursor.started = true;
        Ok(AnimationSample {
            frame: cursor.sequence_index,
            motion: self.emit_motion.then_some([1, 1]),
            finished: self.frames_finished,
        })
    }
}
struct Sink {
    heroes: [Option<HeroView>; 4],
    requests: Vec<ContactRequest>,
    bucks: [u8; 4],
    inventory: [u8; 4],
    health: [u8; 4],
    reject: bool,
}
impl Sink {
    fn new(heroes: [Option<HeroView>; 4]) -> Self {
        Self {
            heroes,
            requests: Vec::new(),
            bucks: [10; 4],
            inventory: [4; 4],
            health: [10; 4],
            reject: false,
        }
    }
}
impl ContactSink for Sink {
    fn hero(&self, seat: SeatId) -> Option<HeroView> {
        self.heroes[seat.index()]
    }
    fn apply(&mut self, request: ContactRequest) -> Result<ContactOutcome> {
        self.requests.push(request);
        if self.reject {
            return Ok(ContactOutcome::Rejected);
        }
        let index = request.seat.index();
        match request.effect {
            ContactEffect::Service {
                stage: ServiceStage::Begin,
                cost,
                ..
            } => {
                if self.bucks[index] < cost {
                    return Ok(ContactOutcome::Rejected);
                }
                self.bucks[index] -= cost;
            }
            ContactEffect::Service {
                stage: ServiceStage::Tick,
                service: Service::Heal,
                ..
            } => {
                self.health[index] += 1;
                return Ok(if self.health[index] >= 12 {
                    ContactOutcome::Complete
                } else {
                    ContactOutcome::Applied
                });
            }
            ContactEffect::MoleTheft { .. } => {
                if self.inventory[index] == 0 {
                    return Ok(ContactOutcome::Rejected);
                }
                self.inventory[index] -= 1;
            }
            _ => {}
        }
        Ok(ContactOutcome::Applied)
    }
}
fn staged(kinds: &[Kind], bodies: u16) -> (PopulationState, DefinitionCatalog) {
    let definitions = catalog();
    let mut state = PopulationState::new(
        PopulationLimits {
            per_floor: 64,
            active_bodies: bodies,
        },
        [7; 32],
    )
    .unwrap();
    let authored: Vec<_> = kinds
        .iter()
        .enumerate()
        .map(|(index, &kind)| AuthoredActor {
            local: index as u16,
            kind,
            position: position(640 + index as i32 * 10, 400),
            mail_order: false,
        })
        .collect();
    state
        .stage_floor(level(2), &authored, &definitions)
        .unwrap();
    (state, definitions)
}
fn attach(state: &mut PopulationState, heroes: &[Option<HeroView>; 4]) {
    for view in heroes.iter().flatten() {
        state.retain(view.seat, view.level).unwrap();
    }
    state.refresh_activity(heroes).unwrap();
}
#[test]
fn default_count_and_density_are_not_render_or_seat_policies() {
    assert_eq!(ORIGINAL_POPULATION, 20);
    let baseline = tje_world::population::PopulationBaseline {
        walkable_tiles: 1000,
        ordinary_items: 20,
        earthlings: 20,
        reserved_items: 1,
        ship_piece: true,
        mailbox: true,
    };
    assert_eq!(
        PopulationState::density_budget(baseline, 4000, Density::Original)
            .unwrap()
            .earthlings,
        20
    );
    assert_eq!(
        PopulationState::density_budget(baseline, 4000, Density::Busy)
            .unwrap()
            .earthlings,
        64
    );
    assert_eq!(
        PopulationState::density_budget(baseline, 1000, Density::Low)
            .unwrap()
            .earthlings,
        15
    );
}
#[test]
fn four_same_level_heroes_retain_one_population_and_release_without_regeneration() {
    let (mut state, definitions) = staged(&[Kind::Devil; 20], 80);
    let heroes = SeatId::ALL.map(|seat| Some(hero(seat.index(), 2, 650, 400)));
    attach(&mut state, &heroes);
    assert_eq!(state.floors().len(), 1);
    assert_eq!(state.active_bodies().count(), 20);
    for actor in &state.floors()[0].actors {
        assert_eq!(actor.interest, SeatMask::ALL);
    }
    let id = state.floors()[0].actors[0].id;
    state.actor_mut(id).unwrap().timer = 17;
    let body = state.actor(id).unwrap().body.unwrap();
    for seat in SeatId::ALL {
        state.release(seat).unwrap();
    }
    state.refresh_activity(&[None; 4]).unwrap();
    assert!(state.resolve(body).is_none());
    assert_eq!(state.actor(id).unwrap().timer, 17);
    assert!(!state.stage_floor(level(2), &[], &definitions).unwrap());
    attach(&mut state, &heroes);
    assert_ne!(state.actor(id).unwrap().body, Some(body));
    state.validate().unwrap();
}
#[test]
fn four_disjoint_floors_pause_dormant_timers_and_survive_return_visits() {
    let (mut state, definitions) = staged(&[Kind::Devil], 8);
    for floor in 3..=5 {
        state
            .stage_floor(
                level(floor),
                &[AuthoredActor {
                    local: 0,
                    kind: Kind::Devil,
                    position: position(640, 400),
                    mail_order: false,
                }],
                &definitions,
            )
            .unwrap();
    }
    let heroes =
        SeatId::ALL.map(|seat| Some(hero(seat.index(), u16::from(seat.get()) + 2, 700, 400)));
    attach(&mut state, &heroes);
    let mut sink = Sink::new(heroes);
    state
        .step(1, &Ground::default(), &definitions, &[], &mut sink)
        .unwrap();
    for floor in state.floors() {
        assert_eq!(floor.actors[0].interest.bits().count_ones(), 1);
    }
    let id = PopulationId {
        level: level(5),
        local: 0,
    };
    let saved_timer = state.actor(id).unwrap().timer;
    sink.heroes[3] = None;
    state.release(SeatId::ALL[3]).unwrap();
    state
        .step(2, &Ground::default(), &definitions, &[], &mut sink)
        .unwrap();
    state
        .step(3, &Ground::default(), &definitions, &[], &mut sink)
        .unwrap();
    assert_eq!(state.actor(id).unwrap().timer, saved_timer);
    assert!(state.actor(id).unwrap().body.is_none());
    sink.heroes[3] = heroes[3];
    state.retain(SeatId::ALL[3], level(5)).unwrap();
    state
        .step(4, &Ground::default(), &definitions, &[], &mut sink)
        .unwrap();
    state.validate().unwrap();
}
#[test]
fn canonical_interest_has_exact_boundaries_and_no_camera_input() {
    let (mut state, _) = staged(&[Kind::Devil, Kind::ShipPiece], 4);
    let mut heroes = [None; 4];
    heroes[3] = Some(hero(3, 2, 640 + 480, 400 + 336));
    attach(&mut state, &heroes);
    assert!(state.floors()[0].actors[0].body.is_some());
    heroes[3].as_mut().unwrap().position.x += 256;
    state.refresh_activity(&heroes).unwrap();
    assert!(state.floors()[0].actors[0].body.is_none());
    assert!(state.floors()[0].actors[1].body.is_some());
}
#[test]
fn allocation_pressure_is_atomic_and_never_relocates_or_removes() {
    let (mut state, _) = staged(&[Kind::Devil; 4], 3);
    let before = state.clone();
    assert!(state.preflight_retain(SeatId::ALL[0], level(2)).is_err());
    assert!(state.retain(SeatId::ALL[0], level(2)).is_err());
    assert_eq!(state, before);
}
#[test]
fn nearest_target_prefers_decoys_and_higher_seat_on_equal_distance() {
    let (state, definitions) = staged(&[Kind::Devil], 8);
    let actor = &state.floors()[0].actors[0];
    let heroes = SeatId::ALL.map(|seat| Some(hero(seat.index(), 2, 644, 400)));
    assert_eq!(
        nearest_target(actor, 100, 0, &heroes, &[]).unwrap().target,
        Target::Hero(SeatId::ALL[3])
    );
    let decoys = SeatId::ALL.map(|seat| DecoyView {
        actor: ActorId::new(352 + u16::from(seat.get()), 1).unwrap(),
        owner: seat,
        level: level(2),
        width: 24,
        velocity: [0; 3],
        position: position(680, 400),
    });
    assert_eq!(
        nearest_target(
            actor,
            definitions.get(Kind::Devil).target_distance,
            0,
            &heroes,
            &decoys
        )
        .unwrap()
        .target,
        Target::Decoy(decoys[3].actor)
    );
    let wrong = [DecoyView {
        level: level(3),
        ..decoys[3]
    }];
    assert!(matches!(
        nearest_target(actor, 100, 0, &heroes, &wrong)
            .unwrap()
            .target,
        Target::Hero(_)
    ));
}
#[test]
fn eligibility_covers_four_seats_mole_latches_and_cupid_remap() {
    let (mut state, _) = staged(&[Kind::Mole, Kind::Cupid], 8);
    let mut heroes = SeatId::ALL.map(|seat| Some(hero(seat.index(), 2, 700, 400)));
    heroes[3].as_mut().unwrap().elevator = true;
    heroes[2].as_mut().unwrap().has_inventory = false;
    let mole = &mut state.floors[0].actors[0];
    mole.mole_contacts = SeatMask::new(2).unwrap();
    assert_eq!(
        nearest_target(mole, 100, 0, &heroes, &[]).unwrap().target,
        Target::Hero(SeatId::ALL[0])
    );
    heroes[1].as_mut().unwrap().remapped = true;
    heroes[2].as_mut().unwrap().remapped = true;
    assert_eq!(
        nearest_target(&state.floors[0].actors[1], 100, 0, &heroes, &[])
            .unwrap()
            .target,
        Target::Hero(SeatId::ALL[0])
    );
    state.clear_mole_contacts(SeatId::ALL[1], level(2)).unwrap();
    assert_eq!(state.floors[0].actors[0].mole_contacts, SeatMask::NONE);
}
#[test]
fn stable_removal_collection_and_generation_extend_beyond_bit_31() {
    let (mut state, _) = staged(&[Kind::Devil; 64], 64);
    let heroes = [Some(hero(0, 2, 960, 400)), None, None, None];
    attach(&mut state, &heroes);
    let id = PopulationId {
        level: level(2),
        local: 63,
    };
    let body = state.actor(id).unwrap().body.unwrap();
    assert!(state.remove(id, true).unwrap());
    assert!(!state.remove(id, true).unwrap());
    assert!(state.floors()[0].removed.contains(63));
    assert!(state.floors()[0].collected.contains(63));
    assert!(state.resolve(body).is_none());
    state.validate().unwrap();
}
#[test]
fn every_original_kind_runs_its_typed_dispatch_without_cartridge_callbacks() {
    let (mut state, definitions) = staged(&Kind::ALL, 64);
    let heroes = SeatId::ALL.map(|seat| Some(hero(seat.index(), 2, 700, 400)));
    attach(&mut state, &heroes);
    let mut sink = Sink::new(heroes);
    for tick in 0..16 {
        state
            .step(tick, &Ground::default(), &definitions, &[], &mut sink)
            .unwrap();
        state
            .advance_motion(tick, &Ground::default(), &definitions, &[], &mut sink)
            .unwrap();
        state
            .advance_animation(tick, &Ground::default(), &definitions, &[], &mut sink)
            .unwrap();
    }
    state.validate().unwrap();
    for kind in Kind::ALL {
        assert!(
            state.floors()[0]
                .actors
                .iter()
                .any(|actor| actor.kind == kind)
        );
    }
    assert!(
        sink.requests
            .iter()
            .any(|request| request.seat == SeatId::ALL[3])
    );
}
#[test]
fn contacts_rotate_once_per_hero_and_overlays_do_not_make_heroes_immune() {
    let (mut state, definitions) = staged(&[Kind::Devil], 8);
    let heroes = SeatId::ALL.map(|seat| Some(hero(seat.index(), 2, 650, 400)));
    attach(&mut state, &heroes);
    let mut sink = Sink::new(heroes);
    state
        .step(1, &Ground::default(), &definitions, &[], &mut sink)
        .unwrap();
    let seats: Vec<_> = sink
        .requests
        .iter()
        .filter(|request| matches!(request.effect, ContactEffect::Hurt { .. }))
        .map(|request| request.seat)
        .collect();
    assert_eq!(seats, SeatId::ALL);
    let saved = state.clone();
    let count = sink.requests.len();
    assert!(
        !state
            .step(1, &Ground::default(), &definitions, &[], &mut sink)
            .unwrap()
    );
    assert_eq!(state, saved);
    assert_eq!(sink.requests.len(), count);
}
#[test]
fn nice_states_and_original_immunities_survive_damage_pressure() {
    let kinds = [
        Kind::ShipPiece,
        Kind::Santa,
        Kind::LemonadeStand,
        Kind::HotTub,
        Kind::Wizard,
        Kind::OperaSinger,
        Kind::Wiseman,
    ];
    let (mut state, _) = staged(&kinds, 16);
    for local in 0..4 {
        assert!(
            !state
                .damage(
                    PopulationId {
                        level: level(2),
                        local
                    },
                    127
                )
                .unwrap()
        );
    }
    state.floors[0].actors[4].state = 2;
    state.floors[0].actors[5].state = 3;
    state.floors[0].actors[6].interactions[3] = Interaction::Active;
    for local in 4..7 {
        state
            .damage(
                PopulationId {
                    level: level(2),
                    local,
                },
                127,
            )
            .unwrap();
        assert_eq!(state.floors[0].actors[usize::from(local)].health, 1);
    }
}
#[test]
fn sleep_is_kind_and_state_specific_and_not_a_draw_residency_test() {
    let (mut state, _) = staged(
        &[
            Kind::Devil,
            Kind::Shark,
            Kind::Santa,
            Kind::Whirlwind,
            Kind::Wizard,
            Kind::Cupid,
            Kind::Mole,
        ],
        20,
    );
    let heroes = [Some(hero(0, 2, 680, 400)), None, None, None];
    attach(&mut state, &heroes);
    state.floors[0].actors[4].state = 4;
    state.floors[0].actors[5].state = 3;
    state.floors[0].actors[6].state = 3;
    let source = crate::EffectId::new(0, 1).unwrap();
    assert_eq!(
        state
            .sleep_near(source, level(2), position(680, 400))
            .unwrap(),
        2
    );
    assert_eq!(state.floors[0].actors[0].state, SLEEP_STATE);
    assert_eq!(state.floors[0].actors[4].state, SLEEP_STATE);
    state.wake_level(source, level(2)).unwrap();
    assert_eq!(state.floors[0].actors[0].state, 0);
}
#[test]
fn services_have_explicit_customer_cost_and_four_independent_latches() {
    let (mut state, definitions) = staged(&[Kind::Wizard], 8);
    let heroes = SeatId::ALL.map(|seat| Some(hero(seat.index(), 2, 650, 400)));
    attach(&mut state, &heroes);
    let mut sink = Sink::new(heroes);
    let actor = &mut state.floors[0].actors[0];
    contacts::dispatch(actor, definitions.get(Kind::Wizard), 3, &mut sink).unwrap();
    assert_eq!(actor.service_owner, Some(SeatId::ALL[3]));
    sink.heroes[3].as_mut().unwrap().response = Some((actor.id, true));
    contacts::dispatch(actor, definitions.get(Kind::Wizard), 0, &mut sink).unwrap();
    assert_eq!(sink.bucks, [10, 10, 10, 9]);
    actor.animation_finished = true;
    contacts::dispatch(actor, definitions.get(Kind::Wizard), 0, &mut sink).unwrap();
    contacts::dispatch(actor, definitions.get(Kind::Wizard), 0, &mut sink).unwrap();
    assert_eq!(sink.bucks[3], 9);
    assert_eq!(sink.health[3], 12);
    assert_eq!(actor.interactions[3], Interaction::Complete);
}
#[test]
fn bounded_placement_failure_is_stable_and_required_failure_does_not_publish() {
    let definitions = catalog();
    let mut state = PopulationState::new(PopulationLimits::default(), [7; 32]).unwrap();
    let ground = Ground {
        no_placement: true,
        ..Ground::default()
    };
    state
        .populate_floor(level(2), &[Kind::Devil; 20], &[], &ground, &definitions)
        .unwrap();
    assert!((0..20).all(|local| state.floors()[0].failed.contains(local)));
    let saved = state.clone();
    assert!(
        state
            .populate_floor(level(3), &[Kind::ShipPiece], &[], &ground, &definitions)
            .is_err()
    );
    assert_eq!(state, saved);
    state.validate().unwrap();
}
#[test]
fn present_capacity_failure_does_not_touch_records_rng_or_existing_actor() {
    let (mut state, definitions) = staged(&[Kind::Devil], 1);
    let view = hero(0, 2, 640, 400);
    attach(&mut state, &[Some(view), None, None, None]);
    let saved = state.clone();
    assert!(
        state
            .spawn_present(
                view.seat,
                view.level,
                view.position,
                17,
                &Ground::default(),
                &definitions
            )
            .is_err()
    );
    assert_eq!(state, saved);
}
#[test]
fn present_spawn_and_relocation_are_bounded_and_preserve_stable_ids() {
    let (mut state, definitions) = staged(&[Kind::Devil], 8);
    let view = hero(3, 2, 900, 500);
    attach(&mut state, &[None, None, None, Some(view)]);
    let id = state
        .spawn_present(
            view.seat,
            view.level,
            view.position,
            17,
            &Ground::default(),
            &definitions,
        )
        .unwrap();
    assert_eq!(id.local, 1);
    assert_eq!(state.actor(id).unwrap().id.level, level(2));
    let before = state.actor(id).unwrap().clone();
    let failed = Ground {
        no_placement: true,
        ..Ground::default()
    };
    assert!(!state.relocate(id, &failed, &definitions).unwrap());
    assert_eq!(state.actor(id).unwrap(), &before);
}
#[test]
fn all_four_mole_victims_have_distinct_contact_latches() {
    let (mut state, definitions) = staged(&[Kind::Mole], 8);
    let heroes = SeatId::ALL.map(|seat| Some(hero(seat.index(), 2, 650, 400)));
    attach(&mut state, &heroes);
    let mut sink = Sink::new(heroes);
    for seat in SeatId::ALL {
        let actor = &mut state.floors[0].actors[0];
        actor.set_state(4);
        actor.service_owner = None;
        contacts::dispatch(actor, definitions.get(Kind::Mole), seat.get(), &mut sink).unwrap();
        assert!(actor.mole_contacts.contains(seat));
        contacts::dispatch(actor, definitions.get(Kind::Mole), seat.get(), &mut sink).unwrap();
        assert_eq!(sink.inventory[seat.index()], 3);
    }
    assert_eq!(state.floors[0].actors[0].mole_contacts, SeatMask::ALL);
}
#[test]
fn protected_attack_states_cannot_be_overwritten_by_nearest_steering() {
    let (mut state, definitions) = staged(
        &[
            Kind::Dentist,
            Kind::DentistVariant,
            Kind::Boogieman,
            Kind::Chickens,
        ],
        8,
    );
    let heroes = [Some(hero(0, 2, 900, 400)), None, None, None];
    attach(&mut state, &heroes);
    let mut sink = Sink::new(heroes);
    for actor in &mut state.floors[0].actors {
        actor.set_state(10);
        ai::think(actor, &definitions, &heroes, &[], &mut sink).unwrap();
        assert_eq!(actor.state, 10);
    }
}
#[test]
fn snapshot_roundtrip_and_resimulation_keep_full_population_state() {
    let (mut state, definitions) = staged(&Kind::ALL, 64);
    let heroes = [Some(hero(0, 2, 900, 400)), None, None, None];
    attach(&mut state, &heroes);
    let mut sink = Sink::new(heroes);
    for tick in 0..7 {
        state
            .step(tick, &Ground::default(), &definitions, &[], &mut sink)
            .unwrap();
        state
            .advance_motion(tick, &Ground::default(), &definitions, &[], &mut sink)
            .unwrap();
        state
            .advance_animation(tick, &Ground::default(), &definitions, &[], &mut sink)
            .unwrap();
    }
    let config = bincode::config::standard()
        .with_fixed_int_encoding()
        .with_little_endian()
        .with_limit::<4194304>();
    let bytes = bincode::encode_to_vec(&state, config).unwrap();
    let (mut restored, consumed): (PopulationState, _) =
        bincode::decode_from_slice(&bytes, config).unwrap();
    assert_eq!(consumed, bytes.len());
    restored.validate().unwrap();
    assert_eq!(restored, state);
    let mut other_sink = Sink::new(sink.heroes);
    for tick in 7..20 {
        state
            .step(tick, &Ground::default(), &definitions, &[], &mut sink)
            .unwrap();
        restored
            .step(tick, &Ground::default(), &definitions, &[], &mut other_sink)
            .unwrap();
        state
            .advance_motion(tick, &Ground::default(), &definitions, &[], &mut sink)
            .unwrap();
        state
            .advance_animation(tick, &Ground::default(), &definitions, &[], &mut sink)
            .unwrap();
        restored
            .advance_motion(tick, &Ground::default(), &definitions, &[], &mut other_sink)
            .unwrap();
        restored
            .advance_animation(tick, &Ground::default(), &definitions, &[], &mut other_sink)
            .unwrap();
    }
    assert_eq!(state, restored);
}
#[test]
fn malformed_cross_references_and_generation_exhaustion_fail_closed() {
    let (mut state, _) = staged(&[Kind::Devil], 8);
    let heroes = [Some(hero(0, 2, 640, 400)), None, None, None];
    attach(&mut state, &heroes);
    let id = state.floors[0].actors[0].id;
    state.floors[0].actors[0].body = ActorId::new(1023, 1);
    assert!(state.validate().is_err());
    state.floors[0].actors[0].body = ActorId::new(32, u32::MAX);
    state.bodies[0].generation = u32::MAX;
    state.validate().unwrap();
    state.release(SeatId::ALL[0]).unwrap();
    let saved = state.clone();
    assert!(state.refresh_activity(&[None; 4]).is_err());
    assert_eq!(state, saved);
    assert!(state.remove(id, false).is_err());
    assert_eq!(state, saved);
}

fn synthetic_rom(revision: Revision, unknown_callback: bool) -> AddressSpace {
    let mut bytes = vec![0; revision.rom_size()];
    let definitions = tje_mem::rom::earthling_definitions(revision).address() as usize;
    let descriptor = tje_mem::rom_data_units(revision)
        .iter()
        .find(|unit| unit.name == "AnimationDescriptors_0AF0D8")
        .unwrap()
        .address;
    for kind in Kind::ALL {
        let address = definitions + kind.index() * 38;
        bytes[address..address + 4].copy_from_slice(&descriptor.to_be_bytes());
        let update = ids::EARTHLING_MOVE_ON_ALLOWED_TERRAIN
            .metadata()
            .address(revision);
        let thinker = ids::AI_GENERIC_ENEMY_THINK.metadata().address(revision);
        let touch = if unknown_callback {
            0xffff00
        } else {
            ids::PLAYER_YOUCH.metadata().address(revision)
        };
        for (offset, pointer) in [(4, update), (8, thinker), (12, touch)] {
            bytes[address + offset..address + offset + 4].copy_from_slice(&pointer.to_be_bytes());
        }
        bytes[address + 16] = 3;
        bytes[address + 17] = 20;
        bytes[address + 18] = 24;
        bytes[address + 21] = 50;
        bytes[address + 22] = 100;
        bytes[address + 23] = 50;
        bytes[address + 26] = 30;
        bytes[address + 27] = 20;
        bytes[address + 28] = 2;
        bytes[address + 36] = 3;
    }
    for state in 0..16 {
        let address = descriptor as usize + state * 10;
        bytes[address..address + 4].copy_from_slice(&0x1000_u32.to_be_bytes());
        bytes[address + 4..address + 8].copy_from_slice(&0x1100_u32.to_be_bytes());
        bytes[address + 8] = 6;
    }
    AddressSpace::new(bytes, revision)
}
#[test]
fn both_revisions_import_finite_callbacks_and_reject_unknown_addresses() {
    for revision in Revision::ALL {
        let memory = synthetic_rom(revision, false);
        let imported = rom::RomDefinitions::new(&memory).unwrap();
        for kind in Kind::ALL {
            assert_eq!(imported.catalog().get(kind).thinker, Thinker::Generic);
            assert_eq!(imported.descriptor(kind, 0).unwrap().period, 6);
        }
        assert!(imported.descriptor(Kind::Devil, 16).is_err());
        assert!(rom::RomDefinitions::new(&synthetic_rom(revision, true)).is_err());
        assert!(rom::RomDefinitions::new(&AddressSpace::new(vec![0; 100], revision)).is_err());
    }
}
#[test]
#[ignore = "requires private retail ROM paths; emits no asset data"]
fn private_retail_definition_catalogues_support_every_kind() {
    for (revision, variable) in [
        (Revision::Rev00, "TJE_ROM_REV00"),
        (Revision::Rev02, "TJE_ROM_REV02"),
    ] {
        let path = std::env::var(variable).expect("private ROM environment variable");
        let memory = AddressSpace::new(std::fs::read(path).unwrap(), revision);
        let imported = rom::RomDefinitions::new(&memory).unwrap();
        for kind in Kind::ALL {
            imported.catalog().get(kind).validate().unwrap();
            imported.descriptor(kind, 0).unwrap();
        }
    }
}

#[test]
fn motion_and_animation_have_distinct_saved_cadences_for_every_active_actor() {
    let (mut state, definitions) = staged(&[Kind::Devil; 2], 8);
    let heroes = [Some(hero(0, 2, 900, 600)), None, None, None];
    attach(&mut state, &heroes);
    state.floors[0].actors[1].motion.position.y += 30 * 256;
    for actor in &mut state.floors[0].actors {
        actor.motion.velocity[0] = 256;
        actor.set_state(7);
    }
    let mut sink = Sink::new(heroes);
    let old: Vec<_> = state.floors[0]
        .actors
        .iter()
        .map(|actor| actor.motion.position)
        .collect();
    state
        .advance_motion(1, &Ground::default(), &definitions, &[], &mut sink)
        .unwrap();
    assert_eq!(state.floors[0].actors[0].motion.position, old[0]);
    state
        .advance_animation(1, &Ground::default(), &definitions, &[], &mut sink)
        .unwrap();
    for (actor, old) in state.floors[0].actors.iter().zip(&old) {
        assert_eq!(actor.motion.position.x, old.x + 256);
    }
    let saved = state.clone();
    assert!(
        !state
            .advance_animation(1, &Ground::default(), &definitions, &[], &mut sink)
            .unwrap()
    );
    assert_eq!(state, saved);
    state
        .advance_animation(
            2,
            &Ground {
                emit_motion: false,
                ..Ground::default()
            },
            &definitions,
            &[],
            &mut sink,
        )
        .unwrap();
    for (actor, old) in state.floors[0].actors.iter().zip(&old) {
        assert_eq!(actor.motion.position.x, old.x + 256);
    }
}

#[test]
fn earthlings_collide_using_both_half_widths_without_draw_list_residency() {
    let (mut state, definitions) = staged(&[Kind::Devil; 2], 8);
    let heroes = [Some(hero(0, 2, 900, 400)), None, None, None];
    attach(&mut state, &heroes);
    state.floors[0].actors[0].motion.velocity[0] = 256;
    state.floors[0].actors[0].set_state(7);
    let before = state.floors[0].actors[0].motion.position;
    let mut sink = Sink::new(heroes);
    state
        .advance_animation(1, &Ground::default(), &definitions, &[], &mut sink)
        .unwrap();
    assert_eq!(state.floors[0].actors[0].motion.position, before);
    assert!(state.floors[0].actors[0].blocked);
}

#[test]
fn terrain_rules_restrict_sharks_and_lawnmowers_but_allow_ordinary_walkers() {
    for (kind, terrain, allowed) in [
        (Kind::Shark, 0x10, false),
        (Kind::Shark, 0x80, true),
        (Kind::Lawnmower, 0x30, false),
        (Kind::Lawnmower, 0x10, true),
        (Kind::Devil, 0x30, true),
    ] {
        let (mut state, definitions) = staged(&[kind], 8);
        let heroes = [Some(hero(0, 2, 900, 400)), None, None, None];
        attach(&mut state, &heroes);
        state.floors[0].actors[0].motion.velocity[0] = 256;
        state.floors[0].actors[0].set_state(7);
        let old = state.floors[0].actors[0].motion.position.x;
        state
            .advance_animation(
                1,
                &Ground {
                    terrain,
                    ..Ground::default()
                },
                &definitions,
                &[],
                &mut Sink::new(heroes),
            )
            .unwrap();
        assert_eq!(
            state.floors[0].actors[0].motion.position.x != old,
            allowed,
            "{kind:?}"
        );
    }
}

#[test]
fn santa_drop_failure_is_atomic_and_retreat_uses_hero_interest_not_cameras() {
    let (mut state, definitions) = staged(&[Kind::Santa], 8);
    let heroes = [Some(hero(0, 2, 650, 400)), None, None, None];
    attach(&mut state, &heroes);
    let mut sink = Sink::new(heroes);
    sink.reject = true;
    let before = state.floors[0].actors[0].clone();
    contacts::dispatch(
        &mut state.floors[0].actors[0],
        definitions.get(Kind::Santa),
        0,
        &mut sink,
    )
    .unwrap();
    assert_eq!(state.floors[0].actors[0], before);
    state.floors[0].actors[0].set_state(4);
    sink.heroes[0].as_mut().unwrap().position = position(2000, 1000);
    state
        .retreat_santas(&sink.heroes, &Ground::default(), &definitions)
        .unwrap();
    let santa = &state.floors[0].actors[0];
    assert_eq!(santa.state, 0);
    let target = sink.heroes[0].unwrap().position;
    assert!(
        (i64::from(santa.motion.position.x >> 8) - i64::from(target.x >> 8)).abs() > INTEREST_X
            || (i64::from(santa.motion.position.y >> 8) - i64::from(target.y >> 8)).abs()
                > INTEREST_Y
    );
}

#[test]
fn cupid_decoy_attack_and_chicken_launches_use_typed_kinematics_once_per_frame() {
    let (mut state, definitions) = staged(&[Kind::Cupid, Kind::Chickens], 8);
    let heroes = [Some(hero(0, 2, 900, 400)), None, None, None];
    attach(&mut state, &heroes);
    let decoy = DecoyView {
        actor: ActorId::new(352, 1).unwrap(),
        owner: SeatId::ALL[0],
        level: level(2),
        position: position(710, 400),
        width: 24,
        velocity: [0; 3],
    };
    contacts::decoy_contacts(
        &mut state.floors[0].actors[0],
        definitions.get(Kind::Cupid),
        &[decoy],
    );
    assert_eq!(state.floors[0].actors[0].state, 11);
    state.floors[0].actors[0].frame = 2;
    let mut sink = Sink::new(heroes);
    ai::think(
        &mut state.floors[0].actors[0],
        &definitions,
        &heroes,
        &[],
        &mut sink,
    )
    .unwrap();
    ai::think(
        &mut state.floors[0].actors[0],
        &definitions,
        &heroes,
        &[],
        &mut sink,
    )
    .unwrap();
    let remaps: Vec<_> = sink
        .requests
        .iter()
        .filter(|request| {
            matches!(
                request.effect,
                ContactEffect::Projectile { remap: true, .. }
            )
        })
        .collect();
    assert_eq!(remaps.len(), 1);
    if let ContactEffect::Projectile {
        velocity,
        acceleration,
        ..
    } = remaps[0].effect
    {
        assert_eq!(velocity, [896, 0, 0]);
        assert_eq!(acceleration, [0, 0, -32]);
    }
    let chicken = &mut state.floors[0].actors[1];
    chicken.set_state(11);
    chicken.frame = 2;
    chicken.phase_count = -1;
    ai::think(chicken, &definitions, &heroes, &[], &mut sink).unwrap();
    assert!(sink.requests.iter().any(|request| matches!(
        request.effect,
        ContactEffect::Projectile {
            remap: false,
            velocity: [_, _, 768],
            acceleration: [0, 0, -64..=-16],
            ..
        }
    )));
}

#[test]
fn four_maximum_density_floors_fit_negotiated_body_domain_and_codec_budget() {
    let definitions = catalog();
    let mut state = PopulationState::new(
        PopulationLimits {
            per_floor: 64,
            active_bodies: 320,
        },
        [9; 32],
    )
    .unwrap();
    for id in 2..6 {
        let actors: Vec<_> = (0..64)
            .map(|local| AuthoredActor {
                local,
                kind: Kind::Devil,
                position: position(
                    600 + i32::from(local % 8) * 30,
                    400 + i32::from(local / 8) * 20,
                ),
                mail_order: false,
            })
            .collect();
        state.stage_floor(level(id), &actors, &definitions).unwrap();
    }
    let heroes =
        SeatId::ALL.map(|seat| Some(hero(seat.index(), u16::from(seat.get()) + 2, 700, 450)));
    attach(&mut state, &heroes);
    assert_eq!(state.active_bodies().count(), 256);
    assert!(
        state
            .active_bodies()
            .all(|(body, _, _)| (32..352).contains(&body.slot()))
    );
    state.validate().unwrap();
    let encoded = bincode::encode_to_vec(
        &state,
        bincode::config::standard().with_fixed_int_encoding(),
    )
    .unwrap();
    assert!(encoded.len() < 256 * 1024);
}

#[test]
fn stock_special_prefix_uses_exact_seed_normalization_modes_and_level_clearance() {
    use stock::{StockSpecialRequest, prepare_stock_specials};
    use tje_world::{Anchors, Cell, CellPosition, LevelGeometry};
    for revision in Revision::ALL {
        // Newly authored uniform support, not a cartridge map/table golden.
        let mut bytes = synthetic_rom(revision, false).rom().to_vec();
        let base = tje_mem::rom::level_metatile_maps(revision).address() as usize + 30 * 256;
        for offset in (base..base + 256).step_by(2) {
            bytes[offset..offset + 2].copy_from_slice(&1_u16.to_be_bytes());
        }
        bytes[tje_mem::rom::player_tile_property_lookup(revision).address() as usize + 1] = 0x10;
        let memory = AddressSpace::new(bytes, revision);
        let resources = tje_world::rom::RomResources::new(&memory).unwrap();
        let dimensions = Dimensions::ORIGINAL;
        let geometry = LevelGeometry::new(
            dimensions,
            vec![Cell::new(30, 0).unwrap(); dimensions.area()],
            vec![0; dimensions.area()],
            Anchors {
                arrival: CellPosition { x: 0, y: 0 },
                exit: CellPosition { x: 18, y: 25 },
                ship_piece: None,
                regions: vec![],
            },
        )
        .unwrap();
        let request = StockSpecialRequest {
            level: level(2),
            level_seed: 0,
            ship_position: Some(position(2048, 1408)),
        };
        let zero = prepare_stock_specials(request, &geometry, &resources, &catalog()).unwrap();
        let one = prepare_stock_specials(
            StockSpecialRequest {
                level_seed: 1,
                ..request
            },
            &geometry,
            &resources,
            &catalog(),
        )
        .unwrap();
        assert_eq!(zero, one);
        assert_eq!(zero.failed, 0);
        assert!(zero.records[0].unwrap().mail_order);
        assert_eq!(zero.records[1].unwrap().kind, Kind::ShipPiece);
        assert!(zero.is_near(2048, 1408));
        let mailbox = prepare_stock_specials(
            StockSpecialRequest {
                level: level(4),
                ship_position: None,
                ..request
            },
            &geometry,
            &resources,
            &catalog(),
        )
        .unwrap();
        assert!(!mailbox.records[0].unwrap().mail_order);
        let mailbox = prepare_stock_specials(
            StockSpecialRequest {
                level: level(12),
                ship_position: None,
                ..request
            },
            &geometry,
            &resources,
            &catalog(),
        )
        .unwrap();
        assert!(mailbox.records[0].unwrap().mail_order);
        for id in [0, 1, 26] {
            let special = prepare_stock_specials(
                StockSpecialRequest {
                    level: level(id),
                    ship_position: None,
                    ..request
                },
                &geometry,
                &resources,
                &catalog(),
            )
            .unwrap();
            assert_eq!(special.records, [None; 2]);
        }
    }
}

#[test]
fn historical_effect_sources_do_not_resolve_as_current_service_bodies() {
    let (mut state, _) = staged(&[Kind::Wizard], 8);
    assert!(!state.known_body(ActorId::new(32, 1).unwrap()));
    let heroes = [Some(hero(0, 2, 650, 400)), None, None, None];
    attach(&mut state, &heroes);
    let body = state.floors[0].actors[0].body.unwrap();
    assert!(state.known_body(body));
    assert_eq!(state.service(body).unwrap().1, Service::Heal);
    state.release(SeatId::ALL[0]).unwrap();
    state.refresh_activity(&[None; 4]).unwrap();
    assert!(state.known_body(body));
    assert!(state.resolve(body).is_none());
    assert!(state.service(body).is_none());
    assert!(!state.known_body(ActorId::new(32, 2).unwrap()));
}

#[test]
fn stock_population_preparation_preserves_holes_and_publishes_atomically() {
    let definitions = catalog();
    let mut state = PopulationState::new(PopulationLimits::default(), [7; 32]).unwrap();
    let mut slots = [None; 20];
    slots[1] = Some(Kind::Devil);
    slots[7] = Some(Kind::Dentist);
    slots[19] = Some(Kind::HulaDancer);
    let required = [AuthoredActor {
        local: 0,
        kind: Kind::ShipPiece,
        position: position(1800, 1000),
        mail_order: false,
    }];
    let before = state.clone();
    let mut floor = state
        .prepare_stock_floor(
            level(2),
            &slots,
            &required,
            &Ground::default(),
            &definitions,
        )
        .unwrap();
    assert_eq!(state, before);
    assert_eq!(
        floor
            .actors
            .iter()
            .map(|actor| actor.id.local)
            .collect::<Vec<_>>(),
        [0, 1, 7, 19]
    );
    assert_eq!(floor.declared_slots, (1 << 20) - 1);
    floor.removed.insert(18).unwrap();
    floor.removed.insert(0).unwrap();
    floor.collected.insert(0).unwrap();
    state.preflight_prepared_floor(&floor).unwrap();
    let duplicate = floor.clone();
    state.install_prepared_floor(floor).unwrap();
    assert!(
        state
            .actor(PopulationId {
                level: level(2),
                local: 0
            })
            .unwrap()
            .retired
    );
    let installed = state.clone();
    assert!(state.install_prepared_floor(duplicate).is_err());
    assert_eq!(state, installed);
    state.validate().unwrap();
    let view = hero(3, 2, 900, 500);
    attach(&mut state, &[None, None, None, Some(view)]);
    let spawned = state
        .spawn_present(
            view.seat,
            view.level,
            view.position,
            17,
            &Ground::default(),
            &definitions,
        )
        .unwrap();
    assert_eq!(spawned.local, 20);
    state.validate().unwrap();
}

#[test]
fn prepared_population_failure_flags_are_bounded_to_declared_sources() {
    let definitions = catalog();
    let mut state = PopulationState::new(PopulationLimits::default(), [3; 32]).unwrap();
    let mut slots = [None; 20];
    slots[19] = Some(Kind::Devil);
    let unavailable = Ground {
        no_placement: true,
        ..Ground::default()
    };
    let mut floor = state
        .prepare_stock_floor(level(2), &slots, &[], &unavailable, &definitions)
        .unwrap();
    assert!(floor.failed.contains(19));
    floor.failed.insert(7).unwrap();
    state.preflight_prepared_floor(&floor).unwrap();
    floor.failed.insert(40).unwrap();
    assert!(state.preflight_prepared_floor(&floor).is_err());
    assert!(state.install_prepared_floor(floor).is_err());
    assert!(state.floors().is_empty());
    slots[19] = Some(Kind::ShipPiece);
    assert!(
        state
            .prepare_stock_floor(level(2), &slots, &[], &unavailable, &definitions)
            .is_err()
    );
    assert!(state.floors().is_empty());
}

#[test]
fn overlapping_music_leases_wake_only_their_own_source_generation() {
    let (mut state, _) = staged(&[Kind::Devil], 8);
    let views = [Some(hero(0, 2, 650, 400)), None, None, None];
    attach(&mut state, &views);
    let first = crate::EffectId::new(0, 1).unwrap();
    let second = crate::EffectId::new(1, 1).unwrap();
    assert_eq!(
        state
            .sleep_near(first, level(2), position(640, 400))
            .unwrap(),
        1
    );
    assert_eq!(
        state
            .sleep_near(first, level(2), position(640, 400))
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .sleep_near(second, level(2), position(640, 400))
            .unwrap(),
        1
    );
    assert_eq!(state.sleep_sources().count(), 2);
    state.retire_effect(first, level(2)).unwrap();
    assert_eq!(state.floors[0].actors[0].state, SLEEP_STATE);
    let stale = crate::EffectId::new(1, 2).unwrap();
    assert_eq!(state.wake_level(stale, level(2)).unwrap(), 0);
    assert_eq!(state.floors[0].actors[0].state, SLEEP_STATE);
    state.release(SeatId::ALL[0]).unwrap();
    state.refresh_activity(&[None; 4]).unwrap();
    state.retire_effect(second, level(2)).unwrap();
    assert_eq!(state.floors[0].actors[0].state, 0);
    state.validate().unwrap();
}

#[test]
fn rose_damage_is_generation_scoped_and_rearms_only_on_exit() {
    let (mut state, _) = staged(&[Kind::Devil], 8);
    attach(&mut state, &[Some(hero(0, 2, 650, 400)), None, None, None]);
    let source = crate::EffectId::new(0, 1).unwrap();
    assert_eq!(
        state
            .rose_contacts(source, level(2), position(640, 400))
            .unwrap(),
        1
    );
    assert_eq!(state.floors[0].actors[0].health, 2);
    assert_eq!(
        state
            .rose_contacts(source, level(2), position(640, 400))
            .unwrap(),
        0
    );
    state.floors[0].actors[0].motion.position.x += 30 * 256;
    state
        .rose_contacts(source, level(2), position(640, 400))
        .unwrap();
    state.floors[0].actors[0].motion.position.x -= 30 * 256;
    state
        .rose_contacts(source, level(2), position(640, 400))
        .unwrap();
    assert_eq!(state.floors[0].actors[0].health, 1);
    state.retire_effect(source, level(2)).unwrap();
    assert_eq!(state.thorn_sources().count(), 0);
    state.validate().unwrap();
}

#[test]
fn projectile_impacts_are_idempotent_and_immune_bodies_still_consume() {
    let (mut state, definitions) = staged(&[Kind::Devil, Kind::Santa], 8);
    attach(&mut state, &[Some(hero(0, 2, 700, 400)), None, None, None]);
    let source = crate::EffectId::new(0, 1).unwrap();
    let mut point = position(650, 400);
    point.elevation = 10 * 256;
    assert!(
        state
            .projectile_contact(source, None, level(2), point, 1, &definitions)
            .unwrap()
    );
    assert_eq!(state.floors[0].actors[1].health, 3);
    point.x = 640 * 256;
    assert!(
        state
            .projectile_contact(source, None, level(2), point, 1, &definitions)
            .unwrap()
    );
    assert_eq!(state.floors[0].actors[0].health, 3);
    let next = crate::EffectId::new(0, 2).unwrap();
    assert!(
        state
            .projectile_contact(next, None, level(2), point, 1, &definitions)
            .unwrap()
    );
    assert_eq!(state.floors[0].actors[0].health, 2);
    state.validate().unwrap();
}

#[test]
fn rejected_actor_generation_release_does_not_partially_damage_or_receipt() {
    let (mut state, definitions) = staged(&[Kind::Devil], 8);
    attach(&mut state, &[Some(hero(0, 2, 700, 400)), None, None, None]);
    state.bodies[0].generation = u32::MAX;
    state.floors[0].actors[0].body = ActorId::new(32, u32::MAX);
    let before = state.clone();
    let mut point = position(640, 400);
    point.elevation = 10 * 256;
    assert!(
        state
            .projectile_contact(
                crate::EffectId::new(0, 1).unwrap(),
                None,
                level(2),
                point,
                3,
                &definitions
            )
            .is_err()
    );
    assert_eq!(state, before);
}

#[test]
fn active_opera_contacts_publish_each_performance_tick_and_completion() {
    let (mut state, definitions) = staged(&[Kind::OperaSinger], 8);
    let views = [None, None, None, Some(hero(3, 2, 650, 400))];
    attach(&mut state, &views);
    let mut sink = Sink::new(views);
    let actor = &mut state.floors[0].actors[0];
    actor.set_state(3);
    actor.service_owner = Some(SeatId::ALL[3]);
    actor.interactions[3] = Interaction::Active;
    actor.animation_finished = false;
    contacts::dispatch(actor, definitions.get(Kind::OperaSinger), 0, &mut sink).unwrap();
    assert!(
        sink.requests
            .iter()
            .any(|request| !request.animation_finished
                && matches!(
                    request.effect,
                    ContactEffect::Service {
                        service: Service::Opera,
                        stage: ServiceStage::Tick,
                        ..
                    }
                ))
    );
    actor.animation_finished = true;
    contacts::dispatch(actor, definitions.get(Kind::OperaSinger), 0, &mut sink).unwrap();
    assert!(
        sink.requests
            .iter()
            .any(|request| request.animation_finished
                && matches!(
                    request.effect,
                    ContactEffect::Service {
                        service: Service::Opera,
                        stage: ServiceStage::Tick,
                        ..
                    }
                ))
    );
}

#[test]
fn full_stock_twenty_keeps_initial_count_but_reserves_room_for_real_present_spawn() {
    let definitions = catalog();
    let mut state = PopulationState::new(
        PopulationLimits {
            per_floor: 64,
            active_bodies: 316,
        },
        [5; 32],
    )
    .unwrap();
    let slots = [Some(Kind::Devil); 20];
    let floor = state
        .prepare_stock_floor(level(2), &slots, &[], &Ground::default(), &definitions)
        .unwrap();
    assert_eq!(floor.actors.len(), 20);
    assert_eq!(floor.declared_slots, (1_u64 << 20) - 1);
    state.install_prepared_floor(floor).unwrap();
    let view = hero(0, 2, 900, 500);
    attach(&mut state, &[Some(view), None, None, None]);
    let id = state
        .spawn_present(
            view.seat,
            view.level,
            view.position,
            0,
            &Ground::default(),
            &definitions,
        )
        .unwrap();
    assert_eq!(id.local, 20);
    assert_eq!(state.floors()[0].actors.len(), 21);
    assert_eq!(state.actor(id).unwrap().owner, Some(view.seat));
    assert!(
        state.floors()[0].actors[..20]
            .iter()
            .all(|actor| actor.owner.is_none())
    );
    while state.floors()[0].declared_slots != u64::MAX {
        state
            .spawn_present(
                view.seat,
                view.level,
                view.position,
                0,
                &Ground::default(),
                &definitions,
            )
            .unwrap();
    }
    let exhausted = state.clone();
    assert!(
        state
            .spawn_present(
                view.seat,
                view.level,
                view.position,
                0,
                &Ground::default(),
                &definitions
            )
            .is_err()
    );
    assert_eq!(state, exhausted);
    state.validate().unwrap();
}

#[test]
fn cohort_retention_reserves_the_complete_final_floor_set_before_commit() {
    let definitions = catalog();
    let mut state = PopulationState::new(
        PopulationLimits {
            per_floor: 64,
            active_bodies: 3,
        },
        [5; 32],
    )
    .unwrap();
    for floor in 2..6 {
        state
            .stage_floor(
                level(floor),
                &[AuthoredActor {
                    local: 0,
                    kind: Kind::Devil,
                    position: position(640, 400),
                    mail_order: false,
                }],
                &definitions,
            )
            .unwrap();
    }
    for seat in SeatId::ALL {
        state
            .preflight_retain(seat, level(u16::from(seat.get()) + 2))
            .unwrap();
    }
    let before = state.clone();
    let together = SeatId::ALL.map(|seat| Some(level(u16::from(seat.get()) + 2)));
    assert!(state.prepare_retains(together).is_err());
    assert_eq!(state, before);
    let shared = [Some(level(2)); 4];
    let prepared = state.prepare_retains(shared).unwrap();
    state.commit_retains(prepared);
    assert_eq!(state.retained(), &shared);
}

#[test]
fn missing_or_released_customer_cannot_permanently_lock_a_friendly_actor() {
    let (mut state, definitions) = staged(&[Kind::Wizard], 8);
    let views = [None, None, None, Some(hero(3, 2, 650, 400))];
    attach(&mut state, &views);
    let actor = &mut state.floors[0].actors[0];
    actor.service_owner = Some(SeatId::ALL[3]);
    actor.interactions[3] = Interaction::Active;
    actor.set_state(2);
    let mut sink = Sink::new([None; 4]);
    contacts::dispatch(actor, definitions.get(Kind::Wizard), 0, &mut sink).unwrap();
    assert_eq!(actor.service_owner, None);
    assert_eq!(actor.interactions[3], Interaction::Idle);
    assert!(sink.requests.iter().any(|request| matches!(
        request.effect,
        ContactEffect::Service {
            stage: ServiceStage::End,
            ..
        }
    )));
    actor.service_owner = Some(SeatId::ALL[3]);
    actor.interactions[3] = Interaction::Active;
    state.release_seat_contacts(SeatId::ALL[3]).unwrap();
    assert_eq!(state.floors[0].actors[0].service_owner, None);
}

#[test]
fn population_preparation_yields_one_candidate_per_unit_and_restores_every_cursor() {
    struct Counted {
        ground: Ground,
        candidates: std::cell::Cell<u32>,
    }
    impl Environment for Counted {
        fn dimensions(&self, level: LevelId) -> Result<Dimensions> {
            self.ground.dimensions(level)
        }
        fn terrain(&self, level: LevelId, x: i32, y: i32) -> Result<u8> {
            self.ground.terrain(level, x, y)
        }
        fn base_metatile(&self, level: LevelId, x: i32, y: i32) -> Result<Option<u8>> {
            self.ground.base_metatile(level, x, y)
        }
        fn placement(
            &self,
            level: LevelId,
            kind: Kind,
            column: u16,
            row: u16,
            rng: &mut StageRng,
        ) -> Result<Option<(i32, i32)>> {
            self.candidates.set(self.candidates.get() + 1);
            self.ground.placement(level, kind, column, row, rng)
        }
        fn allows_placement(&self, level: LevelId, x: i32, y: i32) -> Result<bool> {
            self.ground.allows_placement(level, x, y)
        }
        fn animation_step(
            &self,
            _: Kind,
            _: u8,
            _: &mut AnimationCursor,
        ) -> Result<AnimationSample> {
            panic!("population preparation must never run animation")
        }
    }
    let definitions = catalog();
    let state = PopulationState::new(PopulationLimits::default(), [4; 32]).unwrap();
    let slots = [Some(Kind::Devil); 20];
    let environment = Counted {
        ground: Ground {
            no_placement: true,
            ..Ground::default()
        },
        candidates: std::cell::Cell::new(0),
    };
    let mut job = state
        .prepare_stock_job(level(2), &slots, &[], Dimensions::ORIGINAL)
        .unwrap();
    assert!(job.clone().finish().is_err());
    let config = bincode::config::standard()
        .with_fixed_int_encoding()
        .with_limit::<4194304>();
    let mut units = 0;
    while job.progress() != StockPopulationProgress::Complete {
        let before = environment.candidates.get();
        job.advance(&environment, &definitions, 1).unwrap();
        assert_eq!(environment.candidates.get() - before, 1);
        units += 1;
        if units % 37 == 0 {
            let bytes = bincode::encode_to_vec(&job, config).unwrap();
            let (restored, used): (StockPopulationJob, usize) =
                bincode::decode_from_slice(&bytes, config).unwrap();
            assert_eq!(used, bytes.len());
            assert_eq!(restored, job);
            state.validate_preparation_job(&restored).unwrap();
            job = restored;
        }
    }
    assert_eq!(units, 20 * (10 + Dimensions::ORIGINAL.area()));
    let expected = job.finish().unwrap();
    assert_eq!(expected.actors.len(), 20);
    assert!((0..20).all(|id| expected.failed.contains(id)));
    for quantum in [7, 64, 1024] {
        let mut job = state
            .prepare_stock_job(level(2), &slots, &[], Dimensions::ORIGINAL)
            .unwrap();
        while job.advance(&environment, &definitions, quantum).unwrap()
            != StockPopulationProgress::Complete
        {}
        assert_eq!(job.finish().unwrap(), expected);
    }
    assert!(state.floors().is_empty());
}

#[test]
fn preparation_continuation_rejects_bad_work_identity_and_retains_sparse_order() {
    let definitions = catalog();
    let state = PopulationState::new(PopulationLimits::default(), [4; 32]).unwrap();
    let mut slots = [None; 20];
    slots[3] = Some(Kind::Devil);
    slots[19] = Some(Kind::Dentist);
    let required = [AuthoredActor {
        local: 0,
        kind: Kind::ShipPiece,
        position: position(1800, 1000),
        mail_order: false,
    }];
    let mut job = state
        .prepare_stock_job(level(2), &slots, &required, Dimensions::ORIGINAL)
        .unwrap();
    let before = job.clone();
    assert!(job.advance(&Ground::default(), &definitions, 0).is_err());
    assert_eq!(job, before);
    let other = PopulationState::new(PopulationLimits::default(), [5; 32]).unwrap();
    assert!(other.validate_preparation_job(&job).is_err());
    while job.advance(&Ground::default(), &definitions, 1).unwrap()
        != StockPopulationProgress::Complete
    {}
    let floor = job.finish().unwrap();
    assert_eq!(
        floor
            .actors
            .iter()
            .map(|actor| actor.id.local)
            .collect::<Vec<_>>(),
        [0, 3, 19]
    );
    assert_eq!(floor.declared_slots, (1_u64 << 20) - 1);
}

#[test]
fn reserved_state_three_requires_a_real_typed_producer() {
    let base = base_definition();
    assert!(!base.supports_state(Kind::HulaDancer, 3));
    assert!(!base.supports_state(Kind::Dentist, 3));
    assert!(base.supports_state(Kind::Whirlwind, 3)); // Explicit initial state.
    for thinker in [
        Thinker::HorizontalTurn,
        Thinker::HorizontalReversal,
        Thinker::Whirlwind,
        Thinker::Santa,
    ] {
        assert!(Definition { thinker, ..base }.supports_state(Kind::Devil, 3));
    }
    for touch in [
        Touch::Cupid,
        Touch::Mole,
        Touch::Mailbox,
        Touch::Wizard,
        Touch::Opera,
        Touch::Santa,
    ] {
        assert!(Definition { touch, ..base }.supports_state(Kind::Devil, 3));
    }
    for state in [0, 1, 4, 5, 6, 7] {
        assert!(base.supports_state(Kind::HulaDancer, state));
    }
    assert!(!base.supports_state(Kind::HulaDancer, 2));
    assert!(
        Definition {
            companion: true,
            ..base
        }
        .supports_state(Kind::HulaDancer, 2)
    );
    assert!(!base.supports_state(Kind::Devil, 8));
    assert!(base.supports_state(Kind::Dentist, 8));
    assert!(base.supports_state(Kind::DentistVariant, 11));
    assert!(!base.supports_state(Kind::IceCreamTruck, 15));
    assert!(
        Definition {
            thinker: Thinker::Timed,
            ..base
        }
        .supports_state(Kind::IceCreamTruck, 15)
    );
    assert!(base.supports_state(Kind::HulaDancer, SLEEP_STATE));
    assert!(!base.supports_state(Kind::ShipPiece, SLEEP_STATE));
    assert!(!base.supports_state(Kind::HulaDancer, 16));
    for revision in Revision::ALL {
        let memory = synthetic_rom(revision, false);
        let imported = rom::RomDefinitions::new(&memory).unwrap();
        assert!(imported.descriptor(Kind::HulaDancer, 3).is_err());
        assert!(imported.optional_descriptor(Kind::HulaDancer, 3).is_err());
    }
}

#[test]
fn restored_population_floor_reserves_the_lifetime_arena_without_changing_wire_bytes() {
    let state = crate::session_tests::session(123);
    let original = &state.population.floors()[0];
    assert!(original.actors.len() < usize::from(MAX_POPULATION));
    let config = bincode::config::standard()
        .with_fixed_int_encoding()
        .with_little_endian();
    let bytes = bincode::encode_to_vec(original, config).unwrap();
    let (restored, consumed): (PopulationFloor, _) =
        bincode::decode_from_slice(&bytes, config).unwrap();
    assert_eq!(consumed, bytes.len());
    assert_eq!(&restored, original);
    assert_eq!(restored.actors.capacity(), usize::from(MAX_POPULATION));
    assert_eq!(bincode::encode_to_vec(&restored, config).unwrap(), bytes);
}

#[test]
fn population_floor_decoder_rejects_an_oversized_arena_before_actor_decode() {
    let config = bincode::config::standard()
        .with_fixed_int_encoding()
        .with_little_endian();
    for length in [usize::from(MAX_POPULATION) + 1, usize::MAX] {
        let prefix = bincode::encode_to_vec((level(2), length), config).unwrap();
        let error = bincode::decode_from_slice::<PopulationFloor, _>(&prefix, config).unwrap_err();
        assert!(matches!(
            error,
            bincode::error::DecodeError::Other("persistent actor floor capacity")
        ));
    }
}

#[test]
fn restored_population_reserves_both_contact_ledgers_and_floor_insertions() {
    let state = crate::session_tests::session(123);
    let original = &state.population;
    let config = bincode::config::standard()
        .with_fixed_int_encoding()
        .with_little_endian();
    let bytes = bincode::encode_to_vec(original, config).unwrap();
    let (restored, consumed): (PopulationState, _) =
        bincode::decode_from_slice(&bytes, config).unwrap();
    assert_eq!(consumed, bytes.len());
    assert_eq!(&restored, original);
    assert_eq!(restored.floors.capacity(), 32);
    assert_eq!(restored.sleep_contacts.capacity(), effects::MAX_CONTACTS);
    assert_eq!(restored.thorn_contacts.capacity(), effects::MAX_CONTACTS);
    assert_eq!(bincode::encode_to_vec(&restored, config).unwrap(), bytes);
}
