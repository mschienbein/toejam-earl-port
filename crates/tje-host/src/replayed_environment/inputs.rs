//! Actual-path callee inputs, including values saved to a private slot and
//! restored before an algorithm use. No preserved-register subtraction by name.
use super::frame::private_access as private;
use super::{
    Failure, Unsupported, UnsupportedKind,
    record::{Access, Chronology, Direction, Event, Invocation},
};
use crate::abi::canonical_ram_address;
use std::{collections::BTreeMap, collections::btree_map::Entry};
use tje_engine_core::{
    InstructionIr, ReadKind,
    dependencies::{
        DependencyInput, DependencyReport, DependencyScratch, DependencyUseKind,
        ObservedAddressDomain, ObservedRead, StateMask, analyze_instruction,
    },
    ir::{StateField, Width},
};
mod analysis;
mod exception_frame;
#[cfg(all(test, feature = "reference-proof-hooks"))]
pub(super) use exception_frame::ExceptionFrame;
mod origins;
use analysis::Analyzer;
pub(super) use origins::{MemoryInput, OutputInputs};

#[derive(Default)]
pub(super) struct CompilerCache {
    instructions: BTreeMap<u32, InstructionIr>,
}
impl CompilerCache {
    fn instruction<'a>(
        &'a mut self,
        reader: &dyn Fn(u32) -> Option<u16>,
        pc: u32,
        divider: u32,
    ) -> Result<&'a InstructionIr, Failure> {
        Ok(match self.instructions.entry(pc) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(
                tje_engine_core::compile_instruction_with_word_reader(reader, pc, divider)
                    .map_err(|error| unsupported(format!("original IR at {pc:06x}: {error:?}")))?,
            ),
        })
    }
}
fn unsupported(detail: impl Into<String>) -> Failure {
    Failure::Unsupported(Unsupported {
        kind: UnsupportedKind::RegisterDependency,
        detail: detail.into(),
    })
}
fn kind(purpose: m68000_emu::ReadPurpose) -> ReadKind {
    match purpose {
        m68000_emu::ReadPurpose::Semantic => ReadKind::Semantic,
        m68000_emu::ReadPurpose::Prefetch => ReadKind::Prefetch,
        m68000_emu::ReadPurpose::Discarded => ReadKind::Discarded,
    }
}
fn stack(access: Access) -> bool {
    access.ram()
        && access.bytes().all(|(a, _)| {
            (crate::catalogue::STACK_BOTTOM..crate::catalogue::STACK_TOP)
                .contains(&canonical_ram_address(a))
        })
}

