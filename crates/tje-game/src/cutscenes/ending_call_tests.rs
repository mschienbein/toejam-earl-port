//! ROM-free caller-ABI regressions with controlled callees, not ROM equivalence.
//! Source callsites are from matching REV00/02 c_ending.o/c_ending.s. The cadence
//! and sprite-service probes finish one script iteration so only caller setup,
//! retained slots, return PCs, frame restoration and SP balance are exercised.

use super::*;
use crate::execution::{CallCeilingScope, ExecutionClass};
use crate::stack_read::{LocalWidth, UninitializedLocalRead, Unsupported, UnsupportedReason};
use crate::{Dispatcher, FramedTranslation, FunctionId, call_frame::FrameState};
use std::cell::RefCell;
use tje_mem::{AddressSpace, Bus, Revision};

#[derive(Debug, PartialEq, Eq)]
struct PreparedCall {
    id: FunctionId,
    entry_sp: u32,
    return_pc: u32,
    arguments: [u32; 2],
    count: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum MovementAccess {
    Explicit(u16),
    Read(u16),
    Write(u16),
}

struct ResidualProbe {
    address: u32,
    reject: bool,
    declarations: Vec<UninitializedLocalRead>,
    accesses: RefCell<Vec<MovementAccess>>,
}

impl ResidualProbe {
    fn new(address: u32, reject: bool) -> Self {
        Self {
            address,
            reject,
            declarations: Vec::with_capacity(1),
            accesses: RefCell::new(Vec::with_capacity(6)),
        }
    }
}

struct Machine {
    memory: AddressSpace,
    data: [u32; 8],
    address: [u32; 7],
    sp: u32,
    frame: FrameState,
    class: ExecutionClass,
    sr: u16,
    calls: Vec<PreparedCall>,
    allocations: Vec<(u32, u32)>,
    discarded: Vec<(u8, u32)>,
    state_read_stacks: RefCell<Vec<u32>>,
    residual: Option<ResidualProbe>,
    fail_bounce_entry: bool,
}

impl Machine {
    fn new(revision: Revision, sp: u32) -> Self {
        Self {
            memory: AddressSpace::new(vec![0; 0x100000], revision),
            data: std::array::from_fn(|i| 0x1100_0000 + i as u32),
            address: std::array::from_fn(|i| 0x2200_0000 + i as u32 * 0x100),
            sp,
            frame: FrameState::new(sp),
            class: ExecutionClass::TimeAware,
            sr: 0x2000,
            calls: Vec::with_capacity(3),
            allocations: Vec::with_capacity(1),
            discarded: Vec::with_capacity(4),
            state_read_stacks: RefCell::new(Vec::with_capacity(3)),
            residual: None,
            fail_bounce_entry: false,
        }
    }

