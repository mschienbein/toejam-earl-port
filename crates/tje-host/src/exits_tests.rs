//! Authored machine-code fixtures only; no cartridge code or data.
use super::*;
use crate::{
    abi::run_original_atomic_until, compare_calls_with_execution, sinks::NoCartridgeSaves,
};
use genesis_config::GenesisEmulatorConfig;
use genesis_core::GenesisEmulator;
use jgenesis_common::frontend::PartialClone;
use tje_game::{
    Dispatcher, FunctionMetadata,
    execution::ExecutionClass,
    exits::{MainLoopStage, StackBanks},
    ids,
};

const CODE: u32 = 0xfff000;
const RESUME: u32 = CODE + 0x100;
const STACK: u32 = 0xff7000;
const USER_STACK: u32 = 0xff4000;

fn machine(revision: Revision, words: &[u16]) -> GenesisEmulator {
    let mut emulator = GenesisEmulator::create(
        None,
        GenesisEmulatorConfig::default(),
        &mut NoCartridgeSaves,
    );
    {
        let mut context = HookContext::new(
            &mut emulator,
            revision,
            Dispatcher::hybrid_with(&[]),
            crate::Mode::Replace,
        );
        for (index, &word) in words.iter().enumerate() {
            context.write_u16(CODE + 2 * index as u32, word);
        }
        context.write_u16(RESUME, 0x4e71); // Authored NOP landing pad.
        context.write_u16(RESUME + 2, 0x4e71);
        context.set_status_register(0x2700);
        context
            .set_stack_banks(StackBanks {
                user: USER_STACK,
                supervisor: STACK,
            })
            .unwrap();
        for index in 0..8 {
            context.set_data_register(index, 0x12340000 + index as u32);
        }
        for index in 0..7 {
            context.set_address_register(index, 0x56780000 + index as u32);
        }
        context.transfer_to(CODE).unwrap();
    }
    emulator
}

fn hook(revision: Revision, exit_contract: ExitContract, body: crate::hooks::HookFn) -> Hook {
    let mut symbol: FunctionMetadata = ids::GET_RANDOM_NUMBER.metadata().clone();
    symbol.name = "AuthoredNonRtsEntry";
    symbol.addresses = [CODE; 2];
    symbol.sizes = [32; 2];
    symbol.source = "authored synthetic exit-contract fixture";
    symbol.abi.argument_count = 0;
    symbol.abi.returns_value = false;
    let mut registry = crate::hooks::HookRegistry::empty(revision);
    registry
        .register_exit_contract(symbol, ExecutionClass::Atomic, exit_contract, body)
        .unwrap();
    registry.at(CODE).unwrap().clone()
}

fn run_pair(
    original: &mut GenesisEmulator,
    hook: &Hook,
    revision: Revision,
) -> (GenesisEmulator, crate::abi::AtomicRun, EntryOutcome) {
    let mut translated = original.partial_clone();
    let entry = {
        let context = HookContext::new(
            original,
            revision,
            Dispatcher::hybrid_with(&[]),
            crate::Mode::Replace,
        );
        EntryState::capture(&context, hook.exit_contract).unwrap()
    };
    let outcome = {
        let mut context = HookContext::new(
            &mut translated,
            revision,
            Dispatcher::hybrid_with(&[]),
            crate::Mode::Replace,
        );
        let outcome = hook.execute(&mut context).unwrap();
        finish(&mut context, hook, outcome).unwrap();
        outcome
    };
    let boundary = reference_boundary(hook, outcome, entry, revision).unwrap();
    let audit = run_original_atomic_until(original, boundary, true, false).unwrap();
    (translated, audit, outcome)
}

fn strict_compare(
    original: &mut GenesisEmulator,
    translated: &mut GenesisEmulator,
    hook: &Hook,
) -> std::result::Result<u64, String> {
    let mut abi = hook.symbol.abi.clone();
    abi.strict_registers = true;
    let live_stack_start = original.hybrid_cpu().stack_pointer().min(STACK);
    compare_calls_with_execution(
        original,
        translated,
        live_stack_start,
        &abi,
        ExecutionClass::Atomic,
    )
}

