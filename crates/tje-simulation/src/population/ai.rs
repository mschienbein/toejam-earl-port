use super::*;

/// G/targeting.c:18-121. Decoys are a separate, higher-priority candidate set.
/// Distances use signed quarter coordinates and wide squared intermediates.
pub fn nearest_target(
    actor: &PersistentActor,
    maximum: u8,
    separation: i32,
    heroes: &[Option<HeroView>; 4],
    decoys: &[DecoyView],
) -> Option<TargetSelection> {
    let distance = |position: Position| {
        let (x, y) = actor.pixels();
        let tx = position.x >> 8;
        let ty = position.y >> 8;
        let dx = ((tx - x) >> 2)
            + if x < tx {
                -(separation >> 2)
            } else {
                separation >> 2
            };
        let dy = (ty - y) >> 2;
        if dx.unsigned_abs() >= 160 || dy.unsigned_abs() >= 112 {
            return None;
        }
        Some((
            dx,
            dy,
            i64::from(dx) * i64::from(dx) + i64::from(dy) * i64::from(dy),
        ))
    };
    let mut decoy_best: Option<(u8, ActorId, TargetSelection)> = None;
    for decoy in decoys {
        if decoy.level != actor.id.level {
            continue;
        }
        if let Some((dx, dy, squared)) = distance(decoy.position) {
            if squared > 0x1800 {
                continue;
            }
            let better = decoy_best.as_ref().is_none_or(|(seat, id, old)| {
                squared < old.distance_squared
                    || squared == old.distance_squared
                        && (decoy.owner.get(), decoy.actor) > (*seat, *id)
            });
            if better {
                decoy_best = Some((
                    decoy.owner.get(),
                    decoy.actor,
                    TargetSelection {
                        target: Target::Decoy(decoy.actor),
                        delta_x: dx,
                        delta_y: dy,
                        distance_squared: squared,
                        position: decoy.position,
                    },
                ));
            }
        }
    }
    if let Some((_, _, selected)) = decoy_best {
        return Some(selected);
    }
    let limit = if actor.state == 2 {
        0x400
    } else {
        i64::from(maximum) * 256
    };
    let mut best: Option<(SeatId, TargetSelection)> = None;
    for hero in heroes.iter().flatten() {
        if hero.level != actor.id.level
            || hero.elevator
            || hero.suppressed
            || !(actor.state != 2 && hero.targetable || hero.active_motion || hero.swimming)
            || actor.kind == Kind::Cupid && hero.remapped
            || actor.kind == Kind::Mole
                && (!hero.has_inventory || actor.mole_contacts.contains(hero.seat))
        {
            continue;
        }
        if let Some((dx, dy, squared)) = distance(hero.position) {
            if squared > limit {
                continue;
            }
            if best.as_ref().is_none_or(|(seat, old)| {
                squared < old.distance_squared
                    || squared == old.distance_squared && hero.seat > *seat
            }) {
                best = Some((
                    hero.seat,
                    TargetSelection {
                        target: Target::Hero(hero.seat),
                        delta_x: dx,
                        delta_y: dy,
                        distance_squared: squared,
                        position: hero.position,
                    },
                ));
            }
        }
    }
    best.map(|(_, target)| target)
}

