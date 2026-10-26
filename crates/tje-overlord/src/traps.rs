use crate::{
    DIRECTOR, Error, HEROES, LevelId, Result, SeatId, SeatMask,
    config::{SNARE_TICKS, TELEGRAPH_TICKS, TRAP_LIFETIME},
    placement::Point,
};
use bincode::{Decode, Encode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum TrapKind {
    Snare,
    Bump,
}
impl TrapKind {
    pub const fn cost(self) -> u16 {
        match self {
            Self::Snare => 10,
            Self::Bump => 15,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum Stage {
    Telegraph,
    Armed,
    Triggered,
    Expired,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct Trap {
    pub id: u32,
    pub kind: TrapKind,
    pub level: LevelId,
    pub point: Point,
    pub owner: SeatId,
    pub stage: Stage,
    pub arm_left: u16,
    pub life_left: u16,
    pub triggered: SeatMask,
}
impl Trap {
    pub fn new(id: u32, kind: TrapKind, level: LevelId, point: Point) -> Self {
        Self {
            id,
            kind,
            level,
            point,
            owner: DIRECTOR,
            stage: Stage::Telegraph,
            arm_left: TELEGRAPH_TICKS,
            life_left: TRAP_LIFETIME,
            triggered: SeatMask::NONE,
        }
    }
    pub const fn active(self) -> bool {
        matches!(self.stage, Stage::Telegraph | Stage::Armed)
    }
    pub fn validate(self) -> Result<()> {
        self.point.validate()?;
        self.level
            .validate()
            .map_err(|_| Error::Invalid("trap level"))?;
        if self.id == 0
            || self.owner != DIRECTOR
            || self.arm_left > TELEGRAPH_TICKS
            || self.life_left > TRAP_LIFETIME
            || self.triggered.bits() & !HEROES.bits() != 0
            || self.triggered.bits().count_ones() > 1
            || (self.stage == Stage::Triggered) != (self.triggered != SeatMask::NONE)
            || (self.active() && self.life_left == 0)
            || self.stage == Stage::Armed && self.arm_left != 0
        {
            return Err(Error::Invalid("trap lifecycle/owner"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hero {
    pub seat: SeatId,
    pub level: LevelId,
    pub x: i32,
    pub y: i32,
    /// The shared adapter includes effect immunity, menu, airborne/forced motion,
    /// service/elevator, arrival and respawn eligibility in this flag.
    pub eligible: bool,
}
impl Hero {
    fn overlaps(self, trap: &Trap) -> bool {
        let dx = i64::from(self.x) - i64::from(trap.point.x);
        let dy = i64::from(self.y) - i64::from(trap.point.y);
        self.level == trap.level && dx * dx + dy * dy <= 24 * 24
    }
}
/// Required aggregate transactions, never arbitrary HP writes or terrain edits.
pub trait Sink {
    fn hero(&self, seat: SeatId) -> Option<Hero>;
    fn may_arm(&mut self, trap: &Trap) -> Result<bool>;
    /// Shared player service preflights <=12 pixels, supported legal ground,
    /// collision/effect eligibility and objective/elevator exclusions atomically.
    fn bump(&mut self, seat: SeatId, level: LevelId, x: i32, y: i32) -> Result<bool>;
}

/// One contact phase, after shared movement; video timers are advanced separately.
/// Seat order is fixed 0,1,2 and each trap can succeed only once. No recursion.
pub fn resolve(
    traps: &mut [Option<Trap>; crate::MAX_TRAPS],
    snares: &mut [u16; 3],
    sink: &mut impl Sink,
) -> Result<()> {
    // Slot reuse is not trigger order: increasing stable ID wins.
    let mut order: [usize; crate::MAX_TRAPS] = std::array::from_fn(|index| index);
    order.sort_unstable_by_key(|&index| traps[index].map_or(u32::MAX, |trap| trap.id));
    for index in order {
        let Some(trap) = &mut traps[index] else {
            continue;
        };
        if trap.stage == Stage::Telegraph && trap.arm_left == 0 {
            if !sink.may_arm(trap)? {
                trap.stage = Stage::Expired;
                continue;
            }
            if HEROES
                .iter()
                .filter_map(|seat| sink.hero(seat))
                .any(|hero| hero.overlaps(trap))
            {
                continue;
            }
            trap.stage = Stage::Armed;
        }
        if trap.stage != Stage::Armed {
            continue;
        }
        for seat in HEROES.iter() {
            let Some(hero) = sink.hero(seat) else {
                continue;
            };
            if hero.seat != seat || !hero.eligible || !hero.overlaps(trap) {
                continue;
            }
            let accepted = match trap.kind {
                TrapKind::Snare => {
                    if snares[seat.index()] != 0 {
                        false
                    } else {
                        snares[seat.index()] = SNARE_TICKS;
                        true
                    }
                }
                TrapKind::Bump => {
                    let dx = hero.x - trap.point.x;
                    let dy = hero.y - trap.point.y;
                    let (x, y) = if dx == 0 && dy == 0 {
                        (hero.x, hero.y - 12)
                    } else if dx.unsigned_abs() >= dy.unsigned_abs() {
                        (hero.x + dx.signum() * 12, hero.y)
                    } else {
                        (hero.x, hero.y + dy.signum() * 12)
                    };
                    sink.bump(seat, trap.level, x, y)?
                }
            };
            if accepted {
                trap.stage = Stage::Triggered;
                trap.triggered = SeatMask::new(1 << seat.get()).expect("hero seat");
                break;
            }
        }
    }
    Ok(())
}
