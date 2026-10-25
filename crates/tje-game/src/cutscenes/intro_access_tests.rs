//! ROM-free access regressions. Callees are explicit test doubles: these tests
//! exercise the owning intro instructions, not emulator or replay-tier proofs.
use super::*;
use crate::{Dispatcher, FunctionId, call_frame::FrameState, execution::ExecutionClass};
use std::cell::RefCell;
use tje_mem::{AddressSpace, Bus, Revision};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    Read(u32, u8, u32),
    Write(u32, u8, u32),
    Discard(u32, u8),
}

struct AuditContext {
    memory: AddressSpace,
    accesses: RefCell<Vec<Access>>,
    data: [u32; 8],
    address: [u32; 7],
    status: u16,
    interrupted_part: Option<FunctionId>,
    calls: Vec<FunctionId>,
    class: ExecutionClass,
    service_calls: usize,
    pad_reads: usize,
    action_held: bool,
    fail_discard: bool,
    frame: Option<FrameState>,
    sp: u32,
    prepared_calls: Vec<(FunctionId, Vec<u32>, u32)>,
    replace_background_on_allocate: Option<i16>,
    observed_allocation_d2: Option<u32>,
    start_samples: usize,
}

impl AuditContext {
    fn new(revision: Revision, action_held: bool) -> Self {
        Self {
            memory: AddressSpace::new(vec![0; 0x10_0000], revision),
            accesses: RefCell::new(Vec::new()),
            data: [0; 8],
            address: [0; 7],
            status: 0x2000,
            interrupted_part: None,
            calls: Vec::new(),
            class: ExecutionClass::TimeAware,
            service_calls: 0,
            action_held,
            pad_reads: 0,
            fail_discard: false,
            frame: Some(FrameState::new(0xfff000)),
            sp: 0xfff000,
            prepared_calls: Vec::new(),
            replace_background_on_allocate: None,
            observed_allocation_d2: None,
            start_samples: 0,
        }
    }

    fn at(&self, address: u32) -> Vec<Access> {
        self.accesses
            .borrow()
            .iter()
            .copied()
            .filter(|access| match access {
                Access::Read(raw, _, _) | Access::Write(raw, _, _) | Access::Discard(raw, _) => {
                    *raw == address
                }
            })
            .collect()
    }
    fn original_effects(&mut self, function: FunctionId, _: &[u32]) -> Result<()> {
        self.calls.push(function);
        if function == ids::INTRO_ALLOCATE_LARGE_PLAYER_PARTS {
            self.observed_allocation_d2 = Some(self.data[2]);
        }
        if function == ids::INTRO_ALLOCATE_LARGE_PLAYER_PARTS
            && let Some(value) = self.replace_background_on_allocate
        {
            self.frame
                .unwrap()
                .argument_word(0)
                .write(&mut self.memory, value);
        }
        if self.interrupted_part == Some(function) {
            return {
                let value = 1;
                if function.metadata().abi.returns_value {
                    self.set_data_register(0, value);
                }
                Ok(())
            };
        }
        if function == ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE
            || function == ids::WAIT_FOR_NEXT_V_BLANK
        {
            // Accelerate the ROM-free state-machine test, never production code.
            self.service_calls += 1;
            assert!(self.service_calls < 500, "intro test did not terminate");
            ram::g_cutscene_script_countdown(self.revision()).write(&mut self.memory, 1);
        }
        if function == ids::GET_CTRL2_INPUT {
            self.pad_reads += 1;
        }
        {
            let value = if function == ids::GET_CTRL2_INPUT && self.start_samples != 0 {
                self.start_samples -= 1;
                0x80
            } else if function == ids::GET_CTRL2_INPUT
                && self.action_held
                && self.pad_reads % 3 == 2
            {
                0x10
            } else {
                0
            };
            if function.metadata().abi.returns_value {
                self.set_data_register(0, value);
            }
            Ok(())
        }
    }
}