/// Reused, generic SSA target-EA/control analysis for an actual successful call.
#[derive(Default)]
pub(super) struct CallTarget {
    scratch: DependencyScratch,
    report: DependencyReport,
    reads: Vec<ObservedRead>,
}
impl CallTarget {
    pub fn registers(
        &mut self,
        owner: &Invocation,
        instruction: &super::record::Instruction,
        reader: &dyn Fn(u32) -> Option<u16>,
        divider: u32,
        cache: &mut CompilerCache,
    ) -> Result<StateMask, Failure> {
        self.reads.clear();
        for &access in &owner.trace.issues[instruction.issues.clone()] {
            if let Direction::Read(purpose) = access.direction {
                self.reads.push(ObservedRead {
                    address: access.address,
                    width: if access.word {
                        Width::Word
                    } else {
                        Width::Byte
                    },
                    kind: kind(purpose),
                    value: u32::from(access.value),
                });
            }
        }
        let ir = cache.instruction(reader, instruction.actual.pc, divider)?;
        if ir.opcode != instruction.actual.opcode
            || !crate::call_observation::successful_call_instruction(instruction.actual)
        {
            return Err(unsupported(
                "target analysis lacks its actual successful call opcode",
            ));
        }
        let before = instruction.before.cpu.engine();
        analyze_instruction(
            ir,
            DependencyInput::Executed {
                before: &before,
                reads: &self.reads,
                addresses: ObservedAddressDomain::M68000Bus24,
            },
            &mut self.scratch,
            &mut self.report,
        )
        .map_err(|error| unsupported(format!("actual call target binding: {error:?}")))?;
        if self
            .report
            .instruction
            .obligations
            .exception_taken
            .is_some()
            || self.report.instruction.obligations.opaque_memory_values
            || self.report.instruction.physical_writes.get(StateField::Pc) == 0
        {
            return Err(unsupported(
                "actual call target has an unresolved/faulted PC output",
            ));
        }
        let target = self
            .scratch
            .output_reads(StateField::Pc, u32::MAX)
            .map_err(|error| unsupported(format!("bound call target dependency: {error}")))?;
        let required = target.reads | target.control_reads;
        let registers = required.registers_only();
        if target.memory_values || required != registers {
            return Err(unsupported(format!(
                "call target has unbound memory/non-register control dependencies: {required:?}"
            )));
        }
        Ok(registers)
    }
}
pub(super) struct CalleeInputs {
    pub registers: StateMask,
    pub memory: BTreeMap<u32, MemoryInput>,
    pub output_inputs: OutputInputs,
    /// The root's certified RTS is transport; any other physical observation of
    /// its return slot needs a Rust-owned prepared call boundary, not a scalar
    /// adapter's copy of an original return site.
    pub return_slot_as_data: bool,
}
#[cfg(test)]
pub(super) struct InputDemandSite {
    pub owner: Option<tje_game::FunctionId>,
    pub pc: u32,
    pub opcode: u16,
    pub kind: String,
    pub private_traffic: bool,
    pub registers: StateMask,
}

pub(super) fn analyze(
    invocation: &Invocation,
    reader: &dyn Fn(u32) -> Option<u16>,
    divider: u32,
    cache: &mut CompilerCache,
    declared_outputs: StateMask,
) -> Result<CalleeInputs, Failure> {
    let mut analyzer = Analyzer::new(invocation);
    analyzer.walk(invocation, reader, divider, cache, false)?;
    let outputs = analyzer.origins.registers(declared_outputs);
    analyzer.require(outputs);
    let required = analyzer.origins.demand(analyzer.required)?;
    Ok(CalleeInputs {
        registers: required.registers,
        memory: required.memory,
        return_slot_as_data: analyzer.return_slot_as_data,
        output_inputs: analyzer.origins.finish(),
    })
}

/// Diagnostic union by actual Rust root, vector and handler entry PC. No counts,
/// admission, import permissions or all-path contracts are derived from it.
pub(super) type InterruptedRegisterReads = BTreeMap<(tje_game::FunctionId, u32, u32), StateMask>;
pub(super) struct InterruptInputs {
    pub registers: StateMask,
    pub memory: BTreeMap<u32, MemoryInput>,
    #[cfg(test)]
    pub demands: Option<Vec<InputDemandSite>>,
}
/// ISR preservation transport is not an algorithmic import. Its actual input
/// dependencies still propagate through private saves/restores into every
/// branch, address, external write and DMA value that consumes them.
pub(super) fn interrupt_inputs(
    invocation: &Invocation,
    reader: &dyn Fn(u32) -> Option<u16>,
    divider: u32,
    cache: &mut CompilerCache,
) -> Result<InterruptInputs, Failure> {
    let mut analyzer = Analyzer::new(invocation);
    #[cfg(test)]
    if std::env::var("TJE_REPLAY_ISR_DEMANDS").ok().as_deref() == Some("1") {
        analyzer.demands = Some(Vec::new());
    }
    analyzer.walk(invocation, reader, divider, cache, true)?;
    let required = analyzer.origins.demand(analyzer.required)?;
    Ok(InterruptInputs {
        registers: required.registers,
        memory: required.memory,
        #[cfg(test)]
        demands: analyzer.demands,
    })
}

