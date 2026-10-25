//! Readable translation adapter and explicit native-root closure validation.
//! This adapter is for subsystem differential proofs. Cycle-exact shipped logic
//! uses `ClockedDevices` and generated instruction continuations instead.

use crate::{ClockedDevices, DeviceRuntime, Registers};
use genesis_config::GenesisEmulatorConfig;
use genesis_core::native::PeripheralState;
use tje_game::call_frame::{CallSite, FrameError, FrameState};
use tje_game::{
    Context, Dispatcher, Error, FunctionId, Platform, Result,
    execution::{ExecutionClass, RamWidth, WaitCondition, WaitRequest},
    exits::{self, EntryOutcome, Exit, StackBanks},
};
use tje_mem::{AddressSpace, Bus, Revision};

#[derive(Debug, Clone, Copy, Default)]
pub struct InterruptHandlers {
    pub line: Option<FunctionId>,
    pub vblank: Option<FunctionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    MissingTranslations(Vec<FunctionId>),
    InvalidState(String),
}
impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingTranslations(ids) => {
                write!(
                    f,
                    "native dependency closure has {} missing translations:",
                    ids.len()
                )?;
                for id in ids {
                    write!(f, " {}", id.metadata().name)?;
                }
                Ok(())
            }
            Self::InvalidState(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for StartError {}

pub struct NativePlatform {
    devices: DeviceRuntime,
    registers: Registers,
    dispatcher: Dispatcher,
    class: ExecutionClass,
    handlers: InterruptHandlers,
    accepted_interrupts: u64,
    program_counter: u32,
    frame: FrameState,
}

impl NativePlatform {
    fn dispatcher(
        roots: &[FunctionId],
        handlers: InterruptHandlers,
    ) -> std::result::Result<Dispatcher, StartError> {
        let mut entries = roots.to_vec();
        entries.extend(handlers.line);
        entries.extend(handlers.vblank);
        Dispatcher::native(&entries).map_err(StartError::MissingTranslations)
    }

    pub fn new(
        memory: AddressSpace,
        roots: &[FunctionId],
        handlers: InterruptHandlers,
        config: &GenesisEmulatorConfig,
    ) -> std::result::Result<Self, StartError> {
        let dispatcher = Self::dispatcher(roots, handlers)?;
        if memory.rom().len() < 8 {
            return Err(StartError::InvalidState(
                "native cartridge has no reset vectors".into(),
            ));
        }
        let sp = memory.read_u32(0);
        let pc = memory.read_u32(4);
        let mut registers = Registers {
            status: 0x2700,
            program_counter: pc,
            supervisor_stack_pointer: sp,
            ..Registers::default()
        };
        registers.address[7] = sp;
        let frame = FrameState::new(sp);
        Ok(Self {
            devices: DeviceRuntime::new(memory, config),
            registers,
            dispatcher,
            class: ExecutionClass::TimeAware,
            handlers,
            accepted_interrupts: 0,
            program_counter: pc,
            frame,
        })
    }

    pub fn from_state(
        memory: AddressSpace,
        state: PeripheralState,
        roots: &[FunctionId],
        handlers: InterruptHandlers,
    ) -> std::result::Result<Self, StartError> {
        let dispatcher = Self::dispatcher(roots, handlers)?;
        let registers = Registers::from_state(&state);
        let frame = FrameState::new(registers.address[7]);
        let devices = DeviceRuntime::from_state(memory, state).map_err(StartError::InvalidState)?;
        Ok(Self {
            devices,
            registers,
            dispatcher,
            class: ExecutionClass::TimeAware,
            handlers,
            accepted_interrupts: 0,
            program_counter: registers.program_counter,
            frame,
        })
    }

    pub fn devices(&self) -> &DeviceRuntime {
        &self.devices
    }
    pub fn devices_mut(&mut self) -> &mut DeviceRuntime {
        &mut self.devices
    }
    pub fn registers(&self) -> &Registers {
        &self.registers
    }
    pub fn capture_state(&self) -> PeripheralState {
        self.devices.capture_state(self.registers)
    }
    pub fn accepted_interrupts(&self) -> u64 {
        self.accepted_interrupts
    }

    /// Invoke an already-prepared MCC68K entry from a captured state. No JSR,
    /// invented return PC, emulator fallback, or arbitrary instruction charge.
    pub fn enter(&mut self, root: FunctionId, arguments: &[u32]) -> Result<EntryOutcome> {
        self.program_counter = root.metadata().address(self.revision());
        self.registers.program_counter = self.program_counter;
        self.devices.set_instruction_pc(self.program_counter);
        self.frame = FrameState::new(self.active_sp());
        let outcome = exits::enter(self, root, arguments)?;
        if let EntryOutcome::Transfer(transfer) = outcome
            && matches!(transfer.exit, Exit::Exception { .. })
        {
            // Readable native has no admitted synchronous-entry capability.
            // Keep the real source save/frame; never run an interpreter,
            // finalize RTS, or pretend that a handler PC has been installed.
            return Err(Error::cleanup_after_transfer(
                transfer,
                Error::Platform(
                    "native readable synchronous exception entry is unsupported".into(),
                ),
            ));
        }
        Ok(outcome)
    }

    /// Convenience for ordinary RTS roots. Nonreturning events remain explicit.
    pub fn invoke(&mut self, root: FunctionId, arguments: &[u32]) -> Result<u32> {
        let entry_sp = self.active_sp();
        if entry_sp & 1 != 0 {
            return Err(FrameError::UnalignedStack(entry_sp).into());
        }
        match self.enter(root, arguments)? {
            EntryOutcome::Returned(_) => {
                CallSite::new(self.read_u32(entry_sp)).validate()?;
                Ok(self.data_register(0))
            }
            EntryOutcome::Transfer(transfer) => Err(Error::ControlTransfer(transfer)),
        }
    }

    fn active_sp(&self) -> u32 {
        if self.registers.status & 0x2000 != 0 {
            self.registers.supervisor_stack_pointer
        } else {
            self.registers.user_stack_pointer
        }
    }
    fn set_active_sp(&mut self, sp: u32) {
        if self.registers.status & 0x2000 != 0 {
            self.registers.supervisor_stack_pointer = sp;
        } else {
            self.registers.user_stack_pointer = sp;
        }
        self.registers.address[7] = sp;
    }

    /// Readable-layer interrupt checkpoint. Exact instruction-boundary admission
    /// is owned by the compiled runtime, which consumes the same device lines.
    pub fn service_interrupt(&mut self) -> Result<bool> {
        let level = self.devices.pending_interrupt();
        if level == 0 || u16::from(level) <= (self.registers.status >> 8) & 7 {
            return Ok(false);
        }
        let root = match level {
            6 => self.handlers.vblank,
            4 => self.handlers.line,
            _ => None,
        }
        .ok_or_else(|| Error::Platform(format!("no native translated handler for IRQ{level}")))?;
        let parent_frame = self.frame;
        let status = self.registers.status;
        let pc = self.program_counter;
        let sp = self.registers.supervisor_stack_pointer.wrapping_sub(6);
        self.registers.supervisor_stack_pointer = sp;
        self.set_status_register((status & 0x78ff) | 0x2000 | (u16::from(level) << 8));
        self.write_u16(sp, status);
        self.write_u32(sp.wrapping_add(2), pc);
        self.devices.acknowledge_interrupt();
        match self.enter(root, &[])? {
            EntryOutcome::Transfer(transfer)
                if transfer.entry == root && transfer.exit == Exit::ExceptionReturn =>
            {
                let frame = exits::restore_exception(self)?;
                self.program_counter = frame.program_counter;
                self.registers.program_counter = frame.program_counter;
                self.devices.set_instruction_pc(frame.program_counter);
                self.frame = parent_frame;
                self.accepted_interrupts += 1;
                Ok(true)
            }
            EntryOutcome::Transfer(transfer) => Err(Error::ControlTransfer(transfer)),
            EntryOutcome::Returned(_) => Err(Error::Platform(
                "native IRQ entry must use its reviewed exception-return contract".into(),
            )),
        }
    }

    fn suspend_ram_or_yield(&mut self, request: Option<WaitRequest>) -> Result<()> {
        let max_steps = request.map_or(WaitRequest::DEFAULT_MAX_TICKS, |request| request.max_ticks);
        if let Some(WaitRequest {
            condition:
                WaitCondition::Ram {
                    address,
                    width,
                    mask,
                    value,
                    ..
                },
            ..
        }) = request
            && !width.valid(address, mask, value)
        {
            return Err(Error::Platform(
                "RAM wait has an invalid address, width or operand bits".into(),
            ));
        }
        let before_clock = self.devices.clock();
        let mut steps = 0;
        loop {
            let wait = self.devices.wait_pending();
            let halted = self.devices.cpu_halted();
            let level = self.devices.pending_interrupt();
            let irq = level != 0 && u16::from(level) > ((self.registers.status >> 8) & 7);
            if !wait && !halted && !irq {
                let done = match request {
                    Some(request) => self.condition(request.condition, self.accepted_interrupts)?,
                    None => self.devices.clock() > before_clock,
                };
                if done {
                    return Ok(());
                }
            }
            if steps >= max_steps {
                return Err(Error::Platform(format!(
                    "native {} exhausted {max_steps} scheduler steps",
                    if request.is_some() {
                        "RAM wait"
                    } else {
                        "poll_yield"
                    },
                )));
            }
            if !wait && !halted && irq {
                self.service_interrupt()?;
            } else if wait {
                self.devices
                    .take_wait_step(self.program_counter)
                    .expect("observed actual native wait debt");
            } else {
                self.devices.advance_master_clocks(15);
            }
            steps += 1;
        }
    }

    fn condition(&mut self, condition: WaitCondition, interrupts: u64) -> Result<bool> {
        Ok(match condition {
            WaitCondition::RamWordChanged { address, previous } => {
                self.read_u16(address) != previous
            }
            WaitCondition::Ram {
                address,
                width,
                mask,
                value,
                comparison,
            } => {
                if !width.valid(address, mask, value) {
                    return Err(Error::Platform(
                        "RAM wait has an invalid address, width or operand bits".into(),
                    ));
                }
                let actual = match width {
                    RamWidth::Byte => u32::from(self.devices.read_bus_byte(address)),
                    RamWidth::Word => u32::from(self.devices.read_bus_word(address)),
                    RamWidth::Long => {
                        let high = self.devices.read_bus_word(address);
                        let low = self.devices.read_bus_word(address.wrapping_add(2));
                        (u32::from(high) << 16) | u32::from(low)
                    }
                };
                comparison.satisfied(actual, mask, value)
            }
            WaitCondition::DeviceWord {
                address,
                mask,
                value,
            } => self.read_device_u16(address) & mask == value,
            WaitCondition::DeviceByte {
                address,
                mask,
                value,
            } => self.read_device_u8(address) & mask == value,
            WaitCondition::Interrupt => self.accepted_interrupts != interrupts,
            WaitCondition::VdpFillComplete => {
                let vdp = self.devices.vdp();
                if vdp.hybrid_registers().dma_mode
                    != genesis_components::vdp::registers::DmaMode::VramFill
                {
                    return Err(Error::Platform(
                        "a non-observing fill join requires VRAM-fill DMA mode".into(),
                    ));
                }
                !vdp.timing_dma_active()
            }
        })
    }
}

impl Bus for NativePlatform {
    fn read_u8(&self, address: u32) -> u8 {
        self.devices.memory().read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.devices.memory_mut().write_u8(address, value);
    }
}
impl tje_mem::Platform for NativePlatform {
    fn write_vdp_control(&mut self, value: u16) {
        self.write_device_u16(0xc00004, value);
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.write_device_u16(0xc00000, value);
    }
}
impl Platform for NativePlatform {
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.devices.discard_bus_byte(address);
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.devices.discard_bus_word(address);
        Ok(())
    }
    fn read_device_u8(&mut self, address: u32) -> u8 {
        self.devices.read_bus_byte(address)
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        self.devices.read_bus_word(address)
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.devices.write_immediate_byte(address, value);
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.devices.write_immediate_word(address, value);
    }
    fn poll_yield(&mut self) -> Result<()> {
        if self.class != ExecutionClass::TimeAware {
            return Err(Error::Platform(
                "scheduler yield requires TimeAware execution".into(),
            ));
        }
        self.suspend_ram_or_yield(None)
    }
    fn wait_until(&mut self, request: WaitRequest) -> Result<()> {
        if matches!(request.condition, WaitCondition::Ram { .. }) {
            if self.class != ExecutionClass::TimeAware {
                return Err(Error::Platform(
                    "RAM polling wait requires TimeAware execution".into(),
                ));
            }
            return self.suspend_ram_or_yield(Some(request));
        }
        let interrupts = self.accepted_interrupts;
        if self.condition(request.condition, interrupts)? {
            return Ok(());
        }
        for _ in 0..request.max_ticks {
            self.devices.advance_master_clocks(15);
            self.service_interrupt()?;
            if self.condition(request.condition, interrupts)? {
                return Ok(());
            }
        }
        Err(Error::Platform(format!(
            "native wait exhausted {} device steps: {:?}",
            request.max_ticks, request.condition
        )))
    }
}
impl Context for NativePlatform {
    fn revision(&self) -> Revision {
        self.devices.memory().revision()
    }
    fn data_register(&self, index: usize) -> u32 {
        self.registers.data[index]
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.registers.data[index] = value;
    }
    fn address_register(&self, index: usize) -> u32 {
        if index == 7 {
            self.active_sp()
        } else {
            self.registers.address[index]
        }
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        if index == 7 {
            self.set_active_sp(value);
        } else {
            self.registers.address[index] = value;
        }
    }
    fn status_register(&self) -> u16 {
        self.registers.status
    }
    fn set_status_register(&mut self, value: u16) {
        self.registers.status = value & 0xa71f;
        self.registers.address[7] = self.active_sp();
    }
    fn stack_pointer(&self) -> Result<u32> {
        Ok(self.active_sp())
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.set_active_sp(value);
        Ok(())
    }
    fn frame_state(&self) -> Option<FrameState> {
        Some(self.frame)
    }
    fn set_frame_state(&mut self, frame: FrameState) -> Result<()> {
        self.frame = frame;
        Ok(())
    }
    fn stack_banks(&self) -> Result<StackBanks> {
        Ok(StackBanks {
            user: self.registers.user_stack_pointer,
            supervisor: self.registers.supervisor_stack_pointer,
        })
    }
    fn set_stack_banks(&mut self, banks: StackBanks) -> Result<()> {
        self.registers.user_stack_pointer = banks.user;
        self.registers.supervisor_stack_pointer = banks.supervisor;
        self.registers.address[7] = self.active_sp();
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        self.dispatcher
    }
    fn execution_class(&self) -> ExecutionClass {
        self.class
    }
    fn set_execution_class(&mut self, class: ExecutionClass) -> Result<()> {
        self.class = class;
        Ok(())
    }
    fn call_original(&mut self, function: FunctionId, _: &[u32]) -> Result<u32> {
        Err(Error::Untranslated(function))
    }
}