impl Bus for AuditContext {
    fn read_u8(&self, address: u32) -> u8 {
        let value = self.memory.read_u8(address);
        self.accesses
            .borrow_mut()
            .push(Access::Read(address, 1, u32::from(value)));
        value
    }

    fn read_u16(&self, address: u32) -> u16 {
        let value = self.memory.read_u16(address);
        self.accesses
            .borrow_mut()
            .push(Access::Read(address, 2, u32::from(value)));
        value
    }

    fn write_u8(&mut self, address: u32, value: u8) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 1, u32::from(value)));
        self.memory.write_u8(address, value);
    }

    fn write_u16(&mut self, address: u32, value: u16) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 2, u32::from(value)));
        self.memory.write_u16(address, value);
    }
}

impl tje_mem::Platform for AuditContext {
    fn write_vdp_control(&mut self, _: u16) {}
    fn write_vdp_data(&mut self, _: u16) {}
}

impl crate::Platform for AuditContext {
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.accesses.borrow_mut().push(Access::Discard(address, 1));
        if self.fail_discard {
            Err(crate::Error::Platform("test discarded read failed".into()))
        } else {
            Ok(())
        }
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.accesses.borrow_mut().push(Access::Discard(address, 2));
        if self.fail_discard {
            Err(crate::Error::Platform("test discarded read failed".into()))
        } else {
            Ok(())
        }
    }
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("intro body has no direct device reads");
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("intro body has no direct device reads");
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("intro body has no byte device writes");
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {}
}

impl Context for AuditContext {
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
        assert_eq!(value & !0x001f, self.status & !0x001f);
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
        self.frame
    }
    fn set_frame_state(&mut self, frame: FrameState) -> Result<()> {
        self.frame = Some(frame);
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with_frames(&[], &[])
    }
    fn execution_class(&self) -> ExecutionClass {
        self.class
    }
    fn set_execution_class(&mut self, class: ExecutionClass) -> Result<()> {
        self.class = class;
        Ok(())
    }
    fn enter_call_ceiling(&mut self, _: FunctionId) -> Result<crate::execution::CallCeilingScope> {
        // Every callee here is a test double, not a time-progressing emulator.
        Ok(crate::execution::CallCeilingScope::not_required())
    }
    fn leave_call_ceiling(&mut self, scope: crate::execution::CallCeilingScope) -> Result<()> {
        assert!(!scope.is_required());
        Ok(())
    }
    fn call_original_with_execution(
        &mut self,
        function: FunctionId,
        arguments: &[u32],
        _: ExecutionClass,
    ) -> Result<u32> {
        // Unit doubles cover every callee class without executing original code.
        self.call_original(function, arguments)
    }
    fn call_original(&mut self, function: FunctionId, arguments: &[u32]) -> Result<u32> {
        self.original_effects(function, arguments)?;
        Ok(self.data_register(0))
    }
    fn call_original_void_with_execution(
        &mut self,
        function: FunctionId,
        arguments: &[u32],
        _class: crate::execution::ExecutionClass,
    ) -> Result<()> {
        self.original_effects(function, arguments)
    }
    fn call_original_entry_void(&mut self, function: FunctionId, _: ExecutionClass) -> Result<()> {
        let entry = self.frame.unwrap();
        let arguments: Vec<_> = (0..function.metadata().abi.argument_count)
            .map(|index| entry.argument_slot(i32::from(index)).read(&self.memory))
            .collect();
        let return_pc = self.memory.read_u32(entry.entry_sp);
        self.prepared_calls
            .push((function, arguments.clone(), return_pc));
        self.original_effects(function, &arguments)?;
        self.sp = self.sp.wrapping_add(4);
        Ok(())
    }
    fn call_original_entry(&mut self, function: FunctionId, class: ExecutionClass) -> Result<u32> {
        self.call_original_entry_void(function, class)?;
        Ok(self.data_register(0))
    }
}

