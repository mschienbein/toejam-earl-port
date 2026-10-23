//! Dispatcher regressions: an entry event must not become a stale D0 return.
use super::*;

struct DispatchContext {
    inner: TestContext,
    dispatcher: Dispatcher,
    class: ExecutionClass,
    original_calls: usize,
    revision: Revision,
    frame: crate::call_frame::FrameState,
}
impl DispatchContext {
    fn new(translations: &'static [crate::Translation]) -> Self {
        let inner = TestContext::new(0x2700);
        let frame = crate::call_frame::FrameState::new(inner.banks.supervisor);
        Self {
            inner,
            dispatcher: Dispatcher::hybrid_with(translations),
            class: ExecutionClass::TimeAware,
            original_calls: 0,
            revision: Revision::Rev02,
            frame,
        }
    }
}
impl Bus for DispatchContext {
    fn read_u8(&self, address: u32) -> u8 {
        self.inner.read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.inner.write_u8(address, value);
    }
}
impl tje_mem::Platform for DispatchContext {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("unexpected device write");
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("unexpected device write");
    }
}
impl Platform for DispatchContext {
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
impl Context for DispatchContext {
    fn revision(&self) -> Revision {
        self.revision
    }
    fn data_register(&self, index: usize) -> u32 {
        self.inner.data[index]
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.inner.data[index] = value;
    }
    fn address_register(&self, index: usize) -> u32 {
        self.inner.address[index]
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.inner.address[index] = value;
    }
    fn status_register(&self) -> u16 {
        self.inner.status
    }
    fn set_status_register(&mut self, value: u16) {
        self.inner.status = value;
    }
    fn stack_banks(&self) -> Result<StackBanks> {
        Ok(self.inner.banks)
    }
    fn set_stack_banks(&mut self, banks: StackBanks) -> Result<()> {
        self.inner.banks = banks;
        Ok(())
    }
    fn stack_pointer(&self) -> Result<u32> {
        Ok(if self.inner.status & 0x2000 != 0 {
            self.inner.banks.supervisor
        } else {
            self.inner.banks.user
        })
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        if self.inner.status & 0x2000 != 0 {
            self.inner.banks.supervisor = value;
        } else {
            self.inner.banks.user = value;
        }
        Ok(())
    }
    fn frame_state(&self) -> Option<crate::call_frame::FrameState> {
        Some(self.frame)
    }
    fn set_frame_state(&mut self, frame: crate::call_frame::FrameState) -> Result<()> {
        self.frame = frame;
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        self.dispatcher
    }
    fn execution_class(&self) -> ExecutionClass {
        self.class
    }
    fn set_execution_class(&mut self, class: ExecutionClass) -> Result<()> {
        self.class = class;
        Ok(())
    }
    fn call_original(&mut self, _: FunctionId, _: &[u32]) -> Result<u32> {
        self.original_calls += 1;
        Ok(0xfeed)
    }
    fn call_original_with_execution(
        &mut self,
        _: FunctionId,
        _: &[u32],
        _: ExecutionClass,
    ) -> Result<u32> {
        self.original_calls += 1;
        Ok(0xfeed)
    }
}

fn caller(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    crate::call(context, ids::GAME_PROGRAM, &[])?;
    context.set_data_register(7, 0xbad);
    Ok(Some(0xfeed))
}
fn transfers(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let mut banks = context.stack_banks()?;
    banks.supervisor = 0xff6000;
    context.set_stack_banks(banks)?;
    call_transfer(ids::GAME_PROGRAM, ids::VDP_INITIALISATION, 0x322)
}
fn incorrectly_returns(_: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    Ok(Some(0xfeed))
}

static TRANSFERS: &[crate::Translation] = &[
    crate::Translation {
        id: ids::START,
        function: caller,
    },
    crate::Translation {
        id: ids::GAME_PROGRAM,
        function: transfers,
    },
];
static WRONG_RETURN: &[crate::Translation] = &[
    crate::Translation {
        id: ids::START,
        function: caller,
    },
    crate::Translation {
        id: ids::GAME_PROGRAM,
        function: incorrectly_returns,
    },
];

#[test]
fn nested_exit_keeps_the_abandoned_stack_and_never_runs_the_caller_continuation() {
    let mut context = DispatchContext::new(TRANSFERS);
    let result = crate::call(&mut context, ids::START, &[]);
    let expected = ControlTransfer {
        entry: ids::GAME_PROGRAM,
        exit: Exit::Call {
            target: ids::VDP_INITIALISATION,
            return_pc: 0x322,
        },
    };
    assert_eq!(result, Err(Error::ControlTransfer(expected)));
    assert_eq!(context.inner.data[7], 0);
    assert_eq!(context.inner.banks.supervisor, 0xff6000);
    assert_eq!(context.class, ExecutionClass::TimeAware);
    assert_eq!(context.original_calls, 0);
}

#[test]
fn a_non_rts_callee_returning_normally_is_rejected_before_its_caller_continues() {
    let mut context = DispatchContext::new(WRONG_RETURN);
    assert!(crate::call(&mut context, ids::START, &[]).is_err());
    assert_eq!(context.inner.data[7], 0);
    assert_eq!(context.original_calls, 0);
}

#[test]
fn untranslated_non_rts_entries_never_enter_the_rts_only_original_bridge() {
    let mut context = DispatchContext::new(&[]);
    assert_eq!(
        crate::call(&mut context, ids::START, &[]),
        Err(Error::Untranslated(ids::START))
    );
    assert_eq!(context.original_calls, 0);
}

fn framed_caller(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let outgoing = crate::call_frame::OutgoingArguments::push(context, &[0x11223344])?;
    let dispatcher = context.dispatcher();
    outgoing.call(
        context,
        dispatcher,
        ids::GAME_PROGRAM,
        crate::call_frame::CallSite::new(0xffe012),
    )?;
    outgoing.release(context)?;
    context.set_data_register(7, 0xbad);
    Ok(Some(0xfeed))
}

fn atomic_scope_marker(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    assert_eq!(context.execution_class(), ExecutionClass::Atomic);
    context.set_data_register(6, 0x600d);
    Ok(None)
}

fn framed_transfer(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    crate::call(context, ids::DISABLE_ALL_INTERRUPTS, &[])?;
    assert_eq!(context.execution_class(), ExecutionClass::TimeAware);
    context.set_stack_pointer(0xff6000)?;
    context.write_u32(0xff6000, 0x89abcdef);
    call_transfer(
        ids::GAME_PROGRAM,
        ids::VDP_INITIALISATION,
        ids::GAME_PROGRAM.metadata().address(context.revision()) + 38,
    )
}

fn ordinary_error(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    assert_eq!(context.execution_class(), ExecutionClass::Atomic);
    context.set_stack_pointer(0xff5000)?;
    context.write_u32(0xff5000, 0x55667788);
    Err(Error::Platform("authored ordinary call failure".into()))
}

#[test]
fn framed_transfer_keeps_child_frame_and_abandons_caller_cleanup_in_both_revisions() {
    static BODIES: &[crate::Translation] = &[
        crate::Translation {
            id: ids::START,
            function: framed_caller,
        },
        crate::Translation {
            id: ids::GAME_PROGRAM,
            function: framed_transfer,
        },
        crate::Translation {
            id: ids::DISABLE_ALL_INTERRUPTS,
            function: atomic_scope_marker,
        },
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = DispatchContext::new(BODIES);
        context.revision = revision;
        let parent_sp = context.stack_pointer().unwrap();
        let parent_frame = context.frame;
        let result = crate::call(&mut context, ids::START, &[]);
        assert_eq!(
            result,
            Err(Error::ControlTransfer(ControlTransfer {
                entry: ids::GAME_PROGRAM,
                exit: Exit::Call {
                    target: ids::VDP_INITIALISATION,
                    return_pc: ids::GAME_PROGRAM.metadata().address(revision) + 38,
                },
            }))
        );
        assert_eq!(context.stack_pointer().unwrap(), 0xff6000);
        assert_eq!(context.read_u32(0xff6000), 0x89abcdef);
        assert_eq!(context.read_u32(parent_sp - 4), 0x11223344);
        assert_eq!(context.read_u32(parent_sp - 8), 0xffe012);
        assert_eq!(context.frame.entry_sp, parent_sp - 8);
        assert_ne!(context.frame, parent_frame);
        assert_eq!(context.inner.data[7], 0);
        assert_eq!(context.inner.data[6], 0x600d);
        assert_eq!(context.class, ExecutionClass::TimeAware);
        assert_eq!(context.original_calls, 0);
    }
}

#[test]
fn ordinary_framed_errors_restore_bookkeeping_and_leave_cleanup_to_caller() {
    static BODIES: &[crate::Translation] = &[crate::Translation {
        id: ids::ABS,
        function: ordinary_error,
    }];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = DispatchContext::new(BODIES);
        context.revision = revision;
        let parent_sp = context.stack_pointer().unwrap();
        let parent_frame = context.frame;
        let outgoing = crate::call_frame::OutgoingArguments::push(&mut context, &[7]).unwrap();
        let dispatcher = context.dispatcher();
        let error = outgoing
            .call(
                &mut context,
                dispatcher,
                ids::ABS,
                crate::call_frame::CallSite::new(0xffe012),
            )
            .unwrap_err();
        assert_eq!(
            error,
            Error::Platform("authored ordinary call failure".into())
        );
        assert_eq!(context.stack_pointer().unwrap(), parent_sp - 4);
        assert_eq!(context.frame, parent_frame);
        assert_eq!(context.class, ExecutionClass::TimeAware);
        assert_eq!(context.read_u32(0xff5000), 0x55667788);
        outgoing.release(&mut context).unwrap();
        assert_eq!(context.stack_pointer().unwrap(), parent_sp);
    }
}
