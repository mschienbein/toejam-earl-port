use super::*;
use crate::{Dispatcher, Error, FunctionId, Platform};
use std::cell::RefCell;
use tje_mem::{AddressSpace, Bus, Revision};

#[derive(Debug, PartialEq, Eq)]
enum Access {
    Read(u32),
    Byte(u32, u8),
    Word(u32, u16),
    Long(u32, u32),
}

struct TestContext {
    memory: AddressSpace,
    data: [u32; 8],
    addresses: [u32; 8],
    status: u16,
    accesses: RefCell<Vec<Access>>,
}

impl TestContext {
    fn new(revision: Revision, rom_bytes: Vec<u8>) -> Self {
        let mut memory = AddressSpace::new(rom_bytes, revision);
        memory.ram_mut().fill(0xa5);
        Self {
            memory,
            data: std::array::from_fn(|index| 0x1234_5670 | index as u32),
            addresses: std::array::from_fn(|index| 0xabcd_ef00 | index as u32),
            status: 0x271f,
            accesses: RefCell::new(Vec::new()),
        }
    }
}

impl Bus for TestContext {
    fn read_u8(&self, address: u32) -> u8 {
        self.accesses.borrow_mut().push(Access::Read(address));
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.accesses
            .borrow_mut()
            .push(Access::Byte(address, value));
        self.memory.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.accesses
            .borrow_mut()
            .push(Access::Word(address, value));
        self.memory.write_u16(address, value);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.accesses
            .borrow_mut()
            .push(Access::Long(address, value));
        self.memory.write_u32(address, value);
    }
}

impl tje_mem::Platform for TestContext {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("RAM-only audio leaf accessed VDP control");
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("RAM-only audio leaf accessed VDP data");
    }
}

impl Platform for TestContext {
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("RAM-only audio leaf read a device byte");
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("RAM-only audio leaf read a device word");
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("RAM-only audio leaf wrote a device byte");
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("RAM-only audio leaf wrote a device word");
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
        self.status
    }
    fn set_status_register(&mut self, value: u16) {
        self.status = value;
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::native(ATOMIC_CAPABLE).unwrap()
    }
    fn call_original(&mut self, _: FunctionId, _: &[u32]) -> Result<u32> {
        panic!("RAM-only audio leaf called an original function");
    }
}

#[test]
fn restore_copies_only_the_revision_resource_and_preserves_assembly_state() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let source = rom::music_initial_state(revision).address();
        let destination = ram::g_music_fm_channel_owners(revision).address();
        for (last, nz) in [(0, 4), (0x7f, 0), (0x80, 8), (0xff, 8)] {
            let mut bytes = vec![0; source as usize + 0x114];
            for offset in 0..0x114 {
                bytes[source as usize + offset] = (offset as u8).wrapping_mul(37).wrapping_add(11);
            }
            bytes[source as usize + 0x113] = last;
            let mut context = TestContext::new(revision, bytes);
            let data_before = context.data;
            let addresses_before = context.addresses;
            assert_eq!(
                music_restore_initial_state(&mut context, &[]).unwrap(),
                None
            );
            let mut expected = Vec::new();
            for offset in 0..0x114 {
                let value = context.memory.read_u8(source + offset);
                expected.push(Access::Read(source + offset));
                expected.push(Access::Byte(destination + offset, value));
                assert_eq!(context.memory.read_u8(destination + offset), value);
            }
            assert_eq!(*context.accesses.borrow(), expected);
            assert_eq!(context.memory.read_u8(destination - 1), 0xa5);
            assert_eq!(context.memory.read_u8(destination + 0x114), 0xa5);
            assert_eq!(context.data[0], 0x0000_ffff);
            assert_eq!(context.data[1..], data_before[1..]);
            assert_eq!(context.addresses[0], source + 0x114);
            assert_eq!(context.addresses[1], destination + 0x114);
            assert_eq!(context.addresses[2..], addresses_before[2..]);
            assert_eq!(context.status, 0x2700 | nz);
        }
    }
}

