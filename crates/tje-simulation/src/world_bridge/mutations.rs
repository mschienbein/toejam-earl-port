use super::*;
use crate::loading::DoorwayCrossing;
use crate::players::{ActorEffect, PlayerWorld, WorldCommit, WorldPresent};
use crate::resources::PresentKind;

impl<R: Resources> WorldBridge<'_, R> {
    pub fn bind_props(&mut self) -> PlayerResult<()> {
        for slot in 0..64 {
            let view = if self.effects.changed & (1_u64 << slot) != 0 {
                self.effects.values[slot]
            } else {
                self.effects.base[slot]
            };
            if let Some(view) = view {
                bind_prop(self.loading, view)?;
            }
        }
        Ok(())
    }
    fn checked_effect(&self, id: EffectId, level: LevelId) -> PlayerResult<EffectView> {
        let view = self.effects.get(id).ok_or(PlayerError::Rejected(
            "effect generation is not live in this phase",
        ))?;
        if view.effect.level != level {
            return Err(PlayerError::Rejected("effect changed world level"));
        }
        Ok(view)
    }
    fn effect_owner(
        &self,
        id: EffectId,
        level: LevelId,
        kind: EffectKind,
    ) -> PlayerResult<(EffectView, SeatId)> {
        let view = self.checked_effect(id, level)?;
        if view.effect.kind != kind {
            return Err(PlayerError::Rejected(
                "effect kind does not match world operation",
            ));
        }
        let EffectOwner::Seat(owner) = view.effect.owner else {
            return Err(PlayerError::Rejected(
                "effect does not belong to a hero seat",
            ));
        };
        Ok((view, owner))
    }
    fn checked_prop_pose(
        &self,
        owner: SeatId,
        level: LevelId,
        position: Position,
    ) -> PlayerResult<()> {
        self.require_owner(owner, level)?;
        position
            .validate()
            .map_err(|_| PlayerError::Geometry("prop pose"))?;
        if self
            .dimensions(level)?
            .world_cell(position.x >> 8, position.y >> 8)
            .is_none()
        {
            return Err(PlayerError::Rejected("prop lies outside the loaded level"));
        }
        Ok(())
    }
    fn clear_elevator_pose(
        &self,
        level: LevelId,
        mut position: Position,
    ) -> PlayerResult<Position> {
        let barriers = self
            .loading
            .elevator_barriers(self.world, level)
            .map_err(map_world)?;
        let dimensions = self.dimensions(level)?;
        for _ in 0..usize::from(dimensions.width()) * 16 {
            if dimensions
                .world_cell(position.x >> 8, position.y >> 8)
                .is_none()
            {
                return Err(PlayerError::Rejected("present placement leaves the level"));
            }
            if !barriers
                .iter()
                .any(|barrier| barrier.contains(level, (position.x >> 8, position.y >> 8)))
            {
                return Ok(position);
            }
            position.x = position
                .x
                .checked_add(8 * 256)
                .ok_or(PlayerError::Geometry("prop X placement overflow"))?;
        }
        Err(PlayerError::Rejected("no elevator-clear present placement"))
    }
    fn prop_landing_is_safe(&self, hero: SeatId, landing: Landing) -> PlayerResult<bool> {
        let geometry = self.geometry();
        geometry.safe_landing(hero, self.hero(hero)?.archetype, landing)
    }
}

