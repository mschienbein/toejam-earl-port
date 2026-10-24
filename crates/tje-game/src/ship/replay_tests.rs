//! ROM-free access-order regressions. Callees are explicit unit-test doubles;
//! these tests are not emulator proofs or ReplayedEnvironment evidence.
use std::cell::RefCell;

use crate::{Context, Dispatcher, FunctionId, Result, call_frame::FrameState, execution, ids};
use tje_mem::{AddressSpace, Bus, Platform, Revision, ram, rom};

const SP: u32 = 0xff7000;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Read(u32, u8, u32),
    Discard(u32, u8, u32),
    Write(u32, u8, u32, u32),
    Control(u16, u32),
    Call(FunctionId, Vec<u32>, u32),
}

struct Machine {
    memory: AddressSpace,
    events: RefCell<Vec<Event>>,
    data: [u32; 8],
    address: [u32; 8],
    frame: FrameState,
    allocated: u32,
}

impl Machine {
    fn new(revision: Revision) -> Self {
        // Authored synthetic frame table: ten empty frames. No cartridge data.
        let mut image = vec![0; 0x100000];
        let pointer = rom::intro_and_ship_animation_pointers(revision)
            .at(1)
            .address() as usize;
        image[pointer..pointer + 4].copy_from_slice(&0x1000_u32.to_be_bytes());
        for piece in 0..10 {
            image[0x1000 + piece * 4..0x1004 + piece * 4]
                .copy_from_slice(&(0x2000_u32 + piece as u32 * 16).to_be_bytes());
        }
        let mut machine = Self {
            memory: AddressSpace::new(image, revision),
            events: RefCell::default(),
            data: [0; 8],
            address: [0; 8],
            frame: FrameState::new(SP),
            allocated: 0,
        };
        machine.address[7] = SP;
        machine
    }

    fn reset_events(&self) {
        self.events.borrow_mut().clear();
    }

    fn stub(&mut self, id: FunctionId, arguments: &[u32]) -> Result<u32> {
        self.events
            .borrow_mut()
            .push(Event::Call(id, arguments.to_vec(), self.address[7]));
        if id == ids::ALLOCATE_ENTITY {
            return Ok(self.allocated);
        }
        if id == ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES {
            tje_mem::Address::<tje_mem::ObjectState>::new(arguments[0])
                .draw_flags()
                .write(&mut self.memory, 3);
        }
        if id == ids::ANIMATE_SHIP_SCREEN_BACKGROUND {
            ram::g_frame_counter(self.revision()).write(&mut self.memory, 240);
        }
        if id == ids::ANIMATE_TRANSITION_BACKGROUND {
            ram::g_frame_counter(self.revision()).write(&mut self.memory, 480);
        }
        Ok(0)
    }
}

impl Bus for Machine {
    fn read_u8(&self, address: u32) -> u8 {
        let value = self.memory.read_u8(address);
        self.events
            .borrow_mut()
            .push(Event::Read(address, 1, u32::from(value)));
        value
    }
    fn read_u16(&self, address: u32) -> u16 {
        let value = self.memory.read_u16(address);
        self.events
            .borrow_mut()
            .push(Event::Read(address, 2, u32::from(value)));
        value
    }
    fn read_u32(&self, address: u32) -> u32 {
        let value = self.memory.read_u32(address);
        self.events
            .borrow_mut()
            .push(Event::Read(address, 4, value));
        value
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.events
            .borrow_mut()
            .push(Event::Write(address, 1, u32::from(value), self.address[7]));
        self.memory.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.events
            .borrow_mut()
            .push(Event::Write(address, 2, u32::from(value), self.address[7]));
        self.memory.write_u16(address, value);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.events
            .borrow_mut()
            .push(Event::Write(address, 4, value, self.address[7]));
        self.memory.write_u32(address, value);
    }
}
impl Platform for Machine {
    fn write_vdp_control(&mut self, value: u16) {
        self.events
            .borrow_mut()
            .push(Event::Control(value, self.address[7]));
    }
    fn write_vdp_data(&mut self, _: u16) {}
}
impl crate::Platform for Machine {
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        let value = self.memory.read_u8(address);
        self.events
            .borrow_mut()
            .push(Event::Discard(address, 1, u32::from(value)));
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        let value = self.memory.read_u16(address);
        self.events
            .borrow_mut()
            .push(Event::Discard(address, 2, u32::from(value)));
        Ok(())
    }
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("unexpected unit device access")
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("unexpected unit device access")
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("unexpected unit device access")
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("unexpected unit device access")
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
        self.address[index]
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.address[index] = value;
    }
    fn status_register(&self) -> u16 {
        0x2000
    }
    fn set_status_register(&mut self, _: u16) {}
    fn stack_pointer(&self) -> Result<u32> {
        Ok(self.address[7])
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.address[7] = value;
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
        Dispatcher::hybrid_with(&[])
    }
    fn execution_class(&self) -> execution::ExecutionClass {
        execution::ExecutionClass::TimeAware
    }
    fn enter_call_ceiling(&mut self, _: FunctionId) -> Result<execution::CallCeilingScope> {
        Ok(execution::CallCeilingScope::not_required())
    }
    fn call_original(&mut self, id: FunctionId, args: &[u32]) -> Result<u32> {
        self.stub(id, args)
    }
    fn call_original_with_execution(
        &mut self,
        id: FunctionId,
        args: &[u32],
        _: execution::ExecutionClass,
    ) -> Result<u32> {
        self.stub(id, args)
    }
    fn call_original_entry(&mut self, id: FunctionId, _: execution::ExecutionClass) -> Result<u32> {
        let sp = self.address[7];
        let args: Vec<_> = (0..u32::from(id.metadata().abi.argument_count))
            .map(|index| self.memory.read_u32(sp + 4 + index * 4))
            .collect();
        let result = self.stub(id, &args)?;
        self.address[7] = sp + 4;
        Ok(result)
    }
}

