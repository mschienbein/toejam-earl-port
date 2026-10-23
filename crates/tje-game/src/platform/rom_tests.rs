//! Opt-in host harness, intentionally not part of tje-game's ROM-free dependency graph.
//! Point a separate scratch Cargo test target here; see the lane's private report.
use m68000_emu::BusInterface;
use std::{fs, path::PathBuf};
use tje_game::{FunctionId, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram};

fn evidence_dir() -> PathBuf {
    PathBuf::from(
        std::env::var_os("TJE_PLATFORM_EVIDENCE_DIR").expect("private evidence directory"),
    )
}

fn save_report(host: &Host, natural: &serde_json::Value) {
    let report = serde_json::json!({
        "revision": host.revision().name(),
        "frames": host.frame_count(),
        "title_frame": host.measurements.title_frame,
        "menu_frame": host.measurements.menu_frame,
        "game_frame": host.measurements.game_frame,
        "logic_ticks": host.measurements.logic_ticks,
        "natural": natural,
        "with_live_fixtures": host.checks,
    });
    fs::write(
        evidence_dir().join(format!("{}-fixtures.json", host.revision().name())),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("PLATFORM_COVERAGE {report}");
}

fn fixture(host: &mut Host, id: FunctionId, natural: &serde_json::Value) {
    host.check_fixture(id.metadata().address(host.revision()), &[])
        .unwrap_or_else(|error| {
            save_report(host, natural);
            panic!("{}: {error}", id.metadata().name)
        });
}

fn drive_live_fixtures(host: &mut Host, natural: &serde_json::Value) {
    // These are labeled synthetic register/device boundary inputs on a live
    // post-gameplay fork, not claims that normal play produces these values.
    for id in [ids::DISABLE_ALL_INTERRUPTS, ids::ENABLE_ALL_INTERRUPTS] {
        for priority in [0, 3, 7] {
            for ccr in 0..32 {
                let mut context = host.context();
                let cpu = context.cpu();
                let mut data = cpu.data_registers();
                for (index, register) in data.iter_mut().enumerate() {
                    *register = 0x1234_5678_u32.wrapping_add(0x1111_1111 * index as u32);
                }
                cpu.set_data_registers(data);
                cpu.set_status_register(0x2000 | (priority << 8) | ccr);
                fixture(host, id, natural);
            }
        }
    }
    for id in [ids::GET_CTRL1_INPUT, ids::GET_CTRL2_INPUT] {
        for delay in [0_u8, 1, 2, 255] {
            for psg in [0_u8, 1, 128, 255] {
                for d1 in [0_u32, 0x89ab_cdef, u32::MAX] {
                    let revision = host.revision();
                    let mut context = host.context();
                    let cpu = context.cpu();
                    let mut data = cpu.data_registers();
                    data[0] = 0xdead_beef;
                    data[1] = d1;
                    cpu.set_data_registers(data);
                    cpu.set_status_register(0x271f);
                    context.bus().write_word(0xa11100, 0x0100);
                    context.bus().write_byte(0xa0004a, delay);
                    context.bus().write_byte(0xa0004b, psg);
                    context.bus().write_word(0xa11100, 0);
                    ram::g_psg_effect_status(revision).write(&mut context, 0x35a5);
                    fixture(host, id, natural);
                }
            }
        }
    }
}

#[test]
#[ignore = "requires both private retail ROMs and a private evidence directory"]
fn platform_10000_frames_and_labeled_live_fixtures_both_revisions() {
    let script =
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap();
    for (revision, key) in [
        (Revision::Rev02, "TJE_REV02_ROM"),
        (Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let bytes = fs::read(std::env::var_os(key).expect("private cartridge path")).unwrap();
        let registry = HookRegistry::filtered(
            revision,
            &[],
            &[
                ids::DISABLE_ALL_INTERRUPTS,
                ids::ENABLE_ALL_INTERRUPTS,
                ids::GET_CTRL1_INPUT,
                ids::GET_CTRL2_INPUT,
            ],
        )
        .unwrap();
        let mut checked = Host::new(&bytes, Mode::Check, registry.clone()).unwrap();
        let mut reference = Host::new(&bytes, Mode::Reference, registry).unwrap();
        for frame in 0..10_000 {
            let a = checked.run_frame(script.at(frame)).unwrap_or_else(|error| {
                let natural = serde_json::to_value(&checked.checks).unwrap();
                save_report(&checked, &natural);
                panic!("{revision:?} frame {frame}: {error}")
            });
            let b = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(
                a.frame_counter, b.frame_counter,
                "{revision:?} frame {frame}"
            );
            assert_eq!(a.ram, b.ram, "{revision:?} whole RAM frame {frame}");
            assert_eq!(a.vdp, b.vdp, "{revision:?} VDP frame {frame}");
            assert_eq!(a.writes, b.writes, "{revision:?} writes frame {frame}");
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples.iter().flatten().map(|v| v.to_bits()).eq(b
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits()))
            );
            let (pa, sa) = checked.frame_buffer();
            let (pb, sb) = reference.frame_buffer();
            assert_eq!(sa, sb);
            assert_eq!(&pa[..sa.len() as usize], &pb[..sb.len() as usize]);
        }
        assert!(checked.measurements.title_frame.is_some());
        assert!(checked.measurements.menu_frame.is_some());
        assert!(checked.measurements.game_frame.is_some());
        let natural = serde_json::to_value(&checked.checks).unwrap();
        drive_live_fixtures(&mut checked, &natural);
        save_report(&checked, &natural);
        assert_eq!(checked.checks.mismatches, 0);
        assert_eq!(checked.checks.nested_original_calls, 0);
        assert_eq!(checked.checks.dead_stack_differing_calls, 0);
        assert!(checked.checks.uninitialized_stack_reads.is_empty());
        assert!(checked.checks.unexercised.is_empty());
    }
}
