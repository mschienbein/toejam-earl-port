//! Structural evidence for one live compiled JSR/BSR, never a whole source root.
//! This module and its constructor do not exist without `call-window-proofs`.
use super::*;
use crate::timing_region::{
    RegionCall, RegionStorage, ReturnTarget, ReviewedRegion, TimingProgram,
};
use tje_engine_core::{
    decode::{AddressingMode as Ea, Instruction, decode},
    timing_region::RustPathChoice,
};

/// Diagnostic structural-window failure observation only, never authority.
/// It cannot change the first cause, frame, CPU, queues or clock.
pub struct WindowFailureObservation<'a> {
    pub cause: &'a Error,
    pub frame: FrameState,
    pub class: ExecutionClass,
    pub cpu: &'a CpuState,
    pub devices: &'a crate::PlatformDevices,
}

/// A finite structural-window observation, NOT a whole-function completion or
/// readable-coverage receipt. No invocation/completion authority escapes it.
#[derive(Debug)]
pub struct CallWindowReceipt {
    pub scope: &'static str,
    pub caller: FunctionId,
    pub callee: FunctionId,
    pub call_pc: u32,
    pub return_pc: u32,
    pub profile: &'static str,
    pub callee_label: &'static str,
    pub separate_callee_proof_required: bool,
    pub source_instructions: u64,
    pub compiled_original_callee_instructions: u64,
    pub compiled_isr_instructions: u64,
    pub scheduler_steps: u64,
    pub master_clocks: u64,
}

