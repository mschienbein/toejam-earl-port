use super::*;
use tje_game::cutscenes::timed_clear_lower::{self, ClearLowerSource};

static NONLEAF_FRAMED: &[FramedTranslation] = &[
    FramedTranslation {
        id: ids::CLEAR_LOWER_VRAM_TILEMAP,
        function: timed_clear_lower::clear_lower_vram_tilemap_framed,
    },
    FramedTranslation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: tje_game::core::wait_for_next_vblank_framed,
    },
];
static TIMED_CALLEE: &[&game::Program] = &[
    ClearLowerSource::definition(),
    tje_game::core::timed_wait::WaitSource::definition(),
];
static SUBSTITUTED_CALLEE: &[&game::Program] = &[ClearLowerSource::definition()];
fn nonleaf_registry(revision: tje_rom::Revision, timed: bool) -> HookRegistry {
    let registry = HookRegistry::proof_clocked_strict(
        revision,
        &[],
        NONLEAF_FRAMED,
        &[ids::CLEAR_LOWER_VRAM_TILEMAP],
    )
    .unwrap();
    assert!(
        registry
            .at(ids::WAIT_FOR_NEXT_V_BLANK.metadata().address(revision))
            .is_none()
    );
    registry
        .with_timing_sources(if timed {
            TIMED_CALLEE
        } else {
            SUBSTITUTED_CALLEE
        })
        .unwrap()
}
fn canonical_inputs(revision: tje_rom::Revision) -> Arc<dyn WaitInputPoller> {
    let key = match revision {
        tje_rom::Revision::Rev02 => "TJE_TIMING_SESSION_REV02",
        tje_rom::Revision::Rev00 => "TJE_TIMING_SESSION_REV00",
    };
    let session =
        tje_session::Session::from_json(&fs::read_to_string(std::env::var(key).unwrap()).unwrap())
            .unwrap();
    Arc::new(controller_script(&session))
}
fn capture_nonleaf(
    rom: &[u8],
    revision: tje_rom::Revision,
    registry: HookRegistry,
    inputs: Arc<dyn WaitInputPoller>,
) -> Host {
    let mut host = Host::new(rom, Mode::Reference, registry).unwrap();
    let mut clock = WaitClock {
        inputs: inputs.at(0).unwrap(),
        poller: Some(inputs),
        ..WaitClock::default()
    };
    let mut audio = HeadlessAudio::default();
    for _ in 0..30_000_000 {
        let cpu = host.emulator.hybrid_cpu();
        let bus = host.emulator.hybrid_bus();
        if cpu.pc() == ids::CLEAR_LOWER_VRAM_TILEMAP.metadata().address(revision)
            && cpu.hybrid_instruction_ready(bus)
            && bus.cycles.m68k_wait_cpu_cycles == 0
        {
            host.frame = clock.frame;
            host.wait_clock = clock;
            host.mode = Mode::Check;
            return host;
        }
        let event = host
            .emulator
            .tick(
                &mut HeadlessOutput,
                &mut audio,
                &mut Inputs(clock.inputs),
                &mut NoCartridgeSaves,
            )
            .unwrap();
        audio.samples.clear();
        if event == TickEffect::FrameRendered {
            clock.advance_frame(&mut host.emulator).unwrap();
        }
    }
    panic!("canonical nonleaf ClearLowerVramTilemap entry not observed");
}

