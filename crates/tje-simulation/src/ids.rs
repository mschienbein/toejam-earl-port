use bincode::{Decode, Encode};

/// A runtime body, never an original signed-byte object selector or pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Encode, Decode)]
pub struct ActorId {
    slot: u16,
    generation: u32,
}
impl ActorId {
    pub const CAPACITY: u16 = 1024;
    pub const HERO_START: u16 = 0;
    pub const POPULATION_START: u16 = 32;
    pub const POPULATION_CAPACITY: u16 = 320;
    pub const EFFECT_START: u16 = 352;
    pub const DROP_START: u16 = 416;

    pub const fn new(slot: u16, generation: u32) -> Option<Self> {
        if slot < Self::CAPACITY && generation != 0 {
            Some(Self { slot, generation })
        } else {
            None
        }
    }
    pub const fn slot(self) -> u16 {
        self.slot
    }
    pub const fn index(self) -> usize {
        self.slot as usize
    }
    pub const fn generation(self) -> u32 {
        self.generation
    }
    pub const fn valid(self) -> bool {
        Self::new(self.slot, self.generation).is_some()
    }
}

/// An authoritative effect pool reference, distinct from its optional draw body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Encode, Decode)]
pub struct EffectId {
    slot: u16,
    generation: u32,
}
impl EffectId {
    pub const CAPACITY: u16 = 64;
    pub const fn new(slot: u16, generation: u32) -> Option<Self> {
        if slot < Self::CAPACITY && generation != 0 {
            Some(Self { slot, generation })
        } else {
            None
        }
    }
    pub const fn slot(self) -> u16 {
        self.slot
    }
    pub const fn index(self) -> usize {
        self.slot as usize
    }
    pub const fn generation(self) -> u32 {
        self.generation
    }
    pub const fn valid(self) -> bool {
        Self::new(self.slot, self.generation).is_some()
    }
    pub fn actor(self) -> Option<ActorId> {
        Self::new(self.slot, self.generation)?;
        ActorId::new(ActorId::EFFECT_START + self.slot, self.generation)
    }
}

macro_rules! resource_id {
    ($name:ident, $width:ty, $limit:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Encode, Decode)]
        pub struct $name($width);
        impl $name {
            pub const fn new(value: $width) -> Option<Self> {
                if value < $limit {
                    Some(Self(value))
                } else {
                    None
                }
            }
            pub const fn get(self) -> $width {
                self.0
            }
            pub const fn valid(self) -> bool {
                Self::new(self.0).is_some()
            }
        }
    };
}

// Logical catalogue indices resolved from the user's ROM, not executable addresses.
resource_id!(DefinitionId, u16, 4096);
resource_id!(AnimationId, u16, 4096);
resource_id!(BehaviorId, u16, 4096);
resource_id!(FrameId, u32, 65536);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_reject_sentinels_and_keep_generations_distinct() {
        assert!(ActorId::new(1024, 1).is_none());
        assert!(ActorId::new(0, 0).is_none());
        assert!(EffectId::new(64, 1).is_none());
        assert_ne!(ActorId::new(32, 1), ActorId::new(32, 2));
        assert_eq!(EffectId::new(3, 7).unwrap().actor(), ActorId::new(355, 7));
        assert!(DefinitionId::new(4096).is_none());
        assert!(AnimationId::new(u16::MAX).is_none());
        assert!(FrameId::new(65536).is_none());
    }
}
