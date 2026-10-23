use super::*;
use crate::{Dispatcher, FunctionId, Platform};
use tje_mem::{AddressSpace, Bus, Revision};

#[derive(Debug, PartialEq, Eq)]
enum Access {
    ReadByte(u32, u8),
    WriteByte(u32, u8),
    WriteWord(u32, u16),
    RamByte(u32, u8),
}

struct TestContext {
    memory: AddressSpace,
    data: [u32; 8],
    addresses: [u32; 8],
    sr: u16,
    pad_data: [u8; 2],
    pad_low: u8,
    pad_high: u8,
    delay: u8,
    psg: u8,
    trace: Vec<Access>,
}

impl TestContext {
    fn new(revision: Revision) -> Self {
        Self {
            memory: AddressSpace::new(vec![], revision),
            data: [
                0x1234_5678,
                0x89ab_cdef,
                0x2233_4455,
                0x3344_5566,
                0x4455_6677,
                0x5566_7788,
                0x6677_8899,
                0x7788_99aa,
            ],
            addresses: [
                0x0123_4567,
                0x1234_5678,
                0x2345_6789,
                0x3456_789a,
                0x4567_89ab,
                0x5678_9abc,
                0x6789_abcd,
                0x789a_bcde,
            ],
            sr: 0x271f,
            pad_data: [0x40; 2],
            pad_low: 0,
            pad_high: 0,
            delay: 0,
            psg: 0,
            trace: Vec::new(),
        }
    }
}

impl Bus for TestContext {
    fn read_u8(&self, address: u32) -> u8 {
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.trace.push(Access::RamByte(address, value));
        self.memory.write_u8(address, value);
    }
}

impl tje_mem::Platform for TestContext {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("controller/interrupt routine must not write the VDP");
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("controller/interrupt routine must not write the VDP");
    }
}

impl Platform for TestContext {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        let value = match address {
            0xa10003 | 0xa10005 => {
                let second = usize::from(address == 0xa10005);
                if self.pad_data[second] & 0x40 == 0 {
                    self.pad_low
                } else {
                    self.pad_high
                }
            }
            0xa0004a => self.delay,
            0xa0004b => self.psg,
            _ => panic!("unexpected device read {address:08x}"),
        };
        self.trace.push(Access::ReadByte(address, value));
        value
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        panic!("unexpected device word read {address:08x}");
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        match address {
            0xa10003 | 0xa10005 => {
                self.pad_data[usize::from(address == 0xa10005)] = value;
            }
            0xa10009 | 0xa1000b => assert_eq!(value, 0x40),
            0xa0004a => self.delay = value,
            _ => panic!("unexpected device byte write {address:08x}"),
        }
        self.trace.push(Access::WriteByte(address, value));
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        assert_eq!(address, 0xa11100);
        assert!(value == 0 || value == 0x0100);
        self.trace.push(Access::WriteWord(address, value));
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
        self.addresses[index]
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.addresses[index] = value;
    }
    fn status_register(&self) -> u16 {
        self.sr
    }
    fn set_status_register(&mut self, value: u16) {
        self.sr = value;
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid()
    }
    fn call_original(&mut self, function: FunctionId, _: &[u32]) -> Result<u32> {
        panic!(
            "platform leaf unexpectedly called {}",
            function.metadata().name
        );
    }
}

#[test]
fn interrupt_masks_preserve_all_registers_and_every_ccr_combination() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision);
        let data = context.data;
        let addresses = context.addresses;
        for control in [0, 0x2000, 0x8000, 0xa000] {
            for priority in 0..8 {
                for ccr in 0..32 {
                    let entry_sr = control | (priority << 8) | ccr;
                    context.sr = entry_sr;
                    assert_eq!(disable_all_interrupts(&mut context, &[]).unwrap(), None);
                    assert_eq!(context.sr, entry_sr | 0x0700);
                    assert_eq!(context.data, data);
                    assert_eq!(context.addresses, addresses);
                    context.sr = entry_sr;
                    assert_eq!(enable_all_interrupts(&mut context, &[]).unwrap(), None);
                    assert_eq!(context.sr, entry_sr & 0xf8ff);
                    assert_eq!(context.data, data);
                    assert_eq!(context.addresses, addresses);
                }
            }
        }
        assert!(context.trace.is_empty());
    }
}

