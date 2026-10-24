//! Structural CALL-window probes, not whole-caller translation coverage. The
//! original prefix reaches a real callsite; Rust owns its real JSR, then invokes
//! the canonical Atomic/Functional timing provider with the same inherited clock.
use super::*;
use crate::{
    Mode,
    input::InputScript,
    sinks::{HeadlessAudio, HeadlessOutput, Inputs, NoCartridgeSaves},
    timing_region::{
        self, ProgressOwner, RustInstruction, TimingHost, TimingLedger, rust_bus_effect,
    },
};
use jgenesis_common::frontend::{EmulatorTrait, PartialClone, TickEffect};
use std::{collections::BTreeMap, fs, sync::Arc};
use tje_engine_core::{
    CpuState,
    timing_region::{BusObservation, ExpectedAccess, RustPathChoice},
};
use tje_engine_runtime::{
    portable_timing::CallProfile,
    timing_region::{
        CalleeKind, RegionCall, RegionEdge, RegionStorage, ReviewedRegion, TimingProgram,
        TimingRegion,
    },
};
use tje_game::{FramedTranslation, ids, platform::timed, timing::TimingSource};
use tje_rom::Rom;

static FRAMED: &[FramedTranslation] = &[
    FramedTranslation {
        id: ids::GET_CTRL1_INPUT,
        function: timed::get_ctrl1_input,
    },
    FramedTranslation {
        id: ids::COPY_PALETTE_TO_CRAM_SLOT,
        function: timed::copy_palette_to_cram_slot,
    },
];
struct JsrSource {
    entry: CpuState,
    next: u32,
    return_pc: u32,
    writes: usize,
}
impl RustInstruction for JsrSource {
    fn entry_state(&self) -> &CpuState {
        &self.entry
    }
    fn effect(
        &mut self,
        emulator: &mut GenesisEmulator,
        expected: ExpectedAccess,
    ) -> Result<BusObservation> {
        let value = if expected.kind == tje_engine_core::timing_region::AccessKind::Write {
            let value = match self.writes {
                0 => self.return_pc >> 16,
                1 => self.return_pc & 0xffff,
                _ => {
                    return Err(HostError::Metadata(
                        "real JSR emitted an extra stack transfer".into(),
                    ));
                }
            };
            self.writes += 1;
            Some(value)
        } else {
            None
        };
        rust_bus_effect(emulator, expected, value)
    }
    fn finish(&mut self, emulator: &mut GenesisEmulator) -> Result<RustPathChoice> {
        assert_eq!(self.writes, 2);
        set_sp(emulator.hybrid_parts().0, self.entry.read_address(7) - 4);
        Ok(RustPathChoice { next_pc: self.next })
    }
}
#[derive(Default)]
struct RootBudget;
impl TimingLedger for RootBudget {
    fn before_step(&mut self, _: ProgressOwner, clock: &mut WaitClock) -> Result<()> {
        clock.call_clock.before_tick()?;
        Ok(())
    }
    fn record_step(
        &mut self,
        _: ProgressOwner,
        step: tje_engine_runtime::DeviceStep,
        _: &GenesisEmulator,
        clock: &mut WaitClock,
    ) -> Result<()> {
        clock.call_clock.record(tje_game::execution::ClockUsage {
            ticks: 1,
            instructions: 0,
            master_clocks: step.master_clocks,
            frames: u64::from(step.frame_complete),
            dispatch_steps: 0,
        })?;
        Ok(())
    }
}
fn actual_calls(cartridge: &Rom, target: FunctionId) -> BTreeMap<u32, (FunctionId, u32, u32)> {
    use tje_engine_core::decode::{AddressingMode, Instruction, decode};
    let program = tje_engine_core::metadata::compile_program(
        cartridge.bytes(),
        &tje_engine_core::metadata::game_metadata(cartridge.revision()),
    )
    .unwrap();
    let target_pc = target.metadata().address(cartridge.revision());
    program
        .instructions
        .iter()
        .filter_map(|instruction| {
            if decode(instruction.opcode)
                != Instruction::JumpToSubroutine(AddressingMode::AbsoluteLong)
            {
                return None;
            }
            let start = instruction.pc as usize + 2;
            let address =
                u32::from_be_bytes(cartridge.bytes()[start..start + 4].try_into().unwrap());
            if address != target_pc {
                return None;
            }
            let next = program.instruction(instruction.next_pc)?;
            if matches!(
                decode(next.opcode),
                Instruction::JumpToSubroutine(_) | Instruction::BranchToSubroutine(_)
            ) {
                return None;
            }
            let caller = tje_game::functions()
                .filter(|function| {
                    let start = function.address(cartridge.revision());
                    function.callees.contains(&target)
                        && instruction.pc >= start
                        && instruction.pc < start + function.size(cartridge.revision())
                        && tje_game::execution_class(function.id).is_some_and(|class| {
                            class.permits(tje_game::execution_class(target).unwrap())
                        })
                })
                .max_by_key(|function| function.address(cartridge.revision()))?;
            Some((
                instruction.pc,
                (caller.id, instruction.next_pc, next.next_pc),
            ))
        })
        .collect()
}

