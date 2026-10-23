//! Address-preserving game translations, independent of the emulator and frontends.
//!
//! Storage is accessed through `tje_mem::Bus`/typed views; device accesses use
//! `Platform`. Calls use inventory identities, never host pointers or ROM offsets.

use std::fmt;
use tje_mem::{Bus, Revision, arithmetic::ArithmeticTrap};

pub mod audio;
pub mod call_frame;
pub mod camera;
pub mod collision;
pub mod core;
pub mod cutscenes;
pub mod earthlings;
pub mod elevator;
pub mod execution;
pub mod jamout;
pub mod map;
mod metadata;
pub mod platform;
pub mod player;
pub mod presents;
pub mod runtime;
pub mod ship;
pub mod speech;
pub mod sprites;
pub mod ui;
pub use metadata::ids;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FunctionId(u16);

impl FunctionId {
    pub fn metadata(self) -> &'static FunctionMetadata {
        let index = usize::from(self.0);
        &metadata::FUNCTION_CHUNKS[index / 108][index % 108]
    }
}

#[derive(Debug, Clone)]
pub struct Abi {
    pub kind: &'static str,
    pub signature: &'static str,
    /// Complete inventory ABI, including assembly input/output registers and evidence.
    pub details_json: &'static str,
    pub argument_count: u8,
    pub returns_value: bool,
    pub preserved_data: u8,
    pub preserved_address: u8,
    /// Handwritten assembly has no compiler-level scratch-register abstraction.
    pub strict_registers: bool,
}

#[derive(Debug, Clone)]
pub struct FunctionMetadata {
    pub id: FunctionId,
    pub inventory_id: &'static str,
    pub name: &'static str,
    pub addresses: [u32; 2],
    pub sizes: [u32; 2],
    pub classification: &'static str,
    pub kind: &'static str,
    pub subsystem: &'static str,
    pub source: &'static str,
    pub source_line: u32,
    pub wave: u8,
    pub callees: &'static [FunctionId],
    pub abi: Abi,
}

impl FunctionMetadata {
    pub fn address(&self, revision: Revision) -> u32 {
        self.addresses[revision_index(revision)]
    }
    pub fn size(&self, revision: Revision) -> u32 {
        self.sizes[revision_index(revision)]
    }
}

const fn revision_index(revision: Revision) -> usize {
    match revision {
        Revision::Rev02 => 0,
        Revision::Rev00 => 1,
    }
}

pub fn functions() -> impl Iterator<Item = &'static FunctionMetadata> + Clone {
    metadata::FUNCTION_CHUNKS
        .iter()
        .flat_map(|chunk| chunk.iter())
}

pub fn function_at(revision: Revision, address: u32) -> Option<FunctionId> {
    functions()
        .find(|function| function.address(revision) == address)
        .map(|function| function.id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Untranslated(FunctionId),
    Unclassified(FunctionId),
    ExecutionUnavailable {
        caller: execution::ExecutionClass,
        callee: execution::ExecutionClass,
    },
    Arithmetic(ArithmeticTrap),
    Frame(call_frame::FrameError),
    Platform(String),
}
impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Untranslated(id) => {
                write!(formatter, "untranslated function: {}", id.metadata().name)
            }
            Self::Unclassified(id) => write!(
                formatter,
                "no unique reviewed execution class: {}",
                id.metadata().name
            ),
            Self::ExecutionUnavailable { caller, callee } => {
                write!(formatter, "{callee:?} call unavailable inside {caller:?}")
            }
            Self::Arithmetic(trap) => write!(formatter, "68000 arithmetic exception: {trap:?}"),
            Self::Frame(error) => write!(formatter, "emulated call frame: {error:?}"),
            Self::Platform(message) => formatter.write_str(message),
        }
    }
}
impl std::error::Error for Error {}
impl From<ArithmeticTrap> for Error {
    fn from(value: ArithmeticTrap) -> Self {
        Self::Arithmetic(value)
    }
}
impl From<call_frame::FrameError> for Error {
    fn from(value: call_frame::FrameError) -> Self {
        Self::Frame(value)
    }
}
impl Error {
    /// Ordinary errors unwind suspended call bookkeeping; the exit layer marks
    /// non-returning control transfers so their live frame is not unwound.
    pub fn preserves_call_frame(&self) -> bool {
        false
    }
}
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Side-effecting, width-preserving device bus. `Bus` remains ROM/RAM-only so
/// typed storage reads cannot accidentally hide a controller/VDP read side effect.
/// The implementation supplies the actual programmed platform, not fake registers.
pub trait Platform: tje_mem::Platform {
    fn read_device_u8(&mut self, address: u32) -> u8;
    fn read_device_u16(&mut self, address: u32) -> u16;
    fn write_device_u8(&mut self, address: u32, value: u8);
    fn write_device_u16(&mut self, address: u32, value: u16);

