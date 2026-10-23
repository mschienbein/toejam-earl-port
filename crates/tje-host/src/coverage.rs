//! Index-keyed counters: registration allocates once, replacement only updates
//! existing slots. Names are borrowed metadata and used solely for reports/queries.
use crate::{FunctionCoverage, hooks::HookRegistry};
use serde::{Serialize, Serializer, ser::SerializeMap};
use std::ops::Index;

#[derive(Debug, Default)]
struct Entry {
    name: &'static str,
    counts: FunctionCoverage,
}

#[derive(Debug, Default)]
pub struct FunctionCoverageTable {
    entries: Vec<Entry>,
}
impl FunctionCoverageTable {
    pub(crate) fn registered(registry: &HookRegistry) -> Self {
        let mut entries = Vec::with_capacity(registry.len());
        entries.resize_with(registry.len(), Entry::default);
        for hook in registry.iter() {
            entries[hook.coverage_index].name = hook.symbol.name;
        }
        Self { entries }
    }
    pub(crate) fn at_mut(&mut self, index: usize) -> &mut FunctionCoverage {
        &mut self.entries[index].counts
    }
    pub fn get(&self, name: &str) -> Option<&FunctionCoverage> {
        self.entries
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| &entry.counts)
    }
    pub fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &FunctionCoverage)> {
        self.entries.iter().map(|entry| (entry.name, &entry.counts))
    }
    pub fn values(&self) -> impl Iterator<Item = &FunctionCoverage> {
        self.entries.iter().map(|entry| &entry.counts)
    }
}
impl Index<&str> for FunctionCoverageTable {
    type Output = FunctionCoverage;
    fn index(&self, name: &str) -> &Self::Output {
        self.get(name)
            .unwrap_or_else(|| panic!("function not registered for coverage: {name}"))
    }
}
impl Serialize for FunctionCoverageTable {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for entry in &self.entries {
            map.serialize_entry(entry.name, &entry.counts)?;
        }
        map.end()
    }
}