#[test]
fn speech_eviction_reads_incumbent_before_each_candidate() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        ram::g_player_info(revision)
            .at(0)
            .draw_flags()
            .write(&mut c.memory, 3);
        for (slot, lifetime) in [30, 20, 20, 10].into_iter().enumerate() {
            ram::g_speech_bubble_entities(revision)
                .at(slot as i32)
                .write(&mut c.memory, slot as i8 + 6);
            ram::g_speech_bubble_lifetimes(revision)
                .at(slot as i32)
                .write(&mut c.memory, lifetime);
        }
        c.reset_events();
        assert_eq!(
            crate::speech::generate_speech(&mut c, &[0, 0xfff000, 0]).unwrap(),
            Some(0)
        );
        let base = ram::g_speech_bubble_lifetimes(revision).address();
        let reads: Vec<_> = c
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Event::Read(address, 1, _) if (base..base + 4).contains(address) => {
                    Some(*address - base)
                }
                _ => None,
            })
            .collect();
        assert_eq!(reads, [0, 1, 1, 2, 1, 3]);
        assert!(c.events.borrow().iter().any(|event| matches!(event, Event::Call(id, args, _) if *id == ids::FREE_ENTITY && args == &[9])));
    }
}

#[test]
fn speech_update_reads_second_player_level_before_first() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        ram::g_player_info(revision)
            .at(1)
            .level()
            .write(&mut c.memory, 1);
        c.reset_events();
        crate::speech::update_speech_and_dialogue(&mut c, &[]).unwrap();
        let first = ram::g_player_info(revision).at(0).level().address();
        let second = ram::g_player_info(revision).at(1).level().address();
        let levels: Vec<_> = c
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Event::Read(address, 1, _) if *address == first || *address == second => {
                    Some(*address)
                }
                _ => None,
            })
            .collect();
        assert_eq!(levels, [second, first]);
        assert_eq!(c.address[7], SP);
    }
}

#[test]
fn ship_screen_preserves_local_predicates_and_retained_dma_ack_slots() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        for piece in 0..10 {
            ram::g_ship_piece_level_assignments(revision)
                .at(piece)
                .write(&mut c.memory, 2);
        }
        super::show_ship_pieces(&mut c, &[u32::MAX, 2]).unwrap();
        let fp = SP - 4;
        let events = c.events.borrow();
        let pass_init = events
            .iter()
            .position(|e| matches!(e, Event::Write(a, 2, 0, _) if *a == fp - 12))
            .unwrap();
        assert_eq!(
            events[pass_init + 1],
            Event::Write(fp - 2, 2, 0, fp - 420 - 36)
        );
        assert_eq!(events[pass_init + 2], Event::Read(fp - 12, 2, 0));
        assert_eq!(events[pass_init + 3], Event::Read(fp - 2, 2, 0));
        let base_tile_updates: Vec<_> = events
            .windows(3)
            .filter(|w| matches!(w[2], Event::Write(a, 2, _, _) if a == fp - 10))
            .collect();
        // The first store initializes base_tile; the other ten are source RMWs.
        assert_eq!(base_tile_updates.len(), 11);
        assert!(matches!(
            base_tile_updates[0][2],
            Event::Write(_, 2, 129, _)
        ));
        for window in base_tile_updates.into_iter().skip(1) {
            assert!(matches!(window[0], Event::Read(a, 2, _) if a == fp - 4));
            assert!(matches!(window[1], Event::Read(a, 2, _) if a == fp - 10));
        }
        let mode = ram::g_mode2_reg_softcopy(revision).address();
        for (id, slots) in [
            (ids::DMA_COPY_WORDS_TO_VRAM, 12),
            (ids::WAIT_FOR_SHIP_SCREEN_ACKNOWLEDGEMENT, 4),
        ] {
            let call = events
                .iter()
                .position(|e| matches!(e, Event::Call(target, _, _) if *target == id))
                .unwrap();
            let Event::Call(_, _, entry_sp) = events[call] else {
                unreachable!()
            };
            let mode_write = events[call + 1..]
                .iter()
                .find(|e| matches!(e, Event::Write(a, 1, _, _) if *a == mode))
                .unwrap();
            assert!(matches!(mode_write, Event::Write(_, _, _, sp) if *sp == entry_sp + 4));
            let control = events[call + 1..]
                .iter()
                .find(|e| matches!(e, Event::Control(_, _)))
                .unwrap();
            assert!(matches!(control, Event::Control(_, sp) if *sp == entry_sp + 4 + slots));
        }
        assert_eq!(c.address[7], SP);
    }
}

