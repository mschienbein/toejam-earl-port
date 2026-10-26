use super::*;
use tje_world::{
    Density,
    population::{PopulationBaseline, PopulationBudget},
};

impl PopulationState {
    /// Shared measured-walkable-area policy. Original preserves the supplied
    /// twenty-actor baseline; hero count and renderer dimensions never scale it.
    pub fn density_budget(
        baseline: PopulationBaseline,
        walkable_tiles: u32,
        density: Density,
    ) -> Result<PopulationBudget> {
        PopulationBudget::calculate(baseline, walkable_tiles, density)
    }
    /// Resolve a whole floor before publication. The supplied order is the stable
    /// stock/course identity; required anchors replace those identities explicitly.
    /// Failed optional placements remain tombstones with stable failure bits.
    pub fn populate_floor(
        &mut self,
        level: LevelId,
        kinds: &[Kind],
        required: &[AuthoredActor],
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
    ) -> Result<bool> {
        if self.floor_index(level).is_ok() {
            return Ok(false);
        }
        if kinds.len() > usize::from(self.limits.per_floor) {
            return Err(Error::Capacity("requested Earthling population"));
        }
        if required
            .iter()
            .any(|anchor| kinds.get(usize::from(anchor.local)) != Some(&anchor.kind))
        {
            return Err(Error::State("required Earthling identity"));
        }
        let mut slots = [None; MAX_POPULATION as usize];
        for (slot, &kind) in slots.iter_mut().zip(kinds) {
            *slot = Some(kind);
        }
        let floor =
            self.prepare_slots(level, &slots[..kinds.len()], required, environment, catalog)?;
        self.install_prepared_floor(floor)?;
        Ok(true)
    }
    /// A present-created actor uses free stable capacity, never an inactive
    /// original actor's identity. Failure leaves RNG, pools and records unchanged.
    pub fn spawn_present(
        &mut self,
        owner: SeatId,
        level: LevelId,
        origin: Position,
        choice: u16,
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
    ) -> Result<PopulationId> {
        owner
            .validate()
            .map_err(|_| Error::State("Earthling present owner"))?;
        origin
            .validate()
            .map_err(|_| Error::Bounds("Earthling present origin"))?;
        if self.retained[owner.index()] != Some(level) {
            return Err(Error::State("Earthling present hero retain"));
        }
        let index = self.floor_index(level)?;
        let floor = &self.floors[index];
        let local = (u64::BITS - floor.declared_slots.leading_zeros()) as u16;
        if local >= self.limits.per_floor {
            return Err(Error::Capacity("stable Earthling present records"));
        }
        let mut rng = floor.spawn_rng;
        let (hero_x, hero_y) = (origin.x >> 8, origin.y >> 8);
        let current = environment.terrain(level, hero_x, hero_y)?;
        let (maximum_x, maximum_y) = environment.dimensions(level)?.world_envelope();
        let mut position = None;
        for _ in 0..4 {
            let x = hero_x + if rng.next_u16() & 1 != 0 { 40 } else { -40 };
            let y = hero_y + if rng.next_u16() & 1 != 0 { 25 } else { -25 };
            let terrain = environment.terrain(level, x, y)?;
            if x >= 8
                && y >= 8
                && x < maximum_x - 8
                && y < maximum_y - 8
                && (current == 0x80) == (terrain == 0x80)
                && environment.allows_placement(level, x, y)?
            {
                position = Some((x, y, terrain));
                break;
            }
        }
        let (x, y, terrain) =
            position.ok_or(Error::Capacity("near-hero Earthling present placement"))?;
        let mut allowed = [None; 22];
        let mut count = 0;
        if level.get() == 0 {
            allowed[0] = Some(Kind::HulaDancer);
            count = 1;
        } else if terrain == 0x80 {
            allowed[0] = Some(Kind::Shark);
            count = 1;
        } else if terrain == 0x90 || terrain == 0x50 {
            allowed[0] = Some(Kind::BeesVariant);
            count = 1;
        } else {
            for kind in Kind::ALL.into_iter().skip(1).take(22) {
                if kind == Kind::Shark
                    || kind == Kind::Mailbox && level.get() < 2
                    || kind == Kind::Whirlwind && terrain != 0x20
                    || kind == Kind::Lawnmower && terrain != 0x10
                {
                    continue;
                }
                if matches!(
                    kind,
                    Kind::OperaSinger | Kind::Santa | Kind::Wiseman | Kind::Wizard | Kind::Mailbox
                ) && floor
                    .actors
                    .iter()
                    .any(|actor| actor.kind == kind && !actor.retired)
                {
                    continue;
                }
                allowed[count] = Some(kind);
                count += 1;
            }
        }
        if count == 0 {
            return Err(Error::Capacity("Earthling present kind eligibility"));
        }
        let kind = allowed[usize::from(choice) % count].expect("bounded candidate set");
        let id = PopulationId { level, local };
        let actor_rng = initial_actor_rng(&self.identity, id);
        let mut actor = PersistentActor::new(
            id,
            kind,
            Position {
                x: x * 256,
                y: y * 256,
                elevation: 0,
            },
            catalog.get(kind),
            actor_rng,
            false,
        );
        actor.owner = Some(owner);
        let required_bodies = 1 + usize::from(actor.companion);
        self.preflight_growth(required_bodies)?;
        if self
            .bodies
            .iter()
            .filter(|slot| slot.owner.is_none())
            .count()
            < required_bodies
        {
            return Err(Error::Capacity("Earthling present bodies"));
        }
        actor.body = Some(allocate_body(&mut self.bodies, id, false)?);
        if actor.companion {
            actor.companion_body = Some(allocate_body(&mut self.bodies, id, true)?);
        }
        actor.interest = SeatMask::new(1 << owner.get()).expect("four seats");
        self.floors[index].actors.push(actor);
        self.floors[index].declared_slots |= 1_u64 << local;
        self.floors[index].spawn_rng = rng;
        Ok(id)
    }
    /// Bounded relocation, with the same ten random cells and cyclic full scan.
    /// No camera-driven retry loop or exhausted-bank write is retained.
    pub fn relocate(
        &mut self,
        id: PopulationId,
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
    ) -> Result<bool> {
        self.relocate_away(id, &[], environment, catalog)
    }
    fn relocate_away(
        &mut self,
        id: PopulationId,
        heroes: &[Option<HeroView>],
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
    ) -> Result<bool> {
        let index = self.floor_index(id.level)?;
        let floor = &self.floors[index];
        let actor = self
            .actor(id)
            .ok_or(Error::State("relocate unknown Earthling"))?;
        if actor.pending_director || actor.retired || actor.kind.required() {
            return Err(Error::State("cannot relocate removed/required Earthling"));
        }
        let mut rng = actor.rng;
        let kind = actor.kind;
        let chosen = find_spawn(id.level, kind, environment, &mut rng, |x, y| {
            for hero in heroes.iter().flatten() {
                if hero.level == id.level
                    && (i64::from(x) - i64::from(hero.position.x >> 8)).abs() <= INTEREST_X
                    && (i64::from(y) - i64::from(hero.position.y >> 8)).abs() <= INTEREST_Y
                {
                    return Ok(false);
                }
            }
            for other in &floor.actors {
                if other.id == id || other.retired || floor.failed.contains(other.id.local) {
                    continue;
                }
                let clearance = (i64::from(catalog.get(kind).radius)
                    + i64::from(catalog.get(other.kind).radius))
                    * 4;
                let (ox, oy) = other.pixels();
                if (i64::from(x) - i64::from(ox)).abs() < clearance
                    && (i64::from(y) - i64::from(oy)).abs() < clearance
                {
                    return Ok(false);
                }
            }
            Ok(true)
        })?;
        let Some((x, y)) = chosen else {
            return Ok(false);
        };
        let actor = self.actor_mut(id).expect("validated identity");
        actor.motion.position.x = x * 256;
        actor.motion.position.y = y * 256;
        actor.rng = rng;
        Ok(true)
    }
    pub(super) fn retreat_santas(
        &mut self,
        heroes: &[Option<HeroView>; 4],
        environment: &impl Environment,
        catalog: &DefinitionCatalog,
    ) -> Result<()> {
        for floor_index in 0..self.floors.len() {
            for actor_index in 0..self.floors[floor_index].actors.len() {
                let actor = &self.floors[floor_index].actors[actor_index];
                if actor.kind != Kind::Santa
                    || actor.retired
                    || actor.body.is_none()
                    || actor.state & 0xfc != 4
                {
                    continue;
                }
                let same_floor = |hero: &&HeroView| hero.level == actor.id.level;
                let mut heroes_here = heroes.iter().flatten().filter(same_floor);
                if heroes_here.clone().next().is_none()
                    || heroes_here.any(|hero| {
                        let (x, y) = actor.pixels();
                        (i64::from(x) - i64::from(hero.position.x >> 8)).abs() <= INTEREST_X
                            && (i64::from(y) - i64::from(hero.position.y >> 8)).abs() <= INTEREST_Y
                    })
                {
                    continue;
                }
                let id = actor.id;
                if !self.relocate_away(id, heroes, environment, catalog)? {
                    return Err(Error::Capacity("Santa relocation outside hero interest"));
                }
                let actor = self.actor_mut(id).expect("persistent Santa");
                actor.motion.velocity = [0; 3];
                actor.motion.acceleration = [0; 3];
                actor.motion.position.elevation = 0;
                actor.set_state(0);
            }
        }
        Ok(())
    }
    /// Hazard damage is level-keyed, not the player/camera bank chosen by a draw.
    pub fn damage_on_hazards(
        &mut self,
        level: LevelId,
        environment: &impl Environment,
    ) -> Result<usize> {
        let index = self.floor_index(level)?;
        let mut removed = 0;
        for actor in &mut self.floors[index].actors {
            if actor.pending_director || actor.body.is_none() || actor.kind.hazard_immune() {
                continue;
            }
            let (x, y) = actor.pixels();
            if environment.terrain(level, x, y)? == 0x90
                && !actor.mail_order
                && actor.kind.index() < Kind::LemonadeStand.index()
            {
                actor.retired = true;
                actor.health = 0;
                removed += 1;
            }
        }
        self.remove_collected()?;
        Ok(removed)
    }
}
pub(super) fn find_spawn(
    level: LevelId,
    kind: Kind,
    environment: &impl Environment,
    rng: &mut StageRng,
    mut allows: impl FnMut(i32, i32) -> Result<bool>,
) -> Result<Option<(i32, i32)>> {
    let dimensions = environment.dimensions(level)?;
    scan_spawn(dimensions, rng, |column, row, rng| {
        let Some((x, y)) = environment.placement(level, kind, column, row, rng)? else {
            return Ok(None);
        };
        if !environment.allows_placement(level, x, y)? || !allows(x, y)? {
            return Ok(None);
        }
        Ok(Some((x, y)))
    })
}
pub(crate) fn scan_spawn<R: tje_world::random::Random16>(
    dimensions: Dimensions,
    rng: &mut R,
    mut try_cell: impl FnMut(u16, u16, &mut R) -> Result<Option<(i32, i32)>>,
) -> Result<Option<(i32, i32)>> {
    for _ in 0..10 {
        let row = rng.next_u16() % dimensions.height();
        let column = rng.next_u16() % dimensions.width();
        if let Some(position) = try_cell(column, row, rng)? {
            return Ok(Some(position));
        }
    }
    let start_row = rng.next_u16() % dimensions.height();
    let start_column = rng.next_u16() % dimensions.width();
    for dy in 0..dimensions.height() {
        for dx in 0..dimensions.width() {
            if let Some(position) = try_cell(
                (start_column + dx) % dimensions.width(),
                (start_row + dy) % dimensions.height(),
                rng,
            )? {
                return Ok(Some(position));
            }
        }
    }
    Ok(None)
}
