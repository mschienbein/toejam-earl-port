//! Borrowed S6 world services. PlayerServices is never stored here: its caller
//! freezes compact views before lending it mutably to player/present execution.
mod deltas;
mod mutations;
mod population_environment;
pub use deltas::{sync_population_deltas, validate_population_deltas};
use population_environment::PopulationEnvironment;
mod portal;
use portal::PortalSafety;
#[cfg(test)]
mod tests;

use crate::loading::LoadingState;
use crate::players::{
    Landing, PlayerError, PlayerGeometry, PlayerMode, PlayerResult, PlayerServices,
};
use crate::population::{
    self, AnimationCursor, AnimationSample, DefinitionCatalog, Kind, PersistentActor,
    PopulationState,
};
use crate::resources::{
    Effect, EffectKind, EffectOwner, EffectSource, PresentCatalog, ResourceState,
};
use crate::{ActorId, EffectId, LevelId, SeatId, SeatMask};
use tje_four_player::{
    Archetype, CoopState,
    state::{LifeState, Position},
};
use tje_mem::{AddressSpace, Bus, Revision, rom};
use tje_world::{
    Dimensions, Resources,
    random::StageRng,
    terrain::{
        DoorwaySupport, ElevatorContact, Footprint, TerrainCache, TerrainMask, position_in_cell,
    },
    world::WorldState,
};

