use super::*;
use tje_world::terrain::ElevatorBarrier;

/// A compact candidate-check receipt avoids borrowing mutable loading state
/// through its own doorway-search callback. It contains no geometry clone/RNG.
pub(super) struct PortalSafety {
    hero: SeatId,
    archetype: Archetype,
    level: LevelId,
    heroes: [Option<HeroBody>; 4],
    effects: EffectViews,
    barriers: [ElevatorBarrier; 2],
    travelling: SeatMask,
}
impl PortalSafety {
    pub(super) fn capture<R: Resources>(
        geometry: &GeometryView<'_, R>,
        hero: SeatId,
        level: LevelId,
    ) -> PlayerResult<Self> {
        let archetype = geometry.hero(hero)?.archetype;
        let mut heroes = [None; 4];
        for seat in SeatId::ALL {
            if geometry.heroes[seat.index()].is_some() {
                heroes[seat.index()] = Some(geometry.hero(seat)?);
            }
        }
        Ok(Self {
            hero,
            archetype,
            level,
            heroes,
            effects: std::array::from_fn(|index| geometry.effects.at(index)),
            barriers: geometry
                .loading
                .elevator_barriers(geometry.world, level)
                .map_err(map_world)?,
            travelling: geometry.loading.travelling_mask(),
        })
    }
    pub(super) fn blocked<R: Resources>(
        &self,
        world: &WorldState,
        population: &PopulationState,
        catalog: &DefinitionCatalog,
        resources: &R,
        landing: Landing,
    ) -> PlayerResult<bool> {
        if landing.level != self.level {
            return Err(PlayerError::Rejected("doorway candidate changed floor"));
        }
        landing
            .position
            .validate()
            .map_err(|_| PlayerError::Geometry("portal candidate pose"))?;
        let floor = world
            .floor(self.level)
            .ok_or(PlayerError::Geometry("portal candidate floor absent"))?;
        let geometry = &floor.generated.geometry;
        let point = (landing.position.x >> 8, landing.position.y >> 8);
        let Some(cell) = geometry.dimensions().world_cell(point.0, point.1) else {
            return Ok(true);
        };
        if geometry.path_state()[cell.index(geometry.dimensions()).map_err(map_world)?] != 0
            || !Footprint::new(self.archetype.dimensions().0.div_ceil(8), 2)
                .map_err(map_world)?
                .supported_at(
                    geometry,
                    self.level,
                    point,
                    TerrainMask::LAND_OR_SAND,
                    resources,
                )
                .map_err(map_world)?
            || self
                .barriers
                .iter()
                .any(|barrier| barrier.contains(self.level, point))
        {
            return Ok(true);
        }
        let index = population
            .floors()
            .binary_search_by_key(&self.level, |floor| floor.level)
            .map_err(|_| PlayerError::Geometry("portal population floor absent"))?;
        if population.floors()[index].actors.iter().any(|actor| {
            let dimensions = catalog.get(actor.kind);
            !actor.retired
                && !actor.pending_director
                && !actor.hidden
                && actor.collidable
                && !population.floors()[index].failed.contains(actor.id.local)
                && volumes_overlap(
                    landing.position,
                    self.archetype.dimensions(),
                    actor.motion.position,
                    (dimensions.width, dimensions.height),
                )
        }) {
            return Ok(true);
        }
        if self.heroes.iter().flatten().any(|hero| {
            hero.seat != self.hero
                && hero.collidable
                && !self.travelling.contains(hero.seat)
                && hero.level == Some(self.level)
                && volumes_overlap(
                    landing.position,
                    self.archetype.dimensions(),
                    hero.position,
                    hero.archetype.dimensions(),
                )
        }) {
            return Ok(true);
        }
        Ok(self.effects.iter().flatten().any(|view| {
            view.effect.level == self.level
                && effect_collider(view.effect.kind).is_some_and(|dimensions| {
                    volumes_overlap(
                        landing.position,
                        self.archetype.dimensions(),
                        view.effect.position,
                        dimensions,
                    )
                })
        }))
    }
}
