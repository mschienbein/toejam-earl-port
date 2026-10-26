//! Authored ISA/body pairs, never catalogue-body or cartridge proof credit.
use super::*;
use tje_game::stack_read::{LocalWidth, Reader, UnsupportedReason};

const LOCAL: u32 = crate::catalogue::STACK_TOP - 8;

fn reused_local(
    context: &mut dyn Context,
    byte: bool,
    store: Option<u16>,
) -> tje_game::Result<Option<u32>> {
    let frame = context.frame_state().unwrap();
    let sp = context.stack_pointer()?;
    context.set_stack_pointer(sp - 4)?;
    if let Some(value) = store {
        if byte {
            context.write_u8(sp - 4, value as u8);
        } else {
            context.write_u16(sp - 4, value);
        }
    }
    context.set_stack_pointer(sp)?;
    context.set_stack_pointer(sp - 4)?;
    let value = context.read_uninitialized_local(frame.uninitialized_local(
        ROOT,
        -4,
        LocalWidth::Word,
        "authored reallocation: same local address after an earlier lifetime",
    ))?;
    context.write_u16(RESULT, value as u16);
    context.set_stack_pointer(sp)?;
    Ok(None)
}
const REUSED: &[(bool, Option<u16>, bool)] = &[
    (false, Some(7), true),
    (false, None, false),
    (false, Some(9), false),
    (true, Some(0x12), true),
    (true, None, false),
    (true, Some(0x13), false),
];
static REUSED_BODIES: &[&[FramedTranslation]] = &[
    &[FramedTranslation {
        id: ROOT,
        function: |ctx| reused_local(ctx, false, Some(7)),
    }],
    &[FramedTranslation {
        id: ROOT,
        function: |ctx| reused_local(ctx, false, None),
    }],
    &[FramedTranslation {
        id: ROOT,
        function: |ctx| reused_local(ctx, false, Some(9)),
    }],
    &[FramedTranslation {
        id: ROOT,
        function: |ctx| reused_local(ctx, true, Some(0x12)),
    }],
    &[FramedTranslation {
        id: ROOT,
        function: |ctx| reused_local(ctx, true, None),
    }],
    &[FramedTranslation {
        id: ROOT,
        function: |ctx| reused_local(ctx, true, Some(0x13)),
    }],
];
#[test]
fn explicit_reallocation_never_repairs_an_earlier_missing_or_wrong_own_store() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (index, &(byte, store, succeeds)) in REUSED.iter().enumerate() {
            let code = vec![
                0x598f,
                if byte { 0x1ebc } else { 0x3ebc },
                if byte { 0x12 } else { 7 },
                0x588f,
                0x598f,
                0x3217,
                0x33c1,
                0x00ff,
                0x9002,
                0x588f,
                0x4e75,
            ];
            let mut host = host(
                revision,
                &[],
                REUSED_BODIES[index],
                &[ROOT],
                &main_call(revision, false),
                &[(ROOT, code)],
            );
            host.context().write_u16(LOCAL, 0x5534);
            let result = finish_one(&mut host);
            assert_eq!(
                result.is_ok(),
                succeeds,
                "{revision:?}/{byte}/{store:?}: {result:?}"
            );
            assert_eq!(host.finalize_replayed_campaign().is_ok(), succeeds);
            let report = host.checks.replayed_environment.as_ref().unwrap();
            if succeeds {
                assert_eq!(report.completed_proofs, 1);
                assert_eq!(
                    host.context().read_u16(RESULT),
                    if byte { 0x1234 } else { 7 }
                );
            } else {
                assert_eq!(report.own_matches, 0);
                assert_eq!(report.completed_proofs, 0);
                let detail = &report.failures[0].check.as_ref().unwrap().detail;
                assert!(
                    detail.contains("paired")
                        || detail.contains("repair")
                        || detail.contains("earlier"),
                    "{detail}"
                );
            }
        }
    }
}

