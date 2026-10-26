//! Real compiled-IRQ/output failures through the prepared-callee ABI.
use super::*;
use tje_game::{Context, Error, call_frame::CallSite};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureAt {
    RteCapture,
    Output,
    Clock,
    PendingReturn(u32),
}
struct Saved {
    cpu: CpuState,
    peripherals: genesis_core::native::PeripheralState,
    samples: Vec<[u64; 2]>,
    clock: u64,
}
struct RecordFailure {
    at: FailureAt,
    boundaries: usize,
    steps: u64,
    saved: Rc<RefCell<Option<Saved>>>,
}
impl StepObserver for RecordFailure {
    fn completed_step(
        &mut self,
        _: ProgressOwner,
        step: DeviceStep,
        cpu: &CpuState,
        devices: &PlatformDevices,
    ) -> tje_game::Result<()> {
        self.steps += 1;
        let boundaries = devices.exception_trace().unwrap();
        let rte = boundaries[self.boundaries..]
            .iter()
            .any(|event| event.boundary == tje_engine_core::ExceptionBoundary::Returned);
        self.boundaries = boundaries.len();
        let capture = match self.at {
            FailureAt::RteCapture => rte,
            FailureAt::Output => step.frame_complete,
            FailureAt::Clock => self.steps == 8,
            FailureAt::PendingReturn(pc) => cpu.pc == pc,
        };
        if capture && self.saved.borrow().is_none() {
            *self.saved.borrow_mut() = Some(Saved {
                cpu: cpu.clone(),
                peripherals: devices.capture_state(registers(cpu)),
                samples: devices
                    .audio_samples()
                    .iter()
                    .map(|sample| sample.map(f64::to_bits))
                    .collect(),
                clock: devices.clock(),
            });
            if self.at == FailureAt::RteCapture {
                return Err(Error::Platform(
                    "authored actual compiled RTE capture failure".into(),
                ));
            }
        }
        Ok(())
    }
}
struct OutputFailure<'a> {
    timeline: &'a Timeline,
    fail: bool,
}
impl FrameTimeline for OutputFailure<'_> {
    fn latched_inputs(&self, frame: u64) -> tje_game::Result<GenesisInputs> {
        self.timeline.latched_inputs(frame)
    }
    fn sample_inputs(&self, frame: u64) -> tje_game::Result<GenesisInputs> {
        self.timeline.sample_inputs(frame)
    }
    fn before_frame(&self, frame: u64, memory: &mut dyn Bus) -> tje_game::Result<()> {
        self.timeline.before_frame(frame, memory)
    }
    fn frame_completed(&self, frame: u64, devices: &PlatformDevices) -> tje_game::Result<()> {
        if self.fail {
            return Err(Error::Platform(
                "authored actual frame output failure".into(),
            ));
        }
        self.timeline.frame_completed(frame, devices)
    }
}

