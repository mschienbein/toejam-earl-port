//! Portable readable-source binding. Only adapters own clocks, IRQ authority and
//! the active root ledger; a source cannot configure any of those policies.
use crate::CpuState;
use crate::timing_region::{
    RegionEdge, RegionStorage, ReturnTarget, ReviewedRegion, TimingProgram, compile_schedule_ir,
};
use std::collections::{BTreeMap, BTreeSet};
use tje_engine_core::{
    ReadKind, compile_instruction_with_divider,
    decode::{Instruction, decode},
    ir::Width as IrWidth,
    timing_region::{AccessKind, BusObservation, ExpectedAccess, RustPathChoice, TimingError},
};
use tje_game::{FunctionId, timing as game};
use tje_mem::Revision;
use tje_rom::Rom;

mod completion;
pub use completion::{CertifiedReturn, CompletionLedger, Invocation};

#[cfg(feature = "devices")]
pub mod engine;

/// Preserve source-owned callback failures without coupling the ROM-independent
/// timing core to the readable game. Successful execution allocates nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortableError {
    Timing(TimingError),
    Source(tje_game::Error),
}
impl std::fmt::Display for PortableError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timing(cause) => cause.fmt(formatter),
            Self::Source(cause) => cause.fmt(formatter),
        }
    }
}
impl std::error::Error for PortableError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Timing(cause) => Some(cause),
            Self::Source(cause) => Some(cause),
        }
    }
}
impl From<TimingError> for PortableError {
    fn from(cause: TimingError) -> Self {
        Self::Timing(cause)
    }
}
impl From<tje_game::Error> for PortableError {
    fn from(cause: tje_game::Error) -> Self {
        Self::Source(cause)
    }
}
impl PortableError {
    pub fn into_game_error(self) -> tje_game::Error {
        match self {
            Self::Source(cause) => cause,
            Self::Timing(cause) => tje_game::Error::Platform(cause.to_string()),
        }
    }
}

/// Trusted compiled-backend boundary. Implementations must report their actual
/// published artifact, not an interpreter label. Sources cannot select one.
pub trait TimingBackend: crate::Backend {
    fn contains_compiled_instruction(&self, pc: u32) -> bool;
    fn compiled_identity(&self) -> ([u8; 20], [u8; 32]);
    fn authenticates_compilation(
        &self,
        rom_sha1: [u8; 20],
        program: &tje_engine_core::Program,
    ) -> bool;
}

struct BoundSite {
    source: game::InstructionSite,
    ordinary_return: bool,
    opcode: u16,
}

