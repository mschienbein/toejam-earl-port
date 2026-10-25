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
pub mod control;
pub mod core;
pub mod cutscenes;
pub mod earthlings;
pub mod elevator;
pub mod execution;
pub mod exits;
mod framed_entry;
pub mod jamout;
pub mod map;
mod metadata;
pub mod open_bus;
pub mod platform;
pub mod player;
pub mod presents;
pub mod runtime;
pub mod ship;
pub mod speech;
pub mod sprites;
pub mod stack_read;
pub mod timing;
pub mod ui;
pub use framed_entry::{FramedTranslation, FramedTranslationFn, SelectedEntry, TranslatedEntry};
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
    CallReturnType {
        function: FunctionId,
        requested_value: bool,
    },
    ExecutionUnavailable {
        caller: execution::ExecutionClass,
        callee: execution::ExecutionClass,
    },
    Arithmetic(ArithmeticTrap),
    Frame(call_frame::FrameError),
    Clock(execution::CallClockError),
    /// A nonlocal transfer still abandons its materialized frame when a clock
    /// capability error prevents approving that transfer.
    ClockAfterTransfer {
        transfer: exits::ControlTransfer,
        error: execution::CallClockError,
    },
    /// Arbitrary fatal cleanup errors retain a nonlocal transfer's abandoned
    /// frame and original cause. Boxing is confined to this recursive error path.
    CleanupAfterTransfer {
        transfer: exits::ControlTransfer,
        error: Box<Error>,
    },
    /// An admitted timed invocation failed with its actual CPU/device frame
    /// still live. This is a failure, never authority for a nonlocal transfer.
    TimingExecutionFailure {
        entry: FunctionId,
        cause: Box<Error>,
    },
    /// A compiled callee in the unclocked extras bridge failed with its actual
    /// frame live. This grants no timing, transfer or production-body authority.
    #[cfg(feature = "enhanced-mode")]
    EnhancedExecutionFailure {
        entry: FunctionId,
        cause: Box<Error>,
    },
    /// A deliberate non-RTS exit. Callers propagate it without running cleanup.
    ControlTransfer(exits::ControlTransfer),
    OpenBus(open_bus::OpenBusError),
    UnsupportedStackRead(stack_read::Unsupported),
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
            Self::CallReturnType {
                function,
                requested_value,
            } => {
                if *requested_value {
                    write!(
                        formatter,
                        "{} is ABI-void; use call_void/call_at_void, then an explicit authenticated register read if required",
                        function.metadata().name
                    )
                } else {
                    write!(
                        formatter,
                        "{} returns a value; use call/call_at",
                        function.metadata().name
                    )
                }
            }
            Self::ExecutionUnavailable { caller, callee } => {
                write!(formatter, "{callee:?} call unavailable inside {caller:?}")
            }
            Self::Arithmetic(trap) => write!(formatter, "68000 arithmetic exception: {trap:?}"),
            Self::Frame(error) => write!(formatter, "emulated call frame: {error:?}"),
            Self::Clock(error) => error.fmt(formatter),
            Self::ClockAfterTransfer { transfer, error } => write!(
                formatter,
                "{error} during control transfer from {}: {:?}",
                transfer.entry.metadata().name,
                transfer.exit,
            ),
            Self::CleanupAfterTransfer { transfer, error } => write!(
                formatter,
                "{error} during control transfer from {}: {:?}",
                transfer.entry.metadata().name,
                transfer.exit,
            ),
            Self::TimingExecutionFailure { entry, cause } => write!(
                formatter,
                "timed execution of {} failed: {cause}",
                entry.metadata().name,
            ),
            #[cfg(feature = "enhanced-mode")]
            Self::EnhancedExecutionFailure { entry, cause } => write!(
                formatter,
                "enhanced compiled call to {} failed: {cause}",
                entry.metadata().name,
            ),
            Self::ControlTransfer(transfer) => write!(
                formatter,
                "control transfer from {}: {:?}",
                transfer.entry.metadata().name,
                transfer.exit,
            ),
            Self::OpenBus(error) => fmt::Display::fmt(error, formatter),
            Self::UnsupportedStackRead(error) => fmt::Display::fmt(error, formatter),
            Self::Platform(message) => formatter.write_str(message),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Clock(error) | Self::ClockAfterTransfer { error, .. } => Some(error),
            Self::CleanupAfterTransfer { error, .. } => Some(error.as_ref()),
            Self::TimingExecutionFailure { cause, .. } => Some(cause.as_ref()),
            #[cfg(feature = "enhanced-mode")]
            Self::EnhancedExecutionFailure { cause, .. } => Some(cause.as_ref()),
            _ => None,
        }
    }
}
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
impl From<open_bus::OpenBusError> for Error {
    fn from(value: open_bus::OpenBusError) -> Self {
        Self::OpenBus(value)
    }
}
impl From<execution::CallClockError> for Error {
    fn from(value: execution::CallClockError) -> Self {
        Self::Clock(value)
    }
}
impl From<stack_read::Unsupported> for Error {
    fn from(value: stack_read::Unsupported) -> Self {
        Self::UnsupportedStackRead(value)
    }
}
impl Error {
    pub fn unsupported_stack_read(&self) -> Option<stack_read::Unsupported> {
        match self {
            Self::UnsupportedStackRead(error) => Some(*error),
            Self::CleanupAfterTransfer { error, .. } => error.unsupported_stack_read(),
            Self::TimingExecutionFailure { cause, .. } => cause.unsupported_stack_read(),
            _ => None,
        }
    }
    /// Already-failed execution keeps its first cause and actual frame.
    pub fn is_failed_execution(&self) -> bool {
        match self {
            Self::ClockAfterTransfer { .. }
            | Self::CleanupAfterTransfer { .. }
            | Self::TimingExecutionFailure { .. }
            | Self::UnsupportedStackRead(_) => true,
            #[cfg(feature = "enhanced-mode")]
            Self::EnhancedExecutionFailure { .. } => true,
            _ => false,
        }
    }

