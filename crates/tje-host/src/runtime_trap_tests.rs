//! Authored sparse-ROM tests of binding/finalization, not retail helper proofs.
use super::*;
use crate::{abi::HookContext, sinks::NoCartridgeSaves};
use genesis_config::GenesisEmulatorConfig;
use genesis_core::GenesisEmulator;
use tje_game::{
    Dispatcher,
    call_frame::FrameState,
    control::{ExceptionRequest, SynchronousTrap},
    ids,
};

const STACK: u32 = 0xff7000;
const HANDLER: u32 = 0x1000;
fn machine(revision: Revision, entry: FunctionId, opcode: u16) -> GenesisEmulator {
    let mut rom = vec![0; 0x30000];
    rom[..4].copy_from_slice(&STACK.to_be_bytes());
    rom[4..8].copy_from_slice(&0x40u32.to_be_bytes());
    rom[20..24].copy_from_slice(&HANDLER.to_be_bytes());
    rom[HANDLER as usize..HANDLER as usize + 2].copy_from_slice(&0x60feu16.to_be_bytes());
    let fault = entry
        .metadata()
        .address(revision)
        .wrapping_add_signed(tje_game::control::runtime_divide_fault_offset(entry).unwrap());
    rom[fault as usize..fault as usize + 2].copy_from_slice(&opcode.to_be_bytes());
    rom[fault as usize + 2..fault as usize + 4].copy_from_slice(&0x4e71u16.to_be_bytes());
    let mut emulator = GenesisEmulator::create(
        Some(rom),
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    let mut context = HookContext::new(
        &mut emulator,
        revision,
        Dispatcher::hybrid_with(&[]),
        crate::Mode::Replace,
    );
    context.set_status_register(0x2710);
    context
        .set_stack_banks(exits::StackBanks {
            user: 0xff6000,
            supervisor: STACK,
        })
        .unwrap();
    context.set_frame_state(FrameState::new(STACK)).unwrap();
    context.set_data_register(0, 7);
    context.set_data_register(1, 0);
    context.set_data_register(2, 0x89abcdef);
    context
        .transfer_to(entry.metadata().address(revision))
        .unwrap();
    emulator
}
fn prepare(context: &mut HookContext<'_>, entry: FunctionId) -> ExceptionRequest {
    let body = match entry {
        ids::LMODT => tje_game::runtime::lmodt,
        ids::LDIVT => tje_game::runtime::ldivt,
        ids::ULMODT => tje_game::runtime::ulmodt,
        ids::ULDIVT => tje_game::runtime::uldivt,
        _ => panic!("authored runtime fixture uses four reviewed entries only"),
    };
    let result = body(context, &[]);
    let tje_game::Error::ControlTransfer(transfer) = result.unwrap_err() else {
        panic!("expected trap");
    };
    let Exit::Exception { request } = transfer.exit else {
        panic!("expected synchronous exception");
    };
    request
}
#[test]
fn source_bound_trap_uses_real_vector_and_keeps_source_save() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for entry in [ids::LMODT, ids::LDIVT, ids::ULMODT, ids::ULDIVT] {
            let mut emulator = machine(revision, entry, 0x82c2);
            let mut context = HookContext::new(
                &mut emulator,
                revision,
                Dispatcher::hybrid_with(&[]),
                crate::Mode::Replace,
            );
            context.set_frame_state(FrameState::new(STACK)).unwrap();
            let request = prepare(&mut context, entry);
            context
                .enter_source_exception(entry, entry, request)
                .unwrap();
            assert_eq!(context.cpu().pc(), HANDLER);
            assert_eq!(context.stack_pointer().unwrap(), STACK - 10);
            assert_eq!(context.read_u32(STACK - 4), 0x89abcdef);
            assert_eq!(context.read_u16(STACK - 10), 0x2710);
            assert_eq!(context.read_u32(STACK - 8), request.saved_pc);
            assert_eq!(context.cpu().hybrid_exception_depth(), 1);
            assert_eq!(context.cpu().hybrid_last_exception_vector(), Some(5));
            assert_eq!(context.cpu().user_stack_pointer(), 0xff6000);
            assert!(context.cpu().last_instruction_was_mul_or_div());
            assert!(!context.cpu().address_error());
        }
    }
}
#[test]
fn wrong_vector_site_next_pc_and_prepared_registers_reject_before_entry_effects() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for mutant in 0..6 {
            let entry = ids::LMODT;
            let mut emulator = machine(revision, entry, if mutant == 4 { 0x80c2 } else { 0x82c2 });
            let mut context = HookContext::new(
                &mut emulator,
                revision,
                Dispatcher::hybrid_with(&[]),
                crate::Mode::Replace,
            );
            context.set_frame_state(FrameState::new(STACK)).unwrap();
            let mut request = prepare(&mut context, entry);
            match mutant {
                0 => request.trap = SynchronousTrap::Overflow,
                1 => request.site.byte_offset += 2,
                2 => request.saved_pc += 2,
                3 => context.set_data_register(1, 1),
                4 => {}
                5 => request.prepared.status |= 0x8000,
                _ => unreachable!(),
            }
            let before = context.cpu().hybrid_state_words();
            let error = context
                .enter_source_exception(entry, entry, request)
                .unwrap_err();
            assert!(matches!(error, HostError::Translation(ref e) if e.preserves_call_frame()));
            assert_eq!(context.cpu().hybrid_state_words(), before);
            assert_eq!(context.read_u32(STACK - 4), 0x89abcdef);
            assert_eq!(context.read_u32(STACK - 10), 0);
            assert_eq!(context.cpu().hybrid_exception_depth(), 0);
        }
    }
}
#[test]
fn expected_exception_boundary_never_accepts_handler_pc_readiness_alone() {
    let revision = Revision::Rev02;
    let entry = ids::LMODT;
    let mut emulator = machine(revision, entry, 0x82c2);
    let mut context = HookContext::new(
        &mut emulator,
        revision,
        Dispatcher::hybrid_with(&[]),
        crate::Mode::Replace,
    );
    context.set_frame_state(FrameState::new(STACK)).unwrap();
    let request = prepare(&mut context, entry);
    let boundary = exception_reference_boundary(
        request,
        EntryState {
            sp: STACK,
            return_pc: 0,
            exception_pc: 0,
            exception_depth: 0,
        },
        revision,
    )
    .unwrap();
    context.transfer_to(HANDLER).unwrap();
    assert!(!boundary.reached(context.cpu(), Some(request.site.address(revision)), 100));
    assert!(!boundary.reached(context.cpu(), None, 100));
}