    fn capture(&mut self, id: FunctionId, count: usize) {
        let mut arguments = [0; 2];
        for (index, argument) in arguments[..count].iter_mut().enumerate() {
            *argument = self.frame.argument_slot(index as i32).read(&self.memory);
        }
        self.calls.push(PreparedCall {
            id,
            entry_sp: self.sp,
            return_pc: self.memory.read_u32(self.sp),
            arguments,
            count,
        });
    }
}

impl Bus for Machine {
    fn read_u8(&self, address: u32) -> u8 {
        self.memory.read_u8(address)
    }
    fn read_u16(&self, address: u32) -> u16 {
        if address == ram::g_cutscene_script_state(self.memory.revision()).address() {
            self.state_read_stacks.borrow_mut().push(self.sp);
        }
        let value = self.memory.read_u16(address);
        if let Some(probe) = &self.residual
            && address == probe.address
        {
            probe
                .accesses
                .borrow_mut()
                .push(MovementAccess::Read(value));
        }
        value
    }
    fn read_u32(&self, address: u32) -> u32 {
        self.memory.read_u32(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.memory.write_u16(address, value);
        if let Some(probe) = &self.residual
            && address == probe.address
        {
            probe
                .accesses
                .borrow_mut()
                .push(MovementAccess::Write(value));
        }
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.memory.write_u32(address, value);
    }
}

impl tje_mem::Platform for Machine {
    fn write_vdp_control(&mut self, _: u16) {}
    fn write_vdp_data(&mut self, _: u16) {}
}

impl crate::Platform for Machine {
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.discarded.push((1, address));
        let _ = self.memory.read_u8(address);
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.discarded.push((2, address));
        let _ = self.memory.read_u16(address);
        Ok(())
    }
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("unexpected device read");
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("unexpected device read");
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("unexpected device write");
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("unexpected device write");
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
    fn execution_class(&self) -> ExecutionClass {
        self.class
    }
    fn set_execution_class(&mut self, value: ExecutionClass) -> Result<()> {
        self.class = value;
        Ok(())
    }
    fn enter_call_ceiling(&mut self, _: FunctionId) -> Result<CallCeilingScope> {
        Ok(CallCeilingScope::not_required())
    }
    fn leave_call_ceiling(&mut self, _: CallCeilingScope) -> Result<()> {
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with_frames(&[], PROBES)
    }

    fn read_uninitialized_local(&mut self, read: UninitializedLocalRead) -> Result<u32> {
        let Some(probe) = &mut self.residual else {
            return Err(Unsupported::unavailable(read).into());
        };
        assert_eq!(read.owner, ids::ENDING_PLAY_BOUNCING_ROCKETSHIP_SCENE);
        assert_eq!(read.frame, self.frame);
        assert_eq!(read.address(), probe.address);
        assert_eq!((read.offset, read.width), (-6, LocalWidth::Word));
        assert!(read.address() >= self.sp && read.address() + 2 <= self.frame.entry_sp);
        assert!(read.citation.contains("1193-1194"));
        probe.declarations.push(read);
        if probe.reject {
            return Err(Unsupported::paired(
                read.owner,
                read.frame.entry_sp,
                read,
                UnsupportedReason::MissingOriginalFrameWitness,
            )
            .into());
        }
        let value = self.memory.read_u16(read.address());
        probe
            .accesses
            .borrow_mut()
            .push(MovementAccess::Explicit(value));
        Ok(u32::from(value))
    }

    fn call_original_entry(&mut self, id: FunctionId, _: ExecutionClass) -> Result<u32> {
        if self.fail_bounce_entry && id == ids::ENDING_PLAY_BOUNCING_ROCKETSHIP_SCENE {
            let read = self.frame.uninitialized_local(
                id,
                -6,
                LocalWidth::Word,
                "controlled child failure; no original witness is supplied",
            );
            return Err(Unsupported::paired(
                id,
                self.frame.entry_sp,
                read,
                UnsupportedReason::MissingOriginalFrameWitness,
            )
            .into());
        }
        self.sp += 4;
        Ok(0)
    }

    fn call_original(&mut self, id: FunctionId, _: &[u32]) -> Result<u32> {
        assert!(self.frame.is_prepared_for(id));
        assert_eq!(self.frame.entry_sp, self.sp);
        assert_eq!(id, ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE);
        self.capture(id, 2);
        ram::g_cutscene_script_state(self.revision()).write(self, -1);
        Ok(1)
    }

    fn call_original_void_with_execution(
        &mut self,
        id: FunctionId,
        _: &[u32],
        _: ExecutionClass,
    ) -> Result<()> {
        if id == ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER {
            assert!(self.frame.is_prepared_for(id));
            assert_eq!(self.frame.entry_sp, self.sp);
            self.capture(id, 1);
        }
        if id == ids::CUTSCENE_ALLOCATE_ROCKETSHIP_PAIR {
            self.allocations
                .push((self.sp, self.memory.read_u32(self.sp)));
        }
        if id == ids::UPDATE_AND_QUEUE_ENTITY_SPRITES {
            ram::g_cutscene_script_state(self.revision()).write(self, -1);
        }
        Ok(())
    }

