//! Explicit original-only PreparedEntry measurements, never canonical Rust proofs.
use std::{fs, path::PathBuf, sync::Arc};
use tje_game::{Context, execution::CallCeiling, ids};
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Revision, ram, rom};

#[test]
#[ignore = "requires private ROMs and reviewed original-only measurement seam"]
fn exact_original_zero_word_copy_cost_both_revisions() {
    let script = Arc::new(
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap(),
    );
    let directory = PathBuf::from(std::env::var_os("TJE_PLATFORM_EVIDENCE_DIR").unwrap());
    let mut records = Vec::new();
    for (revision, key) in [
        (Revision::Rev02, "TJE_REV02_ROM"),
        (Revision::Rev00, "TJE_REV00_ROM"),
    ] {
        let bytes = fs::read(std::env::var_os(key).unwrap()).unwrap();
        let mut host = Host::new(&bytes, Mode::Reference, HookRegistry::empty(revision)).unwrap();
        host.set_wait_input_poller(script.clone());
        let entry = ids::MOVE_DATA_TO_VRAM.metadata().address(revision);
        host.enable_session_trace(&[entry], &[], &[], false)
            .unwrap();
        host.enable_call_observations(&[entry]).unwrap();
        for frame in 0..10_000 {
            host.run_frame(script.at(frame)).unwrap();
            host.take_call_observations();
            host.take_session_observations();
        }
        host.settle_fixture_boundary(script.at(host.frame_count()))
            .unwrap();
        // Reproduce EXACT public four-leaf fixture setup at its failed zero case.
        // Earlier isolated controller/nonzero fixtures leave these authored seeds,
        // not their speculative device/CPU results, in the live caller trajectory.
        let upper_sr = host.context().status_register() & !0x1f;
        host.context().set_data_register(1, 0xffff_0000);
        host.context().set_status_register(upper_sr | 0x10);
        let buffer = ram::g_sprite_tile_upload_buffer(revision).address();
        for index in 0..32_u32 {
            host.context()
                .write_u32(buffer + index * 4, 0x8000_0000 | index);
        }
        let caller_pc = host.cpu().pc();
        let caller_sp = host.cpu().stack_pointer();
        let source = rom::starfield_byte_map(revision).address();
        // Main-authorised per-invocation diagnostic I/T/F. This is not a game
        // policy, source bound, new class, clock charge or equivalence approval.
        let ceiling = CallCeiling {
            ticks: 16_000_000,
            frames: 512,
            reference_instructions: Some(32_000_000),
        };
        let record = host
            .measure_original_clocked_fixture(ids::MOVE_DATA_TO_VRAM, &[source, 0], ceiling)
            .unwrap();
        assert_eq!(record.purpose, "measurement");
        assert!(!record.approves_translation);
        assert_eq!(record.caller_pc, caller_pc);
        assert_eq!(record.entry_sp, caller_sp - 12);
        assert_eq!(record.return_pc, caller_pc);
        assert_eq!(record.return_sp, caller_sp - 8);
        assert_eq!(record.usage_to_rts.faulted_opcode_attempts, 0);
        assert_eq!(record.usage_to_rts.other_exception_entries, 0);
        assert_eq!(host.cpu().pc(), caller_pc);
        assert_eq!(host.cpu().stack_pointer(), caller_sp);
        let evidence = serde_json::json!({
            "scope": "measurement only; exact authored prepared-entry original; no natural-JSR/equivalence claim",
            "base": "signed bridge e3f0648; Main-authorised observer seam",
            "fixture": "10k original start-game; genuine settle; same D1/CCR-only/128-byte setup as canonical four-leaf fixture; no IRQ/device/queue reset",
            "record": record,
        });
        fs::write(
            directory.join(format!("{}-move-zero-original-cost.json", revision.name())),
            serde_json::to_vec_pretty(&evidence).unwrap(),
        )
        .unwrap();
        println!("MOVE_ZERO_ORIGINAL_COST {evidence}");
        records.push(evidence);
    }
    fs::write(directory.join("move-zero-original-cost-receipt.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "purpose":"measurement", "approves_translation":false,
        "records":records,
        "policy_rule":"only after completed both-revision measurements: double independent observed maxima, round I/T up to million and F to hundred; operational ceilings, not source bounds",
    })).unwrap()).unwrap();
}
