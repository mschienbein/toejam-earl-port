//! Authored machine-code fixtures, not cartridge instruction/data listings.
//! Regression counterparts of HostReview HR01-HR07.
use m68000_emu::BusInterface;
use std::{fs, path::PathBuf};
use tje_game::{
    Context, FunctionMetadata, Translation,
    call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments},
    execution::ExecutionClass,
    ids,
};
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
) -> (FunctionMetadata, bool) {
    let mut contract = ids::GET_RANDOM_NUMBER.metadata().clone();
    contract.name = name;
    contract.addresses = [address; 2];
    contract.sizes = [32; 2];
    contract.source = "authored synthetic test routine";
    contract.abi.argument_count = arguments;
    contract.abi.returns_value = returns;
    (contract, atomic)
}
fn code(host: &mut Host, address: u32, words: &[u16]) {
    for (index, word) in words.iter().enumerate() {
        host.context().write_u16(address + 2 * index as u32, *word);
    }
}
fn fixture_host(
    revision: Revision,
    contract: (FunctionMetadata, bool),
    hook: tje_host::hooks::HookFn,
) -> Host {
    let mut registry = HookRegistry::empty(revision);
    registry
        .register_contract(contract.0, contract.1, hook)
        .unwrap();
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
                .call_original_with_contract(&outer.0, outer.1, &[])
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
        registry
            .register_contract(a.0.clone(), a.1, omitted)
            .unwrap();
        registry
            .register_contract(b.0.clone(), b.1, previous)
            .unwrap();
        for mode in [Mode::Check, Mode::Reference, Mode::Replace] {
            let attempt = Host::new(&bytes(revision), mode, registry.clone());
            if mode == Mode::Replace {
                let error = attempt.err().expect("instrumentation cannot dispatch live");
                assert!(error.to_string().contains("canonical production bodies"));
                continue;
            }
            let mut host = attempt.unwrap();
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
                assert!(host.checks.nondependent_stack_reads.is_empty());
                assert_eq!(host.checks.stack_dependency_reruns, 4);
                assert!(error.to_string().contains("original dependency"), "{error}");
            } else {
                result.unwrap();
                assert_eq!(host.context().read_u32(0xffe000), 0x12345678);
            }
        }
    }
}