    /// Suspend while the platform makes real clock/interrupt progress. Native
    /// platforms implement the same condition with their frame scheduler.
    fn wait_until(&mut self, _request: execution::WaitRequest) -> Result<()> {
        Err(Error::Platform(
            "this platform has no time-progressing wait contract".into(),
        ))
    }

    fn wait_vblank_frame(&mut self, address: u32, previous: u16) -> Result<()> {
        self.wait_until(execution::WaitRequest::vblank_frame(address, previous))
    }

    fn wait_for_interrupt(&mut self) -> Result<()> {
        self.wait_until(execution::WaitRequest::interrupt())
    }

    fn read_vdp_status(&mut self) -> u16 {
        self.read_device_u16(0xc00004)
    }
    fn read_vdp_data(&mut self) -> u16 {
        self.read_device_u16(0xc00000)
    }
    fn write_ym2612(&mut self, port: u8, value: u8) {
        self.write_device_u8(0xa04000 + u32::from(port & 3), value);
    }
    fn read_ym2612_status(&mut self) -> u8 {
        self.read_device_u8(0xa04000)
    }
    fn write_psg(&mut self, value: u8) {
        self.write_device_u8(0xc00011, value);
    }
    fn read_z80_ram(&mut self, offset: u16) -> u8 {
        self.read_device_u8(0xa00000 + u32::from(offset & 0x1fff))
    }
    fn write_z80_ram(&mut self, offset: u16, value: u8) {
        self.write_device_u8(0xa00000 + u32::from(offset & 0x1fff), value);
    }
    fn write_z80_bus_request(&mut self, value: u16) {
        self.write_device_u16(0xa11100, value);
    }
    fn read_z80_bus_request(&mut self) -> u16 {
        self.read_device_u16(0xa11100)
    }
    fn write_z80_reset(&mut self, value: u16) {
        self.write_device_u16(0xa11200, value);
    }
    fn read_controller_data(&mut self, second: bool) -> u8 {
        self.read_device_u8(if second { 0xa10005 } else { 0xa10003 })
    }
    fn write_controller_data(&mut self, second: bool, value: u8) {
        self.write_device_u8(if second { 0xa10005 } else { 0xa10003 }, value);
    }
    fn write_controller_control(&mut self, second: bool, value: u8) {
        self.write_device_u8(if second { 0xa1000b } else { 0xa10009 }, value);
    }
}

/// The bridge owns residual register bits and original-call mechanics. Arguments
/// passed to a translation are raw four-byte MCC68K slots, narrowed by the callee.
/// Register-runtime helpers read D0/D1 instead of manufacturing stack arguments.
pub trait Context: Bus + Platform {
    fn revision(&self) -> Revision;
    fn data_register(&self, index: usize) -> u32;
    fn set_data_register(&mut self, index: usize, value: u32);
    fn address_register(&self, index: usize) -> u32;
    fn set_address_register(&mut self, index: usize, value: u32);
    fn status_register(&self) -> u16;
    fn set_status_register(&mut self, value: u16);
    fn stack_pointer(&self) -> Result<u32> {
        Err(call_frame::FrameError::Unavailable.into())
    }
    fn set_stack_pointer(&mut self, _value: u32) -> Result<()> {
        Err(call_frame::FrameError::Unavailable.into())
    }
    fn frame_state(&self) -> Option<call_frame::FrameState> {
        None
    }
    fn set_frame_state(&mut self, _state: call_frame::FrameState) -> Result<()> {
        Err(call_frame::FrameError::Unavailable.into())
    }
    /// Run an original callee at an already materialized entry SP, consuming its
    /// actual RTS without synthesizing another argument area.
    fn call_original_entry(
        &mut self,
        function: FunctionId,
        _class: execution::ExecutionClass,
    ) -> Result<u32> {
        Err(Error::Untranslated(function))
    }
    fn dispatcher(&self) -> Dispatcher;
    /// Default contexts support Atomic only. Clock-capable platforms override
    /// both methods and enforce their entry execution envelope.
    fn execution_class(&self) -> execution::ExecutionClass {
        execution::ExecutionClass::Atomic
    }
    fn set_execution_class(&mut self, class: execution::ExecutionClass) -> Result<()> {
        if class == execution::ExecutionClass::Atomic {
            Ok(())
        } else {
            Err(Error::ExecutionUnavailable {
                caller: self.execution_class(),
                callee: class,
            })
        }
    }
    fn call_original(&mut self, function: FunctionId, arguments: &[u32]) -> Result<u32>;

