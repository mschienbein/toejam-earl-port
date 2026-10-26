use super::*;
use tje_engine_runtime::portable_timing::CallProfile;
use tje_game::Context as _;
use tje_game::cutscenes::timed_clear_lower::{self, ClearLowerSource};
thread_local! {
    static ORPHAN_FRAME: std::cell::Cell<Option<tje_game::call_frame::FrameState>> =
        const { std::cell::Cell::new(None) };
}
fn orphan_after_real_wait(context: &mut dyn tje_game::Context) -> tje_game::Result<Option<u32>> {
    let value = tje_game::core::wait_for_next_vblank_framed(context)?;
    let _orphan = context.enter_call_ceiling(ids::GET_RANDOM_NUMBER)?;
    ORPHAN_FRAME.with(|frame| frame.set(context.frame_state()));
    Ok(value)
}
static ORPHAN_WAIT: &[FramedTranslation] = &[FramedTranslation {
    id: ids::WAIT_FOR_NEXT_V_BLANK,
    function: orphan_after_real_wait,
}];
static ROOT_AND_ORPHAN: &[FramedTranslation] = &[
    FramedTranslation {
        id: ids::CLEAR_LOWER_VRAM_TILEMAP,
        function: timed_clear_lower::clear_lower_vram_tilemap_framed,
    },
    FramedTranslation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: orphan_after_real_wait,
    },
];

pub(super) static ROOT_ONLY: &[FramedTranslation] = &[FramedTranslation {
    id: ids::CLEAR_LOWER_VRAM_TILEMAP,
    function: timed_clear_lower::clear_lower_vram_tilemap_framed,
}];
static ROOT_AND_WAIT: &[FramedTranslation] = &[
    FramedTranslation {
        id: ids::CLEAR_LOWER_VRAM_TILEMAP,
        function: timed_clear_lower::clear_lower_vram_tilemap_framed,
    },
    FramedTranslation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: tje_game::core::wait_for_next_vblank_framed,
    },
];
static BOTH_SOURCES: &[&tje_game::timing::Program] = &[
    ClearLowerSource::definition(),
    tje_game::core::timed_wait::WaitSource::definition(),
];
static CALLER_SOURCE: &[&tje_game::timing::Program] = &[ClearLowerSource::definition()];
const STRICT: CaseConfig = CaseConfig {
    root: ids::CLEAR_LOWER_VRAM_TILEMAP,
    sources: BOTH_SOURCES,
    framed: ROOT_AND_WAIT,
    profile: CallProfile::NativeStrictReadable,
};
pub(super) const TRANSITIONAL: CaseConfig = CaseConfig {
    root: ids::CLEAR_LOWER_VRAM_TILEMAP,
    sources: CALLER_SOURCE,
    framed: ROOT_ONLY,
    profile: CallProfile::NativeCompiledOriginalCallees,
};

