//! Synthetic source-access regressions, not ROM/replay proof receipts.
//! This arithmetic/annotation/dataflow model deliberately keeps the default
//! no-op call observer, which mints no child token. It provides no Host/Native/
//! Replace invocation, lifetime, witness, admission or proof coverage.
use super::{audio_callbacks, frame_support, lifecycle, scene_layouts};
use crate::{
    Context, Dispatcher, FramedTranslationFn, FunctionId, Result, TranslationFn,
    call_frame::FrameState,
    execution::{CallCeilingScope, ExecutionClass},
    ids,
    stack_read::{Guard, LocalWidth, Reader, UninitializedLocalRead},
};
use std::{cell::RefCell, collections::VecDeque, sync::Arc};
use tje_mem::{AddressSpace, Bus, Platform, Revision, ram, rom};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Access {
    Read(u32, u8, u32),
    Write(u32, u8, u32),
    Discard(u32, u8),
    Control(u16),
    Data(u16),
    Call(FunctionId, u32, u32, Vec<u32>),
}

pub(super) struct Machine {
    memory: AddressSpace,
    accesses: RefCell<Vec<Access>>,
    data: [u32; 8],
    address: [u32; 7],
    sr: u16,
    sp: u32,
    frame: FrameState,
    random: VecDeque<i32>,
    modal: bool,
    updates: u32,
    pad_reads: u32,
    allocations: VecDeque<u32>,
    guard: Arc<Guard>,
    declared_locals: Vec<(UninitializedLocalRead, u8, u32)>,
}

impl Machine {
    pub(super) fn new(revision: Revision) -> Self {
        Self {
            memory: AddressSpace::new(vec![0; 0x100000], revision),
            accesses: RefCell::new(Vec::new()),
            data: [0xa5a5_1234; 8],
            address: [0x1234_5678; 7],
            sr: 0x271f,
            sp: 0xfff000,
            frame: FrameState::new(0xfff000),
            random: VecDeque::new(),
            modal: false,
            updates: 0,
            pad_reads: 0,
            allocations: VecDeque::new(),
            guard: Arc::new(Guard::default()),
            declared_locals: Vec::new(),
        }
    }
    pub(super) fn clear_accesses(&self) {
        self.accesses.borrow_mut().clear();
    }
    pub(super) fn run(&mut self, function: TranslationFn, arguments: &[u32]) {
        for (index, value) in arguments.iter().enumerate() {
            self.memory
                .write_u32(self.frame.entry_sp + 4 + index as u32 * 4, *value);
        }
        function(self, arguments).unwrap();
        assert_eq!(self.sp, self.frame.entry_sp);
    }
    pub(super) fn run_framed(&mut self, function: FramedTranslationFn, arguments: &[u32]) {
        for (index, value) in arguments.iter().enumerate() {
            self.memory
                .write_u32(self.frame.entry_sp + 4 + index as u32 * 4, *value);
        }
        function(self).unwrap();
        assert_eq!(self.sp, self.frame.entry_sp);
    }
    fn calls(&self) -> Vec<(FunctionId, u32, u32, Vec<u32>)> {
        self.accesses
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Access::Call(id, sp, pc, arguments) => Some((*id, *sp, *pc, arguments.clone())),
                _ => None,
            })
            .collect()
    }
    fn emulate_callee(&mut self, id: FunctionId) -> u32 {
        let revision = self.revision();
        match id {
            ids::ALLOCATE_ENTITY => self.allocations.pop_front().unwrap_or(0),
            ids::GET_RANDOM_NUMBER => self.random.pop_front().unwrap_or(0) as u32,
            ids::LMODT => ((self.data[0] as i32) % (self.data[1] as i32)) as u32,
            ids::GET_CTRL1_INPUT | ids::GET_CTRL2_INPUT => {
                self.pad_reads += 1;
                if self.modal
                    && ((self.updates == 0 && self.pad_reads <= 2)
                        || (self.updates != 0 && self.updates != 5))
                {
                    0x80
                } else {
                    0
                }
            }
            ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE => {
                self.updates += 1;
                if self.updates == 1 {
                    ram::g_jam_player1_pending_animation_input(revision).write(self, 0xff);
                    ram::g_jam_player2_pending_animation_input(revision).write(self, 0xff);
                }
                0
            }
            _ => 0,
        }
    }
    fn original_entry_effects(&mut self, function: FunctionId, _: ExecutionClass) -> Result<()> {
        // These raw reads belong to the synthetic test driver, not the body.
        let return_pc = self.memory.read_u32(self.sp);
        let arguments = (0..u32::from(function.metadata().abi.argument_count))
            .map(|index| self.memory.read_u32(self.sp + 4 + index * 4))
            .collect();
        self.accesses
            .borrow_mut()
            .push(Access::Call(function, self.sp, return_pc, arguments));
        let value = self.emulate_callee(function);
        self.data[0] = value;
        self.sp += 4;
        {
            if function.metadata().abi.returns_value {
                self.set_data_register(0, value);
            }
            Ok(())
        }
    }
}