#[test]
#[ignore = "requires both private ROMs and committed controller sessions; full canonical CALL path"]
fn both_revisions_canonical_nonleaf_calls_timed_rust_and_original_substituted_wait() {
    let _proof = proof::Enable::new();
    for (revision, rom_key, session_key) in [
        (
            tje_rom::Revision::Rev02,
            "TJE_REV02_ROM",
            "TJE_TIMING_SESSION_REV02",
        ),
        (
            tje_rom::Revision::Rev00,
            "TJE_REV00_ROM",
            "TJE_TIMING_SESSION_REV00",
        ),
    ] {
        let rom = fs::read(std::env::var(rom_key).unwrap()).unwrap();
        let session = tje_session::Session::from_json(
            &fs::read_to_string(std::env::var(session_key).unwrap()).unwrap(),
        )
        .unwrap();
        for timed in [true, false] {
            let mut host = capture_nonleaf(
                &rom,
                revision,
                nonleaf_registry(revision, timed),
                Arc::new(controller_script(&session)),
            );
            assert_eq!(host.mode, Mode::Check);
            let before =
                bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap();
            let live_authority = format!(
                "{:?}",
                host.timing_services.as_ref().unwrap().authority.completion
            );
            check(&mut host).unwrap();
            let receipt = proof::receipts().pop().unwrap();
            let usage = host
                .last_timing_source_usage(ids::CLEAR_LOWER_VRAM_TILEMAP)
                .unwrap();
            assert_eq!(usage.call_profile, Some("HostCheckOriginalSubstitution"));
            assert_eq!(
                usage.instructions + usage.nested_source_instructions,
                receipt.source
            );
            if timed {
                assert_eq!(usage.timed_callee_calls, 1);
                assert!(usage.nested_source_instructions > 0);
                assert_eq!(usage.original_substitution_calls, 0);
                assert_eq!(usage.original_callee_instructions, 0);
                assert_eq!(usage.separate_callee_proof_obligations, 0);
            } else {
                assert_eq!(usage.original_substitution_calls, 1);
                assert!(usage.original_callee_instructions > 0);
                assert_eq!(usage.timed_callee_calls, 0);
                assert_eq!(usage.nested_source_instructions, 0);
                assert_eq!(
                    usage.original_substitution_label,
                    Some("original-callee substitution (callee proven separately)")
                );
                assert_eq!(usage.separate_callee_proof_obligations, 1);
            }
            assert_eq!(
                before,
                bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap()
            );
            assert_eq!(
                live_authority,
                format!(
                    "{:?}",
                    host.timing_services.as_ref().unwrap().authority.completion
                )
            );
            assert!(
                host.registry
                    .at(ids::WAIT_FOR_NEXT_V_BLANK.metadata().address(revision))
                    .is_none(),
                "executing a callee is not an independent hook proof or activation"
            );
            eprintln!(
                "{revision:?}: canonical ClearLower timed_callee={timed}, own_source={}, nested_Rust={}, original_callee={}, original_ISR={}, steps={}, obligations={}; full per-step CPU/bus/RAM and complete devices/audio exact",
                usage.instructions,
                usage.nested_source_instructions,
                usage.original_callee_instructions,
                usage.original_isr_instructions,
                usage.scheduler_steps,
                usage.separate_callee_proof_obligations
            );
            proof::mutate_next(2);
            let error = check(&mut host).unwrap_err();
            assert!(
                error.to_string().contains("wrong-cycle mutant rejected"),
                "{error}"
            );
            assert!(proof::receipts().is_empty());
            check(&mut host).unwrap();
            assert_eq!(proof::receipts().len(), 1);
            assert_eq!(
                live_authority,
                format!(
                    "{:?}",
                    host.timing_services.as_ref().unwrap().authority.completion
                )
            );
        }
    }
}

