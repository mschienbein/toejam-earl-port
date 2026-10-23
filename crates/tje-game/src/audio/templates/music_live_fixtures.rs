//! Opt-in retail differential proof, built by the external lane harness.
//! Streams below are authored opcode fixtures, never copied song/ROM bytes.

use std::{
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tje_game::{Context, FunctionId, Translation, audio, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Address, Revision, ram, rom_data_units, rom_symbol};

const TARGETS: [FunctionId; 4] = [
    ids::MUSIC_LOAD_SONG_CHANNEL_TEMPLATES,
    ids::MUSIC_SELECT_SONG_TEMPLATE_BANK,
    ids::MUSIC_PROCESS_TEMPLATE_RESTART_HOOK,
    ids::MUSIC_UPDATE_SEQUENCER,
];
// Fixture bytecode lives above the stack region, not in dead stack storage.
const STREAM: u32 = 0xfff000;

fn translations() -> &'static [Translation] {
    // Keep pending overrides composed with all proven callees. The loader is
    // also a checked entry, not merely an uncounted nested Rust call.
    static ENTRIES: LazyLock<Vec<Translation>> = LazyLock::new(|| {
        TARGETS
            .into_iter()
            .map(|id| {
                *audio::PENDING_TRANSLATIONS
                    .iter()
                    .chain(audio::TRANSLATIONS.iter())
                    .find(|entry| entry.id == id)
                    .unwrap()
            })
            .collect()
    });
    &ENTRIES
}

fn schedule_translations() -> &'static [Translation] {
    static ENTRIES: LazyLock<Vec<Translation>> = LazyLock::new(|| {
        translations()
            .iter()
            .copied()
            .filter(|entry| entry.id != ids::MUSIC_UPDATE_SEQUENCER)
            .collect()
    });
    &ENTRIES
}

#[test]
fn native_schedule_closes_and_sequencer_reports_mailbox_dependency_closure() {
    assert!(
        tje_game::Dispatcher::native_with(
            &[
                ids::MUSIC_SELECT_SONG_TEMPLATE_BANK,
                ids::MUSIC_PROCESS_TEMPLATE_RESTART_HOOK
            ],
            schedule_translations(),
        )
        .is_ok()
    );
    let mut expected = vec![
        ids::AUDIO_RETRY_PENDING_PCM_MAILBOX,
        ids::AUDIO_RETRY_PENDING_PSG_MAILBOX,
        ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
        ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
        ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
    ];
    expected.sort_unstable();
    let missing = tje_game::Dispatcher::native_with(&[ids::MUSIC_UPDATE_SEQUENCER], translations())
        .err()
        .expect("the mailbox owners and their retry fragments intentionally remain original");
    assert_eq!(missing, expected);
}

fn bytes(revision: Revision) -> Vec<u8> {
    let key = match revision {
        Revision::Rev02 => "TJE_REV02_ROM",
        Revision::Rev00 => "TJE_REV00_ROM",
    };
    std::fs::read(std::env::var_os(key).expect(key)).unwrap()
}

fn revision_key(revision: Revision) -> &'static str {
    match revision {
        Revision::Rev02 => "rev02",
        Revision::Rev00 => "rev00",
    }
}

fn boot(revision: Revision) -> Host {
    let registry = HookRegistry::proof(revision, translations()).unwrap();
    let mut host = Host::new(&bytes(revision), Mode::Reference, registry).unwrap();
    let script = InputScript::parse(include_str!(
        "../../../../tje-host/examples/start-game.json"
    ))
    .unwrap();
    for frame in 0..1300 {
        host.run_frame(script.at(frame)).unwrap();
    }
    assert!(host.measurements.game_frame.is_some());
    host
}

fn record(host: &Host, name: &str, cases: &[serde_json::Value]) {
    let metrics = serde_json::json!({
        "revision": revision_key(host.revision()), "name": name,
        "frames": host.frame_count(), "measurements": {
            "title_frame": host.measurements.title_frame,
            "menu_frame": host.measurements.menu_frame,
            "game_frame": host.measurements.game_frame,
            "check_original_wait_frames": host.measurements.check_original_wait_frames,
            "check_rust_wait_frames": host.measurements.check_rust_wait_frames,
        },
        "checks": host.checks, "cases": cases,
    });
    if let Some(directory) = std::env::var_os("TJE_MUSIC_METRICS_DIR") {
        std::fs::write(
            PathBuf::from(directory).join(format!("{}-{name}.json", revision_key(host.revision()))),
            serde_json::to_vec_pretty(&metrics).unwrap(),
        )
        .unwrap();
    }
    println!(
        "MUSIC_PROOF {} {name} checked={} mismatches={}",
        revision_key(host.revision()),
        host.checks.calls,
        host.checks.mismatches
    );
}