impl Bus for Machine {
    fn read_u8(&self, address: u32) -> u8 {
        self.guard.read(address, 1, Reader::RustMemory);
        let value = self.memory.read_u8(address);
        self.accesses
            .borrow_mut()
            .push(Access::Read(address, 1, u32::from(value)));
        value
    }
    fn read_u16(&self, address: u32) -> u16 {
        self.guard.read(address, 2, Reader::RustMemory);
        let value = self.memory.read_u16(address);
        self.accesses
            .borrow_mut()
            .push(Access::Read(address, 2, u32::from(value)));
        value
    }
    fn read_u32(&self, address: u32) -> u32 {
        (u32::from(self.read_u16(address)) << 16) | u32::from(self.read_u16(address + 2))
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 1, u32::from(value)));
        self.memory.write_u8(address, value);
        self.guard.commit(address, 1, true);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 2, u32::from(value)));
        self.memory.write_u16(address, value);
        self.guard.commit(address, 2, true);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.write_u16(address, (value >> 16) as u16);
        self.write_u16(address + 2, value as u16);
    }
}
impl Platform for Machine {
    fn write_vdp_control(&mut self, value: u16) {
        self.accesses.borrow_mut().push(Access::Control(value));
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.accesses.borrow_mut().push(Access::Data(value));
    }
}
impl crate::Platform for Machine {
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("unexpected direct device read")
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("unexpected direct device read")
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("unexpected direct device write")
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("unexpected direct device write")
    }
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        let _ = self.memory.read_u8(address);
        self.accesses.borrow_mut().push(Access::Discard(address, 1));
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        let _ = self.memory.read_u16(address);
        self.accesses.borrow_mut().push(Access::Discard(address, 2));
        Ok(())
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
        self.sr
    }
    fn set_status_register(&mut self, value: u16) {
        self.sr = value;
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
    fn read_uninitialized_local(&mut self, read: UninitializedLocalRead) -> Result<u32> {
        let address =
            self.guard
                .explicit_local(read, self.frame, self.sp, self.sr & 0x2000 != 0)?;
        let missing = self.guard.unwritten_mask(address, read.width.bytes());
        let value = match read.width {
            LocalWidth::Byte => u32::from(self.memory.read_u8(address)),
            LocalWidth::Word => u32::from(self.memory.read_u16(address)),
            LocalWidth::Long => self.memory.read_u32(address),
        };
        self.accesses
            .borrow_mut()
            .push(Access::Read(address, read.width.bytes(), value));
        self.declared_locals.push((read, missing, value));
        Ok(value)
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with(&[])
    }
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::TimeAware
    }
    fn enter_call_ceiling(&mut self, _: FunctionId) -> Result<CallCeilingScope> {
        Ok(CallCeilingScope::not_required())
    }
    fn call_original(&mut self, function: FunctionId, _: &[u32]) -> Result<u32> {
        panic!(
            "missing source-backed call site for {}",
            function.metadata().name
        )
    }
    fn call_original_entry(&mut self, function: FunctionId, class: ExecutionClass) -> Result<u32> {
        self.original_entry_effects(function, class)?;
        Ok(self.data_register(0))
    }
    fn call_original_entry_void(
        &mut self,
        function: FunctionId,
        class: ExecutionClass,
    ) -> Result<()> {
        self.original_entry_effects(function, class)
    }
}

