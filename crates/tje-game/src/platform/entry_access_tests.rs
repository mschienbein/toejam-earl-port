//! Synthetic source-access tests, not replay/control/timing proof of the cartridge.
use super::*;
use crate::{Dispatcher, Error, Platform, exits::StackBanks};
use std::cell::RefCell;
use tje_mem::{AddressSpace, Bus, Revision};

#[derive(Debug, PartialEq, Eq)]
enum Access {
    MemoryWord(u32, u16),
    MemoryByte(u32, u8),
    DeviceWord(u32, u16),
    DeviceByte(u32, u8),
    WriteWord(u32, u16),
    WriteByte(u32, u8),
    Wait(WaitRequest),
}

struct AccessContext {
    memory: AddressSpace,
    trace: RefCell<Vec<Access>>,
    data: [u32; 8],
    address: [u32; 7],
    banks: StackBanks,
    sr: u16,
    warm_ab: u16,
    warm_c: u16,
    bus_busy: bool,
}

impl AccessContext {
    fn new(revision: Revision) -> Self {
        // Authored setup contents for a small unit: hardware addresses are not
        // cartridge data. No bytes are read/copied from either private ROM.
        let setup = rom_symbol(revision, "SetupValues").unwrap().address as usize;
        let mut bytes = vec![0; setup + 256];
        for (index, value) in [0x8000_u16, 3, 0x0100].into_iter().enumerate() {
            bytes[setup + index * 2..setup + index * 2 + 2].copy_from_slice(&value.to_be_bytes());
        }
        for (index, value) in [0xa00000_u32, 0xa11100, 0xa11200, 0xc00000, 0xc00004]
            .into_iter()
            .enumerate()
        {
            bytes[setup + 6 + index * 4..setup + 10 + index * 4]
                .copy_from_slice(&value.to_be_bytes());
        }
        for (index, byte) in bytes[setup + 26..].iter_mut().enumerate() {
            *byte = 0x40_u8.wrapping_add(index as u8);
        }
        Self {
            memory: AddressSpace::new(bytes, revision),
            trace: RefCell::new(Vec::new()),
            data: [0x1234_5678; 8],
            address: [0x1357_2468; 7],
            banks: StackBanks {
                user: 0xff7200,
                supervisor: 0xff7ffc,
            },
            sr: 0x271f,
            warm_ab: 0,
            warm_c: 0,
            bus_busy: true,
        }
    }
}

impl Bus for AccessContext {
    fn read_u8(&self, address: u32) -> u8 {
        let value = self.memory.read_u8(address);
        self.trace
            .borrow_mut()
            .push(Access::MemoryByte(address, value));
        value
    }
    fn read_u16(&self, address: u32) -> u16 {
        let value = self.memory.read_u16(address);
        self.trace
            .borrow_mut()
            .push(Access::MemoryWord(address, value));
        value
    }
    fn read_u32(&self, address: u32) -> u32 {
        (u32::from(self.read_u16(address)) << 16)
            | u32::from(self.read_u16(address.wrapping_add(2)))
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
        self.trace
            .borrow_mut()
            .push(Access::WriteByte(address, value));
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.memory.write_u16(address, value);
        self.trace
            .borrow_mut()
            .push(Access::WriteWord(address, value));
    }
}