    /// Explicit reviewed residual call. Native platforms never fall back to ROM;
    /// hybrid platforms reject execution classes unavailable in the caller.
    fn call_original_with_execution(
        &mut self,
        function: FunctionId,
        arguments: &[u32],
        class: execution::ExecutionClass,
    ) -> Result<u32> {
        if class == execution::ExecutionClass::Atomic {
            self.call_original(function, arguments)
        } else {
            Err(Error::Untranslated(function))
        }
    }
}

pub type TranslationFn = fn(&mut dyn Context, &[u32]) -> Result<Option<u32>>;
#[derive(Clone, Copy)]
pub struct Translation {
    pub id: FunctionId,
    pub function: TranslationFn,
}

/// A lane owns its module's entries and atomic classification. The shared list is
/// assembled once; adding a translation never requires changing generated data.
pub struct Subsystem {
    pub name: &'static str,
    pub translations: &'static [Translation],
    pub atomic_capable: &'static [FunctionId],
    pub execution_contracts: &'static [(FunctionId, execution::ExecutionClass)],
}

pub fn subsystem(name: &str) -> Option<&'static Subsystem> {
    SUBSYSTEMS.iter().find(|subsystem| subsystem.name == name)
}

pub fn is_atomic_capable(id: FunctionId) -> bool {
    execution_class(id) == Some(execution::ExecutionClass::Atomic)
}

#[derive(Clone, Copy)]
enum ClassDeclaration {
    Missing,
    Declared(execution::ExecutionClass),
    Conflict,
}

static EXECUTION_CLASSES: std::sync::LazyLock<[ClassDeclaration; 648]> =
    std::sync::LazyLock::new(|| {
        let mut classes = [ClassDeclaration::Missing; 648];
        for owner in SUBSYSTEMS {
            for (id, class, valid) in
                owner
                    .atomic_capable
                    .iter()
                    .map(|id| (*id, execution::ExecutionClass::Atomic, true))
                    .chain(owner.execution_contracts.iter().map(|(id, class)| {
                        (*id, *class, *class != execution::ExecutionClass::Atomic)
                    }))
            {
                let slot = &mut classes[usize::from(id.0)];
                *slot = if valid
                    && id.metadata().subsystem == owner.name
                    && matches!(*slot, ClassDeclaration::Missing)
                {
                    ClassDeclaration::Declared(class)
                } else {
                    ClassDeclaration::Conflict
                };
            }
        }
        classes
    });

/// Owner declarations are indexed once without allocation. Conflicting,
/// duplicate, cross-owner and misfiled Atomic declarations fail closed.
pub fn execution_class(id: FunctionId) -> Option<execution::ExecutionClass> {
    match EXECUTION_CLASSES[usize::from(id.0)] {
        ClassDeclaration::Declared(class) => Some(class),
        _ => None,
    }
}

pub fn translations() -> impl Iterator<Item = &'static Translation> + Clone {
    SUBSYSTEMS
        .iter()
        .flat_map(|subsystem| subsystem.translations.iter())
}

#[derive(Clone, Copy)]
pub struct Dispatcher {
    translations: &'static [Translation],
    subsystems: &'static [Subsystem],
    original_fallback: bool,
}
impl Dispatcher {
    pub const fn hybrid() -> Self {
        Self {
            translations: &[],
            subsystems: SUBSYSTEMS,
            original_fallback: true,
        }
    }
    /// A deliberately bounded hybrid registry is useful when verifying nested
    /// original calls. Missing callees are explicit calls through Context.
    pub const fn hybrid_with(translations: &'static [Translation]) -> Self {
        Self {
            translations,
            subsystems: &[],
            original_fallback: true,
        }
    }

    /// Explicit proof overrides retain all proven Rust callees. Supplied entries
    /// remain outside the production registry until their proof lands.
    pub const fn hybrid_with_overrides(translations: &'static [Translation]) -> Self {
        Self {
            translations,
            subsystems: SUBSYSTEMS,
            original_fallback: true,
        }
    }
    /// Native startup validates the full dependency closure of each requested
    /// entry. Failure lists every missing target; it never enables ROM fallback.
    pub fn native(roots: &[FunctionId]) -> std::result::Result<Self, Vec<FunctionId>> {
        Self::native_with(roots, &[])
    }