#[test]
fn callbacks_read_requested_before_current_and_use_byte_bit_tests() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for counter in [0, 1] {
            let mut machine = Machine::new(revision);
            ram::g_frame_counter(revision).write(&mut machine, counter);
            let pending = if counter == 0 {
                ram::g_jam_audio_route1_pending_input(revision)
            } else {
                ram::g_jam_audio_route2_pending_input(revision)
            };
            pending.write(&mut machine, 0xffff);
            machine.clear_accesses();
            let saved = machine.address;
            machine.run(audio_callbacks::vblank, &[]);
            assert_eq!(machine.address[2..5], saved[2..5]);
            let reads = machine
                .accesses
                .borrow()
                .iter()
                .filter_map(|event| match event {
                    Access::Read(address, width, _) => Some((*address, *width)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                &reads[..2],
                &[
                    (ram::g_requested_music_selection(revision).address(), 1),
                    (ram::g_current_music_selection(revision).address(), 1),
                ]
            );
            let route_reads = reads
                .into_iter()
                .filter(|(address, _)| {
                    *address == pending.address() || *address == pending.address() + 1
                })
                .collect::<Vec<_>>();
            let mut expected = vec![(pending.address(), 2)];
            expected.extend([(pending.address() + 1, 1); 3]);
            expected.push((pending.address(), 2));
            if counter == 0 {
                expected.extend([(pending.address() + 1, 1); 5]);
            }
            assert_eq!(route_reads, expected);
            let calls = machine.calls();
            assert!(calls.iter().all(|(_, _, pc, _)| *pc != 0));
            if counter == 0 {
                let offsets = calls
                    .iter()
                    .filter(|(id, _, _, _)| *id == ids::PLAY_SOUND_EFFECT)
                    .map(|(_, _, pc, _)| pc - ids::JAM_OUT_V_BLANK.metadata().address(revision))
                    .collect::<Vec<_>>();
                assert_eq!(
                    offsets,
                    [0x136, 0x154, 0x172, 0x19c, 0x1ba, 0x1d8, 0x1f6, 0x214]
                );
            }
        }
    }
}

#[test]
fn random_template_cycle_rereads_increment_but_not_the_move_byte_result() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut machine = Machine::new(revision);
        ram::g_jam_audio_route2_pending_input(revision).write(&mut machine, 0x80);
        let cycle = ram::g_jam_template_bank_cycle_index(revision);
        cycle.write(&mut machine, 0);
        machine.clear_accesses();
        machine.run(audio_callbacks::random_sample_vblank, &[]);
        let cycle_events = machine
            .accesses
            .borrow()
            .iter()
            .filter(|event| match event {
                Access::Read(address, _, _) | Access::Write(address, _, _) => {
                    *address == cycle.address()
                }
                _ => false,
            })
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            cycle_events,
            [
                Access::Read(cycle.address(), 2, 0),
                Access::Write(cycle.address(), 2, 1),
                Access::Read(cycle.address(), 2, 1),
                Access::Read(cycle.address(), 2, 1),
                Access::Read(cycle.address(), 2, 1),
            ]
        );
        let requested_reads = machine.accesses.borrow().iter().filter(|event|
            matches!(event, Access::Read(address, 1, _) if *address == ram::g_requested_music_selection(revision).address())
        ).count();
        assert_eq!(requested_reads, 1);
        assert_eq!(ram::g_current_music_selection(revision).read(&machine), 1);
    }
}

#[test]
fn scroll_reads_each_step_before_destination_and_rereads_incremented_phase() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut machine = Machine::new(revision);
        let step = ram::g_jam_scroll_step(revision);
        let horizontal = ram::g_jam_horizontal_scroll(revision);
        let vertical = ram::g_jam_vertical_scroll(revision);
        let countdown = ram::g_jam_layout_countdown(revision);
        let phase = ram::g_jam_layout_phase(revision);
        step.write(&mut machine, -1);
        countdown.write(&mut machine, 1);
        phase.write(&mut machine, 2);
        machine.clear_accesses();
        machine.run_framed(scene_layouts::update_layouts_and_scroll, &[0]);
        let reads = machine
            .accesses
            .borrow()
            .iter()
            .filter_map(|event| match event {
                Access::Read(address, 2, _)
                    if [
                        step.address(),
                        horizontal.address(),
                        vertical.address(),
                        countdown.address(),
                        phase.address(),
                    ]
                    .contains(address) =>
                {
                    Some(*address)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            reads,
            [
                step.address(),
                step.address(),
                horizontal.address(),
                step.address(),
                vertical.address(),
                horizontal.address(),
                vertical.address(),
                countdown.address(),
                phase.address(),
                phase.address(),
                phase.address()
            ]
        );
        let table_word = ids::JAM_OUT_UPDATE_LAYOUTS_AND_SCROLL
            .metadata()
            .address(revision)
            + 0xe2
            + 6;
        assert!(
            machine
                .accesses
                .borrow()
                .contains(&Access::Read(table_word, 2, 0))
        );
    }
}

