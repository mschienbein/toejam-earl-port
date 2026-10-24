//! Live admission is independent of the source class and owner check envelope.
//! Only a sealed selection from the trusted production lookup grants origin.
use super::{Body, HostError, Result};
use tje_game::{FramedTranslation, Translation};

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
    }

    pub fn record_hook(&mut self, origin: BodyProvenance) {
        self.noncanonical_hooks |= origin != BodyProvenance::CanonicalProduction;
    }

    /// Constructors and registrations only add immutable bodies. Retaining each
    /// independent barrier makes the actual warm dispatch guard constant-time.
    pub fn ensure_replacement(&self) -> Result<()> {
        let reason = if self.proof_only {
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
