//! Concrete adapters borrow the shared services. No copied player/AI/world
//! algorithms or original RAM object records sit behind the director rules.
use crate::{
    ActorId, Error, Kind, LevelId, OverlordState, PopulationId, Result, command, placement,
    state::{self, PossessionInfo, Spawn},
    traps,
};
use tje_four_player::{
    CoopState, SeatId,
    state::{LifeState, Overlay},
};
use tje_simulation::{
    players::{PlayerGeometry, PlayerMode, PlayerServices},
    population::{DefinitionCatalog, Environment, HeroView, PopulationState, director},
    session::Roles,
};
use tje_world::{Resources, world::WorldState};

pub struct Shared<'a, R, E> {
    pub world: &'a WorldState,
    pub resources: &'a R,
    pub heroes: &'a [Option<HeroView>; 4],
    pub population: &'a mut PopulationState,
    pub catalog: &'a DefinitionCatalog,
    pub roles: Roles,
    pub environment: &'a E,
    pub scratch: &'a mut placement::Scratch,
}
impl<R: Resources, E: Environment> state::World for Shared<'_, R, E> {
    fn possession(&self, actor: ActorId) -> Result<PossessionInfo> {
        let status = self
            .population
            .control_status(actor)
            .map_err(|_| Error::StaleActor)?;
        Ok(PossessionInfo {
            kind: status.kind,
            level: status.level,
            enterable: status.enterable,
            retainable: status.retainable,
        })
    }
    fn validate_placement(
        &mut self,
        level: LevelId,
        point: placement::Point,
        kind: Option<Kind>,
        ignore: Option<PopulationId>,
    ) -> Result<()> {
        placement::Context {
            world: self.world,
            resources: self.resources,
            heroes: self.heroes,
            population: self.population,
            catalog: self.catalog,
        }
        .validate_ignoring(level, point, kind, ignore, self.scratch)
    }
    fn reserve_spawn(
        &mut self,
        source: u32,
        kind: Kind,
        level: LevelId,
        point: placement::Point,
    ) -> Result<PopulationId> {
        self.population
            .reserve_director(
                director::SpawnRequest {
                    roles: self.roles,
                    owner: crate::DIRECTOR,
                    source,
                    level,
                    kind,
                    position: tje_four_player::state::Position {
                        x: point.x * 256,
                        y: point.y * 256,
                        elevation: 0,
                    },
                },
                self.environment,
                self.catalog,
            )
            .map(|ticket| ticket.id)
            .map_err(|_| Error::Shared)
    }
    fn activate_spawn(&mut self, spawn: &Spawn) -> Result<()> {
        self.population
            .activate_director(ticket(spawn))
            .map(|_| ())
            .map_err(|_| Error::Shared)
    }
    fn cancel_spawn(&mut self, spawn: &Spawn) -> Result<()> {
        self.population
            .cancel_director(ticket(spawn))
            .map(|_| ())
            .map_err(|_| Error::Shared)
    }
    fn spawn_alive(&self, spawn: &Spawn) -> bool {
        self.population.director_ticket_live(ticket(spawn))
    }
    fn validate_spawn(&self, spawn: &Spawn) -> Result<()> {
        let slot = self
            .population
            .director_slots()
            .iter()
            .flatten()
            .find(|slot| slot.ticket == ticket(spawn))
            .ok_or(Error::StaleActor)?;
        let actor = self
            .population
            .actor(spawn.population)
            .ok_or(Error::StaleActor)?;
        let pending = spawn.stage == state::SpawnStage::Telegraph;
        if !self.spawn_alive(spawn)
            || actor.kind != spawn.kind
            || slot.pending != pending
            || pending && actor.pixels() != (spawn.point.x, spawn.point.y)
        {
            return Err(Error::Invalid("director/population spawn receipt"));
        }
        Ok(())
    }
    fn next_source(&self) -> u32 {
        self.population.next_director_source()
    }
}
fn ticket(spawn: &Spawn) -> director::SpawnTicket {
    director::SpawnTicket {
        id: spawn.population,
        owner: spawn.owner,
        source: spawn.source,
    }
}

