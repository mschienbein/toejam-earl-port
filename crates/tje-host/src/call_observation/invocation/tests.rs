//! Authored recorder-model controls; end-to-end CPU/framework fixtures live in
//! readable_stack::nested_tests. These controls do not claim ROM equivalence.
use super::*;
use genesis_core::HybridAccess;
use tje_game::ids;

const ROOT: u32 = 0x600;
const SP: u32 = 0xff8000;
fn point(pc: u32, sp: u32, exception_depth: u32) -> InvocationPoint {
    InvocationPoint {
        pc,
        sp,
        status: 0x2700,
        usp: 0,
        ssp: sp,
        exception_depth,
    }
}
fn rooted() -> InvocationTracker {
    InvocationTracker::rooted(
        Revision::Rev02,
        ids::NO_OP,
        ROOT,
        SP,
        point(ROOT, SP, 0),
        Some(0x800),
        false,
    )
}
fn receipt(point: InvocationPoint, opcode: u16) -> HybridExecutedInstruction {
    HybridExecutedInstruction {
        pc: point.pc,
        opcode,
        exception_depth: point.exception_depth,
        completed_successfully: true,
    }
}
fn frame_writes(tracker: &mut InvocationTracker, sp: u32, return_pc: u32) {
    for (address, value) in [(sp, (return_pc >> 16) as u16), (sp + 2, return_pc as u16)] {
        tracker.observe_actual_access(HybridAccess::Write {
            address,
            word: true,
            value,
        });
    }
}
fn call(
    tracker: &mut InvocationTracker,
    before: InvocationPoint,
    target: u32,
    return_pc: u32,
) -> (InvocationId, InvocationPoint) {
    tracker.begin_actual_step(before, 0x4eb9, true);
    let after = point(target, before.sp - 4, before.exception_depth);
    frame_writes(tracker, after.sp, return_pc);
    let update = tracker.finish_actual_step(Some(receipt(before, 0x4eb9)), after, Some(return_pc));
    (update.entered.expect("authored successful call"), after)
}
fn return_call(tracker: &mut InvocationTracker, entry: InvocationPoint, return_pc: u32) {
    let before = point(entry.pc + 8, entry.sp, entry.exception_depth);
    tracker.begin_actual_step(before, 0x4e75, true);
    let after = point(return_pc, entry.sp + 4, entry.exception_depth);
    assert!(
        tracker
            .finish_actual_step(Some(receipt(before, 0x4e75)), after, None)
            .returned
            .is_some()
    );
}
fn child() -> u32 {
    ids::GET_RANDOM_NUMBER.metadata().address(Revision::Rev02)
}
fn child_pair() -> (InvocationTracker, InvocationWitness) {
    let mut tracker = rooted();
    call(
        &mut tracker,
        point(ROOT + 4, SP - 12, 0),
        child(),
        ROOT + 10,
    );
    let witness = tracker.current_witness().unwrap();
    (tracker, witness)
}

#[test]
fn actual_return_restores_only_metadata_and_a_fresh_sibling_can_be_observed() {
    let mut tracker = rooted();
    let (id, entry) = call(
        &mut tracker,
        point(ROOT + 4, SP - 12, 0),
        child(),
        ROOT + 10,
    );
    let raw = tracker.actual_call(id).unwrap();
    assert!(tracker.current_witness().is_some());
    assert!(
        tracker.step_witness().is_none(),
        "JSR EA reads cannot borrow the child"
    );
    return_call(&mut tracker, entry, ROOT + 10);
    assert_eq!(tracker.current().unwrap().owner(), Some(ids::NO_OP));
    assert!(
        tracker.current_witness().is_none(),
        "matching RTS never rearms parent reads"
    );
    assert_eq!(
        tracker.resolve_actual(raw).unwrap().entry_sp(),
        Some(entry.sp)
    );
    let (second, _) = call(
        &mut tracker,
        point(ROOT + 4, SP - 12, 0),
        child(),
        ROOT + 10,
    );
    assert_eq!(tracker.record(second).unwrap().occurrence(), 2);
    assert!(tracker.current_witness().is_some());
}

