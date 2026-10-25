//! One opt-in audio campaign plus labelled real-state fixtures.
//! ReplayedEnvironment is semantic evidence only, never live replacement.
//! The bus/reset/open-bus paths still need the tier's explicit admission review.

#[path = "templates/music_live_fixtures.rs"]
mod music;

use tje_game::{FunctionId, audio, ids};
use tje_host::{CheckFailurePolicy, Host, Mode, hooks::HookRegistry};
use tje_mem::{Revision, ram};

// Retry fragments remain real fallthroughs inside their owner's captured path.
// Registering one as a fabricated ordinary RTS fixture would change its ABI.
const STRONGER_ROOTS: &[FunctionId] = &[
    ids::LOAD_SOUND_DRIVER_TO_Z80,
    ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
    ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
    ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
];
// The modular tier proves the VM's own RAM instructions and authenticates each
// original callee segment's consumed inputs; it is not a composition proof.
const REPLAY_ROOTS: &[FunctionId] = &[ids::MUSIC_UPDATE_SEQUENCER];
const RETRY_ROLES: &[FunctionId] = &[
    ids::AUDIO_RETRY_PENDING_PCM_MAILBOX,
    ids::AUDIO_RETRY_PENDING_PSG_MAILBOX,
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

fn vm_cases(host: &mut Host, cases: &mut Vec<serde_json::Value>) {
    let revision = host.revision();
    for flags in [0, 0x1f] {
        for opcode in 0..=12 {
            for owned in [false, true] {
                let channel = music::seed(host, flags);
                if !owned {
                    let mut context = host.context();
                    let other = channel.byte_offset(38);
                    other.byte_offset(13).write(&mut context, 127);
                    ram::g_music_fm_channel_owners(revision)
                        .at(0)
                        .cast::<u32>()
                        .write(&mut context, other.address());
                }
                let program = match opcode {
                    1 | 3 => vec![opcode, 0x12, 0x34, 0x81],
                    4 => vec![opcode, 0, 0x81],
                    5 => vec![opcode, 0, 3, 0x81],
                    6 => vec![opcode, 0, 0, 0x81, 0, 0, 0x81],
                    7 => vec![opcode, 0, 0, 0, 0, 0, 0x81],
                    8 => vec![opcode, 0, 1, 0, 0x81],
                    _ => vec![opcode, 0x81],
                };
                music::stream(host, &program);
                check(
                    host,
                    ids::MUSIC_UPDATE_SEQUENCER,
                    &[],
                    &format!("vm/opcode-{opcode}/owned-{owned}/ccr-{flags:02x}"),
                    cases,
                );
            }
        }
    }
    for status in [i16::MIN, -1, 0, 1, 2, i16::MAX] {
        for cooldown in [i16::MIN, -1, 0, 1, 2, i16::MAX] {
            for blocking in [0, 1, -1] {
                music::seed(host, 0x1f);
                let mut context = host.context();
                ram::g_psg_effect_status(revision).write(&mut context, status);
                ram::g_pcm_effect_cooldown_ticks(revision).write(&mut context, cooldown);
                ram::g_pcm_blocks_sequencer(revision).write(&mut context, blocking);
                music::stream(host, &[0x81]);
                check(
                    host,
                    ids::MUSIC_UPDATE_SEQUENCER,
                    &[],
                    &format!("vm/timers/psg-{status}/pcm-{cooldown}/blocks-{blocking}"),
                    cases,
                );
            }
        }
    }
}

#[test]
#[ignore = "requires both private ROMs and the landed, reviewed ReplayedEnvironment envelope"]
fn replay_preparation_single_audio_campaign_and_live_fixtures_both_revisions() {
    let mut failures = Vec::new();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = match revision {
            Revision::Rev02 => "TJE_REV02_ROM",
            Revision::Rev00 => "TJE_REV00_ROM",
        };
        let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
        let registry = HookRegistry::proof_replayed_environment(
            revision,
            audio::PENDING_TRANSLATIONS,
            audio::PENDING_FRAMED_TRANSLATIONS,
            REPLAY_ROOTS,
        )
        .unwrap();
        let mut host = Host::new(&bytes, Mode::Check, registry).unwrap();
        host.set_check_failure_policy(CheckFailurePolicy::ContinueOriginal)
            .unwrap();
        let (script, frames) = music::session("jam-all", revision);
        host.set_wait_input_poller(script.clone());
        for frame in 0..frames {
            host.run_frame(script.at(frame)).unwrap();
        }
        let natural = serde_json::to_value(&host.checks).unwrap();
        let boundary = host
            .settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        let mut cases = Vec::new();
        vm_cases(&mut host, &mut cases);
        failures.extend(
            cases
                .iter()
                .filter(|case| case["result"] != "pass")
                .cloned(),
        );
        let evidence = serde_json::json!({
            "revision": revision.name(), "proof_envelope": "ReplayedEnvironment",
            "live_replacement_eligible": false, "single_natural_session": "jam-all",
            "frames": frames, "natural": natural, "fixtures": cases,
            "with_fixtures": host.checks,
            "modular_callee_policy": "authenticated actual original callee segments; not Rust composition evidence",
            "retry_role_scope": "not proved by the VM; owner/fragment proof needs stronger admission and no fabricated RTS",
            "retry_roles": RETRY_ROLES.iter().map(|id| id.metadata().name).collect::<Vec<_>>(),
            "excluded_direct_bus_roots": STRONGER_ROOTS.iter().map(|id| id.metadata().name).collect::<Vec<_>>(),
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
                    .join(format!("{}-replay-prep.json", revision.name())),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
        }
        println!("AUDIO_REPLAY_PREPARATION {evidence}");
        if host.checks.mismatches != 0 {
            failures.push(serde_json::json!({"revision": revision.name(), "mismatches": host.checks.mismatches}));
        }
        for id in REPLAY_ROOTS {
            if host.checks.by_function[id.metadata().name].calls_checked == 0 {
                failures.push(serde_json::json!({"revision": revision.name(), "unexercised": id.metadata().name}));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