#[cfg(test)]
mod discard_tests {
    use super::*;

    #[test]
    fn readable_native_discard_keeps_bus_effects_without_register_operands() {
        let mut rom = vec![0; 256];
        rom[..4].copy_from_slice(&0xff7ff0_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x40_u32.to_be_bytes());
        rom[0x40..0x44].copy_from_slice(&[0x12, 0x34, 0xab, 0xcd]);
        let mut memory = AddressSpace::new(rom, Revision::Rev02);
        memory.write_u16(0xff1234, 0x5678);
        let mut platform = NativePlatform::new(
            memory,
            &[],
            InterruptHandlers::default(),
            &GenesisEmulatorConfig::default(),
        )
        .unwrap();
        let before = platform.registers;
        platform.devices.enable_trace();
        platform.discard_u16(0xff1234).unwrap();
        assert_eq!(platform.capture_state().open_bus, 0x5678);
        platform.discard_u8(0x43).unwrap();
        assert_eq!(platform.capture_state().open_bus, 0xcd78);
        platform.write_device_u16(0xc00004, 0x4000);
        assert!(
            platform
                .devices
                .vdp()
                .hybrid_checkpoint()
                .control_word_pending
        );
        platform.discard_u16(0xc00004).unwrap();
        assert!(
            !platform
                .devices
                .vdp()
                .hybrid_checkpoint()
                .control_word_pending
        );
        assert_eq!(platform.registers, before);
        let accesses = &platform.devices.trace().unwrap().accesses;
        assert_eq!(accesses.len(), 2);
        assert_eq!(accesses[1].purpose, Some(2));
        assert_eq!((accesses[1].address, accesses[1].word), (0xc00004, true));
    }

