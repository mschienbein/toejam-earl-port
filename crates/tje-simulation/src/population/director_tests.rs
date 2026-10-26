use super::*;
fn request(source: u32, position: Position) -> SpawnRequest {
    SpawnRequest {
        roles: roles(),
        owner: SeatId::ALL[3],
        source,
        level: level(),
        kind: Kind::Devil,
        position,
    }
}

fn level() -> LevelId {
    LevelId::new(2).unwrap()
}
fn position(x: i32, y: i32) -> Position {
    Position {
        x: x * 256,
        y: y * 256,
        elevation: 0,
    }
}
fn roles() -> crate::session::Roles {
    crate::session::Roles {
        inputs: SeatMask::ALL,
        heroes: SeatMask::new(7).unwrap(),
        local_owners: SeatMask::ALL,
        director: Some(SeatId::ALL[3]),
    }
}
fn definition() -> Definition {
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
            minimum: 2,
            maximum: 4,
        },
        state_delay: 0,
        health: 3,
    }
}
fn catalog() -> DefinitionCatalog {
    DefinitionCatalog::new(
        [definition(); 25],
        ProjectileVectors {
            thresholds: [8, 16, 24, 32, 40, 48, 56, 64],
            velocities: [[256, 256]; 9],
        },
    )
    .unwrap()
}
struct Ground;
impl Environment for Ground {
    fn dimensions(&self, _: LevelId) -> Result<Dimensions> {
        Ok(Dimensions::ORIGINAL)
    }
    fn terrain(&self, _: LevelId, _: i32, _: i32) -> Result<u8> {
        Ok(0x10)
    }
    fn base_metatile(&self, _: LevelId, _: i32, _: i32) -> Result<Option<u8>> {
        Ok(Some(30))
    }
    fn placement(
        &self,
        _: LevelId,
        _: Kind,
        x: u16,
        y: u16,
        _: &mut StageRng,
    ) -> Result<Option<(i32, i32)>> {
        Ok(Some((
            (i32::from(x) + 1) * 128 + 64,
            (i32::from(y) + 1) * 64 + 32,
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
        cursor.started = true;
        cursor.sequence_index = (cursor.sequence_index + 1) % 4;
        Ok(AnimationSample {
            frame: cursor.sequence_index,
            motion: Some([1, 0]),
            finished: cursor.sequence_index == 0,
        })
    }
}
struct Contacts {
    hero: HeroView,
    count: u32,
    last: Option<ContactEffect>,
}
impl ContactSink for Contacts {
    fn hero(&self, seat: SeatId) -> Option<HeroView> {
        (seat == self.hero.seat).then_some(self.hero)
    }
    fn apply(&mut self, request: ContactRequest) -> Result<ContactOutcome> {
        self.count += 1;
        self.last = Some(request.effect);
        Ok(ContactOutcome::Rejected)
    }
}
fn contacts() -> Contacts {
    Contacts {
        hero: HeroView {
            seat: SeatId::ALL[0],
            level: level(),
            position: position(768, 512),
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
        },
        count: 0,
        last: None,
    }
}
fn population(kind: Kind) -> (PopulationState, ActorId) {
    let mut state = PopulationState::new(
        PopulationLimits {
            per_floor: 64,
            active_bodies: 64,
        },
        [19; 32],
    )
    .unwrap();
    state
        .stage_floor(
            level(),
            &[
                AuthoredActor {
                    local: 0,
                    kind,
                    position: position(864, 512),
                    mail_order: false,
                },
                AuthoredActor {
                    local: 1,
                    kind: Kind::Devil,
                    position: position(800, 600),
                    mail_order: false,
                },
            ],
            &catalog(),
        )
        .unwrap();
    state.retain(SeatId::ALL[0], level()).unwrap();
    let sink = contacts();
    state
        .refresh_activity(&[Some(sink.hero), None, None, None])
        .unwrap();
    let body = state
        .actor(PopulationId {
            level: level(),
            local: 0,
        })
        .unwrap()
        .body
        .unwrap();
    (state, body)
}
#[test]
fn stock_step_and_controlled_none_are_exactly_identical() {
    let (mut stock, _) = population(Kind::Dentist);
    let mut controlled = stock.clone();
    let mut a = contacts();
    let mut b = contacts();
    for tick in 0..100 {
        let old = stock.step(tick, &Ground, &catalog(), &[], &mut a).unwrap();
        let new = controlled
            .step_controlled(tick, &Ground, &catalog(), &[], &mut b, None)
            .unwrap();
        assert_eq!(old, new.advanced);
        assert_eq!(stock, controlled);
        assert_eq!(a.count, b.count);
        stock
            .advance_motion(tick, &Ground, &catalog(), &[], &mut a)
            .unwrap();
        controlled
            .advance_motion(tick, &Ground, &catalog(), &[], &mut b)
            .unwrap();
        stock
            .advance_animation(tick, &Ground, &catalog(), &[], &mut a)
            .unwrap();
        controlled
            .advance_animation(tick, &Ground, &catalog(), &[], &mut b)
            .unwrap();
    }
}
#[test]
fn every_supported_kind_obeys_decision_slots_and_other_ai_is_unchanged() {
    for kind in [
        Kind::Devil,
        Kind::Dentist,
        Kind::DentistVariant,
        Kind::Lawnmower,
        Kind::NerdHerd,
    ] {
        for direction in [
            Direction::Neutral,
            Direction::Up,
            Direction::Down,
            Direction::Left,
            Direction::Right,
        ] {
            let (mut state, body) = population(kind);
            let mut baseline = state.clone();
            let mut sink = contacts();
            let mut other = contacts();
            let id = state.resolve(body).unwrap().id;
            state.actor_mut(id).unwrap().timer = 5;
            baseline.actor_mut(id).unwrap().timer = 5;
            let intent = Some(DecisionIntent {
                actor: body,
                direction,
                ability: false,
            });
            assert!(
                !state
                    .step_controlled(0, &Ground, &catalog(), &[], &mut sink, intent)
                    .unwrap()
                    .decision
                    .decided
            );
            baseline
                .step(0, &Ground, &catalog(), &[], &mut other)
                .unwrap();
            assert_eq!(state, baseline);
            state.actor_mut(id).unwrap().timer = 0;
            let result = state
                .step_controlled(1, &Ground, &catalog(), &[], &mut sink, intent)
                .unwrap();
            assert!(!result.decision.decided); // opposite stable-record parity
            let result = state
                .step_controlled(2, &Ground, &catalog(), &[], &mut sink, intent)
                .unwrap();
            baseline
                .step(1, &Ground, &catalog(), &[], &mut other)
                .unwrap();
            baseline
                .step(2, &Ground, &catalog(), &[], &mut other)
                .unwrap();
            let other_id = PopulationId {
                level: level(),
                local: 1,
            };
            assert_eq!(state.actor(other_id), baseline.actor(other_id));
            assert!(result.decision.decided);
            let actor = state.resolve(body).unwrap();
            assert_eq!(actor.health, 3);
            assert_eq!(actor.motion.position, position(864, 512));
            match direction {
                Direction::Up => assert!(actor.motion.velocity[1] < 0),
                Direction::Down => assert!(actor.motion.velocity[1] > 0),
                Direction::Left => assert!(actor.motion.velocity[0] < 0),
                Direction::Right => assert!(actor.motion.velocity[0] > 0),
                Direction::Neutral => assert_eq!(actor.motion.velocity, [0; 3]),
            }
            let before = state.clone();
            assert!(
                !state
                    .step_controlled(2, &Ground, &catalog(), &[], &mut sink, intent)
                    .unwrap()
                    .advanced
            );
            assert_eq!(state, before);
            // Release performs no immediate second decision and no timer reset.
            let timer = state.resolve(body).unwrap().timer;
            state.step(3, &Ground, &catalog(), &[], &mut sink).unwrap();
            assert_eq!(state.resolve(body).unwrap().timer, timer);
        }
    }
}
#[test]
fn dentist_ability_preserves_protected_animation_sleep_and_cooldown() {
    for kind in [Kind::Dentist, Kind::DentistVariant] {
        let (mut state, body) = population(kind);
        let id = state.resolve(body).unwrap().id;
        let mut sink = contacts();
        let intent = Some(DecisionIntent {
            actor: body,
            direction: Direction::Left,
            ability: true,
        });
        let receipt = state
            .step_controlled(0, &Ground, &catalog(), &[], &mut sink, intent)
            .unwrap();
        assert!(receipt.decision.ability_used);
        assert_eq!(state.resolve(body).unwrap().state, 10);
        assert_eq!(state.resolve(body).unwrap().motion.velocity, [0; 3]);
        let before = state.resolve(body).unwrap().clone();
        state
            .step_controlled(1, &Ground, &catalog(), &[], &mut sink, intent)
            .unwrap();
        state
            .step_controlled(2, &Ground, &catalog(), &[], &mut sink, intent)
            .unwrap();
        assert_eq!(*state.resolve(body).unwrap(), before); // protected uncompleted attack
        state.actor_mut(id).unwrap().set_state(SLEEP_STATE);
        let before = state.resolve(body).unwrap().clone();
        state
            .step_controlled(3, &Ground, &catalog(), &[], &mut sink, intent)
            .unwrap();
        state
            .step_controlled(4, &Ground, &catalog(), &[], &mut sink, intent)
            .unwrap();
        assert_eq!(*state.resolve(body).unwrap(), before);
    }
}
#[test]
fn stale_generation_unsupported_kind_and_foreign_ticket_reject_before_mutation() {
    let (mut state, body) = population(Kind::Devil);
    let mut sink = contacts();
    let before = state.clone();
    let stale = ActorId::new(body.slot(), body.generation() + 1).unwrap();
    assert!(
        state
            .step_controlled(
                0,
                &Ground,
                &catalog(),
                &[],
                &mut sink,
                Some(DecisionIntent {
                    actor: stale,
                    direction: Direction::Left,
                    ability: false
                })
            )
            .is_err()
    );
    assert_eq!(state, before);
    let (mut state, body) = population(Kind::Santa);
    let before = state.clone();
    assert!(
        state
            .step_controlled(
                0,
                &Ground,
                &catalog(),
                &[],
                &mut sink,
                Some(DecisionIntent {
                    actor: body,
                    direction: Direction::Left,
                    ability: false
                })
            )
            .is_err()
    );
    assert_eq!(state, before);
}
#[test]
fn director_reservation_is_real_bounded_inert_and_never_reuses_stock_identity() {
    let (mut state, _) = population(Kind::Devil);
    let stock = state.floors[0].declared_slots;
    let mut tickets = vec![];
    for source in 1..=6 {
        tickets.push(
            state
                .reserve_director(
                    request(source, position(1120 + source as i32 * 8, 512)),
                    &Ground,
                    &catalog(),
                )
                .unwrap(),
        );
    }
    assert_eq!(tickets.len(), 6);
    assert!(tickets.iter().all(|ticket| ticket.id.local >= 2));
    assert_eq!(state.floors[0].declared_slots & stock, stock);
    let before = state.clone();
    assert!(
        state
            .reserve_director(request(7, position(1200, 512)), &Ground, &catalog())
            .is_err()
    );
    assert_eq!(state, before);
    let saved: Vec<_> = tickets
        .iter()
        .map(|ticket| state.actor(ticket.id).unwrap().clone())
        .collect();
    let mut sink = contacts();
    for tick in 0..8 {
        state
            .step(tick, &Ground, &catalog(), &[], &mut sink)
            .unwrap();
        state
            .advance_motion(tick, &Ground, &catalog(), &[], &mut sink)
            .unwrap();
        state
            .advance_animation(tick, &Ground, &catalog(), &[], &mut sink)
            .unwrap();
    }
    for (ticket, saved) in tickets.iter().zip(saved) {
        assert_eq!(*state.actor(ticket.id).unwrap(), saved);
    }
    let forged = SpawnTicket {
        owner: SeatId::ALL[0],
        ..tickets[0]
    };
    let before = state.clone();
    assert!(state.cancel_director(forged).is_err());
    assert_eq!(state, before);
    state.cancel_director(tickets[0]).unwrap();
    assert!(state.actor(tickets[0].id).unwrap().retired);
    let next = state
        .reserve_director(request(7, position(1200, 512)), &Ground, &catalog())
        .unwrap();
    assert!(next.id.local > tickets[5].id.local);
    assert_ne!(next.id, tickets[0].id);
    state.activate_director(next).unwrap();
    assert!(!state.actor(next.id).unwrap().hidden);
    assert!(!state.activate_director(next).unwrap());
    let before = state.clone();
    assert!(state.cancel_director(next).is_err());
    assert_eq!(state, before);
    state.validate().unwrap();
}
#[test]
fn lifetime_record_exhaustion_cannot_erase_original_holes_or_masks() {
    let (mut state, _) = population(Kind::Devil);
    state.floors[0].declared_slots = u64::MAX;
    let before = state.clone();
    assert!(
        state
            .reserve_director(request(1, position(1120, 512)), &Ground, &catalog())
            .is_err()
    );
    assert_eq!(state, before);
}

struct Water;
impl Environment for Water {
    fn dimensions(&self, level: LevelId) -> Result<Dimensions> {
        Ground.dimensions(level)
    }
    fn terrain(&self, _: LevelId, _: i32, _: i32) -> Result<u8> {
        Ok(0x80)
    }
    fn base_metatile(&self, level: LevelId, x: i32, y: i32) -> Result<Option<u8>> {
        Ground.base_metatile(level, x, y)
    }
    fn placement(
        &self,
        level: LevelId,
        kind: Kind,
        x: u16,
        y: u16,
        rng: &mut StageRng,
    ) -> Result<Option<(i32, i32)>> {
        Ground.placement(level, kind, x, y, rng)
    }
    fn allows_placement(&self, level: LevelId, x: i32, y: i32) -> Result<bool> {
        Ground.allows_placement(level, x, y)
    }
    fn animation_step(
        &self,
        kind: Kind,
        state: u8,
        cursor: &mut AnimationCursor,
    ) -> Result<AnimationSample> {
        Ground.animation_step(kind, state, cursor)
    }
}
#[test]
fn controlled_bodies_still_obey_shared_terrain_and_decoy_collision() {
    for kind in [
        Kind::Devil,
        Kind::Dentist,
        Kind::DentistVariant,
        Kind::Lawnmower,
        Kind::NerdHerd,
    ] {
        let (mut state, body) = population(kind);
        let mut sink = contacts();
        state
            .step_controlled(
                0,
                &Ground,
                &catalog(),
                &[],
                &mut sink,
                Some(DecisionIntent {
                    actor: body,
                    direction: Direction::Right,
                    ability: false,
                }),
            )
            .unwrap();
        let before = state.resolve(body).unwrap().motion.position;
        state
            .advance_motion(0, &Water, &catalog(), &[], &mut sink)
            .unwrap();
        state
            .advance_animation(0, &Water, &catalog(), &[], &mut sink)
            .unwrap();
        assert_eq!(state.resolve(body).unwrap().motion.position, before);
        assert!(state.resolve(body).unwrap().blocked);
        let decoy = DecoyView {
            actor: ActorId::new(600, 1).unwrap(),
            owner: SeatId::ALL[0],
            level: level(),
            position: position(877, 512),
            width: 24,
            velocity: [0; 3],
        };
        for tick in 1..5 {
            state
                .advance_animation(tick, &Ground, &catalog(), &[decoy], &mut sink)
                .unwrap();
        }
        assert!(state.resolve(body).unwrap().motion.position.x < 877 * 256);
        assert!(matches!(sink.last, Some(ContactEffect::DecoyHit { .. })));
    }
}

#[test]
fn pending_telegraph_rejects_all_present_effects_and_keeps_body_reservation() {
    let (mut state, _) = population(Kind::Devil);
    let ticket = state
        .reserve_director(request(1, position(1120, 512)), &Ground, &catalog())
        .unwrap();
    let before = state.actor(ticket.id).unwrap().clone();
    let effect = crate::EffectId::new(0, 1).unwrap();
    state
        .sleep_near(effect, level(), position(1120, 512))
        .unwrap();
    state
        .rose_contacts(effect, level(), position(1120, 512))
        .unwrap();
    state.burp(SeatId::ALL[0], level()).unwrap();
    assert!(
        !state
            .projectile_contact(
                crate::EffectId::new(1, 1).unwrap(),
                None,
                level(),
                Position {
                    elevation: 8 * 256,
                    ..position(1120, 512)
                },
                4,
                &catalog()
            )
            .unwrap()
    );
    assert!(!state.damage(ticket.id, 3).unwrap());
    assert!(!state.damage_body(before.body.unwrap(), 3).unwrap());
    assert!(state.relocate(ticket.id, &Ground, &catalog()).is_err());
    state.release(SeatId::ALL[0]).unwrap();
    state.refresh_activity(&[None; 4]).unwrap();
    assert_eq!(state.actor(ticket.id), Some(&before));
    assert!(!state.sleep_sources().any(|(_, id)| id == ticket.id));
    assert!(!state.thorn_sources().any(|(_, id)| id == ticket.id));
    state.validate().unwrap();
    let bytes = bincode::encode_to_vec(&state, bincode::config::standard()).unwrap();
    let (restored, used): (PopulationState, _) =
        bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(state, restored);
}

#[test]
fn another_heroes_present_body_is_never_possessable() {
    let (mut state, body) = population(Kind::Dentist);
    let id = state.resolve(body).unwrap().id;
    state.actor_mut(id).unwrap().owner = Some(SeatId::ALL[2]);
    let before = state.clone();
    let mut sink = contacts();
    assert!(
        state
            .step_controlled(
                0,
                &Ground,
                &catalog(),
                &[],
                &mut sink,
                Some(DecisionIntent {
                    actor: body,
                    direction: Direction::Left,
                    ability: false
                })
            )
            .is_err()
    );
    assert_eq!(state, before);
}
