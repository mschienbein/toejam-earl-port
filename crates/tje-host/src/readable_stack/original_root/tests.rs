//! Authored adapter-model controls for indexed chronology, not IRQ execution
//! or cartridge proof. Actual CPU IRQ/bank revocation is exercised separately.
use super::*;
use crate::fixture::tests::synthetic_host;
use genesis_core::HybridExceptionBoundary;
use tje_game::ids;
fn scope() -> (crate::Host, OriginalRootScope) {
    let host = synthetic_host(&[0x60fe], &[0x4e73]);
    let cpu = host.emulator.hybrid_cpu();
    let recorder = OriginalRootScope::new(
        cpu,
        host.emulator.hybrid_bus(),
        Revision::Rev02,
        ids::NO_OP,
        cpu.pc(),
        cpu.stack_pointer(),
        false,
    );
    (host, recorder)
}
fn marker(vector: u32, handler: u32, depth: u32) -> HybridExceptionBoundary {
    HybridExceptionBoundary {
        master_clock: 70,
        boundary: m68000_emu::ExceptionBoundary::Entered {
            vector,
            handler,
            depth,
        },
    }
}
#[test]
fn only_the_consumed_index_after_the_read_is_later_history_and_never_its_owner() {
    let (host, scope) = scope();
    scope.before_step(host.emulator.hybrid_cpu(), true);
    let witness = scope.witness().unwrap();
    scope.observe_stream(
        HybridAccess::Read {
            address: 0xff7ff0,
            word: true,
            purpose: m68000_emu::ReadPurpose::Semantic,
            value: 0x1234,
        },
        &[],
    );
    assert!(scope.first_later_exception().is_none());
    let payloads = [marker(28, 0x200, 1), marker(30, 0x300, 1)];
    scope.observe_stream(HybridAccess::ExceptionBoundary { index: 1 }, &payloads);
    let evidence = scope.first_later_exception().unwrap();
    assert_eq!(
        (
            evidence.vector,
            evidence.handler,
            evidence.depth,
            evidence.occurrence
        ),
        (30, 0x300, 1, 1)
    );
    assert_eq!(evidence.call_parent_owner, Some(ids::NO_OP.metadata().name));
    assert!(evidence.parent_occurrence.is_none());
    assert!(
        scope.witness().is_none(),
        "the later marker revoked current permission"
    );
    assert_eq!(
        scope.record(witness).unwrap().owner(),
        Some(ids::NO_OP),
        "historical read ownership was not rewritten"
    );
    scope.observe_stream(
        HybridAccess::ExceptionBoundary { index: 0 },
        &[marker(28, 0x400, 2)],
    );
    assert_eq!(
        scope.first_later_exception().unwrap().handler,
        0x300,
        "nested later history cannot replace the first consumed marker"
    );
}
#[test]
fn a_marker_before_the_read_or_missing_payload_cannot_be_reported_as_a_new_witness() {
    for missing in [false, true] {
        let (host, scope) = scope();
        let old = scope.witness().unwrap();
        scope.before_step(host.emulator.hybrid_cpu(), true);
        scope.observe_stream(
            HybridAccess::ExceptionBoundary {
                index: if missing { 1 } else { 0 },
            },
            &[marker(30, 0x300, 1)],
        );
        assert!(scope.witness().is_none());
        scope.observe_stream(
            HybridAccess::Read {
                address: 0xff7ff0,
                word: true,
                purpose: m68000_emu::ReadPurpose::Semantic,
                value: 0x1234,
            },
            &[],
        );
        assert!(scope.witness().is_none());
        if missing {
            assert!(scope.first_later_exception().is_none());
            assert!(
                scope.record(old).is_none(),
                "missing payload rejects the capture, not an invented IRQ"
            );
        }
    }
}
