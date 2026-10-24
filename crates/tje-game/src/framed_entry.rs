//! Frame-consuming entries read their argument slots at the source use site.
//!
//! A legacy value snapshot and a prepared frame are different entry contracts;
//! neither invocation silently manufactures the other.
use crate::{
    Context, Dispatcher, Error, FunctionId, Result, SUBSYSTEMS, Translation, call_frame::FrameError,
};

pub type FramedTranslationFn = fn(&mut dyn Context) -> Result<Option<u32>>;

#[derive(Clone, Copy)]
pub struct FramedTranslation {
    pub id: FunctionId,
    pub function: FramedTranslationFn,
}

/// Borrowed execution payload. This is not production-admission authority:
/// callers may construct a temporary payload for immediate diagnostic execution.
#[derive(Clone, Copy)]
pub enum TranslatedEntry<'a> {
    Values(&'a Translation),
    Framed(&'a FramedTranslation),
}

impl TranslatedEntry<'_> {
    pub const fn id(self) -> FunctionId {
        match self {
            Self::Values(translation) => translation.id,
            Self::Framed(translation) => translation.id,
        }
    }

    /// Ordinary value-only calls do not have a source-backed callee frame.
    pub fn invoke_values(
        self,
        context: &mut dyn Context,
        arguments: &[u32],
    ) -> Result<Option<u32>> {
        match self {
            Self::Values(translation) => (translation.function)(context, arguments),
            Self::Framed(_) => Err(Error::Frame(FrameError::PreparedEntryRequired(self.id()))),
        }
    }

    /// Already prepared original-address entry. Framed bodies consume only their
    /// actual source reads; legacy bodies retain an audited value snapshot.
    pub fn invoke_prepared(self, context: &mut dyn Context) -> Result<Option<u32>> {
        match self {
            Self::Framed(translation) => {
                let frame = context.frame_state().ok_or(FrameError::Unavailable)?;
                let sp = context.stack_pointer()?;
                if sp & 1 != 0 || frame.entry_sp & 1 != 0 {
                    return Err(FrameError::UnalignedStack(if sp & 1 != 0 {
                        sp
                    } else {
                        frame.entry_sp
                    })
                    .into());
                }
                if sp != frame.entry_sp {
                    return Err(FrameError::UnexpectedStack {
                        expected: frame.entry_sp,
                        actual: sp,
                    }
                    .into());
                }
                context.set_frame_state(frame.for_prepared_entry(translation.id))?;
                (translation.function)(context)
            }
            Self::Values(translation) if translation.id.metadata().abi.argument_count == 0 => {
                (translation.function)(context, &[])
            }
            Self::Values(translation) => {
                let frame = context.frame_state().ok_or(FrameError::Unavailable)?;
                let count = usize::from(translation.id.metadata().abi.argument_count);
                let mut arguments = [0; 16];
                if count > arguments.len() {
                    return Err(FrameError::TooManyArguments(count).into());
                }
                for (index, argument) in arguments[..count].iter_mut().enumerate() {
                    *argument = frame.argument_slot(index as i32).read(context);
                }
                (translation.function)(context, &arguments[..count])
            }
        }
    }

    /// Explicit root/continuation entry. Values are only a legacy convenience;
    /// a framed body consumes already prepared RAM, never a second value source.
    pub fn invoke_entry(self, context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
        match self {
            Self::Values(translation) => (translation.function)(context, arguments),
            Self::Framed(_) if arguments.is_empty() => self.invoke_prepared(context),
            Self::Framed(_) => Err(Error::Frame(FrameError::UnexpectedValueArguments {
                id: self.id(),
                count: arguments.len(),
            })),
        }
    }
}

/// Immutable selection from a dispatcher. Only trusted production resolution
/// grants canonical origin; extracting its raw payload cannot recreate authority.
///
/// A raw execution payload cannot be wrapped into a canonical selection:
/// ```compile_fail
/// use tje_game::{Dispatcher, SelectedEntry, ids};
/// let raw = Dispatcher::hybrid().resolve_entry(ids::ABS).unwrap().unwrap().entry();
/// let _: SelectedEntry = raw.into();
/// ```
/// Nor can a caller attach an origin flag to its chosen payload:
/// ```compile_fail
/// use tje_game::{Dispatcher, SelectedEntry, ids};
/// let raw = Dispatcher::hybrid().resolve_entry(ids::ABS).unwrap().unwrap().entry();
/// let _ = SelectedEntry { entry: raw, canonical_production: true };
/// ```
#[derive(Clone, Copy)]
pub struct SelectedEntry {
    entry: TranslatedEntry<'static>,
    canonical_production: bool,
}