    /// Native proof overrides obey the same complete-closure check and never
    /// introduce ROM fallback; all non-overridden proven Rust callees remain.
    pub fn native_with(
        roots: &[FunctionId],
        translations: &'static [Translation],
    ) -> std::result::Result<Self, Vec<FunctionId>> {
        let dispatcher = Self {
            translations,
            subsystems: SUBSYSTEMS,
            original_fallback: false,
        };
        let mut visited = std::collections::BTreeSet::new();
        let mut pending = roots.to_vec();
        let mut missing = Vec::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            if dispatcher.translation(id).is_none() || execution_class(id).is_none() {
                missing.push(id);
            }
            pending.extend_from_slice(id.metadata().callees);
        }
        missing.sort_unstable();
        if missing.is_empty() {
            Ok(dispatcher)
        } else {
            Err(missing)
        }
    }
    pub fn translation(self, id: FunctionId) -> Option<Translation> {
        self.translations()
            .find(|translation| translation.id == id)
            .copied()
    }
    pub fn translations(self) -> impl Iterator<Item = &'static Translation> + Clone {
        self.translations.iter().chain(
            self.subsystems
                .iter()
                .flat_map(|subsystem| subsystem.translations.iter()),
        )
    }
    pub fn call(self, context: &mut dyn Context, id: FunctionId, arguments: &[u32]) -> Result<u32> {
        let translation = self.translation(id);
        if translation.is_none() && !self.original_fallback {
            return Err(Error::Untranslated(id));
        }
        let class = execution_class(id).ok_or(Error::Unclassified(id))?;
        let parent = context.execution_class();
        if !parent.permits(class) {
            return Err(Error::ExecutionUnavailable {
                caller: parent,
                callee: class,
            });
        }
        if let Some(translation) = translation {
            let scoped = parent != class;
            if scoped {
                context.set_execution_class(class)?;
            }
            let result = (translation.function)(context, arguments).map(|value| {
                if let Some(value) = value {
                    context.set_data_register(0, value);
                }
                context.data_register(0)
            });
            let restore = if scoped {
                context.set_execution_class(parent)
            } else {
                Ok(())
            };
            match (result, restore) {
                (Err(error), _) | (_, Err(error)) => Err(error),
                (Ok(value), Ok(())) => Ok(value),
            }
        } else {
            context.call_original_with_execution(id, arguments, class)
        }
    }

    pub(crate) fn allows_original_fallback(self) -> bool {
        self.original_fallback
    }

    /// Exact-stack counterpart for slot/local/return-address-sensitive calls.
    pub fn call_at(
        self,
        context: &mut dyn Context,
        id: FunctionId,
        arguments: &[u32],
        site: call_frame::CallSite,
    ) -> Result<u32> {
        call_frame::call_at(context, self, id, arguments, site)
    }
}

/// A translated caller never chooses its callee's implementation or address.
pub fn call(context: &mut dyn Context, id: FunctionId, arguments: &[u32]) -> Result<u32> {
    context.dispatcher().call(context, id, arguments)
}

pub fn call_at(
    context: &mut dyn Context,
    id: FunctionId,
    arguments: &[u32],
    site: call_frame::CallSite,
) -> Result<u32> {
    context.dispatcher().call_at(context, id, arguments, site)
}

