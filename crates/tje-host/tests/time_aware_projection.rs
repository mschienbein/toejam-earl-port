//! Private-cartridge integration regressions for the readable logic projection.
//! Authored text and mutation inputs only; no captured cartridge state is stored.

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tje_game::execution::ExecutionClass;
use tje_game::{Context, FunctionId, Translation, core, earthlings, ids, speech};
use tje_host::{
    Host, HostError, Mode,
    abi::HookContext,
    hooks::{HookFn, HookRegistry},
    input::InputScript,
};
use tje_mem::{Platform, Ptr, Revision, ram};

fn rom(revision: Revision) -> Vec<u8> {
    let (variable, name) = match revision {
        Revision::Rev02 => ("TJE_REV02_ROM", "rev02.bin"),
        Revision::Rev00 => ("TJE_REV00_ROM", "rev00.bin"),
    };
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../roms").join(name));
    fs::read(path).expect("private retail cartridge")
}

fn pending_registry(revision: Revision) -> HookRegistry {
    static PENDING: LazyLock<Vec<Translation>> = LazyLock::new(|| {
        core::TRANSLATIONS
            .iter()
            .chain(core::PENDING_TRANSLATIONS)
            .chain(speech::TRANSLATIONS)
            .chain(speech::PENDING_TRANSLATIONS)
            .chain(earthlings::TRANSLATIONS)
            .chain(earthlings::PENDING_TRANSLATIONS)
            .filter(|translation| {
                [
                    ids::WAIT_FOR_NEXT_V_BLANK,
                    ids::GENERATE_SPEECH,
                    ids::START_DIALOGUE,
                    ids::UPDATE_SPEECH_AND_DIALOGUE,
                    ids::PROCESS_EARTHLINGS,
                ]
                .contains(&translation.id)
            })
            .copied()
            .collect()
    });
    assert_eq!(
        PENDING.len(),
        5,
        "all named regression bodies must be available"
    );
    HookRegistry::proof(revision, &PENDING).unwrap()
}

fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| panic!("{:?} {}: {error}", host.revision(), id.metadata().name));
}

fn speech_wait_fixture(host: &mut Host) {
    let revision = host.revision();
    let mut c = host.context();
    c.write_u8(0xfff000, b'H');
    c.write_u8(0xfff001, b'i');
    c.write_u8(0xfff002, 0);
    ram::g_mode2_reg_softcopy(revision).write(&mut c, 0x40);
    ram::g_v_blank_func(revision).write(&mut c, Ptr::new(0));
    ram::g_v_blank_graphics_enabled(revision).write(&mut c, 1);
    ram::g_h_blank_graphics_enabled(revision).write(&mut c, 0);
    ram::g_v_blank_graphics_paused(revision).write(&mut c, 0);
    ram::g_v_blank_audio_input_override(revision).write(&mut c, 0);
    ram::g_inventory_redraw_phase(revision).write(&mut c, 0);
    ram::g_terrain_priority_update_pending(revision).write(&mut c, 0);
    ram::g_sprite_graphics_upload_pending(revision).write(&mut c, 1);
    ram::g_sprite_tile_buffer_offset(revision).write(&mut c, 0);
    ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut c, 0);
    let player = ram::g_player_info(revision).at(0);
    player.draw_flags().write(&mut c, 3);
    player.sprite_state().write(&mut c, 0);
    let sr = c.status_register() & !0x0700;
    c.set_status_register(sr);
    c.write_vdp_control(0x8164);
}

#[test]
#[ignore = "requires both private cartridges; named natural windows before frame 2438"]
fn named_natural_logic_windows_both_revisions() {
    let script = Arc::new(InputScript::parse(include_str!("../examples/start-game.json")).unwrap());
    for revision in [Revision::Rev02, Revision::Rev00] {
        let bytes = rom(revision);
        let mut checked = Host::new(&bytes, Mode::Check, pending_registry(revision)).unwrap();
        let mut reference =
            Host::new(&bytes, Mode::Reference, HookRegistry::empty(revision)).unwrap();
        checked.set_wait_input_poller(script.clone());
        for frame in 0..2438 {
            let a = checked
                .run_frame(script.at(frame))
                .unwrap_or_else(|error| panic!("{revision:?} frame {frame}: {error}"));
            let b = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(a.ram, b.ram, "{revision:?} frame {frame} RAM");
            assert_eq!(a.vdp, b.vdp, "{revision:?} frame {frame} VDP");
            assert_eq!(a.writes, b.writes, "{revision:?} frame {frame} chip writes");
            assert!(
                a.audio_samples
                    .iter()
                    .flatten()
                    .map(|sample| sample.to_bits())
                    .eq(b
                        .audio_samples
                        .iter()
                        .flatten()
                        .map(|sample| sample.to_bits())),
                "{revision:?} frame {frame} audio"
            );
            if matches!(frame, 492 | 2437) {
                println!(
                    "PROJECTION_BOUNDARY {revision:?} frame={frame} {}",
                    serde_json::to_string(&checked.checks.time_aware).unwrap()
                );
            }
        }
        for id in [
            ids::WAIT_FOR_NEXT_V_BLANK,
            ids::UPDATE_SPEECH_AND_DIALOGUE,
            ids::PROCESS_EARTHLINGS,
        ] {
            let summary = &checked.checks.time_aware[id.metadata().name];
            assert!(summary.calls > 0, "{} not checked", id.metadata().name);
            assert_eq!(summary.original_interrupts, summary.translated_interrupts);
            assert!(
                summary.raw_device_trace_differing_calls
                    + summary.raw_sound_video_trace_differing_calls
                    > 0,
                "{} did not exercise its raw timing difference",
                id.metadata().name
            );
        }
        assert_eq!(checked.checks.mismatches, 0);
        println!(
            "PROJECTION_NAMED_WINDOWS_PASS {revision:?} frames=2438 {}",
            serde_json::to_string(&checked.checks).unwrap()
        );
    }
}

