use super::*;
use crate::{
    Context, Dispatcher, Error, FramedTranslation, Platform, TranslatedEntry,
    call_frame::{FrameError, FrameState},
    ids,
};
use tje_mem::{AddressSpace, Bus, Revision};

struct FrozenContext {
    memory: AddressSpace,
    frame: FrameState,
    sp: u32,
    frame_updates: usize,
    contract_error: Option<Error>,
}
impl FrozenContext {
    fn new() -> Self {
        Self {
            memory: AddressSpace::new(vec![0; 8], Revision::Rev02),
            frame: FrameState::new(0xff4000),
            sp: 0xff4000,
            frame_updates: 0,
            contract_error: None,
        }
    }
}
impl Bus for FrozenContext {
    fn read_u8(&self, _: u32) -> u8 {
        panic!("no storage read before timing capability admission")
    }
    fn write_u8(&mut self, _: u32, _: u8) {
        panic!("no storage write before timing capability admission")
    }
}
impl tje_mem::Platform for FrozenContext {
    fn write_vdp_control(&mut self, _: u16) {
        panic!("no VDP effect")
    }
    fn write_vdp_data(&mut self, _: u16) {
        panic!("no VDP effect")
    }
    fn write_vdp_command(&mut self, _: u32) {
        panic!("no VDP effect")
    }
}
impl Platform for FrozenContext {
    fn check_mmio_contract(&self) -> crate::Result<()> {
        self.contract_error.clone().map_or(Ok(()), Err)
    }
    fn read_device_u8(&mut self, _: u32) -> u8 {
        panic!("no MMIO effect")
    }
    fn read_device_u16(&mut self, _: u32) -> u16 {
        panic!("no MMIO effect")
    }
    fn write_device_u8(&mut self, _: u32, _: u8) {
        panic!("no MMIO effect")
    }
    fn write_device_u16(&mut self, _: u32, _: u16) {
        panic!("no MMIO effect")
    }
}
impl Context for FrozenContext {
    fn revision(&self) -> Revision {
        self.memory.revision()
    }
    fn data_register(&self, _: usize) -> u32 {
        panic!("no register effect")
    }
    fn set_data_register(&mut self, _: usize, _: u32) {
        panic!("no register effect")
    }
    fn address_register(&self, _: usize) -> u32 {
        panic!("no register effect")
    }
    fn set_address_register(&mut self, _: usize, _: u32) {
        panic!("no register effect")
    }
    fn status_register(&self) -> u16 {
        0x2000
    }
    fn set_status_register(&mut self, _: u16) {
        panic!("no register effect")
    }
    fn stack_pointer(&self) -> Result<u32> {
        Ok(self.sp)
    }
    fn frame_state(&self) -> Option<FrameState> {
        Some(self.frame)
    }
    fn set_frame_state(&mut self, state: FrameState) -> Result<()> {
        self.frame_updates += 1;
        self.frame = state;
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with(&[])
    }
    fn call_original(&mut self, entry: FunctionId, _: &[u32]) -> Result<u32> {
        Err(Error::Untranslated(entry))
    }
}
struct NeverInspect;
impl TimingSource for NeverInspect {
    fn program(&self) -> &'static Program {
        panic!("default must not inspect a source")
    }
    fn begin_instruction(&mut self, _: InstructionSite, _: Registers) -> Result<()> {
        panic!("default must not begin source")
    }
    fn next_effect(&mut self) -> Result<Option<Effect>> {
        panic!("default must not request effects")
    }
    fn acknowledge(&mut self, _: Observation) -> Result<()> {
        panic!("default must not acknowledge effects")
    }
    fn finish_instruction(&mut self) -> Result<Commit> {
        panic!("default must not finish a source")
    }
}
fn entry() -> TranslatedEntry<'static> {
    TranslatedEntry::Framed(&FramedTranslation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: crate::core::wait_for_next_vblank_framed,
    })
}
#[test]
fn missing_capability_does_not_inspect_source_or_touch_game_state() {
    let mut context = FrozenContext::new();
    assert!(
        context
            .execute_timing_program(ids::WAIT_FOR_NEXT_V_BLANK, &mut NeverInspect)
            .is_err()
    );
    assert_eq!(context.frame_updates, 0);
}
#[test]
fn value_only_wrong_target_odd_and_parent_sp_frames_fail_before_source_effects() {
    let mut context = FrozenContext::new();
    assert!(matches!(
        entry().invoke_values(&mut context, &[1]),
        Err(Error::Frame(FrameError::PreparedEntryRequired(_)))
    ));
    assert_eq!(context.frame_updates, 0);
    context.frame = context.frame.for_prepared_entry(ids::HANDLE_V_BLANK);
    assert!(crate::core::wait_for_next_vblank_framed(&mut context).is_err());
    assert_eq!(context.frame_updates, 0);
    context.sp |= 1;
    assert!(entry().invoke_prepared(&mut context).is_err());
    assert_eq!(context.frame_updates, 0);
    context.sp = context.frame.entry_sp.wrapping_sub(4);
    assert!(entry().invoke_prepared(&mut context).is_err());
    assert_eq!(context.frame_updates, 0);
}
#[test]
fn actual_framed_entry_marks_only_its_identity_then_default_fails_closed() {
    let mut context = FrozenContext::new();
    assert!(entry().invoke_prepared(&mut context).is_err());
    assert!(context.frame.is_prepared_for(ids::WAIT_FOR_NEXT_V_BLANK));
    assert!(!context.frame.is_prepared_for(ids::HANDLE_V_BLANK));
    assert_eq!(context.frame_updates, 1);
}

