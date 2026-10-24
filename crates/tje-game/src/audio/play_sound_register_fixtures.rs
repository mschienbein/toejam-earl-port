//! Opt-in all-register/CCR proof for the real PlaySound producer.
//! These are labelled live-state fixtures, not natural-gameplay inputs.

use std::sync::Arc;
use tje_game::{Context, audio, ids};
use tje_host::{Host, Mode, abi::HookContext, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram};

fn strict_play_sound(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let arguments = [context.argument_u32(0), context.argument_u32(1)];
    Ok(audio::play_sound(context, &arguments)?)
}

fn stale_data0(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let entry = context.data_register(0);
    let result = strict_play_sound(context)?;
    context.set_data_register(0, entry);
    Ok(result)
}

fn stale_ccr(context: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let entry = context.status_register();
    let result = strict_play_sound(context)?;
    context.set_status_register(entry);
    Ok(result)
}

fn registry(revision: Revision, hook: tje_host::hooks::HookFn) -> HookRegistry {
    let mut symbol = ids::PLAY_SOUND.metadata().clone();
    symbol.abi.strict_registers = true;
    let mut registry = HookRegistry::empty(revision);
    registry.register_contract(symbol, true, hook).unwrap();
    registry
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn play_sound_all_paths_preserve_full_cpu_and_exact_ccr_both_revisions() {
    let script = Arc::new(
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap(),
    );
    // label, cooldown, PSG status, old priority, requested priority, sound number
    let cases = [
        ("idle-zero", 0_i16, 0_i16, i16::MAX, 0_i16, i16::MIN),
        ("idle-negative", 0, 0, i16::MAX, i16::MIN, -1),
        ("idle-skates", 0, 0, i16::MAX, 99, 6),
        ("cooldown-admit", 1, -1, 1, 99, 6),
        ("cooldown-equal-negative", i16::MIN, 0, -1, -1, 1),
        ("cooldown-reject-overflow", 1, 1, 1, i16::MIN, 6),
        ("cooldown-reject-negative", -1, 1, 0, -1, 6),
        ("psg-admit-overflow", 0, 1, i16::MIN, i16::MAX, 6),
        ("psg-equal-zero", 0, i16::MIN, 0, 0, 0),
        ("psg-reject-borrow", 0, 1, i16::MAX, 0, 6),
        ("psg-reject-overflow", 0, -1, i16::MAX, i16::MIN, 6),
        ("both-busy-reject", 1, i16::MIN, 1, 0, 6),
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = match revision {
            Revision::Rev02 => "TJE_REV02_ROM",
            Revision::Rev00 => "TJE_REV00_ROM",
        };
        let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
        let mut host =
            Host::new(&bytes, Mode::Check, registry(revision, strict_play_sound)).unwrap();
        host.set_wait_input_poller(script.clone());
        for frame in 0..10_000 {
            host.run_frame(script.at(frame)).unwrap();
        }
        let natural = serde_json::to_value(&host.checks).unwrap();
        let calls_before = host.checks.by_function["PlaySound"].calls_checked;
        let boundary = host
            .settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        let control = host.context().status_register() & !0x1f;
        for high in [0_u32, 0x1234_0000, 0xffff_0000] {
            for ccr in 0..32_u16 {
                for (label, cooldown, psg, old_priority, priority, sound) in cases {
                    {
                        let mut context = host.context();
                        ram::g_pcm_effect_cooldown_ticks(revision).write(&mut context, cooldown);
                        ram::g_psg_effect_status(revision).write(&mut context, psg);
                        ram::g_sound_effect_priority(revision).write(&mut context, old_priority);
                        ram::g_pending_psg_effect(revision).write(&mut context, 0x4567);
                        context.set_status_register(control | ccr);
                        context.set_data_register(0, high | 0xabcd);
                        for index in 1..8 {
                            context.set_data_register(index, 0x1020_3040 | index as u32);
                        }
                        for index in 0..7 {
                            context.set_address_register(index, 0xabcd_e000 | index as u32);
                        }
                    }
                    host.check_fixture(
                        ids::PLAY_SOUND.metadata().address(revision),
                        &[
                            0xdead_0000 | u32::from(sound as u16),
                            0xbeef_0000 | u32::from(priority as u16),
                        ],
                    )
                    .unwrap_or_else(|error| {
                        panic!("{key} {label} D0.high={high:08x} CCR={ccr:02x}: {error}")
                    });
                }
            }
        }
        assert_eq!(
            host.checks.by_function["PlaySound"].calls_checked - calls_before,
            (cases.len() * 3 * 32) as u64
        );
        assert_eq!(host.checks.mismatches, 0);
        assert!(host.checks.uninitialized_stack_reads.is_empty());
        assert_eq!(host.checks.nested_original_calls, 0);
        let positive = serde_json::json!({
            "revision": revision.name(), "strict_registers": true,
            "frames": host.frame_count(), "natural": natural,
            "fixture_cases": cases.iter().map(|case| case.0).collect::<Vec<_>>(),
            "fixtures_checked": cases.len() * 3 * 32,
            "d0_upper_words": [0_u32, 0x1234_0000, 0xffff_0000_u32],
            "entry_ccr_combinations": 32,
            "with_fixtures": host.checks,
            "boundary": {
                "start_frame": boundary.start_frame, "end_frame": boundary.end_frame,
                "start_pc": boundary.start_pc, "end_pc": boundary.end_pc,
                "ticks": boundary.ticks, "master_clocks": boundary.master_clocks,
                "handlers_completed": boundary.handlers_completed,
                "completed_frames": boundary.completed_frames.iter().map(|frame| format!("{frame:#?}")).collect::<Vec<_>>(),
            },
        });
        let mut negative = Vec::new();
        for (label, hook, expected) in [
            ("stale-D0", stale_data0 as tje_host::hooks::HookFn, "D0"),
            (
                "stale-CCR",
                stale_ccr as tje_host::hooks::HookFn,
                "SR control bits",
            ),
        ] {
            let mut mutant = Host::new(&bytes, Mode::Reference, registry(revision, hook)).unwrap();
            mutant.set_wait_input_poller(script.clone());
            for frame in 0..1300 {
                mutant.run_frame(script.at(frame)).unwrap();
            }
            let advance = mutant
                .settle_fixture_boundary(script.at(mutant.frame_count()))
                .unwrap();
            {
                let mut context = mutant.context();
                ram::g_pcm_effect_cooldown_ticks(revision).write(&mut context, 0);
                ram::g_psg_effect_status(revision).write(&mut context, 0);
                ram::g_sound_effect_priority(revision).write(&mut context, 0);
                context.set_data_register(0, 0x1234_abcd);
                let control = context.status_register() & !0x1f;
                context.set_status_register(control | 0x1f);
            }
            let error = mutant
                .check_fixture(ids::PLAY_SOUND.metadata().address(revision), &[6, 99])
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{key} {label}: {error}");
            assert_eq!(mutant.checks.mismatches, 1);
            negative.push(serde_json::json!({
                "mutation": label, "rejection": error, "checks": mutant.checks,
                "boundary": {
                    "start_frame": advance.start_frame, "end_frame": advance.end_frame,
                    "start_pc": advance.start_pc, "end_pc": advance.end_pc,
                    "ticks": advance.ticks, "master_clocks": advance.master_clocks,
                    "handlers_completed": advance.handlers_completed,
                    "completed_frames": advance.completed_frames.iter().map(|frame| format!("{frame:#?}")).collect::<Vec<_>>(),
                },
            }));
        }
        let evidence = serde_json::json!({"positive": positive, "negative_controls": negative});
        if let Some(directory) = std::env::var_os("TJE_AUDIO_ABI_METRICS_DIR") {
            std::fs::write(
                std::path::Path::new(&directory)
                    .join(format!("{}-play-sound-registers.json", revision.name())),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
        }
        println!("PLAY_SOUND_REGISTERS {evidence}");
    }
}