fn check(
    host: &mut Host,
    id: FunctionId,
    arguments: &[u32],
    label: &str,
    cases: &mut Vec<serde_json::Value>,
) {
    let result = host.check_fixture(id.metadata().address(host.revision()), arguments);
    cases.push(
        serde_json::json!({"label": label, "function": id.metadata().name,
        "result": result.map(|_| "pass".to_owned()).unwrap_or_else(|error| error.to_string())}),
    );
}

fn seed(host: &mut Host, flags: u16) -> Address<u8> {
    let revision = host.revision();
    let mut context = host.context();
    for index in 0..8 {
        context.set_data_register(index, 0xa1b2_c300 | index as u32);
    }
    for index in 0..7 {
        context.set_address_register(index, 0xb2c3_d400 | index as u32);
    }
    let status = context.status_register();
    context.set_status_register((status & !0x1f) | flags);
    ram::g_fm_channel_release_requested(revision).write(&mut context, 0);
    ram::g_music_track_reset_requested(revision).write(&mut context, 0);
    ram::g_pending_fm_effect(revision).write(&mut context, -1);
    ram::g_pending_pcm_sample(revision).write(&mut context, 0);
    ram::g_pending_psg_effect(revision).write(&mut context, -1);
    ram::g_psg_effect_status(revision).write(&mut context, 0);
    ram::g_pcm_effect_cooldown_ticks(revision).write(&mut context, 0);
    ram::g_pcm_blocks_sequencer(revision).write(&mut context, 0);
    ram::g_music_alternate_template_bank_enabled(revision).write(&mut context, 0);
    ram::g_boom_box_music_template_hold(revision).write(&mut context, 0);
    ram::g_music_template_restart_state(revision).write(&mut context, 2);
    ram::g_current_music_selection(revision).write(&mut context, 0);
    ram::g_music_channel_scan_count_minus_one(revision)
        .cast::<u16>()
        .write(&mut context, 0);
    for index in 0..6 {
        ram::g_music_fm_channel_owners(revision)
            .at(0)
            .cast::<u32>()
            .offset(index)
            .write(&mut context, 0);
    }
    let channel = ram::g_music_channels(revision).cast::<u8>();
    for offset in 0..9 * 38 {
        channel.byte_offset(offset).write(&mut context, 0);
    }
    for offset in 0..96 {
        context.write_u8(STREAM + offset, 0);
    }
    channel
        .byte_offset(4)
        .cast::<u32>()
        .write(&mut context, STREAM);
    channel.byte_offset(14).write(&mut context, 0);
    context.set_address_register(0, channel.address());
    channel
}

