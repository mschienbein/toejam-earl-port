use super::*;
use crate::{
    Host, Mode,
    execution::WaitInputPoller,
    hooks::HookRegistry,
    input::{Button, InputHold, InputScript},
};
use std::{fs, sync::Arc};
use tje_game::{FramedTranslation, ids};
use tje_mem::ram;

static FRAMED: &[FramedTranslation] = &[FramedTranslation {
    id: ids::WAIT_FOR_NEXT_V_BLANK,
    function: tje_game::core::wait_for_next_vblank_framed,
}];
static SOURCES: &[&game::Program] = &[tje_game::core::timed_wait::WaitSource::definition()];
fn registry(revision: tje_rom::Revision) -> HookRegistry {
    HookRegistry::proof_with_frames(revision, &[], FRAMED)
        .unwrap()
        .with_timing_sources(SOURCES)
        .unwrap()
}
fn strict_registry(revision: tje_rom::Revision) -> HookRegistry {
    HookRegistry::proof_clocked_strict(revision, &[], FRAMED, &[])
        .unwrap()
        .with_timing_sources(SOURCES)
        .unwrap()
}
fn bind_registry(host: &mut Host, bytes: &[u8], registry: HookRegistry) {
    let old = host.timing_services.take();
    if let Some(services) = &old {
        assert!(services.authority.completion.protected_entry().is_none());
        assert!(services.authority.parents.is_empty());
    }
    let cartridge = Rom::import(bytes).unwrap();
    let mut prepared = if registry.timing_sources().is_empty() {
        None
    } else {
        Some(
            TimingServices::prepare(
                &cartridge,
                registry.timing_sources(),
                host.emulator.hybrid_bus().cycles.m68k_divider_u32.get(),
                registry.dispatcher(),
                host.mode,
            )
            .unwrap(),
        )
    };
    if let (Some(old), Some(new)) = (old, prepared.as_mut()) {
        // Explicit cold selection revalidation retains live trajectory identity.
        new.authority = old.authority;
    }
    host.registry = registry;
    host.timing_services = prepared;
}
fn capture(
    cartridge: &[u8],
    revision: tje_rom::Revision,
    inputs: Arc<dyn WaitInputPoller>,
    first: u64,
) -> Host {
    let mut host = Host::new(cartridge, Mode::Reference, registry(revision)).unwrap();
    let mut clock = WaitClock {
        inputs: inputs.at(0).unwrap(),
        poller: Some(inputs),
        ..WaitClock::default()
    };
    let mut audio = HeadlessAudio::default();
    for _ in 0..30_000_000 {
        let cpu = host.emulator.hybrid_cpu();
        let bus = host.emulator.hybrid_bus();
        if clock.frame >= first
            && cpu.pc() == ids::WAIT_FOR_NEXT_V_BLANK.metadata().address(revision)
            && cpu.hybrid_instruction_ready(bus)
            && bus.cycles.m68k_wait_cpu_cycles == 0
            && bus.memory.read_main_ram_word(cpu.stack_pointer() + 6) == 1
            && bus
                .memory
                .read_main_ram_byte(ram::g_mode2_reg_softcopy(revision).address())
                & 0x40
                != 0
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
    panic!("natural canonical source entry not reached");
}
fn check(host: &mut Host) -> Result<()> {
    let hook = host.registry.at(host.cpu().pc()).unwrap().clone();
    let outcome = host.check_hook(&hook)?;
    assert!(matches!(
        outcome,
        tje_game::exits::EntryOutcome::Returned(None)
    ));
    Ok(())
}
fn controller_script(session: &tje_session::Session) -> InputScript {
    assert!(matches!(session.start, tje_session::Start::PowerOn));
    let connected = session.runs[0].2 & tje_session::CONNECTED;
    let buttons = [
        (tje_session::UP, Button::Up),
        (tje_session::DOWN, Button::Down),
        (tje_session::LEFT, Button::Left),
        (tje_session::RIGHT, Button::Right),
        (tje_session::A, Button::A),
        (tje_session::B, Button::B),
        (tje_session::C, Button::C),
        (tje_session::START, Button::Start),
    ];
    let mut start = 0;
    let holds = session
        .runs
        .iter()
        .map(|&tje_session::Run(frames, p1, p2)| {
            assert_eq!(p1 & tje_session::CONNECTED, tje_session::CONNECTED);
            assert_eq!(p2 & tje_session::CONNECTED, connected);
            let end = start + frames;
            let hold = InputHold {
                start,
                end,
                p1: buttons
                    .iter()
                    .filter_map(|&(bit, button)| (p1 & bit != 0).then_some(button))
                    .collect(),
                p2: buttons
                    .iter()
                    .filter_map(|&(bit, button)| (p2 & bit != 0).then_some(button))
                    .collect(),
            };
            start = end;
            hold
        })
        .collect();
    InputScript {
        p2_connected: connected != 0,
        holds,
    }
}

#[test]
#[ignore = "requires both private ROMs and recorded controller-edge sessions"]
fn both_revision_canonical_framed_context_strict_steps_edge_cases_and_cycle_mutant() {
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
        let rom = fs::read(std::env::var(rom_key).expect("private ROM path")).unwrap();
        let cartridge = Rom::import(&rom).unwrap();
        assert_eq!(cartridge.revision(), revision);
        let session = tje_session::Session::from_json(
            &fs::read_to_string(std::env::var(session_key).expect("controller-edge session"))
                .unwrap(),
        )
        .unwrap();
        let script = controller_script(&session);
        assert_ne!(script.at(837), script.at(838));
        for (first, inputs) in [
            (
                480,
                Arc::new(InputScript::default()) as Arc<dyn WaitInputPoller>,
            ),
            (837, Arc::new(script) as Arc<dyn WaitInputPoller>),
        ] {
            let mut host = capture(&rom, revision, inputs, first);
            bind_registry(&mut host, &rom, strict_registry(revision));
            host.checks = crate::CheckReport::registered(&host.registry);
            let initial_frame = host.frame_count();
            check(&mut host).unwrap();
            let receipt = proof::receipts().pop().unwrap();
            assert_eq!(receipt.frame, initial_frame);
            if first == 837 {
                assert!(receipt.returned_frame >= 838);
            }
            assert!(receipt.source > 11 && receipt.original_isr > 0);
            let usage = host
                .last_timing_source_usage(ids::WAIT_FOR_NEXT_V_BLANK)
                .unwrap();
            assert_eq!(usage.instructions, receipt.source);
            assert_eq!(usage.original_isr_instructions, receipt.original_isr);
            assert_eq!(usage.compiled_isr_instructions, 0);
            eprintln!(
                "{revision:?}: CANONICAL framed host frame={}→{} source={} explicit_original_ISR={} effects={} steps={}; full per-step CPU/bus/RAM +returned hidden devices/audio exact",
                receipt.frame,
                receipt.returned_frame,
                receipt.source,
                receipt.original_isr,
                receipt.bus_effects,
                receipt.steps
            );
            let sp = host.cpu().stack_pointer();
            for count in [0, u16::MAX] {
                host.emulator
                    .hybrid_bus_mut()
                    .memory
                    .write_main_ram_word(sp + 6, count);
                check(&mut host).unwrap();
                assert_eq!(proof::receipts().len(), 1);
            }
            host.emulator
                .hybrid_bus_mut()
                .memory
                .write_main_ram_word(sp + 6, 1);
            let display = ram::g_mode2_reg_softcopy(revision).address();
            let mode = host
                .emulator
                .hybrid_bus()
                .memory
                .read_main_ram_byte(display);
            host.emulator
                .hybrid_bus_mut()
                .memory
                .write_main_ram_byte(display, mode & !0x40);
            check(&mut host).unwrap();
            assert_eq!(proof::receipts().len(), 1);
            host.emulator
                .hybrid_bus_mut()
                .memory
                .write_main_ram_byte(display, mode);
            proof::mutate_next(2);
            let error = check(&mut host).unwrap_err();
            assert!(
                error.to_string().contains("wrong-cycle mutant rejected"),
                "{error}"
            );
            assert!(proof::receipts().is_empty());
            let live = format!(
                "{:?}",
                host.timing_services.as_ref().unwrap().authority.completion
            );
            check(&mut host).unwrap();
            assert_eq!(
                proof::receipts().len(),
                1,
                "a failed discarded fork cannot poison the next probe"
            );
            assert_eq!(
                live,
                format!(
                    "{:?}",
                    host.timing_services.as_ref().unwrap().authority.completion
                )
            );
            eprintln!(
                "{revision:?}: canonical +2-cycle prefix mutant REJECTED; zero/negative/display-off actual-frame cases PASS"
            );
        }
    }
}

