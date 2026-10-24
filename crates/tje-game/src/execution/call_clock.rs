//! Whole-call diagnostic resources, never a claim of source termination time.
//!
//! The clock provider records actual progress. A scope can tighten an enclosing
//! absolute deadline, but neither entering nor leaving it resets consumption.

use crate::FunctionId;
use std::{
    fmt,
    sync::atomic::{AtomicUsize, Ordering},
};

// Pointer-width atomics are supported by native and wasm32 targets. Allocate an
// identity only at the first required scope; never wrap and reuse an old owner.
static NEXT_CLOCK_OWNER: AtomicUsize = AtomicUsize::new(1);

fn allocate_clock_owner(counter: &AtomicUsize) -> Result<usize, CallClockError> {
    counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |owner| {
            if owner == 0 {
                None
            } else {
                owner.checked_add(1)
            }
        })
        .map_err(|_| CallClockError::IdentityExhausted)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallCeiling {
    pub ticks: u64,
    pub frames: u64,
    /// None adds no instruction limit; an enclosing finite limit still applies.
    pub reference_instructions: Option<u64>,
}

impl CallCeiling {
    /// Retain the existing generic tick ceiling. A frame ceiling is explicit too;
    /// neither limit implies that every source path completes within it.
    pub const DEFAULT: Self = Self {
        ticks: 1_000_000,
        frames: 1_000_000,
        reference_instructions: None,
    };

    pub const fn new(ticks: u64, frames: u64) -> Result<Self, CallClockError> {
        Self {
            ticks,
            frames,
            reference_instructions: None,
        }
        .validate()
    }

    pub const fn validate(self) -> Result<Self, CallClockError> {
        if self.ticks == 0 || self.frames == 0 || matches!(self.reference_instructions, Some(0)) {
            Err(CallClockError::InvalidCeiling)
        } else {
            Ok(self)
        }
    }

