//! Whole-call operational ceilings and separately reported check-side progress.

use crate::{Hook, HostError, Result, execution::WaitClock};
use serde::Serialize;
use tje_game::{
    FunctionId,
    execution::{CallCeiling, CallCeilingScope, ClockUsage},
};

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) enum ClockRunPurpose {
    Baseline,
    IndependenceProbe { xor_mask: u8, byte: Option<u32> },
    DependentSeed { seed: u64, index: u16 },
    DependentByte { address: u32, value: u8 },
    DependentDmaFill { value: u8 },
    FixtureReplay,
    Replacement,
}

#[derive(Debug, Serialize)]
pub struct CallClockRecord {
    pub function: &'static str,
    pub side: &'static str,
    pub(crate) purpose: ClockRunPurpose,
    /// Operational resource limits, never elapsed source time or a bound claim.
    pub configured_tick_ceiling: u64,
    pub configured_frame_ceiling: u64,
    pub configured_reference_instruction_ceiling: Option<u64>,
    /// Actual scope/platform-dispatch entries, never CPU cycles.
    pub configured_dispatch_step_ceiling: u64,
    pub rationale: &'static str,
    pub reviewed_evidence: Option<&'static str>,
    pub proof_envelope: &'static str,
    pub explicit_proof_only_admission: bool,
    pub comparison_profile: &'static str,
    pub consumed_ticks: u64,
    pub consumed_reference_instructions: u64,
    pub consumed_master_clocks: u64,
    pub consumed_video_frames: u64,
    pub consumed_dispatch_steps: u64,
    pub actual_exit_pc: u32,
    pub actual_exit_sp: u32,
    /// Execution/scope completion only, not equivalence approval.
    pub completed: bool,
    pub failure: Option<String>,
}

impl CallClockRecord {
    pub(crate) fn new(
        hook: &Hook,
        side: &'static str,
        purpose: ClockRunPurpose,
        usage: ClockUsage,
        exit: (u32, u32),
        failure: Option<String>,
    ) -> Self {
        let ceiling = hook.clock_ceiling;
        Self {
            function: hook.symbol.name,
            side,
            purpose,
            configured_tick_ceiling: ceiling.ticks,
            configured_frame_ceiling: ceiling.frames,
            configured_reference_instruction_ceiling: ceiling.reference_instructions,
            configured_dispatch_step_ceiling: ceiling.ticks,
            rationale: match (&hook.symbol, hook.clock_evidence) {
                (_, Some(_)) => {
                    "Reviewed measured operational ceiling; not a source bound or approval"
                }
                (crate::hooks::HookSymbol::Authored(_), None) => {
                    "Authored diagnostic operational ceiling; no source termination bound is claimed"
                }
                (crate::hooks::HookSymbol::Generated(_), None) => {
                    "Generic one-million-tick operational ceiling; no source termination bound is claimed"
                }
            },
            reviewed_evidence: hook.clock_evidence,
            proof_envelope: hook.proof_envelope.as_str(),
            explicit_proof_only_admission: hook.proof_only_admission(),
            comparison_profile: match hook.check_route() {
                crate::CheckForkRoute::FrozenAtomic => "frozen Atomic ABI/RAM/device check",
                crate::CheckForkRoute::FrozenFunctional => {
                    "frozen Functional transaction-latency quotient"
                }
                crate::CheckForkRoute::TimeAwareLogic => "readable TimeAware ISR logic projection",
                crate::CheckForkRoute::ClockedCommutingInterrupts => {
                    "strict identity only; paired commutation relocation is not yet admitted; no quotient"
                }
                crate::CheckForkRoute::ClockedStrict => {
                    "full encoded physical state + raw CPU/RAM/DMA/timed-device/audio; no quotient"
                }
                crate::CheckForkRoute::ReplayedEnvironment => {
                    "recorded environment, own accesses exact; callees substituted and proven separately"
                }
            },
            consumed_ticks: usage.ticks,
            consumed_reference_instructions: usage.instructions,
            consumed_master_clocks: usage.master_clocks,
            consumed_video_frames: usage.frames,
            consumed_dispatch_steps: usage.dispatch_steps,
            actual_exit_pc: exit.0,
            actual_exit_sp: exit.1,
            completed: failure.is_none(),
            failure,
        }
    }
}

impl WaitClock {
    pub(crate) fn enter_call_ceiling(
        &mut self,
        function: FunctionId,
        ceiling: CallCeiling,
    ) -> Result<CallCeilingScope> {
        self.call_clock
            .enter(function, ceiling)
            .map_err(HostError::Clock)
    }

    pub(crate) fn leave_call_ceiling(&mut self, scope: CallCeilingScope) -> Result<ClockUsage> {
        self.call_clock.leave(scope).map_err(HostError::Clock)
    }
}