#[test]
#[ignore = "requires both private ROMs; real-return strict fixture and passive capture"]
fn public_clocked_fixture_preserves_real_caller_and_source_capture_in_both_revisions() {
    use crate::check_capture::{AccessActor, CapturedSide, Observation};
    let _proof = proof::Enable::new();
    for (revision, key) in [
        (tje_rom::Revision::Rev02, "TJE_REV02_ROM"),
        (tje_rom::Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let rom = fs::read(std::env::var(key).unwrap()).unwrap();
        let mut host = capture(&rom, revision, Arc::new(InputScript::default()), 480);
        bind_registry(&mut host, &rom, strict_registry(revision));
        host.checks = crate::CheckReport::registered(&host.registry);
        host.set_check_capture(true).unwrap();
        host.settle_fixture_boundary(host.wait_clock.inputs)
            .unwrap();
        assert!(host.fixture_boundary_is_settled());
        assert_ne!(host.cpu().pc(), crate::abi::CALL_SENTINEL);
        let before = host.cpu().hybrid_state_words();
        let before_devices =
            bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap();
        let live_authority = format!(
            "{:?}",
            host.timing_services.as_ref().unwrap().authority.completion
        );
        assert_eq!(
            host.check_clocked_fixture(
                ids::WAIT_FOR_NEXT_V_BLANK.metadata().address(revision),
                &[1],
            )
            .unwrap(),
            tje_game::exits::EntryOutcome::Returned(None),
        );
        assert_eq!(host.cpu().hybrid_state_words(), before);
        assert_eq!(
            bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap(),
            before_devices,
            "discarded strict fixture must restore the real caller's complete devices",
        );
        assert_eq!(
            format!(
                "{:?}",
                host.timing_services.as_ref().unwrap().authority.completion
            ),
            live_authority,
        );
        let receipt = proof::receipts().pop().unwrap();
        assert!(receipt.source > 11 && receipt.original_isr > 0);
        let capture = host
            .actual_check_capture()
            .expect("actual strict fork capture");
        assert!(
            capture.grouping_issues.is_empty(),
            "{:?}",
            capture.grouping_issues
        );
        let CapturedSide::Completed(original) = &capture.original else {
            panic!("actual original fork was not completed");
        };
        let CapturedSide::Completed(translated) = &capture.translated else {
            panic!("actual source fork was not completed");
        };
        for side in [original, translated] {
            assert!(side.issues.is_empty(), "{:?}", side.issues);
            assert_eq!(side.admission, "capture_only_no_projection_approval");
        }
        assert!(
            translated.observations.iter().any(|observation| matches!(
                observation,
                Observation::Access(access)
                    if matches!(access.actor, AccessActor::Native) && !access.exact_operand,
            )),
            "source effects keep native attribution and checkpoint-bounded physical phases",
        );
        let mut real_isr_opcodes = 0;
        for observation in &translated.observations {
            if let Observation::Instruction(instruction) = observation {
                assert!(
                    instruction.instruction.exception_depth > translated.entry.exception_depth,
                    "a source/hardware-wait step must not invent an original main opcode",
                );
                real_isr_opcodes += 1;
            }
        }
        assert!(real_isr_opcodes > 0 && !translated.exceptions.is_empty());
        eprintln!(
            "{revision:?}: public ClockedStrict real-return fixture; actual source/native access capture and {real_isr_opcodes} real ISR opcodes, no fabricated main opcode, no capture issues",
        );
    }
}

#[test]
fn descriptor_admission_does_not_activate_an_entry_or_accept_a_value_only_candidate() {
    let empty = HookRegistry::empty(tje_rom::Revision::Rev02);
    assert!(empty.with_timing_sources(SOURCES).is_err());
    static LEGACY: &[tje_game::Translation] = &[tje_game::Translation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: tje_game::core::wait_for_next_vblank,
    }];
    assert!(
        HookRegistry::proof(tje_rom::Revision::Rev02, LEGACY)
            .unwrap()
            .with_timing_sources(SOURCES)
            .is_err()
    );
    let registry = registry(tje_rom::Revision::Rev02);
    assert_eq!(registry.len(), 1);
    assert!(tje_game::framed_translations().all(|entry| entry.id != ids::WAIT_FOR_NEXT_V_BLANK));
}