pub(super) fn dma_commands(source: u32) -> [u16; 9] {
    let source = source >> 1;
    [
        0x8114,
        0x8f02,
        0x9301,
        0x9400,
        0x9500 | source as u16 & 0xff,
        0x9600 | (source >> 8) as u16 & 0xff,
        0x9700 | (source >> 16) as u16 & 0x7f,
        0x4200,
        0x0080,
    ]
}
fn dma_old_local(context: &mut dyn Context, store: bool) -> tje_game::Result<Option<u32>> {
    let sp = context.stack_pointer()?;
    context.set_stack_pointer(sp - 4)?;
    if store {
        context.write_u16(sp - 4, 0x1234);
    }
    context.set_stack_pointer(sp)?;
    for word in dma_commands(LOCAL) {
        context.write_device_u16(0xc00004, word);
    }
    Ok(None)
}
static DMA_STORED: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: |ctx| dma_old_local(ctx, true),
}];
static DMA_OMITTED: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: |ctx| dma_old_local(ctx, false),
}];
#[test]
fn completed_actual_dma_requires_the_released_local_producer_even_when_entry_ram_coincides() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (body, succeeds) in [(DMA_STORED, true), (DMA_OMITTED, false)] {
            let mut code = vec![0x598f, 0x3ebc, 0x1234, 0x588f];
            for word in dma_commands(LOCAL) {
                code.extend([0x33fc, word, 0x00c0, 4]);
            }
            // Genuine authored CPU work, not a synthetic join/read/timing charge.
            code.extend([0x4e71; 32]);
            code.push(0x4e75);
            let mut host = host(
                revision,
                &[],
                body,
                &[ROOT],
                &main_call(revision, false),
                &[(ROOT, code)],
            );
            host.context().write_u16(LOCAL, 0x1234);
            let result = finish_one(&mut host);
            assert_eq!(result.is_ok(), succeeds, "{revision:?}: {result:?}");
            assert!(
                !record::DmaBoundary::capture(host.emulator.hybrid_bus()).unresolved(),
                "the actual DMA must already be complete; endpoint debt must not mask missing initialization"
            );
            assert_eq!(host.finalize_replayed_campaign().is_ok(), succeeds);
            let report = host.checks.replayed_environment.as_ref().unwrap();
            if succeeds {
                assert_eq!(report.completed_proofs, 1);
            } else {
                assert_eq!((report.own_matches, report.completed_proofs), (0, 0));
                let reason = report.failures[0]
                    .check
                    .as_ref()
                    .unwrap()
                    .unsupported_stack_read
                    .unwrap();
                assert_eq!(reason.reader, Reader::VdpDma.as_str());
                assert_eq!(reason.reason, UnsupportedReason::Unwritten.as_str());
                assert_eq!(reason.address, LOCAL);
            }
        }
    }
}

fn unresolved_command(context: &mut dyn Context, fill: bool) -> tje_game::Result<Option<u32>> {
    if fill {
        for word in [0x8114, 0x8f02, 0x9300, 0x9400, 0x9780, 0x4000, 0x0080] {
            context.write_device_u16(0xc00004, word);
        }
        context.write_device_u16(0xc00000, 0x1234);
    } else {
        context.write_device_u16(0xc00004, 0x4000);
    }
    Ok(None)
}
static PARTIAL: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: |ctx| unresolved_command(ctx, false),
}];
static FILL: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: |ctx| unresolved_command(ctx, true),
}];
#[test]
fn active_fill_and_partial_command_at_actual_return_are_unsupported_not_adopted_or_joined() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (body, fill) in [(PARTIAL, false), (FILL, true)] {
            let mut code = Vec::new();
            if fill {
                for word in [0x8114, 0x8f02, 0x9300, 0x9400, 0x9780, 0x4000, 0x0080] {
                    code.extend([0x33fc, word, 0x00c0, 4]);
                }
                code.extend([0x33fc, 0x1234, 0x00c0, 0]);
            } else {
                code.extend([0x33fc, 0x4000, 0x00c0, 4]);
            }
            code.push(0x4e75);
            let mut host = host(
                revision,
                &[],
                body,
                &[ROOT],
                &main_call(revision, false),
                &[(ROOT, code)],
            );
            let error = finish_one(&mut host).unwrap_err();
            assert!(
                error.to_string().contains("unresolved DMA/command debt"),
                "{error}"
            );
            let report = host.checks.replayed_environment.as_ref().unwrap();
            assert_eq!((report.own_matches, report.completed_proofs), (0, 0));
            assert!(!report.complete);
        }
    }
}

