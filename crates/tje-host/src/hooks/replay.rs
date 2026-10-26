//! Bind effective selected payloads to process-local versions exactly once.
//! No pointer equality, status lookup or borrowed continuation can mint a route.
use super::{Result, admission::BodyIdentity};
use tje_game::{Dispatcher, FunctionId, SelectedEntry};

#[derive(Clone, Copy)]
pub(crate) struct ReplayBinding {
    selection: SelectedEntry,
    body: BodyIdentity,
}
impl ReplayBinding {
    pub(crate) fn selection(self) -> SelectedEntry {
        self.selection
    }
    pub(crate) fn body(self) -> BodyIdentity {
        self.body
    }
    pub(crate) fn id(self) -> FunctionId {
        self.selection.id()
    }
}

#[derive(Clone, Default)]
pub(super) struct ReplayRoutes {
    entries: Vec<ReplayBinding>,
}
impl ReplayRoutes {
    pub(super) fn bind(dispatcher: Dispatcher) -> Result<Self> {
        // This factory receives the registry's immutable effective dispatcher.
        // A copied binding carries its exact selected payload and nonce together.
        let mut entries = Vec::new();
        for function in tje_game::functions() {
            if let Some(selection) = dispatcher.resolve_entry(function.id)? {
                entries.push(ReplayBinding {
                    selection,
                    body: BodyIdentity::new(),
                });
            }
        }
        entries.sort_unstable_by_key(|entry| entry.id());
        Ok(Self { entries })
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = ReplayBinding> + '_ {
        self.entries.iter().copied()
    }
    pub(super) fn get(&self, id: FunctionId) -> Option<ReplayBinding> {
        self.entries
            .binary_search_by_key(&id, |entry| entry.id())
            .ok()
            .map(|index| self.entries[index])
    }
}
