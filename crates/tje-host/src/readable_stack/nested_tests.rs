//! Newly authored 68000 programs plus real prepared Rust calls. No cartridge
//! instructions/data are copied, and these fixtures grant no body promotion.
use crate::{
    CheckReport, Host,
    abi::HookContext,
    catalogue::RevisionSymbols,
    fixture::tests::synthetic_host,
    hooks::{HookFn, HookRegistry},
};
use tje_game::{
    Context, FramedTranslation, Translation,
    call_frame::{self, CallSite, FrameLayout, FrameState, LocalFrame},
    execution::{ExecutionClass, ProofEnvelope},
    ids,
    stack_read::{LocalWidth, Reader, UnsupportedReason},
};
use tje_rom::Revision;

const ROOT: u32 = 0x600;
const LIVE: u32 = 0xff9000;
const CHILD: tje_game::FunctionId = ids::GET_RANDOM_NUMBER;
const PARENT: tje_game::FunctionId = ids::ABS;
const ROOT_ID: tje_game::FunctionId = ids::NO_OP;
const CHILD_CODE: &[u16] = &[
    0x4e56, 0xfff8, // LINK A6,#-8
    0x1d7c, 0x007f, 0xfffd, // MOVE.B #127,-3(A6)
    0x4280, // CLR.L D0: the authored word result is zero-extended
    0x302e, 0xfffc, // MOVE.W -4(A6),D0: own entry SP-8, mixed word
    0x33c0, 0x00ff, 0x9000, // MOVE.W D0,absolute work RAM
    0x4e5e, 0x4e75, // UNLK A6; RTS
];

fn child<const MUTATION: u8>(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(8, 0, 0))?;
    frame.local::<u8>(-3).write(context, 0x7f);
    let owner = if MUTATION == 1 { ROOT_ID } else { CHILD };
    let declared_frame = if MUTATION == 2 {
        FrameState::new(frame.entry().entry_sp.wrapping_sub(4))
    } else {
        frame.entry()
    };
    let offset = if MUTATION == 3 { 0 } else { -8 };
    if MUTATION == 4 {
        context.set_status_register(context.status_register() ^ 0x2000)
    }
    let value = context.read_uninitialized_local(declared_frame.uninitialized_local(
        owner,
        offset,
        LocalWidth::Word,
        "authored nested frame's exact mixed word at own entry SP-8",
    ))?;
    context.write_u16(LIVE, value as u16);
    frame.leave(context)?;
    Ok(Some(value))
}
fn authored_rts(_: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    Ok(None)
}
fn authored_value_rts(_: &mut dyn Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
    Ok(None)
}
fn elided_parent(context: &mut dyn Context, _: &[u32]) -> tje_game::Result<Option<u32>> {
    let site = CallSite::new(PARENT.metadata().address(context.revision()) + 6);
    let dispatcher = context.dispatcher();
    let value = call_frame::call_at(context, dispatcher, CHILD, &[], site)?;
    Ok(Some(value))
}
fn deep_parent(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(8, 0, 0))?;
    let pc = PARENT.metadata().address(context.revision());
    let dispatcher = context.dispatcher();
    call_frame::call_at_void(context, dispatcher, ROOT_ID, &[], CallSite::new(pc + 10))?;
    let value = call_frame::call_at(context, dispatcher, CHILD, &[], CallSite::new(pc + 16))?;
    frame.leave(context)?;
    Ok(Some(value))
}
static NORMAL: &[FramedTranslation] = &[FramedTranslation {
    id: CHILD,
    function: child::<0>,
}];
static WRONG_OWNER: &[FramedTranslation] = &[FramedTranslation {
    id: CHILD,
    function: child::<1>,
}];
static WRONG_FRAME: &[FramedTranslation] = &[FramedTranslation {
    id: CHILD,
    function: child::<2>,
}];
static ABOVE_FRAME: &[FramedTranslation] = &[FramedTranslation {
    id: CHILD,
    function: child::<3>,
}];
static WRONG_BANK: &[FramedTranslation] = &[FramedTranslation {
    id: CHILD,
    function: child::<4>,
}];
static DEEP: &[FramedTranslation] = &[
    FramedTranslation {
        id: CHILD,
        function: child::<0>,
    },
    FramedTranslation {
        id: PARENT,
        function: deep_parent,
    },
    FramedTranslation {
        id: ROOT_ID,
        function: authored_rts,
    },
];
static VALUE_RTS: &[Translation] = &[Translation {
    id: ROOT_ID,
    function: authored_value_rts,
}];
static ELIDED: &[Translation] = &[Translation {
    id: PARENT,
    function: elided_parent,
}];