#[test]
#[ignore = "requires both private ROMs"]
fn rust_only_dead_stack_reads_cannot_hide_behind_a_constant_original_result() {
    fn omitted(_: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        Ok(None)
    }
    fn inherent(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let address = c.cpu().stack_pointer().wrapping_sub(4);
        Ok(Some(c.read_u32(address)))
    }
    fn view(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let address = 0xef0000 | (c.cpu().stack_pointer().wrapping_sub(4) & 0xffff);
        Ok(Some(tje_mem::Address::<u32>::new(address).read(c)))
    }
    fn facade(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let address = 0xe00000 | (c.cpu().stack_pointer().wrapping_sub(4) & 0xffff);
        let high = c.bus().read_word(address);
        let low = c.bus().read_word(address + 2);
        Ok(Some((u32::from(high) << 16) | u32::from(low)))
    }
    fn initialized(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let address = c.cpu().stack_pointer().wrapping_sub(4);
        c.write_u32(address, 0x12345678);
        Ok(Some(tje_mem::Address::<u32>::new(address).read(c)))
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        for read in [inherent as tje_host::hooks::HookFn, view, facade] {
            let mut registry = HookRegistry::empty(revision);
            let (a, atomic) = contract("OmittedDeadStore", CODE, false, 0, true);
            registry.register_contract(a, atomic, omitted).unwrap();
            let (b, atomic) = contract("ConstantOriginalRustStackRead", CODE + 0x20, true, 0, true);
            registry.register_contract(b, atomic, read).unwrap();
            for mode in [Mode::Check, Mode::Reference, Mode::Replace] {
                let attempt = Host::new(&bytes(revision), mode, registry.clone());
                if mode == Mode::Replace {
                    let error = attempt.err().expect("instrumentation cannot dispatch live");
                    assert!(error.to_string().contains("canonical production bodies"));
                    continue;
                }
                let mut host = attempt.unwrap();
                code(&mut host, CODE, &[0x2f7c, 0x1234, 0x5678, 0xfffc, 0x4e75]);
                // Original B returns a constant; only the incorrect Rust body reads stack.
                code(&mut host, CODE + 0x20, &[0x203c, 0x1234, 0x5678, 0x4e75]);
                code(
                    &mut host,
                    CODE + 0x100,
                    &[
                        0x4eb9, 0x00ff, 0xf000, 0x4eb9, 0x00ff, 0xf020, 0x23c0, 0x00ff, 0xe000,
                        0x60fe,
                    ],
                );
                let mut cpu = host.cpu().clone();
                cpu.set_pc(CODE + 0x100, &mut host.context().bus());
                *host.context().cpu() = cpu;
                let result = host.run_frame(InputScript::default().at(0));
                if mode == Mode::Check {
                    let error = result.unwrap_err();
                    assert!(
                        error.to_string().contains("uninitialized stack read"),
                        "{error}"
                    );
                    assert_eq!(host.checks.calls, 2);
                    assert_eq!(host.checks.mismatches, 1);
                    assert_eq!(host.checks.uninitialized_stack_reads.len(), 4);
                    assert!(host.checks.nondependent_stack_reads.is_empty());
                    assert_eq!(host.checks.stack_dependency_reruns, 2);
                    assert!(error.to_string().contains("Rust dependency"), "{error}");
                } else {
                    result.unwrap();
                    assert_eq!(host.context().read_u32(0xffe000), 0x12345678);
                }
            }
        }
        let spec = contract("InitializedRustLocal", CODE, true, 0, true);
        let mut host = fixture_host(revision, spec, initialized);
        code(&mut host, CODE, &[0x203c, 0x1234, 0x5678, 0x4e75]);
        host.check_fixture(CODE, &[]).unwrap();
        assert!(host.checks.uninitialized_stack_reads.is_empty());
    }
}