pub type ActorViews = [Option<ActorBody>; population::MAX_ACTIVE_BODIES as usize];
pub type EffectViews = [Option<EffectView>; 64];
pub type AnimationStep<'a> =
    dyn Fn(Kind, u8, &mut AnimationCursor) -> tje_world::Result<AnimationSample> + 'a;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeroBody {
    pub seat: SeatId,
    pub archetype: Archetype,
    pub life: LifeState,
    pub level: Option<LevelId>,
    pub position: Position,
    pub collidable: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorBody {
    pub actor: ActorId,
    pub level: LevelId,
    pub position: Position,
    pub width: u8,
    pub height: u8,
    pub collidable: bool,
}
impl ActorBody {
    fn from_actor(actor: &PersistentActor, catalog: &DefinitionCatalog) -> Option<Self> {
        if actor.pending_director {
            return None;
        }
        let definition = catalog.get(actor.kind);
        Some(Self {
            actor: actor.body?,
            level: actor.id.level,
            position: actor.motion.position,
            width: definition.width,
            height: definition.height,
            collidable: actor.collidable && !actor.hidden && !actor.retired,
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectView {
    pub id: EffectId,
    pub effect: Effect,
}

pub fn capture_heroes(coop: &CoopState, services: &PlayerServices) -> [Option<HeroBody>; 4] {
    SeatId::ALL.map(|seat| {
        let player = &coop.players[seat.index()];
        (player.life != LifeState::Disabled).then_some(HeroBody {
            seat,
            archetype: player.character.archetype,
            life: player.life,
            level: player.level,
            position: player.motion.position,
            collidable: player.life == LifeState::Alive
                && player.health != 0
                && !matches!(
                    services.runtime[seat.index()].mode,
                    PlayerMode::Dying | PlayerMode::Falling | PlayerMode::AwaitingFloor
                ),
        })
    })
}
pub fn capture_effects(resources: &ResourceState) -> EffectViews {
    let mut views = [None; 64];
    for (id, effect) in resources.effects() {
        views[id.index()] = Some(EffectView {
            id,
            effect: *effect,
        });
    }
    views
}
/// Contacts can use this frozen collider set while PopulationState is mutably
/// executing thinkers/contacts. No immutable population borrow is retained.
pub fn capture_actor_bodies(
    population: &PopulationState,
    catalog: &DefinitionCatalog,
) -> PlayerResult<ActorViews> {
    let mut views = [None; population::MAX_ACTIVE_BODIES as usize];
    for floor in population.floors() {
        for actor in &floor.actors {
            let Some(body) = ActorBody::from_actor(actor, catalog) else {
                continue;
            };
            let index = body
                .actor
                .slot()
                .checked_sub(ActorId::POPULATION_START)
                .ok_or(PlayerError::Invalid("population body domain"))?
                as usize;
            let slot = views
                .get_mut(index)
                .ok_or(PlayerError::Invalid("population collider capacity"))?;
            if slot.is_some() || !body.actor.valid() {
                return Err(PlayerError::Invalid(
                    "population collider generation/duplicate",
                ));
            }
            *slot = Some(body);
        }
    }
    Ok(views)
}

#[derive(Clone, Copy)]
pub struct BridgeViews<'a> {
    pub heroes: &'a [Option<HeroBody>; 4],
    pub effects: &'a EffectViews,
}
/// Immutable, checked control-remap catalogue. These are runtime imports, not
/// serialized cartridge bytes or an embedded substitute permutation table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlRemaps {
    rows: [[u8; 16]; 4],
}
impl ControlRemaps {
    pub fn from_rows(rows: [[u8; 16]; 4]) -> PlayerResult<Self> {
        const LEGAL: [usize; 8] = [1, 2, 4, 5, 6, 8, 9, 10];
        for row in rows {
            if row[0] != 0
                || row.iter().any(|&value| value > 15)
                || LEGAL
                    .iter()
                    .any(|&input| !LEGAL.contains(&usize::from(row[input])))
                || row[11..].iter().any(|&value| value != 0)
            {
                return Err(PlayerError::Invalid("control remap mask domain"));
            }
        }
        Ok(Self { rows })
    }
    pub fn from_rom(memory: &AddressSpace) -> PlayerResult<Self> {
        Self::import(memory, memory.revision(), memory.rom().len())
    }
    pub fn from_verified_bus(bus: &impl Bus, revision: Revision) -> PlayerResult<Self> {
        Self::import(bus, revision, revision.rom_size())
    }
    fn import(bus: &impl Bus, revision: Revision, length: usize) -> PlayerResult<Self> {
        let address = rom::control_remap_directions(revision).address();
        if (address as usize)
            .checked_add(40)
            .is_none_or(|end| end > length)
        {
            return Err(PlayerError::Geometry("control remap ROM range"));
        }
        Self::from_rows(std::array::from_fn(|row| {
            std::array::from_fn(|mask| {
                if (1..=10).contains(&mask) {
                    bus.read_u8(address + row as u32 * 10 + mask as u32 - 1)
                } else {
                    0
                }
            })
        }))
    }
    pub fn directions(&self, selection: u8) -> PlayerResult<[u8; 16]> {
        self.rows
            .get(usize::from(selection))
            .copied()
            .ok_or(PlayerError::Invalid("control remap selection"))
    }
}
pub struct BridgeResources<'a, R: Resources> {
    pub terrain: &'a R,
    pub earthlings: &'a DefinitionCatalog,
    pub presents: &'a PresentCatalog,
    pub remaps: &'a ControlRemaps,
    pub animation_step: &'a AnimationStep<'a>,
}

/// Effect births/retirements inside a mutably owned player tick update this
/// fixed delta overlay; the borrowed input snapshot itself is immutable.
struct EffectOverlay<'a> {
    base: &'a EffectViews,
    changed: u64,
    values: EffectViews,
}
impl<'a> EffectOverlay<'a> {
    fn new(base: &'a EffectViews) -> Self {
        Self {
            base,
            changed: 0,
            values: [None; 64],
        }
    }
    fn get(&self, id: EffectId) -> Option<EffectView> {
        if !id.valid() {
            return None;
        }
        let value = if self.changed & (1_u64 << id.slot()) != 0 {
            self.values[id.index()]
        } else {
            self.base[id.index()]
        };
        value.filter(|view| view.id == id)
    }
    fn set(&mut self, id: EffectId, value: Option<EffectView>) {
        self.changed |= 1_u64 << id.slot();
        self.values[id.index()] = value;
    }
}

