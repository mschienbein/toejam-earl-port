//! Finite external decisions and bounded director reservations. This adds no
//! gameplay selector. Ordinary population stepping supplies no external intent.
use super::*;
use crate::session::Roles;

use tje_world::terrain::TerrainMask;
pub const MAX_DIRECTOR_SPAWNS: usize = 6;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct SpawnTicket {
    pub id: PopulationId,
    pub owner: SeatId,
    pub source: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct DirectorSlot {
    pub ticket: SpawnTicket,
    pub pending: bool,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Encode, Decode)]
pub enum Direction {
    #[default]
    Neutral,
    Up,
    Down,
    Left,
    Right,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct DecisionIntent {
    pub actor: ActorId,
    pub direction: Direction,
    pub ability: bool,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecisionReceipt {
    pub decided: bool,
    pub ability_used: bool,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StepReceipt {
    pub advanced: bool,
    pub decision: DecisionReceipt,
}
#[derive(Default)]
pub(super) struct DecisionSlot {
    pub intent: Option<DecisionIntent>,
    pub receipt: DecisionReceipt,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlStatus {
    pub kind: Kind,
    pub level: LevelId,
    pub enterable: bool,
    pub retainable: bool,
}
#[derive(Debug, Clone, Copy)]
pub struct SpawnRequest {
    pub roles: Roles,
    pub owner: SeatId,
    pub source: u32,
    pub level: LevelId,
    pub kind: Kind,
    pub position: Position,
}

pub const fn supported(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Devil | Kind::Dentist | Kind::DentistVariant | Kind::Lawnmower | Kind::NerdHerd
    )
}
pub const fn manual_ability(kind: Kind) -> bool {
    matches!(kind, Kind::Dentist | Kind::DentistVariant)
}

impl PopulationState {
    pub fn director_pending(&self, id: PopulationId) -> bool {
        self.actor(id).is_some_and(|actor| actor.pending_director)
    }
    pub fn director_slots(&self) -> &[Option<DirectorSlot>; MAX_DIRECTOR_SPAWNS] {
        &self.directors
    }
    pub fn next_director_source(&self) -> u32 {
        self.next_director_source
    }
    pub fn control_status(&self, body: ActorId) -> Result<ControlStatus> {
        let actor = self
            .resolve(body)
            .ok_or(Error::State("stale director body generation"))?;
        if actor.body != Some(body)
            || !supported(actor.kind)
            || actor.owner.is_some_and(|owner| owner != SeatId::ALL[3])
        {
            return Err(Error::State("unsupported/companion director body"));
        }
        let retainable = !actor.retired
            && !actor.hidden
            && actor.health != 0
            && actor.id.level.get() != 0
            && actor.interest != SeatMask::NONE
            && !self.director_pending(actor.id);
        Ok(ControlStatus {
            kind: actor.kind,
            level: actor.id.level,
            retainable,
            enterable: retainable && actor.state < 8 && actor.state != 2,
        })
    }
    pub fn validate_control(
        &self,
        intent: DecisionIntent,
        catalog: &DefinitionCatalog,
    ) -> Result<()> {
        let status = self.control_status(intent.actor)?;
        if !status.retainable
            || catalog.get(status.kind).thinker != Thinker::Generic
            || intent.ability && !manual_ability(status.kind)
        {
            return Err(Error::State("illegal director decision capability/state"));
        }
        Ok(())
    }
    /// Append-only stable identity. All stock/authored holes remain declared;
    /// present and director records are never reused. The finite lifetime budget
    /// is `limits.per_floor` records total on each floor (at most64), not an
    /// unbounded tombstone list. Busy/cumulative exhaustion is a defined failure.
    pub fn reserve_director(
        &mut self,
        request: SpawnRequest,
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
    ) -> Result<SpawnTicket> {
        let SpawnRequest {
            roles,
            owner,
            source,
            level,
            kind,
            position,
        } = request;
        roles
            .validate()
            .map_err(|_| Error::State("director role manifest"))?;
        if roles.director != Some(owner)
            || roles.heroes.contains(owner)
            || !supported(kind)
            || owner != SeatId::ALL[3]
            || roles.heroes.bits() != 7
            || roles.inputs != SeatMask::ALL
            || source != self.next_director_source
            || source == u32::MAX
        {
            return Err(Error::State("director reservation role/source/kind"));
        }
        position
            .validate()
            .map_err(|_| Error::Bounds("director spawn position"))?;
        if position.elevation != 0
            || !roles
                .heroes
                .iter()
                .any(|seat| self.retained[seat.index()] == Some(level))
        {
            return Err(Error::State("director spawn requires hero-active ground"));
        }
        let floor_index = self.floor_index(level)?;
        let floor = &self.floors[floor_index];
        let local = (u64::BITS - floor.declared_slots.leading_zeros()) as u16;
        let local = local.max(floor.actors.last().map_or(0, |actor| actor.id.local + 1));
        if local >= self.limits.per_floor {
            return Err(Error::Capacity("director lifetime persistent identities"));
        }
        let slot = self
            .directors
            .iter()
            .position(|entry| {
                entry.is_none_or(|entry| {
                    self.actor(entry.ticket.id)
                        .is_some_and(|actor| actor.retired)
                })
            })
            .ok_or(Error::Capacity("six live/pending director spawns"))?;
        if self.bodies.iter().all(|slot| slot.owner.is_some()) {
            return Err(Error::Capacity("director reserved body"));
        }
        self.preflight_growth(1)?;
        let (x, y) = (position.x >> 8, position.y >> 8);
        if environment.dimensions(level)?.world_cell(x, y).is_none()
            || !environment.allows_placement(level, x, y)?
            || !TerrainMask::LAND_OR_SAND.accepts(environment.terrain(level, x, y)?)
            || kind == Kind::Lawnmower && environment.terrain(level, x, y)? != 0x10
        {
            return Err(Error::Bounds("director placement terrain"));
        }
        let id = PopulationId { level, local };
        let ticket = SpawnTicket { id, owner, source };
        let mut label = *b"director-ai\0\0\0\0";
        label[11..].copy_from_slice(&source.to_le_bytes());
        let mut actor = PersistentActor::new(
            id,
            kind,
            position,
            catalog.get(kind),
            StageRng::derive(&self.identity, level, 0, &label),
            false,
        );
        // A director record has one body. Companion selection is not a director
        // ability and cannot silently double the negotiated reservation.
        actor.companion = false;
        actor.set_state(0);
        actor.owner = Some(owner);
        actor.pending_director = true;
        actor.hidden = true;
        actor.collidable = false;
        actor.body = Some(allocate_body(&mut self.bodies, id, false)?);
        self.floors[floor_index].actors.push(actor);
        self.floors[floor_index].declared_slots |= 1_u64 << local;
        self.directors[slot] = Some(DirectorSlot {
            ticket,
            pending: true,
        });
        self.next_director_source += 1;
        Ok(ticket)
    }
    fn director_index(&self, ticket: SpawnTicket) -> Result<usize> {
        self.directors
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.ticket == ticket))
            .ok_or(Error::State("stale/foreign director spawn receipt"))
    }
    pub fn director_ticket_live(&self, ticket: SpawnTicket) -> bool {
        self.director_index(ticket).is_ok()
            && self
                .actor(ticket.id)
                .is_some_and(|actor| !actor.retired && actor.health != 0)
    }
    /// Caller revalidates the shared footprint/hero exclusion/safe-zone/reachability
    /// transaction immediately before activation. This operation changes no
    /// position/health/timer, performs no AI and emits no duplicate contact.
    pub fn activate_director(&mut self, ticket: SpawnTicket) -> Result<bool> {
        let index = self.director_index(ticket)?;
        let slot = self.directors[index].expect("resolved director slot");
        if !slot.pending {
            return Ok(false);
        }
        let actor = self
            .actor_mut(ticket.id)
            .ok_or(Error::State("director record missing"))?;
        if actor.retired || actor.body.is_none() {
            return Err(Error::State("director activation lost reservation"));
        }
        actor.hidden = false;
        actor.collidable = true;
        actor.pending_director = false;
        self.directors[index] = Some(DirectorSlot {
            ticket,
            pending: false,
        });
        Ok(true)
    }
    /// Only this exact owner's unactivated reservation can be canceled. The
    /// ordinary remove transaction updates only this new record's own bit.
    pub fn cancel_director(&mut self, ticket: SpawnTicket) -> Result<bool> {
        let index = self.director_index(ticket)?;
        if !self.directors[index]
            .expect("resolved director slot")
            .pending
        {
            return Err(Error::State(
                "cannot refund/cancel an activated director spawn",
            ));
        }
        let removed = self.remove(ticket.id, false)?;
        self.actor_mut(ticket.id)
            .expect("removed director record")
            .pending_director = false;
        self.directors[index] = Some(DirectorSlot {
            ticket,
            pending: false,
        });
        Ok(removed)
    }
    pub(super) fn validate_directors(&self) -> Result<()> {
        if self.next_director_source == 0 {
            return Err(Error::State("director source counter"));
        }
        for actor in self
            .floors
            .iter()
            .flat_map(|floor| &floor.actors)
            .filter(|actor| actor.pending_director)
        {
            if !self
                .directors
                .iter()
                .flatten()
                .any(|slot| slot.pending && slot.ticket.id == actor.id)
            {
                return Err(Error::State("pending director record has no owner receipt"));
            }
        }
        for (index, slot) in self
            .directors
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.map(|slot| (index, slot)))
        {
            let actor = self
                .actor(slot.ticket.id)
                .ok_or(Error::State("director ticket has no persistent record"))?;
            slot.ticket
                .owner
                .validate()
                .map_err(|_| Error::State("director spawn owner"))?;
            if slot.ticket.source == 0
                || slot.ticket.source >= self.next_director_source
                || !supported(actor.kind)
                || actor.pending_director != slot.pending
                || actor.owner != Some(slot.ticket.owner)
                || slot.ticket.owner != SeatId::ALL[3]
                || self.directors[..index].iter().flatten().any(|other| {
                    other.ticket.id == slot.ticket.id || other.ticket.source == slot.ticket.source
                })
                || slot.pending
                    && (actor.retired
                        || actor.body.is_none()
                        || !actor.hidden
                        || actor.collidable
                        || actor.companion
                        || actor.interest != SeatMask::NONE)
            {
                return Err(Error::State("director spawn receipt/reservation"));
            }
        }
        Ok(())
    }
}