#[test]
fn unresolved_actual_entry_dma_or_partial_command_cannot_be_adopted_by_a_new_semantic_root() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for fill in [false, true] {
            let mut main = vec![0x46fc, 0x2700];
            if fill {
                for word in [0x8114, 0x8f02, 0x9300, 0x9400, 0x9780, 0x4000, 0x0080] {
                    main.extend([0x33fc, word, 0x00c0, 4]);
                }
                main.extend([0x33fc, 0x1234, 0x00c0, 0]);
            } else {
                main.extend([0x33fc, 0x4000, 0x00c0, 4]);
            }
            main.extend(jsr(ROOT, revision));
            main.push(0x60fe);
            let mut host = host(
                revision,
                &[],
                LITERAL,
                &[ROOT],
                &main,
                &[(ROOT, literal_code())],
            );
            let error = finish_one(&mut host).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("unresolved entry DMA/command debt"),
                "{error}"
            );
            let report = host.checks.replayed_environment.as_ref().unwrap();
            assert_eq!((report.own_matches, report.completed_proofs), (0, 0));
            assert_eq!(report.by_function[ROOT.metadata().inventory_id].checked, 0);
            assert!(host.finalize_replayed_campaign().is_err());
        }
    }
}

fn pointer_store(
    context: &mut dyn Context,
    width: u8,
    dereference: bool,
) -> tje_game::Result<Option<u32>> {
    let sp = context.stack_pointer()?;
    context.set_stack_pointer(sp - 4)?;
    let pointer = sp - 4;
    match width {
        1 => {
            for (index, byte) in pointer.to_be_bytes().into_iter().enumerate() {
                context.write_u8(RESULT + index as u32, byte);
            }
        }
        2 => {
            context.write_u16(RESULT, (pointer >> 16) as u16);
            context.write_u16(RESULT + 2, pointer as u16);
        }
        4 => context.write_u32(RESULT, pointer),
        _ => unreachable!(),
    }
    if dereference {
        let pointer = context.read_u32(RESULT);
        let _ = context.read_u8(pointer);
    }
    context.set_stack_pointer(sp)?;
    Ok(None)
}
static POINTERS: &[&[FramedTranslation]] = &[
    &[FramedTranslation {
        id: ROOT,
        function: |ctx| pointer_store(ctx, 1, false),
    }],
    &[FramedTranslation {
        id: ROOT,
        function: |ctx| pointer_store(ctx, 2, false),
    }],
    &[FramedTranslation {
        id: ROOT,
        function: |ctx| pointer_store(ctx, 4, false),
    }],
    &[FramedTranslation {
        id: ROOT,
        function: |ctx| pointer_store(ctx, 1, true),
    }],
    &[FramedTranslation {
        id: ROOT,
        function: |ctx| pointer_store(ctx, 2, true),
    }],
    &[FramedTranslation {
        id: ROOT,
        function: |ctx| pointer_store(ctx, 4, true),
    }],
];
#[test]
fn pointer_bits_are_width_invariant_but_never_grant_pointee_initialization_or_lifetime() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (index, body) in POINTERS.iter().enumerate() {
            let width = [1, 2, 4][index % 3];
            let dereference = index >= 3;
            let mut code = vec![0x598f];
            match width {
                1 => {
                    for (lane, byte) in LOCAL.to_be_bytes().into_iter().enumerate() {
                        code.extend([0x13fc, u16::from(byte), 0x00ff, (0x9002 + lane) as u16]);
                    }
                }
                2 => {
                    code.extend([
                        0x33fc,
                        (LOCAL >> 16) as u16,
                        0x00ff,
                        0x9002,
                        0x33fc,
                        LOCAL as u16,
                        0x00ff,
                        0x9004,
                    ]);
                }
                4 => {
                    code.extend([0x23fc, (LOCAL >> 16) as u16, LOCAL as u16, 0x00ff, 0x9002]);
                }
                _ => unreachable!(),
            }
            if dereference {
                code.extend([0x2079, 0x00ff, 0x9002, 0x1010]);
            }
            code.extend([0x588f, 0x4e75]);
            let mut host = host(
                revision,
                &[],
                body,
                &[ROOT],
                &main_call(revision, false),
                &[(ROOT, code)],
            );
            let result = finish_one(&mut host);
            assert_eq!(
                result.is_ok(),
                !dereference,
                "{revision:?}/{width}: {result:?}"
            );
            assert_eq!(host.finalize_replayed_campaign().is_ok(), !dereference);
            let report = host.checks.replayed_environment.as_ref().unwrap();
            assert_eq!(report.completed_proofs, u64::from(!dereference));
            if dereference {
                assert_eq!(
                    report.failures[0]
                        .check
                        .as_ref()
                        .unwrap()
                        .unsupported_stack_read
                        .unwrap()
                        .reason,
                    UnsupportedReason::Unwritten.as_str()
                );
            }
        }
    }
}
