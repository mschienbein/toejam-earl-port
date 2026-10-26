use super::*;

/// The world keeps original source placements, while PopulationState owns live
/// AI/motion and dynamic IDs. Only actual source actor IDs participate in this
/// mirror; a present/director lifetime ID never writes an unrepresented bit.
pub fn sync_population_deltas(
    world: &mut WorldState,
    population: &PopulationState,
) -> PlayerResult<()> {
    preflight_source_references(world, population)?;
    for floor in population.floors() {
        let Some(stored) = world.floor_mut(floor.level) else {
            return Err(PlayerError::Geometry("population delta floor absent"));
        };
        for record in &stored.generated.population.earthlings {
            if floor.removed.contains(record.id) || floor.failed.contains(record.id) {
                stored
                    .generated
                    .population
                    .removed
                    .insert(record.id)
                    .map_err(map_world)?;
            }
        }
    }
    validate_population_deltas(world, population)
}

pub fn validate_population_deltas(
    world: &WorldState,
    population: &PopulationState,
) -> PlayerResult<()> {
    preflight_source_references(world, population)?;
    for stored in world.floors() {
        let floor = population
            .floors()
            .iter()
            .find(|floor| floor.level == stored.spec.level())
            .ok_or(PlayerError::Geometry(
                "world source floor has no population owner",
            ))?;
        for record in &stored.generated.population.earthlings {
            if stored.generated.population.removed.contains(record.id)
                != (floor.removed.contains(record.id) || floor.failed.contains(record.id))
            {
                return Err(PlayerError::Invalid(
                    "world/population source removal ledgers disagree",
                ));
            }
        }
    }
    Ok(())
}
fn preflight_source_references(
    world: &WorldState,
    population: &PopulationState,
) -> PlayerResult<()> {
    for stored in world.floors() {
        let floor = population
            .floors()
            .iter()
            .find(|floor| floor.level == stored.spec.level())
            .ok_or(PlayerError::Geometry(
                "world source floor has no population owner",
            ))?;
        stored
            .generated
            .population
            .removed
            .validate(population::MAX_POPULATION)
            .map_err(map_world)?;
        for record in &stored.generated.population.earthlings {
            if record.id >= population::MAX_POPULATION
                || floor.declared_slots & (1_u64 << record.id) == 0
            {
                return Err(PlayerError::Invalid(
                    "world population source identity not declared",
                ));
            }
            if stored.generated.population.removed.contains(record.id)
                && !floor.removed.contains(record.id)
                && !floor.failed.contains(record.id)
            {
                return Err(PlayerError::Invalid(
                    "population would resurrect a removed world source",
                ));
            }
        }
    }
    for floor in population.floors() {
        if world.floor(floor.level).is_none() {
            return Err(PlayerError::Geometry("population source geometry missing"));
        }
    }
    Ok(())
}
