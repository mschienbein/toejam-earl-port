//! Authored machine-code fixtures, not cartridge instruction/data listings.
//! Regression counterparts of HostReview HR01-HR07.
use m68000_emu::BusInterface;
use std::{fs, path::PathBuf};
use tje_game::{Context, FunctionMetadata, ids};
use tje_host::{Host, HostError, Mode, abi::HookContext, hooks::HookRegistry, input::InputScript};
use tje_mem::Revision;

const CODE: u32 = 0xFFF000;
fn bytes(revision: Revision) -> Vec<u8> {
    let (key, name) = match revision {
        Revision::Rev02 => ("TJE_REV02_ROM", "rev02.bin"),
        Revision::Rev00 => ("TJE_REV00_ROM", "rev00.bin"),
    };
    let path = std::env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../roms").join(name));
    fs::read(path).unwrap()
}
fn contract(
    name: &'static str,
    address: u32,
    returns: bool,
    arguments: u8,
    atomic: bool,
) -> FunctionMetadata {
    let mut contract = ids::GET_RANDOM_NUMBER.metadata().clone();
    contract.name = name;
    contract.addresses = [address; 2];
    contract.sizes = [32; 2];
    contract.source = "authored synthetic test routine";
    contract.atomic_capable = atomic;
    contract.abi.argument_count = arguments;
    contract.abi.returns_value = returns;
    contract
}
fn code(host: &mut Host, address: u32, words: &[u16]) {
    for (index, word) in words.iter().enumerate() {
        host.context().write_u16(address + 2 * index as u32, *word);
    }
}
fn fixture_host(
    revision: Revision,
    contract: FunctionMetadata,
    hook: tje_host::hooks::HookFn,
) -> Host {
    let mut registry = HookRegistry::empty(revision);
    registry.register_contract(contract, hook).unwrap();
    Host::new(&bytes(revision), Mode::Check, registry).unwrap()
}

#[test]
#[ignore = "requires both private ROMs"]
fn authored_abi_slots_narrow_results_and_banked_stack() {
    fn byte(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        Ok(Some(
            (c.data_register(0) & 0xffffff00) | (c.argument_u32(0) & 255),
        ))
    }
    fn short(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        Ok(Some(
            (c.data_register(0) & 0xffff0000) | (c.argument_u32(1) & 65535),
        ))
    }
    fn long(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        Ok(Some(c.argument_u32(2)))
    }
    fn incorrect(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        Ok(Some(c.argument_u32(0) & 255))
    }
    fn bad_usp(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let a = c.cpu().address_registers();
        let s = c.cpu().supervisor_stack_pointer();
        let u = c.cpu().user_stack_pointer();
        c.cpu().set_address_registers(a, u.wrapping_add(2), s);
        Ok(None)
    }
    let arguments = [0x11223380, 0x4455ff81, 0x89abcdef];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (name, hook, instructions, expected) in [
            (
                "ByteSlot",
                byte as tje_host::hooks::HookFn,
                vec![0x102f, 7, 0x4e75],
                0xa1b2c380,
            ),
            ("ShortSlot", short, vec![0x302f, 10, 0x4e75], 0xa1b2ff81),
            ("LongSlot", long, vec![0x202f, 12, 0x4e75], 0x89abcdef),
        ] {
            let mut host = fixture_host(revision, contract(name, CODE, true, 3, true), hook);
            code(&mut host, CODE, &instructions);
            let mut d = host.context().cpu().data_registers();
            d[0] = 0xa1b2c3d4;
            host.context().cpu().set_data_registers(d);
            let sp = host.cpu().stack_pointer();
            assert_eq!(host.check_call(CODE, &arguments).unwrap(), expected);
            assert_eq!(host.cpu().stack_pointer(), sp);
        }
        let mut host = fixture_host(
            revision,
            contract("BadByteResult", CODE, true, 3, true),
            incorrect,
        );
        code(&mut host, CODE, &[0x102f, 7, 0x4e75]);
        let mut d = host.context().cpu().data_registers();
        d[0] = 0xa1b2c3d4;
        host.context().cpu().set_data_registers(d);
        assert!(matches!(
            host.check_fixture(CODE, &arguments),
            Err(HostError::Mismatch { .. })
        ));
        let mut host = fixture_host(
            revision,
            contract("InactiveUsp", CODE, false, 0, true),
            bad_usp,
        );
        code(&mut host, CODE, &[0x4e75]);
        let error = host.check_fixture(CODE, &[]).unwrap_err();
        assert!(error.to_string().contains("banked USP/SSP"), "{error}");
    }
}