#[test]
fn completion_page_loop_has_only_source_reads_and_retains_initial_wait() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        super::play_ship_completion_sequence(&mut c, &[0]).unwrap();
        let page = SP - 8;
        let events = c.events.borrow();
        let page_reads = events
            .iter()
            .filter(|e| matches!(e, Event::Read(a, 2, _) if *a == page))
            .count();
        // Eight row command reads, one increment read and one tail-test per page.
        assert_eq!(page_reads, 4 * (8 + 1 + 1));
        let mode = ram::g_mode2_reg_softcopy(revision).address();
        for id in [
            ids::WAIT_FOR_NEXT_V_BLANK,
            ids::ANIMATE_TRANSITION_BACKGROUND,
        ] {
            let call = events
                .iter()
                .position(|e| matches!(e, Event::Call(target, _, _) if *target == id))
                .unwrap();
            let Event::Call(_, _, entry_sp) = events[call] else {
                unreachable!()
            };
            let mode_write = events[call + 1..]
                .iter()
                .find(|e| matches!(e, Event::Write(a, 1, _, _) if *a == mode))
                .unwrap();
            assert!(matches!(mode_write, Event::Write(_, _, _, sp) if *sp == entry_sp + 4));
        }
        assert_eq!(c.address[7], SP);
    }
}

#[test]
fn pending_framed_entries_read_only_the_compiled_argument_widths() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for entry in super::PENDING_FRAMED_TRANSLATIONS
            .iter()
            .chain(crate::speech::PENDING_FRAMED_TRANSLATIONS)
        {
            let mut c = Machine::new(revision);
            c.memory.write_u32(SP + 4, 0);
            c.memory.write_u32(SP + 8, 0xff5000);
            c.memory.write_u32(SP + 12, 0);
            (entry.function)(&mut c).unwrap();
            let reads: Vec<_> = c
                .events
                .borrow()
                .iter()
                .filter_map(|event| match event {
                    Event::Read(address, width, _) if (SP + 4..SP + 16).contains(address) => {
                        Some((*address - SP, *width))
                    }
                    _ => None,
                })
                .collect();
            let expected: &[(u32, u8)] = match entry.id {
                ids::ANIMATE_SHIP_SCREEN_BACKGROUND => &[(7, 1)],
                ids::SHIP_PIECE_ABILITY => &[(6, 2), (8, 4)],
                ids::SHOW_SHIP_PIECES | ids::START_DIALOGUE => &[(6, 2), (10, 2)],
                ids::GENERATE_SPEECH => &[(6, 2), (8, 4), (15, 1)],
                _ => &[(6, 2)],
            };
            assert_eq!(reads, expected, "{}", entry.id.metadata().name);
            assert_eq!(c.address[7], SP);
        }
    }
}

