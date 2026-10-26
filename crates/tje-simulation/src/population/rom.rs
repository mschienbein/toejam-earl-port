//! Checked runtime imports from a verified user cartridge. Callback addresses are
//! resolved once to finite typed behaviours; they never enter persistent state.
use super::*;
use tje_game::{FunctionId, function_at, ids};
use tje_mem::{AddressSpace, Bus, Revision, rom};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimationDescriptor {
    pub sequence: u32,
    pub frames: u32,
    pub period: u8,
}

pub struct RomDefinitions<'a, B: Bus + ?Sized = AddressSpace> {
    bus: &'a B,
    revision: Revision,
    length: usize,
    catalog: DefinitionCatalog,
    state_tables: [u32; 25],
    state_counts: [u8; 25],
    callbacks: [[Option<FunctionId>; 3]; 25],
}
impl<'a> RomDefinitions<'a> {
    pub fn new(memory: &'a AddressSpace) -> Result<Self> {
        Self::import(memory, memory.revision(), memory.rom().len())
    }
}
impl<'a, B: Bus + ?Sized> RomDefinitions<'a, B> {
    pub fn from_verified_bus(bus: &'a B, revision: Revision) -> Result<Self> {
        Self::import(bus, revision, revision.rom_size())
    }
    fn import(bus: &'a B, revision: Revision, length: usize) -> Result<Self> {
        let table = rom::earthling_definitions(revision).address();
        checked_range(table, 25 * 38, length)?;
        let mut definitions = Vec::with_capacity(25);
        let mut states = [0; 25];
        let mut callbacks = [[None; 3]; 25];
        for kind in Kind::ALL {
            let address = table + kind.index() as u32 * 38;
            states[kind.index()] = bus.read_u32(address);
            checked_range(states[kind.index()], 10, length)?;
            let update = resolve(revision, bus.read_u32(address + 4))?;
            let ability = resolve(revision, bus.read_u32(address + 8))?;
            let touch = resolve(revision, bus.read_u32(address + 12))?;
            callbacks[kind.index()] = [update, ability, touch];
            let flags = bus.read_u8(address + 24);
            // The complete recovered consumer set tests only bits 1 and 2:
            // G/generic_enemy.c:133 and spawning_and_banks.c:392,627,736.
            // Other authored bits have no operation to generalize.
            let definition = Definition {
                movement: movement(update)?,
                thinker: thinker(ability)?,
                touch: contact(touch)?,
                axes: bus.read_u8(address + 16),
                width: bus.read_u8(address + 17),
                height: bus.read_u8(address + 18),
                target_chance: bus.read_u8(address + 21),
                target_distance: bus.read_u8(address + 22),
                turn_chance: bus.read_u8(address + 23),
                companion: flags & 1 != 0,
                change_axis: flags & 2 != 0,
                radius: bus.read_u8(address + 25),
                contact_x: bus.read_u8(address + 26),
                contact_y: bus.read_u8(address + 27),
                damage: bus.read_u8(address + 28),
                target_time: TickRange {
                    minimum: bus.read_u8(address + 29),
                    maximum: bus.read_u8(address + 30),
                },
                turn_time: TickRange {
                    minimum: bus.read_u8(address + 31),
                    maximum: bus.read_u8(address + 32),
                },
                move_time: TickRange {
                    minimum: bus.read_u8(address + 33),
                    maximum: bus.read_u8(address + 34),
                },
                state_delay: bus.read_u8(address + 35),
                health: bus.read_u8(address + 36),
            };
            definition.validate()?;
            definitions.push(definition);
        }
        let mut counts = [0; 25];
        for (index, &start) in states.iter().enumerate() {
            let container = tje_mem::rom_data_units(revision)
                .iter()
                .find(|unit| start >= unit.address && start < unit.address + unit.size);
            let end = container
                .map(|unit| unit.address + unit.size)
                .ok_or(Error::Resource(
                    "Earthling animation table has no bounded resource",
                ))?;
            let end = states
                .iter()
                .copied()
                .filter(|&next| next > start && next < end)
                .min()
                .unwrap_or(end);
            let count = (end - start) / 10;
            if count == 0 {
                return Err(Error::Resource("empty Earthling animation table"));
            }
            counts[index] = count.min(16) as u8;
        }
        let thresholds = rom::earthling_thresholds(revision).address();
        let vectors = rom::earthling_vector_pairs(revision).address();
        checked_range(thresholds, 8 * 2, length)?;
        checked_range(vectors, 9 * 4, length)?;
        let projectiles = ProjectileVectors {
            thresholds: std::array::from_fn(|index| {
                bus.read_u16(thresholds + index as u32 * 2) as i16
            }),
            velocities: std::array::from_fn(|index| {
                std::array::from_fn(|axis| {
                    bus.read_u16(vectors + index as u32 * 4 + axis as u32 * 2) as i16
                })
            }),
        };
        Ok(Self {
            bus,
            revision,
            length,
            catalog: DefinitionCatalog::new(
                definitions
                    .try_into()
                    .map_err(|_| Error::Resource("Earthling kind count"))?,
                projectiles,
            )?,
            state_tables: states,
            state_counts: counts,
            callbacks,
        })
    }
    pub fn catalog(&self) -> &DefinitionCatalog {
        &self.catalog
    }
    pub fn callbacks(&self) -> &[[Option<FunctionId>; 3]; 25] {
        &self.callbacks
    }
    /// Bounded slot extent, including explicit null holes; not an admitted-pose count.
    pub fn state_count(&self, kind: Kind) -> u8 {
        self.state_counts[kind.index()]
    }
    pub fn descriptor(&self, kind: Kind, state: u8) -> Result<AnimationDescriptor> {
        self.optional_descriptor(kind, state)?
            .ok_or(Error::Resource(
                "Earthling animation state has no authored descriptor",
            ))
    }
    /// Bounds are an envelope, not proof that every slot is authored. A complete
    /// null pointer pair denotes an absent state; malformed pairs still reject.
    pub fn optional_descriptor(
        &self,
        kind: Kind,
        state: u8,
    ) -> Result<Option<AnimationDescriptor>> {
        if !self.catalog.get(kind).supports_state(kind, state) {
            return Err(Error::Resource(
                "Earthling animation state has no typed producer",
            ));
        }
        let (address, sleep) = if state == SLEEP_STATE {
            if !kind.supports_sleep() {
                return Err(Error::Resource("kind has no sleep descriptor"));
            }
            (
                rom::sleep_animation_descriptors(self.revision).address() + kind.index() as u32 * 8,
                true,
            )
        } else {
            if state >= self.state_counts[kind.index()] {
                return Err(Error::Resource(
                    "Earthling animation state outside definition",
                ));
            }
            (
                self.state_tables[kind.index()] + u32::from(state) * 10,
                false,
            )
        };
        read_descriptor(self.bus, address, sleep, self.length)
    }
    /// Original campaign composition is resolved from the user's ROM, not an
    /// embedded list. Authored courses provide Kind references directly instead.
    pub fn stock_kind(&self, level: LevelId, local: u16) -> Result<Option<Kind>> {
        if level.get() >= 26 || local >= ORIGINAL_POPULATION {
            return Err(Error::Bounds("stock Earthling choice"));
        }
        let address = rom::level_earthling_choices(self.revision).address()
            + u32::from(level.get()) * 20
            + u32::from(local);
        checked_range(address, 1, self.length)?;
        let raw = self.bus.read_u8(address);
        if raw == 255 {
            Ok(None)
        } else {
            Kind::new(raw)
                .map(Some)
                .ok_or(Error::Resource("unknown stock Earthling kind"))
        }
    }
}
fn read_descriptor(
    bus: &(impl Bus + ?Sized),
    address: u32,
    sleep: bool,
    length: usize,
) -> Result<Option<AnimationDescriptor>> {
    checked_range(address, if sleep { 8 } else { 10 }, length)?;
    if address & 1 != 0 {
        return Err(Error::Resource("unaligned Earthling animation descriptor"));
    }
    let sequence = bus.read_u32(address);
    let frames = bus.read_u32(address + 4);
    if sequence == 0 && frames == 0 {
        return Ok(None);
    }
    if sequence == 0 || frames == 0 {
        return Err(Error::Resource(
            "partial null Earthling animation descriptor",
        ));
    }
    checked_range(sequence, 1, length)?;
    checked_range(frames, 4, length)?;
    if frames & 1 != 0 {
        return Err(Error::Resource("unaligned Earthling frame pointer table"));
    }
    let period = if sleep { 6 } else { bus.read_u8(address + 8) };
    if period > 127 {
        return Err(Error::Resource("invalid Earthling animation period"));
    }
    Ok(Some(AnimationDescriptor {
        sequence,
        frames,
        period,
    }))
}

