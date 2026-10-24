use super::*;
use crate::{Dispatcher, Platform, call_frame::FrameState};
use tje_mem::{AddressSpace, Bus, Revision};

struct Machine {
    memory: AddressSpace,
    data: [u32; 8],
    addresses: [u32; 7],
    status: u16,
    sp: u32,
    frame: FrameState,
    z80: [u8; 0x2000],
    requested: bool,
    reset: bool,
    bus_reads: usize,
    writes: Vec<(u32, u16, bool)>,
}
impl Machine {
    fn new(revision: Revision) -> Self {
        let mut rom = vec![0; 0x40000];
        for id in [
            ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
            ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
            ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
        ] {
            let offset = if id == ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE {
                0x44
            } else {
                0x18
            };
            let address = id.metadata().address(revision) as usize + offset;
            rom[address..address + 2].copy_from_slice(&0x1234_u16.to_be_bytes());
        }
        Self {
            memory: AddressSpace::new(rom, revision),
            data: std::array::from_fn(|index| 0xa1b2_c300 | index as u32),
            addresses: std::array::from_fn(|index| 0x1234_5600 | index as u32),
            status: 0x271f,
            sp: 0xff7f00,
            frame: FrameState::new(0xff7f00),
            z80: [0; 0x2000],
            requested: false,
            reset: false,
            bus_reads: 0,
            writes: Vec::new(),
        }
    }
}
impl Bus for Machine {
    fn read_u8(&self, address: u32) -> u8 {
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
    }
}
impl tje_mem::Platform for Machine {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("mailbox touched VDP");
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("mailbox touched VDP");
    }
}
impl Platform for Machine {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        assert!(self.requested && !self.reset);
        self.z80[(address - 0xa00000) as usize]
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("BUSACK requires an explicit source");
    }
    fn read_device_u16_with_open_bus(
        &mut self,
        address: u32,
        source: OpenBusSource,
    ) -> Result<u16> {
        assert_eq!(address, 0xa11100);
        self.bus_reads += 1;
        let operand = self.memory.read_u16(source.address()?);
        Ok((operand & !0x100)
            | if self.requested && !self.reset {
                0
            } else {
                0x100
            })
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        assert!(self.requested && !self.reset);
        self.writes.push((address, u16::from(value), false));
        self.z80[(address - 0xa00000) as usize] = value;
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        assert_eq!(address, 0xa11100);
        self.requested = value & 0x100 != 0;
        self.writes.push((address, value, true));
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
        panic!("mailbox body called original");
    }
}

#[test]
fn direct_pcm_keeps_packet_order_timer_divu_residue_and_zero_length_request() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for length in [0_u16, 1, 137, 138, 139, u16::MAX] {
            let mut machine = Machine::new(revision);
            let sample = 0xff9200;
            machine.memory.write_u16(sample, length);
            ram::g_pending_pcm_sample(revision).write(&mut machine.memory, sample as i32);
            ram::g_pending_psg_effect(revision).write(&mut machine.memory, 12);
            audio_submit_pending_pcm_sample(&mut machine, &[]).unwrap();
            let payload = sample + 2;
            assert_eq!(
                machine.writes,
                [
                    (0xa11100, 0x100, true),
                    (0xa00044, payload as u8 as u16, false),
                    (0xa00045, (payload >> 8) as u8 as u16, false),
                    (0xa00046, (payload >> 16) as u8 as u16, false),
                    (0xa00048, length as u8 as u16, false),
                    (0xa00049, (length >> 8) as u8 as u16, false),
                    (0xa00042, (payload >> 16) as u8 as u16, false),
                    (0xa00040, 2, false),
                    (0xa11100, 0, true),
                ]
            );
            assert_eq!(machine.bus_reads, 1);
            assert_eq!(machine.data[0], payload >> 16);
            assert_eq!(machine.data[1], (0xa1b2_0000 | u32::from(length)) >> 8);
            assert_eq!(
                machine.data[2],
                (u32::from(length % 138) << 16) | u32::from(length / 138 + 5)
            );
            assert_eq!(machine.addresses[0], payload);
            assert_eq!(
                ram::g_pcm_effect_cooldown_ticks(revision).read(&machine.memory),
                (length / 138 + 5) as i16
            );
            assert_eq!(
                ram::g_pending_psg_effect(revision).read(&machine.memory),
                -1
            );
            assert_eq!(ram::g_pending_pcm_sample(revision).read(&machine.memory), 0);
            assert_eq!(machine.status, 0x2704);
        }
    }
}