impl SelectedEntry {
    pub const fn id(self) -> FunctionId {
        self.entry.id()
    }

    pub const fn entry(self) -> TranslatedEntry<'static> {
        self.entry
    }

    pub const fn is_canonical_production(self) -> bool {
        self.canonical_production
    }

    pub fn invoke_values(
        self,
        context: &mut dyn Context,
        arguments: &[u32],
    ) -> Result<Option<u32>> {
        self.entry.invoke_values(context, arguments)
    }

    pub fn invoke_prepared(self, context: &mut dyn Context) -> Result<Option<u32>> {
        self.entry.invoke_prepared(context)
    }
}

impl<'a> From<&'a Translation> for TranslatedEntry<'a> {
    fn from(translation: &'a Translation) -> Self {
        Self::Values(translation)
    }
}

impl<'a> From<&'a FramedTranslation> for TranslatedEntry<'a> {
    fn from(translation: &'a FramedTranslation) -> Self {
        Self::Framed(translation)
    }
}

impl Dispatcher {
    /// A bounded mixed registry. The same framed body serves a hook, a prepared
    /// Rust call and CPU-free native dispatch; no argument-snapshot adapter exists.
    pub const fn hybrid_with_frames(
        translations: &'static [Translation],
        framed_translations: &'static [FramedTranslation],
    ) -> Self {
        Self {
            translations,
            framed_translations,
            subsystems: &[],
            original_fallback: true,
        }
    }

    /// Explicit proof overlays retain every proven callee in either entry form.
    pub const fn hybrid_with_overrides_and_frames(
        translations: &'static [Translation],
        framed_translations: &'static [FramedTranslation],
    ) -> Self {
        Self {
            translations,
            framed_translations,
            subsystems: SUBSYSTEMS,
            original_fallback: true,
        }
    }

    /// Resolve the effective entry. An explicit overlay replaces a proven body,
    /// but two candidates within either layer are an error, never ROM fallback.
    pub fn resolve_entry(self, id: FunctionId) -> Result<Option<SelectedEntry>> {
        let mut overrides = self
            .translations
            .iter()
            .filter(|entry| entry.id == id)
            .map(TranslatedEntry::Values)
            .chain(
                self.framed_translations
                    .iter()
                    .filter(|entry| entry.id == id)
                    .map(TranslatedEntry::Framed),
            );
        let entry = overrides.next();
        if overrides.next().is_some() {
            return Err(FrameError::ConflictingEntries(id).into());
        }
        if let Some(entry) = entry {
            return Ok(Some(SelectedEntry {
                entry,
                canonical_production: false,
            }));
        }
        // Every public constructor sets this private catalogue to either the
        // committed SUBSYSTEMS or empty. Caller tables always use the explicit
        // override route above, even when their data happens to alias production.
        let mut proven = self
            .subsystems
            .iter()
            .flat_map(|owner| {
                owner
                    .translations
                    .iter()
                    .map(TranslatedEntry::Values)
                    .chain(
                        owner
                            .framed_translations
                            .iter()
                            .map(TranslatedEntry::Framed),
                    )
            })
            .filter(|entry| entry.id() == id);
        let entry = proven.next();
        if proven.next().is_some() {
            return Err(FrameError::ConflictingEntries(id).into());
        }
        Ok(entry.map(|entry| SelectedEntry {
            entry,
            canonical_production: true,
        }))
    }

    pub fn framed_translation(self, id: FunctionId) -> Option<&'static FramedTranslation> {
        match self.resolve_entry(id).ok().flatten()?.entry() {
            TranslatedEntry::Framed(translation) => Some(translation),
            _ => None,
        }
    }

    /// Validate uniqueness and execution declarations for every supplied body,
    /// including pending callees which a root filter does not hook.
    pub fn validate_entries(self) -> Result<()> {
        let supplied = self
            .translations
            .iter()
            .map(|entry| entry.id)
            .chain(self.framed_translations.iter().map(|entry| entry.id));
        let proven = self.subsystems.iter().flat_map(|owner| {
            owner
                .translations
                .iter()
                .map(|entry| entry.id)
                .chain(owner.framed_translations.iter().map(|entry| entry.id))
        });
        fn validate_layer(ids: impl Iterator<Item = FunctionId>) -> Result<()> {
            let mut seen = [false; 648];
            for id in ids {
                if std::mem::replace(&mut seen[usize::from(id.0)], true) {
                    return Err(FrameError::ConflictingEntries(id).into());
                }
                crate::execution_class(id).ok_or(Error::Unclassified(id))?;
            }
            Ok(())
        }
        validate_layer(supplied)?;
        validate_layer(proven)
    }
}