#[test]
fn action_assignment_branches_from_move_flags_then_only_guard_reloads() {
    // G accepted c_intro.s: MOVE.W D2,gCutsceneAdvanceButtonMask; BEQ;
    // ...; TST.W gCutsceneAdvanceButtonMask; BNE. No IF operand reload.
    for revision in [Revision::Rev02, Revision::Rev00] {
        for held in [false, true] {
            for function in [intro_part1, intro_part2, intro_part3, intro_part4] {
                let mut context = AuditContext::new(revision, held);
                assert_eq!(function(&mut context, &[]).unwrap(), Some(0));
                let mask = ram::g_cutscene_advance_button_mask(revision).address();
                let accesses = context.at(mask);
                assert!(!accesses.is_empty());
                assert_eq!(accesses.len() % 2, 0);
                if held {
                    assert!(accesses.iter().any(|access| {
                        matches!(access, Access::Write(_, 2, value) if *value != 0)
                    }));
                }
                for pair in accesses.as_chunks::<2>().0 {
                    let Access::Write(address, 2, value) = pair[0] else {
                        panic!("assignment must precede guard: {pair:?}");
                    };
                    assert_eq!(pair[1], Access::Read(address, 2, value));
                }
            }
        }
    }
}

#[test]
fn countdown_switches_reload_the_incremented_state_word() {
    // G accepted c_intro.s: ADDQ.W #1,state; MOVE.W state,D0. Test the
    // actual seven scene bodies so a caller cannot bypass the reload helper.
    for revision in [Revision::Rev02, Revision::Rev00] {
        for function in [
            intro_part2,
            intro_part4,
            intro_part5,
            intro_part6,
            intro_part7,
            intro_part8,
            intro_part9,
        ] {
            let mut context = AuditContext::new(revision, false);
            assert_eq!(function(&mut context, &[]).unwrap(), Some(0));
            let state = ram::g_cutscene_script_state(revision).address();
            let accesses = context.at(state);
            let mut increments = 0;
            for (index, access) in accesses.iter().enumerate() {
                if let Access::Write(_, 2, value @ 1..=11) = *access {
                    increments += 1;
                    assert_eq!(accesses[index - 1], Access::Read(state, 2, value - 1));
                    assert_eq!(accesses[index + 1], Access::Read(state, 2, value));
                }
            }
            assert!(increments >= 2);
        }
    }
}

#[test]
fn switch_reload_preserves_wrapped_word_for_both_revisions() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = AuditContext::new(revision, false);
        let state = ram::g_cutscene_script_state(revision);
        for old in [i16::MIN, -1, 0, i16::MAX] {
            state.write(&mut context.memory, old);
            context.accesses.borrow_mut().clear();
            let next = old.wrapping_add(1);
            assert_eq!(increment_and_reload(&mut context, state), next);
            assert_eq!(
                context.at(state.address()),
                vec![
                    Access::Read(state.address(), 2, u32::from(old as u16)),
                    Access::Write(state.address(), 2, u32::from(next as u16)),
                    Access::Read(state.address(), 2, u32::from(next as u16)),
                ]
            );
        }
    }
}

#[test]
fn dense_switches_retain_the_compilers_semantic_rom_word_reads() {
    // Function-relative table starts from both accepted c_intro.o objects.
    let switches: [(FunctionId, crate::TranslationFn, u32, i16); 8] = [
        (ids::INTRO_PART1, intro_part1, 0xd6, 39),
        (ids::INTRO_PART2, intro_part2, 0x96, 10),
        (ids::INTRO_PART3, intro_part3, 0xa4, 8),
        (ids::INTRO_PART4, intro_part4, 0x156, 5),
        (ids::INTRO_PART5, intro_part5, 0x156, 11),
        (ids::INTRO_PART6, intro_part6, 0xe0, 10),
        (ids::INTRO_PART7, intro_part7, 0x18a, 7),
        (ids::INTRO_PART10, intro_part10, 0x9e, 6),
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (owner, function, offset, upper) in switches {
            let mut context = AuditContext::new(revision, false);
            assert_eq!(function(&mut context, &[]).unwrap(), Some(0));
            let base = owner.metadata().address(revision) + offset;
            for state in 1..=upper {
                let address = base + (state as u32 - 1) * 2;
                let reads = context.at(address);
                assert!(!reads.is_empty(), "{owner:?}: state {state}");
                assert!(
                    reads
                        .iter()
                        .all(|read| *read == Access::Read(address, 2, 0))
                );
            }
            context.accesses.borrow_mut().clear();
            for state in [i16::MIN, -1, 0, upper + 1, i16::MAX] {
                read_switch_entry(&mut context, owner, offset, upper, state);
            }
            assert!(context.accesses.borrow().is_empty());
        }
    }
}