pub struct WorldBridge<'a, R: Resources> {
    world: &'a mut WorldState,
    population: &'a mut PopulationState,
    loading: &'a mut LoadingState,
    resources: BridgeResources<'a, R>,
    heroes: &'a [Option<HeroBody>; 4],
    effects: EffectOverlay<'a>,
    tick: u64,
    hero_positions: [Option<(LevelId, Position)>; 4],
    hero_life: [Option<(LifeState, bool)>; 4],
}
impl<'a, R: Resources> WorldBridge<'a, R> {
    pub fn new(
        world: &'a mut WorldState,
        population: &'a mut PopulationState,
        loading: &'a mut LoadingState,
        resources: BridgeResources<'a, R>,
        views: BridgeViews<'a>,
        tick: u64,
    ) -> PlayerResult<Self> {
        validate_views(world, views)?;
        for view in views.effects.iter().flatten() {
            validate_effect(world, population, views.heroes, *view)?;
        }
        Ok(Self {
            world,
            population,
            loading,
            resources,
            heroes: views.heroes,
            effects: EffectOverlay::new(views.effects),
            tick,
            hero_positions: [None; 4],
            hero_life: [None; 4],
        })
    }
    fn geometry(&self) -> GeometryView<'_, R> {
        GeometryView {
            world: self.world,
            loading: self.loading,
            terrain: self.resources.terrain,
            heroes: self.heroes,
            actors: ActorGeometry::Live(self.population, self.resources.earthlings),
            effects: GeometryEffects::Overlay(&self.effects),
            hero_positions: &self.hero_positions,
            hero_life: &self.hero_life,
        }
    }
    fn current_pose(&self, hero: HeroBody) -> (Option<LevelId>, Position) {
        self.hero_positions[hero.seat.index()]
            .map_or((hero.level, hero.position), |(level, position)| {
                (Some(level), position)
            })
    }
    fn hero(&self, seat: SeatId) -> PlayerResult<HeroBody> {
        let mut hero = hero(self.heroes, seat)?;
        (hero.level, hero.position) = self.current_pose(hero);
        if let Some((life, collidable)) = self.hero_life[seat.index()] {
            hero.life = life;
            hero.collidable = collidable;
            if life == LifeState::Eliminated {
                hero.level = None;
            }
        }
        Ok(hero)
    }
    fn require_owner(&self, seat: SeatId, level: LevelId) -> PlayerResult<HeroBody> {
        let hero = self.hero(seat)?;
        if hero.life != LifeState::Alive
            || hero.level != Some(level)
            || self.world.active()[seat.index()] != Some(level)
            || self.loading.travelling_mask().contains(seat)
        {
            return Err(PlayerError::Rejected(
                "world present owner is not an active hero",
            ));
        }
        Ok(hero)
    }
}
impl<R: Resources> PlayerGeometry for WorldBridge<'_, R> {
    fn dimensions(&self, level: LevelId) -> PlayerResult<Dimensions> {
        self.geometry().dimensions(level)
    }
    fn terrain(&self, level: LevelId, x: i32, y: i32) -> PlayerResult<u8> {
        self.geometry().terrain(level, x, y)
    }
    fn terrain_cache(&self, landing: Landing) -> PlayerResult<[[u8; 5]; 5]> {
        self.geometry().terrain_cache(landing)
    }
    fn blocked(
        &self,
        seat: SeatId,
        archetype: Archetype,
        level: LevelId,
        position: Position,
    ) -> PlayerResult<bool> {
        self.geometry().blocked(seat, archetype, level, position)
    }
    fn movement_blocked(
        &self,
        seat: SeatId,
        archetype: Archetype,
        level: LevelId,
        previous: Position,
        proposed: Position,
    ) -> PlayerResult<bool> {
        self.geometry()
            .movement_blocked(seat, archetype, level, previous, proposed)
    }
    fn respawn(&self, seat: SeatId, checkpoint: Landing) -> PlayerResult<Landing> {
        self.geometry().respawn(seat, checkpoint)
    }
}

