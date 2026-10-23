//! Native interrupt authority for readable timing regions. The provider owns
//! Rust game effects, but cannot claim an RTE or advance an invented clock.
//! Each call returns after ONE actual device step, including frame boundaries.
use super::*;
use crate::{StepKind, exception};

/// A request is not a receipt. ExceptionReturn is accepted only at an actual
/// prefetched RTE; the controller itself performs the real exception-frame
/// return and device step. It is not an original-opcode fallback.
pub enum IsrAction {
    SourceInstruction(InstructionTiming),
    ExceptionReturn,
    NoProgress,
}
pub trait ScheduledIsrClosure {
    fn timing_program(&self) -> &std::sync::Arc<NativeIsrProgram>;
    fn supports_pc(&self, pc: u32) -> bool;
    fn step(&mut self, source: &mut IsrSourceContext<'_>) -> Result<IsrAction, TimingError>;
}

/// Minted only after actual accepted interrupt entry, real scheduled progress,
/// an actual RTE frame return, and exact bank/pipeline validation. Not Clone,
/// not constructible by the provider, and never accepted as a replay token.
#[derive(Debug)]
pub struct RteReceipt {
    owner: ScratchIdentity,
    serial: u64,
    steps: u64,
    master_clocks: u64,
    scheduled_instructions: u64,
}
impl RteReceipt {
    pub fn identity(&self) -> (ScratchIdentity, u64) {
        (self.owner, self.serial)
    }
    pub fn steps(&self) -> u64 {
        self.steps
    }
    pub fn master_clocks(&self) -> u64 {
        self.master_clocks
    }
    pub fn scheduled_instructions(&self) -> u64 {
        self.scheduled_instructions
    }
}
#[derive(Debug)]
pub struct NativeInterruptStep {
    pub devices: DeviceStep,
    pub kind: StepKind,
    pub returned: Option<RteReceipt>,
}
pub(super) struct Frame {
    pc: u32,
    prefetch: u32,
    sr: u32,
    usp: u32,
    ssp: u32,
}
pub(super) struct ControllerState {
    token: InterruptToken,
    frames: Vec<Frame>,
    entered: bool,
    instructions: u64,
}

impl TimingRegion<'_> {
    pub fn start_native_interrupt(
        &mut self,
        token: InterruptToken,
        cpu: &CpuState,
    ) -> Result<(), TimingError> {
        if self.ownership != IsrOwnership::ScheduledRust || self.native_isr.is_none() {
            return Err(TimingError::MissingIsrCapability);
        }
        if !self.suspended
            || self.native_controller.is_some()
            || token.owner != self.owner
            || token.serial != self.serial
            || cpu.pc != token.pipeline.pc
            || cpu.prefetch != token.pipeline.prefetch
            || cpu.usp != token.usp
            || cpu.ssp != token.ssp
            || cpu.sr != token.sr
            || cpu.address_error != 0
        {
            return Err(TimingError::WrongToken);
        }
        self.check_step_budget()?;
        let mut frames = std::mem::take(&mut self.native_frames);
        frames.clear();
        self.native_controller = Some(ControllerState {
            token,
            frames,
            entered: false,
            instructions: 0,
        });
        Ok(())
    }
    /// Resumable: inputs are supplied for this actual step; FrameRendered is
    /// returned to the caller before another step so its existing timeline can
    /// sample/apply the next frame exactly once. No stored platform borrow.
    pub fn step_native_interrupt<D: Devices>(
        &mut self,
        cpu: &mut CpuState,
        devices: &mut D,
        inputs: D::Inputs,
    ) -> Result<NativeInterruptStep, TimingError> {
        self.check_step_budget()?;
        let mut controller = self
            .native_controller
            .take()
            .ok_or(TimingError::WrongToken)?;
        if cpu.address_error != 0 {
            return Err(TimingError::PendingFault);
        }
        devices.set_inputs(inputs);
        let pc = cpu.pc;
        let opcode = cpu.prefetch as u16;
        devices.begin_instruction(pc, opcode);
        if let Some(step) = devices.take_wait_step(pc) {
            self.record_progress(step)?;
            self.native_controller = Some(controller);
            return Ok(NativeInterruptStep {
                devices: step,
                kind: StepKind::Wait,
                returned: None,
            });
        }
        if devices.reset_asserted() {
            return Err(TimingError::InvalidIr(
                "native timing region abandoned by reset",
            ));
        }
        let (cycles, kind, returned) = if devices.cpu_halted() || cpu.frozen != 0 {
            devices.prepare_noninstruction();
            (1, StepKind::Halted, false)
        } else {
            cpu.last_muldiv = 0;
            if cpu.pending_interrupt != 0 {
                let level = cpu.pending_interrupt as u8;
                if !controller.entered
                    && controller.token.action != (BoundaryAction::InterruptAcknowledge { level })
                    && controller.token.action != (BoundaryAction::InterruptAdmission { level })
                {
                    return Err(TimingError::WrongEntry);
                }
                // Fixed diagnostic capacity, not an ISA claim about nesting.
                // No growth/allocation is permitted while servicing an ISR.
                if controller.frames.len() == controller.frames.capacity() {
                    return Err(TimingError::Exhausted);
                }
                let frame = Frame {
                    pc: cpu.pc,
                    prefetch: cpu.prefetch,
                    sr: cpu.sr,
                    usp: cpu.usp,
                    ssp: cpu.ssp,
                };
                devices.prepare_noninstruction();
                devices.acknowledge_interrupt(level);
                cpu.pending_interrupt = 0;
                cpu.stopped = 0;
                let cycles = exception::interrupt(cpu, devices, level);
                controller.frames.push(frame);
                controller.entered = true;
                (cycles, StepKind::InterruptAcknowledge { level }, false)
            } else {
                let level = devices.pending_interrupt() & 7;
                if level > ((cpu.sr >> 8) & 7) as u8 {
                    if !controller.entered
                        && controller.token.action != (BoundaryAction::InterruptAdmission { level })
                    {
                        return Err(TimingError::WrongEntry);
                    }
                    devices.prepare_noninstruction();
                    cpu.pending_interrupt = u32::from(level);
                    (10, StepKind::InterruptAdmission { level }, false)
                } else if cpu.stopped != 0 {
                    devices.prepare_noninstruction();
                    (4, StepKind::Stopped, false)
                } else {
                    if !controller.entered || controller.frames.is_empty() {
                        return Err(TimingError::InvalidIr(
                            "ISR provider called without actual accepted entry",
                        ));
                    }
                    let program = self
                        .native_program
                        .as_ref()
                        .ok_or(TimingError::MissingIsrCapability)?;
                    let expected_instruction = program
                        .instructions
                        .get(&pc)
                        .ok_or(TimingError::MissingIsrCapability)?;
                    if expected_instruction.opcode() != opcode {
                        return Err(TimingError::WrongEntry);
                    }
                    let handler = self
                        .native_isr
                        .take()
                        .ok_or(TimingError::MissingIsrCapability)?;
                    let mut evidence = super::native_source::SourceEvidence::default();
                    let action = handler.step(&mut IsrSourceContext::new(
                        cpu,
                        devices,
                        program,
                        &mut self.scratch,
                        &mut evidence,
                    ));
                    self.native_isr = Some(handler);
                    if evidence.failed {
                        return Err(TimingError::WrongObservation);
                    }
                    match action? {
                        IsrAction::NoProgress => {
                            return Err(TimingError::InvalidIr(
                                "scheduled ISR made no instruction progress",
                            ));
                        }
                        IsrAction::ExceptionReturn => {
                            if opcode != 0x4e73
                                || cpu.pc != pc
                                || cpu.prefetch != u32::from(opcode)
                                || cpu.sr & 0x2000 == 0
                            {
                                return Err(TimingError::WrongToken);
                            }
                            let frame = controller.frames.last().ok_or(TimingError::WrongToken)?;
                            if cpu.usp != frame.usp || cpu.ssp != frame.ssp.wrapping_sub(6) {
                                return Err(TimingError::WrongEntry);
                            }
                            devices.prepare_noninstruction();
                            let cycles = exception::return_from_exception(cpu, devices);
                            if cycles == 20 && cpu.address_error == 0 {
                                let frame =
                                    controller.frames.pop().ok_or(TimingError::WrongToken)?;
                                if cpu.pc != frame.pc
                                    || cpu.prefetch != frame.prefetch
                                    || cpu.sr != frame.sr
                                    || cpu.usp != frame.usp
                                    || cpu.ssp != frame.ssp
                                {
                                    return Err(TimingError::WrongEntry);
                                }
                            }
                            controller.instructions = controller
                                .instructions
                                .checked_add(1)
                                .ok_or(TimingError::Exhausted)?;
                            (cycles, StepKind::Instruction, controller.frames.is_empty())
                        }
                        IsrAction::SourceInstruction(timing) => {
                            if evidence.owner != Some(timing.identity())
                                || evidence.bus_effects != timing.bus_effects()
                                || evidence.phase_updates != timing.phase_updates()
                                || timing.pc() != pc
                                || timing.opcode() != opcode
                                || cpu.pc != pc
                                || cpu.prefetch != u32::from(opcode)
                                || opcode == 0x4e73
                                || !program.edges.contains(&(pc, timing.pipeline().pc))
                            {
                                return Err(TimingError::WrongToken);
                            }
                            let pipeline = timing.pipeline();
                            cpu.pc = pipeline.pc;
                            cpu.prefetch = pipeline.prefetch;
                            cpu.last_muldiv = pipeline.last_muldiv;
                            controller.instructions = controller
                                .instructions
                                .checked_add(1)
                                .ok_or(TimingError::Exhausted)?;
                            (timing.cycles(), StepKind::Instruction, false)
                        }
                    }
                }
            }
        };
        let step = devices.finish_instruction(pc, cycles);
        if kind == StepKind::Instruction {
            self.usage.scheduled_isr_instructions = self
                .usage
                .scheduled_isr_instructions
                .checked_add(1)
                .ok_or(TimingError::Exhausted)?;
        }
        self.record_progress(step)?;
        if step.cpu_cycles != cycles
            || step.master_clocks == 0
            || cpu.address_error != 0
            || cpu.frozen != 0
        {
            return Err(TimingError::PendingFault);
        }
        let divider = self
            .native_program
            .as_ref()
            .ok_or(TimingError::MissingIsrCapability)?
            .divider;
        if step.master_clocks != u64::from(step.cpu_cycles) * u64::from(divider) {
            return Err(TimingError::WrongEntry);
        }
        if returned {
            let receipt = RteReceipt {
                owner: self.owner,
                serial: controller.token.serial,
                steps: self.usage.scheduler_steps - controller.token.start_steps,
                master_clocks: self.usage.master_clocks - controller.token.start_clocks,
                scheduled_instructions: controller.instructions,
            };
            self.native_frames = controller.frames;
            self.resume_interrupt(controller.token, cpu, controller.instructions, true)?;
            Ok(NativeInterruptStep {
                devices: step,
                kind,
                returned: Some(receipt),
            })
        } else {
            self.native_controller = Some(controller);
            Ok(NativeInterruptStep {
                devices: step,
                kind,
                returned: None,
            })
        }
    }
}