// 0 normal; 1/2 wrong child owner/frame; 3 wrong return site; 4 forged DTO;
// 5 elided ancestor; 6 matching earlier elided sibling; 7 missing sibling;
// 8 parent reread after child return; 9 above frame; 10 bank switch; 11 deep sibling.
fn root<const VARIANT: u8>(context: &mut HookContext<'_>) -> crate::Result<Option<u32>> {
    let frame = LocalFrame::enter(context, FrameLayout::link(8, 0, 0))?;
    let dispatcher = context.dispatcher();
    if VARIANT == 4 {
        let sp = context.stack_pointer()?.wrapping_sub(4);
        context.set_stack_pointer(sp)?;
        context.set_frame_state(FrameState::new(sp))?;
        child::<0>(context)?;
    } else if VARIANT == 5 {
        dispatcher.call(context, PARENT, &[0])?;
    } else if VARIANT == 11 {
        call_frame::call_at(context, dispatcher, PARENT, &[0], CallSite::new(ROOT + 12))?;
    } else {
        if VARIANT == 6 {
            dispatcher.call_void(context, ROOT_ID, &[])?
        }
        let site = if VARIANT == 3 {
            ROOT + 12
        } else if matches!(VARIANT, 6 | 7) {
            ROOT + 16
        } else {
            ROOT + 10
        };
        call_frame::call_at(context, dispatcher, CHILD, &[], CallSite::new(site))?;
        if VARIANT == 8 {
            context.read_uninitialized_local(frame.entry().uninitialized_local(
                ROOT_ID,
                -8,
                LocalWidth::Word,
                "authored root reread after a real child return must stay held",
            ))?;
        }
    }
    frame.leave(context)?;
    Ok(None)
}
fn put(program: &mut Vec<u16>, address: u32, words: &[u16]) {
    let offset = ((address - 0x100) / 2) as usize;
    program.resize(program.len().max(offset + words.len()), 0);
    program[offset..offset + words.len()].copy_from_slice(words);
}
fn jsr(words: &mut Vec<u16>, address: u32) {
    words.extend([0x4eb9, (address >> 16) as u16, address as u16]);
}
fn fixture(revision: Revision, class: ExecutionClass, variant: u8) -> Host {
    let child_pc = CHILD.metadata().address(revision);
    let parent_pc = PARENT.metadata().address(revision);
    let noop_pc = ROOT_ID.metadata().address(revision);
    assert!(2 * CHILD_CODE.len() <= CHILD.metadata().size(revision) as usize);
    let mut program = vec![0x60fe]; // authored reset program parks at its own BRA
    let mut caller = vec![0x4e56, 0xfff8];
    if matches!(variant, 5 | 11) {
        caller.push(0x42a7); // CLR.L -(SP), one real four-byte ABI argument
        jsr(&mut caller, parent_pc);
        caller.extend([0x4fef, 4]); // LEA 4(SP),SP
    } else {
        if matches!(variant, 6 | 7) {
            jsr(&mut caller, noop_pc)
        }
        jsr(&mut caller, child_pc);
        if variant == 8 {
            caller.extend([0x302e, 0xfffc]);
        }
    }
    caller.extend([0x4e5e, 0x4e75]);
    put(&mut program, ROOT, &caller);
    put(&mut program, child_pc, CHILD_CODE);
    put(&mut program, noop_pc, &[0x4e75]);
    if variant == 5 {
        let mut wrapper = Vec::new();
        jsr(&mut wrapper, child_pc);
        wrapper.push(0x4e75);
        put(&mut program, parent_pc, &wrapper);
    } else if variant == 11 {
        let mut wrapper = vec![0x4e56, 0xfff8];
        jsr(&mut wrapper, noop_pc);
        jsr(&mut wrapper, child_pc);
        wrapper.extend([0x4e5e, 0x4e75]);
        put(&mut program, parent_pc, &wrapper);
    }
    let framed = match variant {
        1 => WRONG_OWNER,
        2 => WRONG_FRAME,
        9 => ABOVE_FRAME,
        10 => WRONG_BANK,
        11 => DEEP,
        _ => NORMAL,
    };
    let values = match variant {
        5 => ELIDED,
        6 => VALUE_RTS,
        _ => &[],
    };
    let mut registry = HookRegistry::from_entries(revision, values, framed).unwrap();
    let mut symbol = ROOT_ID.metadata().clone();
    symbol.name = "AuthoredNestedFrameRoot";
    symbol.addresses = [ROOT; 2];
    symbol.sizes = [128; 2];
    symbol.abi.argument_count = 0;
    symbol.abi.returns_value = false;
    let function: HookFn = match variant {
        0 => root::<0>,
        1 => root::<1>,
        2 => root::<2>,
        3 => root::<3>,
        4 => root::<4>,
        5 => root::<5>,
        6 => root::<6>,
        7 => root::<7>,
        8 => root::<8>,
        9 => root::<9>,
        10 => root::<10>,
        11 => root::<11>,
        _ => unreachable!(),
    };
    registry
        .register_execution_contract(symbol, class, function)
        .unwrap();
    let mut host = synthetic_host(&program, &[0x4e73]);
    host.mode = crate::Mode::Check;
    host.revision = revision;
    host.symbols = RevisionSymbols::for_revision(revision);
    host.registry = registry;
    host.checks = CheckReport::registered(&host.registry);
    host.settle_fixture_boundary(Default::default()).unwrap();
    let sp = host.emulator.hybrid_cpu().stack_pointer();
    for address in sp - 128..sp {
        host.context().write_u8(address, 0x12);
    }
    host.context().write_u16(LIVE, 0xbeef);
    host
}