fn checked_range(address: u32, count: usize, length: usize) -> Result<()> {
    if address as usize >= length
        || (address as usize)
            .checked_add(count)
            .is_none_or(|end| end > length)
    {
        return Err(Error::Resource("Earthling ROM reference out of range"));
    }
    Ok(())
}
fn resolve(revision: Revision, address: u32) -> Result<Option<FunctionId>> {
    if address == 0 {
        return Ok(None);
    }
    function_at(revision, address)
        .map(Some)
        .ok_or(Error::Resource("unknown Earthling callback"))
}
fn movement(id: Option<FunctionId>) -> Result<Movement> {
    match id {
        None | Some(ids::ENTITY_UPDATE_TERRAIN_DRAW_FLAG) => Ok(Movement::None),
        Some(ids::EARTHLING_MOVE_ON_ALLOWED_TERRAIN) => Ok(Movement::AllowedTerrain),
        Some(ids::EARTHLING_MOVE_WITHIN_MAP_BOUNDS) => Ok(Movement::MapBounds),
        _ => Err(Error::Resource("unowned Earthling movement callback")),
    }
}
fn thinker(id: Option<FunctionId>) -> Result<Thinker> {
    match id {
        None => Ok(Thinker::None),
        Some(ids::AI_GENERIC_ENEMY_THINK) => Ok(Thinker::Generic),
        Some(ids::EARTHLING_THINK_TIMED_MOVEMENT) => Ok(Thinker::Timed),
        Some(ids::EARTHLING_THINK_HORIZONTAL_WANDER) => Ok(Thinker::HorizontalWander),
        Some(ids::EARTHLING_ADVANCE_HORIZONTAL_TURN_ANIMATION) => Ok(Thinker::HorizontalTurn),
        Some(ids::EARTHLING_THINK_HORIZONTAL_REVERSAL) => Ok(Thinker::HorizontalReversal),
        Some(ids::EARTHLING_THINK_TARGETED_RESET_CYCLE) => Ok(Thinker::Whirlwind),
        Some(ids::SANTA_LOGIC) => Ok(Thinker::Santa),
        _ => Err(Error::Resource("unowned Earthling thinker callback")),
    }
}
fn contact(id: Option<FunctionId>) -> Result<Touch> {
    match id {
        Some(ids::SHIP_PIECE_ABILITY) => Ok(Touch::ShipPiece),
        Some(ids::PLAYER_YOUCH) => Ok(Touch::Hurt),
        Some(ids::PLAYER_SQUISH) => Ok(Touch::Squish),
        Some(ids::HULA_ABILITY) => Ok(Touch::Hula),
        Some(ids::CUPID_ABILITY) => Ok(Touch::Cupid),
        Some(ids::WHIRL_WIND_ABILITY) => Ok(Touch::Whirlwind),
        Some(ids::CHICKEN_ABILITY) => Ok(Touch::Chicken),
        Some(ids::AI_BOOGIE_TOUCH) => Ok(Touch::Boogie),
        Some(ids::AI_MAILBOX_TOUCH) => Ok(Touch::Mailbox),
        Some(ids::AI_WIZARD_TOUCH) => Ok(Touch::Wizard),
        Some(ids::AI_WISEMAN_TOUCH) => Ok(Touch::Wiseman),
        Some(ids::AI_OPERA_SINGER_TOUCH) => Ok(Touch::Opera),
        Some(ids::MOLE_ABILITY) => Ok(Touch::Mole),
        Some(ids::DROP_PRESENTS_ABILITY) => Ok(Touch::Santa),
        Some(ids::LEMONADE_ABILITY) => Ok(Touch::Lemonade),
        Some(ids::HOTTUB_ABILITY) => Ok(Touch::HotTub),
        _ => Err(Error::Resource("unowned Earthling touch callback")),
    }
}