    fn call_original_entry_void(&mut self, id: FunctionId, class: ExecutionClass) -> Result<()> {
        assert_eq!(self.frame.entry_sp, self.sp);
        if id == ids::CLEAR_LOWER_VRAM_TILEMAP {
            // Dialogue retains cadence arguments until original ADDQ.L #8.
            self.capture(id, 2);
        }
        self.call_original_void_with_execution(id, &[], class)?;
        self.sp += 4;
        Ok(())
    }
}

fn border_probe(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = context.frame_state().unwrap();
    assert_eq!(frame.argument_word(0).read(context), 1);
    let class = context.execution_class();
    context.call_original_void_with_execution(
        ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
        &[],
        class,
    )?;
    Ok(None)
}

fn cadence_probe(context: &mut dyn Context) -> Result<Option<u32>> {
    let frame = context.frame_state().unwrap();
    assert_eq!(frame.argument_word(0).read(context), 0);
    assert_eq!(frame.argument_word(1).read(context), 0);
    context.call_original(ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE, &[])?;
    Ok(Some(1))
}

static PROBES: &[FramedTranslation] = &[
    FramedTranslation {
        id: ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
        function: border_probe,
    },
    FramedTranslation {
        id: ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
        function: cadence_probe,
    },
];

struct CallerCase {
    caller: FunctionId,
    body: crate::TranslationFn,
    saved_bytes: u32,
    calls: &'static [(FunctionId, u32, u32)],
}

#[test]
fn ending_callers_prepare_source_frames_and_retained_slots_in_both_revisions() {
    const SP: u32 = 0xff7f00;
    let cases = [
        CallerCase {
            caller: ids::ENDING_PLAY_DESCENDING_ROCKETSHIP_SCENE,
            body: ending_play_descending_rocketship_scene,
            saved_bytes: 24,
            calls: &[
                (
                    ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
                    0xc6a - 0xc50,
                    SP - 32,
                ),
                (
                    ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
                    0xcea - 0xc50,
                    SP - 36,
                ),
                (
                    ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
                    0xdcc - 0xc50,
                    SP - 36,
                ),
            ],
        },
        CallerCase {
            caller: ids::ENDING_PLAY_UPWARD_ROCKETSHIP_DEPARTURE,
            body: ending_play_upward_rocketship_departure,
            saved_bytes: 24,
            calls: &[
                (
                    ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
                    0x5ac - 0x592,
                    SP - 32,
                ),
                (
                    ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
                    0x640 - 0x592,
                    SP - 36,
                ),
                (
                    ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
                    0x770 - 0x592,
                    SP - 36,
                ),
            ],
        },
        CallerCase {
            caller: ids::ENDING_PLAY_SCROLLING_CREDITS,
            body: ending_play_scrolling_credits,
            saved_bytes: 20,
            calls: &[
                (
                    ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER,
                    0x7c2 - 0x790,
                    SP - 28,
                ),
                (
                    ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
                    0xa8a - 0x790,
                    SP - 32,
                ),
            ],
        },
        CallerCase {
            caller: ids::ENDING_PLAY_DIALOGUE_AND_DEPARTURE,
            body: ending_play_dialogue_and_departure,
            saved_bytes: 32,
            calls: &[
                (
                    ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE,
                    0x522 - 0x158,
                    SP - 44,
                ),
                (ids::CLEAR_LOWER_VRAM_TILEMAP, 0x528 - 0x158, SP - 44),
            ],
        },
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut machine = Machine::new(revision, SP);
        for CallerCase {
            caller,
            body,
            saved_bytes,
            calls: expected,
        } in &cases
        {
            machine.sp = SP;
            machine.frame = FrameState::new(SP);
            machine.memory.write_u32(SP, 0x123456);
            machine.calls.clear();
            machine.allocations.clear();
            machine.discarded.clear();
            machine.state_read_stacks.get_mut().clear();
            let data = machine.data;
            let address = machine.address;
            assert_eq!(body(&mut machine, &[]).unwrap(), Some(0));
            assert_eq!(machine.sp, SP, "{}", caller.metadata().name);
            assert_eq!(machine.frame, FrameState::new(SP));
            assert_eq!(machine.data[2..], data[2..]);
            assert_eq!(machine.address, address);
            assert!(machine.discarded.contains(&(2, SP)));
            assert_eq!(machine.calls.len(), expected.len());
            for (actual, &(id, offset, entry_sp)) in machine.calls.iter().zip(expected.iter()) {
                assert_eq!(actual.id, id);
                assert_eq!(actual.entry_sp, entry_sp);
                assert_eq!(
                    actual.return_pc,
                    caller.metadata().address(revision) + offset
                );
                if id == ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER {
                    assert_eq!((actual.arguments, actual.count), ([1, 0], 1));
                } else {
                    assert_eq!((actual.arguments, actual.count), ([0, 0], 2));
                }
            }
            assert_eq!(machine.allocations.len(), 1);
            let retains_border = matches!(
                *caller,
                ids::ENDING_PLAY_DESCENDING_ROCKETSHIP_SCENE
                    | ids::ENDING_PLAY_UPWARD_ROCKETSHIP_DEPARTURE
            );
            let expected_allocation_sp = SP - *saved_bytes - if retains_border { 4 } else { 0 };
            if retains_border {
                let base = SP - *saved_bytes;
                assert_eq!(
                    machine.state_read_stacks.borrow().as_slice(),
                    &[base, base - 8, base]
                );
            }
            assert_eq!(machine.allocations[0].0, expected_allocation_sp);
            if expected_allocation_sp == SP - *saved_bytes - 4 {
                assert_eq!(machine.allocations[0].1, 1);
            }
        }
    }
}

#[test]
fn movement_declares_only_the_first_rmw_input_then_reads_own_written_words() {
    const SP: u32 = 0xff7f00;
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut machine = Machine::new(revision, SP);
        machine.residual = Some(ResidualProbe::new(SP - 6, false));
        for old in [0_u16, 0x7fff, 0x8000, 0xfffe, 0xffff] {
            machine.memory.write_u16(SP - 6, old);
            machine.discarded.clear();
            let probe = machine.residual.as_mut().unwrap();
            probe.declarations.clear();
            probe.accesses.get_mut().clear();
            let frame = LocalFrame::enter(&mut machine, FrameLayout::link(4, 0x7c, 0x1c)).unwrap();
            let word = frame.local::<i16>(-2);
            let mut first = true;
            let next = old.wrapping_add(1);
            assert_eq!(
                increment_movement_frame(&mut machine, word, &mut first).unwrap(),
                next as i16
            );
            assert!(!first);
            let again = next.wrapping_add(1);
            assert_eq!(
                increment_movement_frame(&mut machine, word, &mut first).unwrap(),
                again as i16
            );
            let probe = machine.residual.as_ref().unwrap();
            assert_eq!(probe.declarations.len(), 1);
            assert_eq!(
                probe.accesses.borrow().as_slice(),
                &[
                    MovementAccess::Explicit(old),
                    MovementAccess::Write(next),
                    MovementAccess::Read(next),
                    MovementAccess::Read(next),
                    MovementAccess::Write(again),
                    MovementAccess::Read(again),
                ]
            );
            frame.leave(&mut machine).unwrap();
        }
    }
}

