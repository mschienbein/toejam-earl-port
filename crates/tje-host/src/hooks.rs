use crate::{
    HostError, Result,
    abi::HookContext,
    catalogue::{FunctionCatalogue, FunctionSymbol},
};
use std::{collections::BTreeMap, ops::Deref, sync::Arc};
use tje_game::{Dispatcher, FunctionId, Translation};
use tje_rom::Revision;

pub type HookFn = fn(&mut HookContext<'_>) -> Result<Option<u32>>;
#[derive(Clone, Copy)]
enum Body {
    Host(HookFn),
    Game(Translation),
}
/// Production metadata is borrowed without allocation. Authored instrumentation
/// contracts are owned and shared across registry clones without leaking memory.
#[derive(Clone)]
pub enum HookSymbol {
    Generated(&'static FunctionSymbol),
    Authored(Arc<FunctionSymbol>),
}
impl Deref for HookSymbol {
    type Target = FunctionSymbol;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Generated(symbol) => symbol,
            Self::Authored(symbol) => symbol,
        }
    }
}
#[derive(Clone)]
pub struct Hook {
    pub symbol: HookSymbol,
    body: Body,
}
impl Hook {
    pub(crate) fn execute(&self, context: &mut HookContext<'_>) -> Result<Option<u32>> {
        match self.body {
            Body::Host(function) => function(context),
            Body::Game(translation) => {
                let mut arguments = [0; 16];
                let count = usize::from(self.symbol.abi.argument_count);
                for (index, argument) in arguments[..count].iter_mut().enumerate() {
                    *argument = context.argument_u32(index as u32);
                }
                Ok((translation.function)(context, &arguments[..count])?)
            }
        }
    }
}