#[test]
#[ignore = "HELD mixed diagnostic: selected WAIT Rust is Replay-only; separate compiled-original control is replay_policy"]
fn both_revisions_nonleaf_strict_rust_and_explicit_compiled_original_callee_profiles() {
    for (revision, rom_key, session_key) in [
        (Revision::Rev02, "TJE_REV02_ROM", "TJE_TIMING_SESSION_REV02"),
        (Revision::Rev00, "TJE_REV00_ROM", "TJE_TIMING_SESSION_REV00"),
    ] {
        let cartridge = Rom::import(&fs::read(std::env::var(rom_key).unwrap()).unwrap()).unwrap();
        assert_eq!(cartridge.revision(), revision);
        let session =
            Session::from_json(&fs::read_to_string(std::env::var(session_key).unwrap()).unwrap())
                .unwrap();
        let timeline = Timeline::new(session.clone());
        let (entry, frame) = capture(&cartridge, &timeline, ids::CLEAR_LOWER_VRAM_TILEMAP, 0);
        let program =
            metadata::compile_program(cartridge.bytes(), &metadata::game_metadata(revision))
                .unwrap();
        let digest = sha1::Sha1::digest(cartridge.bytes()).into();
        let backend = NativeProgram::compile(digest, &program).unwrap();
        let (machine, _, _) = run_case(&cartridge, &entry, frame, &session, backend, &STRICT);
        let (_, _, backend) = machine.into_parts();
        let (machine, _, _) = run_case(&cartridge, &entry, frame, &session, backend, &TRANSITIONAL);
        let (_, _, backend) = machine.into_parts();
        let mut backend = orphan_source_parent(&cartridge, &entry, frame, &session, backend);
        // A selected Rust body without roles is NOT an original fallback in
        // either native profile, even though actual Host CHECK permits it.
        for profile in [
            CallProfile::NativeStrictReadable,
            CallProfile::NativeCompiledOriginalCallees,
        ] {
            let devices = DeviceRuntime::from_state(
                AddressSpace::new(cartridge.bytes().to_vec(), revision),
                entry.capture_native_state(),
            )
            .unwrap()
            .into();
            let mut machine = Engine::from_state(cpu(&entry), devices, backend, frame);
            let capability = CompiledTimingCapability::validate(&machine, &cartridge).unwrap();
            let binding = Arc::new(
                BoundProgram::compile(
                    &cartridge,
                    ids::CLEAR_LOWER_VRAM_TILEMAP,
                    ClearLowerSource::definition(),
                    7,
                    &[4, 6],
                )
                .unwrap(),
            );
            let mut state = NativeSourceState::new(binding, &[], profile).unwrap();
            let dispatcher = Dispatcher::hybrid_with_frames(&[], ROOT_AND_WAIT);
            let mut clock = CallClock::default();
            let _root = clock
                .enter(ids::CLEAR_LOWER_VRAM_TILEMAP, CallCeiling::DEFAULT)
                .unwrap();
            let before = machine.cpu().clone();
            let peripherals = machine.devices().capture_state(registers(machine.cpu()));
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
                Err(error) => assert!(error.to_string().contains("timing roles"), "{error}"),
                Ok(_) => panic!("selected untimed Rust body gained a native original fallback"),
            }
            assert_eq!(never.0.get(), 0);
            assert_eq!(machine.cpu(), &before);
            assert_peripheral(
                &machine.devices().capture_state(registers(machine.cpu())),
                &peripherals,
            );
            assert_eq!(clock.usage().ticks, 0);
            (_, _, backend) = machine.into_parts();
        }
    }
}

struct FailWindowAfterRealCall;
impl StepObserver for FailWindowAfterRealCall {
    fn completed_step(
        &mut self,
        owner: ProgressOwner,
        _step: DeviceStep,
        _cpu: &CpuState,
        _devices: &PlatformDevices,
    ) -> tje_game::Result<()> {
        assert_eq!(owner, ProgressOwner::SourceInstruction);
        Err(tje_game::Error::Platform(
            "seeded structural window callback failure".into(),
        ))
    }
}

struct RevisableLatch<'a> {
    inner: &'a Timeline,
    frame: std::cell::Cell<Option<u64>>,
}
impl FrameTimeline for RevisableLatch<'_> {
    fn latched_inputs(&self, frame: u64) -> tje_game::Result<GenesisInputs> {
        let latched = self.frame.get().ok_or_else(|| {
            tje_game::Error::Platform("missing current-frame latched-input".into())
        })?;
        if latched != frame {
            return Err(tje_game::Error::Platform(
                "stale current-frame latched-input".into(),
            ));
        }
        self.inner.latched_inputs(frame)
    }
    fn sample_inputs(&self, frame: u64) -> tje_game::Result<GenesisInputs> {
        let inputs = self.inner.sample_inputs(frame)?;
        self.frame.set(Some(frame));
        Ok(inputs)
    }
    fn before_frame(&self, frame: u64, memory: &mut dyn tje_mem::Bus) -> tje_game::Result<()> {
        self.inner.before_frame(frame, memory)
    }
    fn frame_completed(&self, frame: u64, devices: &PlatformDevices) -> tje_game::Result<()> {
        self.inner.frame_completed(frame, devices)
    }
}