    #[test]
    fn native_ram_predicates_and_yield_keep_real_width_and_clock_effects() {
        let mut rom = vec![0; 256];
        rom[..4].copy_from_slice(&0xff7ff0_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x40_u32.to_be_bytes());
        let mut memory = AddressSpace::new(rom, Revision::Rev02);
        memory.write_u32(0xff1234, 0x56789abc);
        let mut platform = NativePlatform::new(
            memory,
            &[],
            InterruptHandlers::default(),
            &GenesisEmulatorConfig::default(),
        )
        .unwrap();
        platform
            .set_execution_class(ExecutionClass::TimeAware)
            .unwrap();
        platform
            .wait_until(WaitRequest::ram_byte_equal(0xff1235, 255, 0x78))
            .unwrap();
        assert_eq!(platform.capture_state().open_bus, 0x7800);
        platform
            .wait_until(WaitRequest::ram_word_equal(0xff1234, 65535, 0x5678))
            .unwrap();
        assert_eq!(platform.capture_state().open_bus, 0x5678);
        platform
            .wait_until(WaitRequest::ram_long_equal(0xff1234, u32::MAX, 0x56789abc))
            .unwrap();
        let before = platform.capture_state();
        assert_eq!(before.open_bus, 0x9abc);
        platform.poll_yield().unwrap();
        let after = platform.capture_state();
        assert!(after.cycles.m68k_mclk_cycles > before.cycles.m68k_mclk_cycles);
        assert_eq!(
            after.open_bus, before.open_bus,
            "yield adds no condition operand"
        );
        let mut request = WaitRequest::ram_byte_not_equal(0xff1235, 255, 0x78);
        request.max_ticks = 2;
        assert!(platform.wait_until(request).is_err());
    }
}

