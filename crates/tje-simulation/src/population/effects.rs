use super::*;
use crate::EffectId;

/// At most 64 source effects, each touching at most one floor's 64 actors.
pub(super) const MAX_CONTACTS: usize = 64 * MAX_POPULATION as usize;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Encode, Decode)]
pub(super) struct EffectContact {
    pub source: EffectId,
    pub actor: PopulationId,
}

impl PopulationState {
    pub fn sleep_sources(&self) -> impl Iterator<Item = (EffectId, PopulationId)> + '_ {
        self.sleep_contacts
            .iter()
            .map(|contact| (contact.source, contact.actor))
    }
    pub fn thorn_sources(&self) -> impl Iterator<Item = (EffectId, PopulationId)> + '_ {
        self.thorn_contacts
            .iter()
            .map(|contact| (contact.source, contact.actor))
    }
    pub fn projectile_receipts(&self) -> impl Iterator<Item = EffectId> + '_ {
        self.consumed_projectiles
            .iter()
            .enumerate()
            .filter_map(|(slot, &generation)| EffectId::new(slot as u16, generation))
    }
    /// A second music source acquires its own lease even while another source
    /// already sleeps the actor. No renderer residency or original health byte
    /// doubles as this ownership state.
    pub fn sleep_near(
        &mut self,
        source: EffectId,
        level: LevelId,
        position: Position,
    ) -> Result<usize> {
        validate_source(source, position)?;
        let index = self.floor_index(level)?;
        if self
            .sleep_contacts
            .iter()
            .any(|contact| contact.source == source && contact.actor.level != level)
        {
            return Err(Error::State("boombox generation changed level"));
        }
        let candidates = self.floors[index]
            .actors
            .iter()
            .filter(|actor| can_sleep(actor, position))
            .filter(|actor| {
                self.sleep_contacts
                    .binary_search(&EffectContact {
                        source,
                        actor: actor.id,
                    })
                    .is_err()
            })
            .count();
        if self.sleep_contacts.len() + candidates > MAX_CONTACTS {
            return Err(Error::Capacity("boombox actor leases"));
        }
        for actor in self.floors[index]
            .actors
            .iter()
            .filter(|actor| can_sleep(actor, position))
        {
            preflight_body_release(&self.bodies, actor.companion_body)?;
        }
        for actor in &mut self.floors[index].actors {
            if !can_sleep(actor, position) {
                continue;
            }
            let contact = EffectContact {
                source,
                actor: actor.id,
            };
            let Err(insertion) = self.sleep_contacts.binary_search(&contact) else {
                continue;
            };
            self.sleep_contacts.insert(insertion, contact);
            actor.set_state(SLEEP_STATE);
            actor.motion.velocity = [0; 3];
            release_body(&mut self.bodies, &mut actor.companion_body)?;
        }
        Ok(candidates)
    }
    pub fn wake_level(&mut self, source: EffectId, level: LevelId) -> Result<usize> {
        if !source.valid() {
            return Err(Error::State("invalid boombox source generation"));
        }
        let index = self.floor_index(level)?;
        if self
            .sleep_contacts
            .iter()
            .any(|contact| contact.source == source && contact.actor.level != level)
        {
            return Err(Error::State("boombox wake source level"));
        }
        let before = self.sleep_contacts.len();
        let affected = self
            .sleep_contacts
            .iter()
            .filter(|contact| contact.source == source)
            .fold(0_u64, |bits, contact| bits | (1_u64 << contact.actor.local));
        self.sleep_contacts
            .retain(|contact| contact.source != source);
        for actor in &mut self.floors[index].actors {
            if affected & (1_u64 << actor.id.local) != 0
                && actor.state == SLEEP_STATE
                && !actor.retired
                && !self
                    .sleep_contacts
                    .iter()
                    .any(|contact| contact.actor == actor.id)
            {
                actor.set_state(0);
            }
        }
        Ok(before - self.sleep_contacts.len())
    }
    /// Thorn damage is once on entry for each source/actor pair. Exiting the
    /// region releases that contact, allowing a genuine later entry to hurt.
    pub fn rose_contacts(
        &mut self,
        source: EffectId,
        level: LevelId,
        position: Position,
    ) -> Result<usize> {
        validate_source(source, position)?;
        let index = self.floor_index(level)?;
        if self
            .thorn_contacts
            .iter()
            .any(|contact| contact.source == source && contact.actor.level != level)
        {
            return Err(Error::State("rose generation changed level"));
        }
        let kept = self
            .thorn_contacts
            .iter()
            .filter(|contact| {
                contact.source != source
                    || self
                        .actor(contact.actor)
                        .is_some_and(|actor| thorn_inside(actor, position))
            })
            .count();
        let entering = self.floors[index]
            .actors
            .iter()
            .filter(|actor| thorn_inside(actor, position))
            .filter(|actor| {
                self.thorn_contacts
                    .binary_search(&EffectContact {
                        source,
                        actor: actor.id,
                    })
                    .is_err()
            })
            .count();
        if kept + entering > MAX_CONTACTS {
            return Err(Error::Capacity("rose actor contact latches"));
        }
        for actor in self.floors[index]
            .actors
            .iter()
            .filter(|actor| thorn_inside(actor, position))
        {
            if self
                .thorn_contacts
                .binary_search(&EffectContact {
                    source,
                    actor: actor.id,
                })
                .is_err()
            {
                self.prepared_damage(actor.id, 1)?;
            }
        }
        let floors = &self.floors;
        self.thorn_contacts.retain(|contact| {
            contact.source != source
                || actor_in(floors, contact.actor)
                    .is_some_and(|actor| thorn_inside(actor, position))
        });
        let mut contacts = 0;
        for actor_index in 0..self.floors[index].actors.len() {
            let actor = &self.floors[index].actors[actor_index];
            if !thorn_inside(actor, position) {
                continue;
            }
            let id = actor.id;
            let contact = EffectContact { source, actor: id };
            let Err(insertion) = self.thorn_contacts.binary_search(&contact) else {
                continue;
            };
            self.thorn_contacts.insert(insertion, contact);
            self.damage(id, 1)?;
            contacts += 1;
        }
        Ok(contacts)
    }
    /// ProjectileFindCollisionExcludingOwner's logical predicate, in stable
    /// population order and wide coordinates. A real immune-body impact still
    /// consumes the projectile; it must not pass through Santa/ship/services.
    pub fn projectile_contact(
        &mut self,
        source: EffectId,
        owner: Option<ActorId>,
        level: LevelId,
        position: Position,
        damage: u8,
        catalog: &DefinitionCatalog,
    ) -> Result<bool> {
        validate_source(source, position)?;
        let receipt = self.consumed_projectiles[source.index()];
        if source.generation() < receipt {
            return Err(Error::State("stale projectile actor-contact generation"));
        }
        if source.generation() == receipt {
            return Ok(true);
        }
        if damage == 0 {
            return Err(Error::Bounds("zero projectile damage"));
        }
        let index = self.floor_index(level)?;
        if position.elevation <= 0 || position.elevation > 25 * 256 {
            return Ok(false);
        }
        let target = self.floors[index]
            .actors
            .iter()
            .find(|actor| {
                let Some(body) = actor.body else {
                    return false;
                };
                if actor.pending_director
                    || Some(body) == owner
                    || actor.retired
                    || actor.hidden
                    || !actor.collidable
                {
                    return false;
                }
                let definition = catalog.get(actor.kind);
                let reach = i64::from(definition.width / 2) - 1;
                let x =
                    (i64::from(position.x >> 8) - i64::from(actor.motion.position.x >> 8)).abs();
                let y =
                    (i64::from(position.y >> 8) - i64::from(actor.motion.position.y >> 8)).abs();
                let z = i64::from(position.elevation >> 8);
                let base = i64::from(actor.motion.position.elevation >> 8);
                x < reach
                    && y <= 8
                    && if z > base {
                        z < base + i64::from(definition.height) + 8
                    } else {
                        z + 7 > base
                    }
            })
            .map(|actor| actor.id);
        let Some(target) = target else {
            return Ok(false);
        };
        self.damage(target, damage)?;
        self.consumed_projectiles[source.index()] = source.generation();
        Ok(true)
    }
    /// Burps cancel eligible companion state in canonical owner interest; they
    /// are not arbitrary damage, sleep cancellation, or viewport draw visits.
    pub fn burp(&mut self, owner: SeatId, level: LevelId) -> Result<usize> {
        owner.validate().map_err(|_| Error::State("burp owner"))?;
        let index = self.floor_index(level)?;
        for actor in &self.floors[index].actors {
            if !actor.pending_director
                && actor.body.is_some()
                && !actor.retired
                && actor.companion
                && actor.state == 2
                && actor.interest.contains(owner)
            {
                preflight_body_release(&self.bodies, actor.companion_body)?;
            }
        }
        let mut changed = 0;
        for actor in &mut self.floors[index].actors {
            if !actor.pending_director
                && actor.body.is_some()
                && !actor.retired
                && actor.companion
                && actor.state == 2
                && actor.interest.contains(owner)
            {
                actor.set_state(0);
                actor.timer = 0;
                release_body(&mut self.bodies, &mut actor.companion_body)?;
                changed += 1;
            }
        }
        Ok(changed)
    }
    pub fn retire_effect(&mut self, source: EffectId, level: LevelId) -> Result<usize> {
        if self
            .thorn_contacts
            .iter()
            .any(|contact| contact.source == source && contact.actor.level != level)
        {
            return Err(Error::State("retired rose source level"));
        }
        let woke = self.wake_level(source, level)?;
        let before = self.thorn_contacts.len();
        self.thorn_contacts
            .retain(|contact| contact.source != source);
        Ok(woke + before - self.thorn_contacts.len())
    }
    pub(super) fn discard_actor_contacts(&mut self, id: PopulationId) {
        self.sleep_contacts.retain(|contact| contact.actor != id);
        self.thorn_contacts.retain(|contact| contact.actor != id);
    }
    pub(super) fn discard_removed_contacts(&mut self) {
        let floors = &self.floors;
        for contacts in [&mut self.sleep_contacts, &mut self.thorn_contacts] {
            contacts.retain(|contact| {
                actor_in(floors, contact.actor).is_some_and(|actor| !actor.retired)
            });
        }
    }
    pub(super) fn validate_effect_contacts(&self) -> Result<()> {
        for contacts in [&self.sleep_contacts, &self.thorn_contacts] {
            if contacts.len() > MAX_CONTACTS || contacts.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(Error::State("actor-effect contact order/capacity"));
            }
            let mut previous: Option<EffectContact> = None;
            for contact in contacts {
                let actor = self
                    .actor(contact.actor)
                    .ok_or(Error::State("actor-effect contact has no stable actor"))?;
                if !contact.source.valid()
                    || actor.retired
                    || actor.pending_director
                    || previous.is_some_and(|last| {
                        last.source == contact.source && last.actor.level != contact.actor.level
                    })
                {
                    return Err(Error::State("actor-effect generation/level/owner"));
                }
                previous = Some(*contact);
            }
        }
        for contact in &self.sleep_contacts {
            if self
                .actor(contact.actor)
                .is_none_or(|actor| actor.state != SLEEP_STATE)
            {
                return Err(Error::State("sleep lease has no sleeping actor"));
            }
        }
        for floor in &self.floors {
            for actor in &floor.actors {
                if actor.state == SLEEP_STATE
                    && !actor.retired
                    && !floor.failed.contains(actor.id.local)
                    && !self
                        .sleep_contacts
                        .iter()
                        .any(|contact| contact.actor == actor.id)
                {
                    return Err(Error::State("sleeping actor has no effect lease"));
                }
            }
        }
        Ok(())
    }
}