#[test]
fn every_frame_identity_and_parent_field_participates_in_cross_run_matching() {
    let (baseline, witness) = child_pair();
    for mutation in 0..12 {
        let (mut other, right) = child_pair();
        let index = right.id().index;
        match mutation {
            0 => other.records[index].owner = Some(ids::ABS),
            1 => other.records[index].entry_pc += 2,
            2 => other.records[index].entry_sp = Some(SP - 20),
            3 => other.records[index].return_pc = Some(ROOT + 12),
            4 => other.records[index].return_site = Some(ROOT + 12),
            5 => other.records[index].supervisor = false,
            6 => other.records[index].exception_depth += 1,
            7 => other.records[index].invocation_depth += 1,
            8 => other.records[index].occurrence += 1,
            9 => other.records[index].parent = None,
            10 => other.records[0].entry_sp = Some(SP - 4),
            11 => other.records[0].owner = Some(ids::ABS),
            _ => unreachable!(),
        }
        assert!(
            !baseline.witnesses_match(witness, &other, right),
            "mutation {mutation}"
        );
        assert!(
            other.resolve(witness).is_none(),
            "foreign capture brand is not an arena index"
        );
    }
    let (other, right) = child_pair();
    assert!(baseline.witnesses_match(witness, &other, right));
}

#[test]
fn identical_sibling_headers_do_not_erase_their_occurrence_or_recursive_parent() {
    let (baseline, first) = child_pair();
    let (mut other, old) = child_pair();
    let entry = other.record(old.id()).unwrap().entry_point();
    return_call(&mut other, entry, ROOT + 10);
    call(&mut other, point(ROOT + 4, SP - 12, 0), child(), ROOT + 10);
    assert!(!baseline.witnesses_match(first, &other, other.current_witness().unwrap()));

    let (mut recursive, outer) = child_pair();
    let entry = recursive.record(outer.id()).unwrap().entry_point();
    call(
        &mut recursive,
        point(entry.pc + 4, entry.sp - 12, 0),
        child(),
        entry.pc + 10,
    );
    let nested = recursive.current_witness().unwrap();
    let (mut forged, shallow) = child_pair();
    let nested_record = *recursive.record(nested.id()).unwrap();
    let shallow_record = &mut forged.records[shallow.id().index];
    shallow_record.entry_pc = nested_record.entry_pc;
    shallow_record.entry_sp = nested_record.entry_sp;
    shallow_record.return_pc = nested_record.return_pc;
    shallow_record.return_site = nested_record.return_site;
    shallow_record.invocation_depth = nested_record.invocation_depth;
    assert!(
        !recursive.witnesses_match(nested, &forged, shallow),
        "matching leaf tuples are not a parent lineage"
    );
}

#[test]
fn final_ram_or_external_mutations_cannot_substitute_for_actual_return_word_issues() {
    for mutation in 0..6 {
        let mut tracker = rooted();
        let before = point(ROOT + 4, SP - 12, 0);
        let after = point(child(), before.sp - 4, 0);
        tracker.begin_actual_step(before, 0x4eb9, true);
        let high = HybridAccess::Write {
            address: after.sp,
            word: true,
            value: 0,
        };
        let low = HybridAccess::Write {
            address: after.sp + 2,
            word: true,
            value: (ROOT + 10) as u16,
        };
        match mutation {
            0 => tracker.observe_actual_access(low),
            1 => {
                tracker.observe_actual_access(low);
                tracker.observe_actual_access(high);
            }
            2 => {
                tracker.observe_actual_access(HybridAccess::HostMutation {
                    address: after.sp,
                    word: true,
                    value: 0,
                });
                tracker.observe_actual_access(HybridAccess::HostMutation {
                    address: after.sp + 2,
                    word: true,
                    value: (ROOT + 10) as u16,
                });
            }
            3 => {
                tracker.observe_actual_access(high);
                tracker.observe_actual_access(HybridAccess::Write {
                    address: after.sp + 4,
                    word: true,
                    value: (ROOT + 10) as u16,
                });
            }
            4 => {
                tracker.observe_actual_access(high);
                tracker.observe_actual_access(HybridAccess::Write {
                    address: after.sp + 2,
                    word: true,
                    value: 7,
                });
            }
            5 => {
                tracker.observe_actual_access(high);
                tracker.observe_actual_access(HybridAccess::Write {
                    address: after.sp + 2,
                    word: false,
                    value: (ROOT + 10) as u16,
                });
            }
            _ => unreachable!(),
        }
        let update =
            tracker.finish_actual_step(Some(receipt(before, 0x4eb9)), after, Some(ROOT + 10));
        assert!(
            tracker.actual_call(update.entered.unwrap()).is_none(),
            "mutation {mutation}"
        );
        assert!(tracker.current_witness().is_none(), "mutation {mutation}");
    }
}

