//! Authored transport mechanisms only; no owner-body/catalogue proof credit.
use super::*;
use tje_game::call_frame::CallSite;

#[derive(Clone, Copy, Debug)]
enum Demand {
    Stack,
    LazyStack,
    LazyCcr,
    ReturnSite,
}
fn scalar(context: &mut dyn Context, demand: Demand) -> tje_game::Result<Option<u32>> {
    // Deliberately omits the original caller's eight-byte local allocation.
    // A scalar ABI adapter must not replace this Rust SP with an original SP.
    let value = match demand {
        Demand::Stack | Demand::ReturnSite => tje_game::call(context, VALUE_CHILD, &[9, 0, 0])?,
        Demand::LazyStack => {
            call_void(context, CHILD, &[])?;
            context.data_register(0)
        }
        Demand::LazyCcr => {
            call_void(context, CHILD, &[])?;
            u32::from(context.status_register() & 0x1f)
        }
    };
    context.write_u32(RESULT, value);
    Ok(None)
}
static SCALAR: &[(Demand, &[FramedTranslation])] = &[
    (
        Demand::Stack,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| scalar(ctx, Demand::Stack),
        }],
    ),
    (
        Demand::LazyStack,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| scalar(ctx, Demand::LazyStack),
        }],
    ),
    (
        Demand::LazyCcr,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| scalar(ctx, Demand::LazyCcr),
        }],
    ),
    (
        Demand::ReturnSite,
        &[FramedTranslation {
            id: ROOT,
            function: |ctx| scalar(ctx, Demand::ReturnSite),
        }],
    ),
];
#[test]
fn scalar_geometry_never_authenticates_semantic_stack_or_return_site_data_even_deferred() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for &(demand, framed) in SCALAR {
            let id = if matches!(demand, Demand::LazyStack | Demand::LazyCcr) {
                CHILD
            } else {
                VALUE_CHILD
            };
            let count = usize::from(id.metadata().abi.argument_count);
            let mut parent = vec![0x518f];
            for _ in 0..count {
                parent.extend([0x2f3c, 0, 0]);
            }
            parent.extend(jsr(id, revision));
            if count != 0 {
                parent.extend([0x4fef, 4 * count as u16]);
            }
            parent.push(0x508f);
            if matches!(demand, Demand::LazyCcr) {
                parent.extend([0x40c0, 0x0280, 0, 0x1f]);
            }
            parent.extend([0x23c0, 0x00ff, 0x9002, 0x4e75]);
            let child = match demand {
                Demand::Stack => vec![0x200f, 0x4e75],
                Demand::ReturnSite => vec![0x2017, 0x4e75],
                Demand::LazyStack | Demand::LazyCcr => vec![0x598f, 0x2e8f, 0x2017, 0x588f, 0x4e75],
            };
            // Value child is deliberately unhooked: this checks the pre-import
            // causal guard, not a forged proof for any canonical owner body.
            let mut host = host(
                revision,
                &[],
                framed,
                &[ROOT],
                &main_call(revision, false),
                &[(ROOT, parent), (id, child)],
            );
            let error = finish_one(&mut host).unwrap_err();
            if matches!(demand, Demand::ReturnSite) {
                assert!(
                    error.to_string().contains("scalar call")
                        && error.to_string().contains("return-site"),
                    "{error}"
                );
            } else {
                assert!(
                    matches!(&error, crate::HostError::Mismatch { .. })
                        && error.to_string().contains("consumes Ssp"),
                    "{revision:?}/{demand:?}: {error}"
                );
            }
            let report = host.checks.replayed_environment.as_ref().unwrap();
            assert_eq!((report.own_matches, report.completed_proofs), (0, 0));
        }
    }
}