#[test]
#[ignore = "HELD: selected WAIT Rust is Replay-only; old prepared-failure diagnostic is not native authority"]
fn prepared_callee_failures_keep_actual_native_state_and_do_not_release_arguments() {
    for (revision, rom_key, session_key) in [
        (Revision::Rev02, "TJE_REV02_ROM", "TJE_TIMING_SESSION_REV02"),
        (Revision::Rev00, "TJE_REV00_ROM", "TJE_TIMING_SESSION_REV00"),
    ] {
        let cartridge = Rom::import(&fs::read(std::env::var(rom_key).unwrap()).unwrap()).unwrap();
        let timeline = Timeline::new(
            Session::from_json(&fs::read_to_string(std::env::var(session_key).unwrap()).unwrap())
                .unwrap(),
        );
        let (entry, frame) = capture(&cartridge, &timeline, ids::WAIT_FOR_NEXT_V_BLANK, 837);
        let program =
            metadata::compile_program(cartridge.bytes(), &metadata::game_metadata(revision))
                .unwrap();
        let digest = sha1::Sha1::digest(cartridge.bytes()).into();
        let mut backend = NativeProgram::compile(digest, &program).unwrap();
        let memory = AddressSpace::new(cartridge.bytes().to_vec(), revision);
        let cache = Arc::new(
            BoundProgram::compile(
                &cartridge,
                ids::WAIT_FOR_NEXT_V_BLANK,
                tje_game::core::timed_wait::WaitSource::definition(),
                7,
                &[4, 6],
            )
            .unwrap(),
        );
        for at in [FailureAt::RteCapture, FailureAt::Output, FailureAt::Clock] {
            let devices = DeviceRuntime::from_state(memory.clone(), entry.capture_native_state())
                .unwrap()
                .into();
            let mut machine = Engine::from_state(cpu(&entry), devices, backend, frame);
            machine.devices_mut().enable_exception_trace();
            let capability = CompiledTimingCapability::validate(&machine, &cartridge).unwrap();
            let mut state = NativeSourceState::new(
                Arc::clone(&cache),
                &[],
                tje_engine_runtime::portable_timing::CallProfile::NativeStrictReadable,
            )
            .unwrap();
            let sp = machine.cpu().read_address(7);
            let return_pc = machine.devices().memory().read_u32(sp);
            let mut clock = CallClock::default();
            let ceiling = if at == FailureAt::Clock {
                CallCeiling::new(8, CallCeiling::DEFAULT.frames).unwrap()
            } else {
                CallCeiling::DEFAULT
            };
            let _root = clock.enter(ids::WAIT_FOR_NEXT_V_BLANK, ceiling).unwrap();
            let saved = Rc::new(RefCell::new(None));
            let mut observer = RecordFailure {
                at,
                boundaries: 0,
                steps: 0,
                saved: Rc::clone(&saved),
            };
            let outputs = OutputFailure {
                timeline: &timeline,
                fail: at == FailureAt::Output,
            };
            let dispatcher =
                Dispatcher::native_with_frames(&[ids::WAIT_FOR_NEXT_V_BLANK], &[], FRAMED).unwrap();
            {
                let mut context = NativeTimingContext::new(
                    &mut machine,
                    &cartridge,
                    &capability,
                    &mut clock,
                    &outputs,
                    dispatcher,
                    &mut state,
                )
                .unwrap()
                .with_observer(&mut observer);
                let error = tje_game::call_frame::call_at_void(
                    &mut context,
                    dispatcher,
                    ids::WAIT_FOR_NEXT_V_BLANK,
                    &[2],
                    CallSite::new(return_pc),
                )
                .unwrap_err();
                assert!(
                    matches!(error, Error::TimingExecutionFailure { .. }),
                    "{error}"
                );
                assert!(error.is_failed_execution() && error.preserves_call_frame());
                let actual_frame = context.frame_state().unwrap();
                assert_eq!(actual_frame.entry_sp, sp - 8);
                assert!(actual_frame.is_prepared_for(ids::WAIT_FOR_NEXT_V_BLANK));
            }
            let saved = saved
                .borrow_mut()
                .take()
                .expect("actual failure boundary captured");
            assert_eq!(
                machine.cpu(),
                &saved.cpu,
                "{revision:?} {at:?}: exact failed CPU/pipeline/banks"
            );
            assert_eq!(machine.devices().clock(), saved.clock);
            assert_peripheral(
                &machine.devices().capture_state(registers(machine.cpu())),
                &saved.peripherals,
            );
            assert!(
                machine
                    .devices()
                    .audio_samples()
                    .iter()
                    .map(|sample| sample.map(f64::to_bits))
                    .eq(saved.samples)
            );
            assert_ne!(
                machine.cpu().read_address(7),
                sp,
                "caller arguments were not released"
            );
            let never = NeverSample(std::cell::Cell::new(0));
            let rejected = NativeTimingContext::new(
                &mut machine,
                &cartridge,
                &capability,
                &mut clock,
                &never,
                dispatcher,
                &mut state,
            );
            match rejected {
                Err(error) => assert!(error.preserves_call_frame()),
                Ok(_) => panic!("actual failed source re-admitted without a reset"),
            }
            assert_eq!(never.0.get(), 0);
            (_, _, backend) = machine.into_parts();
        }
        for preboxed in [
            Error::timing_execution_failure(
                ids::WAIT_FOR_NEXT_V_BLANK,
                Error::Platform("preboxed actual native source failure".into()),
            ),
            Error::ClockAfterTransfer {
                transfer: tje_game::exits::ControlTransfer {
                    entry: ids::WAIT_FOR_NEXT_V_BLANK,
                    exit: tje_game::exits::Exit::ExceptionReturn,
                },
                error: tje_game::execution::CallClockError::Unavailable,
            },
            Error::ControlTransfer(tje_game::exits::ControlTransfer {
                entry: ids::WAIT_FOR_NEXT_V_BLANK,
                exit: tje_game::exits::Exit::Terminal,
            }),
        ] {
            let mut devices = PlatformDevices::from(
                DeviceRuntime::from_state(memory.clone(), entry.capture_native_state()).unwrap(),
            );
            let mut initial = cpu(&entry);
            initial.sr |= 0x700;
            let sp = initial.read_address(7);
            let return_pc = devices.memory().read_u32(sp);
            devices.memory_mut().write_u16(sp + 6, 0);
            let mut machine = Engine::from_state(initial, devices, backend, frame);
            machine.devices_mut().enable_exception_trace();
            let capability = CompiledTimingCapability::validate(&machine, &cartridge).unwrap();
            let mut state = NativeSourceState::new(
                Arc::clone(&cache),
                &[],
                tje_engine_runtime::portable_timing::CallProfile::NativeStrictReadable,
            )
            .unwrap();
            let mut clock = CallClock::default();
            let _root = clock
                .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
                .unwrap();
            let saved = Rc::new(RefCell::new(None));
            let mut observer = RecordFailure {
                at: FailureAt::PendingReturn(return_pc),
                boundaries: 0,
                steps: 0,
                saved: Rc::clone(&saved),
            };
            let dispatcher =
                Dispatcher::native_with_frames(&[ids::WAIT_FOR_NEXT_V_BLANK], &[], FRAMED).unwrap();
            {
                let mut context = NativeTimingContext::new(
                    &mut machine,
                    &cartridge,
                    &capability,
                    &mut clock,
                    &timeline,
                    dispatcher,
                    &mut state,
                )
                .unwrap()
                .with_observer(&mut observer);
                assert_eq!(
                    tje_game::exits::enter(&mut context, ids::WAIT_FOR_NEXT_V_BLANK, &[]).unwrap(),
                    tje_game::exits::EntryOutcome::Returned(None),
                );
                let frame_before = context.frame_state();
                let expected = if preboxed.is_failed_execution() {
                    preboxed.clone()
                } else {
                    Error::timing_execution_failure(ids::WAIT_FOR_NEXT_V_BLANK, preboxed.clone())
                };
                assert_eq!(context.protect_timing_failure(preboxed), expected);
                assert!(
                    context
                        .consume_timing_completion(tje_game::timing::CompletionRequest {
                            entry: ids::WAIT_FOR_NEXT_V_BLANK,
                            entry_sp: sp,
                            kind: tje_game::timing::CompletionKind::Rts,
                            expected_pc: Some(return_pc),
                            value: None,
                        })
                        .is_err(),
                    "a caught preboxed fatal error cannot consume its formerly valid receipt",
                );
                assert_eq!(context.frame_state(), frame_before);
            }
            let saved = saved.borrow_mut().take().expect("actual completed RTS");
            assert_eq!(machine.cpu(), &saved.cpu);
            assert_eq!(machine.devices().clock(), saved.clock);
            assert_peripheral(
                &machine.devices().capture_state(registers(machine.cpu())),
                &saved.peripherals,
            );
            assert!(
                machine
                    .devices()
                    .audio_samples()
                    .iter()
                    .map(|sample| sample.map(f64::to_bits))
                    .eq(saved.samples)
            );
            let never = NeverSample(std::cell::Cell::new(0));
            assert!(
                NativeTimingContext::new(
                    &mut machine,
                    &cartridge,
                    &capability,
                    &mut clock,
                    &never,
                    dispatcher,
                    &mut state,
                )
                .is_err(),
            );
            assert_eq!(never.0.get(), 0);
            (_, _, backend) = machine.into_parts();
        }
        eprintln!(
            "{revision:?}: prepared real compiled-RTE capture, actual frame output and clock failures retain exact CPU/devices/pending state/audio; no caller release, no re-admission"
        );
    }
}