#[derive(Clone)]
pub struct HookRegistry {
    hooks: BTreeMap<u32, Hook>,
    revision: Revision,
    dispatcher: Dispatcher,
}
impl Default for HookRegistry {
    fn default() -> Self {
        Self::empty(Revision::Rev02)
    }
}
impl HookRegistry {
    pub fn empty(revision: Revision) -> Self {
        Self {
            hooks: BTreeMap::new(),
            revision,
            dispatcher: Dispatcher::hybrid(),
        }
    }
    pub fn translated(revision: Revision) -> Result<Self> {
        Self::filtered(revision, &[], &[])
    }
    /// Hook selection changes what is checked, not translated-callee dispatch.
    /// A selected caller still uses every Rust callee available in the game.
    pub fn filtered(
        revision: Revision,
        subsystems: &[&str],
        functions: &[FunctionId],
    ) -> Result<Self> {
        for name in subsystems {
            if tje_game::subsystem(name).is_none() {
                return Err(HostError::Metadata(format!("unknown subsystem: {name}")));
            }
        }
        let dispatcher = Dispatcher::hybrid();
        for id in functions {
            if dispatcher.translation(*id).is_none() {
                return Err(HostError::Metadata(format!(
                    "requested function is not translated: {}",
                    id.metadata().name
                )));
            }
        }
        let mut registry = Self::empty(revision);
        let all = subsystems.is_empty() && functions.is_empty();
        for &translation in tje_game::translations() {
            let symbol = translation.id.metadata();
            if all || subsystems.contains(&symbol.subsystem) || functions.contains(&translation.id)
            {
                registry.insert(
                    HookSymbol::Generated(symbol),
                    tje_game::is_atomic_capable(symbol.id),
                    Body::Game(translation),
                )?;
            }
        }
        Ok(registry)
    }
    pub fn from_translations(
        revision: Revision,
        translations: &'static [Translation],
    ) -> Result<Self> {
        let mut registry = Self::empty(revision);
        registry.dispatcher = Dispatcher::hybrid_with(translations);
        for &translation in translations {
            registry.insert(
                HookSymbol::Generated(translation.id.metadata()),
                tje_game::is_atomic_capable(translation.id),
                Body::Game(translation),
            )?;
        }
        Ok(registry)
    }
    fn insert(&mut self, symbol: HookSymbol, atomic_capable: bool, body: Body) -> Result<()> {
        if !atomic_capable {
            return Err(HostError::NotAtomicCapable {
                function: symbol.name.to_owned(),
            });
        }
        if symbol.kind != "function" {
            return Err(HostError::Metadata(format!(
                "not a function entry: {}",
                symbol.name
            )));
        }
        if symbol.abi.argument_count > 16 {
            return Err(HostError::Metadata(format!(
                "too many stack arguments: {}",
                symbol.name
            )));
        }
        let address = symbol.address(self.revision);
        if self.hooks.contains_key(&address) {
            return Err(HostError::Metadata(format!(
                "duplicate hook at {address:06X}"
            )));
        }
        self.hooks.insert(address, Hook { symbol, body });
        Ok(())
    }
    /// Explicit host-side instrumentation hook; production game bodies live only
    /// in tje-game. This also lets mismatch tests mutate the real bridge.
    pub fn register(
        &mut self,
        catalogue: &FunctionCatalogue,
        name: &str,
        function: HookFn,
    ) -> Result<()> {
        if catalogue.revision() != self.revision {
            return Err(HostError::Metadata(
                "hook/catalogue revision mismatch".into(),
            ));
        }
        let symbol = catalogue
            .named(name)
            .ok_or_else(|| HostError::Metadata(format!("function not catalogued: {name}")))?;
        self.insert(
            HookSymbol::Generated(symbol),
            tje_game::is_atomic_capable(symbol.id),
            Body::Host(function),
        )
    }
    /// An explicit synthetic/instrumentation contract. The caller supplies its
    /// real ABI and atomic capability; production entries use subsystem-owned lists.
    pub fn register_contract(
        &mut self,
        symbol: FunctionSymbol,
        atomic_capable: bool,
        function: HookFn,
    ) -> Result<()> {
        self.insert(
            HookSymbol::Authored(Arc::new(symbol)),
            atomic_capable,
            Body::Host(function),
        )
    }
    pub fn revision(&self) -> Revision {
        self.revision
    }
    pub(crate) fn dispatcher(&self) -> Dispatcher {
        self.dispatcher
    }
    pub fn at(&self, address: u32) -> Option<&Hook> {
        self.hooks.get(&address)
    }
    pub fn iter(&self) -> impl Iterator<Item = &Hook> {
        self.hooks.values()
    }
    pub fn len(&self) -> usize {
        self.hooks.len()
    }
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tje_game::ids;
    #[test]
    fn complete_revision_tables_and_rejected_duplicates() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let registry = HookRegistry::translated(revision).unwrap();
            assert_eq!(registry.len(), tje_game::translations().count());
            for translation in tje_game::translations() {
                assert_eq!(
                    registry
                        .at(translation.id.metadata().address(revision))
                        .unwrap()
                        .symbol
                        .id,
                    translation.id
                );
            }
        }
        fn dummy(_: &mut HookContext<'_>) -> Result<Option<u32>> {
            unreachable!()
        }
        let mut registry = HookRegistry::translated(Revision::Rev02).unwrap();
        let catalogue = FunctionCatalogue::new(Revision::Rev02);
        assert!(registry.register(&catalogue, "gRNGState", dummy).is_err());
        assert!(
            registry
                .register(&catalogue, ids::GET_RANDOM_NUMBER.metadata().name, dummy)
                .is_err()
        );
    }

    #[test]
    fn filters_select_hook_entries_without_disabling_rust_callees() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let map = HookRegistry::filtered(revision, &["map"], &[]).unwrap();
            assert_eq!(map.len(), tje_game::map::TRANSLATIONS.len());
            assert!(map.iter().all(|hook| hook.symbol.subsystem == "map"));
            assert!(map.at(ids::LMULT.metadata().address(revision)).is_none());
            assert!(map.dispatcher().translation(ids::LMULT).is_some());
            let union = HookRegistry::filtered(revision, &["camera"], &[ids::ABS]).unwrap();
            assert_eq!(union.len(), tje_game::camera::TRANSLATIONS.len() + 1);
            let one = HookRegistry::filtered(revision, &[], &[ids::PRE_GENERATE_RANDOMS]).unwrap();
            assert_eq!(one.len(), 1);
            assert!(
                one.dispatcher()
                    .translation(ids::GET_RANDOM_NUMBER)
                    .is_some()
            );
            assert_eq!(
                HookRegistry::filtered(revision, &["audio"], &[])
                    .unwrap()
                    .len(),
                tje_game::audio::TRANSLATIONS.len()
            );
            assert!(HookRegistry::filtered(revision, &["unknown"], &[]).is_err());
            assert!(HookRegistry::filtered(revision, &[], &[ids::FURTHER_INITIALISATION]).is_err());
        }
    }
}
