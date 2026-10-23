//! Opt-in host harness: use this file as an external Cargo [[test]] target with
//! path dependencies on tje-game, tje-host, tje-mem and serde_json. This avoids a
//! game-to-host dependency cycle. TJE_REV02_ROM/TJE_REV00_ROM select private ROMs;
//! TJE_AUDIO_METRICS_DIR optionally receives textual coverage, never game data.
//! The standard 10,000-frame session uses only the audio entry-hook filter, and
//! every added synthetic input is separately labeled as a live-state fixture.

use std::sync::Arc;
use tje_game::{Context, FunctionId, audio, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Ptr, Revision, ram};

fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) -> u32 {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| {
            panic!(
                "live-state fixture {:?} {} {arguments:08x?}: {error}",
                host.revision(),
                id.metadata().name
            )
        })
}

fn drive_live_fixtures(host: &mut Host) {
    let revision = host.revision();
    for flags in [0, 0x1f] {
        let mut context = host.context();
        let status = context.status_register();
        context.set_status_register((status & !0x1f) | flags);
        context.set_data_register(0, 0xaabb_ccdd);
        context.set_address_register(0, 0x1234_5678);
        context.set_address_register(1, 0x8765_4321);
        assert_eq!(check(host, ids::MUSIC_RESTORE_INITIAL_STATE, &[]), 0xffff);
    }
    for flags in [0, 0x1f] {
        let mut context = host.context();
        let status = context.status_register();
        context.set_status_register((status & !0x1f) | flags);
        context.set_data_register(0, 0xdead_beef);
        for index in 0..6 {
            ram::g_music_fm_channel_owners(revision)
                .at(0)
                .cast::<u32>()
                .offset(index)
                .write(&mut context, 0x1234_0000 | index as u32);
            ram::g_music_channels(revision)
                .at(index)
                .current_stream()
                .write(&mut context, Ptr::new(0x5678_0000 | index as u32));
        }
        ram::g_music_track_reset_requested(revision).write(&mut context, -1);
        assert_eq!(check(host, ids::MUSIC_CLEAR_TRACK_POINTERS, &[]), 0xffff);
    }
    for value in [i16::MIN, 0, i16::MAX] {
        ram::g_music_track_reset_requested(revision).write(&mut host.context(), value);
        check(host, ids::MUSIC_REQUEST_TRACK_POINTER_RESET, &[]);
        ram::g_fm_channel_release_requested(revision).write(&mut host.context(), value);
        check(host, ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, &[]);
    }
    for raw in [0, 1, 0x7fff, 0x8000, 0xffff, 0xaaaa_5555, 0x1234_8000] {
        check(host, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[raw]);
    }
    for cooldown in [i16::MIN, 0, 1] {
        for psg in [-1_i16, 0, i16::MAX] {
            for old_priority in [i16::MIN, -1, 0, 1, i16::MAX] {
                for priority in [i16::MIN, -1, 0, 1, i16::MAX] {
                    let mut context = host.context();
                    ram::g_pcm_effect_cooldown_ticks(revision).write(&mut context, cooldown);
                    ram::g_psg_effect_status(revision).write(&mut context, psg);
                    ram::g_pending_psg_effect(revision).write(&mut context, -1);
                    ram::g_sound_effect_priority(revision).write(&mut context, old_priority);
                    let sound = priority.wrapping_add(0x1234);
                    check(
                        host,
                        ids::PLAY_SOUND,
                        &[
                            0xdead_0000 | u32::from(sound as u16),
                            0xbeef_0000 | u32::from(priority as u16),
                        ],
                    );
                }
            }
        }
    }
    for cooldown in [i16::MIN, 0, 1] {
        for psg in [-1_i16, 0, i16::MAX] {
            for old_priority in [i16::MIN, -1, 0, 1, i16::MAX] {
                for priority in [i16::MIN, -1, 0, 1, i16::MAX] {
                    for stop in [0, 1, 0xffff] {
                        let mut context = host.context();
                        ram::g_pcm_effect_cooldown_ticks(revision).write(&mut context, cooldown);
                        ram::g_psg_effect_status(revision).write(&mut context, psg);
                        ram::g_sound_effect_priority(revision).write(&mut context, old_priority);
                        ram::g_pcm_blocks_sequencer(revision).write(&mut context, -1);
                        check(
                            host,
                            ids::PLAY_SOUND_EFFECT,
                            &[
                                0xfedc_0000 | u32::from(priority as u16),
                                0xbeef_0000 | u32::from(priority as u16),
                                stop,
                            ],
                        );
                    }
                }
            }
        }
    }
}

