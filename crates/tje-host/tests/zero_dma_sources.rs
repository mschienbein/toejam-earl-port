//! Opt-in real-ROM proofs for the two production zero-encoded DMA roots.
//! Natural calls remain unmarked; the restricted marker starts before zero seeds.
use std::{
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::Arc,
};
use tje_game::{FunctionId, ids};
use tje_host::{
    Host, Mode, dependent_equivalence::DependentProofConfig, hooks::HookRegistry,
    input::InputScript,
};
use tje_mem::{Revision, ram, rom};

#[path = "../../tje-game/src/sprites/check_dma.rs"]
mod authored_upload;
// Preserve the author's combined fixture entry for its existing harnesses.
const _: fn(&mut Host) = authored_upload::fixtures;

fn check(host: &mut Host, id: FunctionId, arguments: &[u32]) {
    if !host.checks.by_function.contains_key(id.metadata().name) {
        return;
    }
    host.check_fixture(id.metadata().address(host.revision()), arguments)
        .unwrap_or_else(|error| panic!("{} {arguments:08x?}: {error}", id.metadata().name));
}
pub fn platform_ordinary(host: &mut Host) {
    let source = ram::g_sprite_tile_upload_buffer(host.revision()).address();
    for index in 0..32_u32 {
        host.context()
            .write_u32(source + 4 * index, 0x8000_0000 | index);
    }
    let rom_source = rom::starfield_byte_map(host.revision()).address();
    for (pointer, destination, count) in [
        (source, 0, 1),
        (source, 0xd000, 3),
        (source, 0xfffe, 64),
        (rom_source, 0xc002, 0),
    ] {
        check(
            host,
            ids::DMA_COPY_WORDS_TO_VRAM,
            &[pointer, destination, count],
        );
    }
}
pub fn paired_natural(checked: &mut Host, reference: &mut Host, script: &InputScript, frames: u64) {
    for frame in 0..frames {
        let a = checked.run_frame(script.at(frame)).unwrap();
        let b = reference.run_frame(script.at(frame)).unwrap();
        assert_eq!(a.frame_counter, b.frame_counter);
        assert_eq!(a.ram, b.ram, "RAM frame {frame}");
        assert_eq!(a.vdp, b.vdp, "VDP frame {frame}");
        assert_eq!(a.writes, b.writes, "writes frame {frame}");
        assert_eq!(a.idle_random_draws, b.idle_random_draws);
        assert_eq!(a.lag_events, b.lag_events);
        assert!(
            a.audio_samples.iter().flatten().map(|x| x.to_bits()).eq(b
                .audio_samples
                .iter()
                .flatten()
                .map(|x| x.to_bits()))
        );
        let (a, sa) = checked.frame_buffer();
        let (b, sb) = reference.frame_buffer();
        assert_eq!(sa, sb);
        assert_eq!(&a[..sa.len() as usize], &b[..sb.len() as usize]);
    }
}
fn run_case(revision: Revision, id: FunctionId, frames: u64, directory: &Path) -> bool {
    let (key, slug) = match revision {
        Revision::Rev02 => ("TJE_REV02_ROM", "rev02"),
        Revision::Rev00 => ("TJE_REV00_ROM", "rev00"),
    };
    let path = std::env::var_os(key).map(PathBuf::from).unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../roms/{slug}.bin"))
    });
    let bytes = fs::read(path).unwrap();
    let registry = HookRegistry::filtered(revision, &[], &[id]).unwrap();
    let mut checked = Host::new(&bytes, Mode::Check, registry.clone()).unwrap();
    let mut reference = Host::new(&bytes, Mode::Reference, registry).unwrap();
    let upload = id == ids::UPLOAD_QUEUED_SPRITE_GRAPHICS;
    let script = Arc::new(
        InputScript::parse(if upload {
            include_str!("../examples/moving-player.json")
        } else {
            include_str!("../examples/start-game.json")
        })
        .unwrap(),
    );
    checked.set_wait_input_poller(script.clone());
    reference.set_wait_input_poller(script.clone());
    let mut natural_calls = 0;
    let mut ordinary_calls = 0;
    let mut nonzero_rejected = false;
    let result = catch_unwind(AssertUnwindSafe(|| {
        paired_natural(&mut checked, &mut reference, &script, frames);
        assert!(checked.measurements.game_frame.is_some());
        natural_calls = checked.checks.calls;
        assert_eq!(checked.checks.mismatches, 0);
        assert!(checked.checks.dependent_equivalence.is_empty());
        let a = checked
            .settle_fixture_boundary(script.at(checked.frame_count()))
            .unwrap();
        let b = reference
            .settle_fixture_boundary(script.at(reference.frame_count()))
            .unwrap();
        assert_eq!(
            (a.start_pc, a.end_pc, a.ticks, a.master_clocks),
            (b.start_pc, b.end_pc, b.ticks, b.master_clocks)
        );
        assert_eq!(a.completed_frames.len(), b.completed_frames.len());
        if upload {
            authored_upload::ordinary_fixtures(&mut checked);
        } else {
            platform_ordinary(&mut checked);
        }
        ordinary_calls = checked.checks.calls;
        assert_eq!(ordinary_calls - natural_calls, if upload { 54 } else { 4 });
        // No clocks/PC/SP/SR/state reset and no backfilled natural approval.
        checked
            .enable_dependent_equivalence(&[id], DependentProofConfig::default())
            .unwrap();
        assert_eq!(checked.checks.calls, ordinary_calls);
        let source = ram::g_sprite_tile_upload_buffer(revision).address();
        let args = if upload {
            let runs = ram::g_sprite_tile_upload_runs(revision);
            runs.at(0).write(&mut checked.context(), 640);
            runs.at(1).write(&mut checked.context(), 1);
            ram::g_sprite_tile_upload_queue_word_count(revision).write(&mut checked.context(), 2);
            vec![0, 1]
        } else {
            vec![source, 0xc002, 1]
        };
        let error = checked
            .check_fixture(id.metadata().address(revision), &args)
            .unwrap_err();
        assert!(error.to_string().contains("nonzero"), "{error}");
        nonzero_rejected = true;
        assert_eq!(checked.checks.calls, ordinary_calls);
        if upload {
            authored_upload::zero_fixture(&mut checked);
        } else {
            check(&mut checked, id, &[source, 0xc002, 0]);
        }
        let proof = &checked.checks.dependent_equivalence[id.metadata().name];
        assert_eq!(proof.calls_proven, 1);
        assert_eq!(proof.calls_rejected, 0);
        assert_eq!(proof.seeded_probes_completed, 16);
        assert_eq!(proof.byte_probes_completed, 0);
        assert_eq!(proof.collective_dma_probes_completed, 4);
        assert_eq!(proof.probe_pairs_matched, 20);
        assert!(proof.sensitive_probe_pairs > 0);
        let source_evidence = proof.last_dma_source.as_ref().unwrap();
        assert_eq!(source_evidence.words_per_execution, 65536);
        assert_eq!(source_evidence.transfers_per_execution, 1);
        assert_eq!(source_evidence.raw_first_address, source);
        assert!(proof.last_dma_dependent_byte_count > 0);
        assert!(proof.last_read_trace.is_empty());
        assert!(proof.dependent_bytes.is_empty());
        assert!(checked.checks.dependent_stack_reads.is_empty());
        assert_eq!(checked.checks.calls, ordinary_calls + 1);
        assert_eq!(checked.checks.mismatches, 0);
        assert!(checked.checks.uninitialized_stack_reads.is_empty());
        if frames == 10000 {
            assert_eq!(natural_calls, if upload { 5338 } else { 5810 });
            assert_eq!(checked.checks.calls, if upload { 5393 } else { 5815 });
        }
    }));
    let error = result.err().map(|error| {
        error
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| error.downcast_ref::<&str>().map(|text| text.to_string()))
            .unwrap_or_else(|| "panic".into())
    });
    // Retain only receipts, not the multi-megabyte private RAM source trace.
    let proof = checked.checks.dependent_equivalence.get(id.metadata().name).map(|p| serde_json::json!({
        "calls_proven":p.calls_proven,"calls_rejected":p.calls_rejected,"seed":p.seed,
        "seeded_probes_completed":p.seeded_probes_completed,"byte_probes_completed":p.byte_probes_completed,
        "collective_dma_probes_completed":p.collective_dma_probes_completed,"probe_pairs_matched":p.probe_pairs_matched,
        "sensitive_probe_pairs":p.sensitive_probe_pairs,"original_runs_completed":p.original_runs_completed,
        "rust_runs_completed":p.rust_runs_completed,"dependent_bytes":p.last_dma_dependent_byte_count,
        "source":p.last_dma_source,
        "retained_read_events":p.last_read_trace.len(),
    }));
    let receipt = serde_json::json!({"revision":slug,"function":id.metadata().name,"frames":frames,
        "natural_calls":natural_calls,"ordinary_calls":ordinary_calls,"final_calls":checked.checks.calls,
        "mismatches":checked.checks.mismatches,"nonzero_rejected":nonzero_rejected,"proof":proof,"error":error,
        "producer_history":"consumer-only proof; all historical RAM producers or exact cycle-engine state remain required"});
    fs::write(
        directory.join(format!("{slug}-{}-{frames}.json", id.metadata().name)),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    println!("ZERO_DMA_SOURCE {receipt}");
    receipt["error"].is_null()
}
fn suite(frames: u64, roots: &[FunctionId]) {
    let directory = std::env::var_os("TJE_DMA_ZERO_RECEIPTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/tje-dma-zero-sources"));
    fs::create_dir_all(&directory).unwrap();
    let mut passed = true;
    for &id in roots {
        for revision in [Revision::Rev02, Revision::Rev00] {
            passed &= run_case(revision, id, frames, &directory);
        }
    }
    assert!(
        passed,
        "a real DMA source proof failed; see ZERO_DMA_SOURCE receipts"
    );
}
#[test]
#[ignore = "requires both private ROMs; live zero-count baseline/probe diagnostics"]
fn zero_dma_live_baselines_both_revisions() {
    suite(
        1300,
        &[
            ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
            ids::DMA_COPY_WORDS_TO_VRAM,
        ],
    );
}
#[test]
#[ignore = "requires both private ROMs; 10000 paired frames per production root/revision plus source proofs"]
fn production_zero_dma_sources_10000_frames_both_revisions() {
    suite(
        10000,
        &[
            ids::UPLOAD_QUEUED_SPRITE_GRAPHICS,
            ids::DMA_COPY_WORDS_TO_VRAM,
        ],
    );
}

#[test]
#[ignore = "requires both private ROMs; focused 10000-frame platform DMA source proof per revision"]
fn production_platform_zero_dma_10000_frames_both_revisions() {
    suite(10000, &[ids::DMA_COPY_WORDS_TO_VRAM]);
}