#[test]
fn actual_jsr_child_mixed_word_pairs_in_every_logical_profile_and_revision() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for class in [
            ExecutionClass::Atomic,
            ExecutionClass::Functional,
            ExecutionClass::TimeAware,
        ] {
            let mut host = fixture(revision, class, 0);
            host.check_fixture(ROOT, &[]).unwrap();
            assert_eq!((host.checks.calls, host.checks.mismatches), (1, 0));
            let input = &host.checks.declared_stack_inputs[0];
            assert_eq!(
                (input.root, input.owner, input.unwritten_mask, input.value),
                ("AuthoredNestedFrameRoot", CHILD.metadata().name, 1, 0x127f)
            );
            assert_eq!(
                (
                    input.original_frame.invocation_depth,
                    input.rust_frame.invocation_depth
                ),
                (1, 1)
            );
            assert_eq!(
                (
                    input.original_frame.return_site,
                    input.rust_frame.return_site
                ),
                (Some(ROOT + 10), Some(ROOT + 10))
            );
            assert_eq!(
                input.original_frame.parent_owner,
                Some(ROOT_ID.metadata().name)
            );
            assert_eq!(
                input.original_frame.observed_cpu_pc,
                CHILD.metadata().address(revision)
            );
            assert_eq!(
                input.rust_frame.observed_cpu_pc, ROOT,
                "logical Rust calls do not repair the CPU PC"
            );
        }
    }
}

#[test]
fn a_paired_nested_input_is_not_a_clocked_identity_certificate() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for class in [
            ExecutionClass::Atomic,
            ExecutionClass::Functional,
            ExecutionClass::TimeAware,
        ] {
            let mut host = fixture(revision, class, 0);
            let mut hook = host.registry.at(ROOT).unwrap().clone();
            hook.proof_envelope = ProofEnvelope::ClockedStrict;
            let sp = host.emulator.hybrid_cpu().stack_pointer().wrapping_sub(4);
            host.context().write_u32(sp, 0x100);
            let (cpu, bus) = host.emulator.hybrid_parts();
            crate::abi::set_sp(cpu, sp);
            cpu.set_pc(ROOT, bus);
            let error = host.check_hook(&hook).unwrap_err();
            assert!(error.typed_stack_read().is_none(), "{error}");
            assert!(error.to_string().contains("ClockedStrict"), "{error}");
            assert_eq!(host.checks.declared_stack_inputs.len(), 1);
            assert_eq!((host.checks.calls, host.checks.mismatches), (1, 1));
        }
    }
}