#[test]
fn layouts_keep_movem_saves_retained_areas_and_true_return_sites() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (body, root, save_bytes, first_offset, final_offset, final_slots) in [
            (
                scene_layouts::show_lead_pair_layout as TranslationFn,
                ids::JAM_OUT_SHOW_LEAD_PAIR_LAYOUT,
                8,
                0x10,
                0x4c,
                6,
            ),
            (
                scene_layouts::show_four_pair_layout,
                ids::JAM_OUT_SHOW_FOUR_PAIR_LAYOUT,
                16,
                0x22,
                0xc8,
                32,
            ),
            (
                scene_layouts::show_nine_pair_layout,
                ids::JAM_OUT_SHOW_NINE_PAIR_LAYOUT,
                8,
                0x1c,
                0x114,
                54,
            ),
        ] {
            let mut machine = Machine::new(revision);
            let saved_data = machine.data;
            let saved_address = machine.address;
            machine.run(body, &[]);
            assert_eq!(machine.data[2..], saved_data[2..]);
            assert_eq!(machine.address[2..], saved_address[2..]);
            let calls = machine.calls();
            let first_arguments = if root == ids::JAM_OUT_SHOW_NINE_PAIR_LAYOUT {
                12
            } else {
                4
            };
            assert_eq!(calls[0].1, machine.sp - save_bytes - first_arguments - 4);
            assert_eq!(calls[0].2, root.metadata().address(revision) + first_offset);
            let last = calls.last().unwrap();
            assert_eq!(last.1, machine.sp - save_bytes - final_slots * 4 - 4);
            assert_eq!(last.2, root.metadata().address(revision) + final_offset);
            if root == ids::JAM_OUT_SHOW_LEAD_PAIR_LAYOUT {
                let map = rom::crash_packed_tilemap_a(revision).address();
                let count = machine.accesses.borrow().iter().filter(|event|
                    matches!(event, Access::Read(address, 1, _) if (map..map+352).contains(address))
                ).count();
                assert_eq!(count, 352);
            }
        }
    }
}

#[test]
fn start_keeps_consecutive_hold_mode_rereads_and_retained_update_arguments() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for song in [0, 1, 7] {
            let mut machine = Machine::new(revision);
            machine.modal = true;
            let data = machine.data;
            let address = machine.address;
            machine.run_framed(lifecycle::start_jam_out, &[song]);
            assert_eq!(
                machine.updates, 56,
                "release on update five resets the hold"
            );
            assert_eq!(machine.data[2..], data[2..]);
            assert_eq!(machine.address[2..], address[2..]);
            let mode = ram::g_mode2_reg_softcopy(revision).address();
            let mode_events = machine
                .accesses
                .borrow()
                .iter()
                .filter(|event| match event {
                    Access::Read(at, _, _) | Access::Write(at, _, _) => *at == mode,
                    _ => false,
                })
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(
                mode_events,
                [
                    Access::Read(mode, 1, 0),
                    Access::Write(mode, 1, 0),
                    Access::Read(mode, 1, 0),
                    Access::Read(mode, 1, 0),
                    Access::Write(mode, 1, 0x40),
                    Access::Read(mode, 1, 0x40),
                    Access::Read(mode, 1, 0x40),
                    Access::Write(mode, 1, 0),
                    Access::Read(mode, 1, 0),
                ]
            );
            let calls = machine.calls();
            let root = ids::START_JAM_OUT.metadata().address(revision);
            let cadence = calls
                .iter()
                .find(|(id, _, _, _)| *id == ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE)
                .unwrap();
            assert_eq!(
                (cadence.1, cadence.2),
                (machine.sp - 24 - 8 - 4, root + 0x23c)
            );
            let layout = calls
                .iter()
                .find(|(_, _, pc, _)| *pc == root + 0x242)
                .unwrap();
            assert_eq!(layout.1, machine.sp - 24 - 12 - 4);
            let clear_lower = calls
                .iter()
                .find(|(id, _, _, _)| *id == ids::CLEAR_LOWER_VRAM_TILEMAP)
                .unwrap();
            assert_eq!(clear_lower.1, machine.sp - 24 - 4 - 4);
            assert_eq!(ram::g_v_blank_func(revision).read(&machine).address(), 0);
            assert_eq!(
                ram::g_shared_tile_sprite_copies_enabled(revision).read(&machine),
                0
            );
        }
    }
}