fn stream(host: &mut Host, bytes: &[u8]) {
    let mut context = host.context();
    for (offset, &byte) in bytes.iter().enumerate() {
        context.write_u8(STREAM + offset as u32, byte);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn song_selection_and_restart_branch_fixtures_both_revisions() {
    let mut failures = Vec::new();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = boot(revision);
        let mut cases = Vec::new();
        let template_count = rom_data_units(revision)
            .iter()
            .find(|unit| unit.name == "SongChannelTemplates")
            .unwrap()
            .size
            / 228;
        for selection in 0..template_count {
            for flags in [0, 0x1f] {
                seed(&mut host, flags);
                check(
                    &mut host,
                    ids::MUSIC_LOAD_SONG_CHANNEL_TEMPLATES,
                    &[0xbeef_0000 | selection],
                    &format!("loader/template-{selection}/ccr-{flags:02x}"),
                    &mut cases,
                );
            }
        }
        for alternate in [0, 1, -1] {
            for selection in 0..7 {
                for flags in [0, 0x1f] {
                    seed(&mut host, flags);
                    ram::g_music_alternate_template_bank_enabled(revision)
                        .write(&mut host.context(), alternate);
                    check(
                        &mut host,
                        ids::MUSIC_SELECT_SONG_TEMPLATE_BANK,
                        &[0xbeef_0000 | selection],
                        &format!("select/song-{selection}/bank-{alternate}/ccr-{flags:02x}"),
                        &mut cases,
                    );
                }
            }
        }
        // The +7 is a word update, with wrap; the upper half of the slot stays.
        seed(&mut host, 0x1f);
        ram::g_music_alternate_template_bank_enabled(revision).write(&mut host.context(), 1);
        check(
            &mut host,
            ids::MUSIC_SELECT_SONG_TEMPLATE_BANK,
            &[0xbeef_fff9],
            "select/low-word-wrap-to-zero",
            &mut cases,
        );
        for state in [i16::MIN, -3, -2, -1, 0, 1, 2, 3, i16::MAX] {
            for alternate in [0, 1, -1] {
                for hold in [0, 1, -1] {
                    for selection in [0, 6] {
                        let channel = seed(&mut host, 0x1f);
                        let mut context = host.context();
                        ram::g_current_music_selection(revision).write(&mut context, selection);
                        ram::g_music_template_restart_state(revision).write(&mut context, state);
                        ram::g_music_alternate_template_bank_enabled(revision)
                            .write(&mut context, alternate);
                        ram::g_boom_box_music_template_hold(revision).write(&mut context, hold);
                        channel
                            .byte_offset(8)
                            .cast::<u32>()
                            .write(&mut context, 0x1234_5678);
                        check(
                            &mut host,
                            ids::MUSIC_PROCESS_TEMPLATE_RESTART_HOOK,
                            &[],
                            &format!(
                                "restart/state-{state}/bank-{alternate}/hold-{hold}/song-{selection}"
                            ),
                            &mut cases,
                        );
                    }
                }
            }
        }
        record(&host, "schedule-fixtures", &cases);
        failures.extend(cases.into_iter().filter(|case| case["result"] != "pass"));
    }
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn every_sequencer_opcode_and_counter_flag_edges_both_revisions() {
    let mut failures = Vec::new();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = boot(revision);
        let mut cases = Vec::new();
        for flags in [0, 0x1f] {
            for opcode in 0..=12 {
                for owned in [false, true] {
                    let channel = seed(&mut host, flags);
                    if !owned {
                        let mut context = host.context();
                        let other = channel.byte_offset(38);
                        other.byte_offset(13).write(&mut context, 127);
                        ram::g_music_fm_channel_owners(revision)
                            .at(0)
                            .cast::<u32>()
                            .write(&mut context, other.address());
                    }
                    let program: Vec<u8> = match opcode {
                        1 | 3 => vec![opcode, 0x12, 0x34, 0x81],
                        4 => vec![opcode, 0, 0x81],
                        5 => vec![opcode, 0, 3, 0x81],
                        6 => vec![opcode, 0, 0, 0x81, 0, 0, 0x81],
                        7 => vec![opcode, 0, 0, 0, 0, 0, 0x81],
                        8 => vec![opcode, 0, 1, 0, 0x81],
                        _ => vec![opcode, 0x81],
                    };
                    stream(&mut host, &program);
                    check(
                        &mut host,
                        ids::MUSIC_UPDATE_SEQUENCER,
                        &[],
                        &format!("opcode-{opcode}/owned-{owned}/ccr-{flags:02x}"),
                        &mut cases,
                    );
                }
            }
        }
        for opcode in 9..=12 {
            for (gate, cooldown, alternate) in [("cooldown", 2, 0), ("alternate-bank", 0, 1)] {
                seed(&mut host, 0x1f);
                let mut context = host.context();
                ram::g_pcm_effect_cooldown_ticks(revision).write(&mut context, cooldown);
                ram::g_music_alternate_template_bank_enabled(revision)
                    .write(&mut context, alternate);
                stream(&mut host, &[opcode, 0x81]);
                check(
                    &mut host,
                    ids::MUSIC_UPDATE_SEQUENCER,
                    &[],
                    &format!("opcode-{opcode}/pcm-gate-{gate}"),
                    &mut cases,
                );
            }
        }
        for start in [0_u32, 1] {
            let channel = seed(&mut host, 0x1f);
            let callback = ids::MUSIC_PROCESS_TEMPLATE_RESTART_HOOK
                .metadata()
                .address(revision);
            let mut context = host.context();
            channel
                .byte_offset(4)
                .cast::<u32>()
                .write(&mut context, STREAM + start);
            context.write_u8(STREAM + start, 7);
            // Opcode 7 aligns the address after the opcode, not the opcode.
            context.write_u32(STREAM + 2, callback);
            context.write_u8(STREAM + 6, 0x81);
            check(
                &mut host,
                ids::MUSIC_UPDATE_SEQUENCER,
                &[],
                &format!("callback-load/opcode-address-parity-{start}"),
                &mut cases,
            );
        }
        for opcode in [6, 8] {
            let channel = seed(&mut host, 0x1f);
            let mut context = host.context();
            channel
                .byte_offset(4)
                .cast::<u32>()
                .write(&mut context, STREAM + 4);
            channel.byte_offset(20).write(&mut context, 1);
            channel.byte_offset(22).write(&mut context, 0);
            channel.byte_offset(23).write(&mut context, 2);
            let displacement = if opcode == 6 { 0xf6 } else { 0xf9 };
            stream(&mut host, &[0x81, 0, 0, 0, opcode, 0xff, displacement]);
            check(
                &mut host,
                ids::MUSIC_UPDATE_SEQUENCER,
                &[],
                &format!("relative-branch/opcode-{opcode}/negative-displacement"),
                &mut cases,
            );
        }
        for token in [0x80, 0x81, 0xff] {
            for old_wait in [0, 1, 2, 127, 128, 255] {
                seed(&mut host, 0x1f)
                    .byte_offset(15)
                    .write(&mut host.context(), old_wait);
                stream(&mut host, &[token]);
                check(
                    &mut host,
                    ids::MUSIC_UPDATE_SEQUENCER,
                    &[],
                    &format!("wait/token-{token:02x}/old-{old_wait}"),
                    &mut cases,
                );
            }
        }
        for opcode in [13, 14, 63, 127] {
            seed(&mut host, 0x1f);
            stream(&mut host, &[opcode, 0x81]);
            check(
                &mut host,
                ids::MUSIC_UPDATE_SEQUENCER,
                &[],
                &format!("unknown-positive-opcode-{opcode}"),
                &mut cases,
            );
        }
        for count in [
            i16::MIN,
            i16::MIN + 1,
            -2,
            -1,
            0,
            1,
            2,
            3,
            255,
            256,
            i16::MAX,
        ] {
            for depth in [1_u8, 2, 7, 127, 128, 255] {
                let channel = seed(&mut host, 0x1f);
                let mut context = host.context();
                channel.byte_offset(20).write(&mut context, depth);
                let offset = i32::from((i16::from(depth as i8)).wrapping_mul(2));
                channel
                    .byte_offset(20 + offset)
                    .write(&mut context, (count as u16 >> 8) as u8);
                channel
                    .byte_offset(21 + offset)
                    .write(&mut context, count as u8);
                // Positive branch skips three padding bytes; exit reads the
                // first wait. Both outcomes terminate without fake VM bounds.
                stream(&mut host, &[6, 0, 0, 0x81, 0, 0, 0x81]);
                check(
                    &mut host,
                    ids::MUSIC_UPDATE_SEQUENCER,
                    &[],
                    &format!("loop-branch/count-{count}/depth-{depth}"),
                    &mut cases,
                );
            }
        }
        for depth in [0_u8, 1, 7, 127, 128, 255] {
            seed(&mut host, 0x1f)
                .byte_offset(20)
                .write(&mut host.context(), depth);
            stream(&mut host, &[5, 0x80, 0x01, 0x81]);
            check(
                &mut host,
                ids::MUSIC_UPDATE_SEQUENCER,
                &[],
                &format!("loop-push/depth-{depth}"),
                &mut cases,
            );
        }
        for status in [i16::MIN, -1, 0, 1, 2, i16::MAX] {
            for cooldown in [i16::MIN, -1, 0, 1, 2, i16::MAX] {
                for blocks in [0, 1, -1] {
                    seed(&mut host, 0x1f);
                    let mut context = host.context();
                    ram::g_psg_effect_status(revision).write(&mut context, status);
                    ram::g_pcm_effect_cooldown_ticks(revision).write(&mut context, cooldown);
                    ram::g_pcm_blocks_sequencer(revision).write(&mut context, blocks);
                    stream(&mut host, &[0x81]);
                    check(
                        &mut host,
                        ids::MUSIC_UPDATE_SEQUENCER,
                        &[],
                        &format!("timers/psg-{status}/pcm-{cooldown}/blocks-{blocks}"),
                        &mut cases,
                    );
                }
            }
        }
        for scan in [0_u16, 5, 8] {
            let channel = seed(&mut host, 0x1f);
            let mut context = host.context();
            ram::g_music_channel_scan_count_minus_one(revision)
                .cast::<u16>()
                .write(&mut context, scan);
            channel.byte_offset(4).cast::<u32>().write(&mut context, 0);
            let last = channel.byte_offset(i32::from(scan) * 38);
            last.byte_offset(4)
                .cast::<u32>()
                .write(&mut context, STREAM);
            last.byte_offset(12).write(&mut context, 5);
            stream(&mut host, &[0x81]);
            check(
                &mut host,
                ids::MUSIC_UPDATE_SEQUENCER,
                &[],
                &format!("inclusive-scan/{scan}-plus-one"),
                &mut cases,
            );
        }
        for left in [i8::MIN, -1, 0, 1, i8::MAX] {
            for right in [i8::MIN, -1, 0, 1, i8::MAX] {
                let channel = seed(&mut host, 0x1f);
                let mut context = host.context();
                ram::g_music_channel_scan_count_minus_one(revision)
                    .cast::<u16>()
                    .write(&mut context, 1);
                channel.byte_offset(13).write(&mut context, left as u8);
                let other = channel.byte_offset(38);
                other
                    .byte_offset(4)
                    .cast::<u32>()
                    .write(&mut context, STREAM);
                other.byte_offset(13).write(&mut context, right as u8);
                stream(&mut host, &[0x81]);
                check(
                    &mut host,
                    ids::MUSIC_UPDATE_SEQUENCER,
                    &[],
                    &format!("owner-priority/{left}-versus-{right}"),
                    &mut cases,
                );
            }
        }
        for state in [-1, 1, 2] {
            for hold in [0, 1] {
                let channel = seed(&mut host, 0x1f);
                let callback = ids::MUSIC_PROCESS_TEMPLATE_RESTART_HOOK
                    .metadata()
                    .address(revision);
                let mut context = host.context();
                channel
                    .byte_offset(8)
                    .cast::<u32>()
                    .write(&mut context, callback);
                ram::g_music_template_restart_state(revision).write(&mut context, state);
                ram::g_boom_box_music_template_hold(revision).write(&mut context, hold);
                stream(&mut host, &[0x81]);
                check(
                    &mut host,
                    ids::MUSIC_UPDATE_SEQUENCER,
                    &[],
                    &format!("callback/restart-{state}/hold-{hold}"),
                    &mut cases,
                );
            }
        }
        for request in [
            "release",
            "reset",
            "fm-clear",
            "pcm",
            "psg",
            "pcm-before-psg",
        ] {
            seed(&mut host, 0x1f);
            stream(&mut host, &[0x81]);
            let mut context = host.context();
            match request {
                "release" => ram::g_fm_channel_release_requested(revision).write(&mut context, 1),
                "reset" => ram::g_music_track_reset_requested(revision).write(&mut context, 1),
                "fm-clear" => ram::g_pending_fm_effect(revision).write(&mut context, 0),
                "pcm" | "pcm-before-psg" => {
                    let sample = rom_symbol(revision, "Sfx_JamClap").unwrap().address;
                    ram::g_pending_pcm_sample(revision).write(&mut context, sample as i32);
                    if request == "pcm-before-psg" {
                        ram::g_pending_psg_effect(revision).write(&mut context, 0);
                    }
                }
                _ => ram::g_pending_psg_effect(revision).write(&mut context, 0),
            }
            check(
                &mut host,
                ids::MUSIC_UPDATE_SEQUENCER,
                &[],
                &format!("request/{request}"),
                &mut cases,
            );
        }
        record(&host, "vm-fixtures", &cases);
        failures.extend(cases.into_iter().filter(|case| case["result"] != "pass"));
    }
    assert!(failures.is_empty(), "{failures:?}");
}

fn session(name: &str, revision: Revision) -> (Arc<InputScript>, u64) {
    let directory =
        PathBuf::from(std::env::var_os("TJE_MUSIC_SESSION_DIR").expect("TJE_MUSIC_SESSION_DIR"));
    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.join(format!("{}-{name}.tje.json", revision_key(revision))))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        value["start"]["kind"], "power_on",
        "natural sessions must not mutate RAM"
    );
    let buttons = ["up", "down", "left", "right", "a", "b", "c", "start"];
    let mut holds = Vec::new();
    let mut start = 0;
    let mut p2_connected = None;
    for run in value["runs"].as_array().unwrap() {
        let frames = run["frames"].as_u64().unwrap();
        let pads = run["pads"].as_array().unwrap();
        let pads = [
            pads[0].as_u64().unwrap() as u16,
            pads[1].as_u64().unwrap() as u16,
        ];
        assert_ne!(pads[0] & 0x100, 0);
        let connected = pads[1] & 0x100 != 0;
        assert_eq!(*p2_connected.get_or_insert(connected), connected);
        let names = pads.map(|pad| {
            buttons
                .iter()
                .enumerate()
                .filter(|(bit, _)| pad & (1 << bit) != 0)
                .map(|(_, &name)| name)
                .collect::<Vec<_>>()
        });
        holds.push(serde_json::json!({"start": start, "end": start + frames, "p1": names[0], "p2": names[1]}));
        start += frames;
    }
    assert_eq!(value["frames"].as_u64().unwrap(), start);
    let script = serde_json::json!({"p2_connected": p2_connected.unwrap_or(false), "holds": holds});
    (
        Arc::new(InputScript::parse(&script.to_string()).unwrap()),
        start,
    )
}

