//! Explicit non-RTS control flow. These events unwind Rust callers without
//! pretending that a reset, exception return, or persistent loop returned a D0.

use crate::{Context, Error, FunctionId, Result, TranslationFn, execution::ExecutionClass, ids};
use tje_mem::Revision;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackBanks {
    pub user: u32,
    pub supervisor: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExceptionFrame {
    pub status_register: u16,
    pub program_counter: u32,
}

/// 68000 RTE consumes six bytes from SSP before restoring SR selects USP/SSP.
/// The platform additionally records a successful exception return, if observed.
pub fn restore_exception(context: &mut dyn Context) -> Result<ExceptionFrame> {
    if context.status_register() & 0x2000 == 0 {
        return Err(Error::Platform("RTE requires supervisor mode".into()));
    }
    let mut banks = context.stack_banks()?;
    let sp = banks.supervisor;
    let frame = ExceptionFrame {
        status_register: context.read_u16(sp),
        program_counter: context.read_u32(sp.wrapping_add(2)),
    };
    banks.supervisor = sp.wrapping_add(6);
    context.set_stack_banks(banks)?;
    context.set_status_register(frame.status_register);
    Ok(frame)
}

/// Materialize the actual final JSR return word; never substitute a sentinel.
pub fn push_call(context: &mut dyn Context, return_pc: u32) -> Result<()> {
    let mut banks = context.stack_banks()?;
    let sp = if context.status_register() & 0x2000 != 0 {
        &mut banks.supervisor
    } else {
        &mut banks.user
    };
    *sp = sp.wrapping_sub(4);
    context.write_u32(*sp, return_pc);
    context.set_stack_banks(banks)
}

/// Finite boundaries of the persistent main loop, not video-frame boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainLoopStage {
    AfterIntro,
    AfterMenu,
    /// Snapshot the frame counter, advance the word-sized phase, and do game work.
    Begin,
    /// Draw idle random numbers until a real interrupt changes the frame counter.
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    ExceptionReturn,
    Tail {
        target: FunctionId,
    },
    /// A JSR boundary, including modal startup calls resumed in another stage.
    Call {
        target: FunctionId,
        return_pc: u32,
    },
    /// Stop game logic, without popping an exception or caller frame.
    /// Device clocks and interrupt servicing belong to the platform scheduler.
    Terminal,
    MainLoop {
        stage: MainLoopStage,
    },
}

/// The source identity survives propagation through any number of Rust callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlTransfer {
    pub entry: FunctionId,
    pub exit: Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallCheckpoint {
    pub target: FunctionId,
    pub return_offset: u32,
    pub resume: MainLoopStage,
}

const STARTUP_CALLS: &[CallCheckpoint] = &[
    CallCheckpoint {
        target: ids::SEGA_AND_INTRO_VIDEO_ROUTINE,
        return_offset: 0xe0,
        resume: MainLoopStage::AfterIntro,
    },
    CallCheckpoint {
        target: ids::NEW_GAME_MAIN_MENU,
        return_offset: 0xe6,
        resume: MainLoopStage::AfterMenu,
    },
];

/// Reviewed exits are orthogonal to Atomic / Functional / TimeAware execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitContract {
    Rts,
    ExceptionReturn,
    Tail {
        target: FunctionId,
    },
    Call {
        target: FunctionId,
        return_offset: u32,
    },
    Terminal,
    MainLoop {
        begin_offset: u32,
        idle_offset: u32,
        calls: &'static [CallCheckpoint],
    },
}

/// Source authority: G src/asm/interrupts.s:14-73, src/asm/startup.s:23-102,
/// src/core/error_trap.c:5, and src/core/startup_and_main_loop.c:58,607-707.
/// This reviewed policy is deliberately separate from generated inventory ABI.
pub fn contract(entry: FunctionId) -> ExitContract {
    match entry {
        ids::LEVEL4_ISR | ids::LEVEL6_ISR => ExitContract::ExceptionReturn,
        ids::RESTART_FROM_RESET_ENTRY => ExitContract::Tail { target: ids::START },
        ids::START => ExitContract::Tail {
            target: ids::GAME_PROGRAM,
        },
        ids::GAME_PROGRAM => ExitContract::Call {
            target: ids::VDP_INITIALISATION,
            return_offset: 38,
        },
        ids::VDP_INITIALISATION => ExitContract::Call {
            target: ids::FURTHER_INITIALISATION,
            return_offset: 8,
        },
        ids::ERROR_TRAP => ExitContract::Terminal,
        ids::FURTHER_INITIALISATION => ExitContract::MainLoop {
            begin_offset: 0x260,
            idle_offset: 0x2fc,
            calls: STARTUP_CALLS,
        },
        _ => ExitContract::Rts,
    }
}

impl ExitContract {
    pub fn accepts(self, entry: FunctionId, exit: Exit, revision: Revision) -> bool {
        self.accepts_at(entry.metadata().address(revision), exit)
    }

