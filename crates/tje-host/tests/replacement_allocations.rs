//! Actual replacement dispatch, measured after warming the reusable play buffers.
use std::{fs, path::PathBuf};
use tje_game::ids;
use tje_host::{Host, Mode, hooks::HookRegistry};
use tje_mem::Revision;

#[path = "support/allocation_counter.rs"]
mod allocation_counter;
use allocation_counter::ALLOCATIONS;

fn rom(revision: Revision) -> Vec<u8> {
    let (key, file) = match revision {
        Revision::Rev02 => ("TJE_REV02_ROM", "rev02.bin"),
        Revision::Rev00 => ("TJE_REV00_ROM", "rev00.bin"),
    };
    let path = std::env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../roms").join(file));
    fs::read(path).unwrap()
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn warm_replacement_dispatch_allocates_zero_times_in_both_revisions() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let registry = HookRegistry::filtered(revision, &[], &[ids::GET_RANDOM_NUMBER]).unwrap();
        let mut host = Host::new(&rom(revision), Mode::Replace, registry).unwrap();
        let address = ids::GET_RANDOM_NUMBER.metadata().address(revision);
        // Authored RAM driver: JSR GetRandomNumber; BRA back to JSR.
        for (index, word) in [0x4eb9, (address >> 16) as u16, address as u16, 0x60f8]
            .into_iter()
            .enumerate()
        {
            host.context().write_u16(0xfff000 + 2 * index as u32, word);
        }
        let mut cpu = host.cpu().clone();
        cpu.set_pc(0xfff000, &mut host.context().bus());
        *host.context().cpu() = cpu;
        host.context().write_u32(0xff91e2, 1);
        for _ in 0..10 {
            host.play_frame(Default::default()).unwrap();
        }
        let before = host.checks.by_function["GetRandomNumber"].calls_replaced;
        ALLOCATIONS.with(|counter| counter.set((true, 0, 0)));
        for _ in 0..100 {
            std::hint::black_box(host.play_frame(Default::default()).unwrap());
        }
        let (_, allocations, bytes) = ALLOCATIONS.with(|counter| {
            let result = counter.get();
            counter.set((false, result.1, result.2));
            result
        });
        let replaced = host.checks.by_function["GetRandomNumber"].calls_replaced - before;
        assert!(replaced > 100_000);
        assert_eq!(
            allocations, 0,
            "{revision:?}: {allocations} allocations/{bytes} bytes for {replaced} replacements"
        );
        println!(
            "REPLACEMENT_ALLOCATIONS {revision:?}: calls={replaced} frames=100 allocations={allocations} bytes={bytes}"
        );
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn functional_noop_replacement_allocates_nothing_and_keeps_no_success_journal() {
    fn noop(_: &mut tje_host::abi::HookContext<'_>) -> tje_host::Result<Option<u32>> {
        Ok(None)
    }
    const DRIVER: u32 = 0xfff000;
    const TARGET: u32 = 0xfff010;
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
        symbol.name = "AuthoredFunctionalNoop";
        symbol.addresses = [TARGET; 2];
        symbol.sizes = [2; 2];
        symbol.abi.argument_count = 0;
        symbol.abi.returns_value = false;
        let mut registry = HookRegistry::empty(revision);
        registry
            .register_execution_contract(
                symbol,
                tje_game::execution::ExecutionClass::Functional,
                noop,
            )
            .unwrap();
        let mut host = Host::new(&rom(revision), Mode::Replace, registry).unwrap();
        for (index, word) in [0x4eb9, (TARGET >> 16) as u16, TARGET as u16, 0x60f8]
            .into_iter()
            .enumerate()
        {
            host.context().write_u16(DRIVER + 2 * index as u32, word);
        }
        host.context().write_u16(TARGET, 0x4e75);
        let mut cpu = host.cpu().clone();
        cpu.set_status_register(0x2700);
        cpu.set_pc(DRIVER, &mut host.context().bus());
        *host.context().cpu() = cpu;
        for _ in 0..10 {
            host.play_frame(Default::default()).unwrap();
        }
        assert!(host.checks.call_clocks.is_empty());
        let before = host.checks.by_function["AuthoredFunctionalNoop"].calls_replaced;
        ALLOCATIONS.with(|counter| counter.set((true, 0, 0)));
        for _ in 0..100 {
            std::hint::black_box(host.play_frame(Default::default()).unwrap());
        }
        let (_, allocations, bytes) = ALLOCATIONS.with(|counter| {
            let result = counter.get();
            counter.set((false, result.1, result.2));
            result
        });
        let replaced = host.checks.by_function["AuthoredFunctionalNoop"].calls_replaced - before;
        assert!(replaced > 100_000);
        assert_eq!(
            allocations, 0,
            "{revision:?}: {allocations} allocations/{bytes} bytes"
        );
        assert!(host.checks.call_clocks.is_empty());
        println!(
            "FUNCTIONAL_REPLACEMENT_ALLOCATIONS {revision:?}: calls={replaced} frames=100 allocations={allocations} bytes={bytes} success_receipts=0"
        );
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn reset_reinitializes_every_registered_zero_call_coverage_entry() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let selected = [
            ids::GET_RANDOM_NUMBER,
            ids::SET_CAMERA_TILE_PRIORITY_ENABLED,
            ids::SET_NEXT_SPEECH_LIFETIME,
        ];
        let registry = HookRegistry::filtered(revision, &[], &selected).unwrap();
        let mut host = Host::new(&rom(revision), Mode::Check, registry).unwrap();
        host.context().write_u32(0xff91e2, 1);
        host.check_fixture(selected[0].metadata().address(revision), &[])
            .unwrap();
        assert_eq!(host.checks.unexercised.len(), 2);
        host.reset();
        assert_eq!(host.checks.by_function.len(), selected.len());
        assert_eq!(host.checks.unexercised.len(), selected.len());
        assert_eq!(host.checks.calls, 0);
        assert_eq!(host.checks.by_subsystem.len(), tje_game::SUBSYSTEMS.len());
        for id in selected {
            let name = id.metadata().name;
            let coverage = &host.checks.by_function[name];
            assert_eq!(coverage.calls_checked, 0);
            assert_eq!(coverage.calls_replaced, 0);
            assert_eq!(coverage.mismatches, 0);
            assert!(host.checks.unexercised.contains(name));
            assert!(
                host.checks.by_subsystem[id.metadata().subsystem]
                    .unexercised
                    .contains(name)
            );
        }
    }
}