fn capture_before_original_call(
    cartridge: &Rom,
    timeline: &Timeline,
) -> (GenesisEmulator, u64, tje_game::FunctionId) {
    let mut reference = GenesisEmulator::create(
        Some(cartridge.bytes().to_vec()),
        GenesisEmulatorConfig::default(),
        &mut Sink,
    );
    let target = ids::CLEAR_LOWER_VRAM_TILEMAP
        .metadata()
        .address(cartridge.revision());
    let mut frame = 0;
    let mut audio = Audio::default();
    for _ in 0..30_000_000 {
        let original = reference.hybrid_cpu();
        let bus = reference.hybrid_bus();
        let pc = original.pc();
        let opcode = original.hybrid_state_words()[19] as u16;
        if original.hybrid_instruction_ready(bus)
            && bus.cycles.m68k_wait_cpu_cycles == 0
            && opcode == 0x4eb9
            && cartridge
                .bytes()
                .get(pc as usize + 2..pc as usize + 6)
                .is_some_and(|bytes| u32::from_be_bytes(bytes.try_into().unwrap()) == target)
        {
            let caller = tje_game::functions()
                .find(|function| {
                    let start = function.address(cartridge.revision());
                    start <= pc
                        && pc < start + function.size(cartridge.revision())
                        && function.callees.contains(&ids::CLEAR_LOWER_VRAM_TILEMAP)
                })
                .unwrap()
                .id;
            return (reference, frame, caller);
        }
        let event = reference
            .tick(
                &mut Sink,
                &mut audio,
                &mut Inputs(timeline.sample(frame)),
                &mut Sink,
            )
            .unwrap();
        audio.0.clear();
        if event == TickEffect::FrameRendered {
            frame += 1;
        }
    }
    panic!("canonical real JSR to original ClearLowerVramTilemap was not reached");
}

