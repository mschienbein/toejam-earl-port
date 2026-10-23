//! Opt-in real-ROM proof target for Functional FM writes.
//! Use the same external harness dependencies as audio/live_fixtures.rs.
//! All inputs below are explicitly labeled live-state fixtures after gameplay.

use tje_game::{Context, FunctionId, Platform, audio, execution::ExecutionClass, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram, rom_data_units};

fn host(revision: Revision, id: FunctionId) -> Host {
    let key = match revision {
        Revision::Rev02 => "TJE_REV02_ROM",
        Revision::Rev00 => "TJE_REV00_ROM",
    };
    let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
    let translation = audio::TRANSLATIONS
        .iter()
        .chain(audio::PENDING_TRANSLATIONS.iter())
        .find(|entry| entry.id == id)
        .unwrap();
    let registry = HookRegistry::proof(revision, std::slice::from_ref(translation)).unwrap();
    assert_eq!(
        registry
            .at(id.metadata().address(revision))
            .unwrap()
            .execution_class,
        ExecutionClass::Functional
    );
    let mut host = Host::new(&bytes, Mode::Reference, registry).unwrap();
    let script =
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap();
    for frame in 0..1300 {
        host.run_frame(script.at(frame)).unwrap();
    }
    assert!(host.measurements.game_frame.is_some());
    host
}

fn prepare(host: &mut Host, delay: u8) {
    let mut context = host.context();
    for index in 0..8 {
        context.set_data_register(index, 0x1234_5670 | index as u32);
    }
    for index in 0..7 {
        context.set_address_register(index, 0x2345_6780 | index as u32);
    }
    let status = context.status_register();
    context.set_status_register((status & !0x1f) | 0x1f);
    context.write_z80_reset(0x100);
    context.write_z80_bus_request(0x100);
    context.write_z80_ram(0x004a, delay);
}

fn check(host: &mut Host, id: FunctionId) {
    check_arguments(host, id, &[]);
}

fn check_arguments(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| {
            panic!(
                "live-state fixture {:?} {}: {error}",
                host.revision(),
                id.metadata().name
            )
        });
}

