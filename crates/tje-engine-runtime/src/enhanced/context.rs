use super::{Budget, EnhancedMode, ready};
use crate::{Backend, Bus as EngineBus, Engine, ReadKind, StepKind};
use genesis_config::GenesisInputs;
use std::fmt;
use tje_game::{
    Context as GameContext, Dispatcher, FunctionId, Result,
    call_frame::{FrameState, OutgoingArguments},
    execution::{CallCeilingScope, ExecutionClass},
    exits::{ExitContract, StackBanks},
};
use tje_mem::{Bus, Revision};

pub(super) fn error(message: &str) -> tje_game::Error {
    tje_game::Error::Platform(message.into())
}

pub(super) fn validate_stack<B: Backend>(engine: &Engine<B>, sp: u32) -> Result<()> {
    if sp & 1 != 0 || !(0xe00000..=0xfffffc).contains(&sp) {
        return Err(error(
            "enhanced callback requires an aligned return address in work RAM",
        ));
    }
    validate_return(engine.devices().memory().read_u32(sp))
}

pub(super) fn validate_return(pc: u32) -> Result<()> {
    if pc & 1 != 0 || pc >= 0x400000 {
        return Err(error(
            "enhanced callback requires an aligned cartridge return PC",
        ));
    }
    Ok(())
}

pub(super) fn set_pc<B: Backend>(engine: &mut Engine<B>, pc: u32) {
    engine.devices_mut().set_read_offset_mclk(0);
    let prefetch = engine.devices_mut().read16(pc, ReadKind::Prefetch);
    engine.cpu_mut().pc = pc;
    engine.cpu_mut().prefetch = u32::from(prefetch);
}

pub(super) struct Context<'a, B: Backend> {
    mode: &'a EnhancedMode,
    engine: &'a mut Engine<B>,
    inputs: GenesisInputs,
    dispatcher: Dispatcher,
    frame: FrameState,
    class: ExecutionClass,
    budget: &'a mut Budget,
}
impl<'a, B: Backend> Context<'a, B> {
    pub(super) fn new(
        mode: &'a EnhancedMode,
        engine: &'a mut Engine<B>,
        inputs: GenesisInputs,
        dispatcher: Dispatcher,
        sp: u32,
        budget: &'a mut Budget,
    ) -> Self {
        let pc = engine.cpu().pc;
        engine.devices_mut().set_instruction_pc(pc);
        engine.devices_mut().set_read_offset_mclk(0);
        Self {
            mode,
            engine,
            inputs,
            dispatcher,
            frame: FrameState::new(sp),
            class: ExecutionClass::Functional,
            budget,
        }
    }
}

impl<B: Backend> Context<'_, B>
where
    B::Error: fmt::Display,
{
    fn original(&mut self, id: FunctionId, class: ExecutionClass) -> Result<()> {
        self.original_inner(id, class).map_err(|cause| {
            if cause.preserves_call_frame() {
                cause
            } else {
                tje_game::Error::EnhancedExecutionFailure {
                    entry: id,
                    cause: Box::new(cause),
                }
            }
        })
    }

    fn original_inner(&mut self, id: FunctionId, class: ExecutionClass) -> Result<()> {
        if class == ExecutionClass::TimeAware || tje_game::exits::contract(id) != ExitContract::Rts
        {
            return Err(error(
                "enhanced callbacks cannot synchronously call a TimeAware or non-RTS function",
            ));
        }
        let sp = self.engine.cpu().read_address(7);
        validate_stack(self.engine, sp)?;
        let return_pc = self.engine.devices().memory().read_u32(sp);
        let saved_pc = self.engine.cpu().pc;
        let entry_pc = id.metadata().address(self.revision());
        set_pc(self.engine, entry_pc);
        let mut returned = false;
        loop {
            if returned && ready(self.engine) {
                if self.engine.cpu().pc != return_pc
                    || self.engine.cpu().read_address(7) != sp.wrapping_add(4)
                {
                    return Err(error(
                        "compiled original call did not retain its RTS boundary",
                    ));
                }
                self.engine.devices_mut().set_instruction_pc(saved_pc);
                return Ok(());
            }
            let opcode = self.engine.cpu().prefetch as u16;
            let before_sp = self.engine.cpu().read_address(7);
            let step = self
                .mode
                .step_inner(self.engine, self.inputs, self.budget)
                .map_err(|cause| match cause {
                    super::Error::Override(cause) => cause,
                    cause => error(&cause.to_string()),
                })?;
            if !returned
                && before_sp == sp
                && (matches!(step.kind, StepKind::Instruction) && opcode == 0x4e75
                    || matches!(step.kind, StepKind::EnhancedOverride { .. }))
            {
                if self.engine.cpu().pc != return_pc
                    || self.engine.cpu().read_address(7) != sp.wrapping_add(4)
                {
                    return Err(error(
                        "compiled original call changed its return address or stack",
                    ));
                }
                returned = true;
            }
        }
    }

    fn original_values(
        &mut self,
        id: FunctionId,
        arguments: &[u32],
        class: ExecutionClass,
    ) -> Result<()> {
        // Value-only callers have no source callsite. Their current real ROM PC
        // is a return location, not an invented trampoline or executable stub.
        let return_pc = self.engine.cpu().pc;
        let outgoing = OutgoingArguments::push(self, arguments)?;
        let parent = self.frame;
        let sp = self.stack_pointer()?.wrapping_sub(4);
        self.write_u32(sp, return_pc);
        self.set_stack_pointer(sp)?;
        self.frame = FrameState::new(sp);
        self.original(id, class)?;
        self.frame = parent;
        outgoing.release(self)
    }
}

