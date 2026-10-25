use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize, de};
use std::{fmt, marker::PhantomData};

pub const SCHEMA: &str = "tje-maker/1";
pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CODE_BYTES: usize = 3 * 1024 * 1024;

fn nullable_u32<'de, D: de::Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    Option::<u32>::deserialize(deserializer)
}

// Do not trust a JSON array's size hint: enforce each bound before reading its
// next element, rather than collecting an unbounded array then validating it.
fn bounded<'de, D, T, const N: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: de::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Items<T, const N: usize>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const N: usize> de::Visitor<'de> for Items<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "an array containing at most {N} entries")
        }
        fn visit_seq<A: de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut items = Vec::new();
            while items.len() < N {
                match sequence.next_element()? {
                    Some(item) => items.push(item),
                    None => return Ok(items),
                }
            }
            if sequence.next_element::<de::IgnoredAny>()?.is_some() {
                return Err(de::Error::custom(format!("array exceeds {N} entries")));
            }
            Ok(items)
        }
    }
    deserializer.deserialize_seq(Items::<T, N>(PhantomData))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
pub enum Catalog {
    #[serde(rename = "tje91-rev02/1")]
    Rev02,
    #[serde(rename = "tje91-rev00/1")]
    Rev00,
}
impl Catalog {
    pub fn revision(self) -> tje_mem::Revision {
        match self {
            Self::Rev02 => tje_mem::Revision::Rev02,
            Self::Rev00 => tje_mem::Revision::Rev00,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(deny_unknown_fields)]
pub struct Course {
    pub schema: String,
    pub catalog: Catalog,
    pub title: String,
    pub author: String,
    pub description: String,
    pub runtime_seed: String,
    pub rules: Rules,
    pub start: Start,
    pub finish: Finish,
    #[serde(deserialize_with = "bounded::<_, _, 32>")]
    pub levels: Vec<Level>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum Ruleset {
    ClassicV1,
    OverlordV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub ruleset: Ruleset,
    pub heroes: u8,
    #[serde(deserialize_with = "nullable_u32")]
    pub tick_limit: Option<u32>,
    pub lives: u8,
    pub bucks: u8,
    #[serde(deserialize_with = "bounded::<_, _, 16>")]
    pub inventory: Vec<u8>,
    pub identify_presents: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(deny_unknown_fields)]
pub struct Start {
    pub level: u16,
    #[serde(deserialize_with = "bounded::<_, _, 4>")]
    pub spawns: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum FinishKind {
    ReachExit,
    CollectAndExit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(deny_unknown_fields)]
pub struct Finish {
    pub kind: FinishKind,
    pub exit: u32,
    #[serde(deserialize_with = "bounded::<_, _, 8192>")]
    pub required: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Earth,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum Music {
    Gameplay,
    Quiet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    pub id: u32,
    pub position: Point,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(deny_unknown_fields)]
pub struct Level {
    pub id: u16,
    pub width: u8,
    pub height: u8,
    pub environment: Environment,
    pub music: Music,
    #[serde(deserialize_with = "bounded::<_, _, 4096>")]
    pub cells: Vec<(u16, u16)>,
    #[serde(deserialize_with = "bounded::<_, _, 16>")]
    pub spawn_points: Vec<Spawn>,
    #[serde(deserialize_with = "bounded::<_, _, 256>")]
    pub objects: Vec<Object>,
    #[serde(deserialize_with = "bounded::<_, _, 64>")]
    pub links: Vec<Link>,
    pub fall_target: FallTarget,
    #[serde(deserialize_with = "bounded::<_, _, 64>")]
    pub paths: Vec<Path>,
}
impl Level {
    pub fn cell(&self, x: usize, y: usize) -> Option<u16> {
        if x >= usize::from(self.width) || y >= usize::from(self.height) {
            return None;
        }
        let mut index = y * usize::from(self.width) + x;
        for &(count, cell) in &self.cells {
            if index < usize::from(count) {
                return Some(cell);
            }
            index -= usize::from(count);
        }
        None
    }
    pub fn spawn(&self, id: u32) -> Option<Point> {
        self.spawn_points
            .iter()
            .find(|spawn| spawn.id == id)
            .map(|spawn| spawn.position)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum InitialState {
    Awake,
    Asleep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum Earthling {
    Devil,
    Hamster,
    Shopper,
    Dentist,
    NerdHerd,
    Whirlwind,
    Cupid,
    Shark,
    Lawnmower,
    Boogieman,
    Chicken,
    Mole,
    IceCreamTruck,
    Santa,
    DentistVariant,
    HulaDancer,
    Bees,
    BeesVariant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    Wiseman,
    Wizard,
    OperaSinger,
    Mailbox,
    LemonadeStand,
    HotTub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Item {
    Present { identity: u8 },
    Food { identity: u8 },
    Buck,
    Large,
    SinkingLarge,
    HurtHazard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Object {
    Earthling {
        id: u32,
        definition: Earthling,
        position: Point,
        initial: InitialState,
    },
    Service {
        id: u32,
        definition: Service,
        position: Point,
    },
    Item {
        id: u32,
        definition: Item,
        position: Point,
    },
    ShipPiece {
        id: u32,
        piece: u8,
        position: Point,
    },
    Telephone {
        id: u32,
        position: Point,
    },
    Elevator {
        id: u32,
        position: Point,
    },
    Exit {
        id: u32,
        position: Point,
    },
}
impl Object {
    pub fn id(&self) -> u32 {
        match self {
            Self::Earthling { id, .. }
            | Self::Service { id, .. }
            | Self::Item { id, .. }
            | Self::ShipPiece { id, .. }
            | Self::Telephone { id, .. }
            | Self::Elevator { id, .. }
            | Self::Exit { id, .. } => *id,
        }
    }
    pub fn position(&self) -> Point {
        match self {
            Self::Earthling { position, .. }
            | Self::Service { position, .. }
            | Self::Item { position, .. }
            | Self::ShipPiece { position, .. }
            | Self::Telephone { position, .. }
            | Self::Elevator { position, .. }
            | Self::Exit { position, .. } => *position,
        }
    }
    pub fn collectible(&self) -> bool {
        matches!(self, Self::Item { .. } | Self::ShipPiece { .. })
    }
    pub fn endpoint(&self) -> bool {
        matches!(self, Self::Elevator { .. } | Self::Exit { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    pub level: u16,
    pub spawn: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub endpoint: u32,
    pub destination: Destination,
    pub two_way: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FallTarget {
    Level { level: u16, spawn: u32 },
    LoseLifeAndRespawn { spawn: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(deny_unknown_fields)]
pub struct PathCell {
    pub x: u8,
    pub y: u8,
    pub directions: u8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(rename_all = "snake_case")]
pub enum PathTrigger {
    StepOn,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
#[serde(deny_unknown_fields)]
pub struct Path {
    pub id: u32,
    #[serde(deserialize_with = "bounded::<_, _, 4096>")]
    pub cells: Vec<PathCell>,
    pub trigger: PathTrigger,
    pub arming_ticks: u16,
    pub delay_ticks: u16,
    pub replacement: u16,
}