    pub fn accepts_at(self, entry_address: u32, exit: Exit) -> bool {
        match (self, exit) {
            (Self::ExceptionReturn, Exit::ExceptionReturn)
            | (Self::Terminal, Exit::Terminal)
            | (
                Self::MainLoop { .. },
                Exit::MainLoop {
                    stage: MainLoopStage::Begin | MainLoopStage::Idle,
                },
            ) => true,
            (Self::Tail { target: expected }, Exit::Tail { target }) => target == expected,
            (
                Self::Call {
                    target: expected,
                    return_offset,
                },
                Exit::Call { target, return_pc },
            ) => target == expected && return_pc == entry_address + return_offset,
            (Self::MainLoop { calls, .. }, Exit::Call { target, return_pc }) => {
                calls.iter().any(|call| {
                    call.target == target && return_pc == entry_address + call.return_offset
                })
            }
            _ => false,
        }
    }

    pub fn checkpoint(
        self,
        entry: FunctionId,
        stage: MainLoopStage,
        revision: Revision,
    ) -> Option<u32> {
        self.checkpoint_at(entry.metadata().address(revision), stage)
    }

    pub fn checkpoint_at(self, entry_address: u32, stage: MainLoopStage) -> Option<u32> {
        let Self::MainLoop {
            begin_offset,
            idle_offset,
            calls,
        } = self
        else {
            return None;
        };
        Some(
            entry_address
                + match stage {
                    MainLoopStage::AfterIntro | MainLoopStage::AfterMenu => {
                        calls
                            .iter()
                            .find(|call| call.resume == stage)?
                            .return_offset
                    }
                    MainLoopStage::Begin => begin_offset,
                    MainLoopStage::Idle => idle_offset,
                },
        )
    }
}

/// Never returns success: `?` propagates a transfer through ordinary translated
/// callers, so abandoned callers cannot execute their remaining code or cleanup.
pub fn request<T>(entry: FunctionId, exit: Exit) -> Result<T> {
    Err(Error::ControlTransfer(ControlTransfer { entry, exit }))
}

