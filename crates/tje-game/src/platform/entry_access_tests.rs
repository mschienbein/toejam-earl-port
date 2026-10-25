//! Synthetic source-access tests, not replay/control/timing proof of the cartridge.
use super::*;
use crate::{Dispatcher, Error, Platform, exits::StackBanks};
use std::cell::RefCell;
use tje_mem::{AddressSpace, Bus, Revision};

#[derive(Debug, PartialEq, Eq)]
enum Access {
    MemoryWord(u32, u16),
    MemoryByte(u32, u8),
    DiscardedWord(u32, u16),
    DiscardedByte(u32, u8),
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
    complete_fill: bool,
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
            complete_fill: false,
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
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        let value = self.memory.read_u8(address);
        self.trace
            .borrow_mut()
            .push(Access::DiscardedByte(address, value));
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        let value = self.memory.read_u16(address);
        self.trace
            .borrow_mut()
            .push(Access::DiscardedWord(address, value));
        Ok(())
    }
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
        if self.complete_fill && request.condition == WaitCondition::VdpFillComplete {
            return Ok(());
        }
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

#[test]
fn manual_startup_movem_reads_keep_terminal_word_purposes_and_order() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = AccessContext::new(revision);
        context.bus_busy = false;
        context.complete_fill = true;
        let setup = rom_symbol(revision, "SetupValues").unwrap().address;
        let expected_supervisor = context.banks.supervisor;
        let result = start(&mut context, &[]);
        let Err(Error::ControlTransfer(transfer)) = result else {
            panic!("complete authored cold input must request GameProgram tail");
        };
        assert_eq!(transfer.entry, ids::START);
        assert_eq!(
            transfer.exit,
            exits::Exit::Tail {
                target: ids::GAME_PROGRAM
            }
        );
        assert_eq!(context.data, [0; 8]);
        assert_eq!(context.address, [0; 7]);
        assert_eq!(context.sr, 0x2700);
        assert_eq!(context.banks.user, 0);
        assert_eq!(context.banks.supervisor, expected_supervisor);
        let trace = context.trace.borrow();
        let first = trace
            .iter()
            .position(|access| *access == Access::MemoryWord(setup, 0x8000))
            .unwrap();
        assert_eq!(
            &trace[first..first + 6],
            &[
                Access::MemoryWord(setup, 0x8000),
                Access::MemoryWord(setup + 2, 3),
                Access::MemoryWord(setup + 4, 0x0100),
                Access::DiscardedWord(setup + 6, 0x00a0),
                Access::MemoryWord(setup + 6, 0x00a0),
                Access::MemoryWord(setup + 8, 0),
            ]
        );
        assert_eq!(trace[first + 14], Access::DiscardedWord(setup + 26, 0x4041));
        assert_eq!(trace[first + 15], Access::DeviceByte(0xa10001, 0));
        // The authored setup uses D6=3: four backwards clear longs. A6's
        // original non-postincrement MOVEM EA is FFFFFFF0, terminal EA=2C.
        let terminal = 0_u32.wrapping_sub(16).wrapping_add(60);
        let discards: Vec<_> = trace
            .iter()
            .filter_map(|access| {
                if let Access::DiscardedWord(address, _) = access {
                    Some(*address)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(discards, [setup + 6, setup + 26, terminal]);
        assert_eq!(trace.last(), Some(&Access::DiscardedWord(terminal, 0)));
        let end = trace.len() - 1;
        for word in 0..30_u32 {
            assert_eq!(
                trace[end - 30 + word as usize],
                Access::MemoryWord(0_u32.wrapping_sub(16).wrapping_add(word * 2), 0)
            );
        }
    }
}

#[test]
fn owned_context_discard_operations_never_record_semantic_or_split_word_reads() {
    let mut context = AccessContext::new(Revision::Rev02);
    let setup = rom_symbol(Revision::Rev02, "SetupValues").unwrap().address;
    context.discard_u8(setup).unwrap();
    context.discard_u16(setup).unwrap();
    assert_eq!(
        *context.trace.borrow(),
        [
            Access::DiscardedByte(setup, 0x80),
            Access::DiscardedWord(setup, 0x8000),
        ]
    );
}
