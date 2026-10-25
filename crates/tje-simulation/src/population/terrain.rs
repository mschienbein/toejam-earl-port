use super::*;
use tje_world::{Resources, world::WorldState};

/// The two closures bridge the aggregate's checked elevator placement and S5's
/// simulation animation decoder. Neither accepts camera or presentation state.
pub struct WorldEnvironment<'a, R, A, P> {
    pub world: &'a WorldState,
    pub resources: &'a R,
    pub animation: A,
    pub placement_allowed: P,
}
impl<R: Resources, A, P> Environment for WorldEnvironment<'_, R, A, P>
where
    A: Fn(Kind, u8, &mut AnimationCursor) -> Result<AnimationSample>,
    P: Fn(LevelId, i32, i32) -> Result<bool>,
{
    fn dimensions(&self, level: LevelId) -> Result<Dimensions> {
        self.world
            .floor(level)
            .map(|floor| floor.generated.geometry.dimensions())
            .ok_or(Error::State("population geometry missing"))
    }
    fn terrain(&self, level: LevelId, x: i32, y: i32) -> Result<u8> {
        self.world
            .floor(level)
            .ok_or(Error::State("population geometry missing"))?
            .generated
            .geometry
            .terrain_at(level, x, y, self.resources)
    }
    fn base_metatile(&self, level: LevelId, x: i32, y: i32) -> Result<Option<u8>> {
        let geometry = &self
            .world
            .floor(level)
            .ok_or(Error::State("population geometry missing"))?
            .generated
            .geometry;
        Ok(geometry
            .dimensions()
            .world_cell(x, y)
            .and_then(|position| geometry.cell(i32::from(position.x), i32::from(position.y)))
            .map(|cell| cell.base() & 0x7f))
    }
    fn placement(
        &self,
        level: LevelId,
        kind: Kind,
        column: u16,
        row: u16,
        rng: &mut StageRng,
    ) -> Result<Option<(i32, i32)>> {
        let geometry = &self
            .world
            .floor(level)
            .ok_or(Error::State("population geometry missing"))?
            .generated
            .geometry;
        let cell = geometry
            .cell(i32::from(column), i32::from(row))
            .ok_or(Error::Bounds("Earthling spawn cell"))?;
        let (width, height, mask) = kind.spawn_footprint();
        let local = tje_world::terrain::position_in_cell(
            cell,
            tje_world::terrain::Footprint::new(width, height)?,
            tje_world::terrain::TerrainMask::new(mask)?,
            false,
            rng,
            self.resources,
        )?;
        Ok(local.map(|(x, y)| {
            (
                (i32::from(column) + 1) * 128 + x,
                (i32::from(row) + 1) * 64 + y,
            )
        }))
    }
    fn allows_placement(&self, level: LevelId, x: i32, y: i32) -> Result<bool> {
        (self.placement_allowed)(level, x, y)
    }
    fn animation_step(
        &self,
        kind: Kind,
        state: u8,
        cursor: &mut AnimationCursor,
    ) -> Result<AnimationSample> {
        (self.animation)(kind, state, cursor)
    }
}

pub(super) struct CollisionScene<'a> {
    pub heroes: &'a [Option<HeroView>; 4],
    pub decoys: &'a [DecoyView],
    pub actors: &'a [Option<CollisionBody>; MAX_ACTIVE_BODIES as usize],
}
#[derive(Clone, Copy)]
enum Collision {
    Body,
    Decoy(ActorId, SeatId),
}
fn collision_at(
    actor: &PersistentActor,
    definition: &Definition,
    position: Position,
    heroes_collide: bool,
    scene: &CollisionScene<'_>,
) -> Option<Collision> {
    let overlaps = |other: Position, width: u8| {
        let reach = i64::from(definition.width / 2) + i64::from(width / 2) - 10;
        reach > 0
            && ai::in_range(position, other, reach, 8)
            && (i64::from(actor.motion.position.elevation >> 8) - i64::from(other.elevation >> 8))
                .abs()
                <= 10
    };
    if heroes_collide {
        for hero in scene.heroes.iter().flatten() {
            if hero.level == actor.id.level
                && !hero.elevator
                && !hero.suppressed
                && overlaps(hero.position, hero.width)
            {
                return Some(Collision::Body);
            }
        }
    }
    for body in scene.actors.iter().flatten() {
        if Some(body.actor) != actor.body
            && body.level == actor.id.level
            && body.collidable
            && overlaps(body.position, body.width)
        {
            return Some(Collision::Body);
        }
    }
    for decoy in scene.decoys {
        if decoy.level == actor.id.level && overlaps(decoy.position, decoy.width) {
            return Some(Collision::Decoy(decoy.actor, decoy.owner));
        }
    }
    None
}