fn drive_functional_live_fixtures(host: &mut Host) {
    let revision = host.revision();
    for id in [
        ids::SETUP_YM2612,
        ids::YM2612_UPDATE_FREQUENCY,
        ids::AUDIO_WRITE_FM_CHANNEL_RELEASE_MASK,
        ids::AUDIO_RELEASE_ALL_FM_CHANNELS,
        ids::AUDIO_LOAD_PENDING_CHANNEL_TEMPLATE_EFFECT,
        ids::MUSIC_LOAD_SONG_CHANNEL_TEMPLATES,
    ] {
        let mut context = host.context();
        context.set_data_register(1, 0);
        context.set_data_register(3, 0);
        context.set_data_register(4, 0x12);
        context.set_data_register(5, 0x34);
        ram::g_pending_fm_effect(revision).write(&mut context, 0);
        if id == ids::MUSIC_LOAD_SONG_CHANNEL_TEMPLATES {
            check(host, id, &[0]);
        } else {
            check(host, id, &[]);
        }
    }
    {
        let mut context = host.context();
        ram::g_music_alternate_template_bank_enabled(revision).write(&mut context, 0);
        check(host, ids::MUSIC_SELECT_SONG_TEMPLATE_BANK, &[0]);
    }
    {
        let mut context = host.context();
        context.set_address_register(0, ram::g_music_channels(revision).at(0).address());
        ram::g_music_template_restart_state(revision).write(&mut context, 2);
        ram::g_boom_box_music_template_hold(revision).write(&mut context, 0);
        check(host, ids::MUSIC_PROCESS_TEMPLATE_RESTART_HOOK, &[]);
    }
}

