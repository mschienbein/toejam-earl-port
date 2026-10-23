use crate::{Context, Dispatcher, FunctionId, Result};
use tje_mem::{AddressSpace, Bus, Platform, Revision};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Write {
    Ram(u32, u8),
    Control(u16),
    Data(u16),
}

pub(crate) struct TestContext {
    pub memory: AddressSpace,
    pub writes: Vec<Write>,
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
            data: [0; 8],
            address: [0; 8],
            status: 0x2000,
        }
    }
}

impl Bus for TestContext {
    fn read_u8(&self, address: u32) -> u8 {
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.writes.push(Write::Ram(address, value));
        self.memory.write_u8(address, value);
    }
}
impl Platform for TestContext {
    fn write_vdp_control(&mut self, value: u16) {
        self.writes.push(Write::Control(value));
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.writes.push(Write::Data(value));
    }
}
impl crate::Platform for TestContext {
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
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid()
    }
    fn call_original(&mut self, function: FunctionId, _: &[u32]) -> Result<u32> {
        panic!(
            "unexpected original leaf callee {}",
            function.metadata().name
        )
    }
}