#[test]
#[ignore = "requires private ROMs; explicit actual host mode, not proof-envelope or audit inference"]
fn missing_callee_roles_do_not_gain_substitution_in_reference_or_replace() {
    for (revision, key) in [
        (tje_rom::Revision::Rev02, "TJE_REV02_ROM"),
        (tje_rom::Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let rom = fs::read(std::env::var(key).unwrap()).unwrap();
        let cartridge = Rom::import(&rom).unwrap();
        let registry = nonleaf_registry(revision, false);
        assert!(
            TimingServices::prepare(
                &cartridge,
                SUBSTITUTED_CALLEE,
                7,
                registry.dispatcher(),
                Mode::Replace
            )
            .is_err()
        );
        let mut host = capture_nonleaf(&rom, revision, registry, canonical_inputs(revision));
        host.mode = Mode::Reference;
        let hook = host.registry.at(host.cpu().pc()).unwrap().clone();
        let before = bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap();
        let clock = host.wait_clock.clone();
        let mut context = host
            .context()
            .with_execution(ExecutionClass::TimeAware, clock);
        let error = hook.execute(&mut context).unwrap_err();
        assert!(error.to_string().contains("timing roles"), "{error}");
        drop(context);
        assert_eq!(
            before,
            bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap()
        );
    }
}

struct AuthoredCallerInstruction {
    entry: CpuState,
    next: u32,
    stack: u32,
    status: u16,
    writes: Vec<u16>,
    issued: usize,
}
impl RustInstruction for AuthoredCallerInstruction {
    fn entry_state(&self) -> &CpuState {
        &self.entry
    }
    fn effect(
        &mut self,
        emulator: &mut GenesisEmulator,
        expected: ExpectedAccess,
    ) -> Result<BusObservation> {
        let value = if expected.kind == tje_engine_core::timing_region::AccessKind::Write {
            let value = *self.writes.get(self.issued).ok_or_else(|| {
                HostError::Metadata("authored caller has an unexpected write".into())
            })?;
            self.issued += 1;
            Some(u32::from(value))
        } else {
            None
        };
        rust_bus_effect(emulator, expected, value)
    }
    fn finish(&mut self, emulator: &mut GenesisEmulator) -> Result<RustPathChoice> {
        assert_eq!(self.issued, self.writes.len());
        let cpu = emulator.hybrid_parts().0;
        crate::abi::set_sp(cpu, self.stack);
        cpu.set_status_register(self.status);
        Ok(RustPathChoice { next_pc: self.next })
    }
}

#[test]
fn original_child_pointer_read_retains_chronological_root_local_initialization() {
    use crate::abi::{AtomicRun, HookContext, StackAuditState};
    use tje_engine_runtime::{
        portable_timing::CompletionOwners,
        timing_region::{
            CalleeKind, RegionCall, RegionEdge, RegionStorage, ReviewedRegion, TimingProgram,
        },
    };
    const MAIN: u32 = 0x200;
    const CHILD: u32 = 0x300;
    const STACK: u32 = 0xff7f00;
    for initialized in [false, true] {
        let mut rom = vec![0_u8; 0x100000];
        rom[..4].copy_from_slice(&STACK.to_be_bytes());
        rom[4..8].copy_from_slice(&MAIN.to_be_bytes());
        let mut words = vec![0x598f_u16]; // SUBQ.L #4,A7: reserve the caller local.
        if initialized {
            words.extend([0x2ebc, 0x1234, 0x5678]);
        } // MOVE.L #value,(A7).
        let push_pc = MAIN + 2 * words.len() as u32;
        words.push(0x4857); // PEA (A7): pass the REAL local address, no preload.
        let call_pc = MAIN + 2 * words.len() as u32;
        words.extend([0x4eb9, 0, CHILD as u16, 0x4e71, 0x4e71]);
        let resume = call_pc + 6;
        for (index, word) in words.into_iter().enumerate() {
            let at = MAIN as usize + 2 * index;
            rom[at..at + 2].copy_from_slice(&word.to_be_bytes());
        }
        // Authored original child: MOVEA.L 4(A7),A0; MOVE.B (A0),D0; RTS.
        for (index, word) in [0x206f_u16, 4, 0x1010, 0x4e75].into_iter().enumerate() {
            let at = CHILD as usize + 2 * index;
            rom[at..at + 2].copy_from_slice(&word.to_be_bytes());
        }
        let mut pcs = vec![MAIN];
        let mut edges = Vec::new();
        if initialized {
            pcs.push(MAIN + 2);
            edges.push(RegionEdge {
                from: MAIN,
                to: MAIN + 2,
            });
            edges.push(RegionEdge {
                from: MAIN + 2,
                to: push_pc,
            });
        } else {
            edges.push(RegionEdge {
                from: MAIN,
                to: push_pc,
            });
        }
        pcs.extend([push_pc, call_pc, resume]);
        edges.extend([
            RegionEdge {
                from: push_pc,
                to: call_pc,
            },
            RegionEdge {
                from: resume,
                to: resume + 2,
            },
        ]);
        let program = TimingProgram::compile(
            &rom,
            ReviewedRegion {
                name: "authored caller-local scope",
                source: "synthetic allocate/store/PEA/JSR; original pointer reader",
                entry: MAIN,
                instruction_pcs: pcs,
                edges,
                calls: vec![RegionCall {
                    from: call_pc,
                    target: CHILD,
                    resume,
                }],
                return_pc: Some(resume + 2),
                maximum_steps: 1000,
                required_interrupts: &[],
            },
            7,
        )
        .unwrap();
        let mut emulator = GenesisEmulator::create(
            Some(rom),
            genesis_config::GenesisEmulatorConfig::default(),
            &mut NoCartridgeSaves,
        );
        emulator.hybrid_bus_mut().hybrid_cpu_accesses = Some(Vec::new());
        emulator.hybrid_bus_mut().hybrid_writes = Some(Vec::new());
        let mut clock = WaitClock::default();
        let _root = clock
            .call_clock
            .enter(
                ids::CLEAR_LOWER_VRAM_TILEMAP,
                tje_game::execution::CallCeiling::DEFAULT,
            )
            .unwrap();
        let initial = state(&emulator);
        let mut storage = RegionStorage::new(&program);
        let mut region = TimingRegion::begin_oracle(
            &program,
            &initial,
            program.fixed_target().unwrap(),
            &mut storage,
        )
        .unwrap();
        let mut owners = CompletionOwners::default();
        owners.begin_root_scope().unwrap();
        let parent = owners
            .begin(ids::CLEAR_LOWER_VRAM_TILEMAP, STACK, &region)
            .unwrap();
        let mut prefix = ClockRun::default();
        {
            let mut ledger = SourceLedger {
                run: &mut prefix,
                observers: PhysicalObservers::default(),
                base_depth: 0,
                before: None,
                auditing: true,
                frame: 0,
                parents: &mut [],
                proof: SourceProof::default(),
                original_segment: None,
            };
            let mut host =
                TimingHost::with_audio(&mut emulator, &mut clock, &mut ledger, Vec::new()).unwrap();
            while host.state().pc != CHILD {
                let entry = host.state();
                let (next, stack, status, writes) = if entry.pc == MAIN {
                    (
                        if initialized { MAIN + 2 } else { push_pc },
                        STACK - 4,
                        entry.sr as u16,
                        Vec::new(),
                    )
                } else if initialized && entry.pc == MAIN + 2 {
                    (
                        push_pc,
                        STACK - 4,
                        (entry.sr as u16) & !0xf,
                        vec![0x1234, 0x5678],
                    )
                } else if entry.pc == push_pc {
                    (
                        call_pc,
                        STACK - 8,
                        entry.sr as u16,
                        vec![((STACK - 4) >> 16) as u16, (STACK - 4) as u16],
                    )
                } else {
                    assert_eq!(entry.pc, call_pc);
                    (
                        CHILD,
                        STACK - 12,
                        entry.sr as u16,
                        vec![(resume >> 16) as u16, resume as u16],
                    )
                };
                host.step_source(
                    &mut region,
                    &mut AuthoredCallerInstruction {
                        entry,
                        next,
                        stack,
                        status,
                        writes,
                        issued: 0,
                    },
                )
                .unwrap();
            }
            ledger.run.audio = host.into_audio();
        }
        let actual = state(&emulator);
        let token = region.authorize_call(&actual).unwrap();
        let _parked = region.park().unwrap();
        // Root ownership is set BEFORE the child; metadata frame changes must
        // not turn caller locals into untracked incoming bytes of a fresh root.
        let mut context = HookContext::new(
            &mut emulator,
            tje_rom::Revision::Rev02,
            tje_game::Dispatcher::hybrid_with(&[]),
            Mode::Check,
        )
        .with_execution(ExecutionClass::TimeAware, clock);
        context.enable_stack_audit(StackAuditState::new(STACK, true), AtomicRun::default());
        context.merge_clock_run(prefix);
        let stack = context.stack_audit.take().unwrap();
        let mut clock = std::mem::take(&mut context.wait_clock);
        let audio = std::mem::take(&mut context.time_audio);
        let audit = std::mem::take(context.audit.get_mut());
        drop(context);
        owners
            .enter_child(
                &parent,
                &token,
                ids::ABS,
                tje_engine_runtime::portable_timing::CallProfile::HostCheckOriginalSubstitution,
                CalleeKind::ReferenceOriginalSubstitution,
            )
            .unwrap();
        let mut controller = owners.begin_original(&token, &state(&emulator)).unwrap();
        let (result, run) = run_original(
            ExecutionScope {
                emulator: &mut emulator,
                frame: FrameState::new(STACK - 12),
                clock: &mut clock,
                audio,
                audit: true,
                observers: stack.physical_observers(),
            },
            &mut [],
            &mut controller,
        );
        result.unwrap();
        let mut context = HookContext::new(
            &mut emulator,
            tje_rom::Revision::Rev02,
            tje_game::Dispatcher::hybrid_with(&[]),
            Mode::Check,
        )
        .with_execution(ExecutionClass::TimeAware, clock);
        context.enable_stack_audit(stack, audit);
        context.merge_clock_run(run);
        let uninitialized = context
            .audit
            .borrow()
            .uninitialized_reads
            .iter()
            .any(|read| read.address == STACK - 4);
        assert_eq!(
            uninitialized, !initialized,
            "the original child's pointer read must use the chronological CALLER root bitmap"
        );
        assert_eq!(
            context.cpu().stack_pointer(),
            STACK - 8,
            "caller argument cleanup has not executed"
        );
    }
}

thread_local! {
    static NESTED_FAILURE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}
fn nested_cause(kind: u8) -> tje_game::Error {
    let entry = ids::WAIT_FOR_NEXT_V_BLANK;
    match kind {
        0 => tje_game::Error::Platform("authored nested begin failure".into()),
        1 => tje_game::Error::timing_execution_failure(
            entry,
            tje_game::Error::Platform("authored preboxed nested failure".into()),
        ),
        2 => tje_game::Error::ControlTransfer(tje_game::exits::ControlTransfer {
            entry,
            exit: tje_game::exits::Exit::Terminal,
        }),
        _ => tje_game::Error::Clock(tje_game::execution::CallClockError::InvalidScope),
    }
}
struct FailingNestedSource(tje_game::core::timed_wait::WaitSource);
impl game::TimingSource for FailingNestedSource {
    fn program(&self) -> &'static game::Program {
        self.0.program()
    }
    fn begin_instruction(
        &mut self,
        site: game::InstructionSite,
        registers: game::Registers,
    ) -> tje_game::Result<()> {
        self.0.begin_instruction(site, registers)?;
        Err(nested_cause(NESTED_FAILURE.with(std::cell::Cell::get)))
    }
    fn next_effect(&mut self) -> tje_game::Result<Option<game::Effect>> {
        self.0.next_effect()
    }
    fn acknowledge(&mut self, actual: game::Observation) -> tje_game::Result<()> {
        self.0.acknowledge(actual)
    }
    fn finish_instruction(&mut self) -> tje_game::Result<game::Commit> {
        self.0.finish_instruction()
    }
}
fn nested_failure_body(context: &mut dyn tje_game::Context) -> tje_game::Result<Option<u32>> {
    if NESTED_FAILURE.with(std::cell::Cell::get) == 3 {
        let value = tje_game::core::wait_for_next_vblank_framed(context)?;
        let _orphan = context.enter_call_ceiling(ids::GET_RANDOM_NUMBER)?;
        return Ok(value);
    }
    let mut source = FailingNestedSource(tje_game::core::timed_wait::WaitSource::new(
        context.revision(),
    ));
    Ok(context
        .execute_timing_program(ids::WAIT_FOR_NEXT_V_BLANK, &mut source)?
        .value)
}

