//! Reviewed execution and scheduling contracts, independent of the 68000 host.
//!
//! These classes describe a logic proof, not a cycle-equivalence promise. Native
//! platforms implement the same conditions using their frame/device scheduler.

mod call_clock;
pub use call_clock::{
    CallCeiling, CallCeilingScope, CallClock, CallClockError, ClockResource, ClockUsage,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionClass {
    /// No peripheral/interrupt progress is needed or observed.
    #[default]
    Atomic,
    /// Ordered transactions, with device transaction latency quotiented out.
    /// Direct VDP status/HV reads are forbidden; concurrent observers require review.
    Functional,
    /// Waits advance the platform and execute interrupts. Phase is not a logic output.
    TimeAware,
}

impl ExecutionClass {
    pub const fn permits(self, required: Self) -> bool {
        match self {
            Self::Atomic => matches!(required, Self::Atomic),
            Self::Functional => !matches!(required, Self::TimeAware),
            Self::TimeAware => true,
        }
    }
}

/// A check's observation envelope is independent of the source execution class.
/// Stronger diagnostic envelopes never grant permission to call a stronger class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProofEnvelope {
    #[default]
    ClassDefined,
    /// Both forks advance the actual clock and compare without a timing quotient.
    ClockedStrict,
    /// Actual IRQ transactions may move only with a trace-derived commutation proof.
    ClockedCommutingInterrupts,
}

impl ProofEnvelope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClassDefined => "ClassDefined",
            Self::ClockedStrict => "ClockedStrict",
            Self::ClockedCommutingInterrupts => "ClockedCommutingInterrupts",
        }
    }

    pub const fn progresses_time(self, class: ExecutionClass) -> bool {
        !matches!(self, Self::ClassDefined) || matches!(class, ExecutionClass::TimeAware)
    }
}

/// Literal owner metadata, not an activation or a source-termination bound.
/// Ordinary Atomic declarations remain in the owner's ATOMIC_CAPABLE list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionContract {
    pub function: crate::FunctionId,
    pub class: ExecutionClass,
    pub proof_envelope: ProofEnvelope,
    /// An operational resource ceiling. Exhaustion is a check failure.
    pub operational_ceiling: Option<CallCeiling>,
    /// The reviewed measurement receipt and observed maxima, never a bound claim.
    pub evidence: Option<&'static str>,
}

impl ExecutionContract {
    pub fn is_valid(self) -> bool {
        match (self.operational_ceiling, self.evidence) {
            (None, None) => true,
            (Some(ceiling), Some(evidence)) => ceiling.validate().is_ok() && !evidence.is_empty(),
            _ => false,
        }
    }
}

/// Explicitly observable condition. Device reads retain their real side effects;
/// this is not permission to invent ready bits or erase polling transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitCondition {
    RamWordChanged {
        address: u32,
        previous: u16,
    },
    DeviceWord {
        address: u32,
        mask: u16,
        value: u16,
    },
    DeviceByte {
        address: u32,
        mask: u8,
        value: u8,
    },
    /// Resume only after an actually accepted, unmasked interrupt handler returns.
    Interrupt,
    /// Source-backed join after independent work allowed a VRAM fill to finish.
    /// Unlike DMA status polling, this creates no game-visible device read.
    /// Only VRAM-fill mode is eligible; it is not a RAM-source DMA guarantee.
    VdpFillComplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitRequest {
    pub condition: WaitCondition,
    /// A failure bound, never a fabricated timing charge. Platforms must report
    /// an unsatisfied condition, not return success when this bound is exhausted.
    pub max_ticks: u64,
}

impl WaitRequest {
    pub const DEFAULT_MAX_TICKS: u64 = 1_000_000;

    pub const fn new(condition: WaitCondition) -> Self {
        Self {
            condition,
            max_ticks: Self::DEFAULT_MAX_TICKS,
        }
    }

    pub const fn vblank_frame(address: u32, previous: u16) -> Self {
        Self::new(WaitCondition::RamWordChanged { address, previous })
    }

    pub const fn dma_done() -> Self {
        Self::new(WaitCondition::DeviceWord {
            address: 0xc00004,
            mask: 2,
            value: 0,
        })
    }

    pub const fn vdp_fill_complete() -> Self {
        Self::new(WaitCondition::VdpFillComplete)
    }

    pub const fn z80_busack() -> Self {
        Self::new(WaitCondition::DeviceWord {
            address: 0xa11100,
            mask: 0x100,
            value: 0,
        })
    }

    pub const fn ym2612_ready() -> Self {
        Self::new(WaitCondition::DeviceByte {
            address: 0xa04000,
            mask: 0x80,
            value: 0,
        })
    }

    pub const fn interrupt() -> Self {
        Self::new(WaitCondition::Interrupt)
    }
}
