//! Opt-in pending-upload proof; the five mailbox entries are not RTS hooks here.

use std::sync::Arc;
use tje_game::{Context, Platform, audio, ids, open_bus::OpenBusSource};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::Revision;

#[test]
#[ignore = "requires both private retail ROMs"]
fn pending_upload_matches_both_rom_revisions_and_real_reset_denial() {
    let script = Arc::new(
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap(),
    );
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = match revision {
            Revision::Rev02 => "TJE_REV02_ROM",
            Revision::Rev00 => "TJE_REV00_ROM",
        };
        let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
        let id = ids::LOAD_SOUND_DRIVER_TO_Z80;
        let entry = audio::PENDING_TRANSLATIONS
            .iter()
            .find(|entry| entry.id == id)
            .unwrap();
        let registry = HookRegistry::proof(revision, std::slice::from_ref(entry)).unwrap();
        let mut host = Host::new(&bytes, Mode::Check, registry).unwrap();
        host.set_wait_input_poller(script.clone());
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        assert!(host.measurements.game_frame.is_some());
        let natural = serde_json::to_value(&host.checks).unwrap();
        let boundary = host
            .settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        let source = OpenBusSource::prefetch(id.metadata().address(revision).wrapping_add(0x1a));
        let operand = source.word_from_rom(&bytes).unwrap() & !0x100;
        let first = 0xffff_u32.wrapping_mul(u32::from(operand));
        let expected = u32::from(first as u16).wrapping_mul(u32::from(operand));
        {
            let mut context = host.context();
            context.write_z80_reset(0x100);
            context.set_data_register(0, 0x1357_9bdf);
            context.set_data_register(1, 0xdead_beef);
            context.set_data_register(2, 0x89ab_cdef);
        }
        assert!(host.fixture_boundary_is_settled());
        assert_eq!(
            host.check_fixture(id.metadata().address(revision), &[])
                .unwrap(),
            expected
        );
        {
            let mut context = host.context();
            context.write_z80_reset(0);
            context.set_data_register(0, 0x2468_ace0);
            context.set_data_register(1, 0xcafe_0000);
            context.set_data_register(2, 0xfedc_ba98);
        }
        assert!(host.fixture_boundary_is_settled());
        assert_eq!(
            host.check_fixture(id.metadata().address(revision), &[])
                .unwrap(),
            0x2468_ace0
        );
        host.context().write_z80_reset(0x100);
        assert!(host.fixture_boundary_is_settled());
        host.check_fixture(id.metadata().address(revision), &[])
            .unwrap();
        assert_eq!(host.checks.mismatches, 0);
        assert!(host.checks.uninitialized_stack_reads.is_empty());
        assert_eq!(host.checks.nested_original_calls, 0);
        assert!(host.checks.by_function[id.metadata().name].calls_checked > 0);
        let metrics = serde_json::json!({
            "revision": revision.name(), "function": id.metadata().name,
            "prefetch_address": source.address().unwrap(), "natural": natural,
            "with_live_fixtures": host.checks, "frames": host.frame_count(),
            "game_frame": host.measurements.game_frame,
            "boundary": {
                "start_pc": boundary.start_pc, "end_pc": boundary.end_pc,
                "start_frame": boundary.start_frame, "end_frame": boundary.end_frame,
                "ticks": boundary.ticks, "master_clocks": boundary.master_clocks,
                "handlers_completed": boundary.handlers_completed,
                "completed_frames": boundary.completed_frames.iter().map(|frame| format!("{frame:#?}")).collect::<Vec<_>>(),
            },
        });
        if let Some(directory) = std::env::var_os("TJE_AUDIO_REMAINING_METRICS_DIR") {
            std::fs::write(
                std::path::Path::new(&directory)
                    .join(format!("{}-remaining-upload.json", revision.name())),
                serde_json::to_vec_pretty(&metrics).unwrap(),
            )
            .unwrap();
        }
        println!("REMAINING_UPLOAD {metrics}");
    }
}
