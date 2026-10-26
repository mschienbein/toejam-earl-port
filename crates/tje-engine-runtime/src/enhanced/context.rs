use super::{Budget, Driver, EnhancedInputs, Sidecars};
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
    mode: &'a Driver,
    engine: &'a mut Engine<B>,
    inputs: GenesisInputs,
    dispatcher: Dispatcher,
    frame: FrameState,
    class: ExecutionClass,
    budget: &'a mut Budget,
    sidecars: Option<&'a mut Sidecars>,
    enhanced_inputs: EnhancedInputs,
    entry_sp: u32,
    entry_id: Option<FunctionId>,
}
impl<'a, B: Backend> Context<'a, B> {
    pub(super) fn new(
        mode: &'a Driver,
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
            sidecars: None,
            enhanced_inputs: inputs.into(),
            entry_sp: sp,
            entry_id: None,
        }
    }

    pub(super) fn with_extra(
        mut self,
        sidecars: &'a mut Sidecars,
        inputs: EnhancedInputs,
        id: FunctionId,
    ) -> Self {
        self.sidecars = Some(sidecars);
        self.enhanced_inputs = inputs;
        self.entry_id = Some(id);
        self.class = ExecutionClass::TimeAware;
        self
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
        loop {
            let opcode = self.engine.cpu().prefetch as u16;
            let before_sp = self.engine.cpu().read_address(7);
            let step = self
                .mode
                .step_inner(self.engine, self.inputs, self.budget)
                .map_err(|cause| match cause {
                    super::Error::Override(cause) => cause,
                    cause => error(&cause.to_string()),
                })?;
            if before_sp == sp
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
                // This actual RTS is the owned return boundary, even when its
                // device advance has made an IRQ ready. Do not enter that IRQ
                // (or execute the caller) while trying to rediscover the same
                // return PC. The outer driver retains all pending device/IRQ
                // state and services it when this bounded callback releases.
                self.engine.devices_mut().set_instruction_pc(saved_pc);
                return Ok(());
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
        if class == ExecutionClass::TimeAware && self.sidecars.is_none() {
            return Err(error(
                "synchronous callbacks must use a resumable override for TimeAware execution",
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

impl<B: Backend> super::Context for Context<'_, B>
where
    B::Error: fmt::Display,
{
    fn sidecars(&self) -> &Sidecars {
        self.sidecars.as_deref().expect("resumable context")
    }

    fn sidecars_mut(&mut self) -> &mut Sidecars {
        self.sidecars.as_deref_mut().expect("resumable context")
    }

    fn state_and_memory(&mut self) -> (&mut Sidecars, &tje_mem::AddressSpace) {
        (
            self.sidecars.as_deref_mut().expect("resumable context"),
            self.engine.devices().memory(),
        )
    }

    fn inputs(&self) -> EnhancedInputs {
        self.enhanced_inputs
    }

    fn frame_count(&self) -> u64 {
        self.engine.frame_count()
    }

    fn entry_arguments(&self) -> Result<Vec<u32>> {
        let id = self.entry_id.expect("resumable context");
        let count = u32::from(id.metadata().abi.argument_count);
        if self.entry_sp.saturating_add(4 * count + 4) > 0x1000000 {
            return Err(error("enhanced entry arguments exceed work RAM"));
        }
        Ok((0..count)
            .map(|index| {
                self.engine
                    .devices()
                    .memory()
                    .read_u32(self.entry_sp + 4 + index * 4)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CpuState, StepResult};
    use genesis_config::GenesisEmulatorConfig;
    use std::convert::Infallible;
    use tje_game::ids;
    use tje_mem::AddressSpace;
    use tje_native_platform::DeviceRuntime;

    /// An authored RTS backend admits a pending IRQ at the exact return edge.
    /// It is structural scheduler evidence, not a retail instruction proof.
    struct ReturnWithIrq;
    impl Backend for ReturnWithIrq {
        type Error = Infallible;
        fn execute(
            &mut self,
            cpu: &mut CpuState,
            bus: &mut dyn EngineBus,
        ) -> std::result::Result<StepResult, Self::Error> {
            let sp = cpu.read_address(7);
            let high = bus.read16(sp, ReadKind::Semantic);
            let low = bus.read16(sp + 2, ReadKind::Semantic);
            cpu.pc = (u32::from(high) << 16) | u32::from(low);
            cpu.write_address(7, sp + 4);
            cpu.prefetch = u32::from(bus.read16(cpu.pc, ReadKind::Prefetch));
            cpu.pending_interrupt = 6;
            Ok(StepResult::normal(16))
        }
    }
    #[test]
    fn synchronous_child_returns_at_its_rts_without_draining_a_pending_irq() {
        for revision in Revision::ALL {
            let mut rom = vec![0; 0x100000];
            rom[..4].copy_from_slice(&0x00ff8000_u32.to_be_bytes());
            rom[4..8].copy_from_slice(&0x200_u32.to_be_bytes());
            rom[0x200..0x202].copy_from_slice(&0x4e71_u16.to_be_bytes());
            let child = ids::ABS.metadata().address(revision) as usize;
            rom[child..child + 2].copy_from_slice(&0x4e75_u16.to_be_bytes());
            let mut engine = Engine::from_devices(
                DeviceRuntime::new(
                    AddressSpace::new(rom, revision),
                    &GenesisEmulatorConfig::default(),
                ),
                ReturnWithIrq,
            );
            engine.cpu_mut().write_address(7, 0xff7000);
            engine.cpu_mut().set_sr(0x2000);
            let driver = Driver::default();
            let mut budget = Budget::default();
            {
                let mut context = Context::new(
                    &driver,
                    &mut engine,
                    GenesisInputs::default(),
                    Dispatcher::hybrid_with(&[]),
                    0xff7000,
                    &mut budget,
                );
                context
                    .call_original_with_execution(ids::ABS, &[7], ExecutionClass::Atomic)
                    .unwrap();
            }
            assert_eq!(engine.cpu().pc, 0x200);
            assert_eq!(engine.cpu().read_address(7), 0xff7000);
            assert_eq!(engine.cpu().pending_interrupt, 6);
            assert!(!super::super::ready(&engine));
        }
    }
}