pub struct BoundProgram {
    source: &'static game::Program,
    timing: TimingProgram,
    sites: BTreeMap<u32, BoundSite>,
    locations: BTreeMap<game::Location, u32>,
    revision: Revision,
    rom_digest: &'static str,
    divider: u32,
    irq_policy: &'static [u8],
    rom_length: usize,
}
impl BoundProgram {
    /// IRQ requirements are adapter-reviewed policy, never source authority.
    /// Region-local accounting has no deadline: the active root ledger owns it.
    pub fn compile(
        cartridge: &Rom,
        entry: FunctionId,
        source: &'static game::Program,
        divider: u32,
        required_interrupts: &'static [u8],
    ) -> Result<Self, TimingError> {
        // This ordinary capability owns one dispatcher scope. Crossing into
        // another function needs its own declared class/ceiling admission;
        // source paths must not bypass that boundary.
        if source.functions.len() != 1 || source.functions[0].function != entry {
            return Err(TimingError::InvalidIr(
                "ordinary timing sources require exactly one reviewed function",
            ));
        }
        let rom = cartridge.bytes();
        let revision = cartridge.revision();
        if source.entry.function != entry
            || source.entry.role != 0
            || source.name.is_empty()
            || source.source.is_empty()
            || tje_game::exits::contract(entry) != tje_game::exits::ExitContract::Rts
            || divider == 0
        {
            return Err(TimingError::WrongEntry);
        }
        let mut functions = BTreeSet::new();
        let mut locations = BTreeMap::new();
        let mut sites = BTreeMap::new();
        let mut decoded = Vec::new();
        for declaration in source.functions {
            if declaration.roles == 0 || !functions.insert(declaration.function) {
                return Err(TimingError::InvalidIr("duplicate or empty source function"));
            }
        }
        if !functions.contains(&entry)
            || source.functions.iter().any(|declaration| {
                declaration
                    .function
                    .metadata()
                    .callees
                    .iter()
                    .any(|callee| !functions.contains(callee))
            })
        {
            return Err(TimingError::InvalidIr(
                "source function/callee closure is incomplete",
            ));
        }
        for declaration in source.functions {
            let metadata = declaration.function.metadata();
            let mut pc = metadata.address(revision);
            let end = pc
                .checked_add(metadata.size(revision))
                .ok_or(TimingError::WrongEntry)?;
            for role in 0..declaration.roles {
                if pc >= end {
                    return Err(TimingError::InvalidIr("source has extra instruction roles"));
                }
                let ir = compile_instruction_with_divider(rom, pc, divider).map_err(|_| {
                    TimingError::InvalidIr("source role does not decode in user ROM")
                })?;
                if ir.next_pc <= pc || ir.next_pc > end {
                    return Err(TimingError::InvalidIr(
                        "source role crosses its committed function extent",
                    ));
                }
                // An ordinary-return profile cannot silently implement an RTE
                // from a snapshot: that needs the controller-owned IRQ profile.
                if ir.opcode == 0x4e73 {
                    return Err(TimingError::InvalidIr(
                        "exception-return source profile is unavailable",
                    ));
                }
                let location = game::Location {
                    function: declaration.function,
                    role,
                };
                locations.insert(location, pc);
                if sites
                    .insert(
                        pc,
                        BoundSite {
                            source: game::InstructionSite {
                                location,
                                address: pc,
                                fallthrough: ir.next_pc,
                            },
                            ordinary_return: matches!(
                                decode(ir.opcode),
                                Instruction::Return { restore_ccr: false }
                            ),
                            opcode: ir.opcode,
                        },
                    )
                    .is_some()
                {
                    return Err(TimingError::InvalidIr("overlapping source functions"));
                }
                pc = ir.next_pc;
                decoded.push(ir);
            }
            if pc != end {
                return Err(TimingError::InvalidIr("source omits instruction roles"));
            }
        }
        let address = |location: game::Location| {
            locations
                .get(&location)
                .copied()
                .ok_or(TimingError::InvalidIr(
                    "source edge names an undeclared role",
                ))
        };
        let mut edges = Vec::with_capacity(source.edges.len());
        for edge in source.edges {
            edges.push(RegionEdge {
                from: address(edge.from)?,
                to: address(edge.to)?,
            });
        }
        let reviewed = ReviewedRegion {
            name: source.name,
            source: source.source,
            entry: address(source.entry)?,
            instruction_pcs: sites.keys().copied().collect(),
            edges,
            return_pc: None,
            maximum_steps: u64::MAX,
            required_interrupts,
        };
        let schedules = compile_schedule_ir(rom, decoded.into_iter().map(Ok), &[], divider)?;
        let timing = TimingProgram::from_schedules(reviewed, schedules, divider, None)?;
        Ok(Self {
            source,
            timing,
            sites,
            locations,
            revision,
            rom_digest: revision.sha1(),
            divider,
            irq_policy: required_interrupts,
            rom_length: rom.len(),
        })
    }
    pub fn timing(&self) -> &TimingProgram {
        &self.timing
    }
    pub fn entry(&self) -> FunctionId {
        self.source.entry.function
    }
    pub fn accepts(&self, entry: FunctionId, definition: &'static game::Program) -> bool {
        entry == self.entry() && std::ptr::eq(definition, self.source)
    }
    pub fn prepare_storage(&self) -> RegionStorage {
        RegionStorage::new(&self.timing)
    }
    pub fn matches(&self, cartridge: &Rom, divider: u32, irq_policy: &[u8]) -> bool {
        self.revision == cartridge.revision()
            && self.rom_digest == cartridge.revision().sha1()
            && self.divider == divider
            && self.irq_policy == irq_policy
    }
    pub fn return_target(&self, cartridge: &Rom, pc: u32) -> Result<ReturnTarget, TimingError> {
        if !self.matches(cartridge, self.divider, self.irq_policy) {
            return Err(TimingError::WrongEntry);
        }
        ReturnTarget::in_rom(cartridge.bytes().len(), pc)
    }
    pub fn invocation_target(&self, pc: u32) -> Result<ReturnTarget, TimingError> {
        ReturnTarget::in_rom(self.rom_length, pc)
    }
    pub fn begin<'a>(
        &'a self,
        source: &'a mut dyn game::TimingSource,
        entry: &'a CpuState,
        target: ReturnTarget,
    ) -> Result<PortableInstruction<'a>, PortableError> {
        if !std::ptr::eq(source.program(), self.source) {
            return Err(TimingError::WrongEntry.into());
        }
        let site = self.sites.get(&entry.pc).ok_or(TimingError::WrongEntry)?;
        if entry.address_error != 0
            || entry.pc & 1 != 0
            || entry.frozen != 0
            || entry.stopped != 0
            || entry.pending_interrupt != 0
            || entry.prefetch != u32::from(site.opcode)
        {
            return Err(TimingError::WrongEntry.into());
        }
        source.begin_instruction(site.source, registers(entry))?;
        Ok(PortableInstruction {
            bound: self,
            source,
            entry,
            site,
            target,
            pending: None,
            expected: None,
            issued: 0,
            acknowledged: 0,
            failed: false,
        })
    }
}