    /// Ordinary errors may unwind diagnostic bookkeeping. Nonlocal transfers
    /// and admitted timed failures must retain their actual live call frame.
    pub fn preserves_call_frame(&self) -> bool {
        matches!(self, Self::ControlTransfer(_)) || self.is_failed_execution()
    }

    /// Wrap once, only after timing admission/effects or a receipt protocol
    /// failure. No PC, stack, SR, exit or transfer is invented here.
    pub fn timing_execution_failure(entry: FunctionId, cause: Self) -> Self {
        if cause.is_failed_execution() {
            return cause;
        }
        Self::TimingExecutionFailure {
            entry,
            cause: Box::new(cause),
        }
    }

    /// Reject cleanup after an already-validated transfer without unwinding its
    /// abandoned frame. This constructs a failure, never an approved exit.
    pub fn cleanup_after_transfer(transfer: exits::ControlTransfer, error: Error) -> Self {
        if error.is_failed_execution() {
            return error;
        }
        match error {
            Self::Clock(error) => Self::ClockAfterTransfer { transfer, error },
            error => Self::CleanupAfterTransfer {
                transfer,
                error: Box::new(error),
            },
        }
    }

    /// Apply a post-call contract check after validating any fresh transfer.
    /// Already-failed transfers retain their first cause and frame ownership;
    /// a newly failing check converts a fresh transfer into a rejection.
    pub fn after_contract_check<T>(context: &dyn Context, result: Result<T>) -> Result<T> {
        // Protect the primary cause before running a secondary contract check.
        let result = result.map_err(|error| context.protect_timing_failure(error));
        let check = context
            .check_mmio_contract()
            .map_err(|error| context.protect_timing_failure(error));
        match (result, check) {
            (Err(error), _) if error.is_failed_execution() => Err(error),
            (_, Err(error)) if error.is_failed_execution() => Err(error),
            (Err(Self::ControlTransfer(transfer)), Err(error)) => {
                Err(Self::cleanup_after_transfer(transfer, error))
            }
            (result, Ok(())) => result,
            (_, Err(error)) => Err(error),
        }
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

    /// Read a partially driven device operand with the original instruction's
    /// explicitly declared ROM prefetch source. Native platforms resolve that
    /// source from the user's ROM; unsupported platforms fail closed.
    fn read_device_u8_with_open_bus(
        &mut self,
        _address: u32,
        _source: open_bus::OpenBusSource,
    ) -> Result<u8> {
        Err(open_bus::OpenBusError::UnsupportedPlatform.into())
    }

    fn read_device_u16_with_open_bus(
        &mut self,
        _address: u32,
        _source: open_bus::OpenBusSource,
    ) -> Result<u16> {
        Err(open_bus::OpenBusError::UnsupportedPlatform.into())
    }

    /// Deferred device violations survive nested scopes and handled errors.
    /// Eagerly validating platforms have no deferred violation to report.
    fn check_mmio_contract(&self) -> Result<()> {
        Ok(())
    }

    /// Perform one byte bus read for its hardware effects, without exposing an
    /// operand to software. Implementations must retain the Discarded purpose.
    fn discard_u8(&mut self, _address: u32) -> Result<()> {
        Err(Error::Platform(
            "discarded bus reads are unavailable".into(),
        ))
    }

    /// Word counterpart; this is one physical 16-bit read, not two byte reads.
    fn discard_u16(&mut self, _address: u32) -> Result<()> {
        Err(Error::Platform(
            "discarded bus reads are unavailable".into(),
        ))
    }

    /// Suspend while the platform makes real clock/interrupt progress. Native
    /// platforms implement the same condition with their frame scheduler.
    fn wait_until(&mut self, _request: execution::WaitRequest) -> Result<()> {
        Err(Error::Platform(
            "this platform has no time-progressing wait contract".into(),
        ))
    }

    /// Mark an iteration boundary without asserting a polling predicate.
    /// Clocked platforms make real progress; recorded-environment checks do
    /// not consume an access, fabricate readiness or add a timing claim.
    fn poll_yield(&mut self) -> Result<()> {
        Err(Error::Platform("scheduler yield is unavailable".into()))
    }

    fn wait_vblank_frame(&mut self, address: u32, previous: u16) -> Result<()> {
        self.wait_until(execution::WaitRequest::vblank_frame(address, previous))
    }

    fn wait_for_interrupt(&mut self) -> Result<()> {
        self.wait_until(execution::WaitRequest::interrupt())
    }

    fn read_vdp_status(&mut self, source: open_bus::OpenBusSource) -> Result<u16> {
        self.read_device_u16_with_open_bus(0xc00004, source)
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
    fn read_z80_bus_request(&mut self, source: open_bus::OpenBusSource) -> Result<u16> {
        self.read_device_u16_with_open_bus(0xa11100, source)
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
    /// Optional, sealed framework metadata. Absence creates no invocation witness.
    /// Observing a boundary never supplies an operand, advances time or executes a call.
    fn observe_call_boundary(&mut self, _event: call_frame::CallBoundaryEvent) {}
    /// Deliberate source-cited original bug, never a generic stack-read waiver.
    /// Implementations validate the exact current own-frame occurrence before
    /// exposing unchanged entry residue. Unsupported platforms never read RAM.
    fn read_uninitialized_local(
        &mut self,
        read: stack_read::UninitializedLocalRead,
    ) -> Result<u32> {
        self.check_mmio_contract()?;
        Err(stack_read::Unsupported::unavailable(read).into())
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
    /// Exact prepared original call with no scalar-result observation.
    /// Never implement by obtaining a D0 result and discarding it afterward.
    fn call_original_entry_void(
        &mut self,
        function: FunctionId,
        _class: execution::ExecutionClass,
    ) -> Result<()> {
        Err(Error::Untranslated(function))
    }
    /// Required by RTE and reset; Atomic-only contexts fail closed rather than
    /// inventing an inactive stack bank.
    fn stack_banks(&self) -> Result<exits::StackBanks> {
        Err(Error::Platform("banked stack access is unavailable".into()))
    }
    fn set_stack_banks(&mut self, _banks: exits::StackBanks) -> Result<()> {
        Err(Error::Platform("banked stack access is unavailable".into()))
    }
    /// Execute a reviewed source-role program from an actual prepared entry.
    /// A missing clocked capability rejects before inspecting any source effect.
    fn execute_timing_program(
        &mut self,
        _entry: FunctionId,
        _source: &mut dyn timing::TimingSource,
    ) -> Result<timing::TimedExit> {
        Err(Error::Platform(
            "clocked source timing is unavailable".into(),
        ))
    }
    /// Only a platform-private completion receipt may suppress ordinary return
    /// finalization. Expectations supplied by a body are never such authority.
    fn consume_timing_completion(
        &mut self,
        _request: timing::CompletionRequest,
    ) -> Result<timing::Completion> {
        Ok(timing::Completion::Absent)
    }
    /// Classify an error using actual platform-owned timing admission only.
    /// Default/frozen contexts preserve ordinary cleanup; no public source
    /// expectation, Program or TimedExit may activate this protection.
    fn protect_timing_failure(&self, error: Error) -> Error {
        error
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
    /// The outer clock envelope survives temporary Atomic callee scoping.
    /// A clock-capable implementation overrides this independently of its
    /// currently selected execution class.
    fn clocked_execution(&self) -> bool {
        false
    }
    fn enter_call_ceiling(&mut self, function: FunctionId) -> Result<execution::CallCeilingScope> {
        if self.clocked_execution()
            || execution_class(function).ok_or(Error::Unclassified(function))?
                != execution::ExecutionClass::Atomic
        {
            Err(execution::CallClockError::Unavailable.into())
        } else {
            Ok(execution::CallCeilingScope::not_required())
        }
    }
    fn leave_call_ceiling(&mut self, scope: execution::CallCeilingScope) -> Result<()> {
        if scope.is_required() || self.clocked_execution() {
            Err(execution::CallClockError::Unavailable.into())
        } else {
            Ok(())
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
    /// Explicit residual call that preserves the real register/device effects
    /// but never observes or supplies a scalar D0 result.
    fn call_original_void_with_execution(
        &mut self,
        function: FunctionId,
        _arguments: &[u32],
        _class: execution::ExecutionClass,
    ) -> Result<()> {
        Err(Error::Untranslated(function))
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
    pub framed_translations: &'static [FramedTranslation],
    /// Check-only semantic proofs, never part of canonical production dispatch.
    pub replayed_translations: &'static [Translation],
    pub replayed_framed_translations: &'static [FramedTranslation],
    /// Check-only candidate availability, never a live-dispatch or status grant.
    pub pending_translations: &'static [Translation],
    pub pending_framed_translations: &'static [FramedTranslation],
    pub atomic_capable: &'static [FunctionId],
    pub execution_contracts: &'static [execution::ExecutionContract],
}

pub fn subsystem(name: &str) -> Option<&'static Subsystem> {
    SUBSYSTEMS.iter().find(|subsystem| subsystem.name == name)
}

pub fn is_atomic_capable(id: FunctionId) -> bool {
    execution_class(id) == Some(execution::ExecutionClass::Atomic)
}

#[derive(Clone, Copy)]
enum ContractDeclaration {
    Missing,
    Declared(execution::ExecutionContract),
    Conflict,
}

fn index_execution_contracts(owners: &[Subsystem]) -> [ContractDeclaration; 648] {
    use execution::{ExecutionClass, ExecutionContract, ProofEnvelope};
    let mut contracts = [ContractDeclaration::Missing; 648];
    for owner in owners {
        let atomic = owner.atomic_capable.iter().map(|&function| {
            (
                ExecutionContract {
                    function,
                    class: ExecutionClass::Atomic,
                    proof_envelope: ProofEnvelope::ClassDefined,
                    operational_ceiling: None,
                    evidence: None,
                },
                true,
            )
        });
        let extended = owner.execution_contracts.iter().copied().map(|contract| {
            let placement = contract.class != ExecutionClass::Atomic
                || contract.operational_ceiling.is_some()
                || contract.proof_envelope != ProofEnvelope::ClassDefined;
            (contract, placement)
        });
        for (contract, placement) in atomic.chain(extended) {
            let slot = &mut contracts[usize::from(contract.function.0)];
            *slot = if placement
                && contract.is_valid()
                && contract.function.metadata().subsystem == owner.name
                && matches!(*slot, ContractDeclaration::Missing)
            {
                ContractDeclaration::Declared(contract)
            } else {
                ContractDeclaration::Conflict
            };
        }
    }
    contracts
}

static EXECUTION_CONTRACT_INDEX: std::sync::LazyLock<[ContractDeclaration; 648]> =
    std::sync::LazyLock::new(|| index_execution_contracts(SUBSYSTEMS));

/// One owner declaration per original id. Missing, duplicate, cross-owner,
/// malformed and misplaced Atomic declarations fail closed.
pub fn execution_contract(id: FunctionId) -> Option<&'static execution::ExecutionContract> {
    match EXECUTION_CONTRACT_INDEX.get(usize::from(id.0))? {
        ContractDeclaration::Declared(contract) => Some(contract),
        _ => None,
    }
}

pub fn execution_class(id: FunctionId) -> Option<execution::ExecutionClass> {
    execution_contract(id).map(|contract| contract.class)
}

pub fn translations() -> impl Iterator<Item = &'static Translation> + Clone {
    SUBSYSTEMS
        .iter()
        .flat_map(|subsystem| subsystem.translations.iter())
}

pub fn framed_translations() -> impl Iterator<Item = &'static FramedTranslation> + Clone {
    SUBSYSTEMS
        .iter()
        .flat_map(|subsystem| subsystem.framed_translations.iter())
}

/// Check-only bodies proven under the recorded environment, not live replacements.
pub fn replayed_translations() -> impl Iterator<Item = &'static Translation> + Clone {
    SUBSYSTEMS
        .iter()
        .flat_map(|subsystem| subsystem.replayed_translations.iter())
}

/// Framed check-only bodies; kept separate from canonical production entries.
pub fn replayed_framed_translations() -> impl Iterator<Item = &'static FramedTranslation> + Clone {
    SUBSYSTEMS
        .iter()
        .flat_map(|subsystem| subsystem.replayed_framed_translations.iter())
}

/// Complete canonical pending value availability; callers select proof roots separately.
pub fn pending_translations() -> impl Iterator<Item = &'static Translation> + Clone {
    SUBSYSTEMS
        .iter()
        .flat_map(|subsystem| subsystem.pending_translations.iter())
}

/// Complete canonical pending frame availability, without live replacement eligibility.
pub fn pending_framed_translations() -> impl Iterator<Item = &'static FramedTranslation> + Clone {
    SUBSYSTEMS
        .iter()
        .flat_map(|subsystem| subsystem.pending_framed_translations.iter())
}

