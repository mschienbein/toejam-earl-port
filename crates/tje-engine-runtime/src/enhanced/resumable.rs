use super::{Budget, EnhancedMode, Error, MAX_DEPTH, Sidecars, context, ready};
use crate::{Backend, Devices, Engine, RuntimeStep, StepKind};
use bincode::{Decode, Encode};
use genesis_config::{GenesisController, GenesisInputs};
use std::fmt;
use tje_game::{FunctionId, Result, exits::ExitContract};
use tje_mem::Bus;

/// Complete input for extras. Original hardware sees only pads zero and one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub struct EnhancedInputs {
    pub pads: [GenesisController; 4],
}
impl From<GenesisInputs> for EnhancedInputs {
    fn from(inputs: GenesisInputs) -> Self {
        Self {
            pads: [
                inputs.p1,
                inputs.p2,
                GenesisController::None,
                GenesisController::None,
            ],
        }
    }
}
impl Default for EnhancedInputs {
    fn default() -> Self {
        GenesisInputs::default().into()
    }
}
impl EnhancedInputs {
    pub fn original(self) -> GenesisInputs {
        GenesisInputs {
            p1: self.pads[0],
            p2: self.pads[1],
        }
    }
}

/// This is the entire resumable Rust activation; no host stack is archived.
#[derive(Debug, Clone, Default, PartialEq, Eq, Encode, Decode)]
pub struct Continuation {
    pub stage: u32,
    pub locals: [u32; 16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    Frames(u32),
    Interrupt,
    /// One 64-cycle scheduler quantum. Recheck the real predicate on resumption.
    Poll,
}

pub enum OverrideStep {
    Return(Option<u32>),
    Wait(Wait),
    /// Run one compiled invocation, bypassing this root's PC/SP (including loop
    /// backedges). Nested calls at other frames retain normal enhanced dispatch.
    CallOriginal {
        function: FunctionId,
        arguments: Vec<u32>,
    },
    /// A before-only hook. Execute this original entry once without retaining a
    /// return obligation (suitable for roots whose callees may reset/nonlocally exit).
    ContinueOriginal,
}

pub trait Context: tje_game::Context {
    fn sidecars(&self) -> &Sidecars;
    fn sidecars_mut(&mut self) -> &mut Sidecars;
    fn inputs(&self) -> EnhancedInputs;
    fn frame_count(&self) -> u64;
    fn entry_arguments(&self) -> Result<Vec<u32>>;
}

pub type ResumableFn = fn(&mut dyn Context, &mut Continuation) -> Result<OverrideStep>;
#[derive(Clone, Copy)]
pub struct ResumableTranslation {
    pub id: FunctionId,
    pub function: ResumableFn,
}

#[derive(Encode, Decode)]
pub(super) enum Action {
    Ready,
    Frames(u64),
    Interrupt(u64),
    Poll(u64),
    Original { pc: u32, sp: u32, started: bool },
    PassThrough,
}
#[derive(Encode, Decode)]
pub(super) struct Invocation {
    pub pc: u32,
    pub sp: u32,
    pub return_pc: u32,
    pub continuation: Continuation,
    pub action: Action,
}

impl EnhancedMode {
    pub(super) fn scheduled_step<B: Backend>(
        &mut self,
        engine: &mut Engine<B>,
    ) -> std::result::Result<RuntimeStep, Error<B::Error>>
    where
        B::Error: fmt::Display,
    {
        let input = self.inputs.original();
        engine.devices_mut().set_inputs(input);
        if let Some(invocation) = self.pending.last_mut() {
            if let Action::Original {
                sp, started: true, ..
            } = invocation.action
                && engine.cpu().pc == invocation.pc
                && engine.cpu().read_address(7) == sp + 4
                && ready(engine)
            {
                engine.cpu_mut().write_address(7, invocation.sp);
                return self.resume(engine).map_err(Error::Override);
            }
            let at_boundary =
                engine.cpu().pc == invocation.pc && engine.cpu().read_address(7) == invocation.sp;
            if at_boundary && ready(engine) {
                if matches!(invocation.action, Action::PassThrough) {
                    self.pending.pop();
                    let step = engine.step(input).map_err(Error::Backend)?;
                    self.observe(step);
                    return Ok(step);
                }
                let resume = match invocation.action {
                    Action::Ready => true,
                    Action::Frames(frame) => engine.frame_count() >= frame,
                    Action::Interrupt(after) => self.interrupts > after,
                    Action::Poll(clock) => engine.devices().clock() >= clock,
                    Action::Original { started, .. } => started,
                    Action::PassThrough => unreachable!("handled before resumable waits"),
                };
                if resume {
                    return self.resume(engine).map_err(Error::Override);
                }
                return Ok(charge(engine, invocation.pc, 64));
            }
            if let Action::Original { pc, sp, started } = &mut invocation.action
                && ready(engine)
                && engine.cpu().pc == *pc
                && engine.cpu().read_address(7) == *sp
            {
                *started = true;
                let step = engine.step(input).map_err(Error::Backend)?;
                self.observe(step);
                return Ok(step);
            }
        }
        if ready(engine)
            && let Ok(index) = self
                .driver
                .resumable
                .binary_search_by_key(&engine.cpu().pc, |entry| entry.0)
        {
            if self.pending.len() >= usize::from(MAX_DEPTH) {
                return Err(Error::Override(context::error(
                    "enhanced resumable nesting limit exhausted",
                )));
            }
            let pc = self.driver.resumable[index].0;
            let sp = engine.cpu().read_address(7);
            context::validate_stack(engine, sp).map_err(Error::Override)?;
            let return_pc = engine.devices().memory().read_u32(sp);
            engine.cpu_mut().address_error = 0;
            engine.cpu_mut().last_muldiv = 0;
            self.pending.push(Invocation {
                pc,
                sp,
                return_pc,
                continuation: Continuation::default(),
                action: Action::Ready,
            });
            return self.resume(engine).map_err(Error::Override);
        }
        let step = self
            .driver
            .step_inner(engine, input, &mut Budget::default())?;
        self.observe(step);
        Ok(step)
    }