fn rte(_: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    exits::exception_return(ids::GET_RANDOM_NUMBER).map_err(Into::into)
}
fn terminal_body(_: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    exits::terminal(ids::GET_RANDOM_NUMBER).map_err(Into::into)
}
fn call_body(_: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    exits::call_transfer(ids::GET_RANDOM_NUMBER, ids::START, CODE + 6).map_err(Into::into)
}
fn tail_body(context: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    let mut banks = context.stack_banks()?;
    banks.supervisor = 0xff6000;
    context.set_stack_banks(banks)?;
    exits::tail(ids::GET_RANDOM_NUMBER, ids::START).map_err(Into::into)
}
fn loop_body(context: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    let mut banks = context.stack_banks()?;
    banks.supervisor -= 4;
    context.write_u32(banks.supervisor, context.data_register(2));
    context.set_stack_banks(banks)?;
    exits::main_loop(ids::GET_RANDOM_NUMBER, MainLoopStage::Begin).map_err(Into::into)
}

#[test]
fn authored_rte_matches_original_for_supervisor_and_user_resume_in_both_revisions() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for saved_status in [0x0015, 0x231a] {
            let mut original = machine(revision, &[0x4e73]); // Authored RTE.
            {
                let mut context = HookContext::new(
                    &mut original,
                    revision,
                    Dispatcher::hybrid_with(&[]),
                    crate::Mode::Replace,
                );
                context.write_u16(STACK, saved_status);
                context.write_u32(STACK + 2, RESUME);
                context.write_u32(USER_STACK, 0x5a6b7c8d);
            }
            let hook = hook(revision, ExitContract::ExceptionReturn, rte);
            let (mut translated, audit, _) = run_pair(&mut original, &hook, revision);
            assert_eq!(audit.instructions, 1);
            assert_eq!(strict_compare(&mut original, &mut translated, &hook), Ok(0));
            assert_eq!(translated.hybrid_cpu().pc(), RESUME);
            assert_eq!(translated.hybrid_cpu().status_register(), saved_status);
            assert_eq!(
                translated.hybrid_cpu().supervisor_stack_pointer(),
                STACK + 6
            );
            assert_eq!(translated.hybrid_cpu().user_stack_pointer(), USER_STACK);
            assert_eq!(
                translated.hybrid_cpu().stack_pointer(),
                if saved_status & 0x2000 != 0 {
                    STACK + 6
                } else {
                    USER_STACK
                }
            );
        }
    }
}

#[test]
fn terminal_executes_one_original_branch_without_popping_a_frame() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut original = machine(revision, &[0x60fe]); // Authored self-branch.
        let hook = hook(revision, ExitContract::Terminal, terminal_body);
        let (mut translated, audit, outcome) = run_pair(&mut original, &hook, revision);
        assert_eq!(audit.instructions, 1);
        assert_eq!(terminal(outcome), Some(ids::GET_RANDOM_NUMBER));
        assert_eq!(strict_compare(&mut original, &mut translated, &hook), Ok(0));
        assert_eq!(translated.hybrid_cpu().pc(), CODE);
        assert_eq!(translated.hybrid_cpu().stack_pointer(), STACK);
    }
}

#[test]
fn final_jsr_pushes_the_authored_return_word_instead_of_a_sentinel() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let target = ids::START.metadata().address(revision);
        let mut original = machine(revision, &[0x4eb9, (target >> 16) as u16, target as u16]);
        let hook = hook(
            revision,
            ExitContract::Call {
                target: ids::START,
                return_offset: 6,
            },
            call_body,
        );
        let (mut translated, audit, _) = run_pair(&mut original, &hook, revision);
        assert_eq!(audit.instructions, 1);
        assert_eq!(strict_compare(&mut original, &mut translated, &hook), Ok(0));
        assert_eq!(translated.hybrid_cpu().stack_pointer(), STACK - 4);
        let context = HookContext::new(
            &mut translated,
            revision,
            Dispatcher::hybrid_with(&[]),
            crate::Mode::Replace,
        );
        assert_eq!(context.read_u32(STACK - 4), CODE + 6);
    }
}

#[test]
fn tail_transfer_keeps_the_reloaded_stack_and_does_not_run_rts_cleanup() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let target = ids::START.metadata().address(revision);
        // Authored MOVEA.L immediate to SP, then JMP absolute long.
        let mut original = machine(
            revision,
            &[
                0x2e7c,
                0x00ff,
                0x6000,
                0x4ef9,
                (target >> 16) as u16,
                target as u16,
            ],
        );
        let hook = hook(
            revision,
            ExitContract::Tail { target: ids::START },
            tail_body,
        );
        let (mut translated, audit, _) = run_pair(&mut original, &hook, revision);
        assert_eq!(audit.instructions, 2);
        assert_eq!(strict_compare(&mut original, &mut translated, &hook), Ok(0));
        assert_eq!(translated.hybrid_cpu().stack_pointer(), 0xff6000);
        assert_eq!(translated.hybrid_cpu().pc(), target);
    }
}