fn report(host: &Host, id: FunctionId) {
    assert_eq!(host.checks.mismatches, 0);
    assert!(host.checks.uninitialized_stack_reads.is_empty());
    assert!(host.checks.by_function[id.metadata().name].calls_checked > 0);
    let metrics = serde_json::json!({
        "revision": host.revision().name(),
        "function": id.metadata().name,
        "class": "Functional",
        "frames": host.frame_count(),
        "game_frame": host.measurements.game_frame,
        "fixtures": host.checks,
    });
    if let Some(directory) = std::env::var_os("TJE_AUDIO_METRICS_DIR") {
        std::fs::write(
            std::path::Path::new(&directory).join(format!(
                "{}-{}-functional.json",
                host.revision().name(),
                id.metadata().name
            )),
            serde_json::to_vec_pretty(&metrics).unwrap(),
        )
        .unwrap();
    }
    println!("AUDIO_FUNCTIONAL {metrics}");
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn release_masks_match_all_channels_masks_pacing_and_invalid_returns() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let id = ids::AUDIO_WRITE_FM_CHANNEL_RELEASE_MASK;
        let mut host = host(revision, id);
        for channel in [0, 1, 2, 3, 4, 5, 6, 0x7fff, 0xffff] {
            for mask in [0, 1, 5, 15, 0xff] {
                for delay in [0, 1, 5, 6, 0xff] {
                    prepare(&mut host, delay);
                    let mut context = host.context();
                    context.set_data_register(1, 0xabcd_0000 | channel);
                    if (channel as i16) < 6 {
                        ram::g_fm_channel_key_masks(revision)
                            .byte_offset(i32::from(channel as i16))
                            .write(&mut context, mask);
                    }
                    check(&mut host, id);
                }
            }
        }
        report(&host, id);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn frequency_writes_match_both_banks_pacing_and_invalid_returns() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let id = ids::YM2612_UPDATE_FREQUENCY;
        let mut host = host(revision, id);
        for channel in [0, 1, 2, 3, 4, 5, 6, 0x7fff, 0xffff] {
            for delay in [0, 1, 5, 6, 0xff] {
                prepare(&mut host, delay);
                let mut context = host.context();
                context.set_data_register(1, 0xabcd_0000 | channel);
                context.set_data_register(4, 0x1234_5678);
                context.set_data_register(5, 0x9876_54ff);
                check(&mut host, id);
            }
        }
        report(&host, id);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn patch_upload_matches_every_rom_record_both_frequency_modes_and_pacing() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let id = ids::SETUP_YM2612;
        let mut host = host(revision, id);
        let patch_count = rom_data_units(revision)
            .iter()
            .find(|unit| unit.name == "FMPatches")
            .unwrap()
            .size
            / 38;
        for channel in [0, 1, 2, 3, 4, 5] {
            for patch in 0..patch_count {
                for delay in [0, 1, 5, 6, 0xff] {
                    prepare(&mut host, delay);
                    let mut context = host.context();
                    context.set_data_register(1, 0xabcd_0000 | channel);
                    context.set_data_register(3, 0x9876_0000 | patch);
                    context.set_data_register(4, 0x1234_5678);
                    context.set_data_register(5, 0x9876_54ff);
                    ram::g_music_animation_channel(revision).write(&mut context, channel as i8);
                    ram::g_music_animation_event(revision).write(&mut context, 0);
                    check(&mut host, id);
                }
            }
        }
        for channel in [6, 0x7f, 0xff] {
            prepare(&mut host, 0);
            host.context().set_data_register(1, 0xabcd_0000 | channel);
            host.context().set_data_register(3, 0x9876_0000);
            check(&mut host, id);
        }
        report(&host, id);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn release_all_preserves_saved_registers_and_six_channel_order() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let id = ids::AUDIO_RELEASE_ALL_FM_CHANNELS;
        let mut host = host(revision, id);
        for delay in [0, 1, 5, 6, 0xff] {
            for mask in [0, 1, 5, 15, 0xff] {
                prepare(&mut host, delay);
                let mut context = host.context();
                for channel in 0..6 {
                    ram::g_fm_channel_key_masks(revision)
                        .byte_offset(channel)
                        .write(&mut context, mask);
                }
                ram::g_fm_channel_release_requested(revision).write(&mut context, -1);
                check(&mut host, id);
            }
        }
        report(&host, id);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn auxiliary_templates_match_all_requests_and_active_channel_subsets() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let id = ids::AUDIO_LOAD_PENDING_CHANNEL_TEMPLATE_EFFECT;
        let mut host = host(revision, id);
        let effect_count = rom_data_units(revision)
            .iter()
            .find(|unit| unit.name == "SfxChannelTemplateCounts")
            .unwrap()
            .size;
        for effect in 0..effect_count {
            for active in 0..8 {
                for delay in [0, 5] {
                    prepare(&mut host, delay);
                    let mut context = host.context();
                    for channel in 0..3 {
                        ram::g_fm_effect_channels(revision)
                            .byte_offset(4 + channel * 38)
                            .cast::<u32>()
                            .write(
                                &mut context,
                                if active & (1 << channel) != 0 {
                                    0x1234_5678
                                } else {
                                    0
                                },
                            );
                    }
                    ram::g_pending_fm_effect(revision).write(&mut context, effect as i16);
                    check(&mut host, id);
                }
            }
        }
        // Labeled index-width fixtures: ASL.W #2 wraps both offsets to zero,
        // while the unscaled count lookup still uses the signed original word.
        for effect in [0x4000_i16, -0x4000_i16] {
            prepare(&mut host, 0);
            let mut context = host.context();
            for channel in 0..3 {
                ram::g_fm_effect_channels(revision)
                    .byte_offset(4 + channel * 38)
                    .cast::<u32>()
                    .write(&mut context, 0);
            }
            ram::g_pending_fm_effect(revision).write(&mut context, effect);
            check(&mut host, id);
        }
        report(&host, id);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn song_template_loader_matches_every_template_and_unsigned_argument_word() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let id = ids::MUSIC_LOAD_SONG_CHANNEL_TEMPLATES;
        let mut host = host(revision, id);
        let template_count = rom_data_units(revision)
            .iter()
            .find(|unit| unit.name == "SongChannelTemplates")
            .unwrap()
            .size
            / 228;
        for template in 0..template_count {
            for upper in [0, 0xbeef_0000] {
                for delay in [0, 1, 5, 6, 0xff] {
                    prepare(&mut host, delay);
                    check_arguments(&mut host, id, &[upper | template]);
                }
            }
        }
        // MULU.W produces a full long offset, unlike a wrapping word index.
        // These unsigned argument words address the original RAM mirror; no
        // pointer normalization, range clamp, or ROM-byte transcription occurs.
        for template in [0xfffe, 0xffff] {
            prepare(&mut host, 0);
            check_arguments(&mut host, id, &[0xdead_0000 | template]);
        }
        report(&host, id);
    }
}