#[test]
fn unknown_faulted_nonlocal_irq_and_bank_paths_never_rearm_read_permission() {
    let mut unknown = rooted();
    call(
        &mut unknown,
        point(ROOT + 4, SP - 12, 0),
        0x3ffff0,
        ROOT + 10,
    );
    assert!(unknown.current_witness().is_none());
    for opcode in [0x4eb9, 0x4ef9, 0x4e75, 0x4e40, 0x4e73, 0x4afc] {
        let mut tracker = rooted();
        let before = point(ROOT, SP, 0);
        tracker.begin_actual_step(before, opcode, true);
        assert!(tracker.step_witness().is_none(), "opcode {opcode:04x}");
        let mut failed = receipt(before, opcode);
        failed.completed_successfully = false;
        tracker.finish_actual_step(Some(failed), before, None);
        assert!(tracker.current_witness().is_none());
    }
    for altered in [
        point(ROOT, SP, 1),
        InvocationPoint {
            status: 0x0700,
            ..point(ROOT, SP, 0)
        },
    ] {
        let mut tracker = rooted();
        tracker.observe_point(altered);
        tracker.observe_point(point(ROOT, SP, 0));
        assert!(tracker.current_witness().is_none());
    }
}

#[test]
fn actual_irq_context_is_separate_and_raw_call_seals_survive_its_return() {
    let mut tracker = rooted();
    let irq = tracker
        .observe_exception_boundary(m68000_emu::ExceptionBoundary::Entered {
            vector: 30,
            handler: 0x200,
            depth: 1,
        })
        .exception_entered
        .unwrap();
    let record = *tracker.exception_record(irq).unwrap();
    assert_eq!(
        (
            record.vector,
            record.handler,
            record.depth,
            record.occurrence
        ),
        (30, 0x200, 1, 1)
    );
    assert!(record.parent.is_none());
    assert!(record.call_parent.is_some());
    let (id, entry) = call(&mut tracker, point(0x204, SP - 10, 1), child(), 0x20a);
    let raw = tracker.actual_call(id).unwrap();
    assert_eq!(tracker.record(id).unwrap().exception_context(), Some(irq));
    assert!(tracker.current_witness().is_none());
    return_call(&mut tracker, entry, 0x20a);
    assert_eq!(
        tracker
            .observe_exception_boundary(m68000_emu::ExceptionBoundary::Returned { depth: 1 })
            .exception_returned,
        Some(irq)
    );
    tracker.observe_point(point(ROOT, SP, 0));
    assert!(tracker.current_witness().is_none());
    assert!(tracker.resolve_actual(raw).is_some());
    let next = tracker
        .observe_exception_boundary(m68000_emu::ExceptionBoundary::Entered {
            vector: 30,
            handler: 0x200,
            depth: 1,
        })
        .exception_entered
        .unwrap();
    assert_ne!(irq, next);
    assert_eq!(tracker.exception_record(next).unwrap().occurrence, 2);
}

#[test]
fn exhausted_brands_and_child_occurrences_cannot_mint_or_cross_resolve_seals() {
    let mut a = rooted();
    let mut b = rooted();
    a.capture = 0;
    b.capture = 0;
    a.lineage_intact = false;
    b.lineage_intact = false;
    let forged = ActualCallId(InvocationId {
        capture: 0,
        index: 0,
        serial: 0,
    });
    assert!(a.resolve_actual(forged).is_none());
    assert!(b.resolve_actual(forged).is_none());
    assert!(a.actual_call(forged.0).is_none());
    assert!(
        a.exception_record(ExceptionContextId {
            capture: 0,
            index: 0
        })
        .is_none()
    );
    let before = point(ROOT, SP, 0);
    a.begin_actual_step(before, 0x4eb9, true);
    assert_eq!(
        a.finish_actual_step(
            Some(receipt(before, 0x4eb9)),
            point(child(), SP - 4, 0),
            Some(ROOT + 6)
        )
        .invalidated,
        Some(Invalidation::Exhausted)
    );

    let mut tracker = rooted();
    let (id, entry) = call(
        &mut tracker,
        point(ROOT + 4, SP - 12, 0),
        child(),
        ROOT + 10,
    );
    let raw = tracker.actual_call(id).unwrap();
    return_call(&mut tracker, entry, ROOT + 10);
    tracker.active[0].child_occurrences = u64::MAX;
    let before = point(ROOT + 4, SP - 12, 0);
    tracker.begin_actual_step(before, 0x4eb9, true);
    frame_writes(&mut tracker, before.sp - 4, ROOT + 10);
    let update = tracker.finish_actual_step(
        Some(receipt(before, 0x4eb9)),
        point(child(), before.sp - 4, 0),
        Some(ROOT + 10),
    );
    assert_eq!(update.invalidated, Some(Invalidation::Exhausted));
    assert!(update.entered.is_none());
    assert!(
        tracker.resolve_actual(raw).is_some(),
        "old raw seals do not depend on revocable read permission"
    );
    assert!(
        tracker
            .record(InvocationId {
                capture: tracker.capture,
                index: usize::MAX,
                serial: 0,
            })
            .is_none()
    );
}

