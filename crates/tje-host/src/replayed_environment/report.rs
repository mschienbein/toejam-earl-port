//! Output-only evidence. None of these constructible/serializable data objects
//! is an admission, invocation, body, trajectory or successful-proof capability.
use super::{
    BodyForm, Profile, RootAdmission, Trajectory, Unsupported, Verdict,
    record::{Entry, EntryKind, Invocation},
};
use crate::{
    CheckFailure,
    hooks::{BodyProvenance, Hook, HookRegistry, ReplayBinding},
};
use serde::Serialize;
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u8 = 1;
pub const LABEL: &str = "replayed environment, callees substituted (proven separately)";

#[derive(Debug, Clone, Copy, Serialize)]
pub struct BodyReport {
    /// Sealed payload AND body-version binding, process-local only. An outer
    /// source/build manifest must never equate these numbers across runs.
    pub instance: u64,
    pub form: &'static str,
    pub origin: &'static str,
}
impl BodyReport {
    pub(super) fn admitted(admission: &RootAdmission) -> Self {
        let identity = admission.identity();
        Self {
            instance: identity.body.diagnostic_id(),
            form: match identity.form {
                BodyForm::Values => "values",
                BodyForm::Framed => "framed",
            },
            origin: origin(identity.origin),
        }
    }
    pub(super) fn bound(binding: ReplayBinding) -> Self {
        let selection = binding.selection();
        Self {
            instance: binding.body().diagnostic_id(),
            form: match selection.entry() {
                tje_game::TranslatedEntry::Values(_) => "values",
                tje_game::TranslatedEntry::Framed(_) => "framed",
            },
            origin: if selection.is_canonical_production() {
                "canonical_production"
            } else {
                "candidate_override"
            },
        }
    }
    fn registered(hook: &Hook) -> Result<Self, Unsupported> {
        let entry = hook.replay_selection().ok_or_else(|| Unsupported {
            kind: super::UnsupportedKind::ConcreteOrEphemeralBody,
            detail: "output registration requires a sealed static readable body, not concrete/borrowed instrumentation".into(),
        })?;
        Ok(Self {
            instance: hook.body_identity().diagnostic_id(),
            form: match entry.entry() {
                tje_game::TranslatedEntry::Values(_) => "values",
                tje_game::TranslatedEntry::Framed(_) => "framed",
            },
            origin: origin(hook.body_provenance()),
        })
    }
}
fn origin(origin: BodyProvenance) -> &'static str {
    match origin {
        BodyProvenance::CanonicalProduction => "canonical_production",
        BodyProvenance::CandidateOverride => "candidate_override",
        BodyProvenance::HostInstrumentation => "host_instrumentation",
    }
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ContractReport {
    pub execution_class: &'static str,
    pub proof_envelope: &'static str,
    pub route: &'static str,
    pub profile: &'static str,
}
impl ContractReport {
    pub(super) fn registered(hook: &Hook, profile: Profile) -> Self {
        Self {
            execution_class: match hook.execution_class {
                tje_game::execution::ExecutionClass::Atomic => "Atomic",
                tje_game::execution::ExecutionClass::Functional => "Functional",
                tje_game::execution::ExecutionClass::TimeAware => "TimeAware",
            },
            proof_envelope: hook.proof_envelope.as_str(),
            route: hook.check_route().as_str(),
            profile: match profile {
                Profile::OrdinaryRts => "ordinary_rts",
            },
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct WitnessReport {
    pub trajectory_id: u64,
    pub invocation_id: u64,
    pub parent_invocation_id: Option<u64>,
    pub entry_frame: u64,
    pub return_frame: Option<u64>,
    pub entry_pc: u32,
    pub entry_sp: u32,
    pub entry_sr: u16,
    pub exception_depth: u32,
    pub caller_pc: Option<u32>,
    pub return_pc: Option<u32>,
}
impl WitnessReport {
    pub(super) fn entry(entry: &Entry) -> Self {
        let (caller_pc, return_pc) = match entry.kind {
            EntryKind::Call {
                caller_pc,
                return_pc,
                ..
            } => (Some(caller_pc), Some(return_pc)),
            EntryKind::Interrupt { .. } => (None, None),
        };
        Self {
            trajectory_id: entry.identity.trajectory().0,
            invocation_id: entry.identity.serial(),
            parent_invocation_id: entry.parent.map(|id| id.serial()),
            entry_frame: entry.boundary.frame,
            return_frame: None,
            entry_pc: entry.boundary.cpu.pc(),
            entry_sp: entry.boundary.cpu.sp(),
            entry_sr: entry.boundary.cpu.sr() as u16,
            exception_depth: entry.boundary.cpu.exception_depth,
            caller_pc,
            return_pc,
        }
    }
    pub(super) fn completed(invocation: &Invocation) -> Self {
        Self {
            return_frame: Some(invocation.exit.frame),
            ..Self::entry(&invocation.entry)
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct StateBits {
    pub field: &'static str,
    pub bits: u32,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct OutputsReport {
    pub registers: Vec<StateBits>,
    /// All caller-visible/non-private effects, including mutable incoming
    /// argument slots and address-taken caller locals; no blanket stack mask.
    pub nonframe_ram: bool,
    pub ordered_device_effects: bool,
}
impl OutputsReport {
    pub(super) fn covered(coverage: &super::ledger::OutputCoverage) -> Self {
        const NAMES: [&str; 25] = [
            "D0",
            "D1",
            "D2",
            "D3",
            "D4",
            "D5",
            "D6",
            "D7",
            "A0",
            "A1",
            "A2",
            "A3",
            "A4",
            "A5",
            "A6",
            "USP",
            "SSP",
            "PC",
            "SR",
            "prefetch",
            "pending_interrupt",
            "stopped",
            "frozen",
            "address_error",
            "last_muldiv",
        ];
        let mut registers = Vec::with_capacity(tje_engine_core::ir::StateField::COUNT);
        for (field, bits) in coverage.state.iter() {
            registers.push(StateBits {
                field: NAMES[field.index() as usize],
                bits,
            });
        }
        Self {
            registers,
            nonframe_ram: coverage.nonframe_ram,
            ordered_device_effects: coverage.ordered_device_effects,
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RootRole {
    ReplayedRequested,
    StrictAuto,
}
#[derive(Debug, Clone, Serialize)]
pub struct RootReport {
    pub inventory_id: &'static str,
    pub name: &'static str,
    pub role: RootRole,
    pub body: BodyReport,
    pub effective_contract: ContractReport,
    pub entered: u64,
    pub reference_completed: u64,
    pub checked: u64,
    pub own_matches: u64,
    pub passed: u64,
    pub failed: u64,
    pub unsupported: u64,
    pub open: u64,
    pub provisional: u64,
    pub first_failure_index: Option<usize>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ComparisonDetail {
    pub point: &'static str,
    pub original_pc: Option<u32>,
    pub original_opcode: Option<u16>,
    pub own_access_index: Option<u64>,
    /// Only actually observed values. Older strict failures may not expose a
    /// structured operand; their complete canonical CheckFailure is retained.
    pub expected: Option<serde_json::Value>,
    pub actual: Option<serde_json::Value>,
    pub detail: String,
}
#[derive(Debug, Serialize)]
pub struct FailurePacket {
    pub inventory_id: &'static str,
    pub verdict: Verdict,
    pub witness: WitnessReport,
    pub body: BodyReport,
    pub effective_contract: ContractReport,
    pub unsupported: Option<Unsupported>,
    pub check: Option<CheckFailure>,
    pub comparison: Option<ComparisonDetail>,
}
#[derive(Debug, Clone, Serialize)]
pub struct CompletedProofReport {
    pub witness: WitnessReport,
    pub body: BodyReport,
    pub effective_contract: ContractReport,
    pub checked_outputs: OutputsReport,
    pub result: &'static str,
}
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationStatus {
    Pending,
    Discharged,
    Unsupported,
    Failed,
}
#[derive(Debug, Clone, Serialize)]
pub struct ObligationReport {
    pub id: u64,
    pub caller_invocation_id: u64,
    pub callee_inventory_id: &'static str,
    pub callee_witness: WitnessReport,
    pub body: Option<BodyReport>,
    pub effective_contract: Option<ContractReport>,
    pub required_outputs: OutputsReport,
    pub status: ObligationStatus,
    pub discharged_by: Option<CompletedProofReport>,
    pub reason: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u8,
    pub label: &'static str,
    pub trajectory_id: u64,
    pub attempted: u64,
    pub reference_completions: u64,
    pub own_matches: u64,
    pub completed_proofs: u64,
    pub substitutions: u64,
    pub open_segments: u64,
    pub unsupported: u64,
    pub mismatches: u64,
    pub unresolved_obligations: u64,
    pub finalized: bool,
    pub complete: bool,
    pub by_function: BTreeMap<&'static str, RootReport>,
    pub failures: Vec<FailurePacket>,
    pub obligations: Vec<ObligationReport>,
    pub last_verdict: Option<Verdict>,
    pub last_unsupported: Option<Unsupported>,
}
impl Report {
    pub(super) fn registered(
        trajectory: Trajectory,
        registry: &HookRegistry,
    ) -> Result<Self, Unsupported> {
        let mut by_function = BTreeMap::new();
        for hook in registry.iter() {
            let role =
                if hook.proof_envelope == tje_game::execution::ProofEnvelope::ReplayedEnvironment {
                    RootRole::ReplayedRequested
                } else {
                    RootRole::StrictAuto
                };
            by_function.insert(
                hook.symbol.inventory_id,
                RootReport {
                    inventory_id: hook.symbol.inventory_id,
                    name: hook.symbol.name,
                    role,
                    body: BodyReport::registered(hook)?,
                    effective_contract: ContractReport::registered(hook, Profile::OrdinaryRts),
                    entered: 0,
                    reference_completed: 0,
                    checked: 0,
                    own_matches: 0,
                    passed: 0,
                    failed: 0,
                    unsupported: 0,
                    open: 0,
                    provisional: 0,
                    first_failure_index: None,
                },
            );
        }
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            label: LABEL,
            trajectory_id: trajectory.0,
            attempted: 0,
            reference_completions: 0,
            own_matches: 0,
            completed_proofs: 0,
            substitutions: 0,
            open_segments: 0,
            unsupported: 0,
            mismatches: 0,
            unresolved_obligations: 0,
            finalized: false,
            complete: false,
            by_function,
            failures: Vec::new(),
            obligations: Vec::new(),
            last_verdict: None,
            last_unsupported: None,
        })
    }
}