#[test]
fn scene_results_define_d0_and_all_ccr_bits_without_changing_sr_control() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = AuditContext::new(revision, false);
        let state = ram::g_cutscene_script_state(revision);
        for control in [0x2000, 0x2700, 0xa000] {
            for ccr in 0..32 {
                for value in [-1, 0, 1, i16::MIN] {
                    state.write(&mut context.memory, value);
                    context.status = control | ccr;
                    let interrupted = value != -1;
                    assert_eq!(
                        scene_result(&mut context, state).unwrap(),
                        Some(u32::from(interrupted))
                    );
                    assert_eq!(
                        context.status,
                        control | if interrupted { 0x10 } else { 0x04 }
                    );
                }
            }
        }
    }
}

#[test]
fn begin_scene_final_move_uses_the_restored_full_d2_and_preserves_x() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = AuditContext::new(revision, false);
        for d2 in [0, 1, 0x0000_ffff, 0x8000_0000, u32::MAX] {
            for ccr in 0..32 {
                context.data[2] = d2;
                context.status = 0x2700 | ccr;
                context
                    .frame
                    .unwrap()
                    .argument_slot(0)
                    .write(&mut context.memory, 0x1234_0080);
                intro_begin_large_player_scene(&mut context).unwrap();
                let flags = if d2 == 0 {
                    4
                } else if d2 & 0x8000_0000 != 0 {
                    8
                } else {
                    0
                };
                assert_eq!(context.status, 0x2700 | (ccr & 0x10) | flags);
                assert_eq!(context.data[2], d2);
                assert_eq!(
                    ram::g_scene_background_pair_index(revision).read(&context),
                    -128
                );
            }
        }
    }
}

#[test]
fn intro_dispatcher_final_ext_preserves_x_and_reports_short_circuit_result() {
    let parts = [
        ids::INTRO_PART1,
        ids::INTRO_PART2,
        ids::INTRO_PART3,
        ids::INTRO_PART4,
        ids::INTRO_PART5,
        ids::INTRO_PART6,
        ids::INTRO_PART7,
        ids::INTRO_PART8,
        ids::INTRO_PART9,
        ids::INTRO_PART10,
        ids::INTRO_PART11,
    ];
    let sites: [u32; 11] = [
        0x128, 0x134, 0x13e, 0x148, 0x152, 0x15c, 0x166, 0x170, 0x17a, 0x184, 0x190,
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for abort in [None, Some(ids::INTRO_PART1), Some(ids::INTRO_PART10)] {
            for ccr in 0..32 {
                let mut context = AuditContext::new(revision, false);
                context.status = 0x2700 | ccr;
                context.interrupted_part = abort;
                assert_eq!(
                    intro_video(&mut context, &[]).unwrap(),
                    Some(u32::from(abort.is_some()))
                );
                let count = abort
                    .and_then(|id| parts.iter().position(|part| *part == id))
                    .map_or(parts.len(), |index| index + 1);
                let calls: Vec<_> = context
                    .calls
                    .iter()
                    .copied()
                    .filter(|id| parts.contains(id))
                    .collect();
                assert_eq!(calls, parts[..count]);
                let prepared: Vec<_> = context
                    .prepared_calls
                    .iter()
                    .filter(|call| parts.contains(&call.0))
                    .map(|(id, arguments, pc)| {
                        assert!(arguments.is_empty());
                        (*id, *pc - ids::INTRO_VIDEO.metadata().address(revision))
                    })
                    .collect();
                assert_eq!(
                    prepared,
                    parts
                        .iter()
                        .copied()
                        .zip(sites)
                        .take(count)
                        .collect::<Vec<_>>()
                );
                assert_eq!(context.sp, 0xfff000);
                assert_eq!(context.frame, Some(FrameState::new(0xfff000)));
                assert_eq!(
                    context.status,
                    0x2700 | (ccr & 0x10) | if abort.is_some() { 0 } else { 4 }
                );
            }
        }
    }
}