#[derive(Default)]
pub(super) struct FrozenInterruptRequirements {
    owner: Option<super::record::InvocationId>,
    supervisor_stack_aligned: bool,
}
impl FrozenInterruptRequirements {
    pub fn verify(
        &self,
        candidate: &super::strict::CandidateObservation,
        admission: &super::RootAdmission,
        original: &Invocation,
    ) -> Result<(), Failure> {
        if self
            .owner
            .is_some_and(|owner| owner != original.entry.identity)
        {
            return Err(unsupported(
                "frozen IRQ requirements belong to a different actual invocation",
            ));
        }
        if self.supervisor_stack_aligned && !candidate.frozen_stack_aligned(admission, original) {
            return Err(unsupported(
                "actual certified ISR requires SSP bit0=0, but this admitted Rust fork lacks an eligible all-mutations stack-alignment witness",
            ));
        }
        Ok(())
    }
}

/// A frozen strict fork has no provider at an actual asynchronous boundary.
/// Do not infer those ISR inputs from its final CPU snapshot or ABI clobbers.
pub(super) fn frozen_interrupt_guard(
    invocation: &Invocation,
    reader: &dyn Fn(u32) -> Option<u16>,
    divider: u32,
    cache: &mut CompilerCache,
) -> Result<FrozenInterruptRequirements, Failure> {
    fn minimum(invocation: &Invocation, mut low: u32) -> u32 {
        for instruction in &invocation.trace.instructions {
            low = low
                .min(canonical_ram_address(instruction.before.cpu.sp()))
                .min(canonical_ram_address(instruction.after_sp));
        }
        for event in &invocation.trace.events {
            if let Event::Callee(child) | Event::Interrupt(child) = event {
                low = minimum(child, low);
            }
        }
        low
    }
    fn walk(
        invocation: &Invocation,
        private: &std::ops::Range<u32>,
        reader: &dyn Fn(u32) -> Option<u16>,
        divider: u32,
        cache: &mut CompilerCache,
        requirements: &mut FrozenInterruptRequirements,
    ) -> Result<(), Failure> {
        for event in &invocation.trace.events {
            match event {
                Event::Interrupt(interrupt) => {
                    let consumed = interrupt_inputs(interrupt, reader, divider, cache)?;
                    // interrupt_inputs succeeds only after the actual accepted
                    // hardware save and matching own RTE are certified. The
                    // one timeless bit below still needs this candidate fork's
                    // all-mutations invariant; no endpoint/provider is invented.
                    let alignment = StateMask::single(StateField::Ssp, 1);
                    if consumed.registers.get(StateField::Ssp) & 1 != 0 {
                        if interrupt.entry.boundary.cpu.words[16] & 1 != 0 {
                            return Err(unsupported(
                                "actual certified ISR has no even-SSP alignment relation",
                            ));
                        }
                        requirements.supervisor_stack_aligned = true;
                    }
                    let unavailable = consumed.registers - alignment;
                    if !unavailable.is_empty() {
                        return Err(unsupported(format!(
                            "actual ISR {:06x} consumes register/CCR inputs {:?} unavailable at a frozen strict boundary; no final-snapshot equivalence is inferred",
                            interrupt.entry.boundary.cpu.pc(),
                            unavailable,
                        )));
                    }
                    if consumed
                        .memory
                        .keys()
                        .any(|address| private.contains(&canonical_ram_address(*address)))
                    {
                        return Err(Failure::Unsupported(Unsupported { kind: UnsupportedKind::Frame,
                            detail: "actual ISR observes private frame/residue outside a frozen strict fork's measured output domain".into() }));
                    }
                    walk(interrupt, private, reader, divider, cache, requirements)?;
                }
                Event::Callee(child) => walk(child, private, reader, divider, cache, requirements)?,
                _ => {}
            }
        }
        Ok(())
    }
    let high = canonical_ram_address(invocation.entry.boundary.cpu.sp());
    let mut requirements = FrozenInterruptRequirements {
        owner: Some(invocation.entry.identity),
        ..Default::default()
    };
    walk(
        invocation,
        &(minimum(invocation, high)..high),
        reader,
        divider,
        cache,
        &mut requirements,
    )?;
    Ok(requirements)
}