/// Proof-only owned authority for ONE actual settled callsite. The constructor
/// derives caller/callee and the transitive selection seed from the live CPU,
/// immutable attested call instruction and committed metadata. There is no
/// caller-authored seed, source-root descriptor, original-main directive, state
/// import, or whole-root completion API.
pub struct CallWindow<'a, B> {
    machine: &'a mut Engine<B>,
    cartridge: &'a Rom,
    capability: &'a CompiledTimingCapability,
    clock: &'a mut CallClock,
    timeline: &'a dyn FrameTimeline,
    dispatcher: Dispatcher,
    caller: FunctionId,
    callee: FunctionId,
    call_pc: u32,
    resume: u32,
    entry_sp: u32,
    program: TimingProgram,
    storage: RegionStorage,
    state: NativeSourceState,
}
impl<'a, B: TimingBackend> CallWindow<'a, B>
where
    B::Error: std::fmt::Display,
{
    pub fn new(
        machine: &'a mut Engine<B>,
        cartridge: &'a Rom,
        capability: &'a CompiledTimingCapability,
        clock: &'a mut CallClock,
        timeline: &'a dyn FrameTimeline,
        dispatcher: Dispatcher,
        sources: &[&'static game::Program],
    ) -> Result<Self> {
        capability.check(machine, cartridge)?;
        let cpu = machine.cpu();
        clock.before_tick()?;
        if classify_boundary(cpu, signals(machine)) != BoundaryAction::Ready {
            return Err(error(
                "structural call window requires the actual settled instruction boundary",
            ));
        }
        let call_pc = cpu.pc;
        let call = capability
            .calls
            .get(&call_pc)
            .ok_or_else(|| error("live instruction is not an attested compiled JSR/BSR"))?;
        if cpu.prefetch != u32::from(call.opcode)
            || !machine.backend().contains_compiled_instruction(call.resume)
        {
            return Err(error("live call opcode or compiled continuation changed"));
        }
        let target_pc = actual_target(cartridge.bytes(), cpu, call.opcode)?;
        let callee = tje_game::function_at(cartridge.revision(), target_pc)
            .ok_or_else(|| error("actual call EA has no committed function entry"))?;
        let mut callers = tje_game::functions().filter(|function| {
            let start = function.address(cartridge.revision());
            start <= call_pc
                && call_pc < start + function.size(cartridge.revision())
                && function.callees.contains(&callee)
        });
        let caller = callers
            .next()
            .ok_or_else(|| error("actual callsite has no committed caller/callee edge"))?
            .id;
        if callers.next().is_some() {
            return Err(error(
                "actual callsite has ambiguous committed caller authority",
            ));
        }
        if !clock.contains_scope(caller) {
            return Err(error(
                "structural call window requires the actual caller's shared clock scope",
            ));
        }
        if dispatcher.resolve_entry(callee)?.is_some() {
            return Err(error(
                "structural compiled-original call window requires an unselected original callee",
            ));
        }
        let entry_sp = cpu.read_address(7);
        if entry_sp & 1 != 0 || !(0xe00004..=0x00ffffff).contains(&entry_sp) {
            return Err(error(
                "actual callsite stack is not an aligned work-RAM call frame",
            ));
        }
        // Source bindings describe only genuine selected callees. In particular
        // there is NO fabricated BoundProgram for caller A or original B.
        let plans = PreparedSources::new(
            sources
                .iter()
                .map(|source| {
                    BoundProgram::compile(
                        cartridge,
                        source.entry.function,
                        source,
                        capability.divider,
                        &[4, 6],
                    )
                    .map(Arc::new)
                })
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(error)?,
        )
        .map_err(error)?;
        // The sole extra closure root is derived above from the live attested
        // callsite; public/source code cannot supply it or skip this validation.
        let selected_calls = plans
            .validate_window_selection(dispatcher, caller, callee)
            .map_err(PortableError::into_game_error)?;
        let diagnostic = selected_calls.has_diagnostic_candidates();
        let mut completion = CompletionOwners::default();
        completion.prepare_depth(plans.len());
        completion.begin_root_scope().map_err(error)?;
        let state = NativeSourceState {
            root_cache: None,
            parents: Vec::with_capacity(plans.len()),
            irq_frames: Vec::with_capacity(8),
            plans,
            completion,
            profile: CallProfile::NativeCompiledOriginalCallees,
            validated_dispatcher: Some((
                dispatcher,
                CallProfile::NativeCompiledOriginalCallees,
                diagnostic,
            )),
            selected_calls: Some(selected_calls),
        };
        let program = TimingProgram::compile(
            cartridge.bytes(),
            ReviewedRegion {
                name: "structural-one-real-call-window",
                source: "proof-only live compiled callsite; not a source-root translation",
                entry: call_pc,
                instruction_pcs: vec![call_pc],
                edges: vec![],
                calls: vec![RegionCall {
                    from: call_pc,
                    target: target_pc,
                    resume: call.resume,
                }],
                return_pc: Some(call.resume),
                maximum_steps: u64::MAX,
                required_interrupts: &[4, 6],
            },
            capability.divider,
        )
        .map_err(error)?;
        let storage = RegionStorage::new(&program);
        Ok(Self {
            machine,
            cartridge,
            capability,
            clock,
            timeline,
            dispatcher,
            caller,
            callee,
            call_pc,
            resume: call.resume,
            entry_sp,
            program,
            storage,
            state,
        })
    }

    pub fn invoke(
        mut self,
        observer: Option<&mut dyn StepObserver>,
        mut failure: Option<&mut dyn FnMut(WindowFailureObservation<'_>)>,
    ) -> Result<CallWindowReceipt> {
        let mut observed = WindowObserver {
            inner: observer,
            counts: WindowCounts::default(),
        };
        let class =
            tje_game::execution_class(self.caller).ok_or(Error::Unclassified(self.caller))?;
        let inputs = self
            .timeline
            .latched_inputs(self.machine.frame_count())
            .map_err(|cause| {
                // Ordinary missing/stale latches remain retryable; an already
                // fatal protocol cause cannot mint a fresh healthy window.
                let first = self.state.completion.protect_failure(cause);
                if first.is_failed_execution() {
                    self.clock.invalidate_scope();
                    if let Some(inspect) = failure.as_mut() {
                        inspect(WindowFailureObservation {
                            cause: &first,
                            frame: FrameState::new(self.entry_sp),
                            class,
                            cpu: self.machine.cpu(),
                            devices: self.machine.devices(),
                        });
                    }
                }
                first
            })?;
        let mut context = NativeTimingContext {
            machine: &mut *self.machine,
            cartridge: self.cartridge,
            capability: self.capability,
            clock: &mut *self.clock,
            timeline: self.timeline,
            inputs,
            dispatcher: self.dispatcher,
            frame: FrameState::new(self.entry_sp),
            class,
            usage: None,
            observer: Some(&mut observed),
            state: &mut self.state,
        };
        let result = (|| {
            let target =
                ReturnTarget::from_rom(self.cartridge.bytes(), self.resume).map_err(error)?;
            let mut region = TimingRegion::begin_compiled(
                &self.program,
                context.machine.cpu(),
                target,
                &mut self.storage,
            )
            .map_err(error)?;
            region
                .set_call_profile(context.state.profile.label())
                .map_err(error)?;
            let invocation = context
                .state
                .completion
                .begin(self.caller, self.entry_sp, &region)
                .map_err(error)?;
            context.clock.dispatch_step()?;
            context.clock.before_tick()?;
            let target_pc = self.callee.metadata().address(context.cartridge.revision());
            let timing = {
                let mut cursor = region
                    .cursor(&context.machine.cpu, BoundaryAction::Ready)
                    .map_err(error)?;
                context.machine.devices.set_inputs(context.inputs);
                context
                    .machine
                    .devices
                    .begin_instruction(self.call_pc, context.machine.cpu.prefetch as u16);
                loop {
                    match cursor.next_event().map_err(error)? {
                        TimingEvent::Bus { token, expected } => {
                            let value = match expected.kind {
                                AccessKind::Read(kind) => match expected.width {
                                    tje_engine_core::ir::Width::Byte => u32::from(
                                        context.machine.devices.read8(expected.address, kind),
                                    ),
                                    tje_engine_core::ir::Width::Word => u32::from(
                                        context.machine.devices.read16(expected.address, kind),
                                    ),
                                },
                                AccessKind::Write => {
                                    let sp = self.entry_sp.wrapping_sub(4);
                                    if expected.width != tje_engine_core::ir::Width::Word
                                        || ![sp, sp + 2].contains(&expected.address)
                                    {
                                        return Err(error(
                                            "call-window source attempted a non-RA write",
                                        ));
                                    }
                                    let value = if expected.address == sp {
                                        (self.resume >> 16) as u16
                                    } else {
                                        self.resume as u16
                                    };
                                    context.machine.devices.write16(expected.address, value);
                                    u32::from(value)
                                }
                            };
                            cursor
                                .acknowledge_bus(
                                    token,
                                    BusObservation {
                                        address: expected.address,
                                        width: expected.width,
                                        kind: expected.kind,
                                        value,
                                    },
                                )
                                .map_err(error)?;
                        }
                        TimingEvent::SetReadOffset {
                            token,
                            master_clocks,
                        } => {
                            context.machine.devices.set_read_offset_mclk(master_clocks);
                            cursor.acknowledge_offset(token).map_err(error)?;
                        }
                        TimingEvent::InstructionComplete { token, .. } => {
                            break cursor
                                .complete_instruction(token, RustPathChoice { next_pc: target_pc })
                                .map_err(error)?;
                        }
                        TimingEvent::Exception { .. } => {
                            return Err(error("actual call instruction raised an exception"));
                        }
                    }
                }
            };
            let timing = region.accept_instruction(timing).map_err(error)?;
            context
                .machine
                .cpu
                .write_address(7, self.entry_sp.wrapping_sub(4));
            let step = advance_and_commit(
                &mut context.machine.devices,
                &mut context.machine.cpu,
                timing,
            )
            .map_err(error)?;
            context.machine.frames = context
                .machine
                .frames
                .checked_add(u64::from(step.frame_complete))
                .ok_or_else(|| error("native frame counter overflow"))?;
            NativeDrive {
                machine: context.machine,
                clock: context.clock,
                timeline: context.timeline,
                inputs: &mut context.inputs,
                observer: &mut context.observer,
                irq_frames: &mut context.state.irq_frames,
                parents: &mut context.state.parents,
            }
            .record(Some(&mut region), step, ProgressOwner::SourceInstruction)?;
            let token = region
                .authorize_call(context.machine.cpu())
                .map_err(error)?;
            if context
                .machine
                .devices
                .memory()
                .read_u32(token.entry_stack())
                != token.return_pc()
            {
                return Err(error(
                    "real call instruction did not publish its actual RAM return address",
                ));
            }
            let parked = region.park().map_err(error)?;
            let parent_frame = context.frame;
            let parent_class = context.class;
            context.invoke_callee(&invocation, self.callee, &token)?;
            let returned = context
                .state
                .completion
                .finish_child(token, context.machine.cpu())
                .map_err(error)?;
            let kind = returned.kind();
            let mut region = parked
                .resume(&self.program, &mut self.storage)
                .map_err(error)?;
            returned
                .resume(&mut region, context.machine.cpu(), signals(context.machine))
                .map_err(error)?;
            context.frame = parent_frame;
            context.class = parent_class;
            // Deliberately NO root certify/consume/finalize. The owned incomplete
            // window ledger dies here; only the callee's real RTS was sealed.
            Ok(kind)
        })();
        let kind = result.map_err(|cause| {
            let first = context.state.completion.protect_failure(cause);
            // A fresh window cannot evade an admitted failed owner by dropping
            // this affine value and reusing the same physical/shared-clock run.
            if first.is_failed_execution() {
                context.clock.invalidate_scope();
                if let Some(inspect) = failure.as_mut() {
                    inspect(WindowFailureObservation {
                        cause: &first,
                        frame: context.frame,
                        class: context.class,
                        cpu: context.machine.cpu(),
                        devices: context.machine.devices(),
                    });
                }
            }
            first
        })?;
        Ok(CallWindowReceipt {
            scope: "structural call window (not whole-root translation)",
            caller: self.caller,
            callee: self.callee,
            call_pc: self.call_pc,
            return_pc: self.resume,
            profile: CallProfile::NativeCompiledOriginalCallees.label(),
            callee_label: kind.label(),
            separate_callee_proof_required: kind.separate_proof_required(),
            source_instructions: observed.counts.source,
            compiled_original_callee_instructions: observed.counts.original,
            compiled_isr_instructions: observed.counts.isr,
            scheduler_steps: observed.counts.steps,
            master_clocks: observed.counts.clocks,
        })
    }
}

fn actual_target(rom: &[u8], cpu: &CpuState, opcode: u16) -> Result<u32> {
    let word = |at: u32| -> Result<u16> {
        let bytes = rom
            .get(at as usize..at as usize + 2)
            .ok_or_else(|| error("call extension outside immutable ROM"))?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    };
    let base = cpu.pc.wrapping_add(2);
    let indexed = |base: u32, ext: u16| {
        let register = ((ext >> 12) & 7) as u8;
        let value = if ext & 0x8000 != 0 {
            cpu.read_address(register)
        } else {
            cpu.d[register as usize]
        };
        let value = if ext & 0x0800 == 0 {
            value as i16 as i32 as u32
        } else {
            value
        };
        base.wrapping_add(value)
            .wrapping_add(ext as i8 as i32 as u32)
    };
    match decode(opcode) {
        Instruction::BranchToSubroutine(displacement) => {
            Ok(base.wrapping_add(if displacement == 0 {
                word(base)? as i16 as i32 as u32
            } else {
                displacement as i32 as u32
            }))
        }
        Instruction::JumpToSubroutine(ea) => Ok(match ea {
            Ea::AddressIndirect(register) => cpu.read_address(register.0),
            Ea::AddressIndirectDisplacement(register) => cpu
                .read_address(register.0)
                .wrapping_add(word(base)? as i16 as i32 as u32),
            Ea::AddressIndirectIndexed(register) => {
                indexed(cpu.read_address(register.0), word(base)?)
            }
            Ea::PcRelativeDisplacement => base.wrapping_add(word(base)? as i16 as i32 as u32),
            Ea::PcRelativeIndexed => indexed(base, word(base)?),
            Ea::AbsoluteShort => word(base)? as i16 as i32 as u32,
            Ea::AbsoluteLong => (u32::from(word(base)?) << 16) | u32::from(word(base + 2)?),
            _ => return Err(error("attested JSR has an unsupported control EA")),
        }),
        _ => Err(error("attested opcode is not JSR/BSR")),
    }
}
#[derive(Default)]
struct WindowCounts {
    source: u64,
    original: u64,
    isr: u64,
    steps: u64,
    clocks: u64,
}
struct WindowObserver<'a> {
    inner: Option<&'a mut dyn StepObserver>,
    counts: WindowCounts,
}
impl StepObserver for WindowObserver<'_> {
    fn completed_step(
        &mut self,
        owner: ProgressOwner,
        step: DeviceStep,
        cpu: &CpuState,
        devices: &crate::PlatformDevices,
    ) -> Result<()> {
        self.counts.source += u64::from(owner == ProgressOwner::SourceInstruction);
        self.counts.original +=
            u64::from(owner == (ProgressOwner::CompiledOriginalCallee { instruction: true }));
        self.counts.isr +=
            u64::from(owner == (ProgressOwner::CompiledEngineInterrupt { instruction: true }));
        self.counts.steps += 1;
        self.counts.clocks += step.master_clocks;
        match &mut self.inner {
            Some(inner) => inner.completed_step(owner, step, cpu, devices),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_root_window_authority_rejects_the_shared_normal_constructor_and_invoke_guard() {
        let state = NativeSourceState {
            root_cache: None,
            plans: PreparedSources::default(),
            parents: Vec::new(),
            irq_frames: Vec::new(),
            completion: CompletionOwners::default(),
            profile: CallProfile::NativeCompiledOriginalCallees,
            validated_dispatcher: None,
            selected_calls: None,
        };
        // Both ordinary NativeTimingContext::new and invoke_prepared must pass
        // this same guard before admitting a whole source root.
        let Err(cause) = state.root_binding() else {
            panic!("window acquired a normal root binding")
        };
        assert!(cause.to_string().contains("whole source root"));
        assert!(state.root_cache.is_none());
    }
    fn target(cpu: &CpuState, opcode: u16, extension: u32) -> u32 {
        let mut rom = vec![0; 0x200];
        rom[0x102..0x106].copy_from_slice(&extension.to_be_bytes());
        actual_target(&rom, cpu, opcode).unwrap()
    }
    #[test]
    fn control_eas_keep_original_word_index_sign_and_wrapping_displacements() {
        let mut cpu = CpuState {
            pc: 0x100,
            ..CpuState::default()
        };
        cpu.write_address(0, 0x1000);
        cpu.write_address(1, 0x5678_8000);
        cpu.d[1] = 0x1234_ffff;
        assert_eq!(target(&cpu, 0x4eb9, 0x0012_3456), 0x0012_3456);
        assert_eq!(target(&cpu, 0x4eb8, 0x8000_0000), 0xffff_8000);
        assert_eq!(target(&cpu, 0x4e90, 0), 0x1000);
        assert_eq!(target(&cpu, 0x4ea8, 0x8002_0000), 0xffff_9002);
        assert_eq!(target(&cpu, 0x4eb0, 0x10fe_0000), 0xffd);
        assert_eq!(target(&cpu, 0x4eb0, 0x90fe_0000), 0xffff_8ffe);
        assert_eq!(target(&cpu, 0x4eb0, 0x18fe_0000), 0x1235_0ffd);
        assert_eq!(target(&cpu, 0x4eba, 0x8000_0000), 0xffff_8102);
        assert_eq!(target(&cpu, 0x4ebb, 0x10fe_0000), 0xff);
        assert_eq!(target(&cpu, 0x61fe, 0), 0x100);
        assert_eq!(target(&cpu, 0x6100, 0x8000_0000), 0xffff_8102);
        assert!(actual_target(&[0; 4], &cpu, 0x4eb9).is_err());
        assert!(actual_target(&[0; 4], &cpu, 0x4e71).is_err());
    }
}
