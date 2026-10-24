//! The shipped peripheral adapter. No 68000 CPU object is stored or consulted.

use crate::{
    Backend, Bus, CpuState, DeviceStep, Devices, ExceptionBoundary, Machine, ReadKind, exception,
};
use genesis_config::GenesisInputs;
use jgenesis_common::frontend::{Color, FrameSize};
use std::fmt;
use std::ops::{Deref, DerefMut};
use tje_native_platform::{BusReadPurpose, ClockedDevices, DeviceRuntime};

pub type Engine<B> = Machine<PlatformDevices, B>;

/// Bridges the compiler's bus trait without coupling the peripheral crate to
/// the compiler. Device inspection remains borrowed through `Deref`.
pub struct PlatformDevices {
    devices: DeviceRuntime,
    instruction_trace: Option<Vec<BusAccess>>,
    exception_trace: Option<Vec<ExceptionObservation>>,
}

/// Optional physical 68000 bus journal. Addresses are normalized to 24 bits;
/// writes are recorded at issue, before the device layer defers publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusAccess {
    Read {
        address: u32,
        word: bool,
        value: u16,
        kind: ReadKind,
    },
    Write {
        address: u32,
        word: bool,
        value: u16,
    },
    AcknowledgeInterrupt {
        level: u8,
    },
}

/// Indexed architectural boundary, before the following peripheral advance.
/// The index refers to the optional CPU bus journal, not RAM commit order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExceptionObservation {
    pub master_clock: u64,
    pub bus_access_index: usize,
    pub boundary: ExceptionBoundary,
}

impl From<DeviceRuntime> for PlatformDevices {
    fn from(devices: DeviceRuntime) -> Self {
        Self {
            devices,
            instruction_trace: None,
            exception_trace: None,
        }
    }
}

impl PlatformDevices {
    pub fn into_inner(self) -> DeviceRuntime {
        self.devices
    }

    pub fn enable_instruction_trace(&mut self) {
        self.instruction_trace
            .get_or_insert_with(|| Vec::with_capacity(40));
    }

    pub fn disable_instruction_trace(&mut self) {
        self.instruction_trace = None;
        self.exception_trace = None;
    }

    pub fn clear_instruction_trace(&mut self) {
        if let Some(trace) = &mut self.instruction_trace {
            trace.clear();
        }
        if let Some(trace) = &mut self.exception_trace {
            trace.clear();
        }
    }

    pub fn instruction_trace(&self) -> Option<&[BusAccess]> {
        self.instruction_trace.as_deref()
    }

    pub fn enable_exception_trace(&mut self) {
        self.enable_instruction_trace();
        self.exception_trace.get_or_insert_with(Vec::new);
    }

    pub fn exception_trace(&self) -> Option<&[ExceptionObservation]> {
        self.exception_trace.as_deref()
    }

    fn record(&mut self, access: BusAccess) {
        if let Some(trace) = &mut self.instruction_trace {
            trace.push(access);
        }
    }
}

impl Deref for PlatformDevices {
    type Target = DeviceRuntime;

    fn deref(&self) -> &Self::Target {
        &self.devices
    }
}

impl DerefMut for PlatformDevices {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.devices
    }
}

/// Invalid output rates are rejected before the hardware resampler is changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleRateError(pub u32);

impl fmt::Display for SampleRateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "audio sample rate {} must be between 8000 and 192000 Hz",
            self.0
        )
    }
}

impl std::error::Error for SampleRateError {}

impl From<tje_native_platform::DeviceStep> for DeviceStep {
    fn from(step: tje_native_platform::DeviceStep) -> Self {
        Self {
            cpu_cycles: step.cpu_cycles,
            master_clocks: step.master_clocks,
            frame_complete: step.frame_complete,
        }
    }
}

fn native_read_purpose(kind: ReadKind) -> BusReadPurpose {
    match kind {
        ReadKind::Semantic => BusReadPurpose::Semantic,
        ReadKind::Prefetch => BusReadPurpose::Prefetch,
        ReadKind::Discarded => BusReadPurpose::Discarded,
    }
}

impl Bus for PlatformDevices {
    fn read8(&mut self, address: u32, kind: ReadKind) -> u8 {
        let value = self.read_bus_byte_tagged(address, native_read_purpose(kind));
        self.record(BusAccess::Read {
            address: address & 0xFFFFFF,
            word: false,
            value: value.into(),
            kind,
        });
        value
    }

    fn read16(&mut self, address: u32, kind: ReadKind) -> u16 {
        let value = self.read_bus_word_tagged(address, native_read_purpose(kind));
        self.record(BusAccess::Read {
            address: address & 0xFFFFFF,
            word: true,
            value,
            kind,
        });
        value
    }

    fn write8(&mut self, address: u32, value: u8) {
        self.record(BusAccess::Write {
            address: address & 0xFFFFFF,
            word: false,
            value: value.into(),
        });
        self.write_bus_byte(address, value);
    }

    fn write16(&mut self, address: u32, value: u16) {
        self.record(BusAccess::Write {
            address: address & 0xFFFFFF,
            word: true,
            value,
        });
        self.write_bus_word(address, value);
    }

    fn set_read_offset_mclk(&mut self, offset: u32) {
        DeviceRuntime::set_read_offset_mclk(&mut self.devices, u64::from(offset));
    }

    fn observe_exception_boundary(&mut self, boundary: ExceptionBoundary) {
        if let Some(trace) = &mut self.exception_trace {
            trace.push(ExceptionObservation {
                master_clock: self.devices.clock(),
                bus_access_index: self
                    .instruction_trace
                    .as_ref()
                    .expect("boundary bus trace")
                    .len(),
                boundary,
            });
        }
    }
}

