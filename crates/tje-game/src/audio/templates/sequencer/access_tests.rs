use super::music_update_sequencer;
use crate::{Context, Dispatcher, FunctionId, Platform, Result, call_frame::FrameState};
use std::{cell::RefCell, collections::VecDeque};
use tje_mem::{AddressSpace, Bus, Revision, ram};

struct Machine {
    memory: AddressSpace,
    data: [u32; 8],
    addresses: [u32; 7],
    status: u16,
    sp: u32,
    frame: FrameState,
    operands: RefCell<VecDeque<(u32, u16)>>,
    word_reads: RefCell<Vec<u32>>,
}

impl Machine {
    fn new(revision: Revision) -> Self {
        let mut memory = AddressSpace::new(Vec::new(), revision);
        ram::g_pending_fm_effect(revision).write(&mut memory, -1);
        ram::g_pending_psg_effect(revision).write(&mut memory, -1);
        Self {
            memory,
            data: std::array::from_fn(|index| 0xa1b2_c300 | index as u32),
            addresses: std::array::from_fn(|index| 0x1234_5600 | index as u32),
            status: 0x271f,
            sp: 0xff7f00,
            frame: FrameState::new(0xff7f00),
            operands: RefCell::new(VecDeque::new()),
            word_reads: RefCell::new(Vec::new()),
        }
    }
}

impl Bus for Machine {
    fn read_u8(&self, address: u32) -> u8 {
        self.memory.read_u8(address)
    }

    fn read_u16(&self, address: u32) -> u16 {
        self.word_reads.borrow_mut().push(address);
        let mut operands = self.operands.borrow_mut();
        if operands
            .front()
            .is_some_and(|(target, _)| *target == address)
        {
            return operands.pop_front().unwrap().1;
        }
        self.memory.read_u16(address)
    }

    fn write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
    }
}

impl tje_mem::Platform for Machine {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("RAM-only sequencer timer fixture accessed VDP");
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("RAM-only sequencer timer fixture accessed VDP");
    }
}

impl Platform for Machine {
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("RAM-only sequencer timer fixture accessed a device");
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("RAM-only sequencer timer fixture accessed a device");
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("RAM-only sequencer timer fixture accessed a device");
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("RAM-only sequencer timer fixture accessed a device");
    }
}

impl Context for Machine {
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
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid()
    }
    fn call_original(&mut self, _: FunctionId, _: &[u32]) -> Result<u32> {
        panic!("RAM-only sequencer timer fixture called an original");
    }
}

#[test]
fn timer_tests_and_memory_decrements_read_separate_operands_in_source_order() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (psg_test, psg_operand, speech_test, speech_operand, blocking) in [
            (0_u16, 0_u16, 0_u16, 0_u16, false),
            (1, 7, 0, 0, false),
            (0, 0, 1, 9, false),
            (1, 0x8000, 1, 1, true),
            (0xffff, 0, 1, 2, true),
        ] {
            let mut machine = Machine::new(revision);
            let psg = ram::g_psg_effect_status(revision);
            let speech = ram::g_pcm_effect_cooldown_ticks(revision);
            let blocks = ram::g_pcm_blocks_sequencer(revision);
            psg.write(&mut machine.memory, psg_test as i16);
            speech.write(&mut machine.memory, speech_test as i16);
            blocks.write(&mut machine.memory, i16::from(blocking));
            let mut expected_reads = vec![psg.address()];
            machine
                .operands
                .borrow_mut()
                .push_back((psg.address(), psg_test));
            if psg_test != 0 {
                expected_reads.push(psg.address());
                machine
                    .operands
                    .borrow_mut()
                    .push_back((psg.address(), psg_operand));
            }
            expected_reads.push(speech.address());
            machine
                .operands
                .borrow_mut()
                .push_back((speech.address(), speech_test));
            if speech_test != 0 {
                expected_reads.push(speech.address());
                machine
                    .operands
                    .borrow_mut()
                    .push_back((speech.address(), speech_operand));
                expected_reads.push(blocks.address());
                if blocking {
                    expected_reads.push(speech.address());
                }
            }
            let saved_data = machine.data;
            let saved_addresses = machine.addresses;
            let saved_sp = machine.sp;
            music_update_sequencer(&mut machine, &[]).unwrap();
            assert!(machine.operands.borrow().is_empty());
            let actual_reads: Vec<_> = machine
                .word_reads
                .borrow()
                .iter()
                .copied()
                .filter(|address| {
                    [psg.address(), speech.address(), blocks.address()].contains(address)
                })
                .collect();
            assert_eq!(actual_reads, expected_reads);
            let expected_psg = if psg_test != 0 {
                psg_operand.wrapping_sub(1)
            } else {
                psg_test
            };
            let expected_speech = if speech_test != 0 {
                speech_operand.wrapping_sub(1)
            } else {
                speech_test
            };
            assert_eq!(psg.read(&machine.memory) as u16, expected_psg);
            assert_eq!(speech.read(&machine.memory) as u16, expected_speech);
            assert_eq!(
                blocks.read(&machine.memory),
                i16::from(blocking && expected_speech != 0)
            );
            assert_eq!(machine.data[2..], saved_data[2..]);
            assert_eq!(machine.addresses[2..], saved_addresses[2..]);
            assert_eq!(machine.sp, saved_sp);
        }
    }
}
