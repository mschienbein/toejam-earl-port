use bincode::{Decode, Encode};
use tje_world::{Error, Result};

/// Logical definition indices, not object indices or callable cartridge addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Encode, Decode)]
#[repr(u8)]
pub enum Kind {
    ShipPiece,
    Devil,
    Hamster,
    Shopper,
    Dentist,
    HulaDancer,
    Bees,
    Cupid,
    Whirlwind,
    Mailbox,
    Wizard,
    Wiseman,
    NerdHerd,
    Lawnmower,
    Shark,
    Chickens,
    Boogieman,
    OperaSinger,
    Mole,
    IceCreamTruck,
    Santa,
    DentistVariant,
    BeesVariant,
    LemonadeStand,
    HotTub,
}
impl Kind {
    pub const ALL: [Self; 25] = [
        Self::ShipPiece,
        Self::Devil,
        Self::Hamster,
        Self::Shopper,
        Self::Dentist,
        Self::HulaDancer,
        Self::Bees,
        Self::Cupid,
        Self::Whirlwind,
        Self::Mailbox,
        Self::Wizard,
        Self::Wiseman,
        Self::NerdHerd,
        Self::Lawnmower,
        Self::Shark,
        Self::Chickens,
        Self::Boogieman,
        Self::OperaSinger,
        Self::Mole,
        Self::IceCreamTruck,
        Self::Santa,
        Self::DentistVariant,
        Self::BeesVariant,
        Self::LemonadeStand,
        Self::HotTub,
    ];
    pub fn new(index: u8) -> Option<Self> {
        Self::ALL.get(usize::from(index)).copied()
    }
    pub const fn index(self) -> usize {
        self as usize
    }
    pub const fn required(self) -> bool {
        matches!(
            self,
            Self::ShipPiece | Self::Mailbox | Self::LemonadeStand | Self::HotTub
        )
    }
    pub const fn service(self) -> Option<Service> {
        match self {
            Self::Mailbox => Some(Service::MailOrder),
            Self::Wizard => Some(Service::Heal),
            Self::Wiseman => Some(Service::Identify),
            Self::OperaSinger => Some(Service::Opera),
            Self::LemonadeStand => Some(Service::Lemonade),
            Self::HotTub => Some(Service::HotTub),
            _ => None,
        }
    }
    pub const fn hazard_immune(self) -> bool {
        matches!(
            self,
            Self::ShipPiece
                | Self::Bees
                | Self::BeesVariant
                | Self::Cupid
                | Self::Santa
                | Self::Whirlwind
                | Self::IceCreamTruck
        )
    }
    pub const fn spawn_footprint(self) -> (u8, u8, u8) {
        // G/src/earthlings/spawning_and_banks.c:120-135; masks are terrain bits.
        match self {
            Self::ShipPiece
            | Self::NerdHerd
            | Self::IceCreamTruck
            | Self::Lawnmower
            | Self::Mole => (16, 8, 3),
            Self::Whirlwind => (2, 2, 2),
            Self::Shark => (3, 2, 8),
            _ => (8, 2, 3),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Thinker {
    None,
    Generic,
    Timed,
    HorizontalWander,
    HorizontalTurn,
    HorizontalReversal,
    Whirlwind,
    Santa,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Movement {
    None,
    AllowedTerrain,
    MapBounds,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Touch {
    ShipPiece,
    Hurt,
    Squish,
    Hula,
    Cupid,
    Whirlwind,
    Chicken,
    Boogie,
    Mailbox,
    Wizard,
    Wiseman,
    Opera,
    Mole,
    Santa,
    Lemonade,
    HotTub,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Service {
    MailOrder,
    Heal,
    Identify,
    Opera,
    Lemonade,
    HotTub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct TickRange {
    pub minimum: u8,
    pub maximum: u8,
}
impl TickRange {
    pub fn validate(self) -> Result<()> {
        if self.minimum > self.maximum {
            return Err(Error::Resource("reversed Earthling timer range"));
        }
        Ok(())
    }
    pub fn sample(self, rng: &mut tje_world::random::StageRng) -> i16 {
        i16::from(self.minimum) + rng.bounded(u32::from(self.maximum - self.minimum) + 1) as i16
    }
}

/// Runtime-imported semantics only. No executable pointers, graphics or table payloads
/// are embedded in source or serialized into persistent actors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Definition {
    pub thinker: Thinker,
    pub movement: Movement,
    pub touch: Touch,
    pub axes: u8,
    pub width: u8,
    pub height: u8,
    pub target_chance: u8,
    pub target_distance: u8,
    pub turn_chance: u8,
    pub companion: bool,
    pub change_axis: bool,
    pub radius: u8,
    pub contact_x: u8,
    pub contact_y: u8,
    pub damage: u8,
    pub target_time: TickRange,
    pub turn_time: TickRange,
    pub move_time: TickRange,
    pub state_delay: u8,
    pub health: u8,
}
impl Definition {
    pub fn validate(self) -> Result<()> {
        if self.axes > 3
            || self.width == 0
            || self.height == 0
            || self.target_chance > 100
            || self.turn_chance > 100
            || self.health > 127
        {
            return Err(Error::Resource("invalid Earthling definition"));
        }
        self.target_time.validate()?;
        self.turn_time.validate()?;
        self.move_time.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectileVectors {
    pub thresholds: [i16; 8],
    pub velocities: [[i16; 2]; 9],
}
impl ProjectileVectors {
    pub fn validate(&self) -> Result<()> {
        if self.thresholds.iter().any(|&value| value < 0)
            || self.thresholds.windows(2).any(|pair| pair[0] > pair[1])
            || self
                .velocities
                .iter()
                .flatten()
                .any(|value| value.unsigned_abs() > 64 * 256)
        {
            return Err(Error::Resource("Earthling projectile vectors"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionCatalog {
    definitions: [Definition; 25],
    pub projectiles: ProjectileVectors,
}
impl DefinitionCatalog {
    pub fn new(definitions: [Definition; 25], projectiles: ProjectileVectors) -> Result<Self> {
        for definition in definitions {
            definition.validate()?;
        }
        projectiles.validate()?;
        Ok(Self {
            definitions,
            projectiles,
        })
    }
    pub fn get(&self, kind: Kind) -> &Definition {
        &self.definitions[kind.index()]
    }
}
