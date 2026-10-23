use crate::{
    HostError, Result,
    abi::HookContext,
    catalogue::{FunctionCatalogue, FunctionSymbol},
};
use std::{collections::BTreeMap, ops::Deref, sync::Arc};
use tje_game::execution::ExecutionClass;
use tje_game::{
    Context, Dispatcher, FramedTranslation, FunctionId, TranslatedEntry, Translation,
    exits::{self, EntryOutcome, ExitContract},
};
use tje_rom::Revision;

pub type HookFn = fn(&mut HookContext<'_>) -> Result<Option<u32>>;
#[derive(Clone, Copy)]
enum Body {
    Host(HookFn),
    Game(Translation),
    Framed(FramedTranslation),
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
    pub execution_class: ExecutionClass,
    pub exit_contract: ExitContract,
    pub(crate) dependent_proof: Option<crate::dependent_equivalence::ProofSetting>,
    pub(crate) coverage_index: usize,
    body: Body,
    frame_state: Option<(tje_game::call_frame::FrameState, u32)>,
}
impl Hook {
    pub(crate) fn dependent_frame_sp(&self, current_sp: u32) -> u32 {
        self.frame_state
            .map_or(current_sp, |(frame, _)| frame.entry_sp)
    }

    pub(crate) fn execute(&self, context: &mut HookContext<'_>) -> Result<EntryOutcome> {
        tje_game::Platform::check_mmio_contract(context)?;
        if let Some((frame, expected_sp)) = self.frame_state {
            let sp = context.stack_pointer()?;
            let physical_sp = crate::abi::canonical_ram_address(sp);
            let physical_entry = crate::abi::canonical_ram_address(frame.entry_sp);
            if sp != expected_sp
                || sp & 1 != 0
                || frame.entry_sp & 1 != 0
                || !(crate::catalogue::STACK_BOTTOM..=crate::catalogue::STACK_TOP)
                    .contains(&physical_sp)
                || !(physical_sp..=crate::catalogue::STACK_TOP).contains(&physical_entry)
            {
                return Err(HostError::Metadata(format!(
                    "continuation frame/SP mismatch: saved entry={:08X}, expected current={expected_sp:08X}, actual={sp:08X}",
                    frame.entry_sp,
                )));
            }
            context.set_frame_state(frame)?;
        }
        let result = match self.body {
            Body::Host(function) => match function(context) {
                Ok(value) => Ok(value),
                Err(HostError::Translation(error)) => Err(error),
                Err(error) => {
                    tje_game::Platform::check_mmio_contract(context)?;
                    return Err(error);
                }
            },
            Body::Framed(translation) => {
                TranslatedEntry::Framed(translation).invoke_prepared(context)
            }
            Body::Game(translation) => {
                let mut arguments = [0; 16];
                let count = usize::from(self.symbol.abi.argument_count);
                for (index, argument) in arguments[..count].iter_mut().enumerate() {
                    *argument = context.argument_u32(index as u32);
                }
                (translation.function)(context, &arguments[..count])
            }
        };
        tje_game::Platform::check_mmio_contract(context)?;
        Ok(exits::validate_at(
            self.symbol.id,
            self.exit_contract,
            self.symbol.address(context.revision()),
            result,
            context.revision(),
        )?)
    }

    pub(crate) fn continuation(
        &self,
        stage: tje_game::exits::MainLoopStage,
        function: tje_game::TranslationFn,
        revision: Revision,
        frame: tje_game::call_frame::FrameState,
        expected_sp: u32,
    ) -> Result<(Self, u32)> {
        let pc = self
            .exit_contract
            .checkpoint_at(self.symbol.address(revision), stage)
            .ok_or_else(|| HostError::Metadata("not a reviewed main-loop continuation".into()))?;
        let mut hook = self.clone();
        hook.body = Body::Game(Translation {
            id: self.symbol.id,
            function,
        });
        hook.frame_state = Some((frame, expected_sp));
        Ok((hook, pc))
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
    pub(crate) fn set_dependent_proof(
        &mut self,
        id: FunctionId,
        setting: crate::dependent_equivalence::ProofSetting,
    ) {
        self.hooks
            .get_mut(&id.metadata().address(self.revision))
            .expect("validated dependent root")
            .dependent_proof = Some(setting);
    }

    #[cfg(test)]
    pub(crate) fn set_authored_dependent_proof(
        &mut self,
        address: u32,
        setting: crate::dependent_equivalence::ProofSetting,
    ) {
        self.hooks
            .get_mut(&address)
            .expect("authored hook")
            .dependent_proof = Some(setting);
    }

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
        dispatcher.validate_entries()?;
        for id in functions {
            if dispatcher.resolve_entry(*id)?.is_none() {
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
                registry.register_translation(translation)?;
            }
        }
        for &translation in tje_game::framed_translations() {
            let symbol = translation.id.metadata();
            if all || subsystems.contains(&symbol.subsystem) || functions.contains(&translation.id)
            {
                registry.register_framed_translation(translation)?;
            }
        }
        Ok(registry)
    }
    pub fn from_translations(
        revision: Revision,
        translations: &'static [Translation],
    ) -> Result<Self> {
        Self::from_entries(revision, translations, &[])
    }

    pub fn from_entries(
        revision: Revision,
        translations: &'static [Translation],
        framed: &'static [FramedTranslation],
    ) -> Result<Self> {
        let mut registry = Self::empty(revision);
        registry.dispatcher = Dispatcher::hybrid_with_frames(translations, framed);
        registry.dispatcher.validate_entries()?;
        for &translation in translations {
            registry.register_translation(translation)?;
        }
        for &translation in framed {
            registry.register_framed_translation(translation)?;
        }
        Ok(registry)
    }

    /// Opt-in pending proof: hook only supplied entries, but use them ahead of
    /// all proven translations when a checked Rust caller dispatches a callee.
    pub fn proof(revision: Revision, pending: &'static [Translation]) -> Result<Self> {
        Self::proof_with_frames(revision, pending, &[])
    }

    /// Select legacy proof roots without removing any pending/proven callees.
    pub fn proof_filtered(
        revision: Revision,
        pending: &'static [Translation],
        functions: &[FunctionId],
    ) -> Result<Self> {
        Self::proof_filtered_with_frames(revision, pending, &[], functions)
    }

    pub fn proof_with_frames(
        revision: Revision,
        pending: &'static [Translation],
        framed: &'static [FramedTranslation],
    ) -> Result<Self> {
        Self::proof_filtered_with_frames(revision, pending, framed, &[])
    }

    pub fn proof_filtered_with_frames(
        revision: Revision,
        pending: &'static [Translation],
        framed: &'static [FramedTranslation],
        functions: &[FunctionId],
    ) -> Result<Self> {
        for id in functions {
            if !pending.iter().any(|entry| entry.id == *id)
                && !framed.iter().any(|entry| entry.id == *id)
            {
                return Err(HostError::Metadata(format!(
                    "requested proof root is not a pending entry: {}",
                    id.metadata().name
                )));
            }
        }
        let mut registry = Self::empty(revision);
        registry.dispatcher = Dispatcher::hybrid_with_overrides_and_frames(pending, framed);
        registry.dispatcher.validate_entries()?;
        for translation in pending
            .iter()
            .copied()
            .map(TranslatedEntry::Values)
            .chain(framed.iter().copied().map(TranslatedEntry::Framed))
        {
            if functions.is_empty() || functions.contains(&translation.id()) {
                match translation {
                    TranslatedEntry::Values(entry) => registry.register_translation(entry)?,
                    TranslatedEntry::Framed(entry) => {
                        registry.register_framed_translation(entry)?
                    }
                }
            }
        }
        Ok(registry)
    }

    fn register_translation(&mut self, translation: Translation) -> Result<()> {
        let symbol = translation.id.metadata();
        let class = tje_game::execution_class(translation.id).ok_or_else(|| {
            HostError::Metadata(format!(
                "{} has no unique reviewed execution class",
                symbol.name
            ))
        })?;
        self.insert_execution(
            HookSymbol::Generated(symbol),
            class,
            Body::Game(translation),
        )
    }

    fn register_framed_translation(&mut self, translation: FramedTranslation) -> Result<()> {
        let symbol = translation.id.metadata();
        let class = tje_game::execution_class(translation.id).ok_or_else(|| {
            HostError::Metadata(format!(
                "{} has no unique reviewed execution class",
                symbol.name
            ))
        })?;
        self.insert_execution(
            HookSymbol::Generated(symbol),
            class,
            Body::Framed(translation),
        )
    }
    fn insert(&mut self, symbol: HookSymbol, atomic_capable: bool, body: Body) -> Result<()> {
        if !atomic_capable {
            return Err(HostError::NotAtomicCapable {
                function: symbol.name.to_owned(),
            });
        }
        self.insert_execution(symbol, ExecutionClass::Atomic, body)
    }

    fn insert_execution(
        &mut self,
        symbol: HookSymbol,
        execution_class: ExecutionClass,
        body: Body,
    ) -> Result<()> {
        let exit_contract = match symbol {
            HookSymbol::Generated(_) => exits::contract(symbol.id),
            HookSymbol::Authored(_) => ExitContract::Rts,
        };
        self.insert_exit(symbol, execution_class, exit_contract, body)
    }

    fn insert_exit(
        &mut self,
        symbol: HookSymbol,
        execution_class: ExecutionClass,
        exit_contract: ExitContract,
        body: Body,
    ) -> Result<()> {
        if symbol.kind != "function"
            && !(symbol.kind == "fragment" && exit_contract != ExitContract::Rts)
        {
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
        let coverage_index = self.hooks.len();
        self.hooks.insert(
            address,
            Hook {
                symbol,
                execution_class,
                exit_contract,
                coverage_index,
                body,
                frame_state: None,
                dependent_proof: None,
            },
        );
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

    /// Explicit reviewed non-atomic instrumentation contract. The execution class
    /// is never inferred from wave membership or an absent atomic declaration.
    pub fn register_execution_contract(
        &mut self,
        symbol: FunctionSymbol,
        execution_class: ExecutionClass,
        function: HookFn,
    ) -> Result<()> {
        self.insert_execution(
            HookSymbol::Authored(Arc::new(symbol)),
            execution_class,
            Body::Host(function),
        )
    }

    /// Authored non-RTS fixtures state both their clock and control-flow contract.
    /// Production registrations always use the reviewed game exit policy.
    pub fn register_exit_contract(
        &mut self,
        symbol: FunctionSymbol,
        execution_class: ExecutionClass,
        exit_contract: ExitContract,
        function: HookFn,
    ) -> Result<()> {
        self.insert_exit(
            HookSymbol::Authored(Arc::new(symbol)),
            execution_class,
            exit_contract,
            Body::Host(function),
        )
    }

    /// Production counterpart: a lane supplies the reviewed class explicitly.
    pub fn register_translation_with_execution(
        &mut self,
        translation: Translation,
        execution_class: ExecutionClass,
    ) -> Result<()> {
        self.insert_execution(
            HookSymbol::Generated(translation.id.metadata()),
            execution_class,
            Body::Game(translation),
        )
    }
    pub fn register_framed_translation_with_execution(
        &mut self,
        translation: FramedTranslation,
        execution_class: ExecutionClass,
    ) -> Result<()> {
        let same_body = matches!(
            self.dispatcher.resolve_entry(translation.id)?,
            Some(TranslatedEntry::Framed(entry))
                if std::ptr::fn_addr_eq(entry.function, translation.function)
        );
        if !same_body || tje_game::execution_class(translation.id) != Some(execution_class) {
            return Err(HostError::Metadata(format!(
                "framed hook {} must use its dispatcher body and reviewed execution class",
                translation.id.metadata().name
            )));
        }
        self.insert_execution(
            HookSymbol::Generated(translation.id.metadata()),
            execution_class,
            Body::Framed(translation),
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
            assert_eq!(
                registry.len(),
                tje_game::translations().count() + tje_game::framed_translations().count()
            );
            for id in tje_game::translations()
                .map(|entry| entry.id)
                .chain(tje_game::framed_translations().map(|entry| entry.id))
            {
                assert_eq!(
                    registry
                        .at(id.metadata().address(revision))
                        .unwrap()
                        .symbol
                        .id,
                    id
                );
                let hook = registry.at(id.metadata().address(revision)).unwrap();
                assert_eq!(Some(hook.execution_class), tje_game::execution_class(id));
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

    #[test]
    fn proof_filters_roots_but_retains_all_pending_overrides_and_proven_callees() {
        fn marker(_: &mut dyn tje_game::Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
            Ok(Some(42))
        }
        static PENDING: &[Translation] = &[
            Translation {
                id: ids::GET_RANDOM_NUMBER,
                function: marker,
            },
            Translation {
                id: ids::PRE_GENERATE_RANDOMS,
                function: marker,
            },
        ];
        static DUPLICATE: &[Translation] = &[PENDING[0], PENDING[0], PENDING[1]];
        for revision in [Revision::Rev02, Revision::Rev00] {
            let registry =
                HookRegistry::proof_filtered(revision, PENDING, &[ids::PRE_GENERATE_RANDOMS])
                    .unwrap();
            assert_eq!(registry.len(), 1);
            let selected = registry
                .at(ids::PRE_GENERATE_RANDOMS.metadata().address(revision))
                .unwrap();
            assert_eq!(selected.coverage_index, 0);
            assert!(
                registry
                    .at(ids::GET_RANDOM_NUMBER.metadata().address(revision))
                    .is_none()
            );
            for translation in PENDING {
                let resolved = registry.dispatcher().translation(translation.id).unwrap();
                assert!(std::ptr::fn_addr_eq(
                    resolved.function,
                    translation.function
                ));
            }
            for translation in tje_game::translations() {
                assert!(registry.dispatcher().translation(translation.id).is_some());
            }
            assert_eq!(
                HookRegistry::proof(revision, PENDING).unwrap().len(),
                PENDING.len()
            );
            assert!(HookRegistry::proof_filtered(revision, PENDING, &[ids::ABS]).is_err());
            assert!(
                HookRegistry::proof_filtered(revision, DUPLICATE, &[ids::PRE_GENERATE_RANDOMS])
                    .is_err()
            );
        }
    }

    #[test]
    fn mixed_proof_roots_share_callees_and_reject_conflicting_entry_forms() {
        fn values(_: &mut dyn tje_game::Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
            Ok(Some(11))
        }
        fn framed(_: &mut dyn tje_game::Context) -> tje_game::Result<Option<u32>> {
            Ok(Some(22))
        }
        static VALUES: &[Translation] = &[Translation {
            id: ids::PRE_GENERATE_RANDOMS,
            function: values,
        }];
        static FRAMED: &[FramedTranslation] = &[FramedTranslation {
            id: ids::ABS,
            function: framed,
        }];
        static CONFLICT: &[Translation] = &[Translation {
            id: ids::ABS,
            function: values,
        }];
        static DUPLICATE: &[FramedTranslation] = &[FRAMED[0], FRAMED[0]];
        for revision in [Revision::Rev02, Revision::Rev00] {
            let registry = HookRegistry::proof_filtered_with_frames(
                revision,
                VALUES,
                FRAMED,
                &[ids::PRE_GENERATE_RANDOMS],
            )
            .unwrap();
            assert_eq!(registry.len(), 1);
            let root = registry.iter().next().unwrap();
            assert_eq!(root.coverage_index, 0);
            assert_eq!(root.symbol.id, ids::PRE_GENERATE_RANDOMS);
            assert!(registry.at(ids::ABS.metadata().address(revision)).is_none());
            let callee = registry.dispatcher().framed_translation(ids::ABS).unwrap();
            assert!(std::ptr::fn_addr_eq(callee.function, FRAMED[0].function));
            // A framed override is effective instead of the proven value body.
            assert!(registry.dispatcher().translation(ids::ABS).is_none());
            assert!(
                registry
                    .dispatcher()
                    .translation(ids::GET_RANDOM_NUMBER)
                    .is_some()
            );
            assert_eq!(
                HookRegistry::proof_with_frames(revision, VALUES, FRAMED)
                    .unwrap()
                    .len(),
                2
            );
            assert!(HookRegistry::from_entries(revision, CONFLICT, FRAMED).is_err());
            assert!(HookRegistry::proof_with_frames(revision, VALUES, DUPLICATE).is_err());
            assert!(
                HookRegistry::proof_filtered_with_frames(
                    revision,
                    VALUES,
                    FRAMED,
                    &[ids::GET_RANDOM_NUMBER],
                )
                .is_err()
            );
        }
    }
}