fn steer_delta(actor: &mut PersistentActor, dx: i32, dy: i32) -> u8 {
    actor.motion.velocity[0] = dx.signum() * 256;
    actor.motion.velocity[1] = dy.signum() * 256;
    if dx != 0 && dy != 0 {
        if dy.unsigned_abs() > 5 && dx.unsigned_abs() < 20 {
            actor.motion.velocity[0] = 0;
        } else if dx.unsigned_abs() > 20 && dy.unsigned_abs() < 5 {
            actor.motion.velocity[1] = 0;
        }
    }
    if actor.motion.velocity[0] < 0 {
        6
    } else if actor.motion.velocity[0] > 0 {
        7
    } else if actor.motion.velocity[1] < 0 {
        4
    } else if actor.motion.velocity[1] > 0 {
        5
    } else {
        0
    }
}
fn steer(
    actor: &mut PersistentActor,
    definition: &Definition,
    heroes: &[Option<HeroView>; 4],
    decoys: &[DecoyView],
    sink: &mut impl ContactSink,
) -> Result<Option<u8>> {
    let separation = match actor.kind {
        Kind::Chickens => 100,
        Kind::Cupid => 85,
        _ => 0,
    };
    let Some(target) = nearest_target(
        actor,
        definition.target_distance,
        separation,
        heroes,
        decoys,
    ) else {
        return Ok(None);
    };
    let previous_x = actor.motion.velocity[0];
    let dx = target.delta_x;
    let dy = target.delta_y;
    let mut next = if actor.kind == Kind::NerdHerd {
        actor.motion.velocity[1] = if dy < -2 {
            -256
        } else if dy > 2 {
            256
        } else {
            0
        };
        actor.motion.velocity[0] = if dx < 0 { -256 } else { 256 };
        if dx < 0 { 6 } else { 7 }
    } else {
        steer_delta(actor, dx, dy)
    };
    if actor.kind == Kind::Cupid {
        if actor.motion.velocity[0] == 0 && actor.motion.velocity[1] != 0 {
            actor.motion.velocity[0] = if previous_x != 0 {
                previous_x
            } else if actor.rng.next_u16() & 1 != 0 {
                256
            } else {
                -256
            };
        }
        next = if actor.motion.position.x > target.position.x {
            6
        } else {
            7
        };
    }
    if actor.kind == Kind::Shark
        && let Target::Hero(seat) = target.target
        && heroes[seat.index()].is_some_and(|hero| hero.swimming)
        && !actor.target_alerts.contains(seat)
        && request(actor, seat, ContactEffect::SharkAlert, sink)? != ContactOutcome::Rejected
    {
        actor.target_alerts =
            SeatMask::new(actor.target_alerts.bits() | 1 << seat.get()).expect("four seats");
    }
    Ok(Some(next))
}

fn turn_state(actor: &mut PersistentActor) -> u8 {
    // G/movement_decision.c:24-31.
    match actor.state {
        6 => 0,
        7 => 1,
        _ => {
            if actor.rng.next_u16() & 1 != 0 {
                0
            } else {
                1
            }
        }
    }
}
fn cardinal(actor: &mut PersistentActor, next: u8) {
    if actor.motion.velocity[0] != 0 || actor.motion.velocity[1] != 0 {
        return;
    }
    match next {
        4 => actor.motion.velocity[1] = -320,
        5 => actor.motion.velocity[1] = 320,
        6 => actor.motion.velocity[0] = -320,
        7 => actor.motion.velocity[0] = 320,
        _ => {}
    }
}

pub(super) fn request(
    actor: &PersistentActor,
    seat: SeatId,
    effect: ContactEffect,
    sink: &mut impl ContactSink,
) -> Result<ContactOutcome> {
    sink.apply(ContactRequest {
        source: actor.id,
        actor: actor
            .body
            .ok_or(Error::State("contact without active body"))?,
        seat,
        kind: actor.kind,
        position: actor.motion.position,
        animation_finished: actor.animation_finished,
        interest: actor.interest,
        effect,
    })
}

