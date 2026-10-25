//! Authored guard-state controls. Real framework dispatch is exercised by the
//! Host and NativePlatform integration fixtures; these do not certify a body.
use super::*;
use crate::{ids, stack_read::LocalWidth};
const SP: u32 = 0xff8000;
fn root_frame() -> FrameState {
    FrameState::new(SP)
}
fn frame(owner: FunctionId, sp: u32) -> FrameState {
    FrameState::new(sp).for_prepared_entry(owner)
}
fn enter(
    guard: &Guard,
    owner: FunctionId,
    parent: FrameState,
    entry: FrameState,
) -> Option<MaterializedFrame> {
    let event = CallBoundaryEvent::prepared_enter(
        owner,
        parent,
        entry,
        crate::call_frame::CallSite::new(0x1004),
    );
    guard.observe_call_boundary(&event, entry, entry.entry_sp, false)
}
fn returned(guard: &Guard, owner: FunctionId, parent: FrameState, entry: FrameState) {
    let event = CallBoundaryEvent::ended(
        owner,
        Some(parent),
        Some(entry),
        Some(crate::call_frame::CallSite::new(0x1004)),
        true,
    );
    guard.observe_call_boundary(&event, entry, entry.entry_sp + 4, false);
}
fn read(owner: FunctionId, frame: FrameState, offset: i32) -> UninitializedLocalRead {
    frame.uninitialized_local(
        owner,
        offset,
        LocalWidth::Word,
        "authored own-frame word input",
    )
}
#[test]
fn a_prepared_dto_without_a_real_boundary_is_not_allocation_evidence() {
    let guard = Guard::default();
    let child = frame(ids::GET_RANDOM_NUMBER, SP - 16);
    let error = guard
        .run_root(ids::NO_OP, SP, false, || {
            guard.explicit_local(
                read(ids::GET_RANDOM_NUMBER, child, -8),
                child,
                SP - 32,
                false,
            )
        })
        .unwrap_err();
    assert_eq!(error.reason, UnsupportedReason::InvalidExplicitLocal);
}
#[test]
fn actual_current_prepared_entry_is_bound_to_owner_frame_bank_and_own_extent() {
    let guard = Guard::default();
    let child = frame(ids::GET_RANDOM_NUMBER, SP - 16);
    guard
        .run_root(ids::NO_OP, SP, false, || {
            assert!(enter(&guard, ids::GET_RANDOM_NUMBER, root_frame(), child).is_some());
            assert_eq!(
                guard
                    .explicit_local(
                        read(ids::GET_RANDOM_NUMBER, child, -8),
                        child,
                        SP - 32,
                        false
                    )
                    .unwrap(),
                SP - 24
            );
        })
        .unwrap();
    for mutation in 0..5 {
        let guard = Guard::default();
        let error = guard
            .run_root(ids::NO_OP, SP, false, || {
                assert!(enter(&guard, ids::GET_RANDOM_NUMBER, root_frame(), child).is_some());
                let declared = match mutation {
                    0 => read(ids::ABS, child, -8),
                    1 => read(
                        ids::GET_RANDOM_NUMBER,
                        frame(ids::GET_RANDOM_NUMBER, SP - 20),
                        -8,
                    ),
                    2 => read(ids::GET_RANDOM_NUMBER, child, 0),
                    3 => read(ids::GET_RANDOM_NUMBER, child, -7),
                    _ => read(ids::GET_RANDOM_NUMBER, child, -8),
                };
                guard.explicit_local(declared, child, SP - 32, mutation == 4)
            })
            .unwrap_err();
        assert_eq!(
            error.reason,
            UnsupportedReason::InvalidExplicitLocal,
            "mutation {mutation}"
        );
    }
}
#[test]
fn other_parent_and_elided_ancestor_cannot_materialize_a_descendant() {
    for elided in [false, true] {
        let guard = Guard::default();
        let child = frame(ids::GET_RANDOM_NUMBER, SP - 16);
        let error = guard
            .run_root(ids::NO_OP, SP, false, || {
                let parent = if elided {
                    guard.observe_call_boundary(
                        &CallBoundaryEvent::elided_enter(ids::ABS, Some(root_frame())),
                        root_frame(),
                        SP - 12,
                        false,
                    );
                    root_frame()
                } else {
                    frame(ids::ABS, SP)
                };
                assert!(enter(&guard, ids::GET_RANDOM_NUMBER, parent, child).is_none());
                guard.explicit_local(
                    read(ids::GET_RANDOM_NUMBER, child, -8),
                    child,
                    SP - 32,
                    false,
                )
            })
            .unwrap_err();
        assert_eq!(error.reason, UnsupportedReason::InvalidExplicitLocal);
    }
}
#[test]
fn returned_or_irq_revoked_parent_read_permission_is_never_restored() {
    for irq in [false, true] {
        let guard = Guard::default();
        let child = frame(ids::GET_RANDOM_NUMBER, SP - 16);
        let error = guard
            .run_root(ids::NO_OP, SP, false, || {
                enter(&guard, ids::GET_RANDOM_NUMBER, root_frame(), child).unwrap();
                if irq {
                    guard.revoke_read_permissions();
                    guard.explicit_local(
                        read(ids::GET_RANDOM_NUMBER, child, -8),
                        child,
                        SP - 32,
                        false,
                    )
                } else {
                    returned(&guard, ids::GET_RANDOM_NUMBER, root_frame(), child);
                    guard.explicit_local(
                        read(ids::NO_OP, root_frame(), -8),
                        root_frame(),
                        SP - 12,
                        false,
                    )
                }
            })
            .unwrap_err();
        assert_eq!(error.reason, UnsupportedReason::InvalidExplicitLocal);
    }
}
#[test]
fn shared_parent_token_restores_materialization_only_and_enables_a_fresh_sibling() {
    let guard = Guard::default();
    let parent = frame(ids::ABS, SP - 16);
    let child = frame(ids::NO_OP, SP - 32);
    let sibling = frame(ids::GET_RANDOM_NUMBER, SP - 32);
    guard
        .run_root(ids::NO_OP, SP, false, || {
            let token = enter(&guard, ids::ABS, root_frame(), parent).unwrap();
            enter(&guard, ids::NO_OP, parent, child).unwrap();
            returned(&guard, ids::NO_OP, parent, child);
            assert!(guard.restore_materialization(token, parent, parent.entry_sp - 12, false));
            assert!(!guard.state.lock().unwrap().allocation_read_permitted);
            assert!(!guard.state.lock().unwrap().root_read_permitted);
            enter(&guard, ids::GET_RANDOM_NUMBER, parent, sibling).unwrap();
            assert_eq!(
                guard
                    .explicit_local(
                        read(ids::GET_RANDOM_NUMBER, sibling, -8),
                        sibling,
                        sibling.entry_sp - 16,
                        false
                    )
                    .unwrap(),
                sibling.entry_sp - 8
            );
        })
        .unwrap();
}
#[test]
fn missing_parent_token_stays_unsupported_after_grandchild_return() {
    let guard = Guard::default();
    let parent = frame(ids::ABS, SP - 16);
    let child = frame(ids::NO_OP, SP - 32);
    let sibling = frame(ids::GET_RANDOM_NUMBER, SP - 32);
    let error = guard
        .run_root(ids::NO_OP, SP, false, || {
            enter(&guard, ids::ABS, root_frame(), parent).unwrap();
            enter(&guard, ids::NO_OP, parent, child).unwrap();
            returned(&guard, ids::NO_OP, parent, child);
            assert!(enter(&guard, ids::GET_RANDOM_NUMBER, parent, sibling).is_none());
            guard.explicit_local(
                read(ids::GET_RANDOM_NUMBER, sibling, -8),
                sibling,
                sibling.entry_sp - 16,
                false,
            )
        })
        .unwrap_err();
    assert_eq!(error.reason, UnsupportedReason::InvalidExplicitLocal);
}
#[test]
fn old_sibling_and_epoch_tokens_cannot_restore_an_identical_raw_frame_tuple() {
    let guard = Guard::default();
    let parent = frame(ids::ABS, SP - 16);
    let child = frame(ids::NO_OP, SP - 32);
    let mut old = None;
    guard
        .run_root(ids::NO_OP, SP, false, || {
            let first = enter(&guard, ids::ABS, root_frame(), parent).unwrap();
            enter(&guard, ids::NO_OP, parent, child).unwrap();
            returned(&guard, ids::NO_OP, parent, child);
            assert!(guard.restore_materialization(first, parent, parent.entry_sp - 12, false));
            returned(&guard, ids::ABS, root_frame(), parent);
            let next = enter(&guard, ids::ABS, root_frame(), parent).unwrap();
            enter(&guard, ids::NO_OP, parent, child).unwrap();
            returned(&guard, ids::NO_OP, parent, child);
            assert!(!guard.restore_materialization(first, parent, parent.entry_sp - 12, false));
            assert!(guard.restore_materialization(next, parent, parent.entry_sp - 12, false));
            returned(&guard, ids::ABS, root_frame(), parent);
            old = Some(next);
        })
        .unwrap();
    guard
        .run_root(ids::NO_OP, SP, false, || {
            let current = enter(&guard, ids::ABS, root_frame(), parent).unwrap();
            enter(&guard, ids::NO_OP, parent, child).unwrap();
            returned(&guard, ids::NO_OP, parent, child);
            assert!(!guard.restore_materialization(
                old.unwrap(),
                parent,
                parent.entry_sp - 12,
                false
            ));
            assert!(guard.restore_materialization(current, parent, parent.entry_sp - 12, false));
        })
        .unwrap();
}
#[test]
fn another_guard_token_and_forged_root_dto_cannot_restore_or_reparent() {
    let a = Guard::default();
    let b = Guard::default();
    let parent = frame(ids::ABS, SP - 16);
    let child = frame(ids::NO_OP, SP - 32);
    a.run_root(ids::NO_OP, SP, false, || {
        let foreign = enter(&a, ids::ABS, root_frame(), parent).unwrap();
        b.run_root(ids::NO_OP, SP, false, || {
            let own = enter(&b, ids::ABS, root_frame(), parent).unwrap();
            assert!(
                enter(&b, ids::GET_RANDOM_NUMBER, root_frame(), child).is_none(),
                "an active child cannot be hidden by a copied root FrameState"
            );
            // The malformed reparenting above deliberately loses metadata.
            assert!(!b.restore_materialization(foreign, parent, parent.entry_sp - 12, false));
            assert!(!b.restore_materialization(own, parent, parent.entry_sp - 12, false));
        })
        .unwrap();
    })
    .unwrap();
    let a = Guard::default();
    let b = Guard::default();
    a.run_root(ids::NO_OP, SP, false, || {
        let foreign = enter(&a, ids::ABS, root_frame(), parent).unwrap();
        b.run_root(ids::NO_OP, SP, false, || {
            let own = enter(&b, ids::ABS, root_frame(), parent).unwrap();
            enter(&b, ids::NO_OP, parent, child).unwrap();
            returned(&b, ids::NO_OP, parent, child);
            assert!(!b.restore_materialization(foreign, parent, parent.entry_sp - 12, false));
            assert!(b.restore_materialization(own, parent, parent.entry_sp - 12, false));
        })
        .unwrap();
    })
    .unwrap();
}
