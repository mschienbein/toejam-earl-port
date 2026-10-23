//! Filtered entry hooks retain full Rust-callee dispatch and honest coverage.
use std::{fs, path::PathBuf};
use tje_game::ids;
use tje_host::{Host, Mode, hooks::HookRegistry};
use tje_mem::Revision;

#[test]
#[ignore = "requires both private retail ROMs"]
fn filtered_live_fixtures_report_subsystems_and_unexercised_entries() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (key, file) = match revision {
            Revision::Rev02 => ("TJE_REV02_ROM", "rev02.bin"),
            Revision::Rev00 => ("TJE_REV00_ROM", "rev00.bin"),
        };
        let path = std::env::var_os(key)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("../../roms").join(file));
        let registry = HookRegistry::filtered(
            revision,
            &["camera", "speech"],
            &[ids::PRE_GENERATE_RANDOMS],
        )
        .unwrap();
        let mut host = Host::new(&fs::read(path).unwrap(), Mode::Check, registry).unwrap();
        assert_eq!(host.checks.by_subsystem.len(), tje_game::SUBSYSTEMS.len());
        assert_eq!(host.checks.unexercised.len(), 3);
        assert_eq!(host.checks.by_subsystem["runtime"].registered_functions, 0);
        assert_eq!(host.checks.by_subsystem["map"].registered_functions, 1);
        for (id, arguments) in [
            (ids::PRE_GENERATE_RANDOMS, vec![]),
            (ids::SET_CAMERA_TILE_PRIORITY_ENABLED, vec![u32::MAX]),
            (ids::SET_NEXT_SPEECH_LIFETIME, vec![u32::MAX]),
        ] {
            host.check_fixture(id.metadata().address(revision), &arguments)
                .unwrap();
        }
        assert_eq!(host.checks.calls, 3);
        assert_eq!(host.checks.mismatches, 0);
        assert_eq!(host.checks.nested_original_calls, 0);
        assert!(!host.checks.by_function.contains_key("GetRandomNumber"));
        assert!(host.checks.unexercised.is_empty());
        let mut calls = 0;
        let mut registered = 0;
        for coverage in host.checks.by_subsystem.values() {
            calls += coverage.calls_checked;
            registered += coverage.registered_functions;
            assert_eq!(coverage.mismatches, 0);
            assert!(coverage.unexercised.is_empty());
        }
        assert_eq!(calls, host.checks.calls);
        assert_eq!(registered, 3);
        println!(
            "FILTERED_COVERAGE {}",
            serde_json::json!({"revision": revision.name(), "checks": host.checks})
        );
    }
}
