//! Authored check-only TimeAware/ClassDefined projection-regression fixtures.
//! They invoke current owner entries without changing owner envelopes or status.
//! No replay lineage, coverage, promotion or live/native eligibility is claimed.
//! Authored text/mutations only; captured cartridge state is never committed.

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tje_game::execution::ExecutionClass;
use tje_game::{Context, FunctionId, TranslatedEntry, core, earthlings, ids, speech};
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

const FIXTURE_IDS: [FunctionId; 5] = [
    ids::WAIT_FOR_NEXT_V_BLANK,
    ids::GENERATE_SPEECH,
    ids::START_DIALOGUE,
    ids::UPDATE_SPEECH_AND_DIALOGUE,
    ids::PROCESS_EARTHLINGS,
];
static FIXTURE_ENTRIES: LazyLock<[TranslatedEntry<'static>; 5]> = LazyLock::new(|| {
    FIXTURE_IDS.map(|id| {
        if id == ids::UPDATE_SPEECH_AND_DIALOGUE {
            // This current owner entry is value-form with zero arguments:
            // invoke_prepared performs no eager slot snapshot for it.
            let mut entries = tje_game::translations()
                .chain(speech::PENDING_TRANSLATIONS)
                .filter(|entry| entry.id == id);
            let entry = entries.next().expect("named zero-argument owner entry");
            assert!(entries.next().is_none(), "duplicate named fixture body");
            assert_eq!(entry.id.metadata().abi.argument_count, 0);
            TranslatedEntry::Values(entry)
        } else {
            let mut entries = tje_game::framed_translations()
                .chain(core::PENDING_FRAMED_TRANSLATIONS)
                .chain(speech::PENDING_FRAMED_TRANSLATIONS)
                .chain(earthlings::PENDING_FRAMED_TRANSLATIONS)
                .filter(|entry| entry.id == id);
            let entry = entries.next().expect("named lazy framed owner entry");
            assert!(entries.next().is_none(), "duplicate named fixture body");
            TranslatedEntry::Framed(entry)
        }
    })
});
fn invoke_projection_fixture(
    c: &mut HookContext<'_>,
    id: FunctionId,
) -> tje_host::Result<Option<u32>> {
    FIXTURE_ENTRIES
        .iter()
        .copied()
        .find(|entry| entry.id() == id)
        .expect("explicit named fixture entry")
        .invoke_prepared(c)
        .map_err(HostError::from)
}
fn wait_fixture(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    invoke_projection_fixture(c, ids::WAIT_FOR_NEXT_V_BLANK)
}
fn generate_fixture(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    invoke_projection_fixture(c, ids::GENERATE_SPEECH)
}
fn dialogue_fixture(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    invoke_projection_fixture(c, ids::START_DIALOGUE)
}
fn update_speech_fixture(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    invoke_projection_fixture(c, ids::UPDATE_SPEECH_AND_DIALOGUE)
}
fn earthlings_fixture(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    invoke_projection_fixture(c, ids::PROCESS_EARTHLINGS)
}
fn projection_fixture_registry(revision: Revision) -> HookRegistry {
    // Explicit instrumentation contracts test only the historical comparator.
    // They do not relabel the selected owner's envelope or canonical origin.
    let mut registry = HookRegistry::empty(revision);
    for (id, body) in [
        (ids::WAIT_FOR_NEXT_V_BLANK, wait_fixture as HookFn),
        (ids::GENERATE_SPEECH, generate_fixture),
        (ids::START_DIALOGUE, dialogue_fixture),
        (ids::UPDATE_SPEECH_AND_DIALOGUE, update_speech_fixture),
        (ids::PROCESS_EARTHLINGS, earthlings_fixture),
    ] {
        registry
            .register_execution_contract(id.metadata().clone(), ExecutionClass::TimeAware, body)
            .unwrap();
    }
    registry
}

#[test]
fn projection_fixture_adapters_preserve_owner_envelopes_and_refuse_live_replacement() {
    use tje_game::execution::ProofEnvelope;
    assert_eq!(
        tje_game::execution_contract(ids::WAIT_FOR_NEXT_V_BLANK)
            .unwrap()
            .proof_envelope,
        ProofEnvelope::ReplayedEnvironment
    );
    for (id, entry) in FIXTURE_IDS.into_iter().zip(FIXTURE_ENTRIES.iter()) {
        assert_eq!(entry.id(), id);
        assert!(
            matches!(
                (*entry, id == ids::UPDATE_SPEECH_AND_DIALOGUE),
                (TranslatedEntry::Values(_), true) | (TranslatedEntry::Framed(_), false)
            ),
            "projection fixtures retain the exact current named entry forms"
        );
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        let registry = projection_fixture_registry(revision);
        for id in FIXTURE_IDS {
            let hook = registry.at(id.metadata().address(revision)).unwrap();
            assert_eq!(hook.proof_envelope, ProofEnvelope::ClassDefined);
            assert_eq!(hook.execution_class, ExecutionClass::TimeAware);
        }
        let Err(error) = Host::new(&[], Mode::Replace, registry) else {
            panic!("projection-only instrumentation cannot enter live replacement")
        };
        assert!(matches!(error, HostError::Metadata(_)), "{error}");
        assert!(
            error.to_string().contains("canonical production"),
            "{error}"
        );
    }
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
fn projection_regression_named_natural_logic_windows_both_revisions() {
    let script = Arc::new(InputScript::parse(include_str!("../examples/start-game.json")).unwrap());
    for revision in [Revision::Rev02, Revision::Rev00] {
        let bytes = rom(revision);
        let mut checked =
            Host::new(&bytes, Mode::Check, projection_fixture_registry(revision)).unwrap();
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
#[ignore = "HELD: historical authored frame-10000 fixture, superseded by the genuine committed two-player frame1089-to1090 case; never an old pass or startup/composition proof"]
fn generate_speech_wait_after_reference_10000_frames_both_revisions() {
    let script = Arc::new(InputScript::parse(include_str!("../examples/start-game.json")).unwrap());
    for revision in [Revision::Rev02, Revision::Rev00] {
        // This is an explicitly labelled original-only advance. A continuous
        // readable check separately failed closed at REV 02 frame 7353 on
        // strict VDP state; this fixture does not claim that session passed.
        let mut checked = Host::new(
            &rom(revision),
            Mode::Reference,
            projection_fixture_registry(revision),
        )
        .unwrap();
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
    let _ = c
        .frame_state()
        .expect("prepared projection fixture frame")
        .argument_word(0)
        .read(c);
    Ok(None)
}
fn changed_isr_visible_ram(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let revision = c.revision();
    let previous = ram::g_v_blank_graphics_paused(revision).read(c);
    ram::g_v_blank_graphics_paused(revision).write(c, previous ^ 1);
    invoke_projection_fixture(c, ids::WAIT_FOR_NEXT_V_BLANK)?;
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
