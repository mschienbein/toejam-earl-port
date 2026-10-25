use crate::{
    Context, Dispatcher, FunctionId, Result, call_frame::FrameState, execution::ExecutionClass,
};
use std::{cell::RefCell, collections::VecDeque};
use tje_mem::{AddressSpace, Bus, Revision};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Access {
    Read(u32, u8, u32),
    Discard(u32, u8),
    Write(u32, u8, u32),
    Call(FunctionId, u32),
    Yield,
}
struct ExpectedCall {
    function: FunctionId,
    return_offset: u32,
    arguments: Vec<u32>,
    result: u32,
}
pub(super) struct Fixture {
    pub memory: AddressSpace,
    pub accesses: RefCell<Vec<Access>>,
    pub data: [u32; 8],
    pub addresses: [u32; 7],
    pub status: u16,
    pub sp: u32,
    pub frame: FrameState,
    pub calls: Vec<(FunctionId, u32)>,
    pub yield_writes: VecDeque<Vec<(u32, u8)>>,
    root: FunctionId,
    expected: VecDeque<ExpectedCall>,
}
impl Fixture {
    pub fn new(revision: Revision, root: FunctionId, arguments: &[u32]) -> Self {
        let mut memory = AddressSpace::new(vec![0; 0x100000], revision);
        for (index, &argument) in arguments.iter().enumerate() {
            memory.write_u32(0xff8004 + index as u32 * 4, argument);
        }
        Self {
            memory,
            accesses: RefCell::default(),
            data: std::array::from_fn(|index| 0x76540000 | index as u32),
            addresses: std::array::from_fn(|index| 0xff1000 + index as u32 * 4),
            status: 0x2013,
            sp: 0xff8000,
            frame: FrameState::new(0xff8000),
            calls: Vec::new(),
            yield_writes: VecDeque::new(),
            root,
            expected: VecDeque::new(),
        }
    }
    pub fn expect(
        &mut self,
        function: FunctionId,
        return_offset: u32,
        arguments: &[u32],
        result: u32,
    ) {
        self.expected.push_back(ExpectedCall {
            function,
            return_offset,
            arguments: arguments.to_vec(),
            result,
        });
    }
    pub fn complete(&self) {
        assert!(self.expected.is_empty(), "unconsumed expected calls");
        assert_eq!(self.sp, 0xff8000, "source caller SP not balanced");
    }
    pub fn calls_complete(&self) {
        assert!(self.expected.is_empty(), "unconsumed expected calls");
    }
}
impl Bus for Fixture {
    fn read_u8(&self, address: u32) -> u8 {
        let value = self.memory.read_u8(address);
        self.accesses
            .borrow_mut()
            .push(Access::Read(address, 1, u32::from(value)));
        value
    }
    fn read_u16(&self, address: u32) -> u16 {
        let value = self.memory.read_u16(address);
        self.accesses
            .borrow_mut()
            .push(Access::Read(address, 2, u32::from(value)));
        value
    }
    fn read_u32(&self, address: u32) -> u32 {
        let value = self.memory.read_u32(address);
        self.accesses
            .borrow_mut()
            .push(Access::Read(address, 4, value));
        value
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.accesses
            .get_mut()
            .push(Access::Write(address, 1, u32::from(value)));
        self.memory.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.accesses
            .get_mut()
            .push(Access::Write(address, 2, u32::from(value)));
        self.memory.write_u16(address, value);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.accesses
            .get_mut()
            .push(Access::Write(address, 4, value));
        self.memory.write_u32(address, value);
    }
}
impl tje_mem::Platform for Fixture {
    fn write_vdp_control(&mut self, value: u16) {
        self.accesses
            .get_mut()
            .push(Access::Write(0xc00004, 2, u32::from(value)));
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.accesses
            .get_mut()
            .push(Access::Write(0xc00000, 2, u32::from(value)));
    }
}
impl crate::Platform for Fixture {
    fn poll_yield(&mut self) -> Result<()> {
        self.accesses.get_mut().push(Access::Yield);
        if let Some(writes) = self.yield_writes.pop_front() {
            for (address, value) in writes {
                self.memory.write_u8(address, value);
            }
        }
        Ok(())
    }
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.accesses.get_mut().push(Access::Discard(address, 1));
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.accesses.get_mut().push(Access::Discard(address, 2));
        Ok(())
    }
    fn read_device_u8(&mut self, address: u32) -> u8 {
        panic!("unexpected device read {address:06x}")
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        panic!("unexpected device read {address:06x}")
    }
    fn write_device_u8(&mut self, address: u32, _: u8) {
        panic!("unexpected device write {address:06x}")
    }
    fn write_device_u16(&mut self, address: u32, _: u16) {
        panic!("unexpected device write {address:06x}")
    }
}
impl Context for Fixture {
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
        self.addresses[index]
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.addresses[index] = value;
    }
    fn status_register(&self) -> u16 {
        self.status
    }
    fn set_status_register(&mut self, value: u16) {
        self.status = value;
    }
    fn stack_pointer(&self) -> Result<u32> {
        Ok(self.sp)
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.sp = value;
        Ok(())
    }
    fn frame_state(&self) -> Option<FrameState> {
        Some(self.frame)
    }
    fn set_frame_state(&mut self, value: FrameState) -> Result<()> {
        self.frame = value;
        Ok(())
    }
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::TimeAware
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with(&[])
    }
    fn call_original(&mut self, function: FunctionId, _: &[u32]) -> Result<u32> {
        panic!("unexpected unprepared call {}", function.metadata().name)
    }
    fn call_original_entry(&mut self, function: FunctionId, _: ExecutionClass) -> Result<u32> {
        let expected = self.expected.pop_front().expect("unexpected callee");
        assert_eq!(function, expected.function);
        let pc = self.memory.read_u32(self.sp);
        assert_eq!(
            pc,
            self.root.metadata().address(self.revision()) + expected.return_offset
        );
        for (index, expected) in expected.arguments.iter().enumerate() {
            assert_eq!(
                self.memory.read_u32(self.sp + 4 + index as u32 * 4),
                *expected,
                "{} argument {index}",
                function.metadata().name
            );
        }
        self.calls.push((function, self.sp));
        self.accesses
            .get_mut()
            .push(Access::Call(function, expected.result));
        self.data[0] = expected.result;
        self.status = 0x2013;
        self.sp += 4;
        Ok(expected.result)
    }
}