#[cfg(test)]
mod descriptor_tests {
    use super::*;
    fn descriptor(sequence: u32, frames: u32) -> AddressSpace {
        let mut bytes = vec![0; 512];
        bytes[0x40..0x44].copy_from_slice(&sequence.to_be_bytes());
        bytes[0x44..0x48].copy_from_slice(&frames.to_be_bytes());
        bytes[0x48] = 6;
        AddressSpace::new(bytes, Revision::Rev02)
    }
    #[test]
    fn null_holes_are_explicit_but_partial_or_unmapped_descriptors_reject() {
        assert_eq!(
            read_descriptor(&descriptor(0, 0), 0x40, false, 512).unwrap(),
            None
        );
        let valid = read_descriptor(&descriptor(0x100, 0x180), 0x40, false, 512)
            .unwrap()
            .unwrap();
        assert_eq!(
            (valid.sequence, valid.frames, valid.period),
            (0x100, 0x180, 6)
        );
        for (sequence, frames) in [
            (0, 0x180),
            (0x100, 0),
            (0x100, 0x181),
            (0x100, u32::MAX),
            (u32::MAX, 0x180),
        ] {
            assert!(read_descriptor(&descriptor(sequence, frames), 0x40, false, 512).is_err());
        }
    }
    #[test]
    fn sleep_capability_matches_the_original_pre_index_guard() {
        for kind in Kind::ALL {
            assert_eq!(
                kind.supports_sleep(),
                !matches!(
                    kind,
                    Kind::ShipPiece
                        | Kind::Whirlwind
                        | Kind::Shark
                        | Kind::Santa
                        | Kind::LemonadeStand
                        | Kind::HotTub
                )
            );
        }
    }
}