#[cfg(test)]
mod ram_readiness_tests {
    use super::*;

    fn platform() -> NativePlatform {
        let mut rom = vec![0; 256];
        rom[..4].copy_from_slice(&0xff7ff0_u32.to_be_bytes());
        rom[4..8].copy_from_slice(&0x40_u32.to_be_bytes());
        rom[0x40..0x42].copy_from_slice(&0x1234_u16.to_be_bytes());
        rom[0x42..0x46].copy_from_slice(&[0x4e, 0x71, 0x4e, 0x71]);
        let memory = AddressSpace::new(rom, Revision::Rev02);
        let mut platform = NativePlatform::new(
            memory,
            &[],
            InterruptHandlers::default(),
            &GenesisEmulatorConfig::default(),
        )
        .unwrap();
        platform
            .set_execution_class(ExecutionClass::TimeAware)
            .unwrap();
        platform
    }

    #[test]
    fn already_owed_real_irq_precedes_the_first_ready_ram_read() {
        let mut platform = platform();
        platform.set_status_register(0x2000);
        platform.devices.read_bus_word(0x40);
        platform.write_device_u16(0xc00004, 0x8010);
        platform.write_device_u16(0xc00004, 0x8a00);
        // Execute one authored NOP continuation through the production
        // instruction boundary, including its prefetch. Device-only waits do
        // not sample IRQ-enable latches; never set those latches by hand.
        platform.devices.begin_instruction(0x4e71, 0);
        platform
            .devices
            .read_bus_word_tagged(0x44, crate::BusReadPurpose::Prefetch);
        platform.devices.finish_instruction(0x42, 4);
        platform.program_counter = 0x44;
        platform.registers.program_counter = 0x44;
        for _ in 0..5000 {
            if platform.devices.pending_interrupt() == 4 {
                break;
            }
            platform.devices.advance_master_clocks(15);
        }
        assert_eq!(platform.devices.pending_interrupt(), 4);
        let before = platform.capture_state();
        assert_ne!(before.open_bus, 0);
        let result = platform.wait_until(WaitRequest::ram_byte_equal(0xff1234, 255, 0));
        assert!(matches!(result, Err(Error::Platform(message))
            if message.contains("no native translated handler for IRQ4")));
        assert_eq!(
            platform.capture_state().open_bus,
            before.open_bus,
            "no ready RAM operand may be read ahead of the owed IRQ"
        );
        assert_eq!(platform.devices.pending_interrupt(), 4);
    }