enum ActorGeometry<'a> {
    Live(&'a PopulationState, &'a DefinitionCatalog),
    Frozen(&'a ActorViews),
}
enum GeometryEffects<'a> {
    Frozen(&'a EffectViews),
    Overlay(&'a EffectOverlay<'a>),
}
impl GeometryEffects<'_> {
    fn at(&self, index: usize) -> Option<EffectView> {
        match self {
            Self::Frozen(views) => views[index],
            Self::Overlay(views) => {
                if views.changed & (1_u64 << index) != 0 {
                    views.values[index]
                } else {
                    views.base[index]
                }
            }
        }
    }
}
/// Shared collision implementation for a detached ContactSink. Frozen colliders
/// are captured once before the population borrow; no second geometry convention.
pub struct GeometryView<'a, R: Resources> {
    world: &'a WorldState,
    loading: &'a LoadingState,
    terrain: &'a R,
    heroes: &'a [Option<HeroBody>; 4],
    actors: ActorGeometry<'a>,
    effects: GeometryEffects<'a>,
    hero_positions: &'a [Option<(LevelId, Position)>; 4],
    hero_life: &'a [Option<(LifeState, bool)>; 4],
}
impl<'a, R: Resources> GeometryView<'a, R> {
    pub fn frozen(
        world: &'a WorldState,
        loading: &'a LoadingState,
        terrain: &'a R,
        actors: &'a ActorViews,
        views: BridgeViews<'a>,
    ) -> PlayerResult<Self> {
        validate_views(world, views)?;
        for (index, body) in actors
            .iter()
            .enumerate()
            .filter_map(|(index, body)| body.map(|body| (index, body)))
        {
            if body.actor.slot() != ActorId::POPULATION_START + index as u16
                || !body.actor.valid()
                || body.width == 0
                || body.height == 0
                || world.floor(body.level).is_none()
            {
                return Err(PlayerError::Invalid(
                    "frozen population collider identity/dimensions",
                ));
            }
            body.position
                .validate()
                .map_err(|_| PlayerError::Geometry("frozen actor position"))?;
        }
        Ok(Self {
            world,
            loading,
            terrain,
            heroes: views.heroes,
            actors: ActorGeometry::Frozen(actors),
            effects: GeometryEffects::Frozen(views.effects),
            hero_positions: &[None; 4],
            hero_life: &[None; 4],
        })
    }
    fn hero(&self, seat: SeatId) -> PlayerResult<HeroBody> {
        let mut hero = hero(self.heroes, seat)?;
        if let Some((level, position)) = self.hero_positions[seat.index()] {
            hero.level = Some(level);
            hero.position = position;
        }
        if let Some((life, collidable)) = self.hero_life[seat.index()] {
            hero.life = life;
            hero.collidable = collidable;
            if life == LifeState::Eliminated {
                hero.level = None;
            }
        }
        Ok(hero)
    }
    fn floor(&self, level: LevelId) -> PlayerResult<&tje_world::world::StoredFloor> {
        level.validate().map_err(map_world)?;
        self.world
            .floor(level)
            .ok_or(PlayerError::Geometry("requested world floor is not loaded"))
    }
    fn actor_collision(
        &self,
        archetype: Archetype,
        level: LevelId,
        position: Position,
    ) -> PlayerResult<bool> {
        let collides = |body: ActorBody| {
            body.collidable
                && body.level == level
                && volumes_overlap(
                    position,
                    archetype.dimensions(),
                    body.position,
                    (body.width, body.height),
                )
        };
        Ok(match &self.actors {
            ActorGeometry::Frozen(actors) => actors.iter().flatten().copied().any(collides),
            ActorGeometry::Live(population, catalog) => {
                let index = population
                    .floors()
                    .binary_search_by_key(&level, |floor| floor.level)
                    .map_err(|_| PlayerError::Geometry("loaded floor has no population record"))?;
                let floor = &population.floors()[index];
                floor.actors.iter().any(|actor| {
                    let definition = catalog.get(actor.kind);
                    !actor.retired
                        && !actor.pending_director
                        && !actor.hidden
                        && actor.collidable
                        && !floor.failed.contains(actor.id.local)
                        && volumes_overlap(
                            position,
                            archetype.dimensions(),
                            actor.motion.position,
                            (definition.width, definition.height),
                        )
                })
            }
        })
    }
    fn entity_collision(
        &self,
        seat: SeatId,
        archetype: Archetype,
        level: LevelId,
        position: Position,
    ) -> PlayerResult<bool> {
        if self.actor_collision(archetype, level, position)? {
            return Ok(true);
        }
        let travelling = self.loading.travelling_mask();
        for other in self.heroes.iter().flatten() {
            let other = self.hero(other.seat)?;
            if other.seat != seat
                && other.collidable
                && !travelling.contains(other.seat)
                && other.level == Some(level)
                && volumes_overlap(
                    position,
                    archetype.dimensions(),
                    other.position,
                    other.archetype.dimensions(),
                )
            {
                return Ok(true);
            }
        }
        for index in 0..64 {
            let Some(view) = self.effects.at(index) else {
                continue;
            };
            let Some(dimensions) = effect_collider(view.effect.kind) else {
                continue;
            };
            if view.effect.level == level
                && volumes_overlap(
                    position,
                    archetype.dimensions(),
                    view.effect.position,
                    dimensions,
                )
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn fixed_collision(
        &self,
        seat: SeatId,
        level: LevelId,
        previous: Position,
        proposed: Position,
    ) -> PlayerResult<bool> {
        proposed
            .validate()
            .map_err(|_| PlayerError::Geometry("collision position"))?;
        previous
            .validate()
            .map_err(|_| PlayerError::Geometry("collision previous position"))?;
        let (maximum_x, maximum_y) = self.dimensions(level)?.world_envelope();
        let point = (proposed.x >> 8, proposed.y >> 8);
        if point.0 < 8 || point.1 < 8 || point.0 >= maximum_x - 8 || point.1 >= maximum_y - 8 {
            return Ok(true);
        }
        let barriers = self
            .loading
            .elevator_barriers(self.world, level)
            .map_err(map_world)?;
        Ok(barriers.iter().any(|barrier| {
            barrier.player_contact(
                level,
                (previous.x >> 8, previous.y >> 8),
                point,
                previous.elevation >> 8,
                proposed.elevation >> 8,
                self.loading.travelling_mask().contains(seat),
            ) == ElevatorContact::Blocked
        }))
    }
    fn safe_landing(
        &self,
        seat: SeatId,
        archetype: Archetype,
        landing: Landing,
    ) -> PlayerResult<bool> {
        let floor = self.floor(landing.level)?;
        let geometry = &floor.generated.geometry;
        let (x, y) = (landing.position.x >> 8, landing.position.y >> 8);
        let Some(cell) = geometry.dimensions().world_cell(x, y) else {
            return Ok(false);
        };
        if geometry.path_state()[cell.index(geometry.dimensions()).map_err(map_world)?] != 0 {
            return Ok(false);
        }
        let footprint =
            Footprint::new(archetype.dimensions().0.div_ceil(8), 2).map_err(map_world)?;
        if !footprint
            .supported_at(
                geometry,
                landing.level,
                (x, y),
                TerrainMask::LAND_OR_SAND,
                self.terrain,
            )
            .map_err(map_world)?
            || self.blocked(seat, archetype, landing.level, landing.position)?
        {
            return Ok(false);
        }
        let barriers = self
            .loading
            .elevator_barriers(self.world, landing.level)
            .map_err(map_world)?;
        Ok(!barriers
            .iter()
            .any(|barrier| barrier.contains(landing.level, (x, y))))
    }
}
impl<R: Resources> PlayerGeometry for GeometryView<'_, R> {
    fn dimensions(&self, level: LevelId) -> PlayerResult<Dimensions> {
        Ok(self.floor(level)?.generated.geometry.dimensions())
    }
    fn terrain(&self, level: LevelId, x: i32, y: i32) -> PlayerResult<u8> {
        self.floor(level)?
            .generated
            .geometry
            .terrain_at(level, x, y, self.terrain)
            .map_err(map_world)
    }
    fn terrain_cache(&self, landing: Landing) -> PlayerResult<[[u8; 5]; 5]> {
        landing
            .position
            .validate()
            .map_err(|_| PlayerError::Geometry("terrain cache center"))?;
        let mut supports = [DoorwaySupport {
            owner: 0,
            level: landing.level,
            x: 0,
            y: 0,
            open: false,
        }; 4];
        let mut count = 0;
        for support in self
            .loading
            .doorway_supports(landing.level)
            .into_iter()
            .flatten()
        {
            supports[count] = support;
            count += 1;
        }
        let cache = TerrainCache::sample(
            &self.floor(landing.level)?.generated.geometry,
            landing.level,
            (landing.position.x >> 8, landing.position.y >> 8),
            self.terrain,
            &supports[..count],
        )
        .map_err(map_world)?;
        Ok(std::array::from_fn(|row| {
            std::array::from_fn(|column| cache.tiles[row * 5 + column])
        }))
    }
    fn blocked(
        &self,
        seat: SeatId,
        archetype: Archetype,
        level: LevelId,
        position: Position,
    ) -> PlayerResult<bool> {
        let actor = self.hero(seat)?;
        if actor.archetype != archetype {
            return Err(PlayerError::Invalid(
                "collision archetype differs from hero",
            ));
        }
        let previous = actor
            .level
            .filter(|&source| source == level)
            .map_or(position, |_| actor.position);
        Ok(self.fixed_collision(seat, level, previous, position)?
            || self.entity_collision(seat, archetype, level, position)?)
    }
    fn movement_blocked(
        &self,
        seat: SeatId,
        archetype: Archetype,
        level: LevelId,
        previous: Position,
        proposed: Position,
    ) -> PlayerResult<bool> {
        let actor = self.hero(seat)?;
        if actor.archetype != archetype {
            return Err(PlayerError::Invalid(
                "collision archetype differs from hero",
            ));
        }
        Ok(self.fixed_collision(seat, level, previous, proposed)?
            || !self.entity_collision(seat, archetype, level, previous)?
                && self.entity_collision(seat, archetype, level, proposed)?)
    }
    fn respawn(&self, seat: SeatId, checkpoint: Landing) -> PlayerResult<Landing> {
        let archetype = self.hero(seat)?.archetype;
        checkpoint
            .position
            .validate()
            .map_err(|_| PlayerError::Geometry("respawn checkpoint"))?;
        let checkpoint = Landing {
            position: Position {
                elevation: 0,
                ..checkpoint.position
            },
            ..checkpoint
        };
        if self.safe_landing(seat, archetype, checkpoint)? {
            return Ok(checkpoint);
        }
        let floor = self.floor(checkpoint.level)?;
        let geometry = &floor.generated.geometry;
        let dimensions = geometry.dimensions();
        let mut label = [0_u8; 17];
        label[..8].copy_from_slice(b"respawn/");
        label[8] = seat.get();
        label[9..13].copy_from_slice(&checkpoint.position.x.to_be_bytes());
        label[13..17].copy_from_slice(&checkpoint.position.y.to_be_bytes());
        let mut rng = StageRng::derive(
            self.world.configuration_identity(),
            checkpoint.level,
            0,
            &label,
        );
        let footprint =
            Footprint::new(archetype.dimensions().0.div_ceil(8), 2).map_err(map_world)?;
        let mut selected = None;
        // Exactly ten row-before-column trials, then a cyclic complete cell scan.
        for trial in 0..10 + dimensions.area() {
            let (column, row) = if trial < 10 {
                let row = rng.bounded(u32::from(dimensions.height())) as u16;
                let column = rng.bounded(u32::from(dimensions.width())) as u16;
                (column, row)
            } else {
                let start = *selected.get_or_insert_with(|| {
                    let row = rng.bounded(u32::from(dimensions.height())) as u16;
                    let column = rng.bounded(u32::from(dimensions.width())) as u16;
                    (column, row)
                });
                let offset = trial - 10;
                (
                    (start.0 + (offset % usize::from(dimensions.width())) as u16)
                        % dimensions.width(),
                    (start.1 + (offset / usize::from(dimensions.width())) as u16)
                        % dimensions.height(),
                )
            };
            let cell = geometry
                .cell(i32::from(column), i32::from(row))
                .ok_or(PlayerError::Geometry("respawn cell index"))?;
            if geometry.path_state()
                [usize::from(row) * usize::from(dimensions.width()) + usize::from(column)]
                != 0
            {
                continue;
            }
            let Some((x, y)) = position_in_cell(
                cell,
                footprint,
                TerrainMask::LAND_OR_SAND,
                false,
                &mut rng,
                self.terrain,
            )
            .map_err(map_world)?
            else {
                continue;
            };
            let landing = Landing {
                level: checkpoint.level,
                position: Position {
                    x: ((i32::from(column) + 1) * 128 + x) * 256,
                    y: ((i32::from(row) + 1) * 64 + y) * 256,
                    elevation: 0,
                },
            };
            if self.safe_landing(seat, archetype, landing)? {
                return Ok(landing);
            }
        }
        Err(PlayerError::Geometry(
            "no safe respawn landing in bounded floor scan",
        ))
    }
}

fn hero(heroes: &[Option<HeroBody>; 4], seat: SeatId) -> PlayerResult<HeroBody> {
    seat.validate()
        .map_err(|_| PlayerError::Invalid("world bridge hero seat"))?;
    heroes[seat.index()]
        .filter(|hero| hero.seat == seat)
        .ok_or(PlayerError::Rejected(
            "world operation requires a hero body seat",
        ))
}
fn validate_views(world: &WorldState, views: BridgeViews<'_>) -> PlayerResult<()> {
    for (index, hero) in views.heroes.iter().enumerate() {
        if let Some(hero) = hero {
            if hero.seat.index() != index || hero.life == LifeState::Disabled {
                return Err(PlayerError::Invalid("world bridge hero snapshot identity"));
            }
            hero.position
                .validate()
                .map_err(|_| PlayerError::Geometry("world bridge hero position"))?;
            if hero.life == LifeState::Alive
                && (hero.level.is_none()
                    || hero.level.is_some_and(|level| world.floor(level).is_none()))
            {
                return Err(PlayerError::Geometry("alive hero snapshot has no floor"));
            }
        }
    }
    for (index, view) in views.effects.iter().enumerate() {
        if let Some(view) = view {
            if !view.id.valid()
                || view.id.index() != index
                || world.floor(view.effect.level).is_none()
            {
                return Err(PlayerError::Invalid(
                    "world bridge effect snapshot identity/floor",
                ));
            }
            view.effect
                .position
                .validate()
                .map_err(|_| PlayerError::Geometry("world bridge effect position"))?;
        }
    }
    Ok(())
}
fn validate_effect(
    world: &WorldState,
    population: &PopulationState,
    heroes: &[Option<HeroBody>; 4],
    view: EffectView,
) -> PlayerResult<()> {
    if !view.id.valid() || world.floor(view.effect.level).is_none() || view.effect.ticks_left == 0 {
        return Err(PlayerError::Invalid("effect birth identity/floor/lifetime"));
    }
    view.effect
        .position
        .validate()
        .map_err(|_| PlayerError::Geometry("effect birth position"))?;
    if view
        .effect
        .velocity
        .iter()
        .chain(&view.effect.acceleration)
        .any(|value| value.unsigned_abs() > 64 * 256)
    {
        return Err(PlayerError::Invalid("effect birth motion"));
    }
    match view.effect.owner {
        EffectOwner::Seat(seat) => {
            hero(heroes, seat)?;
        }
        EffectOwner::Actor(actor) if !population.known_body(actor) => {
            return Err(PlayerError::Invalid(
                "effect birth has unknown population source",
            ));
        }
        _ => (),
    }
    if let EffectSource::Actor(definition) = view.effect.source
        && Kind::new(
            u8::try_from(definition.get()).map_err(|_| {
                PlayerError::Invalid("effect definition outside Earthling catalogue")
            })?,
        )
        .is_none()
    {
        return Err(PlayerError::Invalid(
            "effect definition outside Earthling catalogue",
        ));
    }
    Ok(())
}
fn volumes_overlap(
    position: Position,
    dimensions: (u8, u8),
    other: Position,
    other_dimensions: (u8, u8),
) -> bool {
    let reach = i64::from(dimensions.0 / 2) + i64::from(other_dimensions.0 / 2) - 10;
    let x = (i64::from(position.x >> 8) - i64::from(other.x >> 8)).abs();
    let y = (i64::from(position.y >> 8) - i64::from(other.y >> 8)).abs();
    let z = i64::from(position.elevation >> 8);
    let base = i64::from(other.elevation >> 8);
    reach > 0
        && x < reach
        && y < 8
        && if base <= z {
            z < base + i64::from(other_dimensions.1)
        } else {
            z + i64::from(dimensions.1) >= base
        }
}
fn effect_collider(kind: EffectKind) -> Option<(u8, u8)> {
    match kind {
        EffectKind::Decoy => Some((16, 32)),
        EffectKind::RoseBush => Some((24, 20)),
        EffectKind::Doorway => Some((24, 48)),
        _ => None,
    }
}
fn map_world(error: tje_world::Error) -> PlayerError {
    match error {
        tje_world::Error::Bounds(reason)
        | tje_world::Error::State(reason)
        | tje_world::Error::Resource(reason) => PlayerError::Geometry(reason),
        tje_world::Error::Capacity(reason) => PlayerError::Capacity(reason),
        tje_world::Error::InvalidCell(_) => PlayerError::Geometry("invalid cell in shared world"),
        tje_world::Error::Generation { reason, .. } => PlayerError::Geometry(reason),
        tje_world::Error::OwnershipClosure => {
            PlayerError::Rejected("world simulation ownership closure unavailable")
        }
    }
}