fn source_registers() -> Registers {
    Registers {
        data: [0; 8],
        address: [0; 7],
        user_stack: 0xff2000,
        supervisor_stack: 0xff4000,
        status: 0x2000,
    }
}
fn source_site(role: u16) -> InstructionSite {
    InstructionSite {
        location: Location {
            function: ids::WAIT_FOR_NEXT_V_BLANK,
            role,
        },
        address: 0x200,
        fallthrough: 0x204,
    }
}
#[test]
fn wait_source_requires_each_real_ack_and_poisoned_protocol_cannot_resume() {
    use crate::core::timed_wait::WaitSource;
    for mutation in 0..4 {
        let mut source = WaitSource::new(Revision::Rev02);
        source
            .begin_instruction(source_site(0), source_registers())
            .unwrap();
        let effect = source.next_effect().unwrap().unwrap();
        match mutation {
            0 => assert!(source.next_effect().is_err()),
            1 => {
                let mut wrong = effect;
                wrong.address += 2;
                assert!(
                    source
                        .acknowledge(Observation {
                            effect: wrong,
                            value: 1
                        })
                        .is_err()
                );
            }
            2 => {
                source
                    .acknowledge(Observation { effect, value: 1 })
                    .unwrap();
                assert!(
                    source
                        .acknowledge(Observation { effect, value: 1 })
                        .is_err()
                );
            }
            _ => assert!(
                source
                    .begin_instruction(source_site(0), source_registers())
                    .is_err()
            ),
        }
        assert!(source.finish_instruction().is_err());
    }
    let mut source = WaitSource::new(Revision::Rev02);
    source
        .begin_instruction(source_site(0), source_registers())
        .unwrap();
    let effect = source.next_effect().unwrap().unwrap();
    source
        .acknowledge(Observation { effect, value: 1 })
        .unwrap();
    assert!(source.next_effect().unwrap().is_none());
    let committed = source.finish_instruction().unwrap();
    assert_eq!(committed.registers.data[0], 1);
    assert_eq!(
        committed.path,
        Path::Next(Location {
            function: ids::WAIT_FOR_NEXT_V_BLANK,
            role: 1
        })
    );
}

#[test]
fn failed_timing_is_never_a_transfer_and_contract_checks_keep_first_failure() {
    let mut context = FrozenContext::new();
    let primary = Error::timing_execution_failure(
        ids::WAIT_FOR_NEXT_V_BLANK,
        Error::Platform("first actual failure".into()),
    );
    let secondary = Error::timing_execution_failure(
        ids::WAIT_FOR_NEXT_V_BLANK,
        Error::Clock(crate::execution::CallClockError::Unavailable),
    );
    assert!(primary.is_failed_execution() && primary.preserves_call_frame());
    assert_eq!(
        Error::timing_execution_failure(ids::ABS, primary.clone()),
        primary
    );
    context.contract_error = Some(secondary.clone());
    assert_eq!(
        Error::after_contract_check::<()>(&context, Err(primary.clone())),
        Err(primary.clone())
    );
    let transfer = crate::exits::ControlTransfer {
        entry: ids::HANDLE_V_BLANK,
        exit: crate::exits::Exit::ExceptionReturn,
    };
    assert_eq!(
        Error::after_contract_check::<()>(&context, Err(Error::ControlTransfer(transfer))),
        Err(secondary)
    );
    let clock = crate::execution::CallClockError::Unavailable;
    context.contract_error = Some(Error::Clock(clock));
    let expected = Error::ClockAfterTransfer {
        transfer,
        error: clock,
    };
    assert_eq!(
        Error::after_contract_check::<()>(&context, Err(Error::ControlTransfer(transfer))),
        Err(expected.clone())
    );
    assert_eq!(
        Error::timing_execution_failure(ids::ABS, expected.clone()),
        expected
    );
    for revision in [Revision::Rev02, Revision::Rev00] {
        assert_eq!(
            crate::exits::validate_call(ids::WAIT_FOR_NEXT_V_BLANK, Err(primary.clone()), revision),
            Err(primary.clone()),
            "a failed timing invocation cannot become an approved exit"
        );
    }
}
