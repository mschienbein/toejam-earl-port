//! Immutable shared bindings and reusable buffers; no live continuation authority.
use super::{BoundProgram, CallProfile, PortableError, TimingError};
use crate::timing_region::{ParkedRegion, RegionStorage, RegionUsage, RegionWork};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tje_game::{Dispatcher, FunctionId, TranslatedEntry};

struct PreparedSource {
    bound: Arc<BoundProgram>,
    spare: Vec<RegionStorage>,
    usage: Option<RegionUsage>,
}
#[derive(Default)]
pub struct PreparedSources {
    entries: BTreeMap<FunctionId, PreparedSource>,
}
pub struct ValidatedCalls {
    profile: CallProfile,
    originals: BTreeSet<FunctionId>,
    diagnostic_candidates: bool,
}
impl ValidatedCalls {
    pub fn permits_original(&self, profile: CallProfile, entry: FunctionId) -> bool {
        profile == self.profile && self.originals.contains(&entry)
    }
    /// Cache disposition read from sealed resolution, not canonical authority.
    pub fn has_diagnostic_candidates(&self) -> bool {
        self.diagnostic_candidates
    }
}
/// A storage lease for one active invocation, including recursive A→B→A calls.
/// It returns to its own function's pool only after its region borrow is gone.
pub struct ActiveSource {
    pub bound: Arc<BoundProgram>,
    pub storage: RegionStorage,
}
pub struct PausedSource {
    pub active: ActiveSource,
    pub continuation: ParkedRegion,
}
impl PausedSource {
    pub fn observe(
        &mut self,
        step: crate::DeviceStep,
        work: RegionWork,
    ) -> Result<(), TimingError> {
        self.continuation.observe_nested(step, work)
    }
}
impl PreparedSources {
    pub fn new(programs: impl IntoIterator<Item = Arc<BoundProgram>>) -> Result<Self, TimingError> {
        let mut entries = BTreeMap::new();
        for bound in programs {
            let storage = bound.prepare_storage();
            let entry = bound.entry();
            if entries
                .insert(
                    entry,
                    PreparedSource {
                        bound,
                        spare: vec![storage],
                        usage: None,
                    },
                )
                .is_some()
            {
                return Err(TimingError::InvalidIr("duplicate prepared source program"));
            }
        }
        Ok(Self { entries })
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn binding(&self, entry: FunctionId) -> Option<&Arc<BoundProgram>> {
        self.entries.get(&entry).map(|prepared| &prepared.bound)
    }
    pub fn usage(&self, entry: FunctionId) -> Option<RegionUsage> {
        self.entries.get(&entry).and_then(|prepared| prepared.usage)
    }
    pub fn acquire(&mut self, entry: FunctionId) -> Result<ActiveSource, TimingError> {
        let prepared = self
            .entries
            .get_mut(&entry)
            .ok_or(TimingError::WrongEntry)?;
        prepared.usage = None;
        let storage = prepared
            .spare
            .pop()
            .unwrap_or_else(|| prepared.bound.prepare_storage());
        Ok(ActiveSource {
            bound: Arc::clone(&prepared.bound),
            storage,
        })
    }
    pub fn release(&mut self, active: ActiveSource, usage: RegionUsage) -> Result<(), TimingError> {
        let prepared = self
            .entries
            .get_mut(&active.bound.entry())
            .ok_or(TimingError::WrongEntry)?;
        if !Arc::ptr_eq(&prepared.bound, &active.bound)
            || !active.storage.available_for(active.bound.timing())
        {
            return Err(TimingError::WrongToken);
        }
        prepared.usage = Some(usage);
        prepared.spare.push(active.storage);
        Ok(())
    }
    /// Only a trusted owner that has discarded the world or actually reset its
    /// hardware may recycle a suspended lease. No Context capability exposes it.
    pub fn discard_paused(&mut self, paused: PausedSource) -> Result<(), TimingError> {
        let PausedSource {
            mut active,
            continuation,
        } = paused;
        let usage = active.storage.discard_parked(continuation)?;
        self.release(active, usage)
    }
    /// Validate selected bodies before any source effect. Native transitional
    /// original calls also inspect the complete committed original-callee graph:
    /// every selected ready target encountered there must run its timed Rust body.
    pub fn validate_selection(
        &self,
        dispatcher: Dispatcher,
        profile: CallProfile,
    ) -> Result<ValidatedCalls, PortableError> {
        self.validate_selection_from(dispatcher, profile, None)
    }
    #[cfg(feature = "call-window-proofs")]
    pub(super) fn validate_window_selection(
        &self,
        dispatcher: Dispatcher,
        caller: FunctionId,
        callee: FunctionId,
    ) -> Result<ValidatedCalls, PortableError> {
        self.validate_selection_from(
            dispatcher,
            CallProfile::NativeCompiledOriginalCallees,
            Some((caller, callee)),
        )
    }
    fn validate_selection_from(
        &self,
        dispatcher: Dispatcher,
        profile: CallProfile,
        actual_call: Option<(FunctionId, FunctionId)>,
    ) -> Result<ValidatedCalls, PortableError> {
        let mut pending: Vec<_> = actual_call.into_iter().collect();
        let mut originals = BTreeSet::new();
        let mut diagnostic_candidates = false;
        for (&entry, prepared) in &self.entries {
            let selected = dispatcher.resolve_entry(entry)?;
            diagnostic_candidates |= selected.is_some_and(|entry| !entry.is_canonical_production());
            if !matches!(
                selected.map(|entry| entry.entry()),
                Some(TranslatedEntry::Framed(_))
            ) {
                return Err(TimingError::InvalidIr(
                    "timed source requires its selected framed Rust body",
                )
                .into());
            }
            for callee in prepared.bound.callees() {
                pending.push((entry, callee));
            }
        }
        let mut visited = BTreeSet::new();
        while let Some((caller, callee)) = pending.pop() {
            if !visited.insert((caller, callee)) {
                continue;
            }
            let caller_contract =
                tje_game::execution_contract(caller).ok_or(TimingError::WrongEntry)?;
            let callee_contract =
                tje_game::execution_contract(callee).ok_or(TimingError::WrongEntry)?;
            for contract in [caller_contract, callee_contract] {
                contract
                    .operational_ceiling
                    .unwrap_or(tje_game::execution::CallCeiling::DEFAULT)
                    .validate()
                    .map_err(tje_game::Error::from)?;
            }
            if !caller_contract.class.permits(callee_contract.class)
                || tje_game::exits::contract(callee) != tje_game::exits::ExitContract::Rts
            {
                return Err(TimingError::InvalidIr(
                    "source call crosses an unreviewed class or exit",
                )
                .into());
            }
            let selected = dispatcher.resolve_entry(callee)?;
            diagnostic_candidates |= selected.is_some_and(|entry| !entry.is_canonical_production());
            match selected.map(|entry| entry.entry()) {
                Some(TranslatedEntry::Framed(_)) if self.entries.contains_key(&callee) => {}
                Some(_) if profile == CallProfile::HostCheckOriginalSubstitution => {
                    originals.insert(callee);
                }
                Some(_) => {
                    return Err(TimingError::InvalidIr(
                        "selected Rust callee has no framed source timing roles",
                    )
                    .into());
                }
                None if profile == CallProfile::NativeStrictReadable => {
                    return Err(TimingError::InvalidIr(
                        "strict native source call has no selected timed Rust callee",
                    )
                    .into());
                }
                None if profile == CallProfile::NativeCompiledOriginalCallees => {
                    originals.insert(callee);
                    pending.extend(
                        callee
                            .metadata()
                            .callees
                            .iter()
                            .copied()
                            .map(|child| (callee, child)),
                    );
                }
                None => {
                    originals.insert(callee);
                }
            }
        }
        Ok(ValidatedCalls {
            profile,
            originals,
            diagnostic_candidates,
        })
    }
}