#[test]
#[ignore = "private ROMs; real source-controller JSR to canonical Atomic/Functional providers"]
fn actual_atomic_and_functional_child_invocations_keep_class_and_charge_each_instruction() {
    for (revision, key) in [
        (tje_rom::Revision::Rev02, "TJE_REV02_ROM"),
        (tje_rom::Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let cartridge = Rom::import(&fs::read(std::env::var(key).unwrap()).unwrap()).unwrap();
        for (target, leaf, expected_class) in [
            (
                ids::GET_CTRL1_INPUT,
                timed::Leaf::Controller1,
                ExecutionClass::Atomic,
            ),
            (
                ids::COPY_PALETTE_TO_CRAM_SLOT,
                timed::Leaf::Palette,
                ExecutionClass::Functional,
            ),
        ] {
            assert_eq!(tje_game::execution_class(target), Some(expected_class));
            let candidates = actual_calls(&cartridge, target);
            assert!(!candidates.is_empty());
            let mut emulator = GenesisEmulator::create(
                Some(cartridge.bytes().to_vec()),
                genesis_config::GenesisEmulatorConfig::default(),
                &mut NoCartridgeSaves,
            );
            let inputs = Arc::new(InputScript::default());
            let mut clock = WaitClock {
                inputs: inputs.at(0),
                poller: Some(inputs),
                ..WaitClock::default()
            };
            let mut audio = HeadlessAudio::default();
            let (caller, resume, after) = (0..30_000_000)
                .find_map(|_| {
                    let cpu = emulator.hybrid_cpu();
                    let bus = emulator.hybrid_bus();
                    if let Some(&call) = candidates.get(&cpu.pc())
                        && cpu.hybrid_instruction_ready(bus)
                        && bus.cycles.m68k_wait_cpu_cycles == 0
                    {
                        return Some(call);
                    }
                    let event = emulator
                        .tick(
                            &mut HeadlessOutput,
                            &mut audio,
                            &mut Inputs(clock.inputs),
                            &mut NoCartridgeSaves,
                        )
                        .unwrap();
                    audio.samples.clear();
                    if event == TickEffect::FrameRendered {
                        clock.advance_frame(&mut emulator).unwrap();
                    }
                    None
                })
                .expect("real canonical A/F callsite not reached");
            let call_pc = emulator.hybrid_cpu().pc();
            let root_sp = emulator.hybrid_cpu().stack_pointer();
            let parent_class = tje_game::execution_class(caller).unwrap();
            let mut reference = GenesisEmulator::create(
                Some(cartridge.bytes().to_vec()),
                genesis_config::GenesisEmulatorConfig::default(),
                &mut NoCartridgeSaves,
            );
            reference.load_state(emulator.partial_clone());
            let mut reference_clock = clock.clone();
            let reference_root = reference_clock
                .call_clock
                .enter(
                    caller,
                    tje_game::execution_contract(caller)
                        .and_then(|contract| contract.operational_ceiling)
                        .unwrap_or(tje_game::execution::CallCeiling::DEFAULT),
                )
                .unwrap();
            let source = timed::Source::new(revision, leaf);
            let dispatcher = Dispatcher::hybrid_with_frames(&[], FRAMED);
            let mut services = crate::timing_region::game_bridge::TimingServices::prepare(
                &cartridge,
                &[source.program()],
                7,
                dispatcher,
                Mode::Check,
            )
            .unwrap();
            let program = TimingProgram::compile(
                cartridge.bytes(),
                ReviewedRegion {
                    name: "real source-controller A/F call window",
                    source: "actual ROM-bound JSR and canonical timed callee",
                    entry: call_pc,
                    instruction_pcs: vec![call_pc, resume],
                    edges: vec![RegionEdge {
                        from: resume,
                        to: after,
                    }],
                    calls: vec![RegionCall {
                        from: call_pc,
                        target: target.metadata().address(revision),
                        resume,
                    }],
                    return_pc: Some(after),
                    maximum_steps: u64::MAX,
                    required_interrupts: &[4, 6],
                },
                7,
            )
            .unwrap();
            let mut storage = RegionStorage::new(&program);
            let mut context = HookContext::new(&mut emulator, revision, dispatcher, Mode::Check)
                .with_timing_services(Some(&mut services))
                .with_execution(parent_class, clock)
                .with_check_route(crate::CheckForkRoute::ClockedStrict);
            let root = tje_game::Context::enter_call_ceiling(&mut context, caller).unwrap();
            let initial = timing_region::state(context.emulator);
            let mut region = TimingRegion::begin_oracle(
                &program,
                &initial,
                program.fixed_target().unwrap(),
                &mut storage,
            )
            .unwrap();
            let parent = context
                .timing_services
                .as_deref_mut()
                .unwrap()
                .authority
                .completion
                .begin(caller, root_sp, &region)
                .unwrap();
            let mut budget = RootBudget;
            {
                let mut host = TimingHost::with_audio(
                    context.emulator,
                    &mut context.wait_clock,
                    &mut budget,
                    Vec::new(),
                )
                .unwrap();
                host.step_source(
                    &mut region,
                    &mut JsrSource {
                        entry: initial,
                        next: target.metadata().address(revision),
                        return_pc: resume,
                        writes: 0,
                    },
                )
                .unwrap();
                context.time_audio = host.into_audio();
            }
            let token = region
                .authorize_call(&timing_region::state(context.emulator))
                .unwrap();
            let parked = region.park().unwrap();
            let parent_frame = context.frame_state;
            context
                .invoke_timing_callee(&parent, target, &token)
                .unwrap();
            assert_eq!(context.execution_class, expected_class);
            let usage = context
                .timing_services
                .as_deref()
                .unwrap()
                .usage(target)
                .unwrap();
            assert!(usage.instructions > 0 && usage.scheduler_steps > 0 && usage.master_clocks > 0);
            assert_eq!(
                usage.call_profile,
                Some(CallProfile::HostCheckOriginalSubstitution.label())
            );
            let actual = timing_region::state(context.emulator);
            let returned = context
                .timing_services
                .as_deref_mut()
                .unwrap()
                .authority
                .completion
                .finish_child(token, &actual)
                .unwrap();
            assert_eq!(returned.kind(), CalleeKind::TimedRust);
            let mut region = parked.resume(&program, &mut storage).unwrap();
            returned
                .resume(
                    &mut region,
                    &actual,
                    timing_region::boundary_signals(context.emulator),
                )
                .unwrap();
            context.frame_state = parent_frame;
            context.execution_class = parent_class;
            assert_eq!(
                context.cpu().stack_pointer(),
                root_sp,
                "no caller argument release after real child RTS"
            );
            let reference_run = run_original_timed_with_audio(
                &mut reference,
                &mut reference_clock,
                resume,
                root_sp,
                Vec::new(),
                false,
                crate::execution::PhysicalObservers::default(),
            )
            .unwrap();
            reference_clock.call_clock.leave(reference_root).unwrap();
            assert_eq!(
                context.cpu().hybrid_state_words(),
                reference.hybrid_cpu().hybrid_state_words()
            );
            assert_eq!(
                context.wait_clock.call_clock.usage().master_clocks,
                reference_clock.call_clock.usage().master_clocks
            );
            assert_eq!(
                bincode::encode_to_vec(&*context.emulator, bincode::config::standard()).unwrap(),
                bincode::encode_to_vec(&reference, bincode::config::standard()).unwrap()
            );
            assert!(
                context
                    .time_audio
                    .iter()
                    .map(|sample| sample.map(f64::to_bits))
                    .eq(reference_run
                        .audio
                        .iter()
                        .map(|sample| sample.map(f64::to_bits)))
            );
            assert!(context.wait_clock.call_clock.usage().ticks > 1);
            tje_game::Context::leave_call_ceiling(&mut context, root).unwrap();
            eprintln!(
                "{revision:?}: real source-controller call to {} retains {expected_class:?}, {} source instructions; exact CPU/devices/audio and shared actual clock",
                target.metadata().name,
                usage.instructions
            );
        }
    }
}