    pub const fn with_reference_instructions(
        mut self,
        maximum: u64,
    ) -> Result<Self, CallClockError> {
        self.reference_instructions = Some(maximum);
        self.validate()
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClockUsage {
    pub ticks: u64,
    /// Actual reference opcode attempts reaching the decoder/executor, including
    /// faulting attempts. IRQ recognition and zero-time Rust logic are not instructions.
    pub instructions: u64,
    pub master_clocks: u64,
    pub frames: u64,
    /// Actual supported scope entries, not CPU instructions or emulated time.
    /// The operational ceiling equals the configured tick ceiling.
    pub dispatch_steps: u64,
}

impl ClockUsage {
    pub fn since(self, earlier: Self) -> Result<Self, CallClockError> {
        Ok(Self {
            ticks: self
                .ticks
                .checked_sub(earlier.ticks)
                .ok_or(CallClockError::NonMonotonicClock)?,
            instructions: self
                .instructions
                .checked_sub(earlier.instructions)
                .ok_or(CallClockError::NonMonotonicClock)?,
            master_clocks: self
                .master_clocks
                .checked_sub(earlier.master_clocks)
                .ok_or(CallClockError::NonMonotonicClock)?,
            frames: self
                .frames
                .checked_sub(earlier.frames)
                .ok_or(CallClockError::NonMonotonicClock)?,
            dispatch_steps: self
                .dispatch_steps
                .checked_sub(earlier.dispatch_steps)
                .ok_or(CallClockError::NonMonotonicClock)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Deadline {
    ticks: u64,
    tick_owner: FunctionId,
    frames: u64,
    frame_owner: FunctionId,
    dispatch_steps: u64,
    dispatch_owner: FunctionId,
    instructions: Option<(u64, FunctionId)>,
}

/// Move-only LIFO token. It contains no SP or frame bookkeeping and cannot renew
/// an outer deadline when call_at, a bank switch or scope cleanup restores it.
#[derive(Debug)]
pub struct CallCeilingScope {
    clock_owner: usize,
    previous: Option<Deadline>,
    previous_serial: u64,
    serial: u64,
    entry_usage: ClockUsage,
    function: Option<FunctionId>,
}

impl CallCeilingScope {
    /// Only for a genuinely frozen/unsupported Atomic context with no active
    /// clock envelope. Progressing platforms use their ledger even for Atomic
    /// callees inside TimeAware or live Functional execution.
    pub const fn not_required() -> Self {
        Self {
            clock_owner: 0,
            previous: None,
            previous_serial: 0,
            serial: 0,
            entry_usage: ClockUsage {
                ticks: 0,
                instructions: 0,
                master_clocks: 0,
                frames: 0,
                dispatch_steps: 0,
            },
            function: None,
        }
    }

    pub fn is_required(&self) -> bool {
        self.function.is_some()
    }

    pub fn function(&self) -> Option<FunctionId> {
        self.function
    }
}

#[derive(Debug, Default)]
pub struct CallClock {
    owner: usize,
    usage: ClockUsage,
    deadline: Option<Deadline>,
    serial: u64,
    top_serial: u64,
    failure: Option<CallClockError>,
    scope_functions: Vec<FunctionId>,
}

impl Clone for CallClock {
    fn clone(&self) -> Self {
        Self {
            // A fork inherits consumption/limits, never authority over the
            // original provider's outstanding move-only scope tokens.
            owner: 0,
            usage: self.usage,
            deadline: self.deadline,
            serial: self.serial,
            top_serial: self.top_serial,
            failure: self.failure,
            scope_functions: self.scope_functions.clone(),
        }
    }
}

impl CallClock {
    pub fn usage(&self) -> ClockUsage {
        self.usage
    }
    pub fn active(&self) -> bool {
        self.deadline.is_some()
    }

    /// An enclosing operational scope already protects this function. This does
    /// not create a token, renew a deadline or transfer authority to a fork.
    pub fn contains_scope(&self, function: FunctionId) -> bool {
        self.scope_functions.contains(&function)
    }

    /// Latch an unsupported scope boundary without changing counters, deadlines
    /// or token ownership. Cleanup cannot turn a caught contract error into success.
    pub fn invalidate_scope(&mut self) {
        self.failure.get_or_insert(CallClockError::InvalidScope);
    }

    pub fn enter(
        &mut self,
        function: FunctionId,
        ceiling: CallCeiling,
    ) -> Result<CallCeilingScope, CallClockError> {
        ceiling.validate()?;
        self.check()?;
        if let Some(parent) = self.deadline
            && self.usage.dispatch_steps == parent.dispatch_steps
        {
            let error = CallClockError::CeilingExceeded {
                function: parent.dispatch_owner,
                resource: ClockResource::DispatchSteps,
                ceiling: parent.dispatch_steps,
                consumed: self.usage.dispatch_steps,
            };
            self.failure.get_or_insert(error);
            return Err(error);
        }
        // A looser child cannot overflow and reject a representable parent
        // deadline: the mathematical minimum is still the parent's limit.
        let ticks = self
            .usage
            .ticks
            .checked_add(ceiling.ticks)
            .or_else(|| self.deadline.map(|parent| parent.ticks))
            .ok_or(CallClockError::Overflow)?;
        let frames = self
            .usage
            .frames
            .checked_add(ceiling.frames)
            .or_else(|| self.deadline.map(|parent| parent.frames))
            .ok_or(CallClockError::Overflow)?;
        let serial = self.serial.checked_add(1).ok_or(CallClockError::Overflow)?;
        let dispatch_steps = self
            .usage
            .dispatch_steps
            .checked_add(ceiling.ticks)
            .or_else(|| self.deadline.map(|parent| parent.dispatch_steps))
            .ok_or(CallClockError::Overflow)?;
        let next_dispatch_steps = self
            .usage
            .dispatch_steps
            .checked_add(1)
            .ok_or(CallClockError::Overflow)?;
        let instructions = match ceiling.reference_instructions {
            Some(maximum) => {
                let absolute = self
                    .usage
                    .instructions
                    .checked_add(maximum)
                    .or_else(|| {
                        self.deadline
                            .and_then(|parent| parent.instructions.map(|limit| limit.0))
                    })
                    .ok_or(CallClockError::Overflow)?;
                Some((absolute, function))
            }
            None => None,
        };
        let mut deadline = Deadline {
            ticks,
            tick_owner: function,
            frames,
            frame_owner: function,
            dispatch_steps,
            dispatch_owner: function,
            instructions,
        };
        if let Some(parent) = self.deadline {
            if parent.ticks <= deadline.ticks {
                deadline.ticks = parent.ticks;
                deadline.tick_owner = parent.tick_owner;
            }
            if parent.frames <= deadline.frames {
                deadline.frames = parent.frames;
                deadline.frame_owner = parent.frame_owner;
            }
            if parent.dispatch_steps <= deadline.dispatch_steps {
                deadline.dispatch_steps = parent.dispatch_steps;
                deadline.dispatch_owner = parent.dispatch_owner;
            }
            if let Some(limit) = parent.instructions
                && deadline.instructions.is_none_or(|child| limit.0 <= child.0)
            {
                deadline.instructions = Some(limit);
            }
        }
        if self.owner == 0 {
            self.owner = allocate_clock_owner(&NEXT_CLOCK_OWNER)?;
        }
        let scope = CallCeilingScope {
            clock_owner: self.owner,
            previous: self.deadline,
            previous_serial: self.top_serial,
            serial,
            entry_usage: self.usage,
            function: Some(function),
        };
        self.deadline = Some(deadline);
        self.serial = serial;
        self.top_serial = serial;
        self.usage.dispatch_steps = next_dispatch_steps;
        self.scope_functions.push(function);
        Ok(scope)
    }

    /// One actual platform dispatch (for example entering a wait API). This
    /// counts diagnostic work only; it does not advance or estimate game time.
    pub fn dispatch_step(&mut self) -> Result<(), CallClockError> {
        self.check()?;
        if let Some(deadline) = self.deadline
            && self.usage.dispatch_steps == deadline.dispatch_steps
        {
            let error = CallClockError::CeilingExceeded {
                function: deadline.dispatch_owner,
                resource: ClockResource::DispatchSteps,
                ceiling: deadline.dispatch_steps,
                consumed: self.usage.dispatch_steps,
            };
            self.failure.get_or_insert(error);
            return Err(error);
        }
        self.usage.dispatch_steps = self
            .usage
            .dispatch_steps
            .checked_add(1)
            .ok_or(CallClockError::Overflow)?;
        Ok(())
    }

    /// Restore limits even when a ceiling was exceeded, while never restoring
    /// counters. The caller preserves any earlier semantic/MMIO/exit error.
    pub fn leave(&mut self, scope: CallCeilingScope) -> Result<ClockUsage, CallClockError> {
        if scope.function.is_none() {
            if self.active() {
                self.failure.get_or_insert(CallClockError::InvalidScope);
                return Err(CallClockError::InvalidScope);
            }
            return Ok(ClockUsage::default());
        }
        if self.owner == 0
            || scope.clock_owner != self.owner
            || self.top_serial != scope.serial
            || self.deadline.is_none()
            || self.scope_functions.last().copied() != scope.function
        {
            self.failure.get_or_insert(CallClockError::InvalidScope);
            return Err(CallClockError::InvalidScope);
        }
        let result = self
            .check()
            .and_then(|()| self.usage.since(scope.entry_usage));
        self.scope_functions.pop();
        self.deadline = scope.previous;
        self.top_serial = scope.previous_serial;
        if self.deadline.is_none() {
            self.failure = None;
        }
        result
    }

    pub fn before_tick(&mut self) -> Result<(), CallClockError> {
        self.check()?;
        if let Some(deadline) = self.deadline
            && self.usage.ticks == deadline.ticks
        {
            let error = CallClockError::CeilingExceeded {
                function: deadline.tick_owner,
                resource: ClockResource::Ticks,
                ceiling: deadline.ticks,
                consumed: self.usage.ticks,
            };
            self.failure.get_or_insert(error);
            return Err(error);
        }
        Ok(())
    }

    /// Record only actual platform progress. The provider checks after dispatch
    /// so a prior real MMIO/input error is not replaced with a ceiling error.
    pub fn record(&mut self, progress: ClockUsage) -> Result<(), CallClockError> {
        let next = (|| {
            Ok(ClockUsage {
                ticks: self
                    .usage
                    .ticks
                    .checked_add(progress.ticks)
                    .ok_or(CallClockError::Overflow)?,
                instructions: self
                    .usage
                    .instructions
                    .checked_add(progress.instructions)
                    .ok_or(CallClockError::Overflow)?,
                master_clocks: self
                    .usage
                    .master_clocks
                    .checked_add(progress.master_clocks)
                    .ok_or(CallClockError::Overflow)?,
                frames: self
                    .usage
                    .frames
                    .checked_add(progress.frames)
                    .ok_or(CallClockError::Overflow)?,
                dispatch_steps: self
                    .usage
                    .dispatch_steps
                    .checked_add(progress.dispatch_steps)
                    .ok_or(CallClockError::Overflow)?,
            })
        })();
        match next {
            Ok(next) => self.usage = next,
            Err(error) => {
                self.failure.get_or_insert(error);
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn check(&mut self) -> Result<(), CallClockError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if let Some(deadline) = self.deadline {
            if self.usage.ticks > deadline.ticks {
                let error = CallClockError::CeilingExceeded {
                    function: deadline.tick_owner,
                    resource: ClockResource::Ticks,
                    ceiling: deadline.ticks,
                    consumed: self.usage.ticks,
                };
                self.failure.get_or_insert(error);
                return Err(error);
            }
            if self.usage.frames > deadline.frames {
                let error = CallClockError::CeilingExceeded {
                    function: deadline.frame_owner,
                    resource: ClockResource::Frames,
                    ceiling: deadline.frames,
                    consumed: self.usage.frames,
                };
                self.failure.get_or_insert(error);
                return Err(error);
            }
            if let Some((maximum, owner)) = deadline.instructions
                && self.usage.instructions > maximum
            {
                let error = CallClockError::CeilingExceeded {
                    function: owner,
                    resource: ClockResource::Instructions,
                    ceiling: maximum,
                    consumed: self.usage.instructions,
                };
                self.failure.get_or_insert(error);
                return Err(error);
            }
            if self.usage.dispatch_steps > deadline.dispatch_steps {
                let error = CallClockError::CeilingExceeded {
                    function: deadline.dispatch_owner,
                    resource: ClockResource::DispatchSteps,
                    ceiling: deadline.dispatch_steps,
                    consumed: self.usage.dispatch_steps,
                };
                self.failure.get_or_insert(error);
                return Err(error);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockResource {
    Ticks,
    Frames,
    Instructions,
    DispatchSteps,
}

impl fmt::Display for ClockResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Ticks => "ticks",
            Self::Frames => "frames",
            Self::Instructions => "reference opcode attempts",
            Self::DispatchSteps => "dispatch_steps (scope/dispatch entries)",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallClockError {
    InvalidCeiling,
    InvalidScope,
    IdentityExhausted,
    Overflow,
    NonMonotonicClock,
    Unavailable,
    CeilingExceeded {
        function: FunctionId,
        resource: ClockResource,
        ceiling: u64,
        consumed: u64,
    },
}

impl fmt::Display for CallClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCeiling => formatter.write_str(
                "call diagnostic ceiling must have nonzero finite tick and frame limits",
            ),
            Self::InvalidScope => {
                formatter.write_str("call ceiling scopes must be consumed once in LIFO order")
            }
            Self::IdentityExhausted => {
                formatter.write_str("call clock owner identity space exhausted")
            }
            Self::Overflow => formatter.write_str("call diagnostic ceiling arithmetic overflow"),
            Self::NonMonotonicClock => formatter.write_str("call clock progress is not monotonic"),
            Self::Unavailable => {
                formatter.write_str("this platform has no whole-call clock capability")
            }
            Self::CeilingExceeded {
                function,
                resource,
                ceiling,
                consumed,
            } => write!(
                formatter,
                "{} ({}) diagnostic ceiling exceeded: {resource}, consumed={consumed}, absolute ceiling={ceiling}",
                function.metadata().name,
                function.metadata().inventory_id
            ),
        }
    }
}
impl std::error::Error for CallClockError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids;

    #[test]
    fn scope_membership_retains_ancestors_and_recursive_entries_without_renewal() {
        let mut clock = CallClock::default();
        let root = clock
            .enter(ids::PLAY_ENDING_SEQUENCE, CallCeiling::DEFAULT)
            .unwrap();
        let child = clock
            .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
            .unwrap();
        let recursive = clock
            .enter(ids::PLAY_ENDING_SEQUENCE, CallCeiling::DEFAULT)
            .unwrap();
        let before = clock.usage();
        assert!(clock.contains_scope(ids::PLAY_ENDING_SEQUENCE));
        assert!(clock.contains_scope(ids::WAIT_FOR_NEXT_V_BLANK));
        assert!(!clock.contains_scope(ids::GET_RANDOM_NUMBER));
        let fork = clock.clone();
        assert_eq!(fork.owner, 0);
        assert!(fork.contains_scope(ids::PLAY_ENDING_SEQUENCE));
        assert!(fork.contains_scope(ids::WAIT_FOR_NEXT_V_BLANK));
        assert_eq!(clock.usage(), before);
        clock.leave(recursive).unwrap();
        assert!(clock.contains_scope(ids::PLAY_ENDING_SEQUENCE));
        clock.leave(child).unwrap();
        assert!(!clock.contains_scope(ids::WAIT_FOR_NEXT_V_BLANK));
        assert!(clock.contains_scope(ids::PLAY_ENDING_SEQUENCE));
        clock.leave(root).unwrap();
        assert!(!clock.contains_scope(ids::PLAY_ENDING_SEQUENCE));
        assert!(clock.scope_functions.is_empty());
        assert!(clock.scope_functions.capacity() >= 3);
    }

    fn ticks(ticks: u64) -> ClockUsage {
        ClockUsage {
            ticks,
            master_clocks: ticks * 28,
            ..ClockUsage::default()
        }
    }

    #[test]
    fn nested_scopes_never_renew_or_reset_root_consumption() {
        let mut clock = CallClock::default();
        let root = clock
            .enter(ids::PLAY_ENDING_SEQUENCE, CallCeiling::new(10, 10).unwrap())
            .unwrap();
        clock.record(ticks(7)).unwrap();
        let nested = clock
            .enter(
                ids::WAIT_FOR_NEXT_V_BLANK,
                CallCeiling::new(10, 10).unwrap(),
            )
            .unwrap();
        clock.record(ticks(3)).unwrap();
        clock.leave(nested).unwrap();
        assert!(matches!(
            clock.before_tick(),
            Err(CallClockError::CeilingExceeded {
                function: ids::PLAY_ENDING_SEQUENCE,
                ..
            })
        ));
        assert!(clock.leave(root).is_err());
        assert_eq!(clock.usage().ticks, 10);
        assert!(!clock.active());
    }

    #[test]
    fn tighter_nested_ceiling_restores_only_limits() {
        let mut clock = CallClock::default();
        let root = clock
            .enter(ids::PLAY_ENDING_SEQUENCE, CallCeiling::new(20, 20).unwrap())
            .unwrap();
        let nested = clock
            .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::new(2, 2).unwrap())
            .unwrap();
        clock.record(ticks(2)).unwrap();
        clock.leave(nested).unwrap();
        clock.before_tick().unwrap();
        clock.record(ticks(3)).unwrap();
        assert_eq!(clock.leave(root).unwrap().ticks, 5);
    }

    #[test]
    fn looser_child_uses_representable_parent_deadlines_before_overflow() {
        let mut clock = CallClock::default();
        let root = clock
            .enter(ids::PLAY_ENDING_SEQUENCE, CallCeiling::new(20, 20).unwrap())
            .unwrap();
        clock.record(ticks(5)).unwrap();
        let child = clock
            .enter(
                ids::WAIT_FOR_NEXT_V_BLANK,
                CallCeiling::new(u64::MAX, u64::MAX).unwrap(),
            )
            .unwrap();
        clock.record(ticks(2)).unwrap();
        clock.leave(child).unwrap();
        assert_eq!(clock.leave(root).unwrap().ticks, 7);
    }

    #[test]
    fn frame_exhaustion_no_frame_spin_and_overflow_fail_closed() {
        let mut clock = CallClock::default();
        let root = clock
            .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::new(3, 1).unwrap())
            .unwrap();
        clock
            .record(ClockUsage {
                frames: 2,
                ..ClockUsage::default()
            })
            .unwrap();
        assert!(matches!(
            clock.leave(root),
            Err(CallClockError::CeilingExceeded {
                resource: ClockResource::Frames,
                ..
            })
        ));
        let scope = clock
            .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::new(3, 10).unwrap())
            .unwrap();
        clock.record(ticks(3)).unwrap();
        assert!(clock.before_tick().is_err());
        assert!(clock.leave(scope).is_err());
        assert!(CallCeiling::new(0, 1).is_err());
        clock
            .record(ClockUsage {
                ticks: u64::MAX - clock.usage().ticks,
                ..ClockUsage::default()
            })
            .unwrap();
        assert!(matches!(
            clock.enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::new(1, 1).unwrap()),
            Err(CallClockError::Overflow)
        ));
    }

    #[test]
    fn an_exhausted_child_cannot_be_hidden_by_scope_cleanup() {
        let mut clock = CallClock::default();
        let root = clock
            .enter(ids::PLAY_ENDING_SEQUENCE, CallCeiling::new(20, 20).unwrap())
            .unwrap();
        let child = clock
            .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::new(2, 2).unwrap())
            .unwrap();
        clock.record(ticks(2)).unwrap();
        assert!(clock.before_tick().is_err());
        assert!(clock.leave(child).is_err());
        assert!(clock.check().is_err());
        assert!(clock.leave(root).is_err());
        assert!(!clock.active());
        let next = clock
            .enter(ids::PLAY_ENDING_SEQUENCE, CallCeiling::new(20, 20).unwrap())
            .unwrap();
        clock.leave(next).unwrap();
    }

    #[test]
    fn zero_clock_rng_dispatches_exhaust_work_without_charging_time() {
        let mut clock = CallClock::default();
        let root = clock
            .enter(ids::PLAY_ENDING_SEQUENCE, CallCeiling::new(4, 20).unwrap())
            .unwrap();
        for _ in 0..3 {
            let rng = clock
                .enter(ids::GET_RANDOM_NUMBER, CallCeiling::DEFAULT)
                .unwrap();
            clock.leave(rng).unwrap();
        }
        assert!(matches!(
            clock.enter(ids::GET_RANDOM_NUMBER, CallCeiling::DEFAULT),
            Err(CallClockError::CeilingExceeded {
                resource: ClockResource::DispatchSteps,
                ..
            })
        ));
        assert!(clock.leave(root).is_err());
        assert_eq!(clock.usage().ticks, 0);
        assert_eq!(clock.usage().instructions, 0);
        assert_eq!(clock.usage().master_clocks, 0);
        assert_eq!(clock.usage().frames, 0);
        assert_eq!(clock.usage().dispatch_steps, 4);
    }

    #[test]
    fn out_of_order_scope_is_rejected() {
        let mut clock = CallClock::default();
        let root = clock
            .enter(ids::PLAY_ENDING_SEQUENCE, CallCeiling::DEFAULT)
            .unwrap();
        let _nested = clock
            .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
            .unwrap();
        assert_eq!(clock.leave(root), Err(CallClockError::InvalidScope));
    }

    #[test]
    fn foreign_clock_token_cannot_close_a_same_serial_scope() {
        let mut original = CallClock::default();
        let root = original
            .enter(
                ids::WAIT_FOR_NEXT_V_BLANK,
                CallCeiling::new(10, 10).unwrap(),
            )
            .unwrap();
        let mut foreign = CallClock::default();
        let forged = foreign
            .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
            .unwrap();
        let before = original.deadline.unwrap();
        assert_eq!(original.leave(forged), Err(CallClockError::InvalidScope));
        assert_eq!(original.deadline.unwrap().ticks, before.ticks);
        assert!(original.leave(root).is_err());
    }

    #[test]
    fn crafted_nested_foreign_token_cannot_install_its_looser_parent() {
        let mut victim = CallClock::default();
        let root = victim
            .enter(ids::PLAY_ENDING_SEQUENCE, CallCeiling::new(10, 10).unwrap())
            .unwrap();
        let nested = victim
            .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
            .unwrap();
        let mut foreign = CallClock::default();
        let _foreign_root = foreign
            .enter(ids::PLAY_ENDING_SEQUENCE, CallCeiling::DEFAULT)
            .unwrap();
        let forged = foreign
            .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
            .unwrap();
        assert_eq!(victim.leave(forged), Err(CallClockError::InvalidScope));
        assert_eq!(victim.deadline.unwrap().ticks, 10);
        assert_eq!(victim.top_serial, 2);
        assert!(
            victim
                .enter(ids::GET_RANDOM_NUMBER, CallCeiling::DEFAULT)
                .is_err()
        );
        assert!(victim.leave(nested).is_err());
        assert!(victim.leave(root).is_err());
    }

    #[test]
    fn cloning_never_transfers_scope_authority_and_frozen_clocks_stay_lazy() {
        let mut original = CallClock::default();
        original.leave(CallCeilingScope::not_required()).unwrap();
        assert_eq!(original.owner, 0);
        assert_eq!(original.clone().owner, 0);
        let root = original
            .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
            .unwrap();
        let mut fork = original.clone();
        assert_eq!(fork.owner, 0);
        assert_eq!(fork.leave(root), Err(CallClockError::InvalidScope));
        assert!(original.active());
    }

    #[test]
    fn owner_allocator_exhaustion_never_wraps_or_reuses_an_identity() {
        let counter = AtomicUsize::new(usize::MAX - 1);
        let last = allocate_clock_owner(&counter).unwrap();
        assert_eq!(last, usize::MAX - 1);
        for _ in 0..3 {
            assert_eq!(
                allocate_clock_owner(&counter),
                Err(CallClockError::IdentityExhausted)
            );
            assert_eq!(counter.load(Ordering::Relaxed), usize::MAX);
        }
        let reserved = AtomicUsize::new(0);
        assert_eq!(
            allocate_clock_owner(&reserved),
            Err(CallClockError::IdentityExhausted)
        );
        assert_eq!(reserved.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn instruction_limits_inherit_without_counter_reset_and_fail_closed() {
        let mut clock = CallClock::default();
        let policy = CallCeiling::new(20, 20)
            .unwrap()
            .with_reference_instructions(3)
            .unwrap();
        let root = clock.enter(ids::PLAY_ENDING_SEQUENCE, policy).unwrap();
        clock
            .record(ClockUsage {
                instructions: 2,
                ..ClockUsage::default()
            })
            .unwrap();
        let child = clock
            .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
            .unwrap();
        clock
            .record(ClockUsage {
                instructions: 2,
                ..ClockUsage::default()
            })
            .unwrap();
        assert!(matches!(
            clock.check(),
            Err(CallClockError::CeilingExceeded {
                function: ids::PLAY_ENDING_SEQUENCE,
                resource: ClockResource::Instructions,
                ceiling: 3,
                consumed: 4,
            })
        ));
        assert!(clock.leave(child).is_err());
        assert!(clock.leave(root).is_err());
        assert_eq!(clock.usage().instructions, 4);
    }

    #[test]
    fn narrower_instruction_child_restores_limits_not_consumption() {
        let mut clock = CallClock::default();
        let root = clock
            .enter(ids::PLAY_ENDING_SEQUENCE, CallCeiling::new(20, 20).unwrap())
            .unwrap();
        let policy = CallCeiling::new(20, 20)
            .unwrap()
            .with_reference_instructions(2)
            .unwrap();
        let child = clock.enter(ids::WAIT_FOR_NEXT_V_BLANK, policy).unwrap();
        clock
            .record(ClockUsage {
                instructions: 2,
                ..ClockUsage::default()
            })
            .unwrap();
        clock.leave(child).unwrap();
        clock
            .record(ClockUsage {
                instructions: 3,
                ..ClockUsage::default()
            })
            .unwrap();
        assert_eq!(clock.leave(root).unwrap().instructions, 5);
        assert!(CallCeiling::DEFAULT.with_reference_instructions(0).is_err());
    }

    #[test]
    fn looser_instruction_child_cannot_overflow_or_replace_parent_deadline() {
        let mut clock = CallClock::default();
        let policy = CallCeiling::DEFAULT
            .with_reference_instructions(10)
            .unwrap();
        let root = clock.enter(ids::PLAY_ENDING_SEQUENCE, policy).unwrap();
        clock
            .record(ClockUsage {
                instructions: 5,
                ..ClockUsage::default()
            })
            .unwrap();
        let child = clock
            .enter(
                ids::WAIT_FOR_NEXT_V_BLANK,
                CallCeiling::DEFAULT
                    .with_reference_instructions(u64::MAX)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            clock.deadline.unwrap().instructions,
            Some((10, ids::PLAY_ENDING_SEQUENCE))
        );
        clock.leave(child).unwrap();
        assert_eq!(clock.leave(root).unwrap().instructions, 5);
    }
}
