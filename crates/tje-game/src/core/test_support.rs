use std::cell::RefCell;

use crate::call_frame::{FrameError, FrameState};
use crate::execution::{RamWidth, WaitCondition, WaitRequest};
use crate::{Context, Dispatcher, FunctionId, Result};
use tje_mem::{AddressSpace, Bus, Platform, Revision};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Write {
    Ram(u32, u8),
    Control(u16),
    Data(u16),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Access {
    Read(u32, u8),
    Discard(u32, u8),
    Wait(WaitRequest),
    Write(u32, u8, u32),
    Control(u16),
    Data(u16),
}

pub(crate) struct TestContext {
    pub memory: AddressSpace,
    pub writes: Vec<Write>,
    pub trace_accesses: bool,
    pub accesses: RefCell<Vec<Access>>,
    pub frame: Option<FrameState>,
    pub sp: u32,
    pub stop_at_calls: bool,
    pub dispatcher_override: Option<Dispatcher>,
    pub execution: crate::execution::ExecutionClass,
    data: [u32; 8],
    address: [u32; 8],
    status: u16,
}

impl TestContext {
    pub fn new(revision: Revision) -> Self {
        Self::with_rom(revision, vec![0; 0x100000])
    }
    pub fn with_rom(revision: Revision, rom: Vec<u8>) -> Self {
        Self {
            memory: AddressSpace::new(rom, revision),
            writes: Vec::new(),
            trace_accesses: false,
            accesses: RefCell::new(Vec::new()),
            frame: None,
            sp: 0xff7f00,
            stop_at_calls: false,
            dispatcher_override: None,
            execution: crate::execution::ExecutionClass::Atomic,
            data: [0; 8],
            address: [0; 8],
            status: 0x2000,
        }
    }
    pub fn prepare_entry(&mut self, entry: FunctionId, arguments: &[u32]) -> FrameState {
        let frame = FrameState::new(self.sp).for_prepared_entry(entry);
        self.frame = Some(frame);
        for (index, &value) in arguments.iter().enumerate() {
            frame.argument_slot(index as i32).write(self, value);
        }
        frame
    }
    fn record(&self, access: Access) {
        if self.trace_accesses {
            self.accesses.borrow_mut().push(access);
        }
    }
}

impl Bus for TestContext {
    fn read_u8(&self, address: u32) -> u8 {
        self.record(Access::Read(address, 1));
        self.memory.read_u8(address)
    }
    fn read_u16(&self, address: u32) -> u16 {
        self.record(Access::Read(address, 2));
        self.memory.read_u16(address)
    }
    fn read_u32(&self, address: u32) -> u32 {
        (u32::from(self.read_u16(address)) << 16)
            | u32::from(self.read_u16(address.wrapping_add(2)))
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.record(Access::Write(address, 1, u32::from(value)));
        self.writes.push(Write::Ram(address, value));
        self.memory.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.record(Access::Write(address, 2, u32::from(value)));
        for (offset, byte) in value.to_be_bytes().into_iter().enumerate() {
            let address = address.wrapping_add(offset as u32);
            self.writes.push(Write::Ram(address, byte));
            self.memory.write_u8(address, byte);
        }
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.write_u16(address, (value >> 16) as u16);
        self.write_u16(address.wrapping_add(2), value as u16);
    }
}
impl Platform for TestContext {
    fn write_vdp_control(&mut self, value: u16) {
        self.record(Access::Control(value));
        self.writes.push(Write::Control(value));
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.record(Access::Data(value));
        self.writes.push(Write::Data(value));
    }
}
impl crate::Platform for TestContext {
    fn wait_until(&mut self, request: WaitRequest) -> Result<()> {
        self.record(Access::Wait(request));
        let WaitCondition::Ram {
            address,
            width,
            mask,
            value,
            comparison,
        } = request.condition
        else {
            return Err(crate::Error::Platform("unsupported unit wait".into()));
        };
        let actual = match width {
            RamWidth::Byte => u32::from(self.read_u8(address)),
            RamWidth::Word => u32::from(self.read_u16(address)),
            RamWidth::Long => self.read_u32(address),
        };
        if comparison.satisfied(actual, mask, value) {
            Ok(())
        } else {
            // Only a genuinely satisfying RAM sample can complete this
            // fixture's wait. It has no authority to advance emulated time.
            Err(crate::Error::Platform(
                "unit context cannot advance an unsatisfied RAM wait".into(),
            ))
        }
    }
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.record(Access::Discard(address, 1));
        let _ = self.memory.read_u8(address);
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.record(Access::Discard(address, 2));
        let _ = self.memory.read_u16(address);
        Ok(())
    }
    fn read_device_u8(&mut self, address: u32) -> u8 {
        panic!("unexpected leaf device read at {address:08x}")
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        panic!("unexpected leaf device read at {address:08x}")
    }
    fn write_device_u8(&mut self, address: u32, _: u8) {
        panic!("unexpected leaf device write at {address:08x}")
    }
    fn write_device_u16(&mut self, address: u32, _: u16) {
        panic!("unexpected leaf device write at {address:08x}")
    }
}
impl Context for TestContext {
    fn revision(&self) -> Revision {
        self.memory.revision()
    }
    fn data_register(&self, index: usize) -> u32 {
        self.data[index]
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.data[index] = value;
    }
    fn address_register(&self, index: usize) -> u32 {
        self.address[index]
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.address[index] = value;
    }
    fn status_register(&self) -> u16 {
        self.status
    }
    fn set_status_register(&mut self, value: u16) {
        self.status = value;
    }
    fn stack_pointer(&self) -> Result<u32> {
        self.frame.ok_or(FrameError::Unavailable)?;
        Ok(self.sp)
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.frame.ok_or(FrameError::Unavailable)?;
        self.sp = value;
        Ok(())
    }
    fn frame_state(&self) -> Option<FrameState> {
        self.frame
    }
    fn set_frame_state(&mut self, frame: FrameState) -> Result<()> {
        self.frame = Some(frame);
        Ok(())
    }
    fn execution_class(&self) -> crate::execution::ExecutionClass {
        self.execution
    }
    fn set_execution_class(&mut self, class: crate::execution::ExecutionClass) -> Result<()> {
        self.execution = class;
        Ok(())
    }
    fn enter_call_ceiling(&mut self, _: FunctionId) -> Result<crate::execution::CallCeilingScope> {
        // Explicit frozen unit-fixture provider, never a timing proof.
        Ok(crate::execution::CallCeilingScope::not_required())
    }
    fn dispatcher(&self) -> Dispatcher {
        if let Some(dispatcher) = self.dispatcher_override {
            return dispatcher;
        }
        if self.stop_at_calls {
            Dispatcher::hybrid_with(&[])
        } else {
            Dispatcher::hybrid()
        }
    }
    fn call_original(&mut self, function: FunctionId, _: &[u32]) -> Result<u32> {
        if self.stop_at_calls {
            return Err(crate::Error::Untranslated(function));
        }
        panic!(
            "unexpected original leaf callee {}",
            function.metadata().name
        )
    }
}
