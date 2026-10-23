//! Natural non-RTS sessions exclude the still-pending cold-start Rust body.
use std::{fs, path::PathBuf, sync::Arc};
use tje_game::Translation;
pub use tje_game::{Context, Result, call_at, call_frame, execution, exits, ids, open_bus};
#[path = "entries.rs"]
pub mod entries;
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::Revision;

static DIRECT: &[Translation] = &[
    Translation {
        id: ids::GAME_PROGRAM,
        function: entries::game_program,
    },
    Translation {
        id: ids::RESTART_FROM_RESET_ENTRY,
        function: entries::restart_from_reset_entry,
    },
];
static INTERRUPTS: &[Translation] = &[
    Translation {
        id: ids::LEVEL4_ISR,
        function: entries::level4_isr,
    },
    Translation {
        id: ids::LEVEL6_ISR,
        function: entries::level6_isr,
    },
];
static HBLANK: &[Translation] = &[Translation {
    id: ids::LEVEL4_ISR,
    function: entries::level4_isr,
}];

fn save(host: &Host, label: &str) {
    let directory = PathBuf::from(std::env::var_os("TJE_PLATFORM_EVIDENCE_DIR").unwrap());
    let report = serde_json::json!({
        "revision": host.revision().name(), "frames": host.frame_count(),
        "title_frame": host.measurements.title_frame, "menu_frame": host.measurements.menu_frame,
        "game_frame": host.measurements.game_frame, "logic_ticks": host.measurements.logic_ticks,
        "source": label, "checks": host.checks,
    });
    fs::write(
        directory.join(format!("{}-{label}.json", host.revision().name())),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("ENTRY_SESSION {report}");
}

fn session(selected: &'static [Translation], label: &str) {
    let script = Arc::new(
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap(),
    );
    for (revision, key) in [
        (Revision::Rev02, "TJE_REV02_ROM"),
        (Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let bytes = fs::read(std::env::var_os(key).unwrap()).unwrap();
        let registry = HookRegistry::proof(revision, selected).unwrap();
        let mut checked = Host::new(&bytes, Mode::Check, registry.clone()).unwrap();
        let mut reference = Host::new(&bytes, Mode::Reference, registry).unwrap();
        checked.set_wait_input_poller(script.clone());
        for frame in 0..10_000 {
            let a = checked.run_frame(script.at(frame)).unwrap_or_else(|error| {
                save(&checked, label);
                panic!("{revision:?} frame {frame}: {error}")
            });
            let b = reference.run_frame(script.at(frame)).unwrap();
            assert_eq!(a.frame_counter, b.frame_counter);
            assert_eq!(a.ram, b.ram, "{revision:?} RAM frame {frame}");
            assert_eq!(a.vdp, b.vdp, "{revision:?} VDP frame {frame}");
            assert_eq!(a.writes, b.writes, "{revision:?} writes frame {frame}");
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples.iter().flatten().map(|x| x.to_bits()).eq(b
                    .audio_samples
                    .iter()
                    .flatten()
                    .map(|x| x.to_bits()))
            );
            let (a, size_a) = checked.frame_buffer();
            let (b, size_b) = reference.frame_buffer();
            assert_eq!(size_a, size_b);
            assert_eq!(&a[..size_a.len() as usize], &b[..size_b.len() as usize]);
        }
        save(&checked, label);
        assert!(checked.measurements.game_frame.is_some());
        assert_eq!(checked.checks.mismatches, 0);
        assert!(checked.checks.uninitialized_stack_reads.is_empty());
        for entry in selected {
            if entry.id != ids::RESTART_FROM_RESET_ENTRY {
                assert!(checked.checks.by_function[entry.id.metadata().name].calls_checked > 0);
            }
        }
    }
}

#[test]
#[ignore = "requires both private ROMs; independent of pending cold-start fill join"]
fn direct_prefixes_10000_frames_both_revisions() {
    session(DIRECT, "direct-entry-session");
}

#[test]
#[ignore = "requires both private ROMs; actual IRQ observations must pass projection"]
fn interrupt_entries_10000_frames_both_revisions() {
    session(INTERRUPTS, "irq-entry-session");
}

#[test]
#[ignore = "requires both private ROMs; independent of the known VBlank YM phase failure"]
fn hblank_entry_10000_frames_both_revisions() {
    session(HBLANK, "hblank-entry-session");
}
