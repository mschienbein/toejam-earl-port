//! Check-only semantic equivalence under a recorded environment. Catalogued
//! callees are substituted and require separately correlated same-state proofs.
//! No value in this module grants live, source-timing or reset authority.
use crate::{
    HostError, Result,
    catalogue::{STACK_BOTTOM, STACK_TOP},
    hooks::{BodyIdentity, BodyProvenance, Hook},
};
use genesis_core::GenesisEmulator;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use tje_game::{
    FunctionId, SelectedEntry, TranslatedEntry,
    execution::{ExecutionClass, ProofEnvelope},
    exits::ExitContract,
};
use tje_rom::Revision;

mod context;
mod cursor;
mod frame;
mod host;
mod inputs;
mod ledger;
mod lifecycle;
mod record;
mod report;
mod strict;
mod wait;
pub(crate) use lifecycle::State;
pub use report::{
    BodyReport, ComparisonDetail, CompletedProofReport, ContractReport, FailurePacket,
    ObligationReport, ObligationStatus, OutputsReport, Report, RootReport, RootRole, StateBits,
    WitnessReport,
};
pub(crate) use strict::{
    CandidateObservation, CandidateRequest, FrozenStackAlignment, observe_candidate,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedKind {
    Envelope,
    LegacyArguments,
    ConcreteOrEphemeralBody,
    TypedControlProfile,
    EntryBoundary,
    Frame,
    CalleeInput,
    CalleeCoverage,
    RegisterDependency,
    IndirectControl,
    Observation,
    PlatformCapability,
    OperationalCeiling,
    ReferenceCompletion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unsupported {
    pub kind: UnsupportedKind,
    pub detail: String,
}

#[derive(Debug, Clone)]
enum Failure {
    Mismatch(String),
    Compared(Box<report::ComparisonDetail>),
    Unsupported(Unsupported),
    StackRead(tje_game::stack_read::Unsupported),
}
impl Failure {
    fn comparison(&self) -> Option<&report::ComparisonDetail> {
        match self {
            Self::Compared(detail) => Some(detail),
            _ => None,
        }
    }
    fn into_host_error(self, function: &str, frame: u64) -> HostError {
        match self {
            Self::StackRead(reason) => tje_game::Error::UnsupportedStackRead(reason).into(),
            Self::Unsupported(reason) => HostError::ReplayUnsupported {
                function: function.into(),
                reason,
            },
            Self::Mismatch(detail) => HostError::Mismatch {
                frame,
                function: function.into(),
                detail,
            },
            Self::Compared(detail) => HostError::Mismatch {
                frame,
                function: function.into(),
                detail: detail.detail,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Matched,
    ConditionalCallees,
    Mismatch,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Trajectory(u64);
impl Default for Trajectory {
    fn default() -> Self {
        Self(fresh())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Attempt(u64);
fn fresh() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        value.checked_add(1)
    })
    .expect("replay trajectory/attempt identity space exhausted")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyForm {
    Values,
    Framed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Profile {
    OrdinaryRts,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootKind {
    Replayed,
    StrictCanonical,
}

/// The admitted payload and its identity are inseparable. No pointer-membership,
/// serialized label, cached call count or state-shaped receipt constructs this.
pub(crate) struct RootAdmission {
    hook: Hook,
    selection: SelectedEntry,
    identity: RootIdentity,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RootIdentity {
    pub function: FunctionId,
    pub body: BodyIdentity,
    pub form: BodyForm,
    pub origin: BodyProvenance,
    pub class: ExecutionClass,
    pub envelope: ProofEnvelope,
    pub profile: Profile,
    pub trajectory: Trajectory,
    pub attempt: Attempt,
    invocation: record::InvocationId,
    pub kind: RootKind,
}
impl RootAdmission {
    pub fn hook(&self) -> &Hook {
        &self.hook
    }
    pub fn selection(&self) -> SelectedEntry {
        self.selection
    }
    pub fn identity(&self) -> RootIdentity {
        self.identity
    }
    /// Speculative children get immutable facts and distinct attempts, not the
    /// genuine live/reset/source capabilities consumed before fork creation.
    pub fn derived_identity(&self) -> RootIdentity {
        RootIdentity {
            attempt: Attempt(fresh()),
            ..self.identity
        }
    }
}

/// Instruction-class rule, not an endpoint-delta or catalogue-ID exception.
/// Idempotent full-SR writes still require the separate typed control profile.
fn ordinary_control_instruction(opcode: u16) -> bool {
    opcode & 0xfff0 == 0x4e60
        || opcode & 0xffc0 == 0x46c0
        || matches!(opcode, 0x007c | 0x027c | 0x0a7c | 0x4e70 | 0x4e72 | 0x4e73)
}

pub(crate) fn unsupported(
    hook: &Hook,
    kind: UnsupportedKind,
    detail: impl Into<String>,
) -> HostError {
    HostError::ReplayUnsupported {
        function: hook.symbol.name.to_owned(),
        reason: Unsupported {
            kind,
            detail: detail.into(),
        },
    }
}

/// Metadata/form validation does not mint admission or entry authority.
pub(crate) fn validate_configuration(hook: &Hook) -> Result<(SelectedEntry, BodyForm)> {
    if hook.proof_envelope != ProofEnvelope::ReplayedEnvironment {
        return Err(unsupported(
            hook,
            UnsupportedKind::Envelope,
            "root has no literal ReplayedEnvironment declaration",
        ));
    }
    if hook.exit_contract != ExitContract::Rts {
        return Err(unsupported(
            hook,
            UnsupportedKind::TypedControlProfile,
            "declared non-RTS root requires an explicitly reviewed typed control profile; no ordinary return is fabricated",
        ));
    }
    let selection = hook.replay_selection().ok_or_else(|| unsupported(hook,
        UnsupportedKind::ConcreteOrEphemeralBody,
        "concrete HookContext instrumentation and borrowed/ephemeral continuation payloads cannot obtain replay admission"))?;
    if selection.id() != hook.symbol.id {
        return Err(unsupported(
            hook,
            UnsupportedKind::Observation,
            "registered payload/id binding differs",
        ));
    }
    let form = match selection.entry() {
        TranslatedEntry::Values(_) if hook.symbol.abi.argument_count != 0 => {
            return Err(unsupported(
                hook,
                UnsupportedKind::LegacyArguments,
                "nonzero-argument eager-u32 legacy root is unsupported; migrate the pending body to FramedTranslation with lazy source-width slot reads",
            ));
        }
        TranslatedEntry::Values(_) => BodyForm::Values,
        TranslatedEntry::Framed(_) => BodyForm::Framed,
    };
    Ok((selection, form))
}

/// The single pre-fork admission point. An additive stronger profile must
/// consume genuine private entry provenance HERE before deriving child facts.
/// The ordinary profile never accepts a reset/exception state-shaped receipt.
fn admit_root(
    hook: &Hook,
    revision: Revision,
    live: &mut GenesisEmulator,
    trajectory: Trajectory,
    profile: Profile,
    entry: &record::Entry,
) -> Result<RootAdmission> {
    let (selection, form, kind) = if hook.proof_envelope == ProofEnvelope::ReplayedEnvironment {
        let (selection, form) = validate_configuration(hook)?;
        (selection, form, RootKind::Replayed)
    } else {
        let selected = hook.replay_selection().ok_or_else(|| {
            unsupported(
                hook,
                UnsupportedKind::ConcreteOrEphemeralBody,
                "independent strict root has no static readable payload",
            )
        })?;
        if hook.body_provenance() != BodyProvenance::CanonicalProduction
            || !selected.is_canonical_production()
            || selected.id() != hook.symbol.id
            || hook.exit_contract != ExitContract::Rts
        {
            return Err(unsupported(
                hook,
                UnsupportedKind::TypedControlProfile,
                "independent strict coverage requires its exact canonical ordinary-RTS root",
            ));
        }
        let form = match selected.entry() {
            TranslatedEntry::Values(_) => BodyForm::Values,
            TranslatedEntry::Framed(_) => BodyForm::Framed,
        };
        (selected, form, RootKind::StrictCanonical)
    };
    let cpu = live.hybrid_cpu();
    let sp = cpu.stack_pointer();
    let physical_sp = crate::abi::canonical_ram_address(sp);
    if entry.identity.trajectory() != trajectory
        || entry.function != Some(hook.symbol.id)
        || !matches!(entry.kind, record::EntryKind::Call { opcode, .. }
            if opcode & 0xffc0 == 0x4e80 || opcode & 0xff00 == 0x6100)
        || record::CpuSnapshot::capture(live) != entry.boundary.cpu
        || cpu.pc() != hook.symbol.address(revision)
        || sp & 1 != 0
        || !(STACK_BOTTOM..=STACK_TOP - 4).contains(&physical_sp)
        || cpu.status_register() & 0x8000 != 0
    {
        return Err(unsupported(
            hook,
            UnsupportedKind::EntryBoundary,
            "root lacks its exact actual JSR/entry lineage; privileged/exception/reset entry requires a stronger profile",
        ));
    }
    if kind == RootKind::Replayed && entry.boundary.dma.unresolved() {
        return Err(unsupported(
            hook,
            UnsupportedKind::PlatformCapability,
            format!(
                "ordinary replay cannot adopt unresolved entry DMA/command debt: {:?}",
                entry.boundary.dma
            ),
        ));
    }
    // Pending IRQ and non-DMA device phase are recorded, not made ready or skipped.
    // No candidate executes until this real original segment has completed.
    Ok(RootAdmission {
        hook: hook.clone(),
        selection,
        identity: RootIdentity {
            function: hook.symbol.id,
            body: hook.body_identity(),
            form,
            origin: hook.body_provenance(),
            class: hook.execution_class,
            envelope: hook.proof_envelope,
            profile,
            trajectory,
            attempt: Attempt(fresh()),
            invocation: entry.identity,
            kind,
        },
    })
}

#[cfg(all(test, feature = "reference-proof-hooks"))]
mod tests;

#[cfg(all(test, feature = "reference-proof-hooks"))]
mod real_roots;

#[cfg(all(test, not(feature = "reference-proof-hooks")))]
mod unavailable_tests {
    #[test]
    fn missing_actual_read_context_capability_rejects_before_a_body_or_rom_is_used() {
        fn never(_: &mut dyn tje_game::Context) -> tje_game::Result<Option<u32>> {
            panic!("a feature-off replay constructor must not execute a body")
        }
        static BODY: &[tje_game::FramedTranslation] = &[tje_game::FramedTranslation {
            id: tje_game::ids::WAIT_FOR_NEXT_V_BLANK,
            function: never,
        }];
        let result = crate::hooks::HookRegistry::proof_replayed_environment(
            crate::Revision::Rev02,
            &[],
            BODY,
            &[tje_game::ids::WAIT_FOR_NEXT_V_BLANK],
        );
        assert!(matches!(
            result,
            Err(crate::HostError::ReplayUnsupported {
                reason: super::Unsupported {
                    kind: super::UnsupportedKind::PlatformCapability,
                    ..
                },
                ..
            })
        ));
    }
}
