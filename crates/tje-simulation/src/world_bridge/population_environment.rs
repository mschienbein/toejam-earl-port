use super::*;

/// Real resource/geometry operations for present spawning, with no fake
/// animation fallback or host pointer threaded into original code.
pub(super) struct PopulationEnvironment<'a, R: Resources> {
    pub world: &'a WorldState,
    pub loading: &'a LoadingState,
    pub resources: &'a R,
    pub animation_step: &'a AnimationStep<'a>,
}
impl<R: Resources> population::Environment for PopulationEnvironment<'_, R> {
    fn dimensions(&self, level: LevelId) -> tje_world::Result<Dimensions> {
        self.world
            .floor(level)
            .map(|floor| floor.generated.geometry.dimensions())
            .ok_or(tje_world::Error::State("population present floor absent"))
    }
    fn terrain(&self, level: LevelId, x: i32, y: i32) -> tje_world::Result<u8> {
        self.world
            .floor(level)
            .ok_or(tje_world::Error::State("population present floor absent"))?
            .generated
            .geometry
            .terrain_at(level, x, y, self.resources)
    }
    fn base_metatile(&self, level: LevelId, x: i32, y: i32) -> tje_world::Result<Option<u8>> {
        let geometry = &self
            .world
            .floor(level)
            .ok_or(tje_world::Error::State("population present floor absent"))?
            .generated
            .geometry;
        Ok(geometry
            .dimensions()
            .world_cell(x, y)
            .and_then(|point| geometry.cell(i32::from(point.x), i32::from(point.y)))
            .map(|cell| cell.base() & 127))
    }
    fn placement(
        &self,
        level: LevelId,
        kind: Kind,
        column: u16,
        row: u16,
        rng: &mut StageRng,
    ) -> tje_world::Result<Option<(i32, i32)>> {
        let geometry = &self
            .world
            .floor(level)
            .ok_or(tje_world::Error::State("population present floor absent"))?
            .generated
            .geometry;
        let cell = geometry
            .cell(i32::from(column), i32::from(row))
            .ok_or(tje_world::Error::Bounds("population present cell"))?;
        let (width, height, mask) = kind.spawn_footprint();
        Ok(position_in_cell(
            cell,
            Footprint::new(width, height)?,
            TerrainMask::new(mask)?,
            false,
            rng,
            self.resources,
        )?
        .map(|(x, y)| {
            (
                (i32::from(column) + 1) * 128 + x,
                (i32::from(row) + 1) * 64 + y,
            )
        }))
    }
    fn allows_placement(&self, level: LevelId, x: i32, y: i32) -> tje_world::Result<bool> {
        if level.get() == 1 {
            return Ok(true);
        }
        let barriers = self.loading.elevator_barriers(self.world, level)?;
        Ok(!barriers.iter().any(|barrier| {
            (i64::from(x) - i64::from(barrier.x)).abs() < 30
                && (i64::from(y) - i64::from(barrier.y) - 5).abs() < 20
        }))
    }
    fn animation_step(
        &self,
        kind: Kind,
        state: u8,
        cursor: &mut AnimationCursor,
    ) -> tje_world::Result<AnimationSample> {
        (self.animation_step)(kind, state, cursor)
    }
}
