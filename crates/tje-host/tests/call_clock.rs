//! Authored RTS instruction streams and private-cartridge clock envelopes.
//! No cartridge instructions/data are embedded in these tests.

use std::{fs, path::PathBuf, sync::Arc};
use tje_game::{
    Context,
    execution::{CallCeiling, WaitCondition, WaitRequest},
    ids,
};
use tje_host::{
    Host, HostError, Mode,
    abi::HookContext,
    hooks::{HookFn, HookRegistry},
    input::InputScript,
};
use tje_mem::{Platform, Ptr, Revision, ram};

const CODE: u32 = 0xfff000;

fn bytes(revision: Revision) -> Vec<u8> {
    let (variable, name) = match revision {
        Revision::Rev02 => ("TJE_REV02_ROM", "rev02.bin"),
        Revision::Rev00 => ("TJE_REV00_ROM", "rev00.bin"),
    };
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../roms").join(name));
    fs::read(path).expect("private retail cartridge")
}

fn correct(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let count = c.argument_u32(0) as u16;
    for _ in 0..count {
        c.call_original(
            ids::WAIT_FOR_NEXT_V_BLANK.metadata().address(c.revision()),
            &[1],
        )?;
    }
    Ok(None)
}
fn early(_: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    Ok(None)
}
fn wrong_effect(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    correct(c)?;
    c.write_u8(0xff9000, 1);
    Ok(None)
}

fn fixture(revision: Revision, body: HookFn, ceiling: CallCeiling) -> Host {
    let mut symbol = ids::WAIT_FOR_NEXT_V_BLANK.metadata().clone();
    symbol.name = "AuthoredLongRts";
    symbol.addresses = [CODE; 2];
    symbol.sizes = [34; 2];
    symbol.source = "authored synthetic RTS loop";
    symbol.abi.argument_count = 1;
    symbol.abi.returns_value = false;
    let mut registry = HookRegistry::empty(revision);
    registry
        .register_clock_ceiling(symbol, ceiling, body)
        .unwrap();
    let mut host = Host::new(&bytes(revision), Mode::Reference, registry).unwrap();
    host.set_wait_input_poller(Arc::new(InputScript::default()));
    let target = ids::WAIT_FOR_NEXT_V_BLANK.metadata().address(revision);
    // Save D2; load the 32-bit argument's low word; repeat a real counter wait;
    // restore D2; RTS. The branch displacements are authored synthetic code.
    let words = [
        0x48e7,
        0x2000,
        0x342f,
        10,
        0x6712,
        0x2f3c,
        0,
        1,
        0x4eb9,
        (target >> 16) as u16,
        target as u16,
        0x588f,
        0x5342,
        0x66ee,
        0x4cdf,
        4,
        0x4e75,
    ];
    for (index, word) in words.into_iter().enumerate() {
        host.context().write_u16(CODE + 2 * index as u32, word);
    }
    let mut c = host.context();
    // Labelled counter-only IRQ fixture: actual VDP IRQ generation, no fabricated
    // counter increment, audio readiness value, forced interrupt or frame skip.
    ram::g_mode2_reg_softcopy(revision).write(&mut c, 0x40);
    ram::g_v_blank_func(revision).write(&mut c, Ptr::new(0));
    ram::g_v_blank_graphics_enabled(revision).write(&mut c, 0);
    ram::g_h_blank_graphics_enabled(revision).write(&mut c, 0);
    ram::g_v_blank_graphics_paused(revision).write(&mut c, 1);
    ram::g_v_blank_audio_input_override(revision).write(&mut c, 0);
    let sr = c.status_register() & !0x0700;
    c.set_status_register(sr);
    c.write_vdp_control(0x8164);
    host
}

#[test]
#[ignore = "requires both private cartridges; authored 130-frame RTS fixture"]
fn long_rts_completion_and_wrong_translations_both_revisions() {
    let ceiling = CallCeiling::new(3_000_000, 200).unwrap();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = fixture(revision, correct, ceiling);
        host.check_fixture(CODE, &[130]).unwrap();
        assert_eq!(host.checks.mismatches, 0);
        assert!(
            host.checks
                .call_clocks
                .iter()
                .any(|run| run.consumed_ticks > CallCeiling::DEFAULT.ticks)
        );
        assert!(
            host.checks
                .call_clocks
                .iter()
                .all(|run| run.completed && run.configured_tick_ceiling == ceiling.ticks)
        );
        println!(
            "CALL_CEILING_COMPLETED {revision:?} {}",
            serde_json::to_string(&host.checks.call_clocks).unwrap()
        );
        for body in [early as HookFn, wrong_effect] {
            let mut host = fixture(revision, body, ceiling);
            let error = host.check_fixture(CODE, &[3]).unwrap_err();
            assert!(matches!(error, HostError::Mismatch { .. }), "{error}");
            println!("CALL_CEILING_REJECTED {revision:?} wrong translation: {error}");
        }
    }
}

