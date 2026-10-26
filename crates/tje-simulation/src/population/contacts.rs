use super::ai::{in_range, request};
use super::*;

pub(super) fn dispatch(
    actor: &mut PersistentActor,
    definition: &Definition,
    priority: u8,
    sink: &mut impl ContactSink,
) -> Result<()> {
    for offset in 0..4 {
        let seat = SeatId::ALL[usize::from((priority + offset) & 3)];
        let Some(hero) = sink.hero(seat) else {
            if actor.service_owner == Some(seat) {
                if let Some(service) = actor.kind.service() {
                    request(
                        actor,
                        seat,
                        ContactEffect::Service {
                            service,
                            stage: ServiceStage::End,
                            cost: 0,
                        },
                        sink,
                    )?;
                    actor.set_state(0);
                    actor.timer = 0;
                } else if actor.kind == Kind::Mole {
                    request(actor, seat, ContactEffect::MoleRelease, sink)?;
                }
                actor.service_owner = None;
                actor.service_timer = 0;
            }
            actor.interactions[seat.index()] = Interaction::Idle;
            continue;
        };
        let within = hero.level == actor.id.level
            && !hero.elevator
            && !hero.suppressed
            && in_range(
                actor.motion.position,
                hero.position,
                i64::from(definition.contact_x),
                i64::from(definition.contact_y),
            );
        if !within {
            if actor.service_owner == Some(seat) {
                if let Some(service) = actor.kind.service() {
                    request(
                        actor,
                        seat,
                        ContactEffect::Service {
                            service,
                            stage: ServiceStage::End,
                            cost: 0,
                        },
                        sink,
                    )?;
                }
                actor.service_owner = None;
            }
            actor.interactions[seat.index()] = Interaction::Idle;
            continue;
        }
        if actor.companion {
            actor.set_state(0);
            actor.timer = 0;
        }
        match definition.touch {
            Touch::ShipPiece => {
                if request(actor, seat, ContactEffect::ShipPiece, sink)? != ContactOutcome::Rejected
                {
                    actor.service_timer = 1;
                    actor.health = 0;
                    actor.retired = true;
                    break;
                }
            }
            Touch::Hurt => {
                if (actor.kind == Kind::Shark && hero.swimming
                    || actor.kind != Kind::Shark && hero.receives_effects)
                    && request(
                        actor,
                        seat,
                        ContactEffect::Hurt {
                            damage: definition.damage,
                        },
                        sink,
                    )? != ContactOutcome::Rejected
                    && matches!(actor.kind, Kind::Dentist | Kind::DentistVariant)
                {
                    actor.set_state(direction(actor, hero.position, 10) + 8);
                    actor.motion.velocity[0] = 0;
                    actor.motion.velocity[1] = 0;
                }
            }
            Touch::Squish => {
                if hero.receives_effects
                    && (actor.kind != Kind::IceCreamTruck || actor.state & 0xfc == 4)
                    && request(
                        actor,
                        seat,
                        ContactEffect::Squish {
                            damage: definition.damage,
                        },
                        sink,
                    )? != ContactOutcome::Rejected
                    && actor.state != 0
                {
                    actor.timer = 30;
                }
            }
            Touch::Hula => {
                if hero.level.get() != 0 && hero.receives_effects {
                    let dx = (i64::from(actor.motion.position.x >> 8)
                        - i64::from(hero.position.x >> 8))
                    .unsigned_abs();
                    let dy = (i64::from(actor.motion.position.y >> 8)
                        - i64::from(hero.position.y >> 8))
                    .unsigned_abs();
                    if dx <= 100 && dy <= 75 {
                        request(
                            actor,
                            seat,
                            ContactEffect::Hula {
                                distance: dx.max(dy + dy / 3) as u16,
                            },
                            sink,
                        )?;
                    }
                }
            }
            Touch::Cupid => cupid(actor, &hero),
            Touch::Whirlwind => {
                if actor.state == 4
                    && hero.receives_effects
                    && request(actor, seat, ContactEffect::Whirlwind, sink)?
                        != ContactOutcome::Rejected
                {
                    actor.health = 0;
                    actor.retired = true;
                    break;
                }
            }
            Touch::Chicken => {
                if in_range(actor.motion.position, hero.position, 25, 15) && hero.receives_effects {
                    request(
                        actor,
                        seat,
                        ContactEffect::Hurt {
                            damage: definition.damage,
                        },
                        sink,
                    )?;
                }
                let dx = (i64::from(actor.motion.position.x >> 8)
                    - i64::from(hero.position.x >> 8))
                .abs();
                let dy = (i64::from(actor.motion.position.y >> 8)
                    - i64::from(hero.position.y >> 8))
                .abs();
                if actor.rng.bounded(100) >= 92
                    && actor.state & 0xfc != 8
                    && !(dy < 50 && dx < 75)
                    && hero.receives_effects
                {
                    let dx = (hero.position.x >> 8) - (actor.motion.position.x >> 8);
                    let dy = (hero.position.y >> 8) - (actor.motion.position.y >> 8);
                    let facing = if dx.unsigned_abs() < dy.unsigned_abs() {
                        if dy < 0 { 0 } else { 1 }
                    } else if dx < 0 {
                        2
                    } else {
                        3
                    };
                    actor.set_state(facing + 8);
                    actor.phase_count = -1;
                }
            }
            Touch::Boogie => {
                if actor.state & 0xfc != 8
                    && hero.receives_effects
                    && request(
                        actor,
                        seat,
                        ContactEffect::Boogie {
                            damage: definition.damage,
                        },
                        sink,
                    )? != ContactOutcome::Rejected
                {
                    actor.set_state(direction(actor, hero.position, 8) + 8);
                }
            }
            Touch::Mailbox if !actor.mail_order => biting_mailbox(actor, &hero, definition, sink)?,
            Touch::Mailbox => service(actor, &hero, Service::MailOrder, sink)?,
            Touch::Wizard => service(actor, &hero, Service::Heal, sink)?,
            Touch::Wiseman => service(actor, &hero, Service::Identify, sink)?,
            Touch::Opera => service(actor, &hero, Service::Opera, sink)?,
            Touch::Mole => mole(actor, &hero, sink)?,
            Touch::Santa => {
                if actor.state <= 1 {
                    let mut rng = actor.rng;
                    let count = (rng.next_u16() & 1) as u8 + 2;
                    if request(actor, seat, ContactEffect::SantaDrops { count }, sink)?
                        != ContactOutcome::Rejected
                    {
                        actor.rng = rng;
                        actor.set_state(3);
                    }
                }
            }
            Touch::Lemonade => {
                request(
                    actor,
                    seat,
                    ContactEffect::Service {
                        service: Service::Lemonade,
                        stage: ServiceStage::Tick,
                        cost: 0,
                    },
                    sink,
                )?;
            }
            Touch::HotTub => {
                request(
                    actor,
                    seat,
                    ContactEffect::Service {
                        service: Service::HotTub,
                        stage: ServiceStage::Tick,
                        cost: 0,
                    },
                    sink,
                )?;
            }
        }
    }
    Ok(())
}
pub(super) fn decoy_contacts(
    actor: &mut PersistentActor,
    definition: &Definition,
    decoys: &[DecoyView],
) {
    if actor.kind != Kind::Cupid || actor.state & 0xfc == 8 || actor.state == 3 {
        return;
    }
    for seat in SeatId::ALL {
        let Some(decoy) = decoys
            .iter()
            .find(|decoy| decoy.owner == seat && decoy.level == actor.id.level)
        else {
            continue;
        };
        let dx = (decoy.position.x >> 8) - (actor.motion.position.x >> 8);
        if dx.unsigned_abs() > 65
            && in_range(
                actor.motion.position,
                decoy.position,
                i64::from(definition.contact_x),
                i64::from(definition.contact_y),
            )
        {
            actor.set_state(if dx > 0 { 11 } else { 10 });
            actor.phase_count = -1;
            return;
        }
    }
}