#[test]
fn pads_preserve_full_assembly_contract_and_ordered_sound_coupling() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for second in [false, true] {
            let pad_address = if second { 0xa10005 } else { 0xa10003 };
            let control_address = if second { 0xa1000b } else { 0xa10009 };
            let mut context = TestContext::new(revision);
            let status = ram::g_psg_effect_status(revision).address();
            for input in 0..=255_u8 {
                for delay in [0_u8, 1, 2, 3, 127, 128, 255] {
                    context.memory.ram_mut().fill(0xa5);
                    context.data[0] = 0xdead_beef;
                    context.data[1] = 0x89ab_cdef;
                    context.sr = 0xa71b;
                    context.delay = delay;
                    context.psg = input.rotate_left(1);
                    context.pad_low = ((!input & 0xc0) >> 2) | 0x83;
                    context.pad_high = (!input & 0x3f) | 0xc0;
                    context.trace.clear();
                    let data = context.data;
                    let addresses = context.addresses;
                    let translated = if second {
                        get_ctrl2_input
                    } else {
                        get_ctrl1_input
                    };
                    assert_eq!(
                        translated(&mut context, &[]).unwrap(),
                        Some(u32::from(input))
                    );
                    assert_eq!(context.data[0], u32::from(input));
                    assert_eq!(context.data[1], 0x89ab_ff00 | u32::from(!input & 0x3f));
                    assert_eq!(context.data[2..], data[2..]);
                    assert_eq!(context.addresses, addresses);
                    assert_eq!(context.sr, if delay < 2 { 0xa714 } else { 0xa704 });
                    assert_eq!(context.delay, delay.wrapping_sub(2));
                    assert_eq!(context.read_u16(status), u16::from(context.psg) << 8 | 0xa5);
                    assert_eq!(
                        context.trace,
                        [
                            Access::WriteWord(0xa11100, 0x0100),
                            Access::WriteByte(control_address, 0x40),
                            Access::WriteByte(pad_address, 0),
                            Access::ReadByte(pad_address, context.pad_low),
                            Access::WriteByte(pad_address, 0x40),
                            Access::ReadByte(pad_address, context.pad_high),
                            Access::ReadByte(0xa0004a, delay),
                            Access::WriteByte(0xa0004a, delay.wrapping_sub(2)),
                            Access::ReadByte(0xa0004b, context.psg),
                            Access::RamByte(status, context.psg),
                            Access::WriteWord(0xa11100, 0),
                        ]
                    );
                    assert!(
                        context
                            .memory
                            .ram()
                            .iter()
                            .enumerate()
                            .all(|(offset, &byte)| {
                                byte == if offset == (status & 0xffff) as usize {
                                    context.psg
                                } else {
                                    0xa5
                                }
                            })
                    );
                }
            }
        }
    }
}

#[test]
fn delivered_native_closure_is_complete_and_waiting_writers_are_not_approved() {
    let roots: Vec<_> = TRANSLATIONS.iter().map(|entry| entry.id).collect();
    assert!(Dispatcher::native(&roots).is_ok());
    for id in [
        crate::ids::DMA_COPY_WORDS_TO_VRAM,
        crate::ids::CLEAR_VIDEO_RAM,
        crate::ids::CLEAR_VERTICAL_SCROLL_RAM,
        crate::ids::CLEAR_COLOUR_RAM,
        crate::ids::COPY_LONGWORD_RUN_TO_VRAM_TILE,
        crate::ids::FILL_VDP_DATA_WORDS,
        crate::ids::FILL_VDP_DATA_BYTES,
        crate::ids::MOVE_DATA_TO_VRAM,
        crate::ids::UPLOAD_VRAM_WORD_RECTANGLE,
        crate::ids::COPY_PALETTE_TO_CRAM_SLOT,
    ] {
        assert!(!crate::is_atomic_capable(id), "{}", id.metadata().name);
        assert!(Dispatcher::hybrid().translation(id).is_none());
    }
}