impl<R: Resources> PlayerWorld for WorldBridge<'_, R> {
    fn commit_present(&mut self, present: WorldPresent) -> PlayerResult<WorldCommit> {
        match present {
            WorldPresent::SpawnEarthling {
                owner,
                level,
                position,
                choice,
            } => {
                self.require_owner(owner, level)?;
                let environment = PopulationEnvironment {
                    world: self.world,
                    loading: self.loading,
                    resources: self.resources.terrain,
                    animation_step: self.resources.animation_step,
                };
                self.population
                    .spawn_present(
                        owner,
                        level,
                        position,
                        choice,
                        &environment,
                        self.resources.earthlings,
                    )
                    .map_err(map_world)?;
                Ok(WorldCommit {
                    effect_position: None,
                })
            }
            WorldPresent::Togetherness {
                owner,
                target,
                destination,
            } => {
                let source = self.hero(owner)?;
                self.require_owner(
                    owner,
                    source
                        .level
                        .ok_or(PlayerError::Rejected("togetherness source has no level"))?,
                )?;
                let recipient = self.require_owner(target, destination.level)?;
                if owner == target
                    || destination.position.x != recipient.position.x
                    || destination.position.y != recipient.position.y
                    || destination.position.elevation != 70 * 256
                    || self.blocked(
                        owner,
                        source.archetype,
                        destination.level,
                        destination.position,
                    )?
                {
                    return Err(PlayerError::Rejected(
                        "togetherness recipient or landing changed",
                    ));
                }
                if self.terrain(
                    destination.level,
                    destination.position.x >> 8,
                    destination.position.y >> 8,
                )? & 0xf0
                    > 0x50
                {
                    return Err(PlayerError::Rejected(
                        "togetherness destination is unsupported",
                    ));
                }
                self.loading
                    .relocate_world(owner, destination, self.world, self.population)?;
                self.hero_positions[owner.index()] =
                    Some((destination.level, destination.position));
                Ok(WorldCommit {
                    effect_position: None,
                })
            }
            WorldPresent::OpenDoorway {
                owner,
                level,
                position,
                choice,
            } => {
                self.require_owner(owner, level)?;
                let x = (position.x >> 8).div_euclid(8) * 8
                    + if choice & 1 != 0 { -32 } else { 32 }
                    + 16;
                let y = (position.y >> 8).div_euclid(8) * 8 - 14;
                let accepted = self.clear_elevator_pose(
                    level,
                    Position {
                        x: x * 256,
                        y: y * 256,
                        elevation: 0,
                    },
                )?;
                self.checked_prop_pose(owner, level, accepted)?;
                self.loading.open_doorway(owner, level, accepted)?;
                Ok(WorldCommit {
                    effect_position: Some(accepted),
                })
            }
            WorldPresent::OpenTelephone {
                owner,
                level,
                position,
            } => {
                self.require_owner(owner, level)?;
                let mut accepted = Position {
                    x: position.x,
                    y: position
                        .y
                        .checked_add(10 * 256)
                        .ok_or(PlayerError::Geometry("telephone Y overflow"))?,
                    elevation: 0,
                };
                let maximum_steps = usize::from(self.dimensions(level)?.height()) * 64;
                for step in 0..=maximum_steps {
                    self.checked_prop_pose(owner, level, accepted)?;
                    let overlaps = self.heroes.iter().flatten().any(|hero| {
                        let (other_level, other) = self.current_pose(*hero);
                        hero.seat != owner
                            && hero.life == LifeState::Alive
                            && other_level == Some(level)
                            && (i64::from(accepted.x >> 8) - i64::from(other.x >> 8)).abs() < 16
                            && (i64::from(accepted.y >> 8) - i64::from(other.y >> 8)).abs() < 8
                    });
                    if !overlaps {
                        self.loading.open_telephone(owner, level, accepted)?;
                        return Ok(WorldCommit {
                            effect_position: Some(accepted),
                        });
                    }
                    if step == maximum_steps {
                        break;
                    }
                    accepted.y = accepted
                        .y
                        .checked_add(256)
                        .ok_or(PlayerError::Geometry("telephone clearance overflow"))?;
                }
                Err(PlayerError::Rejected("no four-hero telephone clearance"))
            }
            WorldPresent::OpenDecoy {
                owner,
                level,
                position,
            } => {
                self.require_owner(owner, level)?;
                let accepted = self.clear_elevator_pose(
                    level,
                    Position {
                        x: position.x,
                        y: position
                            .y
                            .checked_add(256)
                            .ok_or(PlayerError::Geometry("decoy Y overflow"))?,
                        elevation: 0,
                    },
                )?;
                self.checked_prop_pose(owner, level, accepted)?;
                if self.terrain(level, accepted.x >> 8, accepted.y >> 8)? & 0xf0 > 0x50 {
                    return Err(PlayerError::Rejected("decoy requires supported terrain"));
                }
                Ok(WorldCommit {
                    effect_position: Some(accepted),
                })
            }
        }
    }
    fn commit_arrival(&mut self, seat: SeatId, landing: Landing) -> PlayerResult<()> {
        self.hero(seat)?;
        if !self.prop_landing_is_safe(seat, landing)? {
            return Err(PlayerError::Geometry("unsafe world arrival"));
        }
        self.loading
            .arrive_hero_world(seat, landing, self.world, self.population)?;
        self.hero_life[seat.index()] = Some((LifeState::Alive, true));
        self.hero_positions[seat.index()] = Some((landing.level, landing.position));
        Ok(())
    }
    fn release_hero(&mut self, seat: SeatId) -> PlayerResult<()> {
        self.hero(seat)?;
        self.loading
            .release_hero_world(seat, self.world, self.population)?;
        self.hero_life[seat.index()] = Some((LifeState::Eliminated, false));
        Ok(())
    }
    fn effect_created(&mut self, id: EffectId, effect: Effect) -> PlayerResult<()> {
        let view = EffectView { id, effect };
        validate_effect(self.world, self.population, self.heroes, view)?;
        let current = if self.effects.changed & (1_u64 << id.slot()) != 0 {
            self.effects.values[id.index()]
        } else {
            self.effects.base[id.index()]
        };
        if current.is_some_and(|old| old.id != id && old.id.generation() > id.generation()) {
            return Err(PlayerError::Rejected(
                "effect birth would replace a newer generation",
            ));
        }
        if let Some(old) = current
            && old.id != id
        {
            return Err(PlayerError::Rejected(
                "effect birth requires prior retirement",
            ));
        }
        bind_prop(self.loading, view)?;
        self.effects.set(id, Some(view));
        Ok(())
    }
    fn apply_actor_effect(&mut self, request: ActorEffect) -> PlayerResult<bool> {
        match request {
            ActorEffect::Projectile {
                effect,
                owner,
                level,
                position,
                damage,
            } => {
                let view = self.checked_effect(effect, level)?;
                if view.effect.owner != owner
                    || !matches!(
                        view.effect.kind,
                        EffectKind::Tomato
                            | EffectKind::Slingshot
                            | EffectKind::RainTomato
                            | EffectKind::ControlRemap
                    )
                {
                    return Err(PlayerError::Rejected("projectile owner/kind changed"));
                }
                if view.effect.kind == EffectKind::ControlRemap {
                    return Ok(false);
                }
                self.population
                    .projectile_contact(
                        effect,
                        if let EffectOwner::Actor(actor) = owner {
                            Some(actor)
                        } else {
                            None
                        },
                        level,
                        position,
                        damage,
                        self.resources.earthlings,
                    )
                    .map_err(map_world)
            }
            ActorEffect::Roses {
                effect,
                owner,
                level,
                position,
            } => {
                let (view, actual_owner) =
                    self.effect_owner(effect, level, EffectKind::RoseBush)?;
                if owner != actual_owner {
                    return Err(PlayerError::Rejected("rose owner changed"));
                }
                if !view.effect.animation_complete {
                    return Ok(false);
                }
                Ok(self
                    .population
                    .rose_contacts(effect, level, position)
                    .map_err(map_world)?
                    != 0)
            }
            ActorEffect::Boombox {
                effect,
                level,
                position,
            } => {
                self.effect_owner(effect, level, EffectKind::Boombox)?;
                Ok(self
                    .population
                    .sleep_near(effect, level, position)
                    .map_err(map_world)?
                    != 0)
            }
            ActorEffect::WakeBoombox { effect, level } => {
                self.effect_owner(effect, level, EffectKind::Boombox)?;
                Ok(self
                    .population
                    .wake_level(effect, level)
                    .map_err(map_world)?
                    != 0)
            }
            ActorEffect::Burp {
                owner,
                level,
                position,
            } => {
                self.require_owner(owner, level)?;
                position
                    .validate()
                    .map_err(|_| PlayerError::Geometry("burp world position"))?;
                Ok(self.population.burp(owner, level).map_err(map_world)? != 0)
            }
            ActorEffect::ClearMoleContacts { seat, level } => {
                self.require_owner(seat, level)?;
                if self.population.retained()[seat.index()] != Some(level) {
                    return Err(PlayerError::Rejected(
                        "mole-contact pickup floor is not retained by the hero",
                    ));
                }
                let changed = self
                    .population
                    .floors()
                    .iter()
                    .find(|floor| floor.level == level)
                    .ok_or(PlayerError::Geometry(
                        "mole-contact pickup population floor is absent",
                    ))?
                    .actors
                    .iter()
                    .any(|actor| actor.kind == Kind::Mole && actor.mole_contacts.contains(seat));
                self.population
                    .clear_mole_contacts(seat, level)
                    .map_err(map_world)?;
                Ok(changed)
            }
            ActorEffect::ServiceCollision { actor, collidable } => {
                self.population
                    .set_service_collision(actor, collidable)
                    .map_err(map_world)?;
                Ok(true)
            }
            ActorEffect::Opera { seats } => {
                seats
                    .validate()
                    .map_err(|_| PlayerError::Invalid("opera audience mask"))?;
                if seats.bits() & !self.loading.config().heroes.bits() != 0 {
                    return Err(PlayerError::Rejected(
                        "opera audience contains a nonhero role",
                    ));
                }
                Ok(self.population.damage_interest(seats).map_err(map_world)? != 0)
            }
            ActorEffect::Retire { effect, level } => {
                self.checked_effect(effect, level)?;
                let actor_changes = self
                    .population
                    .retire_effect(effect, level)
                    .map_err(map_world)?;
                let prop_changed = self.loading.retire_effect(effect, level)?;
                self.effects.set(effect, None);
                Ok(actor_changes != 0 || prop_changed)
            }
        }
    }
    fn doorway_crossing(
        &mut self,
        effect: EffectId,
        owner: SeatId,
        hero: SeatId,
        level: LevelId,
        position: Position,
    ) -> PlayerResult<Option<Landing>> {
        let (view, actual_owner) = self.effect_owner(effect, level, EffectKind::Doorway)?;
        if owner != actual_owner {
            return Err(PlayerError::Rejected("doorway owner changed"));
        }
        self.require_owner(hero, level)?;
        bind_prop(self.loading, view)?;
        let geometry = GeometryView {
            world: self.world,
            loading: self.loading,
            terrain: self.resources.terrain,
            heroes: self.heroes,
            actors: ActorGeometry::Live(self.population, self.resources.earthlings),
            effects: GeometryEffects::Overlay(&self.effects),
            hero_positions: &self.hero_positions,
            hero_life: &self.hero_life,
        };
        // Loading's crossing method mutates only its saved portal continuation.
        // Excluding loading from the collision closure avoids aliasing that borrow.
        let safety = PortalSafety::capture(&geometry, hero, level)?;
        let result = self.loading.doorway_crossing(
            DoorwayCrossing {
                effect,
                owner,
                hero,
                level,
                position,
                tick: self.tick,
            },
            self.world,
            self.resources.terrain,
            |landing| {
                safety.blocked(
                    self.world,
                    self.population,
                    self.resources.earthlings,
                    self.resources.terrain,
                    landing,
                )
            },
        )?;
        if let Some(landing) = result {
            self.hero_positions[hero.index()] = Some((landing.level, landing.position));
        }
        Ok(result)
    }
    fn answer_telephone(
        &mut self,
        effect: EffectId,
        hero: SeatId,
        level: LevelId,
    ) -> PlayerResult<()> {
        let (view, owner) = self.effect_owner(effect, level, EffectKind::Telephone)?;
        let hero_view = self.require_owner(hero, level)?;
        if (i64::from(hero_view.position.x >> 8) - i64::from(view.effect.position.x >> 8)).abs()
            >= 16
            || (i64::from(hero_view.position.y >> 8) - i64::from(view.effect.position.y >> 8)).abs()
                >= 8
        {
            return Err(PlayerError::Rejected(
                "telephone answering hero is not at the receiver",
            ));
        }
        bind_prop(self.loading, view)?;
        self.loading.answer_telephone(effect, owner, hero, level)
    }
    fn control_remap(&self, selection: u8) -> PlayerResult<[u8; 16]> {
        self.resources.remaps.directions(selection)
    }
    fn choose_present(&self, rng: &mut StageRng) -> PlayerResult<PresentKind> {
        self.resources.presents.choose(rng)
    }
    fn contact_damage(&self, kind: Kind) -> PlayerResult<u8> {
        Ok(self.resources.earthlings.get(kind).damage)
    }
}

fn bind_prop(loading: &mut LoadingState, view: EffectView) -> PlayerResult<()> {
    let owner = match view.effect.owner {
        EffectOwner::Seat(owner) => owner,
        _ if matches!(
            view.effect.kind,
            EffectKind::Doorway | EffectKind::Telephone
        ) =>
        {
            return Err(PlayerError::Rejected("world prop requires hero ownership"));
        }
        _ => return Ok(()),
    };
    match view.effect.kind {
        EffectKind::Doorway => {
            loading.bind_doorway(view.id, owner, view.effect.level, view.effect.position)
        }
        EffectKind::Telephone => {
            loading.bind_telephone(view.id, owner, view.effect.level, view.effect.position)
        }
        _ => Ok(()),
    }
}