    #[test]
    fn active_real_bus_halt_must_complete_or_exhaust_before_ready_ram_read() {
        let mut platform = platform();
        platform.set_status_register(0x2700);
        platform.devices.read_bus_word(0x40);
        for address in (0xff9000..0xff9080).step_by(2) {
            platform.devices.memory_mut().write_u16(address, 0x5aa5);
        }
        for value in [
            0x8114, 0x8f02, 0x9340, 0x9400, 0x9500, 0x96c8, 0x977f, 0x4000, 0x0080,
        ] {
            platform.write_device_u16(0xc00004, value);
        }
        for _ in 0..100 {
            if platform.devices.cpu_halted() {
                break;
            }
            platform.devices.advance_master_clocks(15);
        }
        assert!(
            platform.devices.cpu_halted(),
            "fixture must start a real halting DMA"
        );
        let before = platform.capture_state();
        let mut request = WaitRequest::ram_byte_equal(0xff1234, 255, 0);
        request.max_ticks = 0;
        assert!(platform.wait_until(request).is_err());
        assert_eq!(
            platform.capture_state().open_bus,
            before.open_bus,
            "exhaustion must not poll through an active bus halt"
        );
        assert!(platform.devices.cpu_halted());
        request.max_ticks = 10000;
        platform.wait_until(request).unwrap();
        assert!(!platform.devices.cpu_halted());
        assert!(!platform.devices.wait_pending());
        assert!(platform.devices.clock() > before.cycles.m68k_mclk_cycles);
    }
}