#[test]
#[ignore = "requires both private ROMs"]
fn immediate_mixed_width_bus_and_nested_sentinel() {
    fn mixed(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        c.bus().write_word(0xc00004, 0x8701);
        c.bus().write_byte(0xc00011, 0x9f);
        Ok(None)
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = fixture_host(
            revision,
            contract("MixedWidths", CODE, false, 0, true),
            mixed,
        );
        code(
            &mut host,
            CODE,
            &[
                0x33fc, 0x8701, 0x00c0, 4, 0x13fc, 0x009f, 0x00c0, 0x0011, 0x4e75,
            ],
        );
        host.check_fixture(CODE, &[]).unwrap();
        assert_eq!(host.checks.mismatches, 0);
        // Authored outer function calls an inner RTS, then returns 42 rather than 17.
        code(&mut host, CODE, &[0x4eb9, 0x00ff, 0xf080, 0x702a, 0x4e75]);
        code(&mut host, CODE + 0x80, &[0x7011, 0x4e75]);
        let outer = contract("OuterSentinel", CODE, true, 0, true);
        let (sp, pc) = (host.cpu().stack_pointer(), host.cpu().pc());
        assert_eq!(
            host.context()
                .call_original_with_contract(&outer, &[])
                .unwrap(),
            42
        );
        assert_eq!((host.cpu().stack_pointer(), host.cpu().pc()), (sp, pc));
    }
}

#[test]
#[ignore = "requires both private ROMs"]
fn sequential_dead_stack_composition_fails_approval() {
    fn omitted(_: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        Ok(None)
    }
    fn previous(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let sp = c.cpu().stack_pointer();
        Ok(Some(c.read_u32(sp - 4)))
    }
    let a = contract("WritesPreviousLocal", CODE, false, 0, true);
    let b = contract("ReadsPreviousLocal", CODE + 0x20, true, 0, true);
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut registry = HookRegistry::empty(revision);
        registry.register_contract(a.clone(), omitted).unwrap();
        registry.register_contract(b.clone(), previous).unwrap();
        for mode in [Mode::Check, Mode::Reference, Mode::Replace] {
            let mut host = Host::new(&bytes(revision), mode, registry.clone()).unwrap();
            code(&mut host, CODE, &[0x2f7c, 0x1234, 0x5678, 0xfffc, 0x4e75]);
            code(&mut host, CODE + 0x20, &[0x202f, 0xfffc, 0x4e75]);
            code(
                &mut host,
                CODE + 0x100,
                &[
                    0x4eb9, 0x00ff, 0xf000, 0x4eb9, 0x00ff, 0xf020, 0x23c0, 0x00ff, 0xe000, 0x60fe,
                ],
            );
            let mut cpu = host.cpu().clone();
            cpu.set_pc(CODE + 0x100, &mut host.context().bus());
            *host.context().cpu() = cpu;
            let result = host.run_frame(InputScript::default().at(0));
            if mode == Mode::Check {
                let error = result.unwrap_err();
                assert!(matches!(error, HostError::Mismatch { .. }), "{error}");
                assert!(error.to_string().contains("uninitialized stack read"));
                assert_eq!(host.checks.calls, 2);
                assert_eq!(host.checks.mismatches, 1);
                assert_eq!(host.checks.uninitialized_stack_reads.len(), 4);
            } else {
                result.unwrap();
                assert_eq!(
                    host.context().read_u32(0xffe000),
                    if mode == Mode::Reference {
                        0x12345678
                    } else {
                        0
                    }
                );
            }
        }
    }
}

#[test]
fn non_atomic_contracts_are_rejected_before_execution() {
    fn never(_: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        panic!("non-atomic hook ran")
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        let catalogue = tje_host::catalogue::FunctionCatalogue::new(revision);
        for id in [
            ids::SETUP_YM2612,
            ids::WAIT_FOR_NEXT_V_BLANK,
            ids::FURTHER_INITIALISATION,
        ] {
            let mut registry = HookRegistry::empty(revision);
            let error = registry
                .register(&catalogue, id.metadata().name, never)
                .unwrap_err();
            assert!(
                matches!(error, HostError::NotAtomicCapable { .. }),
                "{error}"
            );
        }
        let mut registry = HookRegistry::empty(revision);
        assert!(matches!(
            registry
                .register_contract(contract("StopNeedsInterrupt", CODE, false, 0, false), never),
            Err(HostError::NotAtomicCapable { .. })
        ));
    }
}

#[test]
fn registers_650_explicit_contracts_without_demo_table() {
    fn never(_: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        unreachable!()
    }
    let mut registry = HookRegistry::empty(Revision::Rev02);
    for index in 0..650 {
        registry
            .register_contract(
                contract("AuthoredHook", 0x100000 + index * 4, false, 0, true),
                never,
            )
            .unwrap();
    }
    assert_eq!(registry.len(), 650);
    assert!(registry.at(0x100000 + 649 * 4).is_some());
}