fn validate_source(source: EffectId, position: Position) -> Result<()> {
    if !source.valid() {
        return Err(Error::State("invalid actor-effect generation"));
    }
    position
        .validate()
        .map_err(|_| Error::Bounds("actor-effect position"))
}
fn can_sleep(actor: &PersistentActor, position: Position) -> bool {
    if actor.pending_director
        || actor.body.is_none()
        || actor.retired
        || actor.mail_order
        || !actor.kind.supports_sleep()
    {
        return false;
    }
    if actor.state != SLEEP_STATE
        && (matches!(actor.kind, Kind::Cupid | Kind::Mole) && actor.state == 3
            || actor.kind == Kind::Wiseman && actor.state < 2
            || matches!(actor.kind, Kind::OperaSinger | Kind::Wizard) && actor.state & 0xfc != 4)
    {
        return false;
    }
    (i64::from(actor.motion.position.x >> 8) - i64::from(position.x >> 8)).abs() < 320
        && (i64::from(actor.motion.position.y >> 8) - i64::from(position.y >> 8)).abs() < 224
}
fn thorn_inside(actor: &PersistentActor, position: Position) -> bool {
    !actor.pending_director
        && actor.body.is_some()
        && !actor.retired
        && !actor.hidden
        && actor.collidable
        && (i64::from(actor.motion.position.x >> 8) - i64::from(position.x >> 8)).abs() < 25
        && (i64::from(actor.motion.position.y >> 8) - i64::from(position.y >> 8)).abs() < 10
        && (i64::from(actor.motion.position.elevation) - i64::from(position.elevation)).abs()
            <= 10 * 256
}
fn actor_in(floors: &[PopulationFloor], id: PopulationId) -> Option<&PersistentActor> {
    let floor = &floors[floors
        .binary_search_by_key(&id.level, |floor| floor.level)
        .ok()?];
    floor
        .actors
        .binary_search_by_key(&id.local, |actor| actor.id.local)
        .ok()
        .map(|index| &floor.actors[index])
}