    fn observe(&mut self, step: RuntimeStep) {
        if matches!(step.kind, StepKind::InterruptAcknowledge { .. }) {
            self.interrupts = self.interrupts.wrapping_add(1);
        }
    }

    fn resume<B: Backend>(&mut self, engine: &mut Engine<B>) -> Result<RuntimeStep>
    where
        B::Error: fmt::Display,
    {
        let mut invocation = self.pending.pop().expect("scheduled resumable activation");
        let index = self
            .driver
            .resumable
            .binary_search_by_key(&invocation.pc, |entry| entry.0)
            .map_err(|_| context::error("resumable entry is no longer selected"))?;
        let entry = self.driver.resumable[index].1;
        validate_boundary(engine, &invocation)?;
        let mut budget = Budget::default();
        let before_frame = engine.frame_count();
        let outcome = {
            let profile = self.driver.profile.expect("selected profile");
            let mut context = context::Context::new(
                &self.driver,
                engine,
                self.inputs.original(),
                profile.dispatcher(),
                invocation.sp,
                &mut budget,
            )
            .with_extra(&mut self.sidecars, self.inputs, entry.id);
            (entry.function)(&mut context, &mut invocation.continuation)?
        };
        validate_boundary(engine, &invocation)?;
        let transition = match outcome {
            OverrideStep::Return(value) => {
                if let Some(value) = value {
                    engine.cpu_mut().d[0] = value;
                }
                engine.cpu_mut().write_address(7, invocation.sp + 4);
                context::set_pc(engine, invocation.return_pc);
                charge(engine, invocation.pc, 16)
            }
            OverrideStep::ContinueOriginal => {
                invocation.action = Action::PassThrough;
                let step = charge(engine, invocation.pc, 4);
                self.pending.push(invocation);
                step
            }
            OverrideStep::Wait(wait) => {
                invocation.action = match wait {
                    Wait::Frames(frames) => Action::Frames(
                        engine
                            .frame_count()
                            .checked_add(u64::from(frames))
                            .ok_or_else(|| context::error("enhanced frame wait overflow"))?,
                    ),
                    Wait::Interrupt => Action::Interrupt(self.interrupts),
                    Wait::Poll => Action::Poll(
                        engine
                            .devices()
                            .clock()
                            .checked_add(64)
                            .ok_or_else(|| context::error("enhanced polling clock overflow"))?,
                    ),
                };
                let step = charge(engine, invocation.pc, 4);
                self.pending.push(invocation);
                step
            }
            OverrideStep::CallOriginal {
                function,
                arguments,
            } => {
                if tje_game::exits::contract(function) != ExitContract::Rts
                    || arguments.len() != usize::from(function.metadata().abi.argument_count)
                {
                    return Err(context::error(
                        "enhanced original call requires an RTS entry and exact ABI slots",
                    ));
                }
                let bytes = 4 * (arguments.len() as u32 + 1);
                let sp = invocation
                    .sp
                    .checked_sub(bytes)
                    .ok_or_else(|| context::error("enhanced original call stack overflow"))?;
                if sp < 0xe00000 {
                    return Err(context::error(
                        "enhanced original call stack leaves work RAM",
                    ));
                }
                let memory = engine.devices_mut().memory_mut();
                memory.write_u32(sp, invocation.pc);
                for (index, argument) in arguments.into_iter().enumerate() {
                    memory.write_u32(sp + 4 + 4 * index as u32, argument);
                }
                let pc = function
                    .metadata()
                    .address(self.driver.revision.expect("selected revision"));
                invocation.action = Action::Original {
                    pc,
                    sp,
                    started: false,
                };
                engine.cpu_mut().write_address(7, sp);
                context::set_pc(engine, pc);
                let step = charge(engine, invocation.pc, 4);
                self.pending.push(invocation);
                step
            }
        };
        // Synchronous bounded helper calls can contribute progress before the
        // transition. Preserve it in the reported step without recounting frames.
        Ok(RuntimeStep {
            devices: crate::DeviceStep {
                cpu_cycles: budget
                    .progress
                    .cpu_cycles
                    .saturating_add(transition.devices.cpu_cycles),
                master_clocks: budget.progress.master_clocks + transition.devices.master_clocks,
                frame_complete: transition.devices.frame_complete
                    || engine.frame_count() != before_frame,
            },
            kind: StepKind::EnhancedOverride { function: entry.id },
            instruction_pc: None,
        })
    }
}

fn validate_boundary<B: Backend>(engine: &Engine<B>, invocation: &Invocation) -> Result<()> {
    if engine.cpu().read_address(7) != invocation.sp
        || engine.cpu().pc != invocation.pc
        || engine.devices().memory().read_u32(invocation.sp) != invocation.return_pc
    {
        return Err(context::error(
            "resumable callback changed its entry stack or caller return address",
        ));
    }
    Ok(())
}

fn charge<B: Backend>(engine: &mut Engine<B>, pc: u32, cycles: u32) -> RuntimeStep {
    engine.devices_mut().begin_instruction(pc, 0x4e71);
    let devices = engine.devices_mut().finish_instruction(pc, cycles);
    engine.complete(devices, StepKind::Wait, None)
}