#[test]
#[ignore = "requires both private retail ROMs; 10,000 frames per revision plus a timed reference"]
fn audio_10000_frames_and_labeled_live_fixtures_both_revisions() {
    let script = Arc::new(
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap(),
    );
    let roots: Vec<_> = audio::TRANSLATIONS.iter().map(|entry| entry.id).collect();
    assert!(tje_game::Dispatcher::native(&roots).is_ok());
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = match revision {
            Revision::Rev02 => "TJE_REV02_ROM",
            Revision::Rev00 => "TJE_REV00_ROM",
        };
        let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
        let registry = HookRegistry::filtered(revision, &["audio"], &[]).unwrap();
        assert_eq!(registry.len(), audio::TRANSLATIONS.len());
        let mut checked = Host::new(&bytes, Mode::Check, registry.clone()).unwrap();
        let mut reference = Host::new(&bytes, Mode::Reference, registry).unwrap();
        checked.set_wait_input_poller(script.clone());
        reference.set_wait_input_poller(script.clone());
        for frame in 0..10_000 {
            let a = checked.run_frame(script.at(frame)).unwrap();
            let b = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(
                a.frame_counter, b.frame_counter,
                "{revision:?} frame {frame}"
            );
            assert_eq!(a.ram, b.ram, "{revision:?} whole RAM frame {frame}");
            assert_eq!(a.vdp, b.vdp, "{revision:?} VDP frame {frame}");
            assert_eq!(
                a.writes, b.writes,
                "{revision:?} hardware writes frame {frame}"
            );
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits())),
                "{revision:?} audio frame {frame}"
            );
            let (pa, sa) = checked.frame_buffer();
            let (pb, sb) = reference.frame_buffer();
            assert_eq!(sa, sb);
            assert_eq!(
                &pa[..sa.len() as usize],
                &pb[..sb.len() as usize],
                "{revision:?} pixels frame {frame}"
            );
        }
        assert!(checked.measurements.title_frame.is_some());
        assert!(checked.measurements.menu_frame.is_some());
        assert!(checked.measurements.game_frame.is_some());
        let natural = serde_json::to_value(&checked.checks).unwrap();
        let boundary = checked
            .settle_fixture_boundary(script.at(checked.frame_count()))
            .unwrap();
        let reference_boundary = reference
            .settle_fixture_boundary(script.at(reference.frame_count()))
            .unwrap();
        assert_eq!(boundary.start_pc, reference_boundary.start_pc);
        assert_eq!(boundary.end_pc, reference_boundary.end_pc);
        assert_eq!(boundary.start_frame, reference_boundary.start_frame);
        assert_eq!(boundary.end_frame, reference_boundary.end_frame);
        assert_eq!(boundary.ticks, reference_boundary.ticks);
        assert_eq!(boundary.master_clocks, reference_boundary.master_clocks);
        assert_eq!(
            boundary.handlers_completed,
            reference_boundary.handlers_completed
        );
        assert_eq!(
            boundary.completed_frames.len(),
            reference_boundary.completed_frames.len()
        );
        for (a, b) in boundary
            .completed_frames
            .iter()
            .zip(&reference_boundary.completed_frames)
        {
            assert_eq!(a.frame, b.frame);
            assert_eq!(a.frame_counter, b.frame_counter);
            assert_eq!(a.pc, b.pc);
            assert_eq!(a.ram, b.ram);
            assert_eq!(a.vdp, b.vdp);
            assert_eq!(a.writes, b.writes);
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples
                    .iter()
                    .flatten()
                    .map(|value| value.to_bits())
                    .eq(b
                        .audio_samples
                        .iter()
                        .flatten()
                        .map(|value| value.to_bits()))
            );
        }
        assert_eq!(checked.frame_buffer(), reference.frame_buffer());
        drive_live_fixtures(&mut checked);
        drive_functional_live_fixtures(&mut checked);
        assert_eq!(checked.checks.mismatches, 0);
        assert!(checked.checks.uninitialized_stack_reads.is_empty());
        assert!(checked.checks.unexercised.is_empty());
        assert_eq!(checked.checks.nested_original_calls, 0);
        let metrics = serde_json::json!({
            "revision": revision.name(),
            "frames": checked.frame_count(),
            "title_frame": checked.measurements.title_frame,
            "menu_frame": checked.measurements.menu_frame,
            "game_frame": checked.measurements.game_frame,
            "logic_ticks": checked.measurements.logic_ticks,
            "whole_session_equal": true,
            "natural_frames": 10_000,
            "boundary": {
                "start_pc": boundary.start_pc,
                "end_pc": boundary.end_pc,
                "start_frame": boundary.start_frame,
                "end_frame": boundary.end_frame,
                "ticks": boundary.ticks,
                "master_clocks": boundary.master_clocks,
                "handlers_completed": boundary.handlers_completed,
                "completed_frames": boundary.completed_frames.iter()
                    .map(|frame| format!("{frame:#?}")).collect::<Vec<_>>(),
            },
            "natural": natural,
            "with_live_fixtures": checked.checks,
        });
        if let Some(directory) = std::env::var_os("TJE_AUDIO_METRICS_DIR") {
            std::fs::write(
                std::path::Path::new(&directory)
                    .join(format!("{}-audio-all-fixtures.json", revision.name())),
                serde_json::to_vec_pretty(&metrics).unwrap(),
            )
            .unwrap();
        }
        println!("AUDIO_COVERAGE {metrics}");
    }
}