#[test]
fn rejected_first_movement_input_neither_fetches_nor_initializes_the_word() {
    const SP: u32 = 0xff7f00;
    let mut machine = Machine::new(Revision::Rev02, SP);
    machine.memory.write_u16(SP - 6, 0x8123);
    machine.residual = Some(ResidualProbe::new(SP - 6, true));
    let frame = LocalFrame::enter(&mut machine, FrameLayout::link(4, 0x7c, 0x1c)).unwrap();
    let mut first = true;
    let error =
        increment_movement_frame(&mut machine, frame.local::<i16>(-2), &mut first).unwrap_err();
    assert_eq!(
        error.unsupported_stack_read().unwrap().reason,
        UnsupportedReason::MissingOriginalFrameWitness
    );
    assert!(error.preserves_call_frame());
    assert!(first);
    assert_eq!(machine.memory.read_u16(SP - 6), 0x8123);
    let probe = machine.residual.as_ref().unwrap();
    assert_eq!(probe.declarations.len(), 1);
    assert!(probe.accesses.borrow().is_empty());
    assert_eq!(machine.sp, frame.save_base());
}

#[test]
fn bouncing_wrapper_keeps_the_allocated_frame_on_missing_first_input_witness() {
    const SP: u32 = 0xff7f00;
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut machine = Machine::new(revision, SP);
        machine.memory.write_u16(SP - 6, 0xa123);
        machine.residual = Some(ResidualProbe::new(SP - 6, true));
        let error = ending_play_bouncing_rocketship_scene(&mut machine, &[]).unwrap_err();
        let failure = error.unsupported_stack_read().unwrap();
        assert_eq!(failure.root, ids::ENDING_PLAY_BOUNCING_ROCKETSHIP_SCENE);
        assert_eq!(
            failure.reason,
            UnsupportedReason::MissingOriginalFrameWitness
        );
        assert_eq!(machine.sp, SP - 40);
        assert_eq!(machine.address[6], SP - 4);
        assert_eq!(machine.memory.read_u16(SP - 6), 0xa123);
        let probe = machine.residual.as_ref().unwrap();
        assert_eq!(probe.declarations.len(), 1);
        assert!(probe.accesses.borrow().is_empty());
    }
}

