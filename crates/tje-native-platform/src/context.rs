//! Readable translation adapter and explicit native-root closure validation.
//! This adapter is for subsystem differential proofs. Cycle-exact shipped logic
//! uses `ClockedDevices` and generated instruction continuations instead.

use crate::{ClockedDevices, DeviceRuntime, Registers};
use genesis_config::GenesisEmulatorConfig;
use genesis_core::native::PeripheralState;
use tje_game::call_frame::{CallSite, FrameError, FrameState};
use tje_game::{
    Context, Dispatcher, Error, FunctionId, Platform, Result,
    execution::{ExecutionClass, WaitCondition, WaitRequest},
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
        exits::enter(self, root, arguments)
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

    fn condition(&mut self, condition: WaitCondition, interrupts: u64) -> bool {
        match condition {
            WaitCondition::RamWordChanged { address, previous } => {
                self.read_u16(address) != previous
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
        }
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
    fn wait_until(&mut self, request: WaitRequest) -> Result<()> {
        let interrupts = self.accepted_interrupts;
        if self.condition(request.condition, interrupts) {
            return Ok(());
        }
        for _ in 0..request.max_ticks {
            self.devices.advance_master_clocks(15);
            self.service_interrupt()?;
            if self.condition(request.condition, interrupts) {
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
