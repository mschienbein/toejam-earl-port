//! Finite real-state audio guards, independent of the pending replay envelope.
//! These checks do not approve asynchronous success/retries or live replacement.

use std::sync::Arc;
use tje_game::{Context, FunctionId, Platform, audio, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram, rom_symbol};

const ROOTS: &[FunctionId] = &[
    ids::LOAD_SOUND_DRIVER_TO_Z80,
    ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
    ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
    ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
];

fn check(
    host: &mut Host,
    id: FunctionId,
    arguments: &[u32],
    label: &str,
    cases: &mut Vec<serde_json::Value>,
) {
    let result = host.check_fixture(id.metadata().address(host.revision()), arguments);
    cases.push(serde_json::json!({
        "function": id.metadata().name, "label": label, "arguments": arguments,
        "result": result.map(|_| "pass".to_owned()).unwrap_or_else(|error| error.to_string()),
    }));
}
fn psg_active_gate_case(host: &mut Host, script: &InputScript, cases: &mut Vec<serde_json::Value>) {
    let revision = host.revision();
    {
        let mut context = host.context();
        ram::g_pending_pcm_sample(revision).write(&mut context, 0);
        ram::g_pending_psg_effect(revision).write(&mut context, 30);
        ram::g_pcm_effect_cooldown_ticks(revision).write(&mut context, 0);
        ram::g_music_alternate_template_bank_enabled(revision).write(&mut context, 0);
    }
    // This is reference-only state preparation, not a store of a fabricated
    // hardware flag. The real Z80 DispatchPSGEffect sets active before rejecting
    // ID30, and its ORIGINAL BUG does not clear it (sound_driver.s:134-144).
    // The original controller reader mirrors that real flag into the high byte
    // of gPsgEffectStatus (controllers.s:32/60).
    let mut prepared = false;
    let start_frame = host.frame_count();
    for _ in 0..4 {
        host.run_frame(script.at(host.frame_count())).unwrap();
        let context = host.context();
        prepared = ram::g_pending_psg_effect(revision).read(&context) == -1
            && ram::g_psg_effect_status(revision).read(&context) as u16 & 0xff00 != 0;
        if prepared {
            break;
        }
    }
    assert!(prepared, "actual Z80 PSG-active state was not established");
    let boundary = host
        .settle_fixture_boundary(script.at(host.frame_count()))
        .unwrap();
    {
        let mut context = host.context();
        ram::g_pcm_effect_cooldown_ticks(revision).write(&mut context, 0);
        ram::g_music_alternate_template_bank_enabled(revision).write(&mut context, 0);
        let status = context.status_register();
        context.set_status_register(status | 0x0700);
    }
    check(
        host,
        ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
        &[0x00a0_0001],
        "immediate/actual-z80-psg-active/invalid-unused-sample",
        cases,
    );
    cases.last_mut().unwrap()["reference_preparation"] = serde_json::json!({
        "pending_psg_input":30, "no_hardware_flag_write":true,
        "start_frame":start_frame, "end_frame":host.frame_count(),
        "boundary": {
            "start_frame":boundary.start_frame,"end_frame":boundary.end_frame,
            "start_pc":boundary.start_pc,"end_pc":boundary.end_pc,
            "ticks":boundary.ticks,"master_clocks":boundary.master_clocks,
            "handlers_completed":boundary.handlers_completed,
            "completed_frames":boundary.completed_frames.iter().map(|frame|format!("{frame:#?}")).collect::<Vec<_>>(),
        },
    });
}