#[test]
fn clear_pre_reads_keep_discarded_purpose_width_order_and_failure() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = AuditContext::new(revision, false);
        let byte = ram::g_requested_music_selection(revision);
        let word = ram::g_cutscene_script_state(revision);
        byte.write(&mut context.memory, -91);
        word.write(&mut context.memory, -16657);
        clear_byte(&mut context, byte.address()).unwrap();
        clear_word(&mut context, word).unwrap();
        assert_eq!(
            *context.accesses.borrow(),
            vec![
                Access::Discard(byte.address(), 1),
                Access::Write(byte.address(), 1, 0),
                Access::Discard(word.address(), 2),
                Access::Write(word.address(), 2, 0),
            ]
        );
        byte.write(&mut context.memory, -91);
        word.write(&mut context.memory, -16657);
        context.accesses.borrow_mut().clear();
        context.fail_discard = true;
        assert!(clear_byte(&mut context, byte.address()).is_err());
        assert!(clear_word(&mut context, word).is_err());
        assert_eq!(
            *context.accesses.borrow(),
            vec![
                Access::Discard(byte.address(), 1),
                Access::Discard(word.address(), 2),
            ]
        );
        assert_eq!(byte.read(&context.memory), -91);
        assert_eq!(word.read(&context.memory), -16657);
    }
}

#[test]
fn all_intro_bodies_retain_their_own_nonframe_clear_instructions() {
    // Counts on the completed, unheld-controller path with test-only countdown
    // acceleration. Every tagged pre-read must immediately precede its zero store.
    let bodies: [(crate::TranslationFn, usize, usize); 11] = [
        (intro_video, 2, 4),
        (intro_part1, 0, 4),
        (intro_part2, 0, 1),
        (intro_part3, 1, 1),
        (intro_part4, 0, 1),
        (intro_part5, 0, 1),
        (intro_part6, 0, 1),
        (intro_part7, 7, 3),
        (intro_part8, 0, 2),
        (intro_part9, 0, 1),
        (intro_part10, 0, 0),
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (function, byte_reads, word_reads) in bodies {
            let mut context = AuditContext::new(revision, false);
            function(&mut context, &[7]).unwrap();
            let accesses = context.accesses.borrow();
            let mut counts = [0; 2];
            for (index, access) in accesses.iter().enumerate() {
                if let Access::Discard(address, width) = *access {
                    counts[usize::from(width == 2)] += 1;
                    assert_eq!(accesses[index + 1], Access::Write(address, width, 0));
                }
            }
            assert_eq!(counts, [byte_reads, word_reads]);
        }
    }
}