#[test]
#[ignore = "HELD: selected timed WAIT grandchild is Replay-only; original-only native receipts remain distinct"]
fn both_revisions_original_callee_redispatches_its_selected_timed_grandchild() {
    use tje_engine_runtime::portable_timing::engine::{CallWindow, WindowFailureObservation};
    for (revision, rom_key, session_key) in [
        (Revision::Rev02, "TJE_REV02_ROM", "TJE_TIMING_SESSION_REV02"),
        (Revision::Rev00, "TJE_REV00_ROM", "TJE_TIMING_SESSION_REV00"),
    ] {
        let cartridge = Rom::import(&fs::read(std::env::var(rom_key).unwrap()).unwrap()).unwrap();
        assert_eq!(cartridge.revision(), revision);
        let session =
            Session::from_json(&fs::read_to_string(std::env::var(session_key).unwrap()).unwrap())
                .unwrap();
        let timeline = Timeline::new(session.clone());
        let (entry, frame, caller) = capture_before_original_call(&cartridge, &timeline);
        let program =
            metadata::compile_program(cartridge.bytes(), &metadata::game_metadata(revision))
                .unwrap();
        let backend =
            NativeProgram::compile(sha1::Sha1::digest(cartridge.bytes()).into(), &program).unwrap();
        let devices = DeviceRuntime::from_state(
            AddressSpace::new(cartridge.bytes().to_vec(), revision),
            entry.capture_native_state(),
        )
        .unwrap()
        .into();
        let mut machine = Engine::from_state(cpu(&entry), devices, backend, frame);
        let capability = CompiledTimingCapability::validate(&machine, &cartridge).unwrap();
        // The ACTUAL callsite-derived A→B closure must reject selected C before
        // the JSR if C has no timing roles. There is no caller-supplied seed and
        // no fake source descriptor merely to reach this startup check.
        let dispatcher = Dispatcher::hybrid_with_frames(&[], FRAMED);
        let mut clock = CallClock::default();
        let _root = clock.enter(caller, CallCeiling::DEFAULT).unwrap();
        let before = machine.cpu().clone();
        let peripherals = machine.devices().capture_state(registers(machine.cpu()));
        let never = NeverSample(std::cell::Cell::new(0));
        match CallWindow::new(
            &mut machine,
            &cartridge,
            &capability,
            &mut clock,
            &never,
            dispatcher,
            &[],
        ) {
            Err(error) => assert!(error.to_string().contains("timing roles"), "{error}"),
            Ok(_) => panic!("selected untimed grandchild gained a compiled-original fallback"),
        }
        assert_eq!(never.0.get(), 0);
        assert_eq!(machine.cpu(), &before);
        assert_peripheral(
            &machine.devices().capture_state(registers(machine.cpu())),
            &peripherals,
        );
        assert_eq!(clock.usage().ticks, 0);
        let (_, _, backend) = machine.into_parts();
        let backend = orphan_original_parent(&cartridge, &entry, frame, caller, &session, backend);
        // Failure after the genuine JSR retains its exact physical state and
        // first semantic cause. Dropping the affine window cannot renew the
        // still-active failed shared clock/owner as another window.
        let devices = DeviceRuntime::from_state(
            AddressSpace::new(cartridge.bytes().to_vec(), revision),
            entry.capture_native_state(),
        )
        .unwrap()
        .into();
        let mut machine = Engine::from_state(cpu(&entry), devices, backend, frame);
        let capability = CompiledTimingCapability::validate(&machine, &cartridge).unwrap();
        let mut clock = CallClock::default();
        let _root = clock.enter(caller, CallCeiling::DEFAULT).unwrap();
        let window = CallWindow::new(
            &mut machine,
            &cartridge,
            &capability,
            &mut clock,
            &timeline,
            dispatcher,
            &[tje_game::core::timed_wait::WaitSource::definition()],
        )
        .unwrap();
        let mut root_failure_observed = 0;
        let mut inspect_root = |observed: WindowFailureObservation<'_>| {
            root_failure_observed += 1;
            assert_eq!(
                observed.cause,
                &tje_game::Error::timing_execution_failure(
                    caller,
                    tje_game::Error::Platform("seeded structural window callback failure".into())
                )
            );
            assert_eq!(observed.frame.entry_sp, before.read_address(7));
            assert_eq!(observed.class, tje_game::execution_class(caller).unwrap());
            assert_eq!(observed.cpu.read_address(7), before.read_address(7) - 4);
            assert_eq!(
                observed.cpu.pc,
                ids::CLEAR_LOWER_VRAM_TILEMAP.metadata().address(revision)
            );
        };
        let failure = window
            .invoke(Some(&mut FailWindowAfterRealCall), Some(&mut inspect_root))
            .unwrap_err();
        assert_eq!(
            root_failure_observed, 1,
            "root-JSR failure was not protected before diagnostic observation"
        );
        assert!(failure.is_failed_execution());
        assert!(
            failure
                .to_string()
                .contains("seeded structural window callback failure")
        );
        let mut expected = fork(&cartridge, &entry);
        let mut audio = Audio::default();
        expected
            .tick(
                &mut Sink,
                &mut audio,
                &mut Inputs(timeline.sample(frame)),
                &mut Sink,
            )
            .unwrap();
        assert_eq!(
            words(machine.cpu()),
            expected.hybrid_cpu().hybrid_state_words()
        );
        assert_peripheral(
            &machine.devices().capture_state(registers(machine.cpu())),
            &expected.capture_native_state(),
        );
        assert!(
            machine
                .devices()
                .audio_samples()
                .iter()
                .map(|sample| sample.map(f64::to_bits))
                .eq(audio.0.iter().copied())
        );
        assert_eq!(
            machine.cpu().pc,
            ids::CLEAR_LOWER_VRAM_TILEMAP.metadata().address(revision)
        );
        assert_eq!(machine.cpu().read_address(7), before.read_address(7) - 4);
        assert_eq!(clock.usage().ticks, 1);
        assert!(
            clock.before_tick().is_err(),
            "failed window renewed its shared clock"
        );
        let failed_cpu = machine.cpu().clone();
        assert!(
            CallWindow::new(
                &mut machine,
                &cartridge,
                &capability,
                &mut clock,
                &never,
                dispatcher,
                &[tje_game::core::timed_wait::WaitSource::definition()]
            )
            .is_err()
        );
        assert_eq!(machine.cpu(), &failed_cpu);
        assert_eq!(never.0.get(), 0);
        let (_, _, backend) = machine.into_parts();

        let mut devices: PlatformDevices = DeviceRuntime::from_state(
            AddressSpace::new(cartridge.bytes().to_vec(), revision),
            entry.capture_native_state(),
        )
        .unwrap()
        .into();
        devices.enable_instruction_trace();
        devices.enable_ram_write_trace();
        devices.enable_trace();
        let mut machine = Engine::from_state(cpu(&entry), devices, backend, frame);
        let capability = CompiledTimingCapability::validate(&machine, &cartridge).unwrap();
        let oracle = Rc::new(RefCell::new(Oracle {
            reference: fork(&cartridge, &entry),
            audio: Audio::default(),
            frame,
            timeline: Rc::new(Timeline::new(session.clone())),
            accesses: 0,
            commits: 0,
            source: 0,
            compiled: 0,
            compiled_original_callee: 0,
            steps: 0,
        }));
        let mut timeline = Timeline::new(session);
        timeline.oracle = Some(Rc::clone(&oracle));
        let mut observer = Observe(Rc::clone(&oracle));
        let mut clock = CallClock::default();
        let root = clock.enter(caller, CallCeiling::DEFAULT).unwrap();
        let latch = RevisableLatch {
            inner: &timeline,
            frame: std::cell::Cell::new(None),
        };
        let entry_cpu = machine.cpu().clone();
        let entry_devices = machine.devices().capture_state(registers(machine.cpu()));
        let entry_clock = clock.usage();
        for unavailable in [None, Some(frame + 1)] {
            latch.frame.set(unavailable);
            let window = CallWindow::new(
                &mut machine,
                &cartridge,
                &capability,
                &mut clock,
                &latch,
                dispatcher,
                &[tje_game::core::timed_wait::WaitSource::definition()],
            )
            .unwrap();
            let failed_taps = std::cell::Cell::new(0);
            let mut unexpected =
                |_: WindowFailureObservation<'_>| failed_taps.set(failed_taps.get() + 1);
            let cause = window.invoke(None, Some(&mut unexpected)).unwrap_err();
            assert_eq!(
                failed_taps.get(),
                0,
                "ordinary pre-admission error ran the fatal diagnostic tap"
            );
            assert!(
                !cause.is_failed_execution(),
                "pre-admission latch error became fatal"
            );
            assert!(cause.to_string().contains("latched-input"));
            assert_eq!(machine.cpu(), &entry_cpu);
            assert_eq!(machine.frame_count(), frame);
            assert_peripheral(
                &machine.devices().capture_state(registers(machine.cpu())),
                &entry_devices,
            );
            assert_eq!(clock.usage(), entry_clock);
            assert!(clock.contains_scope(caller));
            assert!(
                clock.before_tick().is_ok(),
                "pre-admission error poisoned its unchanged scope"
            );
            assert_eq!(oracle.borrow().steps, 0);
        }
        // Correct only the caller-owned input latch, not any CPU/device/frame
        // state, then retry the same real callsite and shared owner successfully.
        latch.frame.set(Some(frame));
        let window = CallWindow::new(
            &mut machine,
            &cartridge,
            &capability,
            &mut clock,
            &latch,
            dispatcher,
            &[tje_game::core::timed_wait::WaitSource::definition()],
        )
        .unwrap();
        let receipt = window.invoke(Some(&mut observer), None).unwrap();
        clock.leave(root).unwrap();
        assert_eq!(
            receipt.scope,
            "structural call window (not whole-root translation)"
        );
        assert_eq!(receipt.caller, caller);
        assert_eq!(receipt.callee, ids::CLEAR_LOWER_VRAM_TILEMAP);
        assert_eq!(
            receipt.profile,
            CallProfile::NativeCompiledOriginalCallees.label()
        );
        assert!(receipt.separate_callee_proof_required);
        // Deleting transitive interception can remain semantically exact, but
        // then C executes original and this source-count assertion MUST fail.
        assert!(
            receipt.source_instructions > 1,
            "only the window JSR ran as Rust; selected C was bypassed"
        );
        assert!(receipt.compiled_original_callee_instructions > 0);
        assert_eq!(clock.usage().ticks, receipt.scheduler_steps);
        assert_eq!(clock.usage().master_clocks, receipt.master_clocks);
        let oracle = oracle.borrow();
        assert_eq!(receipt.source_instructions, oracle.source);
        assert_eq!(
            receipt.compiled_original_callee_instructions,
            oracle.compiled_original_callee
        );
        assert_eq!(receipt.compiled_isr_instructions, oracle.compiled);
        assert_eq!(receipt.scheduler_steps, oracle.steps);
        assert_eq!(machine.cpu().pc, receipt.return_pc);
        assert_eq!(machine.cpu().read_address(7), cpu(&entry).read_address(7));
        assert_eq!(
            words(machine.cpu()),
            oracle.reference.hybrid_cpu().hybrid_state_words()
        );
        assert_peripheral(
            &machine.devices().capture_state(registers(machine.cpu())),
            &oracle.reference.capture_native_state(),
        );
        assert!(
            machine
                .devices()
                .audio_samples()
                .iter()
                .map(|sample| sample.map(f64::to_bits))
                .eq(oracle.audio.0.iter().copied())
        );
        assert_eq!(
            machine.devices().trace().unwrap().writes,
            *oracle
                .reference
                .hybrid_bus()
                .native_writes
                .as_ref()
                .unwrap()
        );
        eprintln!(
            "{revision:?}: STRUCTURAL ONE-CALL window caller={} original B={} selected timed C=WaitForNextVBlank; source={}; compiled-original-callee={}; compiled-ISR={}; steps={}; full25CPU/bus/RAM/devices/audio exact; NOT whole-root translation",
            caller.metadata().name,
            receipt.callee.metadata().name,
            receipt.source_instructions,
            receipt.compiled_original_callee_instructions,
            receipt.compiled_isr_instructions,
            receipt.scheduler_steps
        );
    }
}