#[test]
fn clear_interleaves_six_owner_and_stream_longs_then_clears_request() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for status in [0x270f, 0x271f] {
            let mut context = TestContext::new(revision, Vec::new());
            context.status = status;
            let data_before = context.data;
            let addresses_before = context.addresses;
            let owners = ram::g_music_fm_channel_owners(revision).address();
            let channels = ram::g_music_channels(revision);
            let mut expected = Vec::new();
            let mut expected_ram = context.memory.clone();
            for index in 0..6 {
                let owner = owners + index as u32 * 4;
                let stream = channels.at(index).current_stream().address();
                expected.push(Access::Long(owner, 0));
                expected.push(Access::Long(stream, 0));
                expected_ram.write_u32(owner, 0);
                expected_ram.write_u32(stream, 0);
            }
            let request = ram::g_music_track_reset_requested(revision).address();
            expected.push(Access::Word(request, 0));
            expected_ram.write_u16(request, 0);
            assert_eq!(music_clear_track_pointers(&mut context, &[]).unwrap(), None);
            assert_eq!(*context.accesses.borrow(), expected);
            assert_eq!(context.memory.ram(), expected_ram.ram());
            assert_eq!(context.data[0], 0x0000_ffff);
            assert_eq!(context.data[1..], data_before[1..]);
            assert_eq!(context.addresses[0], owners + 24);
            assert_eq!(context.addresses[1], channels.at(6).address());
            assert_eq!(context.addresses[2..], addresses_before[2..]);
            assert_eq!(context.status, (status & !0xf) | 4);
        }
    }
}

#[test]
fn request_wrappers_write_exactly_one_word_and_narrow_the_raw_slot() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision, Vec::new());
        let data_before = context.data;
        let addresses_before = context.addresses;
        let mut expected = Vec::new();
        let mut expected_ram = context.memory.clone();
        assert_eq!(
            music_request_track_pointer_reset(&mut context, &[]).unwrap(),
            None
        );
        let reset = ram::g_music_track_reset_requested(revision).address();
        expected.push(Access::Word(reset, 1));
        expected_ram.write_u16(reset, 1);
        assert_eq!(
            audio_request_fm_channel_release(&mut context, &[]).unwrap(),
            None
        );
        let release = ram::g_fm_channel_release_requested(revision).address();
        expected.push(Access::Word(release, 1));
        expected_ram.write_u16(release, 1);
        let effect = ram::g_pending_fm_effect(revision).address();
        for raw in [0, 1, 0xdead_7fff, 0xbeef_8000, 0x1234_ffff] {
            assert_eq!(
                audio_request_channel_template_effect(&mut context, &[raw]).unwrap(),
                None
            );
            expected.push(Access::Word(effect, raw as u16));
            expected_ram.write_u16(effect, raw as u16);
        }
        assert_eq!(*context.accesses.borrow(), expected);
        assert_eq!(context.memory.ram(), expected_ram.ram());
        assert_eq!(context.data, data_before);
        assert_eq!(context.addresses, addresses_before);
    }
}

#[test]
fn play_sound_preserves_signed_priority_short_circuits_and_store_order() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision, Vec::new());
        let cooldown = ram::g_pcm_effect_cooldown_ticks(revision).address();
        let status = ram::g_psg_effect_status(revision).address();
        let pending = ram::g_pending_psg_effect(revision).address();
        let active = ram::g_sound_effect_priority(revision).address();
        for ticks in [i16::MIN, 0, 1] {
            for psg in [-1_i16, 0, i16::MAX] {
                for old in [i16::MIN, -1, 0, 1, i16::MAX] {
                    for priority in [i16::MIN, -1, 0, 1, i16::MAX] {
                        context.memory.ram_mut().fill(0xa5);
                        context.memory.write_i16(cooldown, ticks);
                        context.memory.write_i16(status, psg);
                        context.memory.write_i16(active, old);
                        context.accesses.borrow_mut().clear();
                        let data_before = context.data;
                        let addresses_before = context.addresses;
                        context.status = 0xa700
                            | ((ticks as u16)
                                .wrapping_add(psg as u16)
                                .wrapping_add(priority as u16)
                                & 0x1f);
                        let status_before = context.status;
                        let mut expected_ram = context.memory.clone();
                        let mut expected = vec![Access::Read(cooldown), Access::Read(cooldown + 1)];
                        if ticks == 0 {
                            expected.extend([Access::Read(status), Access::Read(status + 1)]);
                        }
                        if ticks != 0 || psg != 0 {
                            expected.extend([Access::Read(active), Access::Read(active + 1)]);
                        }
                        let sound = priority.wrapping_add(0x1234);
                        if (ticks == 0 && psg == 0) || priority >= old {
                            expected.push(Access::Word(pending, sound as u16));
                            expected.push(Access::Word(active, priority as u16));
                            expected_ram.write_i16(pending, sound);
                            expected_ram.write_i16(active, priority);
                        }
                        let arguments = [
                            0xdead_0000 | u32::from(sound as u16),
                            0xbeef_0000 | u32::from(priority as u16),
                        ];
                        assert_eq!(play_sound(&mut context, &arguments).unwrap(), None);
                        assert_eq!(*context.accesses.borrow(), expected);
                        assert_eq!(context.memory.ram(), expected_ram.ram());
                        assert_eq!(
                            context.data[0],
                            (data_before[0] & 0xffff_0000) | u32::from(priority as u16)
                        );
                        assert_eq!(context.data[1..], data_before[1..]);
                        assert_eq!(context.addresses, addresses_before);
                        let admitted = (ticks == 0 && psg == 0) || priority >= old;
                        let result = priority.wrapping_sub(old);
                        let flags = if admitted {
                            (u16::from(priority < 0) << 3) | (u16::from(priority == 0) << 2)
                        } else {
                            let widened = i32::from(priority) - i32::from(old);
                            (u16::from(result < 0) << 3)
                                | (u16::from(result == 0) << 2)
                                | (u16::from(i16::try_from(widened).is_err()) << 1)
                                | u16::from((priority as u16) < old as u16)
                        };
                        assert_eq!(context.status, (status_before & !0x0f) | flags);
                    }
                }
            }
        }
    }
}