#[test]
fn actual_and_prepared_sibling_order_and_deep_parent_materialization_are_sealed() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for variant in [6, 11] {
            let mut host = fixture(revision, ExecutionClass::Atomic, variant);
            host.check_fixture(ROOT, &[]).unwrap();
            let input = &host.checks.declared_stack_inputs[0];
            let (depth, occurrence, parent) = if variant == 6 {
                (1, 2, ROOT_ID)
            } else {
                (2, 2, PARENT)
            };
            assert_eq!(
                (
                    input.original_frame.invocation_depth,
                    input.rust_frame.invocation_depth
                ),
                (depth, depth)
            );
            assert_eq!(
                (input.original_frame.occurrence, input.rust_frame.occurrence),
                (occurrence, occurrence)
            );
            assert_eq!(
                input.original_frame.parent_owner,
                Some(parent.metadata().name)
            );
            assert_eq!((host.checks.calls, host.checks.mismatches), (1, 0));
        }
    }
}

#[test]
fn wrong_child_frame_site_owner_sibling_elision_bank_and_parent_reread_fail_closed() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for variant in [1, 2, 3, 4, 5, 7, 8, 9, 10] {
            let mut host = fixture(revision, ExecutionClass::Atomic, variant);
            let error = host.check_fixture(ROOT, &[]).unwrap_err();
            let read = error
                .typed_stack_read()
                .expect("structured held input, not a string mismatch");
            assert_eq!(
                read.reader,
                Reader::ExplicitLocal,
                "variant {variant}: {error}"
            );
            assert!(
                matches!(
                    read.reason,
                    UnsupportedReason::InvalidExplicitLocal
                        | UnsupportedReason::MissingOriginalFrameWitness
                ),
                "variant {variant}: {error}"
            );
            assert_eq!(
                (host.checks.calls, host.checks.mismatches),
                (1, 1),
                "variant {variant}"
            );
            assert!(host.checks.declared_stack_inputs.is_empty());
        }
    }
}

#[test]
#[ignore = "requires both private retail ROMs; no ROM-derived fixture bytes"]
fn authentic_sprite_checks_chooser_child_pairs_its_exact_original_invocation() {
    use std::path::PathBuf;
    use tje_mem::ram;
    for revision in [Revision::Rev02, Revision::Rev00] {
        let (key, file) = match revision {
            Revision::Rev02 => ("TJE_REV02_ROM", "rev02.bin"),
            Revision::Rev00 => ("TJE_REV00_ROM", "rev00.bin"),
        };
        let path = std::env::var_os(key).map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../roms")
                .join(file)
        });
        let registry = HookRegistry::filtered(revision, &[], &[ids::SPRITE_CHECKS]).unwrap();
        let mut host =
            Host::new(&std::fs::read(path).unwrap(), crate::Mode::Check, registry).unwrap();
        host.settle_fixture_boundary(Default::default()).unwrap();
        {
            let mut context = host.context();
            for address in (0xff0000..0x1000000).step_by(2) {
                context.write_u16(address, 0);
            }
            context.set_status_register(0x2700);
            context.set_stack_pointer(0xff7fc0).unwrap();
            let player = ram::g_player_info(revision).at(0);
            player.x().write(&mut context, 128);
            player.y().write(&mut context, 128);
            player.width().write(&mut context, 8);
            player.animation_motion_axes().write(&mut context, 0);
            // Existing caller state, not an invented store in either body.
            context.write_u8(0xff7f82, 0x5a);
        }
        host.check_fixture(ids::SPRITE_CHECKS.metadata().address(revision), &[0, 0])
            .unwrap();
        assert_eq!((host.checks.calls, host.checks.mismatches), (1, 0));
        let input = host
            .checks
            .declared_stack_inputs
            .iter()
            .find(|input| input.owner == ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT.metadata().name)
            .unwrap();
        assert_eq!(
            (
                input.frame_entry_sp,
                input.raw_address,
                input.unwritten_mask
            ),
            (0xff7f8c, 0xff7f82, 1)
        );
        assert_eq!(
            input.original_frame.parent_owner,
            Some(ids::SPRITE_CHECKS.metadata().name)
        );
        assert_eq!(
            (
                input.original_frame.invocation_depth,
                input.rust_frame.invocation_depth
            ),
            (1, 1)
        );
        let site = ids::SPRITE_CHECKS.metadata().address(revision) + 0x212;
        assert_eq!(
            (
                input.original_frame.return_site,
                input.rust_frame.return_site
            ),
            (Some(site), Some(site))
        );
        assert_eq!(
            input.original_frame.entry_pc,
            ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT
                .metadata()
                .address(revision)
        );
        assert_ne!(input.original_frame.caller_pc, None);
    }
}