impl tje_mem::Platform for AccessContext {
    fn write_vdp_control(&mut self, value: u16) {
        self.write_device_u16(0xc00004, value);
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.write_device_u16(0xc00000, value);
    }
    fn write_vdp_command(&mut self, value: u32) {
        self.write_device_u16(0xc00004, (value >> 16) as u16);
        self.write_device_u16(0xc00006, value as u16);
    }
}
impl Platform for AccessContext {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        let value = match address {
            0xa10001 => 0,
            0xa11100 => u8::from(self.bus_busy),
            _ => panic!("unexpected authored byte source {address:08x}"),
        };
        self.trace
            .borrow_mut()
            .push(Access::DeviceByte(address, value));
        value
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        let value = match address {
            0xa10008 => self.warm_ab,
            0xa1000a => 0,
            0xa1000c => self.warm_c,
            0xc00004 => 0,
            _ => panic!("unexpected authored word source {address:08x}"),
        };
        self.trace
            .borrow_mut()
            .push(Access::DeviceWord(address, value));
        value
    }
    fn read_device_u8_with_open_bus(&mut self, address: u32, source: OpenBusSource) -> Result<u8> {
        assert_eq!(
            source,
            OpenBusSource::prefetch(ids::START.metadata().address(self.revision()) + 0x4c)
        );
        Ok(self.read_device_u8(address))
    }
    fn read_device_u16_with_open_bus(
        &mut self,
        address: u32,
        source: OpenBusSource,
    ) -> Result<u16> {
        assert_eq!(
            source,
            OpenBusSource::prefetch(ids::START.metadata().address(self.revision()) + 0x30)
        );
        Ok(self.read_device_u16(address))
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.trace
            .borrow_mut()
            .push(Access::WriteByte(address, value));
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.trace
            .borrow_mut()
            .push(Access::WriteWord(address, value));
    }
    fn wait_until(&mut self, request: WaitRequest) -> Result<()> {
        self.trace.borrow_mut().push(Access::Wait(request));
        // Intentional unit boundary, not a production readiness implementation.
        Err(Error::Platform("authored wait boundary".into()))
    }
}
impl Context for AccessContext {
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
        if index == 7 {
            if self.sr & 0x2000 != 0 {
                self.banks.supervisor
            } else {
                self.banks.user
            }
        } else {
            self.address[index]
        }
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.address[index] = value;
    }
    fn status_register(&self) -> u16 {
        self.sr
    }
    fn set_status_register(&mut self, value: u16) {
        self.sr = value;
    }
    fn stack_banks(&self) -> Result<StackBanks> {
        Ok(self.banks)
    }
    fn set_stack_banks(&mut self, banks: StackBanks) -> Result<()> {
        self.banks = banks;
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid()
    }
    fn call_original(&mut self, function: crate::FunctionId, _: &[u32]) -> Result<u32> {
        Err(Error::Untranslated(function))
    }
}

#[test]
fn cold_start_wait_uses_the_original_byte_bit_zero_predicate() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = AccessContext::new(revision);
        let result = start(&mut context, &[]);
        assert!(
            matches!(&result, Err(Error::Platform(reason)) if reason == "authored wait boundary")
        );
        let trace = context.trace.borrow();
        let wait = trace
            .iter()
            .find_map(|access| {
                if let Access::Wait(request) = access {
                    Some(*request)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(
            wait.condition,
            WaitCondition::DeviceByte {
                address: 0xa11100,
                mask: 1,
                value: 0
            }
        );
        assert_eq!(wait.max_ticks, WaitRequest::DEFAULT_MAX_TICKS);
        assert!(matches!(
            trace[trace.len() - 2],
            Access::DeviceByte(0xa11100, 1)
        ));
        assert!(
            !trace
                .iter()
                .any(|access| matches!(access, Access::DeviceWord(0xa11100, _)))
        );
    }
}

#[test]
fn warm_skip_keeps_control_reads_in_order_and_does_not_touch_cold_state() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for first in [true, false] {
            let mut context = AccessContext::new(revision);
            if first {
                context.warm_ab = 0x8000;
            } else {
                context.warm_c = 1;
            }
            let data = context.data;
            let addresses = context.address;
            let banks = context.banks;
            let result = start(&mut context, &[]);
            let Err(Error::ControlTransfer(transfer)) = result else {
                panic!("warm startup must request its real tail boundary");
            };
            assert_eq!(transfer.entry, ids::START);
            assert_eq!(
                transfer.exit,
                exits::Exit::Tail {
                    target: ids::GAME_PROGRAM
                }
            );
            assert_eq!(context.data, data);
            assert_eq!(context.address, addresses);
            assert_eq!(context.banks, banks);
            assert_eq!(context.sr, if first { 0x2718 } else { 0x2710 });
            let expected = if first {
                vec![
                    Access::DeviceWord(0xa10008, 0x8000),
                    Access::DeviceWord(0xa1000a, 0),
                ]
            } else {
                vec![
                    Access::DeviceWord(0xa10008, 0),
                    Access::DeviceWord(0xa1000a, 0),
                    Access::DeviceWord(0xa1000c, 1),
                ]
            };
            assert_eq!(*context.trace.borrow(), expected);
        }
    }
}