#[test]
fn elided_nested_value_call_cannot_claim_a_materialized_exception_entry() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let entry = ids::LMODT;
        let mut emulator = machine(revision, entry, 0x82c2);
        let mut context = HookContext::new(
            &mut emulator,
            revision,
            Dispatcher::hybrid_with(&[]),
            crate::Mode::Replace,
        );
        context.set_frame_state(FrameState::new(STACK)).unwrap();
        let request = prepare(&mut context, entry);
        let before = context.cpu().hybrid_state_words();
        // GetRandomNumber really depends on LMODT, but a value-only Rust edge
        // has not materialized the callee's JSR/return frame.
        assert!(tje_game::exits::reaches(ids::GET_RANDOM_NUMBER, entry));
        assert!(
            context
                .enter_source_exception(ids::GET_RANDOM_NUMBER, entry, request)
                .is_err()
        );
        assert_eq!(context.cpu().hybrid_state_words(), before);
        assert_eq!(context.read_u32(STACK - 4), 0x89abcdef);
    }
}

#[test]
fn explicit_prepared_call_keeps_its_caller_word_and_runtime_save_on_transfer() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let entry = ids::LMODT;
        let mut emulator = machine(revision, entry, 0x82c2);
        let mut context = HookContext::new(
            &mut emulator,
            revision,
            Dispatcher::hybrid_with(tje_game::runtime::TRANSLATIONS),
            crate::Mode::Replace,
        );
        context.set_frame_state(FrameState::new(STACK)).unwrap();
        // Authored call boundary, not a claim about natural source call timing.
        let error = tje_game::call_at(
            &mut context,
            entry,
            &[],
            tje_game::call_frame::CallSite::new(0x1200),
        )
        .unwrap_err();
        let tje_game::Error::ControlTransfer(transfer) = error else {
            panic!("expected transfer");
        };
        let Exit::Exception { request } = transfer.exit else {
            panic!("expected trap");
        };
        assert!(context.frame_state().unwrap().is_prepared_for(entry));
        context
            .enter_source_exception(ids::GET_RANDOM_NUMBER, entry, request)
            .unwrap();
        assert_eq!(context.stack_pointer().unwrap(), STACK - 14);
        assert_eq!(context.read_u32(STACK - 4), 0x1200);
        assert_eq!(context.read_u32(STACK - 8), 0x89abcdef);
        assert_eq!(context.cpu().pc(), HANDLER);
    }
}

#[test]
fn prepared_runtime_normal_return_restores_parent_and_reuses_frame() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut emulator = machine(revision, ids::LMODT, 0x82c2);
        let mut context = HookContext::new(
            &mut emulator,
            revision,
            Dispatcher::hybrid_with(tje_game::runtime::TRANSLATIONS),
            crate::Mode::Replace,
        );
        let parent = FrameState::new(STACK);
        context.set_frame_state(parent).unwrap();
        for (entry, expected) in [(ids::LMODT, 1), (ids::LDIVT, 2)] {
            context.set_data_register(0, 7);
            context.set_data_register(1, 3);
            assert_eq!(
                tje_game::call_at(
                    &mut context,
                    entry,
                    &[],
                    tje_game::call_frame::CallSite::new(0x1200),
                )
                .unwrap(),
                expected
            );
            assert_eq!(context.stack_pointer().unwrap(), STACK);
            assert_eq!(context.frame_state().unwrap(), parent);
            assert_eq!(context.read_u32(STACK - 4), 0x1200);
            assert_eq!(context.data_register(2), 0x89abcdef);
        }
    }
}

#[test]
fn traced_runtime_entry_rejects_before_any_prefix_effect() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for entry in [ids::LMODT, ids::LDIVT, ids::ULMODT, ids::ULDIVT] {
            let mut emulator = machine(revision, entry, 0x82c2);
            let ram_before: Vec<_> = (0..0x10000)
                .map(|address| emulator.hybrid_bus().memory.read_main_ram_byte(address))
                .collect();
            {
                let mut context = HookContext::new(
                    &mut emulator,
                    revision,
                    Dispatcher::hybrid_with(tje_game::runtime::TRANSLATIONS),
                    crate::Mode::Replace,
                );
                context.set_status_register(context.status_register() | 0x8000);
                let cpu_before = context.cpu().hybrid_state_words();
                let frame_before = context.frame_state().unwrap();
                let error = tje_game::call(&mut context, entry, &[]).unwrap_err();
                assert!(matches!(
                    error,
                    tje_game::Error::Platform(ref message)
                        if message.contains("trace-enabled entry")
                ));
                assert_eq!(context.cpu().hybrid_state_words(), cpu_before);
                assert_eq!(context.frame_state().unwrap(), frame_before);
            }
            let ram_after: Vec<_> = (0..0x10000)
                .map(|address| emulator.hybrid_bus().memory.read_main_ram_byte(address))
                .collect();
            assert_eq!(ram_after, ram_before);
        }
    }
}
