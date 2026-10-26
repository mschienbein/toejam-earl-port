//! Private-ROM route controls. No selected Replay-only Rust diagnostic exception.
use super::*;
use tje_engine_runtime::portable_timing::CallProfile;

#[test]
#[ignore = "requires both user ROMs and committed two-player controller sessions"]
fn bound_replay_rust_is_denied_before_effects_but_unselected_original_wait_still_runs_natively() {
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
        let (entry, frame) = capture(&cartridge, &timeline, ids::WAIT_FOR_NEXT_V_BLANK, 837);
        let program =
            metadata::compile_program(cartridge.bytes(), &metadata::game_metadata(revision))
                .unwrap();
        let digest = sha1::Sha1::digest(cartridge.bytes()).into();
        let mut backend = NativeProgram::compile(digest, &program).unwrap();
        let binding = Arc::new(
            BoundProgram::compile(
                &cartridge,
                ids::WAIT_FOR_NEXT_V_BLANK,
                tje_game::core::timed_wait::WaitSource::definition(),
                7,
                &[4, 6],
            )
            .unwrap(),
        );
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
            let mut state = NativeSourceState::new(Arc::clone(&binding), &[], profile).unwrap();
            let dispatcher = Dispatcher::hybrid_with_frames(&[], FRAMED);
            assert!(
                dispatcher
                    .resolve_entry(ids::WAIT_FOR_NEXT_V_BLANK)
                    .unwrap()
                    .is_some()
            );
            let mut clock = CallClock::default();
            let scope = clock
                .enter(ids::WAIT_FOR_NEXT_V_BLANK, CallCeiling::DEFAULT)
                .unwrap();
            let never = NeverSample(std::cell::Cell::new(0));
            let before_cpu = machine.cpu().clone();
            let before_devices = machine.devices().capture_state(registers(machine.cpu()));
            let before_frame = machine.frame_count();
            let error = match NativeTimingContext::new(
                &mut machine,
                &cartridge,
                &capability,
                &mut clock,
                &never,
                dispatcher,
                &mut state,
            ) {
                Err(error) => error,
                Ok(_) => panic!("a valid timing plan/capability cannot upgrade Replay-only Rust"),
            };
            assert!(
                error.to_string().contains("ReplayedEnvironment Rust"),
                "{error}"
            );
            assert_eq!(never.0.get(), 0);
            assert_eq!(machine.cpu(), &before_cpu);
            assert_eq!(machine.frame_count(), before_frame);
            assert_peripheral(
                &machine.devices().capture_state(registers(machine.cpu())),
                &before_devices,
            );
            let usage = clock.usage();
            assert_eq!((usage.ticks, usage.instructions, usage.frames), (0, 0, 0));
            clock.leave(scope).unwrap();
            eprintln!(
                "{revision:?}: selected Replay WAIT rejected before all effects; profile={}; ROM-bound plan present; original fallback forbidden",
                profile.label()
            );
            (_, _, backend) = machine.into_parts();
        }
        // This route is deliberately different: selected NON-Replay caller,
        // no selected WAIT Rust and no WAIT timing-source registration.
        let dispatcher = Dispatcher::hybrid_with_frames(&[], nonleaf::ROOT_ONLY);
        assert!(
            dispatcher
                .resolve_entry(ids::WAIT_FOR_NEXT_V_BLANK)
                .unwrap()
                .is_none(),
            "the positive must execute exact original instructions, never a selected Rust body"
        );
        assert_eq!(
            tje_game::execution_contract(ids::WAIT_FOR_NEXT_V_BLANK)
                .unwrap()
                .proof_envelope,
            tje_game::execution::ProofEnvelope::ReplayedEnvironment
        );
        assert_ne!(
            tje_game::execution_contract(ids::CLEAR_LOWER_VRAM_TILEMAP)
                .unwrap()
                .proof_envelope,
            tje_game::execution::ProofEnvelope::ReplayedEnvironment
        );
        let (caller_entry, caller_frame) =
            capture(&cartridge, &timeline, ids::CLEAR_LOWER_VRAM_TILEMAP, 0);
        let (machine, source, _) = run_case(
            &cartridge,
            &caller_entry,
            caller_frame,
            &session,
            backend,
            &nonleaf::TRANSITIONAL,
        );
        assert!(source > 0);
        eprintln!(
            "{revision:?}: Replay-catalog WAIT was UNSELECTED; exact compiled-original callee receipt/counters and per-step CPU/RAM/bus/full devices/audio comparison passed; not readable WAIT/native Rust proof"
        );
        drop(machine);
    }
}