const VALUE_CHILD: FunctionId = ids::GENERATE_SPEECH;
#[derive(Clone, Copy)]
enum Prepared {
    Stack,
    ReturnSite,
    PrivatePointer,
}
fn child_value(context: &mut dyn Context, kind: Prepared) -> tje_game::Result<Option<u32>> {
    let sp = context.stack_pointer()?;
    let value = match kind {
        Prepared::Stack => sp,
        Prepared::ReturnSite => context.read_u32(sp),
        Prepared::PrivatePointer => {
            context.set_stack_pointer(sp - 64)?;
            context.write_u16(sp - 64, 7);
            context.set_stack_pointer(sp)?;
            sp - 64
        }
    };
    context.set_data_register(0, value);
    Ok(Some(value))
}
fn prepared(
    context: &mut dyn Context,
    kind: Prepared,
    alias_read: bool,
) -> tje_game::Result<Option<u32>> {
    let count = usize::from(VALUE_CHILD.metadata().abi.argument_count);
    let arguments = [0_u32; 16];
    let sp = context.stack_pointer()?;
    context.set_stack_pointer(sp - 8)?;
    let revision = context.revision();
    let value = tje_game::call_at(
        context,
        VALUE_CHILD,
        &arguments[..count],
        CallSite::relative(ROOT, revision, 8 + 6 * count as u32),
    )?;
    context.set_stack_pointer(sp)?;
    context.write_u32(RESULT, value);
    if alias_read {
        // Same physical private byte through a rewrapped work-RAM mirror.
        let alias = 0xe00000 | (value & 0xffff);
        let _ = context.read_u8(alias);
    }
    let _ = kind;
    Ok(None)
}
static PREPARED: &[(Prepared, bool, &[FramedTranslation])] = &[
    (
        Prepared::Stack,
        false,
        &[
            FramedTranslation {
                id: ROOT,
                function: |ctx| prepared(ctx, Prepared::Stack, false),
            },
            FramedTranslation {
                id: VALUE_CHILD,
                function: |ctx| child_value(ctx, Prepared::Stack),
            },
        ],
    ),
    (
        Prepared::ReturnSite,
        false,
        &[
            FramedTranslation {
                id: ROOT,
                function: |ctx| prepared(ctx, Prepared::ReturnSite, false),
            },
            FramedTranslation {
                id: VALUE_CHILD,
                function: |ctx| child_value(ctx, Prepared::ReturnSite),
            },
        ],
    ),
    (
        Prepared::PrivatePointer,
        false,
        &[
            FramedTranslation {
                id: ROOT,
                function: |ctx| prepared(ctx, Prepared::PrivatePointer, false),
            },
            FramedTranslation {
                id: VALUE_CHILD,
                function: |ctx| child_value(ctx, Prepared::PrivatePointer),
            },
        ],
    ),
    (
        Prepared::PrivatePointer,
        true,
        &[
            FramedTranslation {
                id: ROOT,
                function: |ctx| prepared(ctx, Prepared::PrivatePointer, true),
            },
            FramedTranslation {
                id: VALUE_CHILD,
                function: |ctx| child_value(ctx, Prepared::PrivatePointer),
            },
        ],
    ),
];
#[test]
fn authentic_prepared_boundaries_supply_sp_and_return_data_but_abi_pointer_bits_never_supply_private_ram()
 {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for &(kind, alias_read, framed) in PREPARED {
            let count = usize::from(VALUE_CHILD.metadata().abi.argument_count);
            let mut parent = vec![0x518f];
            for _ in 0..count {
                parent.extend([0x2f3c, 0, 0]);
            }
            parent.extend(jsr(VALUE_CHILD, revision));
            if count != 0 {
                parent.extend([0x4fef, 4 * count as u16]);
            }
            parent.extend([0x508f, 0x23c0, 0x00ff, 0x9002]);
            if alias_read {
                parent.extend([0x2200, 0x0281, 0, 0xffff, 0x0081, 0x00e0, 0, 0x2041, 0x1210]);
            }
            parent.push(0x4e75);
            let child = match kind {
                Prepared::Stack => vec![0x200f, 0x4e75],
                Prepared::ReturnSite => vec![0x2017, 0x4e75],
                Prepared::PrivatePointer => {
                    vec![0x4fef, 0xffc0, 0x3ebc, 7, 0x200f, 0x4fef, 64, 0x4e75]
                }
            };
            let mut host = host(
                revision,
                &[],
                framed,
                &[ROOT, VALUE_CHILD],
                &main_call(revision, false),
                &[(ROOT, parent), (VALUE_CHILD, child)],
            );
            let result = finish_one(&mut host);
            assert_eq!(
                result.is_ok(),
                !alias_read,
                "{revision:?}/{alias_read}: {result:?}"
            );
            assert_eq!(host.finalize_replayed_campaign().is_ok(), !alias_read);
            let report = host.checks.replayed_environment.as_ref().unwrap();
            assert_eq!(
                report.by_function[VALUE_CHILD.metadata().inventory_id].passed,
                1,
                "this synthetic child proves its numeric ABI output only, never the private pointee"
            );
            if alias_read {
                assert_eq!(report.by_function[ROOT.metadata().inventory_id].passed, 0);
                assert!(
                    report.failures.iter().any(|packet| packet
                        .check
                        .as_ref()
                        .unwrap()
                        .detail
                        .contains("private residue")),
                    "{report:?}"
                );
            } else {
                assert_eq!(report.completed_proofs, 2);
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Observer {
    Callee,
    Interrupt,
    Dma,
}
fn observe_pointer(context: &mut dyn Context, observer: Observer) -> tje_game::Result<Option<u32>> {
    prepared(context, Prepared::PrivatePointer, false)?;
    match observer {
        Observer::Callee => call_void(context, CHILD, &[])?,
        Observer::Interrupt => context.wait_until(WaitRequest::ram_byte_equal(FLAG, 255, 0))?,
        Observer::Dma => {
            let pointer = context.data_register(0);
            for word in super::frame_causality::dma_commands(pointer) {
                context.write_device_u16(0xc00004, word);
            }
        }
    }
    Ok(None)
}
static OBSERVERS: &[(Observer, &[FramedTranslation])] = &[
    (
        Observer::Callee,
        &[
            FramedTranslation {
                id: ROOT,
                function: |ctx| observe_pointer(ctx, Observer::Callee),
            },
            FramedTranslation {
                id: VALUE_CHILD,
                function: |ctx| child_value(ctx, Prepared::PrivatePointer),
            },
        ],
    ),
    (
        Observer::Interrupt,
        &[
            FramedTranslation {
                id: ROOT,
                function: |ctx| observe_pointer(ctx, Observer::Interrupt),
            },
            FramedTranslation {
                id: VALUE_CHILD,
                function: |ctx| child_value(ctx, Prepared::PrivatePointer),
            },
        ],
    ),
    (
        Observer::Dma,
        &[
            FramedTranslation {
                id: ROOT,
                function: |ctx| observe_pointer(ctx, Observer::Dma),
            },
            FramedTranslation {
                id: VALUE_CHILD,
                function: |ctx| child_value(ctx, Prepared::PrivatePointer),
            },
        ],
    ),
];
#[test]
fn returned_pointer_bits_never_discharge_later_callee_irq_or_dma_private_pointee_reads() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for &(observer, framed) in OBSERVERS {
            let count = usize::from(VALUE_CHILD.metadata().abi.argument_count);
            let mut parent = vec![0x518f];
            for _ in 0..count {
                parent.extend([0x2f3c, 0, 0]);
            }
            parent.extend(jsr(VALUE_CHILD, revision));
            if count != 0 {
                parent.extend([0x4fef, 4 * count as u16]);
            }
            parent.extend([0x508f, 0x23c0, 0x00ff, 0x9002]);
            let pointer = crate::catalogue::STACK_TOP - 4 - 8 - 4 * count as u32 - 4 - 64;
            match observer {
                Observer::Callee => parent.extend(jsr(CHILD, revision)),
                Observer::Interrupt => parent.extend([0x4a39, 0x00ff, 0x9001, 0x66f8]),
                Observer::Dma => {
                    for word in super::frame_causality::dma_commands(pointer) {
                        parent.extend([0x33fc, word, 0x00c0, 4]);
                    }
                    parent.extend([0x4e71; 32]);
                }
            }
            parent.push(0x4e75);
            let child = vec![0x4fef, 0xffc0, 0x3ebc, 7, 0x200f, 0x4fef, 64, 0x4e75];
            let observer_code = vec![0x2079, 0x00ff, 0x9002, 0x1010, 0x4e75];
            let handler = [
                0x48e7, 0x8080, 0x2079, 0x00ff, 0x9002, 0x1010, 0x13fc, 0, 0x00ff, 0x9001, 0x5239,
                0x00ff, 0x9004, 0x4cdf, 0x0101, 0x4e73,
            ];
            let mut host = host_with_handler(
                revision,
                &[],
                framed,
                &[ROOT, VALUE_CHILD],
                &main_call(revision, matches!(observer, Observer::Interrupt)),
                &[(ROOT, parent), (VALUE_CHILD, child), (CHILD, observer_code)],
                &handler,
            );
            if matches!(observer, Observer::Interrupt) {
                host.context().write_u8(FLAG, 1);
                let bus = host.emulator.hybrid_bus_mut();
                bus.write_word(0xc00004, 0x8010);
                bus.write_word(0xc00004, 0x8a00);
                bus.apply_pending_writes();
            }
            let error = finish_one(&mut host).unwrap_err();
            assert!(
                error.to_string().contains("private residue"),
                "{revision:?}/{observer:?}: {error}"
            );
            assert!(host.finalize_replayed_campaign().is_err());
            let report = host.checks.replayed_environment.as_ref().unwrap();
            assert_eq!(
                report.by_function[VALUE_CHILD.metadata().inventory_id].passed,
                1,
                "numeric pointer return proof exists; it is never a pointee proof"
            );
            assert_eq!(report.by_function[ROOT.metadata().inventory_id].passed, 0);
            if matches!(observer, Observer::Dma) {
                assert!(
                    !record::DmaBoundary::capture(host.emulator.hybrid_bus()).unresolved(),
                    "actual DMA completed; not a pending-command failure substitute"
                );
            }
        }
    }
}

fn scalar_transport_only(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    let value = tje_game::call(context, VALUE_CHILD, &[0, 0, 0])?;
    context.write_u32(RESULT, value);
    Ok(None)
}
fn constant_child(context: &mut dyn Context) -> tje_game::Result<Option<u32>> {
    context.set_data_register(0, 9);
    Ok(Some(9))
}
static TRANSPORT_ONLY: &[FramedTranslation] = &[
    FramedTranslation {
        id: ROOT,
        function: scalar_transport_only,
    },
    FramedTranslation {
        id: VALUE_CHILD,
        function: constant_child,
    },
];
#[test]
fn scalar_transport_alone_does_not_turn_private_frame_layout_into_algorithmic_sp_dependence() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut parent = vec![0x518f, 0x2f3c, 0, 0, 0x2f3c, 0, 0, 0x2f3c, 0, 0];
        parent.extend(jsr(VALUE_CHILD, revision));
        parent.extend([0x4fef, 12, 0x508f, 0x23c0, 0x00ff, 0x9002, 0x4e75]);
        let mut host = host(
            revision,
            &[],
            TRANSPORT_ONLY,
            &[ROOT, VALUE_CHILD],
            &main_call(revision, false),
            &[(ROOT, parent), (VALUE_CHILD, vec![0x7009, 0x4e75])],
        );
        finish_one(&mut host).unwrap();
        host.finalize_replayed_campaign().unwrap();
        assert_eq!(
            host.checks
                .replayed_environment
                .as_ref()
                .unwrap()
                .completed_proofs,
            2
        );
    }
}