#[test]
#[ignore = "requires both private cartridges"]
fn root_nested_and_no_frame_loops_exhaust_the_same_deadline() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (ceiling, count, resource) in [
            (CallCeiling::new(3, 100).unwrap(), 5, "ticks"),
            (CallCeiling::new(3_000_000, 2).unwrap(), 5, "frames"),
        ] {
            let mut host = fixture(revision, correct, ceiling);
            let error = host.check_fixture(CODE, &[count]).unwrap_err();
            let message = error.to_string();
            assert!(
                message.contains("ceiling exceeded") && message.contains(resource),
                "{message}"
            );
            assert!(host.checks.call_clocks.iter().any(|run| !run.completed));
            println!("CALL_CEILING_REJECTED {revision:?} nested {resource}: {error}");
        }
        let mut host = fixture(revision, early, CallCeiling::new(4, 100).unwrap());
        host.context().write_u16(CODE, 0x60fe); // Authored BRA.S to self.
        let error = host.check_fixture(CODE, &[0]).unwrap_err();
        assert!(
            error.to_string().contains("ceiling exceeded: ticks"),
            "{error}"
        );
        let failed = host
            .checks
            .call_clocks
            .iter()
            .find(|run| !run.completed)
            .unwrap();
        assert_eq!(failed.consumed_video_frames, 0);
        assert_eq!(failed.consumed_ticks, 4);
        println!("CALL_CEILING_REJECTED {revision:?} no-frame loop: {error}");
    }
}

#[test]
#[ignore = "requires both private cartridges"]
fn individual_wait_cap_and_missing_input_are_not_relaxed() {
    fn short_wait(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        tje_game::Platform::wait_until(
            c,
            WaitRequest {
                condition: WaitCondition::RamWordChanged {
                    address: 0xff9000,
                    previous: 0,
                },
                max_ticks: 2,
            },
        )?;
        Ok(None)
    }
    struct MissingInput;
    impl tje_host::execution::WaitInputPoller for MissingInput {
        fn at(&self, frame: u64) -> tje_host::Result<genesis_config::GenesisInputs> {
            Err(HostError::Input(format!(
                "authored missing input sample at frame {frame}"
            )))
        }
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = fixture(
            revision,
            short_wait,
            CallCeiling::new(3_000_000, 200).unwrap(),
        );
        let error = host.check_fixture(CODE, &[1]).unwrap_err();
        assert!(error.to_string().contains("after 2 real ticks"), "{error}");
        assert!(!error.to_string().contains("ceiling exceeded"), "{error}");
        println!("CALL_CEILING_REJECTED {revision:?} individual wait: {error}");
        let mut host = fixture(revision, correct, CallCeiling::new(3_000_000, 200).unwrap());
        host.set_wait_input_poller(Arc::new(MissingInput));
        let error = host.check_fixture(CODE, &[3]).unwrap_err();
        assert!(
            error.to_string().contains("authored missing input sample"),
            "{error}"
        );
        println!("CALL_CEILING_REJECTED {revision:?} missing input: {error}");
    }
}

#[test]
#[ignore = "requires both private cartridges"]
fn independence_probes_use_the_identical_configured_contract() {
    fn discarded(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let address = c.cpu().stack_pointer().wrapping_sub(6);
        let _ = c.read_u16(address);
        Ok(None)
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        let ceiling = CallCeiling::new(100, 10).unwrap();
        let mut host = fixture(revision, discarded, ceiling);
        // LINK A6,#-4; MOVE.W -2(A6),D0; ANDI.L #0,D0; UNLK A6; RTS.
        let words = [0x4e56, 0xfffc, 0x302e, 0xfffe, 0x0280, 0, 0, 0x4e5e, 0x4e75];
        for (index, word) in words.into_iter().enumerate() {
            host.context().write_u16(CODE + index as u32 * 2, word);
        }
        host.check_fixture(CODE, &[0]).unwrap();
        assert!(host.checks.stack_dependency_reruns > 0);
        assert!(host.checks.call_clocks.len() > 2);
        assert!(host.checks.call_clocks.iter().all(|run| run.completed
            && run.configured_tick_ceiling == ceiling.ticks
            && run.configured_frame_ceiling == ceiling.frames));
        println!(
            "CALL_CEILING_PROBES {revision:?} {}",
            serde_json::to_string(&host.checks.call_clocks).unwrap()
        );
    }
}

#[test]
fn malformed_authored_ceiling_fails_before_execution() {
    let mut registry = HookRegistry::empty(Revision::Rev02);
    let mut symbol = ids::WAIT_FOR_NEXT_V_BLANK.metadata().clone();
    symbol.addresses = [CODE; 2];
    assert!(
        registry
            .register_clock_ceiling(
                symbol,
                CallCeiling {
                    ticks: 0,
                    frames: 10,
                    reference_instructions: None,
                },
                early,
            )
            .is_err()
    );
    assert!(registry.at(CODE).is_none());
}