fn direction(actor: &PersistentActor, target: Position, inset: u32) -> u8 {
    let dx = (target.x >> 8) - (actor.motion.position.x >> 8);
    let dy = (target.y >> 8) - (actor.motion.position.y >> 8);
    if dx.unsigned_abs() > inset {
        if dx > 0 { 3 } else { 2 }
    } else if dy > 0 {
        1
    } else {
        0
    }
}
fn cupid(actor: &mut PersistentActor, hero: &HeroView) {
    if !hero.receives_effects || actor.state == 3 {
        return;
    }
    let elevation = hero.position.elevation >> 8;
    let dx = (hero.position.x >> 8) - (actor.motion.position.x >> 8);
    let dy = (hero.position.y >> 8) - (actor.motion.position.y >> 8);
    if actor.state & 0xfc != 8 && elevation < 40 && dy.unsigned_abs() <= 8 && dx.unsigned_abs() > 65
    {
        actor.set_state(if dx > 0 { 11 } else { 10 });
        actor.phase_count = -1;
    }
    if elevation > 10 && elevation < 75 && dx.unsigned_abs() < 25 {
        actor.set_state(3);
        actor.motion.acceleration[2] = -32;
        actor.motion.velocity[2] = -512;
        actor.collidable = false;
    }
}
fn mole(actor: &mut PersistentActor, hero: &HeroView, sink: &mut impl ContactSink) -> Result<()> {
    let seat = hero.seat;
    if actor.state == 3 {
        if actor.service_owner != Some(seat) {
            return Ok(());
        }
        if actor.timer < 15 {
            request(actor, seat, ContactEffect::MoleRelease, sink)?;
            actor.service_owner = None;
        } else if actor.service_timer == 0 || actor.service_timer < 3 && actor.rng.bounded(100) < 8
        {
            let random = actor.rng.next_u16();
            if request(actor, seat, ContactEffect::MoleTheft { random }, sink)?
                != ContactOutcome::Rejected
            {
                actor.service_timer += 1;
            }
        }
        return Ok(());
    }
    if hero.has_inventory
        && !actor.mole_contacts.contains(seat)
        && hero.receives_effects
        && request(actor, seat, ContactEffect::MoleCapture, sink)? != ContactOutcome::Rejected
    {
        actor.set_state(3);
        actor.timer = 30;
        actor.collidable = true;
        actor.draw_late = true;
        actor.mole_contacts =
            SeatMask::new(actor.mole_contacts.bits() | 1 << seat.get()).expect("four seats");
        actor.service_owner = Some(seat);
        actor.service_timer = 0;
    }
    Ok(())
}
fn service(
    actor: &mut PersistentActor,
    hero: &HeroView,
    service: Service,
    sink: &mut impl ContactSink,
) -> Result<()> {
    let seat = hero.seat;
    let radius_x = if service == Service::MailOrder {
        30
    } else {
        35
    };
    if !hero.service_eligible || !in_range(actor.motion.position, hero.position, radius_x, 15) {
        if actor.service_owner == Some(seat) {
            request(
                actor,
                seat,
                ContactEffect::Service {
                    service,
                    stage: ServiceStage::End,
                    cost: 0,
                },
                sink,
            )?;
            actor.service_owner = None;
        }
        actor.interactions[seat.index()] = Interaction::Idle;
        return Ok(());
    }
    // The one service actor admits one active customer, not one implicit partner.
    if actor.service_owner.is_some_and(|owner| owner != seat) {
        return Ok(());
    }
    let cost = match service {
        Service::Heal => 1,
        Service::Identify => 2,
        Service::Opera => 3,
        _ => 0,
    };
    let state = actor.interactions[seat.index()];
    match state {
        Interaction::Idle => {
            if request(
                actor,
                seat,
                ContactEffect::Service {
                    service,
                    stage: ServiceStage::Offer,
                    cost,
                },
                sink,
            )? != ContactOutcome::Rejected
            {
                actor.interactions[seat.index()] = Interaction::Offered;
                actor.service_owner = Some(seat);
                actor.state_delay = 1;
            }
        }
        Interaction::Offered => {
            let Some((source, accept)) = hero.response else {
                return Ok(());
            };
            if source != actor.id {
                return Ok(());
            }
            if !accept {
                actor.interactions[seat.index()] = Interaction::Complete;
                actor.service_owner = None;
                request(
                    actor,
                    seat,
                    ContactEffect::Service {
                        service,
                        stage: ServiceStage::End,
                        cost: 0,
                    },
                    sink,
                )?;
                return Ok(());
            }
            match request(
                actor,
                seat,
                ContactEffect::Service {
                    service,
                    stage: ServiceStage::Begin,
                    cost,
                },
                sink,
            )? {
                ContactOutcome::Rejected => {}
                ContactOutcome::Applied => {
                    actor.interactions[seat.index()] = Interaction::Active;
                    if service == Service::Heal {
                        actor.set_state(2);
                    }
                    if service == Service::Opera {
                        actor.set_state(3);
                    }
                }
                ContactOutcome::Complete => {
                    actor.interactions[seat.index()] = Interaction::Complete;
                    actor.service_owner = None;
                }
            }
        }
        Interaction::Active => {
            if service == Service::Heal && !actor.animation_finished {
                return Ok(());
            }
            if request(
                actor,
                seat,
                ContactEffect::Service {
                    service,
                    stage: ServiceStage::Tick,
                    cost: 0,
                },
                sink,
            )? == ContactOutcome::Complete
            {
                actor.interactions[seat.index()] = Interaction::Complete;
                actor.service_owner = None;
                actor.set_state(if service == Service::Heal { 3 } else { 0 });
                request(
                    actor,
                    seat,
                    ContactEffect::Service {
                        service,
                        stage: ServiceStage::End,
                        cost: 0,
                    },
                    sink,
                )?;
            }
        }
        Interaction::Complete => {}
    }
    Ok(())
}
fn biting_mailbox(
    actor: &mut PersistentActor,
    hero: &HeroView,
    definition: &Definition,
    sink: &mut impl ContactSink,
) -> Result<()> {
    // Timer admission is actor-scoped; contact visits may not count it four times.
    if actor.state & 0xfc == 4 {
        if hero.receives_effects && in_range(actor.motion.position, hero.position, 20, 10) {
            request(
                actor,
                hero.seat,
                ContactEffect::Hurt {
                    damage: definition.damage,
                },
                sink,
            )?;
        }
    } else if actor.state == 0
        && actor.service_timer == 0
        && in_range(actor.motion.position, hero.position, 100, 40)
    {
        actor.set_state(3);
    }
    Ok(())
}