pub static SUBSYSTEMS: &[Subsystem] = &[
    Subsystem {
        name: "audio",
        translations: audio::TRANSLATIONS,
        atomic_capable: audio::ATOMIC_CAPABLE,
        execution_contracts: audio::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "camera",
        translations: camera::TRANSLATIONS,
        atomic_capable: camera::ATOMIC_CAPABLE,
        execution_contracts: camera::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "collision",
        translations: collision::TRANSLATIONS,
        atomic_capable: collision::ATOMIC_CAPABLE,
        execution_contracts: collision::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "core",
        translations: core::TRANSLATIONS,
        atomic_capable: core::ATOMIC_CAPABLE,
        execution_contracts: core::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "cutscenes",
        translations: cutscenes::TRANSLATIONS,
        atomic_capable: cutscenes::ATOMIC_CAPABLE,
        execution_contracts: cutscenes::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "earthlings",
        translations: earthlings::TRANSLATIONS,
        atomic_capable: earthlings::ATOMIC_CAPABLE,
        execution_contracts: earthlings::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "elevator",
        translations: elevator::TRANSLATIONS,
        atomic_capable: elevator::ATOMIC_CAPABLE,
        execution_contracts: elevator::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "jamout",
        translations: jamout::TRANSLATIONS,
        atomic_capable: jamout::ATOMIC_CAPABLE,
        execution_contracts: jamout::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "map",
        translations: map::TRANSLATIONS,
        atomic_capable: map::ATOMIC_CAPABLE,
        execution_contracts: map::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "platform",
        translations: platform::TRANSLATIONS,
        atomic_capable: platform::ATOMIC_CAPABLE,
        execution_contracts: platform::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "player",
        translations: player::TRANSLATIONS,
        atomic_capable: player::ATOMIC_CAPABLE,
        execution_contracts: player::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "presents",
        translations: presents::TRANSLATIONS,
        atomic_capable: presents::ATOMIC_CAPABLE,
        execution_contracts: presents::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "runtime",
        translations: runtime::TRANSLATIONS,
        atomic_capable: runtime::ATOMIC_CAPABLE,
        execution_contracts: runtime::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "ship",
        translations: ship::TRANSLATIONS,
        atomic_capable: ship::ATOMIC_CAPABLE,
        execution_contracts: ship::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "speech",
        translations: speech::TRANSLATIONS,
        atomic_capable: speech::ATOMIC_CAPABLE,
        execution_contracts: speech::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "sprites",
        translations: sprites::TRANSLATIONS,
        atomic_capable: sprites::ATOMIC_CAPABLE,
        execution_contracts: sprites::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "ui",
        translations: ui::TRANSLATIONS,
        atomic_capable: ui::ATOMIC_CAPABLE,
        execution_contracts: ui::EXECUTION_CONTRACTS,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_metadata_and_native_closure() {
        let functions: Vec<_> = functions().collect();
        assert_eq!(functions.len(), 648);
        assert_eq!(
            functions
                .iter()
                .filter(|function| function.kind == "function")
                .count(),
            645
        );
        for function in functions {
            assert_eq!(function.id.metadata().inventory_id, function.inventory_id);
            for revision in [Revision::Rev02, Revision::Rev00] {
                assert_eq!(
                    function_at(revision, function.address(revision)),
                    Some(function.id)
                );
            }
            assert!(!function.abi.signature.is_empty());
            assert!(!function.abi.details_json.is_empty());
            assert!(function.source.starts_with("src/"));
        }
        assert!(Dispatcher::native(&[ids::GET_RANDOM_NUMBER, ids::PRE_GENERATE_RANDOMS]).is_ok());
        let startup = ids::FURTHER_INITIALISATION;
        let missing = Dispatcher::native(&[startup]).err().unwrap();
        assert!(missing.contains(&startup));
        assert!(missing.len() > 100);
        let mut seen = std::collections::BTreeSet::new();
        for translation in translations() {
            assert!(seen.insert(translation.id));
        }
    }

    #[test]
    fn subsystem_ownership_and_execution_review_are_complete_and_disjoint() {
        let mut names = std::collections::BTreeSet::new();
        let mut classified = std::collections::BTreeSet::new();
        let mut translated = std::collections::BTreeSet::new();
        for owner in SUBSYSTEMS {
            assert!(names.insert(owner.name));
            for id in owner.atomic_capable {
                assert_eq!(id.metadata().subsystem, owner.name);
                assert!(classified.insert(*id));
                assert_eq!(
                    execution_class(*id),
                    Some(execution::ExecutionClass::Atomic)
                );
            }
            for (id, class) in owner.execution_contracts {
                assert_eq!(id.metadata().subsystem, owner.name);
                assert_ne!(*class, execution::ExecutionClass::Atomic);
                assert!(classified.insert(*id));
                assert_eq!(execution_class(*id), Some(*class));
            }
            for entry in owner.translations {
                assert_eq!(entry.id.metadata().subsystem, owner.name);
                assert!(translated.insert(entry.id));
                assert!(
                    execution_class(entry.id).is_some(),
                    "unclassified translation: {}",
                    entry.id.metadata().name
                );
            }
        }
        for function in functions() {
            assert!(subsystem(function.subsystem).is_some());
        }
        assert!(!is_atomic_capable(ids::SETUP_YM2612));
        let waiting = ids::WAIT_FOR_NEXT_V_BLANK;
        assert!(!is_atomic_capable(waiting));
    }
}
