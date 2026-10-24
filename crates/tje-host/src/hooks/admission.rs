//! Live admission is independent of the source class and owner check envelope.
//! Only the committed production entry form and body establish canonical origin.
use super::{Body, HostError, Result};
use std::sync::LazyLock;
use tje_game::{FramedTranslation, FunctionId, TranslatedEntry, Translation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyProvenance {
    CanonicalProduction,
    CandidateOverride,
    HostInstrumentation,
}

#[derive(Clone, Copy)]
enum CanonicalBody {
    Missing,
    Entry(TranslatedEntry),
    Conflict(FunctionId),
}

impl CanonicalBody {
    fn id(&self) -> Option<FunctionId> {
        match self {
            Self::Missing => None,
            Self::Entry(entry) => Some(entry.id()),
            Self::Conflict(id) => Some(*id),
        }
    }
}

fn index_canonical_bodies(
    entries: impl IntoIterator<Item = TranslatedEntry>,
) -> [CanonicalBody; 648] {
    let mut bodies = [CanonicalBody::Missing; 648];
    for (index, entry) in entries.into_iter().enumerate() {
        let Some(slot) = bodies.get_mut(index) else {
            // Invalid production metadata cannot approve any body.
            return [CanonicalBody::Conflict(entry.id()); 648];
        };
        *slot = CanonicalBody::Entry(entry);
    }
    bodies.sort_unstable_by_key(CanonicalBody::id);
    for index in 1..bodies.len() {
        if let Some(id) = bodies[index].id()
            && bodies[index - 1].id() == Some(id)
        {
            bodies[index - 1] = CanonicalBody::Conflict(id);
            bodies[index] = CanonicalBody::Conflict(id);
        }
    }
    bodies
}

// This fixed, immutable production-only index uses FunctionId's public ordering,
// not its crate-private representation. No heap or pending/proof table is used.
static CANONICAL_BODIES: LazyLock<[CanonicalBody; 648]> = LazyLock::new(|| {
    index_canonical_bodies(
        tje_game::translations()
            .copied()
            .map(TranslatedEntry::Values)
            .chain(
                tje_game::framed_translations()
                    .copied()
                    .map(TranslatedEntry::Framed),
            ),
    )
});

fn is_canonical(entry: TranslatedEntry) -> bool {
    let Ok(index) = CANONICAL_BODIES.binary_search_by_key(&Some(entry.id()), CanonicalBody::id)
    else {
        return false;
    };
    match (CANONICAL_BODIES.get(index), entry) {
        (
            Some(CanonicalBody::Entry(TranslatedEntry::Values(expected))),
            TranslatedEntry::Values(actual),
        ) => expected.id == actual.id && std::ptr::fn_addr_eq(expected.function, actual.function),
        (
            Some(CanonicalBody::Entry(TranslatedEntry::Framed(expected))),
            TranslatedEntry::Framed(actual),
        ) => expected.id == actual.id && std::ptr::fn_addr_eq(expected.function, actual.function),
        _ => false,
    }
}

pub(super) fn body_provenance(body: Body) -> BodyProvenance {
    let canonical = match body {
        Body::Game(entry) => is_canonical(TranslatedEntry::Values(entry)),
        Body::Framed(entry) => is_canonical(TranslatedEntry::Framed(entry)),
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
        self.noncanonical_dispatcher |= values
            .iter()
            .copied()
            .map(TranslatedEntry::Values)
            .chain(framed.iter().copied().map(TranslatedEntry::Framed))
            .any(|entry| !is_canonical(entry));
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
