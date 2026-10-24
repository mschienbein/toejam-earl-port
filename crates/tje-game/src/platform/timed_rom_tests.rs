//! Opt-in canonical ClockedStrict proof; the public source bodies stay unregistered.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tje_game::{
    Context, FramedTranslation, FunctionId, exits::EntryOutcome, ids, platform::timed,
    timing::TimingSource,
};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram, rom};

static CANDIDATES: &[FramedTranslation] = &[
    FramedTranslation {
        id: ids::GET_CTRL1_INPUT,
        function: timed::get_ctrl1_input,
    },
    FramedTranslation {
        id: ids::GET_CTRL2_INPUT,
        function: timed::get_ctrl2_input,
    },
    FramedTranslation {
        id: ids::MOVE_DATA_TO_VRAM,
        function: timed::move_data_to_vram,
    },
    FramedTranslation {
        id: ids::COPY_PALETTE_TO_CRAM_SLOT,
        function: timed::copy_palette_to_cram_slot,
    },
];

// Programs contain only source roles/FunctionIds, not revision-specific ROM data.
// Use the SAME public provider that each exported candidate constructs.
static SOURCES: LazyLock<[&tje_game::timing::Program; 4]> = LazyLock::new(|| {
    [
        timed::Leaf::Controller1,
        timed::Leaf::Controller2,
        timed::Leaf::WordStream,
        timed::Leaf::Palette,
    ]
    .map(|leaf| timed::Source::new(Revision::Rev02, leaf).program())
});

// Completed genuine reference JSR/BSR calls: count, entry exception depths,
// actual return PCs. These summaries are not invented per-call pair receipts.
type NaturalCalls = BTreeMap<u32, (u64, BTreeSet<u32>, BTreeSet<u32>)>;

fn save(host: &Host, natural: &serde_json::Value, observed: &NaturalCalls) {
    let directory = PathBuf::from(std::env::var_os("TJE_PLATFORM_EVIDENCE_DIR").unwrap());
    let report = serde_json::json!({
        "revision": host.revision().name(),
        "profile": "ClockedStrict; original classes unchanged; canonical public platform::timed functions",
        "source_preservation_commit": "6913967",
        "frames": host.frame_count(),
        "game_frame": host.measurements.game_frame,
        "natural": natural,
        "reference_completed_calls_count_depths_returns": observed,
        "with_live_fixtures": host.checks,
    });
    fs::write(
        directory.join(format!(
            "{}-clocked-platform-four.json",
            host.revision().name()
        )),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!(
        "CLOCKED_PLATFORM_FOUR {}",
        serde_json::json!({
            "revision": host.revision().name(),
            "natural": natural.get("by_function"),
            "final": host.checks.by_function,
            "mismatches": host.checks.mismatches,
            "unexercised": host.checks.unexercised,
            "reference_completed_calls_count_depths_returns": observed,
        })
    );
}

fn fixture(
    host: &mut Host,
    id: FunctionId,
    arguments: &[u32],
    natural: &serde_json::Value,
    observed: &NaturalCalls,
) {
    let outcome = host
        .check_clocked_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| {
            save(host, natural, observed);
            panic!(
                "{} real-return fixture {arguments:08x?}: {error}",
                id.metadata().name
            )
        });
    assert!(matches!(outcome, EntryOutcome::Returned(_)));
}