#[test]
fn pcm_timeout_and_retry_fragment_keep_pending_pointer_and_exact_poll_limit() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for retry in [false, true] {
            let mut machine = Machine::new(revision);
            machine.reset = true;
            ram::g_pending_pcm_sample(revision).write(&mut machine.memory, 0x1234_5678);
            let entry_d0 = machine.data[0];
            if retry {
                audio_retry_pending_pcm_mailbox(&mut machine, &[]).unwrap();
            } else {
                audio_submit_pending_pcm_sample(&mut machine, &[]).unwrap();
            }
            assert_eq!(machine.bus_reads, 19);
            assert_eq!(machine.data[0], if retry { 0xffff } else { entry_d0 });
            assert_eq!(machine.data[1], 0xa1b2_1334);
            assert_eq!(machine.data[2], 0xa1b2_0000);
            assert_eq!(
                ram::g_pending_pcm_sample(revision).read(&machine.memory),
                0x1234_5678
            );
            assert_eq!(
                ram::g_pending_psg_effect(revision).read(&machine.memory),
                -1
            );
            assert!(!machine.requested);
            assert_eq!(machine.status, 0x2708);
        }
    }
}

#[test]
fn indexed_psg_publishes_id_then_command_and_always_consumes_on_timeout() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for reset in [false, true] {
            for retry in [false, true] {
                let mut machine = Machine::new(revision);
                machine.reset = reset;
                ram::g_pending_psg_effect(revision).write(&mut machine.memory, 0x1234);
                ram::g_psg_effect_status(revision).write(&mut machine.memory, -7);
                if retry {
                    audio_retry_pending_psg_mailbox(&mut machine, &[]).unwrap();
                } else {
                    audio_submit_pending_indexed_command(&mut machine, &[]).unwrap();
                }
                assert_eq!(machine.bus_reads, if reset { 19 } else { 1 });
                assert_eq!(
                    ram::g_pending_psg_effect(revision).read(&machine.memory),
                    -1
                );
                assert_eq!(
                    ram::g_psg_effect_status(revision).read(&machine.memory),
                    if reset { -7 } else { 15 }
                );
                let bytes: Vec<_> = machine
                    .writes
                    .iter()
                    .copied()
                    .filter(|entry| !entry.2)
                    .collect();
                assert_eq!(
                    bytes,
                    if reset {
                        Vec::new()
                    } else {
                        vec![(0xa00042, 0x34, false), (0xa00040, 1, false)]
                    }
                );
                assert!(!machine.requested);
                assert_eq!(machine.status, 0x2708);
            }
        }
    }
}

#[test]
fn immediate_pcm_preserves_saved_registers_and_all_early_return_paths() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (cooldown, inhibit, active, reset) in [
            (1, 0, 0, false),
            (0, -1, 0, false),
            (0, 0, 1, false),
            (0, 0, 0, true),
            (0, 0, 0, false),
        ] {
            let mut machine = Machine::new(revision);
            machine.reset = reset;
            machine.z80[PSG_ACTIVE as usize] = active;
            machine
                .frame
                .argument_slot(0)
                .write(&mut machine.memory, 0xff9200);
            machine.memory.write_u16(0xff9200, 138);
            ram::g_pcm_effect_cooldown_ticks(revision).write(&mut machine.memory, cooldown);
            ram::g_music_alternate_template_bank_enabled(revision)
                .write(&mut machine.memory, inhibit);
            let data = machine.data;
            let addresses = machine.addresses;
            let sp = machine.sp;
            audio_try_submit_sequencer_pcm_sample(&mut machine, &[]).unwrap();
            assert_eq!(machine.data, data);
            assert_eq!(machine.addresses, addresses);
            assert_eq!(machine.sp, sp);
            assert_eq!(
                ram::g_pcm_effect_cooldown_ticks(revision).read(&machine.memory),
                cooldown
            );
            assert_eq!(
                machine.z80[0x40],
                if cooldown == 0 && inhibit == 0 && active == 0 && !reset {
                    2
                } else {
                    0
                }
            );
            assert!(!machine.requested);
        }
    }
}