#[test]
fn persistent_frame_bytes_are_live_below_the_initial_entry_stack() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        // Authored MOVEM.L D2,-(SP); NOP landing pad.
        let mut original = machine(revision, &[0x48e7, 0x2000, 0x4e71]);
        let contract = ExitContract::MainLoop {
            begin_offset: 4,
            idle_offset: 6,
            calls: &[],
        };
        let hook = hook(revision, contract, loop_body);
        let (mut translated, audit, _) = run_pair(&mut original, &hook, revision);
        assert_eq!(audit.instructions, 1);
        assert_eq!(strict_compare(&mut original, &mut translated, &hook), Ok(0));
        HookContext::new(
            &mut translated,
            revision,
            Dispatcher::hybrid_with(&[]),
            crate::Mode::Replace,
        )
        .write_u8(STACK - 1, 0xff);
        let mismatch = strict_compare(&mut original, &mut translated, &hook).unwrap_err();
        assert!(mismatch.contains("RAM"));
    }
}

#[test]
fn non_rts_comparison_rejects_scratch_register_and_inactive_bank_corruption() {
    let revision = Revision::Rev02;
    let mut original = machine(revision, &[0x60fe]);
    let hook = hook(revision, ExitContract::Terminal, terminal_body);
    let (mut translated, _, _) = run_pair(&mut original, &hook, revision);
    let mut registers = translated.hybrid_cpu().data_registers();
    registers[0] ^= 1;
    translated.hybrid_parts().0.set_data_registers(registers);
    assert!(
        strict_compare(&mut original, &mut translated, &hook)
            .unwrap_err()
            .contains("D0")
    );
    registers[0] ^= 1;
    translated.hybrid_parts().0.set_data_registers(registers);
    let cpu = translated.hybrid_parts().0;
    cpu.set_address_registers(
        cpu.address_registers(),
        USER_STACK + 2,
        cpu.supervisor_stack_pointer(),
    );
    assert!(
        strict_compare(&mut original, &mut translated, &hook)
            .unwrap_err()
            .contains("banked")
    );
}

fn missing_source_then_transfer(context: &mut HookContext<'_>) -> Result<Option<u32>> {
    let _ = tje_game::Platform::read_device_u16(context, 0xa11100);
    tje_game::exits::terminal(ids::GET_RANDOM_NUMBER).map_err(HostError::from)
}

fn missing_source_then_error(context: &mut HookContext<'_>) -> Result<Option<u32>> {
    let _ = tje_game::Platform::read_device_u16(context, 0xa11100);
    Err(HostError::Metadata("ordinary body error".into()))
}

#[test]
fn poisoned_mmio_cannot_be_finalized_as_a_terminal_transfer() {
    for (body, transfers) in [
        (missing_source_then_transfer as crate::hooks::HookFn, true),
        (missing_source_then_error, false),
    ] {
        let revision = Revision::Rev02;
        let mut emulator = machine(revision, &[0x60fe]);
        let hook = hook(revision, ExitContract::Terminal, body);
        let mut context = HookContext::new(
            &mut emulator,
            revision,
            Dispatcher::hybrid_with(&[]),
            crate::Mode::Replace,
        );
        let before = (context.cpu().pc(), context.cpu().stack_pointer());
        let error = hook.execute(&mut context).unwrap_err();
        match (&error, transfers) {
            (HostError::Translation(tje_game::Error::CleanupAfterTransfer { error, .. }), true) => {
                assert!(matches!(error.as_ref(), tje_game::Error::OpenBus(_)))
            }
            (HostError::Translation(tje_game::Error::OpenBus(_)), false) => {}
            _ => panic!("poisoned MMIO lost its first cause or transfer ownership: {error}"),
        }
        if let HostError::Translation(error) = &error {
            assert_eq!(error.preserves_call_frame(), transfers);
        }
        let transfer = match tje_game::exits::terminal::<()>(ids::GET_RANDOM_NUMBER) {
            Err(tje_game::Error::ControlTransfer(transfer)) => transfer,
            other => panic!("expected authored transfer: {other:?}"),
        };
        assert!(finish(&mut context, &hook, EntryOutcome::Transfer(transfer)).is_err());
        assert_eq!((context.cpu().pc(), context.cpu().stack_pointer()), before);
    }
}

#[path = "exits_idle_tests.rs"]
mod idle;
