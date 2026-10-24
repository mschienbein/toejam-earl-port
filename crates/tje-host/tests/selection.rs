//! Filtered entry hooks retain full Rust-callee dispatch and honest coverage.
use std::{fs, path::PathBuf};
use tje_game::ids;
use tje_host::{Host, HostError, Mode, hooks::HookRegistry};
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
            &[],
            &[
                ids::PRE_GENERATE_RANDOMS,
                ids::SET_CAMERA_TILE_PRIORITY_ENABLED,
                ids::SET_NEXT_SPEECH_LIFETIME,
            ],
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

#[test]
#[ignore = "requires both private retail ROMs"]
fn live_constructors_reject_real_pending_and_manual_candidates_in_both_revisions() {
    fn instrumentation(_: &mut tje_host::abi::HookContext<'_>) -> tje_host::Result<Option<u32>> {
        panic!("an unapproved body reached live dispatch")
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (key, file) = match revision {
            Revision::Rev02 => ("TJE_REV02_ROM", "rev02.bin"),
            Revision::Rev00 => ("TJE_REV00_ROM", "rev00.bin"),
        };
        let path = std::env::var_os(key)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("../../roms").join(file));
        let bytes = fs::read(path).unwrap();
        let canonical = HookRegistry::filtered(revision, &[], &[ids::GET_RANDOM_NUMBER]).unwrap();
        for host in [
            Host::new(&bytes, Mode::Replace, canonical.clone()).unwrap(),
            Host::from_rom(
                tje_rom::Rom::import(&bytes).unwrap(),
                Mode::Replace,
                canonical,
            )
            .unwrap(),
        ] {
            assert_eq!(host.mode(), Mode::Replace);
            assert_eq!(host.frame_count(), 0);
            assert_eq!(host.checks.calls, 0);
        }
        let mut manual = HookRegistry::empty(revision);
        manual
            .register(
                &tje_host::catalogue::FunctionCatalogue::new(revision),
                "GetRandomNumber",
                instrumentation,
            )
            .unwrap();
        for (label, registry) in [
            (
                "ordinary core pending proof",
                HookRegistry::proof(revision, tje_game::core::PENDING_TRANSLATIONS).unwrap(),
            ),
            (
                "ordinary framed pending proof",
                HookRegistry::proof_with_frames(
                    revision,
                    &[],
                    tje_game::sprites::PENDING_FRAMED_TRANSLATIONS,
                )
                .unwrap(),
            ),
            (
                "bounded pending values",
                HookRegistry::from_entries(revision, tje_game::core::PENDING_TRANSLATIONS, &[])
                    .unwrap(),
            ),
            (
                "bounded pending frames",
                HookRegistry::from_entries(
                    revision,
                    &[],
                    tje_game::sprites::PENDING_FRAMED_TRANSLATIONS,
                )
                .unwrap(),
            ),
            ("manual instrumentation", manual),
        ] {
            let new_error = Host::new(&bytes, Mode::Replace, registry.clone())
                .err()
                .expect("pending/proof body cannot construct live Host");
            let imported_error = Host::from_rom(
                tje_rom::Rom::import(&bytes).unwrap(),
                Mode::Replace,
                registry,
            )
            .err()
            .expect("pending/proof body cannot construct live Host from Rom");
            for error in [new_error, imported_error] {
                assert!(matches!(&error, HostError::Metadata(_)), "{error}");
                assert!(
                    error.to_string().contains("canonical production bodies"),
                    "{error}"
                );
            }
            println!("REPLACEMENT_ADMISSION {revision:?} {label}: both constructors REJECTED");
        }
    }
}
