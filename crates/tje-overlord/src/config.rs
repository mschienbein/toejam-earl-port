use crate::{DIRECTOR, Error, HEROES, Kind, Result, SeatMask};
use bincode::{Decode, Encode};
use sha2::{Digest, Sha256};
use tje_simulation::{
    LevelId,
    session::{Roles, SessionConfig},
};

pub const VERSION: u16 = 1;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Preset {
    ShortCourse { required: u32, exit: LevelId },
    Campaign,
}
impl Preset {
    pub fn required(self) -> u32 {
        match self {
            Self::ShortCourse { required, .. } => required,
            Self::Campaign => 0x3ff,
        }
    }
    pub fn exit(self) -> LevelId {
        match self {
            Self::ShortCourse { exit, .. } => exit,
            Self::Campaign => LevelId::new(26).expect("campaign exit"),
        }
    }
    pub const fn time_limit(self) -> Option<u64> {
        match self {
            Self::ShortCourse { .. } => Some(15 * 60 * 60),
            Self::Campaign => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct Config {
    pub version: u16,
    pub shared: SessionConfig,
    pub preset: Preset,
    pub seed: u64,
}
impl Config {
    pub fn roles(local_owners: SeatMask) -> Roles {
        Roles {
            inputs: SeatMask::ALL,
            heroes: HEROES,
            local_owners,
            director: Some(DIRECTOR),
        }
    }
    pub fn new(shared: SessionConfig, preset: Preset, seed: u64) -> Result<Self> {
        let config = Self {
            version: VERSION,
            shared,
            preset,
            seed,
        };
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        self.shared.validate().map_err(|_| Error::Shared)?;
        let roles = self.shared.roles;
        if self.version != VERSION
            || roles.inputs != SeatMask::ALL
            || roles.heroes != HEROES
            || roles.director != Some(DIRECTOR)
            || self.shared.coop.participants != HEROES
        {
            return Err(Error::Invalid(
                "versioned three-hero/four-input role manifest",
            ));
        }
        // Conservative whole-floor reservation, independent of starting density:
        // no director body may borrow a hero/progression/effect/companion quota.
        if self.shared.capacities.earthlings_per_floor
            < tje_simulation::population::ORIGINAL_POPULATION + crate::MAX_SPAWNS as u16
        {
            return Err(Error::Invalid("director body capacity is not reserved"));
        }
        self.preset
            .exit()
            .validate()
            .map_err(|_| Error::Invalid("objective exit"))?;
        match self.preset {
            Preset::Campaign if self.shared.generator != tje_world::GeneratorId::OriginalV1 => {
                return Err(Error::Invalid("campaign requires OriginalV1"));
            }
            Preset::ShortCourse { exit, .. } => {
                let course = self
                    .shared
                    .course
                    .as_ref()
                    .ok_or(Error::Invalid("short course manifest"))?;
                if course.sources.len() != 3
                    || !course.sources.iter().any(|source| source.level == exit)
                {
                    return Err(Error::Invalid(
                        "short preset requires three named floors and its exit",
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn identity(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"tje-overlord/config/1\0");
        hash.update(self.version.to_le_bytes());
        hash.update(self.shared.identity());
        hash.update(self.seed.to_le_bytes());
        hash.update([u8::from(matches!(self.preset, Preset::Campaign))]);
        hash.update(self.preset.required().to_le_bytes());
        hash.update(self.preset.exit().get().to_le_bytes());
        hash.update(self.preset.time_limit().unwrap_or(0).to_le_bytes());
        hash.finalize().into()
    }
}

pub const START_ENERGY: u16 = 40;
pub const ENERGY_CAP: u16 = 100;
pub const ENERGY_PERIOD: u8 = 60;
pub const POSSESSION_COST: u16 = 10;
pub const POSSESSION_COOLDOWN: u16 = 300;
pub const SPAWN_COOLDOWN: u16 = 900;
pub const TRAP_COOLDOWN: u16 = 300;
pub const TELEGRAPH_TICKS: u16 = 60;
pub const TRAP_LIFETIME: u16 = 600;
pub const SNARE_TICKS: u16 = 120;

pub const fn spawn_cost(kind: Kind) -> Option<u16> {
    match kind {
        Kind::Devil => Some(20),
        Kind::Dentist | Kind::DentistVariant => Some(25),
        Kind::Lawnmower | Kind::NerdHerd => Some(30),
        _ => None,
    }
}
