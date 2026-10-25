//! Instruction-boundary scheduling for generated 68000 continuations.
//!
//! There is no instruction decoder or semantic executor here. A backend executes
//! one already-compiled instruction; devices retain the reference's deferred
//! writes and timing. IRQ admission and acknowledgement are separate steps.

pub mod abi;
pub mod portable_timing;
pub mod timing_region;

mod exception;

#[cfg(feature = "devices")]
mod platform;
#[cfg(feature = "devices")]
mod snapshot;

#[cfg(feature = "devices")]
pub use snapshot::{MachineSnapshot, RewindBuffer, SNAPSHOT_VERSION, SnapshotError};

pub use tje_engine_core::{Bus, CpuState, ExceptionBoundary, ReadKind, StepResult};

#[cfg(feature = "devices")]
pub use platform::{BusAccess, Engine, ExceptionObservation, PlatformDevices, SampleRateError};

/// One original instruction, not an entire source-level function or basic block.
/// Every successful call must leave PC and prefetch at the next original
/// instruction boundary, and return the complete base cycle charge. A nonzero
/// exception vector describes the frame still to be entered by the runtime.
pub trait Backend {
    type Error;

    fn execute(&mut self, cpu: &mut CpuState, bus: &mut dyn Bus)
    -> Result<StepResult, Self::Error>;
}

/// Device progress for one instruction, wait, interrupt phase, or stopped tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeviceStep {
    pub cpu_cycles: u32,
    pub master_clocks: u64,
    pub frame_complete: bool,
}

/// Timing-aware bus implemented by the platform, or a synthetic test machine.
/// `finish_instruction` must advance Z80, input/PSG, VDP, and YM in reference
/// order, update interrupt latches, and only then publish deferred CPU writes.
pub trait Devices: Bus {
    type Inputs;