pub fn exception_return<T>(entry: FunctionId) -> Result<T> {
    request(entry, Exit::ExceptionReturn)
}
pub fn tail<T>(entry: FunctionId, target: FunctionId) -> Result<T> {
    request(entry, Exit::Tail { target })
}
pub fn call_transfer<T>(entry: FunctionId, target: FunctionId, return_pc: u32) -> Result<T> {
    request(entry, Exit::Call { target, return_pc })
}
pub fn terminal<T>(entry: FunctionId) -> Result<T> {
    request(entry, Exit::Terminal)
}
pub fn main_loop<T>(entry: FunctionId, stage: MainLoopStage) -> Result<T> {
    request(entry, Exit::MainLoop { stage })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryOutcome {
    Returned(Option<u32>),
    Transfer(ControlTransfer),
}

/// An entry can terminate through its own exit or a genuinely reachable callee.
/// The fixed worklist allocates nothing and visits each catalogue identity once.
pub fn reaches(root: FunctionId, entry: FunctionId) -> bool {
    if root == entry {
        return true;
    }
    let mut seen = [false; 648];
    let mut pending = [root; 648];
    let mut count = 1;
    seen[usize::from(root.0)] = true;
    while count != 0 {
        count -= 1;
        let id = pending[count];
        if id == entry {
            return true;
        }
        for &callee in id.metadata().callees {
            if !seen[usize::from(callee.0)] {
                seen[usize::from(callee.0)] = true;
                pending[count] = callee;
                count += 1;
            }
        }
    }
    false
}

pub fn validate(
    root: FunctionId,
    root_contract: ExitContract,
    result: Result<Option<u32>>,
    revision: Revision,
) -> Result<EntryOutcome> {
    validate_at(
        root,
        root_contract,
        root.metadata().address(revision),
        result,
        revision,
    )
}

pub fn validate_at(
    root: FunctionId,
    root_contract: ExitContract,
    root_address: u32,
    result: Result<Option<u32>>,
    revision: Revision,
) -> Result<EntryOutcome> {
    match result {
        Ok(value) if root_contract == ExitContract::Rts => Ok(EntryOutcome::Returned(value)),
        Ok(_) => Err(Error::Platform(format!(
            "{} returned normally despite its {root_contract:?} exit contract",
            root.metadata().name,
        ))),
        Err(Error::ControlTransfer(transfer)) => {
            let (reviewed, address) = if transfer.entry == root {
                (root_contract, root_address)
            } else {
                (
                    contract(transfer.entry),
                    transfer.entry.metadata().address(revision),
                )
            };
            if !reaches(root, transfer.entry) || !reviewed.accepts_at(address, transfer.exit) {
                return Err(Error::Platform(format!(
                    "invalid control transfer from {} while entering {}: {:?}",
                    transfer.entry.metadata().name,
                    root.metadata().name,
                    transfer.exit,
                )));
            }
            Ok(EntryOutcome::Transfer(transfer))
        }
        Err(error) => Err(error),
    }
}

/// Ordinary nested calls validate their callee but never consume its nonreturning
/// event. This is what prevents a caller from continuing with a stale D0.
pub fn validate_call(
    entry: FunctionId,
    result: Result<Option<u32>>,
    revision: Revision,
) -> Result<Option<u32>> {
    match validate(entry, contract(entry), result, revision)? {
        EntryOutcome::Returned(value) => Ok(value),
        EntryOutcome::Transfer(transfer) => Err(Error::ControlTransfer(transfer)),
    }
}

/// Execute an already prepared entry; unlike `call`, this does not invent a JSR
/// or consume a caller frame. Native schedulers consume the resulting event.
pub fn enter(
    context: &mut dyn Context,
    entry: FunctionId,
    arguments: &[u32],
) -> Result<EntryOutcome> {
    context.check_mmio_contract()?;
    let translation = context
        .dispatcher()
        .resolve_entry(entry)?
        .ok_or(Error::Untranslated(entry))?;
    enter_body(context, entry, arguments, translation)
}

fn enter_body(
    context: &mut dyn Context,
    entry: FunctionId,
    arguments: &[u32],
    body: crate::TranslatedEntry,
) -> Result<EntryOutcome> {
    context.check_mmio_contract()?;
    let class = crate::execution_class(entry).ok_or(Error::Unclassified(entry))?;
    let parent = context.execution_class();
    if !parent.permits(class) {
        return Err(Error::ExecutionUnavailable {
            caller: parent,
            callee: class,
        });
    }
    if !arguments.is_empty() && matches!(body, crate::TranslatedEntry::Framed(_)) {
        return Err(crate::call_frame::FrameError::UnexpectedValueArguments {
            id: entry,
            count: arguments.len(),
        }
        .into());
    }
    let ceiling = context.enter_call_ceiling(entry)?;
    if class != parent
        && let Err(error) = context.set_execution_class(class)
    {
        let _ = context.leave_call_ceiling(ceiling);
        return Err(error);
    }
    let result = body.invoke_entry(context, arguments);
    let result = validate(entry, contract(entry), result, context.revision());
    let restore = if class != parent {
        context.set_execution_class(parent)
    } else {
        Ok(())
    };
    let mmio = context.check_mmio_contract();
    let ceiling = context.leave_call_ceiling(ceiling);
    let outcome = match result {
        Ok(EntryOutcome::Transfer(transfer)) => match mmio.and(restore).and(ceiling) {
            Err(error) => return Err(Error::cleanup_after_transfer(transfer, error)),
            Ok(()) => EntryOutcome::Transfer(transfer),
        },
        Err(error) if error.preserves_call_frame() => return Err(error),
        Err(error) => return mmio.and(Err(error)),
        Ok(outcome) => {
            mmio.and(restore).and(ceiling)?;
            outcome
        }
    };
    if let EntryOutcome::Returned(Some(value)) = outcome {
        context.set_data_register(0, value);
    }
    Ok(outcome)
}

#[derive(Clone, Copy)]
pub struct MainLoopBodies {
    pub entry: FunctionId,
    pub after_intro: TranslationFn,
    pub after_menu: TranslationFn,
    pub begin: TranslationFn,
    pub idle: TranslationFn,
}

impl MainLoopBodies {
    pub fn body(self, stage: MainLoopStage) -> TranslationFn {
        match stage {
            MainLoopStage::AfterIntro => self.after_intro,
            MainLoopStage::AfterMenu => self.after_menu,
            MainLoopStage::Begin => self.begin,
            MainLoopStage::Idle => self.idle,
        }
    }
}

/// Native continuation bodies retain phase/time in the original registers and
/// materialized frame RAM. No Rust stack must remain suspended across a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MainLoopContinuation {
    pub entry: FunctionId,
    pub stage: MainLoopStage,
}
impl MainLoopContinuation {
    pub fn resume(self, context: &mut dyn Context, bodies: MainLoopBodies) -> Result<EntryOutcome> {
        if !matches!(contract(self.entry), ExitContract::MainLoop { .. })
            || bodies.entry != self.entry
            || crate::execution_class(self.entry) != Some(ExecutionClass::TimeAware)
        {
            return Err(Error::Platform(
                "not a reviewed persistent main loop".into(),
            ));
        }
        enter_body(
            context,
            self.entry,
            &[],
            crate::TranslatedEntry::Values(crate::Translation {
                id: self.entry,
                function: bodies.body(self.stage),
            }),
        )
    }

    pub fn from_return(entry: FunctionId, return_pc: u32, revision: Revision) -> Option<Self> {
        let ExitContract::MainLoop { calls, .. } = contract(entry) else {
            return None;
        };
        calls
            .iter()
            .find(|call| entry.metadata().address(revision) + call.return_offset == return_pc)
            .map(|call| Self {
                entry,
                stage: call.resume,
            })
    }
}

#[cfg(test)]
#[path = "exits_tests.rs"]
mod tests;