#[test]
fn sample_modulo_calls_lmodt_only_on_actual_divs_overflow() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for value in [
            0,
            -1,
            42 * 32767,
            -42 * 32768,
            42 * 32768,
            -42 * 32769,
            i32::MIN,
            i32::MAX,
        ] {
            let mut machine = Machine::new(revision);
            machine.random.push_back(value);
            let result =
                frame_support::random_sample_index(&mut machine, ids::START_JAM_OUT, 0x102)
                    .unwrap();
            assert_eq!(result, (value % 42) as u16);
            let calls = machine.calls();
            assert_eq!(calls[0].0, ids::GET_RANDOM_NUMBER);
            let overflow = i16::try_from(value / 42).is_err();
            assert_eq!(calls.len(), if overflow { 2 } else { 1 });
            if overflow {
                assert_eq!(calls[1].0, ids::LMODT);
                assert_eq!(
                    calls[1].2,
                    ids::START_JAM_OUT.metadata().address(revision) + 0x10e
                );
            }
        }
    }
}

#[test]
fn clr_reads_are_discarded_and_long_transfers_keep_word_beat_order() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for ccr in 0..32 {
            let mut machine = Machine::new(revision);
            machine.sr = 0xa700 | ccr;
            frame_support::clear_long(&mut machine, 0xff0100).unwrap();
            assert_eq!(
                &*machine.accesses.borrow(),
                &[
                    Access::Discard(0xff0100, 2),
                    Access::Discard(0xff0102, 2),
                    Access::Write(0xff0100, 2, 0),
                    Access::Write(0xff0102, 2, 0),
                ]
            );
            assert_eq!(machine.sr, 0xa704 | (ccr & 0x10));
        }
    }
}

#[test]
fn callback_exit_ccr_is_its_own_final_compare_or_counter_subtraction() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for body in [
            audio_callbacks::vblank as TranslationFn,
            audio_callbacks::random_sample_vblank,
        ] {
            for selection in [0_u8, 1, 2, 0x80, 0xff] {
                for counter in [0_u16, 0x7ffe, 0xfffe, 0xffff] {
                    let mut machine = Machine::new(revision);
                    machine.sr = 0xa71f;
                    ram::g_player_selection(revision).write(&mut machine, selection as i8);
                    ram::g_frame_counter(revision).write(&mut machine, counter);
                    machine.run(body, &[]);
                    let (left, sign, extend) = if selection == 1 {
                        (u32::from(counter.wrapping_add(2)), 0x8000, true)
                    } else {
                        (u32::from(selection), 0x80, false)
                    };
                    let result = if extend {
                        left.wrapping_sub(1) & 0xffff
                    } else {
                        left.wrapping_sub(1) & 0xff
                    };
                    let borrow = left == 0;
                    let overflow = ((left ^ 1) & (left ^ result) & sign) != 0;
                    let x = if extend { borrow } else { counter == 0xffff };
                    let expected = (u16::from(x) << 4)
                        | (u16::from(result & sign != 0) << 3)
                        | (u16::from(result == 0) << 2)
                        | (u16::from(overflow) << 1)
                        | u16::from(borrow);
                    assert_eq!(machine.sr, 0xa700 | expected);
                }
            }
        }
    }
}

#[test]
fn random_bank_counter_wrap_retains_addq_extend_through_clears_and_final_compare() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut machine = Machine::new(revision);
        ram::g_jam_audio_route2_pending_input(revision).write(&mut machine, 0x80);
        ram::g_jam_template_bank_cycle_index(revision).write(&mut machine, -1);
        machine.run(audio_callbacks::random_sample_vblank, &[]);
        assert_eq!(machine.sr & 0x1f, 0x19);
        let calls = machine.calls();
        assert!(
            calls
                .iter()
                .any(|(id, _, _, _)| *id == ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE)
        );
        assert!(
            calls
                .iter()
                .any(|(id, _, _, _)| *id == ids::MUSIC_REQUEST_TRACK_POINTER_RESET)
        );
    }
}