    fn set_inputs(&mut self, inputs: Self::Inputs);
    fn begin_instruction(&mut self, pc: u32, opcode: u16);
    /// Set the prefetched opcode's device-read phase before noninstruction bus
    /// accesses or progress. Compiled instructions set their own phase in IR.
    fn prepare_noninstruction(&mut self);
    fn take_wait_step(&mut self, pc: u32) -> Option<DeviceStep>;
    fn finish_instruction(&mut self, pc: u32, cpu_cycles: u32) -> DeviceStep;
    fn cpu_halted(&self) -> bool;
    fn pending_interrupt(&self) -> u8;
    fn acknowledge_interrupt(&mut self, level: u8);
    fn reset_asserted(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Instruction,
    Wait,
    Reset,
    Halted,
    Stopped,
    InterruptAdmission { level: u8 },
    InterruptAcknowledge { level: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeStep {
    pub devices: DeviceStep,
    pub kind: StepKind,
    /// Only actual continuation executions contribute instruction coverage.
    pub instruction_pc: Option<u32>,
}

/// Owns architectural state and the exact boundary between code and devices.
/// Construction reads reset vectors/prefetch without advancing device clocks,
/// matching `GenesisEmulator::create`. Hardware reset asserted during execution
/// is instead a clocked 132-cycle step.
pub struct Machine<D, B> {
    cpu: CpuState,
    devices: D,
    backend: B,
    frames: u64,
}

impl<D: Devices, B: Backend> Machine<D, B> {
    pub fn new(mut devices: D, backend: B) -> Self {
        let mut cpu = CpuState {
            sr: 0x2700,
            ..CpuState::default()
        };
        devices.prepare_noninstruction();
        exception::reset(&mut cpu, &mut devices);
        Self {
            cpu,
            devices,
            backend,
            frames: 0,
        }
    }

    /// Resume an already synchronized CPU/device snapshot without any bus read,
    /// queue drain, input update, or device-clock movement.
    pub fn from_state(cpu: CpuState, devices: D, backend: B, frames: u64) -> Self {
        Self {
            cpu,
            devices,
            backend,
            frames,
        }
    }

    pub fn cpu(&self) -> &CpuState {
        &self.cpu
    }

    pub fn cpu_mut(&mut self) -> &mut CpuState {
        &mut self.cpu
    }

    pub fn devices(&self) -> &D {
        &self.devices
    }

    pub fn devices_mut(&mut self) -> &mut D {
        &mut self.devices
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn frame_count(&self) -> u64 {
        self.frames
    }

    pub fn into_parts(self) -> (CpuState, D, B) {
        (self.cpu, self.devices, self.backend)
    }

    /// Execute exactly one step of the reference loop. Hardware waits always
    /// precede reset/interrupt admission; a level latched before the ten-cycle
    /// admission delay remains latched even if the device line later changes.
    pub fn step(&mut self, inputs: D::Inputs) -> Result<RuntimeStep, B::Error> {
        self.devices.set_inputs(inputs);
        let opcode = self.cpu.prefetch as u16;
        self.devices.begin_instruction(self.cpu.pc, opcode);
        let pc = self.cpu.pc;
        if let Some(devices) = self.devices.take_wait_step(pc) {
            return Ok(self.complete(devices, StepKind::Wait, None));
        }
        let (cycles, kind, instruction_pc) = if self.devices.reset_asserted() {
            self.devices.prepare_noninstruction();
            exception::reset(&mut self.cpu, &mut self.devices);
            (132, StepKind::Reset, None)
        } else if self.devices.cpu_halted() || self.cpu.frozen != 0 {
            self.devices.prepare_noninstruction();
            // The reference polls HALT/frozen once per CPU cycle. STOP, below,
            // is distinct and polls in four-cycle steps while admitting IRQs.
            (1, StepKind::Halted, None)
        } else {
            self.cpu.address_error = 0;
            self.cpu.last_muldiv = 0;
            if self.cpu.pending_interrupt != 0 {
                self.devices.prepare_noninstruction();
                let level = self.cpu.pending_interrupt as u8;
                self.cpu.pending_interrupt = 0;
                self.devices.acknowledge_interrupt(level);
                self.cpu.stopped = 0;
                let cycles = exception::interrupt(&mut self.cpu, &mut self.devices, level);
                (cycles, StepKind::InterruptAcknowledge { level }, None)
            } else {
                let level = self.devices.pending_interrupt() & 7;
                let mask = ((self.cpu.sr >> 8) & 7) as u8;
                // Deliberately matches the reference, including its masked
                // level-seven behavior rather than inventing an NMI edge.
                if level > mask {
                    self.devices.prepare_noninstruction();
                    self.cpu.pending_interrupt = u32::from(level);
                    (10, StepKind::InterruptAdmission { level }, None)
                } else if self.cpu.stopped != 0 {
                    self.devices.prepare_noninstruction();
                    (4, StepKind::Stopped, None)
                } else if pc & 1 != 0 {
                    self.devices.prepare_noninstruction();
                    // A failed opcode fetch precedes decoding, so the reference
                    // executor's opcode is still zero for this address frame.
                    let result = StepResult {
                        cycles: 50,
                        vector: 3,
                        saved_pc: pc.wrapping_sub(2),
                        fault_address: pc.wrapping_add(2),
                        fault_status: 0x15,
                    };
                    let cycles = exception::finish(&mut self.cpu, &mut self.devices, 0, result);
                    (cycles, StepKind::Instruction, Some(pc))
                } else {
                    let result = self.backend.execute(&mut self.cpu, &mut self.devices)?;
                    // The compiled RTE has completed all successful frame reads
                    // and its return prefetch; peripherals have not advanced.
                    if opcode == 0x4e73
                        && result.vector == 0
                        && result.cycles == 20
                        && self.cpu.address_error == 0
                    {
                        self.devices
                            .observe_exception_boundary(ExceptionBoundary::Returned);
                    }
                    if result.vector != 0 {
                        self.devices.prepare_noninstruction();
                    }
                    let cycles =
                        exception::finish(&mut self.cpu, &mut self.devices, opcode, result);
                    (cycles, StepKind::Instruction, Some(pc))
                }
            }
        };
        let devices = self.devices.finish_instruction(pc, cycles);
        Ok(self.complete(devices, kind, instruction_pc))
    }

    fn complete(
        &mut self,
        devices: DeviceStep,
        kind: StepKind,
        instruction_pc: Option<u32>,
    ) -> RuntimeStep {
        if devices.frame_complete {
            self.frames += 1;
        }
        RuntimeStep {
            devices,
            kind,
            instruction_pc,
        }
    }
}

#[cfg(test)]
mod tests;