#[test]
#[ignore = "private ROMs; genuine suspended source-parent fatal boundary"]
fn nested_source_callback_failure_preboxed_and_transfer_keep_first_failed_chain_state() {
    use tje_game::Context;
    static FRAMED: &[FramedTranslation] = &[
        FramedTranslation {
            id: ids::CLEAR_LOWER_VRAM_TILEMAP,
            function: timed_clear_lower::clear_lower_vram_tilemap_framed,
        },
        FramedTranslation {
            id: ids::WAIT_FOR_NEXT_V_BLANK,
            function: nested_failure_body,
        },
    ];
    let _proof = proof::Enable::new();
    for (revision, key) in [
        (tje_rom::Revision::Rev02, "TJE_REV02_ROM"),
        (tje_rom::Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let rom = fs::read(std::env::var(key).unwrap()).unwrap();
        for kind in 0..4 {
            NESTED_FAILURE.with(|value| value.set(kind));
            let registry = HookRegistry::proof_clocked_strict(
                revision,
                &[],
                FRAMED,
                &[ids::CLEAR_LOWER_VRAM_TILEMAP],
            )
            .unwrap()
            .with_timing_sources(TIMED_CALLEE)
            .unwrap();
            let mut host = capture_nonleaf(&rom, revision, registry, canonical_inputs(revision));
            let root_sp = host.cpu().stack_pointer();
            let hook = host.registry.at(host.cpu().pc()).unwrap().clone();
            let clock = host.wait_clock.clone();
            let frame;
            let input_frame;
            {
                let mut context = host
                    .context()
                    .with_execution(ExecutionClass::TimeAware, clock);
                let error = hook.execute(&mut context).unwrap_err();
                let HostError::Translation(failure) = error else {
                    panic!("nested source lost its typed failure");
                };
                let expected = nested_cause(kind);
                if expected.is_failed_execution() {
                    assert_eq!(failure, expected, "preboxed fatal cause is not rewrapped");
                } else {
                    assert_eq!(
                        failure,
                        tje_game::Error::timing_execution_failure(
                            ids::WAIT_FOR_NEXT_V_BLANK,
                            expected
                        )
                    );
                }
                assert!(failure.is_failed_execution() && failure.preserves_call_frame());
                frame = context.frame_state().unwrap();
                input_frame = context.wait_clock.frame;
                assert!(frame.is_prepared_for(ids::WAIT_FOR_NEXT_V_BLANK));
                assert_eq!(frame.entry_sp, root_sp - 12);
                assert_eq!(
                    context.cpu().stack_pointer(),
                    root_sp - if kind == 3 { 8 } else { 12 }
                );
                let before = context.cpu().hybrid_state_words();
                assert!(context.set_frame_state(FrameState::new(root_sp)).is_err());
                assert_eq!(context.frame_state(), Some(frame));
                assert!(
                    context
                        .consume_timing_completion(game::CompletionRequest {
                            entry: ids::WAIT_FOR_NEXT_V_BLANK,
                            entry_sp: root_sp - 12,
                            kind: game::CompletionKind::Rts,
                            expected_pc: None,
                            value: None,
                        })
                        .is_err()
                );
                assert_eq!(context.cpu().hybrid_state_words(), before);
            }
            let first = proof::failure().expect("first actual nested failure captured");
            assert_eq!(first.cpu, host.cpu().hybrid_state_words());
            assert_eq!(first.frame, frame);
            assert_eq!(first.input_frame, input_frame);
            assert_eq!(
                first.bytes,
                bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap(),
                "no suspended parent restore, argument release or pending-queue repair after failure"
            );
            let before = host.cpu().hybrid_state_words();
            let mut context = host
                .context()
                .with_execution(ExecutionClass::TimeAware, WaitClock::default());
            assert!(hook.execute(&mut context).is_err());
            assert_eq!(context.cpu().hybrid_state_words(), before);
        }
    }
}

#[test]
#[ignore = "private ROMs; raw root wrapper transfer after an actual completed source"]
fn root_wrapper_transfer_after_real_source_is_protected_before_exit_validation() {
    use tje_game::Context;
    static WRAPPER: &[FramedTranslation] = &[FramedTranslation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: failing_after_real_source,
    }];
    let _proof = proof::Enable::new();
    for (revision, key) in [
        (tje_rom::Revision::Rev02, "TJE_REV02_ROM"),
        (tje_rom::Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let rom = fs::read(std::env::var(key).unwrap()).unwrap();
        let mut host = capture(&rom, revision, Arc::new(InputScript::default()), 480);
        bind_registry(
            &mut host,
            &rom,
            HookRegistry::proof_clocked_strict(revision, &[], WRAPPER, &[])
                .unwrap()
                .with_timing_sources(SOURCES)
                .unwrap(),
        );
        WRAPPER_FAILURE.with(|fault| fault.set(7));
        let hook = host.registry.at(host.cpu().pc()).unwrap().clone();
        let clock = host.wait_clock.clone();
        let frame;
        {
            let mut context = host
                .context()
                .with_execution(ExecutionClass::TimeAware, clock);
            let error = hook.execute(&mut context).unwrap_err();
            let HostError::Translation(error) = error else {
                panic!("root wrapper lost typed failure");
            };
            let original = tje_game::Error::ControlTransfer(tje_game::exits::ControlTransfer {
                entry: ids::WAIT_FOR_NEXT_V_BLANK,
                exit: tje_game::exits::Exit::Terminal,
            });
            assert_eq!(
                error,
                tje_game::Error::timing_execution_failure(ids::WAIT_FOR_NEXT_V_BLANK, original)
            );
            frame = context.frame_state().unwrap();
            let before = context.cpu().hybrid_state_words();
            assert!(
                context
                    .consume_timing_completion(game::CompletionRequest {
                        entry: ids::WAIT_FOR_NEXT_V_BLANK,
                        entry_sp: frame.entry_sp,
                        kind: game::CompletionKind::Rts,
                        expected_pc: None,
                        value: None,
                    })
                    .is_err()
            );
            assert_eq!(context.cpu().hybrid_state_words(), before);
            assert_eq!(context.frame_state(), Some(frame));
        }
        let first = proof::failure().unwrap();
        assert_eq!(first.frame, frame);
        assert_eq!(first.cpu, host.cpu().hybrid_state_words());
        assert_eq!(
            first.bytes,
            bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap()
        );
        WRAPPER_FAILURE.with(|fault| fault.set(0));
    }
}