pub fn registers(cpu: &CpuState) -> game::Registers {
    game::Registers {
        data: cpu.d,
        address: cpu.a,
        user_stack: cpu.usp,
        supervisor_stack: cpu.ssp,
        status: cpu.sr as u16,
    }
}
pub fn apply_registers(cpu: &mut CpuState, registers: &game::Registers) {
    cpu.d = registers.data;
    cpu.a = registers.address;
    cpu.usp = registers.user_stack;
    cpu.ssp = registers.supervisor_stack;
    cpu.set_sr(u32::from(registers.status));
}

pub struct PortableInstruction<'a> {
    bound: &'a BoundProgram,
    source: &'a mut dyn game::TimingSource,
    entry: &'a CpuState,
    site: &'a BoundSite,
    target: ReturnTarget,
    pending: Option<game::Effect>,
    expected: Option<ExpectedAccess>,
    issued: u32,
    acknowledged: u32,
    failed: bool,
}
impl PortableInstruction<'_> {
    pub fn entry_state(&self) -> &CpuState {
        self.entry
    }
    /// None is pipeline infrastructure. All semantic/discarded reads and all
    /// write payloads must be independently requested by the Rust source.
    pub fn request(
        &mut self,
        expected: ExpectedAccess,
    ) -> Result<Option<game::Effect>, PortableError> {
        if self.failed || self.expected.is_some() {
            self.failed = true;
            return Err(TimingError::PendingAcknowledgement.into());
        }
        self.issued = self.issued.checked_add(1).ok_or(TimingError::Exhausted)?;
        self.expected = Some(expected);
        if expected.kind == AccessKind::Read(ReadKind::Prefetch) {
            return Ok(None);
        }
        let result = self.request_source(expected);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn request_source(
        &mut self,
        expected: ExpectedAccess,
    ) -> Result<Option<game::Effect>, PortableError> {
        let effect = self.source.next_effect()?.ok_or(TimingError::InvalidIr(
            "source dropped a required bus effect",
        ))?;
        let width = match expected.width {
            IrWidth::Byte => game::Width::Byte,
            IrWidth::Word => game::Width::Word,
        };
        let matches_kind = match (expected.kind, effect.kind) {
            (
                AccessKind::Read(ReadKind::Semantic),
                game::EffectKind::Read(game::ReadPurpose::Semantic),
            )
            | (
                AccessKind::Read(ReadKind::Discarded),
                game::EffectKind::Read(game::ReadPurpose::Discarded),
            ) => true,
            (AccessKind::Write, game::EffectKind::Write(value)) => {
                width == game::Width::Word || value <= 255
            }
            _ => false,
        };
        if effect.address & 0xffffff != expected.address & 0xffffff
            || effect.width != width
            || !matches_kind
        {
            return Err(TimingError::WrongObservation.into());
        }
        self.pending = Some(effect);
        Ok(Some(effect))
    }
    pub fn acknowledge(&mut self, actual: BusObservation) -> Result<(), PortableError> {
        if self.failed {
            return Err(TimingError::WrongObservation.into());
        }
        let Some(expected) = self.expected.take() else {
            self.failed = true;
            return Err(TimingError::PendingAcknowledgement.into());
        };
        if expected.address != actual.address
            || expected.width != actual.width
            || expected.kind != actual.kind
        {
            self.failed = true;
            return Err(TimingError::WrongObservation.into());
        }
        self.acknowledged = self
            .acknowledged
            .checked_add(1)
            .ok_or(TimingError::Exhausted)?;
        if actual.kind == AccessKind::Read(ReadKind::Prefetch) {
            if self.pending.is_some() || actual.value > 65535 {
                self.failed = true;
                return Err(TimingError::WrongObservation.into());
            }
            return Ok(());
        }
        let Some(effect) = self.pending.take() else {
            self.failed = true;
            return Err(TimingError::PendingAcknowledgement.into());
        };
        let width = match actual.width {
            IrWidth::Byte => game::Width::Byte,
            IrWidth::Word => game::Width::Word,
        };
        let same = effect.address & 0xffffff == actual.address & 0xffffff
            && effect.width == width
            && match (effect.kind, actual.kind) {
                (
                    game::EffectKind::Read(game::ReadPurpose::Semantic),
                    AccessKind::Read(ReadKind::Semantic),
                )
                | (
                    game::EffectKind::Read(game::ReadPurpose::Discarded),
                    AccessKind::Read(ReadKind::Discarded),
                ) => true,
                (game::EffectKind::Write(value), AccessKind::Write) => {
                    u32::from(value) == actual.value
                }
                _ => false,
            };
        if !same
            || actual.value
                > if width == game::Width::Word {
                    65535
                } else {
                    255
                }
        {
            self.failed = true;
            return Err(TimingError::WrongObservation.into());
        }
        let result = self
            .source
            .acknowledge(game::Observation {
                effect,
                value: actual.value as u16,
            })
            .map_err(PortableError::Source);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    pub fn finish(
        self,
    ) -> Result<(game::Registers, RustPathChoice, Option<game::TimedExit>), PortableError> {
        if self.failed
            || self.expected.is_some()
            || self.pending.is_some()
            || self.issued != self.acknowledged
        {
            return Err(TimingError::PendingAcknowledgement.into());
        }
        if self.source.next_effect()?.is_some() {
            return Err(TimingError::WrongObservation.into());
        }
        let commit = self.source.finish_instruction()?;
        if commit.registers.status & !0xa71f != 0 {
            return Err(TimingError::WrongObservation.into());
        }
        let (next_pc, completed) = match commit.path {
            game::Path::Next(location) => {
                let pc = *self
                    .bound
                    .locations
                    .get(&location)
                    .ok_or(TimingError::InvalidIr("source selected an undeclared role"))?;
                if !self.bound.timing.permits_edge(self.entry.pc, pc) {
                    return Err(TimingError::InvalidIr("source path escapes reviewed edges").into());
                }
                (pc, None)
            }
            game::Path::Return { address, value } => {
                if !self.site.ordinary_return
                    || address != self.target.pc
                    || commit.registers.stack_pointer()
                        != self.entry.read_address(7).wrapping_add(4)
                    || value.is_some_and(|v| v != commit.registers.data[0])
                    || value.is_some()
                        != self
                            .bound
                            .source
                            .entry
                            .function
                            .metadata()
                            .abi
                            .returns_value
                {
                    return Err(TimingError::InvalidIr(
                        "source return boundary or D0 result mismatch",
                    )
                    .into());
                }
                (
                    address,
                    Some(game::TimedExit {
                        entry: self.bound.source.entry.function,
                        program_counter: address,
                        stack_pointer: commit.registers.stack_pointer(),
                        value,
                    }),
                )
            }
        };
        Ok((commit.registers, RustPathChoice { next_pc }, completed))
    }
}

#[cfg(test)]
mod error_tests {
    use super::*;
    #[test]
    fn callback_errors_retain_owned_type_reason_and_standard_cause() {
        for cause in [
            tje_game::Error::Platform("distinct source callback reason".into()),
            tje_game::Error::Clock(tje_game::execution::CallClockError::Unavailable),
        ] {
            let owned = PortableError::from(cause.clone());
            assert_eq!(
                std::error::Error::source(&owned)
                    .unwrap()
                    .downcast_ref::<tje_game::Error>(),
                Some(&cause)
            );
            assert_eq!(owned.into_game_error(), cause);
        }
        let protocol = PortableError::from(TimingError::WrongObservation);
        assert_eq!(
            std::error::Error::source(&protocol)
                .unwrap()
                .downcast_ref::<TimingError>(),
            Some(&TimingError::WrongObservation)
        );
    }
}