#[test]
#[ignore = "requires both private ROMs; canonical source plus prepared caller finalization"]
fn prepared_outgoing_call_consumes_real_source_completion_before_parent_restore() {
    use tje_game::{
        Context,
        call_frame::{CallSite, OutgoingArguments},
    };
    let _proof = proof::Enable::new();
    for (revision, key) in [
        (tje_rom::Revision::Rev02, "TJE_REV02_ROM"),
        (tje_rom::Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let rom = fs::read(std::env::var(key).unwrap()).unwrap();
        let mut host = capture(&rom, revision, Arc::new(InputScript::default()), 480);
        let original_sp = host.cpu().stack_pointer();
        let memory = &host.emulator.hybrid_bus().memory;
        let actual_return = (u32::from(memory.read_main_ram_word(original_sp)) << 16)
            | u32::from(memory.read_main_ram_word(original_sp + 2));
        let clock = host.wait_clock.clone();
        {
            // Authored prepared-callee ABI fixture, not a claim that this
            // diagnostic caller's zero-time argument setup is clock-exact.
            let mut context = host
                .context()
                .with_execution(ExecutionClass::TimeAware, clock);
            let parent = context.frame_state().unwrap();
            let outgoing = OutgoingArguments::push(&mut context, &[0]).unwrap();
            let dispatcher = context.dispatcher();
            outgoing
                .call_void(
                    &mut context,
                    dispatcher,
                    ids::WAIT_FOR_NEXT_V_BLANK,
                    CallSite::new(actual_return),
                )
                .unwrap();
            // Actual timed-callee D0 remains observable independently of the
            // ABI-void call's unit result (authored prepared-ABI fixture).
            assert_eq!(context.cpu().data_registers()[0], 0);
            assert_eq!(context.cpu().pc(), actual_return);
            assert_eq!(
                context.stack_pointer().unwrap(),
                original_sp - 4,
                "clocked RTS was not popped twice"
            );
            assert_eq!(context.frame_state(), Some(parent));
            outgoing.release(&mut context).unwrap();
            assert_eq!(context.stack_pointer().unwrap(), original_sp);
        }
        assert_eq!(proof::receipts().len(), 1);
    }
}

fn mixed_registry(revision: tje_rom::Revision) -> HookRegistry {
    static LEGACY: &[tje_game::Translation] = &[tje_game::Translation {
        id: ids::ABS,
        function: tje_game::runtime::abs,
    }];
    HookRegistry::proof_with_frames(revision, LEGACY, FRAMED)
        .unwrap()
        .with_timing_sources(SOURCES)
        .unwrap()
}

fn seed_lifetime_entry(host: &mut Host, id: FunctionId, sp: u32, return_pc: u32, argument: u32) {
    // Authored prepared-root ABI fixture, not a game/caller timing claim.
    let address = id.metadata().address(host.revision);
    let (cpu, bus) = host.emulator.hybrid_parts();
    for (offset, word) in [
        (0, (return_pc >> 16) as u16),
        (2, return_pc as u16),
        (4, (argument >> 16) as u16),
        (6, argument as u16),
    ] {
        bus.memory.write_main_ram_word(sp + offset, word);
    }
    cpu.set_address_registers(cpu.address_registers(), cpu.user_stack_pointer(), sp);
    cpu.set_status_register(0x2700);
    cpu.set_pc(address, bus);
}

fn actual_lifetime_root(
    host: &mut Host,
    id: FunctionId,
    finalize: bool,
) -> Result<tje_game::exits::EntryOutcome> {
    let hook = host
        .registry
        .at(id.metadata().address(host.revision))
        .unwrap()
        .clone();
    let clock = host.wait_clock.clone();
    let mut context = host.context().with_execution(hook.execution_class, clock);
    let outcome = hook.execute(&mut context).and_then(|outcome| {
        if finalize {
            crate::exits::finish(&mut context, &hook, outcome)?;
        }
        Ok(outcome)
    });
    let clock = std::mem::take(&mut context.wait_clock);
    let audio = std::mem::take(&mut context.time_audio);
    drop(context);
    host.frame = clock.frame;
    host.wait_clock = clock;
    host.audio.samples.extend(audio);
    outcome
}

#[test]
#[ignore = "requires both private ROMs; actual mixed roots and discarded proof authority"]
fn mixed_root_epochs_speculative_failure_and_real_reset_are_isolated() {
    let _proof = proof::Enable::new();
    for (revision, key) in [
        (tje_rom::Revision::Rev02, "TJE_REV02_ROM"),
        (tje_rom::Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let rom = fs::read(std::env::var(key).unwrap()).unwrap();
        let mut host = capture(&rom, revision, Arc::new(InputScript::default()), 480);
        bind_registry(&mut host, &rom, mixed_registry(revision));
        host.checks = crate::CheckReport::registered(&host.registry);
        let sp = host.cpu().stack_pointer();
        let memory = &host.emulator.hybrid_bus().memory;
        let return_pc = (u32::from(memory.read_main_ram_word(sp)) << 16)
            | u32::from(memory.read_main_ram_word(sp + 2));
        for id in [
            ids::WAIT_FOR_NEXT_V_BLANK,
            ids::ABS,
            ids::WAIT_FOR_NEXT_V_BLANK,
        ] {
            let argument = if id == ids::ABS { (-7_i32) as u32 } else { 0 };
            seed_lifetime_entry(&mut host, id, sp, return_pc, argument);
            let outcome = actual_lifetime_root(&mut host, id, true).unwrap();
            assert_eq!(
                outcome,
                tje_game::exits::EntryOutcome::Returned((id == ids::ABS).then_some(7),)
            );
            assert_eq!(host.cpu().pc(), return_pc);
            assert_eq!(host.cpu().stack_pointer(), sp + 4);
            drop(proof::receipts());
        }
        seed_lifetime_entry(&mut host, ids::WAIT_FOR_NEXT_V_BLANK, sp, return_pc, 0);
        let live = format!(
            "{:?}",
            host.timing_services.as_ref().unwrap().authority.completion
        );
        host.set_check_failure_policy(crate::CheckFailurePolicy::ContinueOriginal)
            .unwrap();
        proof::mutate_next(2);
        let timed = host.registry.at(host.cpu().pc()).unwrap().clone();
        host.check_live_hook(&timed).unwrap(); // Real ContinueOriginal policy seam.
        assert!(
            host.disabled_check_roots
                .contains(&timed.symbol.address(revision))
        );
        assert!(
            host.checks
                .failures
                .last()
                .unwrap()
                .detail
                .contains("wrong-cycle mutant rejected")
        );
        assert_eq!(
            live,
            format!(
                "{:?}",
                host.timing_services.as_ref().unwrap().authority.completion
            )
        );
        host.isolated_fixture(|host| {
            seed_lifetime_entry(host, ids::ABS, sp, return_pc, (-7_i32) as u32);
            let legacy = host.registry.at(host.cpu().pc()).unwrap().clone();
            assert_eq!(
                host.check_hook(&legacy)?,
                tje_game::exits::EntryOutcome::Returned(Some(7))
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(
            live,
            format!(
                "{:?}",
                host.timing_services.as_ref().unwrap().authority.completion
            )
        );
        check(&mut host).unwrap(); // Explicit correct diagnostic probe, not live reactivation.
        assert_eq!(proof::receipts().len(), 1);
        assert_eq!(
            live,
            format!(
                "{:?}",
                host.timing_services.as_ref().unwrap().authority.completion
            )
        );
        assert!(
            host.disabled_check_roots
                .contains(&timed.symbol.address(revision))
        );
        // An ACTUAL pending receipt survives Context destruction and cannot be
        // mistaken for another legacy root's ordinary absence.
        actual_lifetime_root(&mut host, ids::WAIT_FOR_NEXT_V_BLANK, false).unwrap();
        seed_lifetime_entry(&mut host, ids::ABS, sp, return_pc, (-7_i32) as u32);
        let before = host.cpu().hybrid_state_words();
        let clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
        assert!(actual_lifetime_root(&mut host, ids::ABS, true).is_err());
        assert_eq!(before, host.cpu().hybrid_state_words());
        assert_eq!(clock, host.emulator.hybrid_bus().cycles.m68k_mclk_cycles);
        host.reset(); // Only the real physical reset permits new authority.
        seed_lifetime_entry(&mut host, ids::WAIT_FOR_NEXT_V_BLANK, sp, return_pc, 0);
        actual_lifetime_root(&mut host, ids::WAIT_FOR_NEXT_V_BLANK, true).unwrap();
        drop(proof::receipts());
        seed_lifetime_entry(&mut host, ids::WAIT_FOR_NEXT_V_BLANK, sp, return_pc, 0);
        proof::mutate_next(2);
        assert!(actual_lifetime_root(&mut host, ids::WAIT_FOR_NEXT_V_BLANK, true).is_err());
        seed_lifetime_entry(&mut host, ids::ABS, sp, return_pc, (-7_i32) as u32);
        let before = host.cpu().hybrid_state_words();
        let clock = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
        assert!(actual_lifetime_root(&mut host, ids::ABS, true).is_err());
        assert_eq!(before, host.cpu().hybrid_state_words());
        assert_eq!(clock, host.emulator.hybrid_bus().cycles.m68k_mclk_cycles);
        host.reset();
        seed_lifetime_entry(&mut host, ids::WAIT_FOR_NEXT_V_BLANK, sp, return_pc, 0);
        actual_lifetime_root(&mut host, ids::WAIT_FOR_NEXT_V_BLANK, true).unwrap();
        drop(proof::receipts());
        eprintln!(
            "{revision:?}: timed→legacy→timed actual roots, ContinueOriginal bad SPEC→other root→correct probe, live epoch unchanged; actual pending/failed Context drop rejects, real-reset recovery PASS"
        );
    }
}

thread_local! {
    static WRAPPER_FAILURE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}
fn failing_after_real_source(context: &mut dyn tje_game::Context) -> tje_game::Result<Option<u32>> {
    let value = tje_game::core::wait_for_next_vblank_framed(context)?;
    let fault = WRAPPER_FAILURE.with(std::cell::Cell::get);
    if matches!(fault, 2 | 3) {
        // A real status read without its required open-bus source latches the
        // existing MMIO contract error, including the real read side effects.
        context.read_device_u16(0xc00004);
    }
    if fault == 4 {
        let _orphan = context.enter_call_ceiling(ids::WAIT_FOR_NEXT_V_BLANK)?;
    }
    if fault == 5 {
        return Err(tje_game::Error::timing_execution_failure(
            ids::WAIT_FOR_NEXT_V_BLANK,
            tje_game::Error::Platform("preboxed actual source failure".into()),
        ));
    }
    if fault == 6 {
        return Err(tje_game::Error::ClockAfterTransfer {
            transfer: tje_game::exits::ControlTransfer {
                entry: ids::WAIT_FOR_NEXT_V_BLANK,
                exit: tje_game::exits::Exit::ExceptionReturn,
            },
            error: tje_game::execution::CallClockError::Unavailable,
        });
    }
    if fault == 7 {
        return Err(tje_game::Error::ControlTransfer(
            tje_game::exits::ControlTransfer {
                entry: ids::WAIT_FOR_NEXT_V_BLANK,
                exit: tje_game::exits::Exit::Terminal,
            },
        ));
    }
    if matches!(fault, 1 | 3) {
        return Err(tje_game::Error::Platform(
            "authored primary after actual RTS".into(),
        ));
    }
    Ok(value)
}

#[test]
#[ignore = "requires both private ROMs; real prepared-callee failure boundaries"]
fn admitted_prepared_failures_preserve_actual_frames_and_first_causes() {
    use tje_game::{Context, Error, call_frame::CallSite, execution::CallCeiling};
    static BAD_WRAPPER: &[FramedTranslation] = &[FramedTranslation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: failing_after_real_source,
    }];
    let _proof = proof::Enable::new();
    for (revision, key) in [
        (tje_rom::Revision::Rev02, "TJE_REV02_ROM"),
        (tje_rom::Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let rom = fs::read(std::env::var(key).unwrap()).unwrap();
        for case in 0..12 {
            let mut host = capture(&rom, revision, Arc::new(InputScript::default()), 480);
            if case >= 6 {
                bind_registry(
                    &mut host,
                    &rom,
                    HookRegistry::proof_with_frames(revision, &[], BAD_WRAPPER)
                        .unwrap()
                        .with_timing_sources(SOURCES)
                        .unwrap(),
                );
            }
            WRAPPER_FAILURE.with(|value| value.set(if case >= 6 { case - 5 } else { 0 }));
            let sp = host.cpu().stack_pointer();
            let memory = &host.emulator.hybrid_bus().memory;
            let return_pc = (u32::from(memory.read_main_ram_word(sp)) << 16)
                | u32::from(memory.read_main_ram_word(sp + 2));
            match case {
                0 => proof::fail_after(proof::FailurePoint::Capture),
                1 => proof::fail_after(proof::FailurePoint::RteTail),
                2 => proof::fail_after(proof::FailurePoint::Frame),
                4 => proof::fault_receipt(proof::ReceiptFault::WrongKind),
                5 => proof::fault_receipt(proof::ReceiptFault::Duplicate),
                _ => {}
            }
            let clock = host.wait_clock.clone();
            let frame;
            let input_frame;
            let failure;
            {
                let mut context = host
                    .context()
                    .with_execution(ExecutionClass::TimeAware, clock);
                if case == 3 {
                    let _root = context
                        .wait_clock
                        .call_clock
                        .enter(
                            ids::WAIT_FOR_NEXT_V_BLANK,
                            CallCeiling::new(8, CallCeiling::DEFAULT.frames).unwrap(),
                        )
                        .unwrap();
                }
                let dispatcher = context.dispatcher();
                let count = if case == 2 {
                    2
                } else if case <= 3 {
                    1
                } else {
                    0
                };
                failure = tje_game::call_frame::call_at_void(
                    &mut context,
                    dispatcher,
                    ids::WAIT_FOR_NEXT_V_BLANK,
                    &[count],
                    CallSite::new(return_pc),
                )
                .unwrap_err();
                assert!(
                    failure.is_failed_execution() && failure.preserves_call_frame(),
                    "{failure}"
                );
                if case == 11 {
                    assert!(
                        matches!(failure, Error::ClockAfterTransfer { .. }),
                        "{failure}"
                    );
                } else {
                    assert!(
                        matches!(failure, Error::TimingExecutionFailure { .. }),
                        "{failure}"
                    );
                }
                if matches!(case, 6 | 8) {
                    match &failure {
                        Error::TimingExecutionFailure { cause, .. } => assert_eq!(
                            cause.as_ref(),
                            &Error::Platform("authored primary after actual RTS".into()),
                        ),
                        _ => unreachable!(),
                    }
                }
                if matches!(case, 10 | 11) {
                    let before = context.cpu().hybrid_state_words();
                    let frame_before = context.frame_state();
                    assert!(
                        context
                            .consume_timing_completion(game::CompletionRequest {
                                entry: ids::WAIT_FOR_NEXT_V_BLANK,
                                entry_sp: sp - 8,
                                kind: game::CompletionKind::Rts,
                                expected_pc: Some(return_pc),
                                value: None,
                            })
                            .is_err(),
                        "a caught preboxed fatal error cannot consume its old valid receipt"
                    );
                    assert_eq!(before, context.cpu().hybrid_state_words());
                    assert_eq!(frame_before, context.frame_state());
                }
                frame = context.frame_state().unwrap();
                input_frame = context.wait_clock.frame;
            }
            let actual = proof::failure().expect("an actual failure boundary was observed");
            assert_eq!(
                actual.cpu,
                host.cpu().hybrid_state_words(),
                "{revision:?} case {case}: CPU"
            );
            assert_eq!(
                actual.frame, frame,
                "{revision:?} case {case}: no caller frame restoration"
            );
            assert_eq!(actual.input_frame, input_frame);
            assert_eq!(
                actual.bytes,
                bincode::encode_to_vec(&host.emulator, bincode::config::standard()).unwrap(),
                "{revision:?} case {case}: all physical state including pending writes stays as failed"
            );
            assert_ne!(
                host.cpu().stack_pointer(),
                sp,
                "caller arguments were not released"
            );
            drop(proof::receipts());
        }
        // A rejected plan before admission has no physical timing authority;
        // retain ordinary diagnostic cleanup, rather than poisoning old roots.
        let mut host = capture(&rom, revision, Arc::new(InputScript::default()), 480);
        host.timing_services = None;
        let sp = host.cpu().stack_pointer();
        let bus = host.emulator.hybrid_bus();
        let return_pc = (u32::from(bus.memory.read_main_ram_word(sp)) << 16)
            | u32::from(bus.memory.read_main_ram_word(sp + 2));
        let before_clock = bus.cycles.m68k_mclk_cycles;
        let clock = host.wait_clock.clone();
        {
            let mut context = host
                .context()
                .with_execution(ExecutionClass::TimeAware, clock);
            let parent = context.frame_state().unwrap();
            let dispatcher = context.dispatcher();
            let error = tje_game::call_frame::call_at_void(
                &mut context,
                dispatcher,
                ids::WAIT_FOR_NEXT_V_BLANK,
                &[0],
                CallSite::new(return_pc),
            )
            .unwrap_err();
            assert!(!error.preserves_call_frame());
            assert_eq!(context.stack_pointer().unwrap(), sp);
            assert_eq!(context.frame_state(), Some(parent));
        }
        assert_eq!(
            before_clock,
            host.emulator.hybrid_bus().cycles.m68k_mclk_cycles
        );
        eprintln!(
            "{revision:?}: real capture/RTE-tail/frame/clock failures, wrong+duplicate consumption, post-RTS primary/MMIO/ceiling errors preserve exact failed CPU/frame/pending writes; pre-admission cleanup PASS"
        );
    }
}

fn ordinary_legacy_failure(
    _: &mut dyn tje_game::Context,
    _: &[u32],
) -> tje_game::Result<Option<u32>> {
    Err(tje_game::Error::Platform(
        "ordinary legacy diagnostic".into(),
    ))
}
#[test]
#[ignore = "requires both private ROMs; consumed timing authority must not taint later ordinary errors"]
fn consumed_source_does_not_make_new_legacy_diagnostic_errors_fatal() {
    use tje_game::{
        Context,
        call_frame::{CallSite, call_at},
    };
    static LEGACY_ERROR: &[tje_game::Translation] = &[tje_game::Translation {
        id: ids::ABS,
        function: ordinary_legacy_failure,
    }];
    let _proof = proof::Enable::new();
    for (revision, key) in [
        (tje_rom::Revision::Rev02, "TJE_REV02_ROM"),
        (tje_rom::Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let rom = fs::read(std::env::var(key).unwrap()).unwrap();
        let mut host = capture(&rom, revision, Arc::new(InputScript::default()), 480);
        let sp = host.cpu().stack_pointer();
        let memory = &host.emulator.hybrid_bus().memory;
        let return_pc = (u32::from(memory.read_main_ram_word(sp)) << 16)
            | u32::from(memory.read_main_ram_word(sp + 2));
        seed_lifetime_entry(&mut host, ids::WAIT_FOR_NEXT_V_BLANK, sp, return_pc, 0);
        actual_lifetime_root(&mut host, ids::WAIT_FOR_NEXT_V_BLANK, true).unwrap();
        drop(proof::receipts());
        bind_registry(
            &mut host,
            &rom,
            HookRegistry::proof_with_frames(revision, LEGACY_ERROR, FRAMED)
                .unwrap()
                .with_timing_sources(SOURCES)
                .unwrap(),
        );
        seed_lifetime_entry(&mut host, ids::ABS, sp, return_pc, 7);
        let before = host.emulator.hybrid_bus().cycles.m68k_mclk_cycles;
        let clock = host.wait_clock.clone();
        {
            let mut context = host
                .context()
                .with_execution(ExecutionClass::TimeAware, clock);
            let parent = context.frame_state().unwrap();
            let dispatcher = context.dispatcher();
            let error = call_at(
                &mut context,
                dispatcher,
                ids::ABS,
                &[7],
                CallSite::new(return_pc),
            )
            .unwrap_err();
            assert_eq!(
                error,
                tje_game::Error::Platform("ordinary legacy diagnostic".into())
            );
            assert!(!error.preserves_call_frame());
            assert_eq!(context.stack_pointer().unwrap(), sp);
            assert_eq!(context.frame_state(), Some(parent));
        }
        assert_eq!(before, host.emulator.hybrid_bus().cycles.m68k_mclk_cycles);
    }
}

thread_local! {
    static SOURCE_FAILURE_STAGE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}
struct SourceFailure {
    source: tje_game::core::timed_wait::WaitSource,
    stage: u8,
}
fn source_sentinel(stage: u8) -> tje_game::Error {
    let message = match stage {
        0 => "owned source begin sentinel",
        1 => "owned source next sentinel",
        2 => "owned source acknowledgement sentinel",
        _ => "owned source finish sentinel",
    };
    tje_game::Error::Platform(message.into())
}
impl game::TimingSource for SourceFailure {
    fn program(&self) -> &'static game::Program {
        self.source.program()
    }
    fn begin_instruction(
        &mut self,
        site: game::InstructionSite,
        state: game::Registers,
    ) -> tje_game::Result<()> {
        if self.stage == 0 {
            return Err(source_sentinel(0));
        }
        self.source.begin_instruction(site, state)
    }
    fn next_effect(&mut self) -> tje_game::Result<Option<game::Effect>> {
        if self.stage == 1 {
            return Err(source_sentinel(1));
        }
        self.source.next_effect()
    }
    fn acknowledge(&mut self, actual: game::Observation) -> tje_game::Result<()> {
        self.source.acknowledge(actual)?;
        if self.stage == 2 {
            return Err(source_sentinel(2));
        }
        Ok(())
    }
    fn finish_instruction(&mut self) -> tje_game::Result<game::Commit> {
        if self.stage == 3 {
            return Err(source_sentinel(3));
        }
        self.source.finish_instruction()
    }
}
fn source_failure_body(context: &mut dyn tje_game::Context) -> tje_game::Result<Option<u32>> {
    let mut source = SourceFailure {
        source: tje_game::core::timed_wait::WaitSource::new(context.revision()),
        stage: SOURCE_FAILURE_STAGE.with(std::cell::Cell::get),
    };
    let primary = context.execute_timing_program(ids::WAIT_FOR_NEXT_V_BLANK, &mut source);
    // Seed a genuine secondary MMIO contract failure after the callback's
    // error; it must never replace or stringify the first source-owned cause.
    context.read_device_u16(0xc00004);
    primary.map(|exit| exit.value)
}
#[test]
#[ignore = "requires both private ROMs; source-owned first-cause preservation"]
fn source_callback_causes_survive_owned_boundaries_and_secondary_checks() {
    use tje_game::{Context, Error, call_frame::CallSite};
    static FAILING: &[FramedTranslation] = &[FramedTranslation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: source_failure_body,
    }];
    for (revision, key) in [
        (tje_rom::Revision::Rev02, "TJE_REV02_ROM"),
        (tje_rom::Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let rom = fs::read(std::env::var(key).unwrap()).unwrap();
        for stage in 0..4 {
            let mut host = capture(&rom, revision, Arc::new(InputScript::default()), 480);
            bind_registry(
                &mut host,
                &rom,
                HookRegistry::proof_with_frames(revision, &[], FAILING)
                    .unwrap()
                    .with_timing_sources(SOURCES)
                    .unwrap(),
            );
            SOURCE_FAILURE_STAGE.with(|value| value.set(stage));
            let sp = host.cpu().stack_pointer();
            let memory = &host.emulator.hybrid_bus().memory;
            let return_pc = (u32::from(memory.read_main_ram_word(sp)) << 16)
                | u32::from(memory.read_main_ram_word(sp + 2));
            let clock = host.wait_clock.clone();
            let mut context = host
                .context()
                .with_execution(ExecutionClass::TimeAware, clock);
            let dispatcher = context.dispatcher();
            let error = tje_game::call_frame::call_at_void(
                &mut context,
                dispatcher,
                ids::WAIT_FOR_NEXT_V_BLANK,
                &[0],
                CallSite::new(return_pc),
            )
            .unwrap_err();
            match &error {
                Error::TimingExecutionFailure { entry, cause } => {
                    assert_eq!(*entry, ids::WAIT_FOR_NEXT_V_BLANK);
                    assert_eq!(cause.as_ref(), &source_sentinel(stage));
                    assert_eq!(
                        std::error::Error::source(&error).unwrap().to_string(),
                        source_sentinel(stage).to_string()
                    );
                }
                _ => panic!("source failure lost its owned cause: {error}"),
            }
            assert!(
                matches!(
                    tje_game::Platform::check_mmio_contract(&context),
                    Err(Error::OpenBus(_))
                ),
                "the independent secondary MMIO error must actually exist"
            );
        }
        eprintln!(
            "{revision:?}: owned begin/next/ack/finish causes retained exactly through final fatal failure despite secondary MMIO errors"
        );
    }
}

#[test]
#[ignore = "requires both private ROMs; empty-descriptor startup fast path"]
fn empty_source_registry_keeps_host_timing_state_absent() {
    for (revision, key) in [
        (tje_rom::Revision::Rev02, "TJE_REV02_ROM"),
        (tje_rom::Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let rom = fs::read(std::env::var(key).unwrap()).unwrap();
        let host = Host::new(&rom, Mode::Reference, HookRegistry::empty(revision)).unwrap();
        assert!(host.timing_services.is_none());
    }
}

#[path = "nonleaf_tests.rs"]
mod nonleaf;