impl Devices for PlatformDevices {
    type Inputs = GenesisInputs;

    fn set_inputs(&mut self, inputs: GenesisInputs) {
        self.devices.set_inputs(inputs);
    }

    fn begin_instruction(&mut self, pc: u32, opcode: u16) {
        self.set_instruction_pc(pc);
        ClockedDevices::begin_instruction(&mut self.devices, opcode, 0);
    }

    fn prepare_noninstruction(&mut self) {
        let cycles = tje_engine_core::timing::cycles_if_move_btst_cmp(self.devices.prefetch_word())
            .map_or(8, |cycles| cycles - 4);
        let clocks = u64::from(cycles) * self.devices.cycles().m68k_divider.get();
        self.devices.set_read_offset_mclk(clocks);
    }

    fn take_wait_step(&mut self, pc: u32) -> Option<DeviceStep> {
        if self.devices.wait_pending() {
            self.prepare_noninstruction();
        }
        ClockedDevices::take_wait_step(&mut self.devices, pc).map(Into::into)
    }

    fn finish_instruction(&mut self, pc: u32, cpu_cycles: u32) -> DeviceStep {
        ClockedDevices::finish_instruction(&mut self.devices, pc, cpu_cycles).into()
    }

    fn cpu_halted(&self) -> bool {
        ClockedDevices::cpu_halted(&self.devices)
    }

    fn pending_interrupt(&self) -> u8 {
        ClockedDevices::pending_interrupt(&self.devices)
    }

    fn acknowledge_interrupt(&mut self, level: u8) {
        self.record(BusAccess::AcknowledgeInterrupt { level });
        ClockedDevices::acknowledge_interrupt(&mut self.devices);
    }

    fn reset_asserted(&self) -> bool {
        self.devices.reset_asserted()
    }
}

impl<B: Backend> Machine<PlatformDevices, B> {
    pub fn from_devices(devices: DeviceRuntime, backend: B) -> Self {
        Self::new(devices.into(), backend)
    }

    /// Advance to the next rendered frame and borrow its exact stereo samples.
    /// Allocation, frame/RAM snapshots, and trace collection are not performed
    /// by this method; device recording remains opt-in.
    pub fn play_frame(&mut self, inputs: GenesisInputs) -> Result<&[[f64; 2]], B::Error> {
        self.devices.clear_audio();
        loop {
            if self.step(inputs)?.devices.frame_complete {
                return Ok(self.devices.audio_samples());
            }
        }
    }

    pub fn frame_buffer(&self) -> (&[Color], FrameSize) {
        self.devices.frame_buffer()
    }

    pub fn target_fps(&self) -> f64 {
        self.devices.target_fps()
    }

    pub fn set_audio_sample_rate(&mut self, rate: u32) -> Result<(), SampleRateError> {
        if !(8_000..=192_000).contains(&rate) {
            return Err(SampleRateError(rate));
        }
        self.devices.set_audio_sample_rate(rate);
        Ok(())
    }

    /// Recreate devices and power-on CPU state, retaining the compiled ROM
    /// continuations and configured output sample rate. No reset clocks elapse.
    pub fn reset(&mut self) {
        self.devices.reset();
        self.cpu = CpuState {
            sr: 0x2700,
            ..CpuState::default()
        };
        self.devices.prepare_noninstruction();
        exception::reset(&mut self.cpu, &mut self.devices);
        self.frames = 0;
    }

    /// A non-consuming query that includes both hardware waits and IRQ entry.
    pub fn instruction_ready(&self) -> bool {
        !self.devices.wait_pending()
            && !self.devices.reset_asserted()
            && !ClockedDevices::cpu_halted(&self.devices.devices)
            && self.cpu.frozen == 0
            && self.cpu.stopped == 0
            && self.cpu.pending_interrupt == 0
            && u32::from(ClockedDevices::pending_interrupt(&self.devices.devices))
                <= ((self.cpu.sr >> 8) & 7)
    }
}

#[cfg(test)]
mod discard_forwarding_tests {
    use super::*;

    #[test]
    fn read_kind_reaches_one_device_read_and_the_unchanged_cpu_journal() {
        let memory = tje_mem::AddressSpace::new(vec![0; 256], tje_mem::Revision::Rev02);
        let runtime = DeviceRuntime::new(memory, &genesis_config::GenesisEmulatorConfig::default());
        let mut devices = PlatformDevices::from(runtime);
        devices.enable_trace();
        devices.enable_instruction_trace();
        for kind in [ReadKind::Semantic, ReadKind::Prefetch, ReadKind::Discarded] {
            Bus::read16(&mut devices, 0xa10003, kind);
        }
        let cpu = devices.instruction_trace().unwrap();
        let hardware = &devices.trace().unwrap().accesses;
        assert_eq!(cpu.len(), 3);
        assert_eq!(hardware.len(), 3);
        for (index, kind) in [ReadKind::Semantic, ReadKind::Prefetch, ReadKind::Discarded]
            .into_iter()
            .enumerate()
        {
            assert!(matches!(cpu[index], BusAccess::Read {
                address: 0xa10003, word: true, kind: actual, ..
            } if actual == kind));
            assert_eq!(
                hardware[index].purpose,
                Some(native_read_purpose(kind) as u8)
            );
            assert_eq!(
                (
                    hardware[index].address,
                    hardware[index].word,
                    hardware[index].write
                ),
                (0xa10003, true, false)
            );
        }
    }
}