/// Called only inside generic's expired decision slot, after the original
/// protected-state and blocked-motion rules. It never invokes the thinker again
/// and never integrates motion or applies contact damage. Timer minima are
/// imported type parameters, not new speed/HP buffs or random stream draws.
pub(super) fn decide(
    actor: &mut PersistentActor,
    definition: &Definition,
    intent: DecisionIntent,
) -> Result<DecisionReceipt> {
    if actor.body != Some(intent.actor) || !supported(actor.kind) || actor.timer >= 0 {
        return Err(Error::State("director decision boundary/body"));
    }
    let ability_used = intent.ability && manual_ability(actor.kind);
    if intent.ability && !ability_used {
        return Err(Error::State("no manual ability for passive contact kind"));
    }
    if actor.state_delay != 0 {
        actor.state_delay -= 1;
        actor.timer = 1;
        return Ok(DecisionReceipt::default());
    }
    actor.motion.velocity[0] = 0;
    actor.motion.velocity[1] = 0;
    let facing = match intent.direction {
        Direction::Up => 0,
        Direction::Down => 1,
        Direction::Left => 2,
        Direction::Right => 3,
        Direction::Neutral => match actor.state {
            0..=1 => actor.state,
            4..=7 => actor.state - 4,
            8..=11 => actor.state - 8,
            _ => 0,
        },
    };
    if ability_used {
        actor.set_state(8 + facing);
        actor.timer = i16::from(definition.target_time.minimum).max(1);
    } else {
        let mut state = if intent.direction == Direction::Neutral {
            facing & 1
        } else {
            4 + facing
        };
        if definition.change_axis
            && state >= 4
            && actor.state != state
            && state & 2 == actor.state & 2
        {
            // Preserve the imported axis-change constraint. The ruleset chooses
            // the old facing bit, not a draw from the stock AI RNG stream.
            state = ((state ^ 2) & 0xfe) | (actor.state & 1);
        }
        actor.set_state(state);
        match state {
            4 => actor.motion.velocity[1] = -320,
            5 => actor.motion.velocity[1] = 320,
            6 => actor.motion.velocity[0] = -320,
            7 => actor.motion.velocity[0] = 320,
            _ => {}
        }
        actor.timer = i16::from(definition.move_time.minimum).max(1);
    }
    Ok(DecisionReceipt {
        decided: true,
        ability_used,
    })
}

#[cfg(test)]
#[path = "director_tests.rs"]
mod tests;