#[test]
fn sequence_wrapper_never_repairs_a_frame_preserving_failed_child() {
    const SP: u32 = 0xff7f00;
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut machine = Machine::new(revision, SP);
        machine.fail_bounce_entry = true;
        let error = play_ending_sequence(&mut machine, &[]).unwrap_err();
        let failure = error.unsupported_stack_read().unwrap();
        assert_eq!(failure.root, ids::ENDING_PLAY_BOUNCING_ROCKETSHIP_SCENE);
        assert_eq!(
            failure.reason,
            UnsupportedReason::MissingOriginalFrameWitness
        );
        // Parent saves12, retains40argument bytes, then pushes the actual RA.
        assert_eq!(failure.entry_sp, SP - 56);
        assert_eq!(machine.sp, failure.entry_sp);
        assert_eq!(machine.frame.entry_sp, failure.entry_sp);
    }
}

#[test]
fn owned_replay_roots_keep_their_original_classes_and_zero_argument_abi() {
    for id in [
        ids::PLAY_ENDING_SEQUENCE,
        ids::ENDING_PLAY_DIALOGUE_AND_DEPARTURE,
        ids::ENDING_PLAY_UPWARD_ROCKETSHIP_DEPARTURE,
        ids::ENDING_PLAY_SCROLLING_CREDITS,
        ids::ENDING_PLAY_DESCENDING_ROCKETSHIP_SCENE,
        ids::ENDING_PLAY_BOUNCING_ROCKETSHIP_SCENE,
        ids::FUNKATRON_UPDATE_SCENE_ENTITIES,
    ] {
        let contract = super::super::EXECUTION_CONTRACTS
            .iter()
            .find(|contract| contract.function == id)
            .unwrap();
        assert_eq!(contract.class, ExecutionClass::TimeAware);
        assert_eq!(
            contract.proof_envelope,
            crate::execution::ProofEnvelope::ReplayedEnvironment
        );
        assert_eq!(id.metadata().abi.argument_count, 0);
    }
}