#[test]
fn speech_prepare_retains_the_text_queue_arguments_until_shared_cleanup() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        c.allocated = 6;
        ram::g_player_info(revision)
            .at(0)
            .draw_flags()
            .write(&mut c.memory, 3);
        assert_eq!(
            crate::speech::generate_speech(&mut c, &[0, 0xfff000, 1]).unwrap(),
            Some(6)
        );
        let events = c.events.borrow();
        let queue = events
            .iter()
            .find_map(|event| match event {
                Event::Call(id, args, sp) if *id == ids::QUEUE_SPEECH_TEXT_TILES => {
                    Some((args, *sp))
                }
                _ => None,
            })
            .unwrap();
        let prepare = events
            .iter()
            .find_map(|event| match event {
                Event::Call(id, args, sp) if *id == ids::PREPARE_ENTITY_SPRITE_FRAME => {
                    Some((args, *sp))
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(queue.0[0..2], [0xfff000, 0]);
        assert_eq!(prepare.1 + 16, queue.1);
        assert_eq!(c.address[7], SP);
    }
}

#[test]
fn initialization_retains_all_four_palette_argument_pairs() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        super::initialize_ship_screen_tilemap_and_palettes(&mut c, &[]).unwrap();
        let calls: Vec<_> = c
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Event::Call(id, args, sp) if *id == ids::COPY_PALETTE_TO_CRAM_SLOT => {
                    Some((args[1], *sp))
                }
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 4);
        for (index, &(slot, sp)) in calls.iter().enumerate() {
            assert_eq!(slot, index as u32);
            assert_eq!(sp, calls[0].1 - 8 * index as u32);
        }
        assert_eq!(c.address[7], SP);
    }
}

#[test]
fn ship_name_reads_the_compiled_switch_word_only_for_valid_piece_indices() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let table = ids::SHOW_SHIP_PIECE_NAME.metadata().address(revision) + 0x20;
        for piece in -1_i16..=10 {
            let mut c = Machine::new(revision);
            c.data[3] = 0xff5000;
            super::show_ship_piece_name(&mut c, &[i32::from(piece) as u32]).unwrap();
            let reads: Vec<_> = c
                .events
                .borrow()
                .iter()
                .filter_map(|event| match event {
                    Event::Read(address, 2, _) if (table..table + 20).contains(address) => {
                        Some(*address)
                    }
                    _ => None,
                })
                .collect();
            if (0..10).contains(&piece) {
                assert_eq!(reads, [table + u32::from(piece as u16) * 2]);
            } else {
                assert!(reads.is_empty());
            }
            assert_eq!(c.data[3], 0xff5000);
            assert_eq!(c.address[7], SP);
        }
    }
}

fn discarded_clear(events: &[Event], address: u32, width: u8, old: u32) {
    assert!(
        events.windows(2).any(|pair| matches!(
            (&pair[0], &pair[1]),
            (Event::Discard(a, w, value), Event::Write(b, next_width, 0, _))
                if *a == address && *b == address && *w == width && *next_width == width && *value == old
        )),
        "missing tagged discarded read before CLR at {address:08x}",
    );
}

#[test]
fn ship_and_speech_clr_reads_keep_discarded_purpose_before_zero_stores() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Machine::new(revision);
        ram::g_frame_counter(revision).write(&mut c.memory, 0x1234);
        super::wait_for_ship_screen_acknowledgement(&mut c, &[2]).unwrap();
        discarded_clear(
            &c.events.borrow(),
            ram::g_frame_counter(revision).address(),
            2,
            0x1234,
        );

        let mut c = Machine::new(revision);
        c.allocated = 6;
        ram::g_player_info(revision)
            .at(0)
            .draw_flags()
            .write(&mut c.memory, 3);
        ram::g_next_speech_lifetime_override(revision).write(&mut c.memory, 0x1234);
        crate::speech::generate_speech(&mut c, &[0, 0xfff000, 1]).unwrap();
        discarded_clear(
            &c.events.borrow(),
            ram::g_next_speech_lifetime_override(revision).address(),
            2,
            0x1234,
        );
        assert_eq!(
            ram::g_speech_bubble_lifetimes(revision)
                .at(0)
                .read(&c.memory),
            0x34
        );

        let mut c = Machine::new(revision);
        ram::g_speech_bubble_entities(revision)
            .at(0)
            .write(&mut c.memory, 6);
        ram::g_speech_bubble_lifetimes(revision)
            .at(0)
            .write(&mut c.memory, 0);
        ram::g_player_info(revision)
            .at(1)
            .level()
            .write(&mut c.memory, 1);
        crate::speech::update_speech_and_dialogue(&mut c, &[]).unwrap();
        discarded_clear(
            &c.events.borrow(),
            ram::g_speech_bubble_entities(revision).at(0).address(),
            1,
            6,
        );

        let mut c = Machine::new(revision);
        let record = tje_mem::Address::<tje_mem::EarthlingState>::new(0xff6000);
        record.object().write(&mut c.memory, 6);
        super::ship_piece_ability(&mut c, &[0, record.address()]).unwrap();
        discarded_clear(&c.events.borrow(), record.object().address(), 1, 6);
    }
}