#[test]
fn begin_scene_reads_only_the_live_background_word_before_allocating() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for argument in [0x1234_0080, 0xdead_ffff, 0xbeef_8000, 0xaaaa_7fff] {
            let mut context = AuditContext::new(revision, false);
            let entry = context.frame.unwrap();
            context.data[2] = 0xface_babe;
            entry.argument_slot(0).write(&mut context.memory, argument);
            context.replace_background_on_allocate = Some(1);
            intro_begin_large_player_scene(&mut context).unwrap();
            assert_eq!(
                context.observed_allocation_d2,
                Some(0xface_0000 | (argument & 0xffff))
            );
            assert_eq!(context.data[2], 0xface_babe);
            let read = Access::Read(entry.entry_sp + 6, 2, argument & 0xffff);
            assert_eq!(context.accesses.borrow()[0], read);
            for offset in 4..8 {
                let accesses = context.at(entry.entry_sp + offset);
                assert_eq!(accesses, if offset == 6 { vec![read] } else { vec![] });
            }
            assert_eq!(
                ram::g_scene_background_pair_index(revision).read(&context.memory),
                argument as i8
            );
            // The allocator changed the live slot after the source's MOVE.W;
            // the background must still be the value loaded before that call.
            assert_eq!(entry.argument_word(0).read(&context.memory), 1);
        }
    }
}

#[test]
fn begin_scene_is_pending_framed_and_rejects_the_value_adapter() {
    assert!(
        !super::super::PENDING_TRANSLATIONS
            .iter()
            .any(|entry| entry.id == ids::INTRO_BEGIN_LARGE_PLAYER_SCENE)
    );
    for revision in [Revision::Rev02, Revision::Rev00] {
        let dispatcher =
            Dispatcher::hybrid_with_frames(&[], super::super::PENDING_FRAMED_TRANSLATIONS);
        let mut context = AuditContext::new(revision, false);
        assert!(matches!(
            dispatcher.call_void(&mut context, ids::INTRO_BEGIN_LARGE_PLAYER_SCENE, &[7]),
            Err(crate::Error::Frame(crate::call_frame::FrameError::PreparedEntryRequired(id)))
                if id == ids::INTRO_BEGIN_LARGE_PLAYER_SCENE
        ));
        assert!(context.accesses.borrow().is_empty());
        let entry = context.frame.unwrap();
        dispatcher
            .call_at_void(
                &mut context,
                ids::INTRO_BEGIN_LARGE_PLAYER_SCENE,
                &[0x1234_0080],
                CallSite::new(0x100),
            )
            .unwrap();
        assert_eq!(context.frame, Some(entry));
        assert_eq!(context.sp, entry.entry_sp);
        assert_eq!(
            ram::g_scene_background_pair_index(revision).read(&context.memory),
            -128
        );
    }
}

#[test]
fn intro_begin_callers_supply_prepared_slots_and_real_return_sites() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (caller, function) in [
            (ids::INTRO_PART1, intro_part1 as crate::TranslationFn),
            (ids::INTRO_PART3, intro_part3 as crate::TranslationFn),
        ] {
            let mut context = AuditContext::new(revision, false);
            let entry = context.frame.unwrap();
            function(&mut context, &[]).unwrap();
            context
                .prepared_calls
                .retain(|call| call.0 == ids::INTRO_BEGIN_LARGE_PLAYER_SCENE);
            assert_eq!(
                context.prepared_calls,
                vec![(
                    ids::INTRO_BEGIN_LARGE_PLAYER_SCENE,
                    vec![7],
                    caller.metadata().address(revision) + 0x1e,
                )]
            );
            assert_eq!(context.frame, Some(entry));
            assert_eq!(context.sp, entry.entry_sp);
        }
    }
}

#[test]
fn begin_scene_requires_an_available_lazy_frame_before_any_access() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = AuditContext::new(revision, false);
        context.frame = None;
        assert!(matches!(
            intro_begin_large_player_scene(&mut context),
            Err(crate::Error::Frame(
                crate::call_frame::FrameError::Unavailable
            ))
        ));
        assert!(context.accesses.borrow().is_empty());
        assert!(context.calls.is_empty());
    }
}