pub(super) fn move_actor(
    actor: &mut PersistentActor,
    definition: &Definition,
    sample: AnimationSample,
    physics: bool,
    environment: &impl Environment,
    scene: &CollisionScene<'_>,
    sink: &mut impl ContactSink,
) -> Result<()> {
    if actor.state == SLEEP_STATE || actor.mail_order {
        return Ok(());
    }
    let mut delta = [0; 2];
    for (axis, displacement) in delta.iter_mut().enumerate() {
        if physics && definition.axes & (1 << axis) == 0 {
            actor.motion.velocity[axis] = actor.motion.velocity[axis]
                .checked_add(actor.motion.acceleration[axis])
                .ok_or(Error::Bounds("Earthling acceleration overflow"))?;
            *displacement = actor.motion.velocity[axis];
        } else if !physics
            && definition.axes & (1 << axis) != 0
            && let Some(motion) = sample.motion
        {
            *displacement = actor.motion.velocity[axis]
                .checked_mul(i32::from(motion[axis]))
                .ok_or(Error::Bounds("Earthling authored motion overflow"))?;
        }
    }
    if physics {
        actor.motion.velocity[2] = actor.motion.velocity[2]
            .checked_add(actor.motion.acceleration[2])
            .ok_or(Error::Bounds("Earthling elevation acceleration overflow"))?;
        actor.motion.position.elevation = actor
            .motion
            .position
            .elevation
            .checked_add(actor.motion.velocity[2])
            .ok_or(Error::Bounds("Earthling elevation overflow"))?;
    }
    if actor.kind == Kind::Cupid && actor.state == 3 {
        if physics && actor.motion.position.elevation < 3 * 256 && actor.motion.velocity[2] < 0 {
            actor.motion.acceleration[2] = -64;
            actor.motion.velocity[2] = 512;
            actor.motion.position.elevation = 0;
            let (x, y) = actor.pixels();
            if environment.terrain(actor.id.level, x, y)? > 0x50 {
                actor.health = actor.health.saturating_sub(1);
            }
        }
        if !physics && actor.animation_finished {
            actor.health = actor.health.saturating_sub(1);
        }
        if actor.health == 0 {
            actor.retired = true;
        }
        return Ok(());
    }
    if actor.hidden {
        return Ok(());
    }
    let x = actor
        .motion
        .position
        .x
        .checked_add(delta[0])
        .ok_or(Error::Bounds("Earthling X position overflow"))?;
    let y = actor
        .motion
        .position
        .y
        .checked_add(delta[1])
        .ok_or(Error::Bounds("Earthling Y position overflow"))?;
    let mut position = Position {
        x,
        y,
        elevation: actor.motion.position.elevation,
    };
    position
        .validate()
        .map_err(|_| Error::Bounds("Earthling motion bounds"))?;
    if definition.movement == Movement::None {
        actor.motion.position = position;
        return Ok(());
    }
    let old = actor.motion.position;
    let mut px = x >> 8;
    let mut py = y >> 8;
    if definition.movement == Movement::MapBounds {
        let (width, height) = environment.dimensions(actor.id.level)?.world_envelope();
        if px - i32::from(definition.width) <= 8 || px + i32::from(definition.width) >= width - 16 {
            position.x = old.x;
            px = old.x >> 8;
            actor.blocked = true;
        }
        if py - i32::from(definition.height) <= 8 || py >= height - 16 {
            position.y = old.y;
            py = old.y >> 8;
            actor.blocked = true;
        }
    }
    let narrow = matches!(
        actor.kind,
        Kind::HulaDancer | Kind::Wizard | Kind::Wiseman | Kind::OperaSinger
    );
    if actor.kind != Kind::Mole
        && collision_at(actor, definition, old, narrow, scene).is_none()
        && let Some(collision) = collision_at(actor, definition, position, narrow, scene)
    {
        actor.blocked = true;
        if let Collision::Decoy(decoy, owner) = collision {
            ai::request(actor, owner, ContactEffect::DecoyHit { decoy }, sink)?;
        }
        return Ok(());
    }
    let (probe_x, probe_y) = if matches!(actor.kind, Kind::Shark | Kind::Mole) {
        match actor.state {
            4 => (px, py - 8),
            5 => (px, py + 8),
            6 => (px - 16, py),
            7 => (px + 16, py),
            _ => (px, py),
        }
    } else {
        (px, py)
    };
    let terrain = environment.terrain(actor.id.level, probe_x, probe_y)?;
    if definition.movement == Movement::AllowedTerrain {
        let allowed = match actor.kind {
            Kind::Lawnmower => terrain == 0x10,
            Kind::Shark => terrain == 0x80,
            _ => terrain < 0x50,
        };
        if !allowed {
            actor.blocked = true;
            return Ok(());
        }
    } else if actor.kind == Kind::IceCreamTruck {
        if environment.base_metatile(actor.id.level, px, py)? == Some(34) {
            actor.blocked = true;
            return Ok(());
        }
        actor.terrain_status = terrain;
    }
    if matches!(actor.kind, Kind::Bees | Kind::BeesVariant) {
        actor.masked = terrain == 0x90;
    } else {
        actor.shadow = terrain != 0x90;
    }
    actor.motion.position = position;
    Ok(())
}
