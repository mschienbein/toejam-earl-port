//! Full-trace replay input authority is distinct from the live prefix witness.
//! These are authored functions only, never real catalogue-body proof credit.
use super::*;
use tje_game::stack_read::LocalWidth;
const LOCAL: u32 = crate::catalogue::STACK_TOP - 8;
fn child_rts(_: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    Ok(None)
}
fn child_write(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.write_u16(LOCAL, 0x1234);
    Ok(None)
}
fn after_child(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let frame = context.frame_state().unwrap();
    let sp = context.stack_pointer()?;
    context.set_stack_pointer(sp - 8)?;
    let arguments = [0; 16];
    call_void(
        context,
        CHILD,
        &arguments[..usize::from(CHILD.metadata().abi.argument_count)],
    )?;
    let value = context.read_uninitialized_local(frame.uninitialized_local(
        ROOT,
        -4,
        LocalWidth::Word,
        "authored after-child exact own local; child must never have written this byte",
    ))?;
    context.write_u16(RESULT, value as u16);
    context.set_stack_pointer(sp)?;
    Ok(None)
}
static CHILD_CLEAN: &[FramedTranslation] = &[
    FramedTranslation {
        id: ROOT,
        function: after_child,
    },
    FramedTranslation {
        id: CHILD,
        function: child_rts,
    },
];
static CHILD_WRITES: &[FramedTranslation] = &[
    FramedTranslation {
        id: ROOT,
        function: after_child,
    },
    FramedTranslation {
        id: CHILD,
        function: child_write,
    },
];
#[test]
fn full_trace_explicit_input_after_child_is_not_a_rearmed_prefix_and_rejects_actual_child_writer() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (framed, writes) in [(CHILD_CLEAN, false), (CHILD_WRITES, true)] {
            let count = usize::from(CHILD.metadata().abi.argument_count);
            let mut parent = vec![0x518f];
            for _ in 0..count {
                parent.extend([0x2f3c, 0, 0]);
            }
            parent.extend(jsr(CHILD, revision));
            if count != 0 {
                parent.extend([0x4fef, 4 * count as u16]);
            }
            parent.extend([0x322f, 4, 0x33c1, 0x00ff, 0x9002, 0x508f, 0x4e75]);
            let child = if writes {
                vec![0x33fc, 0x1234, (LOCAL >> 16) as u16, LOCAL as u16, 0x4e75]
            } else {
                vec![0x4e71, 0x4e75]
            };
            let mut host = host(
                revision,
                &[],
                framed,
                &[ROOT, CHILD],
                &main_call(revision, false),
                &[(ROOT, parent), (CHILD, child)],
            );
            host.context().write_u16(LOCAL, 0x1234);
            let result = finish_one(&mut host);
            assert_eq!(result.is_ok(), !writes, "{revision:?}/{writes}: {result:?}");
            assert_eq!(host.finalize_replayed_campaign().is_ok(), !writes);
            let report = host.checks.replayed_environment.as_ref().unwrap();
            assert_eq!(report.by_function[CHILD.metadata().inventory_id].passed, 1);
            if writes {
                assert_eq!(report.by_function[ROOT.metadata().inventory_id].passed, 0);
                let reason = report.failures[0]
                    .check
                    .as_ref()
                    .unwrap()
                    .unsupported_stack_read
                    .unwrap();
                assert_eq!(
                    reason.reason,
                    tje_game::stack_read::UnsupportedReason::InterveningNonOwnWrite.as_str()
                );
            } else {
                assert_eq!(report.completed_proofs, 2);
            }
        }
    }
}
fn after_irq(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let frame = context.frame_state().unwrap();
    let sp = context.stack_pointer()?;
    context.set_stack_pointer(sp - 4)?;
    context.wait_until(WaitRequest::ram_byte_equal(FLAG, 255, 0))?;
    let value = context.read_uninitialized_local(frame.uninitialized_local(
        ROOT,
        -4,
        LocalWidth::Word,
        "authored post-IRQ exact own local; full history, not parent InvocationWitness rearm",
    ))?;
    context.write_u16(RESULT, value as u16);
    context.set_stack_pointer(sp)?;
    Ok(None)
}
static AFTER_IRQ: &[FramedTranslation] = &[FramedTranslation {
    id: ROOT,
    function: after_irq,
}];
#[test]
fn full_trace_explicit_input_after_actual_irq_requires_whole_root_never_written_ownership() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for writes in [false, true] {
            let code = vec![
                0x598f, 0x4a39, 0x00ff, 0x9001, 0x66f8, 0x3217, 0x33c1, 0x00ff, 0x9002, 0x588f,
                0x4e75,
            ];
            let mut handler = Vec::new();
            if writes {
                handler.extend([0x33fc, 0x1234, (LOCAL >> 16) as u16, LOCAL as u16]);
            }
            handler.extend([0x13fc, 0, 0x00ff, 0x9001, 0x5239, 0x00ff, 0x9004, 0x4e73]);
            let mut host = host_with_handler(
                revision,
                &[],
                AFTER_IRQ,
                &[ROOT],
                &main_call(revision, true),
                &[(ROOT, code)],
                &handler,
            );
            host.context().write_u16(LOCAL, 0x1234);
            host.context().write_u8(FLAG, 1);
            {
                let bus = host.emulator.hybrid_bus_mut();
                bus.write_word(0xc00004, 0x8010);
                bus.write_word(0xc00004, 0x8a00);
                bus.apply_pending_writes();
            }
            let result = finish_one(&mut host);
            assert_eq!(result.is_ok(), !writes, "{revision:?}/{writes}: {result:?}");
            assert_ne!(
                host.context().read_u8(IRQ_VISIBLE),
                0,
                "only real accepted IRQ changes marker"
            );
            assert_eq!(host.finalize_replayed_campaign().is_ok(), !writes);
            let report = host.checks.replayed_environment.as_ref().unwrap();
            if writes {
                assert_eq!((report.own_matches, report.completed_proofs), (0, 0));
                let reason = report.failures[0]
                    .check
                    .as_ref()
                    .unwrap()
                    .unsupported_stack_read
                    .unwrap();
                assert_eq!(
                    reason.reason,
                    tje_game::stack_read::UnsupportedReason::InterveningNonOwnWrite.as_str()
                );
            } else {
                assert_eq!(report.completed_proofs, 1);
            }
        }
    }
}