#[test]
fn every_intro_title_callee_has_prepared_arguments_and_its_compiled_return_site() {
    use std::collections::BTreeSet;
    const ANIMATE: FunctionId = ids::ANIMATE_TITLE_TRANSITION_BACKGROUND;
    const SCREEN: FunctionId = ids::CUTSCENE_DRAW_RANDOMIZED_SCREEN_BORDER;
    const RECTANGLE: FunctionId = ids::CUTSCENE_DRAW_RANDOMIZED_RECTANGLE_BORDER;
    const CADENCE: FunctionId = ids::CUTSCENE_STEP_FRAME_AND_MUSIC_CADENCE;
    // Return PCs from both accepted c_intro.o objects; no ROM instructions/data.
    let expected = [
        (ids::INTRO_VIDEO, SCREEN, 0x102),
        (ids::INTRO_BEGIN_LARGE_PLAYER_SCENE, ANIMATE, 0x22),
        (ids::INTRO_PART1, CADENCE, 0x48),
        (ids::INTRO_PART1, CADENCE, 0x76),
        (ids::INTRO_PART1, CADENCE, 0x54e),
        (ids::INTRO_PART2, CADENCE, 0x46),
        (ids::INTRO_PART2, CADENCE, 0x1a0),
        (ids::INTRO_PART3, CADENCE, 0x4c),
        (ids::INTRO_PART3, CADENCE, 0x218),
        (ids::INTRO_PART4, SCREEN, 0x32),
        (ids::INTRO_PART4, CADENCE, 0x21e),
        (ids::INTRO_PART5, RECTANGLE, 0x46),
        (ids::INTRO_PART5, RECTANGLE, 0x5c),
        (ids::INTRO_PART5, CADENCE, 0xf8),
        (ids::INTRO_PART5, CADENCE, 0x2b8),
        (ids::INTRO_PART6, ANIMATE, 0x5c),
        (ids::INTRO_PART6, CADENCE, 0x94),
        (ids::INTRO_PART6, CADENCE, 0x212),
        (ids::INTRO_PART7, ANIMATE, 0x76),
        (ids::INTRO_PART7, ANIMATE, 0xa8),
        (ids::INTRO_PART7, CADENCE, 0x2de),
        (ids::INTRO_PART8, CADENCE, 0xd8),
        (ids::INTRO_PART8, CADENCE, 0x160),
        (ids::INTRO_PART9, ANIMATE, 0x28),
        (ids::INTRO_PART9, RECTANGLE, 0x3e),
        (ids::INTRO_PART9, RECTANGLE, 0x54),
        (ids::INTRO_PART9, CADENCE, 0xb2),
        (ids::INTRO_PART9, CADENCE, 0x13a),
        (ids::INTRO_PART10, SCREEN, 0x20),
        (ids::INTRO_PART10, CADENCE, 0x80),
        (ids::INTRO_PART10, CADENCE, 0x17a),
    ];
    let bodies: [(FunctionId, crate::TranslationFn); 11] = [
        (ids::INTRO_VIDEO, intro_video),
        (ids::INTRO_PART1, intro_part1),
        (ids::INTRO_PART2, intro_part2),
        (ids::INTRO_PART3, intro_part3),
        (ids::INTRO_PART4, intro_part4),
        (ids::INTRO_PART5, intro_part5),
        (ids::INTRO_PART6, intro_part6),
        (ids::INTRO_PART7, intro_part7),
        (ids::INTRO_PART8, intro_part8),
        (ids::INTRO_PART9, intro_part9),
        (ids::INTRO_PART10, intro_part10),
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut observed = BTreeSet::new();
        let mut contexts = Vec::with_capacity(12);
        let mut begin = AuditContext::new(revision, false);
        begin
            .frame
            .unwrap()
            .argument_slot(0)
            .write(&mut begin.memory, 7);
        intro_begin_large_player_scene(&mut begin).unwrap();
        contexts.push((ids::INTRO_BEGIN_LARGE_PLAYER_SCENE, begin));
        for (owner, function) in bodies {
            let mut context = AuditContext::new(revision, false);
            if owner == ids::INTRO_PART1 {
                context.start_samples = 1; // Exercise the initial Start-release loop.
            }
            function(&mut context, &[]).unwrap();
            contexts.push((owner, context));
        }
        for (owner, context) in contexts {
            assert_eq!(context.sp, 0xfff000);
            assert_eq!(context.frame, Some(FrameState::new(0xfff000)));
            for (callee, arguments, return_pc) in context.prepared_calls {
                if ![ANIMATE, SCREEN, RECTANGLE, CADENCE].contains(&callee) {
                    continue;
                }
                let offset = return_pc - owner.metadata().address(revision);
                assert!(expected.contains(&(owner, callee, offset)));
                observed.insert((owner, callee, offset));
                if callee == ANIMATE {
                    assert_eq!(
                        arguments,
                        [u32::from(owner != ids::INTRO_PART7 || offset != 0xa8)]
                    );
                } else if callee == SCREEN {
                    assert_eq!(arguments, [u32::from(owner != ids::INTRO_VIDEO)]);
                } else if callee == RECTANGLE {
                    let first = (owner == ids::INTRO_PART5 && offset == 0x46)
                        || (owner == ids::INTRO_PART9 && offset == 0x3e);
                    assert_eq!(
                        arguments,
                        if first {
                            [11, 7, 30, 20]
                        } else {
                            [12, 8, 29, 19]
                        }
                    );
                } else {
                    assert_eq!(arguments.len(), 2);
                    assert!(arguments[0] <= u32::from(owner == ids::INTRO_PART2));
                    assert!(
                        arguments[1]
                            <= u32::from([ids::INTRO_PART1, ids::INTRO_PART3].contains(&owner))
                    );
                }
            }
        }
        assert_eq!(observed, expected.into_iter().collect());
    }
}

