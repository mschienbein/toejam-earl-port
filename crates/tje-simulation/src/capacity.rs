use crate::{ActorId, Error, Result};
use bincode::{Decode, Encode};

pub const AUTHORITATIVE_LIMIT: u32 = 4 * 1024 * 1024;
pub const PRESENTATION_LIMIT: u32 = 64 * 1024 * 1024;
pub const FLOOR_RESERVATION: u32 = 48 * 1024;
pub const GENERATION_RESERVATION: u32 = 512 * 1024;
pub const CONTROL_RESERVATION: u32 = 128 * 1024;
pub const ACTOR_RESERVATION: u32 = 512;

/// Simultaneous worst-case requirements, not average occupancy or draw counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct CapacityPlan {
    pub floors: u16,
    pub active_floors: u16,
    pub actors: u16,
    pub earthlings_per_floor: u16,
    pub items_per_floor: u16,
    pub progression: u16,
    pub companions: u16,
    pub effects: u16,
    pub drops: u16,
    pub presentation_bytes: u32,
}
impl CapacityPlan {
    /// Four independent stock-density floors plus full global resource pools.
    /// The old proposed 256-entry arena cannot cover that simultaneous maximum.
    pub const STOCK_COOP: Self = Self {
        floors: 27,
        active_floors: 4,
        actors: 1024,
        earthlings_per_floor: 20,
        items_per_floor: 32,
        progression: 28,
        companions: 60,
        effects: 64,
        drops: 128,
        presentation_bytes: PRESENTATION_LIMIT,
    };
    pub fn reserve(self, heroes: u16) -> Result<Reservation> {
        if heroes == 0
            || heroes > 4
            || self.floors == 0
            || self.floors > 32
            || self.active_floors == 0
            || self.active_floors > heroes
            || self.active_floors > self.floors
            || self.actors > ActorId::CAPACITY
            || self.earthlings_per_floor > 64
            || self.items_per_floor > 128
            || self.progression > 28
            || self.companions > 60
            || self.effects > 64
            || self.drops > 128
            || self.presentation_bytes > PRESENTATION_LIMIT
        {
            return Err(Error::Capacity(
                "unsupported simultaneous capacity envelope",
            ));
        }
        let population = u32::from(self.active_floors) * u32::from(self.earthlings_per_floor)
            + u32::from(self.companions);
        let items = u32::from(self.active_floors) * u32::from(self.items_per_floor);
        if population > u32::from(ActorId::POPULATION_CAPACITY) || items > 480 {
            return Err(Error::Capacity("actor identity domain reservation"));
        }
        let actors = u32::from(heroes)
            + population
            + items
            + u32::from(self.progression)
            + u32::from(self.effects)
            + u32::from(self.drops);
        if actors > u32::from(self.actors) {
            return Err(Error::Capacity("required actors exceed the runtime arena"));
        }
        let authoritative_bytes = u32::from(self.floors) * FLOOR_RESERVATION
            + GENERATION_RESERVATION
            + CONTROL_RESERVATION
            + u32::from(self.actors) * ACTOR_RESERVATION;
        // Leaves runtime envelope/continuations/input/config room; never spend
        // the complete format-3 allowance on course geometry alone.
        if authoritative_bytes > AUTHORITATIVE_LIMIT {
            return Err(Error::Capacity(
                "worst-case authoritative snapshot reservation",
            ));
        }
        Ok(Reservation {
            required_actors: actors as u16,
            authoritative_bytes,
            presentation_bytes: self.presentation_bytes,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Reservation {
    pub required_actors: u16,
    pub authoritative_bytes: u32,
    pub presentation_bytes: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_all_independent_floors_and_full_effect_drop_pools() {
        let reservation = CapacityPlan::STOCK_COOP.reserve(4).unwrap();
        assert_eq!(reservation.required_actors, 592);
        assert!(reservation.authoritative_bytes < AUTHORITATIVE_LIMIT);
        let maximum = CapacityPlan {
            floors: 32,
            earthlings_per_floor: 64,
            items_per_floor: 112,
            ..CapacityPlan::STOCK_COOP
        }
        .reserve(4)
        .unwrap();
        assert_eq!(maximum.required_actors, 988);
        assert!(maximum.authoritative_bytes < AUTHORITATIVE_LIMIT);
    }

    #[test]
    fn impossible_joint_maxima_and_small_arenas_reject() {
        assert!(
            CapacityPlan {
                actors: 256,
                ..CapacityPlan::STOCK_COOP
            }
            .reserve(4)
            .is_err()
        );
        assert!(
            CapacityPlan {
                items_per_floor: 128,
                ..CapacityPlan::STOCK_COOP
            }
            .reserve(4)
            .is_err()
        );
        assert!(CapacityPlan::STOCK_COOP.reserve(3).is_err());
        assert!(
            CapacityPlan {
                floors: 33,
                ..CapacityPlan::STOCK_COOP
            }
            .reserve(4)
            .is_err()
        );
    }
}