fn mailbox_gate_cases(host: &mut Host, script: &InputScript, cases: &mut Vec<serde_json::Value>) {
    let revision = host.revision();
    let sample = rom_symbol(revision, "Sfx_JamClap").unwrap().address;
    for (cooldown, inhibit) in [(1, 0), (-1, 0), (i16::MIN, 0), (0, 1), (0, -1)] {
        let mut context = host.context();
        ram::g_pcm_effect_cooldown_ticks(revision).write(&mut context, cooldown);
        ram::g_music_alternate_template_bank_enabled(revision).write(&mut context, inhibit);
        check(
            host,
            ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
            &[0x00a0_0001],
            &format!("immediate/gates/cooldown-{cooldown}/inhibit-{inhibit}"),
            cases,
        );
    }
    psg_active_gate_case(host, script, cases);
    // Real reset assertion denies BUSACK. It does not fabricate a grant or a lock
    // byte. Success/lock-retry paths must come from the captured natural driver
    // state and the stronger asynchronous-device admission, not injected readiness.
    {
        let mut context = host.context();
        context.write_z80_reset(0);
        ram::g_pcm_effect_cooldown_ticks(revision).write(&mut context, 0);
        ram::g_music_alternate_template_bank_enabled(revision).write(&mut context, 0);
        ram::g_pending_pcm_sample(revision).write(&mut context, sample as i32);
        ram::g_pending_psg_effect(revision).write(&mut context, 6);
    }
    assert!(host.fixture_boundary_is_settled());
    check(
        host,
        ids::LOAD_SOUND_DRIVER_TO_Z80,
        &[],
        "upload/reset-denies-grant",
        cases,
    );
    check(
        host,
        ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
        &[],
        "pcm/reset-denies-19-polls",
        cases,
    );
    check(
        host,
        ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
        &[],
        "psg/reset-denies-19-polls",
        cases,
    );
    check(
        host,
        ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
        &[0x00a0_0001],
        "immediate/reset-denies-19-polls",
        cases,
    );
}

#[test]
#[ignore = "requires both private ROMs; finite guard paths only, not asynchronous retry/publication approval"]
fn replay_preparation_low_level_real_reset_and_gate_states_both_revisions() {
    let script = Arc::new(
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap(),
    );
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = match revision {
            Revision::Rev02 => "TJE_REV02_ROM",
            Revision::Rev00 => "TJE_REV00_ROM",
        };
        let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
        let registry = HookRegistry::proof_filtered_with_frames(
            revision,
            audio::PENDING_TRANSLATIONS,
            audio::PENDING_FRAMED_TRANSLATIONS,
            ROOTS,
        )
        .unwrap();
        let mut host = Host::new(&bytes, Mode::Reference, registry).unwrap();
        host.set_wait_input_poller(script.clone());
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        let boundary = host
            .settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        let mut cases = Vec::new();
        mailbox_gate_cases(&mut host, &script, &mut cases);
        let evidence = serde_json::json!({
            "revision": revision.name(), "proof_envelope": "ClassDefined guard diagnostics only",
            "live_replacement_eligible": false, "reference_boot_frames":1300, "frames":host.frame_count(),
            "fixtures": cases, "checks": host.checks,
            "not_covered": "asynchronous success/lock-retry/publication and standalone retry-fragment entry",
            "boundary": {
                "start_frame": boundary.start_frame, "end_frame": boundary.end_frame,
                "start_pc": boundary.start_pc, "end_pc": boundary.end_pc,
                "ticks": boundary.ticks, "master_clocks": boundary.master_clocks,
                "handlers_completed": boundary.handlers_completed,
                "completed_frames": boundary.completed_frames.iter().map(|frame| format!("{frame:#?}")).collect::<Vec<_>>(),
            },
        });
        if let Some(directory) = std::env::var_os("TJE_AUDIO_REPLAY_METRICS_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                std::path::Path::new(&directory)
                    .join(format!("{}-bus-guard-fixtures.json", revision.name())),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
        }
        println!("AUDIO_BUS_GUARD_FIXTURES {evidence}");
        assert!(
            cases.iter().all(|case| case["result"] == "pass"),
            "{cases:?}"
        );
        assert_eq!(host.checks.mismatches, 0);
    }
}
