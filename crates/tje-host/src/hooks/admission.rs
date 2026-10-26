//! Body origin and diagnostic admission are independent barriers. The explicitly
//! check-only ReplayedEnvironment tier is never live/composition authority.
use super::{Body, HostError, Result};
use std::sync::atomic::{AtomicU64, Ordering};
use tje_game::{FramedTranslation, Translation};

/// Sealed process-local identity of the actual registered body instance.
/// No function/data-pointer membership test or serialized id can mint one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct BodyIdentity(u64);
impl BodyIdentity {
    pub(super) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("body identity space exhausted");
        Self(id)
    }
    /// Diagnostic output only; this number cannot reconstruct the sealed token.
    pub(crate) fn diagnostic_id(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyProvenance {
    CanonicalProduction,
    CandidateOverride,
    HostInstrumentation,
}

pub(super) fn body_provenance(body: Body) -> BodyProvenance {
    let canonical = match body {
        Body::Game(selected) | Body::Framed(selected) => selected.is_canonical_production(),
        Body::Continuation(_) => false,
        Body::Host(_) => return BodyProvenance::HostInstrumentation,
    };
    if canonical {
        BodyProvenance::CanonicalProduction
    } else {
        BodyProvenance::CandidateOverride
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct RegistryAdmission {
    pub proof_only: bool,
    pub noncanonical_dispatcher: bool,
    pub noncanonical_hooks: bool,
    pub replayed_environment: bool,
}

impl RegistryAdmission {
    pub fn record_dispatcher_entries(
        &mut self,
        values: &'static [Translation],
        framed: &'static [FramedTranslation],
    ) {
        // Caller-supplied tables are always explicit overlays, even if they
        // alias a canonical static table or contain identical function bodies.
        self.noncanonical_dispatcher |= !values.is_empty() || !framed.is_empty();
        self.replayed_environment |= values
            .iter()
            .map(|entry| entry.id)
            .chain(framed.iter().map(|entry| entry.id))
            .any(|id| {
                tje_game::execution_contract(id).is_some_and(|contract| {
                    contract.proof_envelope
                        == tje_game::execution::ProofEnvelope::ReplayedEnvironment
                })
            });
    }

    pub fn record_hook(&mut self, origin: BodyProvenance) {
        self.noncanonical_hooks |= origin != BodyProvenance::CanonicalProduction;
    }

    pub fn record_dispatcher_policy(&mut self, dispatcher: tje_game::Dispatcher) {
        self.replayed_environment |= dispatcher.requires_replayed_environment();
    }

    pub fn record_envelope(&mut self, envelope: tje_game::execution::ProofEnvelope) {
        self.replayed_environment |=
            envelope == tje_game::execution::ProofEnvelope::ReplayedEnvironment;
    }

    /// Constructors and registrations only add immutable bodies. Retaining each
    /// independent barrier makes the actual warm dispatch guard constant-time.
    pub fn ensure_replacement(&self) -> Result<()> {
        let reason = if self.replayed_environment {
            "ReplayedEnvironment is check-only, including effective unhooked callees"
        } else if self.proof_only {
            "explicit proof admission"
        } else if self.noncanonical_dispatcher {
            "noncanonical callee overlay, including unhooked entries"
        } else if self.noncanonical_hooks {
            "noncanonical or instrumentation hook body"
        } else {
            return Ok(());
        };
        Err(HostError::Metadata(format!(
            "replacement/play requires canonical production bodies: {reason}"
        )))
    }
}

#[cfg(test)]
mod tests;