#[cfg(test)]
mod synchronous_trap_tests {
    use super::*;
    use tje_game::ids;

    #[test]
    fn unsupported_readable_trap_preserves_real_save_and_never_finishes_rts() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            for entry in [ids::LMODT, ids::LDIVT, ids::ULMODT, ids::ULDIVT] {
                let mut rom = vec![0; 256];
                rom[..4].copy_from_slice(&0xff7000u32.to_be_bytes());
                rom[4..8].copy_from_slice(&0x40u32.to_be_bytes());
                let mut platform = NativePlatform::new(
                    AddressSpace::new(rom, revision),
                    &[entry],
                    InterruptHandlers::default(),
                    &GenesisEmulatorConfig::default(),
                )
                .unwrap();
                platform.set_data_register(0, 7);
                platform.set_data_register(1, 0);
                platform.set_data_register(2, 0x89abcdef);
                let clock = platform.devices.clock();
                let error = platform.enter(entry, &[]).unwrap_err();
                assert!(error.preserves_call_frame());
                assert_eq!(platform.active_sp(), 0xff6ffc);
                assert_eq!(platform.read_u32(0xff6ffc), 0x89abcdef);
                assert_eq!(platform.data_register(2), 0);
                assert_eq!(platform.program_counter, entry.metadata().address(revision));
                assert_eq!(platform.devices.clock(), clock);
                assert_eq!(platform.accepted_interrupts, 0);
            }
        }
    }
}