#[test]
fn play_sound_effect_keeps_sample_bits_priority_and_one_way_blocking_latch() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = TestContext::new(revision, Vec::new());
        let cooldown = ram::g_pcm_effect_cooldown_ticks(revision).address();
        let status = ram::g_psg_effect_status(revision).address();
        let pending = ram::g_pending_pcm_sample(revision).address();
        let active = ram::g_sound_effect_priority(revision).address();
        let block = ram::g_pcm_blocks_sequencer(revision).address();
        let release = ram::g_fm_channel_release_requested(revision).address();
        for ticks in [i16::MIN, 0, 1] {
            for psg in [-1_i16, 0, i16::MAX] {
                for old in [i16::MIN, -1, 0, 1, i16::MAX] {
                    for priority in [i16::MIN, -1, 0, 1, i16::MAX] {
                        for stop in [0, 1, 0xffff] {
                            context.memory.ram_mut().fill(0xa5);
                            context.memory.write_i16(cooldown, ticks);
                            context.memory.write_i16(status, psg);
                            context.memory.write_i16(active, old);
                            context.accesses.borrow_mut().clear();
                            let mut expected_ram = context.memory.clone();
                            let mut expected =
                                vec![Access::Read(cooldown), Access::Read(cooldown + 1)];
                            if ticks == 0 {
                                expected.extend([Access::Read(status), Access::Read(status + 1)]);
                            }
                            if ticks != 0 || psg != 0 {
                                expected.extend([Access::Read(active), Access::Read(active + 1)]);
                            }
                            let sample = 0xfedc_0000 | u32::from(priority as u16);
                            if (ticks == 0 && psg == 0) || priority >= old {
                                expected.push(Access::Long(pending, sample));
                                expected.push(Access::Word(active, priority as u16));
                                expected_ram.write_u32(pending, sample);
                                expected_ram.write_i16(active, priority);
                                if stop != 0 {
                                    expected.push(Access::Word(block, 1));
                                    expected.push(Access::Word(release, 1));
                                    expected_ram.write_u16(block, 1);
                                    expected_ram.write_u16(release, 1);
                                }
                            }
                            assert_eq!(
                                play_sound_effect(
                                    &mut context,
                                    &[sample, 0xbeef_0000 | u32::from(priority as u16), stop]
                                )
                                .unwrap(),
                                None
                            );
                            assert_eq!(*context.accesses.borrow(), expected);
                            assert_eq!(context.memory.ram(), expected_ram.ram());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn delivered_audio_roots_have_native_closures_and_pending_bodies_fail_closed() {
    // ATOMIC_CAPABLE also classifies untranslated originals; the native closure
    // obligation applies to the translated roots.
    let roots: Vec<_> = TRANSLATIONS
        .iter()
        .map(|translation| translation.id)
        .collect();
    let dispatcher = Dispatcher::native(&roots).unwrap();
    for id in roots {
        if crate::is_atomic_capable(id) {
            for callee in id.metadata().callees {
                assert!(crate::is_atomic_capable(*callee));
            }
        }
        assert!(dispatcher.translation(id).is_some());
    }
    for translation in TRANSLATIONS {
        assert!(
            crate::is_atomic_capable(translation.id)
                || EXECUTION_CONTRACTS
                    .iter()
                    .any(|contract| contract.function == translation.id)
        );
    }
    for id in [
        ids::AUDIO_RETRY_PENDING_PCM_MAILBOX,
        ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
        ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
        ids::AUDIO_RETRY_PENDING_PSG_MAILBOX,
        ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
    ] {
        assert!(!crate::is_atomic_capable(id));
        assert!(dispatcher.translation(id).is_none());
        let mut context = TestContext::new(Revision::Rev02, Vec::new());
        assert_eq!(
            dispatcher.call_void(&mut context, id, &[]),
            Err(Error::Untranslated(id))
        );
    }
    for id in PENDING {
        assert!(crate::translations().all(|entry| entry.id != *id));
    }
}
