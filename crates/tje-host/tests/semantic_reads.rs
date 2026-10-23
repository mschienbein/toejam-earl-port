//! Newly authored 68000 routines isolate bus classification and value flow.
use m68000_emu::BusInterface;
use std::{fs, path::PathBuf};
use tje_game::{Context, execution::ExecutionClass, ids};
use tje_host::{
    Host, HostError, Mode,
    abi::HookContext,
    hooks::{HookFn, HookRegistry},
};
use tje_mem::{Address, RenderFrame, RenderPiece, Revision, ram};

const CODE: u32 = 0xfff000;
const LIVE_RAM: u32 = 0xffe000;

fn no_effect(_: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
    Ok(None)
}
fn bytes(revision: Revision) -> Vec<u8> {
    let (key, file) = match revision {
        Revision::Rev02 => ("TJE_REV02_ROM", "rev02.bin"),
        Revision::Rev00 => ("TJE_REV00_ROM", "rev00.bin"),
    };
    let path = std::env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../roms").join(file));
    fs::read(path).unwrap()
}

fn fixture(revision: Revision, words: &[u16], returns: bool, hook: HookFn) -> Host {
    fixture_with_execution(revision, words, returns, hook, ExecutionClass::Atomic)
}

fn fixture_with_execution(
    revision: Revision,
    words: &[u16],
    returns: bool,
    hook: HookFn,
    class: ExecutionClass,
) -> Host {
    let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
    symbol.name = "AuthoredSemanticReadProbe";
    symbol.addresses = [CODE; 2];
    symbol.abi.returns_value = returns;
    let mut registry = HookRegistry::empty(revision);
    registry
        .register_execution_contract(symbol, class, hook)
        .unwrap();
    let mut host = Host::new(&bytes(revision), Mode::Check, registry).unwrap();
    for (index, word) in words.iter().enumerate() {
        host.context().write_u16(0xfff000 + 2 * index as u32, *word);
    }
    // check_fixture pushes its return address immediately below the entry SP.
    // Initialize our candidate bytes, not an assumed power-on RAM value.
    let entry_sp = host.cpu().stack_pointer() - 4;
    host.context().write_u16(entry_sp - 2, 0);
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
            false,
            no_effect,
        );
        valid.check_fixture(0xfff000, &[]).unwrap();
        assert!(valid.checks.uninitialized_stack_reads.is_empty());
        // Returning D0 makes this real MOVE observable, even though CLR follows.
        let mut invalid = fixture(
            revision,
            &[
                0x4e56, 0xfffc, 0x302e, 0xfffe, 0x426e, 0xfffe, 0x4e5e, 0x4e75,
            ],
            true,
            no_effect,
        );
        let error = invalid.check_fixture(0xfff000, &[]).unwrap_err();
        assert!(matches!(error, HostError::Mismatch { .. }), "{error}");
        assert_eq!(invalid.checks.uninitialized_stack_reads.len(), 2);
        assert!(error.to_string().contains("original dependency"), "{error}");
        assert_eq!(invalid.checks.stack_dependency_reruns, 2);
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
            false,
            no_effect,
        );
        valid.check_fixture(0xfff000, &[]).unwrap();
        assert!(valid.checks.uninitialized_stack_reads.is_empty());
        // Returning D0 makes the explicit MOVE consume that same frame-gap word.
        let mut invalid = fixture(
            revision,
            &[
                0x4e56, 0xfffc, 0x48e7, 0x3e20, 0x4cee, 0x047c, 0xffe4, 0x302e, 0xfffc, 0x4e5e,
                0x4e75,
            ],
            true,
            no_effect,
        );
        let error = invalid.check_fixture(0xfff000, &[]).unwrap_err();
        assert!(matches!(error, HostError::Mismatch { .. }), "{error}");
        assert_eq!(invalid.checks.uninitialized_stack_reads.len(), 2);
        assert!(error.to_string().contains("original dependency"), "{error}");
        assert_eq!(invalid.checks.stack_dependency_reruns, 2);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn discarded_word_high_byte_is_informational_on_either_side() {
    fn constant(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        c.write_u16(LIVE_RAM, 0xff80);
        Ok(None)
    }
    fn discarded_high(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let address = c.cpu().stack_pointer() - 2;
        c.write_u8(address + 1, 0x80);
        let word = c.read_u16(address);
        c.write_u16(LIVE_RAM, word as u8 as i8 as i16 as u16);
        Ok(None)
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        for class in [
            ExecutionClass::Atomic,
            ExecutionClass::Functional,
            ExecutionClass::TimeAware,
        ] {
            // LINK #-2; write only the low local byte; read the whole word;
            // EXT.W D1 discards the unwritten high byte before the live store.
            let mut original = fixture_with_execution(
                revision,
                &[
                    0x4e56, 0xfffe, 0x1d7c, 0x0080, 0xffff, 0x322e, 0xfffe, 0x4881, 0x33c1, 0x00ff,
                    0xe000, 0x4e5e, 0x4e75,
                ],
                false,
                constant,
                class,
            );
            original.check_fixture(CODE, &[]).unwrap();
            assert!(original.checks.uninitialized_stack_reads.is_empty());
            assert_eq!(original.checks.nondependent_stack_reads.len(), 1);
            assert_eq!(original.checks.stack_dependency_reruns, 2);
            assert_eq!(original.checks.calls, 1);

            // Only Rust reads an unwritten byte; the original writes a constant.
            let mut rust = fixture_with_execution(
                revision,
                &[0x33fc, 0xff80, 0x00ff, 0xe000, 0x4e75],
                false,
                discarded_high,
                class,
            );
            rust.check_fixture(CODE, &[]).unwrap();
            assert!(rust.checks.uninitialized_stack_reads.is_empty());
            assert_eq!(rust.checks.nondependent_stack_reads.len(), 1);
            assert_eq!(rust.checks.stack_dependency_reruns, 2);
            assert_eq!(rust.checks.calls, 1);
            assert_eq!(rust.measurements.check_original_wait_frames, 0);
            assert_eq!(rust.measurements.check_rust_wait_frames, 0);
        }
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn prepare_entity_sprite_frame_ca36_discarded_high_byte_passes() {
    let id = ids::PREPARE_ENTITY_SPRITE_FRAME;
    let translation = *tje_game::translations()
        .chain(tje_game::sprites::PENDING_TRANSLATIONS)
        .find(|translation| translation.id == id)
        .unwrap();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut registry = HookRegistry::empty(revision);
        registry
            .register_translation_with_execution(
                translation,
                tje_game::execution_class(id).unwrap(),
            )
            .unwrap();
        let mut host = Host::new(&bytes(revision), Mode::Check, registry).unwrap();
        // Synthetic reuse input: no decoder, clipping, masking or upload callee.
        let entity = ram::g_player_info(revision).at(39);
        entity.entity_index().write(&mut host.context(), 6);
        entity.tile().write(&mut host.context(), 640);
        entity
            .primary_sprites()
            .at(0)
            .write(&mut host.context(), 25);
        let frame = Address::<RenderFrame>::new(
            ram::g_sprite_tile_upload_buffer(revision).address() + 7168,
        );
        frame.piece_count().write(&mut host.context(), 1);
        let piece = Address::<RenderPiece>::new(frame.address() + 4);
        piece.width().write(&mut host.context(), 1);
        piece.height().write(&mut host.context(), 1);
        piece.tile_format().write(&mut host.context(), 0);
        host.check_fixture(
            id.metadata().address(revision),
            &[entity.address(), frame.address(), 1, 0],
        )
        .unwrap();
        let read_pc = match revision {
            Revision::Rev02 => 0xca36,
            Revision::Rev00 => 0xca16,
        };
        assert!(
            host.checks
                .nondependent_stack_reads
                .iter()
                .any(|site| site.pc == read_pc),
            "{:?}",
            host.checks.nondependent_stack_reads
        );
        assert!(host.checks.uninitialized_stack_reads.is_empty());
        assert_eq!(host.checks.calls, 1);
        assert_eq!(host.checks.mismatches, 0);
        assert_eq!(host.checks.nested_original_calls, 0);
        assert!(host.checks.stack_dependency_reruns >= 2);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn live_ram_preserved_registers_and_device_traces_retain_true_dependencies() {
    fn live(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let address = c.cpu().stack_pointer() - 2;
        let value = c.read_u16(address);
        c.write_u16(LIVE_RAM, value);
        Ok(None)
    }
    fn register(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let address = c.cpu().stack_pointer() - 2;
        let value = c.read_u16(address);
        let high = c.data_register(2) & 0xffff0000;
        c.set_data_register(2, high | u32::from(value));
        Ok(None)
    }
    fn device(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let address = c.cpu().stack_pointer() - 2;
        let value = c.read_u16(address);
        c.bus().write_word(0xc00010, value);
        Ok(None)
    }
    let cases: [(&[u16], HookFn, &str); 3] = [
        (
            &[0x302f, 0xfffe, 0x33c0, 0x00ff, 0xe000, 0x4e75],
            live,
            "RAM FFE000",
        ),
        (&[0x342f, 0xfffe, 0x4e75], register, "ABI-preserved D2"),
        (
            &[0x302f, 0xfffe, 0x33c0, 0x00c0, 0x0010, 0x4e75],
            device,
            "trace differs",
        ),
    ];
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (words, hook, detail) in cases {
            let mut host = fixture(revision, words, false, hook);
            let error = host.check_fixture(CODE, &[]).unwrap_err();
            assert!(matches!(error, HostError::Mismatch { .. }), "{error}");
            assert!(error.to_string().contains("original dependency"), "{error}");
            assert!(error.to_string().contains(detail), "{error}");
            assert!(host.checks.nondependent_stack_reads.is_empty());
            assert_eq!(host.checks.stack_dependency_reruns, 4);
            assert_eq!(host.checks.calls, 1);
            assert_eq!(host.checks.mismatches, 1);
        }
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn dependency_checks_compare_ram_at_device_writes_not_just_final_ram() {
    fn transient(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let address = c.cpu().stack_pointer() - 2;
        let value = c.read_u16(address);
        c.write_u16(LIVE_RAM, value);
        c.bus().write_byte(0xc00011, 0x9f);
        c.write_u16(LIVE_RAM, 0);
        Ok(None)
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = fixture(
            revision,
            &[
                0x302f, 0xfffe, 0x33c0, 0x00ff, 0xe000, 0x13fc, 0x009f, 0x00c0, 0x0011, 0x4279,
                0x00ff, 0xe000, 0x4e75,
            ],
            false,
            transient,
        );
        let error = host.check_fixture(CODE, &[]).unwrap_err();
        assert!(
            error.to_string().contains("differs at device access"),
            "{error}"
        );
        assert_eq!(host.checks.stack_dependency_reruns, 4);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn bytewise_probes_catch_cancellation_that_collective_patterns_cannot() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        // Return signed (old_high_byte - old_low_byte). The baseline and both
        // collective XOR patterns produce zero, but individual changes do not.
        let mut host = fixture(
            revision,
            &[0x102f, 0xfffe, 0x902f, 0xffff, 0x4880, 0x48c0, 0x4e75],
            true,
            no_effect,
        );
        let error = host.check_fixture(CODE, &[]).unwrap_err();
        assert!(error.to_string().contains("original dependency"), "{error}");
        assert!(error.to_string().contains("D0:"), "{error}");
        assert_eq!(host.checks.stack_dependency_reruns, 4);
        assert_eq!(host.checks.uninitialized_stack_reads.len(), 2);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn newly_exposed_unwritten_bytes_are_discharge_candidates_too() {
    fn discarded_branch(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let sp = c.cpu().stack_pointer();
        if c.read_u8(sp - 1) != 0 {
            let _discarded = c.read_u8(sp - 2);
        }
        Ok(None)
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut host = fixture(revision, &[0x4e75], false, discarded_branch);
        host.check_fixture(CODE, &[]).unwrap();
        assert!(host.checks.uninitialized_stack_reads.is_empty());
        assert_eq!(host.checks.nondependent_stack_reads.len(), 2);
        assert_eq!(host.checks.stack_dependency_reruns, 8);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn nonreturning_dependency_probe_fails_closed_as_a_check_mismatch() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        // A zero original local returns; either perturbation loops forever.
        let mut host = fixture(
            revision,
            &[0x302f, 0xfffe, 0x66fe, 0x4e75],
            false,
            no_effect,
        );
        let error = host.check_fixture(CODE, &[]).unwrap_err();
        assert!(matches!(error, HostError::Mismatch { .. }), "{error}");
        assert!(
            error
                .to_string()
                .contains("dependency probe did not complete"),
            "{error}"
        );
        assert_eq!(host.checks.stack_dependency_reruns, 2);
        assert!(host.checks.nondependent_stack_reads.is_empty());
        assert_eq!(host.checks.uninitialized_stack_reads.len(), 2);
        assert_eq!(host.checks.mismatches, 1);
    }
}

#[test]
#[ignore = "requires both private retail ROMs"]
fn rust_nested_original_reads_are_perturbed_at_their_own_stack_addresses() {
    fn nested(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
        symbol.addresses = [CODE + 0x80; 2];
        symbol.abi.argument_count = 0;
        symbol.abi.returns_value = false;
        c.call_original_with_contract(&symbol, true, &[])?;
        Ok(None)
    }
    fn nested_return(c: &mut HookContext<'_>) -> tje_host::Result<Option<u32>> {
        let mut symbol = ids::GET_RANDOM_NUMBER.metadata().clone();
        symbol.addresses = [CODE + 0x80; 2];
        symbol.abi.argument_count = 0;
        Ok(Some(c.call_original_with_contract(&symbol, true, &[])?))
    }
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut independent = fixture(revision, &[0x4e75], false, nested);
        for (index, word) in [
            0x4e56, 0xfffe, 0x1d7c, 0x0080, 0xffff, 0x322e, 0xfffe, 0x4881, 0x4e5e, 0x4e75,
        ]
        .into_iter()
        .enumerate()
        {
            independent
                .context()
                .write_u16(CODE + 0x80 + 2 * index as u32, word);
        }
        independent.check_fixture(CODE, &[]).unwrap();
        assert!(independent.checks.uninitialized_stack_reads.is_empty());
        assert_eq!(independent.checks.nondependent_stack_reads.len(), 1);
        assert_eq!(independent.checks.stack_dependency_reruns, 2);
        assert_eq!(independent.checks.nested_original_calls, 1);

        let mut dependent = fixture(revision, &[0x7000, 0x4e75], true, nested_return);
        for (index, word) in [0x302f, 0xfffe, 0x4e75].into_iter().enumerate() {
            dependent
                .context()
                .write_u16(CODE + 0x80 + 2 * index as u32, word);
        }
        let error = dependent.check_fixture(CODE, &[]).unwrap_err();
        assert!(error.to_string().contains("Rust dependency"), "{error}");
        assert!(dependent.checks.nondependent_stack_reads.is_empty());
        assert_eq!(dependent.checks.uninitialized_stack_reads.len(), 2);
        assert_eq!(dependent.checks.stack_dependency_reruns, 2);
        assert_eq!(dependent.checks.nested_original_calls, 1);
    }
}
