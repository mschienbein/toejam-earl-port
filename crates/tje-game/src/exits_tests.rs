use super::*;
use crate::{Dispatcher, Platform};
use tje_mem::{AddressSpace, Bus};

struct TestContext {
    memory: AddressSpace,
    data: [u32; 8],
    address: [u32; 7],
    status: u16,
    banks: StackBanks,
}
impl TestContext {
    fn new(status: u16) -> Self {
        Self {
            memory: AddressSpace::new(Vec::new(), Revision::Rev02),
            data: [0; 8],
            address: [0; 7],
            status,
            banks: StackBanks {
                user: 0xff4000,
                supervisor: 0xff7000,
            },
        }
    }
}
impl Bus for TestContext {
    fn read_u8(&self, address: u32) -> u8 {
        self.memory.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.memory.write_u8(address, value);
    }
}
impl tje_mem::Platform for TestContext {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("unexpected device write");
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("unexpected device write");
    }
}
impl Platform for TestContext {
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
impl Context for TestContext {
    fn revision(&self) -> Revision {
        Revision::Rev02
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
        self.status = value;
    }
    fn stack_banks(&self) -> Result<StackBanks> {
        Ok(self.banks)
    }
    fn set_stack_banks(&mut self, banks: StackBanks) -> Result<()> {
        self.banks = banks;
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with(&[])
    }
    fn call_original(&mut self, id: FunctionId, _: &[u32]) -> Result<u32> {
        Err(Error::Untranslated(id))
    }
}

#[test]
fn rte_consumes_the_supervisor_frame_before_switching_to_user_stack() {
    for saved_status in [0x0015, 0x230a] {
        let mut context = TestContext::new(0x2600);
        let initial = context.banks;
        context.write_u16(initial.supervisor, saved_status);
        context.write_u32(initial.supervisor + 2, 0x00123456);
        context.write_u32(initial.user, 0x76543210);
        let frame = restore_exception(&mut context).unwrap();
        assert_eq!(frame.status_register, saved_status);
        assert_eq!(frame.program_counter, 0x00123456);
        assert_eq!(context.status, saved_status);
        assert_eq!(context.banks.user, initial.user);
        assert_eq!(context.banks.supervisor, initial.supervisor + 6);
        assert_eq!(context.read_u32(initial.user), 0x76543210);
        assert_eq!(context.read_u16(initial.supervisor), saved_status);
    }
}

#[test]
fn rte_in_user_mode_fails_without_consuming_any_stack() {
    let mut context = TestContext::new(0x0004);
    let initial = context.banks;
    assert!(restore_exception(&mut context).is_err());
    assert_eq!(context.banks, initial);
    assert_eq!(context.status, 0x0004);
}

#[test]
fn call_transfer_pushes_the_real_return_pc_on_the_active_bank_only() {
    for supervisor in [false, true] {
        let mut context = TestContext::new(if supervisor { 0x271f } else { 0x001f });
        let initial = context.banks;
        let status = context.status;
        push_call(&mut context, 0x322).unwrap();
        let sp = if supervisor {
            context.banks.supervisor
        } else {
            context.banks.user
        };
        assert_eq!(context.read_u32(sp), 0x322);
        assert_eq!(context.status, status);
        if supervisor {
            assert_eq!(sp, initial.supervisor - 4);
            assert_eq!(context.banks.user, initial.user);
        } else {
            assert_eq!(sp, initial.user - 4);
            assert_eq!(context.banks.supervisor, initial.supervisor);
        }
    }
}

#[test]
fn reviewed_boundaries_are_explicit_in_both_revisions() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        assert_eq!(contract(ids::LEVEL4_ISR), ExitContract::ExceptionReturn);
        assert_eq!(contract(ids::LEVEL6_ISR), ExitContract::ExceptionReturn);
        assert_eq!(contract(ids::HANDLE_H_BLANK), ExitContract::Rts);
        assert_eq!(contract(ids::HANDLE_V_BLANK), ExitContract::Rts);
        assert!(contract(ids::RESTART_FROM_RESET_ENTRY).accepts(
            ids::RESTART_FROM_RESET_ENTRY,
            Exit::Tail { target: ids::START },
            revision,
        ));
        for (entry, target, return_offset) in [
            (ids::GAME_PROGRAM, ids::VDP_INITIALISATION, 38),
            (ids::VDP_INITIALISATION, ids::FURTHER_INITIALISATION, 8),
        ] {
            let return_pc = entry.metadata().address(revision) + return_offset;
            assert!(contract(entry).accepts(entry, Exit::Call { target, return_pc }, revision));
            assert!(!contract(entry).accepts(
                entry,
                Exit::Call {
                    target,
                    return_pc: return_pc + 2
                },
                revision
            ));
        }
        let entry = ids::FURTHER_INITIALISATION;
        for (stage, offset) in [
            (MainLoopStage::AfterIntro, 0xe0),
            (MainLoopStage::AfterMenu, 0xe6),
            (MainLoopStage::Begin, 0x260),
            (MainLoopStage::Idle, 0x2fc),
        ] {
            let pc = contract(entry).checkpoint(entry, stage, revision).unwrap();
            assert_eq!(pc, entry.metadata().address(revision) + offset);
        }
        assert_eq!(
            MainLoopContinuation::from_return(entry, 0x1366, revision)
                .unwrap()
                .stage,
            MainLoopStage::AfterIntro
        );
        assert_eq!(
            MainLoopContinuation::from_return(entry, 0x136c, revision)
                .unwrap()
                .stage,
            MainLoopStage::AfterMenu
        );
        assert!(MainLoopContinuation::from_return(entry, 0x136e, revision).is_none());
    }
}