#[test]
#[ignore = "requires both private ROMs"]
fn missing_reset_write_is_rejected_even_when_register_value_was_already_zero() {
    fn passthrough(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        c.call_original(ids::RESET_VDP.metadata().address(c.revision()), &[])?;
        Ok(None)
    }
    fn missing_final_write(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        // G/src/core/startup_and_main_loop.c:33 ResetVDP, omitting its final write.
        // These are peripheral register programs, not cartridge code/data bytes.
        for word in [
            0x8004, 0x8144, 0x8228, 0x8334, 0x8407, 0x857c, 0x8700, 0x8a00, 0x8b00, 0x8c89, 0x8d3f,
            0x8f02, 0x9001, 0x9100, 0x9200, 0x9302, 0x9400, 0x9500, 0x9600,
        ] {
            c.bus().write_word(0xc00004, word);
        }
        Ok(None)
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        let source = ids::RESET_VDP.metadata();
        let mut correct = fixture_host(revision, (source.clone(), true), passthrough);
        correct
            .check_fixture(source.address(revision), &[])
            .unwrap();
        assert_eq!(correct.checks.mismatches, 0);
        let mut incorrect = fixture_host(revision, (source.clone(), true), missing_final_write);
        let error = incorrect
            .check_fixture(source.address(revision), &[])
            .unwrap_err();
        assert!(matches!(error, HostError::Mismatch { .. }), "{error}");
        assert_eq!(incorrect.checks.mismatches, 1);
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
        let (contract, atomic) = contract("StopNeedsInterrupt", CODE, false, 0, false);
        assert!(matches!(
            registry.register_contract(contract, atomic, never),
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
        let (contract, atomic) = contract("AuthoredHook", 0x100000 + index * 4, false, 0, true);
        registry.register_contract(contract, atomic, never).unwrap();
    }
    assert_eq!(registry.len(), 650);
    assert!(registry.at(0x100000 + 649 * 4).is_some());
}

fn framed_abs(c: &mut HookContext<'_>, initialize: bool) -> tje_host::Result<Option<u32>> {
    let frame = LocalFrame::enter(c, FrameLayout::link(4, 0, 0))?;
    if initialize {
        frame.local::<u32>(-4).write(c, 40);
    }
    let outgoing = OutgoingArguments::existing(c, 0)?;
    let dispatcher = c.dispatcher();
    let value = outgoing.call(c, dispatcher, ids::ABS, CallSite::new(CODE + 0x80 + 18))?;
    outgoing.release(c)?;
    frame.leave(c)?;
    Ok(Some(value))
}

fn stored_framed_abs(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    framed_abs(c, true)
}

fn omitted_framed_abs(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    framed_abs(c, false)
}

fn original_reads_local_then_stores(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    let frame = LocalFrame::enter(c, FrameLayout::link(4, 0, 0))?;
    let local = frame.local::<u32>(-4);
    let (child, atomic) = contract("ReadsBeforeLaterStore", CODE + 0x100, true, 1, true);
    let value = c.call_original_with_contract(&child, atomic, &[local.address()])?;
    frame.leave(c)?;
    Ok(Some(value))
}

fn framed_sequence(
    revision: Revision,
    mode: Mode,
    hook: tje_host::hooks::HookFn,
    rust_abs: bool,
    later_store: bool,
    class: ExecutionClass,
) -> tje_host::Result<Host> {
    fn omitted(_: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        Ok(None)
    }
    static ABS: &[Translation] = &[Translation {
        id: ids::ABS,
        function: tje_game::runtime::abs,
    }];
    let mut registry =
        HookRegistry::from_translations(revision, if rust_abs { ABS } else { &[] }).unwrap();
    let (a, atomic) = contract("LeavesDeadForty", CODE, false, 0, true);
    registry.register_contract(a, atomic, omitted).unwrap();
    let (b, _) = contract("FramedLocalCaller", CODE + 0x80, true, 0, true);
    registry
        .register_execution_contract(b, class, hook)
        .unwrap();
    let mut host = Host::new(&bytes(revision), mode, registry)?;
    code(&mut host, CODE, &[0x2f7c, 0, 40, 0xfff8, 0x4e75]);
    if later_store {
        code(
            &mut host,
            CODE + 0x80,
            &[
                0x4e56, 0xfffc, 0x2d7c, 0, 40, 0xfffc, 0x486e, 0xfffc, 0x4eb9, 0x00ff, 0xf100,
                0x588f, 0x4e5e, 0x4e75,
            ],
        );
        // Incoming pointer addresses an unwritten caller local ABOVE child SP.
        // Its value is consumed before the child initializes the same bytes.
        code(
            &mut host,
            CODE + 0x100,
            &[0x206f, 4, 0x2010, 0x20bc, 0, 40, 0x4e75],
        );
    } else {
        let abs = ids::ABS.metadata().address(revision);
        code(
            &mut host,
            CODE + 0x80,
            &[
                0x4e56,
                0xfffc,
                0x2d7c,
                0,
                40,
                0xfffc,
                0x4eb9,
                (abs >> 16) as u16,
                abs as u16,
                0x4e5e,
                0x4e75,
            ],
        );
    }
    code(
        &mut host,
        CODE + 0x200,
        &[
            0x4eb9, 0x00ff, 0xf000, 0x4eb9, 0x00ff, 0xf080, 0x23c0, 0x00ff, 0xe000, 0x60fe,
        ],
    );
    let mut cpu = host.cpu().clone();
    cpu.set_pc(CODE + 0x200, &mut host.context().bus());
    *host.context().cpu() = cpu;
    Ok(host)
}

#[test]
#[ignore = "requires both private ROMs"]
fn prepared_original_and_rust_arguments_share_the_suspended_root_audit() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for class in [ExecutionClass::Atomic, ExecutionClass::TimeAware] {
            for rust_abs in [false, true] {
                for (missing, hook) in [
                    (false, stored_framed_abs as tje_host::hooks::HookFn),
                    (true, omitted_framed_abs),
                ] {
                    for mode in [Mode::Check, Mode::Reference, Mode::Replace] {
                        let attempt = framed_sequence(revision, mode, hook, rust_abs, false, class);
                        if mode == Mode::Replace {
                            let error =
                                attempt.err().expect("instrumentation cannot dispatch live");
                            assert!(error.to_string().contains("canonical production bodies"));
                            continue;
                        }
                        let mut host = attempt.unwrap();
                        let result = host.run_frame(InputScript::default().at(0));
                        if mode == Mode::Check && missing {
                            let error = result.unwrap_err();
                            assert!(matches!(error, HostError::Mismatch { .. }), "{error}");
                            assert!(error.to_string().contains("Rust dependency"), "{error}");
                            assert_eq!(host.checks.calls, 2);
                            assert_eq!(host.checks.mismatches, 1);
                            assert_eq!(host.checks.uninitialized_stack_reads.len(), 4);
                            if !rust_abs {
                                let abs = ids::ABS.metadata();
                                assert!(host.checks.uninitialized_stack_reads.iter().all(|site| {
                                    (abs.address(revision)
                                        ..abs.address(revision) + abs.size(revision))
                                        .contains(&site.pc)
                                }));
                            }
                        } else {
                            result.unwrap();
                            assert_eq!(host.checks.mismatches, 0);
                            assert!(host.checks.uninitialized_stack_reads.is_empty());
                            assert_eq!(host.context().read_u32(0xffe000), 40);
                        }
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "requires both private ROMs"]
fn nested_original_read_before_a_later_write_is_not_retroactively_initialized() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for class in [ExecutionClass::Atomic, ExecutionClass::TimeAware] {
            let mut host = framed_sequence(
                revision,
                Mode::Check,
                original_reads_local_then_stores,
                false,
                true,
                class,
            )
            .unwrap();
            let error = host.run_frame(InputScript::default().at(0)).unwrap_err();
            assert!(matches!(error, HostError::Mismatch { .. }), "{error}");
            assert!(error.to_string().contains("Rust dependency"), "{error}");
            assert_eq!(host.checks.calls, 2);
            assert_eq!(host.checks.uninitialized_stack_reads.len(), 4);
            assert!(
                host.checks
                    .uninitialized_stack_reads
                    .iter()
                    .all(|site| site.pc == CODE + 0x104)
            );
        }
    }
}

#[test]
#[ignore = "requires both private ROMs"]
fn eager_unused_framed_arguments_are_informational_not_dependencies() {
    fn caller(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let frame = LocalFrame::enter(c, FrameLayout::link(4, 0, 0))?;
        let outgoing = OutgoingArguments::existing(c, 0)?;
        let dispatcher = c.dispatcher();
        outgoing.call(
            c,
            dispatcher,
            ids::WAIT_FOR_NEXT_V_BLANK,
            CallSite::new(CODE + 0x80 + 10),
        )?;
        outgoing.release(c)?;
        frame.leave(c)?;
        Ok(Some(7))
    }
    static WAIT: &[Translation] = &[Translation {
        id: ids::WAIT_FOR_NEXT_V_BLANK,
        function: tje_game::core::wait_for_next_vblank,
    }];
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut registry = HookRegistry::from_translations(revision, WAIT).unwrap();
        let (symbol, _) = contract("UnusedFramedArgument", CODE + 0x80, true, 0, true);
        registry
            .register_execution_contract(symbol, ExecutionClass::TimeAware, caller)
            .unwrap();
        let mut host = Host::new(&bytes(revision), Mode::Check, registry).unwrap();
        // The real disabled-display fast path never consumes its count. Rust's
        // eager slot preload may read it, but perturbing it must not change output.
        tje_mem::ram::g_mode2_reg_softcopy(revision).write(&mut host.context(), 0);
        let wait = ids::WAIT_FOR_NEXT_V_BLANK.metadata().address(revision);
        code(
            &mut host,
            CODE + 0x80,
            &[
                0x4e56,
                0xfffc,
                0x4eb9,
                (wait >> 16) as u16,
                wait as u16,
                0x4e5e,
                0x7007,
                0x4e75,
            ],
        );
        host.check_fixture(CODE + 0x80, &[]).unwrap();
        assert_eq!(host.checks.mismatches, 0);
        assert!(host.checks.uninitialized_stack_reads.is_empty());
        assert!(!host.checks.nondependent_stack_reads.is_empty());
        assert!(host.checks.stack_dependency_reruns >= 2);
    }
}