#[test]
fn missing_jam_leads_declare_each_tile_read_only_until_a_lead_stores_it() {
    // The allocator results are authored callee doubles, not a claim that a
    // natural allocation trajectory can fail and then succeed without a free.
    for revision in Revision::ALL {
        for scenario in 0..3 {
            let mut c = Machine::new(revision);
            c.sp = 0xff7000;
            c.frame = FrameState::new(c.sp);
            let entry_sp = c.sp;
            let residue = 0x55aa;
            c.memory.write_u16(entry_sp - 6, residue);
            let mut allocations = [0; 18];
            match scenario {
                0 => {
                    allocations[1] = 2;
                    allocations[2] = 3;
                    allocations[9] = 4;
                    allocations[10] = 5;
                }
                1 => {
                    allocations[0] = 1;
                    allocations[1] = 2;
                    allocations[10] = 5;
                }
                _ => {
                    allocations[10] = 2;
                    allocations[11] = 3;
                }
            }
            c.allocations = allocations.into_iter().collect();
            let guard = Arc::clone(&c.guard);
            guard
                .run_root(
                    ids::JAM_OUT_CREATE_CHARACTER_COPIES,
                    entry_sp,
                    c.sr & 0x2000 != 0,
                    || {
                        c.run(super::entities::create_character_copies, &[]);
                    },
                )
                .unwrap();
            assert!(guard.failure().is_none());
            assert!(c.allocations.is_empty());
            assert_eq!(c.declared_locals.len(), if scenario == 1 { 0 } else { 2 });
            for &(read, missing, value) in &c.declared_locals {
                assert_eq!(read.owner, ids::JAM_OUT_CREATE_CHARACTER_COPIES);
                assert_eq!(read.frame.entry_sp, entry_sp);
                assert_eq!((read.offset, read.width), (-6, LocalWidth::Word));
                assert!(read.citation.contains(if scenario == 0 {
                    "1115-1155"
                } else {
                    "1164-1204"
                }));
                assert_eq!((missing, value), (3, u32::from(residue)));
            }
            let expected = if scenario == 1 { 0x8000 } else { residue };
            for object in [2, 3] {
                if scenario != 1 || object == 2 {
                    assert_eq!(
                        ram::g_player_info(revision)
                            .at(object)
                            .tile()
                            .read(&c.memory),
                        expected
                    );
                }
            }
            if scenario != 2 {
                assert_eq!(
                    ram::g_player_info(revision).at(5).tile().read(&c.memory),
                    0x8000
                );
            }
            assert_eq!(
                guard.unwritten_mask(entry_sp - 6, 2),
                if scenario == 2 { 3 } else { 0 }
            );
            assert_eq!(c.sp, entry_sp);
        }
    }
}

#[test]
fn undeclared_jam_lead_tile_still_trips_the_guard() {
    for revision in Revision::ALL {
        let mut c = Machine::new(revision);
        c.sp = 0xff7000;
        c.frame = FrameState::new(c.sp);
        let entry_sp = c.sp;
        c.memory.write_u16(entry_sp - 6, 0x55aa);
        let guard = Arc::clone(&c.guard);
        let failure = guard
            .run_root(
                ids::JAM_OUT_CREATE_CHARACTER_COPIES,
                entry_sp,
                c.sr & 0x2000 != 0,
                || {
                    let frame = crate::call_frame::LocalFrame::enter(
                        &mut c,
                        crate::call_frame::FrameLayout::link(4, 0x7c, 0x1c),
                    )
                    .unwrap();
                    frame.local::<u16>(-2).read(&c)
                },
            )
            .unwrap_err();
        assert_eq!(failure.root, ids::JAM_OUT_CREATE_CHARACTER_COPIES);
        assert_eq!((failure.address, failure.width), (entry_sp - 6, 2));
        assert_eq!(
            failure.reason,
            crate::stack_read::UnsupportedReason::Unwritten
        );
        assert!(c.declared_locals.is_empty());
    }
}