#[test]
fn rootless_actual_capture_seal_is_not_a_measurement_or_stack_read_witness() {
    let mut captured = InvocationTracker::capturing(Revision::Rev02, None);
    let (id, entry) = call(
        &mut captured,
        point(ROOT + 4, SP - 12, 0),
        child(),
        ROOT + 10,
    );
    let raw = captured.actual_call(id).unwrap();
    assert!(captured.current_witness().is_none());
    assert_eq!(captured.resolve_actual(raw).unwrap().parent(), None);
    return_call(&mut captured, entry, ROOT + 10);
    assert!(captured.resolve_actual(raw).is_some());
    let mut measured = InvocationTracker::observing(Revision::Rev02);
    let update = measured.observe_unrooted_boundary(
        receipt(point(ROOT + 4, SP - 12, 0), 0x4eb9),
        entry,
        Some(ROOT + 10),
    );
    assert!(measured.actual_call(update.entered.unwrap()).is_none());
    assert!(measured.current_witness().is_none());
    assert!(measured.resolve_actual(raw).is_none());
}

#[test]
fn measurement_retirement_never_aliases_a_reused_slot_or_retires_a_proof_seal() {
    let mut measured = InvocationTracker::observing(Revision::Rev02);
    let before = point(ROOT, SP, 0);
    let entry = point(child(), SP - 4, 0);
    let old = measured
        .observe_unrooted_boundary(receipt(before, 0x4eb9), entry, Some(ROOT + 6))
        .entered
        .unwrap();
    assert!(
        !measured.retire_measurement_return(old),
        "an active record cannot retire"
    );
    let returned = measured
        .observe_unrooted_boundary(receipt(entry, 0x4e75), point(ROOT + 6, SP, 0), None)
        .returned
        .unwrap();
    assert_eq!(returned, old);
    assert!(measured.retire_measurement_return(old));
    assert_eq!(measured.measurement_storage().0, 0);
    let next = measured
        .observe_unrooted_boundary(receipt(before, 0x4eb9), entry, Some(ROOT + 6))
        .entered
        .unwrap();
    assert_eq!(
        next.index, old.index,
        "reuse preallocated storage rather than append history"
    );
    assert_ne!(next, old);
    assert!(
        measured.record(old).is_none(),
        "stale IDs cannot bind another call at the same slot"
    );
    assert!(measured.record(next).is_some());
    assert!(measured.actual_call(next).is_none());

    let mut captured = InvocationTracker::capturing(Revision::Rev02, None);
    let (id, entry) = call(&mut captured, before, child(), ROOT + 6);
    let raw = captured.actual_call(id).unwrap();
    return_call(&mut captured, entry, ROOT + 6);
    assert!(
        !captured.retire_measurement_return(id),
        "capture retains immutable completed proof records"
    );
    assert!(captured.resolve_actual(raw).is_some());
    assert_eq!(captured.measurement_storage().0, 1);
    captured.next_record_serial = u64::MAX;
    captured.begin_actual_step(before, 0x4eb9, true);
    frame_writes(&mut captured, SP - 4, ROOT + 6);
    let update = captured.finish_actual_step(Some(receipt(before, 0x4eb9)), entry, Some(ROOT + 6));
    assert_eq!(update.invalidated, Some(Invalidation::Exhausted));
    assert!(update.entered.is_none());
    assert!(captured.resolve_actual(raw).is_some());
}