#[test]
#[ignore = "HELD: historical Reference-only frame-10000 comparator fixture; awaiting genuine actual-call replay replacement, not passing evidence"]
fn generate_speech_wait_after_reference_10000_frames_both_revisions() {
    let script = Arc::new(InputScript::parse(include_str!("../examples/start-game.json")).unwrap());
    for revision in [Revision::Rev02, Revision::Rev00] {
        // This is an explicitly labelled original-only advance. A continuous
        // readable check separately failed closed at REV 02 frame 7353 on
        // strict VDP state; this fixture does not claim that session passed.
        let mut checked =
            Host::new(&rom(revision), Mode::Reference, pending_registry(revision)).unwrap();
        checked.set_wait_input_poller(script.clone());
        for frame in 0..10000 {
            checked.run_frame(script.at(frame)).unwrap();
        }
        assert_eq!(checked.checks.calls, 0);
        let boundary = checked
            .settle_fixture_boundary(script.at(checked.frame_count()))
            .unwrap();
        println!(
            "PROJECTION_FIXTURE_BOUNDARY {revision:?} ticks={} frames={}..{}",
            boundary.ticks, boundary.start_frame, boundary.end_frame
        );
        speech_wait_fixture(&mut checked);
        check(&mut checked, ids::GENERATE_SPEECH, &[0, 0xfff000, 1]);
        let summary = &checked.checks.time_aware[ids::GENERATE_SPEECH.metadata().name];
        assert!(
            summary.original_interrupts > 0,
            "real pending-upload ISR must run"
        );
        assert_eq!(summary.original_interrupts, summary.translated_interrupts);
        assert_eq!(checked.checks.mismatches, 0);
        println!(
            "PROJECTION_REFERENCE_ADVANCED_FIXTURE_PASS {revision:?} {}",
            serde_json::to_string(&checked.checks).unwrap()
        );
    }
}

fn wrong_wait(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    // Seed the real bug: incorrectly treat an enabled-display VBlank wait as done.
    let _ = c.argument_u32(0);
    Ok(None)
}
fn changed_isr_visible_ram(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let revision = c.revision();
    let previous = ram::g_v_blank_graphics_paused(revision).read(c);
    ram::g_v_blank_graphics_paused(revision).write(c, previous ^ 1);
    let count = c.argument_u32(0);
    core::wait_for_next_vblank(c, &[count])?;
    ram::g_v_blank_graphics_paused(revision).write(c, previous);
    Ok(None)
}

#[test]
#[ignore = "HELD: historical Reference-bootstrap mutant comparator fixtures; awaiting genuine actual-call replay negatives, not passing evidence"]
fn seeded_wrong_wait_and_isr_visible_translation_are_rejected() {
    let script = Arc::new(InputScript::parse(include_str!("../examples/start-game.json")).unwrap());
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (name, body) in [
            ("wrong wait condition", wrong_wait as HookFn),
            ("changed ISR-visible RAM", changed_isr_visible_ram),
        ] {
            let mut registry = HookRegistry::empty(revision);
            registry
                .register_execution_contract(
                    ids::WAIT_FOR_NEXT_V_BLANK.metadata().clone(),
                    ExecutionClass::TimeAware,
                    body,
                )
                .unwrap();
            let mut host = Host::new(&rom(revision), Mode::Reference, registry).unwrap();
            host.set_wait_input_poller(script.clone());
            for frame in 0..600 {
                host.run_frame(script.at(frame)).unwrap();
            }
            host.settle_fixture_boundary(script.at(host.frame_count()))
                .unwrap();
            let error = host
                .check_fixture(
                    ids::WAIT_FOR_NEXT_V_BLANK.metadata().address(revision),
                    &[1],
                )
                .unwrap_err();
            assert!(
                matches!(error, HostError::Mismatch { .. }),
                "{name}: {error}"
            );
            println!("PROJECTION_REJECTED {revision:?} {name}: {error}");
        }
    }
}

#[test]
#[ignore = "requires both private cartridges"]
fn dropped_main_write_fails_even_when_final_ram_matches() {
    fn dropped(_: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        Ok(None)
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut contract = ids::GET_RANDOM_NUMBER.metadata().clone();
        contract.name = "AuthoredDroppedMainWrite";
        contract.addresses = [0xfff100; 2];
        contract.sizes = [18; 2];
        contract.abi.argument_count = 0;
        contract.abi.returns_value = false;
        let mut registry = HookRegistry::empty(revision);
        registry
            .register_execution_contract(contract, ExecutionClass::TimeAware, dropped)
            .unwrap();
        let mut host = Host::new(&rom(revision), Mode::Check, registry).unwrap();
        // Authored ISA: MOVE.B #1,$FFF800; MOVE.B #0,$FFF800; RTS.
        let words = [0x13fc, 1, 0x00ff, 0xf800, 0x13fc, 0, 0x00ff, 0xf800, 0x4e75];
        for (index, word) in words.into_iter().enumerate() {
            host.context().write_u16(0xfff100 + index as u32 * 2, word);
        }
        host.context().write_u8(0xfff800, 0);
        let error = host.check_fixture(0xfff100, &[]).unwrap_err();
        assert!(matches!(error, HostError::Mismatch { .. }), "{error}");
        assert!(
            error.to_string().contains("TimeAware logic trace"),
            "{error}"
        );
        assert_eq!(host.context().read_u8(0xfff800), 0);
        println!(
            "PROJECTION_REJECTED {revision:?} dropped main write with matching final RAM: {error}"
        );
    }
}