fn failure_pair(
    cartridge: &Rom,
    entry: &GenesisEmulator,
    frame: u64,
    session: &Session,
    backend: NativeProgram,
) -> (Engine<NativeProgram>, Rc<RefCell<Oracle>>, Timeline) {
    let mut devices: PlatformDevices = DeviceRuntime::from_state(
        AddressSpace::new(cartridge.bytes().to_vec(), cartridge.revision()),
        entry.capture_native_state(),
    )
    .unwrap()
    .into();
    devices.enable_instruction_trace();
    devices.enable_ram_write_trace();
    devices.enable_trace();
    let machine = Engine::from_state(cpu(entry), devices, backend, frame);
    let oracle = Rc::new(RefCell::new(Oracle {
        reference: fork(cartridge, entry),
        audio: Audio::default(),
        frame,
        timeline: Rc::new(Timeline::new(session.clone())),
        accesses: 0,
        commits: 0,
        source: 0,
        compiled: 0,
        compiled_original_callee: 0,
        steps: 0,
    }));
    let mut timeline = Timeline::new(session.clone());
    timeline.oracle = Some(Rc::clone(&oracle));
    (machine, oracle, timeline)
}
fn assert_failed_pair(machine: &Engine<NativeProgram>, oracle: &Oracle) {
    assert_eq!(
        words(machine.cpu()),
        oracle.reference.hybrid_cpu().hybrid_state_words()
    );
    assert_peripheral(
        &machine.devices().capture_state(registers(machine.cpu())),
        &oracle.reference.capture_native_state(),
    );
    assert!(
        machine
            .devices()
            .audio_samples()
            .iter()
            .map(|sample| sample.map(f64::to_bits))
            .eq(oracle.audio.0.iter().copied())
    );
    assert_eq!(
        machine.devices().trace().unwrap().writes,
        *oracle
            .reference
            .hybrid_bus()
            .native_writes
            .as_ref()
            .unwrap()
    );
}
fn orphan_cause() -> tje_game::Error {
    tje_game::Error::timing_execution_failure(
        ids::WAIT_FOR_NEXT_V_BLANK,
        tje_game::Error::Clock(tje_game::execution::CallClockError::InvalidScope),
    )
}
fn orphan_source_parent(
    cartridge: &Rom,
    entry: &GenesisEmulator,
    frame: u64,
    session: &Session,
    backend: NativeProgram,
) -> NativeProgram {
    let (mut machine, oracle, timeline) = failure_pair(cartridge, entry, frame, session, backend);
    let capability = CompiledTimingCapability::validate(&machine, cartridge).unwrap();
    let root_sp = machine.cpu().read_address(7);
    let root = Arc::new(
        BoundProgram::compile(
            cartridge,
            ids::CLEAR_LOWER_VRAM_TILEMAP,
            ClearLowerSource::definition(),
            7,
            &[4, 6],
        )
        .unwrap(),
    );
    let wait = Arc::new(
        BoundProgram::compile(
            cartridge,
            ids::WAIT_FOR_NEXT_V_BLANK,
            tje_game::core::timed_wait::WaitSource::definition(),
            7,
            &[4, 6],
        )
        .unwrap(),
    );
    let mut state =
        NativeSourceState::new(root, &[wait], CallProfile::NativeStrictReadable).unwrap();
    let mut observer = Observe(Rc::clone(&oracle));
    let mut clock = CallClock::default();
    let _root = clock
        .enter(ids::CLEAR_LOWER_VRAM_TILEMAP, CallCeiling::DEFAULT)
        .unwrap();
    ORPHAN_FRAME.with(|frame| frame.set(None));
    {
        let mut context = NativeTimingContext::new(
            &mut machine,
            cartridge,
            &capability,
            &mut clock,
            &timeline,
            Dispatcher::hybrid_with_frames(&[], ROOT_AND_ORPHAN),
            &mut state,
        )
        .unwrap()
        .with_observer(&mut observer);
        let cause = context
            .invoke_prepared(ids::CLEAR_LOWER_VRAM_TILEMAP)
            .unwrap_err();
        assert_eq!(
            cause,
            orphan_cause(),
            "child cleanup retains its first typed fatal cause"
        );
        let child = ORPHAN_FRAME.with(std::cell::Cell::get).unwrap();
        assert!(child.is_prepared_for(ids::WAIT_FOR_NEXT_V_BLANK));
        assert_eq!(child.entry_sp, root_sp - 12);
        assert_eq!(
            context.frame_state(),
            Some(child),
            "failed finalization restored its source parent"
        );
        assert_eq!(
            context.execution_class(),
            tje_game::execution::ExecutionClass::TimeAware
        );
        assert_eq!(
            context.machine().cpu().read_address(7),
            root_sp - 8,
            "caller argument cleanup ran after child finalization failed"
        );
        let before = context.machine().cpu().clone();
        assert!(
            context
                .set_frame_state(tje_game::call_frame::FrameState::new(root_sp))
                .is_err()
        );
        assert_eq!(context.frame_state(), Some(child));
        assert_eq!(context.machine().cpu(), &before);
    }
    assert!(clock.before_tick().is_err());
    assert_failed_pair(&machine, &oracle.borrow());
    eprintln!(
        "{:?}: nested source-parent post-real-RTS orphan ceiling rejects; child metadata/first fatal/actual CPU/devices/audio retained; no parent argument cleanup",
        cartridge.revision()
    );
    let (_, _, backend) = machine.into_parts();
    backend
}
fn orphan_original_parent(
    cartridge: &Rom,
    entry: &GenesisEmulator,
    frame: u64,
    caller: tje_game::FunctionId,
    session: &Session,
    backend: NativeProgram,
) -> NativeProgram {
    use tje_engine_runtime::portable_timing::engine::{CallWindow, WindowFailureObservation};
    let (mut machine, oracle, timeline) = failure_pair(cartridge, entry, frame, session, backend);
    let capability = CompiledTimingCapability::validate(&machine, cartridge).unwrap();
    let root_sp = machine.cpu().read_address(7);
    let mut observer = Observe(Rc::clone(&oracle));
    let mut clock = CallClock::default();
    let _root = clock.enter(caller, CallCeiling::DEFAULT).unwrap();
    ORPHAN_FRAME.with(|frame| frame.set(None));
    let mut inspected = 0;
    let mut inspect = |failure: WindowFailureObservation<'_>| {
        inspected += 1;
        assert_eq!(failure.cause, &orphan_cause());
        let child = ORPHAN_FRAME.with(std::cell::Cell::get).unwrap();
        assert_eq!(
            failure.frame, child,
            "failed finalization restored its original parent"
        );
        assert!(child.is_prepared_for(ids::WAIT_FOR_NEXT_V_BLANK));
        assert_eq!(child.entry_sp, root_sp - 16);
        assert_eq!(
            failure.class,
            tje_game::execution::ExecutionClass::TimeAware
        );
        assert_eq!(failure.cpu.read_address(7), root_sp - 12);
        let expected = oracle.borrow();
        assert_eq!(
            words(failure.cpu),
            expected.reference.hybrid_cpu().hybrid_state_words()
        );
        assert_peripheral(
            &failure.devices.capture_state(registers(failure.cpu)),
            &expected.reference.capture_native_state(),
        );
    };
    let window = CallWindow::new(
        &mut machine,
        cartridge,
        &capability,
        &mut clock,
        &timeline,
        Dispatcher::hybrid_with_frames(&[], ORPHAN_WAIT),
        &[tje_game::core::timed_wait::WaitSource::definition()],
    )
    .unwrap();
    let cause = window
        .invoke(Some(&mut observer), Some(&mut inspect))
        .unwrap_err();
    assert_eq!(cause, orphan_cause());
    assert_eq!(
        inspected, 1,
        "diagnostic structural-window failure tap is single and observational"
    );
    assert_eq!(machine.cpu().read_address(7), root_sp - 12);
    assert!(clock.before_tick().is_err());
    assert_failed_pair(&machine, &oracle.borrow());
    eprintln!(
        "{:?}: diagnostic STRUCTURAL window original-parent post-real-RTS orphan ceiling rejects; timed-C metadata/first fatal/actual state retained; no whole-root receipt or parent cleanup",
        cartridge.revision()
    );
    let (_, _, backend) = machine.into_parts();
    backend
}