fn natural_sessions(include_vm: bool) {
    let mut failures = Vec::new();
    for revision in [Revision::Rev02, Revision::Rev00] {
        for name in ["one-player", "jam-all", "secret-ending"] {
            let (script, frames) = session(name, revision);
            let entries = if include_vm {
                translations()
            } else {
                schedule_translations()
            };
            let registry = HookRegistry::proof(revision, entries).unwrap();
            let mut host = Host::new(&bytes(revision), Mode::Check, registry).unwrap();
            host.set_wait_input_poller(script.clone());
            let milestones = [
                "TitleScreenVideo",
                "InGameLoop",
                "StartJamOut",
                "PlayEndingSequence",
                "EndingPlayScrollingCredits",
                "MapEnterFinalLevel",
            ];
            let addresses = milestones.map(|name| {
                if name == "InGameLoop" {
                    // This is an interior startup label, not a function entry.
                    tje_host::catalogue::RevisionSymbols::for_revision(revision).main_loop
                } else {
                    tje_game::functions()
                        .find(|function| function.name == name)
                        .expect("scene milestone is catalogued")
                        .address(revision)
                }
            });
            host.enable_session_trace(&addresses, &[], &[], false)
                .unwrap();
            let mut cases = Vec::new();
            let mut songs = std::collections::BTreeSet::new();
            while host.frame_count() < frames {
                let frame = host.frame_count();
                if let Err(error) = host.run_frame(script.at(frame)) {
                    cases.push(serde_json::json!({"label": format!("natural/{name}/frame-{frame}"), "result": error.to_string()}));
                    break;
                }
                songs.insert(ram::g_current_music_selection(revision).read(&host.context()));
            }
            cases.push(serde_json::json!({"label": "observed-music-selections", "songs": songs}));
            let milestone_counts = milestones
                .into_iter()
                .zip(host.session_entry_counts().unwrap())
                .map(|(name, &count)| (name, count))
                .collect::<std::collections::BTreeMap<_, _>>();
            cases.push(serde_json::json!({"label": "natural-scene-entry-counts", "counts": milestone_counts}));
            let prefix = if include_vm { "sequencer" } else { "schedule" };
            record(&host, &format!("{prefix}-{name}"), &cases);
            if host.frame_count() != frames || host.checks.mismatches != 0 {
                failures.push(format!("{revision:?} {name}: {cases:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
#[ignore = "requires both private retail ROMs and recorded natural sessions"]
fn natural_schedule_title_levels_jam_out_and_ending_both_revisions() {
    natural_sessions(false);
}

#[test]
#[ignore = "requires both private retail ROMs; sequencer remains pending on exact timed device traces"]
fn natural_sequencer_title_levels_jam_out_and_ending_both_revisions() {
    natural_sessions(true);
}