#[test]
fn normal_return_and_wrong_source_or_destination_fail_closed() {
    let revision = Revision::Rev02;
    for entry in [
        ids::ERROR_TRAP,
        ids::LEVEL4_ISR,
        ids::LEVEL6_ISR,
        ids::START,
        ids::GAME_PROGRAM,
        ids::FURTHER_INITIALISATION,
    ] {
        assert!(validate(entry, contract(entry), Ok(None), revision).is_err());
    }
    assert!(
        validate(
            ids::START,
            contract(ids::START),
            tail(ids::START, ids::VDP_INITIALISATION),
            revision
        )
        .is_err()
    );
    assert!(
        validate(
            ids::GET_RANDOM_NUMBER,
            ExitContract::Rts,
            terminal(ids::ERROR_TRAP),
            revision
        )
        .is_err()
    );
    let expected = ControlTransfer {
        entry: ids::RESTART_FROM_RESET_ENTRY,
        exit: Exit::Tail { target: ids::START },
    };
    assert_eq!(
        validate(
            ids::FURTHER_INITIALISATION,
            contract(ids::FURTHER_INITIALISATION),
            Err(Error::ControlTransfer(expected)),
            revision
        )
        .unwrap(),
        EntryOutcome::Transfer(expected)
    );
}

#[test]
fn authored_exit_addresses_are_not_replaced_by_catalogue_addresses() {
    let contract = ExitContract::Call {
        target: ids::START,
        return_offset: 6,
    };
    let result = call_transfer(ids::GET_RANDOM_NUMBER, ids::START, 0xff1006);
    let outcome = validate_at(
        ids::GET_RANDOM_NUMBER,
        contract,
        0xff1000,
        result,
        Revision::Rev02,
    )
    .unwrap();
    assert!(matches!(outcome, EntryOutcome::Transfer(_)));
    assert!(contract.accepts_at(
        0xff1000,
        Exit::Call {
            target: ids::START,
            return_pc: 0xff1006
        }
    ));
}

#[test]
fn a_nonreturning_event_propagates_without_executing_caller_continuation() {
    fn caller(continued: &mut bool) -> Result<Option<u32>> {
        terminal::<u32>(ids::ERROR_TRAP)?;
        *continued = true;
        Ok(Some(123))
    }
    let mut continued = false;
    let outcome = validate(
        ids::ERROR_TRAP,
        ExitContract::Terminal,
        caller(&mut continued),
        Revision::Rev02,
    )
    .unwrap();
    assert_eq!(
        outcome,
        EntryOutcome::Transfer(ControlTransfer {
            entry: ids::ERROR_TRAP,
            exit: Exit::Terminal
        })
    );
    assert!(!continued);
}

#[path = "exits_dispatch_tests.rs"]
mod dispatch;