#[test]
fn intro_refresh_callers_prepare_the_zero_argument_callee_frame() {
    let cases: [(FunctionId, crate::TranslationFn, FunctionId, u32); 3] = [
        (
            ids::INTRO_PART1,
            intro_part1,
            ids::INTRO_RELEASE_LARGE_PLAYER_SPRITES_AND_REFRESH,
            0x53e,
        ),
        (
            ids::INTRO_PART2,
            intro_part2,
            ids::INTRO_RELEASE_FRAME_ENTITIES_AND_REFRESH,
            0x196,
        ),
        (
            ids::INTRO_PART3,
            intro_part3,
            ids::INTRO_RELEASE_LARGE_PLAYER_SPRITES_AND_REFRESH,
            0x208,
        ),
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (owner, function, callee, offset) in cases {
            let mut context = AuditContext::new(revision, false);
            function(&mut context, &[]).unwrap();
            let calls: Vec<_> = context
                .prepared_calls
                .into_iter()
                .filter(|call| call.0 == callee)
                .collect();
            assert_eq!(
                calls,
                vec![(callee, vec![], owner.metadata().address(revision) + offset)]
            );
            assert_eq!(context.sp, 0xfff000);
            assert_eq!(context.frame, Some(FrameState::new(0xfff000)));
        }
    }
}

#[test]
fn intro_selector_callers_supply_the_real_prepared_zero_player_slot() {
    let cases: [(FunctionId, crate::TranslationFn, u32); 2] = [
        (ids::INTRO_PART4, intro_part4, 0xd0),
        (ids::INTRO_PART7, intro_part7, 0x11c),
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (owner, function, offset) in cases {
            let mut context = AuditContext::new(revision, false);
            function(&mut context, &[]).unwrap();
            let calls: Vec<_> = context
                .prepared_calls
                .into_iter()
                .filter(|call| call.0 == ids::SELECT_VIEWPORT_SPRITE_CHAIN)
                .collect();
            assert!(!calls.is_empty());
            for call in calls {
                assert_eq!(
                    call,
                    (
                        ids::SELECT_VIEWPORT_SPRITE_CHAIN,
                        vec![0],
                        owner.metadata().address(revision) + offset,
                    )
                );
            }
            assert_eq!(context.sp, 0xfff000);
            assert_eq!(context.frame, Some(FrameState::new(0xfff000)));
        }
    }
}
