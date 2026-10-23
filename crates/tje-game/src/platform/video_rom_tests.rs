//! Opt-in proof target: published platform leaves plus explicitly pending DMA.
use std::{fs, path::PathBuf};
use tje_game::{Context, FunctionId, ids, platform};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram, rom};

fn registry(revision: Revision) -> HookRegistry {
    let mut registry = HookRegistry::proof(revision, platform::PENDING_TRANSLATIONS).unwrap();
    for &entry in platform::TRANSLATIONS {
        let class = tje_game::execution_class(entry.id).unwrap();
        registry
            .register_translation_with_execution(entry, class)
            .unwrap();
    }
    registry
}

fn save(host: &Host, natural: &serde_json::Value) {
    let directory = PathBuf::from(std::env::var_os("TJE_PLATFORM_EVIDENCE_DIR").unwrap());
    let report = serde_json::json!({
        "revision": host.revision().name(), "frames": host.frame_count(),
        "title_frame": host.measurements.title_frame, "menu_frame": host.measurements.menu_frame,
        "game_frame": host.measurements.game_frame, "logic_ticks": host.measurements.logic_ticks,
        "natural": natural, "with_live_fixtures": host.checks,
    });
    fs::write(
        directory.join(format!("{}-functional.json", host.revision().name())),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("FUNCTIONAL_PLATFORM {report}");
}

fn check(host: &mut Host, id: FunctionId, arguments: &[u32], natural: &serde_json::Value) {
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| {
            save(host, natural);
            panic!("{} {arguments:08x?}: {error}", id.metadata().name)
        });
}

fn fixtures(host: &mut Host, natural: &serde_json::Value) {
    // Synthetic inputs occupy a bounded live upload buffer, above the dead-stack
    // region. Long zero-counter copies read the user's ROM through a named view;
    // they must not accidentally consume uninitialized stack bytes.
    let source = ram::g_sprite_tile_upload_buffer(host.revision()).address();
    let long_source = rom::starfield_byte_map(host.revision()).address();
    for index in 0..32_u32 {
        host.context()
            .write_u32(source + 4 * index, 0x8000_0000 | index);
    }
    for id in [
        ids::CLEAR_VIDEO_RAM,
        ids::CLEAR_VERTICAL_SCROLL_RAM,
        ids::CLEAR_COLOUR_RAM,
    ] {
        for (d0, sr) in [(0, 0x2700), (0x1234_8000, 0x271f), (0xffff_0000, 0x2704)] {
            host.context().set_data_register(0, d0);
            host.context().set_status_register(sr);
            check(host, id, &[], natural);
        }
    }
    for (tile, count) in [
        (0, 0),
        (0x201, 1),
        (u32::MAX, 0x2000_0001),
        (0xffff, 0x2000),
    ] {
        check(
            host,
            ids::COPY_LONGWORD_RUN_TO_VRAM_TILE,
            &[source, tile, count],
            natural,
        );
    }
    for id in [ids::FILL_VDP_DATA_WORDS, ids::FILL_VDP_DATA_BYTES] {
        for value in [0, 0x80, 0x8000, 0x1234_ffff] {
            for count in [1, 3, 0x1234_0001, 0] {
                check(host, id, &[value, count], natural);
            }
        }
    }
    for count in [1, 3, 0x1234_0001, 0] {
        let pointer = if count == 0 { long_source } else { source };
        check(host, ids::MOVE_DATA_TO_VRAM, &[pointer, count], natural);
    }
    for saved_d2 in [0, 0x8000_0000, 1] {
        host.context().set_data_register(2, saved_d2);
        for arguments in [
            [0xfffe, source, 1, 1, 0],
            [0x1fffe, source, 3, 2, 0x8000_0001],
            [0xc002, long_source, 0, 1, 4],
        ] {
            check(host, ids::UPLOAD_VRAM_WORD_RECTANGLE, &arguments, natural);
        }
    }
    for slot in [0, 1, 3, u32::MAX] {
        check(
            host,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            &[source, slot],
            natural,
        );
    }
    for (destination, count) in [(0, 1), (0xd000, 3), (0xfffe, 64), (0xc002, 0)] {
        let pointer = if count == 0 { long_source } else { source };
        check(
            host,
            ids::DMA_COPY_WORDS_TO_VRAM,
            &[pointer, destination, count],
            natural,
        );
    }
    // This observes DMA's own MOVEM save area. It must pass before DMA can leave
    // PENDING; the earlier ROM-source zero-count proof did not exercise it.
    check(
        host,
        ids::DMA_COPY_WORDS_TO_VRAM,
        &[source, 0xc002, 0],
        natural,
    );
}

#[test]
#[ignore = "requires both private ROMs; 10,000 filtered frames plus live-state fixtures"]
fn functional_platform_sessions_and_live_fixtures() {
    let script =
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap();
    for (revision, key) in [
        (Revision::Rev02, "TJE_REV02_ROM"),
        (Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let bytes = fs::read(std::env::var_os(key).unwrap()).unwrap();
        let registry = registry(revision);
        let mut host = Host::new(&bytes, Mode::Check, registry.clone()).unwrap();
        let mut reference = Host::new(&bytes, Mode::Reference, registry).unwrap();
        for frame in 0..10_000 {
            let a = host.run_frame(script.at(frame)).unwrap_or_else(|error| {
                let natural = serde_json::to_value(&host.checks).unwrap();
                save(&host, &natural);
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
            let (a, size_a) = host.frame_buffer();
            let (b, size_b) = reference.frame_buffer();
            assert_eq!(size_a, size_b);
            assert_eq!(&a[..size_a.len() as usize], &b[..size_b.len() as usize]);
        }
        assert!(host.measurements.title_frame.is_some());
        assert!(host.measurements.game_frame.is_some());
        let natural = serde_json::to_value(&host.checks).unwrap();
        fixtures(&mut host, &natural);
        save(&host, &natural);
        assert_eq!(host.checks.mismatches, 0);
        assert!(host.checks.uninitialized_stack_reads.is_empty());
        assert!(host.checks.unexercised.is_empty());
    }
}