#[derive(Clone, Copy)]
pub(crate) enum CallReturn {
    Value,
    Discard,
}
impl CallReturn {
    fn check(self, function: FunctionId) -> Result<()> {
        let requested_value = matches!(self, Self::Value);
        if requested_value != function.metadata().abi.returns_value {
            Err(Error::CallReturnType {
                function,
                requested_value,
            })
        } else {
            Ok(())
        }
    }
    fn observe(self, context: &dyn Context) -> Option<u32> {
        match self {
            Self::Value => Some(context.data_register(0)),
            Self::Discard => None,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Dispatcher {
    translations: &'static [Translation],
    framed_translations: &'static [FramedTranslation],
    subsystems: &'static [Subsystem],
    original_fallback: bool,
}
impl Dispatcher {
    pub const fn hybrid() -> Self {
        Self {
            translations: &[],
            framed_translations: &[],
            subsystems: SUBSYSTEMS,
            original_fallback: true,
        }
    }
    /// A deliberately bounded hybrid registry is useful when verifying nested
    /// original calls. Missing callees are explicit calls through Context.
    pub const fn hybrid_with(translations: &'static [Translation]) -> Self {
        Self {
            translations,
            framed_translations: &[],
            subsystems: &[],
            original_fallback: true,
        }
    }

    /// Explicit proof overrides retain all proven Rust callees. Supplied entries
    /// remain outside the production registry until their proof lands.
    pub const fn hybrid_with_overrides(translations: &'static [Translation]) -> Self {
        Self {
            translations,
            framed_translations: &[],
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
        Self::native_with_frames(roots, translations, &[])
    }

    pub fn native_with_frames(
        roots: &[FunctionId],
        translations: &'static [Translation],
        framed_translations: &'static [FramedTranslation],
    ) -> std::result::Result<Self, Vec<FunctionId>> {
        let dispatcher = Self {
            translations,
            framed_translations,
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
            if !matches!(dispatcher.resolve_entry(id), Ok(Some(_))) || execution_class(id).is_none()
            {
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
    /// Effective legacy value entry only. Use `resolve_entry` for capability
    /// checks or dispatch, since a framed overlay replaces this form completely.
    pub fn translation(self, id: FunctionId) -> Option<&'static Translation> {
        match self.resolve_entry(id).ok().flatten()?.entry() {
            TranslatedEntry::Values(translation) => Some(translation),
            _ => None,
        }
    }
    pub fn translations(self) -> impl Iterator<Item = &'static Translation> + Clone {
        self.translations.iter().chain(
            self.subsystems
                .iter()
                .flat_map(|subsystem| subsystem.translations.iter()),
        )
    }
    pub fn call(self, context: &mut dyn Context, id: FunctionId, arguments: &[u32]) -> Result<u32> {
        self.call_with_return(context, id, arguments, CallReturn::Value)
            .map(|value| value.expect("value-returning call requested D0"))
    }
    /// ABI-void call: discard intent reaches the callee before any D0 read.
    pub fn call_void(
        self,
        context: &mut dyn Context,
        id: FunctionId,
        arguments: &[u32],
    ) -> Result<()> {
        self.call_with_return(context, id, arguments, CallReturn::Discard)
            .map(|_| ())
    }
    fn call_with_return(
        self,
        context: &mut dyn Context,
        id: FunctionId,
        arguments: &[u32],
        result_use: CallReturn,
    ) -> Result<Option<u32>> {
        context.check_mmio_contract()?;
        result_use.check(id)?;
        let entry = self.resolve_entry(id)?;
        if let Some(entry) = entry {
            self.invoke_translated(context, entry, Some(arguments), result_use)
        } else if self.original_fallback && exits::contract(id) == exits::ExitContract::Rts {
            let class = execution_class(id).ok_or(Error::Unclassified(id))?;
            if !context.execution_class().permits(class) {
                return Err(Error::ExecutionUnavailable {
                    caller: context.execution_class(),
                    callee: class,
                });
            }
            let ceiling = context.enter_call_ceiling(id)?;
            let boundary_parent = context.frame_state();
            context.observe_call_boundary(call_frame::CallBoundaryEvent::elided_enter(
                id,
                boundary_parent,
            ));
            let result = match result_use {
                CallReturn::Value => context
                    .call_original_with_execution(id, arguments, class)
                    .map(Some),
                CallReturn::Discard => context
                    .call_original_void_with_execution(id, arguments, class)
                    .map(|()| None),
            }
            .map_err(|error| context.protect_timing_failure(error));
            let result = exits::validate_call(id, result, context.revision());
            let result = Error::after_contract_check(context, result);
            let cleanup = context.leave_call_ceiling(ceiling);
            let result = match result {
                Err(Error::ControlTransfer(transfer)) => match cleanup {
                    Err(error) => Err(Error::cleanup_after_transfer(transfer, error)),
                    Ok(()) => Err(Error::ControlTransfer(transfer)),
                },
                Err(error) => Err(error),
                Ok(value) => cleanup.map(|()| value),
            };
            let result = result.map_err(|error| context.protect_timing_failure(error));
            context.observe_call_boundary(call_frame::CallBoundaryEvent::ended(
                id,
                boundary_parent,
                None,
                None,
                result.is_ok(),
            ));
            result
        } else {
            Err(Error::Untranslated(id))
        }
    }

    pub(crate) fn call_prepared(
        self,
        context: &mut dyn Context,
        id: FunctionId,
        result_use: CallReturn,
    ) -> Result<Option<u32>> {
        context.check_mmio_contract()?;
        result_use.check(id)?;
        let entry = self.resolve_entry(id)?.ok_or(Error::Untranslated(id))?;
        self.invoke_translated(context, entry, None, result_use)
    }

    fn invoke_translated(
        self,
        context: &mut dyn Context,
        entry: SelectedEntry,
        arguments: Option<&[u32]>,
        result_use: CallReturn,
    ) -> Result<Option<u32>> {
        context.check_mmio_contract()?;
        let class = execution_class(entry.id()).ok_or(Error::Unclassified(entry.id()))?;
        let parent = context.execution_class();
        if !parent.permits(class) {
            return Err(Error::ExecutionUnavailable {
                caller: parent,
                callee: class,
            });
        }
        if arguments.is_some() && matches!(entry.entry(), TranslatedEntry::Framed(_)) {
            return Err(call_frame::FrameError::PreparedEntryRequired(entry.id()).into());
        }
        let ceiling = context.enter_call_ceiling(entry.id())?;
        let scoped = parent != class;
        if scoped && let Err(error) = context.set_execution_class(class) {
            let _ = context.leave_call_ceiling(ceiling);
            return Err(error);
        }
        let boundary_parent = arguments.map(|_| context.frame_state());
        if let Some(parent) = boundary_parent {
            context.observe_call_boundary(call_frame::CallBoundaryEvent::elided_enter(
                entry.id(),
                parent,
            ));
        }
        let result = match arguments {
            Some(arguments) => entry.invoke_values(context, arguments),
            None => entry.invoke_prepared(context),
        };
        let result = result.map_err(|error| context.protect_timing_failure(error));
        let result = exits::validate_call(entry.id(), result, context.revision());
        let result = Error::after_contract_check(context, result).map(|value| {
            if let Some(value) = value {
                context.set_data_register(0, value);
            }
            result_use.observe(context)
        });
        let restore = if scoped {
            context.set_execution_class(parent)
        } else {
            Ok(())
        };
        let mmio = context.check_mmio_contract();
        let ceiling = context.leave_call_ceiling(ceiling);
        let result = match result {
            Err(Error::ControlTransfer(transfer)) => match mmio.and(restore).and(ceiling) {
                Err(error) => Err(Error::cleanup_after_transfer(transfer, error)),
                Ok(()) => Err(Error::ControlTransfer(transfer)),
            },
            Err(error) if error.preserves_call_frame() => Err(error),
            Err(error) => mmio.and(Err(error)),
            Ok(value) => mmio.and(restore).and(ceiling).map(|()| value),
        };
        let result = result.map_err(|error| context.protect_timing_failure(error));
        if let Some(parent) = boundary_parent {
            context.observe_call_boundary(call_frame::CallBoundaryEvent::ended(
                entry.id(),
                parent,
                None,
                None,
                result.is_ok(),
            ));
        }
        result
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
    /// Exact-stack ABI-void call without a scalar D0 observation.
    pub fn call_at_void(
        self,
        context: &mut dyn Context,
        id: FunctionId,
        arguments: &[u32],
        site: call_frame::CallSite,
    ) -> Result<()> {
        call_frame::call_at_void(context, self, id, arguments, site)
    }
}

/// A translated caller never chooses its callee's implementation or address.
pub fn call(context: &mut dyn Context, id: FunctionId, arguments: &[u32]) -> Result<u32> {
    context.dispatcher().call(context, id, arguments)
}

pub fn call_void(context: &mut dyn Context, id: FunctionId, arguments: &[u32]) -> Result<()> {
    context.dispatcher().call_void(context, id, arguments)
}

pub fn call_at(
    context: &mut dyn Context,
    id: FunctionId,
    arguments: &[u32],
    site: call_frame::CallSite,
) -> Result<u32> {
    context.dispatcher().call_at(context, id, arguments, site)
}

pub fn call_at_void(
    context: &mut dyn Context,
    id: FunctionId,
    arguments: &[u32],
    site: call_frame::CallSite,
) -> Result<()> {
    context
        .dispatcher()
        .call_at_void(context, id, arguments, site)
}

pub static SUBSYSTEMS: &[Subsystem] = &[
    Subsystem {
        name: "audio",
        translations: audio::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: audio::PROVEN_REPLAYED,
        replayed_framed_translations: audio::PROVEN_REPLAYED_FRAMED,
        pending_translations: audio::PENDING_TRANSLATIONS,
        pending_framed_translations: audio::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: audio::ATOMIC_CAPABLE,
        execution_contracts: audio::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "camera",
        translations: camera::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: camera::PROVEN_REPLAYED,
        replayed_framed_translations: camera::PROVEN_REPLAYED_FRAMED,
        pending_translations: camera::PENDING_TRANSLATIONS,
        pending_framed_translations: camera::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: camera::ATOMIC_CAPABLE,
        execution_contracts: camera::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "collision",
        translations: collision::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: collision::PROVEN_REPLAYED,
        replayed_framed_translations: collision::PROVEN_REPLAYED_FRAMED,
        pending_translations: collision::PENDING_TRANSLATIONS,
        pending_framed_translations: collision::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: collision::ATOMIC_CAPABLE,
        execution_contracts: collision::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "core",
        translations: core::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: core::PROVEN_REPLAYED,
        replayed_framed_translations: core::PROVEN_REPLAYED_FRAMED,
        pending_translations: core::PENDING_TRANSLATIONS,
        pending_framed_translations: core::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: core::ATOMIC_CAPABLE,
        execution_contracts: core::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "cutscenes",
        translations: cutscenes::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: cutscenes::PROVEN_REPLAYED,
        replayed_framed_translations: cutscenes::PROVEN_REPLAYED_FRAMED,
        pending_translations: cutscenes::PENDING_TRANSLATIONS,
        pending_framed_translations: cutscenes::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: cutscenes::ATOMIC_CAPABLE,
        execution_contracts: cutscenes::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "earthlings",
        translations: earthlings::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: earthlings::PROVEN_REPLAYED,
        replayed_framed_translations: earthlings::PROVEN_REPLAYED_FRAMED,
        pending_translations: earthlings::PENDING_TRANSLATIONS,
        pending_framed_translations: earthlings::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: earthlings::ATOMIC_CAPABLE,
        execution_contracts: earthlings::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "elevator",
        translations: elevator::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: elevator::PROVEN_REPLAYED,
        replayed_framed_translations: elevator::PROVEN_REPLAYED_FRAMED,
        pending_translations: elevator::PENDING_TRANSLATIONS,
        pending_framed_translations: elevator::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: elevator::ATOMIC_CAPABLE,
        execution_contracts: elevator::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "jamout",
        translations: jamout::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: jamout::PROVEN_REPLAYED,
        replayed_framed_translations: jamout::PROVEN_REPLAYED_FRAMED,
        pending_translations: jamout::PENDING_TRANSLATIONS,
        pending_framed_translations: jamout::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: jamout::ATOMIC_CAPABLE,
        execution_contracts: jamout::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "map",
        translations: map::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: map::PROVEN_REPLAYED,
        replayed_framed_translations: map::PROVEN_REPLAYED_FRAMED,
        pending_translations: map::PENDING_TRANSLATIONS,
        pending_framed_translations: map::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: map::ATOMIC_CAPABLE,
        execution_contracts: map::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "platform",
        translations: platform::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: platform::PROVEN_REPLAYED,
        replayed_framed_translations: platform::PROVEN_REPLAYED_FRAMED,
        pending_translations: platform::PENDING_TRANSLATIONS,
        pending_framed_translations: platform::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: platform::ATOMIC_CAPABLE,
        execution_contracts: platform::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "player",
        translations: player::TRANSLATIONS,
        framed_translations: player::FRAMED_TRANSLATIONS,
        replayed_translations: player::PROVEN_REPLAYED,
        replayed_framed_translations: player::PROVEN_REPLAYED_FRAMED,
        pending_translations: player::PENDING_TRANSLATIONS,
        pending_framed_translations: player::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: player::ATOMIC_CAPABLE,
        execution_contracts: player::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "presents",
        translations: presents::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: presents::PROVEN_REPLAYED,
        replayed_framed_translations: presents::PROVEN_REPLAYED_FRAMED,
        pending_translations: presents::PENDING,
        pending_framed_translations: presents::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: presents::ATOMIC_CAPABLE,
        execution_contracts: presents::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "runtime",
        translations: runtime::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: runtime::PROVEN_REPLAYED,
        replayed_framed_translations: runtime::PROVEN_REPLAYED_FRAMED,
        pending_translations: runtime::PENDING_TRANSLATIONS,
        pending_framed_translations: runtime::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: runtime::ATOMIC_CAPABLE,
        execution_contracts: runtime::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "ship",
        translations: ship::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: ship::PROVEN_REPLAYED,
        replayed_framed_translations: ship::PROVEN_REPLAYED_FRAMED,
        pending_translations: ship::PENDING_TRANSLATIONS,
        pending_framed_translations: ship::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: ship::ATOMIC_CAPABLE,
        execution_contracts: ship::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "speech",
        translations: speech::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: speech::PROVEN_REPLAYED,
        replayed_framed_translations: speech::PROVEN_REPLAYED_FRAMED,
        pending_translations: speech::PENDING_TRANSLATIONS,
        pending_framed_translations: speech::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: speech::ATOMIC_CAPABLE,
        execution_contracts: speech::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "sprites",
        translations: sprites::TRANSLATIONS,
        framed_translations: sprites::FRAMED_TRANSLATIONS,
        replayed_translations: sprites::PROVEN_REPLAYED,
        replayed_framed_translations: sprites::PROVEN_REPLAYED_FRAMED,
        pending_translations: sprites::PENDING_TRANSLATIONS,
        pending_framed_translations: sprites::PENDING_FRAMED_TRANSLATIONS,
        atomic_capable: sprites::ATOMIC_CAPABLE,
        execution_contracts: sprites::EXECUTION_CONTRACTS,
    },
    Subsystem {
        name: "ui",
        translations: ui::TRANSLATIONS,
        framed_translations: &[],
        replayed_translations: ui::PROVEN_REPLAYED,
        replayed_framed_translations: ui::PROVEN_REPLAYED_FRAMED,
        pending_translations: ui::PENDING_TRANSLATIONS,
        pending_framed_translations: ui::PENDING_FRAMED_TRANSLATIONS,
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
            for contract in owner.execution_contracts {
                let id = contract.function;
                assert_eq!(id.metadata().subsystem, owner.name);
                assert!(contract.is_valid());
                assert!(
                    contract.class != execution::ExecutionClass::Atomic
                        || contract.operational_ceiling.is_some()
                        || contract.proof_envelope != execution::ProofEnvelope::ClassDefined
                );
                assert!(classified.insert(id));
                assert_eq!(execution_contract(id), Some(contract));
            }
            for id in owner
                .translations
                .iter()
                .map(|entry| entry.id)
                .chain(owner.framed_translations.iter().map(|entry| entry.id))
            {
                assert_eq!(id.metadata().subsystem, owner.name);
                assert!(translated.insert(id), "duplicate translated entry: {id:?}");
                assert!(
                    execution_class(id).is_some(),
                    "unclassified translation: {}",
                    id.metadata().name
                );
            }
        }
        for function in functions() {
            assert!(subsystem(function.subsystem).is_some());
            assert!(
                classified.contains(&function.id),
                "missing class: {}",
                function.name
            );
            assert!(
                execution_contract(function.id).is_some(),
                "invalid class: {}",
                function.name
            );
        }
        assert!(!is_atomic_capable(ids::SETUP_YM2612));
        let waiting = ids::WAIT_FOR_NEXT_V_BLANK;
        assert!(!is_atomic_capable(waiting));
    }

    #[test]
    fn replayed_availability_never_enters_default_production_selection() {
        fn framed_fixture(_: &mut dyn Context) -> Result<Option<u32>> {
            Ok(None)
        }
        // Availability metadata is not a proof or a production entry.
        static OWNERS: &[Subsystem] = &[Subsystem {
            name: "runtime",
            translations: &[],
            framed_translations: &[],
            pending_translations: &[],
            pending_framed_translations: &[],
            replayed_translations: &[Translation {
                id: ids::ABS,
                function: runtime::abs,
            }],
            replayed_framed_translations: &[FramedTranslation {
                id: ids::LMODT,
                function: framed_fixture,
            }],
            atomic_capable: &[],
            execution_contracts: &[],
        }];
        let production = Dispatcher {
            subsystems: OWNERS,
            ..Dispatcher::hybrid()
        };
        assert_eq!(production.translations().count(), 0);
        for id in [ids::ABS, ids::LMODT] {
            assert!(production.resolve_entry(id).unwrap().is_none());
        }
        assert_eq!(
            replayed_translations().count(),
            SUBSYSTEMS
                .iter()
                .map(|owner| owner.replayed_translations.len())
                .sum::<usize>()
        );
        assert_eq!(
            replayed_framed_translations().count(),
            SUBSYSTEMS
                .iter()
                .map(|owner| owner.replayed_framed_translations.len())
                .sum::<usize>()
        );
        for id in replayed_translations()
            .map(|entry| entry.id)
            .chain(replayed_framed_translations().map(|entry| entry.id))
        {
            assert!(!translations().any(|entry| entry.id == id));
            assert!(!framed_translations().any(|entry| entry.id == id));
            assert!(Dispatcher::hybrid().resolve_entry(id).unwrap().is_none());
        }
    }

    #[test]
    fn contract_index_rejects_duplicate_and_missing_declarations() {
        static ONE: &[FunctionId] = &[ids::ABS];
        static DUPLICATE: &[FunctionId] = &[ids::ABS, ids::ABS];
        let owner = |atomic_capable| Subsystem {
            name: "runtime",
            translations: &[],
            framed_translations: &[],
            replayed_translations: &[],
            replayed_framed_translations: &[],
            pending_translations: &[],
            pending_framed_translations: &[],
            atomic_capable,
            execution_contracts: &[],
        };
        let single = index_execution_contracts(&[owner(ONE)]);
        assert!(matches!(
            single[usize::from(ids::ABS.0)],
            ContractDeclaration::Declared(_)
        ));
        assert!(matches!(
            single[usize::from(ids::GET_RANDOM_NUMBER.0)],
            ContractDeclaration::Missing
        ));
        let duplicate = index_execution_contracts(&[owner(DUPLICATE)]);
        assert!(matches!(
            duplicate[usize::from(ids::ABS.0)],
            ContractDeclaration::Conflict
        ));
    }

    #[test]
    fn measured_compute_contracts_remain_atomic_and_pending_without_implicit_bounds() {
        for (id, instructions, ticks, frames) in [
            (ids::MAP_ASSIGN_TRANSITION_CHAIN, 5_000_000, 5_000_000, 300),
            (
                ids::EARTHLING_RETAIN_LEVEL_BANK,
                13_000_000,
                14_000_000,
                1_100,
            ),
            (
                ids::FIND_SPAWN_LOCATION_FOR_EARTHLING,
                4_000_000,
                4_000_000,
                300,
            ),
        ] {
            let contract = execution_contract(id).unwrap();
            assert_eq!(contract.class, execution::ExecutionClass::Atomic);
            assert_eq!(
                contract.proof_envelope,
                execution::ProofEnvelope::ClockedCommutingInterrupts
            );
            let ceiling = contract.operational_ceiling.unwrap();
            assert_eq!(ceiling.reference_instructions, Some(instructions));
            assert_eq!(ceiling.ticks, ticks);
            assert_eq!(ceiling.frames, frames);
            assert!(
                contract
                    .evidence
                    .unwrap()
                    .contains("Operational ceiling, not a bound")
            );
            assert!(Dispatcher::hybrid().translation(id).is_none());
            assert!(Dispatcher::hybrid().framed_translation(id).is_none());
        }
        assert!(!execution::ExecutionClass::Atomic.permits(execution::ExecutionClass::Functional));
    }
}
