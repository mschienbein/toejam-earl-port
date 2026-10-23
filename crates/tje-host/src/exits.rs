//! Bounded reference execution and native realization of non-RTS exits.

use crate::{HostError, Result, abi::HookContext, hooks::Hook};
use m68000_emu::M68000;
use tje_game::{
    Context, FunctionId,
    exits::{self, EntryOutcome, Exit, ExitContract},
};
use tje_rom::Revision;

/// A reference instruction boundary, not an invented RTS destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReferenceBoundary {
    pub pc: u32,
    pub sp: Option<u32>,
    pub source: Option<(u32, u32)>,
    pub min_instructions: u64,
    pub entry_exception_return: bool,
}
impl ReferenceBoundary {
    pub(crate) fn returned(pc: u32, sp: u32) -> Self {
        Self {
            pc,
            sp: Some(sp),
            source: None,
            min_instructions: 0,
            entry_exception_return: false,
        }
    }
    pub(crate) fn reached(self, cpu: &M68000, previous_pc: Option<u32>, instructions: u64) -> bool {
        instructions >= self.min_instructions
            && cpu.pc() == self.pc
            && self.sp.is_none_or(|sp| cpu.stack_pointer() == sp)
            && self
                .source
                .is_none_or(|(start, end)| previous_pc.is_some_and(|pc| (start..end).contains(&pc)))
    }
}

/// Snapshot the original entry frame before either side executes. RTE reads its
/// six-byte exception frame, never the four-byte ordinary-call return word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EntryState {
    pub sp: u32,
    pub return_pc: u32,
    pub exception_pc: u32,
}
impl EntryState {
    pub(crate) fn capture(context: &HookContext<'_>, contract: ExitContract) -> Result<Self> {
        tje_game::Platform::check_mmio_contract(context)?;
        let banks = context.stack_banks()?;
        let sp = if context.status_register() & 0x2000 != 0 {
            banks.supervisor
        } else {
            banks.user
        };
        Ok(Self {
            sp,
            return_pc: if contract == ExitContract::Rts {
                context.read_u32(sp)
            } else {
                0
            },
            exception_pc: if contract == ExitContract::ExceptionReturn {
                context.read_u32(sp.wrapping_add(2))
            } else {
                0
            },
        })
    }
}

pub(crate) fn reference_boundary(
    hook: &Hook,
    outcome: EntryOutcome,
    entry: EntryState,
    revision: Revision,
) -> Result<ReferenceBoundary> {
    match outcome {
        EntryOutcome::Returned(_) => {
            if hook.exit_contract != ExitContract::Rts {
                return Err(HostError::Metadata(
                    "non-RTS entry returned normally".into(),
                ));
            }
            Ok(ReferenceBoundary::returned(
                entry.return_pc,
                entry.sp.wrapping_add(4),
            ))
        }
        EntryOutcome::Transfer(transfer) => {
            let symbol: &tje_game::FunctionMetadata = if transfer.entry == hook.symbol.id {
                &hook.symbol
            } else {
                transfer.entry.metadata()
            };
            let start = symbol.address(revision);
            let source = Some((start, start + symbol.size(revision)));
            let (pc, source) = match transfer.exit {
                Exit::ExceptionReturn => (entry.exception_pc, source),
                Exit::Tail { target } | Exit::Call { target, .. } => {
                    (target.metadata().address(revision), source)
                }
                Exit::Terminal => (start, source),
                Exit::MainLoop { stage } => {
                    let reviewed = if transfer.entry == hook.symbol.id {
                        hook.exit_contract
                    } else {
                        exits::contract(transfer.entry)
                    };
                    let pc = reviewed.checkpoint_at(start, stage).ok_or_else(|| {
                        HostError::Metadata("missing main-loop checkpoint".into())
                    })?;
                    // A callee's RTS may be the instruction that reaches the checkpoint.
                    (pc, None)
                }
            };
            Ok(ReferenceBoundary {
                pc,
                sp: None,
                source,
                min_instructions: 1,
                entry_exception_return: transfer.exit == Exit::ExceptionReturn,
            })
        }
    }
}

/// Apply only the declared transfer. Abandoned caller frames are not unwound by
/// RTS, and the terminal/main-loop outcomes never manufacture a D0 result.
pub(crate) fn finish(
    context: &mut HookContext<'_>,
    hook: &Hook,
    outcome: EntryOutcome,
) -> Result<()> {
    tje_game::Platform::check_mmio_contract(context)?;
    match outcome {
        EntryOutcome::Returned(value) => context.return_to_caller(value)?,
        EntryOutcome::Transfer(transfer) => match transfer.exit {
            Exit::ExceptionReturn => {
                let restored = exits::restore_exception(context)?;
                context.transfer_to(restored.program_counter)?;
                context.exception_returned();
            }
            Exit::Tail { target } => {
                context.transfer_to(target.metadata().address(context.revision()))?
            }
            Exit::Call { target, return_pc } => {
                exits::push_call(context, return_pc)?;
                context.transfer_to(target.metadata().address(context.revision()))?;
            }
            Exit::Terminal => {
                let pc = if transfer.entry == hook.symbol.id {
                    hook.symbol.address(context.revision())
                } else {
                    transfer.entry.metadata().address(context.revision())
                };
                context.transfer_to(pc)?;
            }
            Exit::MainLoop { stage } => {
                let (reviewed, address) = if transfer.entry == hook.symbol.id {
                    (hook.exit_contract, hook.symbol.address(context.revision()))
                } else {
                    (
                        exits::contract(transfer.entry),
                        transfer.entry.metadata().address(context.revision()),
                    )
                };
                let pc = reviewed
                    .checkpoint_at(address, stage)
                    .ok_or_else(|| HostError::Metadata("missing main-loop checkpoint".into()))?;
                context.transfer_to(pc)?;
            }
        },
    }
    Ok(())
}

pub(crate) fn terminal(outcome: EntryOutcome) -> Option<FunctionId> {
    match outcome {
        EntryOutcome::Transfer(transfer) if transfer.exit == Exit::Terminal => Some(transfer.entry),
        _ => None,
    }
}

#[cfg(test)]
#[path = "exits_tests.rs"]
mod tests;