pub fn intent(state: &OverlordState) -> Option<director::DecisionIntent> {
    state.control.map(|control| director::DecisionIntent {
        actor: control.actor,
        ability: control.ability,
        direction: match control.direction {
            command::Direction::Neutral => director::Direction::Neutral,
            command::Direction::Up => director::Direction::Up,
            command::Direction::Down => director::Direction::Down,
            command::Direction::Left => director::Direction::Left,
            command::Direction::Right => director::Direction::Right,
        },
    })
}
pub fn acknowledge(
    state: &mut OverlordState,
    actor: ActorId,
    receipt: director::StepReceipt,
) -> Result<()> {
    if receipt.decision.ability_used {
        state.acknowledge_ability(actor)?;
    }
    Ok(())
}
pub fn voluntary_scales(state: &OverlordState) -> [tje_simulation::players::VoluntaryScale; 4] {
    SeatId::ALL.map(|seat| {
        if seat.index() < 3 && state.snares[seat.index()] != 0 {
            tje_simulation::players::VoluntaryScale::Snared
        } else {
            tje_simulation::players::VoluntaryScale::Normal
        }
    })
}

pub fn pause_agreed(
    state: &OverlordState,
    coop: &CoopState,
    players: &PlayerServices,
) -> Result<bool> {
    if coop.config() != &state.config().shared.coop {
        return Err(Error::Role);
    }
    Ok(state.pause_requested && players.pause_agreed(coop))
}
/// Deterministic three-button selection: nearest eligible main body within
/// thirty-two pixels, then stable generation-checked ID. No render hit testing.
pub fn select_at(
    population: &PopulationState,
    level: LevelId,
    point: placement::Point,
) -> Option<ActorId> {
    population
        .floors()
        .iter()
        .filter(|floor| floor.level == level)
        .flat_map(|floor| &floor.actors)
        .filter_map(|actor| {
            let body = actor.body?;
            if !population.control_status(body).ok()?.enterable {
                return None;
            }
            let dx = i64::from(point.x) - i64::from(actor.pixels().0);
            let dy = i64::from(point.y) - i64::from(actor.pixels().1);
            let distance = dx * dx + dy * dy;
            (distance <= 32 * 32).then_some((distance, body))
        })
        .min_by_key(|&(distance, body)| (distance, body))
        .map(|(_, body)| body)
}

pub struct TrapAdapter<'a, R, G> {
    pub coop: &'a mut CoopState,
    pub players: &'a mut PlayerServices,
    pub geometry: &'a G,
    pub placement: placement::Context<'a, R>,
    pub scratch: &'a mut placement::Scratch,
}
impl<R: Resources, G: PlayerGeometry> traps::Sink for TrapAdapter<'_, R, G> {
    fn hero(&self, seat: SeatId) -> Option<traps::Hero> {
        if !crate::HEROES.contains(seat) {
            return None;
        }
        let hero = &self.coop.players[seat.index()];
        if hero.life != LifeState::Alive {
            return None;
        }
        let runtime = &self.players.runtime[seat.index()];
        Some(traps::Hero {
            seat,
            level: hero.level?,
            x: hero.motion.position.x >> 8,
            y: hero.motion.position.y >> 8,
            eligible: runtime.mode == PlayerMode::Grounded
                && self.players.can_receive_effect(self.coop, seat)
                && runtime.service.is_none()
                && runtime.fall_token.is_none()
                && hero.ui.overlay == Overlay::Closed
                && hero.counters.respawn_suppression_ticks == 0
                && hero.motion.position.elevation == 0,
        })
    }
    fn may_arm(&mut self, trap: &traps::Trap) -> Result<bool> {
        match self
            .placement
            .validate(trap.level, trap.point, None, self.scratch)
        {
            Ok(()) => Ok(true),
            Err(Error::Placement(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }
    fn bump(&mut self, seat: SeatId, level: LevelId, x: i32, y: i32) -> Result<bool> {
        if self.placement.safe_zone(level, placement::Point { x, y })? {
            return Ok(false);
        }
        self.players
            .try_ground_bump(
                self.coop,
                seat,
                tje_simulation::players::Landing {
                    level,
                    position: tje_four_player::state::Position {
                        x: x * 256,
                        y: y * 256,
                        elevation: 0,
                    },
                },
                self.geometry,
            )
            .map_err(|_| Error::Shared)
    }
}