#[cfg(test)]
pub(super) fn think(
    actor: &mut PersistentActor,
    catalog: &DefinitionCatalog,
    heroes: &[Option<HeroView>; 4],
    decoys: &[DecoyView],
    sink: &mut impl ContactSink,
) -> Result<()> {
    think_controlled(
        actor,
        catalog,
        heroes,
        decoys,
        sink,
        &mut director::DecisionSlot::default(),
    )
}
pub(super) fn think_controlled(
    actor: &mut PersistentActor,
    catalog: &DefinitionCatalog,
    heroes: &[Option<HeroView>; 4],
    decoys: &[DecoyView],
    sink: &mut impl ContactSink,
    decision: &mut director::DecisionSlot,
) -> Result<()> {
    if actor.mail_order || actor.state == SLEEP_STATE {
        return Ok(());
    }
    let definition = catalog.get(actor.kind);
    if actor.kind == Kind::Mailbox {
        if actor.service_timer != 0 {
            actor.service_timer -= 1;
        }
        if actor.state & 0xfc == 4 && actor.service_timer == 0 {
            actor.set_state(0);
            actor.service_timer = 175;
        } else if actor.state == 3 && actor.animation_finished {
            actor.set_state(4);
            actor.timer = 0;
            actor.service_timer = 33;
        } else if actor.state == 1 && actor.animation_finished {
            actor.set_state(0);
        }
    }
    match definition.thinker {
        Thinker::None => {}
        Thinker::Generic => generic(
            actor,
            definition,
            &catalog.projectiles,
            heroes,
            decoys,
            sink,
            decision,
        )?,
        Thinker::Timed => timed(
            actor,
            definition,
            &catalog.projectiles,
            heroes,
            decoys,
            sink,
        )?,
        Thinker::HorizontalWander => horizontal_wander(actor, definition, heroes, decoys, sink)?,
        Thinker::HorizontalTurn => horizontal_turn(actor),
        Thinker::HorizontalReversal => horizontal_reversal(actor),
        Thinker::Whirlwind => whirlwind(actor, definition, heroes, decoys, sink)?,
        Thinker::Santa => santa(actor, heroes),
    }
    Ok(())
}
fn special(
    actor: &mut PersistentActor,
    projectiles: &ProjectileVectors,
    heroes: &[Option<HeroView>; 4],
    decoys: &[DecoyView],
    sink: &mut impl ContactSink,
) -> Result<bool> {
    if actor.kind == Kind::Mailbox && actor.state & 0xfc != 4 {
        return Ok(true);
    }
    if actor.kind == Kind::OperaSinger && actor.state_delay != 0 {
        actor.state_delay = 0;
        return Ok(true);
    }
    if actor.kind == Kind::Mole {
        if actor.state <= 1 {
            actor.collidable = true;
            actor.draw_late = false;
            if !actor.animation_finished {
                return Ok(true);
            }
            actor.timer = 0;
        } else if actor.state == 3 {
            actor.collidable = true;
            actor.timer = actor.timer.saturating_sub(1);
            if actor.timer > 0 {
                return Ok(true);
            }
        }
    }
    if actor.state & 0xfc == 8 {
        match actor.kind {
            Kind::Dentist | Kind::DentistVariant | Kind::Boogieman => {
                if !actor.animation_finished {
                    return Ok(true);
                }
                actor.timer = 0;
            }
            Kind::Chickens => {
                projectile_attack(actor, false, projectiles, heroes, decoys, sink)?;
                return Ok(true);
            }
            _ => {}
        }
    }
    Ok(false)
}
fn generic(
    actor: &mut PersistentActor,
    definition: &Definition,
    projectiles: &ProjectileVectors,
    heroes: &[Option<HeroView>; 4],
    decoys: &[DecoyView],
    sink: &mut impl ContactSink,
    decision: &mut director::DecisionSlot,
) -> Result<()> {
    if special(actor, projectiles, heroes, decoys, sink)? {
        return Ok(());
    }
    if actor.blocked && actor.state != 0 && actor.state != 2 {
        actor.timer = 0;
        actor.state_delay = definition.state_delay;
        actor.blocked = false;
    }
    actor.timer = actor.timer.saturating_sub(1);
    if actor.timer < 0 {
        if let Some(intent) = decision
            .intent
            .filter(|intent| actor.body == Some(intent.actor))
        {
            decision.receipt = director::decide(actor, definition, intent)?;
            return Ok(());
        }
        actor.motion.velocity[0] = 0;
        actor.motion.velocity[1] = 0;
        let mut next = None;
        if actor.state_delay == 0 {
            if actor.rng.bounded(100) < u32::from(definition.target_chance) {
                next = steer(actor, definition, heroes, decoys, sink)?;
                if next.is_some() {
                    actor.timer = definition.target_time.sample(&mut actor.rng);
                }
                if actor.kind == Kind::Chickens
                    && next.is_some_and(|state| {
                        state != actor.state && actor.state & 4 != 0 && state & 4 != 0
                    })
                {
                    next = Some(0);
                    actor.timer = 2;
                }
            }
        } else {
            actor.state_delay -= 1;
        }
        if next.is_none() && actor.state != 2 {
            if actor.rng.bounded(100) < u32::from(definition.turn_chance) {
                let state = turn_state(actor);
                if state != actor.state {
                    next = Some(state);
                }
                actor.timer = definition.turn_time.sample(&mut actor.rng);
            } else {
                next = Some((actor.rng.next_u16() & 3) as u8 + 4);
                actor.timer = definition.move_time.sample(&mut actor.rng);
            }
        }
        if let Some(mut state) = next {
            if definition.change_axis
                && state >= 4
                && actor.state != state
                && state & 2 == actor.state & 2
            {
                state = ((state ^ 2) & 0xfe) | (actor.rng.next_u16() & 1) as u8;
                actor.motion.velocity[0] = 0;
                actor.motion.velocity[1] = 0;
            }
            cardinal(actor, state);
            actor.set_state(state);
            if actor.kind == Kind::Mole {
                actor.draw_late = state > 1;
            }
        }
        actor.timer = actor.timer.max(1);
    }
    if actor.kind == Kind::Mole && actor.state & 0xfc == 4 {
        actor.collidable = false;
    }
    Ok(())
}
fn timed(
    actor: &mut PersistentActor,
    definition: &Definition,
    projectiles: &ProjectileVectors,
    heroes: &[Option<HeroView>; 4],
    decoys: &[DecoyView],
    sink: &mut impl ContactSink,
) -> Result<()> {
    if actor.kind == Kind::Cupid && actor.state == 3 {
        return Ok(());
    }
    if actor.blocked && actor.state != 0 && actor.state != 2 {
        actor.timer = 0;
        actor.state_delay = definition.state_delay;
        actor.blocked = false;
    }
    actor.timer = actor.timer.saturating_sub(1);
    let mut next = None;
    if actor.timer < 0 {
        actor.motion.velocity[0] = 0;
        actor.motion.velocity[1] = 0;
        if actor.state_delay == 0 {
            if actor.rng.bounded(100) < u32::from(definition.target_chance) {
                next = steer(actor, definition, heroes, decoys, sink)?;
                if next.is_some() {
                    actor.timer = definition.target_time.sample(&mut actor.rng);
                }
                if matches!(actor.kind, Kind::Bees | Kind::BeesVariant)
                    && next
                        .is_some_and(|state| state != actor.state && actor.state >= 4 && state >= 4)
                {
                    next = Some(0);
                    actor.timer = 2;
                }
            }
        } else {
            actor.state_delay -= 1;
        }
        if actor.kind == Kind::IceCreamTruck && next == Some(0) {
            next = None;
        }
        if actor.kind == Kind::Cupid && actor.state & 0xfc == 8 {
            next = None;
        } else if next.is_none() && actor.state != 2 {
            if actor.rng.bounded(100) < u32::from(definition.turn_chance) {
                next = Some(turn_state(actor));
                actor.timer = definition.turn_time.sample(&mut actor.rng);
            } else {
                next = Some((actor.rng.next_u16() & 3) as u8 + 4);
                actor.timer = definition.move_time.sample(&mut actor.rng);
            }
        }
        if let Some(mut state) = next {
            cardinal(actor, state);
            if actor.kind == Kind::IceCreamTruck {
                state = truck_state(actor, state, heroes);
            }
            actor.set_state(state);
        }
        actor.timer = actor.timer.max(1);
    }
    if actor.kind == Kind::IceCreamTruck {
        let base = actor.state & 0xfc;
        let state = match base {
            4 if actor.terrain_status >= 0x80 => Some(actor.state + 4),
            8 if actor.animation_finished => Some(if actor.terrain_status >= 0x80 {
                actor.state + 4
            } else {
                actor.state - 4
            }),
            12 if actor.terrain_status < 0x50 && !near_hero(actor, heroes, 128, 64) => {
                Some(actor.state - 4)
            }
            _ => None,
        };
        if let Some(state) = state {
            actor.set_state(state);
        }
        actor.collidable = actor.state & 0xfc == 4;
    }
    if actor.kind == Kind::Cupid && actor.state & 0xfc == 8 {
        projectile_attack(actor, true, projectiles, heroes, decoys, sink)?;
    }
    Ok(())
}
fn truck_state(actor: &PersistentActor, next: u8, heroes: &[Option<HeroView>; 4]) -> u8 {
    match actor.state & 0xfc {
        4 if actor.terrain_status >= 0x80 => next + 4,
        8 if !actor.animation_finished => next + 4,
        8 if actor.terrain_status >= 0x80 => next + 8,
        12 if actor.terrain_status >= 0x50 || near_hero(actor, heroes, 96, 48) => next + 8,
        12 => next + 4,
        _ => next,
    }
}
fn horizontal_wander(
    actor: &mut PersistentActor,
    definition: &Definition,
    heroes: &[Option<HeroView>; 4],
    decoys: &[DecoyView],
    sink: &mut impl ContactSink,
) -> Result<()> {
    if actor.blocked && actor.state != 0 && actor.state != 2 {
        actor.motion.velocity[0] = -actor.motion.velocity[0];
        if actor.rng.next_u16() & 1 != 0 {
            actor.motion.velocity[1] = -actor.motion.velocity[1];
        }
        actor.set_state(actor.state ^ 1);
        actor.timer = 3;
        actor.state_delay = definition.state_delay;
        actor.blocked = false;
    }
    actor.timer = actor.timer.saturating_sub(1);
    if actor.timer >= 0 {
        return Ok(());
    }
    let mut next = None;
    if actor.state_delay == 0 {
        if actor.rng.bounded(100) < u32::from(definition.target_chance) {
            let previous = actor.motion.velocity;
            next = steer(actor, definition, heroes, decoys, sink)?;
            if let Some(state) = next {
                // Fixed simulation-interest replaces the original draw-visibility gate.
                if actor.state != 2
                    && actor.interest != SeatMask::NONE
                    && (actor.state ^ state) & 1 != 0
                {
                    next = None;
                    actor.motion.velocity = previous;
                }
                actor.timer = definition.target_time.sample(&mut actor.rng);
            }
        }
    } else {
        actor.state_delay -= 1;
    }
    if next.is_none() && actor.state != 2 {
        if actor.state == 0 {
            let state = (actor.rng.next_u16() & 1) as u8 + 6;
            actor.motion.velocity[0] = if state == 6 { -320 } else { 320 };
            next = Some(state);
        }
        actor.motion.velocity[1] = if actor.rng.bounded(100) < 33 {
            0
        } else if actor.rng.bounded(100) < 50 {
            -320
        } else {
            320
        };
        actor.timer = definition.move_time.sample(&mut actor.rng);
    }
    if let Some(state) = next {
        actor.set_state(state);
    }
    actor.timer = actor.timer.max(1);
    Ok(())
}
fn horizontal_turn(actor: &mut PersistentActor) {
    if actor.state_delay != 0 {
        actor.state_delay = 0;
        return;
    }
    let next = if actor.blocked && actor.state >= 6 {
        Some(turn_state(actor))
    } else if actor.animation_finished {
        match actor.state {
            2 => Some(3),
            0 | 1 | 3 => {
                let left = actor.motion.velocity[0] > 0;
                actor.motion.velocity[0] = if left { -320 } else { 320 };
                Some(if left { 4 } else { 5 })
            }
            4 => {
                actor.motion.velocity[0] = -320;
                Some(6)
            }
            5 => {
                actor.motion.velocity[0] = 320;
                Some(7)
            }
            _ => None,
        }
    } else {
        None
    };
    if let Some(state) = next {
        actor.set_state(state);
    }
    actor.blocked = false;
}
fn horizontal_reversal(actor: &mut PersistentActor) {
    if actor.state_delay != 0 {
        actor.state_delay = 0;
        return;
    }
    let next = if actor.blocked && actor.state >= 2 {
        Some(turn_state(actor))
    } else if actor.state <= 1 {
        let left = actor.motion.velocity[0] > 0;
        actor.motion.velocity[0] = if left { -320 } else { 320 };
        Some(if left { 2 } else { 3 })
    } else {
        None
    };
    if let Some(state) = next {
        actor.set_state(state);
    }
    actor.blocked = false;
}
fn whirlwind(
    actor: &mut PersistentActor,
    definition: &Definition,
    heroes: &[Option<HeroView>; 4],
    decoys: &[DecoyView],
    sink: &mut impl ContactSink,
) -> Result<()> {
    if actor.state == 2 {
        if actor.animation_finished {
            actor.motion.position = actor.saved_position;
            actor.hidden = true;
            actor.set_state(3);
        }
        return Ok(());
    }
    actor.timer = actor.timer.saturating_sub(1);
    if actor.timer >= 0 {
        return Ok(());
    }
    actor.motion.velocity[0] = 0;
    actor.motion.velocity[1] = 0;
    let next = steer(actor, definition, heroes, decoys, sink)?;
    if next.is_some() {
        actor.timer = definition.target_time.sample(&mut actor.rng);
    }
    if actor.state == 3 {
        if actor.hidden {
            if next.is_some() {
                actor.hidden = false;
                actor.animation = AnimationCursor::default();
            }
            actor.timer = 3;
        } else if actor.animation_finished {
            actor.set_state(4);
            actor.timer = 0;
            actor.phase_count = 7;
        } else {
            actor.timer = 1;
        }
    } else {
        actor.phase_count = actor.phase_count.saturating_sub(1);
        if actor.phase_count < 0 || next.is_none() {
            actor.set_state(2);
        }
    }
    Ok(())
}
fn near_hero(actor: &PersistentActor, heroes: &[Option<HeroView>; 4], x: i64, y: i64) -> bool {
    heroes.iter().flatten().any(|hero| {
        hero.level == actor.id.level
            && !hero.suppressed
            && in_range(actor.motion.position, hero.position, x, y)
    })
}
pub(super) fn in_range(a: Position, b: Position, x: i64, y: i64) -> bool {
    (i64::from(a.x >> 8) - i64::from(b.x >> 8)).abs() < x
        && (i64::from(a.y >> 8) - i64::from(b.y >> 8)).abs() < y
}
fn santa(actor: &mut PersistentActor, heroes: &[Option<HeroView>; 4]) {
    let mut next = None;
    if actor.state == 2 || actor.state == 3 {
        if actor.animation_finished {
            actor.motion.acceleration[2] = 16;
            actor.shadow = true;
            next = Some(5);
        }
    } else if actor.state & 0xfc == 4 {
        if actor.motion.position.elevation > 20 * 256
            && actor.motion.velocity[0] == 0
            && actor.motion.velocity[1] == 0
        {
            let mut state = (actor.rng.next_u16() & 3) as u8 + 4;
            if state == 5 {
                state = 4;
            }
            match state {
                4 => {
                    actor.motion.velocity[1] = -256;
                    actor.motion.acceleration[1] = -32;
                }
                6 => {
                    actor.motion.velocity[0] = -256;
                    actor.motion.acceleration[0] = -32;
                }
                _ => {
                    actor.motion.velocity[0] = 256;
                    actor.motion.acceleration[0] = 32;
                }
            }
            if actor.motion.velocity[1] != 0 {
                let sign = if actor.rng.next_u16() & 1 != 0 { -1 } else { 1 };
                actor.motion.velocity[0] = sign * 256;
                actor.motion.acceleration[0] = sign * 16;
            } else {
                actor.motion.velocity[1] = actor.rng.bounded(129) as i32 - 64;
            }
            next = Some(state);
        }
        if actor.motion.position.elevation > 30 * 256 {
            actor.motion.acceleration[2] = 0;
            actor.motion.velocity[2] = 0;
        }
        // Once out of canonical hero interest Santa becomes dormant; no camera-
        // dependent relocation loop or random draws are allowed here.
    } else if actor.state == 1 {
        if actor.frame > 2 && actor.frame < 5 {
            if heroes.iter().flatten().any(|hero| {
                hero.level == actor.id.level
                    && in_range(
                        actor.motion.position,
                        hero.position,
                        if hero.seat.get() == 1 { 150 } else { 125 },
                        75,
                    )
                    && hero.active_motion
            }) {
                next = Some(2);
            }
        } else if actor.animation_finished {
            next = Some(0);
        }
    } else if actor.state == 0 {
        for hero in heroes.iter().flatten() {
            if hero.level != actor.id.level
                || !in_range(actor.motion.position, hero.position, 125, 75)
            {
                continue;
            }
            if hero.burping {
                next = Some(2);
                break;
            }
            if hero.sneaking {
                if actor.rng.bounded(100) < 10 {
                    next = Some(1);
                }
            } else if hero.active_motion {
                next = Some(2);
                break;
            }
        }
    }
    if let Some(state) = next {
        actor.set_state(state);
    }
}
fn projectile_attack(
    actor: &mut PersistentActor,
    remap: bool,
    projectiles: &ProjectileVectors,
    heroes: &[Option<HeroView>; 4],
    decoys: &[DecoyView],
    sink: &mut impl ContactSink,
) -> Result<()> {
    let firing = if remap {
        actor.frame == 2
    } else {
        actor.frame < 16 && actor.frame & 3 == 2
    };
    if firing && actor.phase_count != actor.frame as i8 {
        actor.phase_count = actor.frame as i8;
        if remap {
            if let Some(hero) = heroes
                .iter()
                .flatten()
                .find(|hero| hero.level == actor.id.level)
            {
                let mut rng = actor.rng;
                let launch = projectile_launch(
                    actor,
                    projectiles,
                    hero.position,
                    hero.velocity,
                    true,
                    &mut rng,
                );
                request(actor, hero.seat, launch, sink)?;
            }
            return Ok(());
        }
        let start = (actor.rng.next_u16() & 3) as usize;
        for offset in 0..4 {
            let Some(hero) = heroes[(start + offset) & 3] else {
                continue;
            };
            if hero.level != actor.id.level || hero.suppressed {
                continue;
            }
            let (position, velocity) = decoys
                .iter()
                .find(|decoy| decoy.owner == hero.seat && decoy.level == actor.id.level)
                .map_or((hero.position, hero.velocity), |decoy| {
                    (decoy.position, decoy.velocity)
                });
            let (x, y) = actor.pixels();
            let dx = (position.x >> 8) - x;
            let dy = (position.y >> 8) - y;
            let ahead = match actor.state {
                8 => dy < -40 && dy > -200,
                9 => dy > 40 && dy < 200,
                10 => dx < -50 && dx > -320,
                _ => dx > 50 && dx < 320,
            };
            if ahead {
                let mut rng = actor.rng;
                let launch =
                    projectile_launch(actor, projectiles, position, velocity, false, &mut rng);
                if request(actor, hero.seat, launch, sink)? != ContactOutcome::Rejected {
                    actor.rng = rng;
                }
                break;
            }
        }
    } else if actor.animation_finished {
        actor.set_state(if remap {
            if actor.state == 10 { 0 } else { 1 }
        } else {
            0
        });
        actor.phase_count = -1;
    }
    Ok(())
}
fn projectile_launch(
    actor: &PersistentActor,
    vectors: &ProjectileVectors,
    target: Position,
    target_velocity: [i32; 3],
    remap: bool,
    rng: &mut StageRng,
) -> ContactEffect {
    let mut position = actor.motion.position;
    if remap {
        let left = actor.state == 10;
        position.x += if left { -22 * 256 } else { 22 * 256 };
        position.y -= 5 * 256;
        return ContactEffect::Projectile {
            remap: true,
            position,
            velocity: [if left { -896 } else { 896 }, 0, 0],
            acceleration: [0, 0, -32],
        };
    }
    let dx = i64::from((target.x >> 8) - (position.x >> 8)) + i64::from(target_velocity[0] >> 3);
    let dy = i64::from((target.y >> 8) - (position.y >> 8)) + i64::from(target_velocity[1] >> 4);
    let (vx, vy) = if dx == 0 {
        (0, 800)
    } else {
        let slope = (dy * 100 / dx).abs();
        let index = vectors
            .thresholds
            .iter()
            .position(|&threshold| slope < i64::from(threshold))
            .unwrap_or(8);
        let pair = vectors.velocities[index];
        (
            i32::from(pair[0]) * if dx < 0 { -1 } else { 1 },
            i32::from(pair[1]),
        )
    };
    let distance = dx.abs().max(dy.abs()) + (dx.abs().min(dy.abs()) >> 1);
    let gravity = (((distance - 40) >> 3) - 64).clamp(-64, -16) as i32;
    position.elevation = match actor.state {
        8 => 10 * 256,
        9 => 15 * 256,
        _ => 27 * 256,
    };
    if actor.state == 10 {
        position.x -= 10 * 256;
    } else if actor.state == 11 {
        position.x += 10 * 256;
    }
    ContactEffect::Projectile {
        remap: false,
        position,
        velocity: [
            vx + rng.bounded(101) as i32 - 50,
            vy * if dy < 0 { -1 } else { 1 } + rng.bounded(61) as i32 - 30,
            768,
        ],
        acceleration: [0, 0, gravity],
    }
}
