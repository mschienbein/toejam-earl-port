//! Newly authored 68000 routines isolate real versus discarded stack reads.
use std::{fs, path::PathBuf};
use tje_game::ids;
use tje_host::{Host, HostError, Mode, abi::HookContext, hooks::HookRegistry};
use tje_mem::Revision;

fn no_effect(_: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    Ok(None)
}
fn fixture(revision: Revision, words: &[u16]) -> Host {
    let (key, file) = match revision {
        Revision::Rev02 => ("TJE_REV02_ROM", "rev02.bin"),
        Revision::Rev00 => ("TJE_REV00_ROM", "rev00.bin"),
    };
    let path = std::env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../roms").join(file));
    let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
    symbol.name = "AuthoredSemanticReadProbe";
    symbol.addresses = [0xfff000; 2];
    symbol.abi.returns_value = false;
    let mut registry = HookRegistry::empty(revision);
    registry.register_contract(symbol, true, no_effect).unwrap();
    let mut host = Host::new(&fs::read(path).unwrap(), Mode::Check, registry).unwrap();
    for (index, word) in words.iter().enumerate() {
        host.context().write_u16(0xfff000 + 2 * index as u32, *word);
    }
    host
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn clr_dummy_read_is_not_a_dependency_but_a_real_preceding_read_is() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        // LINK A6,#-4; CLR.W -2(A6); MOVE.W -2(A6),D0; UNLK A6; RTS.
        let mut valid = fixture(
            revision,
            &[
                0x4e56, 0xfffc, 0x426e, 0xfffe, 0x302e, 0xfffe, 0x4e5e, 0x4e75,
            ],
        );
        valid.check_fixture(0xfff000, &[]).unwrap();
        assert!(valid.checks.uninitialized_stack_reads.is_empty());
        // A real MOVE before CLR reads precisely the same still-unwritten bytes.
        let mut invalid = fixture(
            revision,
            &[
                0x4e56, 0xfffc, 0x302e, 0xfffe, 0x426e, 0xfffe, 0x4e5e, 0x4e75,
            ],
        );
        let error = invalid.check_fixture(0xfff000, &[]).unwrap_err();
        assert!(matches!(error, HostError::Mismatch { .. }), "{error}");
        assert_eq!(invalid.checks.uninitialized_stack_reads.len(), 2);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn movem_terminal_prefetch_is_ignored_without_whitelisting_its_stack_address() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        // LINK #-4; MOVEM.L D2-D6/A2,-(SP); MOVEM.L -28(A6),D2-D6/A2;
        // terminal word is the unused frame gap at -4(A6), not consumed data.
        let mut valid = fixture(
            revision,
            &[
                0x4e56, 0xfffc, 0x48e7, 0x3e20, 0x4cee, 0x047c, 0xffe4, 0x4e5e, 0x4e75,
            ],
        );
        valid.check_fixture(0xfff000, &[]).unwrap();
        assert!(valid.checks.uninitialized_stack_reads.is_empty());
        // The explicit MOVE after restore consumes that very word and must fail.
        let mut invalid = fixture(
            revision,
            &[
                0x4e56, 0xfffc, 0x48e7, 0x3e20, 0x4cee, 0x047c, 0xffe4, 0x302e, 0xfffc, 0x4e5e,
                0x4e75,
            ],
        );
        let error = invalid.check_fixture(0xfff000, &[]).unwrap_err();
        assert!(matches!(error, HostError::Mismatch { .. }), "{error}");
        assert_eq!(invalid.checks.uninitialized_stack_reads.len(), 2);
    }
}