impl<B: Backend> Bus for Context<'_, B> {
    fn read_u8(&self, address: u32) -> u8 {
        self.engine.devices().memory().read_u8(address)
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.engine
            .devices_mut()
            .memory_mut()
            .write_u8(address, value);
    }
}
impl<B: Backend> tje_mem::Platform for Context<'_, B> {
    fn write_vdp_control(&mut self, value: u16) {
        self.engine
            .devices_mut()
            .write_immediate_word(0xc00004, value);
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.engine
            .devices_mut()
            .write_immediate_word(0xc00000, value);
    }
}
impl<B: Backend> tje_game::Platform for Context<'_, B> {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        self.engine.devices_mut().read8(address, ReadKind::Semantic)
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        self.engine
            .devices_mut()
            .read16(address, ReadKind::Semantic)
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.engine
            .devices_mut()
            .write_immediate_byte(address, value);
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.engine
            .devices_mut()
            .write_immediate_word(address, value);
    }
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        self.engine
            .devices_mut()
            .read8(address, ReadKind::Discarded);
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        self.engine
            .devices_mut()
            .read16(address, ReadKind::Discarded);
        Ok(())
    }
}
impl<B: Backend> GameContext for Context<'_, B>
where
    B::Error: fmt::Display,
{
    fn revision(&self) -> Revision {
        self.engine.devices().memory().revision()
    }
    fn data_register(&self, index: usize) -> u32 {
        self.engine.cpu().d[index]
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.engine.cpu_mut().d[index] = value;
    }
    fn address_register(&self, index: usize) -> u32 {
        self.engine.cpu().read_address(index as u8)
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.engine.cpu_mut().write_address(index as u8, value);
    }
    fn status_register(&self) -> u16 {
        self.engine.cpu().sr as u16
    }
    fn set_status_register(&mut self, value: u16) {
        self.engine.cpu_mut().set_sr(u32::from(value));
    }
    fn stack_pointer(&self) -> Result<u32> {
        Ok(self.engine.cpu().read_address(7))
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.engine.cpu_mut().write_address(7, value);
        Ok(())
    }
    fn frame_state(&self) -> Option<FrameState> {
        Some(self.frame)
    }
    fn set_frame_state(&mut self, frame: FrameState) -> Result<()> {
        self.frame = frame;
        Ok(())
    }
    fn stack_banks(&self) -> Result<StackBanks> {
        Ok(StackBanks {
            user: self.engine.cpu().usp,
            supervisor: self.engine.cpu().ssp,
        })
    }
    fn set_stack_banks(&mut self, banks: StackBanks) -> Result<()> {
        self.engine.cpu_mut().usp = banks.user;
        self.engine.cpu_mut().ssp = banks.supervisor;
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        self.dispatcher
    }
    fn execution_class(&self) -> ExecutionClass {
        self.class
    }
    fn set_execution_class(&mut self, class: ExecutionClass) -> Result<()> {
        if class == ExecutionClass::TimeAware {
            return Err(error(
                "enhanced Rust callbacks cannot suspend; TimeAware execution is unavailable",
            ));
        }
        self.class = class;
        Ok(())
    }
    fn enter_call_ceiling(&mut self, _: FunctionId) -> Result<CallCeilingScope> {
        // Enhanced budgets are operational limits, not proof CallClock grants.
        Ok(CallCeilingScope::not_required())
    }
    fn call_original(&mut self, id: FunctionId, arguments: &[u32]) -> Result<u32> {
        self.call_original_with_execution(id, arguments, ExecutionClass::Atomic)
    }
    fn call_original_with_execution(
        &mut self,
        id: FunctionId,
        arguments: &[u32],
        class: ExecutionClass,
    ) -> Result<u32> {
        self.original_values(id, arguments, class)?;
        Ok(self.data_register(0))
    }
    fn call_original_void_with_execution(
        &mut self,
        id: FunctionId,
        arguments: &[u32],
        class: ExecutionClass,
    ) -> Result<()> {
        self.original_values(id, arguments, class)
    }
    fn call_original_entry(&mut self, id: FunctionId, class: ExecutionClass) -> Result<u32> {
        self.original(id, class)?;
        Ok(self.data_register(0))
    }
    fn call_original_entry_void(&mut self, id: FunctionId, class: ExecutionClass) -> Result<()> {
        self.original(id, class)
    }
}