#[test]
#[ignore = "requires both private ROMs; canonical real-clock natural and real-return fixtures"]
fn four_public_platform_sources_match_both_roms_clocked_strict() {
    let script = Arc::new(
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap(),
    );
    for (revision, key) in [
        (Revision::Rev02, "TJE_REV02_ROM"),
        (Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let bytes = fs::read(std::env::var_os(key).unwrap()).unwrap();
        let registry = HookRegistry::proof_clocked_strict(revision, &[], CANDIDATES, &[])
            .unwrap()
            .with_timing_sources(&*SOURCES)
            .unwrap();
        let mut checked = Host::new(&bytes, Mode::Check, registry).unwrap();
        let mut reference =
            Host::new(&bytes, Mode::Reference, HookRegistry::empty(revision)).unwrap();
        checked.set_wait_input_poller(script.clone());
        reference.set_wait_input_poller(script.clone());
        let entries: Vec<_> = CANDIDATES
            .iter()
            .map(|entry| entry.id.metadata().address(revision))
            .collect();
        reference
            .enable_session_trace(&entries, &[], &[], false)
            .unwrap();
        reference.enable_call_observations(&entries).unwrap();
        let mut observed = NaturalCalls::new();
        for frame in 0..10_000 {
            let a = checked.run_frame(script.at(frame)).unwrap_or_else(|error| {
                save(&checked, &serde_json::Value::Null, &observed);
                panic!("{revision:?} natural frame {frame}: {error}")
            });
            let b = reference.run_frame(script.at(frame)).unwrap();
            for call in reference.take_call_observations() {
                let summary = observed.entry(call.entry_pc).or_default();
                summary.0 += 1;
                summary.1.insert(call.entry_exception_depth);
                summary.2.insert(call.return_pc);
            }
            assert_eq!(a.ram, b.ram, "{revision:?} RAM frame {frame}");
            assert_eq!(a.vdp, b.vdp, "{revision:?} VDP frame {frame}");
            assert_eq!(a.writes, b.writes, "{revision:?} writes frame {frame}");
            assert_eq!(a.frame_counter, b.frame_counter);
            assert_eq!(a.idle_random_draws, b.idle_random_draws);
            assert_eq!(a.lag_events, b.lag_events);
            assert!(
                a.audio_samples
                    .iter()
                    .flatten()
                    .map(|sample| sample.to_bits())
                    .eq(b
                        .audio_samples
                        .iter()
                        .flatten()
                        .map(|sample| sample.to_bits()))
            );
            let (a, size_a) = checked.frame_buffer();
            let (b, size_b) = reference.frame_buffer();
            assert_eq!(size_a, size_b);
            assert_eq!(&a[..size_a.len() as usize], &b[..size_b.len() as usize]);
        }
        assert!(checked.measurements.game_frame.is_some());
        let natural = serde_json::to_value(&checked.checks).unwrap();
        checked
            .settle_fixture_boundary(script.at(checked.frame_count()))
            .unwrap();
        // Only CCR changes; retain the genuine live supervisor/interrupt mask.
        let actual_upper_sr = checked.context().status_register() & !0x1f;
        for id in [ids::GET_CTRL1_INPUT, ids::GET_CTRL2_INPUT] {
            for (d1, ccr) in [(0, 0), (0x89ab_ffff, 0x1f), (0xffff_0000, 0x10)] {
                checked.context().set_data_register(1, d1);
                checked.context().set_status_register(actual_upper_sr | ccr);
                fixture(&mut checked, id, &[], &natural, &observed);
            }
        }
        let source = ram::g_sprite_tile_upload_buffer(revision).address();
        for index in 0..32_u32 {
            checked
                .context()
                .write_u32(source + index * 4, 0x8000_0000 | index);
        }
        let rom_source = rom::starfield_byte_map(revision).address();
        for count in [1, 3, 0x1234_0001, 0] {
            let pointer = if count == 0 { rom_source } else { source };
            fixture(
                &mut checked,
                ids::MOVE_DATA_TO_VRAM,
                &[pointer, count],
                &natural,
                &observed,
            );
        }
        for slot in [0, 1, 3, u32::MAX] {
            fixture(
                &mut checked,
                ids::COPY_PALETTE_TO_CRAM_SLOT,
                &[source, slot],
                &natural,
                &observed,
            );
        }
        save(&checked, &natural, &observed);
        assert_eq!(checked.checks.mismatches, 0);
        assert_eq!(checked.checks.nested_original_calls, 0);
        assert_eq!(checked.checks.dead_stack_differing_calls, 0);
        assert!(checked.checks.uninitialized_stack_reads.is_empty());
        assert!(checked.checks.unexercised.is_empty());
        for entry in CANDIDATES {
            assert!(
                observed
                    .get(&entry.id.metadata().address(revision))
                    .is_some_and(|summary| summary.0 > 0)
            );
        }
    }
}
